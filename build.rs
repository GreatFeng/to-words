//! 从托盘图标母版生成窗口图标，并为 Windows 可执行文件嵌入多尺寸图标。

use std::{env, fs, io::Cursor, path::PathBuf};

const SOURCE: &str = "assets/icons/to_words_tray_source.png";
const SIZES: &[u32] = &[16, 24, 32, 48, 64, 128, 256];

fn main() {
    println!("cargo:rerun-if-changed={SOURCE}");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo must provide OUT_DIR"));
    let source = image::open(SOURCE).expect("app icon source image must be readable");
    let mut images = Vec::new();
    for &size in SIZES {
        let icon = source.resize_exact(size, size, image::imageops::FilterType::Lanczos3);
        let mut png = Vec::new();
        icon.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .expect("app icon PNG encoding must succeed");
        if size == 256 {
            fs::write(output.join("to_words_window_256.png"), &png)
                .expect("window icon must be writable");
        }
        images.push((size, png));
    }

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let ico = build_ico(&images);
        fs::write(output.join("to_words.ico"), &ico).expect("ICO must be writable");
        if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
            let resource = output.join("to_words_icon.res");
            fs::write(&resource, build_windows_resource(&images))
                .expect("Windows icon resource must be writable");
            println!("cargo:rustc-link-arg-bin=to_words={}", resource.display());
        }
    }
}

fn build_ico(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut ico = Vec::new();
    ico.extend_from_slice(&0u16.to_le_bytes());
    ico.extend_from_slice(&1u16.to_le_bytes());
    ico.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + images.len() * 16;
    for (size, png) in images {
        ico.extend_from_slice(&icon_dimensions(*size));
        ico.extend_from_slice(&[0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
        ico.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += png.len();
    }
    for (_, png) in images {
        ico.extend_from_slice(png);
    }
    ico
}

fn build_windows_resource(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut resource = Vec::new();
    write_resource(&mut resource, 0, 0, &[]);

    let mut group = Vec::new();
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&(images.len() as u16).to_le_bytes());
    for (index, (size, png)) in images.iter().enumerate() {
        group.extend_from_slice(&icon_dimensions(*size));
        group.extend_from_slice(&[0, 0]);
        group.extend_from_slice(&1u16.to_le_bytes());
        group.extend_from_slice(&32u16.to_le_bytes());
        group.extend_from_slice(&(png.len() as u32).to_le_bytes());
        group.extend_from_slice(&((index + 1) as u16).to_le_bytes());
        write_resource(&mut resource, 3, (index + 1) as u16, png);
    }
    write_resource(&mut resource, 14, 1, &group);
    resource
}

fn icon_dimensions(size: u32) -> [u8; 2] {
    let dimension = if size == 256 { 0 } else { size as u8 };
    [dimension, dimension]
}

fn write_resource(output: &mut Vec<u8>, kind: u16, id: u16, data: &[u8]) {
    output.extend_from_slice(&(data.len() as u32).to_le_bytes());
    output.extend_from_slice(&32u32.to_le_bytes());
    output.extend_from_slice(&0xffffu16.to_le_bytes());
    output.extend_from_slice(&kind.to_le_bytes());
    output.extend_from_slice(&0xffffu16.to_le_bytes());
    output.extend_from_slice(&id.to_le_bytes());
    output.extend_from_slice(&0u32.to_le_bytes());
    output.extend_from_slice(&0x1030u16.to_le_bytes());
    output.extend_from_slice(&0u16.to_le_bytes());
    output.extend_from_slice(&0u32.to_le_bytes());
    output.extend_from_slice(&0u32.to_le_bytes());
    output.extend_from_slice(data);
    while output.len() % 4 != 0 {
        output.push(0);
    }
}
