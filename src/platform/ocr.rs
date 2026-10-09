//! 本地 OCR：先用 Windows GDI 截取屏幕快照，再裁切选区交给本机 Tesseract。
//!
//! 图片只在内存中传给子进程，不上传网络，也不落盘。Tesseract 可放在程序旁、
//! `assets/ocr` 中、系统常见安装目录，或通过 PATH 提供。

use anyhow::{Context, Result, bail};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MonitorBounds {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CaptureRegion {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

/// 框选开始前取得的屏幕快照；预览和识别共用同一份像素。
pub(crate) struct CapturedScreen {
    pub(crate) monitor: MonitorBounds,
    pub(crate) rgb: Vec<u8>,
}

impl CapturedScreen {
    fn crop(&self, region: CaptureRegion) -> Result<Vec<u8>> {
        let left = i64::from(region.x) - i64::from(self.monitor.x);
        let top = i64::from(region.y) - i64::from(self.monitor.y);
        if left < 0
            || top < 0
            || left + i64::from(region.width) > i64::from(self.monitor.width)
            || top + i64::from(region.height) > i64::from(self.monitor.height)
            || region.width == 0
            || region.height == 0
        {
            bail!("OCR 选区超出屏幕截图范围");
        }
        let stride = self.monitor.width as usize * 3;
        let start_x = left as usize * 3;
        let row_bytes = region.width as usize * 3;
        let mut cropped = Vec::with_capacity(row_bytes * region.height as usize);
        for y in top as usize..top as usize + region.height as usize {
            let start = y * stride + start_x;
            cropped.extend_from_slice(
                self.rgb
                    .get(start..start + row_bytes)
                    .context("屏幕截图数据不完整")?,
            );
        }
        Ok(cropped)
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn capture_monitor(monitor: MonitorBounds) -> Result<CapturedScreen> {
    let rgb = capture_rgb(CaptureRegion {
        x: monitor.x,
        y: monitor.y,
        width: monitor.width,
        height: monitor.height,
    })?;
    Ok(CapturedScreen { monitor, rgb })
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn capture_monitor(_monitor: MonitorBounds) -> Result<CapturedScreen> {
    bail!("屏幕 OCR 目前仅支持 Windows")
}

/// 将覆盖层中的两个拖拽点映射到选中显示器的物理像素坐标。
pub(crate) fn selected_region(
    monitor: MonitorBounds,
    viewport: [f32; 2],
    start: [f32; 2],
    end: [f32; 2],
) -> Option<CaptureRegion> {
    if viewport[0] <= 0.0 || viewport[1] <= 0.0 {
        return None;
    }
    let x1 = (start[0].min(end[0]) / viewport[0] * monitor.width as f32)
        .round()
        .clamp(0.0, monitor.width as f32) as i32;
    let x2 = (start[0].max(end[0]) / viewport[0] * monitor.width as f32)
        .round()
        .clamp(0.0, monitor.width as f32) as i32;
    let y1 = (start[1].min(end[1]) / viewport[1] * monitor.height as f32)
        .round()
        .clamp(0.0, monitor.height as f32) as i32;
    let y2 = (start[1].max(end[1]) / viewport[1] * monitor.height as f32)
        .round()
        .clamp(0.0, monitor.height as f32) as i32;
    let width = (x2 - x1) as u32;
    let height = (y2 - y1) as u32;
    (width >= 8 && height >= 8).then_some(CaptureRegion {
        x: monitor.x + x1,
        y: monitor.y + y1,
        width,
        height,
    })
}

#[cfg(target_os = "windows")]
pub(crate) fn monitor_under_cursor() -> Result<MonitorBounds> {
    use windows_sys::Win32::{
        Foundation::POINT,
        Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint},
        UI::WindowsAndMessaging::GetCursorPos,
    };

    // SAFETY: 指针均指向本函数持有的、大小正确的结构体。
    unsafe {
        let mut point = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut point) == 0 {
            bail!("无法取得鼠标所在显示器");
        }
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        if monitor.is_null() {
            bail!("无法取得鼠标所在显示器");
        }
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            bail!("无法读取显示器尺寸");
        }
        let rect = info.rcMonitor;
        Ok(MonitorBounds {
            x: rect.left,
            y: rect.top,
            width: (rect.right - rect.left) as u32,
            height: (rect.bottom - rect.top) as u32,
        })
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn monitor_under_cursor() -> Result<MonitorBounds> {
    bail!("屏幕 OCR 目前仅支持 Windows")
}

#[cfg(target_os = "windows")]
pub(crate) fn recognize_selection(
    screen: &CapturedScreen,
    region: CaptureRegion,
    source: &str,
    target: &str,
) -> Result<String> {
    use image::{ColorType, ImageEncoder, codecs::png::PngEncoder};
    use std::{
        io::Write,
        os::windows::process::CommandExt,
        process::{Command, Stdio},
    };

    let rgb = screen.crop(region)?;
    let (rgb, width, height) = prepare_ocr_image(rgb, region.width, region.height)?;
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(&rgb, width, height, ColorType::Rgb8.into())
        .context("无法编码截图")?;

    let executable = find_tesseract();
    let available = list_languages(&executable)?;
    let language = choose_language(&available, source, target)?;
    let psm = page_segmentation_mode(region);

    // CREATE_NO_WINDOW：发布版没有控制台，避免 OCR 子进程闪出命令行窗口。
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut child = Command::new(&executable)
        .args(["stdin", "stdout", "-l", language, "--psm", psm])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .with_context(|| tesseract_missing_message(&executable))?;
    child
        .stdin
        .take()
        .context("无法向 OCR 引擎传入截图")?
        .write_all(&png)
        .context("无法向 OCR 引擎传入截图")?;
    let output = child.wait_with_output().context("OCR 进程执行失败")?;
    if !output.status.success() {
        bail!(
            "OCR 识别失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let text = String::from_utf8(output.stdout).context("OCR 输出不是有效 UTF-8 文本")?;
    let text = text.trim().replace("\r\n", "\n");
    if text.is_empty() {
        bail!("所选区域未识别到文字，请尝试框选更清晰的区域");
    }
    Ok(text)
}

/// Tesseract 4/5 对浅底深字更稳；只对小选区做轻量处理，避免大图膨胀。
fn prepare_ocr_image(rgb: Vec<u8>, width: u32, height: u32) -> Result<(Vec<u8>, u32, u32)> {
    use image::{Rgb, RgbImage, imageops::FilterType};

    let mut image = RgbImage::from_raw(width, height, rgb).context("OCR 截图数据不完整")?;
    if width > 2048 || height > 2048 {
        return Ok((image.into_raw(), width, height));
    }

    if dark_background(&image) {
        for pixel in image.pixels_mut() {
            for channel in &mut pixel.0 {
                *channel = 255 - *channel;
            }
        }
    }

    // 仅给确实很小的截图放大；对普通小字无条件放大会改变笔画，反而可能误识别。
    if height < 24 {
        image = image::imageops::resize(
            &image,
            width.saturating_mul(2),
            height.saturating_mul(2),
            FilterType::CatmullRom,
        );
    }

    // 紧贴边界的字可能被分割器漏掉；反色后再加白边，避免产生黑色外框。
    const BORDER: u32 = 10;
    let mut padded = RgbImage::from_pixel(
        image.width() + BORDER * 2,
        image.height() + BORDER * 2,
        Rgb([255, 255, 255]),
    );
    image::imageops::replace(&mut padded, &image, BORDER.into(), BORDER.into());
    let (width, height) = padded.dimensions();
    Ok((padded.into_raw(), width, height))
}

fn dark_background(image: &image::RgbImage) -> bool {
    let width = image.width();
    let height = image.height();
    if width == 0 || height == 0 {
        return false;
    }
    let step_x = (width / 16).max(1);
    let step_y = (height / 16).max(1);
    let mut brightness = 0u64;
    let mut samples = 0u64;
    for x in (0..width).step_by(step_x as usize) {
        for y in [0, height - 1] {
            let pixel = image.get_pixel(x, y);
            brightness += u64::from(pixel[0]) + u64::from(pixel[1]) + u64::from(pixel[2]);
            samples += 3;
        }
    }
    for y in (0..height).step_by(step_y as usize) {
        for x in [0, width - 1] {
            let pixel = image.get_pixel(x, y);
            brightness += u64::from(pixel[0]) + u64::from(pixel[1]) + u64::from(pixel[2]);
            samples += 3;
        }
    }
    if brightness / samples >= 110 {
        return false;
    }
    // 深色边框包着白色面板时，不能只看四周，否则会把黑字白底误反色。
    let mut interior_brightness = 0u64;
    let mut interior_samples = 0u64;
    for y in (0..height).step_by(step_y as usize) {
        for x in (0..width).step_by(step_x as usize) {
            let pixel = image.get_pixel(x, y);
            interior_brightness += u64::from(pixel[0]) + u64::from(pixel[1]) + u64::from(pixel[2]);
            interior_samples += 3;
        }
    }
    interior_brightness / interior_samples < 160
}

fn page_segmentation_mode(region: CaptureRegion) -> &'static str {
    if region.height <= 64 && region.width >= region.height {
        "7" // 单行文字
    } else {
        "6" // 多行文字块
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn recognize_selection(
    _screen: &CapturedScreen,
    _region: CaptureRegion,
    _source: &str,
    _target: &str,
) -> Result<String> {
    bail!("屏幕 OCR 目前仅支持 Windows")
}

#[cfg(target_os = "windows")]
fn find_tesseract() -> std::path::PathBuf {
    use std::path::PathBuf;
    let mut paths = Vec::new();
    if let Some(path) = std::env::var_os("TESSERACT_EXE") {
        paths.push(PathBuf::from(path));
    }
    if let Ok(executable) = std::env::current_exe()
        && let Some(directory) = executable.parent()
    {
        paths.push(directory.join("assets/ocr/tesseract.exe"));
        paths.push(directory.join("tesseract.exe"));
    }
    paths.push(crate::project_directory().join("assets/ocr/tesseract.exe"));
    paths.push(PathBuf::from(
        r"C:\Program Files\Tesseract-OCR\tesseract.exe",
    ));
    paths.push(PathBuf::from(
        r"C:\Program Files (x86)\Tesseract-OCR\tesseract.exe",
    ));
    if let Some(path) = installed_tesseract_from_registry() {
        paths.push(path);
    }
    paths
        .into_iter()
        .find(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("tesseract"))
}

#[cfg(target_os = "windows")]
pub(crate) fn engine_status() -> String {
    use crate::i18n::{self, tr};
    let executable = find_tesseract();
    match list_languages(&executable) {
        Ok(languages) => i18n::message(
            "本地 OCR：{path} · 已安装语言：{languages}",
            &[
                ("path", &executable.display().to_string()),
                (
                    "languages",
                    &if languages.is_empty() {
                        tr("无").to_string()
                    } else {
                        languages.join("、")
                    },
                ),
            ],
        ),
        Err(_) => tr("本地 OCR：未找到 Tesseract，请先安装或设置 TESSERACT_EXE。").to_string(),
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn engine_status() -> String {
    crate::i18n::tr("屏幕 OCR 目前仅支持 Windows。").to_string()
}

#[cfg(target_os = "windows")]
fn installed_tesseract_from_registry() -> Option<std::path::PathBuf> {
    use windows_sys::Win32::{
        Foundation::ERROR_SUCCESS,
        System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW},
    };

    for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        for key in [
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Tesseract-OCR",
            r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Tesseract-OCR",
        ] {
            let key: Vec<u16> = key.encode_utf16().chain(std::iter::once(0)).collect();
            let value: Vec<u16> = "DisplayIcon"
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let mut buffer = vec![0u16; 2048];
            let mut bytes = (buffer.len() * 2) as u32;
            // SAFETY: RegGetValueW 只写入 buffer 指定的字节数；键和值字符串均以 NUL 结尾。
            let status = unsafe {
                RegGetValueW(
                    root,
                    key.as_ptr(),
                    value.as_ptr(),
                    RRF_RT_REG_SZ,
                    std::ptr::null_mut(),
                    buffer.as_mut_ptr().cast(),
                    &mut bytes,
                )
            };
            if status == ERROR_SUCCESS {
                let end = buffer
                    .iter()
                    .position(|ch| *ch == 0)
                    .unwrap_or(buffer.len());
                let icon = String::from_utf16_lossy(&buffer[..end]);
                let icon = icon.trim().split(',').next()?.trim().trim_matches('"');
                if let Some(parent) = std::path::Path::new(icon).parent() {
                    let executable = parent.join("tesseract.exe");
                    if executable.is_file() {
                        return Some(executable);
                    }
                }
            }
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn tesseract_missing_message(executable: &std::path::Path) -> String {
    format!(
        "无法启动本地 OCR 引擎（{}）。请安装 Tesseract OCR，或把 tesseract.exe 与 tessdata 放入程序目录的 assets/ocr 文件夹",
        executable.display()
    )
}

#[cfg(target_os = "windows")]
fn list_languages(executable: &std::path::Path) -> Result<Vec<String>> {
    use std::{os::windows::process::CommandExt, process::Command};
    let output = Command::new(executable)
        .arg("--list-langs")
        .creation_flags(0x0800_0000)
        .output()
        .with_context(|| tesseract_missing_message(executable))?;
    if !output.status.success() {
        bail!(
            "无法读取 Tesseract 语言包：{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let output = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && line.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
        .map(str::to_owned)
        .collect())
}

fn tesseract_language(code: &str) -> Option<&'static str> {
    Some(match code {
        "cn" => "chi_sim",
        "zh_tw" => "chi_tra",
        "ko" => "kor",
        "ja" => "jpn",
        "en" => "eng",
        "fr" => "fra",
        "de" => "deu",
        "es" => "spa",
        "pt" => "por",
        "it" => "ita",
        "ru" => "rus",
        "uk" => "ukr",
        "ar" => "ara",
        "he" => "heb",
        "fa" => "fas",
        "hi" => "hin",
        "ur" => "urd",
        "bn" => "ben",
        "ta" => "tam",
        "te" => "tel",
        "th" => "tha",
        "vi" => "vie",
        "id" => "ind",
        "ms" => "msa",
        "fil" => "fil",
        "tr" => "tur",
        "pl" => "pol",
        "nl" => "nld",
        "sv" => "swe",
        "no" => "nor",
        "da" => "dan",
        "fi" => "fin",
        "cs" => "ces",
        "sk" => "slk",
        "hu" => "hun",
        "ro" => "ron",
        "bg" => "bul",
        "el" => "ell",
        "sw" => "swa",
        "km" => "khm",
        "lo" => "lao",
        "my" => "mya",
        "mn" => "mon",
        "kk" => "kaz",
        "uz" => "uzb",
        _ => return None,
    })
}

fn choose_language<'a>(available: &'a [String], source: &str, target: &str) -> Result<&'a str> {
    // 目标语言由用户明确指定时不能静默换用别的模型，否则短词容易被识别成别的文字系统。
    if !target.is_empty() {
        let language = tesseract_language(target)
            .with_context(|| format!("目标语言 {target} 暂无对应的 Tesseract OCR 模型"))?;
        return available
            .iter()
            .find(|item| item.as_str() == language)
            .map(String::as_str)
            .with_context(|| {
                format!("缺少目标语言的 OCR 语言包：{language}.traineddata，请先安装")
            });
    }
    if let Some(language) = tesseract_language(source) {
        return available
            .iter()
            .find(|item| item.as_str() == language)
            .map(String::as_str)
            .with_context(|| format!("请安装原始语言的 {language}.traineddata OCR 语言包"));
    }
    available
        .iter()
        .find(|item| item.as_str() == "chi_sim")
        .or_else(|| available.iter().find(|item| item.as_str() == "eng"))
        .or_else(|| available.iter().find(|item| item.as_str() != "osd"))
        .map(String::as_str)
        .context("没有可用的 OCR 语言包；请安装 Tesseract 的文字识别语言包")
}

#[cfg(target_os = "windows")]
fn capture_rgb(region: CaptureRegion) -> Result<Vec<u8>> {
    use std::ptr;
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BitBlt, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC,
        DeleteObject, GetDC, ReleaseDC, SRCCOPY, SelectObject,
    };

    if region.width == 0 || region.height == 0 || region.width > 16_384 || region.height > 16_384 {
        bail!("截图范围无效");
    }
    let pixel_count = (region.width as usize)
        .checked_mul(region.height as usize)
        .context("截图范围过大")?;
    if pixel_count > 40_000_000 {
        bail!("截图区域过大，请框选更小的范围");
    }
    let byte_count = pixel_count.checked_mul(4).context("截图范围过大")?;

    // SAFETY: 句柄属于当前进程；DIB 指针在删除 bitmap 前有效，且长度由区域大小校验。
    unsafe {
        let screen = GetDC(ptr::null_mut());
        if screen.is_null() {
            bail!("无法读取屏幕图像");
        }
        let memory = CreateCompatibleDC(screen);
        if memory.is_null() {
            ReleaseDC(ptr::null_mut(), screen);
            bail!("无法创建截图缓冲区");
        }
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = std::mem::size_of_val(&info.bmiHeader) as u32;
        info.bmiHeader.biWidth = region.width as i32;
        info.bmiHeader.biHeight = -(region.height as i32);
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut bits = ptr::null_mut();
        let bitmap = CreateDIBSection(screen, &info, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
        if bitmap.is_null() || bits.is_null() {
            DeleteDC(memory);
            ReleaseDC(ptr::null_mut(), screen);
            bail!("无法分配截图缓冲区");
        }
        let old = SelectObject(memory, bitmap);
        let copied = !old.is_null()
            && old as isize != -1
            && BitBlt(
                memory,
                0,
                0,
                region.width as i32,
                region.height as i32,
                screen,
                region.x,
                region.y,
                SRCCOPY,
            ) != 0;
        let mut rgb = Vec::with_capacity(pixel_count * 3);
        if copied {
            let pixels = std::slice::from_raw_parts(bits.cast::<u8>(), byte_count);
            for bgra in pixels.chunks_exact(4) {
                rgb.extend_from_slice(&[bgra[2], bgra[1], bgra[0]]);
            }
        }
        if !old.is_null() && old as isize != -1 {
            SelectObject(memory, old);
        }
        DeleteObject(bitmap);
        DeleteDC(memory);
        ReleaseDC(ptr::null_mut(), screen);
        if !copied {
            bail!("无法截取选中的屏幕区域");
        }
        Ok(rgb)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CaptureRegion, CapturedScreen, MonitorBounds, choose_language, page_segmentation_mode,
        prepare_ocr_image, selected_region, tesseract_language,
    };

    #[test]
    fn crops_from_original_snapshot_instead_of_new_screen_capture() {
        let screen = CapturedScreen {
            monitor: MonitorBounds {
                x: -2,
                y: 4,
                width: 3,
                height: 2,
            },
            rgb: (0..18).collect(),
        };
        assert_eq!(
            screen
                .crop(CaptureRegion {
                    x: -1,
                    y: 4,
                    width: 2,
                    height: 2
                })
                .unwrap(),
            vec![3, 4, 5, 6, 7, 8, 12, 13, 14, 15, 16, 17]
        );
        assert!(
            screen
                .crop(CaptureRegion {
                    x: 1,
                    y: 4,
                    width: 1,
                    height: 1
                })
                .is_err()
        );
    }

    #[test]
    fn selection_maps_to_secondary_monitor_physical_pixels() {
        let monitor = MonitorBounds {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        assert_eq!(
            selected_region(monitor, [960.0, 540.0], [100.0, 100.0], [200.0, 200.0]),
            Some(CaptureRegion {
                x: -1720,
                y: 200,
                width: 200,
                height: 200
            })
        );
    }

    #[test]
    fn tiny_selection_is_ignored() {
        let monitor = MonitorBounds {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        };
        assert_eq!(
            selected_region(monitor, [100.0, 100.0], [1.0, 1.0], [2.0, 2.0]),
            None
        );
    }

    #[test]
    fn uses_only_the_selected_target_language_pack() {
        let available = ["eng".into(), "kor".into(), "chi_sim".into()];
        assert_eq!(choose_language(&available, "auto", "ko").unwrap(), "kor");
        assert_eq!(choose_language(&available, "cn", "ko").unwrap(), "kor");
        assert_eq!(choose_language(&available, "ko", "").unwrap(), "kor");
        assert_eq!(choose_language(&available, "auto", "").unwrap(), "chi_sim");
        assert_eq!(tesseract_language("ja"), Some("jpn"));
        assert_eq!(tesseract_language("ar"), Some("ara"));
        for language in crate::translation_language::LANGUAGES {
            assert!(
                tesseract_language(language.code).is_some(),
                "missing OCR model for {}",
                language.code
            );
        }
        assert!(
            choose_language(&["eng".into()], "auto", "ko")
                .unwrap_err()
                .to_string()
                .contains("kor.traineddata")
        );
    }

    #[test]
    fn dark_text_and_white_text_are_normalized_without_changing_the_original() {
        let mut dark = vec![24u8; 8 * 8 * 3];
        let center = (4 * 8 + 4) * 3;
        dark[center..center + 3].fill(255);
        let untouched = dark.clone();
        let (prepared, width, height) = prepare_ocr_image(dark, 8, 8).unwrap();
        assert_eq!((width, height), (36, 36));
        assert_eq!(untouched[center], 255);
        assert_eq!(&prepared[..3], &[255, 255, 255]);
        assert!(prepared[((18 * width + 18) * 3) as usize] < 80);

        let mut light = vec![255u8; 8 * 8 * 3];
        light[center..center + 3].fill(0);
        let (prepared, _, _) = prepare_ocr_image(light, 8, 8).unwrap();
        assert!(prepared[((18 * width + 18) * 3) as usize] < 80);

        let mut framed = vec![24u8; 12 * 12 * 3];
        for y in 1..11 {
            for x in 1..11 {
                let pixel = (y * 12 + x) * 3;
                framed[pixel..pixel + 3].fill(255);
            }
        }
        assert!(!super::dark_background(
            &image::RgbImage::from_raw(12, 12, framed).unwrap()
        ));
    }

    #[test]
    fn short_selection_uses_single_line_segmentation() {
        let region = CaptureRegion {
            x: 0,
            y: 0,
            width: 90,
            height: 43,
        };
        assert_eq!(page_segmentation_mode(region), "7");
        assert_eq!(
            page_segmentation_mode(CaptureRegion {
                height: 120,
                ..region
            }),
            "6"
        );
    }
}
