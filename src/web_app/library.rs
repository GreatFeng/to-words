//! 词库搜索、词条保存、导出及语言列表命令。

use super::*;

#[tauri::command]
pub(super) fn list_languages() -> Vec<LanguageOption> {
    translation_language::LANGUAGES
        .iter()
        .map(|item| LanguageOption {
            code: item.code,
            name: item.label,
        })
        .collect()
}

#[tauri::command]
pub(super) fn search_words(
    query: String,
    page: usize,
    page_size: usize,
    state: State<'_, WebState>,
) -> Result<SearchPage, String> {
    let config = state.config.lock().map_err(error)?.clone();
    let query = query.trim();
    if query.is_empty() {
        return Ok(SearchPage {
            hits: Vec::new(),
            total: 0,
            language_mismatch: false,
        });
    }
    if let Some(language) = translation_language::same_language_pair(
        &config.source_language,
        &config.translation_language,
    ) {
        let valid = translation_language::accepts_same_language_text(query, language);
        return Ok(SearchPage {
            hits: if valid && page == 0 {
                vec![WebHit {
                    key: query.to_owned(),
                    value: String::new(),
                    pronunciation: String::new(),
                    library: String::new(),
                    rank: "exact",
                }]
            } else {
                Vec::new()
            },
            total: usize::from(valid),
            language_mismatch: !valid,
        });
    }
    let source = if config.source_language == "auto" {
        translation_language::detect_source_language(query).unwrap_or("cn")
    } else {
        &config.source_language
    };
    let library = library_name(source, &config.translation_language);
    let mut search = state.search.lock().map_err(error)?;
    if search.library != library {
        search.index = WordIndex::from_rows(storage::load_search_rows(&library).map_err(error)?);
        search.library = library.clone();
        search.previous_query.clear();
        search.previous_hits.clear();
    }
    let hits = search
        .index
        .search(query, &search.previous_query, &search.previous_hits);
    let total = hits.len();
    let start = page.saturating_mul(page_size.clamp(1, 100));
    let page_matches: Vec<_> = hits
        .iter()
        .skip(start)
        .take(page_size.clamp(1, 100))
        .collect();
    let keys: Vec<_> = page_matches
        .iter()
        .filter_map(|hit| {
            search
                .index
                .get(hit.entry_id)
                .map(|entry| entry.key.as_str())
        })
        .collect();
    let pronunciations = storage::load_search_pronunciations(&library, &keys).map_err(error)?;
    let page_hits = page_matches
        .into_iter()
        .filter_map(|hit| {
            search.index.get(hit.entry_id).map(|entry| WebHit {
                key: entry.key.clone(),
                value: entry.value.clone(),
                pronunciation: pronunciations.get(&entry.key).cloned().unwrap_or_default(),
                library: library.clone(),
                rank: match hit.rank {
                    MatchRank::Exact => "exact",
                    MatchRank::Prefix => "prefix",
                    MatchRank::Contains => "contains",
                },
            })
        })
        .collect();
    search.previous_query = query.to_string();
    search.previous_hits = hits;
    Ok(SearchPage {
        hits: page_hits,
        total,
        language_mismatch: false,
    })
}

#[tauri::command]
pub(super) fn list_words(source: String, target: String) -> Result<Vec<StoredWord>, String> {
    storage::list_words_newest(&library_name(&source, &target)).map_err(error)
}

#[tauri::command]
pub(super) fn save_words(
    source: String,
    target: String,
    rows: Vec<StoredWord>,
    state: State<'_, WebState>,
) -> Result<(), String> {
    let library = library_name(&source, &target);
    storage::save_words(&library, &rows).map_err(error)?;
    let mut search = state.search.lock().map_err(error)?;
    search.library.clear();
    search.previous_query.clear();
    search.previous_hits.clear();
    Ok(())
}

#[tauri::command]
pub(super) fn export_words(name: String, rows: Vec<StoredWord>) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\', ':'])
        || name.chars().any(char::is_control)
    {
        return Err("文件名无效".to_string());
    }
    let name = if name.to_ascii_lowercase().ends_with(".json") {
        name.to_owned()
    } else {
        format!("{name}.json")
    };
    let path = project_directory().join(name);
    if path.exists() {
        return Err(format!("文件已存在，请使用其他名称：{}", path.display()));
    }
    let mut words = std::collections::BTreeMap::new();
    for row in rows {
        if !row.key.trim().is_empty() && !row.value.trim().is_empty() {
            words.insert(row.key, row.value);
        }
    }
    let content = serde_json::to_vec_pretty(&words).map_err(error)?;
    std::fs::write(&path, content).map_err(error)?;
    Ok(path.display().to_string())
}
