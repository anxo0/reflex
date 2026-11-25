//! Audio del duplicado de pantalla.
//!
//! El iPhone manda el sonido por RTP sobre UDP al `dataPort` que le damos en el
//! SETUP del flujo 96. Cada paquete trae una trama AAC-ELD (44,1 kHz, estéreo, 480
//! muestras) tras la cabecera RTP de 12 bytes. Los bloques completos de 16 bytes van
//! cifrados con AES-128-CBC (clave de la sesión, IV `eiv` del primer SETUP,
//! reiniciado en cada paquete); el resto final va en claro.

use std::net::UdpSocket;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use aes::Aes128;
use aes::cipher::{BlockCipherDecrypt, KeyInit};
use anyhow::{Context as _, Result, bail};

use super::aac;
use crate::audio::Player;
use crate::logs::log;

/// Tipos de compresión (`ct`) que anuncia el SETUP del flujo de audio.
pub const CT_AAC_ELD: u64 = 8;

/// Configuración AAC-ELD que usa el iPhone al duplicar (44,1 kHz, estéreo, 480).
const ELD_CONFIG: [u8; 4] = [0xf8, 0xe8, 0x50, 0x00];

/// Volumen de AirPlay en dB: de -30 (mínimo) a 0; -144 es silencio.
pub fn gain_from_volume(db: f32) -> f32 {
    if db <= -30.0 { 0.0 } else { 10f32.powf(db.min(0.0) / 20.0) }
}

pub struct Stream {
    pub key: [u8; 16],
    pub iv: [u8; 16],
    /// Ganancia lineal (bits de un `f32`), la ajusta `SET_PARAMETER volume`.
    pub gain: Arc<AtomicU32>,
    pub cancel: Arc<AtomicBool>,
}

pub fn run(sock: UdpSocket, stream: Stream) -> Result<()> {
    sock.set_read_timeout(Some(Duration::from_millis(250)))?;
    let mut pipeline = Pipeline::new(&stream.key, stream.iv)?;
    let rate = pipeline.rate;
    let mut player = Player::new().context("no se pudo abrir la salida de audio")?;
    let mut buf = [0u8; 2048];
    while !stream.cancel.load(Ordering::Relaxed) {
        let n = match sock.recv(&mut buf) {
            Ok(n) => n,
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => continue,
            Err(e) => return Err(e.into()),
        };
        let gain = f32::from_bits(stream.gain.load(Ordering::Relaxed));
        if let Some(pcm) = pipeline.push(&mut buf[..n], gain) {
            player.push_stereo(pcm, rate);
        }
    }
    pipeline.finish();
    Ok(())
}

/// De paquete RTP a PCM: orden, descifrado, decodificación y volumen.
struct Pipeline {
    cipher: Aes128,
    iv: [u8; 16],
    decoder: aac::Decoder,
    rate: f64,
    pcm: Vec<f32>,
    last_seq: Option<u16>,
    lost: u64,
    errors: u64,
    frames: u64,
}

impl Pipeline {
    fn new(key: &[u8; 16], iv: [u8; 16]) -> Result<Self> {
        let decoder = aac::Decoder::new(&ELD_CONFIG)?;
        Ok(Self {
            cipher: Aes128::new(&(*key).into()),
            iv,
            rate: f64::from(decoder.config().sample_rate),
            decoder,
            pcm: Vec::with_capacity(2 * aac::FRAME_LEN),
            last_seq: None,
            lost: 0,
            errors: 0,
            frames: 0,
        })
    }

    /// Procesa un paquete. Devuelve el PCM estéreo intercalado si era una trama válida.
    fn push(&mut self, packet: &mut [u8], gain: f32) -> Option<&[f32]> {
        let (seq, payload) = parse_rtp(packet)?;
        // Los paquetes repetidos o que llegan tarde se descartan.
        if let Some(last) = self.last_seq {
            let delta = seq.wrapping_sub(last);
            if delta == 0 || delta > 0x8000 {
                return None;
            }
            self.lost += u64::from(delta - 1);
        }
        self.last_seq = Some(seq);

        decrypt(&self.cipher, &self.iv, payload);
        self.pcm.clear();
        if let Err(e) = self.decoder.decode(payload, &mut self.pcm) {
            self.errors += 1;
            if self.errors <= 3 || self.errors.is_multiple_of(500) {
                log(format!("AirPlay: trama de audio no válida ({} bytes): {e:#}", payload.len()));
            }
            return None;
        }
        if gain != 1.0 {
            for v in &mut self.pcm {
                *v *= gain;
            }
        }
        self.frames += 1;
        if self.frames == 1 {
            log("AirPlay: recibiendo audio (AAC-ELD 44,1 kHz)");
        }
        Some(&self.pcm)
    }

    fn finish(&self) {
        if self.frames > 0 {
            log(format!("AirPlay: audio terminado ({} tramas, {} perdidas, {} con error)", self.frames, self.lost, self.errors));
        }
    }
}

/// Separa la cabecera RTP. Devuelve el número de secuencia y la carga.
fn parse_rtp(packet: &mut [u8]) -> Option<(u16, &mut [u8])> {
    if packet.len() <= 12 || packet[0] >> 6 != 2 || packet[1] & 0x7f != 0x60 {
        return None;
    }
    let seq = u16::from_be_bytes([packet[2], packet[3]]);
    Some((seq, &mut packet[12..]))
}

/// AES-128-CBC sobre los bloques completos; los bytes sobrantes van en claro.
fn decrypt(cipher: &Aes128, iv: &[u8; 16], data: &mut [u8]) {
    let mut prev = *iv;
    for chunk in data.as_chunks_mut::<16>().0 {
        let encrypted = *chunk;
        let mut block = (*chunk).into();
        cipher.decrypt_block(&mut block);
        for (out, (p, v)) in chunk.iter_mut().zip(prev.iter().zip(block.iter())) {
            *out = p ^ v;
        }
        prev = encrypted;
    }
}

/// Comprueba el tipo de compresión pedido en SETUP.
pub fn check_codec(ct: Option<u64>) -> Result<()> {
    match ct {
        Some(CT_AAC_ELD) | None => Ok(()),
        Some(2) => bail!("el iPhone envía ALAC (AirPlay solo audio), que aún no está soportado"),
        Some(4) => bail!("el iPhone envía AAC-LC, que aún no está soportado"),
        Some(other) => bail!("tipo de audio {other} no soportado"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockCipherEncrypt;

    #[test]
    fn cbc_decrypts_full_blocks_only() {
        let key = [3u8; 16];
        let iv = [7u8; 16];
        let cipher = Aes128::new(&key.into());
        let plain: Vec<u8> = (0..37u8).collect();
        // Cifrado CBC de referencia de los dos bloques completos.
        let mut enc = plain.clone();
        let mut prev = iv;
        for chunk in enc.as_chunks_mut::<16>().0 {
            for (c, p) in chunk.iter_mut().zip(prev) {
                *c ^= p;
            }
            let mut b = (*chunk).into();
            cipher.encrypt_block(&mut b);
            chunk.copy_from_slice(&b);
            prev = *chunk;
        }
        assert_ne!(enc[..32], plain[..32]);
        assert_eq!(enc[32..], plain[32..]);
        decrypt(&cipher, &iv, &mut enc);
        assert_eq!(enc, plain);
    }

    #[test]
    fn rtp_header() {
        let mut p = [0x80, 0xe0, 0x12, 0x34, 0, 0, 0, 0, 0, 0, 0, 0, 0xaa, 0xbb];
        let (seq, payload) = parse_rtp(&mut p).unwrap();
        assert_eq!((seq, &payload[..]), (0x1234, &[0xaa, 0xbb][..]));
        assert!(parse_rtp(&mut [0x80, 0xd4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]).is_none(), "paquete de sincronía");
    }

    /// Hace de iPhone: empaqueta en RTP y cifra tramas AAC-ELD reales (codificadas
    /// con fdk-aac) y comprueba que salen igual que el PCM de referencia de fdk.
    #[test]
    fn decrypts_and_decodes_rtp_stream() {
        use crate::iphone::aac::tests::{fixture, max_error_lsb};
        let (asc, aus, reference) = fixture();
        assert_eq!(asc, ELD_CONFIG);
        let key = [0x11u8; 16];
        let iv = [0x22u8; 16];
        let cipher = Aes128::new(&key.into());
        let mut pipeline = Pipeline::new(&key, iv).unwrap();
        let mut decoded = Vec::new();
        let mut packets: Vec<Vec<u8>> = aus
            .iter()
            .enumerate()
            .map(|(i, au)| {
                let mut p = vec![0x80, 0x60];
                p.extend_from_slice(&(1000 + i as u16).to_be_bytes());
                p.extend_from_slice(&[0; 8]);
                let mut body = au.clone();
                let mut prev = iv;
                for chunk in body.as_chunks_mut::<16>().0 {
                    for (c, x) in chunk.iter_mut().zip(prev) {
                        *c ^= x;
                    }
                    let mut b = (*chunk).into();
                    aes::cipher::BlockCipherEncrypt::encrypt_block(&cipher, &mut b);
                    chunk.copy_from_slice(&b);
                    prev = *chunk;
                }
                p.extend_from_slice(&body);
                p
            })
            .collect();
        // Un duplicado por el camino no debe repetirse.
        packets.insert(3, packets[2].clone());
        for p in &mut packets {
            if let Some(pcm) = pipeline.push(p, 1.0) {
                decoded.extend_from_slice(pcm);
            }
        }
        assert_eq!(pipeline.frames, aus.len() as u64);
        assert!(max_error_lsb(&decoded, &reference) <= 8.0);
    }

    #[test]
    fn volume() {
        assert_eq!(gain_from_volume(0.0), 1.0);
        assert_eq!(gain_from_volume(-144.0), 0.0);
        assert!((gain_from_volume(-20.0) - 0.1).abs() < 1e-6);
    }
}
