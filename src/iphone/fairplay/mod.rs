//! FairPlay SAP v3 del receptor AirPlay (`/fp-setup` y descifrado de `ekey`).
//!
//! Port a Rust de playfair (EstebanKubata, GPL-3.0), la misma implementación por
//! ingeniería inversa que usan shairplay, RPiPlay y UxPlay. Es código ofuscado a
//! propósito: se ha traducido sentencia a sentencia, sin intentar entenderlo, y se
//! comprueba contra vectores sacados del C original (ver las pruebas).
//!
//! La aritmética replica la de C: los `unsigned char` se promocionan a `int`, se
//! opera módulo 2^32 y el resultado se trunca al guardarse en un byte. Por eso
//! `garble` trabaja con `Wrapping<u32>`.

mod tables;

use std::num::Wrapping;

use anyhow::{Result, bail};

use tables::*;

/// Respuesta al primer `/fp-setup` (16 bytes): un mensaje fijo según el modo.
pub fn setup_reply(req: &[u8]) -> Result<[u8; 142]> {
    if req.len() != 16 {
        bail!("fp-setup: se esperaban 16 bytes y llegaron {}", req.len());
    }
    if req[4] != 0x03 {
        bail!("fp-setup: FairPlay versión {} no soportada (solo la 3)", req[4]);
    }
    let mode = req[14] as usize;
    if mode >= 4 {
        bail!("fp-setup: modo {mode} no válido");
    }
    Ok(REPLY_MESSAGE[mode])
}

/// Respuesta al segundo `/fp-setup` (164 bytes). Devuelve también el mensaje, que
/// hace falta después para descifrar la clave del flujo.
pub fn handshake_reply(req: &[u8]) -> Result<([u8; 32], [u8; 164])> {
    if req.len() != 164 {
        bail!("fp-setup: se esperaban 164 bytes y llegaron {}", req.len());
    }
    if req[4] != 0x03 || req[12] >= 4 {
        bail!("fp-setup: mensaje de clave no válido (versión {}, modo {})", req[4], req[12]);
    }
    let mut keymsg = [0u8; 164];
    keymsg.copy_from_slice(req);
    let mut res = [0u8; 32];
    res[..12].copy_from_slice(&FP_HEADER);
    res[12..].copy_from_slice(&req[144..164]);
    Ok((res, keymsg))
}

/// Descifra la `ekey` de 72 bytes del primer SETUP y devuelve la clave AES de 16.
pub fn decrypt_key(keymsg: &[u8; 164], ekey: &[u8]) -> Result<[u8; 16]> {
    if ekey.len() != 72 {
        bail!("ekey de {} bytes (se esperaban 72)", ekey.len());
    }
    Ok(playfair_decrypt(keymsg, ekey))
}

fn playfair_decrypt(message3: &[u8; 164], cipher: &[u8]) -> [u8; 16] {
    let chunk1 = &cipher[16..32];
    let chunk2 = &cipher[56..72];
    let sap_key = generate_session_key(&DEFAULT_SAP, message3);
    let schedule = generate_key_schedule(&sap_key);
    let mut block = [0u8; 16];
    for i in 0..16 {
        block[i] = chunk2[i] ^ Z_KEY[i];
    }
    cycle(&mut block, &schedule);
    let mut key = [0u8; 16];
    for i in 0..16 {
        key[i] = block[i] ^ chunk1[i] ^ X_KEY[i] ^ Z_KEY[i];
    }
    key
}

// ---------------------------------------------------------------------------
// omg_hax.c

fn word(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]])
}

fn set_word(b: &mut [u8], i: usize, v: u32) {
    b[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
}

fn table_index(i: usize) -> &'static [u8] {
    &TABLE_S1[((31 * i) % 0x28) << 8..]
}

fn message_table_index(i: usize) -> &'static [u8] {
    &TABLE_S2[((97 * i) % 144) << 8..]
}

fn permute_table_2(i: usize) -> &'static [u8] {
    &TABLE_S4[((71 * i) % 144) << 8..]
}

fn permute_block_1(b: &mut [u8; 16]) {
    let t = &TABLE_S3;
    b[0] = t[b[0] as usize];
    b[4] = t[0x400 + b[4] as usize];
    b[8] = t[0x800 + b[8] as usize];
    b[12] = t[0xc00 + b[12] as usize];

    let tmp = b[13];
    b[13] = t[0x100 + b[9] as usize];
    b[9] = t[0xd00 + b[5] as usize];
    b[5] = t[0x900 + b[1] as usize];
    b[1] = t[0x500 + tmp as usize];

    let tmp = b[2];
    b[2] = t[0xa00 + b[10] as usize];
    b[10] = t[0x200 + tmp as usize];
    let tmp = b[6];
    b[6] = t[0xe00 + b[14] as usize];
    b[14] = t[0x600 + tmp as usize];

    let tmp = b[3];
    b[3] = t[0xf00 + b[7] as usize];
    b[7] = t[0x300 + b[11] as usize];
    b[11] = t[0x700 + b[15] as usize];
    b[15] = t[0xb00 + tmp as usize];
}

fn permute_block_2(b: &mut [u8; 16], round: usize) {
    let p = |i: usize, v: u8| permute_table_2(round * 16 + i)[v as usize];
    b[0] = p(0, b[0]);
    b[4] = p(4, b[4]);
    b[8] = p(8, b[8]);
    b[12] = p(12, b[12]);

    let tmp = b[13];
    b[13] = p(13, b[9]);
    b[9] = p(9, b[5]);
    b[5] = p(5, b[1]);
    b[1] = p(1, tmp);

    let tmp = b[2];
    b[2] = p(2, b[10]);
    b[10] = p(10, tmp);
    let tmp = b[6];
    b[6] = p(6, b[14]);
    b[14] = p(14, tmp);

    let tmp = b[3];
    b[3] = p(3, b[7]);
    b[7] = p(7, b[11]);
    b[11] = p(11, b[15]);
    b[15] = p(15, tmp);
}

/// Parecido a la expansión de clave de Rijndael, con otras S-boxes.
fn generate_key_schedule(key_material: &[u8; 16]) -> [[u32; 4]; 11] {
    let mut schedule = [[0u32; 4]; 11];
    let mut buf = [0u8; 16];
    for i in 0..16 {
        buf[i] = key_material[i] ^ T_KEY[i];
    }
    let mut ti = 0;
    for round in 0..11 {
        schedule[round][0] = word(&buf, 0);
        let (t1, t2, t3, t4) = (table_index(ti), table_index(ti + 1), table_index(ti + 2), table_index(ti + 3));
        ti += 4;
        buf[0] ^= t1[buf[0x0d] as usize] ^ INDEX_MANGLE[round];
        buf[1] ^= t2[buf[0x0e] as usize];
        buf[2] ^= t3[buf[0x0f] as usize];
        buf[3] ^= t4[buf[0x0c] as usize];
        schedule[round][1] = word(&buf, 1);
        let v = word(&buf, 1) ^ word(&buf, 0);
        set_word(&mut buf, 1, v);
        schedule[round][2] = word(&buf, 2);
        let v = word(&buf, 2) ^ word(&buf, 1);
        set_word(&mut buf, 2, v);
        schedule[round][3] = word(&buf, 3);
        let v = word(&buf, 3) ^ word(&buf, 2);
        set_word(&mut buf, 3, v);
    }
    schedule
}

/// Quizá una variante de AES.
fn cycle(block: &mut [u8; 16], schedule: &[[u32; 4]; 11]) {
    for (i, k) in schedule[10].iter().enumerate() {
        let v = word(block, i) ^ k;
        set_word(block, i, v);
    }
    permute_block_1(block);

    let s5 = |i: u8| TABLE_S5[i as usize];
    let s6 = |i: u8| TABLE_S6[i as usize];
    let s7 = |i: u8| TABLE_S7[i as usize];
    let s8 = |i: u8| TABLE_S8[i as usize];
    for round in 0..9 {
        let k = &schedule[9 - round];
        let k0 = k[0].to_le_bytes();
        let ab = s5(block[3] ^ k0[3]) ^ s6(block[2] ^ k0[2]) ^ s8(block[0] ^ k0[0]) ^ s7(block[1] ^ k0[1]);
        set_word(block, 0, ab);
        let k1 = k[1].to_le_bytes();
        let ab = s5(block[7] ^ k1[3]) ^ s6(block[6] ^ k1[2]) ^ s7(block[5] ^ k1[1]) ^ s8(block[4] ^ k1[0]);
        set_word(block, 1, ab);
        let k2 = k[2].to_le_bytes();
        let w2 = s5(block[11] ^ k2[3]) ^ s6(block[10] ^ k2[2]) ^ s7(block[9] ^ k2[1]) ^ s8(block[8] ^ k2[0]);
        set_word(block, 2, w2);
        let k3 = k[3].to_le_bytes();
        let w3 = s5(block[15] ^ k3[3]) ^ s6(block[14] ^ k3[2]) ^ s7(block[13] ^ k3[1]) ^ s8(block[12] ^ k3[0]);
        set_word(block, 3, w3);
        permute_block_2(block, 8 - round);
    }
    for (i, k) in schedule[0].iter().enumerate() {
        let v = word(block, i) ^ k;
        set_word(block, i, v);
    }
}

fn decrypt_message(msg: &[u8; 164]) -> [u8; 128] {
    let mut out = [0u8; 128];
    let mode = msg[12] as usize;
    let mt = |base: usize, i: usize, v: u8| message_table_index(base + i)[v as usize] ^ MESSAGE_KEY[mode][base + i];
    let s10 = |i: usize, v: u8| TABLE_S10[(i << 8) + v as usize];

    for i in 0..8 {
        let mut b = [0u8; 16];
        let start = if mode == 3 { 0x80 - 0x10 * i } else { 0x10 * (i + 1) };
        b.copy_from_slice(&msg[start..start + 16]);

        for j in 0..9 {
            let base = 0x80 - 0x10 * j;
            b[0x0] = mt(base, 0x0, b[0x0]);
            b[0x4] = mt(base, 0x4, b[0x4]);
            b[0x8] = mt(base, 0x8, b[0x8]);
            b[0xc] = mt(base, 0xc, b[0xc]);

            let tmp = b[0x0d];
            b[0xd] = mt(base, 0xd, b[0x9]);
            b[0x9] = mt(base, 0x9, b[0x5]);
            b[0x5] = mt(base, 0x5, b[0x1]);
            b[0x1] = mt(base, 0x1, tmp);

            let tmp = b[0x02];
            b[0x2] = mt(base, 0x2, b[0xa]);
            b[0xa] = mt(base, 0xa, tmp);
            let tmp = b[0x06];
            b[0x6] = mt(base, 0x6, b[0xe]);
            b[0xe] = mt(base, 0xe, tmp);

            let tmp = b[0x3];
            b[0x3] = mt(base, 0x3, b[0x7]);
            b[0x7] = mt(base, 0x7, b[0xb]);
            b[0xb] = mt(base, 0xb, b[0xf]);
            b[0xf] = mt(base, 0xf, tmp);

            for w in 0..4 {
                let v = TABLE_S9[b[4 * w] as usize]
                    ^ TABLE_S9[0x100 + b[4 * w + 1] as usize]
                    ^ TABLE_S9[0x200 + b[4 * w + 2] as usize]
                    ^ TABLE_S9[0x300 + b[4 * w + 3] as usize];
                set_word(&mut b, w, v);
            }
        }

        b[0x0] = s10(0x0, b[0x0]);
        b[0x4] = s10(0x4, b[0x4]);
        b[0x8] = s10(0x8, b[0x8]);
        b[0xc] = s10(0xc, b[0xc]);

        let tmp = b[0x0d];
        b[0xd] = s10(0xd, b[0x9]);
        b[0x9] = s10(0x9, b[0x5]);
        b[0x5] = s10(0x5, b[0x1]);
        b[0x1] = s10(0x1, tmp);

        let tmp = b[0x02];
        b[0x2] = s10(0x2, b[0xa]);
        b[0xa] = s10(0xa, tmp);
        let tmp = b[0x06];
        b[0x6] = s10(0x6, b[0xe]);
        b[0xe] = s10(0xe, tmp);

        let tmp = b[0x3];
        b[0x3] = s10(0x3, b[0x7]);
        b[0x7] = s10(0x7, b[0xb]);
        b[0xb] = s10(0xb, b[0xf]);
        b[0xf] = s10(0xf, tmp);

        // XOR con el bloque anterior del mensaje (al revés en el modo 3).
        let (dst, prev): (usize, &[u8]) = if mode == 3 {
            let dst = 0x70 - 0x10 * i;
            (dst, if i < 7 { &msg[dst..dst + 16] } else { &MESSAGE_IV[mode] })
        } else {
            let dst = 0x10 * i;
            (dst, if i > 0 { &msg[dst..dst + 16] } else { &MESSAGE_IV[mode] })
        };
        for k in 0..16 {
            out[dst + k] = b[k] ^ prev[k];
        }
    }
    out
}

fn generate_session_key(old_sap: &[u8], msg: &[u8; 164]) -> [u8; 16] {
    let decrypted = decrypt_message(msg);
    let mut new_sap = [0u8; 320];
    new_sap[0x000..0x011].copy_from_slice(&STATIC_SOURCE_1);
    new_sap[0x011..0x091].copy_from_slice(&decrypted);
    new_sap[0x091..0x111].copy_from_slice(&old_sap[0x80..0x100]);
    new_sap[0x111..0x140].copy_from_slice(&STATIC_SOURCE_2);

    let mut key = INITIAL_SESSION_KEY;
    for round in 0..5 {
        let base = &new_sap[round * 64..round * 64 + 64];
        let md5 = modified_md5(base, &key);
        sap_hash(base, &mut key);
        for i in 0..4 {
            let v = word(&key, i).wrapping_add(word(&md5, i));
            set_word(&mut key, i, v);
        }
    }
    for i in (0..16).step_by(4) {
        key.swap(i, i + 3);
        key.swap(i + 1, i + 2);
    }
    for b in &mut key {
        *b ^= 121;
    }
    key
}

// ---------------------------------------------------------------------------
// modified_md5.c

const MD5_SHIFT: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, //
    5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, //
    4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, //
    6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

/// floor(2^32 · |sin(i + 1)|): las constantes de MD5.
const MD5_K: [u32; 64] = [
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501, //
    0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821, //
    0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8, //
    0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, //
    0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, //
    0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, //
    0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1, //
    0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
];

fn modified_md5(block_in: &[u8], key_in: &[u8; 16]) -> [u8; 16] {
    let mut block = [0u8; 64];
    block.copy_from_slice(&block_in[..64]);
    let (mut a, mut b, mut c, mut d) = (word(key_in, 0), word(key_in, 1), word(key_in, 2), word(key_in, 3));
    for i in 0..64 {
        let j = match i {
            0..16 => i,
            16..32 => (5 * i + 1) % 16,
            32..48 => (3 * i + 5) % 16,
            _ => 7 * i % 16,
        };
        let input = u32::from_be_bytes([block[4 * j], block[4 * j + 1], block[4 * j + 2], block[4 * j + 3]]);
        let z = a.wrapping_add(input).wrapping_add(MD5_K[i]);
        let f = match i {
            0..16 => (b & c) | (!b & d),
            16..32 => (b & d) | (c & !d),
            32..48 => b ^ c ^ d,
            _ => c ^ (b | !d),
        };
        let z = z.wrapping_add(f).rotate_left(MD5_SHIFT[i]).wrapping_add(b);
        let tmp = d;
        d = c;
        c = b;
        b = z;
        a = tmp;
        if i == 31 {
            let mut swap = |x: u32, y: u32| {
                let (x, y) = (x as usize, y as usize);
                for k in 0..4 {
                    block.swap(4 * x + k, 4 * y + k);
                }
            };
            swap(a & 15, b & 15);
            swap(c & 15, d & 15);
            swap((a >> 4) & 15, (b >> 4) & 15);
            swap((a >> 8) & 15, (b >> 8) & 15);
            swap((a >> 12) & 15, (b >> 12) & 15);
        }
    }
    let mut out = [0u8; 16];
    for (i, v) in [a, b, c, d].into_iter().enumerate() {
        set_word(&mut out, i, word(key_in, i).wrapping_add(v));
    }
    out
}

// ---------------------------------------------------------------------------
// sap_hash.c

fn sap_hash(block_in: &[u8], key_out: &mut [u8; 16]) {
    let mut b0: [u8; 20] = [
        0x96, 0x5F, 0xC6, 0x53, 0xF8, 0x46, 0xCC, 0x18, 0xDF, 0xBE, 0xB2, 0xF8, 0x38, 0xD7, 0xEC, 0x22, 0x03, 0xD1, 0x20,
        0x8F,
    ];
    let mut b1 = [0u8; 210];
    let mut b2: [u8; 35] = [
        0x43, 0x54, 0x62, 0x7A, 0x18, 0xC3, 0xD6, 0xB3, 0x9A, 0x56, 0xF6, 0x1C, 0x14, 0x3F, 0x0C, 0x1D, 0x3B, 0x36, 0x83,
        0xB1, 0x39, 0x51, 0x4A, 0xAA, 0x09, 0x3E, 0xFE, 0x44, 0xAF, 0xDE, 0xC3, 0x20, 0x9D, 0x42, 0x3A,
    ];
    let mut b3 = [0u8; 132];
    let mut b4: [u8; 21] = [
        0xED, 0x25, 0xD1, 0xBB, 0xBC, 0x27, 0x9F, 0x02, 0xA2, 0xA9, 0x11, 0x00, 0x0C, 0xB3, 0x52, 0xC0, 0xBD, 0xE3, 0x1B,
        0x49, 0xC7,
    ];
    const I0_INDEX: [usize; 11] = [18, 22, 23, 0, 5, 19, 32, 31, 10, 21, 30];

    // La entrada se lee en palabras little-endian y se recorre byte a byte en big-endian.
    for i in 0..210 {
        b1[i] = block_in[4 * ((i % 64) >> 2) + 3 - (i % 4)];
    }
    // Índices "negativos" como en C: módulo sin signo de 32 bits.
    let m = |i: i32, off: i32, n: u32| ((i - off) as u32 % n) as usize;
    for i in 0..840i32 {
        let x = b1[m(i, 155, 210)];
        let y = b1[m(i, 57, 210)];
        let z = b1[m(i, 13, 210)];
        let w = b1[m(i, 0, 210)];
        b1[(i % 210) as usize] = y.rotate_left(5).wrapping_add(z.rotate_left(3) ^ w).wrapping_sub(x.rotate_left(7));
    }
    garble(&mut b0, &mut b1, &mut b2, &mut b3, &mut b4);

    *key_out = [0xE1; 16];
    for i in 0..11 {
        key_out[i] = if i == 3 { 0x3d } else { key_out[i].wrapping_add(b3[I0_INDEX[i] * 4]) };
    }
    for (i, v) in b0.iter().enumerate() {
        key_out[i % 16] ^= v;
    }
    for (i, v) in b2.iter().enumerate() {
        key_out[i % 16] ^= v;
    }
    for (i, v) in b1.iter().enumerate() {
        key_out[i % 16] ^= v;
    }
    for _ in 0..16 {
        for i in 0..16i32 {
            let x = key_out[m(i, 7, 16)];
            let y = key_out[i as usize];
            let z = key_out[m(i, 37, 16)];
            let w = key_out[m(i, 177, 16)];
            key_out[i as usize] = x.rotate_left(1) ^ y ^ z.rotate_left(6) ^ w.rotate_left(5);
        }
    }
}

// ---------------------------------------------------------------------------
// hand_garble.c

type U = Wrapping<u32>;

const fn n(x: u32) -> U {
    Wrapping(x)
}

fn sh(x: U) -> usize {
    x.0 as usize
}

/// `buf[i]` con índice constante, promocionado como en C.
macro_rules! k {
    ($b:ident[$i:expr]) => {
        Wrapping($b[$i] as u32)
    };
}

/// `buf[expr]` con índice calculado.
macro_rules! r {
    ($b:ident, $i:expr) => {
        Wrapping($b[($i).0 as usize] as u32)
    };
}

fn byte(x: U) -> u32 {
    x.0 & 0xff
}

fn rol8(x: U, c: u32) -> U {
    n((byte(x) as u8).rotate_left(c) as u32)
}

fn rol8x(x: U, c: u32) -> U {
    let i = byte(x);
    n((i << c) | (i >> (8 - c)))
}

fn weird_ror8(x: U, c: U) -> U {
    let (i, c) = (byte(x), c.0);
    if c == 0 { n(0) } else { n(((i >> c) & 0xff) | (i << (8 - c))) }
}

fn weird_rol8(x: U, c: U) -> U {
    let (i, c) = (byte(x), c.0);
    if c == 0 { n(0) } else { n(((i << c) & 0xff) | (i >> (8 - c))) }
}

fn weird_rol32(x: U, c: U) -> U {
    let (i, c) = (byte(x), c.0);
    if c == 0 { n(0) } else { n((i << c) ^ (i >> (8 - c))) }
}

#[allow(clippy::too_many_lines, clippy::many_single_char_names)]
fn garble(b0: &mut [u8; 20], b1: &mut [u8; 210], b2: &mut [u8; 35], b3: &mut [u8; 132], b4: &mut [u8; 21]) {
    let (mut a, mut b, mut c, mut d, mut e, mut f, mut g): (U, U, U, U, U, U, U);

    b2[12] = (n(0x14) + (((k!(b1[64]) & n(92)) | ((k!(b1[99]) / n(3)) & n(35))) & r!(b4, rol8x(r!(b4, k!(b1[206]) % n(21)), 4) % n(21)))).0 as u8;
    b1[4] = ((k!(b1[99]) / n(5)) * (k!(b1[99]) / n(5)) * n(2)).0 as u8;
    b2[34] = 0xb8;
    b1[153] ^= (r!(b2, k!(b1[203]) % n(35)) * r!(b2, k!(b1[203]) % n(35)) * k!(b1[190])).0 as u8;
    b0[3] = (k!(b0[3]) - (((r!(b4, k!(b1[205]) % n(21)) >> 1) & n(80)) | n(0xe6440))).0 as u8;
    b0[16] = 0x93;
    b0[13] = 0x62;
    b1[33] = (k!(b1[33]) - (r!(b4, k!(b1[36]) % n(21)) & n(0xf6))).0 as u8;

    let tmp2 = r!(b2, k!(b1[67]) % n(35));
    b2[12] = 0x07;

    let tmp = r!(b0, k!(b1[181]) % n(20));
    b1[2] = (k!(b1[2]) - n(3136)).0 as u8;
    b0[19] = r!(b4, k!(b1[58]) % n(21)).0 as u8;
    b3[0] = (n(92) - r!(b2, k!(b1[32]) % n(35))).0 as u8;
    b3[4] = (r!(b2, k!(b1[15]) % n(35)) + n(0x9e)).0 as u8;
    b1[34] = (k!(b1[34]) + (r!(b4, ((r!(b2, k!(b1[15]) % n(35)) + n(0x9e)) & n(0xff)) % n(21)) / n(5))).0 as u8;
    b0[19] = (k!(b0[19]) + (n(0xfffffee6) - ((r!(b0, k!(b3[4]) % n(20)) >> 1) & n(102)))).0 as u8;

    {
        let b4a = r!(b4, k!(b1[190]) % n(21));
        let x = k!(b1[72]) >> sh(b4a & n(7));
        let y = k!(b1[72]) << sh((n(7) - (b4a - n(1))) & n(7));
        b1[15] = ((n(3) * ((x ^ y) - n(3) * r!(b4, k!(b1[126]) % n(21)))) ^ k!(b1[15])).0 as u8;
    }
    {
        let v = r!(b2, k!(b1[181]) % n(35));
        b0[15] ^= (v * v * v).0 as u8;
    }
    b2[4] ^= (k!(b1[202]) / n(3)).0 as u8;

    a = n(92) - r!(b0, k!(b3[0]) % n(20));
    e = (a & n(0xc6)) | (!k!(b1[105]) & n(0xc6)) | (a & !k!(b1[105]));
    b2[1] = (k!(b2[1]) + e * e * e).0 as u8;

    b0[19] ^= (((n(224) | (r!(b4, k!(b1[92]) % n(21)) & n(27))) * r!(b2, k!(b1[41]) % n(35))) / n(3)).0 as u8;
    b1[140] = (k!(b1[140]) + weird_ror8(n(92), k!(b1[5]) & n(7))).0 as u8;
    {
        let v = !k!(b1[4]) ^ r!(b2, k!(b1[12]) % n(35));
        b2[12] = (k!(b2[12]) + (((v | k!(b1[182])) & n(192)) | (v & k!(b1[182])))).0 as u8;
    }
    b1[36] = b1[36].wrapping_add(125);
    {
        let p = (n(74) & k!(b1[138])) | ((n(74) | k!(b1[138])) & k!(b0[15]));
        let q = r!(b0, k!(b1[43]) % n(20));
        b1[124] = rol8x((p & q) | ((p | q) & n(95)), 4).0 as u8;
    }
    b3[8] = ((((r!(b0, k!(b3[4]) % n(20)) & n(95)) & ((r!(b4, k!(b1[68]) % n(21)) & n(46)) << 1)) | n(16)) ^ n(92)).0 as u8;

    a = k!(b1[177]) + r!(b4, k!(b1[79]) % n(21));
    {
        let t = (n(3) * k!(b1[148])) / n(5);
        d = (((a >> 1) | t) & k!(b2[1])) | ((a >> 1) & t);
    }
    b3[12] = (n(0) - n(34) - d).0 as u8;

    a = n(8) - (k!(b2[22]) & n(7));
    b = k!(b1[33]) >> sh(a & n(7));
    c = k!(b1[33]) << sh(k!(b2[22]) & n(7));
    b2[16] = (k!(b2[16]) + (((r!(b2, k!(b3[0]) % n(35)) & n(159)) | r!(b0, k!(b3[4]) % n(20)) | n(8)) - ((b ^ c) | n(128)))).0 as u8;

    b0[14] ^= b2[(b3[12] % 35) as usize];

    a = weird_rol8(r!(b4, r!(b0, k!(b1[201]) % n(20)) % n(21)), (r!(b2, k!(b1[112]) % n(35)) << 1) & n(7));
    d = (r!(b0, k!(b1[208]) % n(20)) & n(131)) | (r!(b0, k!(b1[164]) % n(20)) & n(124));
    b1[19] = (k!(b1[19]) + ((a & (d / n(5))) | ((a | (d / n(5))) & n(37)))).0 as u8;

    {
        let v = r!(b4, k!(b1[45]) % n(21)) + n(92);
        b2[8] = weird_ror8(n(140), (v * v) & n(7)).0 as u8;
    }
    b1[190] = 56;
    b2[8] ^= b3[0];
    b1[53] = (!((r!(b0, k!(b1[83]) % n(20)) | n(204)) / n(5))).0 as u8;
    b0[13] = (k!(b0[13]) + r!(b0, k!(b1[41]) % n(20))).0 as u8;
    {
        let v = r!(b2, k!(b3[0]) % n(35));
        b0[10] = (((v & k!(b1[2])) | ((v | k!(b1[2])) & k!(b3[12]))) / n(15)).0 as u8;
    }
    {
        let p = r!(b4, k!(b1[2]) % n(21)) & n(68);
        let q = r!(b2, k!(b3[8]) % n(35));
        a = (((n(56) | p) | q) & n(42)) | ((p | n(56)) & q);
    }
    b3[16] = (a * a + n(110)).0 as u8;
    b3[20] = (n(202) - k!(b3[16])).0 as u8;
    b3[24] = b1[151];
    b2[13] ^= b4[(b3[0] % 21) as usize];

    {
        let v = r!(b2, k!(b1[179]) % n(35)) - n(38);
        b = (v & n(177)) | (k!(b3[12]) & n(177));
        c = v & k!(b3[12]);
    }
    b3[28] = (n(30) + (b | c) * (b | c)).0 as u8;
    b3[32] = (k!(b3[28]) + n(62)).0 as u8;

    let tmp3;
    {
        let s = k!(b3[20]) + (k!(b3[0]) & n(74));
        let nb = !r!(b4, k!(b3[0]) % n(21));
        a = (s | nb) & n(121);
        b = s & nb;
        tmp3 = a | b;
        let v = (a | b) ^ n(0xffffffa6);
        c = ((v | k!(b3[0])) & n(4)) | (v & k!(b3[0]));
    }
    b1[47] = ((r!(b2, k!(b1[89]) % n(35)) + c) ^ k!(b1[47])).0 as u8;

    b3[36] = (((rol8((tmp & n(179)) + n(68), 2) & k!(b0[3])) | (tmp2 & !k!(b0[3]))) - n(15)).0 as u8;
    b1[123] ^= 221;

    a = (r!(b4, k!(b3[0]) % n(21)) / n(3)) - r!(b2, k!(b3[4]) % n(35));
    c = (((k!(b3[0]) & n(163)) + n(92)) & n(246)) | (k!(b3[0]) & n(92));
    e = ((c | k!(b3[24])) & n(54)) | (c & k!(b3[24]));
    b3[40] = (a - e).0 as u8;

    b3[44] = (tmp3 ^ n(81) ^ (((k!(b3[0]) >> 1) & n(101)) + n(26))).0 as u8;
    b3[48] = (r!(b2, k!(b3[4]) % n(35)) & n(27)).0 as u8;
    b3[52] = 27;
    b3[56] = 199;

    {
        let (x40, x24) = (k!(b3[40]), k!(b3[24]));
        let b4_20 = r!(b4, k!(b3[0]) % n(20));
        let b4_21 = r!(b4, k!(b3[0]) % n(21));
        let p1 = ((x40 | x24) & n(177)) | (x40 & x24);
        let q1 = ((b4_20 & n(177)) | n(176)) | (b4_21 & !n(3));
        let p2 = ((x40 & x24) | ((x40 | x24) & n(177))) & n(199);
        let q2 = (((b4_21 & n(1)) + n(176)) | (b4_21 & !n(3))) & k!(b3[56]);
        let inner = ((p1 & q1) | (p2 | q2)) & !k!(b3[52]);
        b3[64] = (k!(b3[4]) + (inner | k!(b3[48]))).0 as u8;
    }

    b2[33] ^= b1[26];
    b1[106] ^= (k!(b3[20]) ^ n(133)).0 as u8;
    b2[30] = (((k!(b3[64]) / n(3)) - (n(275) | (k!(b3[0]) & n(247)))) ^ r!(b0, k!(b1[122]) % n(20))).0 as u8;
    b1[22] = ((r!(b2, k!(b1[90]) % n(35)) & n(95)) | n(68)).0 as u8;

    a = (r!(b4, k!(b3[36]) % n(21)) & n(184)) | (r!(b2, k!(b3[44]) % n(35)) & !n(184));
    b2[18] = (k!(b2[18]) + ((a * a * a) >> 1)).0 as u8;
    b2[5] = (k!(b2[5]) - r!(b4, k!(b1[92]) % n(21))).0 as u8;

    a = (((k!(b1[41]) & !n(24)) | (r!(b2, k!(b1[183]) % n(35)) & n(24))) & (k!(b3[16]) + n(53))) | (k!(b3[20]) & r!(b2, k!(b3[20]) % n(35)));
    b = (k!(b1[17]) & !k!(b3[44])) | (r!(b0, k!(b1[59]) % n(20)) & k!(b3[44]));
    b2[18] ^= (a * b).0 as u8;

    a = weird_ror8(k!(b1[11]), r!(b2, k!(b1[28]) % n(35)) & n(7)) & n(7);
    b = (((r!(b0, k!(b1[93]) % n(20)) & !k!(b0[14])) | (k!(b0[14]) & n(150))) & !n(28)) | (k!(b1[7]) & n(28));
    {
        let wr = weird_rol8(r!(b2, k!(b3[0]) % n(35)), a);
        b2[22] = (((((b | wr) & k!(b2[33])) | (b & wr)) + n(74)) & n(0xff)).0 as u8;
    }

    a = r!(b4, (r!(b0, k!(b1[39]) % n(20)) ^ n(217)) % n(21));
    {
        let p = ((k!(b3[20]) | k!(b3[0])) & n(214)) | (k!(b3[20]) & k!(b3[0]));
        b0[15] = (k!(b0[15]) - ((p & a) | ((p | a) & k!(b3[32])))).0 as u8;
    }

    let t;
    {
        let b2x = r!(b2, k!(b1[57]) % n(35));
        let b0x = r!(b0, k!(b3[64]) % n(20));
        b = ((b2x & b0x) | ((b0x | b2x) & n(95)) | (k!(b3[64]) & n(45)) | n(82)) & n(32);
        c = ((b2x & b0x) | ((b2x | b0x) & n(95))) & ((k!(b3[64]) & n(45)) | n(82));
        d = ((k!(b3[0]) / n(3)) - (k!(b3[64]) | k!(b1[22]))) ^ (k!(b3[28]) + n(62)) ^ (b | c);
        t = r!(b0, (d & n(0xff)) % n(20));
    }

    {
        let v = r!(b0, k!(b1[99]) % n(20));
        b3[68] = ((v * v * v * v) | r!(b2, k!(b3[64]) % n(35))).0 as u8;
    }

    let u_ = r!(b0, k!(b1[50]) % n(20));
    let w_ = r!(b2, k!(b1[138]) % n(35));
    let x_ = r!(b4, k!(b1[39]) % n(21));
    let y_ = r!(b0, k!(b1[4]) % n(20));
    let z_ = r!(b4, k!(b1[202]) % n(21));
    let v_ = r!(b0, k!(b1[151]) % n(20));
    let s_ = r!(b2, k!(b1[14]) % n(35));
    let r_ = r!(b0, k!(b1[145]) % n(20));

    {
        let p = r!(b2, k!(b3[68]) % n(35));
        let q = r!(b0, k!(b1[209]) % n(20));
        a = (p & q) | ((p | q) & n(24));
        b = weird_rol8(r!(b4, k!(b1[127]) % n(21)), p & n(7));
        c = (a & k!(b0[10])) | (b & !k!(b0[10]));
        d = n(7) ^ (r!(b4, r!(b2, k!(b3[36]) % n(35)) % n(21)) << 1);
        b3[72] = ((c & n(71)) | (d & !n(71))).0 as u8;
    }

    b2[2] = (k!(b2[2])
        + ((((r!(b0, k!(b3[20]) % n(20)) << 1) & n(159)) | (r!(b4, k!(b1[190]) % n(21)) & !n(159)))
            & ((((r!(b4, k!(b3[64]) % n(21)) & n(110)) | (r!(b0, k!(b1[25]) % n(20)) & !n(110))) & !n(150)) | (k!(b1[25]) & n(150)))))
    .0 as u8;
    b2[14] = (k!(b2[14]) - (((r!(b2, k!(b3[20]) % n(35)) & (k!(b3[72]) ^ r!(b2, k!(b1[100]) % n(35)))) & !n(34)) | (k!(b1[97]) & n(34)))).0 as u8;
    b0[17] = 115;
    {
        let b4p = r!(b4, k!(b1[17]) % n(21));
        let b0p = r!(b0, k!(b3[20]) % n(20));
        let p = ((b4p | b0p) & k!(b3[72])) | (b4p & b0p);
        let q = k!(b1[50]) / n(3);
        b1[23] ^= (((p & q) | ((p | q) & n(246))) << 1).0 as u8;
    }
    {
        let p = r!(b0, k!(b3[40]) % n(20));
        b0[13] = ((((((p | k!(b1[10])) & n(82)) | (p & k!(b1[10]))) & n(209)) | ((r!(b0, k!(b1[39]) % n(20)) << 1) & n(46))) >> 1).0 as u8;
    }
    b2[33] = (k!(b2[33]) - (k!(b1[113]) & n(9))).0 as u8;
    b2[28] = (k!(b2[28]) - ((((n(2) | (k!(b1[110]) & n(222))) >> 1) & !n(223)) | (k!(b3[20]) & n(223)))).0 as u8;

    let j = weird_rol8(v_ | z_, u_ & n(7));
    a = (k!(b2[16]) & t) | (w_ & !k!(b2[16]));
    b = (k!(b1[33]) & n(17)) | (x_ & !n(17));
    e = ((y_ | ((a + b) / n(5))) & n(147)) | (y_ & ((a + b) / n(5)));
    let m;
    {
        let q = r!(b4, ((k!(b3[8]) + j + e) & n(0xff)) % n(21));
        m = (k!(b3[40]) & q) | ((k!(b3[40]) | q) & k!(b2[23]));
    }
    {
        let v = r!(b4, k!(b3[20]) % n(21)) - n(48);
        b0[15] = (((v & !k!(b1[184])) | (v & n(189)) | (n(189) & !k!(b1[184]))) & (m * m * m)).0 as u8;
    }
    b2[22] = b2[22].wrapping_add(b1[183]);
    b3[76] = ((n(3) * r!(b4, k!(b1[1]) % n(21))) ^ k!(b3[0])).0 as u8;

    a = r!(b2, ((k!(b3[8]) + (j + e)) & n(0xff)) % n(35));
    {
        let p = r!(b4, k!(b1[178]) % n(21));
        f = (((p & a) | ((p | a) & n(209))) * r!(b0, k!(b1[13]) % n(20))) * (r!(b4, k!(b1[26]) % n(21)) >> 1);
    }
    g = (f + n(0x733ffff9)) * n(198) - (((f + n(0x733ffff9)) * n(396) + n(212)) & n(212)) + n(85);
    b3[80] = (k!(b3[36]) + (g ^ n(148)) + ((g ^ n(107)) << 1) - n(127)).0 as u8;
    b3[84] = ((r!(b2, k!(b3[64]) % n(35)) & n(245)) | (r!(b2, k!(b3[20]) % n(35)) & n(10))).0 as u8;

    a = r!(b0, k!(b3[68]) % n(20)) | n(81);
    b2[18] = (k!(b2[18]) - (((a * a * a) & !k!(b0[15])) | ((k!(b3[80]) / n(15)) & k!(b0[15])))).0 as u8;
    b3[88] = (k!(b3[8]) + j + e - r!(b0, k!(b1[160]) % n(20)) + (r!(b4, r!(b0, ((k!(b3[8]) + j + e) & n(255)) % n(20)) % n(21)) / n(3))).0 as u8;

    b = ((r_ ^ k!(b3[72])) & !n(198)) | ((s_ * s_) & n(198));
    {
        let p = r!(b4, k!(b1[69]) % n(21));
        f = (p & k!(b1[172])) | ((p | k!(b1[172])) & ((k!(b3[12]) - b) + n(77)));
    }
    b0[16] = (n(147) - ((k!(b3[72]) & ((f & n(251)) | n(1))) | (((f & n(250)) | k!(b3[72])) & n(198)))).0 as u8;

    {
        let p = r!(b4, k!(b1[168]) % n(21));
        let q = r!(b0, k!(b1[29]) % n(20));
        c = (p & q & n(7)) | ((p | q) & n(6));
    }
    {
        let p = r!(b4, k!(b1[155]) % n(21));
        f = (p & k!(b1[105])) | ((p | k!(b1[105])) & n(141));
    }
    b0[3] = (k!(b0[3]) - r!(b4, weird_rol32(f, c) % n(21))).0 as u8;

    b1[5] = (weird_ror8(k!(b0[12]), (r!(b0, k!(b1[61]) % n(20)) / n(5)) & n(7)) ^ ((!r!(b2, k!(b3[84]) % n(35)) & n(0xffffffff)) / n(5))).0 as u8;
    b1[198] = b1[198].wrapping_add(b1[3]);

    a = n(162) | r!(b2, k!(b3[64]) % n(35));
    b1[164] = (k!(b1[164]) + (a * a) / n(5)).0 as u8;

    g = weird_ror8(n(139), k!(b3[80]) & n(7));
    {
        let p = r!(b4, k!(b3[64]) % n(21));
        c = ((p * p * p) & n(95)) | (r!(b0, k!(b3[40]) % n(20)) & !n(95));
    }
    {
        let q = r!(b0, k!(b3[20]) % n(20));
        b3[92] = ((g & n(12)) | (q & n(12)) | (g & q) | c).0 as u8;
    }

    b2[12] = (k!(b2[12]) + ((k!(b1[103]) & n(32)) | (k!(b3[92]) & (k!(b1[103]) | n(60))) | n(16)) / n(3)).0 as u8;
    b3[96] = b1[143];
    b3[100] = 27;
    {
        let p = ((k!(b3[40]) & !k!(b2[8])) | (k!(b1[35]) & k!(b2[8]))) & k!(b3[64]);
        b3[104] = (p ^ n(119)).0 as u8;
        b3[108] = (n(238) & (p << 1)).0 as u8;
        let q = !k!(b3[64]) & (k!(b3[84]) / n(3));
        b3[112] = (q ^ n(49)).0 as u8;
        b3[116] = (n(98) & (q << 1)).0 as u8;
    }

    a = (k!(b1[35]) & k!(b2[8])) | (k!(b3[40]) & !k!(b2[8]));
    b = (a & k!(b3[64])) | ((k!(b3[84]) / n(3)) & !k!(b3[64]));
    {
        let left = b & (n(86) + ((k!(b1[172]) & n(64)) >> 1));
        let q1 = (!k!(b3[64]) & (k!(b3[84]) / n(3))) | (((k!(b3[40]) & !k!(b2[8])) | (k!(b1[35]) & k!(b2[8]))) & k!(b3[64]));
        let right = ((((k!(b1[172]) & n(65)) >> 1) ^ n(86)) | q1) & k!(b3[100]);
        b1[143] = (k!(b3[96]) - (left | right)).0 as u8;
    }
    b2[29] = 162;

    a = ((r!(b4, k!(b3[88]) % n(21)) & n(160)) | (r!(b0, k!(b1[125]) % n(20)) & n(95))) >> 1;
    b = r!(b2, k!(b1[149]) % n(35)) ^ (k!(b1[43]) * k!(b1[43]));
    b0[15] = (k!(b0[15]) + ((b & a) | ((a | b) & n(115)))).0 as u8;
    b3[120] = (k!(b3[64]) - r!(b0, k!(b3[40]) % n(20))).0 as u8;
    b1[95] = b4[(b3[20] % 21) as usize];

    {
        let p = r!(b2, k!(b1[17]) % n(35));
        a = weird_ror8(r!(b2, k!(b3[80]) % n(35)), (p * p * p) & n(7));
    }
    b0[7] = (k!(b0[7]) - a * a).0 as u8;
    {
        let p = r!(b4, k!(b1[202]) % n(21));
        b2[8] = (k!(b2[8]) - k!(b1[184]) + p * p * p).0 as u8;
    }
    b0[16] = ((r!(b2, k!(b1[102]) % n(35)) << 1) & n(132)).0 as u8;
    b3[124] = ((r!(b4, k!(b3[40]) % n(21)) >> 1) ^ k!(b3[68])).0 as u8;
    b0[7] = (k!(b0[7])
        - (r!(b0, k!(b1[191]) % n(20)) - (((r!(b4, k!(b1[80]) % n(21)) << 1) & !n(177)) | (r!(b4, r!(b4, k!(b3[88]) % n(21)) % n(21)) & n(177)))))
    .0 as u8;
    b0[6] = b0[(b1[119] % 20) as usize];

    a = (r!(b4, k!(b1[190]) % n(21)) & !n(209)) | (k!(b1[118]) & n(209));
    {
        let p = r!(b0, k!(b3[120]) % n(20));
        b = p * p;
    }
    b0[12] = ((r!(b0, k!(b3[84]) % n(20)) ^ (r!(b2, k!(b1[71]) % n(35)) + r!(b2, k!(b1[15]) % n(35)))) & ((a & b) | ((a | b) & n(27)))).0 as u8;

    {
        let p = r!(b2, k!(b3[88]) % n(35));
        b = (k!(b1[32]) & p) | ((k!(b1[32]) | p) & n(23));
    }
    d = ((r!(b4, k!(b1[57]) % n(21)) * n(231)) & n(169)) | (b & n(86));
    f = (((r!(b0, k!(b1[82]) % n(20)) & !n(29)) | (r!(b4, k!(b3[124]) % n(21)) & n(29))) & n(190)) | (r!(b4, (d / n(5)) % n(21)) & !n(190));
    let h = {
        let p = r!(b0, k!(b3[40]) % n(20));
        p * p * p
    };
    let kk = (h & k!(b1[82])) | (h & n(92)) | (k!(b1[82]) & n(92));
    b3[128] = (((f & kk) | ((f | kk) & n(192))) ^ (d / n(5))).0 as u8;

    b2[25] ^= (((r!(b0, k!(b3[120]) % n(20)) << 1) * k!(b1[5])) - (weird_rol8(k!(b3[76]), r!(b4, k!(b3[124]) % n(21)) & n(7)) & (k!(b3[20]) + n(110)))).0 as u8;
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn unhex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// (mensaje de clave, ekey, clave AES) generados con el playfair original en C, uno por modo.
    pub(crate) const VECTORS: [(&str, &str, &str); 4] = [
        (
            "6054930a03609ca8980aeadf00f2eec83665587ba8550e731be6e9bd3c57eb953d74cbd9a8570358b90acf065b15c8dedb852764c1dc851907eaf8cad3535632a655cb9cda48e9d93ce3a905fe088e029bd80e0c4020ed4a03667f3f6dd3f5a335bcdac75cb9a6dbf9ade23e138e717bb11cd7e272105ddc1273ea6906a1e7b3cca69b0ed4c7828e6a4826b5005d27df3ee4de98b3c035f98e1d393d5c9421e9306a2d1a",
            "e0832f92986ba8e51ca08f1a89d8162429a8b33e810c171604ebac4d6d5b9630d230dc7167c422e61a05eb6692c2274ebdf2bfdae85f12bfabf08b5309191a7e42d7a8c0a80a990f",
            "ba04550a29d222087c5136e06c4c4365",
        ),
        (
            "3304cb6103d316580161d4a1011115010139d56c931c88d768f0c0e703f0b02292bab39caba068a92c075736ba185f21d48467d97ffa393bf02b7efde1f7f7c7caf7154bfc07f929ddb65e318d1934f1f0cb547a69160b119e2acb4ee57b8af84d6ad37e4240d07e6d5323ea0894a78eb614815059287547fd1ca279762ad518132a1445f75f580340448a8c6b74bbdd37049151a164816f60a90d51c08656b373e69642",
            "192fd7e1c2d3091f30415d509b3b8b86fe98ffd4926c324bd109ca31db923651db97a1a456066e37a57e58223a0aa1a166400a5705d724fe400fa681d2d2727593493980c03185ca",
            "e9d7e1d44fb1524f532378993f3443ef",
        ),
        (
            "28b3a10003cd49fb078cd42402ef5a1ab55f6b0ebd008c2fcbda3d5bff6bc47b189769c9fe612c84f7d53b9e988dcb8b602c6b555160a6336022236347512d5dd9ff7399df28710b977e6c305a7e69fa2a772265c1c8f69c317a2658da936bd77bbb2c161485e9e47598e82739c7226f76c28b22f19665d8e828a0fb60af241bc4e54c7c8c1b1cecc470fd07e29f68cd2df7d2a49f9d59b01093a2b344c7b32c0269de40",
            "767b64a2eec121925160ad512263d6c0cc15557974a87a0024fb699c69b48fc20317f5e2575058768e006f50b9dba4ee294c3639aad73fc185b1d536b70ca7008061b9edd367a91f",
            "6fcef2475a2970ca79b835fa6d9c0936",
        ),
        (
            "cd7191ff0386f447550757fe03476dbc4b3da7ea4a466befe113d2a581182c082d4fa17c7f19b00db0c86a8fa308d64e32b655af8c24a5f548be02318f2f2a4f215bc488208bda5b045263890e903ac261c9bc4a6b49dea1a7d808ea8d358dbd14e71a3db5e0e5e76e03e014aec02ac5c5d77019e42b3e008971cbcebd8c84d601fbd5031933905a93cee357dab287515e9c6e834fed742f3b0fcdb8a7bc98bf3b8cbde9",
            "64f64c3b801e0ce3edd0ff674adf490d3e9fbc9ee79fa786b3d188a8c0e681de74a1d57f2f7930b7f767ddcd3c01b2b322a13580ac27a6fbf4f9fa344fee5816075b6d669c82062c",
            "2afdea39f86012f767669c448810967d",
        ),
    ];

    #[test]
    fn decrypts_like_the_c_reference() {
        for (msg, ekey, expected) in VECTORS {
            let msg: [u8; 164] = unhex(msg).try_into().unwrap();
            assert_eq!(decrypt_key(&msg, &unhex(ekey)).unwrap().to_vec(), unhex(expected));
        }
    }

    #[test]
    fn handshake_replies() {
        let mut req = [0u8; 16];
        req[4] = 3;
        req[14] = 2;
        assert_eq!(setup_reply(&req).unwrap(), REPLY_MESSAGE[2]);
        req[4] = 2;
        assert!(setup_reply(&req).is_err(), "FairPlay v2 no está implementado");

        let mut msg = [7u8; 164];
        msg[4] = 3;
        msg[12] = 1;
        let (res, keymsg) = handshake_reply(&msg).unwrap();
        assert_eq!(&res[..12], &FP_HEADER);
        assert_eq!(&res[12..], &msg[144..]);
        assert_eq!(keymsg, msg);
    }

    /// Comparación función a función con el C de referencia (playfair de UxPlay
    /// compilado aparte): `REFLEX_FAIRPLAY_VECTORS=<fichero> cargo test -- --ignored`.
    /// Cada línea del fichero es `<función> <entradas hex separadas por comas> <salidas>`.
    #[test]
    #[ignore = "necesita un fichero de vectores generado con el código C de referencia"]
    fn matches_c_reference_vectors() {
        let path = std::env::var("REFLEX_FAIRPLAY_VECTORS").expect("falta REFLEX_FAIRPLAY_VECTORS");
        let text = std::fs::read_to_string(path).unwrap();
        let mut fails = std::collections::BTreeMap::<&str, usize>::new();
        let arr16 = |v: &Vec<u8>| -> [u8; 16] { v.clone().try_into().unwrap() };
        let arr164 = |v: &Vec<u8>| -> [u8; 164] { v.clone().try_into().unwrap() };
        for line in text.lines() {
            let mut it = line.split(' ');
            let (kind, input, output) = (it.next().unwrap(), it.next().unwrap(), it.next().unwrap());
            let ins: Vec<Vec<u8>> = input.split(',').map(unhex).collect();
            let outs: Vec<Vec<u8>> = output.split(',').map(unhex).collect();
            let got: Vec<Vec<u8>> = match kind {
                "garble" => {
                    let mut b0: [u8; 20] = ins[0].clone().try_into().unwrap();
                    let mut b1: [u8; 210] = ins[1].clone().try_into().unwrap();
                    let mut b2: [u8; 35] = ins[2].clone().try_into().unwrap();
                    let mut b3 = [0u8; 132];
                    let mut b4: [u8; 21] = ins[3].clone().try_into().unwrap();
                    garble(&mut b0, &mut b1, &mut b2, &mut b3, &mut b4);
                    vec![b0.to_vec(), b1.to_vec(), b2.to_vec(), b3.to_vec(), b4.to_vec()]
                }
                "sap_hash" => {
                    let mut k = [0u8; 16];
                    sap_hash(&ins[0], &mut k);
                    vec![k.to_vec()]
                }
                "md5" => vec![modified_md5(&ins[0], &arr16(&ins[1])).to_vec()],
                "schedule" => vec![generate_key_schedule(&arr16(&ins[0])).iter().flatten().flat_map(|w| w.to_le_bytes()).collect()],
                "cycle" => {
                    let mut b = arr16(&ins[1]);
                    cycle(&mut b, &generate_key_schedule(&arr16(&ins[0])));
                    vec![b.to_vec()]
                }
                "message" => vec![decrypt_message(&arr164(&ins[0])).to_vec()],
                "session" => vec![generate_session_key(&DEFAULT_SAP, &arr164(&ins[0])).to_vec()],
                "decrypt" => vec![playfair_decrypt(&arr164(&ins[0]), &ins[1]).to_vec()],
                other => panic!("tipo desconocido {other}"),
            };
            if got != outs {
                *fails.entry(kind).or_default() += 1;
            }
        }
        assert!(fails.is_empty(), "fallos por función: {fails:?}");
    }
}
