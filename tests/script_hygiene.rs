//! 仓库里的脚本不许自己建临时目录、自己启动浏览器：一律经 tools/headless_edge.py。
//! 自己建的会落在 C: 盘的 %TEMP%，进程一被打断就留下；2026-10-15 那里攒了 130 GB。

use std::fs;
use std::path::Path;

const SHARED: &str = "tools/headless_edge.py";
const SCRIPTS: [&str; 8] = ["py", "ps1", "psm1", "bat", "cmd", "sh", "js", "mjs"];
const SKIP: [&str; 4] = ["target", ".cache", ".git", "node_modules"];
/// (小写写法, 该用什么)
const BANNED: [(&str, &str); 17] = [
    ("mkdtemp", "Scratch"),
    ("temporarydirectory", "Scratch"),
    ("namedtemporaryfile", "Scratch"),
    ("gettempdir", "Scratch"),
    ("tempfile.mktemp", "Scratch"),
    ("env:temp", "Scratch"),
    ("env:tmp", "Scratch"),
    ("%temp%", "Scratch"),
    ("%tmp%", "Scratch"),
    ("['temp']", "Scratch"),
    ("('temp')", "Scratch"),
    ("[\"temp\"]", "Scratch"),
    ("(\"temp\")", "Scratch"),
    ("user-data-dir", "Edge"),
    ("msedge", "Edge"),
    ("chrome.exe", "Edge"),
    ("--headless", "Edge"),
];

fn walk(dir: &Path, root: &Path, found: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if !SKIP.contains(&name.as_str()) {
                walk(&path, root, found);
            }
            continue;
        }
        let ext = path.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
        let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
        if !SCRIPTS.contains(&ext.as_str()) || rel == SHARED {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else { continue };
        for (i, line) in text.lines().enumerate() {
            let low = line.to_lowercase();
            if let Some((bad, fix)) = BANNED.iter().find(|(bad, _)| low.contains(bad)) {
                found.push(format!("{rel}:{}: `{bad}` -> 用 {SHARED} 的 {fix}", i + 1));
            }
        }
    }
}

#[test]
fn scripts_use_the_shared_temp_and_browser_helpers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found = Vec::new();
    walk(root, root, &mut found);
    assert!(
        found.is_empty(),
        "这些脚本自己建临时目录或启动浏览器，改成经 {SHARED}：\n{}",
        found.join("\n")
    );
}
