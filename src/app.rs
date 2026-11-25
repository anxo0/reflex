//! Interfaz gráfica (egui), diseño «Compacta»: una ventana del tamaño de un móvil.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Sense, Stroke, Vec2, pos2, vec2};

use crate::adb::{Adb, Device, MdnsService};
use crate::android::protocol::{ACTION_DOWN, ACTION_MOVE, ACTION_UP, META_CTRL_ON, POINTER_ID_GENERIC_FINGER};
use crate::android::{ControlMsg, Options, Session, Status, keycode};
use crate::logs::{self, log};
use crate::recorder::Recorder;
use crate::i18n::{self, Lang, t};
use crate::{paths, setup};

// Colores del manual de marca.
const BLUE: Color32 = Color32::from_rgb(0x00, 0x14, 0x89);
const BLUE_BRIGHT: Color32 = Color32::from_rgb(0x24, 0x47, 0xE0);
const ICE: Color32 = Color32::from_rgb(0xA9, 0xBF, 0xFF);
const ICE_SOFT: Color32 = Color32::from_rgb(0xEE, 0xF2, 0xFF);
const INK: Color32 = Color32::from_rgb(0x0B, 0x0E, 0x1A);
const MUTED: Color32 = Color32::from_rgb(0x4A, 0x51, 0x70);
const SURFACE: Color32 = Color32::from_rgb(0xF6, 0xF8, 0xFC);
const LINE: Color32 = Color32::from_rgb(0xDC, 0xE2, 0xF0);
const DARK_RAISED: Color32 = Color32::from_rgb(0x15, 0x1A, 0x2E);
const DARK_LINE: Color32 = Color32::from_rgb(0x26, 0x2D, 0x47);
const DARK_TEXT: Color32 = Color32::from_rgb(0xEE, 0xF2, 0xFF);
const DARK_MUTED: Color32 = Color32::from_rgb(0xA3, 0xAB, 0xC8);
const REC: Color32 = Color32::from_rgb(0xFF, 0x5A, 0x4F);
const ERROR: Color32 = Color32::from_rgb(0xB4, 0x23, 0x18);
const OK: Color32 = Color32::from_rgb(0x1D, 0x7A, 0x46);

pub const WINDOW: Vec2 = vec2(440.0, 860.0);
const BEZEL: f32 = 8.0;
const GAP: f32 = 12.0;
const TOOLS_W: f32 = 56.0;
const TOOLS_MIN_H: f32 = 560.0;
const TOOLS_MAX_H: f32 = 760.0;

macro_rules! icon {
    ($name:literal) => {
        egui::include_image!(concat!("../assets/icons/", $name, ".svg"))
    };
}

#[derive(PartialEq, Clone, Copy)]
enum Screen {
    Home,
    Wifi,
    Usb,
    Cast,
    Settings,
}

#[derive(Clone, PartialEq)]
enum WifiStage {
    Idle,
    Working(String),
    Done(String),
    Error(String),
}

pub struct ReflexApp {
    screen: Screen,
    lang: Lang,
    adb: Option<Adb>,
    setup: Arc<Mutex<setup::SetupState>>,
    devices: Arc<Mutex<Vec<Device>>>,
    poller_started: bool,
    mdns: Arc<Mutex<Vec<MdnsService>>>,
    mdns_active: Arc<AtomicBool>,
    wifi: Arc<Mutex<WifiStage>>,
    session: Option<Session>,
    texture: Option<egui::TextureHandle>,
    last_seq: u64,
    sized_for: (u32, u32),
    tools_hidden: bool,
    mirror_aspect: f32,
    /// Proporción con la que se abrió la ventana (la última conocida de ese móvil).
    expected_aspect: f32,
    /// Alto del móvil (vertical) que dejó el usuario; se recuerda entre sesiones.
    mirror_height: f32,
    opts: Options,
    pair_code: String,
    manual: bool,
    manual_pair: String,
    manual_connect: String,
    task_busy: Arc<AtomicBool>,
    task_msg: Arc<Mutex<Option<(bool, String)>>>,
    notice: Option<(String, Instant)>,
    mirror_focus: bool,
    touching: bool,
    fullscreen: bool,
    device_screen_off: bool,
    show_log: bool,
    clipboard_seq: u64,
    last_title: String,
}

impl ReflexApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        crate::window::init(cc);
        apply_theme(&cc.egui_ctx);
        let mut app = Self {
            screen: Screen::Home,
            lang: i18n::init(),
            adb: None,
            setup: Arc::new(Mutex::new(setup::SetupState::default())),
            devices: Arc::new(Mutex::new(Vec::new())),
            poller_started: false,
            mdns: Arc::new(Mutex::new(Vec::new())),
            mdns_active: Arc::new(AtomicBool::new(false)),
            wifi: Arc::new(Mutex::new(WifiStage::Idle)),
            session: None,
            texture: None,
            last_seq: 0,
            sized_for: (0, 0),
            tools_hidden: false,
            mirror_aspect: 9.0 / 20.0,
            expected_aspect: 9.0 / 20.0,
            mirror_height: load_mirror_height(),
            opts: Options::default(),
            pair_code: String::new(),
            manual: false,
            manual_pair: String::new(),
            manual_connect: String::new(),
            task_busy: Arc::new(AtomicBool::new(false)),
            task_msg: Arc::new(Mutex::new(None)),
            notice: None,
            mirror_focus: false,
            touching: false,
            fullscreen: false,
            device_screen_off: false,
            show_log: false,
            clipboard_seq: 1,
            last_title: String::new(),
        };
        app.refresh_adb(&cc.egui_ctx);
        app
    }

    fn refresh_adb(&mut self, ctx: &egui::Context) {
        if self.adb.is_none() {
            self.adb = paths::find_adb().map(Adb::new);
            if let Some(a) = &self.adb {
                log(format!("Usando adb: {}", a.path.display()));
            }
        }
        if let (Some(adb), false) = (&self.adb, self.poller_started) {
            self.poller_started = true;
            let adb = adb.clone();
            let devices = self.devices.clone();
            let mdns = self.mdns.clone();
            let mdns_active = self.mdns_active.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                if let Err(e) = adb.start_server() {
                    log(format!("No se pudo iniciar adb: {e:#}"));
                }
                loop {
                    if let Ok(list) = adb.devices() {
                        let mut d = devices.lock().unwrap();
                        if *d != list {
                            *d = list;
                            ctx.request_repaint();
                        }
                    }
                    // Solo buscamos móviles en la red mientras el asistente Wi-Fi está abierto.
                    if mdns_active.load(Ordering::Relaxed) {
                        let found = adb.mdns_services().unwrap_or_default();
                        let mut m = mdns.lock().unwrap();
                        if *m != found {
                            *m = found;
                            ctx.request_repaint();
                        }
                    }
                    std::thread::sleep(Duration::from_millis(1500));
                }
            });
        }
    }

    fn components_ready(&self) -> bool {
        self.adb.is_some() && !setup::needs_server()
    }

    fn toast(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        log(&msg);
        self.notice = Some((msg, Instant::now()));
    }

    fn send(&self, msg: ControlMsg) {
        if let Some(s) = &self.session {
            s.send(msg);
        }
    }

    fn press(&self, code: u32) {
        for m in ControlMsg::key_press(code, 0) {
            self.send(m);
        }
    }

    fn background(&self, label: &str, job: impl FnOnce(&Adb) -> anyhow::Result<String> + Send + 'static) {
        let Some(adb) = self.adb.clone() else { return };
        if self.task_busy.swap(true, Ordering::SeqCst) {
            return;
        }
        *self.task_msg.lock().unwrap() = Some((true, format!("{label}…")));
        let busy = self.task_busy.clone();
        let msg = self.task_msg.clone();
        std::thread::spawn(move || {
            let r = job(&adb);
            *msg.lock().unwrap() = Some(match r {
                Ok(m) => (true, m),
                Err(e) => (false, format!("{e:#}")),
            });
            busy.store(false, Ordering::SeqCst);
        });
    }

    fn start_session(&mut self, ctx: &egui::Context, serial: String) {
        let Some(adb) = self.adb.clone() else { return };
        self.stop_session(ctx);
        self.texture = None;
        self.last_seq = 0;
        self.sized_for = (0, 0);
        self.device_screen_off = self.opts.screen_off;
        let serial_for_size = serial.clone();
        self.session = Some(Session::start(adb, serial, self.opts.clone(), ctx.clone()));
        // La ventana se abre ya con su tamaño final: el que dejó el usuario y la forma de ese móvil.
        let aspect = load_aspect(&serial_for_size).unwrap_or(9.0 / 20.0);
        self.mirror_height = self.mirror_height.min(max_phone_height(ctx));
        self.expected_aspect = aspect;
        self.mirror_aspect = aspect;
        crate::window::resize(ctx, mirror_window_size(self.mirror_height, aspect));
    }

    fn stop_session(&mut self, ctx: &egui::Context) {
        let had = self.session.is_some();
        if let Some(s) = self.session.take() {
            s.stop();
            for n in s.shared.notices.lock().unwrap().drain(..) {
                self.notice = Some((n, Instant::now()));
            }
        }
        self.texture = None;
        self.touching = false;
        if self.fullscreen {
            self.fullscreen = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
        if had {
            crate::window::resize(ctx, WINDOW);
        }
    }

    /// Asistente Wi-Fi: vincula con el código y se conecta al móvil encontrado en la red.
    fn start_pairing(&mut self, pairing: Option<String>) {
        let Some(adb) = self.adb.clone() else { return };
        let code: String = self.pair_code.chars().filter(|c| c.is_ascii_digit()).collect();
        let manual_connect = self.manual_connect.trim().to_string();
        let wifi = self.wifi.clone();
        let mdns = self.mdns.clone();
        *wifi.lock().unwrap() = WifiStage::Working(t("Vinculando…", "Pairing…").into());
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<String> {
                let mut ip = None;
                if let Some(addr) = pairing {
                    adb.pair(&addr, &code)?;
                    ip = addr.rsplit_once(':').map(|(ip, _)| ip.to_string());
                }
                *wifi.lock().unwrap() = WifiStage::Working(t("Conectando…", "Connecting…").into());
                if !manual_connect.is_empty() {
                    adb.connect(&manual_connect)?;
                    return Ok(manual_connect);
                }
                // Tras vincular, el móvil se anuncia como listo para conectar.
                for _ in 0..20 {
                    let found = adb.mdns_services().unwrap_or_default();
                    *mdns.lock().unwrap() = found.clone();
                    if let Some(s) = found.iter().find(|s| s.is_connect() && ip.as_deref().is_none_or(|ip| s.ip() == ip)) {
                        adb.connect(&s.addr)?;
                        return Ok(s.addr.clone());
                    }
                    std::thread::sleep(Duration::from_millis(700));
                }
                anyhow::bail!("{}", t("Vinculado, pero el móvil no apareció en la red. Escribe su IP y puerto a mano", "Paired, but the phone did not show up on the network. Type its IP and port"))
            })();
            *wifi.lock().unwrap() = match result {
                Ok(serial) => WifiStage::Done(serial),
                Err(e) => WifiStage::Error(format!("{e:#}")),
            };
        });
    }

    fn screenshot(&mut self) {
        let Some(s) = &self.session else { return };
        let (rgba, w, h) = {
            let f = s.shared.frame.lock().unwrap();
            let rgba: Vec<u8> = f.image.as_ref().map(|i| i.pixels.iter().flat_map(|p| p.to_array()).collect()).unwrap_or_default();
            (rgba, f.width as u32, f.height as u32)
        };
        if w == 0 {
            return;
        }
        let dir = paths::screenshots_dir();
        let path = dir.join(format!("captura_{}.png", paths::timestamp()));
        let r = std::fs::create_dir_all(&dir)
            .map_err(anyhow::Error::from)
            .and_then(|_| image::save_buffer(&path, &rgba, w, h, image::ColorType::Rgba8).map_err(anyhow::Error::from));
        match r {
            Ok(()) => self.toast(format!("{} {}", t("Captura guardada en", "Screenshot saved to"), path.display())),
            Err(e) => self.toast(format!("{}: {e:#}", t("No se pudo guardar la captura", "Could not save the screenshot"))),
        }
    }

    fn is_recording(&self) -> bool {
        self.session.as_ref().is_some_and(|s| s.shared.recorder.lock().unwrap().is_some())
    }

    fn toggle_recording(&mut self) {
        let Some(s) = &self.session else { return };
        if self.is_recording() {
            crate::android::session::finish_recording(&s.shared);
        } else {
            let path = paths::recordings_dir().join(format!("grabacion_{}.mp4", paths::timestamp()));
            *s.shared.recorder.lock().unwrap() = Some(Recorder::new(path));
            s.send(ControlMsg::ResetVideo);
            self.toast("Grabando. Pulsa otra vez para parar");
        }
    }

    fn toggle_fullscreen(&mut self, ctx: &egui::Context) {
        self.fullscreen = !self.fullscreen;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
    }

    // ---------------------------------------------------------------- pantallas

    /// Barra de título propia: se arrastra para mover la ventana.
    fn header(&mut self, ui: &mut egui::Ui, title: Option<&str>) {
        let ctx = ui.ctx().clone();
        let bar = egui::Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), 44.0));
        // Se registra antes que los botones para que estos tengan prioridad.
        let drag = ui.interact(bar, ui.scope_id().with("barra"), Sense::click_and_drag());
        if drag.drag_started() {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        ui.horizontal(|ui| {
            ui.set_height(44.0);
            if let Some(title_text) = title {
                if icon_button(ui, icon!("back"), MUTED, t("Volver", "Back")).clicked() {
                    self.screen = Screen::Home;
                    self.mdns_active.store(false, Ordering::Relaxed);
                }
                ui.label(RichText::new(title_text).size(16.0).strong().color(INK));
            } else {
                ui.add(egui::Image::new(egui::include_image!("../assets/icon-64.png")).fit_to_exact_size(vec2(26.0, 26.0)));
                ui.label(RichText::new("Reflex").size(18.0).strong().color(BLUE));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if icon_button(ui, icon!("close"), MUTED, t("Cerrar", "Close")).clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if icon_button(ui, icon!("minus"), MUTED, t("Minimizar", "Minimize")).clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
                if title.is_none() && self.components_ready() && icon_button(ui, icon!("settings"), MUTED, t("Ajustes", "Settings")).clicked() {
                    self.screen = Screen::Settings;
                }
            });
        });
    }

    fn setup_view(&mut self, ui: &mut egui::Ui) {
        self.header(ui, None);
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.add(egui::Image::new(egui::include_image!("../assets/icon-128.png")).fit_to_exact_size(vec2(96.0, 96.0)));
            ui.add_space(16.0);
            ui.label(RichText::new(t("Te damos la bienvenida", "Welcome")).size(24.0).strong().color(INK));
            ui.add_space(8.0);
            ui.label(RichText::new(t("Antes de empezar hay que descargar dos componentes libres (unos 15 MB). Solo se hace una vez.", "Before you start, Reflex needs to download two free components (about 15 MB). This only happens once.")).color(MUTED));
        });
        ui.add_space(24.0);
        let st = self.setup.lock().unwrap();
        let running = st.running;
        let err = st.error.clone();
        if running {
            ui.label(RichText::new(&st.message).color(MUTED));
            ui.add(egui::ProgressBar::new(st.progress).fill(BLUE_BRIGHT).show_percentage());
        }
        drop(st);
        if let Some(e) = err {
            ui.label(RichText::new(format!("{}: {e}", t("No se pudo descargar", "Download failed"))).color(ERROR));
        }
        ui.add_space(12.0);
        if !running && primary_button(ui, t("Descargar y empezar", "Download and start"), None).clicked() {
            setup::start(self.setup.clone(), ui.ctx().clone());
        }
    }

    fn home_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.header(ui, None);
        ui.add_space(16.0);

        let devices = self.devices.lock().unwrap().clone();
        let mut start = None;
        let mut to_wifi = None;
        for d in devices.iter().filter(|d| d.state == "device") {
            card(BLUE, None).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    icon_tile(ui, icon!("smartphone"), Color32::WHITE, BLUE_BRIGHT);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(d.label()).size(16.0).strong().color(Color32::WHITE));
                        let how = if d.is_wireless() { t("Por Wi-Fi", "Over Wi-Fi") } else { t("Por cable USB", "Over USB cable") };
                        ui.label(RichText::new(how).size(12.0).color(Color32::from_rgb(0xDC, 0xE6, 0xFF)));
                    });
                });
                ui.add_space(12.0);
                let b = egui::Button::image_and_text(
                    egui::Image::new(icon!("play")).tint(BLUE).fit_to_exact_size(vec2(16.0, 16.0)),
                    RichText::new(t("Duplicar", "Mirror")).strong().size(15.0).color(BLUE),
                )
                .fill(Color32::WHITE)
                .corner_radius(CornerRadius::same(12))
                .min_size(vec2(ui.available_width(), 46.0));
                let r = ui.add(b);
                hover_overlay(ui, &r, 12, Color32::from_rgba_unmultiplied(0x24, 0x47, 0xE0, 30));
                if r.clicked() {
                    start = Some(d.serial.clone());
                }
                if !d.is_wireless() {
                    ui.add_space(4.0);
                    let link = ui.add(egui::Button::new(RichText::new(t("Pasar a Wi-Fi y quitar el cable", "Switch to Wi-Fi and unplug")).size(12.0).color(Color32::WHITE)).frame(false));
                    if link.clicked() {
                        to_wifi = Some(d.serial.clone());
                    }
                }
            });
            ui.add_space(10.0);
        }
        for d in devices.iter().filter(|d| d.state == "unauthorized") {
            card(Color32::WHITE, Some(LINE)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    icon_tile(ui, icon!("smartphone"), BLUE, ICE_SOFT);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(d.label()).strong().color(INK));
                        ui.label(RichText::new(t("Acepta el aviso «¿Permitir depuración?» en el móvil", "Accept the “Allow debugging?” prompt on the phone")).size(12.0).color(MUTED));
                    });
                });
            });
            ui.add_space(10.0);
        }
        if let Some(serial) = start {
            self.start_session(ctx, serial);
        }
        if let Some(serial) = to_wifi {
            self.background(t("Pasando a Wi-Fi", "Switching to Wi-Fi"), move |adb| adb.switch_to_wifi(&serial).map(|_| t("Listo. Ya puedes quitar el cable", "Done. You can unplug the cable").into()));
        }
        if let Some((ok, msg)) = self.task_msg.lock().unwrap().clone() {
            ui.label(RichText::new(msg).size(12.0).color(if ok { OK } else { ERROR }));
            ui.add_space(6.0);
        }

        ui.add_space(8.0);
        ui.label(RichText::new(if devices.iter().any(|d| d.state == "device") { t("CONECTAR OTRO MÓVIL", "CONNECT ANOTHER PHONE") } else { t("CONECTA TU MÓVIL", "CONNECT YOUR PHONE") }).size(11.0).strong().color(MUTED));
        ui.add_space(6.0);
        if option_card(ui, icon!("wifi"), t("Sin cable", "Wireless"), t("Ver y controlar por Wi-Fi", "View and control over Wi-Fi")).clicked() {
            self.screen = Screen::Wifi;
            self.mdns_active.store(true, Ordering::Relaxed);
            *self.wifi.lock().unwrap() = WifiStage::Idle;
            self.pair_code.clear();
        }
        ui.add_space(8.0);
        if option_card(ui, icon!("plug"), t("Con cable", "With cable"), t("Ver y controlar por USB", "View and control over USB")).clicked() {
            self.screen = Screen::Usb;
        }
        ui.add_space(8.0);
        if option_card(ui, icon!("cast"), t("Desde el móvil", "From the phone"), t("Envía la pantalla desde el móvil. Solo ver", "Cast the screen from the phone. View only")).clicked() {
            self.screen = Screen::Cast;
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
            ui.add_space(8.0);
            ui.label(RichText::new(t("Libre, open source y sin límite de tiempo", "Free, open source, no time limits")).size(11.0).color(MUTED));
        });
    }

    fn wifi_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.header(ui, Some(t("Sin cable", "Wireless")));
        ui.add_space(12.0);
        let stage = self.wifi.lock().unwrap().clone();
        if let WifiStage::Done(serial) = &stage {
            let serial = serial.clone();
            *self.wifi.lock().unwrap() = WifiStage::Idle;
            self.mdns_active.store(false, Ordering::Relaxed);
            self.screen = Screen::Home;
            self.start_session(ctx, serial);
            return;
        }

        let services = self.mdns.lock().unwrap().clone();
        let pairing = services.iter().find(|s| s.is_pairing()).cloned();
        let step = if pairing.is_some() || self.manual { 2 } else { 1 };
        steps(ui, step, 3);
        ui.add_space(20.0);

        if step == 1 {
            ui.label(RichText::new(t("Prepara tu móvil", "Get your phone ready")).size(24.0).strong().color(INK));
            ui.add_space(10.0);
            numbered(ui, 1, t("Conecta el móvil a la misma Wi-Fi que este PC.", "Connect the phone to the same Wi-Fi as this PC."));
            numbered(ui, 2, t("Ajustes, Opciones de desarrollador, activa «Depuración inalámbrica».", "Settings, Developer options, turn on “Wireless debugging”."));
            numbered(ui, 3, t("Entra en «Depuración inalámbrica» y pulsa «Vincular dispositivo con código de vinculación».", "Open “Wireless debugging” and tap “Pair device with pairing code”."));
            ui.add_space(18.0);
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(16.0).color(BLUE_BRIGHT));
                ui.label(RichText::new(t("Buscando tu móvil en la red…", "Looking for your phone on the network…")).color(MUTED));
            });
            ui.add_space(14.0);
            ui.label(RichText::new(t("¿No tienes «Opciones de desarrollador»? Ve a Ajustes, Información del teléfono y pulsa 7 veces «Número de compilación».", "No “Developer options”? Go to Settings, About phone and tap “Build number” 7 times.")).size(12.0).color(MUTED));
        } else {
            ui.label(RichText::new(t("Escribe el código que ves en tu móvil", "Type the code shown on your phone")).size(24.0).strong().color(INK));
            ui.add_space(14.0);
            let input = egui::TextEdit::singleline(&mut self.pair_code)
                .hint_text("123456")
                .font(egui::FontId::monospace(30.0))
                .align(egui::Align2::CENTER_CENTER)
                .char_limit(7)
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(12, 14));
            ui.add(input);
            ui.add_space(12.0);
            if let Some(p) = &pairing {
                card(Color32::WHITE, Some(LINE)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        icon_tile(ui, icon!("smartphone"), BLUE, ICE_SOFT);
                        ui.vertical(|ui| {
                            ui.label(RichText::new(t("Móvil encontrado", "Phone found")).strong().color(INK));
                            ui.label(RichText::new(p.ip()).monospace().size(12.0).color(MUTED));
                        });
                    });
                });
            }
        }

        ui.add_space(14.0);
        match &stage {
            WifiStage::Working(m) => {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(16.0).color(BLUE_BRIGHT));
                    ui.label(RichText::new(m).color(MUTED));
                });
            }
            WifiStage::Error(e) => {
                ui.label(RichText::new(e).color(ERROR));
            }
            _ => {}
        }

        ui.add_space(8.0);
        let manual_label = if self.manual { t("Ocultar opciones a mano", "Hide manual options") } else { t("Escribir la IP a mano", "Type the IP manually") };
        if ui.add(egui::Button::new(RichText::new(manual_label).size(13.0).color(BLUE)).frame(false)).clicked() {
            self.manual = !self.manual;
        }
        if self.manual {
            ui.label(RichText::new(t("IP y puerto de vinculación (opcional si ya está vinculado)", "Pairing IP and port (optional if already paired)")).size(12.0).color(MUTED));
            ui.add(egui::TextEdit::singleline(&mut self.manual_pair).hint_text("192.168.1.40:37123").desired_width(f32::INFINITY));
            ui.label(RichText::new(t("IP y puerto para conectar", "IP and port to connect")).size(12.0).color(MUTED));
            ui.add(egui::TextEdit::singleline(&mut self.manual_connect).hint_text("192.168.1.40:41555").desired_width(f32::INFINITY));
        }

        let digits = self.pair_code.chars().filter(|c| c.is_ascii_digit()).count();
        let working = matches!(stage, WifiStage::Working(_));
        let pair_addr = if !self.manual_pair.trim().is_empty() { Some(self.manual_pair.trim().to_string()) } else { pairing.map(|p| p.addr) };
        let can = !working && ((digits == 6 && pair_addr.is_some()) || (self.manual && !self.manual_connect.trim().is_empty()));
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
            ui.add_space(8.0);
            if primary_button(ui, t("Vincular y duplicar", "Pair and mirror"), None).clicked() && can {
                let addr = if digits == 6 { pair_addr } else { None };
                self.start_pairing(addr);
            }
        });
    }

    fn usb_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.header(ui, Some(t("Con cable", "With cable")));
        ui.add_space(16.0);
        ui.label(RichText::new(t("Conecta tu móvil por USB", "Connect your phone over USB")).size(24.0).strong().color(INK));
        ui.add_space(10.0);
        numbered(ui, 1, t("Ajustes, Información del teléfono: pulsa 7 veces «Número de compilación».", "Settings, About phone: tap “Build number” 7 times."));
        numbered(ui, 2, t("Opciones de desarrollador: activa «Depuración USB».", "Developer options: turn on “USB debugging”."));
        numbered(ui, 3, t("Conecta el cable y acepta el aviso del móvil.", "Plug in the cable and accept the prompt on the phone."));
        ui.add_space(18.0);
        let devices = self.devices.lock().unwrap().clone();
        let usb: Vec<_> = devices.iter().filter(|d| !d.is_wireless()).cloned().collect();
        if usb.is_empty() {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(16.0).color(BLUE_BRIGHT));
                ui.label(RichText::new(t("Esperando a tu móvil…", "Waiting for your phone…")).color(MUTED));
            });
            ui.add_space(10.0);
            ui.label(RichText::new(t("Si no aparece, prueba otro cable: algunos solo cargan. En Xiaomi activa también «Depuración USB (ajustes de seguridad)».", "If it does not show up, try another cable: some only charge. On Xiaomi also turn on “USB debugging (Security settings)”.")).size(12.0).color(MUTED));
        }
        let mut start = None;
        for d in &usb {
            card(Color32::WHITE, Some(LINE)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    icon_tile(ui, icon!("smartphone"), BLUE, ICE_SOFT);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(d.label()).strong().color(INK));
                        let st = if d.state == "device" { t("Listo", "Ready") } else { t("Acepta el aviso en el móvil", "Accept the prompt on the phone") };
                        ui.label(RichText::new(st).size(12.0).color(MUTED));
                    });
                });
                if d.state == "device" {
                    ui.add_space(10.0);
                    if primary_button(ui, t("Duplicar", "Mirror"), Some(icon!("play"))).clicked() {
                        start = Some(d.serial.clone());
                    }
                }
            });
            ui.add_space(8.0);
        }
        if let Some(s) = start {
            self.screen = Screen::Home;
            self.start_session(ctx, s);
        }
    }

    fn cast_view(&mut self, ui: &mut egui::Ui) {
        self.header(ui, Some(t("Desde el móvil", "From the phone")));
        ui.add_space(16.0);
        ui.label(RichText::new(t("Envía la pantalla desde el móvil", "Cast the screen from your phone")).size(24.0).strong().color(INK));
        ui.add_space(8.0);
        ui.label(RichText::new(t("Sin tocar ajustes de desarrollador. Podrás ver y oír el móvil, pero no controlarlo desde el PC.", "No developer settings needed. You can see and hear the phone, but not control it from the PC.")).color(MUTED));
        ui.add_space(16.0);
        card(Color32::WHITE, Some(LINE)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                icon_tile(ui, icon!("cast"), BLUE, ICE_SOFT);
                ui.vertical(|ui| {
                    ui.label(RichText::new(t("Enviar pantalla", "Screen cast")).strong().color(INK));
                    ui.label(RichText::new(t("Samsung, Xiaomi, OPPO, Realme y otros: abre los ajustes rápidos, pulsa «Enviar pantalla» y elige este PC.", "Samsung, Xiaomi, OPPO, Realme and others: open quick settings, tap “Screen cast” and pick this PC.")).size(12.0).color(MUTED));
                });
            });
        });
        ui.add_space(8.0);
        card(Color32::WHITE, Some(LINE)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                icon_tile(ui, icon!("smartphone"), BLUE, ICE_SOFT);
                ui.vertical(|ui| {
                    ui.label(RichText::new(t("App Reflex para Android", "Reflex app for Android")).strong().color(INK));
                    ui.label(RichText::new(t("Para móviles sin «Enviar pantalla», como los Pixel: instala la app y pulsa «Enviar al PC».", "For phones without screen cast, like Pixel: install the app and tap “Send to PC”.")).size(12.0).color(MUTED));
                });
            });
        });
        ui.add_space(16.0);
        card(ICE_SOFT, None).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(t("Próximamente. Mientras tanto, usa «Sin cable» o «Con cable», que además te dejan controlar el móvil.", "Coming soon. Meanwhile use “Wireless” or “With cable”, which also let you control the phone.")).size(13.0).color(BLUE));
        });
    }

    fn settings_view(&mut self, ui: &mut egui::Ui) {
        self.header(ui, Some(t("Ajustes", "Settings")));
        ui.add_space(12.0);
        let footer_h = 44.0;
        egui::ScrollArea::vertical().max_height(ui.available_height() - footer_h).show(ui, |ui| {
            ui.set_width(ui.available_width() - 8.0);
            section(ui, t("IDIOMA", "LANGUAGE"));
            let langs = [(t("Automático", "Automatic"), Lang::Auto), ("Español", Lang::Es), ("English", Lang::En)];
            if let Some(v) = segmented(ui, &langs, &self.lang) {
                self.lang = v;
                i18n::set(v);
            }
            ui.add_space(16.0);

            section(ui, t("CALIDAD", "QUALITY"));
            let quality = [(t("Fluida", "Smooth"), 1280u16), (t("Alta", "High"), 1920), (t("Máxima", "Maximum"), 0)];
            if let Some(size) = segmented(ui, &quality, &self.opts.max_size) {
                self.opts.max_size = size;
                self.opts.bit_rate_mbps = match size {
                    1280 => 4,
                    1920 => 8,
                    _ => 16,
                };
            }
            ui.add_space(8.0);
            ui.label(RichText::new(t("Fotogramas por segundo", "Frames per second")).size(12.0).color(MUTED));
            let fps = [("30", 30u32), ("60", 60), ("90", 90), ("120", 120)];
            if let Some(v) = segmented(ui, &fps, &self.opts.max_fps) {
                self.opts.max_fps = v;
            }
            if self.opts.max_fps > 60 || self.opts.max_size == 0 {
                ui.label(RichText::new(t("Por Wi-Fi puede ir a tirones. Recomendado: Alta y 60.", "May stutter over Wi-Fi. Recommended: High and 60.")).size(11.0).color(MUTED));
            }
            ui.add_space(16.0);

            section(ui, t("AL CONECTAR", "WHEN CONNECTING"));
            card(Color32::WHITE, Some(LINE)).inner_margin(Margin::symmetric(16, 4)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                toggle_row(ui, &mut self.opts.audio, t("Sonido del móvil en el PC", "Phone sound on the PC"), Some(t("Deja de sonar en el móvil. Android 11 o superior", "Stops playing on the phone. Android 11 or newer")));
                if self.opts.audio {
                    row_divider(ui);
                    toggle_row(ui, &mut self.opts.audio_dup, t("Oír también en el móvil", "Also play on the phone"), Some(t("Android 13 o superior", "Android 13 or newer")));
                }
                row_divider(ui);
                toggle_row(ui, &mut self.opts.screen_off, t("Apagar la pantalla del móvil", "Turn off the phone's screen"), Some(t("Ahorra batería: la sigues viendo en el PC", "Saves battery: you still see it on the PC")));
                row_divider(ui);
                toggle_row(ui, &mut self.opts.stay_awake, t("Que el móvil no se bloquee", "Keep the phone from locking"), Some(t("Evita que se corte mientras duplicas", "Avoids cutting out while mirroring")));
                row_divider(ui);
                toggle_row(ui, &mut self.opts.show_touches, t("Mostrar los toques en el móvil", "Show touches on the phone"), None);
            });
            ui.add_space(16.0);
            section(ui, "IPHONE");
            ui.label(RichText::new(t("En desarrollo: el iPhone enviará su pantalla con «Duplicar pantalla» (AirPlay). Solo ver.", "In development: the iPhone will send its screen with “Screen Mirroring” (AirPlay). View only.")).size(12.0).color(MUTED));
        });

        // Pie: ayuda y versión.
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Reflex {}", env!("CARGO_PKG_VERSION"))).size(11.0).color(MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let b = egui::Button::image_and_text(egui::Image::new(icon!("file-text")).tint(MUTED).fit_to_exact_size(vec2(14.0, 14.0)), RichText::new(t("Ver registro", "View log")).size(12.0).color(MUTED)).frame(false);
                    if ui.add(b).on_hover_text(t("Lo que ha pasado por dentro, por si algo falla", "What happened under the hood, in case something fails")).clicked() {
                        self.show_log = true;
                    }
                });
            });
            ui.add(egui::Separator::default().spacing(8.0));
        });
    }

    /// Columna de controles flotante, separada del móvil. Se puede esconder.
    fn mirror_tools(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, rect: egui::Rect) {
        let fg = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, ui.scope_id().with("pistas")));
        let hint: std::cell::Cell<Option<(egui::Rect, &'static str)>> = std::cell::Cell::new(None);

        if self.tools_hidden {
            let tab = egui::Rect::from_min_size(pos2(rect.min.x, rect.center().y - 28.0), vec2(28.0, 56.0));
            let resp = ui.interact(tab, ui.scope_id().with("mostrar_controles"), Sense::click());
            let fill = if resp.hovered() { DARK_LINE } else { DARK_RAISED };
            ui.painter().rect_filled(tab, CornerRadius::same(14), fill);
            ui.painter().rect_stroke(tab, CornerRadius::same(14), Stroke::new(1.0, DARK_LINE), egui::StrokeKind::Inside);
            egui::Image::new(icon!("chevron-left")).tint(DARK_TEXT).paint_at(ui, egui::Rect::from_center_size(tab.center(), vec2(16.0, 16.0)));
            if resp.hovered() {
                hint.set(Some((tab, t("Mostrar controles", "Show controls"))));
            }
            if resp.clicked() {
                self.tools_hidden = false;
            }
        } else {
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Center)));
            egui::Frame::new()
                .fill(DARK_RAISED)
                .stroke(Stroke::new(1.0, DARK_LINE))
                .corner_radius(CornerRadius::same(16))
                .inner_margin(Margin::symmetric(4, 8))
                .show(&mut child, |ui| {
                    ui.set_width(rect.width() - 10.0);
                    ui.set_max_height(rect.height() - 18.0);
                    let tool = |ui: &mut egui::Ui, img: egui::ImageSource<'static>, tint: Color32, tip: &'static str| -> bool {
                        let r = icon_button_raw(ui, img, tint);
                        if r.hovered() {
                            hint.set(Some((r.rect, tip)));
                        }
                        r.clicked()
                    };
                    egui::ScrollArea::vertical().scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            // Asa para mover la ventana (no tiene barra de título).
                            let (r, resp) = ui.allocate_exact_size(vec2(40.0, 28.0), Sense::click_and_drag());
                            egui::Image::new(icon!("grip")).tint(DARK_MUTED).paint_at(ui, egui::Rect::from_center_size(r.center(), vec2(18.0, 18.0)));
                            if resp.drag_started() {
                                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                            }
                            if resp.hovered() {
                                hint.set(Some((r, t("Arrastra para mover", "Drag to move"))));
                            }
                            if tool(ui, icon!("chevron-right"), DARK_MUTED, t("Esconder controles", "Hide controls")) {
                                self.tools_hidden = true;
                            }
                            divider(ui);
                            if tool(ui, icon!("back"), DARK_TEXT, t("Atrás (clic derecho)", "Back (right click)")) {
                                self.press(keycode::BACK);
                            }
                            if tool(ui, icon!("home"), DARK_TEXT, t("Inicio (clic central)", "Home (middle click)")) {
                                self.press(keycode::HOME);
                            }
                            if tool(ui, icon!("recents"), DARK_TEXT, t("Apps recientes", "Recent apps")) {
                                self.press(keycode::APP_SWITCH);
                            }
                            divider(ui);
                            if tool(ui, icon!("volume-up"), DARK_TEXT, t("Subir volumen", "Volume up")) {
                                self.press(keycode::VOLUME_UP);
                            }
                            if tool(ui, icon!("volume-down"), DARK_TEXT, t("Bajar volumen", "Volume down")) {
                                self.press(keycode::VOLUME_DOWN);
                            }
                            if tool(ui, icon!("rotate"), DARK_TEXT, t("Girar", "Rotate")) {
                                self.send(ControlMsg::RotateDevice);
                            }
                            let (img, tip) = if self.device_screen_off {
                                (icon!("sun"), t("Encender pantalla del móvil", "Turn on the phone's screen"))
                            } else {
                                (icon!("moon"), t("Apagar pantalla del móvil (sigue duplicando)", "Turn off the phone's screen (keeps mirroring)"))
                            };
                            if tool(ui, img, DARK_TEXT, tip) {
                                self.device_screen_off = !self.device_screen_off;
                                self.send(ControlMsg::SetDisplayPower(!self.device_screen_off));
                            }
                            if tool(ui, icon!("bell"), DARK_TEXT, t("Notificaciones", "Notifications")) {
                                self.send(ControlMsg::ExpandNotifications);
                            }
                            if tool(ui, icon!("power"), DARK_TEXT, t("Bloquear o desbloquear el móvil", "Lock or wake the phone")) {
                                self.press(keycode::POWER);
                            }
                            divider(ui);
                            if tool(ui, icon!("camera"), DARK_TEXT, t("Hacer captura", "Take a screenshot")) {
                                self.screenshot();
                            }
                            let (img, tip) = if self.is_recording() { (icon!("stop"), t("Parar grabación", "Stop recording")) } else { (icon!("record"), t("Grabar la pantalla", "Record the screen")) };
                            if tool(ui, img, REC, tip) {
                                self.toggle_recording();
                            }
                            if tool(ui, icon!("maximize"), DARK_TEXT, t("Pantalla completa (F11)", "Full screen (F11)")) {
                                self.toggle_fullscreen(ctx);
                            }
                            divider(ui);
                            if tool(ui, icon!("minus"), DARK_TEXT, t("Minimizar", "Minimize")) {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                            }
                            if tool(ui, icon!("close"), DARK_TEXT, t("Desconectar", "Disconnect")) {
                                self.stop_session(ctx);
                            }
                            let (r, resp) = ui.allocate_exact_size(vec2(40.0, 28.0), Sense::click_and_drag());
                            egui::Image::new(icon!("resize")).tint(DARK_MUTED).paint_at(ui, egui::Rect::from_center_size(r.center(), vec2(16.0, 16.0)));
                            // Cambio de tamaño propio: crece en diagonal y mantiene la forma del móvil.
                            if resp.dragged() {
                                self.resize_mirror(ctx, resp.drag_delta());
                            }
                            if resp.drag_stopped() {
                                save_mirror_height(self.mirror_height);
                            }
                            if resp.hovered() {
                                hint.set(Some((r, t("Arrastra para cambiar el tamaño", "Drag to resize"))));
                            }
                        });
                    });
                });
        }

        // La pista sale a la izquierda: a la derecha no cabe dentro de la ventana.
        if let Some((r, tip)) = hint.get() {
            let galley = fg.layout_no_wrap(tip.to_string(), egui::FontId::proportional(12.5), DARK_TEXT);
            let size = galley.size() + vec2(20.0, 12.0);
            let bg = egui::Rect::from_min_size(pos2(rect.min.x - 8.0 - size.x, r.center().y - size.y / 2.0), size);
            fg.rect_filled(bg, CornerRadius::same(8), INK);
            fg.rect_stroke(bg, CornerRadius::same(8), Stroke::new(1.0, DARK_LINE), egui::StrokeKind::Inside);
            fg.galley(bg.min + vec2(10.0, 6.0), galley, DARK_TEXT);
        }
    }

    /// Agranda o encoge el móvil en diagonal, manteniendo su forma.
    fn resize_mirror(&mut self, ctx: &egui::Context, d: Vec2) {
        let Some(inner) = ctx.input(|i| i.viewport().inner_rect) else { return };
        let aspect = self.mirror_aspect;
        let screen_w = |h: f32| (h - 2.0 * BEZEL) * aspect + 2.0 * BEZEL;
        let dh = if (d.x / aspect).abs() > d.y.abs() { d.x / aspect } else { d.y };
        let new_h = (inner.height() + dh).clamp(TOOLS_MIN_H.min(480.0), max_phone_height(ctx));
        crate::window::resize(ctx, vec2(screen_w(new_h) + GAP + TOOLS_W, new_h));
        self.mirror_height = if aspect <= 1.0 { new_h } else { (screen_w(new_h) / 1.28).max(480.0) };
    }

    /// La ventana es el propio móvil; los controles flotan a su derecha.
    fn mirror_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(session) = &self.session else { return };

        let image = {
            let f = session.shared.frame.lock().unwrap();
            match (&f.image, f.seq != self.last_seq) {
                (Some(img), true) => {
                    self.last_seq = f.seq;
                    Some(egui::ImageData::from(img.clone()))
                }
                _ => None,
            }
        };
        if let Some(img) = image {
            match &mut self.texture {
                Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                None => self.texture = Some(ctx.load_texture("espejo", img, egui::TextureOptions::LINEAR)),
            }
        }

        let area = ui.max_rect();
        let status = session.status();
        let video_size = *session.shared.video_size.lock().unwrap();
        let (vw, vh) = match (video_size, &self.texture) {
            ((w, _), _) if w > 0 => video_size,
            (_, Some(t)) => (t.size()[0] as u32, t.size()[1] as u32),
            _ => (0, 0),
        };
        // Sin imagen todavía: se dibuja con la forma prevista para que no haya saltos.
        let aspect = if vh > 0 { vw as f32 / vh as f32 } else { self.expected_aspect };
        self.mirror_aspect = aspect;

        // Ajusta la ventana a la forma del móvil o la tablet, en vertical u horizontal.
        // Solo se redimensiona si la forma real no es la prevista (otro móvil o lo has girado).
        if video_size.0 > 0 && video_size != self.sized_for && !self.fullscreen {
            self.sized_for = video_size;
            if (aspect - self.expected_aspect).abs() > 0.01 {
                crate::window::resize(ctx, mirror_window_size(self.mirror_height, aspect));
                self.expected_aspect = aspect;
            }
            save_aspect(&session.serial, aspect);
        }

        if self.fullscreen {
            ui.painter().rect_filled(area, 0.0, Color32::BLACK);
        }

        // Zona del móvil (a la izquierda) y de la columna (a la derecha).
        let tools_h = area.height().min(TOOLS_MAX_H);
        let tools = egui::Rect::from_min_size(pos2(area.max.x - TOOLS_W, area.center().y - tools_h / 2.0), vec2(TOOLS_W, tools_h));
        let region = egui::Rect::from_min_max(area.min, pos2(tools.min.x - GAP, area.max.y));
        let inner_max = region.shrink(BEZEL);
        let scale = (inner_max.width() / aspect).min(inner_max.height());
        let screen_size = vec2(scale * aspect, scale);
        let screen_center = if self.fullscreen { inner_max.center() } else { pos2(inner_max.max.x - screen_size.x / 2.0, inner_max.center().y) };
        let screen = egui::Rect::from_center_size(screen_center, screen_size);
        let phone = screen.expand(BEZEL);
        // Esquina inferior derecha del móvil: se arrastra para cambiar el tamaño.
        let corner = egui::Rect::from_min_max(phone.max - vec2(30.0, 30.0), phone.max);

        let radius = (phone.width().min(phone.height()) * 0.09).clamp(14.0, 40.0);
        ui.painter().rect_filled(phone, CornerRadius::same(radius as u8), DARK_RAISED);
        ui.painter().rect_stroke(phone, CornerRadius::same(radius as u8), Stroke::new(1.0, DARK_LINE), egui::StrokeKind::Inside);
        // El marco del móvil también sirve para arrastrar la ventana.
        let bezel = ui.interact(phone, ui.scope_id().with("marco"), Sense::click_and_drag());
        if bezel.drag_started() && !self.fullscreen && !screen.contains(bezel.interact_pointer_pos().unwrap_or_default()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }

        let inner_radius = CornerRadius::same((radius - BEZEL).max(4.0) as u8);
        let Some(tex) = &self.texture else {
            ui.painter().rect_filled(screen, inner_radius, Color32::BLACK);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(screen).layout(egui::Layout::top_down(egui::Align::Center)));
            child.add_space(screen.height() / 2.0 - 40.0);
            child.add(egui::Spinner::new().size(32.0).color(ICE));
            child.add_space(10.0);
            let msg = match status {
                Status::Starting(m) => m,
                _ => t("Esperando la imagen…", "Waiting for the picture…").into(),
            };
            child.label(RichText::new(msg).color(DARK_MUTED));
            self.mirror_tools(ui, ctx, tools);
            return;
        };
        egui::Image::from_texture((tex.id(), tex.size_vec2())).corner_radius(inner_radius).paint_at(ui, screen);
        if session.shared.screen_dark.load(std::sync::atomic::Ordering::Relaxed) {
            ui.painter().rect_filled(screen, inner_radius, Color32::from_rgb(0x05, 0x07, 0x10));
            let logo = (screen.width() * 0.22).clamp(48.0, 96.0);
            let c = screen.center();
            egui::Image::new(egui::include_image!("../assets/icon-128.png")).paint_at(ui, egui::Rect::from_center_size(c - vec2(0.0, 20.0), vec2(logo, logo)));
            ui.painter().text(c + vec2(0.0, logo / 2.0 + 4.0), egui::Align2::CENTER_TOP, t("Pantalla apagada", "Screen off"), egui::FontId::proportional(15.0), DARK_TEXT);
            ui.painter().text(c + vec2(0.0, logo / 2.0 + 26.0), egui::Align2::CENTER_TOP, t("Toca para encenderla", "Click to wake it"), egui::FontId::proportional(12.0), DARK_MUTED);
        }
        let resp = ui.interact(screen, ui.scope_id().with("pantalla"), Sense::click_and_drag());
        let mut pending_resize: Option<Vec2> = None;
        let mut save_size = false;
        if !self.fullscreen {
            let handle = ui.interact(corner, ui.scope_id().with("esquina"), Sense::drag());
            let k = ctx.animate_bool_with_time(handle.id, handle.hovered() || handle.dragged(), 0.12);
            if k > 0.0 {
                ctx.set_cursor_icon(egui::CursorIcon::ResizeNwSe);
                // Dos trazos en la esquina, como un asa de tamaño.
                let c = phone.max - vec2(radius * 0.32, radius * 0.32);
                let col = ICE.gamma_multiply(k);
                ui.painter().line_segment([c + vec2(-14.0, 0.0), c + vec2(0.0, -14.0)], Stroke::new(2.0, col));
                ui.painter().line_segment([c + vec2(-7.0, 0.0), c + vec2(0.0, -7.0)], Stroke::new(2.0, col));
            }
            if handle.dragged() {
                pending_resize = Some(handle.drag_delta());
            }
            save_size |= handle.drag_stopped();
        }

        let to_device = |p: egui::Pos2| -> (i32, i32) {
            let x = ((p.x - screen.min.x) / screen.width()).clamp(0.0, 1.0) * vw as f32;
            let y = ((p.y - screen.min.y) / screen.height()).clamp(0.0, 1.0) * vh as f32;
            (x as i32, y as i32)
        };
        let touch = |action: u8, p: egui::Pos2| {
            let (x, y) = to_device(p);
            ControlMsg::Touch { action, pointer: POINTER_ID_GENERIC_FINGER, x, y, w: vw as u16, h: vh as u16, pressure: if action == ACTION_UP { 0.0 } else { 1.0 } }
        };

        let mut out: Vec<ControlMsg> = Vec::new();
        let mut toggle_fs = false;
        let mut zoom = 0.0_f32;
        let hover = resp.hovered();
        let events = ctx.input(|i| i.events.clone());
        for ev in events {
            match ev {
                egui::Event::PointerButton { pos, button, pressed, .. } => {
                    let inside = screen.contains(pos) && !corner.contains(pos);
                    match (button, pressed) {
                        (egui::PointerButton::Primary, true) if inside => {
                            self.touching = true;
                            self.mirror_focus = true;
                            out.push(touch(ACTION_DOWN, pos));
                        }
                        (egui::PointerButton::Primary, true) if !tools.contains(pos) => self.mirror_focus = false,
                        (egui::PointerButton::Primary, false) if self.touching => {
                            self.touching = false;
                            out.push(touch(ACTION_UP, pos));
                        }
                        (egui::PointerButton::Secondary, true) if inside => {
                            out.push(ControlMsg::BackOrScreenOn { action: ACTION_DOWN });
                            out.push(ControlMsg::BackOrScreenOn { action: ACTION_UP });
                        }
                        (egui::PointerButton::Middle, true) if inside => out.extend(ControlMsg::key_press(keycode::HOME, 0)),
                        _ => {}
                    }
                }
                egui::Event::PointerMoved(pos) if self.touching => out.push(touch(ACTION_MOVE, pos)),
                egui::Event::PointerGone if self.touching => {
                    self.touching = false;
                    if let Some(p) = ctx.input(|i| i.pointer.latest_pos()) {
                        out.push(touch(ACTION_UP, p));
                    }
                }
                egui::Event::MouseWheel { delta, modifiers, .. } if modifiers.ctrl => zoom += delta.y.signum(),
                egui::Event::MouseWheel { unit, delta, .. } if hover => {
                    let per = match unit {
                        egui::MouseWheelUnit::Point => 1.0 / 50.0,
                        egui::MouseWheelUnit::Line => 1.0,
                        egui::MouseWheelUnit::Page => 10.0,
                    };
                    if let Some(p) = ctx.input(|i| i.pointer.hover_pos()) {
                        let (x, y) = to_device(p);
                        out.push(ControlMsg::Scroll { x, y, w: vw as u16, h: vh as u16, hscroll: delta.x * per, vscroll: delta.y * per });
                    }
                }
                egui::Event::Key { key: egui::Key::F11, pressed: true, .. } => toggle_fs = true,
                egui::Event::Key { key: egui::Key::Escape, pressed: true, .. } if self.fullscreen => toggle_fs = true,
                egui::Event::Text(t) if self.mirror_focus => out.push(ControlMsg::Text(t)),
                egui::Event::Paste(t) if self.mirror_focus => {
                    self.clipboard_seq += 1;
                    out.push(ControlMsg::SetClipboard { sequence: self.clipboard_seq, paste: true, text: t });
                }
                egui::Event::Copy if self.mirror_focus => out.push(ControlMsg::GetClipboard { copy_key: 1 }),
                egui::Event::Cut if self.mirror_focus => out.push(ControlMsg::GetClipboard { copy_key: 2 }),
                egui::Event::Key { key, pressed, repeat, modifiers, .. } if self.mirror_focus => {
                    if let Some((code, meta)) = map_key(key, modifiers) {
                        let action = if pressed { ACTION_DOWN } else { ACTION_UP };
                        out.push(ControlMsg::Key { action, keycode: code, repeat: repeat as u32, meta });
                    }
                }
                _ => {}
            }
        }
        for m in out {
            session.send(m);
        }
        // Ctrl + rueda: agranda o encoge el móvil.
        if zoom != 0.0 && !self.fullscreen
            && let Some(inner) = ctx.input(|i| i.viewport().inner_rect)
        {
            let factor = if zoom > 0.0 { 1.08 } else { 1.0 / 1.08 };
            let tools = GAP + TOOLS_W;
            let phone = vec2(inner.width() - tools, inner.height()) * factor;
            if phone.y >= 320.0 && phone.y <= max_phone_height(ctx) {
                crate::window::resize(ctx, vec2(phone.x + tools, phone.y));
                self.mirror_height = if aspect <= 1.0 { phone.y } else { (phone.x / 1.28).max(480.0) };
                save_mirror_height(self.mirror_height);
            }
        }

        if self.is_recording() {
            let p = pos2(screen.min.x + 14.0, screen.max.y - 20.0);
            ui.painter().rect_filled(egui::Rect::from_min_size(p - vec2(6.0, 11.0), vec2(92.0, 22.0)), CornerRadius::same(11), Color32::from_black_alpha(170));
            ui.painter().circle_filled(p + vec2(5.0, 0.0), 4.5, REC);
            ui.painter().text(p + vec2(15.0, 0.0), egui::Align2::LEFT_CENTER, t("Grabando", "Recording"), egui::FontId::proportional(12.0), DARK_TEXT);
        }

        self.mirror_tools(ui, ctx, tools);
        if toggle_fs {
            self.toggle_fullscreen(ctx);
        }
        if let Some(d) = pending_resize {
            self.resize_mirror(ctx, d);
        }
        if save_size {
            save_mirror_height(self.mirror_height);
        }
    }

    fn log_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_log;
        egui::Window::new(t("Registro", "Log")).open(&mut open).default_size([400.0, 320.0]).show(ctx, |ui| {
            egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                for line in logs::snapshot() {
                    ui.monospace(line);
                }
            });
        });
        self.show_log = open;
    }
}

// ---------------------------------------------------------------- piezas

fn mirror_size_path() -> std::path::PathBuf {
    paths::data_dir().join("tamano_movil.txt")
}

/// Preferencias de la ventana de duplicado: `alto=…` y `aspecto.<serial>=…`, una por línea.
fn read_mirror_prefs() -> Vec<(String, String)> {
    let text = std::fs::read_to_string(mirror_size_path()).unwrap_or_default();
    text.lines()
        .filter_map(|l| match l.split_once('=') {
            Some((k, v)) => Some((k.trim().to_string(), v.trim().to_string())),
            // Formato antiguo: solo el alto.
            None if !l.trim().is_empty() => Some(("alto".to_string(), l.trim().to_string())),
            None => None,
        })
        .collect()
}

fn write_mirror_pref(key: &str, value: String) {
    let mut prefs = read_mirror_prefs();
    match prefs.iter_mut().find(|(k, _)| k == key) {
        Some(p) => p.1 = value,
        None => prefs.push((key.to_string(), value)),
    }
    let text: String = prefs.iter().map(|(k, v)| format!("{k}={v}
")).collect();
    let _ = std::fs::create_dir_all(paths::data_dir());
    let _ = std::fs::write(mirror_size_path(), text);
}

fn read_mirror_pref(key: &str) -> Option<f32> {
    read_mirror_prefs().into_iter().find(|(k, _)| k == key).and_then(|(_, v)| v.parse().ok())
}

fn load_mirror_height() -> f32 {
    read_mirror_pref("alto").filter(|h| (320.0..4000.0).contains(h)).unwrap_or(WINDOW.y)
}

fn save_mirror_height(h: f32) {
    write_mirror_pref("alto", format!("{h:.0}"));
}

fn load_aspect(serial: &str) -> Option<f32> {
    read_mirror_pref(&format!("aspecto.{serial}")).filter(|a| (0.2..5.0).contains(a))
}

fn save_aspect(serial: &str, aspect: f32) {
    write_mirror_pref(&format!("aspecto.{serial}"), format!("{aspect:.4}"));
}

/// Alto máximo del móvil: lo que cabe en la pantalla, con un margen.
fn max_phone_height(ctx: &egui::Context) -> f32 {
    ctx.input(|i| i.viewport().monitor_size).map(|m| (m.y - 60.0).max(480.0)).unwrap_or(1000.0)
}

/// Tamaño de la ventana de duplicado: el móvil con su marco más la columna de controles.
fn mirror_window_size(height: f32, aspect: f32) -> Vec2 {
    let phone = if aspect <= 1.0 {
        vec2((height - 2.0 * BEZEL) * aspect + 2.0 * BEZEL, height)
    } else {
        let w = height * 1.28;
        vec2(w, (w - 2.0 * BEZEL) / aspect + 2.0 * BEZEL)
    };
    vec2(phone.x + GAP + TOOLS_W, phone.y.max(TOOLS_MIN_H))
}

fn apply_theme(ctx: &egui::Context) {
    let mut v = egui::Visuals::light();
    v.panel_fill = SURFACE;
    v.window_fill = Color32::WHITE;
    v.override_text_color = Some(INK);
    v.hyperlink_color = BLUE;
    v.selection.bg_fill = BLUE_BRIGHT;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.extreme_bg_color = Color32::WHITE;
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = CornerRadius::same(10);
    }
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.inactive.weak_bg_fill = Color32::WHITE;
    v.widgets.hovered.weak_bg_fill = ICE_SOFT;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, ICE);
    v.widgets.active.weak_bg_fill = ICE_SOFT;
    ctx.set_visuals(v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = vec2(8.0, 6.0);
        s.spacing.button_padding = vec2(12.0, 8.0);
    });
}

fn card(fill: Color32, stroke: Option<Color32>) -> egui::Frame {
    egui::Frame::new()
        .fill(fill)
        .stroke(stroke.map(|c| Stroke::new(1.0, c)).unwrap_or(Stroke::NONE))
        .corner_radius(CornerRadius::same(18))
        .inner_margin(Margin::same(16))
}

fn icon_button(ui: &mut egui::Ui, src: egui::ImageSource<'static>, tint: Color32, tip: &str) -> egui::Response {
    icon_button_raw(ui, src, tint).on_hover_text(tip)
}

/// Botón de icono con fondo suave al pasar el ratón y al pulsar.
fn icon_button_raw(ui: &mut egui::Ui, src: egui::ImageSource<'static>, tint: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::click());
    let dark = tint.r() > 0x90 && tint.g() > 0x90;
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    if hover > 0.0 {
        let base = if dark { DARK_LINE } else { ICE_SOFT };
        let fill = if resp.is_pointer_button_down_on() { if dark { Color32::from_rgb(0x33, 0x3B, 0x5C) } else { Color32::from_rgb(0xDC, 0xE6, 0xFF) } } else { base };
        ui.painter().rect_filled(rect.shrink(2.0), CornerRadius::same(10), fill.gamma_multiply(hover));
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let icon_tint = if !dark && resp.hovered() { BLUE } else { tint };
    egui::Image::new(src).tint(icon_tint).paint_at(ui, egui::Rect::from_center_size(rect.center(), vec2(20.0, 20.0)));
    resp
}

fn icon_tile(ui: &mut egui::Ui, src: egui::ImageSource<'static>, tint: Color32, bg: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(12), bg);
    egui::Image::new(src).tint(tint).paint_at(ui, egui::Rect::from_center_size(rect.center(), vec2(22.0, 22.0)));
}

fn option_card(ui: &mut egui::Ui, src: egui::ImageSource<'static>, title: &str, subtitle: &str) -> egui::Response {
    let r = card(Color32::WHITE, Some(LINE))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                icon_tile(ui, src, BLUE, ICE_SOFT);
                ui.vertical(|ui| {
                    ui.label(RichText::new(title).size(15.0).strong().color(INK));
                    ui.label(RichText::new(subtitle).size(12.0).color(MUTED));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(egui::Image::new(icon!("chevron-right")).tint(MUTED).fit_to_exact_size(vec2(18.0, 18.0)));
                });
            });
        })
        .response
        .interact(Sense::click());
    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        ui.painter().rect_stroke(r.rect, CornerRadius::same(18), Stroke::new(1.5, ICE), egui::StrokeKind::Inside);
    }
    r
}

fn primary_button(ui: &mut egui::Ui, label: &str, icon: Option<egui::ImageSource<'static>>) -> egui::Response {
    let text = RichText::new(label).strong().size(15.0).color(Color32::WHITE);
    let b = match icon {
        Some(src) => egui::Button::image_and_text(egui::Image::new(src).tint(Color32::WHITE).fit_to_exact_size(vec2(16.0, 16.0)), text),
        None => egui::Button::new(text),
    };
    let r = ui.add(b.fill(BLUE).corner_radius(CornerRadius::same(12)).min_size(vec2(ui.available_width(), 50.0)));
    hover_overlay(ui, &r, 12, Color32::from_white_alpha(28));
    r
}

/// Aclara u oscurece un botón con color propio al pasar el ratón (egui no lo hace si tiene relleno fijo).
fn hover_overlay(ui: &egui::Ui, r: &egui::Response, radius: u8, color: Color32) {
    let k = ui.ctx().animate_bool_with_time(r.id.with("hover"), r.hovered(), 0.12);
    if k > 0.0 {
        ui.painter().rect_filled(r.rect, CornerRadius::same(radius), color.gamma_multiply(k));
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
}

fn steps(ui: &mut egui::Ui, current: usize, total: usize) {
    let w = (ui.available_width() - 6.0 * (total as f32 - 1.0)) / total as f32;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for i in 1..=total {
            let (rect, _) = ui.allocate_exact_size(vec2(w, 4.0), Sense::hover());
            ui.painter().rect_filled(rect, CornerRadius::same(2), if i <= current { BLUE } else { LINE });
        }
    });
}

fn numbered(ui: &mut egui::Ui, n: usize, text: &str) {
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 12.0, ICE_SOFT);
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, n.to_string(), egui::FontId::proportional(12.0), BLUE);
        ui.add(egui::Label::new(RichText::new(text).color(INK)).wrap());
    });
    ui.add_space(6.0);
}

fn divider(ui: &mut egui::Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(28.0, 9.0), Sense::hover());
    ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0, DARK_LINE));
}

/// Botones segmentados del mismo ancho; devuelve la opción pulsada.
fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, options: &[(&str, T)], current: &T) -> Option<T> {
    let mut picked = None;
    let gap = 6.0;
    let w = (ui.available_width() - gap * (options.len() as f32 - 1.0)) / options.len() as f32;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for (label, value) in options {
            let selected = value == current;
            let b = egui::Button::new(RichText::new(*label).color(if selected { Color32::WHITE } else { INK }))
                .fill(if selected { BLUE } else { Color32::WHITE })
                .stroke(Stroke::new(1.0, if selected { BLUE } else { LINE }))
                .corner_radius(CornerRadius::same(10))
                .min_size(vec2(w, 38.0));
            let r = ui.add(b);
            if !selected {
                hover_overlay(ui, &r, 10, Color32::from_rgba_unmultiplied(0x24, 0x47, 0xE0, 22));
            }
            if r.clicked() {
                picked = Some(*value);
            }
        }
    });
    picked
}

/// Interruptor deslizante.
fn toggle(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(42.0, 24.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let k = ui.ctx().animate_bool_responsive(resp.id, *on);
    let bg = if *on { BLUE_BRIGHT } else { LINE };
    ui.painter().rect_filled(rect, CornerRadius::same(12), bg);
    let x = egui::lerp((rect.left() + 12.0)..=(rect.right() - 12.0), k);
    ui.painter().circle_filled(pos2(x, rect.center().y), 9.0, Color32::WHITE);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

fn toggle_row(ui: &mut egui::Ui, value: &mut bool, label: &str, help: Option<&str>) {
    ui.horizontal(|ui| {
        ui.set_min_height(48.0);
        ui.vertical(|ui| {
            ui.add_space(if help.is_some() { 6.0 } else { 14.0 });
            ui.label(RichText::new(label).color(INK));
            if let Some(h) = help {
                ui.label(RichText::new(h).size(11.0).color(MUTED));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            toggle(ui, value);
        });
    });
}

fn row_divider(ui: &mut egui::Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0, LINE));
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.label(RichText::new(title).size(11.0).strong().color(MUTED));
    ui.add_space(4.0);
}

fn map_key(key: egui::Key, m: egui::Modifiers) -> Option<(u32, u32)> {
    use egui::Key as K;
    let meta = if m.ctrl { META_CTRL_ON } else { 0 } | if m.shift { crate::android::protocol::META_SHIFT_ON } else { 0 };
    let code = match key {
        K::Enter => keycode::ENTER,
        K::Backspace => keycode::DEL,
        K::Delete => keycode::FORWARD_DEL,
        K::Tab => keycode::TAB,
        K::Escape => keycode::BACK,
        K::ArrowUp => keycode::DPAD_UP,
        K::ArrowDown => keycode::DPAD_DOWN,
        K::ArrowLeft => keycode::DPAD_LEFT,
        K::ArrowRight => keycode::DPAD_RIGHT,
        K::Home => keycode::MOVE_HOME,
        K::End => keycode::MOVE_END,
        K::PageUp => keycode::PAGE_UP,
        K::PageDown => keycode::PAGE_DOWN,
        _ if m.ctrl => {
            let name = key.name();
            let c = name.chars().next()?;
            if name.len() != 1 || !c.is_ascii_alphabetic() || matches!(c, 'C' | 'V' | 'X') {
                return None;
            }
            keycode::A + (c as u32 - 'A' as u32)
        }
        _ => return None,
    };
    Some((code, meta))
}

impl eframe::App for ReflexApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        if self.adb.is_none() && self.setup.lock().unwrap().done {
            self.refresh_adb(&ctx);
        }

        let mut ended = None;
        if let Some(s) = &self.session {
            for n in s.shared.notices.lock().unwrap().drain(..) {
                self.notice = Some((n, Instant::now()));
            }
            if let Some(text) = s.shared.clipboard_from_device.lock().unwrap().take() {
                ctx.copy_text(text);
            }
            if let Status::Ended(reason) = s.status() {
                ended = Some(reason);
            }
        }
        if let Some(reason) = ended {
            self.stop_session(&ctx);
            if let Some(r) = reason {
                self.toast(format!("{}: {r}", t("Se cortó la conexión", "Connection lost")));
            }
        }

        let title = match &self.session {
            Some(s) => {
                let n = s.shared.device_name.lock().unwrap().clone();
                if n.is_empty() { "Reflex".to_string() } else { format!("{n} - Reflex") }
            }
            None => "Reflex".to_string(),
        };
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }

        if self.session.is_some() {
            egui::CentralPanel::no_frame().frame(egui::Frame::new().fill(Color32::TRANSPARENT)).show(ui, |ui| self.mirror_view(ui, &ctx));
        } else {
            let notice = self.notice.clone().filter(|(_, at)| at.elapsed() < Duration::from_secs(6));
            // Ventana sin marco de Windows: esquinas redondeadas y barra propia.
            let frame = egui::Frame::new()
                .fill(SURFACE)
                .stroke(Stroke::new(1.0, LINE))
                .corner_radius(CornerRadius::same(14))
                .inner_margin(Margin { left: 20, right: 12, top: 6, bottom: 16 });
            egui::CentralPanel::no_frame().frame(frame).show(ui, |ui| {
                let full = ui.max_rect();
                if !self.components_ready() {
                    self.setup_view(ui);
                } else {
                    match self.screen {
                        Screen::Home => self.home_view(ui, &ctx),
                        Screen::Wifi => self.wifi_view(ui, &ctx),
                        Screen::Usb => self.usb_view(ui, &ctx),
                        Screen::Cast => self.cast_view(ui),
                        Screen::Settings => self.settings_view(ui),
                    }
                }
                if let Some((msg, _)) = notice {
                    let toast = egui::Rect::from_min_max(pos2(full.min.x, full.max.y - 64.0), pos2(full.max.x - 8.0, full.max.y));
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(toast));
                    egui::Frame::new().fill(INK).corner_radius(CornerRadius::same(12)).inner_margin(Margin::symmetric(14, 10)).show(&mut child, |ui| {
                        ui.set_width(toast.width() - 28.0);
                        ui.add(egui::Label::new(RichText::new(msg).size(12.0).color(DARK_TEXT)).wrap());
                    });
                }
                // Asa para cambiar el tamaño de la ventana.
                let grip = egui::Rect::from_min_size(pos2(full.max.x - 6.0, full.max.y + 2.0), vec2(14.0, 14.0));
                let resp = ui.interact(grip, ui.scope_id().with("redimensionar"), Sense::drag());
                if resp.hovered() {
                    ctx.set_cursor_icon(egui::CursorIcon::ResizeNwSe);
                }
                if resp.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(egui::viewport::ResizeDirection::SouthEast));
                }
            });
        }

        if self.show_log {
            self.log_window(&ctx);
        }
        if self.notice.as_ref().is_some_and(|(_, at)| at.elapsed() < Duration::from_secs(7))
            || self.task_busy.load(Ordering::SeqCst)
            || matches!(*self.wifi.lock().unwrap(), WifiStage::Working(_))
        {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    /// Fondo transparente: al duplicar solo se ven el móvil y la columna de controles.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(s) = self.session.take() {
            s.stop();
        }
    }
}
