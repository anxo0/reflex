//! En Windows incrusta el icono y los datos de la app en el .exe.
fn main() {
    println!("cargo:rerun-if-changed=assets/reflex.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/reflex.ico");
        res.set("ProductName", "Reflex");
        res.set("FileDescription", "Reflex: duplica y controla tu móvil en el PC");
        res.set("CompanyName", "Reflex Project");
        res.set("LegalCopyright", "Copyright (C) 2026 anxo0. GPL-3.0-or-later");
        if let Err(e) = res.compile() {
            println!("cargo:warning=No se pudo incrustar el icono: {e}");
        }
    }
}
