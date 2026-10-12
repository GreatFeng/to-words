//! 对较长的语音识别文本做保守断句；不在词语中间强行切分。

const MIN_CHARS: usize = 16;
const TARGET_CHARS: usize = 40;
const MAX_CHARS: usize = 60;

pub(crate) fn split_voice_sentences(text: &str) -> Vec<String> {
    let text = text.trim();
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= MAX_CHARS {
        return vec![text.to_owned()];
    }

    let mut segments = Vec::new();
    let mut start = 0;
    while chars.len() - start > MAX_CHARS {
        let remaining = chars.len() - start;
        let target = if remaining < TARGET_CHARS + MIN_CHARS {
            remaining / 2
        } else {
            TARGET_CHARS
        };
        let last_end = (start + MAX_CHARS).min(chars.len() - MIN_CHARS);
        let boundary = (start + MIN_CHARS..=last_end)
            .filter_map(|end| {
                let priority = boundary_priority(&chars, end)?;
                let distance = (end - start).abs_diff(target);
                Some((distance + priority * 8, end))
            })
            .min_by_key(|(score, _)| *score)
            .map(|(_, end)| end);
        let Some(end) = boundary else {
            break;
        };
        let segment: String = chars[start..end].iter().collect();
        if !segment.trim().is_empty() {
            segments.push(segment.trim().to_owned());
        }
        start = end;
    }
    let remainder: String = chars[start..].iter().collect();
    if !remainder.trim().is_empty() {
        segments.push(remainder.trim().to_owned());
    }
    segments
}

fn boundary_priority(chars: &[char], end: usize) -> Option<usize> {
    let previous = chars[end - 1];
    let next = chars.get(end).copied();
    if previous == '.'
        && chars
            .get(end.wrapping_sub(2))
            .is_some_and(char::is_ascii_digit)
        && next.is_some_and(|value| value.is_ascii_digit())
    {
        return None;
    }
    if matches!(previous, '。' | '！' | '？' | '.' | '!' | '?' | '\n') {
        return Some(0);
    }
    if matches!(previous, '，' | ',' | '；' | ';' | '、' | ':') {
        return Some(1);
    }
    if previous.is_whitespace() {
        return Some(2);
    }
    const CONNECTORS: &[&str] = &[
        "但是", "不过", "然后", "所以", "因为", "如果", "而且", "并且", "可是",
    ];
    let following: String = chars[end..].iter().take(3).collect();
    CONNECTORS
        .iter()
        .any(|word| following.starts_with(word))
        .then_some(3)
}

#[cfg(test)]
mod tests {
    use super::split_voice_sentences;

    #[test]
    fn short_voice_text_is_unchanged() {
        assert_eq!(
            split_voice_sentences("你好，今天一起玩吧。"),
            vec!["你好，今天一起玩吧。"]
        );
    }

    #[test]
    fn long_text_splits_at_natural_punctuation_without_losing_words() {
        let sentence = "今天先去完成任务，接着我们一起回城整理装备。";
        let text = sentence.repeat(5);
        let parts = split_voice_sentences(&text);
        assert!(parts.len() > 1);
        assert_eq!(parts.concat(), text);
        assert!(parts.iter().all(|part| part.chars().count() <= 80));
    }

    #[test]
    fn long_korean_text_can_split_at_word_spaces() {
        let text = "오늘은 퀘스트를 먼저 끝내고 같이 마을로 돌아가서 장비를 정리한 다음에 다시 던전으로 갈까요 ".repeat(3);
        let parts = split_voice_sentences(&text);
        assert!(parts.len() > 1);
        assert_eq!(parts.join(" "), text.trim());
    }

    #[test]
    fn text_without_safe_boundaries_is_not_cut_mid_word() {
        let text = "话".repeat(120);
        assert_eq!(split_voice_sentences(&text), vec![text]);
    }
}
