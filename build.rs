#[cfg(windows)]
fn main() {
    use std::path::PathBuf;

    println!("cargo:rerun-if-changed=assets/icon.png");

    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR missing"));
    let icon_path = out_dir.join("app-icon.ico");

    generate_windows_icon("assets/icon.png", &icon_path).expect("Failed to generate Windows icon");

    let mut res = winresource::WindowsResource::new();
    res.set_icon(icon_path.to_string_lossy().as_ref());
    res.compile().expect("Failed to compile Windows resources");
}

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn generate_windows_icon(
    png_path: &str,
    output_path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    use ico::{IconDir, IconDirEntry, IconImage, ResourceType};
    use image::imageops::FilterType;
    use std::fs::File;

    let source = image::open(png_path)?.into_rgba8();
    let mut icon_dir = IconDir::new(ResourceType::Icon);

    for size in [16, 24, 32, 48, 64, 128, 256] {
        let resized = image::imageops::resize(&source, size, size, FilterType::Lanczos3);
        let icon_image = IconImage::from_rgba_data(size, size, resized.into_raw());
        let entry = IconDirEntry::encode(&icon_image)?;
        icon_dir.add_entry(entry);
    }

    let mut file = File::create(output_path)?;
    icon_dir.write(&mut file)?;
    Ok(())
}
