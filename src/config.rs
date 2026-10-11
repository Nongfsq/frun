use serde::Deserialize;
use std::{collections::HashMap, fs, path::PathBuf};

const TEMPLATE: &str = r#"# frun 配置。修改后在托盘菜单点「重新加载」生效。

# 呼出热键：Ctrl / Alt / Shift / Win 加一个键，如 "Ctrl+Alt+K"
hotkey = "Alt+Space"

# 额外扫描的目录（收录其中的 .exe 和 .lnk）。用单引号，反斜杠不必转义。
dirs = [
    # 'D:\Tools',
]

# 扫描 dirs 时向下进入几层子目录
depth = 2

# 最多显示几条结果
max_results = 8
"#;

#[derive(Deserialize)]
#[serde(default)]
pub struct Config {
    pub hotkey: String,
    pub dirs: Vec<String>,
    pub depth: usize,
    pub max_results: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self { hotkey: "Alt+Space".into(), dirs: Vec::new(), depth: 2, max_results: 8 }
    }
}

fn dir() -> PathBuf {
    PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("frun")
}

pub fn path() -> PathBuf {
    dir().join("config.toml")
}

/// 读取配置；首次运行时写出带注释的模板。返回解析错误信息（如果有）。
pub fn load() -> (Config, Option<String>) {
    let p = path();
    if !p.exists() {
        let _ = fs::create_dir_all(dir());
        let _ = fs::write(&p, TEMPLATE);
    }
    match toml::from_str(&fs::read_to_string(&p).unwrap_or_default()) {
        Ok(c) => (c, None),
        Err(e) => (Config::default(), Some(e.to_string())),
    }
}

/// "Ctrl+Alt+K" -> (RegisterHotKey 的修饰键位, 虚拟键码)
pub fn parse_hotkey(s: &str) -> Option<(u32, u32)> {
    let (mut mods, mut vk) = (0, None);
    for part in s.split('+').map(|p| p.trim().to_ascii_lowercase()) {
        match part.as_str() {
            "alt" => mods |= 1,
            "ctrl" | "control" => mods |= 2,
            "shift" => mods |= 4,
            "win" => mods |= 8,
            "space" => vk = Some(0x20),
            "tab" => vk = Some(0x09),
            "`" => vk = Some(0xC0),
            k if k.len() == 1 && k.as_bytes()[0].is_ascii_alphanumeric() => {
                vk = Some(k.as_bytes()[0].to_ascii_uppercase() as u32)
            }
            k if k.starts_with('f') => match k[1..].parse::<u32>() {
                Ok(n @ 1..=24) => vk = Some(0x6F + n),
                _ => return None,
            },
            _ => return None,
        }
    }
    vk.filter(|_| mods != 0).map(|vk| (mods, vk))
}

fn usage_path() -> PathBuf {
    dir().join("usage.tsv")
}

/// 启动次数，按启动目标记录，用于排序。
pub fn load_usage() -> HashMap<String, u32> {
    fs::read_to_string(usage_path())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .filter_map(|(n, t)| Some((t.to_string(), n.parse().ok()?)))
        .collect()
}

pub fn save_usage(usage: &HashMap<String, u32>) {
    let text: String = usage.iter().map(|(t, n)| format!("{n}\t{t}\n")).collect();
    let _ = fs::write(usage_path(), text);
}
