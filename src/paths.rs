//! Rutas de componentes externos y carpetas de salida.

use std::path::PathBuf;

/// Versión del servidor de scrcpy que se ejecuta en el Android. El protocolo
/// depende de la versión exacta, así que va fijada.
pub const SCRCPY_VERSION: &str = "5.0.1";
pub const SCRCPY_SERVER_URL: &str =
    "https://github.com/Genymobile/scrcpy/releases/download/v5.0.1/scrcpy-server-v5.0.1";
/// SHA-256 publicado en SHA256SUMS.txt de la release v5.0.1.
pub const SCRCPY_SERVER_SHA256: &str =
    "764eb6f79811d5211fe9df341120882ba9994c7a61b897d7bf3fb662e53bc536";

/// Ruta del servidor dentro del dispositivo.
pub const DEVICE_SERVER_PATH: &str = "/data/local/tmp/reflex-server.jar";

pub fn platform_tools_url() -> &'static str {
    if cfg!(target_os = "windows") {
        "https://dl.google.com/android/repository/platform-tools-latest-windows.zip"
    } else if cfg!(target_os = "macos") {
        "https://dl.google.com/android/repository/platform-tools-latest-darwin.zip"
    } else {
        "https://dl.google.com/android/repository/platform-tools-latest-linux.zip"
    }
}

pub fn data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("Reflex")
}

pub fn components_dir() -> PathBuf {
    data_dir().join("components")
}

pub fn server_path() -> PathBuf {
    components_dir().join(format!("scrcpy-server-v{SCRCPY_VERSION}"))
}

fn adb_name() -> &'static str {
    if cfg!(target_os = "windows") { "adb.exe" } else { "adb" }
}

pub fn bundled_adb() -> PathBuf {
    components_dir().join("platform-tools").join(adb_name())
}

/// Busca adb: primero el descargado por la app, luego junto al ejecutable,
/// luego en ANDROID_HOME y por último en el PATH.
pub fn find_adb() -> Option<PathBuf> {
    let bundled = bundled_adb();
    if bundled.is_file() {
        return Some(bundled);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent() {
            for p in [dir.join(adb_name()), dir.join("platform-tools").join(adb_name())] {
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(home) = std::env::var_os(var) {
            let p = PathBuf::from(home).join("platform-tools").join(adb_name());
            if p.is_file() {
                return Some(p);
            }
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(adb_name()))
        .find(|p| p.is_file())
}

pub fn screenshots_dir() -> PathBuf {
    dirs::picture_dir().unwrap_or_else(data_dir).join("Reflex")
}

pub fn recordings_dir() -> PathBuf {
    dirs::video_dir().unwrap_or_else(data_dir).join("Reflex")
}

pub fn timestamp() -> String {
    chrono::Local::now().format("%Y%m%d_%H%M%S").to_string()
}
