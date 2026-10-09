//! 用户词库发现、整理与合并逻辑。
//!
//! 本模块负责识别程序目录及 `word_libraries` 中的词库，校验待合并文件必须位于
//! 允许目录内，并使用有序映射去除重复项、统计冲突，最后安全写入目标词库。

use crate::domain::{ai_word_save, storage};
use crate::{project_directory, word_library_directory};
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

pub(crate) fn list_word_library_files(target_file_name: &str) -> Result<Vec<String>> {
    let directory = word_library_directory();
    fs::create_dir_all(&directory)?;
    let mut files = storage::list_library_names()?;
    files.extend(
        fs::read_dir(&directory)
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
            .collect::<Vec<_>>(),
    );
    files.extend(
        fs::read_dir(project_directory())?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.is_file() && is_legacy_word_file(path))
            .filter_map(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_owned)
            }),
    );
    files.retain(|name| !name.eq_ignore_ascii_case(target_file_name));
    files.sort_by_key(|file_name| file_name.to_ascii_lowercase());
    files.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
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

    let directory = word_library_directory();
    let mut imports = Vec::with_capacity(source_file_names.len());
    for file_name in source_file_names {
        if !is_safe_file_name(file_name) || file_name.eq_ignore_ascii_case(target_file_name) {
            bail!("合并来源文件名无效：{file_name}");
        }
        let preferred = directory.join(file_name);
        let path = if preferred.is_file() {
            preferred
        } else {
            project_directory().join(file_name)
        };
        let words = if path.is_file() {
            read_words(&path)?
        } else if storage::list_library_names()?
            .iter()
            .any(|name| name == file_name)
        {
            storage::load_word_map(file_name)?
        } else {
            bail!("找不到要合并的词库：{file_name}");
        };
        imports.push(words);
    }

    let _guard = ai_word_save::lock_word_library();
    let mut merged = storage::load_word_map(target_file_name)?;
    let mut report = MergeReport {
        source_files: imports.len(),
        added: 0,
        duplicates: 0,
        conflicts: 0,
        total: 0,
    };

    for words in imports {
        for (key, value) in words {
            let key = key.trim().to_owned();
            let value = value.trim().to_owned();
            if storage::obvious_error_reason(target_file_name, &key, &value).is_some() {
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

    storage::save_word_map(target_file_name, &merged)?;
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
