//! Registro de lo que pasa por dentro. Se guarda en `%LOCALAPPDATA%\Reflex\logs\reflex.log`
//! (el de la sesión anterior queda en `reflex.anterior.log`), para poder abrirlo o enviarlo
//! cuando algo falla.

use std::fs::File;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Mutex;

static FILE: Mutex<Option<File>> = Mutex::new(None);

pub fn path() -> PathBuf {
    crate::paths::data_dir().join("logs").join("reflex.log")
}

/// Empieza un archivo nuevo para esta sesión y guarda el anterior.
pub fn init() {
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::rename(&p, p.with_file_name("reflex.anterior.log"));
    if let Ok(f) = File::create(&p) {
        *FILE.lock().unwrap() = Some(f);
    }
    log(format!("Reflex {} en {}", env!("CARGO_PKG_VERSION"), std::env::consts::OS));
}

pub fn log(msg: impl AsRef<str>) {
    let line = format!("[{}] {}", chrono::Local::now().format("%H:%M:%S"), msg.as_ref());
    eprintln!("{line}");
    if let Ok(mut f) = FILE.lock()
        && let Some(f) = f.as_mut()
    {
        let _ = writeln!(f, "{line}");
    }
}

/// Abre la carpeta del registro con el archivo seleccionado.
pub fn reveal() {
    let p = path();
    #[cfg(windows)]
    {
        let mut cmd = std::process::Command::new("explorer.exe");
        cmd.arg(format!("/select,{}", p.display()));
        let _ = cmd.spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg("-R").arg(&p).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(dir) = p.parent() {
            let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
        }
    }
}
