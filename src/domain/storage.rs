//! SQLite 词库与设置持久化；旧 JSON 仅在首次建库时只读导入。

use crate::{project_directory, word_library_directory};
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) const DATABASE_FILE: &str = "to_words.db";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoredWord {
    pub(crate) key: String,
    pub(crate) value: String,
    pub(crate) pronunciation: String,
    pub(crate) created_at: i64,
    pub(crate) created_at_label: String,
}

pub(crate) fn database_path() -> PathBuf {
    project_directory().join(DATABASE_FILE)
}

pub(crate) fn open() -> Result<Connection> {
    let root = project_directory();
    open_at(&root.join(DATABASE_FILE), &root, &word_library_directory())
}

fn open_at(path: &Path, root: &Path, libraries: &Path) -> Result<Connection> {
    let mut connection = Connection::open(path)
        .with_context(|| format!("无法打开 SQLite 数据库：{}", path.display()))?;
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.execute_batch(
        "PRAGMA foreign_keys = ON;
         CREATE TABLE IF NOT EXISTS app_meta (
           name TEXT PRIMARY KEY, value TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS app_settings (
           name TEXT PRIMARY KEY, json_value TEXT NOT NULL,
           updated_at INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS word_libraries (
           name TEXT PRIMARY KEY,
           created_at INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS voice_cache_state (
           name TEXT PRIMARY KEY, json_value TEXT NOT NULL,
           updated_at INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS word_entries (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           library_name TEXT NOT NULL,
           key_text TEXT NOT NULL,
           value_text TEXT NOT NULL,
           pronunciation TEXT NOT NULL DEFAULT '',
           created_at INTEGER NOT NULL,
           updated_at INTEGER NOT NULL,
           origin TEXT NOT NULL DEFAULT 'manual',
           quality_status TEXT NOT NULL DEFAULT 'active'
             CHECK (quality_status IN ('active','quarantined')),
           quality_reason TEXT,
           UNIQUE(library_name, key_text)
         );
         CREATE INDEX IF NOT EXISTS idx_words_newest
           ON word_entries(library_name, quality_status, created_at DESC, id DESC);
         CREATE INDEX IF NOT EXISTS idx_words_lookup
           ON word_entries(library_name, quality_status, key_text);",
    )?;
    migrate_legacy(&mut connection, root, libraries)?;
    migrate_library_catalog(&mut connection, root, libraries)?;
    Ok(connection)
}

fn migrate_library_catalog(
    connection: &mut Connection,
    root: &Path,
    libraries: &Path,
) -> Result<()> {
    let done: Option<String> = connection
        .query_row(
            "SELECT value FROM app_meta WHERE name='library_catalog_imported'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if done.is_some() {
        return Ok(());
    }
    let mut names = HashSet::new();
    for directory in [libraries, root] {
        if !directory.is_dir() {
            continue;
        }
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if path.is_file() && is_legacy_library_name(name) {
                names.insert(name.to_owned());
            }
        }
    }
    let transaction = connection.transaction()?;
    transaction.execute(
        "INSERT OR IGNORE INTO word_libraries(name,created_at)
         SELECT library_name,MIN(created_at) FROM word_entries GROUP BY library_name",
        [],
    )?;
    for name in names {
        transaction.execute(
            "INSERT OR IGNORE INTO word_libraries(name,created_at) VALUES(?1,?2)",
            params![name, now_ms()],
        )?;
    }
    transaction.execute(
        "INSERT INTO app_meta(name,value) VALUES('library_catalog_imported',?1)",
        [now_ms().to_string()],
    )?;
    transaction.commit()?;
    Ok(())
}

fn migrate_legacy(connection: &mut Connection, root: &Path, libraries: &Path) -> Result<()> {
    let done: Option<String> = connection
        .query_row(
            "SELECT value FROM app_meta WHERE name='legacy_json_imported'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if done.is_some() {
        return Ok(());
    }

    // 先完整读取和校验，任一 JSON 错误都不会留下半完成的迁移标记。
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    for directory in [libraries, root] {
        if !directory.is_dir() {
            continue;
        }
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !path.is_file() || !is_legacy_library_name(name) || !seen.insert(name.to_owned()) {
                continue;
            }
            let content = fs::read_to_string(&path)
                .with_context(|| format!("无法读取旧词库：{}", path.display()))?;
            let words: BTreeMap<String, String> = serde_json::from_str(&content)
                .with_context(|| format!("旧词库 JSON 格式错误：{}", path.display()))?;
            let modified = fs::metadata(&path)
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map_or_else(now_ms, |time| time.as_millis() as i64);
            files.push((name.to_owned(), words, modified));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let settings = root.join("ui_config.json");
    let legacy_settings = if settings.is_file() {
        let content = fs::read_to_string(&settings)?;
        serde_json::from_str::<serde_json::Value>(&content)
            .with_context(|| format!("旧设置 JSON 格式错误：{}", settings.display()))?;
        Some(content)
    } else {
        None
    };
    let cache_path = root.join("voice_word_cache.json");
    let legacy_cache = if cache_path.is_file() {
        let content = fs::read_to_string(&cache_path)?;
        serde_json::from_str::<serde_json::Value>(&content)
            .with_context(|| format!("旧语音缓存 JSON 格式错误：{}", cache_path.display()))?;
        Some(content)
    } else {
        None
    };

    let transaction = connection.transaction()?;
    for (library, words, modified) in files {
        for (key, value) in words {
            let reason = obvious_error_reason(&library, &key, &value);
            transaction.execute(
                "INSERT OR IGNORE INTO word_entries
                 (library_name,key_text,value_text,created_at,updated_at,origin,quality_status,quality_reason)
                 VALUES (?1,?2,?3,?4,?4,'legacy_json',?5,?6)",
                params![library, key, value, modified,
                    if reason.is_some() { "quarantined" } else { "active" }, reason],
            )?;
        }
    }
    if let Some(settings) = legacy_settings {
        transaction.execute(
            "INSERT OR IGNORE INTO app_settings(name,json_value,updated_at) VALUES('ui_config',?1,?2)",
            params![settings, now_ms()],
        )?;
    }
    if let Some(cache) = legacy_cache {
        transaction.execute(
            "INSERT OR IGNORE INTO voice_cache_state(name,json_value,updated_at) VALUES('pending',?1,?2)",
            params![cache, now_ms()],
        )?;
    }
    transaction.execute(
        "INSERT INTO app_meta(name,value) VALUES('legacy_json_imported',?1)",
        [now_ms().to_string()],
    )?;
    transaction.commit()?;
    Ok(())
}

fn is_legacy_library_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    (lower == "user_words.json" || lower.starts_with("user_words_"))
        && lower.ends_with(".json")
        && !lower.contains("export")
}

/// 仅隔离机械上可以确认的坏数据；原始 JSON 不改动，其他可疑翻译留待人工复核。
pub(crate) fn obvious_error_reason(library: &str, key: &str, value: &str) -> Option<&'static str> {
    if key.trim().is_empty() || value.trim().is_empty() {
        return Some("empty_key_or_value");
    }
    if key.contains('\u{fffd}') || value.contains('\u{fffd}') {
        return Some("replacement_character");
    }
    let markers = [
        "[BLANK_AUDIO]",
        "<|nospeech|>",
        "[NO_SPEECH]",
        "[LOW_CONFIDENCE]",
    ];
    if markers
        .iter()
        .any(|marker| key.contains(marker) || value.contains(marker))
    {
        return Some("speech_error_marker");
    }
    if !key.chars().any(char::is_alphanumeric) || !value.chars().any(char::is_alphanumeric) {
        return Some("symbol_only");
    }
    // 历史中韩词库中的几条韩文字母碎片被错误映射为“PC认证”。
    if value == "PC认证"
        && key
            .chars()
            .all(|ch| ch == ' ' || ('\u{3130}'..='\u{318f}').contains(&ch))
    {
        return Some("verified_legacy_misrecognition");
    }
    if library == "user_words_cn_ko.json" && key == value && key.contains("翻应快哦") {
        return Some("verified_garbled_phrase");
    }
    None
}

pub(crate) fn load_word_map(library: &str) -> Result<BTreeMap<String, String>> {
    let connection = open()?;
    load_word_map_from(&connection, library)
}

fn load_word_map_from(connection: &Connection, library: &str) -> Result<BTreeMap<String, String>> {
    let mut statement = connection.prepare(
        "SELECT key_text,value_text FROM word_entries
         WHERE library_name=?1 AND quality_status='active' ORDER BY key_text",
    )?;
    let pairs = statement.query_map([library], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    pairs.map(|pair| pair.map_err(Into::into)).collect()
}

pub(crate) fn list_words_newest(library: &str) -> Result<Vec<StoredWord>> {
    let connection = open()?;
    let mut statement = connection.prepare(
        "SELECT key_text,value_text,pronunciation,created_at,
                datetime(created_at/1000,'unixepoch','localtime') FROM word_entries
         WHERE library_name=?1 AND quality_status='active'
         ORDER BY created_at DESC,id DESC",
    )?;
    let rows = statement.query_map([library], |row| {
        Ok(StoredWord {
            key: row.get(0)?,
            value: row.get(1)?,
            pronunciation: row.get(2)?,
            created_at: row.get(3)?,
            created_at_label: row.get(4)?,
        })
    })?;
    rows.map(|row| row.map_err(Into::into)).collect()
}

pub(crate) fn save_words(library: &str, words: &[StoredWord]) -> Result<()> {
    let mut connection = open()?;
    save_words_on(&mut connection, library, words)
}

fn save_words_on(connection: &mut Connection, library: &str, words: &[StoredWord]) -> Result<()> {
    let transaction = connection.transaction()?;
    let now = now_ms();
    transaction.execute(
        "INSERT OR IGNORE INTO word_libraries(name,created_at) VALUES(?1,?2)",
        params![library, now],
    )?;
    let existing = {
        let mut statement = transaction.prepare(
            "SELECT key_text FROM word_entries WHERE library_name=?1 AND quality_status='active'",
        )?;
        statement
            .query_map([library], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let intended: HashSet<_> = words
        .iter()
        .filter(|word| !word.key.trim().is_empty() && !word.value.trim().is_empty())
        .map(|word| word.key.trim())
        .collect();
    for key in existing {
        if !intended.contains(key.as_str()) {
            transaction.execute(
                "DELETE FROM word_entries WHERE library_name=?1 AND key_text=?2 AND quality_status='active'",
                params![library,key],
            )?;
        }
    }
    for word in words {
        let key = word.key.trim();
        let value = word.value.trim();
        if key.is_empty() || value.is_empty() {
            continue;
        }
        transaction.execute(
            "INSERT INTO word_entries
             (library_name,key_text,value_text,pronunciation,created_at,updated_at,origin)
             VALUES (?1,?2,?3,?4,?5,?6,'manual')
             ON CONFLICT(library_name,key_text) DO UPDATE SET
               value_text=excluded.value_text,
               pronunciation=excluded.pronunciation,
               updated_at=CASE WHEN value_text<>excluded.value_text OR pronunciation<>excluded.pronunciation
                 THEN excluded.updated_at ELSE word_entries.updated_at END,
               quality_status='active',quality_reason=NULL
             WHERE word_entries.value_text<>excluded.value_text
                OR word_entries.pronunciation<>excluded.pronunciation
                OR word_entries.quality_status<>'active'",
            params![library,key,value,word.pronunciation.trim(),
                if word.created_at > 0 {word.created_at} else {now},now],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

pub(crate) fn get_word(library: &str, key: &str) -> Result<Option<StoredWord>> {
    let connection = open()?;
    connection
        .query_row(
            "SELECT key_text,value_text,pronunciation,created_at,
                datetime(created_at/1000,'unixepoch','localtime') FROM word_entries
         WHERE library_name=?1 AND key_text=?2 AND quality_status='active'",
            params![library, key],
            |row| {
                Ok(StoredWord {
                    key: row.get(0)?,
                    value: row.get(1)?,
                    pronunciation: row.get(2)?,
                    created_at: row.get(3)?,
                    created_at_label: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}

pub(crate) fn is_quarantined(library: &str, key: &str) -> Result<bool> {
    let connection = open()?;
    let status: Option<String> = connection
        .query_row(
            "SELECT quality_status FROM word_entries WHERE library_name=?1 AND key_text=?2",
            params![library, key],
            |row| row.get(0),
        )
        .optional()?;
    Ok(status.as_deref() == Some("quarantined"))
}

pub(crate) fn insert_word_if_absent(
    library: &str,
    key: &str,
    value: &str,
    pronunciation: &str,
    origin: &str,
) -> Result<bool> {
    let mut connection = open()?;
    let now = now_ms();
    let transaction = connection.transaction()?;
    transaction.execute(
        "INSERT OR IGNORE INTO word_libraries(name,created_at) VALUES(?1,?2)",
        params![library, now],
    )?;
    let changed = transaction.execute(
        "INSERT OR IGNORE INTO word_entries
         (library_name,key_text,value_text,pronunciation,created_at,updated_at,origin)
         VALUES(?1,?2,?3,?4,?5,?5,?6)",
        params![library, key, value, pronunciation, now, origin],
    )?;
    if changed == 0 && !pronunciation.is_empty() {
        transaction.execute(
            "UPDATE word_entries SET pronunciation=?3,updated_at=?4
             WHERE library_name=?1 AND key_text=?2 AND value_text=?5 AND pronunciation='' AND quality_status='active'",
            params![library,key,pronunciation,now,value],
        )?;
    }
    transaction.commit()?;
    Ok(changed != 0)
}

pub(crate) fn load_setting(name: &str) -> Result<Option<String>> {
    let connection = open()?;
    connection
        .query_row(
            "SELECT json_value FROM app_settings WHERE name=?1",
            [name],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
}

pub(crate) fn save_setting(name: &str, json: &str) -> Result<()> {
    serde_json::from_str::<serde_json::Value>(json).context("设置数据不是有效 JSON")?;
    let connection = open()?;
    connection.execute(
        "INSERT INTO app_settings(name,json_value,updated_at) VALUES(?1,?2,?3)
         ON CONFLICT(name) DO UPDATE SET json_value=excluded.json_value,updated_at=excluded.updated_at",
        params![name,json,now_ms()],
    )?;
    Ok(())
}

pub(crate) fn load_voice_cache_at(path: &Path) -> Result<Option<String>> {
    let root = path.parent().context("语音缓存数据库路径缺少父目录")?;
    let connection = open_at(path, root, &root.join("word_libraries"))?;
    connection
        .query_row(
            "SELECT json_value FROM voice_cache_state WHERE name='pending'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
}

pub(crate) fn save_voice_cache_at(path: &Path, json: &str) -> Result<()> {
    serde_json::from_str::<serde_json::Value>(json).context("语音缓存不是有效 JSON")?;
    let root = path.parent().context("语音缓存数据库路径缺少父目录")?;
    let connection = open_at(path, root, &root.join("word_libraries"))?;
    connection.execute(
        "INSERT INTO voice_cache_state(name,json_value,updated_at) VALUES('pending',?1,?2)
         ON CONFLICT(name) DO UPDATE SET json_value=excluded.json_value,updated_at=excluded.updated_at",
        params![json, now_ms()],
    )?;
    Ok(())
}

pub(crate) fn quarantine_count() -> Result<i64> {
    let connection = open()?;
    connection
        .query_row(
            "SELECT COUNT(*) FROM word_entries WHERE quality_status='quarantined'",
            [],
            |row| row.get(0),
        )
        .map_err(Into::into)
}

pub(crate) fn library_statistics() -> Result<Vec<(String, i64, i64)>> {
    let connection = open()?;
    let mut statement = connection.prepare(
        "SELECT libraries.name,
                SUM(CASE WHEN entries.quality_status='active' THEN 1 ELSE 0 END),
                SUM(CASE WHEN entries.quality_status='quarantined' THEN 1 ELSE 0 END)
         FROM word_libraries AS libraries
         LEFT JOIN word_entries AS entries ON entries.library_name=libraries.name
         GROUP BY libraries.name ORDER BY libraries.name",
    )?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    rows.map(|row| row.map_err(Into::into)).collect()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_is_idempotent_and_quarantines_only_verified_errors() {
        let root = std::env::temp_dir().join(format!(
            "to_words_sqlite_{}_{}",
            std::process::id(),
            now_ms()
        ));
        let libraries = root.join("word_libraries");
        fs::create_dir_all(&libraries).unwrap();
        fs::write(
            libraries.join("user_words_cn_ko.json"),
            r#"{"안녕하세요":"你好","ㄶ":"PC认证","ㅋㅋ":"哈哈"}"#,
        )
        .unwrap();
        fs::write(libraries.join("user_words_ko_ko.json"), "{}").unwrap();
        fs::write(root.join("ui_config.json"), r#"{"voice_keep_input":true}"#).unwrap();
        fs::write(root.join("voice_word_cache.json"), r#"{"records":[]}"#).unwrap();
        let path = root.join(DATABASE_FILE);
        let connection = open_at(&path, &root, &libraries).unwrap();
        assert_eq!(
            load_word_map_from(&connection, "user_words_cn_ko.json")
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM word_entries WHERE quality_status='quarantined'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT json_value FROM app_settings WHERE name='ui_config'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            r#"{"voice_keep_input":true}"#
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT json_value FROM voice_cache_state WHERE name='pending'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            r#"{"records":[]}"#
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM word_libraries WHERE name='user_words_ko_ko.json'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        drop(connection);
        let connection = open_at(&path, &root, &libraries).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM word_entries", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            3
        );
        let old_timestamp = connection
            .query_row(
                "SELECT created_at FROM word_entries WHERE library_name='user_words_cn_ko.json' AND key_text='안녕하세요'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        drop(connection);
        let mut connection = open_at(&path, &root, &libraries).unwrap();
        save_words_on(
            &mut connection,
            "user_words_cn_ko.json",
            &[
                StoredWord {
                    key: "안녕하세요".into(),
                    value: "你好".into(),
                    pronunciation: "annyeonghaseyo".into(),
                    created_at: old_timestamp,
                    created_at_label: String::new(),
                },
                StoredWord {
                    key: "감사합니다".into(),
                    value: "谢谢".into(),
                    pronunciation: "gamsahamnida".into(),
                    created_at: 0,
                    created_at_label: String::new(),
                },
            ],
        )
        .unwrap();
        let newest: Vec<_> = {
            let mut statement = connection
                .prepare("SELECT key_text FROM word_entries WHERE quality_status='active' ORDER BY created_at DESC,id DESC")
                .unwrap();
            statement
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };
        assert_eq!(newest, vec!["감사합니다", "안녕하세요"]);
        assert_eq!(
            connection
                .query_row(
                    "SELECT pronunciation FROM word_entries WHERE key_text='안녕하세요'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "annyeonghaseyo"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT created_at FROM word_entries WHERE key_text='안녕하세요'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            old_timestamp
        );
        drop(connection);
        fs::remove_file(libraries.join("user_words_cn_ko.json")).unwrap();
        fs::remove_file(libraries.join("user_words_ko_ko.json")).unwrap();
        fs::remove_file(root.join("ui_config.json")).unwrap();
        fs::remove_file(root.join("voice_word_cache.json")).unwrap();
        fs::remove_file(path).unwrap();
        fs::remove_dir(libraries).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
