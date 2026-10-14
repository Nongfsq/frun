//! 诊断：`frun --doctor [文件路径]`。
//!
//! 重新扫描一次，把"这台电脑上扫到了什么、为什么"写成一份报告并打开，供提交问题时附上。
//! 带文件路径时，先回答这个文件为什么搜得到或搜不到。报告是给开发者看的，固定用英文。
use crate::{
    config::{self, Store},
    index::{self, Entry, Kind},
    scan,
};
use std::{collections::HashMap, fmt::Write, fs, os::windows::fs::MetadataExt, path::Path, time::Instant};
use windows::{
    Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    core::{HSTRING, w},
};

pub fn run(args: &[String]) {
    let began = Instant::now();
    scan::run(&["0".to_string(), "32".to_string()]);
    let took = began.elapsed().as_millis();

    let cfg = config::load();
    let store = Store::load();
    let all = index::read_all();
    let mut out = String::new();
    let _ = writeln!(out, "frun doctor  (version {})", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(out, "This report lists folder and program paths from this computer. Read it before sharing.\n");

    if let Some(path) = args.first() {
        let _ = writeln!(out, "== why: {path}\n{}\n", explain(path, &cfg, &store, &all));
    }

    let _ = writeln!(out, "== scan\nlast scan took {took} ms (includes the system app list and any missing icons)");
    let (stamp, _, barren) = scan::read_barren();
    let age = now().saturating_sub(stamp) / 3600;
    let _ = writeln!(out, "empty subtrees remembered: {} (recorded {age} h ago)", barren.len());
    let _ = writeln!(out, "file types enabled: {}", cfg.exts.join(", "));
    let _ = writeln!(out, "hidden by user: {} programs, {} folders", store.hidden.len() - store.hidden_dirs().len(), store.hidden_dirs().len());

    let _ = writeln!(out, "\n== folders");
    let found = fs::read_to_string(index::found_path()).unwrap_or_default();
    for dir in &cfg.dirs {
        let inside = |e: &&Entry| e.root.eq_ignore_ascii_case(dir);
        let kept = all.iter().filter(inside).filter(|e| e.kind == Kind::Dir);
        let (alone, behind) = kept.fold((0, 0), |n, e| if e.owner.is_empty() { (n.0 + 1, n.1) } else { (n.0, n.1 + 1) });
        let skipped = all.iter().filter(inside).filter(|e| e.kind == Kind::Skip);
        let (console, script) = skipped.fold((0, 0), |n, e| if e.console { (n.0 + 1, n.1) } else { (n.0, n.1 + 1) });
        let types: Vec<String> = found
            .lines()
            .filter_map(|l| {
                let mut f = l.split('\t');
                (f.next()? == dir).then(|| format!("{} {}", f.next().unwrap_or(""), f.next().unwrap_or("")))
            })
            .collect();
        let _ = writeln!(out, "{dir}  exists={}", Path::new(dir).is_dir());
        let _ = writeln!(out, "  indexed: {alone} on their own row, {behind} folded behind a main program");
        let _ = writeln!(out, "  not indexed: {console} console programs, {script} scripts inside program folders");
        let _ = writeln!(out, "  launchable files seen by type: {}", types.join(", "));
    }

    let count = |k: Kind| all.iter().filter(|e| e.kind == k).count();
    let _ = writeln!(out, "\n== system\nstart menu: {}, store apps: {}", count(Kind::Start), count(Kind::Store));

    // 认出的程序目录：主程序后面收了多少个
    let mut mains: HashMap<&str, usize> = HashMap::new();
    for e in all.iter().filter(|e| e.kind == Kind::Dir && !e.owner.is_empty()) {
        *mains.entry(&e.owner).or_default() += 1;
    }
    let mut mains: Vec<(&str, usize)> = mains.into_iter().collect();
    mains.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let _ = writeln!(out, "\n== program folders recognized: {} (largest 20)", mains.len());
    for (main, n) in mains.iter().take(20) {
        let _ = writeln!(out, "{n:>5}  {main}");
    }

    let _ = writeln!(out, "\n== recent activity");
    let log = fs::read_to_string(scan::log_path()).unwrap_or_default();
    let lines: Vec<&str> = log.lines().collect();
    for line in &lines[lines.len().saturating_sub(30)..] {
        let _ = writeln!(out, "{line}");
    }

    let path = config::cache_dir().join("doctor.txt");
    if fs::write(&path, out).is_ok() {
        unsafe {
            ShellExecuteW(None, w!("open"), &HSTRING::from(path.as_os_str()), None, None, SW_SHOWNORMAL);
        }
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// 一个文件为什么搜得到或搜不到。按扫描时判断的先后顺序找第一个原因。
fn explain(path: &str, cfg: &config::Config, store: &Store, all: &[Entry]) -> String {
    let path = path.trim().trim_matches('"');
    if let Some(e) = all.iter().find(|e| e.target.eq_ignore_ascii_case(path)) {
        return match e.kind {
            Kind::Skip if e.console && e.target.to_lowercase().ends_with(".lnk") => {
                "not indexed: the shortcut points to a console program (no window of its own)".into()
            }
            Kind::Skip if e.console => "not indexed: it is a console program (no window of its own)".into(),
            Kind::Skip => format!("not indexed: a script inside a program folder (main program: {})", e.owner),
            _ if store.is_hidden(&e.target) => "indexed, but hidden by the user (Settings > Folders > Hidden)".into(),
            _ if e.owner.is_empty() => "indexed: shown on its own row".into(),
            _ => format!("indexed: folded behind {} (shown directly when the query points at it alone)", e.owner),
        };
    }
    let file = Path::new(path);
    if !file.is_file() {
        return "the file does not exist".into();
    }
    let Some(root) = cfg.dirs.iter().find(|d| scan::under(path, d)) else {
        return "it is not inside any folder listed in Settings > Folders".into();
    };
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if !cfg.exts.contains(&ext) {
        return format!("its type .{ext} is not enabled in Settings > Folders > File types");
    }
    let (_, _, barren) = scan::read_barren();
    let hidden = store.hidden_dirs();
    let mut dir = file.parent();
    while let Some(d) = dir.filter(|d| d.as_os_str().len() >= root.trim_end_matches('\\').len()) {
        let key = d.to_string_lossy().trim_end_matches('\\').to_lowercase();
        let name = d.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if hidden.contains(&key) {
            return format!("the user chose to stop searching {}", d.display());
        }
        if scan::skip_dir(name) {
            return format!("folder {} is never searched (hidden, dependency or system folder name)", d.display());
        }
        if d.join(scan::CACHE_TAG).exists() {
            return format!("folder {} is marked as a cache directory (CACHEDIR.TAG)", d.display());
        }
        if !key.eq_ignore_ascii_case(root.trim_end_matches('\\')) && fs::metadata(d).is_ok_and(|m| m.file_attributes() & 2 != 0) {
            return format!("folder {} has the hidden attribute", d.display());
        }
        if barren.contains(&key) {
            return format!("folder {} was remembered as empty; press Ctrl+R in frun to search everything again", d.display());
        }
        dir = d.parent();
    }
    let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if scan::is_junk(stem, path) {
        return "its name looks like an uninstaller or a document link".into();
    }
    "no reason found: it should be indexed. Press Ctrl+R in frun; if it still does not appear, please report this".into()
}
