//! iPhone / iPad: receptor AirPlay de duplicado de pantalla.
//!
//! Reflex se anuncia en la red como un Apple TV y el iPhone le envía su pantalla
//! desde *Centro de control → Duplicar pantalla*. Solo se puede ver (y grabar): ni
//! AirPlay ni ninguna API pública de Apple permiten controlar el iPhone.

mod aac;
mod audio;
pub mod fairplay;
mod identity;
mod mdns;
mod mirror;
mod ntp;
mod pairing;
mod rtsp;
mod sps;

use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result};
use eframe::egui;

use crate::android::session::Shared;
use crate::logs::log;

/// Puerto habitual de AirPlay. Si está ocupado se usa otro: el iPhone lo lee del anuncio.
const PORT: u16 = 7000;

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Starting,
    /// Anunciado en la red, esperando a que un iPhone se conecte.
    Waiting,
    /// Recibiendo vídeo del dispositivo con este nombre.
    Mirroring(String),
    Failed(String),
}

/// Alto de la pantalla que se anuncia. El iPhone ajusta su imagen a esta altura
/// (un iPhone en vertical llega a unos 1000×2160). Más resolución no se puede
/// decodificar con fluidez en CPU.
const DISPLAY_HEIGHT: u32 = 2160;

pub struct Receiver {
    pub shared: Arc<Shared>,
    pub name: String,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    ctx: Arc<Mutex<Option<Arc<rtsp::Context>>>>,
}

impl Receiver {
    pub fn start(name: String, egui_ctx: egui::Context) -> Receiver {
        let display_height = Arc::new(AtomicU32::new(DISPLAY_HEIGHT));
        let shared = Arc::new(Shared::new());
        let state = Arc::new(Mutex::new(State::Starting));
        let stop = Arc::new(AtomicBool::new(false));
        let ctx_slot = Arc::new(Mutex::new(None));
        let r = Receiver {
            shared: shared.clone(),
            name: name.clone(),
            state: state.clone(),
            stop: stop.clone(),
            ctx: ctx_slot.clone(),
        };

        std::thread::spawn(move || {
            let result = (|| -> Result<()> {
                let identity = identity::Identity::load_or_create()?;
                let listener = TcpListener::bind(("0.0.0.0", PORT))
                    .or_else(|_| TcpListener::bind(("0.0.0.0", 0)))
                    .context("no se pudo abrir el puerto de AirPlay")?;
                let port = listener.local_addr()?.port();
                let advertiser = mdns::Advertiser::start(&identity, &name, port)?;
                log(format!("AirPlay: «{name}» anunciado en el puerto {port} (id {})", identity.device_id()));

                let ctx = Arc::new(rtsp::Context {
                    identity,
                    name,
                    display_height,
                    shared,
                    state: state.clone(),
                    stop: stop.clone(),
                    egui: egui_ctx.clone(),
                    mirror: Mutex::new(None),
                    volume: Arc::new(AtomicU32::new(1f32.to_bits())),
                    generation: AtomicU64::new(0),
                });
                *ctx_slot.lock().unwrap() = Some(ctx.clone());
                ctx.set_state(State::Waiting);
                rtsp::serve(listener, ctx.clone());
                ctx.stop_mirror();
                drop(advertiser);
                Ok(())
            })();
            if let Err(e) = result {
                log(format!("AirPlay: {e:#}"));
                *state.lock().unwrap() = State::Failed(format!("{e:#}"));
                egui_ctx.request_repaint();
            }
        });
        r
    }

    pub fn state(&self) -> State {
        self.state.lock().unwrap().clone()
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(ctx) = self.ctx.lock().unwrap().as_ref() {
            ctx.stop_mirror();
        }
        crate::android::session::finish_recording(&self.shared);
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.stop();
    }
}
