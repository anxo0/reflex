//! Reproduce el audio del móvil: PCM 16 bits estéreo a 48 kHz (`audio_codec=raw`).

use std::collections::VecDeque;
use std::io::Read;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, anyhow, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::android::protocol::{CODEC_RAW_AUDIO, StreamHeader, parse_header};
use crate::android::session::{Shared, read_u32};
use crate::logs::log;

const IN_RATE: f64 = 48_000.0;

pub fn run(mut s: TcpStream, shared: &Arc<Shared>) -> Result<()> {
    let codec = read_u32(&mut s)?;
    match codec {
        CODEC_RAW_AUDIO => {}
        0 => {
            log("El móvil no envía audio (hace falta Android 11 o superior)");
            shared.notice("Audio no disponible: hace falta Android 11 o superior");
            return drain(s);
        }
        1 => bail!("el móvil no pudo capturar el audio"),
        other => bail!("códec de audio no soportado: {other:08x}"),
    }

    let mut player = Player::new()?;
    let mut header = [0u8; 12];
    let mut packet = Vec::new();
    loop {
        s.read_exact(&mut header)?;
        if let StreamHeader::Packet { config, size, .. } = parse_header(&header) {
            packet.resize(size as usize, 0);
            s.read_exact(&mut packet)?;
            if !config {
                player.push_s16le(&packet);
            }
        }
    }
}

/// Mantiene el socket leído aunque no haya audio, para que el servidor no se bloquee.
fn drain(mut s: TcpStream) -> Result<()> {
    let mut buf = [0u8; 4096];
    while s.read(&mut buf)? > 0 {}
    Ok(())
}

struct Player {
    _stream: cpal::Stream,
    buf: Arc<Mutex<VecDeque<f32>>>,
    out_rate: f64,
    out_channels: usize,
    max_buffered: usize,
    /// Posición fraccionaria entre la muestra anterior y la actual (remuestreo lineal).
    t: f64,
    prev: (f32, f32),
}

impl Player {
    fn new() -> Result<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or_else(|| anyhow!("no hay salida de audio en el PC"))?;
        let supported = device.default_output_config().context("no se pudo leer la configuración de audio")?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let out_rate = config.sample_rate as f64;
        let out_channels = config.channels as usize;
        let buf = Arc::new(Mutex::new(VecDeque::<f32>::new()));

        let stream = match format {
            cpal::SampleFormat::F32 => build::<f32>(&device, &config, buf.clone())?,
            cpal::SampleFormat::I16 => build::<i16>(&device, &config, buf.clone())?,
            cpal::SampleFormat::U16 => build::<u16>(&device, &config, buf.clone())?,
            cpal::SampleFormat::I32 => build::<i32>(&device, &config, buf.clone())?,
            other => bail!("formato de audio del PC no soportado: {other:?}"),
        };
        stream.play().context("no se pudo iniciar la reproducción")?;
        log(format!("Audio: salida a {out_rate} Hz, {out_channels} canales"));

        Ok(Self {
            _stream: stream,
            buf,
            out_rate,
            out_channels,
            // ~150 ms como máximo: si se acumula más, se descarta para no perder sincronía.
            max_buffered: (out_rate * 0.15) as usize * out_channels,
            t: 0.0,
            prev: (0.0, 0.0),
        })
    }

    fn push_s16le(&mut self, pcm: &[u8]) {
        let mut out = Vec::with_capacity(pcm.len());
        let step = IN_RATE / self.out_rate;
        for frame in pcm.as_chunks::<4>().0 {
            let l = i16::from_le_bytes([frame[0], frame[1]]) as f32 / 32768.0;
            let r = i16::from_le_bytes([frame[2], frame[3]]) as f32 / 32768.0;
            if (self.out_rate - IN_RATE).abs() < 1.0 {
                emit(&mut out, self.out_channels, l, r);
            } else {
                while self.t <= 1.0 {
                    let t = self.t as f32;
                    emit(&mut out, self.out_channels, self.prev.0 + (l - self.prev.0) * t, self.prev.1 + (r - self.prev.1) * t);
                    self.t += step;
                }
                self.t -= 1.0;
            }
            self.prev = (l, r);
        }
        let mut b = self.buf.lock().unwrap();
        b.extend(out);
        if b.len() > self.max_buffered {
            let excess = b.len() - self.max_buffered / 2;
            let excess = excess - excess % self.out_channels;
            b.drain(..excess);
        }
    }
}

fn emit(out: &mut Vec<f32>, channels: usize, l: f32, r: f32) {
    match channels {
        1 => out.push((l + r) * 0.5),
        n => {
            for c in 0..n {
                out.push(match c {
                    0 => l,
                    1 => r,
                    _ => 0.0,
                });
            }
        }
    }
}

fn build<T>(device: &cpal::Device, config: &cpal::StreamConfig, buf: Arc<Mutex<VecDeque<f32>>>) -> Result<cpal::Stream>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let stream = device.build_output_stream(
        *config,
        move |data: &mut [T], _| {
            let mut b = buf.lock().unwrap();
            for s in data.iter_mut() {
                *s = T::from_sample(b.pop_front().unwrap_or(0.0));
            }
        },
        |e| log(format!("Error de audio: {e}")),
        None,
    )?;
    Ok(stream)
}
