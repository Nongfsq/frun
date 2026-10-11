use crate::index::Item;
use std::collections::HashMap;

/// 返回按相关度排序的条目下标。空查询时列出最常用的条目。
pub fn run(items: &[Item], usage: &HashMap<String, u32>, query: &str, max: usize) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    let mut hits: Vec<(i32, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let used = usage.get(&item.target).copied().unwrap_or(0).min(50) as i32;
            let score = if q.is_empty() {
                used
            } else {
                match item.keys.iter().map(|k| score(k, &q)).max() {
                    Some(m) if m > 0 => m + used * 5,
                    _ => 0,
                }
            };
            (score > 0).then_some((score, i))
        })
        .collect();
    hits.sort_unstable_by(|a, b| {
        b.0.cmp(&a.0).then_with(|| items[a.1].name.len().cmp(&items[b.1].name.len()))
    });
    hits.truncate(max);
    hits.into_iter().map(|h| h.1).collect()
}

/// 前缀 > 词首 > 子串 > 子序列；0 表示不匹配。
fn score(key: &str, q: &str) -> i32 {
    if let Some(pos) = key.find(q) {
        let word_start = key[..pos].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        let base = if pos == 0 {
            1000
        } else if word_start {
            800
        } else {
            600
        };
        return base - (key.len() as i32).min(100);
    }
    // 子序列匹配：字符之间跳过太多就不算，避免毫不相干的结果
    let mut rest = key.chars();
    let (mut lead, mut gaps, mut started) = (0, 0, false);
    for c in q.chars() {
        loop {
            match rest.next() {
                Some(k) if k == c => break,
                Some(_) if started => gaps += 1,
                Some(_) => lead += 1,
                None => return 0,
            }
        }
        started = true;
    }
    if gaps > q.chars().count() as i32 {
        return 0;
    }
    (300 - gaps * 10 - lead).max(1)
}
