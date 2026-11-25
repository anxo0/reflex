//! Graba el flujo H.264 tal cual llega (sin recomprimir) en un archivo MP4.

use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use bytes::Bytes;
use mp4::{AvcConfig, MediaConfig, Mp4Config, Mp4Sample, Mp4Writer, TrackConfig, TrackType};

const TIMESCALE: u32 = 1_000_000; // los pts de scrcpy van en microsegundos
const DEFAULT_FRAME_US: u32 = 16_667;

pub struct Recorder {
    path: PathBuf,
    writer: Option<Mp4Writer<BufWriter<File>>>,
    first_pts: u64,
    /// Guardamos una muestra para conocer su duración al llegar la siguiente.
    pending: Option<(u64, Vec<u8>, bool)>,
}

impl Recorder {
    pub fn new(path: PathBuf) -> Self {
        Self { path, writer: None, first_pts: 0, pending: None }
    }

    pub fn is_started(&self) -> bool {
        self.writer.is_some()
    }

    pub fn push(&mut self, data: &[u8], pts: u64, key_frame: bool, config: Option<&[u8]>, width: u32, height: u32) -> Result<()> {
        if self.writer.is_none() {
            // Empezamos en un fotograma clave, con SPS/PPS conocidos.
            if !key_frame {
                return Ok(());
            }
            let mut sps = None;
            let mut pps = None;
            for nal in config.into_iter().flat_map(nal_units).chain(nal_units(data)) {
                match nal.first().map(|b| b & 0x1f) {
                    Some(7) if sps.is_none() => sps = Some(nal.to_vec()),
                    Some(8) if pps.is_none() => pps = Some(nal.to_vec()),
                    _ => {}
                }
            }
            let (Some(sps), Some(pps)) = (sps, pps) else { return Ok(()) };
            if let Some(dir) = self.path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let file = File::create(&self.path).with_context(|| format!("no se pudo crear {}", self.path.display()))?;
            let cfg = Mp4Config {
                major_brand: "isom".parse()?,
                minor_version: 512,
                compatible_brands: ["isom", "iso2", "avc1", "mp41"].iter().map(|b| b.parse()).collect::<Result<_, _>>()?,
                timescale: 1000,
            };
            let mut w = Mp4Writer::write_start(BufWriter::new(file), &cfg)?;
            w.add_track(&TrackConfig {
                track_type: TrackType::Video,
                timescale: TIMESCALE,
                language: "und".into(),
                media_conf: MediaConfig::AvcConfig(AvcConfig { width: width as u16, height: height as u16, seq_param_set: sps, pic_param_set: pps }),
            })?;
            self.writer = Some(w);
            self.first_pts = pts;
        }

        let sample = annexb_to_avcc(data);
        if sample.is_empty() {
            return Ok(());
        }
        let rel = pts.saturating_sub(self.first_pts);
        if let Some((prev_pts, prev, prev_key)) = self.pending.take() {
            let dur = rel.saturating_sub(prev_pts).clamp(1, u32::MAX as u64) as u32;
            self.write(prev_pts, dur, prev, prev_key)?;
        }
        self.pending = Some((rel, sample, key_frame));
        Ok(())
    }

    fn write(&mut self, start: u64, duration: u32, data: Vec<u8>, sync: bool) -> Result<()> {
        if let Some(w) = self.writer.as_mut() {
            w.write_sample(1, &Mp4Sample { start_time: start, duration, rendering_offset: 0, is_sync: sync, bytes: Bytes::from(data) })?;
        }
        Ok(())
    }

    /// Cierra el archivo. Devuelve `None` si no llegó a empezar.
    pub fn finish(mut self) -> Result<Option<PathBuf>> {
        if self.writer.is_none() {
            return Ok(None);
        }
        if let Some((pts, data, key)) = self.pending.take() {
            self.write(pts, DEFAULT_FRAME_US, data, key)?;
        }
        if let Some(mut w) = self.writer.take() {
            w.write_end()?;
        }
        Ok(Some(self.path))
    }
}

/// Separa un flujo Annex B (códigos de inicio 00 00 01) en unidades NAL.
pub fn nal_units(data: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut units = Vec::with_capacity(starts.len());
    for (k, &s) in starts.iter().enumerate() {
        let mut end = starts.get(k + 1).map(|&n| n - 3).unwrap_or(data.len());
        // Quita el cero extra de un código de inicio de 4 bytes y ceros de relleno.
        while end > s && data[end - 1] == 0 {
            end -= 1;
        }
        if end > s {
            units.push(&data[s..end]);
        }
    }
    units.into_iter()
}

/// Annex B → AVCC (longitud de 4 bytes delante de cada NAL), sin SPS/PPS/AUD.
fn annexb_to_avcc(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 16);
    for nal in nal_units(data) {
        if matches!(nal[0] & 0x1f, 7..=9) {
            continue;
        }
        out.extend_from_slice(&(nal.len() as u32).to_be_bytes());
        out.extend_from_slice(nal);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_nal_units() {
        let data = [0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 0, 0, 0, 1, 0x65, 4, 5];
        let units: Vec<_> = nal_units(&data).collect();
        assert_eq!(units, vec![&[0x67, 1, 2][..], &[0x68, 3][..], &[0x65, 4, 5][..]]);
        let avcc = annexb_to_avcc(&data);
        assert_eq!(avcc, vec![0, 0, 0, 3, 0x65, 4, 5]);
    }
}

/// Prueba de extremo a extremo con un H.264 real generado por ffmpeg (se salta si no hay ffmpeg).
#[cfg(test)]
mod ffmpeg_tests {
    use super::*;
    use openh264::decoder::Decoder;
    use openh264::formats::YUVSource;
    use std::process::Command;

    fn generate(profile: &str) -> Option<Vec<u8>> {
        let out = Command::new("ffmpeg")
            .args(["-v", "error", "-f", "lavfi", "-i", "testsrc=size=640x360:rate=30", "-t", "2", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .args(["-profile:v", profile, "-bf", "0", "-g", "30", "-x264-params", "aud=1", "-f", "h264", "-"])
            .output()
            .ok()?;
        out.status.success().then_some(out.stdout)
    }

    /// Divide el flujo en unidades de acceso usando los AUD (NAL 9), como hace MediaCodec.
    fn access_units(data: &[u8]) -> Vec<Vec<u8>> {
        let mut aus: Vec<Vec<u8>> = Vec::new();
        for nal in nal_units(data) {
            if nal[0] & 0x1f == 9 || aus.is_empty() {
                aus.push(Vec::new());
            }
            let au = aus.last_mut().unwrap();
            au.extend_from_slice(&[0, 0, 0, 1]);
            au.extend_from_slice(nal);
        }
        aus.retain(|a| a.len() > 8);
        aus
    }

    fn check(profile: &str) {
        let Some(stream) = generate(profile) else {
            eprintln!("ffmpeg no disponible: prueba omitida");
            return;
        };
        let aus = access_units(&stream);
        assert_eq!(aus.len(), 60, "unidades de acceso");

        let mut dec = Decoder::new().unwrap();
        let mut decoded = 0;
        let dir = std::env::temp_dir().join(format!("reflex_test_{profile}"));
        let path = dir.join("out.mp4");
        let mut rec = Recorder::new(path.clone());
        for (i, au) in aus.iter().enumerate() {
            if let Some(yuv) = dec.decode(au).unwrap() {
                assert_eq!(yuv.dimensions(), (640, 360));
                decoded += 1;
            }
            let key = nal_units(au).any(|n| n[0] & 0x1f == 5);
            rec.push(au, i as u64 * 33_333, key, None, 640, 360).unwrap();
        }
        assert!(decoded >= 58, "{profile}: solo se decodificaron {decoded} fotogramas");
        assert_eq!(rec.finish().unwrap(), Some(path.clone()));

        if let Ok(o) = Command::new("ffprobe")
            .args(["-v", "error", "-count_frames", "-select_streams", "v:0", "-show_entries", "stream=nb_read_frames,width", "-of", "csv=p=0"])
            .arg(&path)
            .output()
        {
            let s = String::from_utf8_lossy(&o.stdout);
            assert_eq!(s.trim(), "640,60", "ffprobe del MP4 grabado");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn decode_and_record_baseline() {
        check("baseline");
    }

    #[test]
    fn decode_and_record_high() {
        check("high");
    }
}

#[cfg(test)]
mod bench {
    use super::nal_units;
    use openh264::decoder::Decoder;
    use openh264::formats::YUVSource;
    use std::process::Command;
    use std::time::Instant;

    /// `cargo test --release bench_decode -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_decode() {
        let out = Command::new("ffmpeg")
            .args(["-v", "error", "-f", "lavfi", "-i", "testsrc2=size=864x1920:rate=60", "-t", "2", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .args(["-profile:v", "high", "-bf", "0", "-g", "60", "-b:v", "8M", "-x264-params", "aud=1", "-f", "h264", "-"])
            .output()
            .unwrap();
        let mut aus: Vec<Vec<u8>> = Vec::new();
        for nal in nal_units(&out.stdout) {
            if nal[0] & 0x1f == 9 || aus.is_empty() {
                aus.push(Vec::new());
            }
            let au = aus.last_mut().unwrap();
            au.extend_from_slice(&[0, 0, 0, 1]);
            au.extend_from_slice(nal);
        }
        let mut dec = Decoder::new().unwrap();
        let mut rgba = vec![0u8; 864 * 1920 * 4];
        let (mut td, mut tc, mut n) = (0.0, 0.0, 0);
        for au in &aus {
            let t0 = Instant::now();
            let yuv = dec.decode(au).unwrap();
            let t1 = Instant::now();
            if let Some(y) = yuv {
                y.write_rgba8(&mut rgba);
                n += 1;
            }
            td += (t1 - t0).as_secs_f64();
            tc += t1.elapsed().as_secs_f64();
        }
        println!("{n} fotogramas: decodificar {:.2} ms, convertir a RGBA {:.2} ms por fotograma", td * 1000.0 / n as f64, tc * 1000.0 / n as f64);
    }
}
