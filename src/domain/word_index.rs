//! 高性能词库索引与查询逻辑。
//!
//! 词条以按 key 排序的 `Vec<WordEntry>` 保存，查询结果只保存 `EntryId`。
//! `WordIndex` 同时维护字符二元倒排索引，并在输入增长时复用上一次候选集；
//! 搜索会匹配 key 和 value，再按照精确、前缀和包含等级稳定排序。

use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct EntryId(pub(crate) usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MatchRank {
    Exact,
    Prefix,
    Contains,
}

impl MatchRank {
    fn bucket(self) -> usize {
        match self {
            Self::Exact => 0,
            Self::Prefix => 1,
            Self::Contains => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SearchHit {
    pub(crate) entry_id: EntryId,
    pub(crate) rank: MatchRank,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WordEntry {
    pub(crate) key: String,
    pub(crate) value: String,
}

#[derive(Default)]
pub(crate) struct WordIndex {
    entries: Vec<WordEntry>,
    bigrams: HashMap<(char, char), Vec<EntryId>>,
}

impl WordIndex {
    pub(crate) fn new(mut entries: Vec<WordEntry>) -> Self {
        entries.sort_unstable_by(|left, right| left.key.cmp(&right.key));
        let mut bigrams: HashMap<(char, char), Vec<EntryId>> = HashMap::new();
        for (index, entry) in entries.iter().enumerate() {
            let id = EntryId(index);
            for bigram in unique_entry_bigrams(entry) {
                bigrams.entry(bigram).or_default().push(id);
            }
        }
        Self { entries, bigrams }
    }

    pub(crate) fn from_map(words: HashMap<String, String>) -> Self {
        Self::new(
            words
                .into_iter()
                .map(|(key, value)| WordEntry { key, value })
                .collect(),
        )
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn get(&self, id: EntryId) -> Option<&WordEntry> {
        self.entries.get(id.0)
    }

    pub(crate) fn search(
        &self,
        query: &str,
        previous_query: &str,
        previous_matches: &[SearchHit],
    ) -> Vec<SearchHit> {
        if query.is_empty() {
            return Vec::new();
        }

        let candidates = if !previous_query.is_empty()
            && query.starts_with(previous_query)
            && query.len() > previous_query.len()
        {
            let mut candidates: Vec<EntryId> =
                previous_matches.iter().map(|hit| hit.entry_id).collect();
            candidates.sort_unstable();
            candidates
        } else {
            self.index_candidates(query)
        };

        self.rank_candidates(candidates, query)
    }

    fn rank_candidates(&self, candidates: Vec<EntryId>, query: &str) -> Vec<SearchHit> {
        let mut buckets: [Vec<SearchHit>; 3] = std::array::from_fn(|_| Vec::new());
        for entry_id in candidates {
            let Some(entry) = self.get(entry_id) else {
                continue;
            };
            let Some(rank) = match_rank(entry, query) else {
                continue;
            };
            buckets[rank.bucket()].push(SearchHit { entry_id, rank });
        }
        buckets.into_iter().flatten().collect()
    }

    fn index_candidates(&self, query: &str) -> Vec<EntryId> {
        let bigrams = unique_bigrams(query);
        if bigrams.is_empty() {
            return (0..self.entries.len()).map(EntryId).collect();
        }

        let mut posting_lists = Vec::with_capacity(bigrams.len());
        for bigram in bigrams {
            let Some(postings) = self.bigrams.get(&bigram) else {
                return Vec::new();
            };
            posting_lists.push(postings.as_slice());
        }
        posting_lists.sort_unstable_by_key(|postings| postings.len());

        let mut candidates = posting_lists[0].to_vec();
        for postings in posting_lists.into_iter().skip(1) {
            candidates.retain(|id| postings.binary_search(id).is_ok());
            if candidates.is_empty() {
                break;
            }
        }
        candidates
    }
}

fn unique_bigrams(text: &str) -> Vec<(char, char)> {
    let characters: Vec<char> = text.chars().collect();
    let mut seen = HashSet::new();
    characters
        .windows(2)
        .filter_map(|pair| {
            seen.insert((pair[0], pair[1]))
                .then_some((pair[0], pair[1]))
        })
        .collect()
}

fn unique_entry_bigrams(entry: &WordEntry) -> Vec<(char, char)> {
    let mut bigrams = unique_bigrams(&entry.key);
    let mut seen: HashSet<(char, char)> = bigrams.iter().copied().collect();
    bigrams.extend(
        unique_bigrams(&entry.value)
            .into_iter()
            .filter(|bigram| seen.insert(*bigram)),
    );
    bigrams
}

fn match_rank(entry: &WordEntry, query: &str) -> Option<MatchRank> {
    if entry.key == query || entry.value == query {
        Some(MatchRank::Exact)
    } else if entry.key.starts_with(query) || entry.value.starts_with(query) {
        Some(MatchRank::Prefix)
    } else if entry.key.contains(query) || entry.value.contains(query) {
        Some(MatchRank::Contains)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{EntryId, MatchRank, SearchHit, WordEntry, WordIndex};

    fn sample_index() -> WordIndex {
        WordIndex::new(vec![
            WordEntry {
                key: "greet".to_string(),
                value: "你好，朋友".to_string(),
            },
            WordEntry {
                key: "bye".to_string(),
                value: "再见".to_string(),
            },
            WordEntry {
                key: "formal".to_string(),
                value: "向你问好：你好".to_string(),
            },
        ])
    }

    #[test]
    fn entries_and_results_are_sorted_by_key() {
        let index = sample_index();
        let matches = index.search("你好", "", &[]);
        assert_eq!(
            matches,
            vec![
                SearchHit {
                    entry_id: EntryId(2),
                    rank: MatchRank::Prefix,
                },
                SearchHit {
                    entry_id: EntryId(1),
                    rank: MatchRank::Contains,
                },
            ]
        );
        assert_eq!(index.get(matches[0].entry_id).unwrap().key, "greet");
        assert_eq!(index.get(matches[1].entry_id).unwrap().key, "formal");
    }

    #[test]
    fn growing_query_filters_the_previous_result_set() {
        let index = sample_index();
        let previous = index.search("你", "", &[]);
        let incremental = index.search("你好", "你", &previous);
        let full = index.search("你好", "", &[]);
        assert_eq!(incremental, full);
    }

    #[test]
    fn shrinking_query_returns_to_the_index_instead_of_filtering_old_results() {
        let index = WordIndex::new(vec![
            WordEntry {
                key: "hello".to_string(),
                value: "你好".to_string(),
            },
            WordEntry {
                key: "plural".to_string(),
                value: "你们".to_string(),
            },
        ]);
        let previous = index.search("你好", "", &[]);
        let after_backspace = index.search("你", "你好", &previous);
        assert_eq!(
            after_backspace
                .iter()
                .map(|hit| hit.entry_id)
                .collect::<Vec<_>>(),
            vec![EntryId(0), EntryId(1)]
        );
    }

    #[test]
    fn bigram_index_still_requires_the_full_substring_to_match() {
        let index = WordIndex::new(vec![WordEntry {
            key: "false-positive".to_string(),
            value: "甲乙X乙丙".to_string(),
        }]);
        assert!(index.search("甲乙丙", "", &[]).is_empty());
    }

    #[test]
    fn matches_both_fields_and_orders_hits_by_match_rank() {
        let index = WordIndex::new(vec![
            WordEntry {
                key: "가나다".to_string(),
                value: "问候语".to_string(),
            },
            WordEntry {
                key: "包含가나".to_string(),
                value: "其他".to_string(),
            },
            WordEntry {
                key: "另一个".to_string(),
                value: "가나".to_string(),
            },
        ]);
        let matches = index.search("가나", "", &[]);
        assert_eq!(
            matches.iter().map(|hit| hit.rank).collect::<Vec<_>>(),
            vec![MatchRank::Exact, MatchRank::Prefix, MatchRank::Contains]
        );
        assert_eq!(
            matches
                .iter()
                .map(|hit| index.get(hit.entry_id).unwrap().key.as_str())
                .collect::<Vec<_>>(),
            vec!["另一个", "가나다", "包含가나"]
        );
    }
}
