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

/// 脚本类型：在程序目录里时算作程序自带的辅助文件。
const SCRIPTS: &[&str] = &[".bat", ".cmd", ".ps1", ".py", ".vbs", ".jar"];

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
    let began = std::time::Instant::now();
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
    let old = index::read_all();
    let partial = !only.is_empty() && !old.is_empty();
    // 有没有窗口要读文件头才知道；上次读过的不再读
    let known: HashMap<String, bool> = fs::read_to_string(index::index_path())
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            // 旧版本记的 c、g 不算数：认"标成命令行、其实开窗口"的规则变过
            matches!(f.get(7), Some(&"t") | Some(&"w")).then(|| (f[2].to_lowercase(), f[7] == "t"))
        })
        .collect();
    let store = config::Store::load();
    // 上次完整扫描记下的空子树：里面一个可启动的文件都没有，这次不再进去
    let signature = format!("{}|{}", cfg.dirs.join(";"), cfg.exts.join(","));
    let (stamp, old_signature, mut barren) = read_barren();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    // 彻底查一遍并重新记：手动刷新、目录或类型变了、或者距上次已经过了几天
    let deep = !partial && (force || old_signature != signature || now.saturating_sub(stamp) > RECHECK);
    // 用户说了不再查的文件夹，任何时候都不进去
    let mut skip: HashSet<String> = store.hidden_dirs().into_iter().collect();
    let mut seen_dirs = Vec::new();
    let mut entries = Vec::new();
    let mut found = BTreeMap::new();
    if partial {
        let root_of = |p: &str| cfg.dirs.iter().find(|d| under(p, d));
        let apps_too = only.iter().any(|p| root_of(p).is_none());
        entries = old
            .into_iter()
            .filter(|e| match e.kind {
                Kind::Dir | Kind::Skip => {
                    root_of(&e.workdir).is_some() && !only.iter().any(|p| under(&e.workdir, p))
                }
                _ => !apps_too,
            })
            .collect();
        for path in &only {
            if let Some(root) = root_of(path) {
                walk(Path::new(path), root, &cfg.exts, &known, &skip, None, &mut entries, &mut found);
            }
        }
        if apps_too {
            let _ = unsafe { apps(&mut entries) };
        }
        // 变化发生在记为空的子树里：它不再是空的了
        let before = barren.len();
        barren.retain(|b| !only.iter().any(|p| under(p, b)));
        if barren.len() != before {
            write_barren(stamp, &old_signature, &barren);
        }
    } else {
        if !deep {
            skip.extend(barren);
        }
        for dir in &cfg.dirs {
            let record = deep.then_some(&mut seen_dirs);
            walk(Path::new(dir), dir, &cfg.exts, &known, &skip, record, &mut entries, &mut found);
        }
        let _ = unsafe { apps(&mut entries) };
        if deep {
            write_barren(now, &signature, &barren_of(&seen_dirs));
        }
    }
    settle(&mut entries, &declared());

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
                // t 命令行，w 带窗口；空表示没读过文件头
                if e.console {
                    "t"
                } else if matches!(e.kind, Kind::Dir | Kind::Skip)
                    && (e.target.to_lowercase().ends_with(".exe") || e.resolved.split('|').next().unwrap_or("").ends_with(".exe"))
                {
                    "w"
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
    let count = |k: Kind| entries.iter().filter(|e| e.kind == k).count();
    log(&format!(
        "scan {} {} ms: folders {} kept {} skipped, start {} store {}{}",
        if partial { "partial" } else if deep { "deep" } else { "quick" },
        began.elapsed().as_millis(),
        count(Kind::Dir),
        count(Kind::Skip),
        count(Kind::Start),
        count(Kind::Store),
        if partial { format!(" [{}]", only.join(", ")) } else { String::new() }
    ));

    // 常用的排前面：它们最先被看到。目录里扫出来的可能成千上万，没用过的等显示到时再取
    let used = |e: &Entry| store.usage.get(&e.target).copied().unwrap_or(0);
    entries.retain(|e| e.kind != Kind::Skip && (e.kind != Kind::Dir || used(e) > 0));
    entries.sort_by_key(|e| std::cmp::Reverse(used(e)));
    let targets: Vec<String> = entries.into_iter().map(|e| e.target).collect();
    icons(&targets, px, force);
    notify(WM_ICONS);
}

/// `frun --icons <hwnd> <图标像素> <目标>...`：只取这几个图标。
/// 面板要显示的行还没有图标时用它，不必等整轮扫描。
/// 扫到的条目定下去留和归属：哪些不收，哪些是主程序，哪些收在主程序后面。
/// `declared` 是 Windows 记下的主程序（见 [`declared`]）。
fn settle(entries: &mut Vec<Entry>, declared: &HashMap<String, String>) {
    let mut seen = HashSet::new();
    entries.retain(|e| seen.insert(e.target.to_lowercase()));
    // 上次没收的这次重新判断：它所在的程序目录可能变了。
    // 命令行程序只在终端里用，没有人会从启动器里打开：一律不收，也不参与认主程序
    for e in entries.iter_mut().filter(|e| matches!(e.kind, Kind::Dir | Kind::Skip)) {
        e.kind = if e.console { Kind::Skip } else { Kind::Dir };
    }
    // 目录里的排前面并按路径排序，结果与扫描的先后无关
    entries.sort_by_cached_key(|e| (e.kind != Kind::Dir, if e.kind == Kind::Dir { e.target.to_lowercase() } else { String::new() }));
    fold(entries, declared);
    // 程序目录里收在主程序后面的脚本是程序自带的辅助文件：也不收。自己放的零散脚本照常收
    for e in entries.iter_mut() {
        let script = SCRIPTS.iter().any(|x| e.target.to_lowercase().ends_with(x));
        if e.kind == Kind::Dir && !e.owner.is_empty() && script {
            e.kind = Kind::Skip;
        }
    }
}

/// Windows 记下的各程序的主程序：exe 路径 -> 它的安装目录，都是小写。
/// 来自"应用和功能"的卸载信息（图标指向的 exe 和安装位置）和 App Paths。
/// 这份记录不看文件夹怎么起名，换哪台电脑都一样准，所以认主程序时排在按名字猜的前面。
pub fn declared() -> HashMap<String, String> {
    use windows::Win32::System::Registry::*;
    const UNINSTALL: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";
    const APP_PATHS: &str = r"Software\Microsoft\Windows\CurrentVersion\App Paths";
    let sources = [
        (HKEY_LOCAL_MACHINE, KEY_WOW64_64KEY, UNINSTALL),
        (HKEY_LOCAL_MACHINE, KEY_WOW64_32KEY, UNINSTALL),
        (HKEY_CURRENT_USER, REG_SAM_FLAGS(0), UNINSTALL),
        (HKEY_LOCAL_MACHINE, KEY_WOW64_64KEY, APP_PATHS),
        (HKEY_CURRENT_USER, REG_SAM_FLAGS(0), APP_PATHS),
    ];
    // 去掉引号和图标序号（"C:\a\b.exe",0）
    let clean = |s: String| {
        let s = s.trim().trim_matches('"');
        let s = match s.rsplit_once(',') {
            Some((path, n)) if n.trim().parse::<i32>().is_ok() => path,
            _ => s,
        };
        s.trim().trim_matches('"').trim_end_matches('\\').to_lowercase()
    };
    let mut out = HashMap::new();
    // App Paths 登记的是"能直接按名字运行的 exe"：有的程序把自带的几十个工具全登记了（Calibre 连内部进程都在里面），
    // 所以只有一个程序只登记了自己这一个时，才当它是主程序
    let mut paths: HashMap<String, Vec<String>> = HashMap::new();
    for (hive, view, path) in sources {
        unsafe {
            let mut key = HKEY::default();
            if RegOpenKeyExW(hive, &HSTRING::from(path), None, KEY_READ | view, &mut key).is_err() {
                continue;
            }
            for i in 0.. {
                let mut name = [0u16; 260];
                let mut len = name.len() as u32;
                if RegEnumKeyExW(key, i, Some(PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None).is_err() {
                    break;
                }
                let text = |value: &str| {
                    let mut buf = [0u16; 1024];
                    let mut size = (buf.len() * 2) as u32;
                    let value = HSTRING::from(value);
                    let ok = RegGetValueW(
                        key,
                        PCWSTR(name.as_ptr()),
                        if value.is_empty() { PCWSTR::null() } else { PCWSTR(value.as_ptr()) },
                        RRF_RT_REG_SZ,
                        None,
                        Some(buf.as_mut_ptr() as _),
                        Some(&mut size),
                    );
                    if ok.is_err() { String::new() } else { clean(String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)])) }
                };
                let (exe, home) =
                    if path == UNINSTALL { (text("DisplayIcon"), text("InstallLocation")) } else { (text(""), text("Path")) };
                let stem = exe.rsplit(['\\', '/']).next().unwrap_or("");
                // 卸载信息的图标有时指向卸载程序或安装程序本身
                let tool = ["unins", "uninst", "setup", "install", "update"].iter().any(|t| stem.contains(t));
                if !exe.ends_with(".exe") || tool {
                    continue;
                }
                let parent = exe.rsplit_once('\\').map_or(String::new(), |(p, _)| p.to_string());
                // 安装位置要真的包住这个 exe，而且不是整个盘、离它不超过三层；否则就用 exe 所在的文件夹
                let near = |h: &str| h.len() > 3 && under(&exe, h) && exe[h.len()..].matches('\\').count() <= 3;
                let home = if near(&home) { home } else { parent };
                if path == UNINSTALL {
                    out.insert(exe, home);
                } else {
                    paths.entry(home).or_default().push(exe);
                }
            }
            let _ = RegCloseKey(key);
        }
    }
    let taken: HashSet<String> = out.values().cloned().collect();
    for (home, exes) in paths {
        if let [exe] = exes.as_slice()
            && !taken.contains(&home)
        {
            out.entry(exe.clone()).or_insert(home);
        }
    }
    tidy(out)
}

/// 安装位置有时写的是上一层的总目录（"D:\Software"）：名字太泛，或者底下装着别的登记程序，
/// 就不是这一个程序的目录，只认 exe 所在的文件夹。
fn tidy(mut out: HashMap<String, String>) -> HashMap<String, String> {
    let mut parents: HashMap<String, HashSet<String>> = HashMap::new();
    for (exe, home) in &out {
        let parent = exe.rsplit_once('\\').map_or("", |(p, _)| p);
        parents.entry(home.clone()).or_default().insert(parent.to_string());
    }
    for (exe, home) in out.iter_mut() {
        let folder = home.rsplit('\\').next().unwrap_or("");
        let shared = parents.get(home.as_str()).is_some_and(|p| p.len() > 1);
        if GENERIC.contains(&folder) || shared {
            *home = exe.rsplit_once('\\').map_or(String::new(), |(p, _)| p.to_string());
        }
    }
    out
}

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
    skip: &HashSet<String>,
    record: Option<&mut Vec<(String, bool)>>,
    out: &mut Vec<Entry>,
    found: &mut BTreeMap<(String, String), usize>,
) {
    let recording = record.is_some();
    use std::os::windows::fs::MetadataExt;
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    let queue = Mutex::new(vec![PathBuf::from(start)]);
    let pending = AtomicUsize::new(1);
    let total = AtomicUsize::new(out.len());
    let results = Mutex::new((Vec::new(), BTreeMap::<String, usize>::new(), Vec::new()));
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                unsafe {
                    let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                }
                let mut mine = Vec::new();
                let mut counts = BTreeMap::<String, usize>::new();
                // 走过的目录和它里面有没有可启动的文件，用来找出空子树
                let mut walked: Vec<(String, bool)> = Vec::new();
                let key = |p: &Path| p.to_string_lossy().trim_end_matches('\\').to_lowercase();
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
                                let path = entry.path();
                                if skip.is_empty() || !skip.contains(&key(&path)) {
                                    subdirs.push(path);
                                }
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
                    if recording {
                        walked.push((key(&dir), !cache && !files.is_empty()));
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
                            let real = resolved.split('|').next().unwrap_or("");
                            // 指向文件夹的快捷方式不是程序。程序总有扩展名；没有的是文件夹，包括连不上的网络共享
                            let bare = !real.rsplit(['\\', '/']).next().unwrap_or("").contains('.');
                            let folder = ext == "lnk"
                                && !real.is_empty()
                                && (bare || fs::metadata(real).is_ok_and(|m| m.is_dir()));
                            if !folder && !is_junk(&stem, real) {
                                let cached = || known.get(&target.to_lowercase()).copied();
                                let console = match ext.as_str() {
                                    "exe" => cached().unwrap_or_else(|| is_console(&path)),
                                    // 指向命令行程序的快捷方式同样不收；指向系统自带程序（cmd、PowerShell）的是有意放的，照收
                                    "lnk" if real.ends_with(".exe") && !under(real, &system_root()) => {
                                        cached().unwrap_or_else(|| is_console(Path::new(real)))
                                    }
                                    _ => false,
                                };
                                let name = if ext == "lnk" { own_name(&stem, real) } else { stem };
                                mine.push(Entry {
                                    name,
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
                all.2.append(&mut walked);
                for (ext, n) in counts {
                    *all.1.entry(ext).or_default() += n;
                }
            });
        }
    });
    let (mut entries, counts, mut walked) = results.into_inner().unwrap();
    out.append(&mut entries);
    if let Some(record) = record {
        record.append(&mut walked);
    }
    for (ext, n) in counts {
        *found.entry((root.to_string(), ext)).or_default() += n;
    }
}

pub fn log_path() -> PathBuf {
    config::cache_dir().join("frun.log")
}

/// 往日志里记一行（带 UTC 时间）。日志只留最近的一段，供 `--doctor` 附在报告里。
pub fn log(line: &str) {
    use std::io::Write;
    let path = log_path();
    if fs::metadata(&path).is_ok_and(|m| m.len() > 256 * 1024)
        && let Ok(text) = fs::read_to_string(&path)
    {
        let keep = text.len() / 2;
        let from = text[keep..].find('\n').map_or(keep, |i| keep + i + 1);
        let _ = fs::write(&path, &text[from..]);
    }
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    // 从 1970 年起的天数换成年月日
    let (days, rest) = ((secs / 86400) as i64 + 719_468, secs % 86400);
    let era = days.div_euclid(146_097);
    let doe = days.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let (day, month) = (doy - (153 * mp + 2) / 5 + 1, if mp < 10 { mp + 3 } else { mp - 9 });
    let year = yoe + era * 400 + (month <= 2) as i64;
    let stamp = format!("{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}Z", rest / 3600, rest % 3600 / 60, rest % 60);
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{stamp} {line}");
    }
}

/// 空子树多久彻底核对一次（秒）。期间靠目录监视发现里面新出现的程序。
const RECHECK: u64 = 3 * 24 * 3600;

fn barren_path() -> PathBuf {
    config::cache_dir().join("barren.tsv")
}

/// (记录时间, 当时的目录与类型, 空子树的根)
pub fn read_barren() -> (u64, String, Vec<String>) {
    let text = fs::read_to_string(barren_path()).unwrap_or_default();
    let mut lines = text.lines();
    let (stamp, signature) = lines.next().and_then(|l| l.split_once('\t')).unwrap_or(("0", ""));
    (stamp.parse().unwrap_or(0), signature.to_string(), lines.map(str::to_string).collect())
}

fn write_barren(stamp: u64, signature: &str, dirs: &[String]) {
    let mut text = format!("{stamp}\t{signature}\n");
    for d in dirs {
        text.push_str(d);
        text.push('\n');
    }
    write_atomic(&barren_path(), text.as_bytes());
}

/// 从走过的目录里找出空子树的根：自己和所有下级都没有可启动的文件，而上一级有。
fn barren_of(walked: &[(String, bool)]) -> Vec<String> {
    let up = |d: &str| d.rsplit_once('\\').map(|(p, _)| p.to_string());
    let mut live: HashSet<String> = HashSet::new();
    for (dir, hit) in walked {
        let mut at = Some(dir.clone());
        while let Some(d) = at.take_if(|_| *hit) {
            at = up(&d);
            if !live.insert(d) {
                break;
            }
        }
    }
    walked
        .iter()
        .filter(|(d, _)| !live.contains(d) && up(d).is_some_and(|p| live.contains(&p)))
        .map(|(d, _)| d.clone())
        .collect()
}

/// `path` 是不是在 `base` 里面（或就是它）。
pub fn under(path: &str, base: &str) -> bool {
    let (path, base) = (path.to_lowercase(), base.trim_end_matches(['\\', '/']).to_lowercase());
    path.strip_prefix(&base).is_some_and(|rest| rest.is_empty() || rest.starts_with(['\\', '/']))
}

/// Windows 目录，小写。
fn system_root() -> String {
    std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()).to_lowercase()
}

/// 快捷方式的名字里反复出现它指向的程序名时，名字是给别的工具堆的关键词
/// （"工具 - Anki - 记忆卡片 - anki.exe - anki"）：只取程序名那一段，保留原来的大小写。
/// 程序名先看文件名，再看它所在的两层文件夹（"Fluent Search\FluentSearch.exe"）。
fn own_name(name: &str, real: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut parts = real.rsplit(['\\', '/']);
    let file = parts.next().unwrap_or("");
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
    for want in [stem].into_iter().chain(parts.take(2)) {
        let want: Vec<char> = want.chars().collect();
        if want.len() < 2 || want.len() > chars.len() {
            continue;
        }
        let hits: Vec<usize> = (0..=chars.len() - want.len())
            .filter(|&i| chars[i..i + want.len()].iter().zip(&want).all(|(a, b)| a.to_lowercase().eq(b.to_lowercase())))
            // 必须是完整的一个词："Assistant" 里的 ss 不算
            .filter(|&i| {
                let word = |c: Option<&char>| c.is_some_and(|c| c.is_alphanumeric());
                !word(i.checked_sub(1).and_then(|j| chars.get(j))) && !word(chars.get(i + want.len()))
            })
            .collect();
        if hits.len() >= 2 {
            return chars[hits[0]..hits[0] + want.len()].iter().collect();
        }
    }
    name.to_string()
}

/// 只用来开窗口画画面的库：标成命令行的程序链接了它们，其实是有窗口的。
/// Qt 不算：标成命令行又链接 Qt 的多是程序自带的小工具（qtdiag、格式转换器），正式的 Qt 窗口程序都标成窗口程序
const WINDOW_LIBS: &[&str] = &["sdl2.dll", "sdl3.dll", "glfw3.dll"];

/// 读 exe 文件头判断是不是命令行程序（子系统 3），只在终端里用。
/// 有的程序标成命令行、运行时却开自己的窗口：直接链接 SDL、GLFW 的（scrcpy），
/// 或 PyInstaller 打包、里面带着 Qt、Tk、wx 的。这些算带窗口的。
fn is_console(path: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = fs::File::open(path) else { return false };
    let mut head = vec![0u8; 4096];
    let Ok(n) = f.read(&mut head) else { return false };
    head.truncate(n);
    let u16_at = |b: &[u8], i: usize| b.get(i..i + 2).map(|v| u16::from_le_bytes([v[0], v[1]]) as usize);
    let u32_at = |b: &[u8], i: usize| b.get(i..i + 4).map(|v| u32::from_le_bytes(v.try_into().unwrap()) as usize);
    let Some(pe) = u32_at(&head, 0x3C) else { return false };
    // PE 签名 4 字节 + COFF 头 20 字节，之后是可选头；子系统在可选头的第 68 字节
    let opt = pe + 24;
    if u16_at(&head, opt + 68) != Some(3) {
        return false;
    }
    let len = f.metadata().map_or(0, |m| m.len() as usize);
    let mut read_at = |at: usize, len: usize| -> Vec<u8> {
        let mut buf = vec![0u8; len];
        let n = f.seek(SeekFrom::Start(at as u64)).and_then(|_| f.read(&mut buf)).unwrap_or(0);
        buf.truncate(n);
        buf
    };
    // 导入的动态库：导入表在数据目录的第 2 项
    let dirs = if u16_at(&head, opt) == Some(0x20b) { opt + 112 } else { opt + 96 };
    let sections = opt + u16_at(&head, pe + 20).unwrap_or(0);
    let count = u16_at(&head, pe + 6).unwrap_or(0).min(96);
    let offset = |rva: usize| {
        (0..count).find_map(|i| {
            let s = sections + i * 40;
            let (size, va, raw) = (u32_at(&head, s + 8)?.max(u32_at(&head, s + 16)?), u32_at(&head, s + 12)?, u32_at(&head, s + 20)?);
            (va..va + size).contains(&rva).then(|| rva - va + raw)
        })
    };
    if let Some(table) = u32_at(&head, dirs + 8).filter(|&r| r != 0).and_then(offset) {
        let table = read_at(table, 20 * 128);
        for d in table.chunks_exact(20) {
            let Some(name) = u32_at(d, 12).filter(|&r| r != 0) else { break };
            let Some(at) = offset(name) else { continue };
            let raw = read_at(at, 32);
            let lib = String::from_utf8_lossy(raw.split(|&b| b == 0).next().unwrap_or(&[])).to_ascii_lowercase();
            if WINDOW_LIBS.contains(&lib.as_str()) {
                return false;
            }
        }
    }
    // PyInstaller 单文件：文件末尾有一个标记，指向打包内容的目录
    let tail = read_at(len.saturating_sub(4096), 4096);
    const MAGIC: &[u8] = b"MEI\x0c\x0b\x0a\x0b\x0e";
    if let Some(i) = tail.windows(8).rposition(|w| w == MAGIC) {
        let be = |i: usize| tail.get(i..i + 4).map_or(0, |v| u32::from_be_bytes(v.try_into().unwrap()) as usize);
        let (pkg, toc, toc_len) = (be(i + 8), be(i + 12), be(i + 16));
        let start = (len.saturating_sub(4096) + i + 88).saturating_sub(pkg);
        let names = read_at(start + toc, toc_len.min(8 << 20)).to_ascii_lowercase();
        let gui = [&b"qt5gui"[..], b"qt6gui", b"_tkinter", b"wxmsw", b"wx\\_core"];
        if gui.iter().any(|g| names.windows(g.len()).any(|w| w == *g)) {
            return false;
        }
    }
    true
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
/// Windows 记下的主程序（`declared`）不用对名字：它的安装目录就是程序目录，它就是主程序。
/// 认不出主程序的目录（一堆互不相干的工具）保持原样。
fn fold(entries: &mut [Entry], declared: &HashMap<String, String>) {
    let real = |e: &Entry| e.resolved.split('|').next().unwrap_or("").to_string();
    // 开始菜单已经指向的文件 -> 那一项的启动目标
    let mut pinned: HashMap<String, String> = HashMap::new();
    for e in entries.iter().filter(|e| matches!(e.kind, Kind::Start | Kind::Store) && !e.resolved.is_empty()) {
        pinned.entry(real(e)).or_insert_with(|| e.target.clone());
    }
    // 快捷方式是人有意放的指针，不是程序目录的一部分：不当主程序，也不收在别人后面
    let lnk = |e: &Entry| e.target.to_lowercase().ends_with(".lnk");
    let trim = |s: &str| s.trim_end_matches(['\\', '/']).to_lowercase();
    let parent = |d: &str| d.rsplit_once(['\\', '/']).map(|(p, _)| p.to_string());
    // 程序目录 -> (主程序的好坏, 它是第几条)
    type Rank = (bool, bool, bool, bool, usize, std::cmp::Reverse<usize>);
    let mut roots: HashMap<String, (Rank, usize)> = HashMap::new();
    let mut claim = |dir: String, rank: Rank, i: usize| {
        let best = roots.entry(dir).or_insert((rank, i));
        if rank > best.0 {
            *best = (rank, i);
        }
    };
    for (i, e) in entries.iter().enumerate() {
        if e.kind != Kind::Dir || lnk(e) {
            continue;
        }
        if let Some(home) = declared.get(&e.target.to_lowercase()) {
            // 安装目录超出了这个搜索目录时，只认 exe 所在的文件夹
            let dir = if under(home, &e.root) && trim(home).len() > trim(&e.root).len() { trim(home) } else { trim(&e.workdir) };
            let rank = (pinned.contains_key(&real(e)), true, true, !e.console, 3, std::cmp::Reverse(e.name.len()));
            claim(dir, rank, i);
        }
    }
    for (i, e) in entries.iter().enumerate() {
        let stem = plain(&e.name);
        if e.kind != Kind::Dir || lnk(e) || stem.chars().count() < 3 {
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
                let mine = declared.contains_key(&e.target.to_lowercase());
                let rank = (pinned.contains_key(&real(e)), mine, exact, !e.console, 3 - level, std::cmp::Reverse(e.name.len()));
                claim(dir.clone(), rank, i);
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
            if mains.contains(&i) || lnk(e) {
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
pub fn is_junk(name: &str, id: &str) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 一个只有文件头的假 exe：子系统 2 是窗口程序，3 是命令行程序。扫描只看文件头。
    fn exe(path: &Path, console: bool) {
        let mut b = vec![0u8; 0x200];
        b[..2].copy_from_slice(b"MZ");
        b[0x3C] = 0x80;
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        b[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes());
        b[0x94..0x96].copy_from_slice(&240u16.to_le_bytes());
        b[0x98..0x9A].copy_from_slice(&0x20bu16.to_le_bytes());
        b[0x98 + 68] = if console { 3 } else { 2 };
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b).unwrap();
    }

    /// 在临时目录里摆出一种常见的目录结构，扫一遍，看哪些程序各占一行。
    struct Layout {
        root: PathBuf,
    }

    impl Layout {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("frun-layout-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Layout { root }
        }
        fn gui(&self, rel: &str) -> &Self {
            exe(&self.root.join(rel), false);
            self
        }
        fn cli(&self, rel: &str) -> &Self {
            exe(&self.root.join(rel), true);
            self
        }
        fn file(&self, rel: &str) -> &Self {
            let p = self.root.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, "@echo off").unwrap();
            self
        }
        fn path(&self, rel: &str) -> String {
            self.root.join(rel).to_string_lossy().to_lowercase()
        }
        /// 各占一行的程序（排好序），以及每个主程序后面收了几个。`declared` 是 Windows 的记录：(exe, 安装目录)
        fn scan(&self, declared: &[(&str, &str)]) -> (Vec<String>, BTreeMap<String, usize>) {
            let root = self.root.to_string_lossy().into_owned();
            let exts: Vec<String> = ["exe", "lnk", "bat", "cmd"].map(String::from).to_vec();
            let (mut out, mut found) = (Vec::new(), BTreeMap::new());
            walk(&self.root, &root, &exts, &HashMap::new(), &HashSet::new(), None, &mut out, &mut found);
            settle(&mut out, &tidy(declared.iter().map(|(e, h)| (self.path(e), self.path(h))).collect()));
            let shown = out.iter().filter(|e| e.kind == Kind::Dir);
            let mut rows: Vec<String> = shown.clone().filter(|e| e.owner.is_empty()).map(|e| e.name.clone()).collect();
            rows.sort();
            let mut behind = BTreeMap::new();
            for e in shown.filter(|e| !e.owner.is_empty()) {
                let main = out.iter().find(|m| m.target == e.owner).map_or(e.owner.clone(), |m| m.name.clone());
                *behind.entry(main).or_default() += 1;
            }
            (rows, behind)
        }
    }

    impl Drop for Layout {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn rows(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn an_installed_program_takes_one_row() {
        let l = Layout::new("installed");
        l.gui(r"Oracle\VirtualBox\VirtualBox.exe")
            .gui(r"Oracle\VirtualBox\VirtualBoxVM.exe")
            .gui(r"Oracle\VirtualBox\VBoxSDS.exe")
            .cli(r"Oracle\VirtualBox\VBoxManage.exe")
            .gui(r"Oracle\VirtualBox\uninstall.exe");
        let (shown, behind) = l.scan(&[]);
        assert_eq!(shown, rows(&["VirtualBox"]));
        assert_eq!(behind["VirtualBox"], 2);
    }

    #[test]
    fn windows_records_name_the_main_program_when_names_do_not() {
        let l = Layout::new("declared");
        l.gui(r"Rockstar\Launcher.exe").gui(r"Rockstar\RockstarService.exe");
        // 只按名字猜：和文件夹同名的是一个后台服务
        assert_eq!(l.scan(&[]).0, rows(&["RockstarService"]));
        // Windows 记下的主程序是 Launcher
        let (shown, behind) = l.scan(&[(r"Rockstar\Launcher.exe", "Rockstar")]);
        assert_eq!(shown, rows(&["Launcher"]));
        assert_eq!(behind["Launcher"], 1);
    }

    #[test]
    fn a_broad_install_location_does_not_swallow_its_neighbours() {
        let l = Layout::new("broad");
        l.gui(r"Software\Alpha\Alpha.exe").gui(r"Software\Beta\Beta.exe").gui(r"Shared\Gamma\Gamma.exe").gui(r"Shared\Delta\Delta.exe");
        let records = [
            (r"Software\Alpha\Alpha.exe", "Software"),
            (r"Shared\Gamma\Gamma.exe", "Shared"),
            (r"Shared\Delta\Delta.exe", "Shared"),
        ];
        assert_eq!(l.scan(&records).0, rows(&["Alpha", "Beta", "Delta", "Gamma"]));
    }

    #[test]
    fn loose_tools_each_take_a_row_and_console_tools_none() {
        let l = Layout::new("tools");
        l.gui(r"Tools\ImgView.exe").gui(r"Tools\KeyTest.exe").cli(r"Tools\payload-dumper.exe");
        assert_eq!(l.scan(&[]).0, rows(&["ImgView", "KeyTest"]));
    }

    #[test]
    fn a_self_updating_app_shows_the_app_not_its_updater() {
        let l = Layout::new("squirrel");
        l.gui(r"Discord\app-1.0.9\Discord.exe").gui(r"Discord\Update.exe");
        let (shown, behind) = l.scan(&[]);
        assert_eq!(shown, rows(&["Discord"]));
        assert_eq!(behind["Discord"], 1);
    }

    #[test]
    fn a_version_folder_sits_between_the_program_and_its_name() {
        let l = Layout::new("versioned");
        l.gui(r"livehime\8.9.0.11148\livehime.exe").gui(r"livehime\8.9.0.11148\CrashReport.exe");
        assert_eq!(l.scan(&[]).0, rows(&["livehime"]));
    }

    #[test]
    fn a_game_shows_the_game_not_its_crash_handler() {
        let l = Layout::new("game");
        l.gui(r"SteamLibrary\steamapps\common\Hollow Knight\hollow_knight.exe")
            .gui(r"SteamLibrary\steamapps\common\Hollow Knight\UnityCrashHandler64.exe")
            .gui(r"SteamLibrary\steamapps\common\Celeste\Celeste.exe");
        assert_eq!(l.scan(&[]).0, rows(&["Celeste", "hollow_knight"]));
    }

    #[test]
    fn scripts_inside_a_program_fold_away_but_loose_scripts_stay() {
        let l = Layout::new("scripts");
        l.gui(r"Foo\Foo.exe").file(r"Foo\start.bat").file(r"Scripts\backup.bat");
        assert_eq!(l.scan(&[]).0, rows(&["Foo", "backup"]));
    }

    #[test]
    fn dependency_folders_are_never_searched() {
        let l = Layout::new("deps");
        l.gui(r"Obsidian\Obsidian.exe").gui(r"Obsidian\resources\node_modules\x\bin\helper.exe").gui(r"Obsidian\.cache\y.exe");
        let (shown, behind) = l.scan(&[]);
        assert_eq!(shown, rows(&["Obsidian"]));
        assert!(behind.is_empty());
    }

    #[test]
    fn a_keyword_stuffed_shortcut_takes_the_program_name() {
        let stuffed = "工具 - Anki - 记忆卡片 - utility - anki.exe - anki - 工具";
        assert_eq!(own_name(stuffed, r"d:\software\anki\anki.exe"), "Anki");
        assert_eq!(own_name("Anki 2", r"d:\software\anki\anki.exe"), "Anki 2");
        assert_eq!(own_name("记忆卡片", r"d:\software\anki\anki.exe"), "记忆卡片");
        assert_eq!(own_name("AB", r"d:\software\anki\anki.exe"), "AB");
        let aomei = "工具 - AOMEI Partition Assistant - 分区助手 - utility - aomei partition assistant - ss - 工具";
        assert_eq!(own_name(aomei, r"d:\tools\aomei partition assistant\ss.exe"), "AOMEI Partition Assistant");
        let fluent = "软件 - Fluent Search - 快速启动工具。 - utility - fluent search - 工具";
        assert_eq!(own_name(fluent, r"d:\software\fluent search\fluentsearch.exe"), "Fluent Search");
    }
}
