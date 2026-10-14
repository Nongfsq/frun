//! 索引的读取与分组。索引文件由扫描进程（scan.rs）写出，主进程只读。
use crate::config::{self, Store};
use pinyin::ToPinyin;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// 开始菜单里的桌面程序
    Start,
    /// 商店与 UWP 应用
    Store,
    /// 用户指定目录里的文件
    Dir,
    /// 目录里扫到但不收的文件（程序目录里的命令行工具和脚本）。
    /// 只留在索引文件里给扫描进程看，免得每次重读文件头；主进程不读它
    Skip,
}

impl Kind {
    pub fn code(self) -> &'static str {
        match self {
            Kind::Start => "s",
            Kind::Store => "u",
            Kind::Dir => "d",
            Kind::Skip => "x",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "s" => Some(Kind::Start),
            "u" => Some(Kind::Store),
            "d" => Some(Kind::Dir),
            "x" => Some(Kind::Skip),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct Entry {
    pub name: String,
    /// 交给 ShellExecute 的启动目标：文件路径或 shell:AppsFolder\<AUMID>
    pub target: String,
    /// 工作目录；系统应用为空
    pub workdir: String,
    pub kind: Kind,
    /// 所属的用户目录（仅 Kind::Dir）
    pub root: String,
    /// 解析后的真实路径（小写），用于判定重复；未知为空
    pub resolved: String,
    /// 收在哪个主程序后面（它的启动目标）；独立占一行的为空
    pub owner: String,
    /// 命令行程序（没有窗口）
    pub console: bool,
}

impl Entry {
    pub fn icon_key(&self) -> u64 {
        icon_key(&self.target)
    }
}

/// 图标缓存文件名用的键：小写启动目标的 FNV-1a。
pub fn icon_key(target: &str) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in target.to_lowercase().bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    h
}

/// 一个可匹配的键。`map[i]` 是键的第 i 个字符对应名称里的第几个字符，用于标出命中位置。
pub struct Key {
    pub text: Vec<char>,
    pub map: Vec<u16>,
}

/// 同一程序的多个来源折叠成一组，界面上只占一行。
pub struct Group {
    pub members: Vec<Entry>,
    /// 当前显示的成员
    pub cur: usize,
    pub keys: Vec<Key>,
    /// 同一个程序目录里的主程序；有它时这一组平时收在它后面
    pub owner: Option<usize>,
    /// 收在这一组后面的程序
    pub helpers: Vec<usize>,
}

impl Group {
    pub fn entry(&self) -> &Entry {
        &self.members[self.cur]
    }

    pub fn usage(&self, store: &Store) -> u32 {
        self.members.iter().map(|m| store.usage.get(&m.target).copied().unwrap_or(0)).sum()
    }

    /// 显示哪个成员：用户上次从展开列表里打开的那个，否则用得最多的那个。
    pub fn choose(&mut self, store: &Store) {
        let used = |m: &Entry| store.usage.get(&m.target).copied().unwrap_or(0);
        let cur = self.members.iter().position(|m| store.prefer.contains(&m.target)).unwrap_or_else(|| {
            // 都没用过时，系统里登记的名字比文件名好认；
            // 目录里同一个程序既有本体又有快捷方式时，用本体的名字（快捷方式的名字常是给别的工具堆的关键词）
            let lnk = |m: &Entry| m.target.to_lowercase().ends_with(".lnk");
            let mut best = (self.members.iter().position(|m| m.kind != Kind::Dir))
                .or_else(|| self.members.iter().position(|m| !lnk(m)))
                .unwrap_or(0);
            for (i, m) in self.members.iter().enumerate() {
                if used(m) > used(&self.members[best]) {
                    best = i;
                }
            }
            best
        });
        if cur != self.cur || self.keys.is_empty() {
            self.cur = cur;
            self.keys = keys(&self.members[cur].name);
        }
    }
}

#[derive(Default)]
pub struct Index {
    pub groups: Vec<Group>,
    /// 各用户目录里实际存在的可启动文件类型及数量：(目录, 扩展名, 数量)
    pub found: Vec<(String, String, usize)>,
}

impl Index {
    pub fn targets(&self) -> HashSet<String> {
        self.groups.iter().flat_map(|g| g.members.iter().map(|m| m.target.clone())).collect()
    }

    /// 某个用户目录收录了多少个程序。
    pub fn count_in(&self, root: &str) -> usize {
        self.groups
            .iter()
            .flat_map(|g| &g.members)
            .filter(|m| m.kind == Kind::Dir && m.root.eq_ignore_ascii_case(root))
            .count()
    }

    /// 收在 `group` 后面、还没被单独用过的程序。用过的自己占一行。
    pub fn folded<'a>(&'a self, group: usize, store: &'a Store) -> impl Iterator<Item = usize> + 'a {
        self.groups[group].helpers.iter().copied().filter(|&h| self.groups[h].usage(store) == 0)
    }

    /// 按条目里记的主程序把各组连起来。组的位置变了之后要重连。
    pub fn link(&mut self) {
        let at: HashMap<String, usize> = self
            .groups
            .iter()
            .enumerate()
            .flat_map(|(i, g)| g.members.iter().map(move |m| (m.target.to_lowercase(), i)))
            .collect();
        let owners: Vec<Option<usize>> = self
            .groups
            .iter()
            .enumerate()
            .map(|(i, g)| {
                // 只要有一个来源是独立的（比如开始菜单里也有），这一组就独立
                if g.members.iter().any(|m| m.owner.is_empty()) {
                    return None;
                }
                at.get(&g.members[0].owner.to_lowercase()).copied().filter(|&o| o != i)
            })
            .collect();
        for g in &mut self.groups {
            g.helpers.clear();
        }
        for (i, owner) in owners.into_iter().enumerate() {
            self.groups[i].owner = owner;
            if let Some(o) = owner {
                self.groups[o].helpers.push(i);
            }
        }
    }

    pub fn remove(&mut self, target: &str) {
        for g in &mut self.groups {
            if g.members.len() > 1 {
                g.members.retain(|m| m.target != target);
                g.cur = 0;
                g.keys.clear();
            }
        }
        self.groups.retain(|g| g.members[0].target != target);
        self.link();
    }

    pub fn choose_all(&mut self, store: &Store) {
        for g in &mut self.groups {
            g.choose(store);
        }
    }
}

pub fn index_path() -> PathBuf {
    config::cache_dir().join("index.tsv")
}

pub fn found_path() -> PathBuf {
    config::cache_dir().join("found.tsv")
}

pub fn icon_dir(px: u32) -> PathBuf {
    config::cache_dir().join("icons").join(px.to_string())
}

/// 索引里要显示的条目，不分组。
pub fn read() -> Vec<Entry> {
    let mut all = read_all();
    all.retain(|e| e.kind != Kind::Skip);
    all
}

/// 索引文件里的全部条目，包括扫到但不收的。
pub fn read_all() -> Vec<Entry> {
    let text = fs::read_to_string(index_path()).unwrap_or_default();
    text.lines()
        .filter_map(|line| {
            let mut f = line.split('\t');
            Some(Entry {
                kind: Kind::parse(f.next()?)?,
                name: f.next()?.to_string(),
                target: f.next()?.to_string(),
                workdir: f.next()?.to_string(),
                root: f.next()?.to_string(),
                resolved: f.next()?.to_string(),
                owner: f.next().unwrap_or("").to_string(),
                console: matches!(f.next(), Some("t") | Some("c")),
            })
        })
        .collect()
}

pub fn load(store: &Store) -> Index {
    let entries = read().into_iter();
    let found = fs::read_to_string(found_path())
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let mut f = line.split('\t');
            Some((f.next()?.to_string(), f.next()?.to_string(), f.next()?.parse().ok()?))
        })
        .collect();
    let dirs = store.hidden_dirs();
    let inside = |e: &Entry| {
        let at = e.workdir.to_lowercase();
        dirs.iter().any(|d| at.strip_prefix(d.as_str()).is_some_and(|rest| rest.is_empty() || rest.starts_with('\\')))
    };
    let shown = entries.filter(|e| !store.is_hidden(&e.target) && !(e.kind == Kind::Dir && inside(e)));
    let mut index = Index { groups: group(shown), found };
    index.link();
    index.choose_all(store);
    index
}

/// 判定重复：解析后的目标路径相同，或显示名相同。
/// 收在不同主程序后面的同名文件是各自程序的一部分，不算同一个。
fn group(entries: impl Iterator<Item = Entry>) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    let mut by_key: HashMap<String, usize> = HashMap::new();
    for e in entries {
        let name_key = format!("n:{}:{}", e.owner.to_lowercase(), e.name.to_lowercase());
        let path_key = (!e.resolved.is_empty()).then(|| format!("t:{}", e.resolved));
        let at = by_key.get(&name_key).or_else(|| path_key.as_ref().and_then(|k| by_key.get(k))).copied();
        let at = at.unwrap_or_else(|| {
            groups.push(Group { members: Vec::new(), cur: 0, keys: Vec::new(), owner: None, helpers: Vec::new() });
            groups.len() - 1
        });
        by_key.insert(name_key, at);
        if let Some(k) = path_key {
            by_key.insert(k, at);
        }
        if !groups[at].members.iter().any(|m| m.target.eq_ignore_ascii_case(&e.target)) {
            groups[at].members.push(e);
        }
    }
    groups
}

/// 名称的匹配键：小写原名、各词首字母，以及含中文时的全拼和拼音首字母。
pub fn keys(name: &str) -> Vec<Key> {
    let chars: Vec<char> = name.chars().collect();
    let lower: Vec<char> = chars.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
    let identity: Vec<u16> = (0..lower.len() as u16).collect();

    let mut acronym = Key { text: Vec::new(), map: Vec::new() };
    let mut full = Key { text: Vec::new(), map: Vec::new() };
    let mut initials = Key { text: Vec::new(), map: Vec::new() };
    let mut has_hanzi = false;
    let mut prev: Option<char> = None;
    for (i, (&c, &l)) in chars.iter().zip(&lower).enumerate() {
        let i = i as u16;
        match c.to_pinyin() {
            Some(p) => {
                has_hanzi = true;
                for ch in p.plain().chars() {
                    full.text.push(ch);
                    full.map.push(i);
                }
                initials.text.extend(p.first_letter().chars().take(1));
                initials.map.push(i);
            }
            None => {
                full.text.push(l);
                full.map.push(i);
                initials.text.push(l);
                initials.map.push(i);
                // 词首：前一个字符不是字母数字，或小写后面跟着大写（PowerToys -> pt）
                let starts = c.is_alphanumeric()
                    && prev.is_none_or(|p| !p.is_alphanumeric() || (p.is_lowercase() && c.is_uppercase()));
                if starts {
                    acronym.text.push(l);
                    acronym.map.push(i);
                }
            }
        }
        prev = Some(c);
    }

    let mut out = vec![Key { text: lower, map: identity }];
    if acronym.text.len() > 1 {
        out.push(acronym);
    }
    if has_hanzi {
        out.push(full);
        out.push(initials);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, target: &str, kind: Kind, resolved: &str) -> Entry {
        Entry {
            name: name.into(),
            target: target.into(),
            workdir: String::new(),
            kind,
            root: String::new(),
            resolved: resolved.into(),
            owner: String::new(),
            console: false,
        }
    }

    #[test]
    fn same_name_or_same_target_folds_into_one_group() {
        let groups = group(
            vec![
                entry("Visual Studio Code", "shell:AppsFolder\\vsc", Kind::Start, r"c:\vsc\code.exe"),
                entry("Code", r"D:\Portable\Code.lnk", Kind::Dir, r"c:\vsc\code.exe"),
                entry("终端", "shell:AppsFolder\\wt!App", Kind::Store, ""),
                entry("终端", r"D:\Tools\终端.lnk", Kind::Dir, r"c:\wt.exe"),
                entry("Blender", "shell:AppsFolder\\blender", Kind::Start, ""),
            ]
            .into_iter(),
        );
        assert_eq!(groups.iter().map(|g| g.members.len()).collect::<Vec<_>>(), [2, 2, 1]);
    }

    #[test]
    fn a_program_and_its_shortcut_show_the_program_name() {
        let exe = r"d:\tools\inpaint.exe|";
        let mut groups = group(
            vec![
                entry("工具 - Inpaint.exe - utility - inpaint", r"D:\Tools\index\工具 - Inpaint.exe.lnk", Kind::Dir, exe),
                entry("Inpaint", r"D:\Tools\Inpaint.exe", Kind::Dir, exe),
            ]
            .into_iter(),
        );
        assert_eq!(groups.len(), 1);
        groups[0].choose(&Store::default());
        assert_eq!(groups[0].members[groups[0].cur].name, "Inpaint");
    }

    #[test]
    fn same_name_under_different_programs_stays_apart() {
        let mut a = entry("git", r"E:\Git\bin\git.exe", Kind::Dir, "");
        a.owner = r"shell:AppsFolder\git-bash".into();
        let mut b = entry("git", r"E:\Other\git\git.exe", Kind::Dir, "");
        b.owner = r"E:\Other\git\git-gui.exe".into();
        let c = entry("git", r"D:\Tools\git.exe", Kind::Dir, "");
        let d = entry("git", r"D:\Portable\git.exe", Kind::Dir, "");
        let groups = group(vec![a, b, c, d].into_iter());
        assert_eq!(groups.iter().map(|g| g.members.len()).collect::<Vec<_>>(), [1, 1, 2]);
    }

    #[test]
    fn keys_cover_acronym_and_pinyin() {
        let text = |k: &Key| k.text.iter().collect::<String>();
        let k = keys("Visual Studio Code");
        assert_eq!(text(&k[1]), "vsc");
        assert_eq!(k[1].map, [0, 7, 14]);
        assert_eq!(text(&keys("PowerToys")[1]), "pt");
        let k = keys("微信");
        assert_eq!(text(&k[1]), "weixin");
        assert_eq!(k[1].map, [0, 0, 0, 1, 1, 1]);
        assert_eq!(text(&k[2]), "wx");
    }
}
