//! Receptor de la app Reflex para Android: el móvil envía su pantalla y su sonido, y
//! si el usuario lo permite, el PC lo controla. El protocolo está en `docs/APP_PROTOCOL.md`.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use eframe::egui;
use mdns_sd::{IfKind, ServiceDaemon, ServiceInfo};

use crate::android::session::Shared;
use crate::logs::log;
use crate::video::FrameDecoder;

const PORT: u16 = 27184;
const SERVICE: &str = "_reflex._tcp.local.";

const HELLO: u8 = 0x01;
const VIDEO_CONFIG: u8 = 0x02;
const VIDEO: u8 = 0x03;
const AUDIO: u8 = 0x04;
const SIZE: u8 = 0x05;
const CONTROL: u8 = 0x06;
const BYE: u8 = 0x07;

const ACCEPT: u8 = 0x81;
const TOUCH: u8 = 0x82;
const KEY: u8 = 0x83;
const PC_BYE: u8 = 0x84;

/// Teclas que entiende la app (acciones globales de accesibilidad).
#[derive(Clone, Copy)]
pub enum Key {
    Back = 1,
    Home = 2,
    Recents = 3,
    Notifications = 4,
    Lock = 5,
}

#[derive(Clone, Copy)]
pub enum Touch {
    Down = 0,
    Move = 1,
    Up = 2,
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Starting,
    /// Anunciado y esperando al móvil.
    Waiting,
    /// Un móvil sin código pide permiso: se pregunta al usuario.
    Asking(String),
    /// Enviando: nombre del móvil.
    Connected(String),
    Failed(String),
}

pub struct Receiver {
    pub shared: Arc<Shared>,
    pub code: String,
    pub name: String,
    port: Arc<Mutex<u16>>,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    decision: Arc<Mutex<Option<bool>>>,
    writer: Arc<Mutex<Option<TcpStream>>>,
    /// El móvil tiene activado el servicio de accesibilidad: se le puede controlar.
    pub control: Arc<AtomicBool>,
}

impl Receiver {
    pub fn start(ctx: egui::Context) -> Receiver {
        let mut n = [0u8; 2];
        let _ = getrandom::fill(&mut n);
        let code = format!("{:04}", u16::from_le_bytes(n) % 10_000);
        let r = Receiver {
            shared: Arc::new(Shared::new()),
            code: code.clone(),
            name: pc_name(),
            port: Arc::new(Mutex::new(0)),
            state: Arc::new(Mutex::new(State::Starting)),
            stop: Arc::new(AtomicBool::new(false)),
            decision: Arc::new(Mutex::new(None)),
            writer: Arc::new(Mutex::new(None)),
            control: Arc::new(AtomicBool::new(false)),
        };
        let w = Worker {
            shared: r.shared.clone(),
            code,
            name: r.name.clone(),
            port: r.port.clone(),
            state: r.state.clone(),
            stop: r.stop.clone(),
            decision: r.decision.clone(),
            writer: r.writer.clone(),
            control: r.control.clone(),
            ctx,
        };
        std::thread::spawn(move || {
            if let Err(e) = w.run() {
                log(format!("App Reflex: {e:#}"));
                *w.state.lock().unwrap() = State::Failed(format!("{e:#}"));
                w.ctx.request_repaint();
            }
        });
        r
    }

    pub fn state(&self) -> State {
        self.state.lock().unwrap().clone()
    }

    /// Lo que codifica el QR del PC.
    pub fn qr_text(&self) -> Option<String> {
        let port = *self.port.lock().unwrap();
        let ip = local_ip()?;
        (port != 0).then(|| format!("reflex://{ip}:{port}?c={}&n={}", self.code, url_escape(&self.name)))
    }

    pub fn answer(&self, accept: bool) {
        *self.decision.lock().unwrap() = Some(accept);
    }

    pub fn touch(&self, action: Touch, x: f32, y: f32) {
        let mut p = vec![action as u8];
        p.extend_from_slice(&x.clamp(0.0, 1.0).to_be_bytes());
        p.extend_from_slice(&y.clamp(0.0, 1.0).to_be_bytes());
        self.send(TOUCH, &p);
    }

    pub fn key(&self, key: Key) {
        self.send(KEY, &[key as u8]);
    }

    fn send(&self, ty: u8, payload: &[u8]) {
        if let Some(s) = self.writer.lock().unwrap().as_mut() {
            let _ = write_msg(s, ty, payload);
        }
    }

    /// Corta el móvil actual pero sigue esperando a otro.
    pub fn disconnect(&self) {
        if let Some(mut s) = self.writer.lock().unwrap().take() {
            let _ = write_msg(&mut s, PC_BYE, &[]);
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.disconnect();
        crate::android::session::finish_recording(&self.shared);
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Worker {
    shared: Arc<Shared>,
    code: String,
    name: String,
    port: Arc<Mutex<u16>>,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    decision: Arc<Mutex<Option<bool>>>,
    writer: Arc<Mutex<Option<TcpStream>>>,
    control: Arc<AtomicBool>,
    ctx: egui::Context,
}

impl Worker {
    fn set(&self, s: State) {
        *self.state.lock().unwrap() = s;
        self.ctx.request_repaint();
    }

    fn run(&self) -> Result<()> {
        let listener = TcpListener::bind(("0.0.0.0", PORT))
            .or_else(|_| TcpListener::bind(("0.0.0.0", 0)))
            .context("no se pudo abrir el puerto para la app")?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        *self.port.lock().unwrap() = port;
        let _mdns = Advertiser::start(&self.name, port).map_err(|e| log(format!("App Reflex: sin mDNS ({e:#})"))).ok();
        log(format!("App Reflex: esperando en el puerto {port}"));
        self.set(State::Waiting);

        while !self.stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((s, addr)) => {
                    log(format!("App Reflex: conexión desde {addr}"));
                    if let Err(e) = self.serve(s) {
                        log(format!("App Reflex: {e:#}"));
                        if !self.stop.load(Ordering::Relaxed) {
                            self.shared.notice(format!("{}: {e:#}", crate::i18n::t("La app se desconectó", "The app disconnected")));
                        }
                    }
                    *self.writer.lock().unwrap() = None;
                    self.control.store(false, Ordering::Relaxed);
                    crate::android::session::finish_recording(&self.shared);
                    {
                        let mut f = self.shared.frame.lock().unwrap();
                        f.image = None;
                    }
                    if !self.stop.load(Ordering::Relaxed) {
                        self.set(State::Waiting);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(100)),
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    fn serve(&self, mut s: TcpStream) -> Result<()> {
        s.set_nonblocking(false)?;
        s.set_nodelay(true)?;
        s.set_read_timeout(Some(Duration::from_secs(10)))?;
        let (ty, payload) = read_msg(&mut s)?;
        if ty != HELLO {
            bail!("la app no se presentó");
        }
        let hello = parse_hello(&payload);
        let phone = hello.get("name").cloned().filter(|n| !n.is_empty()).unwrap_or_else(|| "Android".into());
        let id = hello.get("id").cloned().unwrap_or_default();
        let code_ok = hello.get("code").is_some_and(|c| *c == self.code);
        let known = !id.is_empty() && accepted_ids().contains(&id);

        let accepted = if code_ok || known {
            true
        } else {
            *self.decision.lock().unwrap() = None;
            self.set(State::Asking(phone.clone()));
            let since = Instant::now();
            loop {
                if let Some(d) = self.decision.lock().unwrap().take() {
                    break d;
                }
                if self.stop.load(Ordering::Relaxed) || since.elapsed() > Duration::from_secs(60) {
                    break false;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        };
        if !accepted {
            let mut p = vec![0u8];
            p.extend_from_slice(if code_ok { b"" } else { "rechazado en el PC".as_bytes() });
            let _ = write_msg(&mut s, ACCEPT, &p);
            bail!("no se aceptó a {phone}");
        }
        if !id.is_empty() && !known {
            remember_id(&id);
        }
        write_msg(&mut s, ACCEPT, &[1])?;
        *self.writer.lock().unwrap() = Some(s.try_clone()?);
        self.control.store(hello.get("control").is_some_and(|c| c == "1"), Ordering::Relaxed);
        *self.shared.device_name.lock().unwrap() = phone.clone();
        let size = (hello.get("w").and_then(|v| v.parse().ok()).unwrap_or(0), hello.get("h").and_then(|v| v.parse().ok()).unwrap_or(0));
        *self.shared.video_size.lock().unwrap() = size;
        log(format!("App Reflex: {phone} conectado ({}x{})", size.0, size.1));
        self.set(State::Connected(phone));

        s.set_read_timeout(Some(Duration::from_secs(15)))?;
        let mut decoder = FrameDecoder::new()?;
        let mut config: Option<Vec<u8>> = None;
        let mut pending_config = false;
        let mut player: Option<crate::audio::Player> = None;
        let mut audio_failed = false;
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            let (ty, mut payload) = read_msg(&mut s)?;
            match ty {
                VIDEO_CONFIG => {
                    config = Some(payload);
                    pending_config = true;
                }
                VIDEO if payload.len() > 9 => {
                    let pts = u64::from_be_bytes(payload[..8].try_into().unwrap());
                    let key = payload[8] & 1 != 0;
                    let mut packet = payload.split_off(9);
                    decoder.count_bytes(packet.len());
                    if let Some(rec) = self.shared.recorder.lock().unwrap().as_mut() {
                        let (w, h) = *self.shared.video_size.lock().unwrap();
                        if let Err(e) = rec.push(&packet, pts, key, config.as_deref(), w, h) {
                            log(format!("Error grabando: {e:#}"));
                        }
                    }
                    if pending_config && let Some(c) = &config {
                        pending_config = false;
                        packet.splice(0..0, c.iter().copied());
                    }
                    let _ = decoder.decode(&packet, &self.shared, &self.ctx);
                }
                AUDIO => {
                    if player.is_none() && !audio_failed {
                        match crate::audio::Player::new() {
                            Ok(p) => player = Some(p),
                            Err(e) => {
                                audio_failed = true;
                                log(format!("App Reflex: sin audio ({e:#})"));
                            }
                        }
                    }
                    if let Some(p) = player.as_mut() {
                        p.push_s16le(&payload);
                    }
                }
                SIZE if payload.len() >= 8 => {
                    let w = u32::from_be_bytes(payload[..4].try_into().unwrap());
                    let h = u32::from_be_bytes(payload[4..8].try_into().unwrap());
                    log(format!("App Reflex: la pantalla giró, {w}x{h}"));
                    *self.shared.video_size.lock().unwrap() = (w, h);
                    if self.shared.recorder.lock().unwrap().as_ref().is_some_and(|r| r.is_started()) {
                        crate::android::session::finish_recording(&self.shared);
                    }
                }
                CONTROL => {
                    self.control.store(payload.first() == Some(&1), Ordering::Relaxed);
                    self.ctx.request_repaint();
                }
                BYE => return Ok(()),
                _ => {}
            }
        }
    }
}

fn read_msg(s: &mut TcpStream) -> Result<(u8, Vec<u8>)> {
    let mut head = [0u8; 5];
    s.read_exact(&mut head)?;
    let len = u32::from_be_bytes(head[1..5].try_into().unwrap()) as usize;
    if len > 16 * 1024 * 1024 {
        bail!("mensaje demasiado grande ({len} bytes)");
    }
    let mut payload = vec![0u8; len];
    s.read_exact(&mut payload)?;
    Ok((head[0], payload))
}

fn write_msg(s: &mut TcpStream, ty: u8, payload: &[u8]) -> std::io::Result<()> {
    let mut buf = Vec::with_capacity(5 + payload.len());
    buf.push(ty);
    buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    buf.extend_from_slice(payload);
    s.write_all(&buf)
}

fn parse_hello(payload: &[u8]) -> HashMap<String, String> {
    String::from_utf8_lossy(payload)
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

fn accepted_path() -> std::path::PathBuf {
    crate::paths::data_dir().join("apps_aceptadas.txt")
}

fn accepted_ids() -> Vec<String> {
    std::fs::read_to_string(accepted_path()).unwrap_or_default().lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()
}

fn remember_id(id: &str) {
    let mut ids = accepted_ids();
    ids.push(id.to_string());
    let _ = std::fs::write(accepted_path(), ids.join("\n"));
}

/// Nombre con el que el PC sale en la app.
pub fn pc_name() -> String {
    let pc = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_default();
    if pc.is_empty() { "Reflex".into() } else { pc }
}

/// IP de este PC en la red local (la de la ruta por defecto; no envía nada).
fn local_ip() -> Option<Ipv4Addr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:80").ok()?;
    match s.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

fn url_escape(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

struct Advertiser {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Advertiser {
    fn start(name: &str, port: u16) -> Result<Self> {
        let daemon = ServiceDaemon::new().context("no se pudo iniciar mDNS")?;
        daemon.disable_interface(IfKind::IPv6).context("no se pudo configurar mDNS")?;
        let label: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
        let host = format!("{}-reflexapp.local.", label.trim_matches('-'));
        let props = [("name", name), ("v", "1")];
        let info = ServiceInfo::new(SERVICE, name, &host, "", port, &props[..]).context("servicio mDNS no válido")?.enable_addr_auto();
        let fullname = info.get_fullname().to_string();
        daemon.register(info).context("no se pudo anunciar el PC")?;
        Ok(Self { daemon, fullname })
    }
}

impl Drop for Advertiser {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

/// Dibuja un QR como imagen (módulos negros sobre blanco, con margen).
pub fn qr_image(text: &str) -> Option<egui::ColorImage> {
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    let n = code.width();
    let colors = code.to_colors();
    let quiet = 2;
    let scale = 6;
    let side = (n + quiet * 2) * scale;
    let mut pixels = vec![egui::Color32::WHITE; side * side];
    for (i, c) in colors.iter().enumerate() {
        if *c == qrcode::Color::Dark {
            let (mx, my) = (i % n + quiet, i / n + quiet);
            for dy in 0..scale {
                for dx in 0..scale {
                    pixels[(my * scale + dy) * side + mx * scale + dx] = egui::Color32::from_rgb(0x0B, 0x0E, 0x1A);
                }
            }
        }
    }
    Some(egui::ColorImage::new([side, side], pixels))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_and_frames() {
        let h = parse_hello(b"v=1\nname=Pixel 8\ncode=0420\ncontrol=1\n");
        assert_eq!(h["name"], "Pixel 8");
        assert_eq!(h["code"], "0420");
        assert_eq!(url_escape("PC SALÓN"), "PC%20SAL%C3%93N");
        assert!(qr_image("reflex://192.168.1.2:27184?c=1234").is_some());
    }

    #[test]
    fn session_over_tcp() {
        let ctx = egui::Context::default();
        let r = Receiver::start(ctx);
        let mut port = 0;
        for _ in 0..50 {
            port = *r.port.lock().unwrap();
            if port != 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_ne!(port, 0);
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let hello = format!("v=1\nname=Prueba\nid=\ncode={}\nw=720\nh=1600\ncontrol=1\n", r.code);
        write_msg(&mut s, HELLO, hello.as_bytes()).unwrap();
        let (ty, p) = read_msg(&mut s).unwrap();
        assert_eq!((ty, p[0]), (ACCEPT, 1));
        for _ in 0..50 {
            if matches!(r.state(), State::Connected(_)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(r.state(), State::Connected("Prueba".into()));
        assert!(r.control.load(Ordering::Relaxed));
        r.key(Key::Home);
        let (ty, p) = read_msg(&mut s).unwrap();
        assert_eq!((ty, p), (KEY, vec![2]));
        write_msg(&mut s, BYE, &[]).unwrap();
        r.stop();
    }
}
