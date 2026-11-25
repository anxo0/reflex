//! Interfaz gráfica (egui).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText, Sense, Stroke, StrokeKind, pos2};

use crate::adb::{Adb, Device};
use crate::android::protocol::{ACTION_DOWN, ACTION_MOVE, ACTION_UP, META_CTRL_ON, POINTER_ID_GENERIC_FINGER};
use crate::android::{ControlMsg, Options, Session, Status, keycode};
use crate::logs::{self, log};
use crate::recorder::Recorder;
use crate::{paths, setup};

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Android,
    Iphone,
}

pub struct ReflexApp {
    tab: Tab,
    adb: Option<Adb>,
    setup: Arc<Mutex<setup::SetupState>>,
    devices: Arc<Mutex<Vec<Device>>>,
    poller_started: bool,
    session: Option<Session>,
    texture: Option<egui::TextureHandle>,
    last_seq: u64,
    opts: Options,
    connect_addr: String,
    pair_addr: String,
    pair_code: String,
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
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let mut app = Self {
            tab: Tab::Android,
            adb: None,
            setup: Arc::new(Mutex::new(setup::SetupState::default())),
            devices: Arc::new(Mutex::new(Vec::new())),
            poller_started: false,
            session: None,
            texture: None,
            last_seq: 0,
            opts: Options::default(),
            connect_addr: String::new(),
            pair_addr: String::new(),
            pair_code: String::new(),
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
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                if let Err(e) = adb.start_server() {
                    log(format!("No se pudo iniciar adb: {e:#}"));
                }
                loop {
                    match adb.devices() {
                        Ok(list) => {
                            let mut d = devices.lock().unwrap();
                            if *d != list {
                                *d = list;
                                ctx.request_repaint();
                            }
                        }
                        Err(e) => log(format!("adb devices: {e:#}")),
                    }
                    std::thread::sleep(Duration::from_secs(2));
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

    /// Lanza una operación de adb en segundo plano (Wi-Fi, emparejar…).
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
        self.texture = None;
        self.last_seq = 0;
        self.device_screen_off = self.opts.screen_off;
        self.session = Some(Session::start(adb, serial, self.opts.clone(), ctx.clone()));
    }

    fn stop_session(&mut self, ctx: &egui::Context) {
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
    }

    fn screenshot(&mut self) {
        let Some(s) = &self.session else { return };
        let (rgba, w, h) = {
            let f = s.shared.frame.lock().unwrap();
            (f.rgba.clone(), f.width as u32, f.height as u32)
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
            Ok(()) => self.toast(format!("Captura guardada en {}", path.display())),
            Err(e) => self.toast(format!("No se pudo guardar la captura: {e:#}")),
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
            // Pide un fotograma clave nuevo para empezar a grabar ya.
            s.send(ControlMsg::ResetVideo);
            self.toast("Grabando… pulsa otra vez para parar");
        }
    }

    fn toggle_fullscreen(&mut self, ctx: &egui::Context) {
        self.fullscreen = !self.fullscreen;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
    }

    // ---------------------------------------------------------------- vistas

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(RichText::new("Reflex").strong());
            ui.add_space(16.0);
            ui.selectable_value(&mut self.tab, Tab::Android, "Android");
            ui.selectable_value(&mut self.tab, Tab::Iphone, "iPhone");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.toggle_value(&mut self.show_log, "Registro");
            });
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(s) = &self.session {
                let name = s.shared.device_name.lock().unwrap().clone();
                let (w, h) = *s.shared.video_size.lock().unwrap();
                let fps = *s.shared.fps.lock().unwrap();
                ui.label(format!("{name}  ·  {w}x{h}  ·  {fps:.0} fps"));
                if self.is_recording() {
                    ui.colored_label(Color32::from_rgb(230, 60, 60), "● Grabando");
                }
            } else {
                ui.label("Sin conexión");
            }
            if let Some((msg, at)) = &self.notice
                && at.elapsed() < Duration::from_secs(6) {
                    ui.separator();
                    ui.label(msg);
                }
        });
    }

    fn setup_view(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(60.0);
            ui.heading("Bienvenido a Reflex");
            ui.add_space(8.0);
            ui.label("Para duplicar tu Android hacen falta dos componentes libres:");
            ui.label("• adb (Android platform-tools, de Google)");
            ui.label(format!("• el servidor de scrcpy v{} (Apache 2.0), que se ejecuta en el móvil", paths::SCRCPY_VERSION));
            ui.add_space(12.0);
            let st = self.setup.lock().unwrap();
            let running = st.running;
            if running {
                ui.label(&st.message);
                ui.add(egui::ProgressBar::new(st.progress).show_percentage().desired_width(360.0));
            }
            if let Some(e) = &st.error {
                ui.colored_label(Color32::from_rgb(230, 90, 90), format!("Error: {e}"));
            }
            drop(st);
            if !running && ui.button(RichText::new("Descargar componentes (~15 MB)").size(16.0)).clicked() {
                setup::start(self.setup.clone(), ui.ctx().clone());
            }
            ui.add_space(8.0);
            ui.small(format!("Se guardan en {}", paths::components_dir().display()));
        });
    }

    fn devices_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Dispositivos");
        ui.add_space(4.0);
        let devices = self.devices.lock().unwrap().clone();
        if devices.is_empty() {
            ui.label("No hay ningún Android conectado.");
        }
        let mut connect_to = None;
        for d in &devices {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(if d.is_wireless() { "📶" } else { "🔌" });
                    ui.strong(d.label());
                });
                ui.small(&d.serial);
                match d.state.as_str() {
                    "device" => {
                        ui.horizontal(|ui| {
                            let active = self.session.as_ref().is_some_and(|s| s.serial == d.serial);
                            if !active && ui.button("▶ Duplicar").clicked() {
                                connect_to = Some(d.serial.clone());
                            }
                            if !d.is_wireless() {
                                let busy = self.task_busy.load(Ordering::SeqCst);
                                if ui.add_enabled(!busy, egui::Button::new("Pasar a Wi-Fi")).on_hover_text("Activa adb por Wi-Fi para desconectar el cable").clicked() {
                                    let serial = d.serial.clone();
                                    self.background("Activando Wi-Fi", move |adb| adb.switch_to_wifi(&serial).map(|m| format!("{m}. Ya puedes quitar el cable")));
                                }
                            } else if ui.button("Desconectar").clicked() {
                                let serial = d.serial.clone();
                                self.background("Desconectando", move |adb| adb.disconnect(&serial).map(|_| "Desconectado".into()));
                            }
                        });
                    }
                    "unauthorized" => {
                        ui.colored_label(Color32::YELLOW, "Acepta el aviso «¿Permitir depuración USB?» en el móvil");
                    }
                    other => {
                        ui.colored_label(Color32::GRAY, format!("Estado: {other}"));
                    }
                }
            });
        }
        if let Some(serial) = connect_to {
            let ctx = ui.ctx().clone();
            self.stop_session(&ctx);
            self.start_session(&ctx, serial);
        }

        ui.add_space(10.0);
        ui.collapsing("Conexión inalámbrica", |ui| {
            let busy = self.task_busy.load(Ordering::SeqCst);
            ui.label("Conectar por IP (IP:puerto):");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.connect_addr).hint_text("192.168.1.40:5555").desired_width(150.0));
                if ui.add_enabled(!busy && !self.connect_addr.trim().is_empty(), egui::Button::new("Conectar")).clicked() {
                    let addr = self.connect_addr.trim().to_string();
                    self.background("Conectando", move |adb| adb.connect(&addr));
                }
            });
            ui.add_space(6.0);
            ui.label("Emparejar (Android 11+, «Depuración inalámbrica» → «Vincular con código»):");
            ui.add(egui::TextEdit::singleline(&mut self.pair_addr).hint_text("IP:puerto de vinculación").desired_width(200.0));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.pair_code).hint_text("Código de 6 cifras").desired_width(120.0));
                if ui.add_enabled(!busy && !self.pair_addr.trim().is_empty() && !self.pair_code.trim().is_empty(), egui::Button::new("Emparejar")).clicked() {
                    let addr = self.pair_addr.trim().to_string();
                    let code = self.pair_code.trim().to_string();
                    self.background("Emparejando", move |adb| {
                        adb.pair(&addr, &code).map(|m| format!("{m}. Ahora conecta con la IP:puerto que aparece en «Depuración inalámbrica»"))
                    });
                }
            });
        });
        if let Some((ok, msg)) = self.task_msg.lock().unwrap().as_ref() {
            ui.add_space(4.0);
            let color = if *ok { Color32::LIGHT_GREEN } else { Color32::from_rgb(230, 90, 90) };
            ui.colored_label(color, msg);
        }

        ui.add_space(10.0);
        ui.collapsing("Ajustes de duplicado", |ui| {
            let sizes: [(u16, &str); 5] = [(0, "Original"), (2560, "2560"), (1920, "1920 (recomendado)"), (1280, "1280"), (1024, "1024")];
            let current = sizes.iter().find(|(v, _)| *v == self.opts.max_size).map(|(_, l)| *l).unwrap_or("Personalizada");
            egui::ComboBox::from_label("Resolución máx.").selected_text(current).show_ui(ui, |ui| {
                for (v, l) in sizes {
                    ui.selectable_value(&mut self.opts.max_size, v, l);
                }
            });
            egui::ComboBox::from_label("FPS máx.").selected_text(self.opts.max_fps.to_string()).show_ui(ui, |ui| {
                for v in [30, 60, 90, 120] {
                    ui.selectable_value(&mut self.opts.max_fps, v, v.to_string());
                }
            });
            ui.add(egui::Slider::new(&mut self.opts.bit_rate_mbps, 1..=32).text("Mbps"));
            ui.checkbox(&mut self.opts.audio, "Audio del móvil (Android 11+)");
            ui.checkbox(&mut self.opts.stay_awake, "Mantener el móvil despierto");
            ui.checkbox(&mut self.opts.screen_off, "Apagar la pantalla del móvil al conectar");
            ui.checkbox(&mut self.opts.show_touches, "Mostrar toques en el móvil");
            ui.small("Los cambios se aplican en la siguiente conexión.");
        });
    }

    fn idle_view(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(80.0);
            ui.heading("Conecta tu Android");
            ui.add_space(10.0);
            ui.label("1. En el móvil: Ajustes → Información del teléfono → pulsa 7 veces «Número de compilación».");
            ui.label("2. Ajustes → Opciones de desarrollador → activa «Depuración USB».");
            ui.label("3. Conecta el cable USB y acepta el aviso en el móvil.");
            ui.label("4. Pulsa «▶ Duplicar» en la lista de la izquierda.");
            ui.add_space(10.0);
            ui.label("Sin cable: conéctalo una vez por USB y pulsa «Pasar a Wi-Fi», o usa «Emparejar» con la depuración inalámbrica.");
        });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let w = ui.available_width();
        let b = |ui: &mut egui::Ui, text: &str, tip: &str| ui.add_sized([w, 30.0], egui::Button::new(text)).on_hover_text(tip).clicked();
        ui.add_space(6.0);
        if b(ui, "◀  Atrás", "Atrás (clic derecho en la pantalla)") {
            self.press(keycode::BACK);
        }
        if b(ui, "⌂  Inicio", "Inicio (clic central)") {
            self.press(keycode::HOME);
        }
        if b(ui, "▤  Recientes", "Aplicaciones recientes") {
            self.press(keycode::APP_SWITCH);
        }
        ui.separator();
        if b(ui, "🔊  Vol +", "Subir volumen") {
            self.press(keycode::VOLUME_UP);
        }
        if b(ui, "🔉  Vol −", "Bajar volumen") {
            self.press(keycode::VOLUME_DOWN);
        }
        if b(ui, "⏻  Encender", "Botón de encendido") {
            self.press(keycode::POWER);
        }
        ui.separator();
        if b(ui, "⟳  Girar", "Girar el móvil") {
            self.send(ControlMsg::RotateDevice);
        }
        if b(ui, "🔔  Notificaciones", "Desplegar notificaciones") {
            self.send(ControlMsg::ExpandNotifications);
        }
        let screen_label = if self.device_screen_off { "☀  Encender pantalla" } else { "🌙  Apagar pantalla" };
        if b(ui, screen_label, "Apaga la pantalla del móvil sin dejar de duplicar") {
            self.device_screen_off = !self.device_screen_off;
            self.send(ControlMsg::SetDisplayPower(!self.device_screen_off));
        }
        ui.separator();
        if b(ui, "📷  Captura", "Guardar captura en PNG") {
            self.screenshot();
        }
        let rec = if self.is_recording() { "⏹  Parar grabación" } else { "⏺  Grabar" };
        if b(ui, rec, "Grabar la pantalla en MP4") {
            self.toggle_recording();
        }
        if b(ui, "⛶  Pantalla completa", "Pantalla completa (F11, Esc para salir)") {
            self.toggle_fullscreen(ctx);
        }
        ui.separator();
        if b(ui, "✖  Desconectar", "Terminar el duplicado") {
            self.stop_session(ctx);
        }
    }

    fn mirror_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(session) = &self.session else { return };

        // Sube el fotograma nuevo a la GPU.
        let image = {
            let f = session.shared.frame.lock().unwrap();
            (f.seq != self.last_seq && f.width > 0).then(|| {
                self.last_seq = f.seq;
                egui::ColorImage::from_rgba_premultiplied([f.width, f.height], &f.rgba)
            })
        };
        if let Some(img) = image {
            match &mut self.texture {
                Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                None => self.texture = Some(ctx.load_texture("espejo", img, egui::TextureOptions::LINEAR)),
            }
        }

        let status = session.status();
        let Some(tex) = &self.texture else {
            ui.centered_and_justified(|ui| {
                let msg = match status {
                    Status::Starting(m) => m,
                    _ => "Esperando vídeo…".into(),
                };
                ui.add(egui::Spinner::new().size(32.0));
                ui.label(msg);
            });
            return;
        };

        let avail = ui.available_rect_before_wrap().shrink(if self.fullscreen { 0.0 } else { 12.0 });
        let tex_size = tex.size_vec2();
        let scale = (avail.width() / tex_size.x).min(avail.height() / tex_size.y).max(0.01);
        let rect = egui::Rect::from_center_size(avail.center(), tex_size * scale);
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        if !self.fullscreen {
            ui.painter().rect_stroke(rect.expand(4.0), 14.0, Stroke::new(8.0, Color32::from_gray(24)), StrokeKind::Outside);
        }
        ui.painter().image(tex.id(), rect, egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);

        let (vw, vh) = {
            let (w, h) = *session.shared.video_size.lock().unwrap();
            if w > 0 { (w, h) } else { (tex_size.x as u32, tex_size.y as u32) }
        };
        let to_device = |p: egui::Pos2| -> (i32, i32) {
            let x = ((p.x - rect.min.x) / rect.width()).clamp(0.0, 1.0) * vw as f32;
            let y = ((p.y - rect.min.y) / rect.height()).clamp(0.0, 1.0) * vh as f32;
            (x as i32, y as i32)
        };
        let touch = |action: u8, p: egui::Pos2| {
            let (x, y) = to_device(p);
            ControlMsg::Touch {
                action,
                pointer: POINTER_ID_GENERIC_FINGER,
                x,
                y,
                w: vw as u16,
                h: vh as u16,
                pressure: if action == ACTION_UP { 0.0 } else { 1.0 },
            }
        };

        let mut out: Vec<ControlMsg> = Vec::new();
        let mut toggle_fs = false;
        let hover = resp.hovered();
        let events = ctx.input(|i| i.events.clone());
        for ev in events {
            match ev {
                egui::Event::PointerButton { pos, button, pressed, .. } => {
                    let inside = rect.contains(pos);
                    match (button, pressed) {
                        (egui::PointerButton::Primary, true) if inside => {
                            self.touching = true;
                            self.mirror_focus = true;
                            out.push(touch(ACTION_DOWN, pos));
                        }
                        (egui::PointerButton::Primary, true) => self.mirror_focus = false,
                        (egui::PointerButton::Primary, false) if self.touching => {
                            self.touching = false;
                            out.push(touch(ACTION_UP, pos));
                        }
                        (egui::PointerButton::Secondary, true) if inside => {
                            out.push(ControlMsg::BackOrScreenOn { action: ACTION_DOWN });
                            out.push(ControlMsg::BackOrScreenOn { action: ACTION_UP });
                        }
                        (egui::PointerButton::Middle, true) if inside => {
                            out.extend(ControlMsg::key_press(keycode::HOME, 0));
                        }
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
        if toggle_fs {
            self.toggle_fullscreen(ctx);
        }
        if self.mirror_focus && !self.fullscreen {
            ui.painter().rect_stroke(rect.expand(9.0), 16.0, Stroke::new(1.5, Color32::from_rgb(70, 130, 220)), StrokeKind::Outside);
        }
    }

    fn iphone_view(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(50.0);
                ui.heading("iPhone / iPad (AirPlay)");
                ui.add_space(8.0);
                ui.colored_label(Color32::YELLOW, "En desarrollo: fase 2 del proyecto");
                ui.add_space(12.0);
                ui.label("El iPhone no permite capturar la pantalla por cable en Windows. La vía es la misma que usan");
                ui.label("las apps de este tipo: Reflex se anunciará en la red como receptor AirPlay y el iPhone le");
                ui.label("enviará su pantalla con «Centro de control → Duplicar pantalla».");
                ui.add_space(12.0);
                ui.label("Hoja de ruta en docs/IPHONE.md.");
            });
        });
    }

    fn log_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_log;
        egui::Window::new("Registro").open(&mut open).default_size([640.0, 320.0]).show(ctx, |ui| {
            egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                for line in logs::snapshot() {
                    ui.monospace(line);
                }
            });
        });
        self.show_log = open;
    }
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
            // Ctrl+letra (Ctrl+A, Ctrl+Z…). Copiar, cortar y pegar llegan como eventos propios.
            let name = key.name();
            let c = name.chars().next()?;
            if name.len() != 1 || !c.is_ascii_alphabetic() || matches!(c, 'C' | 'V' | 'X') {
                return None;
            }
            keycode::A + (c as u32 - 'A' as u32)
        }
        _ => return None,
    };
    // El texto normal llega como Event::Text; aquí solo van teclas especiales.
    Some((code, meta))
}

impl eframe::App for ReflexApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        if self.adb.is_none() && self.setup.lock().unwrap().done {
            self.refresh_adb(&ctx);
        }

        // Fin de sesión, avisos y portapapeles del móvil.
        let mut ended_error = None;
        if let Some(s) = &self.session {
            for n in s.shared.notices.lock().unwrap().drain(..) {
                self.notice = Some((n, Instant::now()));
            }
            if let Some(text) = s.shared.clipboard_from_device.lock().unwrap().take() {
                ctx.copy_text(text);
            }
            if let Status::Ended(reason) = s.status() {
                ended_error = Some(reason);
            }
        }
        if let Some(reason) = ended_error {
            self.stop_session(&ctx);
            if let Some(r) = reason {
                self.toast(format!("Conexión terminada: {r}"));
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

        let mirroring = self.session.is_some() && self.tab == Tab::Android;

        if !self.fullscreen {
            egui::Panel::top("barra").show(ui, |ui| {
                ui.add_space(4.0);
                self.top_bar(ui);
                ui.add_space(2.0);
            });
            egui::Panel::bottom("estado").show(ui, |ui| self.status_bar(ui));
        }

        match self.tab {
            Tab::Iphone => {
                egui::CentralPanel::default_margins().show(ui, |ui| self.iphone_view(ui));
            }
            Tab::Android if !self.components_ready() => {
                egui::CentralPanel::default_margins().show(ui, |ui| self.setup_view(ui));
            }
            Tab::Android => {
                if !self.fullscreen {
                    egui::Panel::left("dispositivos").resizable(true).default_size(300.0).show(ui, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| self.devices_panel(ui));
                    });
                    if mirroring {
                        egui::Panel::right("herramientas").resizable(false).exact_size(170.0).show(ui, |ui| self.toolbar(ui, &ctx));
                    }
                }
                let frame = egui::Frame::new().fill(Color32::from_gray(12));
                egui::CentralPanel::no_frame().frame(frame).show(ui, |ui| {
                    if self.session.is_some() {
                        self.mirror_view(ui, &ctx);
                    } else {
                        self.idle_view(ui);
                    }
                });
            }
        }

        if self.show_log {
            self.log_window(&ctx);
        }
        if self.notice.as_ref().is_some_and(|(_, at)| at.elapsed() < Duration::from_secs(7)) {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        if self.task_busy.load(Ordering::SeqCst) {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn on_exit(&mut self) {
        if let Some(s) = self.session.take() {
            s.stop();
        }
    }
}
