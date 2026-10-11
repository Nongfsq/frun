use pinyin::ToPinyin;
use std::{collections::HashSet, fs, path::Path};
use windows::{
    Win32::{Foundation::SIZE, Graphics::Gdi::HBITMAP, System::Com::CoTaskMemFree, UI::Shell::*},
    core::*,
};

pub struct Item {
    pub name: String,
    /// 用于匹配的小写形式：原名、各词首字母，以及含中文时的全拼和拼音首字母
    pub keys: Vec<String>,
    /// 交给 ShellExecute 的启动目标：文件路径或 shell:AppsFolder\<AUMID>
    pub target: String,
    /// 工作目录；系统应用为空
    pub dir: String,
}

impl Item {
    fn new(name: String, target: String, dir: String) -> Self {
        let lower = name.to_lowercase();
        let (mut full, mut initials, mut has_hanzi) = (String::new(), String::new(), false);
        for (c, p) in lower.chars().zip(lower.as_str().to_pinyin()) {
            match p {
                Some(p) => {
                    has_hanzi = true;
                    full.push_str(p.plain());
                    initials.push_str(p.first_letter());
                }
                None => {
                    full.push(c);
                    initials.push(c);
                }
            }
        }
        let words = lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty());
        let acronym: String = words.filter_map(|w| w.chars().next()).collect();
        let mut keys = vec![lower];
        if acronym.chars().count() > 1 {
            keys.push(acronym);
        }
        if has_hanzi {
            keys.extend([full, initials]);
        }
        Self { name, keys, target, dir }
    }
}

/// 收集自定义目录里的程序和系统「所有应用」列表。调用线程需已初始化 COM。
pub fn scan(dirs: &[String], depth: usize) -> Vec<Item> {
    let mut out = Vec::new();
    for d in dirs {
        walk(Path::new(d), depth, &mut out);
    }
    let _ = unsafe { apps(&mut out) };
    let mut seen = HashSet::new();
    out.retain(|i| seen.insert(i.target.to_lowercase()));
    out
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<Item>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for path in entries.flatten().map(|e| e.path()) {
        if path.is_dir() {
            if depth > 0 {
                walk(&path, depth - 1, out);
            }
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if ext != "exe" && ext != "lnk" {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            out.push(Item::new(
                stem.to_string(),
                path.to_string_lossy().into_owned(),
                dir.to_string_lossy().into_owned(),
            ));
        }
    }
}

/// shell:AppsFolder 同时包含开始菜单快捷方式和 Store 应用。
unsafe fn apps(out: &mut Vec<Item>) -> Result<()> {
    unsafe {
        let folder: IShellItem = SHCreateItemFromParsingName(w!("shell:AppsFolder"), None)?;
        let items: IEnumShellItems = folder.BindToHandler(None, &BHID_EnumItems)?;
        loop {
            let mut next = [None];
            if items.Next(&mut next, None).is_err() {
                break;
            }
            let Some(item) = next[0].take() else { break };
            let (Ok(name), Ok(id)) =
                (text(&item, SIGDN_NORMALDISPLAY), text(&item, SIGDN_PARENTRELATIVEPARSING))
            else {
                continue;
            };
            if !is_junk(&name, &id) {
                out.push(Item::new(name, format!("shell:AppsFolder\\{id}"), String::new()));
            }
        }
        Ok(())
    }
}

unsafe fn text(item: &IShellItem, kind: SIGDN) -> Result<String> {
    unsafe {
        let p = item.GetDisplayName(kind)?;
        let s = p.to_string().unwrap_or_default();
        CoTaskMemFree(Some(p.0 as _));
        Ok(s)
    }
}

/// 开始菜单里的卸载程序、文档和网页链接不算程序。
fn is_junk(name: &str, id: &str) -> bool {
    let (name, id) = (name.to_lowercase(), id.to_lowercase());
    name.contains("uninstall")
        || name.contains("卸载")
        || id.starts_with("http")
        || [".url", ".chm", ".txt", ".htm", ".html", ".pdf"].iter().any(|e| id.ends_with(e))
}

/// 取启动目标的图标（32 位预乘 alpha 位图）。调用线程需已初始化 COM。
pub fn icon(target: &str, size: i32) -> Option<HBITMAP> {
    unsafe {
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(&HSTRING::from(target), None).ok()?;
        factory.GetImage(SIZE { cx: size, cy: size }, SIIGBF_ICONONLY).ok()
    }
}
