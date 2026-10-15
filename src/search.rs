use crate::{
    config::Store,
    index::{Entry, Group, Index, Key, Kind},
};
use std::collections::{HashMap, HashSet};

pub struct Hit {
    pub group: usize,
    /// 显示名里每个字符是否命中，用于着色；空表示不标
    pub marks: Vec<bool>,
    /// 同一个程序目录里收在它后面的程序数
    pub extra: usize,
}

/// 没有使用记录时的起步列表：按启动目标里的标识找，找不到的跳过。
const STARTERS: [&str; 6] = [
    "windows.immersivecontrolpanel",
    "microsoft.windows.explorer",
    "microsoft.windowsterminal",
    "windowscalculator",
    "msedge",
    "windowsnotepad",
];

/// 按相关度排序。空查询时列出最常用的程序。
///
/// 一个程序目录只占一行：目录里的其他程序收在主程序后面。
/// 输入只对上其中一个时直接给出那一个；好几个都以输入开头时给出主程序（vbox -> VirtualBox）；
/// 其余情况给出对得最好的那一个。用过的不再收起。
pub fn run(index: &Index, store: &Store, query: &str, max: usize) -> Vec<Hit> {
    let q: Vec<char> = query.trim().to_lowercase().chars().collect();
    let pick = store.picks.get(&q.iter().collect::<String>());
    let mut hits: Vec<(i32, usize, Vec<bool>)> = Vec::new();
    // 收在主程序后面的：(匹配的档次, 命中)
    let mut behind: HashMap<usize, Vec<(i32, (i32, usize, Vec<bool>))>> = HashMap::new();
    for (gi, g) in index.groups.iter().enumerate() {
        let used = g.usage(store).min(50) as i32;
        if q.is_empty() {
            if used > 0 {
                hits.push((used, gi, Vec::new()));
            }
            continue;
        }
        let Some((score, marks)) = matched(g, &q) else { continue };
        // 同一段输入上次选了谁，这次它排第一
        let picked = pick.is_some_and(|t| g.members.iter().any(|m| &m.target == t));
        let hit = (score + used * 5 + if picked { 5000 } else { 0 } - buried(g.entry()), gi, marks);
        match g.owner {
            Some(owner) if used == 0 => behind.entry(owner).or_default().push((class(score), hit)),
            _ => hits.push(hit),
        }
    }
    let shown: HashSet<usize> = hits.iter().map(|h| h.1).collect();
    for (owner, mut list) in behind {
        if shown.contains(&owner) {
            continue;
        }
        let top = list.iter().map(|h| h.0).max().unwrap_or(0);
        list.retain(|h| h.0 == top);
        // 分数高的在前，同分时名字短的在前
        list.sort_by(|a, b| b.1.0.cmp(&a.1.0).then_with(|| a.1.1.cmp(&b.1.1)));
        if list.len() > 1 && top == PREFIX {
            // 一家子都以输入开头：要的多半是这个程序本身
            hits.push((list[0].1.0 - 1, owner, Vec::new()));
        } else {
            // 否则给出对得最好的那一个，名字和输入对得上
            hits.push(list.remove(0).1);
        }
    }
    // 一次都没用过：列出这台电脑上最可能要开的几个系统程序
    if q.is_empty() && hits.is_empty() {
        for (rank, id) in STARTERS.iter().enumerate() {
            let found = index.groups.iter().position(|g| g.members.iter().any(|m| m.target.to_lowercase().contains(id)));
            if let Some(gi) = found {
                hits.push((100 - rank as i32, gi, Vec::new()));
            }
        }
        hits.dedup_by_key(|h| h.1);
    }
    let len = |gi: usize| index.groups[gi].entry().name.len();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| len(a.1).cmp(&len(b.1))).then_with(|| a.1.cmp(&b.1)));
    hits.truncate(max);
    // 行右侧的数量和 → 展开后看到的一致：有输入时只数对得上的
    let extra = |group: usize| index.folded(group, store).filter(|&h| q.is_empty() || matched(&index.groups[h], &q).is_some()).count();
    hits.into_iter().map(|(_, group, marks)| Hit { group, marks, extra: extra(group) }).collect()
}

/// 匹配的档次：前缀 3、词首 2、子串 1、子序列 0。
fn class(score: i32) -> i32 {
    match score {
        900.. => PREFIX,
        700.. => 2,
        500.. => 1,
        _ => 0,
    }
}
const PREFIX: i32 = 3;

/// 这一组对不对得上输入：分数，以及显示名里哪些字符命中。
pub fn matched(g: &Group, q: &[char]) -> Option<(i32, Vec<bool>)> {
    let mut best: Option<(i32, Vec<usize>, &Key)> = None;
    for key in &g.keys {
        if let Some((score, at)) = score(&key.text, q)
            && best.as_ref().is_none_or(|b| score > b.0)
        {
            best = Some((score, at, key));
        }
    }
    let (score, at, key) = best?;
    let mut marks = vec![false; g.entry().name.chars().count()];
    for i in at {
        if let Some(m) = marks.get_mut(key.map[i] as usize) {
            *m = true;
        }
    }
    Some((score, marks))
}

/// 目录里埋得越深的文件越靠后：同样匹配时，系统里装好的程序和放在目录浅处的工具优先。
fn buried(e: &Entry) -> i32 {
    if e.kind != Kind::Dir {
        return 0;
    }
    let levels = |s: &str| s.matches(['\\', '/']).count() as i32;
    (levels(&e.target) - levels(e.root.trim_end_matches(['\\', '/'])) - 1).clamp(0, 6) * 20
}

/// 前缀 > 词首 > 子串 > 子序列。返回分数和命中的键内位置；None 表示不匹配。
fn score(key: &[char], q: &[char]) -> Option<(i32, Vec<usize>)> {
    if q.len() <= key.len()
        && let Some(pos) = key.windows(q.len()).position(|w| w == q)
    {
        let word_start = pos == 0 || !key[pos - 1].is_alphanumeric();
        let base = if pos == 0 {
            1000
        } else if word_start {
            800
        } else {
            600
        };
        return Some((base - (key.len() as i32).min(100), (pos..pos + q.len()).collect()));
    }
    // 子序列匹配：字符之间跳过太多就不算，避免毫不相干的结果
    let (mut i, mut lead, mut gaps) = (0, 0, 0);
    let mut at = Vec::with_capacity(q.len());
    for &c in q {
        loop {
            let k = *key.get(i)?;
            i += 1;
            if k == c {
                at.push(i - 1);
                break;
            }
            if at.is_empty() {
                lead += 1;
            } else {
                gaps += 1;
            }
        }
    }
    if gaps > q.len() as i32 {
        return None;
    }
    Some(((300 - gaps * 10 - lead).max(1), at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::keys;

    fn index(names: &[&str]) -> Index {
        let groups = names
            .iter()
            .map(|n| Group {
                members: vec![Entry {
                    name: n.to_string(),
                    target: n.to_string(),
                    workdir: String::new(),
                    kind: Kind::Start,
                    root: String::new(),
                    resolved: String::new(),
                    owner: String::new(),
                    console: false,
                }],
                cur: 0,
                keys: keys(n),
                owner: None,
                helpers: Vec::new(),
            })
            .collect();
        Index { groups, found: Vec::new() }
    }

    fn names(index: &Index, store: &Store, q: &str) -> Vec<String> {
        run(index, store, q, 8).iter().map(|h| index.groups[h.group].entry().name.clone()).collect()
    }

    #[test]
    fn ranks_prefix_over_substring_and_rejects_loose_subsequences() {
        let ix = index(&["Microsoft Defender", "Visual Studio Code", "Codex", "微信"]);
        let store = Store::default();
        assert_eq!(names(&ix, &store, "code"), ["Codex", "Visual Studio Code"]);
        assert_eq!(names(&ix, &store, "vsc"), ["Visual Studio Code"]);
        assert_eq!(names(&ix, &store, "wx"), ["微信"]);
        assert!(names(&ix, &store, "qwer").is_empty());
    }

    #[test]
    fn marks_follow_the_matched_key() {
        let ix = index(&["微信", "Visual Studio Code"]);
        let store = Store::default();
        assert_eq!(run(&ix, &store, "wei", 8)[0].marks, [true, false]);
        let marks = &run(&ix, &store, "vsc", 8)[0].marks;
        let on: Vec<usize> = marks.iter().enumerate().filter(|(_, m)| **m).map(|(i, _)| i).collect();
        assert_eq!(on, [0, 7, 14]);
    }

    #[test]
    fn usage_and_query_memory_change_the_order() {
        let ix = index(&["Codex", "Code"]);
        let mut store = Store::default();
        assert_eq!(names(&ix, &store, "co"), ["Code", "Codex"]);
        store.picks.insert("co".into(), "Codex".into());
        assert_eq!(names(&ix, &store, "co"), ["Codex", "Code"]);
        store.usage.insert("Code".into(), 3);
        assert_eq!(names(&ix, &store, ""), ["Code"]);
    }

    #[test]
    fn empty_history_lists_system_starters() {
        let mut ix = index(&["Blender", "终端", "设置"]);
        ix.groups[1].members[0].target = r"shell:AppsFolder\Microsoft.WindowsTerminal_8wekyb3d8bbwe!App".into();
        ix.groups[2].members[0].target = r"shell:AppsFolder\windows.immersivecontrolpanel_cw5n1h2txyewy!microsoft.windows.immersivecontrolpanel".into();
        assert_eq!(names(&ix, &Store::default(), ""), ["设置", "终端"]);
    }

    #[test]
    fn a_program_folder_takes_one_row() {
        let mut ix = index(&["VirtualBox", "VBoxManage", "VBoxSDS", "Blender", "latex-git-log", "mk-git-x"]);
        for helper in [1, 2, 4, 5] {
            ix.groups[helper].members[0].owner = "VirtualBox".into();
        }
        ix.link();
        let mut store = Store::default();
        // 对上好几个：给出主程序，带着收起的、对得上输入的数量（latex-git-log、mk-git-x 不算）
        let hits = run(&ix, &store, "vbox", 8);
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].group, hits[0].extra), (0, 2));
        assert_eq!(run(&ix, &store, "", 8).len(), 0);
        // 对上好几个但都不是以输入开头：给出对得最好的那一个，而不是名字对不上的主程序
        assert_eq!(names(&ix, &store, "git"), ["mk-git-x"]);
        // 只对上一个：直接给出它
        assert_eq!(names(&ix, &store, "vboxm"), ["VBoxManage"]);
        // 主程序自己对上了：其余不再单列
        assert_eq!(names(&ix, &store, "v"), ["VirtualBox"]);
        // 用过的不再收起
        store.usage.insert("VBoxSDS".into(), 1);
        assert_eq!(names(&ix, &store, "vbox"), ["VBoxSDS", "VBoxManage"]);
    }
}
