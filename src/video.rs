//! Recibe el flujo H.264, lo decodifica con OpenH264 y deja el último fotograma
//! en RGBA para que la interfaz lo pinte.

use std::io::Read;
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context as _, Result, bail};
use eframe::egui;
use openh264::decoder::Decoder;
use openh264::formats::YUVSource;

use crate::android::protocol::{CODEC_H264, StreamHeader, parse_header};
use crate::android::session::{Shared, read_u32};
use crate::logs::log;

pub fn run(mut s: TcpStream, shared: &Arc<Shared>, ctx: &egui::Context) -> Result<()> {
    let codec = read_u32(&mut s).context("no llegó la cabecera de vídeo")?;
    match codec {
        CODEC_H264 => {}
        0 => bail!("el móvil desactivó el vídeo"),
        1 => bail!("el móvil no pudo iniciar la captura de pantalla"),
        other => bail!("códec de vídeo no soportado: {other:08x}"),
    }

    let mut decoder = Decoder::new().context("no se pudo crear el decodificador H.264")?;
    let mut packet = Vec::new();
    let mut config: Option<Vec<u8>> = None;
    let mut pending_config = false;
    let mut header = [0u8; 12];
    let mut frames = 0u32;
    let mut fps_since = Instant::now();
    let mut errors = 0u32;
    let mut bytes = 0usize;
    let mut log_at = Instant::now();

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
                bytes += packet.len();

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

                match decoder.decode(&packet) {
                    Ok(Some(yuv)) => {
                        let (w, h) = yuv.dimensions();
                        let mut pixels = vec![egui::Color32::TRANSPARENT; w * h];
                        // SAFETY: Color32 es #[repr(C)] con 4 u8 (RGBA), así que el búfer mide w*h*4 bytes.
                        let bytes = unsafe { std::slice::from_raw_parts_mut(pixels.as_mut_ptr() as *mut u8, pixels.len() * 4) };
                        yuv.write_rgba8(bytes);
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
                        frames += 1;
                        errors = 0;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        errors += 1;
                        if errors <= 3 || errors.is_multiple_of(100) {
                            log(format!("Error decodificando fotograma: {e}"));
                        }
                    }
                }

                let elapsed = fps_since.elapsed().as_secs_f32();
                if elapsed >= 1.0 {
                    *shared.fps.lock().unwrap() = frames as f32 / elapsed;
                    if log_at.elapsed().as_secs() >= 5 {
                        log(format!("Vídeo: {:.0} fps, {:.1} Mb/s", frames as f32 / elapsed, bytes as f32 * 8.0 / 1e6 / elapsed));
                        log_at = Instant::now();
                    }
                    bytes = 0;
                    frames = 0;
                    fps_since = Instant::now();
                }
            }
        }
    }
}

/// Simula el servidor del móvil por TCP y comprueba que llegan fotogramas decodificados.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::android::protocol::{PACKET_FLAG_CONFIG, PACKET_FLAG_KEY_FRAME};
    use crate::recorder::nal_units;
    use std::io::Write;
    use std::net::TcpListener;
    use std::process::Command;

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
