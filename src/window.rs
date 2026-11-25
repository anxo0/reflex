//! Ajustes de la ventana nativa que egui no expone.

/// Quita el borde fino y la sombra que Windows 11 pone a las ventanas sin marco,
/// para que solo se vean el móvil y la columna de controles.
#[cfg(windows)]
pub fn remove_native_border(cc: &eframe::CreationContext<'_>) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_BORDER_COLOR, DWMWA_NCRENDERING_POLICY, DWMWA_WINDOW_CORNER_PREFERENCE, DwmExtendFrameIntoClientArea, DwmSetWindowAttribute};
    use windows_sys::Win32::UI::Controls::MARGINS;

    let Ok(handle) = cc.window_handle() else { return };
    let RawWindowHandle::Win32(w) = handle.as_raw() else { return };
    let hwnd = w.hwnd.get() as windows_sys::Win32::Foundation::HWND;
    // SAFETY: hwnd es la ventana viva de eframe; los punteros apuntan a valores locales del tamaño indicado.
    unsafe {
        // Sin marco no cliente: ni sombra ni borde de 1 px de Windows 11.
        let disabled: i32 = 1; // DWMNCRP_DISABLED
        DwmSetWindowAttribute(hwnd, DWMWA_NCRENDERING_POLICY as u32, &disabled as *const i32 as _, 4);
        let margins = MARGINS { cxLeftWidth: 0, cxRightWidth: 0, cyTopHeight: 0, cyBottomHeight: 0 };
        DwmExtendFrameIntoClientArea(hwnd, &margins);
        let no_color: u32 = 0xFFFF_FFFE; // DWMWA_COLOR_NONE
        DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR as u32, &no_color as *const u32 as _, 4);
        let no_round: i32 = 1; // DWMWCP_DONOTROUND: las esquinas las dibujamos nosotros
        DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE as u32, &no_round as *const i32 as _, 4);
    }
}

#[cfg(not(windows))]
pub fn remove_native_border(_cc: &eframe::CreationContext<'_>) {}
