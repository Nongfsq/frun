use crate::files::{write_atomic, write_durable};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

pub const WIDTH_MIN: u32 = 480;
pub const WIDTH_MAX: u32 = 960;
pub const SCALE_MIN: u32 = 80;
pub const SCALE_MAX: u32 = 160;
pub const ROWS_MIN: u32 = 4;
pub const ROWS_MAX: u32 = 12;
pub const DEFAULT_CX: f32 = 0.5;
pub const DEFAULT_TY: f32 = 0.21;

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    /// "zh" 或 "en"；空表示首次运行，跟随系统语言
    pub language: String,
    /// Ctrl / Alt / Shift / Win 加一个键，如 "Ctrl+Alt+K"
    pub hotkey: String,
    pub ime_english: bool,
    /// "system" / "light" / "dark"
    pub theme: String,
    /// 圆角："round" / "small" / "square"
    pub corners: String,
    /// 整体大小，百分比
    pub scale: u32,
    /// 面板宽度，100% 大小下的逻辑像素
    pub width: u32,
    pub max_rows: u32,
    /// 面板中心占屏幕宽度的比例、上沿占屏幕高度的比例
    pub cx: f32,
    pub ty: f32,
    /// "pointer" / "primary"
    pub screen: String,
    pub reduce_motion: bool,
    /// 默认以管理员身份打开
    pub admin: bool,
    /// 每天检查一次新版本，有就在面板收起时换上
    pub auto_update: bool,
    pub dirs: Vec<String>,
    /// 根据 Windows 的安装记录自动加入的目录，界面上标"自动"
    pub auto_dirs: Vec<String>,
    /// 删掉过的自动目录：不再自动加回来
    pub declined: Vec<String>,
    pub exts: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            language: String::new(),
            hotkey: "Alt+Space".into(),
            ime_english: true,
            theme: "system".into(),
            corners: "round".into(),
            scale: 100,
            width: 640,
            max_rows: 8,
            cx: DEFAULT_CX,
            ty: DEFAULT_TY,
            screen: "pointer".into(),
            reduce_motion: false,
            admin: false,
            auto_update: true,
            dirs: Vec::new(),
            auto_dirs: Vec::new(),
            declined: Vec::new(),
            exts: ["exe", "lnk", "bat", "cmd"].map(String::from).to_vec(),
        }
    }
}

impl Config {
    fn sanitize(mut self) -> Self {
        self.scale = self.scale.clamp(SCALE_MIN, SCALE_MAX);
        self.width = self.width.clamp(WIDTH_MIN, WIDTH_MAX);
        self.max_rows = self.max_rows.clamp(ROWS_MIN, ROWS_MAX);
        self.cx = self.cx.clamp(0.0, 1.0);
        self.ty = self.ty.clamp(0.0, 1.0);
        // A discovery loop (issue #2) once appended the same folder thousands of times.
        for list in [&mut self.dirs, &mut self.auto_dirs] {
            let mut seen = HashSet::new();
            list.retain(|d| seen.insert(d.to_lowercase()));
        }
        self
    }
}

/// 配置与使用记录：%APPDATA%\frun
pub fn data_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("frun")
}

/// 索引与图标缓存，可随时删除重建：%LOCALAPPDATA%\frun
pub fn cache_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("frun")
}

pub fn load() -> Config {
    load_from(&data_dir())
}

pub fn save(cfg: &Config) {
    save_to(&data_dir(), cfg)
}

fn load_from(dir: &Path) -> Config {
    let path = dir.join("config.toml");
    let parsed = match fs::read_to_string(&path) {
        Err(e) if e.kind() == ErrorKind::NotFound => return Config::default(),
        read => read.ok().and_then(|text| toml::from_str::<Config>(&text).ok()),
    };
    parsed
        .unwrap_or_else(|| {
            // Moved aside so the next save cannot replace the user's settings with defaults.
            let _ = fs::rename(&path, dir.join("config.toml.bad"));
            Config::default()
        })
        .sanitize()
}

fn save_to(dir: &Path, cfg: &Config) {
    let _ = fs::create_dir_all(dir);
    if let Ok(text) = toml::to_string(cfg) {
        write_durable(&dir.join("config.toml"), text.as_bytes());
    }
}

pub const MOD_ALT: u32 = 1;
pub const MOD_CTRL: u32 = 2;
pub const MOD_SHIFT: u32 = 4;
pub const MOD_WIN: u32 = 8;

/// "Ctrl+Alt+K" -> (RegisterHotKey 的修饰键位, 虚拟键码)
pub fn parse_hotkey(s: &str) -> Option<(u32, u32)> {
    let (mut mods, mut vk) = (0, None);
    for part in s.split('+').map(|p| p.trim().to_ascii_lowercase()) {
        match part.as_str() {
            "alt" => mods |= MOD_ALT,
            "ctrl" | "control" => mods |= MOD_CTRL,
            "shift" => mods |= MOD_SHIFT,
            "win" => mods |= MOD_WIN,
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

/// parse_hotkey 的反向；键不能用作热键时返回 None。
pub fn format_hotkey(mods: u32, vk: u32) -> Option<String> {
    let key = match vk {
        0x20 => "Space".to_string(),
        0x09 => "Tab".to_string(),
        0xC0 => "`".to_string(),
        0x30..=0x39 | 0x41..=0x5A => (vk as u8 as char).to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        _ => return None,
    };
    if mods == 0 {
        return None;
    }
    let names = [(MOD_CTRL, "Ctrl"), (MOD_ALT, "Alt"), (MOD_SHIFT, "Shift"), (MOD_WIN, "Win")];
    let mut parts: Vec<&str> = names.iter().filter(|(m, _)| mods & m != 0).map(|(_, n)| *n).collect();
    parts.push(&key);
    Some(parts.join("+"))
}

/// 使用记录：启动次数、每段输入上次选了谁、重复项里显示哪一个。
#[derive(Default)]
pub struct Store {
    pub usage: HashMap<String, u32>,
    pub picks: HashMap<String, String>,
    pub prefer: HashSet<String>,
    /// 用户选择不再显示的程序：(启动目标, 名称, 来源)。
    /// 启动目标以反斜杠结尾的是一整个不再查的文件夹
    pub hidden: Vec<(String, String, String)>,
}

impl Store {
    pub fn load() -> Self {
        Self::load_from(&data_dir())
    }

    fn load_from(dir: &Path) -> Self {
        let read = |name: &str| fs::read_to_string(dir.join(name)).unwrap_or_default();
        Self {
            usage: read("usage.tsv")
                .lines()
                .filter_map(|l| l.split_once('\t'))
                .filter_map(|(n, t)| Some((t.to_string(), n.parse().ok()?)))
                .collect(),
            picks: read("picks.tsv")
                .lines()
                .filter_map(|l| l.split_once('\t'))
                .map(|(q, t)| (q.to_string(), t.to_string()))
                .collect(),
            prefer: read("prefer.tsv").lines().map(str::to_string).collect(),
            hidden: read("hidden.tsv")
                .lines()
                .filter_map(|l| {
                    let mut f = l.split('\t');
                    Some((f.next()?.to_string(), f.next()?.to_string(), f.next().unwrap_or("").to_string()))
                })
                .collect(),
        }
    }

    /// 不再查的文件夹：小写、不带结尾的反斜杠。
    pub fn hidden_dirs(&self) -> Vec<String> {
        self.hidden
            .iter()
            .filter(|(t, _, _)| t.ends_with('\\'))
            .map(|(t, _, _)| t.trim_end_matches('\\').to_lowercase())
            .collect()
    }

    pub fn is_hidden(&self, target: &str) -> bool {
        self.hidden.iter().any(|(t, _, _)| t == target)
    }

    pub fn save(&self) {
        self.save_to(&data_dir())
    }

    fn save_to(&self, dir: &Path) {
        let _ = fs::create_dir_all(dir);
        let usage: String = self.usage.iter().map(|(t, n)| format!("{n}\t{t}\n")).collect();
        let picks: String = self.picks.iter().map(|(q, t)| format!("{q}\t{t}\n")).collect();
        let prefer: String = self.prefer.iter().map(|t| format!("{t}\n")).collect();
        let hidden: String = self.hidden.iter().map(|(t, n, s)| format!("{t}\t{n}\t{s}\n")).collect();
        // Saved on every launch from the UI thread, so these skip the flush that write_durable adds.
        for (name, text) in [("usage.tsv", usage), ("picks.tsv", picks), ("prefer.tsv", prefer), ("hidden.tsv", hidden)] {
            write_atomic(&dir.join(name), text.as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkey_round_trips() {
        let (mods, vk) = parse_hotkey("Alt+Space").unwrap();
        assert_eq!((mods, vk), (MOD_ALT, 0x20));
        assert_eq!(format_hotkey(mods, vk).unwrap(), "Alt+Space");
        assert_eq!(format_hotkey(MOD_CTRL | MOD_SHIFT, 0x4B).unwrap(), "Ctrl+Shift+K");
        assert!(parse_hotkey("K").is_none());
        assert!(format_hotkey(0, 0x4B).is_none());
    }

    #[test]
    fn config_survives_a_round_trip() {
        let text = toml::to_string(&Config::default()).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.hotkey, "Alt+Space");
        assert_eq!(back.exts, ["exe", "lnk", "bat", "cmd"]);
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> =
            fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn a_broken_config_is_set_aside_not_overwritten() {
        let dir = crate::testdir::TestDir::new("config-broken");
        let broken = "hotkey = \"Ctrl+K\"\ndirs = [\"D:\\\\Tools\"";
        fs::write(dir.join("config.toml"), broken).unwrap();

        let mut cfg = load_from(&dir);
        assert_eq!(cfg.hotkey, "Alt+Space");
        // The first-run save at startup must leave the user's file alone.
        cfg.language = "en".into();
        save_to(&dir, &cfg);

        assert_eq!(fs::read_to_string(dir.join("config.toml.bad")).unwrap(), broken);
        assert_eq!(load_from(&dir).language, "en");
        assert_eq!(names(&dir), ["config.toml", "config.toml.bad"]);
    }

    #[test]
    fn repeated_folders_are_dropped_on_load() {
        let dir = crate::testdir::TestDir::new("config-repeated");
        let difx = r"C:\Program Files\DIFX";
        let cfg = Config {
            dirs: vec![r"D:\Tools".into(), difx.into(), difx.to_lowercase(), r"D:\Tools".into(), difx.into()],
            auto_dirs: vec![difx.into(), difx.into(), difx.into()],
            ..Config::default()
        };
        save_to(&dir, &cfg);
        let back = load_from(&dir);
        assert_eq!(back.dirs, [r"D:\Tools", difx]);
        assert_eq!(back.auto_dirs, [difx]);
    }

    #[test]
    fn a_missing_config_starts_from_defaults() {
        let dir = crate::testdir::TestDir::new("config-missing");
        assert_eq!(load_from(&dir).hotkey, "Alt+Space");
        assert!(names(&dir).is_empty());
    }

    #[test]
    fn settings_and_records_survive_a_save_and_load() {
        let dir = crate::testdir::TestDir::new("config-saved");
        let cfg = Config { hotkey: "Ctrl+Alt+K".into(), dirs: vec![r"D:\Tools".into()], ..Config::default() };
        save_to(&dir, &cfg);
        let mut store = Store::default();
        store.usage.insert(r"D:\Tools\a.exe".into(), 3);
        store.picks.insert("a".into(), r"D:\Tools\a.exe".into());
        store.prefer.insert(r"D:\Tools\a.exe".into());
        store.hidden.push((r"D:\Tools\b\".into(), "b".into(), String::new()));
        store.save_to(&dir);

        let back = load_from(&dir);
        assert_eq!(back.hotkey, "Ctrl+Alt+K");
        assert_eq!(back.dirs, [r"D:\Tools"]);
        let back = Store::load_from(&dir);
        assert_eq!(back.usage[r"D:\Tools\a.exe"], 3);
        assert_eq!(back.picks["a"], r"D:\Tools\a.exe");
        assert!(back.prefer.contains(r"D:\Tools\a.exe"));
        assert_eq!(back.hidden_dirs(), [r"d:\tools\b"]);
        assert_eq!(names(&dir), ["config.toml", "hidden.tsv", "picks.tsv", "prefer.tsv", "usage.tsv"]);
    }
}
