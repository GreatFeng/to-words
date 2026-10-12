//! 给仅接受窄字符路径的第三方语音 CLI 准备 ASCII 路径。
//!
//! Windows 能用 Unicode 启动程序，但这些 CLI 仍可能把中文模型路径转换成问号。
//! 优先使用系统已有的短路径；不可用时只缓存所需程序、模型和同目录 DLL。

use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

static NEXT_STAGE_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) struct VoiceCommandPaths {
    pub(crate) executable: PathBuf,
    pub(crate) model: PathBuf,
    pub(crate) vad: Option<PathBuf>,
    pub(crate) scratch: PathBuf,
}

pub(crate) fn prepare(
    executable: &Path,
    model: &Path,
    vad: Option<&Path>,
) -> Result<VoiceCommandPaths> {
    let scratch = ascii_scratch_dir()?;
    prepare_with_scratch(executable, model, vad, scratch)
}

fn prepare_with_scratch(
    executable: &Path,
    model: &Path,
    vad: Option<&Path>,
    scratch: PathBuf,
) -> Result<VoiceCommandPaths> {
    let vad_alias = match vad {
        Some(path) => ascii_alias(path).map(Some),
        None => Some(None),
    };
    if let (Some(executable), Some(model), Some(vad)) =
        (ascii_alias(executable), ascii_alias(model), vad_alias)
    {
        return Ok(VoiceCommandPaths {
            executable,
            model,
            vad,
            scratch,
        });
    }

    let source_dir = executable.parent().context("语音识别程序路径无效")?;
    let cache = scratch.join(format!("engine_{:016x}", path_hash(source_dir)));
    fs::create_dir_all(&cache).with_context(|| {
        format!(
            "无法创建语音识别英文路径缓存 {}，请确认临时目录有足够空间和写入权限",
            cache.display()
        )
    })?;
    let executable = stage_file(executable, &cache)?;
    let model = stage_file(model, &cache)?;
    let vad = vad.map(|path| stage_file(path, &cache)).transpose()?;
    for item in fs::read_dir(source_dir).context("无法读取语音识别程序目录")? {
        let path = item?.path();
        if path.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("dll"))
        {
            stage_file(&path, &cache)?;
        }
    }
    Ok(VoiceCommandPaths {
        executable,
        model,
        vad,
        scratch,
    })
}

pub(crate) fn path_warning() -> Option<String> {
    ascii_alias(&std::env::temp_dir()).is_none().then(|| {
        crate::i18n::message(
            "系统临时目录 {path} 含有语音引擎无法读取的字符，也没有可用的短路径。请将 Windows 的 TEMP/TMP 设置为可写的英文路径后重启程序。",
            &[("path", &std::env::temp_dir().display().to_string())],
        )
    })
}

pub(crate) fn resource_notice(
    executable: &Path,
    model: &Path,
    vad: Option<&Path>,
) -> Option<String> {
    (ascii_alias(executable).is_none()
        || ascii_alias(model).is_none()
        || vad.is_some_and(|path| ascii_alias(path).is_none()))
    .then(|| {
        crate::i18n::tr(
            "检测到语音资源位于中文或其他非英文路径。首次识别会准备英文路径兼容缓存，可能需要额外磁盘空间。",
        )
        .to_string()
    })
}

fn ascii_scratch_dir() -> Result<PathBuf> {
    if let Some(warning) = path_warning() {
        bail!("{warning}");
    }
    let temp = ascii_alias(&std::env::temp_dir()).expect("checked above");
    let scratch = temp.join("to_words_voice_runtime");
    fs::create_dir_all(&scratch).with_context(|| {
        format!(
            "无法创建语音识别临时目录 {}，请检查写入权限和剩余空间",
            scratch.display()
        )
    })?;
    Ok(scratch)
}

fn stage_file(source: &Path, cache: &Path) -> Result<PathBuf> {
    let name = source.file_name().context("语音识别资源文件名无效")?;
    if !name.to_string_lossy().is_ascii() {
        bail!("语音识别资源文件名必须使用英文字符：{}", source.display());
    }
    let destination = cache.join(name);
    let stamp = cache.join(format!("{}.stamp", name.to_string_lossy()));
    let source_meta = fs::metadata(source)
        .with_context(|| format!("无法读取语音识别资源：{}", source.display()))?;
    let modified = source_meta
        .modified()
        .unwrap_or(UNIX_EPOCH)
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let fingerprint = format!(
        "{}-{}-{}",
        source_meta.len(),
        modified.as_secs(),
        modified.subsec_nanos()
    );
    if destination
        .metadata()
        .is_ok_and(|meta| meta.len() == source_meta.len())
        && fs::read_to_string(&stamp).is_ok_and(|saved| saved == fingerprint)
    {
        return Ok(destination);
    }

    let temporary = cache.join(format!(
        "{}.{}-{}.tmp",
        name.to_string_lossy(),
        std::process::id(),
        NEXT_STAGE_ID.fetch_add(1, Ordering::Relaxed)
    ));
    if fs::hard_link(source, &temporary).is_err() {
        fs::copy(source, &temporary).with_context(|| {
            format!(
                "无法将语音模型准备到英文路径 {}；请确认临时目录有足够空间，或将程序移到英文目录",
                cache.display()
            )
        })?;
    }
    if destination.exists() {
        fs::remove_file(&destination)
            .with_context(|| format!("无法更新语音识别缓存：{}", destination.display()))?;
    }
    fs::rename(&temporary, &destination)
        .with_context(|| format!("无法完成语音识别缓存：{}", destination.display()))?;
    fs::write(&stamp, fingerprint).context("无法记录语音识别缓存版本")?;
    Ok(destination)
}

fn path_hash(path: &Path) -> u64 {
    path.to_string_lossy()
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
}

fn ascii_alias(path: &Path) -> Option<PathBuf> {
    if path.as_os_str().to_string_lossy().is_ascii() {
        return Some(path.to_path_buf());
    }
    #[cfg(windows)]
    {
        use std::ffi::OsString;
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;

        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        wide.push(0);
        let needed = unsafe { GetShortPathNameW(wide.as_ptr(), std::ptr::null_mut(), 0) };
        if needed == 0 {
            return None;
        }
        let mut result = vec![0_u16; needed as usize];
        let written = unsafe { GetShortPathNameW(wide.as_ptr(), result.as_mut_ptr(), needed) };
        if written == 0 || written >= needed {
            return None;
        }
        let alias = PathBuf::from(OsString::from_wide(&result[..written as usize]));
        alias
            .as_os_str()
            .to_string_lossy()
            .is_ascii()
            .then_some(alias)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{ascii_alias, path_hash, prepare_with_scratch};
    use std::fs;
    use std::path::{Path, PathBuf};

    #[test]
    fn ascii_paths_need_no_alias_and_cache_ids_are_stable() {
        assert_eq!(
            ascii_alias(Path::new("C:\\Voice\\model.gguf")),
            Some("C:\\Voice\\model.gguf".into())
        );
        assert_eq!(
            path_hash(Path::new("C:\\Voice")),
            path_hash(Path::new("C:\\Voice"))
        );
        assert_ne!(
            path_hash(Path::new("C:\\Voice")),
            path_hash(Path::new("C:\\Other"))
        );
    }

    #[test]
    fn chinese_installation_paths_are_exposed_to_cli_as_ascii() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("voice_compat_test_{}", std::process::id()));
        let source = root.join("中文目录");
        let scratch = root.join("ascii_cache");
        if !root.to_string_lossy().is_ascii() {
            return;
        }
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&scratch).unwrap();
        let executable = source.join("voice.exe");
        let model = source.join("model.gguf");
        let vad = source.join("vad.gguf");
        let dll = source.join("engine.dll");
        fs::write(&executable, b"program").unwrap();
        fs::write(&model, b"model").unwrap();
        fs::write(&vad, b"vad").unwrap();
        fs::write(&dll, b"dependency").unwrap();

        let paths = prepare_with_scratch(&executable, &model, Some(&vad), scratch).unwrap();
        assert!(paths.executable.to_string_lossy().is_ascii());
        assert!(paths.model.to_string_lossy().is_ascii());
        assert!(paths.vad.as_ref().unwrap().to_string_lossy().is_ascii());
        assert_eq!(fs::read(&paths.model).unwrap(), b"model");
        if paths.executable.parent() != executable.parent() {
            assert!(
                paths
                    .executable
                    .parent()
                    .unwrap()
                    .join("engine.dll")
                    .is_file()
            );
        }
        let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .canonicalize()
            .unwrap();
        let created = root.canonicalize().unwrap();
        assert!(created.starts_with(&target));
        assert!(
            created
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("voice_compat_test_")
        );
        fs::remove_dir_all(created).unwrap();
    }
}
