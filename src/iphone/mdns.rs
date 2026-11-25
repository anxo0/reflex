//! Anuncio en la red local (Bonjour/mDNS) como receptor AirPlay.
//!
//! Se publican `_airplay._tcp` y `_raop._tcp` con los mismos registros TXT que un
//! Apple TV de 3.ª generación, que es el perfil que el iPhone sabe duplicar sin
//! emparejado HomeKit.

use anyhow::{Context as _, Result};
use mdns_sd::{IfKind, ServiceDaemon, ServiceInfo};

use super::identity::Identity;

pub const MODEL: &str = "AppleTV3,2";
pub const SOURCE_VERSION: &str = "220.68";
pub const PI: &str = "2e388006-13ba-4041-9a67-25dd4a43d536";

/// Bits de capacidades: vídeo y fotos, duplicado, audio, FairPlay y emparejado
/// legacy (bit 27). Los 32 bits altos van a cero: nada de H.265 ni PTP.
pub const FEATURES: u64 = 0x5A7F_FEE6;

pub fn features_txt() -> String {
    format!("0x{:X},0x{:X}", FEATURES & 0xffff_ffff, FEATURES >> 32)
}

pub fn airplay_txt(id: &Identity) -> Vec<(&'static str, String)> {
    vec![
        ("deviceid", id.device_id()),
        ("features", features_txt()),
        ("pw", "false".into()),
        ("flags", "0x4".into()),
        ("model", MODEL.into()),
        ("pk", id.public_key_hex()),
        ("pi", PI.into()),
        ("srcvers", SOURCE_VERSION.into()),
        ("vv", "2".into()),
    ]
}

fn raop_txt(id: &Identity) -> Vec<(&'static str, String)> {
    vec![
        ("ch", "2".into()),
        ("cn", "0,1,2,3".into()),
        ("da", "true".into()),
        ("et", "0,3,5".into()),
        ("vv", "2".into()),
        ("ft", features_txt()),
        ("am", MODEL.into()),
        ("md", "0,1,2".into()),
        ("rhd", "5.6.0.0".into()),
        ("pw", "false".into()),
        ("sf", "0x4".into()),
        ("sr", "44100".into()),
        ("ss", "16".into()),
        ("sv", "false".into()),
        ("tp", "UDP".into()),
        ("txtvers", "1".into()),
        ("vs", SOURCE_VERSION.into()),
        ("vn", "65537".into()),
        ("pk", id.public_key_hex()),
    ]
}

/// El registro TXT en formato de cable (longitud + `clave=valor`), que el iPhone
/// también pide dentro de `GET /info`.
pub fn encode_txt(entries: &[(&str, String)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (k, v) in entries {
        let item = format!("{k}={v}");
        let bytes = &item.as_bytes()[..item.len().min(255)];
        out.push(bytes.len() as u8);
        out.extend_from_slice(bytes);
    }
    out
}

pub struct Advertiser {
    daemon: ServiceDaemon,
    names: Vec<String>,
}

impl Advertiser {
    pub fn start(id: &Identity, name: &str, port: u16) -> Result<Self> {
        let daemon = ServiceDaemon::new().context("no se pudo iniciar mDNS")?;
        // El servidor solo escucha en IPv4: si anunciamos IPv6 el iPhone puede intentarlo por ahí.
        daemon.disable_interface(IfKind::IPv6).context("no se pudo configurar mDNS")?;
        let host = format!("{}.local.", host_label());
        let mut names = Vec::new();

        let services = [
            ("_airplay._tcp.local.", name.to_string(), airplay_txt(id)),
            ("_raop._tcp.local.", format!("{}@{name}", id.device_id_compact()), raop_txt(id)),
        ];
        for (ty, instance, txt) in services {
            let props: Vec<(&str, &str)> = txt.iter().map(|(k, v)| (*k, v.as_str())).collect();
            let info = ServiceInfo::new(ty, &instance, &host, "", port, &props[..])
                .with_context(|| format!("servicio {ty} no válido"))?
                .enable_addr_auto();
            names.push(info.get_fullname().to_string());
            daemon.register(info).with_context(|| format!("no se pudo anunciar {ty}"))?;
        }
        Ok(Self { daemon, names })
    }
}

impl Drop for Advertiser {
    fn drop(&mut self) {
        for n in &self.names {
            let _ = self.daemon.unregister(n);
        }
        let _ = self.daemon.shutdown();
    }
}

/// Nombre de host válido para mDNS a partir del nombre del equipo.
fn host_label() -> String {
    let raw = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_default();
    let label: String = raw
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if label.is_empty() { "reflex".into() } else { format!("{label}-reflex") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn txt_wire_format() {
        let raw = encode_txt(&[("a", "1".into()), ("pw", "false".into())]);
        assert_eq!(raw, b"\x03a=1\x08pw=false");
        assert_eq!(features_txt(), "0x5A7FFEE6,0x0");
    }
}
