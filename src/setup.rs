//! Descarga de componentes la primera vez: adb (platform-tools de Google) y el
//! servidor de scrcpy (Apache 2.0) que corre en el móvil.

use std::io::{Cursor, Read};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, bail};
use sha2::{Digest, Sha256};

use crate::logs::log;
use crate::paths;

#[derive(Default)]
pub struct SetupState {
    pub running: bool,
    pub message: String,
    pub progress: f32,
    pub error: Option<String>,
    pub done: bool,
}

pub fn needs_adb() -> bool {
    paths::find_adb().is_none()
}

pub fn needs_server() -> bool {
    !paths::server_path().is_file()
}

pub fn start(state: Arc<Mutex<SetupState>>, ctx: eframe::egui::Context) {
    {
        let mut s = state.lock().unwrap();
        *s = SetupState { running: true, message: "Preparando…".into(), ..Default::default() };
    }
    std::thread::spawn(move || {
        let result = run(&state, &ctx);
        let mut s = state.lock().unwrap();
        s.running = false;
        match result {
            Ok(()) => {
                s.done = true;
                s.message = "Componentes listos".into();
                log("Componentes descargados correctamente");
            }
            Err(e) => {
                log(format!("Error descargando componentes: {e:#}"));
                s.error = Some(format!("{e:#}"));
            }
        }
        ctx.request_repaint();
    });
}

fn run(state: &Arc<Mutex<SetupState>>, ctx: &eframe::egui::Context) -> Result<()> {
    std::fs::create_dir_all(paths::components_dir())?;

    if needs_server() {
        set(state, ctx, "Descargando servidor de scrcpy…", 0.0);
        let data = download(paths::SCRCPY_SERVER_URL, |p| set(state, ctx, "Descargando servidor de scrcpy…", p))?;
        let hash = Sha256::digest(&data);
        let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
        if hex != paths::SCRCPY_SERVER_SHA256 {
            bail!("la suma SHA-256 del servidor no coincide (esperada {}, obtenida {hex})", paths::SCRCPY_SERVER_SHA256);
        }
        std::fs::write(paths::server_path(), &data)?;
    }

    if needs_adb() {
        set(state, ctx, "Descargando Android platform-tools (adb)…", 0.0);
        let data = download(paths::platform_tools_url(), |p| set(state, ctx, "Descargando Android platform-tools (adb)…", p))?;
        set(state, ctx, "Descomprimiendo platform-tools…", 1.0);
        let mut zip = zip::ZipArchive::new(Cursor::new(data)).context("zip de platform-tools inválido")?;
        zip.extract(paths::components_dir()).context("no se pudo descomprimir platform-tools")?;
        if !paths::bundled_adb().is_file() {
            bail!("adb no aparece tras descomprimir platform-tools");
        }
    }
    Ok(())
}

fn set(state: &Arc<Mutex<SetupState>>, ctx: &eframe::egui::Context, msg: &str, progress: f32) {
    let mut s = state.lock().unwrap();
    s.message = msg.to_string();
    s.progress = progress;
    ctx.request_repaint();
}

fn download(url: &str, mut on_progress: impl FnMut(f32)) -> Result<Vec<u8>> {
    log(format!("Descargando {url}"));
    let mut resp = ureq::get(url).call().with_context(|| format!("no se pudo descargar {url}"))?;
    let total: Option<u64> = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok());
    let mut reader = resp.body_mut().as_reader();
    let mut out = Vec::with_capacity(total.unwrap_or(1 << 20) as usize);
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        if let Some(t) = total {
            on_progress(out.len() as f32 / t as f32);
        }
    }
    Ok(out)
}
