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
        GWL_STYLE, GetWindowLongPtrW, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOCOPYBITS, SWP_NOMOVE, SWP_NOREDRAW, SWP_NOSIZE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos,
        WM_NCACTIVATE, WM_NCCALCSIZE, WM_NCPAINT, WM_SETICON, WM_SETTEXT, WS_VISIBLE,
    };

    // Mensajes no documentados con los que uxtheme pinta la barra de título clásica.
    const WM_NCUAHDRAWCAPTION: u32 = 0x00AE;
    const WM_NCUAHDRAWFRAME: u32 = 0x00AF;

    const SUBCLASS_ID: usize = 0x5245_464C; // «REFL»
    const WM_SIZING: u32 = 0x0214;
    const WM_ERASEBKGND: u32 = 0x0014;
    const WMSZ_LEFT: usize = 1;
    const WMSZ_RIGHT: usize = 2;
    const WMSZ_TOP: usize = 3;
    const WMSZ_TOPLEFT: usize = 4;
    const WMSZ_TOPRIGHT: usize = 5;

    /// Forma que se mantiene al redimensionar con el ratón (en píxeles físicos).
    static ASPECT_LOCK: std::sync::Mutex<Option<super::AspectLock>> = std::sync::Mutex::new(None);

    static HWND_VALUE: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

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
            // El fondo lo pinta egui entero: si Windows lo borra antes, parpadea.
            WM_ERASEBKGND => 1,
            // Mientras arrastras la esquina, Windows nos deja corregir el rectángulo:
            // así la ventana mantiene la forma del móvil y el redimensionado es el nativo, fluido.
            WM_SIZING => {
                if let Some(lock) = *ASPECT_LOCK.lock().unwrap() {
                    // SAFETY: con WM_SIZING, lparam apunta a un RECT válido que podemos modificar.
                    let r = unsafe { &mut *(lparam as *mut windows_sys::Win32::Foundation::RECT) };
                    let (w, h) = lock.fit(r.right - r.left, r.bottom - r.top, matches!(wparam, WMSZ_LEFT | WMSZ_RIGHT));
                    if matches!(wparam, WMSZ_LEFT | WMSZ_TOPLEFT | 7) {
                        r.left = r.right - w;
                    } else {
                        r.right = r.left + w;
                    }
                    if matches!(wparam, WMSZ_TOP | WMSZ_TOPLEFT | WMSZ_TOPRIGHT) {
                        r.top = r.bottom - h;
                    } else {
                        r.bottom = r.top + h;
                    }
                    return 1;
                }
                unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
            }
            // Al activar/desactivar, no repintar el marco (lparam -1 lo evita).
            WM_NCACTIVATE => unsafe { DefSubclassProc(hwnd, msg, wparam, -1) },
            _ => unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
        }
    }

    pub fn init(cc: &eframe::CreationContext<'_>) {
        let Ok(handle) = cc.window_handle() else { return };
        let RawWindowHandle::Win32(w) = handle.as_raw() else { return };
        let hwnd = w.hwnd.get() as HWND;
        HWND_VALUE.store(hwnd as isize, std::sync::atomic::Ordering::Relaxed);
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
        // Tamaño real (sin el marco que ya no existe): genera un evento de tamaño y la
        // superficie de dibujo se ajusta, así el contenido no sale desplazado al arrancar.
        resize(&cc.egui_ctx, crate::app::WINDOW);
    }

    /// Mueve la ventana (posición en puntos de egui, esquina superior izquierda).
    pub fn move_to(ctx: &eframe::egui::Context, pos: eframe::egui::Pos2) {
        let hwnd = HWND_VALUE.load(std::sync::atomic::Ordering::Relaxed);
        if hwnd == 0 {
            ctx.send_viewport_cmd(eframe::egui::ViewportCommand::OuterPosition(pos));
            return;
        }
        let ppp = ctx.pixels_per_point();
        // SAFETY: la ventana sigue viva mientras la app se ejecuta.
        unsafe {
            SetWindowPos(hwnd as HWND, std::ptr::null_mut(), (pos.x * ppp).round() as i32, (pos.y * ppp).round() as i32, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }

    /// Activa o quita la forma fija para el redimensionado con el ratón.
    pub fn set_aspect_lock(lock: Option<super::AspectLock>) {
        *ASPECT_LOCK.lock().unwrap() = lock;
    }

    /// Cambia el tamaño de la ventana directamente con Windows. Pasar por winit no sirve:
    /// le sumaría el marco nativo que hemos quitado y lo volvería a pintar.
    pub fn resize(ctx: &eframe::egui::Context, size: eframe::egui::Vec2) {
        let hwnd = HWND_VALUE.load(std::sync::atomic::Ordering::Relaxed);
        if hwnd == 0 {
            ctx.send_viewport_cmd(eframe::egui::ViewportCommand::InnerSize(size));
            return;
        }
        let ppp = ctx.pixels_per_point();
        let (w, h) = ((size.x * ppp).round() as i32, (size.y * ppp).round() as i32);
        // SAFETY: la ventana sigue viva mientras la app se ejecuta.
        unsafe {
            // Sin copiar ni borrar el contenido antiguo: el siguiente fotograma lo pinta todo.
            // Así no parpadea mientras se arrastra.
            SetWindowPos(hwnd as HWND, std::ptr::null_mut(), 0, 0, w, h, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOCOPYBITS | SWP_NOREDRAW);
        }
    }
}

#[cfg(windows)]
pub use imp::{init, move_to, resize, set_aspect_lock};

/// Cómo calcular el tamaño de la ventana de duplicado a partir de lo que pide el ratón.
/// Todo en píxeles físicos.
#[derive(Clone, Copy, Debug)]
pub struct AspectLock {
    /// Ancho/alto de la pantalla del móvil.
    pub aspect: f32,
    /// Marco alrededor de la pantalla, por cada lado.
    pub bezel: f32,
    /// Lo que ocupa la columna de controles (con su separación).
    pub extra_w: f32,
    pub min_h: f32,
    pub max_h: f32,
}

impl AspectLock {
    /// Tamaño de ventana que mantiene la forma. En vertical manda el alto; en horizontal,
    /// o si solo se arrastra un lado, el ancho.
    pub fn fit(&self, w: i32, h: i32, by_width: bool) -> (i32, i32) {
        let b2 = 2.0 * self.bezel;
        let from_h = |h: f32| {
            let h = h.clamp(self.min_h, self.max_h);
            ((h - b2) * self.aspect + b2 + self.extra_w, h)
        };
        let (w, h) = if by_width || self.aspect > 1.0 {
            let phone_w = (w as f32 - self.extra_w).max(b2 + 40.0);
            let phone_h = (phone_w - b2) / self.aspect + b2;
            if self.aspect > 1.0 {
                let max_w = self.max_h * 1.28;
                let phone_w = phone_w.min(max_w);
                let phone_h = (phone_w - b2) / self.aspect + b2;
                (phone_w + self.extra_w, phone_h.max(self.min_h))
            } else {
                from_h(phone_h)
            }
        } else {
            from_h(h as f32)
        };
        (w.round() as i32, h.round() as i32)
    }
}

#[cfg(not(windows))]
pub fn set_aspect_lock(_lock: Option<AspectLock>) {}

#[cfg(not(windows))]
pub fn init(_cc: &eframe::CreationContext<'_>) {}

#[cfg(not(windows))]
pub fn move_to(ctx: &eframe::egui::Context, pos: eframe::egui::Pos2) {
    ctx.send_viewport_cmd(eframe::egui::ViewportCommand::OuterPosition(pos));
}

#[cfg(not(windows))]
pub fn resize(ctx: &eframe::egui::Context, size: eframe::egui::Vec2) {
    ctx.send_viewport_cmd(eframe::egui::ViewportCommand::InnerSize(size));
}
