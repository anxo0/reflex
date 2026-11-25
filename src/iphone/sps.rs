//! Lectura del SPS de H.264 que manda el iPhone, para el registro.
//!
//! No se modifica: hubo una versión que subía `max_num_ref_frames` a 16 para dar
//! más memoria de imágenes a OpenH264, y estropeaba la imagen. El codificador del
//! iPhone usa referencias a largo plazo, que van detrás de las de corto plazo en la
//! lista de referencias; con más imágenes de corto plazo guardadas, un mismo índice
//! apuntaba a otra imagen y las zonas en movimiento salían desplazadas.

/// Datos del SPS que interesan para el registro.
#[derive(Debug, Clone, PartialEq)]
pub struct SpsInfo {
    pub profile: u8,
    pub level: u8,
    pub max_num_ref_frames: u32,
    pub poc_type: u32,
}

/// Lee los campos del SPS (NAL completo, con su cabecera). `None` si no se entiende.
pub fn parse(nal: &[u8]) -> Option<SpsInfo> {
    if nal.first()? & 0x1f != 7 {
        return None;
    }
    let rbsp = unescape(&nal[1..]);
    let mut r = Reader { data: &rbsp, pos: 0 };
    let profile = r.u(8)? as u8;
    r.u(8)?; // constraint_set flags
    let level = r.u(8)? as u8;
    r.ue()?; // seq_parameter_set_id
    if matches!(profile, 100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135) {
        let chroma_format_idc = r.ue()?;
        if chroma_format_idc == 3 {
            r.u(1)?;
        }
        r.ue()?; // bit_depth_luma_minus8
        r.ue()?; // bit_depth_chroma_minus8
        r.u(1)?; // qpprime_y_zero_transform_bypass_flag
        if r.u(1)? == 1 {
            let lists = if chroma_format_idc == 3 { 12 } else { 8 };
            for i in 0..lists {
                if r.u(1)? == 1 {
                    r.skip_scaling_list(if i < 6 { 16 } else { 64 })?;
                }
            }
        }
    }
    r.ue()?; // log2_max_frame_num_minus4
    let poc_type = r.ue()?;
    match poc_type {
        0 => {
            r.ue()?;
        }
        1 => {
            r.u(1)?;
            r.se()?;
            r.se()?;
            for _ in 0..r.ue()? {
                r.se()?;
            }
        }
        _ => {}
    }
    let max_num_ref_frames = r.ue()?;
    Some(SpsInfo { profile, level, max_num_ref_frames, poc_type })
}

fn bit(data: &[u8], pos: usize) -> u8 {
    (data[pos / 8] >> (7 - pos % 8)) & 1
}

/// Quita los bytes de prevención de emulación (`00 00 03` → `00 00`).
fn unescape(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut zeros = 0;
    for &b in data {
        if zeros >= 2 && b == 3 {
            zeros = 0;
            continue;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        out.push(b);
    }
    out
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn u(&mut self, n: u32) -> Option<u32> {
        let mut v = 0;
        for _ in 0..n {
            if self.pos >= self.data.len() * 8 {
                return None;
            }
            v = (v << 1) | u32::from(bit(self.data, self.pos));
            self.pos += 1;
        }
        Some(v)
    }

    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.u(1)? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        Some((1u32 << zeros) - 1 + self.u(zeros)?)
    }

    fn se(&mut self) -> Option<i32> {
        let k = self.ue()?;
        Some(if !k.is_multiple_of(2) { k.div_ceil(2) as i32 } else { -((k / 2) as i32) })
    }

    fn skip_scaling_list(&mut self, size: usize) -> Option<()> {
        let (mut last, mut next) = (8i32, 8i32);
        for _ in 0..size {
            if next != 0 {
                next = (last + self.se()? + 256) % 256;
            }
            if next != 0 {
                last = next;
            }
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_emulation_prevention() {
        assert_eq!(unescape(&[0, 0, 3, 1, 5, 0, 0, 3, 0, 0, 3, 0, 3, 9]), [0, 0, 1, 5, 0, 0, 0, 0, 0, 3, 9]);
    }

    #[test]
    fn parses_real_sps() {
        use openh264::encoder::Encoder;
        use openh264::formats::{RgbSliceU8, YUVBuffer};
        let rgb = vec![90u8; 64 * 48 * 3];
        let bits = Encoder::new().unwrap().encode(&YUVBuffer::from_rgb8_source(RgbSliceU8::new(&rgb, (64, 48)))).unwrap().to_vec();
        let sps = crate::recorder::nal_units(&bits).find(|n| n[0] & 0x1f == 7).unwrap().to_vec();
        let info = parse(&sps).unwrap();
        assert!((1..=16).contains(&info.max_num_ref_frames));
        assert!(parse(&[0x68, 1, 2]).is_none(), "un PPS no es un SPS");
    }
}
