//! Ajustes de la ventana nativa que egui no expone.

#[cfg(windows)]
mod imp {
    use std::sync::atomic::{AtomicIsize, Ordering};

    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DwmSetWindowAttribute};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_STYLE, GetWindowLongPtrW, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos,
        WS_CAPTION, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
    };

    static HWND_VALUE: AtomicIsize = AtomicIsize::new(0);

    pub fn init(cc: &eframe::CreationContext<'_>) {
        let Ok(handle) = cc.window_handle() else { return };
        let RawWindowHandle::Win32(w) = handle.as_raw() else { return };
        let hwnd = w.hwnd.get();
        HWND_VALUE.store(hwnd, Ordering::Relaxed);
        // SAFETY: hwnd es la ventana viva de eframe; los punteros apuntan a valores locales del tamaño indicado.
        unsafe {
            let hwnd = hwnd as HWND;
            let no_color: u32 = 0xFFFF_FFFE; // DWMWA_COLOR_NONE: sin borde de 1 px de Windows 11
            DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR as u32, &no_color as *const u32 as _, 4);
            let no_round: i32 = 1; // DWMWCP_DONOTROUND: las esquinas las dibujamos nosotros
            DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE as u32, &no_round as *const i32 as _, 4);
        }
        keep_borderless();
    }

    /// Deja la ventana como «popup» puro: sin marco, sin sombra y sin bordes antiguos al
    /// cambiar el tamaño. winit vuelve a poner el marco en algunos casos, así que se
    /// comprueba en cada fotograma (solo cambia algo si hace falta).
    pub fn keep_borderless() {
        let hwnd = HWND_VALUE.load(Ordering::Relaxed);
        if hwnd == 0 {
            return;
        }
        // SAFETY: la ventana sigue viva mientras la app se ejecuta.
        unsafe {
            let hwnd = hwnd as HWND;
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
            let frame = WS_CAPTION | WS_THICKFRAME | WS_MAXIMIZEBOX;
            if style & frame != 0 {
                let new = (style & !frame) | WS_POPUP | WS_MINIMIZEBOX | WS_SYSMENU;
                SetWindowLongPtrW(hwnd, GWL_STYLE, new as isize);
                SetWindowPos(hwnd, std::ptr::null_mut(), 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
    }
}

#[cfg(windows)]
pub use imp::{init, keep_borderless};

#[cfg(not(windows))]
pub fn init(_cc: &eframe::CreationContext<'_>) {}

#[cfg(not(windows))]
pub fn keep_borderless() {}
