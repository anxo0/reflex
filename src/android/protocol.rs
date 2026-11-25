//! Protocolo binario del servidor de scrcpy v5.0.1 (big endian).
//!
//! Referencia: `server/src/main/java/com/genymobile/scrcpy/control/ControlMessageReader.java`
//! y `device/Streamer.java` en <https://github.com/Genymobile/scrcpy/tree/v5.0.1>.

pub const CODEC_H264: u32 = 0x6832_3634;
pub const CODEC_RAW_AUDIO: u32 = 0x0072_6177;

/// Cabecera de 12 bytes que precede a cada paquete del flujo.
pub const PACKET_FLAG_SESSION: u64 = 1 << 63;
pub const PACKET_FLAG_CONFIG: u64 = 1 << 62;
pub const PACKET_FLAG_KEY_FRAME: u64 = 1 << 61;
pub const PTS_MASK: u64 = PACKET_FLAG_KEY_FRAME - 1;

pub enum StreamHeader {
    /// Nueva sesión de captura (arranque, giro o cambio de tamaño).
    Session { width: u32, height: u32 },
    Packet { pts: u64, config: bool, key_frame: bool, size: u32 },
}

pub fn parse_header(h: &[u8; 12]) -> StreamHeader {
    let first = u64::from_be_bytes(h[0..8].try_into().unwrap());
    if first & PACKET_FLAG_SESSION != 0 {
        StreamHeader::Session {
            width: u32::from_be_bytes(h[4..8].try_into().unwrap()),
            height: u32::from_be_bytes(h[8..12].try_into().unwrap()),
        }
    } else {
        StreamHeader::Packet {
            pts: first & PTS_MASK,
            config: first & PACKET_FLAG_CONFIG != 0,
            key_frame: first & PACKET_FLAG_KEY_FRAME != 0,
            size: u32::from_be_bytes(h[8..12].try_into().unwrap()),
        }
    }
}

// Acciones de Android (KeyEvent / MotionEvent).
pub const ACTION_DOWN: u8 = 0;
pub const ACTION_UP: u8 = 1;
pub const ACTION_MOVE: u8 = 2;

/// Puntero de "dedo genérico": se inyecta como toque en pantalla.
pub const POINTER_ID_GENERIC_FINGER: u64 = (-2i64) as u64;

pub const META_CTRL_ON: u32 = 0x1000 | 0x2000;
pub const META_SHIFT_ON: u32 = 0x1 | 0x40;

#[allow(dead_code)] // tabla de referencia de teclas de Android
pub mod keycode {
    pub const HOME: u32 = 3;
    pub const BACK: u32 = 4;
    pub const DPAD_UP: u32 = 19;
    pub const DPAD_DOWN: u32 = 20;
    pub const DPAD_LEFT: u32 = 21;
    pub const DPAD_RIGHT: u32 = 22;
    pub const VOLUME_UP: u32 = 24;
    pub const VOLUME_DOWN: u32 = 25;
    pub const POWER: u32 = 26;
    pub const A: u32 = 29;
    pub const TAB: u32 = 61;
    pub const ENTER: u32 = 66;
    pub const DEL: u32 = 67;
    pub const MENU: u32 = 82;
    pub const PAGE_UP: u32 = 92;
    pub const PAGE_DOWN: u32 = 93;
    pub const ESCAPE: u32 = 111;
    pub const FORWARD_DEL: u32 = 112;
    pub const MOVE_HOME: u32 = 122;
    pub const MOVE_END: u32 = 123;
    pub const VOLUME_MUTE: u32 = 164;
    pub const APP_SWITCH: u32 = 187;
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum ControlMsg {
    Key { action: u8, keycode: u32, repeat: u32, meta: u32 },
    Text(String),
    Touch { action: u8, pointer: u64, x: i32, y: i32, w: u16, h: u16, pressure: f32 },
    Scroll { x: i32, y: i32, w: u16, h: u16, hscroll: f32, vscroll: f32 },
    BackOrScreenOn { action: u8 },
    ExpandNotifications,
    ExpandSettings,
    CollapsePanels,
    /// copy_key: 0 = ninguno, 1 = copiar, 2 = cortar.
    GetClipboard { copy_key: u8 },
    SetClipboard { sequence: u64, paste: bool, text: String },
    SetDisplayPower(bool),
    RotateDevice,
    ResetVideo,
}

const TEXT_MAX: usize = 300;
const CLIPBOARD_MAX: usize = (1 << 18) - 14;

fn truncate_utf8(s: &str, max: usize) -> &[u8] {
    if s.len() <= max {
        return s.as_bytes();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s.as_bytes()[..end]
}

fn position(out: &mut Vec<u8>, x: i32, y: i32, w: u16, h: u16) {
    out.extend_from_slice(&x.to_be_bytes());
    out.extend_from_slice(&y.to_be_bytes());
    out.extend_from_slice(&w.to_be_bytes());
    out.extend_from_slice(&h.to_be_bytes());
}

fn u16_fixed(v: f32) -> u16 {
    if v >= 1.0 { 0xffff } else { (v.max(0.0) * 65536.0) as u16 }
}

fn i16_fixed(v: f32) -> i16 {
    (v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}

impl ControlMsg {
    pub fn serialize(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(32);
        match self {
            ControlMsg::Key { action, keycode, repeat, meta } => {
                o.push(0);
                o.push(*action);
                o.extend_from_slice(&keycode.to_be_bytes());
                o.extend_from_slice(&repeat.to_be_bytes());
                o.extend_from_slice(&meta.to_be_bytes());
            }
            ControlMsg::Text(t) => {
                let b = truncate_utf8(t, TEXT_MAX);
                o.push(1);
                o.extend_from_slice(&(b.len() as u32).to_be_bytes());
                o.extend_from_slice(b);
            }
            ControlMsg::Touch { action, pointer, x, y, w, h, pressure } => {
                o.push(2);
                o.push(*action);
                o.extend_from_slice(&pointer.to_be_bytes());
                position(&mut o, *x, *y, *w, *h);
                o.extend_from_slice(&u16_fixed(*pressure).to_be_bytes());
                o.extend_from_slice(&0u32.to_be_bytes()); // action_button
                o.extend_from_slice(&0u32.to_be_bytes()); // buttons
            }
            ControlMsg::Scroll { x, y, w, h, hscroll, vscroll } => {
                o.push(3);
                position(&mut o, *x, *y, *w, *h);
                // El servidor multiplica por 16: el rango real es [-16, 16].
                o.extend_from_slice(&i16_fixed(hscroll / 16.0).to_be_bytes());
                o.extend_from_slice(&i16_fixed(vscroll / 16.0).to_be_bytes());
                o.extend_from_slice(&0u32.to_be_bytes());
            }
            ControlMsg::BackOrScreenOn { action } => {
                o.push(4);
                o.push(*action);
            }
            ControlMsg::ExpandNotifications => o.push(5),
            ControlMsg::ExpandSettings => o.push(6),
            ControlMsg::CollapsePanels => o.push(7),
            ControlMsg::GetClipboard { copy_key } => {
                o.push(8);
                o.push(*copy_key);
            }
            ControlMsg::SetClipboard { sequence, paste, text } => {
                let b = truncate_utf8(text, CLIPBOARD_MAX);
                o.push(9);
                o.extend_from_slice(&sequence.to_be_bytes());
                o.push(*paste as u8);
                o.extend_from_slice(&(b.len() as u32).to_be_bytes());
                o.extend_from_slice(b);
            }
            ControlMsg::SetDisplayPower(on) => {
                o.push(10);
                o.push(*on as u8);
            }
            ControlMsg::RotateDevice => o.push(11),
            ControlMsg::ResetVideo => o.push(17),
        }
        o
    }

    pub fn key_press(keycode: u32, meta: u32) -> [ControlMsg; 2] {
        [
            ControlMsg::Key { action: ACTION_DOWN, keycode, repeat: 0, meta },
            ControlMsg::Key { action: ACTION_UP, keycode, repeat: 0, meta },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_is_32_bytes() {
        let m = ControlMsg::Touch { action: ACTION_DOWN, pointer: POINTER_ID_GENERIC_FINGER, x: 10, y: 20, w: 1080, h: 2400, pressure: 1.0 };
        let b = m.serialize();
        assert_eq!(b.len(), 32);
        assert_eq!(b[0], 2);
        assert_eq!(&b[2..10], &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe]);
        assert_eq!(&b[22..24], &[0xff, 0xff]);
    }

    #[test]
    fn key_is_14_bytes() {
        assert_eq!(ControlMsg::Key { action: 0, keycode: 3, repeat: 0, meta: 0 }.serialize().len(), 14);
    }

    #[test]
    fn scroll_is_21_bytes() {
        let b = ControlMsg::Scroll { x: 1, y: 1, w: 2, h: 2, hscroll: 0.0, vscroll: 16.0 }.serialize();
        assert_eq!(b.len(), 21);
        assert_eq!(i16::from_be_bytes([b[15], b[16]]), i16::MAX);
    }

    #[test]
    fn text_is_truncated_on_char_boundary() {
        let s = "ñ".repeat(200); // 400 bytes
        let b = ControlMsg::Text(s).serialize();
        let len = u32::from_be_bytes(b[1..5].try_into().unwrap()) as usize;
        assert!(len <= 300 && len.is_multiple_of(2));
    }

    #[test]
    fn parses_headers() {
        let mut h = [0u8; 12];
        h[0] = 0x80;
        h[4..8].copy_from_slice(&1080u32.to_be_bytes());
        h[8..12].copy_from_slice(&2400u32.to_be_bytes());
        assert!(matches!(parse_header(&h), StreamHeader::Session { width: 1080, height: 2400 }));

        let pts_flags = PACKET_FLAG_KEY_FRAME | 123_456;
        h[0..8].copy_from_slice(&pts_flags.to_be_bytes());
        h[8..12].copy_from_slice(&999u32.to_be_bytes());
        match parse_header(&h) {
            StreamHeader::Packet { pts, config, key_frame, size } => {
                assert_eq!((pts, config, key_frame, size), (123_456, false, true, 999));
            }
            _ => panic!(),
        }
    }
}
