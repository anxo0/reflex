//! Reloj de AirPlay: el receptor pregunta la hora al iPhone por UDP (un NTP
//! reducido) mientras dura la sesión. Reflex aún no sincroniza audio y vídeo, pero
//! el iPhone espera estas peticiones para mantener viva la conexión.

use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};

use crate::logs::log;

/// Segundos entre 1900 (época NTP) y 1970 (época Unix).
const NTP_UNIX_OFFSET: u64 = 2_208_988_800;

/// Abre el puerto local de timing y empieza a enviar peticiones a `remote`.
/// Devuelve el puerto local, que se comunica al iPhone en la respuesta a SETUP.
pub fn start(remote: SocketAddr, stop: Arc<AtomicBool>) -> Result<u16> {
    let bind = if remote.is_ipv6() { "[::]:0" } else { "0.0.0.0:0" };
    let sock = UdpSocket::bind(bind).context("no se pudo abrir el puerto de timing")?;
    sock.set_read_timeout(Some(Duration::from_millis(300)))?;
    let port = sock.local_addr()?.port();
    std::thread::spawn(move || {
        let mut buf = [0u8; 128];
        let mut warned = false;
        while !stop.load(Ordering::Relaxed) {
            if let Err(e) = sock.send_to(&request(SystemTime::now()), remote)
                && !warned
            {
                warned = true;
                log(format!("AirPlay: no se pudo enviar la petición de hora: {e}"));
            }
            let _ = sock.recv_from(&mut buf);
            for _ in 0..30 {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    });
    Ok(port)
}

/// Petición de 32 bytes: cabecera `80 d2 00 07` y nuestra hora en los bytes 24..32.
fn request(now: SystemTime) -> [u8; 32] {
    let mut req = [0u8; 32];
    req[..4].copy_from_slice(&[0x80, 0xd2, 0x00, 0x07]);
    req[24..].copy_from_slice(&ntp_timestamp(now).to_be_bytes());
    req
}

/// Marca de tiempo NTP en Q32.32 (segundos desde 1900 y fracción).
pub fn ntp_timestamp(t: SystemTime) -> u64 {
    let d = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs() + NTP_UNIX_OFFSET;
    let frac = (u64::from(d.subsec_nanos()) << 32) / 1_000_000_000;
    (secs << 32) | frac
}

/// Convierte una marca NTP Q32.32 a microsegundos.
pub fn ntp_to_micros(ts: u64) -> u64 {
    (ts >> 32) * 1_000_000 + (((ts & 0xffff_ffff) * 1_000_000) >> 32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps() {
        let t = UNIX_EPOCH + Duration::from_millis(1_500);
        let ts = ntp_timestamp(t);
        assert_eq!(ts >> 32, NTP_UNIX_OFFSET + 1);
        assert_eq!(ntp_to_micros(ts) - NTP_UNIX_OFFSET * 1_000_000, 1_500_000);
        let req = request(t);
        assert_eq!(&req[..4], &[0x80, 0xd2, 0x00, 0x07]);
        assert_eq!(u64::from_be_bytes(req[24..].try_into().unwrap()), ts);
    }
}
