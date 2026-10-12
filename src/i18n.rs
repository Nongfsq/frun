//! 界面文字。所有字符串来自 `locales/` 下的语言表，界面代码里不写字面量。
use serde::Deserialize;

macro_rules! strings {
    ($($key:ident),* $(,)?) => {
        /// 一种语言的全部界面文字；语言表缺键或多键都无法通过解析。
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct Strings { $(pub $key: String),* }
    };
}

strings!(
    placeholder, settings, tab_general, tab_look, tab_dirs, language, lang_zh, lang_en, hotkey,
    recording, hotkey_taken, ime, autostart, theme, theme_system, theme_light, theme_dark, corners, corner_square, corner_small,
    corner_round, size,
    max_rows, rows_unit, position, reset, centered, screen, screen_pointer, screen_primary, reduce,
    file_types, add_ext, path_hint, drop_here, indexed, reindexing, no_match, finding, first_index,
    updating, updated, up_to_date, src_start, src_store, tag_new, tag_current, admin_enter,
    err_missing, err_failed, tray_settings, tray_quit, add_dir, hidden, hidden_flash,
    tray_reindex, admin, menu_open, menu_folder, menu_hide,
);

const ZH: &str = include_str!("../locales/zh-CN.toml");
const EN: &str = include_str!("../locales/en.toml");

/// `lang` 为 "zh" 或 "en"。
pub fn load(lang: &str) -> Strings {
    let text = if lang == "en" { EN } else { ZH };
    toml::from_str(text).expect("语言表与 Strings 的键不一致")
}

#[cfg(test)]
mod tests {
    #[test]
    fn locales_have_identical_keys() {
        // 两张表都必须恰好包含 Strings 的全部键
        toml::from_str::<super::Strings>(super::ZH).unwrap();
        toml::from_str::<super::Strings>(super::EN).unwrap();
    }
}
