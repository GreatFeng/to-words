//! 用户词库发现、整理与合并逻辑。
//!
//! 本模块负责识别程序目录及 `word_libraries` 中的词库，校验待合并文件必须位于
//! 允许目录内，并使用有序映射去除重复项、统计冲突，最后安全写入目标词库。

use crate::{prepare_word_library_file, project_directory, word_library_directory};
use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) struct MergeReport {
    pub(crate) source_files: usize,
    pub(crate) added: usize,
    pub(crate) duplicates: usize,
    pub(crate) conflicts: usize,
    pub(crate) total: usize,
}

pub(crate) fn organize_legacy_word_files() -> Result<usize> {
    let root = project_directory();
    let directory = word_library_directory();
    fs::create_dir_all(&directory)
        .with_context(|| format!("无法创建词库目录：{}", directory.display()))?;

    let mut copied = 0;
    for entry in
        fs::read_dir(&root).with_context(|| format!("无法读取程序目录：{}", root.display()))?
    {
        let path = entry?.path();
        if !path.is_file() || !is_legacy_word_file(&path) {
            continue;
        }
        let Some(file_name) = path.file_name() else {
            continue;
        };
        let target = directory.join(file_name);
        if !target.exists() {
            fs::copy(&path, &target).with_context(|| {
                format!("无法复制旧词库 {}：{}", path.display(), target.display())
            })?;
            copied += 1;
        }
    }
    Ok(copied)
}

pub(crate) fn list_word_library_files(target_file_name: &str) -> Result<Vec<String>> {
    organize_legacy_word_files()?;
    let directory = word_library_directory();
    let mut files = fs::read_dir(&directory)
        .with_context(|| format!("无法读取词库目录：{}", directory.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        })
        .filter_map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
        })
        .filter(|file_name| !file_name.eq_ignore_ascii_case(target_file_name))
        .collect::<Vec<_>>();
    files.sort_by_key(|file_name| file_name.to_ascii_lowercase());
    Ok(files)
}

pub(crate) fn merge_selected_word_files(
    target_file_name: &str,
    source_file_names: &[String],
) -> Result<MergeReport> {
    if source_file_names.is_empty() {
        bail!("请至少选择一个要合并的词库");
    }
    if !is_safe_file_name(target_file_name) {
        bail!("目标词库文件名无效");
    }

    let target = prepare_word_library_file(target_file_name)?;
    let directory = word_library_directory();
    let mut sources = Vec::with_capacity(source_file_names.len());
    for file_name in source_file_names {
        if !is_safe_file_name(file_name) || file_name.eq_ignore_ascii_case(target_file_name) {
            bail!("合并来源文件名无效：{file_name}");
        }
        let path = directory.join(file_name);
        if !path.is_file() {
            bail!("找不到要合并的词库：{file_name}");
        }
        sources.push(path);
    }

    let mut merged = if target.exists() {
        read_words(&target)?
    } else {
        BTreeMap::new()
    };
    let imports = sources
        .iter()
        .map(|path| read_words(path).map(|words| (path, words)))
        .collect::<Result<Vec<_>>>()?;
    let mut report = MergeReport {
        source_files: imports.len(),
        added: 0,
        duplicates: 0,
        conflicts: 0,
        total: 0,
    };

    for (_, words) in imports {
        for (key, value) in words {
            let key = key.trim().to_owned();
            let value = value.trim().to_owned();
            if key.is_empty() || value.is_empty() {
                continue;
            }
            match merged.get(&key) {
                None => report.added += 1,
                Some(existing) if existing == &value => report.duplicates += 1,
                Some(_) => report.conflicts += 1,
            }
            merged.insert(key, value);
        }
    }

    let formatted = serde_json::to_string_pretty(&merged).context("无法格式化合并后的词库")?;
    fs::write(&target, format!("{formatted}\n"))
        .with_context(|| format!("无法写入目标词库：{}", target.display()))?;
    report.total = merged.len();
    Ok(report)
}

fn is_legacy_word_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        && path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(|stem| stem.to_ascii_lowercase().starts_with("user_words"))
}

fn is_safe_file_name(file_name: &str) -> bool {
    let path = PathBuf::from(file_name);
    path.components().count() == 1
        && path.file_name().and_then(|name| name.to_str()) == Some(file_name)
        && path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
}

fn read_words(path: &Path) -> Result<BTreeMap<String, String>> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("无法读取词库文件：{}", path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("词库 JSON 格式错误：{}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::{is_legacy_word_file, is_safe_file_name};
    use std::path::Path;

    #[test]
    fn recognizes_legacy_word_files() {
        assert!(is_legacy_word_file(Path::new("user_words.json")));
        assert!(is_legacy_word_file(Path::new("user_words_cn_ko.JSON")));
        assert!(!is_legacy_word_file(Path::new("ui_config.json")));
    }

    #[test]
    fn selected_merge_files_must_stay_inside_the_library_directory() {
        assert!(is_safe_file_name("user_words_cn_ko.json"));
        assert!(!is_safe_file_name("../user_words.json"));
        assert!(!is_safe_file_name("notes.txt"));
    }
}
