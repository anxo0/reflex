//! Recibe el flujo H.264, lo decodifica con OpenH264 y deja el último fotograma
//! en RGBA para que la interfaz lo pinte.

use std::io::Read;
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context as _, Result, bail};
use eframe::egui;

use crate::android::protocol::{CODEC_H264, StreamHeader, parse_header};
use crate::android::session::{Shared, read_u32};
use crate::h264::H264Decoder;
use crate::logs::log;

pub fn run(mut s: TcpStream, shared: &Arc<Shared>, ctx: &egui::Context) -> Result<()> {
    let codec = read_u32(&mut s).context("no llegó la cabecera de vídeo")?;
    match codec {
        CODEC_H264 => {}
        0 => bail!("el móvil desactivó el vídeo"),
        1 => bail!("el móvil no pudo iniciar la captura de pantalla"),
        other => bail!("códec de vídeo no soportado: {other:08x}"),
    }

    let mut decoder = FrameDecoder::new()?;
    let mut packet = Vec::new();
    let mut config: Option<Vec<u8>> = None;
    let mut pending_config = false;
    let mut header = [0u8; 12];

    loop {
        s.read_exact(&mut header)?;
        match parse_header(&header) {
            StreamHeader::Session { width, height } => {
                log(format!("Sesión de vídeo {width}x{height}"));
                let mut size = shared.video_size.lock().unwrap();
                let changed = *size != (0, 0) && *size != (width, height);
                *size = (width, height);
                drop(size);
                if changed {
                    let mut rec = shared.recorder.lock().unwrap();
                    if rec.as_ref().is_some_and(|r| r.is_started())
                        && let Some(r) = rec.take() {
                            drop(rec);
                            match r.finish() {
                                Ok(Some(p)) => shared.notice(format!("La pantalla giró: grabación guardada en {}", p.display())),
                                _ => shared.notice("La pantalla giró y la grabación se detuvo"),
                            }
                        }
                }
            }
            StreamHeader::Packet { pts, config: is_config, key_frame, size } => {
                packet.resize(size as usize, 0);
                s.read_exact(&mut packet)?;
                decoder.count_bytes(packet.len());

                if is_config {
                    // SPS/PPS: se pegan delante del siguiente fotograma (como hace scrcpy),
                    // porque OpenH264 no los aplica si llegan solos.
                    config = Some(packet.clone());
                    pending_config = true;
                    continue;
                }

                if let Some(rec) = shared.recorder.lock().unwrap().as_mut() {
                    let (w, h) = *shared.video_size.lock().unwrap();
                    if let Err(e) = rec.push(&packet, pts, key_frame, config.as_deref(), w, h) {
                        log(format!("Error grabando: {e:#}"));
                    }
                }

                if pending_config {
                    pending_config = false;
                    if let Some(c) = &config {
                        packet.splice(0..0, c.iter().copied());
                    }
                }

                let _ = decoder.decode(&packet, shared, ctx);
            }
        }
    }
}

/// Decodifica H.264 (Annex B), publica cada fotograma en `shared.frame` y lleva la
/// cuenta de fps y bitrate. Lo usan el duplicado de Android y el de iPhone.
pub struct FrameDecoder {
    decoder: H264Decoder,
    frames: u32,
    fps_since: Instant,
    errors: u32,
    concealed: u32,
    bytes: usize,
    log_at: Instant,
}

impl FrameDecoder {
    pub fn new() -> Result<Self> {
        Ok(Self {
            decoder: H264Decoder::new()?,
            frames: 0,
            fps_since: Instant::now(),
            errors: 0,
            concealed: 0,
            bytes: 0,
            log_at: Instant::now(),
        })
    }

    /// Descarta el estado del decodificador (imágenes de referencia incluidas).
    pub fn reset(&mut self) -> Result<()> {
        self.decoder = H264Decoder::new()?;
        Ok(())
    }

    /// Suma bytes recibidos para el bitrate del registro.
    pub fn count_bytes(&mut self, n: usize) {
        self.bytes += n;
    }

    /// Decodifica y publica el fotograma. Devuelve el estado de OpenH264 si no hubo
    /// imagen por un error; con errores ocultados (hay imagen) devuelve `Ok`.
    pub fn decode(&mut self, packet: &[u8], shared: &Shared, ctx: &egui::Context) -> Result<(), i64> {
        let (state, frame) = self.decoder.decode(packet);
        if frame.is_some() && state != 0 {
            self.concealed += 1;
            if self.concealed <= 3 || self.concealed.is_multiple_of(100) {
                log(format!("Vídeo: OpenH264 ocultó errores en un fotograma (estado 0x{state:x})"));
            }
        }
        let result = match frame {
            // La interfaz aún no ha pintado el anterior: convertir este sería trabajo
            // perdido, y a 2160 de alto la conversión cuesta casi lo que decodificar.
            Some(_) if shared.frame.lock().unwrap().seq > shared.shown_seq.load(std::sync::atomic::Ordering::Relaxed) => {
                self.frames += 1;
                self.errors = 0;
                Ok(())
            }
            Some(yuv) => {
                let (w, h) = (yuv.width, yuv.height);
                let mut pixels = vec![egui::Color32::TRANSPARENT; w * h];
                // SAFETY: Color32 es #[repr(C)] con 4 u8 (RGBA), así que el búfer mide w*h*4 bytes.
                let bytes = unsafe { std::slice::from_raw_parts_mut(pixels.as_mut_ptr() as *mut u8, pixels.len() * 4) };
                yuv420_to_rgba(yuv.y, yuv.u, yuv.v, (w, h), yuv.strides, bytes);
                // Muestra 1 de cada 97 píxeles: si todo es casi negro, la pantalla está apagada.
                let dark = pixels.iter().step_by(97).all(|p| p.r() < 10 && p.g() < 10 && p.b() < 10);
                shared.screen_dark.store(dark, std::sync::atomic::Ordering::Relaxed);
                let mut f = shared.frame.lock().unwrap();
                f.image = Some(Arc::new(egui::ColorImage::new([w, h], pixels)));
                f.width = w;
                f.height = h;
                f.seq += 1;
                drop(f);
                ctx.request_repaint();
                self.frames += 1;
                self.errors = 0;
                Ok(())
            }
            None if state == 0 => Ok(()),
            None => {
                self.errors += 1;
                if self.errors <= 3 || self.errors.is_multiple_of(100) {
                    log(format!("Error decodificando fotograma: estado 0x{state:x} de OpenH264"));
                }
                Err(state)
            }
        };

        let elapsed = self.fps_since.elapsed().as_secs_f32();
        if elapsed >= 1.0 {
            *shared.fps.lock().unwrap() = self.frames as f32 / elapsed;
            if self.log_at.elapsed().as_secs() >= 5 {
                log(format!("Vídeo: {:.0} fps, {:.1} Mb/s", self.frames as f32 / elapsed, self.bytes as f32 * 8.0 / 1e6 / elapsed));
                self.log_at = Instant::now();
            }
            self.bytes = 0;
            self.frames = 0;
            self.fps_since = Instant::now();
        }
        result
    }
}

/// YUV 4:2:0 (BT.601, rango limitado, los mismos coeficientes que OpenH264) a RGBA.
/// Aritmética entera, cualquier ancho y repartida entre varios núcleos: la de
/// OpenH264 solo usa SIMD si el ancho es múltiplo de 8, y el iPhone manda 998.
pub fn yuv420_to_rgba(y: &[u8], u: &[u8], v: &[u8], (w, h): (usize, usize), (ys, us, vs): (usize, usize, usize), out: &mut [u8]) {
    // Coeficientes en Q16.
    const Y_MUL: i32 = (255.0 / 219.0 * 65536.0) as i32;
    const RV: i32 = (255.0 / 224.0 * 1.402 * 65536.0) as i32;
    const GV: i32 = (-255.0 / 224.0 * 1.402 * 0.299 / 0.587 * 65536.0) as i32;
    const GU: i32 = (-255.0 / 224.0 * 1.772 * 0.114 / 0.587 * 65536.0) as i32;
    const BU: i32 = (255.0 / 224.0 * 1.772 * 65536.0) as i32;
    let row = |r: usize, dst: &mut [u8]| {
        let (yr, ur, vr) = (&y[r * ys..r * ys + w], &u[(r / 2) * us..], &v[(r / 2) * vs..]);
        for (x, (px, &yv)) in dst.as_chunks_mut::<4>().0.iter_mut().zip(yr).enumerate() {
            let yy = (i32::from(yv) - 16) * Y_MUL + 32768;
            let cu = i32::from(ur[x / 2]) - 128;
            let cv = i32::from(vr[x / 2]) - 128;
            px[0] = ((yy + RV * cv) >> 16).clamp(0, 255) as u8;
            px[1] = ((yy + GV * cv + GU * cu) >> 16).clamp(0, 255) as u8;
            px[2] = ((yy + BU * cu) >> 16).clamp(0, 255) as u8;
            px[3] = 255;
        }
    };
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 8);
    let rows_per = h.div_ceil(threads).max(16);
    std::thread::scope(|s| {
        for (i, chunk) in out[..w * h * 4].chunks_mut(rows_per * w * 4).enumerate() {
            s.spawn(move || {
                for (k, dst) in chunk.chunks_exact_mut(w * 4).enumerate() {
                    row(i * rows_per + k, dst);
                }
            });
        }
    });
}

/// Simula el servidor del móvil por TCP y comprueba que llegan fotogramas decodificados.
#[cfg(test)]
mod tests {
    use super::*;
    use openh264::decoder::Decoder;
    use openh264::formats::YUVSource;
    use crate::android::protocol::{PACKET_FLAG_CONFIG, PACKET_FLAG_KEY_FRAME};
    use crate::recorder::nal_units;
    use std::io::Write;
    use std::net::TcpListener;
    use std::process::Command;

    /// La conversión propia da lo mismo que la de OpenH264 (±1 por redondeo).
    #[test]
    fn rgba_conversion_matches_openh264() {
        use openh264::formats::{RgbSliceU8, YUVBuffer};
        let (w, h) = (998, 64);
        let rgb: Vec<u8> = (0..w * h).flat_map(|p| [(p * 7 % 256) as u8, (p / w * 3) as u8, (p % w) as u8]).collect();
        let bits = openh264::encoder::Encoder::new().unwrap().encode(&YUVBuffer::from_rgb8_source(RgbSliceU8::new(&rgb, (w, h)))).unwrap().to_vec();
        let mut dec = Decoder::new().unwrap();
        let yuv = dec.decode(&bits).unwrap().unwrap();
        let mut theirs = vec![0u8; w * h * 4];
        yuv.write_rgba8(&mut theirs);
        let mut ours = vec![0u8; w * h * 4];
        yuv420_to_rgba(yuv.y(), yuv.u(), yuv.v(), (w, h), yuv.strides(), &mut ours);
        let max = theirs.iter().zip(&ours).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        assert!(max <= 1, "diferencia máxima {max}");
    }

    #[test]
    fn decodes_stream_from_fake_server() {
        let Ok(out) = Command::new("ffmpeg")
            .args(["-v", "error", "-f", "lavfi", "-i", "testsrc=size=480x270:rate=30", "-t", "1", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .args(["-profile:v", "main", "-bf", "0", "-g", "30", "-x264-params", "aud=1", "-f", "h264", "-"])
            .output()
        else {
            eprintln!("ffmpeg no disponible: prueba omitida");
            return;
        };
        // Igual que MediaCodec: un paquete de configuración (SPS+PPS) y luego un paquete por fotograma.
        let mut config = Vec::new();
        let mut frames: Vec<Vec<u8>> = Vec::new();
        for nal in nal_units(&out.stdout) {
            let t = nal[0] & 0x1f;
            let target = match t {
                7 | 8 => &mut config,
                9 => {
                    frames.push(Vec::new());
                    continue;
                }
                _ => frames.last_mut().unwrap(),
            };
            target.extend_from_slice(&[0, 0, 0, 1]);
            target.extend_from_slice(nal);
        }
        frames.retain(|f| !f.is_empty());
        assert!(out.status.success() && frames.len() == 30, "ffmpeg no generó el vídeo de prueba");
        let n = frames.len();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            s.write_all(&CODEC_H264.to_be_bytes()).unwrap();
            let mut session = [0u8; 12];
            session[0] = 0x80;
            session[4..8].copy_from_slice(&480u32.to_be_bytes());
            session[8..12].copy_from_slice(&270u32.to_be_bytes());
            s.write_all(&session).unwrap();
            let send = |s: &mut TcpStream, flags: u64, data: &[u8]| {
                s.write_all(&flags.to_be_bytes()).unwrap();
                s.write_all(&(data.len() as u32).to_be_bytes()).unwrap();
                s.write_all(data).unwrap();
            };
            send(&mut s, PACKET_FLAG_CONFIG, &config);
            for (i, f) in frames.iter().enumerate() {
                let key = if i % 30 == 0 { PACKET_FLAG_KEY_FRAME } else { 0 };
                send(&mut s, key | (i as u64 * 33_333), f);
            }
        });

        let shared = Arc::new(Shared::new());
        let sock = TcpStream::connect(addr).unwrap();
        let r = run(sock, &shared, &egui::Context::default());
        eprintln!("resultado: {r:?}, fotogramas enviados: {n}");
        assert!(r.is_err(), "el flujo termina con EOF");
        assert_eq!(*shared.video_size.lock().unwrap(), (480, 270));
        let f = shared.frame.lock().unwrap();
        assert_eq!((f.width, f.height), (480, 270));
        assert!(f.seq as usize >= n - 1, "fotogramas decodificados: {} de {n}", f.seq);
        assert_eq!(f.image.as_ref().map(|i| i.pixels.len()), Some(480 * 270));
    }
}
