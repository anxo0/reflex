//! Decodificador AAC-ELD (ER AAC Enhanced Low Delay, ISO/IEC 14496-3).
//!
//! Es el formato del audio que el iPhone manda durante el duplicado de pantalla:
//! 44,1 kHz, estéreo, tramas de 480 muestras, sin SBR. Este módulo implementa solo
//! lo que hace falta para ese perfil (ELD sin SBR ni herramientas de resiliencia,
//! tramas de 480, 44,1 o 48 kHz, mono o estéreo), que es justo lo que AirPlay usa.
//!
//! Es un port a Rust del decodificador AAC de FFmpeg (LGPL-2.1+): la estructura de
//! la trama, la decuantización y el banco de filtros de bajo retardo siguen su
//! implementación en coma flotante. Se comprueba contra fdk-aac (ver las pruebas).

mod tables;

use anyhow::{Result, bail};

use tables::*;

/// Muestras por trama y canal.
pub const FRAME_LEN: usize = 480;

const N: usize = FRAME_LEN;
const MAX_SFB: usize = 64;
const TNS_MAX_ORDER: usize = 12;

const ZERO_BT: u8 = 0;
const ESC_BT: u8 = 11;
const NOISE_BT: u8 = 13;
const INTENSITY_BT2: u8 = 14;
const INTENSITY_BT: u8 = 15;

/// Configuración leída del AudioSpecificConfig.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Config {
    pub sample_rate: u32,
    pub channels: usize,
}

pub struct Decoder {
    config: Config,
    swb_offset: &'static [u16],
    num_swb: usize,
    tns_max_bands: usize,
    scalefactor_huffman: Huffman,
    spectral_huffman: Vec<Huffman>,
    /// Matriz del IMDCT de media longitud (480 × 480), como el de FFmpeg.
    imdct: Vec<f32>,
    pow2sf: Vec<f32>,
    pow43: Vec<f32>,
    random_state: u32,
    ch: [Channel; 2],
    ms_mask: [bool; MAX_SFB],
}

#[derive(Clone)]
struct Channel {
    max_sfb: usize,
    band_type: [u8; MAX_SFB],
    sfo: [i32; MAX_SFB],
    sf: [f32; MAX_SFB],
    coeffs: [f32; N],
    tns: Tns,
    /// Últimas tres salidas del IMDCT, para el solape del banco de bajo retardo.
    saved: [f32; 3 * N],
    output: [f32; N],
}

#[derive(Clone, Default)]
struct Tns {
    present: bool,
    n_filt: usize,
    length: [usize; 4],
    order: [usize; 4],
    direction: [bool; 4],
    coef: [[f32; TNS_MAX_ORDER]; 4],
}

impl Default for Channel {
    fn default() -> Self {
        Self {
            max_sfb: 0,
            band_type: [0; MAX_SFB],
            sfo: [0; MAX_SFB],
            sf: [0.0; MAX_SFB],
            coeffs: [0.0; N],
            tns: Tns::default(),
            saved: [0.0; 3 * N],
            output: [0.0; N],
        }
    }
}

impl Decoder {
    /// Crea el decodificador a partir del AudioSpecificConfig (p. ej. `f8 e8 50 00`).
    pub fn new(asc: &[u8]) -> Result<Self> {
        let config = parse_config(asc)?;
        let mut imdct = vec![0f32; N * N];
        // av_tx "mdct_naive_inv" con longitud 480 y escala 1/(480·32768).
        let scale = 1.0 / (N as f64 * 32768.0);
        let half = N / 2;
        let phase = std::f64::consts::PI / (4.0 * N as f64);
        for i in 0..half {
            let i_d = phase * (2 * N - 2 * i - 1) as f64;
            let i_u = phase * (3 * N + 2 * i + 1) as f64;
            for j in 0..N {
                let a = (2 * j + 1) as f64;
                imdct[i * N + j] = ((a * i_d).cos() * scale) as f32;
                imdct[(i + half) * N + j] = (-(a * i_u).cos() * scale) as f32;
            }
        }
        Ok(Self {
            config,
            swb_offset: &SWB_OFFSET_480_48,
            num_swb: SWB_OFFSET_480_48.len() - 1,
            tns_max_bands: if config.sample_rate == 48000 { 31 } else { 32 },
            scalefactor_huffman: Huffman::build(&SCALEFACTOR_CODE, &SCALEFACTOR_BITS),
            spectral_huffman: SPECTRAL.iter().map(|(c, b)| Huffman::from_u16(c, b)).collect(),
            imdct,
            // 2^((i - 200) / 4)
            pow2sf: (0..428).map(|i| ((i as f64 - 200.0) / 4.0).exp2() as f32).collect(),
            // i^(4/3)
            pow43: (0..8192).map(|i| (i as f64).powf(4.0 / 3.0) as f32).collect(),
            random_state: 0x1f2e_3d4c,
            ch: [Channel::default(), Channel::default()],
            ms_mask: [false; MAX_SFB],
        })
    }

    pub fn config(&self) -> Config {
        self.config
    }

    /// Decodifica una trama y añade a `out` 480 muestras estéreo intercaladas
    /// (L, R, L, R…) en el rango [-1, 1]. El mono se duplica en los dos canales.
    pub fn decode(&mut self, au: &[u8], out: &mut Vec<f32>) -> Result<()> {
        let mut br = BitReader::new(au);
        if self.config.channels == 1 {
            self.decode_ics(&mut br, 0, false)?;
        } else {
            self.decode_cpe(&mut br)?;
        }
        for c in 0..self.config.channels {
            if self.ch[c].tns.present {
                self.apply_tns(c);
            }
            self.imdct_and_window(c);
        }
        let (l, r) = (&self.ch[0].output, &self.ch[self.config.channels - 1].output);
        out.reserve(2 * N);
        for i in 0..N {
            out.push(l[i]);
            out.push(r[i]);
        }
        Ok(())
    }

    fn read_ics_info(&self, br: &mut BitReader) -> Result<usize> {
        let max_sfb = br.read(6)? as usize;
        if max_sfb > self.num_swb {
            bail!("max_sfb {max_sfb} mayor que {} bandas", self.num_swb);
        }
        Ok(max_sfb)
    }

    fn decode_cpe(&mut self, br: &mut BitReader) -> Result<()> {
        // En ELD la ventana es siempre común: no hay bit common_window.
        let max_sfb = self.read_ics_info(br)?;
        self.ch[0].max_sfb = max_sfb;
        self.ch[1].max_sfb = max_sfb;
        let ms_present = br.read(2)?;
        match ms_present {
            0 => {}
            1 => {
                for m in &mut self.ms_mask[..max_sfb] {
                    *m = br.read(1)? == 1;
                }
            }
            2 => self.ms_mask[..max_sfb].fill(true),
            _ => bail!("ms_present = 3 está reservado"),
        }
        self.decode_ics(br, 0, true)?;
        self.decode_ics(br, 1, true)?;

        let offsets = self.swb_offset;
        if ms_present != 0 {
            let [c0, c1] = &mut self.ch;
            for sfb in 0..max_sfb {
                if self.ms_mask[sfb] && c0.band_type[sfb] < NOISE_BT && c1.band_type[sfb] < NOISE_BT {
                    for k in offsets[sfb] as usize..offsets[sfb + 1] as usize {
                        let (a, b) = (c0.coeffs[k], c1.coeffs[k]);
                        c0.coeffs[k] = a + b;
                        c1.coeffs[k] = a - b;
                    }
                }
            }
        }
        // Estéreo de intensidad: el canal derecho se reconstruye a partir del izquierdo.
        let [c0, c1] = &mut self.ch;
        for sfb in 0..max_sfb {
            let bt = c1.band_type[sfb];
            if bt == INTENSITY_BT || bt == INTENSITY_BT2 {
                let mut c = -1.0 + 2.0 * f32::from(bt - INTENSITY_BT2);
                if ms_present != 0 && self.ms_mask[sfb] {
                    c = -c;
                }
                let scale = c * c1.sf[sfb];
                for k in offsets[sfb] as usize..offsets[sfb + 1] as usize {
                    c1.coeffs[k] = c0.coeffs[k] * scale;
                }
            }
        }
        Ok(())
    }

    /// individual_channel_stream en sintaxis ELD (sin pulsos ni control de ganancia).
    fn decode_ics(&mut self, br: &mut BitReader, c: usize, common_window: bool) -> Result<()> {
        let global_gain = br.read(8)? as i32;
        if !common_window {
            self.ch[c].max_sfb = self.read_ics_info(br)?;
        }
        let max_sfb = self.ch[c].max_sfb;

        // section_data
        let mut k = 0;
        while k < max_sfb {
            let band_type = br.read(4)? as u8;
            if band_type == 12 {
                bail!("tipo de banda 12 no válido");
            }
            let mut end = k;
            loop {
                let incr = br.read(5)? as usize;
                end += incr;
                if end > max_sfb {
                    bail!("la sección se sale de max_sfb");
                }
                if incr != 31 {
                    break;
                }
            }
            if end == k {
                bail!("sección vacía");
            }
            self.ch[c].band_type[k..end].fill(band_type);
            k = end;
        }

        // scale_factor_data
        let mut offset = [global_gain, global_gain - 90, 0];
        let mut first_noise = true;
        for sfb in 0..max_sfb {
            let ch = &mut self.ch[c];
            ch.sfo[sfb] = match ch.band_type[sfb] {
                ZERO_BT => 0,
                INTENSITY_BT | INTENSITY_BT2 => {
                    offset[2] += self.scalefactor_huffman.decode(br)? as i32 - 60;
                    offset[2].clamp(-155, 100) - 100
                }
                NOISE_BT => {
                    if first_noise {
                        first_noise = false;
                        offset[1] += br.read(9)? as i32 - 256;
                    } else {
                        offset[1] += self.scalefactor_huffman.decode(br)? as i32 - 60;
                    }
                    offset[1].clamp(-100, 155)
                }
                _ => {
                    offset[0] += self.scalefactor_huffman.decode(br)? as i32 - 60;
                    if !(0..=255).contains(&offset[0]) {
                        bail!("factor de escala {} fuera de rango", offset[0]);
                    }
                    offset[0] - 100
                }
            };
        }
        for sfb in 0..max_sfb {
            let ch = &mut self.ch[c];
            let sfo = ch.sfo[sfb];
            ch.sf[sfb] = match ch.band_type[sfb] {
                ZERO_BT => 0.0,
                INTENSITY_BT | INTENSITY_BT2 => self.pow2sf[(-sfo - 100 + 200) as usize],
                _ => -self.pow2sf[(sfo + 200) as usize],
            };
        }

        // TNS (en la sintaxis ER va justo antes del espectro)
        let tns_present = br.read(1)? == 1;
        self.ch[c].tns.present = tns_present;
        if tns_present {
            self.decode_tns(br, c)?;
        }
        self.decode_spectrum(br, c)
    }

    fn decode_tns(&mut self, br: &mut BitReader, c: usize) -> Result<()> {
        let tns = &mut self.ch[c].tns;
        tns.n_filt = br.read(2)? as usize;
        if tns.n_filt == 0 {
            return Ok(());
        }
        let coef_res = br.read(1)? as usize;
        for f in 0..tns.n_filt {
            tns.length[f] = br.read(6)? as usize;
            tns.order[f] = br.read(5)? as usize;
            if tns.order[f] > TNS_MAX_ORDER {
                bail!("orden de TNS {} mayor que {TNS_MAX_ORDER}", tns.order[f]);
            }
            if tns.order[f] > 0 {
                tns.direction[f] = br.read(1)? == 1;
                let compress = br.read(1)? as usize;
                let coef_len = coef_res + 3 - compress;
                let map = TNS_MAP[2 * compress + coef_res];
                for i in 0..tns.order[f] {
                    tns.coef[f][i] = map[br.read(coef_len as u32)? as usize];
                }
            }
        }
        Ok(())
    }

    fn decode_spectrum(&mut self, br: &mut BitReader, c: usize) -> Result<()> {
        let offsets = self.swb_offset;
        let max_sfb = self.ch[c].max_sfb;
        let start_zero = offsets[max_sfb] as usize;
        self.ch[c].coeffs[start_zero..].fill(0.0);

        for sfb in 0..max_sfb {
            let (lo, hi) = (offsets[sfb] as usize, offsets[sfb + 1] as usize);
            let band_type = self.ch[c].band_type[sfb];
            let sf = self.ch[c].sf[sfb];
            match band_type {
                ZERO_BT | INTENSITY_BT2 | INTENSITY_BT => self.ch[c].coeffs[lo..hi].fill(0.0),
                NOISE_BT => {
                    // PNS: ruido con la energía indicada por el factor de escala.
                    let cf = &mut self.ch[c].coeffs[lo..hi];
                    for v in cf.iter_mut() {
                        self.random_state = self.random_state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                        *v = self.random_state as i32 as f32;
                    }
                    let energy: f32 = cf.iter().map(|v| v * v).sum();
                    let scale = sf / energy.sqrt();
                    for v in cf.iter_mut() {
                        *v *= scale;
                    }
                }
                cb => {
                    let book = &self.spectral_huffman[cb as usize - 1];
                    let (dim, signed, lav): (usize, bool, i32) = match cb {
                        1 | 2 => (4, true, 1),
                        3 | 4 => (4, false, 2),
                        5 | 6 => (2, true, 4),
                        7 | 8 => (2, false, 7),
                        9 | 10 => (2, false, 12),
                        _ => (2, false, 16),
                    };
                    let modulo = if signed { 2 * lav + 1 } else { lav + 1 };
                    let mut k = lo;
                    while k < hi {
                        let mut idx = book.decode(br)? as i32;
                        let mut q = [0i32; 4];
                        for d in (0..dim).rev() {
                            q[d] = idx % modulo - if signed { lav } else { 0 };
                            idx /= modulo;
                        }
                        if !signed {
                            for v in &mut q[..dim] {
                                if *v != 0 && br.read(1)? == 1 {
                                    *v = -*v;
                                }
                            }
                        }
                        if cb == ESC_BT {
                            for v in &mut q[..dim] {
                                if v.abs() == 16 {
                                    let mut n = 0;
                                    while br.read(1)? == 1 {
                                        n += 1;
                                        if n > 8 {
                                            bail!("secuencia de escape demasiado larga");
                                        }
                                    }
                                    let bits = n + 4;
                                    let magnitude = (1 << bits) + br.read(bits)? as i32;
                                    *v = v.signum() * magnitude;
                                }
                            }
                        }
                        for (d, v) in q[..dim].iter().enumerate() {
                            let m = self.pow43[v.unsigned_abs() as usize];
                            self.ch[c].coeffs[k + d] = if *v < 0 { -m } else { m } * sf;
                        }
                        k += dim;
                    }
                }
            }
        }
        Ok(())
    }

    fn apply_tns(&mut self, c: usize) {
        let offsets = self.swb_offset;
        let ch = &mut self.ch[c];
        let mmm = self.tns_max_bands.min(ch.max_sfb);
        if mmm == 0 {
            return;
        }
        let mut bottom = self.num_swb;
        for f in 0..ch.tns.n_filt {
            let top = bottom;
            bottom = top.saturating_sub(ch.tns.length[f]);
            let order = ch.tns.order[f];
            if order == 0 {
                continue;
            }
            // Coeficientes de reflexión → LPC (compute_lpc_coefs de FFmpeg).
            let mut lpc = [0f32; TNS_MAX_ORDER];
            for i in 0..order {
                let r = -ch.tns.coef[f][i];
                lpc[i] = r;
                for j in 0..(i + 1) >> 1 {
                    let (fw, bw) = (lpc[j], lpc[i - 1 - j]);
                    lpc[j] = fw + r * bw;
                    lpc[i - 1 - j] = bw + r * fw;
                }
            }
            let start = offsets[bottom.min(mmm)] as isize;
            let end = offsets[top.min(mmm)] as isize;
            let size = end - start;
            if size <= 0 {
                continue;
            }
            let (mut pos, inc) = if ch.tns.direction[f] { (end - 1, -1) } else { (start, 1) };
            for m in 0..size as usize {
                for i in 1..=m.min(order) {
                    let prev = ch.coeffs[(pos - i as isize * inc) as usize];
                    ch.coeffs[pos as usize] -= prev * lpc[i - 1];
                }
                pos += inc;
            }
        }
    }

    /// Banco de filtros de síntesis de bajo retardo (imdct_and_windowing_eld de FFmpeg).
    fn imdct_and_window(&mut self, c: usize) {
        const N2: usize = N / 2;
        const N4: usize = N / 4;
        let ch = &mut self.ch[c];
        let input = &mut ch.coeffs;
        for i in (0..N2).step_by(2) {
            let t = input[i];
            input[i] = -input[N - 1 - i];
            input[N - 1 - i] = t;
            let t = -input[i + 1];
            input[i + 1] = input[N - 2 - i];
            input[N - 2 - i] = t;
        }
        let mut buf = [0f32; N];
        for (i, b) in buf.iter_mut().enumerate() {
            let row = &self.imdct[i * N..(i + 1) * N];
            *b = row.iter().zip(input.iter()).map(|(m, x)| m * x).sum();
        }
        for i in (0..N).step_by(2) {
            buf[i] = -buf[i];
        }

        let w = &ELD_WINDOW_480;
        let s = &ch.saved;
        let out = &mut ch.output;
        for i in N4..N2 {
            out[i - N4] = buf[N2 - 1 - i] * w[i - N4] + s[i + N2] * w[i + N - N4] - s[N + N2 - 1 - i] * w[i + 2 * N - N4]
                - s[2 * N + N2 + i] * w[i + 3 * N - N4];
        }
        for i in 0..N2 {
            out[N4 + i] = buf[i] * w[i + N2 - N4] - s[N - 1 - i] * w[i + N2 + N - N4] - s[N + i] * w[i + N2 + 2 * N - N4]
                + s[2 * N + N - 1 - i] * w[i + N2 + 3 * N - N4];
        }
        for i in 0..N4 {
            out[N2 + N4 + i] = buf[i + N2] * w[i + N - N4] - s[N2 - 1 - i] * w[i + 2 * N - N4] - s[N + N2 + i] * w[i + 3 * N - N4];
        }
        ch.saved.copy_within(0..2 * N, N);
        ch.saved[..N].copy_from_slice(&buf);
    }
}

fn parse_config(asc: &[u8]) -> Result<Config> {
    let mut br = BitReader::new(asc);
    let mut aot = br.read(5)?;
    if aot == 31 {
        aot = 32 + br.read(6)?;
    }
    if aot != 39 {
        bail!("el audio no es AAC-ELD (tipo de objeto {aot})");
    }
    let sample_rate = match br.read(4)? {
        3 => 48000,
        4 => 44100,
        15 => bail!("frecuencia de muestreo explícita no soportada"),
        i => bail!("índice de frecuencia {i} no soportado (solo 44,1 y 48 kHz)"),
    };
    let channels = match br.read(4)? {
        1 => 1,
        2 => 2,
        n => bail!("configuración de canales {n} no soportada"),
    };
    // ELDSpecificConfig
    if br.read(1)? != 1 {
        bail!("solo se soportan tramas de 480 muestras");
    }
    if br.read(3)? != 0 {
        bail!("las herramientas de resiliencia de AAC no están soportadas");
    }
    if br.read(1)? != 0 {
        bail!("AAC-ELD con SBR no está soportado");
    }
    // Extensiones (se ignoran) hasta ELDEXT_TERM.
    while br.read(4)? != 0 {
        let mut len = br.read(4)?;
        if len == 15 {
            len += br.read(8)?;
        }
        if len == 15 + 255 {
            len += br.read(16)?;
        }
        for _ in 0..len {
            br.read(8)?;
        }
    }
    Ok(Config { sample_rate, channels })
}

/// Lector de bits MSB primero. Leer más allá del final es un error.
struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn read(&mut self, n: u32) -> Result<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            let Some(&byte) = self.data.get(self.pos >> 3) else { bail!("trama AAC truncada") };
            v = (v << 1) | u32::from((byte >> (7 - (self.pos & 7))) & 1);
            self.pos += 1;
        }
        Ok(v)
    }
}

/// Árbol de decodificación Huffman construido a partir de (código, longitud).
struct Huffman {
    /// Nodo: hijos para el bit 0 y el 1. Negativo = hoja con símbolo `-(v + 1)`.
    nodes: Vec<[i32; 2]>,
}

impl Huffman {
    fn from_u16(codes: &[u16], bits: &[u8]) -> Self {
        let codes: Vec<u32> = codes.iter().map(|&c| u32::from(c)).collect();
        Self::build(&codes, bits)
    }

    fn build(codes: &[u32], bits: &[u8]) -> Self {
        let mut nodes = vec![[0i32; 2]];
        for (sym, (&code, &len)) in codes.iter().zip(bits).enumerate() {
            let mut n = 0usize;
            for b in (0..len).rev() {
                let bit = ((code >> b) & 1) as usize;
                if b == 0 {
                    nodes[n][bit] = -(sym as i32 + 1);
                } else {
                    if nodes[n][bit] == 0 {
                        nodes.push([0, 0]);
                        nodes[n][bit] = (nodes.len() - 1) as i32;
                    }
                    n = nodes[n][bit] as usize;
                }
            }
        }
        Self { nodes }
    }

    fn decode(&self, br: &mut BitReader) -> Result<u32> {
        let mut n = 0usize;
        loop {
            let next = self.nodes[n][br.read(1)? as usize];
            if next < 0 {
                return Ok((-next - 1) as u32);
            }
            if next == 0 {
                bail!("código Huffman no válido");
            }
            n = next as usize;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// AudioSpecificConfig que manda el iPhone: ELD, 44,1 kHz, estéreo, 480, sin SBR.
    pub(crate) const IPHONE_ASC: [u8; 4] = [0xf8, 0xe8, 0x50, 0x00];

    #[test]
    fn parses_iphone_config() {
        let d = Decoder::new(&IPHONE_ASC).unwrap();
        assert_eq!(d.config(), Config { sample_rate: 44100, channels: 2 });
        assert!(Decoder::new(&[0x12, 0x10]).is_err(), "AAC-LC no es ELD");
    }

    #[test]
    fn huffman_tables_are_prefix_free() {
        // Si dos códigos chocaran, el árbol perdería símbolos: todos deben decodificarse.
        let check = |codes: &[u32], bits: &[u8]| {
            let h = Huffman::build(codes, bits);
            for (sym, (&code, &len)) in codes.iter().zip(bits).enumerate() {
                let bytes = (code << (32 - len)).to_be_bytes();
                assert_eq!(h.decode(&mut BitReader::new(&bytes)).unwrap() as usize, sym);
            }
        };
        check(&SCALEFACTOR_CODE, &SCALEFACTOR_BITS);
        for (c, b) in SPECTRAL {
            check(&c.iter().map(|&x| u32::from(x)).collect::<Vec<_>>(), b);
        }
    }

    /// 12 tramas codificadas con fdk-aac (160 kbps, misma configuración que el
    /// iPhone) y el PCM que saca el decodificador de fdk: (ASC, tramas, PCM).
    pub(crate) fn fixture() -> (Vec<u8>, Vec<Vec<u8>>, Vec<f32>) {
        let (asc, aus) = read_stream(include_bytes!("testdata/eld_160.bin"));
        let pcm = include_bytes!("testdata/eld_160.pcm").chunks(2).map(|b| f32::from(i16::from_le_bytes([b[0], b[1]]))).collect();
        (asc, aus, pcm)
    }

    /// Diferencia máxima en LSB de 16 bits entre nuestra salida y la de fdk.
    pub(crate) fn max_error_lsb(ours: &[f32], reference: &[f32]) -> f32 {
        assert_eq!(ours.len(), reference.len());
        ours.iter().zip(reference).map(|(a, b)| ((a * 32768.0).clamp(-32768.0, 32767.0) - b).abs()).fold(0.0, f32::max)
    }

    #[test]
    fn matches_fdk_fixture() {
        let (asc, aus, reference) = fixture();
        let mut d = Decoder::new(&asc).unwrap();
        let mut out = Vec::new();
        for au in &aus {
            d.decode(au, &mut out).unwrap();
        }
        let err = max_error_lsb(&out, &reference);
        assert!(err <= 8.0, "error máximo {err} LSB respecto a fdk");
        assert!(reference.iter().any(|v| v.abs() > 1000.0), "la muestra no es silencio");
    }

    #[test]
    fn rejects_garbage_without_panicking() {
        let mut d = Decoder::new(&IPHONE_ASC).unwrap();
        let mut out = Vec::new();
        let mut seed = 1u32;
        for len in 0..400 {
            let junk: Vec<u8> = (0..len)
                .map(|_| {
                    seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    (seed >> 16) as u8
                })
                .collect();
            let _ = d.decode(&junk, &mut out);
        }
    }

    /// Lee un fichero del oráculo: (ASC, tramas).
    pub(crate) fn read_stream(bin: &[u8]) -> (Vec<u8>, Vec<Vec<u8>>) {
        let mut chunks = Vec::new();
        let mut pos = 0;
        while pos < bin.len() {
            let len = u16::from_le_bytes([bin[pos], bin[pos + 1]]) as usize;
            chunks.push(bin[pos + 2..pos + 2 + len].to_vec());
            pos += 2 + len;
        }
        let asc = chunks.remove(0);
        (asc, chunks)
    }

    /// Compara con fdk-aac: `REFLEX_AAC_VECTORS=<carpeta> cargo test --release aac -- --ignored`.
    /// La carpeta tiene `eld_<kbps>.bin` (ASC y tramas) y `eld_<kbps>.pcm` (PCM de fdk).
    ///
    /// Las tramas con PNS (ruido sintético) no se pueden comparar muestra a muestra,
    /// porque cada decodificador usa su propio generador aleatorio: en ellas se
    /// compara la energía. El resto debe coincidir casi exactamente.
    #[test]
    #[ignore = "necesita los ficheros generados con fdk-aac"]
    fn matches_fdk_reference() {
        let dir = std::env::var("REFLEX_AAC_VECTORS").expect("falta REFLEX_AAC_VECTORS");
        for kbps in [24, 48, 96, 160, 256] {
            let (asc, aus) = read_stream(&std::fs::read(format!("{dir}/eld_{kbps}.bin")).unwrap());
            let reference: Vec<f32> = std::fs::read(format!("{dir}/eld_{kbps}.pcm"))
                .unwrap()
                .chunks(2)
                .map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])))
                .collect();
            assert_eq!(reference.len(), aus.len() * 2 * FRAME_LEN);
            let mut d = Decoder::new(&asc).unwrap();
            let (mut exact_frames, mut max_err, mut sig, mut err) = (0, 0f32, 0f64, 0f64);
            let (mut pns_frames, mut pns_ours, mut pns_ref) = (0, 0f64, 0f64);
            // Paso alto (diferencia de tercer orden) por canal: el PNS actúa en los agudos.
            let highpass = |x: &[f32], c: usize| -> f64 {
                let s: Vec<f64> = x.iter().skip(c).step_by(2).map(|v| f64::from(*v)).collect();
                s.windows(4).map(|w| (w[3] - 3.0 * w[2] + 3.0 * w[1] - w[0]).powi(2)).sum()
            };
            let mut last_pns: Option<usize> = None;
            for (f, au) in aus.iter().enumerate() {
                let mut out = Vec::new();
                d.decode(au, &mut out).unwrap();
                let pns = d.ch.iter().any(|ch| ch.band_type[..ch.max_sfb].contains(&NOISE_BT));
                if pns {
                    last_pns = Some(f);
                }
                let r = &reference[f * 2 * FRAME_LEN..(f + 1) * 2 * FRAME_LEN];
                // El solape del banco de filtros arrastra 3 tramas.
                if last_pns.is_none_or(|p| f > p + 3) {
                    exact_frames += 1;
                    for (a, b) in out.iter().map(|v| (v * 32768.0).clamp(-32768.0, 32767.0)).zip(r) {
                        max_err = max_err.max((a - b).abs());
                        sig += f64::from(b * b);
                        err += f64::from((a - b) * (a - b));
                    }
                } else {
                    pns_frames += 1;
                    let scaled: Vec<f32> = out.iter().map(|v| v * 32768.0).collect();
                    pns_ours += highpass(&scaled, 0) + highpass(&scaled, 1);
                    pns_ref += highpass(r, 0) + highpass(r, 1);
                }
            }
            let snr = 10.0 * (sig / err.max(1e-9)).log10();
            let pns_db = if pns_frames > 0 { 10.0 * (pns_ours / pns_ref).log10() } else { 0.0 };
            eprintln!(
                "{kbps:>3} kbps: {exact_frames} tramas exactas (SNR {snr:.1} dB, error máx {max_err:.1} LSB), {pns_frames} con PNS (energía en agudos {pns_db:+.2} dB)"
            );
            assert!(exact_frames > 100, "{kbps} kbps: demasiadas tramas con PNS para comparar");
            assert!(max_err <= 8.0 && snr > 60.0, "{kbps} kbps: no coincide con fdk");
            assert!(pns_db.abs() < 1.0, "{kbps} kbps: la energía del PNS difiere {pns_db:+.2} dB");
        }
    }
}
