//! Emparejado "legacy" de AirPlay (`/pair-setup` y `/pair-verify`).
//!
//! `pair-setup` solo intercambia las claves públicas Ed25519. `pair-verify` hace un
//! ECDH con X25519 y cada parte firma las dos claves efímeras; la firma viaja
//! cifrada con AES-128-CTR. El secreto compartido se usa luego para derivar la
//! clave del flujo de vídeo.

use aes::Aes128;
use anyhow::{Context as _, Result, bail};
use ctr::cipher::{KeyIvInit, StreamCipher};
use ed25519_dalek::{Signature, Signer, Verifier, VerifyingKey};
use sha2::{Digest, Sha512};
use x25519_dalek::{PublicKey, StaticSecret};

use super::identity::Identity;

type Aes128Ctr = ctr::Ctr128BE<Aes128>;

/// Estado tras el primer paso de `pair-verify`.
pub struct Verify {
    our_public: [u8; 32],
    their_public: [u8; 32],
    their_ed: [u8; 32],
    pub shared_secret: [u8; 32],
    pub finished: bool,
}

/// Primer mensaje: `01 00 00 00 | X25519 del iPhone | Ed25519 del iPhone`.
/// Responde con nuestra clave X25519 y la firma cifrada.
pub fn verify_start(id: &Identity, body: &[u8]) -> Result<(Verify, Vec<u8>)> {
    if body.len() != 4 + 32 + 32 || body[0] != 1 {
        bail!("pair-verify: primer mensaje no válido ({} bytes)", body.len());
    }
    let their_public: [u8; 32] = body[4..36].try_into().unwrap();
    let their_ed: [u8; 32] = body[36..68].try_into().unwrap();

    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|e| anyhow::anyhow!("sin fuente de aleatoriedad: {e}"))?;
    let secret = StaticSecret::from(seed);
    let our_public = PublicKey::from(&secret).to_bytes();
    let shared_secret = secret.diffie_hellman(&PublicKey::from(their_public)).to_bytes();

    let mut msg = [0u8; 64];
    msg[..32].copy_from_slice(&our_public);
    msg[32..].copy_from_slice(&their_public);
    let mut signature = id.signing.sign(&msg).to_bytes();
    cipher(&shared_secret).apply_keystream(&mut signature);

    let mut res = Vec::with_capacity(96);
    res.extend_from_slice(&our_public);
    res.extend_from_slice(&signature);
    Ok((Verify { our_public, their_public, their_ed, shared_secret, finished: false }, res))
}

/// Segundo mensaje: `00 00 00 00 | firma del iPhone cifrada`. El flujo AES sigue
/// donde lo dejó nuestra firma, así que se descartan los primeros 64 bytes.
pub fn verify_finish(state: &mut Verify, body: &[u8]) -> Result<()> {
    if body.len() != 4 + 64 || body[0] != 0 {
        bail!("pair-verify: segundo mensaje no válido ({} bytes)", body.len());
    }
    let mut c = cipher(&state.shared_secret);
    c.apply_keystream(&mut [0u8; 64]);
    let mut signature: [u8; 64] = body[4..].try_into().unwrap();
    c.apply_keystream(&mut signature);

    let mut msg = [0u8; 64];
    msg[..32].copy_from_slice(&state.their_public);
    msg[32..].copy_from_slice(&state.our_public);
    let key = VerifyingKey::from_bytes(&state.their_ed).context("clave pública del iPhone no válida")?;
    key.verify(&msg, &Signature::from_bytes(&signature)).context("la firma del iPhone no es válida")?;
    state.finished = true;
    Ok(())
}

fn cipher(shared: &[u8; 32]) -> Aes128Ctr {
    let key = derive(b"Pair-Verify-AES-Key", shared);
    let iv = derive(b"Pair-Verify-AES-IV", shared);
    Aes128Ctr::new(&key.into(), &iv.into())
}

fn derive(salt: &[u8], shared: &[u8; 32]) -> [u8; 16] {
    let hash = Sha512::new().chain_update(salt).chain_update(shared).finalize();
    hash[..16].try_into().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    /// Simula el lado del iPhone y comprueba las dos firmas.
    #[test]
    fn full_handshake() {
        let receiver = Identity { signing: SigningKey::from_bytes(&[1; 32]), device_id: [2; 6] };
        let client_ed = SigningKey::from_bytes(&[3; 32]);
        let client_x = StaticSecret::from([4; 32]);
        let client_pub = PublicKey::from(&client_x).to_bytes();

        let mut m1 = vec![1, 0, 0, 0];
        m1.extend_from_slice(&client_pub);
        m1.extend_from_slice(&client_ed.verifying_key().to_bytes());
        let (mut state, res) = verify_start(&receiver, &m1).unwrap();

        // El iPhone deriva el mismo secreto y comprueba nuestra firma.
        let server_pub: [u8; 32] = res[..32].try_into().unwrap();
        let shared = client_x.diffie_hellman(&PublicKey::from(server_pub)).to_bytes();
        assert_eq!(shared, state.shared_secret);
        let mut c = cipher(&shared);
        let mut sig: [u8; 64] = res[32..].try_into().unwrap();
        c.apply_keystream(&mut sig);
        let mut msg = [0u8; 64];
        msg[..32].copy_from_slice(&server_pub);
        msg[32..].copy_from_slice(&client_pub);
        receiver.signing.verifying_key().verify(&msg, &Signature::from_bytes(&sig)).unwrap();

        // Y firma las claves en el orden contrario, siguiendo el mismo flujo AES.
        msg[..32].copy_from_slice(&client_pub);
        msg[32..].copy_from_slice(&server_pub);
        let mut their_sig = client_ed.sign(&msg).to_bytes();
        c.apply_keystream(&mut their_sig);
        let mut m2 = vec![0, 0, 0, 0];
        m2.extend_from_slice(&their_sig);
        verify_finish(&mut state, &m2).unwrap();
        assert!(state.finished);

        m2[10] ^= 1;
        assert!(verify_finish(&mut state, &m2).is_err());
    }
}
