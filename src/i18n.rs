//! Idiomas: español e inglés. Por defecto el del sistema; se puede cambiar en Ajustes.

use std::sync::atomic::{AtomicU8, Ordering};

use crate::paths;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Lang {
    Auto,
    Es,
    En,
}

// 0 = español, 1 = inglés (el idioma efectivo, ya resuelto).
static CURRENT: AtomicU8 = AtomicU8::new(0);

fn config_path() -> std::path::PathBuf {
    paths::data_dir().join("idioma.txt")
}

fn system_is_spanish() -> bool {
    sys_locale::get_locale().map(|l| l.to_lowercase().starts_with("es")).unwrap_or(true)
}

/// Lee la preferencia guardada y aplica el idioma. Devuelve la preferencia.
pub fn init() -> Lang {
    let pref = match std::fs::read_to_string(config_path()).unwrap_or_default().trim() {
        "es" => Lang::Es,
        "en" => Lang::En,
        _ => Lang::Auto,
    };
    apply(pref);
    pref
}

pub fn set(pref: Lang) {
    apply(pref);
    let v = match pref {
        Lang::Auto => "auto",
        Lang::Es => "es",
        Lang::En => "en",
    };
    let _ = std::fs::create_dir_all(paths::data_dir());
    let _ = std::fs::write(config_path(), v);
}

fn apply(pref: Lang) {
    let english = match pref {
        Lang::Es => false,
        Lang::En => true,
        Lang::Auto => !system_is_spanish(),
    };
    CURRENT.store(english as u8, Ordering::Relaxed);
}

/// Devuelve el texto en el idioma activo.
pub fn t(es: &'static str, en: &'static str) -> &'static str {
    if CURRENT.load(Ordering::Relaxed) == 1 { en } else { es }
}
