//! 从托盘图标母版生成窗口图标，并为 Windows 可执行文件嵌入多尺寸图标。

use std::{env, fs, io::Cursor, path::PathBuf};
use std::{path::Path, process::Command, time::SystemTime};

const SOURCE: &str = "assets/icons/to_words_tray_source.png";
const SIZES: &[u32] = &[16, 24, 32, 48, 64, 128, 256];

fn main() {
    println!("cargo:rerun-if-changed={SOURCE}");
    build_frontend_if_needed();
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
        if let Some(sdk_lib) = windows_sdk_um_lib() {
            println!("cargo:rustc-link-search=native={}", sdk_lib.display());
        }
        let ico = build_ico(&images);
        fs::write(output.join("to_words.ico"), &ico).expect("ICO must be writable");
    }
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(
        tauri_build::WindowsAttributes::new().window_icon_path(output.join("to_words.ico")),
    ))
    .expect("Tauri build metadata must be generated");
}

fn windows_sdk_um_lib() -> Option<PathBuf> {
    let architecture = match env::var("CARGO_CFG_TARGET_ARCH").ok()?.as_str() {
        "x86_64" => "x64",
        "x86" => "x86",
        "aarch64" => "arm64",
        _ => return None,
    };
    let root = env::var_os("ProgramFiles(x86)")
        .map(PathBuf::from)?
        .join("Windows Kits")
        .join("10")
        .join("Lib");
    fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("um").join(architecture))
        .filter(|path| path.join("dinput8.lib").exists())
        .max()
}

fn build_frontend_if_needed() {
    const FRONTEND_SOURCES: &[&str] = &[
        "frontend/src",
        "frontend/index.html",
        "frontend/package.json",
        "frontend/package-lock.json",
        "frontend/vite.config.ts",
        "frontend/tsconfig.json",
        "assets/i18n",
    ];
    for path in FRONTEND_SOURCES {
        println!("cargo:rerun-if-changed={path}");
    }
    let output = Path::new("frontend/dist/index.html");
    let built = fs::metadata(output)
        .and_then(|metadata| metadata.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let source_changed = FRONTEND_SOURCES
        .iter()
        .any(|path| newest_modified(Path::new(path)) > built);
    if !source_changed && output.exists() {
        return;
    }
    assert!(
        Path::new("frontend/node_modules").exists(),
        "React 依赖尚未安装：请先运行 npm --prefix frontend ci"
    );
    // Use the installed project-local tools directly. This also works when a
    // machine has Node.js but its global npm installation is damaged.
    let typecheck = Command::new("node")
        .arg("node_modules/typescript/bin/tsc")
        .arg("--noEmit")
        .current_dir("frontend")
        .status()
        .expect("无法启动 Node.js；请先安装 Node.js");
    assert!(typecheck.success(), "React 前端类型检查失败");
    let bundle = Command::new("node")
        .arg("node_modules/vite/bin/vite.js")
        .arg("build")
        .current_dir("frontend")
        .status()
        .expect("无法启动 Node.js；请先安装 Node.js");
    assert!(bundle.success(), "React 前端构建失败，请检查前端编译错误");
}

fn newest_modified(path: &Path) -> SystemTime {
    if path.is_dir() {
        return fs::read_dir(path)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| newest_modified(&entry.path()))
            .max()
            .unwrap_or(SystemTime::UNIX_EPOCH);
    }
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
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

fn icon_dimensions(size: u32) -> [u8; 2] {
    let dimension = if size == 256 { 0 } else { size as u8 };
    [dimension, dimension]
}
