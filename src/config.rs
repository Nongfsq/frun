use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
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
    pub dirs: Vec<String>,
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
            dirs: Vec::new(),
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

fn path() -> PathBuf {
    data_dir().join("config.toml")
}

pub fn load() -> Config {
    let text = fs::read_to_string(path()).unwrap_or_default();
    toml::from_str::<Config>(&text).unwrap_or_default().sanitize()
}

pub fn save(cfg: &Config) {
    let _ = fs::create_dir_all(data_dir());
    if let Ok(text) = toml::to_string(cfg) {
        let _ = fs::write(path(), text);
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
    /// 用户选择不再显示的程序：(启动目标, 名称, 来源)
    pub hidden: Vec<(String, String, String)>,
}

impl Store {
    pub fn load() -> Self {
        let read = |name: &str| fs::read_to_string(data_dir().join(name)).unwrap_or_default();
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

    pub fn is_hidden(&self, target: &str) -> bool {
        self.hidden.iter().any(|(t, _, _)| t == target)
    }

    pub fn save(&self) {
        let dir = data_dir();
        let _ = fs::create_dir_all(&dir);
        let usage: String = self.usage.iter().map(|(t, n)| format!("{n}\t{t}\n")).collect();
        let picks: String = self.picks.iter().map(|(q, t)| format!("{q}\t{t}\n")).collect();
        let prefer: String = self.prefer.iter().map(|t| format!("{t}\n")).collect();
        let _ = fs::write(dir.join("usage.tsv"), usage);
        let _ = fs::write(dir.join("picks.tsv"), picks);
        let _ = fs::write(dir.join("prefer.tsv"), prefer);
        let hidden: String = self.hidden.iter().map(|(t, n, s)| format!("{t}\t{n}\t{s}\n")).collect();
        let _ = fs::write(dir.join("hidden.tsv"), hidden);
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
}
