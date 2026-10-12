//! 主窗口：状态、输入处理、索引更新。绘制在 view.rs。
use crate::{
    config::{self, Config, Store},
    gfx::{Color, Gfx},
    i18n::{self, Strings},
    index::{self, Index, Kind},
    platform::*,
    scan::{WM_ICONS, WM_INDEX, WM_PROGRESS},
    search,
    textfield::TextField,
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    mem::size_of,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::{Dwm::*, Gdi::*},
        System::{LibraryLoader::*, Ole::OleInitialize, Threading::*},
        UI::{Controls::MARGINS, HiDpi::*, Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
    },
    core::*,
};

const WM_TRAY: u32 = WM_APP + 1;
const WM_SUMMON: u32 = WM_APP + 8;
const HOTKEY_ID: i32 = 1;
const TIMER_FRAME: usize = 1;
const TIMER_DIRTY: usize = 2;
const TIMER_CARET: usize = 3;
const TIMER_TRIM: usize = 4;

// 设计稿里的尺寸，逻辑像素
const CORNERS: [&str; 3] = ["square", "small", "round"];
pub const BAR: f32 = 56.0;
pub const ROW: f32 = 48.0;
pub const PAD: f32 = 6.0;

/// 按住修饰键多久后才显示数字，避免每次用 Alt+Space 呼出都闪一下。
const DIGITS_DELAY: Duration = Duration::from_millis(220);
const PRESS: Duration = Duration::from_millis(110);
const FLASH: Duration = Duration::from_millis(1800);
/// 呼出时如果距上次扫描超过这么久，后台核对一次（商店应用的安装没有文件通知）。
const REVALIDATE: Duration = Duration::from_secs(30);

pub struct Theme {
    pub dark: bool,
    pub tint: Color,
    pub t1: Color,
    pub t2: Color,
    pub t3: Color,
    pub sel: Color,
    pub well: Color,
    pub well2: Color,
    pub raise: Color,
    pub accent: Color,
    pub on_accent: Color,
    pub kbd: Color,
    pub danger: Color,
}

impl Theme {
    fn new(dark: bool) -> Self {
        let accent = system_accent(dark);
        if dark {
            Self {
                dark,
                tint: Color::rgb(0x202020).a(0.50),
                t1: Color::white(0.96),
                t2: Color::white(0.66),
                t3: Color::white(0.50),
                sel: Color::white(0.09),
                well: Color::white(0.08),
                well2: Color::white(0.15),
                raise: Color::white(0.17),
                accent: accent.unwrap_or(Color::rgb(0x4CC2FF)),
                on_accent: Color::rgb(0x06202E),
                kbd: Color::white(0.10),
                danger: Color::rgb(0xFF99A4),
            }
        } else {
            Self {
                dark,
                tint: Color::rgb(0xF6F6F6).a(0.55),
                t1: Color::black(0.90),
                t2: Color::black(0.64),
                t3: Color::black(0.55),
                sel: Color::black(0.06),
                well: Color::black(0.055),
                well2: Color::black(0.13),
                raise: Color::white(1.0),
                accent: accent.unwrap_or(Color::rgb(0x005FB8)),
                on_accent: Color::white(1.0),
                kbd: Color::black(0.07),
                danger: Color::rgb(0xC42B1C),
            }
        }
    }
}

/// 动画值。每个值有一个 id，目标变了就从当前位置缓动过去。
#[derive(Default)]
pub struct Anims {
    map: HashMap<u32, (f32, f32, Instant, f32)>,
    /// 本帧是否还有没走完的动画
    pub active: bool,
    /// 减弱动效：所有值直接到位
    pub off: bool,
}

/// cubic-bezier(.2, .8, .2, 1)：起步快、收尾稳。
fn ease(p: f32) -> f32 {
    let (x1, y1, x2, y2) = (0.2f32, 0.8f32, 0.2f32, 1.0f32);
    let bez = |t: f32, a: f32, b: f32| 3.0 * (1.0 - t) * (1.0 - t) * t * a + 3.0 * (1.0 - t) * t * t * b + t * t * t;
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..20 {
        let mid = (lo + hi) / 2.0;
        if bez(mid, x1, x2) < p { lo = mid } else { hi = mid }
    }
    bez((lo + hi) / 2.0, y1, y2)
}

impl Anims {
    fn value(e: &(f32, f32, Instant, f32), now: Instant) -> (f32, bool) {
        let p = if e.3 <= 0.0 { 1.0 } else { (now - e.2).as_secs_f32() * 1000.0 / e.3 };
        if p >= 1.0 { (e.1, true) } else { (e.0 + (e.1 - e.0) * ease(p), false) }
    }

    /// 取值并把目标设为 `target`；目标变了就从当前位置用 `ms` 毫秒缓动过去。
    pub fn get(&mut self, id: u32, target: f32, ms: f32) -> f32 {
        let now = Instant::now();
        let e = self.map.entry(id).or_insert((target, target, now, 0.0));
        if self.off {
            *e = (target, target, now, 0.0);
            return target;
        }
        if e.1 != target {
            *e = (Self::value(e, now).0, target, now, ms);
        }
        let (v, done) = Self::value(e, now);
        self.active |= !done;
        v
    }

    /// 只取当前值，不改目标。
    pub fn peek(&self, id: u32) -> f32 {
        self.map.get(&id).map_or(0.0, |e| Self::value(e, Instant::now()).0)
    }

    /// 不经动画直接设值。
    pub fn set(&mut self, id: u32, v: f32) {
        self.map.insert(id, (v, v, Instant::now(), 0.0));
    }
}

pub const ID_H: u32 = 1;
pub const ID_OPEN: u32 = 2;
pub const ID_MODE: u32 = 3;
pub const ID_FADE: u32 = 4;
pub const ID_SEL: u32 = 5;
pub const ID_SET_SEL: u32 = 6;
pub const ID_RING: u32 = 7;
pub const ID_TABS: u32 = 8;
pub const ID_FLASH: u32 = 9;
pub const ID_PRESS: u32 = 10;
pub const ID_DIGITS: u32 = 11;
pub const ID_ROW: u32 = 100;
pub const ID_ARRIVE: u32 = 1000;

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Search,
    Settings,
}

/// 设置里的一行。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SRow {
    Lang,
    Hotkey,
    Ime,
    Autostart,
    Admin,
    Theme,
    Corners,
    Size,
    MaxRows,
    Position,
    Screen,
    Reduce,
    Folder(usize),
    Add,
    Ext,
    Status,
    Hidden,
    HiddenItem(usize),
}

/// 可以点的东西。
#[derive(Clone, PartialEq)]
pub enum Act {
    None,
    Bar,
    QueryField,
    Row(usize),
    OpenSettings,
    Back,
    Tab(usize),
    Seg(SRow, usize),
    Toggle(SRow),
    Slider,
    Step(i32),
    Button(SRow),
    Remove(usize),
    Plus,
    PathField,
    ExtField,
    Token(String, bool),
    Refresh,
    HiddenToggle,
    Restore(usize),
    AddDirHint,
}

pub struct Hot {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub act: Act,
    /// 鼠标经过时选中第几行
    pub row: Option<usize>,
}

/// 结果列表里的一行：一个分组，或展开后的某个来源。
pub struct Row {
    pub group: usize,
    pub member: usize,
    pub variant: bool,
    pub current: bool,
    pub stack: usize,
    pub marks: Vec<bool>,
}

#[derive(Clone, Copy, PartialEq)]
enum DragKind {
    Move,
    Left,
    Right,
    Corner,
    Slider,
}

struct Drag {
    kind: Option<DragKind>,
    /// 按下时指着什么；没拖动就当作点击
    act: Act,
    start: POINT,
    win: RECT,
    width: u32,
    scale: u32,
}

#[derive(Clone, Copy, PartialEq)]
enum Scan {
    Silent,
    /// 搜不到时自动补查
    Auto,
    /// 用户按了 Ctrl+R 或刷新按钮
    Manual,
}

pub struct App {
    pub hwnd: HWND,
    pub gfx: Option<Gfx>,
    pub cfg: Config,
    pub s: Strings,
    pub store: Store,
    pub index: Index,
    pub theme: Theme,
    pub anims: Anims,
    pub hots: Vec<Hot>,
    pub hover: Act,

    visible: bool,
    pub work: RECT,
    pub dpi: f32,

    pub mode: Mode,
    pub query: TextField,
    pub rows: Vec<Row>,
    pub sel: usize,
    expanded: Option<usize>,
    pub error: Option<(String, String)>,
    pub fresh: HashSet<String>,
    pub menu: crate::menu::Menu,
    /// 首次建索引时扫描进程报来的条目数
    pub progress: usize,
    /// 刚刚隐藏的程序，Ctrl+Z 可以撤销
    pub last_hidden: Vec<(String, String, String)>,
    /// 设置里的隐藏列表是否展开
    pub hidden_open: bool,
    /// 本帧发现缺图标的启动目标；画完后交给子进程去取
    pub icon_wanted: Vec<String>,
    icon_requested: HashSet<String>,
    pub flash: String,
    flash_until: Option<Instant>,
    pub pressed: Option<Instant>,
    mod_since: Option<Instant>,
    pub caret_on: bool,

    pub tab: usize,
    pub set_sel: usize,
    pub path: TextField,
    pub ext: TextField,
    pub recording: bool,
    pub hotkey_note: Option<Instant>,
    pub scale_draft: Option<u32>,
    pub dropping: bool,
    /// 添加行的路径补全：(算的是哪段输入, (补全后的整条路径, 光标后淡色显示的部分))
    ghost: (String, Option<(String, String)>),
    /// 文件夹选择框开着的时候不因为失去焦点而收起
    pinned: bool,

    pub scanning: bool,
    scan_kind: Scan,
    scan_pending: bool,
    /// 目录监视报来的变化：哪些子目录要重查；`dirty_all` 表示说不清，整个重查
    dirty: Vec<String>,
    dirty_all: bool,
    auto_rescanned: bool,
    last_scan: Instant,
    pub pulse: Instant,

    drag: Option<Drag>,
    watchers: Watchers,
    tray_icon: HICON,
    taskbar_created: u32,
    timer_rate: u32,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

pub fn run() -> Result<()> {
    unsafe {
        let _mutex = CreateMutexW(None, true, w!("frun.single"))?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            // 已经在运行：让它显示出来
            if let Ok(other) = FindWindowW(w!("frun"), None) {
                let _ = PostMessageW(Some(other), WM_SUMMON, WPARAM(0), LPARAM(0));
            }
            return Ok(());
        }
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        OleInitialize(None)?;

        let inst: HINSTANCE = GetModuleHandleW(None)?.into();
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: inst,
            lpszClassName: w!("frun"),
            ..Default::default()
        });
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            w!("frun"),
            w!("frun"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(inst),
            None,
        )?;
        // 亚克力底、圆角和阴影都交给系统合成器
        let set = |attr: DWMWINDOWATTRIBUTE, v: i32| {
            let _ = DwmSetWindowAttribute(hwnd, attr, &v as *const _ as _, 4);
        };
        set(DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_TRANSIENTWINDOW.0);
        let _ = DwmExtendFrameIntoClientArea(
            hwnd,
            &MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 },
        );

        let mut cfg = config::load();
        if cfg.language.is_empty() {
            cfg.language = if system_is_chinese() { "zh" } else { "en" }.into();
            config::save(&cfg);
        }
        let store = Store::load();
        let theme = Theme::new(true);
        let mut app = App {
            hwnd,
            gfx: Some(Gfx::new()?),
            s: i18n::load(&cfg.language),
            index: index::load(&store),
            store,
            tray_icon: tray_icon(),
            theme,
            anims: Anims::default(),
            hots: Vec::new(),
            hover: Act::None,
            visible: false,
            work: RECT::default(),
            dpi: 1.0,
            mode: Mode::Search,
            query: TextField::default(),
            rows: Vec::new(),
            sel: 0,
            expanded: None,
            error: None,
            fresh: HashSet::new(),
            menu: Default::default(),
            progress: 0,
            last_hidden: Vec::new(),
            hidden_open: false,
            icon_wanted: Vec::new(),
            icon_requested: HashSet::new(),
            flash: String::new(),
            flash_until: None,
            pressed: None,
            mod_since: None,
            caret_on: true,
            tab: 0,
            set_sel: 0,
            path: TextField::default(),
            ext: TextField::default(),
            recording: false,
            hotkey_note: None,
            scale_draft: None,
            dropping: false,
            ghost: Default::default(),
            pinned: false,
            scanning: false,
            scan_kind: Scan::Silent,
            scan_pending: false,
            dirty: Vec::new(),
            dirty_all: false,
            auto_rescanned: false,
            last_scan: Instant::now(),
            pulse: Instant::now(),
            drag: None,
            watchers: Watchers::default(),
            taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
            timer_rate: 0,
            cfg,
        };
        app.refresh_theme();
        app.tray(NIM_ADD);
        app.register_hotkey();
        app.watchers.restart(&app.cfg.dirs, &app.cfg.exts, hwnd);
        register_drop(hwnd);
        (app.work, app.dpi) = monitor(true);
        app.scan(Scan::Silent, false);
        APP.with(|a| *a.borrow_mut() = Some(app));

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        Ok(())
    }
}

/// 借用应用状态执行 `f`。重入时借不到，返回 None。
pub fn with_app(mut f: impl FnMut(&mut App) -> Option<LRESULT>) -> Option<LRESULT> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut a| a.as_mut().and_then(&mut f)))
}

/// 窗口过程会重入（SetWindowPos、ShowWindow 等都会同步派发消息）。
/// 状态放在 RefCell 里；重入时借不到，就交给默认处理。
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match with_app(|app| app.on_message(msg, wp, lp)) {
        Some(r) => r,
        None => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

impl App {
    // ---------- 尺寸与位置 ----------

    /// 逻辑像素到物理像素的比例。
    pub fn k(&self) -> f32 {
        self.dpi * self.cfg.scale as f32 / 100.0
    }

    /// 面板宽度（逻辑像素）。实际宽度不超过屏幕的 90%。
    pub fn width(&self) -> f32 {
        let limit = ((self.work.right - self.work.left) as f32 * 0.9 / self.k() / 4.0).floor() * 4.0;
        (self.cfg.width as f32).min(limit.max(320.0))
    }

    pub fn icon_px(&self) -> u32 {
        (28.0 * self.k()).round() as u32
    }

    fn target_h(&self) -> f32 {
        let rows = match self.mode {
            Mode::Settings => self.srows().len(),
            Mode::Search if self.rows.is_empty() => self.note().is_some() as usize,
            Mode::Search => self.rows.len(),
        };
        if rows == 0 { BAR } else { BAR + rows as f32 * ROW + PAD }
    }

    /// 按配置里的比例把窗口放到屏幕上。高度由动画值决定。
    fn place(&mut self, show: bool) {
        let k = self.k();
        let h = self.anims.get(ID_H, self.target_h(), 140.0);
        let (w, h) = ((self.width() * k).round() as i32, (h * k).round() as i32);
        let (sw, sh) = ((self.work.right - self.work.left) as f32, (self.work.bottom - self.work.top) as f32);
        let x = self.work.left + (self.cfg.cx * sw).round() as i32 - w / 2;
        let y = self.work.top + (self.cfg.ty * sh).round() as i32;
        let x = x.clamp(self.work.left, (self.work.right - w).max(self.work.left));
        let y = y.clamp(self.work.top, (self.work.bottom - (BAR * k) as i32).max(self.work.top));
        let flags = SWP_NOACTIVATE | if show { SWP_SHOWWINDOW } else { SWP_NOZORDER };
        unsafe {
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), x, y, w, h, flags);
        }
    }

    // ---------- 显示与收起 ----------

    pub fn show(&mut self) {
        let began = Instant::now();
        (self.work, self.dpi) = monitor(self.cfg.screen == "primary");
        self.refresh_theme();
        self.mode = Mode::Search;
        self.query.clear();
        self.expanded = None;
        self.error = None;
        self.auto_rescanned = false;
        self.pressed = None;
        self.recording = false;
        self.caret_on = true;
        self.hover = Act::None;
        self.mod_since = (key_down(VK_MENU.0 as u32) || key_down(VK_CONTROL.0 as u32)).then(Instant::now);
        self.requery();
        self.anims.off = self.cfg.reduce_motion || system_reduced_motion();
        let h = self.target_h();
        self.anims.set(ID_H, h);
        self.anims.set(ID_MODE, 0.0);
        self.anims.set(ID_FADE, 1.0);
        self.anims.set(ID_PRESS, 0.0);
        self.anims.set(ID_OPEN, 0.0);
        self.visible = true;
        self.place(true);
        unsafe {
            let _ = SetForegroundWindow(self.hwnd);
            let _ = SetFocus(Some(self.hwnd));
            SetTimer(Some(self.hwnd), TIMER_CARET, 530, None);
        }
        if self.cfg.ime_english {
            ime_english(self.hwnd);
        }
        // 目录有监视盯着；商店应用的增删没有通知，呼出时顺手核对一次系统应用列表
        if self.last_scan.elapsed() > REVALIDATE {
            self.scan_only(&["apps".to_string()]);
        }
        self.frame();
        // 设了 FRUN_TRACE 时记录呼出到第一帧画完用了多久
        if std::env::var_os("FRUN_TRACE").is_some() {
            use std::io::Write;
            let path = config::cache_dir().join("trace.log");
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(f, "show {:.1} ms", began.elapsed().as_secs_f32() * 1000.0);
            }
        }
    }

    pub fn hide(&mut self) {
        if !self.visible {
            return;
        }
        self.visible = false;
        self.drop_missing();
        self.cancel_recording();
        self.drag = None;
        self.dropping = false;
        self.fresh.clear();
        self.last_hidden.clear();
        unsafe {
            let _ = ReleaseCapture();
            let _ = KillTimer(Some(self.hwnd), TIMER_CARET);
            let _ = KillTimer(Some(self.hwnd), TIMER_FRAME);
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.timer_rate = 0;
        if let Some(g) = &mut self.gfx {
            g.release();
        }
        // 收起一会儿后把用不到的内存页还给系统；马上又呼出的话就不折腾
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_TRIM, 3000, None);
        }
    }

    pub fn invalidate(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    /// 画一帧；还有动画在走就继续定时。
    fn frame(&mut self) {
        if !self.visible {
            unsafe {
                let _ = ValidateRect(Some(self.hwnd), None);
            }
            return;
        }
        // 按压动画走完再收起，让人看清选中了哪一个
        if self.pressed.is_some_and(|t| t.elapsed() >= PRESS || self.anims.off) {
            self.pressed = None;
            self.hide();
            return;
        }
        if self.flash_until.is_some_and(|t| Instant::now() >= t) {
            self.flash_until = None;
            self.flash.clear();
            self.last_hidden.clear();
        }
        if self.hotkey_note.is_some_and(|t| t.elapsed() > FLASH) {
            self.hotkey_note = None;
        }
        self.anims.active = false;
        self.place(false);
        let Some(mut g) = self.gfx.take() else { return };
        g.k = self.k();
        if g.begin(self.hwnd).is_ok() {
            self.draw(&mut g);
            g.end();
        }
        self.gfx = Some(g);
        unsafe {
            let _ = ValidateRect(Some(self.hwnd), None);
        }
        let wanted: Vec<String> =
            self.icon_wanted.drain(..).filter(|t| self.icon_requested.insert(t.clone())).collect();
        if !wanted.is_empty() {
            spawn_icons(self.hwnd, self.icon_px(), &wanted);
        }
        let waiting = self.pressed.is_some()
            || self.flash_until.is_some()
            || self.hotkey_note.is_some()
            || self.mod_since.is_some_and(|t| t.elapsed() < DIGITS_DELAY);
        // 目录页的呼吸圆点和转圈图标是慢动作，没有别的动画时用低帧率画，省 CPU
        let ambient = self.mode == Mode::Settings && self.tab == 2 && !self.anims.off;
        let rate = if self.anims.active || waiting {
            10
        } else if ambient {
            if self.scanning { 33 } else { 80 }
        } else {
            0
        };
        if rate != self.timer_rate {
            self.timer_rate = rate;
            unsafe {
                if rate > 0 {
                    SetTimer(Some(self.hwnd), TIMER_FRAME, rate, None);
                } else {
                    let _ = KillTimer(Some(self.hwnd), TIMER_FRAME);
                }
            }
        }
    }

    /// 面板里选中条等元素的圆角，跟着窗口圆角走。
    pub fn radius(&self) -> f32 {
        match self.cfg.corners.as_str() {
            "square" => 2.0,
            "small" => 5.0,
            _ => 8.0,
        }
    }

    pub fn digits_on(&self) -> bool {
        self.mod_since.is_some_and(|t| t.elapsed() >= DIGITS_DELAY)
    }

    pub fn admin_on(&self) -> bool {
        key_down(VK_CONTROL.0 as u32) && key_down(VK_SHIFT.0 as u32)
    }

    pub fn theme_is_dark(&self) -> bool {
        self.theme.dark
    }

    /// 圆角由系统合成器裁切，它只提供这三档。
    pub fn corner_preference(&self) -> i32 {
        match self.cfg.corners.as_str() {
            "square" => DWMWCP_DONOTROUND.0,
            "small" => DWMWCP_ROUNDSMALL.0,
            _ => DWMWCP_ROUND.0,
        }
    }

    pub fn show_settings(&mut self) {
        self.show();
        self.open_settings();
        self.invalidate();
    }

    pub fn rebuild_index(&mut self) {
        self.scan(Scan::Manual, true);
    }

    pub fn refresh_theme(&mut self) {
        let dark = match self.cfg.theme.as_str() {
            "light" => false,
            "dark" => true,
            _ => !system_light_theme(),
        };
        self.theme = Theme::new(dark);
        let v = dark as i32;
        let corner = self.corner_preference();
        unsafe {
            let _ = DwmSetWindowAttribute(self.hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &corner as *const _ as _, 4);
            let _ = DwmSetWindowAttribute(self.hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &v as *const _ as _, 4);
        }
    }

    // ---------- 搜索 ----------

    /// 没有结果时显示的那句话。
    pub fn note(&self) -> Option<&str> {
        if !self.rows.is_empty() {
            None
        } else if self.index.groups.is_empty() && self.scanning {
            Some(&self.s.first_index)
        } else if self.query.string().trim().is_empty() {
            None
        } else if self.scanning {
            Some(&self.s.finding)
        } else {
            Some(&self.s.no_match)
        }
    }

    fn requery(&mut self) {
        let max = self.cfg.max_rows as usize;
        let q = self.query.string();
        let mut rows = Vec::new();
        let typed: Vec<char> = q.trim().to_lowercase().chars().collect();
        for hit in search::run(&self.index, &self.store, &q, max) {
            let g = &self.index.groups[hit.group];
            if self.expanded == Some(hit.group) {
                // 展开：每个来源各占一行，当前显示的那个排第一
                let several = g.members.len() > 1;
                let mut order: Vec<usize> = (0..g.members.len()).collect();
                order.sort_by_key(|&i| i != g.cur);
                for i in order {
                    let marks = if i == g.cur { hit.marks.clone() } else { Vec::new() };
                    let current = several && i == g.cur;
                    rows.push(Row { group: hit.group, member: i, variant: several, current, stack: 0, marks });
                }
                // 然后是同一个程序目录里收在它后面的：对得上输入的在前
                let mut behind: Vec<(i32, usize, Vec<bool>)> = self
                    .index
                    .folded(hit.group, &self.store)
                    .map(|h| match search::matched(&self.index.groups[h], &typed) {
                        Some((score, marks)) => (score, h, marks),
                        None => (0, h, Vec::new()),
                    })
                    .collect();
                let name = |h: usize| self.index.groups[h].entry().name.to_lowercase();
                behind.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| name(a.1).cmp(&name(b.1))));
                for (_, h, marks) in behind.into_iter().take(max) {
                    let member = self.index.groups[h].cur;
                    rows.push(Row { group: h, member, variant: true, current: false, stack: 0, marks });
                }
            } else {
                rows.push(Row {
                    group: hit.group,
                    member: g.cur,
                    variant: false,
                    current: false,
                    stack: g.members.len() + hit.extra,
                    marks: hit.marks,
                });
            }
        }
        rows.truncate(max);
        self.rows = rows;
        self.sel = self.sel.min(self.rows.len().saturating_sub(1));
        // 搜不到的时候正是索引可能过期的时候：自动补查一次
        if self.rows.is_empty() && !q.trim().is_empty() && !self.auto_rescanned && self.visible {
            self.auto_rescanned = true;
            self.scan(Scan::Auto, false);
        }
    }

    /// 启动失败是因为文件没了：把它移出索引（下次扫描也不会再收录）。
    fn drop_missing(&mut self) {
        if let Some((target, text)) = self.error.take()
            && text == self.s.err_missing
        {
            self.index.remove(&target);
            self.index.choose_all(&self.store);
        }
    }

    fn typed(&mut self) {
        self.sel = 0;
        self.expanded = None;
        self.drop_missing();
        self.anims.set(ID_SEL, 0.0);
        self.requery();
    }

    fn move_sel(&mut self, d: i32, repeat: bool) {
        let n = self.rows.len() as i32;
        if n > 0 {
            self.sel = (self.sel as i32 + d).rem_euclid(n) as usize;
            // 按住方向键连发时不做滑动，光标要跟得上手
            if repeat {
                self.anims.set(ID_SEL, self.sel as f32 * ROW);
            }
        }
    }

    pub fn launch(&mut self, i: usize, admin: bool) {
        let Some(row) = self.rows.get(i) else { return };
        if self.pressed.is_some() {
            return;
        }
        let entry = self.index.groups[row.group].members[row.member].clone();
        let (variant, group) = (row.variant, row.group);
        self.sel = i;
        let file = HSTRING::from(&entry.target);
        let dir = HSTRING::from(&entry.workdir);
        // 设置里开了"以管理员身份打开"：能提权的都提权；这个类型没有提权动作时退回普通打开
        let default_admin = !admin && self.cfg.admin && entry.kind != Kind::Store;
        let run = |admin: bool| {
            let mut info = SHELLEXECUTEINFOW {
                cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
                fMask: SEE_MASK_FLAG_NO_UI,
                lpVerb: if admin { w!("runas") } else { w!("open") },
                lpFile: PCWSTR(file.as_ptr()),
                lpDirectory: if entry.workdir.is_empty() { PCWSTR::null() } else { PCWSTR(dir.as_ptr()) },
                nShow: SW_SHOWNORMAL.0,
                ..Default::default()
            };
            unsafe { ShellExecuteExW(&mut info) }
        };
        let cancelled = |e: &Error| e.code() == ERROR_CANCELLED.to_hresult();
        let result = match run(admin || default_admin) {
            Err(e) if default_admin && !cancelled(&e) => run(false),
            other => other,
        };
        if let Err(e) = result {
            let code = e.code();
            if cancelled(&e) {
                // 用户在提权确认框里点了否：什么都不说，面板留着
                return;
            }
            let missing = code == ERROR_FILE_NOT_FOUND.to_hresult() || code == ERROR_PATH_NOT_FOUND.to_hresult();
            let text = if missing { &self.s.err_missing } else { &self.s.err_failed };
            self.error = Some((entry.target.clone(), text.clone()));
            return;
        }
        *self.store.usage.entry(entry.target.clone()).or_default() += 1;
        let q = self.query.string().trim().to_lowercase();
        if !q.is_empty() {
            self.store.picks.insert(q, entry.target.clone());
        }
        if variant {
            // 从展开的来源里打开哪一个，以后就显示哪一个
            for m in &self.index.groups[group].members {
                self.store.prefer.remove(&m.target);
            }
            self.store.prefer.insert(entry.target.clone());
        }
        self.store.save();
        self.index.choose_all(&self.store);
        self.pressed = Some(Instant::now());
    }

    /// 在资源管理器里打开程序所在的文件夹并选中它。
    pub fn reveal(&mut self, i: usize) {
        let Some(row) = self.rows.get(i) else { return };
        let Some(path) = location(&self.index.groups[row.group].members[row.member]) else { return };
        let args = HSTRING::from(format!("/select,\"{path}\""));
        let mut info = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_FLAG_NO_UI,
            lpFile: w!("explorer.exe"),
            lpParameters: PCWSTR(args.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        if unsafe { ShellExecuteExW(&mut info) }.is_ok() {
            self.hide();
        }
    }

    // ---------- 索引 ----------

    fn scan(&mut self, kind: Scan, force: bool) {
        if kind == Scan::Manual {
            self.flash = self.s.updating.clone();
            self.flash_until = None;
        }
        if self.scanning {
            self.scan_pending = true;
            if kind != Scan::Silent {
                self.scan_kind = kind;
            }
            return;
        }
        self.scan_kind = kind;
        self.scanning = spawn_scan(self.hwnd, self.icon_px(), force, &[]);
        if force && let Some(g) = &mut self.gfx {
            g.forget_icons();
            self.icon_requested.clear();
        }
    }

    /// 只重查指定的几处（变了的子目录，或 "apps" 表示系统应用列表），其余沿用现有索引。
    fn scan_only(&mut self, only: &[String]) {
        if self.scanning {
            self.scan_pending = true;
        } else {
            self.scan_kind = Scan::Silent;
            self.scanning = spawn_scan(self.hwnd, self.icon_px(), false, only);
        }
    }

    fn index_ready(&mut self) {
        let old = self.index.targets();
        self.index = index::load(&self.store);
        let added: HashSet<String> = self.index.targets().difference(&old).cloned().collect();
        if self.scan_kind != Scan::Silent && !old.is_empty() {
            self.fresh = added.clone();
        }
        if self.scan_kind == Scan::Manual {
            self.flash = if added.is_empty() || old.is_empty() {
                self.s.up_to_date.clone()
            } else {
                self.s.updated.replace("{n}", &added.len().to_string())
            };
            self.flash_until = Some(Instant::now() + FLASH);
        }
        self.expanded = None;
        self.requery();
    }

    // ---------- 设置 ----------

    pub fn srows(&self) -> Vec<SRow> {
        match self.tab {
            0 => vec![SRow::Lang, SRow::Hotkey, SRow::Ime, SRow::Autostart, SRow::Admin],
            1 => vec![SRow::Theme, SRow::Corners, SRow::Size, SRow::MaxRows, SRow::Position, SRow::Screen, SRow::Reduce],
            _ => {
                let mut rows: Vec<SRow> =
                    (0..self.cfg.dirs.len()).map(SRow::Folder).chain([SRow::Add, SRow::Ext, SRow::Status]).collect();
                // 有隐藏项时才多出这一行；展开后每个隐藏项各占一行
                if !self.store.hidden.is_empty() {
                    rows.push(SRow::Hidden);
                    if self.hidden_open {
                        rows.extend((0..self.store.hidden.len()).map(SRow::HiddenItem));
                    }
                }
                rows
            }
        }
    }

    pub fn srow(&self) -> Option<SRow> {
        self.srows().get(self.set_sel).copied()
    }

    /// 换页、换语言、进出设置时让内容淡入一次。
    fn bump(&mut self) {
        self.anims.set(ID_FADE, 0.0);
    }

    fn open_settings(&mut self) {
        if self.mode != Mode::Settings {
            self.mode = Mode::Settings;
            self.set_sel = 0;
            self.bump();
        }
    }

    fn close_settings(&mut self) {
        self.cancel_recording();
        self.scale_draft = None;
        self.mode = Mode::Search;
        self.requery();
        self.bump();
    }

    fn set_tab(&mut self, tab: usize) {
        if self.tab != tab {
            self.cancel_recording();
            self.tab = tab;
            self.set_sel = 0;
            self.anims.set(ID_SET_SEL, 0.0);
            self.bump();
        }
    }

    fn changed(&mut self) {
        config::save(&self.cfg);
    }

    /// 分段选择当前选中第几项。
    pub fn seg_value(&self, row: SRow) -> usize {
        match row {
            SRow::Lang => (self.cfg.language == "en") as usize,
            SRow::Theme => ["system", "light", "dark"].iter().position(|t| *t == self.cfg.theme).unwrap_or(0),
            SRow::Corners => CORNERS.iter().position(|t| *t == self.cfg.corners).unwrap_or(2),
            SRow::Screen => (self.cfg.screen == "primary") as usize,
            _ => 0,
        }
    }

    fn set_seg(&mut self, row: SRow, i: usize) {
        match row {
            SRow::Lang => {
                let lang = if i == 1 { "en" } else { "zh" };
                if self.cfg.language != lang {
                    self.cfg.language = lang.into();
                    self.s = i18n::load(lang);
                    self.bump();
                }
            }
            SRow::Corners => {
                self.cfg.corners = CORNERS[i.min(2)].into();
                self.refresh_theme();
            }
            SRow::Theme => {
                self.cfg.theme = ["system", "light", "dark"][i.min(2)].into();
                self.refresh_theme();
            }
            SRow::Screen => self.cfg.screen = if i == 1 { "primary" } else { "pointer" }.into(),
            _ => return,
        }
        self.changed();
    }

    pub fn switch_value(&self, row: SRow) -> bool {
        match row {
            SRow::Ime => self.cfg.ime_english,
            SRow::Autostart => autostart(),
            SRow::Admin => self.cfg.admin,
            SRow::Reduce => self.cfg.reduce_motion,
            _ => false,
        }
    }

    fn set_switch(&mut self, row: SRow, on: bool) {
        match row {
            SRow::Ime => self.cfg.ime_english = on,
            SRow::Autostart => set_autostart(on),
            SRow::Admin => self.cfg.admin = on,
            SRow::Reduce => {
                self.cfg.reduce_motion = on;
                self.anims.off = on || system_reduced_motion();
            }
            _ => return,
        }
        self.changed();
    }

    fn set_scale(&mut self, v: u32) {
        let v = v.clamp(config::SCALE_MIN, config::SCALE_MAX);
        self.scale_draft = None;
        if self.cfg.scale != v {
            self.cfg.scale = v;
            self.changed();
            // 图标按实际像素缓存，大小变了要补一份
            self.scan(Scan::Silent, false);
        }
    }

    fn step_rows(&mut self, d: i32) {
        let v = (self.cfg.max_rows as i32 + d).clamp(config::ROWS_MIN as i32, config::ROWS_MAX as i32) as u32;
        if v != self.cfg.max_rows {
            self.cfg.max_rows = v;
            self.changed();
        }
    }

    pub fn centered(&self) -> bool {
        (self.cfg.cx - config::DEFAULT_CX).abs() < 0.0005 && (self.cfg.ty - config::DEFAULT_TY).abs() < 0.0005
    }

    fn press_button(&mut self, row: SRow) {
        match row {
            SRow::Hotkey => {
                if self.recording {
                    self.cancel_recording();
                } else {
                    // 录制期间先让出当前热键，否则按它只会收起面板
                    self.recording = true;
                    self.hotkey_note = None;
                    unsafe {
                        let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_ID);
                    }
                }
            }
            SRow::Position => {
                self.cfg.cx = config::DEFAULT_CX;
                self.cfg.ty = config::DEFAULT_TY;
                self.changed();
            }
            _ => {}
        }
    }

    fn register_hotkey(&self) -> bool {
        unsafe {
            let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_ID);
            config::parse_hotkey(&self.cfg.hotkey).is_some_and(|(mods, vk)| {
                RegisterHotKey(Some(self.hwnd), HOTKEY_ID, HOT_KEY_MODIFIERS(mods) | MOD_NOREPEAT, vk).is_ok()
            })
        }
    }

    fn cancel_recording(&mut self) {
        if self.recording {
            self.recording = false;
            self.register_hotkey();
        }
    }

    /// 录制热键时的按键。
    fn record_key(&mut self, vk: u32) {
        if vk == VK_ESCAPE.0 as u32 {
            self.cancel_recording();
            return;
        }
        let mods = [
            (VK_CONTROL, config::MOD_CTRL),
            (VK_MENU, config::MOD_ALT),
            (VK_SHIFT, config::MOD_SHIFT),
            (VK_LWIN, config::MOD_WIN),
            (VK_RWIN, config::MOD_WIN),
        ]
        .iter()
        .filter(|(k, _)| key_down(k.0 as u32))
        .fold(0, |m, (_, bit)| m | bit);
        let Some(hotkey) = config::format_hotkey(mods, vk) else { return };
        let old = std::mem::replace(&mut self.cfg.hotkey, hotkey);
        self.recording = false;
        if self.register_hotkey() {
            self.changed();
        } else {
            // 被别的程序占用：恢复原来的，并在按钮旁说明
            self.cfg.hotkey = old;
            self.register_hotkey();
            self.hotkey_note = Some(Instant::now());
        }
    }

    /// 添加搜索目录。路径不存在时不添加，返回 false。
    fn add_dir(&mut self, path: &str) -> bool {
        let path = path.trim().trim_matches('"').trim_end_matches(['\\', '/']);
        // "D:" 这样的盘符根目录要补回斜杠
        let path = if path.len() == 2 && path.ends_with(':') { format!("{path}\\") } else { path.to_string() };
        if path.is_empty() || !std::path::Path::new(&path).is_dir() {
            return false;
        }
        let on_page = self.mode == Mode::Settings && self.tab == 2;
        self.mode = Mode::Settings;
        self.tab = 2;
        self.path.clear();
        self.dropping = false;
        if !on_page {
            self.bump();
        }
        if let Some(at) = self.cfg.dirs.iter().position(|d| d.eq_ignore_ascii_case(&path)) {
            self.set_sel = at;
            return true;
        }
        self.cfg.dirs.push(path);
        self.set_sel = self.cfg.dirs.len() - 1;
        self.anims.set(ID_ARRIVE + self.set_sel as u32, 0.0);
        self.dirs_changed();
        true
    }

    /// 添加行里正在输入的路径能补全成什么。只在光标位于末尾时给出。
    pub fn path_ghost(&mut self) -> Option<(String, String)> {
        if !self.path.at_end() || self.path.selection().is_some() {
            return None;
        }
        let text = self.path.string();
        if self.ghost.0 != text || text.is_empty() {
            self.ghost = (text.clone(), complete_path(&text));
        }
        self.ghost.1.clone()
    }

    /// 打开系统的文件夹选择框。它开着的时候面板不因失去焦点而收起。
    fn pick(&mut self) {
        if !self.pinned {
            self.pinned = spawn_pick(self.hwnd);
        }
    }

    /// 不再显示选中的程序（它的所有来源）。
    pub fn hide_selected(&mut self) {
        let Some(row) = self.rows.get(self.sel) else { return };
        let group = &self.index.groups[row.group];
        let name = group.entry().name.clone();
        let items: Vec<(String, String, String)> = group
            .members
            .iter()
            .map(|m| (m.target.clone(), m.name.clone(), source_label(&self.s, m).to_string()))
            .collect();
        self.store.hidden.extend(items.iter().cloned());
        self.store.save();
        self.last_hidden = items;
        self.index = index::load(&self.store);
        self.expanded = None;
        self.error = None;
        self.requery();
        self.flash = self.s.hidden_flash.replace("{name}", &name);
        self.flash_until = Some(Instant::now() + Duration::from_secs(5));
    }

    fn undo_hide(&mut self) {
        if self.last_hidden.is_empty() {
            return;
        }
        let back = std::mem::take(&mut self.last_hidden);
        self.store.hidden.retain(|h| !back.contains(h));
        self.unhide();
        self.flash.clear();
        self.flash_until = None;
    }

    fn restore(&mut self, i: usize) {
        if i < self.store.hidden.len() {
            self.store.hidden.remove(i);
            self.unhide();
            self.set_sel = self.set_sel.min(self.srows().len() - 1);
        }
    }

    fn unhide(&mut self) {
        self.store.save();
        self.index = index::load(&self.store);
        self.requery();
    }

    /// 搜不到时的下一步：直接到"设置 · 目录"的添加行。
    fn goto_add(&mut self) {
        self.open_settings();
        self.tab = 2;
        self.set_sel = self.srows().iter().position(|r| *r == SRow::Add).unwrap_or(0);
        self.anims.set(ID_SET_SEL, self.set_sel as f32 * ROW);
    }

    /// 没有结果、也不在补查时，回车的含义是"去添加目录"。
    pub fn can_add_dir(&self) -> bool {
        self.rows.is_empty() && !self.scanning && !self.query.string().trim().is_empty()
    }

    fn remove_dir(&mut self, i: usize) {
        if i < self.cfg.dirs.len() {
            self.cfg.dirs.remove(i);
            self.set_sel = self.set_sel.min(self.srows().len() - 1);
            self.dirs_changed();
        }
    }

    fn dirs_changed(&mut self) {
        self.changed();
        self.watchers.restart(&self.cfg.dirs, &self.cfg.exts, self.hwnd);
        self.scan(Scan::Silent, false);
    }

    fn toggle_ext(&mut self, ext: &str) {
        match self.cfg.exts.iter().position(|e| e == ext) {
            Some(at) => {
                self.cfg.exts.remove(at);
            }
            None => self.cfg.exts.push(ext.to_string()),
        }
        self.dirs_changed();
    }

    fn commit_ext(&mut self) {
        let ext = self.ext.string().trim_start_matches('.').to_lowercase();
        self.ext.clear();
        if !ext.is_empty() && !self.cfg.exts.contains(&ext) {
            self.toggle_ext(&ext);
        }
    }

    /// ← → 调整当前行。
    fn adjust(&mut self, d: i32) {
        let Some(row) = self.srow() else { return };
        match row {
            SRow::Lang | SRow::Theme | SRow::Corners | SRow::Screen => {
                let n = if matches!(row, SRow::Theme | SRow::Corners) { 3 } else { 2 };
                let v = (self.seg_value(row) as i32 + d).clamp(0, n - 1) as usize;
                self.set_seg(row, v);
            }
            SRow::Ime | SRow::Autostart | SRow::Admin | SRow::Reduce => self.set_switch(row, d > 0),
            SRow::Size => self.set_scale((self.cfg.scale as i32 + d * 5) as u32),
            SRow::Hidden => self.hidden_open = d > 0,
            SRow::MaxRows => self.step_rows(d),
            _ => {}
        }
    }

    /// Space / Enter 触发当前行。
    fn activate(&mut self) {
        let Some(row) = self.srow() else { return };
        match row {
            SRow::Ime | SRow::Autostart | SRow::Admin | SRow::Reduce => self.set_switch(row, !self.switch_value(row)),
            SRow::Hotkey | SRow::Position => self.press_button(row),
            SRow::Status => self.scan(Scan::Manual, true),
            SRow::Hidden => self.hidden_open = !self.hidden_open,
            SRow::HiddenItem(i) => self.restore(i),
            _ => {}
        }
    }

    // ---------- 键盘 ----------

    fn key(&mut self, vk: u32, repeat: bool) -> bool {
        let ctrl = key_down(VK_CONTROL.0 as u32);
        let alt = key_down(VK_MENU.0 as u32);
        let shift = key_down(VK_SHIFT.0 as u32);
        let is = |k: VIRTUAL_KEY| vk == k.0 as u32;
        self.caret_on = true;
        if self.recording {
            if !is(VK_CONTROL) && !is(VK_MENU) && !is(VK_SHIFT) && !is(VK_LWIN) && !is(VK_RWIN) {
                self.record_key(vk);
            }
            return true;
        }
        if is(VK_CONTROL) || is(VK_MENU) {
            if self.mod_since.is_none() {
                self.mod_since = Some(Instant::now());
            }
            return true;
        }
        if ctrl && is(VK_OEM_COMMA) {
            match self.mode {
                Mode::Search => self.open_settings(),
                Mode::Settings => self.close_settings(),
            }
            return true;
        }
        if ctrl && vk == b'R' as u32 {
            self.scan(Scan::Manual, true);
            return true;
        }
        match self.mode {
            Mode::Search => self.key_search(vk, ctrl, alt, shift, repeat),
            Mode::Settings => self.key_settings(vk, ctrl, shift),
        }
    }

    fn key_search(&mut self, vk: u32, ctrl: bool, alt: bool, shift: bool, repeat: bool) -> bool {
        let is = |k: VIRTUAL_KEY| vk == k.0 as u32;
        if (ctrl || alt) && (b'1' as u32..=b'8' as u32).contains(&vk) {
            self.launch((vk - b'1' as u32) as usize, false);
            return true;
        }
        if ctrl && is(VK_DELETE) {
            self.hide_selected();
            return true;
        }
        if ctrl && vk == b'Z' as u32 && !self.last_hidden.is_empty() {
            self.undo_hide();
            return true;
        }
        if is(VK_RETURN) && self.can_add_dir() {
            self.goto_add();
            return true;
        }
        if is(VK_RETURN) && ctrl && !shift {
            self.reveal(self.sel);
            return true;
        }
        if is(VK_APPS) || (shift && is(VK_F10)) {
            self.open_row_menu(self.sel, None);
            return true;
        }
        let stack = self.rows.get(self.sel).map_or(0, |r| r.stack);
        if is(VK_DOWN) {
            self.move_sel(1, repeat);
        } else if is(VK_UP) {
            self.move_sel(-1, repeat);
        } else if is(VK_TAB) {
            self.move_sel(if shift { -1 } else { 1 }, repeat);
        } else if is(VK_RIGHT) && self.query.at_end() && stack > 1 {
            self.expanded = self.rows.get(self.sel).map(|r| r.group);
            self.requery();
        } else if (is(VK_LEFT) || is(VK_ESCAPE)) && self.expanded.is_some() {
            let group = self.expanded.take();
            self.requery();
            self.sel = self.rows.iter().position(|r| Some(r.group) == group).unwrap_or(0);
        } else if is(VK_RETURN) {
            self.launch(self.sel, ctrl && shift);
        } else if is(VK_ESCAPE) {
            self.hide();
        } else {
            let (handled, changed) = self.query.key(vk, ctrl, shift, self.hwnd);
            if changed {
                self.typed();
            }
            return handled;
        }
        true
    }

    fn key_settings(&mut self, vk: u32, ctrl: bool, shift: bool) -> bool {
        let is = |k: VIRTUAL_KEY| vk == k.0 as u32;
        let row = self.srow();
        let n = self.srows().len();
        if is(VK_ESCAPE) {
            self.close_settings();
        } else if is(VK_DOWN) || is(VK_UP) {
            self.set_sel = (self.set_sel + if is(VK_DOWN) { 1 } else { n - 1 }) % n;
            self.scale_draft = None;
        } else if row == Some(SRow::Add)
            && ((is(VK_TAB) && !shift) || is(VK_RIGHT))
            && let Some((full, _)) = self.path_ghost()
        {
            // 接受补全的这一层，接着可以输下一层
            self.path.set(&full);
        } else if is(VK_TAB) {
            self.set_tab((self.tab + if shift { 2 } else { 1 }) % 3);
        } else if row == Some(SRow::Add) {
            if is(VK_RETURN) {
                let path = self.path.string();
                if !path.trim().is_empty() {
                    self.add_dir(&path);
                } else {
                    self.pick()
                }
            } else {
                return self.path.key(vk, ctrl, shift, self.hwnd).0;
            }
        } else if row == Some(SRow::Ext) {
            if is(VK_RETURN) {
                self.commit_ext();
            } else if is(VK_BACK) && self.ext.is_empty() {
                // 输入框空着时退格：去掉最后一个类型
                if let Some(last) = self.cfg.exts.last().cloned() {
                    self.toggle_ext(&last);
                }
            } else {
                return self.ext.key(vk, ctrl, shift, self.hwnd).0;
            }
        } else if is(VK_LEFT) || is(VK_RIGHT) {
            self.adjust(if is(VK_RIGHT) { 1 } else { -1 });
        } else if is(VK_SPACE) || is(VK_RETURN) {
            self.activate();
        } else if (is(VK_DELETE) || is(VK_BACK))
            && let Some(SRow::Folder(i)) = row
        {
            self.remove_dir(i);
        } else {
            return false;
        }
        true
    }

    fn char(&mut self, c: u16) {
        if c < 32 || c == 127 || self.recording {
            return;
        }
        self.caret_on = true;
        match (self.mode, self.srow()) {
            (Mode::Search, _) => {
                self.query.insert(&[c]);
                self.typed();
            }
            (Mode::Settings, Some(SRow::Add)) => self.path.insert(&[c]),
            (Mode::Settings, Some(SRow::Ext)) => {
                // 空格和逗号结束一个类型；只收字母数字
                if c == b' ' as u16 || c == b',' as u16 {
                    self.commit_ext();
                } else if (c as u8 as char).is_ascii_alphanumeric() && c < 128 && self.ext.text.len() < 10 {
                    self.ext.insert(&[(c as u8).to_ascii_lowercase() as u16]);
                }
            }
            _ => {}
        }
    }

    // ---------- 鼠标 ----------

    fn hit(&self, x: f32, y: f32) -> Option<&Hot> {
        self.hots.iter().rev().find(|h| x >= h.x && x < h.x + h.w && y >= h.y && y < h.y + h.h)
    }

    /// 面板边缘的拖动区：左右边改宽度，右下角改整体大小。
    fn edge(&self, x: f32, y: f32) -> Option<DragKind> {
        let (w, h) = (self.width(), self.target_h());
        if x > w - 14.0 && y > h - 14.0 {
            Some(DragKind::Corner)
        } else if x < 5.0 {
            Some(DragKind::Left)
        } else if x > w - 5.0 {
            Some(DragKind::Right)
        } else {
            None
        }
    }

    fn cursor(&self, x: f32, y: f32) -> PCWSTR {
        let kind = self.drag.as_ref().and_then(|d| d.kind).or_else(|| self.edge(x, y));
        match kind {
            Some(DragKind::Corner) => IDC_SIZENWSE,
            Some(DragKind::Left | DragKind::Right) => IDC_SIZEWE,
            Some(DragKind::Move) => IDC_SIZEALL,
            _ => match self.hit(x, y).map(|h| &h.act) {
                Some(Act::QueryField | Act::PathField | Act::ExtField) => IDC_IBEAM,
                _ => IDC_ARROW,
            },
        }
    }

    fn mouse_down(&mut self, x: f32, y: f32) {
        let act = self.hit(x, y).map_or(Act::None, |h| h.act.clone());
        let kind = match self.edge(x, y) {
            Some(k) => Some(k),
            None if act == Act::Slider => Some(DragKind::Slider),
            None => None,
        };
        let mut start = POINT::default();
        let mut win = RECT::default();
        unsafe {
            let _ = GetCursorPos(&mut start);
            let _ = GetWindowRect(self.hwnd, &mut win);
            SetCapture(self.hwnd);
        }
        self.drag = Some(Drag { kind, act, start, win, width: self.cfg.width, scale: self.cfg.scale });
        if kind == Some(DragKind::Slider) {
            self.slide(x);
        }
    }

    /// 滑块跟手：拖动时只更新读数，松手才真正缩放，避免滑块在指针下移动。
    fn slide(&mut self, x: f32) {
        if let Some(h) = self.hots.iter().find(|h| h.act == Act::Slider) {
            let t = ((x - h.x - 9.0) / (h.w - 18.0)).clamp(0.0, 1.0);
            let span = (config::SCALE_MAX - config::SCALE_MIN) as f32;
            self.scale_draft = Some(config::SCALE_MIN + ((t * span / 5.0).round() * 5.0) as u32);
        }
    }

    fn mouse_move(&mut self, x: f32, y: f32) {
        let mut now = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut now);
        }
        let Some(d) = &mut self.drag else {
            let (act, row) = self.hit(x, y).map_or((Act::None, None), |h| (h.act.clone(), h.row));
            self.hover = act;
            if let Some(i) = row {
                match self.mode {
                    Mode::Search => self.sel = i,
                    Mode::Settings if self.set_sel != i => {
                        self.set_sel = i;
                        self.cancel_recording();
                    }
                    _ => {}
                }
            }
            return;
        };
        let (dx, dy) = ((now.x - d.start.x) as f32, (now.y - d.start.y) as f32);
        if d.kind.is_none() && matches!(d.act, Act::Bar | Act::QueryField) && dx.abs().max(dy.abs()) > 4.0 {
            d.kind = Some(DragKind::Move);
        }
        let (kind, win, width0, scale0) = (d.kind, d.win, d.width, d.scale);
        let k = self.dpi * scale0 as f32 / 100.0;
        let (sw, sh) = ((self.work.right - self.work.left) as f32, (self.work.bottom - self.work.top) as f32);
        let w_px = (win.right - win.left) as f32;
        match kind {
            Some(DragKind::Move) => {
                let mut cx = win.left as f32 + dx + w_px / 2.0 - self.work.left as f32;
                // 靠近屏幕中线时吸附
                if (cx - sw / 2.0).abs() < 14.0 * k {
                    cx = sw / 2.0;
                }
                let half = w_px / 2.0 / sw;
                self.cfg.cx = (cx / sw).clamp(half, 1.0 - half);
                self.cfg.ty = ((win.top as f32 + dy - self.work.top as f32) / sh).clamp(0.0, 1.0 - BAR * k / sh);
            }
            Some(DragKind::Left | DragKind::Right) => {
                // 两边对称伸缩，面板中心不动
                let sign = if kind == Some(DragKind::Left) { -1.0 } else { 1.0 };
                let w = width0 as f32 + sign * 2.0 * dx / k;
                let limit = (sw * 0.9 / k / 4.0).floor() * 4.0;
                let w = ((w / 4.0).round() * 4.0).min(limit);
                self.cfg.width = (w as u32).clamp(config::WIDTH_MIN, config::WIDTH_MAX);
            }
            Some(DragKind::Corner) => {
                let s = scale0 as f32 * (1.0 + 2.0 * dx / w_px);
                self.cfg.scale = (s.round() as u32).clamp(config::SCALE_MIN, config::SCALE_MAX);
            }
            Some(DragKind::Slider) => self.slide(x),
            None => {}
        }
    }

    fn mouse_up(&mut self, x: f32, y: f32) {
        unsafe {
            let _ = ReleaseCapture();
        }
        let Some(d) = self.drag.take() else { return };
        match d.kind {
            Some(DragKind::Slider) => {
                if let Some(v) = self.scale_draft {
                    self.set_scale(v);
                }
            }
            Some(DragKind::Corner) => {
                self.changed();
                self.scan(Scan::Silent, false);
            }
            Some(_) => self.changed(),
            None => {
                if self.hit(x, y).is_some_and(|h| h.act == d.act) {
                    self.click(d.act, x);
                }
            }
        }
    }

    fn click(&mut self, act: Act, x: f32) {
        self.caret_on = true;
        match act {
            Act::None | Act::Bar | Act::Slider => {}
            Act::QueryField => {
                if let Some(g) = &mut self.gfx {
                    let at = g.field_hit(&self.query, x);
                    self.query.move_to(at, false);
                }
            }
            Act::Row(i) => self.launch(i, false),
            Act::OpenSettings => self.open_settings(),
            Act::Back => self.close_settings(),
            Act::Tab(i) => self.set_tab(i),
            Act::Seg(row, i) => self.set_seg(row, i),
            Act::Toggle(row) => self.set_switch(row, !self.switch_value(row)),
            Act::Step(d) => self.step_rows(d),
            Act::Button(row) => self.press_button(row),
            Act::Remove(i) => self.remove_dir(i),
            Act::Plus => self.pick(),
            Act::PathField => {
                if let Some(g) = &mut self.gfx {
                    let at = g.field_hit(&self.path, x);
                    self.path.move_to(at, false);
                }
            }
            Act::ExtField => {}
            Act::Token(ext, _) => self.toggle_ext(&ext),
            Act::Refresh => self.scan(Scan::Manual, true),
            Act::HiddenToggle => self.hidden_open = !self.hidden_open,
            Act::Restore(i) => self.restore(i),
            Act::AddDirHint => self.goto_add(),
        }
    }

    // ---------- 消息 ----------

    fn tray(&self, action: NOTIFY_ICON_MESSAGE) {
        unsafe {
            let mut data = NOTIFYICONDATAW {
                cbSize: size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: self.hwnd,
                uID: 1,
                uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
                uCallbackMessage: WM_TRAY,
                hIcon: self.tray_icon,
                ..Default::default()
            };
            for (dst, src) in data.szTip.iter_mut().zip("frun".encode_utf16()) {
                *dst = src;
            }
            let _ = Shell_NotifyIconW(action, &data);
        }
    }

    fn toggle(&mut self) {
        if self.visible { self.hide() } else { self.show() }
    }

    fn on_message(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let k = self.k();
        // 结果行的菜单开着时，面板保持原样
        if self.menu.on_row() && matches!(msg, WM_MOUSEMOVE | WM_MOUSEWHEEL | WM_TIMER) && wp.0 != TIMER_FRAME {
            return Some(LRESULT(0));
        }
        let point = || ((lp.0 as i16) as f32 / k, ((lp.0 >> 16) as i16) as f32 / k);
        match msg {
            WM_HOTKEY => self.toggle(),
            WM_SUMMON => self.show(),
            WM_TRAY => match lp.0 as u32 & 0xFFFF {
                WM_LBUTTONUP => self.toggle(),
                WM_RBUTTONUP => self.open_menu(),
                _ => {}
            },
            // 点击面板外收起。设置开着时不收：要留着接收从资源管理器拖来的文件夹
            WM_ACTIVATE if wp.0 & 0xFFFF == WA_INACTIVE as usize => {
                self.mod_since = None;
                if self.mode == Mode::Search && !self.pinned && !self.menu.on_row() {
                    self.hide();
                } else {
                    self.cancel_recording();
                    self.invalidate();
                }
            }
            WM_PAINT => self.frame(),
            WM_ERASEBKGND => return Some(LRESULT(1)),
            WM_TIMER => match wp.0 {
                TIMER_FRAME => self.frame(),
                TIMER_CARET => {
                    self.caret_on = !self.caret_on;
                    self.invalidate();
                }
                TIMER_TRIM => unsafe {
                    let _ = KillTimer(Some(self.hwnd), TIMER_TRIM);
                    if !self.visible {
                        let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
                    }
                },
                TIMER_DIRTY => {
                    unsafe {
                        let _ = KillTimer(Some(self.hwnd), TIMER_DIRTY);
                    }
                    let only = std::mem::take(&mut self.dirty);
                    if std::mem::take(&mut self.dirty_all) || only.len() > 8 {
                        self.scan(Scan::Silent, false);
                    } else {
                        self.scan_only(&only);
                    }
                }
                _ => {}
            },
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                // 输入法组字期间的按键属于输入法
                if wp.0 as u16 == VK_PROCESSKEY.0 {
                    return None;
                }
                let handled = self.key(wp.0 as u32, lp.0 & (1 << 30) != 0);
                self.invalidate();
                if !handled {
                    return None;
                }
            }
            WM_KEYUP | WM_SYSKEYUP => {
                if !key_down(VK_MENU.0 as u32) && !key_down(VK_CONTROL.0 as u32) {
                    self.mod_since = None;
                }
                self.invalidate();
            }
            WM_CHAR => {
                self.char(wp.0 as u16);
                self.invalidate();
            }
            // Alt 组合键产生的字符和菜单激活都不要，否则会响提示音或卡住焦点
            WM_SYSCHAR => {}
            WM_SYSCOMMAND if wp.0 & 0xFFF0 == SC_KEYMENU as usize => {}
            WM_CLOSE => self.hide(),
            WM_IME_STARTCOMPOSITION => {
                let tf = match (self.mode, self.srow()) {
                    (Mode::Search, _) => &self.query,
                    (_, Some(SRow::Ext)) => &self.ext,
                    _ => &self.path,
                };
                let y = if self.mode == Mode::Search { 40.0 } else { BAR + (self.set_sel as f32 + 0.8) * ROW };
                ime_follow(self.hwnd, (tf.caret_x * k) as i32, (y * k) as i32);
                return None;
            }
            WM_MOUSEMOVE => {
                let (x, y) = point();
                self.mouse_move(x, y);
                self.invalidate();
            }
            WM_LBUTTONDOWN => {
                let (x, y) = point();
                self.mouse_down(x, y);
                self.invalidate();
            }
            WM_LBUTTONUP => {
                let (x, y) = point();
                self.mouse_up(x, y);
                self.invalidate();
            }
            WM_RBUTTONUP if self.mode == Mode::Search => {
                let (x, y) = point();
                if let Some(Act::Row(i)) = self.hit(x, y).map(|h| h.act.clone()) {
                    let mut pt = POINT::default();
                    unsafe {
                        let _ = GetCursorPos(&mut pt);
                    }
                    self.open_row_menu(i, Some(pt));
                }
            }
            WM_CAPTURECHANGED => {
                if self.drag.take().is_some() {
                    self.scale_draft = None;
                    self.changed();
                }
            }
            WM_MOUSEWHEEL if self.mode == Mode::Search => {
                self.move_sel(if (wp.0 >> 16) as i16 > 0 { -1 } else { 1 }, false);
                self.invalidate();
            }
            WM_SETCURSOR if lp.0 as u32 & 0xFFFF == HTCLIENT => unsafe {
                let mut pt = POINT::default();
                let _ = GetCursorPos(&mut pt);
                let _ = ScreenToClient(self.hwnd, &mut pt);
                let id = self.cursor(pt.x as f32 / k, pt.y as f32 / k);
                SetCursor(LoadCursorW(None, id).ok());
                return Some(LRESULT(1));
            },
            WM_INDEX => {
                self.index_ready();
                self.invalidate();
            }
            WM_PROGRESS => {
                self.progress = wp.0;
                self.invalidate();
            }
            WM_ICONS => {
                if let Some(g) = &mut self.gfx {
                    g.forget_missing_icons();
                }
                self.invalidate();
            }
            WM_SCAN_DONE => {
                self.scanning = false;
                self.last_scan = Instant::now();
                if self.scan_kind == Scan::Manual && self.flash_until.is_none() {
                    self.flash.clear();
                }
                if std::mem::take(&mut self.scan_pending) {
                    let kind = std::mem::replace(&mut self.scan_kind, Scan::Silent);
                    self.scan(kind, false);
                } else {
                    self.scan_kind = Scan::Silent;
                }
                self.invalidate();
            }
            // 目录里有文件增删：等一秒让连续的变化攒在一起，再扫一次
            WM_DIRTY => unsafe {
                if lp.0 == 0 {
                    self.dirty_all = true;
                } else {
                    let path = *Box::from_raw(lp.0 as *mut String);
                    if !self.dirty.contains(&path) {
                        self.dirty.push(path);
                    }
                }
                SetTimer(Some(self.hwnd), TIMER_DIRTY, 1000, None);
            },
            WM_DROP_HOVER => {
                self.dropping = wp.0 != 0;
                self.invalidate();
            }
            WM_PICKED => {
                self.pinned = false;
                if let Some(path) = unsafe { *Box::from_raw(lp.0 as *mut Option<String>) } {
                    self.add_dir(&path);
                }
                unsafe {
                    let _ = SetForegroundWindow(self.hwnd);
                }
                self.invalidate();
            }
            WM_DROP_PATH => {
                let path = unsafe { *Box::from_raw(lp.0 as *mut String) };
                self.add_dir(&path);
                self.invalidate();
            }
            WM_SETTINGCHANGE | WM_DWMCOLORIZATIONCOLORCHANGED => {
                self.refresh_theme();
                self.invalidate();
            }
            WM_DESTROY => unsafe {
                self.tray(NIM_DELETE);
                PostQuitMessage(0);
            },
            // 资源管理器重启后托盘图标需要重新添加
            m if m == self.taskbar_created => self.tray(NIM_ADD),
            _ => return None,
        }
        Some(LRESULT(0))
    }
}

/// 来源在界面上怎么称呼。
pub fn source_label<'a>(s: &'a Strings, e: &'a index::Entry) -> &'a str {
    match e.kind {
        Kind::Start => &s.src_start,
        Kind::Store => &s.src_store,
        Kind::Dir => &e.workdir,
    }
}


/// 程序所在的位置：快捷方式和开始菜单项给出它们指向的真实文件；商店应用没有。
pub fn location(e: &index::Entry) -> Option<String> {
    let real = e.resolved.split('|').next().unwrap_or("");
    if !real.is_empty() && std::path::Path::new(real).exists() {
        Some(real.to_string())
    } else if e.kind == Kind::Dir {
        Some(e.target.clone())
    } else {
        None
    }
}

/// 路径补全：输入的最后一段是某个子文件夹名的开头时，给出 (补全后的整条路径, 还没输入的部分)。
/// 只处理带盘符的本地路径；网络路径可能一等就是几秒。
fn complete_path(text: &str) -> Option<(String, String)> {
    let text = text.trim_start().trim_start_matches('"');
    let bytes = text.as_bytes();
    if !bytes.first()?.is_ascii_alphabetic() {
        return None;
    }
    // "d" 或 "d:" 补成盘符根目录
    if bytes.len() == 1 || (bytes.len() == 2 && bytes[1] == b':') {
        let root = format!("{}:\\", text[..1].to_uppercase());
        return std::path::Path::new(&root).is_dir().then(|| (root.clone(), root[bytes.len()..].to_string()));
    }
    if bytes[1] != b':' {
        return None;
    }
    let cut = text.rfind(['\\', '/'])? + 1;
    let (parent, typed) = text.split_at(cut);
    if typed.is_empty() {
        return None;
    }
    let typed_lower = typed.to_lowercase();
    let name = std::fs::read_dir(parent)
        .ok()?
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.to_lowercase().starts_with(&typed_lower) && !crate::scan::skip_dir(n))
        .min_by_key(|n| n.to_lowercase())?;
    let rest: String = name.chars().skip(typed.chars().count()).collect();
    Some((format!("{parent}{name}\\"), format!("{rest}\\")))
}
