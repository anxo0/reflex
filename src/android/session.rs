//! Una sesión de duplicado: sube el servidor al móvil, lo arranca, abre los
//! sockets de vídeo, audio y control, y lanza un hilo para cada flujo.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};
use eframe::egui;

use super::protocol::ControlMsg;
use crate::adb::Adb;
use crate::logs::log;
use crate::recorder::Recorder;
use crate::{audio, paths, video};

#[derive(Clone, Debug)]
pub struct Options {
    /// Lado mayor máximo en píxeles (0 = resolución original).
    pub max_size: u16,
    pub bit_rate_mbps: u32,
    pub max_fps: u32,
    pub audio: bool,
    /// Que el sonido siga saliendo también por el móvil (Android 13+).
    pub audio_dup: bool,
    pub stay_awake: bool,
    pub show_touches: bool,
    pub screen_off: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { max_size: 1920, bit_rate_mbps: 8, max_fps: 60, audio: true, audio_dup: false, stay_awake: true, show_touches: false, screen_off: false }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Starting(String),
    Running,
    /// Terminada; `Some` con el motivo si fue por un error.
    Ended(Option<String>),
}

#[derive(Default)]
pub struct FrameSlot {
    /// Último fotograma decodificado. Se comparte con la interfaz sin copiarlo.
    pub image: Option<Arc<egui::ColorImage>>,
    pub width: usize,
    pub height: usize,
    pub seq: u64,
}

pub struct Shared {
    pub status: Mutex<Status>,
    pub frame: Mutex<FrameSlot>,
    /// Tamaño del vídeo según el servidor: las coordenadas de toque van en este espacio.
    pub video_size: Mutex<(u32, u32)>,
    pub device_name: Mutex<String>,
    pub clipboard_from_device: Mutex<Option<String>>,
    pub recorder: Mutex<Option<Recorder>>,
    pub notices: Mutex<Vec<String>>,
    pub fps: Mutex<f32>,
    /// La imagen es negra: el móvil tiene la pantalla apagada.
    pub screen_dark: AtomicBool,
    pub stop: AtomicBool,
    sockets: Mutex<Vec<TcpStream>>,
}

impl Shared {
    pub fn set_status(&self, s: Status) {
        *self.status.lock().unwrap() = s;
    }
    pub fn notice(&self, msg: impl Into<String>) {
        self.notices.lock().unwrap().push(msg.into());
    }
    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
}

pub struct Session {
    pub serial: String,
    pub shared: Arc<Shared>,
    ctrl_tx: Sender<ControlMsg>,
    server: Arc<Mutex<Option<Child>>>,
    port: Arc<Mutex<Option<u16>>>,
    adb: Adb,
}

impl Shared {
    pub fn new() -> Self {
        Shared {
            status: Mutex::new(Status::Starting("Preparando…".into())),
            frame: Mutex::new(FrameSlot::default()),
            video_size: Mutex::new((0, 0)),
            device_name: Mutex::new(String::new()),
            clipboard_from_device: Mutex::new(None),
            recorder: Mutex::new(None),
            notices: Mutex::new(Vec::new()),
            fps: Mutex::new(0.0),
            screen_dark: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            sockets: Mutex::new(Vec::new()),
        }
    }
}

impl Session {
    pub fn start(adb: Adb, serial: String, opts: Options, ctx: egui::Context) -> Session {
        let shared = Arc::new(Shared::new());
        let (ctrl_tx, ctrl_rx) = mpsc::channel();
        let server = Arc::new(Mutex::new(None));
        let port = Arc::new(Mutex::new(None));

        let session = Session { serial: serial.clone(), shared: shared.clone(), ctrl_tx: ctrl_tx.clone(), server: server.clone(), port: port.clone(), adb: adb.clone() };

        std::thread::Builder::new()
            .name("sesion".into())
            .spawn(move || {
                let result = run(&adb, &serial, &opts, &shared, ctrl_rx, ctrl_tx, &server, &port, &ctx);
                let reason = match result {
                    _ if shared.stopped() => None,
                    Ok(()) => Some("El dispositivo cerró la conexión".to_string()),
                    Err(e) => Some(format!("{e:#}")),
                };
                if let Some(r) = &reason {
                    log(format!("Sesión terminada: {r}"));
                }
                finish_recording(&shared);
                cleanup(&adb, &serial, &shared, &server, &port);
                shared.set_status(Status::Ended(reason));
                ctx.request_repaint();
            })
            .expect("no se pudo crear el hilo de sesión");

        session
    }

    pub fn send(&self, msg: ControlMsg) {
        let _ = self.ctrl_tx.send(msg);
    }

    pub fn status(&self) -> Status {
        self.shared.status.lock().unwrap().clone()
    }

    pub fn stop(&self) {
        if self.shared.stop.swap(true, Ordering::SeqCst) {
            return;
        }
        finish_recording(&self.shared);
        cleanup(&self.adb, &self.serial, &self.shared, &self.server, &self.port);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn finish_recording(shared: &Shared) {
    if let Some(rec) = shared.recorder.lock().unwrap().take() {
        match rec.finish() {
            Ok(Some(path)) => shared.notice(format!("Grabación guardada en {}", path.display())),
            Ok(None) => shared.notice("La grabación se canceló antes de recibir vídeo"),
            Err(e) => shared.notice(format!("Error al cerrar la grabación: {e:#}")),
        }
    }
}

fn cleanup(adb: &Adb, serial: &str, shared: &Shared, server: &Mutex<Option<Child>>, port: &Mutex<Option<u16>>) {
    for s in shared.sockets.lock().unwrap().drain(..) {
        let _ = s.shutdown(Shutdown::Both);
    }
    if let Some(mut child) = server.lock().unwrap().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    if let Some(p) = port.lock().unwrap().take() {
        adb.forward_remove(serial, p);
    }
}

fn server_args(scid: u32, opts: &Options) -> Vec<String> {
    let mut a = vec![
        format!("CLASSPATH={}", paths::DEVICE_SERVER_PATH),
        "app_process".into(),
        "/".into(),
        "com.genymobile.scrcpy.Server".into(),
        paths::SCRCPY_VERSION.into(),
        format!("scid={scid:08x}"),
        "log_level=info".into(),
        "tunnel_forward=true".into(),
        "video_codec=h264".into(),
        format!("max_size={}", opts.max_size),
        format!("video_bit_rate={}", opts.bit_rate_mbps * 1_000_000),
        format!("max_fps={}", opts.max_fps),
        "clipboard_autosync=true".into(),
        "power_off_on_close=false".into(),
    ];
    if opts.audio {
        // PCM sin comprimir: no hace falta decodificador de audio.
        a.push("audio_codec=raw".into());
        if opts.audio_dup {
            a.push("audio_dup=true".into());
        }
    } else {
        a.push("audio=false".into());
    }
    if opts.stay_awake {
        a.push("stay_awake=true".into());
    }
    if opts.show_touches {
        a.push("show_touches=true".into());
    }
    a
}

#[allow(clippy::too_many_arguments)]
fn run(
    adb: &Adb,
    serial: &str,
    opts: &Options,
    shared: &Arc<Shared>,
    ctrl_rx: Receiver<ControlMsg>,
    ctrl_tx: Sender<ControlMsg>,
    server: &Arc<Mutex<Option<Child>>>,
    port_slot: &Mutex<Option<u16>>,
    ctx: &egui::Context,
) -> Result<()> {
    let status = |msg: &str| {
        shared.set_status(Status::Starting(msg.into()));
        ctx.request_repaint();
    };

    let server_jar = paths::server_path();
    if !server_jar.is_file() {
        bail!("falta el servidor de scrcpy; descarga los componentes primero");
    }

    status("Copiando el servidor al móvil…");
    adb.push(serial, &server_jar, paths::DEVICE_SERVER_PATH)?;

    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos() ^ d.as_secs() as u32).unwrap_or(1);
    let scid = nanos & 0x7fff_ffff;
    let port = adb.forward_auto(serial, &format!("localabstract:scrcpy_{scid:08x}"))?;
    *port_slot.lock().unwrap() = Some(port);

    status("Arrancando el servidor en el móvil…");
    let args = server_args(scid, opts);
    log(format!("Servidor: adb -s {serial} shell {}", args.join(" ")));
    let mut child = adb
        .command()
        .args(["-s", serial, "shell"])
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("no se pudo arrancar el servidor")?;
    for pipe in [child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>), child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>)]
        .into_iter()
        .flatten()
    {
        std::thread::spawn(move || {
            for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                if !line.trim().is_empty() {
                    log(format!("[móvil] {line}"));
                }
            }
        });
    }
    *server.lock().unwrap() = Some(child);

    status("Conectando…");
    let mut video_sock = connect_first(port, shared, server)?;
    let audio_sock = if opts.audio { Some(connect(port)?) } else { None };
    let control_sock = connect(port)?;
    control_sock.set_nodelay(true)?;
    {
        let mut socks = shared.sockets.lock().unwrap();
        socks.push(video_sock.try_clone()?);
        if let Some(a) = &audio_sock {
            socks.push(a.try_clone()?);
        }
        socks.push(control_sock.try_clone()?);
    }

    let mut name = [0u8; 64];
    video_sock.read_exact(&mut name).context("no llegó el nombre del dispositivo")?;
    let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
    let device_name = String::from_utf8_lossy(&name[..end]).to_string();
    log(format!("Conectado a {device_name}"));
    *shared.device_name.lock().unwrap() = device_name;

    // Escritura de mensajes de control.
    let mut writer = control_sock.try_clone()?;
    std::thread::spawn(move || {
        for msg in ctrl_rx {
            if writer.write_all(&msg.serialize()).is_err() {
                break;
            }
        }
    });

    // Mensajes del móvil (portapapeles).
    {
        let shared = shared.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            if let Err(e) = read_device_messages(control_sock, &shared, &ctx)
                && !shared.stopped() {
                    log(format!("Canal de control cerrado: {e:#}"));
                }
        });
    }

    if let Some(sock) = audio_sock {
        let shared = shared.clone();
        std::thread::Builder::new().name("audio".into()).spawn(move || {
            if let Err(e) = audio::run(sock, &shared)
                && !shared.stopped() {
                    log(format!("Audio detenido: {e:#}"));
                    shared.notice(format!("Sin audio: {e:#}"));
                }
        })?;
    }

    shared.set_status(Status::Running);
    ctx.request_repaint();
    if opts.screen_off {
        let _ = ctrl_tx.send(ControlMsg::SetDisplayPower(false));
    }

    video::run(video_sock, shared, ctx)
}

fn connect(port: u16) -> Result<TcpStream> {
    TcpStream::connect(("127.0.0.1", port)).context("no se pudo abrir el túnel adb")
}

/// adb acepta la conexión aunque el servidor aún no escuche; el servidor manda
/// un byte de relleno cuando de verdad está listo, así que reintentamos hasta leerlo.
fn connect_first(port: u16, shared: &Shared, server: &Mutex<Option<Child>>) -> Result<TcpStream> {
    for _ in 0..100 {
        if shared.stopped() {
            bail!("cancelado");
        }
        if let Some(child) = server.lock().unwrap().as_mut()
            && let Ok(Some(st)) = child.try_wait() {
                bail!("el servidor del móvil terminó nada más arrancar ({st}). Revisa el registro");
            }
        if let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) {
            s.set_read_timeout(Some(Duration::from_secs(2)))?;
            let mut b = [0u8; 1];
            if let Ok(1) = s.read(&mut b) {
                s.set_read_timeout(None)?;
                return Ok(s);
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    bail!("el servidor del móvil no respondió a tiempo")
}

fn read_device_messages(mut s: TcpStream, shared: &Shared, ctx: &egui::Context) -> Result<()> {
    let mut ty = [0u8; 1];
    loop {
        s.read_exact(&mut ty)?;
        match ty[0] {
            0 => {
                let len = read_u32(&mut s)? as usize;
                let mut buf = vec![0u8; len];
                s.read_exact(&mut buf)?;
                *shared.clipboard_from_device.lock().unwrap() = Some(String::from_utf8_lossy(&buf).to_string());
                ctx.request_repaint();
            }
            1 => {
                let mut seq = [0u8; 8];
                s.read_exact(&mut seq)?;
            }
            2 => {
                let mut hdr = [0u8; 4];
                s.read_exact(&mut hdr)?;
                let size = u16::from_be_bytes([hdr[2], hdr[3]]) as usize;
                let mut buf = vec![0u8; size];
                s.read_exact(&mut buf)?;
            }
            other => bail!("mensaje desconocido del dispositivo: {other}"),
        }
    }
}

pub fn read_u32(r: &mut impl Read) -> std::io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_be_bytes(b))
}
