//! 扫描进程：`frun --scan <hwnd> <图标像素> [force]`。
//!
//! 枚举系统应用列表和提取图标会让外壳组件在进程里常驻几十 MB，
//! 所以这些工作放在用完即退的子进程里做，主进程只读结果文件。
use crate::{
    config,
    index::{self, Entry, Kind},
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    mem::size_of,
    path::{Path, PathBuf},
};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::Com::*,
        UI::{Shell::*, WindowsAndMessaging::PostMessageW},
    },
    core::*,
};

pub const WM_INDEX: u32 = 0x8000 + 2;
pub const WM_ICONS: u32 = 0x8000 + 3;
/// 扫描进度：wParam 是已经找到的条目数
pub const WM_PROGRESS: u32 = 0x8000 + 11;

static NOTIFY: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// 每找到一批条目报一次数，首次建索引时面板上的数字靠它跳动。
fn progress(n: usize) {
    let hwnd = NOTIFY.load(std::sync::atomic::Ordering::Relaxed);
    if hwnd != 0 && n % 16 == 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as _)), WM_PROGRESS, WPARAM(n), LPARAM(0));
        }
    }
}

/// 目录里这些类型算"可启动"，会作为候选类型列在设置里。
const LAUNCHABLE: &[&str] =
    &["exe", "lnk", "bat", "cmd", "ps1", "py", "msc", "com", "vbs", "ahk", "jar", "appref-ms", "url"];

const PKEY_LINK_TARGET: PROPERTYKEY =
    PROPERTYKEY { fmtid: GUID::from_u128(0xb9b4b3fc_2b51_4a42_b5d8_324146afcf25), pid: 2 };
const PKEY_LINK_ARGUMENTS: PROPERTYKEY =
    PROPERTYKEY { fmtid: GUID::from_u128(0x436f2667_14e2_4feb_b30a_146c53b5b674), pid: 100 };

/// 判定重复用的键：目标路径加启动参数。同一个 cmd.exe 配不同参数是不同的程序。
fn resolved(path: &str, args: &str) -> String {
    if path.is_empty() { String::new() } else { format!("{path}|{args}").to_lowercase() }
}

pub fn run(args: &[String]) {
    let hwnd = args.first().and_then(|a| a.parse::<isize>().ok()).unwrap_or(0);
    let px = args.get(1).and_then(|a| a.parse::<u32>().ok()).unwrap_or(32).clamp(16, 256);
    let force = args.get(2).is_some_and(|a| a == "force");
    let notify = |msg: u32| unsafe {
        if hwnd != 0 {
            let _ = PostMessageW(Some(HWND(hwnd as _)), msg, WPARAM(0), LPARAM(0));
        }
    };
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    NOTIFY.store(hwnd, std::sync::atomic::Ordering::Relaxed);
    let cfg = config::load();
    // 只查指定的几处：目录里刚有变化的子目录，或 "apps"（系统应用列表）。其余沿用上一次的结果
    let only: Vec<String> = args.iter().skip(2).filter(|a| *a != "force").cloned().collect();
    let old = index::read();
    let partial = !only.is_empty() && !old.is_empty();
    // 有没有窗口要读文件头才知道；上次读过的不再读
    let known: HashMap<String, bool> = fs::read_to_string(index::index_path())
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            matches!(f.get(7), Some(&"c") | Some(&"g")).then(|| (f[2].to_lowercase(), f[7] == "c"))
        })
        .collect();
    let mut entries = Vec::new();
    let mut found = BTreeMap::new();
    if partial {
        let root_of = |p: &str| cfg.dirs.iter().find(|d| under(p, d));
        let apps_too = only.iter().any(|p| root_of(p).is_none());
        entries = old
            .into_iter()
            .filter(|e| match e.kind {
                Kind::Dir => root_of(&e.workdir).is_some() && !only.iter().any(|p| under(&e.workdir, p)),
                _ => !apps_too,
            })
            .collect();
        for path in &only {
            if let Some(root) = root_of(path) {
                walk(Path::new(path), root, &cfg.exts, &known, &mut entries, &mut found);
            }
        }
        if apps_too {
            let _ = unsafe { apps(&mut entries) };
        }
    } else {
        for dir in &cfg.dirs {
            walk(Path::new(dir), dir, &cfg.exts, &known, &mut entries, &mut found);
        }
        let _ = unsafe { apps(&mut entries) };
    }
    let mut seen = HashSet::new();
    entries.retain(|e| seen.insert(e.target.to_lowercase()));
    // 目录里的排前面并按路径排序，结果与扫描的先后无关
    entries.sort_by_cached_key(|e| (e.kind != Kind::Dir, if e.kind == Kind::Dir { e.target.to_lowercase() } else { String::new() }));
    fold(&mut entries);

    let clean = |s: &str| s.replace(['\t', '\r', '\n'], " ");
    let text: String = entries
        .iter()
        .map(|e| {
            format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                e.kind.code(),
                clean(&e.name),
                clean(&e.target),
                clean(&e.workdir),
                clean(&e.root),
                clean(&e.resolved),
                clean(&e.owner),
                // c 命令行，g 带窗口；空表示没读过文件头
                if e.console {
                    "c"
                } else if e.kind == Kind::Dir && e.target.to_lowercase().ends_with(".exe") {
                    "g"
                } else {
                    ""
                }
            )
        })
        .collect();
    let _ = fs::create_dir_all(config::cache_dir());
    write_atomic(&index::index_path(), text.as_bytes());
    // 各类型的数量只有完整扫描才数得全
    if !partial {
        let found: String = found.iter().map(|((root, ext), n)| format!("{root}\t{ext}\t{n}\n")).collect();
        write_atomic(&index::found_path(), found.as_bytes());
    }
    notify(WM_INDEX);

    // 常用的排前面：它们最先被看到。目录里扫出来的可能成千上万，没用过的等显示到时再取
    let store = config::Store::load();
    let used = |e: &Entry| store.usage.get(&e.target).copied().unwrap_or(0);
    entries.retain(|e| e.kind != Kind::Dir || used(e) > 0);
    entries.sort_by_key(|e| std::cmp::Reverse(used(e)));
    let targets: Vec<String> = entries.into_iter().map(|e| e.target).collect();
    icons(&targets, px, force);
    notify(WM_ICONS);
}

/// `frun --icons <hwnd> <图标像素> <目标>...`：只取这几个图标。
/// 面板要显示的行还没有图标时用它，不必等整轮扫描。
pub fn run_icons(args: &[String]) {
    let hwnd = args.first().and_then(|a| a.parse::<isize>().ok()).unwrap_or(0);
    let px = args.get(1).and_then(|a| a.parse::<u32>().ok()).unwrap_or(32).clamp(16, 256);
    icons(args.get(2..).unwrap_or(&[]), px, false);
    unsafe {
        if hwnd != 0 {
            let _ = PostMessageW(Some(HWND(hwnd as _)), WM_ICONS, WPARAM(0), LPARAM(0));
        }
    }
}

/// 提取图标并写入缓存。取图标主要在等磁盘和外壳，多开几个线程能快好几倍。
fn icons(targets: &[String], px: u32, force: bool) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let dir = index::icon_dir(px);
    let _ = fs::create_dir_all(&dir);
    let todo: Vec<(&String, std::path::PathBuf)> = targets
        .iter()
        .map(|t| (t, dir.join(format!("{:016x}", index::icon_key(t)))))
        .filter(|(_, file)| force || !file.exists())
        .collect();
    let next = AtomicUsize::new(0);
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8).min(todo.len().max(1));
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                unsafe {
                    let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                }
                while let Some((target, file)) = todo.get(next.fetch_add(1, Ordering::Relaxed)) {
                    if let Some(bits) = icon(target, px as i32) {
                        write_atomic(file, &bits);
                    }
                }
            });
        }
    });
}

/// 先写临时文件再改名，主进程不会读到写了一半的文件。
fn write_atomic(path: &Path, bytes: &[u8]) {
    let tmp = path.with_extension(format!("tmp{:?}", std::thread::current().id()));
    if fs::write(&tmp, bytes).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}

/// 不进去找的目录：系统与隐藏目录、依赖与缓存目录。它们里面的可执行文件不是用户要启动的程序。
pub fn skip_dir(name: &str) -> bool {
    let name = name.to_lowercase();
    name.starts_with(['.', '$'])
        || ["node_modules", "__pycache__", "system volume information", "winsxs", "windows.old"].contains(&name.as_str())
}

/// 缓存目录的通用标记（cargo 的 target 等都带）：有它的目录整个跳过。
pub const CACHE_TAG: &str = "CACHEDIR.TAG";

/// 把一个用户目录完整走一遍，只收指定类型的文件。
/// 遍历主要在等磁盘，按目录分给多个线程。
fn walk(
    start: &Path,
    root: &str,
    exts: &[String],
    known: &HashMap<String, bool>,
    out: &mut Vec<Entry>,
    found: &mut BTreeMap<(String, String), usize>,
) {
    use std::os::windows::fs::MetadataExt;
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    let queue = Mutex::new(vec![PathBuf::from(start)]);
    let pending = AtomicUsize::new(1);
    let total = AtomicUsize::new(out.len());
    let results = Mutex::new((Vec::new(), BTreeMap::<String, usize>::new()));
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                unsafe {
                    let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                }
                let mut mine = Vec::new();
                let mut counts = BTreeMap::<String, usize>::new();
                loop {
                    let next = queue.lock().unwrap().pop();
                    let Some(dir) = next else {
                        if pending.load(Ordering::Acquire) == 0 {
                            break;
                        }
                        std::thread::yield_now();
                        continue;
                    };
                    let (mut subdirs, mut files) = (Vec::new(), Vec::new());
                    let mut cache = false;
                    for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
                        // 目录项自带类型和属性，不必为每个文件再问一次磁盘
                        let Ok(kind) = entry.file_type() else { continue };
                        let name = entry.file_name();
                        let Some(name) = name.to_str() else { continue };
                        if kind.is_dir() {
                            let hidden = entry.metadata().is_ok_and(|m| m.file_attributes() & 2 != 0);
                            // 链接和挂载点不跟进去，避免绕圈
                            if !kind.is_symlink() && !hidden && !skip_dir(name) {
                                subdirs.push(entry.path());
                            }
                        } else if name == CACHE_TAG {
                            cache = true;
                            break;
                        } else if let Some((stem, ext)) = name.rsplit_once('.') {
                            let ext = ext.to_ascii_lowercase();
                            if LAUNCHABLE.contains(&ext.as_str()) || exts.contains(&ext) {
                                files.push((entry.path(), stem.to_string(), ext));
                            }
                        }
                    }
                    if !cache {
                        for (path, stem, ext) in files {
                            *counts.entry(ext.clone()).or_default() += 1;
                            if !exts.contains(&ext) {
                                continue;
                            }
                            let target = path.to_string_lossy().into_owned();
                            let resolved =
                                if ext == "lnk" { unsafe { link_target(&target) } } else { resolved(&target, "") };
                            if !is_junk(&stem, resolved.split('|').next().unwrap_or("")) {
                                let console = ext == "exe"
                                    && known.get(&target.to_lowercase()).copied().unwrap_or_else(|| is_console(&path));
                                mine.push(Entry {
                                    name: stem,
                                    target,
                                    workdir: dir.to_string_lossy().into_owned(),
                                    kind: Kind::Dir,
                                    root: root.to_string(),
                                    resolved,
                                    owner: String::new(),
                                    console,
                                });
                                progress(total.fetch_add(1, Ordering::Relaxed) + 1);
                            }
                        }
                        pending.fetch_add(subdirs.len(), Ordering::AcqRel);
                        queue.lock().unwrap().append(&mut subdirs);
                    }
                    pending.fetch_sub(1, Ordering::AcqRel);
                }
                let mut all = results.lock().unwrap();
                all.0.append(&mut mine);
                for (ext, n) in counts {
                    *all.1.entry(ext).or_default() += n;
                }
            });
        }
    });
    let (mut entries, counts) = results.into_inner().unwrap();
    out.append(&mut entries);
    for (ext, n) in counts {
        *found.entry((root.to_string(), ext)).or_default() += n;
    }
}

/// `path` 是不是在 `base` 里面（或就是它）。
fn under(path: &str, base: &str) -> bool {
    let (path, base) = (path.to_lowercase(), base.trim_end_matches(['\\', '/']).to_lowercase());
    path.strip_prefix(&base).is_some_and(|rest| rest.is_empty() || rest.starts_with(['\\', '/']))
}

/// 读 exe 文件头里的子系统：3 是命令行程序，没有自己的窗口。
fn is_console(path: &Path) -> bool {
    use std::io::Read;
    let mut buf = [0u8; 1024];
    let Ok(n) = fs::File::open(path).and_then(|mut f| f.read(&mut buf)) else { return false };
    let at = |i: usize, len: usize| buf[..n].get(i..i + len);
    let Some(pe) = at(0x3C, 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize) else { return false };
    // PE 签名 4 字节 + COFF 头 20 字节，子系统在可选头的第 68 字节
    at(pe + 24 + 68, 2).is_some_and(|b| u16::from_le_bytes([b[0], b[1]]) == 3)
}

/// 文件夹名太泛，不能当作程序名来认主程序。
const GENERIC: &[&str] = &[
    "bin", "x64", "x86", "win64", "win32", "amd64", "arm64", "tools", "tool", "lib", "libs", "release", "debug", "app",
    "apps", "program", "programs", "software", "dist", "build", "scripts", "cmd", "usr", "current", "portable",
];

/// 比较程序名和文件夹名时用的写法：去掉位数、版本号和标点。
fn plain(name: &str) -> String {
    let mut s = name.to_lowercase();
    for part in ["x64", "x86", "win64", "win32", "64bit", "32bit", "portable"] {
        s = s.replace(part, "");
    }
    let s: String = s.chars().filter(|c| c.is_alphanumeric()).collect();
    s.trim_end_matches(|c: char| c.is_ascii_digit()).to_string()
}

/// 一个程序目录只让主程序占一行：认出每个目录的主程序，把同目录的其他程序记在它名下。
///
/// 主程序是名字和所在文件夹（或上两级）对得上的那个；有好几个时，
/// 开始菜单已经指向的优先，其次是完全同名的、带窗口的、离得近的、名字短的。
/// 认不出主程序的目录（一堆互不相干的工具）保持原样。
fn fold(entries: &mut [Entry]) {
    let real = |e: &Entry| e.resolved.split('|').next().unwrap_or("").to_string();
    // 开始菜单已经指向的文件 -> 那一项的启动目标
    let mut pinned: HashMap<String, String> = HashMap::new();
    for e in entries.iter().filter(|e| e.kind != Kind::Dir && !e.resolved.is_empty()) {
        pinned.entry(real(e)).or_insert_with(|| e.target.clone());
    }
    let trim = |s: &str| s.trim_end_matches(['\\', '/']).to_lowercase();
    let parent = |d: &str| d.rsplit_once(['\\', '/']).map(|(p, _)| p.to_string());
    // 程序目录 -> (主程序的好坏, 它是第几条)
    let mut roots: HashMap<String, ((bool, bool, bool, usize, std::cmp::Reverse<usize>), usize)> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        let stem = plain(&e.name);
        if e.kind != Kind::Dir || stem.chars().count() < 3 {
            continue;
        }
        let top = trim(&e.root);
        let mut dir = trim(&e.workdir);
        for level in 0..3 {
            // 整个盘加进来时，盘符不是程序目录
            if dir.len() < top.len() || (dir == top && top.len() <= 2) {
                break;
            }
            let folder = dir.rsplit(['\\', '/']).next().unwrap_or("");
            let name = plain(folder);
            let exact = name == stem;
            let close = name.chars().count() >= 3 && (stem.starts_with(&name) || name.starts_with(&stem));
            if !GENERIC.contains(&folder) && (exact || close) {
                let rank = (pinned.contains_key(&real(e)), exact, !e.console, 3 - level, std::cmp::Reverse(e.name.len()));
                let best = roots.entry(dir.clone()).or_insert((rank, i));
                if rank > best.0 {
                    *best = (rank, i);
                }
                break;
            }
            match parent(&dir) {
                Some(up) if dir != top => dir = up,
                _ => break,
            }
        }
    }
    let mains: HashSet<usize> = roots.values().map(|r| r.1).collect();
    // 开始菜单里有的那一项代表这个程序：目录里的同一个文件收在它后面，主程序名下的也归它
    let face = |i: usize| pinned.get(&real(&entries[i])).cloned().unwrap_or_else(|| entries[i].target.clone());
    let owners: Vec<Option<String>> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            if e.kind != Kind::Dir {
                return None;
            }
            if let Some(start) = pinned.get(&real(e)) {
                return Some(start.clone());
            }
            if mains.contains(&i) {
                return None;
            }
            let top = trim(&e.root);
            let mut dir = trim(&e.workdir);
            loop {
                if let Some(&(_, main)) = roots.get(&dir) {
                    return Some(face(main));
                }
                match parent(&dir) {
                    Some(up) if dir.len() > top.len() => dir = up,
                    _ => return None,
                }
            }
        })
        .collect();
    for (i, owner) in owners.into_iter().enumerate() {
        entries[i].owner = owner.unwrap_or_default();
    }
}

/// 快捷方式指向的路径和参数；解析不了返回空串。
unsafe fn link_target(path: &str) -> String {
    unsafe {
        let Ok(link) = CoCreateInstance::<_, IShellLinkW>(&ShellLink, None, CLSCTX_INPROC_SERVER) else {
            return String::new();
        };
        let Ok(file) = link.cast::<IPersistFile>() else { return String::new() };
        if file.Load(&HSTRING::from(path), STGM_READ).is_err() {
            return String::new();
        }
        let mut buf = [0u16; 520];
        if link.GetPath(&mut buf, std::ptr::null_mut(), 0).is_err() {
            return String::new();
        }
        let mut args = [0u16; 1024];
        let _ = link.GetArguments(&mut args);
        let text = |b: &[u16]| String::from_utf16_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(0)]);
        resolved(&text(&buf), &text(&args))
    }
}

/// shell:AppsFolder 同时包含开始菜单快捷方式和商店应用。
unsafe fn apps(out: &mut Vec<Entry>) -> Result<()> {
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
                (take(item.GetDisplayName(SIGDN_NORMALDISPLAY)), take(item.GetDisplayName(SIGDN_PARENTRELATIVEPARSING)))
            else {
                continue;
            };
            let prop = |key: &PROPERTYKEY| {
                item.cast::<IShellItem2>().and_then(|i| take(i.GetString(key))).unwrap_or_default()
            };
            let path = prop(&PKEY_LINK_TARGET);
            if is_junk(&name, &id) || is_junk(&name, &path) {
                continue;
            }
            out.push(Entry {
                name,
                target: format!("shell:AppsFolder\\{id}"),
                workdir: String::new(),
                // 打包应用的标识形如 <包族名>!<应用名>
                kind: if id.contains('!') { Kind::Store } else { Kind::Start },
                root: String::new(),
                resolved: resolved(&path, &prop(&PKEY_LINK_ARGUMENTS)),
                owner: String::new(),
                console: false,
            });
            progress(out.len());
        }
        Ok(())
    }
}

unsafe fn take(p: Result<PWSTR>) -> Result<String> {
    unsafe {
        let p = p?;
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

/// 取启动目标的图标：px×px、自上而下、预乘 alpha 的 BGRA。
fn icon(target: &str, px: i32) -> Option<Vec<u8>> {
    unsafe {
        let factory: IShellItemImageFactory = SHCreateItemFromParsingName(&HSTRING::from(target), None).ok()?;
        let bmp = factory.GetImage(SIZE { cx: px, cy: px }, SIIGBF_ICONONLY).ok()?;
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: px,
                biHeight: -px,
                biPlanes: 1,
                biBitCount: 32,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = vec![0u8; (px * px * 4) as usize];
        let dc = GetDC(None);
        let rows = GetDIBits(dc, bmp, 0, px as u32, Some(bits.as_mut_ptr() as _), &mut info, DIB_RGB_COLORS);
        ReleaseDC(None, dc);
        let _ = DeleteObject(bmp.into());
        if rows != px {
            return None;
        }
        // 没有 alpha 通道的旧图标：整张视为不透明
        if bits.chunks_exact(4).all(|p| p[3] == 0) {
            bits.chunks_exact_mut(4).for_each(|p| p[3] = 255);
        }
        Some(bits)
    }
}
