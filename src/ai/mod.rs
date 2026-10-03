//! DeepSeek 翻译请求与词库候选项。
//!
//! 网络请求只会由用户主动触发，并在后台线程执行。本模块不持久化 API Key。

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::json;
use std::time::Duration;

const ENDPOINT: &str = "https://api.deepseek.com/chat/completions";
const MODEL: &str = "deepseek-flash";

#[derive(Clone, Debug)]
pub(crate) struct AiCandidate {
    pub(crate) source: String,
    pub(crate) translated: String,
    pub(crate) source_language_code: String,
    pub(crate) language_code: String,
    pub(crate) direction: TranslationDirection,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TranslationDirection {
    Forward,
    Reverse,
}

#[derive(Deserialize)]
struct Completion {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: Message,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct Message {
    content: Option<String>,
}

#[derive(Deserialize)]
struct Translation {
    translation: String,
    direction: TranslationDirection,
}

pub(crate) fn translate(
    source: String,
    source_language_code: String,
    language_code: String,
    reverse_language_code: String,
    api_key: String,
) -> Result<AiCandidate> {
    let target_language = crate::translation_language::language_label(&language_code);
    let source_language = crate::translation_language::language_label(&source_language_code);
    let reverse_language = crate::translation_language::language_label(&reverse_language_code);
    if language_code.is_empty() || target_language == "未选择（默认词库）" {
        bail!("请先在设置中选择目标语言");
    }
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(25))
        .build()
        .context("无法初始化网络连接")?;
    let response = client
        .post(ENDPOINT)
        .bearer_auth(api_key)
        .json(&json!({
            "model": MODEL,
            "messages": [
                {"role": "system", "content": format!(
                    "你是双向短语翻译助手。用户通常输入{source_language}，需要翻译成{target_language}；如果输入本身主要是{target_language}，则反向翻译成{reverse_language}。先判断输入语言：反向时 direction 为 reverse，其余情况为 forward。保留原意、语气和标点，适合直接粘贴到聊天框。只返回 JSON 对象，例如 {{\"direction\":\"forward\",\"translation\":\"译文\"}}，不要解释。"
                )},
                {"role": "user", "content": source}
            ],
            "thinking": {"type": "disabled"},
            "response_format": {"type": "json_object"},
            "max_tokens": 256,
            "stream": false
        }))
        .send()
        .context("DeepSeek 请求失败，请检查网络连接")?;
    let status = response.status();
    if !status.is_success() {
        match status.as_u16() {
            401 => bail!("DeepSeek API Key 无效，请在设置中检查"),
            402 => bail!("DeepSeek 账户余额不足"),
            429 => bail!("DeepSeek 请求过于频繁，请稍后重试"),
            _ => bail!("DeepSeek 请求失败：HTTP {status}"),
        }
    }
    let completion: Completion = response.json().context("无法解析 DeepSeek 响应")?;
    parse_completion(
        completion,
        source,
        source_language_code,
        language_code,
        reverse_language_code,
    )
}

fn parse_completion(
    completion: Completion,
    source: String,
    source_language_code: String,
    language_code: String,
    reverse_language_code: String,
) -> Result<AiCandidate> {
    let choice = completion
        .choices
        .into_iter()
        .next()
        .context("DeepSeek 未返回译文")?;
    if choice.finish_reason.as_deref() == Some("length") {
        bail!("DeepSeek 译文不完整，请重试");
    }
    let content = choice.message.content.context("DeepSeek 未返回译文")?;
    let translation =
        serde_json::from_str::<Translation>(&content).context("DeepSeek 返回的译文格式无效")?;
    let translated = translation.translation.trim();
    if translated.is_empty() || translated.chars().count() > 500 {
        bail!("DeepSeek 返回的译文为空或过长");
    }
    Ok(AiCandidate {
        source,
        translated: translated.to_owned(),
        source_language_code: if translation.direction == TranslationDirection::Reverse {
            reverse_language_code
        } else {
            source_language_code
        },
        language_code,
        direction: translation.direction,
    })
}

#[cfg(test)]
mod tests {
    use super::{Completion, TranslationDirection, parse_completion};

    #[test]
    fn accepts_only_complete_structured_translation() {
        let valid: Completion = serde_json::from_str(
            r#"{"choices":[{"message":{"content":"{\"direction\":\"forward\",\"translation\":\"안녕하세요\"}"},"finish_reason":"stop"}]}"#,
        ).unwrap();
        assert_eq!(
            parse_completion(valid, "你好".into(), "cn".into(), "ko".into(), "cn".into())
                .unwrap()
                .translated,
            "안녕하세요"
        );
        let incomplete: Completion = serde_json::from_str(
            r#"{"choices":[{"message":{"content":"{\"direction\":\"forward\",\"translation\":\"안녕\"}"},"finish_reason":"length"}]}"#,
        ).unwrap();
        assert!(
            parse_completion(
                incomplete,
                "你好".into(),
                "cn".into(),
                "ko".into(),
                "cn".into()
            )
            .is_err()
        );
    }

    #[test]
    fn reverse_translation_uses_configured_original_language() {
        let response: Completion = serde_json::from_str(
            r#"{"choices":[{"message":{"content":"{\"direction\":\"reverse\",\"translation\":\"你好\"}"},"finish_reason":"stop"}]}"#,
        ).unwrap();
        let candidate = parse_completion(
            response,
            "안녕하세요".into(),
            "en".into(),
            "ko".into(),
            "cn".into(),
        )
        .unwrap();
        assert_eq!(candidate.direction, TranslationDirection::Reverse);
        assert_eq!(candidate.source_language_code, "cn");
        assert_eq!(candidate.translated, "你好");
    }
}
