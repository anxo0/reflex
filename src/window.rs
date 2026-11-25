//! Ventana sin marco en Windows.
//!
//! winit deja el estilo de ventana con marco (para poder minimizar y redimensionar) y
//! Windows lo pinta en algunos momentos: al moverla, al activarla… Para que nunca se vea,
//! interceptamos los mensajes con los que Windows calcula y dibuja el marco.

#[cfg(windows)]
mod imp {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DwmExtendFrameIntoClientArea, DwmSetWindowAttribute};
    use windows_sys::Win32::UI::Controls::MARGINS;
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_STYLE, GetWindowLongPtrW, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos,
        WM_NCACTIVATE, WM_NCCALCSIZE, WM_NCPAINT, WM_SETICON, WM_SETTEXT, WS_VISIBLE,
    };

    // Mensajes no documentados con los que uxtheme pinta la barra de título clásica.
    const WM_NCUAHDRAWCAPTION: u32 = 0x00AE;
    const WM_NCUAHDRAWFRAME: u32 = 0x00AF;

    const SUBCLASS_ID: usize = 0x5245_464C; // «REFL»

    unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
        match msg {
            // Toda la ventana es zona de cliente: no hay marco que calcular.
            WM_NCCALCSIZE if wparam != 0 => 0,
            // Nunca pintar el marco.
            WM_NCPAINT | WM_NCUAHDRAWCAPTION | WM_NCUAHDRAWFRAME => 0,
            // Cambiar título o icono repinta el marco: lo hacemos con la ventana «invisible» un instante.
            WM_SETTEXT | WM_SETICON => unsafe {
                let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
                SetWindowLongPtrW(hwnd, GWL_STYLE, style & !(WS_VISIBLE as isize));
                let r = DefSubclassProc(hwnd, msg, wparam, lparam);
                SetWindowLongPtrW(hwnd, GWL_STYLE, style);
                r
            },
            // Al activar/desactivar, no repintar el marco (lparam -1 lo evita).
            WM_NCACTIVATE => unsafe { DefSubclassProc(hwnd, msg, wparam, -1) },
            _ => unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
        }
    }

    pub fn init(cc: &eframe::CreationContext<'_>) {
        let Ok(handle) = cc.window_handle() else { return };
        let RawWindowHandle::Win32(w) = handle.as_raw() else { return };
        let hwnd = w.hwnd.get() as HWND;
        // SAFETY: hwnd es la ventana viva de eframe; la subclase vive lo mismo que la ventana
        // y los punteros apuntan a valores locales del tamaño indicado.
        unsafe {
            SetWindowSubclass(hwnd, Some(proc), SUBCLASS_ID, 0);
            let margins = MARGINS { cxLeftWidth: 0, cxRightWidth: 0, cyTopHeight: 0, cyBottomHeight: 0 };
            DwmExtendFrameIntoClientArea(hwnd, &margins);
            let no_color: u32 = 0xFFFF_FFFE; // DWMWA_COLOR_NONE: sin borde de 1 px de Windows 11
            DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR as u32, &no_color as *const u32 as _, 4);
            let no_round: i32 = 1; // DWMWCP_DONOTROUND: las esquinas las dibujamos nosotros
            DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE as u32, &no_round as *const i32 as _, 4);
            // Fuerza a recalcular el marco con la nueva subclase.
            SetWindowPos(hwnd, std::ptr::null_mut(), 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }
}

#[cfg(windows)]
pub use imp::init;

#[cfg(not(windows))]
pub fn init(_cc: &eframe::CreationContext<'_>) {}
