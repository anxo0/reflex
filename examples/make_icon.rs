//! Genera los PNG y el .ico de la app a partir de assets/icon.svg.
//!
//! Uso: `cargo run --example make_icon`

use std::fs::File;
use std::path::Path;

use resvg::{tiny_skia, usvg};

const SIZES: [u32; 8] = [16, 24, 32, 48, 64, 128, 256, 512];

fn render(tree: &usvg::Tree, size: u32) -> Vec<u8> {
    let mut pixmap = tiny_skia::Pixmap::new(size, size).expect("tamaño válido");
    let scale = size as f32 / tree.size().width();
    resvg::render(tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    let svg = std::fs::read_to_string(assets.join("icon.svg"))?;
    let tree = usvg::Tree::from_str(&svg, &usvg::Options::default())?;

    let mut ico = ico::IconDir::new(ico::ResourceType::Icon);
    for size in SIZES {
        let rgba = render(&tree, size);
        image::save_buffer(assets.join(format!("icon-{size}.png")), &rgba, size, size, image::ColorType::Rgba8)?;
        if size <= 256 {
            ico.add_entry(ico::IconDirEntry::encode(&ico::IconImage::from_rgba_data(size, size, rgba))?);
        }
    }
    ico.write(File::create(assets.join("reflex.ico"))?)?;
    println!("Iconos generados en {}", assets.display());
    Ok(())
}
