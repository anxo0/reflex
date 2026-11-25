//! Receptor de «Enviar pantalla» (Miracast): el móvil envía su pantalla al PC sin cables
//! ni ajustes de desarrollador. Solo ver (y oír): Miracast no permite controlar el móvil.
//!
//! Usa la API de Windows `Windows.Media.Miracast`. Windows recibe el vídeo y el sonido;
//! el sonido suena directamente por los altavoces del PC y cada fotograma de vídeo lo
//! copiamos de la GPU a la memoria para pintarlo con egui, igual que con Android.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui;

use crate::android::session::FrameSlot;
use crate::i18n::t;
use crate::logs::log;

#[derive(Clone, PartialEq, Debug)]
pub enum CastStatus {
    Starting,
    /// Esperando al móvil. Lleva el nombre con el que aparece el PC.
    Listening(String),
    /// Hay un móvil enviando su pantalla.
    Connected(String),
    /// Falta la función opcional «Pantalla inalámbrica» de Windows.
    NeedsFeature,
    /// El PC tiene Wi-Fi pero está apagado: Miracast lo necesita aunque internet vaya por cable.
    WifiOff,
    /// El PC no tiene tarjeta Wi-Fi: no puede recibir «Enviar pantalla».
    NoWifi,
    Failed(String),
}

pub struct CastShared {
    pub status: Mutex<CastStatus>,
    pub frame: Mutex<FrameSlot>,
    pub screen_dark: AtomicBool,
}

pub struct Receiver {
    pub shared: Arc<CastShared>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Nombre con el que el PC aparece en la lista del móvil.
pub fn friendly_name() -> String {
    let pc = std::env::var("COMPUTERNAME").unwrap_or_default();
    if pc.is_empty() { "Reflex".to_string() } else { format!("Reflex · {pc}") }
}

impl Receiver {
    pub fn start(ctx: egui::Context) -> Self {
        let shared = Arc::new(CastShared {
            status: Mutex::new(CastStatus::Starting),
            frame: Mutex::new(FrameSlot { image: None, width: 0, height: 0, seq: 0 }),
            screen_dark: AtomicBool::new(false),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let shared = shared.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                let r = imp::run(&shared, &stop, &ctx);
                if let Err(e) = r {
                    log(format!("Enviar pantalla: {e:#}"));
                    let mut st = shared.status.lock().unwrap();
                    if !matches!(*st, CastStatus::NeedsFeature | CastStatus::WifiOff | CastStatus::NoWifi) {
                        *st = CastStatus::Failed(format!("{e:#}"));
                    }
                }
                ctx.request_repaint();
            })
        };
        Self { shared, stop, thread: Some(thread) }
    }

    pub fn status(&self) -> CastStatus {
        self.shared.status.lock().unwrap().clone()
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Instala «Pantalla inalámbrica» con DISM. Windows pide permiso de administrador.
/// Bloquea hasta que termina; devuelve si salió bien.
pub fn install_feature() -> bool {
    #[cfg(windows)]
    {
        let mut cmd = std::process::Command::new("powershell.exe");
        cmd.args([
            "-NoProfile",
            "-Command",
            "$p = Start-Process dism.exe -ArgumentList '/Online','/Add-Capability','/CapabilityName:App.WirelessDisplay.Connect~~~~0.0.1.0','/NoRestart' -Verb RunAs -Wait -PassThru -WindowStyle Hidden; exit $p.ExitCode",
        ]);
        crate::adb::hide_window(&mut cmd);
        match cmd.status() {
            Ok(s) => {
                log(format!("Instalación de Pantalla inalámbrica: {s}"));
                let ok = s.success() || s.code() == Some(3010);
                if ok {
                    let _ = std::fs::write(installed_marker(), chrono::Local::now().to_rfc3339());
                }
                ok
            }
            Err(e) => {
                log(format!("No se pudo instalar Pantalla inalámbrica: {e}"));
                false
            }
        }
    }
    #[cfg(not(windows))]
    false
}

fn installed_marker() -> std::path::PathBuf {
    crate::paths::data_dir().join("pantalla_inalambrica_instalada.txt")
}

/// Ya se instaló desde Reflex en este arranque de Windows o antes: si aún no funciona,
/// lo que falta es reiniciar el PC.
pub fn installed_needs_restart() -> bool {
    installed_marker().exists() && !restarted_since_install()
}

/// Se instaló y ya se ha reiniciado, pero Windows sigue sin dejar recibir: el problema
/// es el Wi-Fi del PC o su controlador, no la instalación.
pub fn installed_but_unsupported() -> bool {
    installed_marker().exists() && restarted_since_install()
}

fn restarted_since_install() -> bool {
    let Ok(installed) = std::fs::metadata(installed_marker()).and_then(|m| m.modified()) else { return false };
    #[cfg(windows)]
    {
        // SAFETY: función sin parámetros que devuelve los milisegundos desde que arrancó Windows.
        let up = std::time::Duration::from_millis(unsafe { windows::Win32::System::SystemInformation::GetTickCount64() });
        let boot = std::time::SystemTime::now() - up;
        installed < boot
    }
    #[cfg(not(windows))]
    {
        let _ = installed;
        false
    }
}

/// Abre los ajustes de Wi-Fi de Windows.
pub fn open_wifi_settings() {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer.exe").arg("ms-settings:network-wifi").spawn();
    }
}

/// Abre «Proyectar en este PC» en Configuración de Windows.
pub fn open_projection_settings() {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer.exe").arg("ms-settings:project").spawn();
    }
}

/// Abre los ajustes de Windows donde se instala «Pantalla inalámbrica».
pub fn open_optional_features() {
    #[cfg(windows)]
    {
        let mut cmd = std::process::Command::new("explorer.exe");
        cmd.arg("ms-settings:optionalfeatures");
        let _ = cmd.spawn();
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::ApplicationModel::Core::CoreApplicationView;
    use windows::Foundation::TypedEventHandler;
    use windows::Graphics::DirectX::Direct3D11::IDirect3DSurface;
    use windows::Media::Miracast::*;
    use windows::Media::Playback::{IMediaPlaybackSource, MediaPlayer};
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
    use windows::Win32::Graphics::Direct3D11::*;
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
    use windows::Win32::Graphics::Dxgi::IDXGISurface;
    use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11SurfaceFromDXGISurface;
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
    use windows::core::{HSTRING, Interface};

    pub fn run(shared: &Arc<CastShared>, stop: &AtomicBool, ctx: &egui::Context) -> anyhow::Result<()> {
        // SAFETY: inicializa WinRT en este hilo, que es nuestro y vive todo el receptor.
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        let receiver = MiracastReceiver::new()?;
        let name = super::friendly_name();
        let settings = receiver.GetDefaultSettings()?;
        settings.SetFriendlyName(&HSTRING::from(name.as_str()))?;
        settings.SetModelName(&HSTRING::from("Reflex"))?;
        settings.SetModelNumber(&HSTRING::from(env!("CARGO_PKG_VERSION")))?;
        settings.SetAuthorizationMethod(MiracastReceiverAuthorizationMethod::None)?;
        settings.SetRequireAuthorizationFromKnownTransmitters(false)?;
        let applied = receiver.DisconnectAllAndApplySettings(&settings)?.Status()?;
        log(format!("Enviar pantalla: ajustes aplicados ({applied:?})"));
        if applied == MiracastReceiverApplySettingsStatus::MiracastNotSupported {
            // Antes de pedir que instalen nada, mira si el problema es el Wi-Fi.
            let st = match wifi_on() {
                None => CastStatus::NoWifi,
                Some(false) => CastStatus::WifiOff,
                Some(true) => CastStatus::NeedsFeature,
            };
            log(format!("Enviar pantalla: no compatible ahora mismo ({st:?})"));
            *shared.status.lock().unwrap() = st.clone();
            if st != CastStatus::NeedsFeature {
                return Ok(());
            }
            anyhow::bail!("{}", t("Falta «Pantalla inalámbrica» en Windows", "Windows is missing “Wireless Display”"));
        }
        if applied != MiracastReceiverApplySettingsStatus::Success {
            anyhow::bail!("{} ({applied:?})", t("Windows no aceptó los ajustes", "Windows rejected the settings"));
        }

        let session = receiver.CreateSession(None::<&CoreApplicationView>)?;
        session.SetAllowConnectionTakeover(true)?;
        session.SetMaxSimultaneousConnections(1)?;

        let player: Arc<Mutex<Option<MediaPlayer>>> = Arc::new(Mutex::new(None));
        let copier = Arc::new(Mutex::new(Copier::new()?));

        {
            let shared = shared.clone();
            let ctx = ctx.clone();
            session.ConnectionCreated(&TypedEventHandler::<MiracastReceiverSession, MiracastReceiverConnectionCreatedEventArgs>::new(move |_, args| {
                let who = args
                    .as_ref()
                    .and_then(|a| a.Connection().ok())
                    .and_then(|c| c.Transmitter().ok())
                    .and_then(|tx| tx.Name().ok())
                    .map(|n| n.to_string())
                    .unwrap_or_default();
                log(format!("Enviar pantalla: conectado {who}"));
                *shared.status.lock().unwrap() = CastStatus::Connected(if who.is_empty() { t("Tu móvil", "Your phone").into() } else { who });
                ctx.request_repaint();
                Ok(())
            }))?;
        }
        {
            let shared = shared.clone();
            let ctx = ctx.clone();
            let player = player.clone();
            let copier = copier.clone();
            session.MediaSourceCreated(&TypedEventHandler::<MiracastReceiverSession, MiracastReceiverMediaSourceCreatedEventArgs>::new(move |_, args| {
                let Some(args) = args.as_ref() else { return Ok(()) };
                let source = args.MediaSource()?;
                let p = MediaPlayer::new()?;
                p.SetRealTimePlayback(true)?;
                p.SetIsVideoFrameServerEnabled(true)?;
                {
                    let shared = shared.clone();
                    let ctx = ctx.clone();
                    let copier = copier.clone();
                    p.VideoFrameAvailable(&TypedEventHandler::new(move |sender: windows::core::Ref<MediaPlayer>, _| {
                        if let Some(sender) = sender.as_ref()
                            && let Err(e) = copier.lock().unwrap().grab(sender, &shared)
                        {
                            log(format!("Enviar pantalla: fotograma perdido: {e}"));
                        }
                        ctx.request_repaint();
                        Ok(())
                    }))?;
                }
                p.SetSource(&source.cast::<IMediaPlaybackSource>()?)?;
                p.Play()?;
                log("Enviar pantalla: reproduciendo");
                *player.lock().unwrap() = Some(p);
                Ok(())
            }))?;
        }
        {
            let shared = shared.clone();
            let ctx = ctx.clone();
            let player = player.clone();
            let name = name.clone();
            session.Disconnected(&TypedEventHandler::<MiracastReceiverSession, MiracastReceiverDisconnectedEventArgs>::new(move |_, _| {
                log("Enviar pantalla: el móvil se desconectó");
                if let Some(p) = player.lock().unwrap().take() {
                    let _ = p.Close();
                }
                *shared.frame.lock().unwrap() = FrameSlot { image: None, width: 0, height: 0, seq: 0 };
                *shared.status.lock().unwrap() = CastStatus::Listening(name.clone());
                ctx.request_repaint();
                Ok(())
            }))?;
        }

        let started = session.StartAsync()?.join()?.Status()?;
        log(format!("Enviar pantalla: sesión ({started:?})"));
        match started {
            MiracastReceiverSessionStartStatus::Success => {}
            MiracastReceiverSessionStartStatus::MiracastNotSupported => {
                *shared.status.lock().unwrap() = CastStatus::NeedsFeature;
                anyhow::bail!("Miracast no disponible");
            }
            MiracastReceiverSessionStartStatus::AccessDenied => {
                anyhow::bail!("{}", t("Windows no dejó recibir pantallas. Revisa Ajustes, Sistema, Proyectar en este PC.", "Windows did not allow receiving screens. Check Settings, System, Projecting to this PC."))
            }
            other => anyhow::bail!("{} ({other:?})", t("No se pudo empezar a recibir", "Could not start receiving")),
        }
        *shared.status.lock().unwrap() = CastStatus::Listening(name);
        ctx.request_repaint();

        while !stop.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
        if let Some(p) = player.lock().unwrap().take() {
            let _ = p.Close();
        }
        let _ = session.Close();
        log("Enviar pantalla: receptor parado");
        Ok(())
    }

    /// Ancho de las franjas negras a cada lado (se suponen simétricas: si una app oscura toca
    /// un borde, el otro lado lo corrige) y alto de las de arriba y abajo.
    fn black_bars(px: &[egui::Color32], w: usize, h: usize) -> (usize, usize) {
        let black = |p: &egui::Color32| p.r() < 14 && p.g() < 14 && p.b() < 14;
        let col_black = |x: usize| (1..16).all(|i| black(&px[(h * i / 16) * w + x]));
        let row_black = |y: usize| (1..16).all(|i| black(&px[y * w + w * i / 16]));
        let left = (0..w / 2).take_while(|&x| col_black(x)).count();
        let right = (0..w / 2).take_while(|&x| col_black(w - 1 - x)).count();
        let top = (0..h / 2).take_while(|&y| row_black(y)).count();
        let bottom = (0..h / 2).take_while(|&y| row_black(h - 1 - y)).count();
        let bx = left.min(right);
        let by = top.min(bottom);
        // Pantalla casi entera negra o franjas diminutas: no se recorta.
        let bx = if bx < 4 || w - 2 * bx < w / 6 { 0 } else { bx };
        let by = if by < 4 || h - 2 * by < h / 6 { 0 } else { by };
        (bx, by)
    }

    /// ¿Está encendido el Wi-Fi del PC? `None` si no tiene tarjeta Wi-Fi.
    fn wifi_on() -> Option<bool> {
        use windows::Devices::Radios::{Radio, RadioKind, RadioState};
        let radios = Radio::GetRadiosAsync().ok()?.join().ok()?;
        let mut found = None;
        for i in 0..radios.Size().ok()? {
            let Ok(r) = radios.GetAt(i) else { continue };
            if r.Kind().ok() == Some(RadioKind::WiFi) {
                let on = r.State().ok() == Some(RadioState::On);
                found = Some(found.unwrap_or(false) || on);
            }
        }
        found
    }

    /// Copia cada fotograma de la GPU a la memoria.
    struct Copier {
        device: ID3D11Device,
        context: ID3D11DeviceContext,
        target: Option<Target>,
        /// Franjas negras a recortar (izquierda/derecha, arriba/abajo). Los móviles envían su
        /// pantalla vertical dentro de un vídeo apaisado con franjas a los lados.
        bars: (usize, usize),
        frames: u64,
    }

    struct Target {
        width: u32,
        height: u32,
        surface: IDirect3DSurface,
        texture: ID3D11Texture2D,
        staging: ID3D11Texture2D,
    }

    // SAFETY: el dispositivo y el contexto solo se usan con el Mutex que envuelve al Copier.
    unsafe impl Send for Copier {}

    impl Copier {
        fn new() -> anyhow::Result<Self> {
            let mut device = None;
            let mut context = None;
            // SAFETY: llamada estándar de Direct3D con punteros a variables locales.
            unsafe {
                D3D11CreateDevice(
                    None,
                    D3D_DRIVER_TYPE_HARDWARE,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut context),
                )?;
            }
            Ok(Self { device: device.ok_or_else(|| anyhow::anyhow!("sin Direct3D"))?, context: context.ok_or_else(|| anyhow::anyhow!("sin Direct3D"))?, target: None, bars: (0, 0), frames: 0 })
        }

        fn ensure(&mut self, width: u32, height: u32) -> anyhow::Result<()> {
            if self.target.as_ref().is_some_and(|t| t.width == width && t.height == height) {
                return Ok(());
            }
            let mut desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let mut texture = None;
            let mut staging = None;
            // SAFETY: descripciones válidas y punteros a variables locales.
            unsafe {
                self.device.CreateTexture2D(&desc, None, Some(&mut texture))?;
                desc.Usage = D3D11_USAGE_STAGING;
                desc.BindFlags = 0;
                desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
                self.device.CreateTexture2D(&desc, None, Some(&mut staging))?;
            }
            let texture = texture.ok_or_else(|| anyhow::anyhow!("sin textura"))?;
            let dxgi: IDXGISurface = texture.cast()?;
            // SAFETY: la superficie DXGI es de una textura viva.
            let surface: IDirect3DSurface = unsafe { CreateDirect3D11SurfaceFromDXGISurface(&dxgi)? }.cast()?;
            self.target = Some(Target { width, height, surface, texture, staging: staging.ok_or_else(|| anyhow::anyhow!("sin textura"))? });
            Ok(())
        }

        fn grab(&mut self, player: &MediaPlayer, shared: &CastShared) -> anyhow::Result<()> {
            let ps = player.PlaybackSession()?;
            let (w, h) = (ps.NaturalVideoWidth()?, ps.NaturalVideoHeight()?);
            if w == 0 || h == 0 {
                return Ok(());
            }
            self.ensure(w, h)?;
            let t = self.target.as_ref().unwrap();
            player.CopyFrameToVideoSurface(&t.surface)?;
            let mut pixels = vec![egui::Color32::BLACK; (w * h) as usize];
            // SAFETY: copiamos a la textura de lectura y la mapeamos solo mientras leemos;
            // cada fila tiene `RowPitch` bytes y al menos `w * 4` válidos.
            unsafe {
                self.context.CopyResource(&t.staging, &t.texture);
                let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                self.context.Map(&t.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
                let base = mapped.pData as *const u8;
                for y in 0..h as usize {
                    let row = std::slice::from_raw_parts(base.add(y * mapped.RowPitch as usize), w as usize * 4);
                    let out = &mut pixels[y * w as usize..(y + 1) * w as usize];
                    for (o, px) in out.iter_mut().zip(row.chunks_exact(4)) {
                        *o = egui::Color32::from_rgb(px[2], px[1], px[0]);
                    }
                }
                self.context.Unmap(&t.staging, 0);
            }
            let dark = pixels.iter().step_by(97).all(|p| p.r() < 10 && p.g() < 10 && p.b() < 10);
            shared.screen_dark.store(dark, Ordering::Relaxed);
            let (w, h) = (w as usize, h as usize);
            // Cada medio segundo, más o menos, vuelve a medir las franjas (si el móvil gira, cambian).
            if !dark && self.frames % 15 == 0 {
                self.bars = black_bars(&pixels, w, h);
            }
            self.frames += 1;
            let (bx, by) = self.bars;
            let (cw, ch) = (w - 2 * bx, h - 2 * by);
            let pixels = if bx == 0 && by == 0 {
                pixels
            } else {
                let mut out = Vec::with_capacity(cw * ch);
                for y in by..by + ch {
                    out.extend_from_slice(&pixels[y * w + bx..y * w + bx + cw]);
                }
                out
            };
            let image = egui::ColorImage::new([cw, ch], pixels);
            let mut f = shared.frame.lock().unwrap();
            f.image = Some(Arc::new(image));
            f.width = cw;
            f.height = ch;
            f.seq += 1;
            Ok(())
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    pub fn run(shared: &Arc<CastShared>, _stop: &AtomicBool, _ctx: &egui::Context) -> anyhow::Result<()> {
        *shared.status.lock().unwrap() = CastStatus::Failed(t("Por ahora solo en Windows", "Windows only for now").into());
        Ok(())
    }
}
