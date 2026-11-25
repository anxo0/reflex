//! Identidad persistente del receptor: clave Ed25519 y un identificador con forma
//! de MAC. El iPhone recuerda los receptores por estos datos, así que se guardan
//! en disco para que Reflex no aparezca como un aparato nuevo cada vez.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use ed25519_dalek::SigningKey;

use crate::paths;

const FILE_LEN: usize = 32 + 6;

pub struct Identity {
    pub signing: SigningKey,
    pub device_id: [u8; 6],
}

impl Identity {
    pub fn load_or_create() -> Result<Self> {
        let path = file();
        if let Ok(bytes) = fs::read(&path)
            && bytes.len() == FILE_LEN
        {
            let seed: [u8; 32] = bytes[..32].try_into().unwrap();
            let device_id: [u8; 6] = bytes[32..].try_into().unwrap();
            return Ok(Self { signing: SigningKey::from_bytes(&seed), device_id });
        }

        let mut bytes = [0u8; FILE_LEN];
        getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("sin fuente de aleatoriedad: {e}"))?;
        // MAC administrada localmente y unicast, para no chocar con una real.
        bytes[32] = (bytes[32] & 0xfc) | 0x02;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).with_context(|| format!("no se pudo crear {}", dir.display()))?;
        }
        fs::write(&path, bytes).with_context(|| format!("no se pudo guardar {}", path.display()))?;
        let seed: [u8; 32] = bytes[..32].try_into().unwrap();
        Ok(Self { signing: SigningKey::from_bytes(&seed), device_id: bytes[32..].try_into().unwrap() })
    }

    pub fn public_key(&self) -> [u8; 32] {
        self.signing.verifying_key().to_bytes()
    }

    pub fn public_key_hex(&self) -> String {
        self.public_key().iter().map(|b| format!("{b:02x}")).collect()
    }

    /// `AA:BB:CC:DD:EE:FF`, como lo anuncia un Apple TV.
    pub fn device_id(&self) -> String {
        self.device_id.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
    }

    /// `AABBCCDDEEFF`, para el nombre del servicio RAOP.
    pub fn device_id_compact(&self) -> String {
        self.device_id.iter().map(|b| format!("{b:02X}")).collect()
    }
}

fn file() -> PathBuf {
    paths::data_dir().join("airplay-identity.bin")
}
