//! Registro sencillo en memoria que la interfaz puede mostrar.

use std::collections::VecDeque;
use std::sync::Mutex;

static LOG: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
const MAX_LINES: usize = 600;

pub fn log(msg: impl AsRef<str>) {
    let line = format!("[{}] {}", chrono::Local::now().format("%H:%M:%S"), msg.as_ref());
    eprintln!("{line}");
    if let Ok(mut l) = LOG.lock() {
        l.push_back(line);
        while l.len() > MAX_LINES {
            l.pop_front();
        }
    }
}

pub fn snapshot() -> Vec<String> {
    LOG.lock().map(|l| l.iter().cloned().collect()).unwrap_or_default()
}
