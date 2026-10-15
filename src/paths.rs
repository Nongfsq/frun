//! 路径模式：输入以盘符、\\ 或 %变量% 开头时，结果列出这一层对得上的子文件夹和可启动文件，
//! Enter 用资源管理器打开文件夹或运行程序。平时的搜索仍然只找程序。
use std::{fs, os::windows::fs::MetadataExt, path::Path};

/// 结果里的一项。
#[derive(Debug, PartialEq)]
pub struct Item {
    /// 显示的名字：文件夹名或文件名（带后缀）
    pub name: String,
    /// 完整路径，文件夹不带末尾的反斜杠
    pub path: String,
    /// 所在的文件夹，显示在行右侧
    pub parent: String,
    pub dir: bool,
    /// 名字开头有几个字是输入的，用来着色
    pub typed: usize,
}

impl Item {
    /// 接受补全后输入框里应该是什么：文件夹补上反斜杠，接着输下一层。
    pub fn completed(&self) -> String {
        if self.dir && !self.path.ends_with('\\') { format!("{}\\", self.path) } else { self.path.clone() }
    }

    /// 光标后淡色显示的部分：补全后的路径里还没输入的那些字。补全不是接着输入往下写时没有。
    pub fn ghost(&self, query: &str) -> Option<String> {
        let done: Vec<char> = self.completed().chars().collect();
        let typed: Vec<char> = query.trim_start_matches('"').chars().collect();
        let same = done.iter().zip(&typed).all(|(a, b)| a.to_lowercase().eq(b.to_lowercase()));
        (done.len() > typed.len() && same).then(|| done[typed.len()..].iter().collect())
    }
}

/// 输入是不是一条路径：带盘符的（d:、d:\so）、网络路径（\\server）或以 %变量% 开头的。
pub fn is_path(q: &str) -> bool {
    let q = q.trim_start().trim_start_matches('"');
    let b = q.as_bytes();
    (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
        || q.starts_with(r"\\")
        || (q.starts_with('%') && q[1..].contains('%'))
}

/// 展开开头的 %变量%；不认识的原样保留。
fn expand(q: &str) -> String {
    if let Some(rest) = q.strip_prefix('%')
        && let Some((name, tail)) = rest.split_once('%')
        && let Ok(value) = std::env::var(name)
    {
        return format!("{value}{tail}");
    }
    q.to_string()
}

/// 列出这一层：输入的最后一段是名字的开头。最后一段为空（以反斜杠结尾）时，先给出这个文件夹本身。
/// `exts` 是启用的可启动类型；隐藏和系统文件不列。文件夹在前，各自按名字排。
/// 网络路径不列（可能一等就是几秒），只给出输入本身，Enter 直接打开。
pub fn list(q: &str, exts: &[String], max: usize) -> Vec<Item> {
    let q = expand(q.trim().trim_matches('"'));
    if q.starts_with(r"\\") {
        let path = q.trim_end_matches('\\').to_string();
        return vec![Item { name: path.clone(), path, parent: String::new(), dir: true, typed: 0 }];
    }
    // "d:" 当作 "D:\"；盘符一律大写，和资源管理器里看到的一样
    let q = if q.len() == 2 { format!("{q}\\") } else { q };
    let q = if q.as_bytes().get(1) == Some(&b':') { q[..1].to_uppercase() + &q[1..] } else { q };
    let cut = q.rfind(['\\', '/']).map_or(0, |i| i + 1);
    let (parent, typed) = q.split_at(cut);
    let mut out = Vec::new();
    if parent.is_empty() || !Path::new(parent).is_dir() {
        return out;
    }
    if typed.is_empty() {
        let path = if parent.len() <= 3 { parent.to_uppercase() } else { parent.trim_end_matches(['\\', '/']).to_string() };
        let (up, name) = match path.trim_end_matches('\\').rsplit_once('\\') {
            Some((up, name)) if path.len() > 3 => (format!("{up}\\"), name.to_string()),
            _ => (String::new(), path.clone()),
        };
        out.push(Item { name, path, parent: up, dir: true, typed: 0 });
    }
    let want = typed.to_lowercase();
    let mut found: Vec<Item> = fs::read_dir(parent)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let meta = e.metadata().ok()?;
            // 隐藏（2）和系统（4）属性的不列
            if meta.file_attributes() & 6 != 0 || !name.to_lowercase().starts_with(&want) {
                return None;
            }
            let dir = meta.is_dir();
            let ext = name.rsplit_once('.').map(|(_, x)| x.to_lowercase()).unwrap_or_default();
            if !dir && !exts.contains(&ext) {
                return None;
            }
            let path = format!("{parent}{name}");
            Some(Item { name, path, parent: parent.to_string(), dir, typed: typed.chars().count() })
        })
        .collect();
    found.sort_by_cached_key(|i| (!i.dir, i.name.to_lowercase()));
    out.extend(found);
    out.truncate(max);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_paths_but_not_names() {
        for q in [r"d:", r"D:\So", r"\\nas\share", "%APPDATA%\\x", r#""c:\Program Files"#] {
            assert!(is_path(q), "{q}");
        }
        for q in ["d", "vbox", "dota 2", "%", "50%"] {
            assert!(!is_path(q), "{q}");
        }
    }

    #[test]
    fn lists_matching_folders_first_then_programs() {
        let root = crate::testdir::TestDir::new("paths");
        for d in ["Software", "Sources", "Other"] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        for f in ["setup.exe", "SoundCheck.exe", "Sonata.txt"] {
            fs::write(root.join(f), "x").unwrap();
        }
        let base = root.to_string_lossy().into_owned();
        let exts = vec!["exe".to_string()];

        let items = list(&format!("{base}\\so"), &exts, 8);
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["Software", "Sources", "SoundCheck.exe"]);
        assert_eq!(items[0].ghost(&format!("{base}\\so")).as_deref(), Some("ftware\\"));
        assert_eq!(items[0].completed(), format!("{base}\\Software\\"));
        assert_eq!(items[2].completed(), format!("{base}\\SoundCheck.exe"));

        // 以反斜杠结尾：先是这个文件夹本身，然后是里面的东西
        let items = list(&format!("{base}\\"), &exts, 8);
        assert_eq!(items[0].path, base);
        assert_eq!(items[0].ghost(&format!("{base}\\")), None);
        assert_eq!(items.len(), 1 + 3 + 2);
    }
}
