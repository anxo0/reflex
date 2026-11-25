//! Flujo de vídeo del duplicado de pantalla.
//!
//! El iPhone abre una conexión TCP al `dataPort` que le damos en SETUP y manda
//! paquetes con una cabecera de 128 bytes:
//!
//! | bytes  | contenido                                                        |
//! |--------|------------------------------------------------------------------|
//! | 0..4   | tamaño de la carga (little-endian)                               |
//! | 4      | tipo: 0 vídeo cifrado, 1 configuración (avcC), 2/5 informes      |
//! | 6      | opciones (0x56 en la configuración = el vídeo se va a pausar)    |
//! | 8..16  | marca de tiempo NTP (little-endian)                              |
//! | 56..64 | ancho y alto de la pantalla, `f32` (solo en la configuración)    |
//!
//! El vídeo va en formato AVCC (cada NAL precedido de su longitud) y cifrado con
//! AES-128-CTR. El flujo de clave es continuo entre paquetes.

use std::io::Read;
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use aes::Aes128;
use anyhow::{Result, bail};
use ctr::cipher::{KeyIvInit, StreamCipher};
use eframe::egui;
use sha2::{Digest, Sha512};

use super::ntp::ntp_to_micros;
use super::sps;
use crate::android::session::Shared;
use crate::logs::log;
use crate::video::FrameDecoder;

type Aes128Ctr = ctr::Ctr128BE<Aes128>;

const START_CODE: [u8; 4] = [0, 0, 0, 1];

/// Claves del flujo: SHA-512 de `"AirPlayStreamKey<id>"` / `"AirPlayStreamIV<id>"`
/// seguido de la clave AES de la sesión; se usan los 16 primeros bytes.
pub fn stream_cipher(aes_key: &[u8; 16], stream_connection_id: u64) -> Aes128Ctr {
    let derive = |label: &str| -> [u8; 16] {
        let h = Sha512::new().chain_update(format!("{label}{stream_connection_id}")).chain_update(aes_key).finalize();
        h[..16].try_into().unwrap()
    };
    Aes128Ctr::new(&derive("AirPlayStreamKey").into(), &derive("AirPlayStreamIV").into())
}

/// Recibe el vídeo hasta que el iPhone cierra la conexión o se activa `cancel`.
///
/// Un hilo solo lee de la red y otro descifra y decodifica. Así el socket se vacía
/// al momento aunque decodificar un fotograma cueste 10-15 ms: si no, el iPhone ve
/// la conexión lenta y baja la calidad del vídeo.
pub fn run(s: TcpStream, mut cipher: Aes128Ctr, shared: &Arc<Shared>, ctx: &egui::Context, cancel: &AtomicBool) -> Result<()> {
    // Lectura no bloqueante para poder cancelar: en Windows cerrar el socket desde
    // otro hilo no despierta un `recv` bloqueado, y tras un timeout de lectura
    // (SO_RCVTIMEO) el estado del socket queda indeterminado.
    s.set_nonblocking(true)?;
    let (tx, rx) = std::sync::mpsc::sync_channel::<([u8; 128], Vec<u8>)>(600);
    let stop_reader = AtomicBool::new(false);
    let queued = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let (stop, queue) = (&stop_reader, &queued);
        let reader = scope.spawn(move || read_packets(s, &tx, cancel, stop, queue));
        let result = process(&rx, &mut cipher, shared, ctx, &queued);
        stop_reader.store(true, Ordering::Relaxed);
        drop(rx);
        let read_result = reader.join().unwrap_or_else(|_| Err(anyhow::anyhow!("el hilo de lectura del vídeo se cayó")));
        result.and(read_result)
    })
}

/// Lee paquetes (cabecera de 128 bytes + carga) y los pasa al hilo de proceso.
fn read_packets(
    mut s: TcpStream,
    tx: &std::sync::mpsc::SyncSender<([u8; 128], Vec<u8>)>,
    cancel: &AtomicBool,
    stop: &AtomicBool,
    queued: &AtomicUsize,
) -> Result<()> {
    let mut header = [0u8; 128];
    loop {
        if read_full(&mut s, &mut header, cancel, stop)? {
            return Ok(());
        }
        let size = u32::from_le_bytes(header[..4].try_into().unwrap()) as usize;
        if size > 32 * 1024 * 1024 {
            bail!("paquete de vídeo demasiado grande ({size} bytes)");
        }
        let mut payload = vec![0u8; size];
        if read_full(&mut s, &mut payload, cancel, stop)? {
            return Ok(());
        }
        queued.fetch_add(1, Ordering::Relaxed);
        if tx.send((header, payload)).is_err() {
            return Ok(());
        }
    }
}

fn process(
    rx: &std::sync::mpsc::Receiver<([u8; 128], Vec<u8>)>,
    cipher: &mut Aes128Ctr,
    shared: &Arc<Shared>,
    ctx: &egui::Context,
    queued: &AtomicUsize,
) -> Result<()> {
    let mut decoder = FrameDecoder::new()?;
    let mut config: Option<Vec<u8>> = None;
    // Configuración (SPS/PPS) que tiene ya el decodificador. Solo se le vuelve a dar
    // si cambia o con un fotograma clave: OpenH264 trata un SPS repetido como una
    // secuencia nueva y descarta las referencias, y el iPhone la reenvía tras cada pausa.
    let mut applied_config: Option<Vec<u8>> = None;
    let mut first_ts: Option<u64> = None;
    let mut paused = false;
    let mut bad_packets = 0u32;
    // Diagnóstico: tipos de NAL de los primeros fotogramas tras cada configuración,
    // tipos de paquete desconocidos (una vez cada uno) e informes del iPhone.
    let mut frames_to_describe = 3u32;
    let mut warned_no_config = false;
    let mut seen_kinds = [false; 256];
    let mut reports = 0u32;
    let mut stats = Stats::new(shared);

    loop {
        let (header, mut payload) = match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(p) => p,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                stats.idle(false);
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        };
        let pending = queued.fetch_sub(1, Ordering::Relaxed).saturating_sub(1);
        let size = payload.len();
        stats.packet(header[4], size, pending);
        decoder.count_bytes(size + 128);
        let ts = u64::from_le_bytes(header[8..16].try_into().unwrap());

        match header[4] {
            0 => {
                cipher.apply_keystream(&mut payload);
                let Some((mut frame, key_frame)) = avcc_to_annexb(&payload) else {
                    bad_packets += 1;
                    if bad_packets <= 3 {
                        log(format!("AirPlay: paquete de vídeo mal formado ({size} bytes), ¿clave incorrecta?"));
                    }
                    continue;
                };
                let base = *first_ts.get_or_insert(ts);
                let pts = ntp_to_micros(ts).saturating_sub(ntp_to_micros(base));

                if let Some(rec) = shared.recorder.lock().unwrap().as_mut() {
                    let (w, h) = *shared.video_size.lock().unwrap();
                    if let Err(e) = rec.push(&frame, pts, key_frame, config.as_deref(), w, h) {
                        log(format!("Error grabando: {e:#}"));
                    }
                }
                if frames_to_describe > 0 {
                    frames_to_describe -= 1;
                    log(format!("AirPlay: fotograma de {size} bytes, NAL {:?}", nal_types(&frame)));
                }
                if config.is_none() && !warned_no_config {
                    warned_no_config = true;
                    log("AirPlay: llegan fotogramas pero el iPhone aún no ha mandado la configuración (SPS/PPS)");
                }
                // SPS/PPS delante del primer fotograma, de cada clave y cuando cambian.
                if let Some(c) = &config
                    && (key_frame || applied_config.as_ref() != Some(c))
                {
                    applied_config = Some(c.clone());
                    frame.splice(0..0, c.iter().copied());
                }
                let result = decoder.decode(&frame, shared, ctx);
                stats.decoded(key_frame, result);
                match result {
                    Ok(()) => {}
                    // dsOutOfMemory: OpenH264 se quedó sin imágenes. Se empieza de cero y se
                    // vuelve a dar la configuración; la ocultación de errores tapa el hueco.
                    Err(code) if code & crate::h264::DS_OUT_OF_MEMORY != 0 => {
                        log("AirPlay: el decodificador se quedó sin memoria de imágenes; se reinicia");
                        decoder.reset()?;
                        applied_config = None;
                    }
                    // dsNoParamSets: se reenvían SPS/PPS con el siguiente fotograma.
                    Err(code) if code & crate::h264::DS_NO_PARAM_SETS != 0 => applied_config = None,
                    Err(_) => {}
                }
            }
            1 => {
                let stopping = header[6] == 0x56 || header[6] == 0x5e;
                if stopping != paused {
                    paused = stopping;
                    log(if paused { "AirPlay: el iPhone ha pausado el vídeo" } else { "AirPlay: el vídeo se reanuda" });
                }
                if payload.len() >= 8 && &payload[4..8] == b"hvc1" {
                    bail!("el iPhone envía H.265, que aún no está soportado");
                }
                let Some(c) = avcc_config(&payload) else {
                    let head: String = payload.iter().take(32).map(|b| format!("{b:02x}")).collect();
                    log(format!("AirPlay: configuración de vídeo no válida ({size} bytes): {head}"));
                    continue;
                };
                if config.as_ref() != Some(&c) {
                    log(format!(
                        "AirPlay: configuración H.264 perfil {} nivel {} ({size} bytes, NAL {:?})",
                        payload[1],
                        payload[3],
                        nal_types(&c)
                    ));
                }
                frames_to_describe = 3;
                let w = f32::from_le_bytes(header[56..60].try_into().unwrap());
                let h = f32::from_le_bytes(header[60..64].try_into().unwrap());
                if w >= 1.0 && h >= 1.0 {
                    let size = (w as u32, h as u32);
                    let mut current = shared.video_size.lock().unwrap();
                    if *current != size {
                        log(format!("AirPlay: pantalla {}x{}", size.0, size.1));
                        *current = size;
                    }
                }
                config = Some(c);
            }
            // 5: informe del iPhone (plist). Se registra de vez en cuando.
            5 => {
                if reports.is_multiple_of(10) {
                    log(format!("AirPlay: informe del iPhone: {}", describe_report(&payload)));
                }
                reports += 1;
            }
            // 2: latido del protocolo antiguo.
            2 => {}
            kind => {
                if !seen_kinds[kind as usize] {
                    seen_kinds[kind as usize] = true;
                    let head: String = header[4..8].iter().map(|b| format!("{b:02x}")).collect();
                    log(format!("AirPlay: paquete de tipo desconocido {head} ({size} bytes)"));
                }
            }
        }
    }
}

/// Lee exactamente `buf.len()` bytes. Devuelve `true` si el iPhone cerró la conexión
/// limpiamente antes de empezar un paquete, o si se ha cancelado.
fn read_full(s: &mut TcpStream, buf: &mut [u8], cancel: &AtomicBool, stop: &AtomicBool) -> Result<bool> {
    let mut read = 0;
    while read < buf.len() {
        if cancel.load(Ordering::Relaxed) || stop.load(Ordering::Relaxed) {
            return Ok(true);
        }
        match s.read(&mut buf[read..]) {
            Ok(0) if read == 0 => return Ok(true),
            Ok(0) => bail!("el iPhone cortó la conexión a mitad de un paquete"),
            Ok(n) => read += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(1)),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(false)
}

/// Resumen periódico en el registro, para distinguir si el iPhone deja de mandar
/// vídeo, si falla la decodificación o si la interfaz deja de pintar.
struct Stats<'a> {
    shared: &'a Shared,
    since: Instant,
    last_data: Instant,
    idle_warned: bool,
    video: u32,
    key_frames: u32,
    bytes: usize,
    configs: u32,
    reports: u32,
    ok: u32,
    errors: u32,
    last_error: i64,
    published_before: u64,
    shown_before: u64,
    max_queue: usize,
}

impl<'a> Stats<'a> {
    fn new(shared: &'a Shared) -> Self {
        let now = Instant::now();
        Self {
            shared,
            since: now,
            last_data: now,
            idle_warned: false,
            video: 0,
            key_frames: 0,
            bytes: 0,
            configs: 0,
            reports: 0,
            ok: 0,
            errors: 0,
            last_error: 0,
            published_before: shared.frame.lock().unwrap().seq,
            shown_before: shared.shown_seq.load(Ordering::Relaxed),
            max_queue: 0,
        }
    }

    fn packet(&mut self, kind: u8, size: usize, queued: usize) {
        self.max_queue = self.max_queue.max(queued);
        self.last_data = Instant::now();
        if self.idle_warned {
            self.idle_warned = false;
            log("AirPlay: vuelve a llegar vídeo");
        }
        self.bytes += size + 128;
        match kind {
            0 => self.video += 1,
            1 => self.configs += 1,
            _ => self.reports += 1,
        }
        self.maybe_log();
    }

    fn decoded(&mut self, key_frame: bool, result: Result<(), i64>) {
        self.key_frames += u32::from(key_frame);
        match result {
            Ok(()) => self.ok += 1,
            Err(code) => {
                self.errors += 1;
                self.last_error = code;
            }
        }
    }

    /// Llamada en cada timeout de lectura; `mid_packet` si va a medias de un paquete.
    fn idle(&mut self, mid_packet: bool) {
        let quiet = self.last_data.elapsed();
        if quiet > Duration::from_secs(3) && !self.idle_warned {
            self.idle_warned = true;
            let where_ = if mid_packet { "a mitad de un paquete" } else { "entre paquetes" };
            log(format!("AirPlay: el iPhone no manda vídeo desde hace {:.0} s ({where_})", quiet.as_secs_f32()));
        }
        self.maybe_log();
    }

    fn maybe_log(&mut self) {
        let elapsed = self.since.elapsed();
        if elapsed < Duration::from_secs(5) {
            return;
        }
        let published = self.shared.frame.lock().unwrap().seq;
        let shown = self.shared.shown_seq.load(Ordering::Relaxed);
        let shown_text = if shown == u64::MAX {
            "sin pintar".to_string()
        } else {
            shown.saturating_sub(if self.shown_before == u64::MAX { 0 } else { self.shown_before }).to_string()
        };
        if self.video + self.configs + self.reports > 0 || published != self.published_before {
            log(format!(
                "AirPlay {:.0} s: {} fotogramas ({} clave, {:.1} Mb/s), {} decodificados, {} errores{}, {} publicados, {} pintados, {} config., {} informes, cola máx. {}",
                elapsed.as_secs_f32(),
                self.video,
                self.key_frames,
                self.bytes as f32 * 8.0 / 1e6 / elapsed.as_secs_f32(),
                self.ok,
                self.errors,
                if self.errors > 0 { format!(" (último 0x{:x})", self.last_error) } else { String::new() },
                published - self.published_before,
                shown_text,
                self.configs,
                self.reports,
                self.max_queue,
            ));
        }
        *self = Self { idle_warned: self.idle_warned, last_data: self.last_data, ..Self::new(self.shared) };
    }
}

/// Cambia las longitudes AVCC por códigos de inicio. Devuelve `None` si la
/// estructura no cuadra (suele ser señal de una clave de descifrado errónea).
fn avcc_to_annexb(data: &[u8]) -> Option<(Vec<u8>, bool)> {
    let mut out = Vec::with_capacity(data.len());
    let mut key_frame = false;
    let mut i = 0;
    while i < data.len() {
        let len = u32::from_be_bytes(data.get(i..i + 4)?.try_into().ok()?) as usize;
        let nal = data.get(i + 4..i + 4 + len)?;
        let first = *nal.first()?;
        if first & 0x80 != 0 {
            return None; // forbidden_zero_bit
        }
        key_frame |= first & 0x1f == 5;
        out.extend_from_slice(&START_CODE);
        out.extend_from_slice(nal);
        i += 4 + len;
    }
    (!out.is_empty()).then_some((out, key_frame))
}

/// Resume un informe del iPhone (plist binario, a veces seguido de un bloque de
/// 25 kB sin identificar) como `clave=valor` de sus campos simples.
fn describe_report(payload: &[u8]) -> String {
    let plist_len = if payload.len() > 25_000 { payload.len() - 25_000 } else { payload.len() };
    let Ok(value) = plist::Value::from_reader(std::io::Cursor::new(&payload[..plist_len])) else {
        return format!("{} bytes no legibles", payload.len());
    };
    let mut out = Vec::new();
    flatten(&value, "", &mut out);
    out.join(", ")
}

fn flatten(v: &plist::Value, prefix: &str, out: &mut Vec<String>) {
    use plist::Value;
    match v {
        Value::Dictionary(d) => {
            for (k, v) in d {
                let key = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                flatten(v, &key, out);
            }
        }
        Value::Array(a) => {
            for (i, v) in a.iter().enumerate() {
                flatten(v, &format!("{prefix}[{i}]"), out);
            }
        }
        Value::Boolean(b) => out.push(format!("{prefix}={b}")),
        Value::Integer(i) => out.push(format!("{prefix}={i}")),
        Value::Real(r) => out.push(format!("{prefix}={r:.3}")),
        Value::String(s) => out.push(format!("{prefix}={s}")),
        Value::Data(d) => out.push(format!("{prefix}=<{} bytes>", d.len())),
        _ => {}
    }
}

/// Tipos de NAL de un fragmento en Annex B (para el registro).
fn nal_types(annexb: &[u8]) -> Vec<u8> {
    crate::recorder::nal_units(annexb).filter_map(|n| n.first().map(|b| b & 0x1f)).collect()
}

/// Extrae SPS y PPS de un registro avcC y los devuelve en Annex B.
fn avcc_config(data: &[u8]) -> Option<Vec<u8>> {
    let sps_len = u16::from_be_bytes(data.get(6..8)?.try_into().ok()?) as usize;
    let sps = data.get(8..8 + sps_len)?;
    let pps_at = 8 + sps_len + 1; // 1 byte con el número de PPS
    let pps_len = u16::from_be_bytes(data.get(pps_at..pps_at + 2)?.try_into().ok()?) as usize;
    let pps = data.get(pps_at + 2..pps_at + 2 + pps_len)?;
    // El SPS se pasa tal cual: solo se lee para el registro (ver `sps.rs`).
    match sps::parse(sps) {
        Some(info) => log(format!(
            "AirPlay: SPS perfil {} nivel {}, {} imágenes de referencia, POC tipo {}",
            info.profile, info.level, info.max_num_ref_frames, info.poc_type
        )),
        None => log("AirPlay: no se pudo interpretar el SPS"),
    }
    let mut out = Vec::with_capacity(sps.len() + pps.len() + 8);
    out.extend_from_slice(&START_CODE);
    out.extend_from_slice(sps);
    out.extend_from_slice(&START_CODE);
    out.extend_from_slice(pps);
    Some(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::recorder::nal_units;
    use std::io::Write;
    use std::net::TcpListener;

    #[test]
    fn report_summary() {
        let mut d = plist::Dictionary::new();
        d.insert("fps".into(), plist::Value::Integer(60u64.into()));
        let mut inner = plist::Dictionary::new();
        inner.insert("bitrate".into(), plist::Value::Real(2.5));
        d.insert("video".into(), plist::Value::Dictionary(inner));
        let mut bin = Vec::new();
        plist::Value::Dictionary(d).to_writer_binary(&mut bin).unwrap();
        assert_eq!(describe_report(&bin), "fps=60, video.bitrate=2.500");
        assert!(describe_report(b"basura").contains("no legibles"));
    }

    #[test]
    fn avcc_parsing() {
        let data = [0, 0, 0, 2, 0x65, 0xaa, 0, 0, 0, 1, 0x41];
        let (out, key) = avcc_to_annexb(&data).unwrap();
        assert!(key);
        assert_eq!(out, [0, 0, 0, 1, 0x65, 0xaa, 0, 0, 0, 1, 0x41]);
        assert!(avcc_to_annexb(&[0, 0, 0, 9, 0x65]).is_none());
        assert!(avcc_to_annexb(&[0, 0, 0, 1, 0xe5]).is_none());

        let avcc = [1, 0x64, 0, 0x1f, 0xff, 0xe1, 0, 2, 0x67, 0x11, 1, 0, 1, 0x68];
        assert_eq!(avcc_config(&avcc).unwrap(), [0, 0, 0, 1, 0x67, 0x11, 0, 0, 0, 1, 0x68]);
    }

    /// Vídeo H.264 de prueba hecho con el codificador de OpenH264, ya en el formato
    /// de AirPlay: el registro avcC y un fotograma AVCC por paquete.
    pub(crate) fn sample_stream(w: usize, h: usize, count: usize) -> (Vec<u8>, Vec<Vec<u8>>) {
        use openh264::encoder::Encoder;
        use openh264::formats::{RgbSliceU8, YUVBuffer};

        let mut encoder = Encoder::new().unwrap();
        let (mut sps, mut pps) = (Vec::new(), Vec::new());
        let mut frames: Vec<Vec<u8>> = Vec::new();
        for i in 0..count {
            let rgb: Vec<u8> = (0..w * h).flat_map(|p| [(p % w + i * 8) as u8, (p / w) as u8, (i * 12) as u8]).collect();
            let yuv = YUVBuffer::from_rgb8_source(RgbSliceU8::new(&rgb, (w, h)));
            let bits = encoder.encode(&yuv).unwrap().to_vec();
            let mut frame = Vec::new();
            for nal in nal_units(&bits) {
                match nal[0] & 0x1f {
                    7 => sps = nal.to_vec(),
                    8 => pps = nal.to_vec(),
                    _ => {
                        frame.extend_from_slice(&(nal.len() as u32).to_be_bytes());
                        frame.extend_from_slice(nal);
                    }
                }
            }
            if !frame.is_empty() {
                frames.push(frame);
            }
        }
        assert!(!sps.is_empty() && !pps.is_empty());
        let mut avcc = vec![1, sps[1], sps[2], sps[3], 0xff, 0xe1];
        avcc.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        avcc.extend_from_slice(&sps);
        avcc.push(1);
        avcc.extend_from_slice(&(pps.len() as u16).to_be_bytes());
        avcc.extend_from_slice(&pps);
        (avcc, frames)
    }

    /// Envía el flujo como lo hace el iPhone: configuración en claro, fotogramas
    /// cifrados e informes intercalados que no deben tocar el flujo AES.
    pub(crate) fn send_stream(s: &mut TcpStream, mut cipher: Aes128Ctr, avcc: Vec<u8>, frames: Vec<Vec<u8>>, w: usize, h: usize) {
        let mut send = |kind: u8, mut body: Vec<u8>| {
            let mut head = [0u8; 128];
            head[..4].copy_from_slice(&(body.len() as u32).to_le_bytes());
            head[4] = kind;
            match kind {
                0 => cipher.apply_keystream(&mut body),
                1 => {
                    head[6] = 0x16;
                    head[56..60].copy_from_slice(&(w as f32).to_le_bytes());
                    head[60..64].copy_from_slice(&(h as f32).to_le_bytes());
                }
                _ => {}
            }
            s.write_all(&head).unwrap();
            s.write_all(&body).unwrap();
        };
        let half = frames.len() / 2;
        for (i, f) in frames.into_iter().enumerate() {
            // Como tras una pausa: la misma configuración otra vez, a mitad de flujo.
            if i == 0 || i == half {
                send(1, avcc.clone());
            }
            send(0, f);
            send(5, vec![0; 7]);
        }
    }

    /// Si se pierde un fotograma, la imagen debe seguir avanzando (ocultación de
    /// errores) en vez de congelarse hasta el siguiente fotograma clave.
    #[test]
    fn keeps_showing_after_a_lost_frame() {
        let (w, h) = (320, 240);
        let (avcc, frames) = sample_stream(w, h, 20);
        let config = avcc_config(&avcc).unwrap();
        let shared = Shared::new();
        let ctx = egui::Context::default();
        let mut d = FrameDecoder::new().unwrap();
        let mut published_after_loss = 0;
        for (i, f) in frames.iter().enumerate() {
            if i == 4 {
                continue; // se pierde el quinto fotograma (P)
            }
            let (mut annexb, key) = avcc_to_annexb(f).unwrap();
            assert_eq!(key, i == 0, "solo el primero es clave");
            if i == 0 {
                annexb.splice(0..0, config.iter().copied());
            }
            let before = shared.frame.lock().unwrap().seq;
            let _ = d.decode(&annexb, &shared, &ctx);
            if i > 4 && shared.frame.lock().unwrap().seq > before {
                published_after_loss += 1;
            }
        }
        assert!(published_after_loss >= 10, "solo {published_after_loss} fotogramas tras la pérdida");
    }

    /// Hace de iPhone: cifra un vídeo H.264 real como lo hace AirPlay y comprueba
    /// que se descifra y decodifica.
    #[test]
    fn decodes_encrypted_stream() {
        let (w, h) = (320, 240);
        let (avcc, frames) = sample_stream(w, h, 20);
        let n = frames.len() as u64;
        let key = [9u8; 16];
        let id = 0xfedc_ba98_7654_3210u64;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            send_stream(&mut s, stream_cipher(&key, id), avcc, frames, w, h);
        });

        let shared = Arc::new(Shared::new());
        let sock = TcpStream::connect(addr).unwrap();
        run(sock, stream_cipher(&key, id), &shared, &egui::Context::default(), &AtomicBool::new(false)).unwrap();
        assert_eq!(*shared.video_size.lock().unwrap(), (w as u32, h as u32));
        let f = shared.frame.lock().unwrap();
        assert_eq!((f.width, f.height), (w, h));
        assert!(f.seq >= n - 1, "fotogramas decodificados: {} de {n}", f.seq);
    }
}
