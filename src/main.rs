#![windows_subsystem = "windows"]

mod config;
mod index;
mod search;

use config::Config;
use index::Item;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    mem::{size_of, transmute},
    ptr::null_mut,
    sync::mpsc,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::{Dwm::*, Gdi::*},
        System::{Com::*, LibraryLoader::*, Registry::*, Threading::*},
        UI::{HiDpi::*, Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
    },
    core::*,
};

const WM_TRAY: u32 = WM_APP + 1;
const WM_INDEX: u32 = WM_APP + 2;
const WM_ICON: u32 = WM_APP + 3;
const HOTKEY_ID: i32 = 1;

const CMD_RELOAD: usize = 1;
const CMD_CONFIG: usize = 2;
const CMD_AUTOSTART: usize = 3;
const CMD_QUIT: usize = 4;

// 96 DPI 下的尺寸
const WIDTH: i32 = 640;
const INPUT_H: i32 = 56;
const ROW_H: i32 = 44;
const PAD: i32 = 16;
const ICON: i32 = 28;

const RESCAN_AFTER: Duration = Duration::from_secs(600);
const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");

/// 颜色为 COLORREF（0x00BBGGRR）
struct Theme {
    bg: u32,
    text: u32,
    dim: u32,
    sel: u32,
}
const DARK: Theme = Theme { bg: 0x202020, text: 0xF2F2F2, dim: 0x8C8C8C, sel: 0x3A3A3A };
const LIGHT: Theme = Theme { bg: 0xF9F9F9, text: 0x1A1A1A, dim: 0x767676, sel: 0xE4E4E4 };

struct State {
    cfg: Config,
    items: Vec<Item>,
    results: Vec<usize>,
    sel: usize,
    usage: HashMap<String, u32>,
    /// 启动目标 -> HBITMAP；0 表示正在加载或没有图标
    icons: HashMap<String, isize>,
    last_scan: Instant,
}

/// 窗口过程会重入（SetWindowPos、ShowWindow 等都会同步派发消息），
/// 所以可变状态放在 RefCell 里，并且不在持有借用时调用会派发消息的 API。
struct App {
    hwnd: HWND,
    edit: HWND,
    scale: f32,
    font_row: HFONT,
    font_hint: HFONT,
    bg_dark: HBRUSH,
    bg_light: HBRUSH,
    dark: Cell<bool>,
    taskbar_created: u32,
    icon_tx: mpsc::Sender<String>,
    s: RefCell<State>,
}

static mut APP: *mut App = null_mut();
static mut EDIT_PROC: isize = 0;

fn main() -> Result<()> {
    unsafe {
        let _mutex = CreateMutexW(None, true, w!("frun.single"))?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return Ok(());
        }
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;

        let inst: HINSTANCE = GetModuleHandleW(None)?.into();
        let class = w!("frun");
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: inst,
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        });
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            class,
            w!("frun"),
            WS_POPUP | WS_CLIPCHILDREN,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(inst),
            None,
        )?;
        let round = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &round as *const _ as _,
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );

        let scale = GetDpiForSystem() as f32 / 96.0;
        let px = |v: i32| (v as f32 * scale).round() as i32;
        let font = |size: i32| {
            CreateFontW(
                -px(size),
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                w!("Segoe UI"),
            )
        };
        let font_input = font(20);

        let edit_h = px(28);
        let edit = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("EDIT"),
            None,
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            px(PAD),
            (px(INPUT_H) - edit_h) / 2,
            px(WIDTH) - 2 * px(PAD),
            edit_h,
            Some(hwnd),
            None,
            Some(inst),
            None,
        )?;
        SendMessageW(edit, WM_SETFONT, Some(WPARAM(font_input.0 as usize)), Some(LPARAM(1)));

        // 图标在后台线程加载，结果通过 WM_ICON 送回
        let (icon_tx, icon_rx) = mpsc::channel::<String>();
        let (target_hwnd, icon_px) = (hwnd.0 as isize, px(ICON));
        std::thread::spawn(move || {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            for target in icon_rx {
                let bmp = index::icon(&target, icon_px).map_or(0, |b| b.0 as usize);
                let boxed = Box::into_raw(Box::new(target));
                let _ = PostMessageW(
                    Some(HWND(target_hwnd as _)),
                    WM_ICON,
                    WPARAM(bmp),
                    LPARAM(boxed as isize),
                );
            }
        });

        APP = Box::into_raw(Box::new(App {
            hwnd,
            edit,
            scale,
            font_row: font(15),
            font_hint: font(12),
            bg_dark: CreateSolidBrush(COLORREF(DARK.bg)),
            bg_light: CreateSolidBrush(COLORREF(LIGHT.bg)),
            dark: Cell::new(true),
            taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
            icon_tx,
            s: RefCell::new(State {
                cfg: Config::default(),
                items: Vec::new(),
                results: Vec::new(),
                sel: 0,
                usage: config::load_usage(),
                icons: HashMap::new(),
                last_scan: Instant::now(),
            }),
        }));
        let app = &*APP;
        EDIT_PROC = SetWindowLongPtrW(edit, GWLP_WNDPROC, editproc as *const () as isize);
        app.tray(NIM_ADD);
        app.reload();

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        Ok(())
    }
}

impl App {
    fn px(&self, v: i32) -> i32 {
        (v as f32 * self.scale).round() as i32
    }

    fn theme(&self) -> &'static Theme {
        if self.dark.get() { &DARK } else { &LIGHT }
    }

    fn bg(&self) -> HBRUSH {
        if self.dark.get() { self.bg_dark } else { self.bg_light }
    }

    /// 读配置、注册热键、重建索引。
    fn reload(&self) {
        let (cfg, err) = config::load();
        let hotkey = cfg.hotkey.clone();
        self.s.borrow_mut().cfg = cfg;
        let registered = unsafe {
            let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_ID);
            config::parse_hotkey(&hotkey).is_some_and(|(mods, vk)| {
                let mods = HOT_KEY_MODIFIERS(mods) | MOD_NOREPEAT;
                RegisterHotKey(Some(self.hwnd), HOTKEY_ID, mods, vk).is_ok()
            })
        };
        self.rescan();
        if let Some(e) = err {
            self.alert(&format!("config.toml 解析失败，已使用默认配置：\n\n{e}"));
        } else if !registered {
            self.alert(&format!(
                "热键 {hotkey} 注册失败：格式不对，或已被其他程序占用。\n请在托盘菜单「打开配置」中修改。"
            ));
        }
    }

    fn alert(&self, text: &str) {
        unsafe {
            MessageBoxW(None, &HSTRING::from(text), w!("frun"), MB_ICONWARNING | MB_SETFOREGROUND);
        }
    }

    fn rescan(&self) {
        let (dirs, depth) = {
            let mut s = self.s.borrow_mut();
            s.last_scan = Instant::now();
            (s.cfg.dirs.clone(), s.cfg.depth)
        };
        let hwnd = self.hwnd.0 as isize;
        std::thread::spawn(move || unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let items = Box::into_raw(Box::new(index::scan(&dirs, depth)));
            let _ = PostMessageW(Some(HWND(hwnd as _)), WM_INDEX, WPARAM(0), LPARAM(items as isize));
        });
    }

    fn show(&self) {
        if self.s.borrow().last_scan.elapsed() > RESCAN_AFTER {
            self.rescan();
        }
        self.dark.set(!light_theme());
        unsafe {
            let _ = SetWindowTextW(self.edit, w!(""));
            self.query();

            // 出现在鼠标所在的显示器上
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST), &mut mi);
            let r = mi.rcWork;
            let x = r.left + (r.right - r.left - self.px(WIDTH)) / 2;
            let y = r.top + (r.bottom - r.top) / 4;
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), x, y, 0, 0, SWP_NOSIZE | SWP_SHOWWINDOW);
            let _ = SetForegroundWindow(self.hwnd);
            let _ = SetFocus(Some(self.edit));
        }
    }

    fn hide(&self) {
        unsafe {
            if IsWindowVisible(self.hwnd).as_bool() {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
        }
    }

    /// 按输入框内容重新搜索。
    fn query(&self) {
        let mut buf = [0u16; 256];
        let len = unsafe { GetWindowTextW(self.edit, &mut buf) } as usize;
        let text = String::from_utf16_lossy(&buf[..len]);
        {
            let mut s = self.s.borrow_mut();
            let s = &mut *s;
            s.results = search::run(&s.items, &s.usage, &text, s.cfg.max_results);
            s.sel = 0;
        }
        let rows = self.s.borrow().results.len() as i32;
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                0,
                0,
                self.px(WIDTH),
                self.px(INPUT_H) + rows * self.px(ROW_H),
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn move_sel(&self, delta: i32) {
        {
            let mut s = self.s.borrow_mut();
            let n = s.results.len() as i32;
            if n > 0 {
                s.sel = (s.sel as i32 + delta).rem_euclid(n) as usize;
            }
        }
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn launch(&self, admin: bool) {
        let (target, dir) = {
            let mut s = self.s.borrow_mut();
            let s = &mut *s;
            let Some(&i) = s.results.get(s.sel) else { return };
            let item = &s.items[i];
            *s.usage.entry(item.target.clone()).or_default() += 1;
            config::save_usage(&s.usage);
            (HSTRING::from(&item.target), HSTRING::from(&item.dir))
        };
        self.hide();
        let dir = if dir.is_empty() { PCWSTR::null() } else { PCWSTR(dir.as_ptr()) };
        let verb = if admin { w!("runas") } else { w!("open") };
        unsafe {
            ShellExecuteW(None, verb, &target, None, dir, SW_SHOWNORMAL);
        }
    }

    fn paint(&self) {
        unsafe {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(self.hwnd, &mut ps);
            let mut rc = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut rc);
            let t = self.theme();

            // 先画到内存位图再整体贴上，避免闪烁
            let mem = CreateCompatibleDC(Some(hdc));
            let bmp = CreateCompatibleBitmap(hdc, rc.right, rc.bottom);
            let old_bmp = SelectObject(mem, bmp.into());
            let icon_dc = CreateCompatibleDC(Some(hdc));
            let sel_brush = CreateSolidBrush(COLORREF(t.sel));
            FillRect(mem, &rc, self.bg());
            SetBkMode(mem, TRANSPARENT);

            let (input_h, row_h, pad, icon) =
                (self.px(INPUT_H), self.px(ROW_H), self.px(PAD), self.px(ICON));
            let mut missing = Vec::new();
            {
                let s = self.s.borrow();
                if !s.results.is_empty() {
                    let line = RECT { left: 0, top: input_h - 1, right: rc.right, bottom: input_h };
                    FillRect(mem, &line, sel_brush);
                }
                for (row, &i) in s.results.iter().enumerate() {
                    let item = &s.items[i];
                    let top = input_h + row as i32 * row_h;
                    let bottom = top + row_h;
                    if row == s.sel {
                        FillRect(mem, &RECT { left: 0, top, right: rc.right, bottom }, sel_brush);
                    }
                    match s.icons.get(&item.target) {
                        Some(&0) => {}
                        Some(&h) => {
                            let hbmp = HBITMAP(h as _);
                            let mut info = BITMAP::default();
                            GetObjectW(
                                hbmp.into(),
                                size_of::<BITMAP>() as i32,
                                Some(&mut info as *mut _ as _),
                            );
                            let old = SelectObject(icon_dc, hbmp.into());
                            let blend = BLENDFUNCTION {
                                BlendOp: AC_SRC_OVER as u8,
                                BlendFlags: 0,
                                SourceConstantAlpha: 255,
                                AlphaFormat: AC_SRC_ALPHA as u8,
                            };
                            let _ = AlphaBlend(
                                mem,
                                pad,
                                top + (row_h - icon) / 2,
                                icon,
                                icon,
                                icon_dc,
                                0,
                                0,
                                info.bmWidth,
                                info.bmHeight,
                                blend,
                            );
                            SelectObject(icon_dc, old);
                        }
                        None => missing.push(item.target.clone()),
                    }
                    // 自定义目录里的条目在右侧显示所在目录
                    let left = pad + icon + self.px(12);
                    let split = if item.dir.is_empty() { rc.right - pad } else { rc.right * 3 / 5 };
                    let name = RECT { left, top, right: split, bottom };
                    draw(mem, self.font_row, t.text, &item.name, name, DT_LEFT | DT_END_ELLIPSIS);
                    if !item.dir.is_empty() {
                        let hint = RECT { left: split + self.px(12), top, right: rc.right - pad, bottom };
                        draw(mem, self.font_hint, t.dim, &item.dir, hint, DT_RIGHT | DT_PATH_ELLIPSIS);
                    }
                }
            }

            let _ = BitBlt(hdc, 0, 0, rc.right, rc.bottom, Some(mem), 0, 0, SRCCOPY);
            SelectObject(mem, old_bmp);
            let _ = DeleteObject(bmp.into());
            let _ = DeleteObject(sel_brush.into());
            let _ = DeleteDC(icon_dc);
            let _ = DeleteDC(mem);
            let _ = EndPaint(self.hwnd, &ps);

            if !missing.is_empty() {
                let mut s = self.s.borrow_mut();
                for target in missing {
                    s.icons.insert(target.clone(), 0);
                    let _ = self.icon_tx.send(target);
                }
            }
        }
    }

    fn tray(&self, action: NOTIFY_ICON_MESSAGE) {
        unsafe {
            let mut data = NOTIFYICONDATAW {
                cbSize: size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: self.hwnd,
                uID: 1,
                uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
                uCallbackMessage: WM_TRAY,
                hIcon: LoadIconW(None, IDI_APPLICATION).unwrap_or_default(),
                ..Default::default()
            };
            for (dst, src) in data.szTip.iter_mut().zip("frun".encode_utf16()) {
                *dst = src;
            }
            let _ = Shell_NotifyIconW(action, &data);
        }
    }

    fn menu(&self) {
        unsafe {
            let Ok(menu) = CreatePopupMenu() else { return };
            let check = if autostart() { MF_CHECKED } else { MF_UNCHECKED };
            let _ = AppendMenuW(menu, MF_STRING, CMD_RELOAD, w!("重新加载"));
            let _ = AppendMenuW(menu, MF_STRING, CMD_CONFIG, w!("打开配置"));
            let _ = AppendMenuW(menu, MF_STRING | check, CMD_AUTOSTART, w!("开机自启"));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            let _ = AppendMenuW(menu, MF_STRING, CMD_QUIT, w!("退出"));
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            // 不先取得前台，菜单在点击别处时不会自动关闭
            let _ = SetForegroundWindow(self.hwnd);
            let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON, pt.x, pt.y, None, self.hwnd, None);
            let _ = DestroyMenu(menu);
        }
    }

    fn command(&self, id: usize) {
        match id {
            CMD_RELOAD => self.reload(),
            CMD_CONFIG => unsafe {
                let path = HSTRING::from(config::path().as_os_str());
                ShellExecuteW(None, w!("open"), &path, None, None, SW_SHOWNORMAL);
            },
            CMD_AUTOSTART => set_autostart(!autostart()),
            CMD_QUIT => unsafe {
                let _ = DestroyWindow(self.hwnd);
            },
            _ => {}
        }
    }
}

unsafe fn draw(dc: HDC, font: HFONT, color: u32, text: &str, mut rc: RECT, flags: DRAW_TEXT_FORMAT) {
    unsafe {
        SelectObject(dc, font.into());
        SetTextColor(dc, COLORREF(color));
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        DrawTextW(dc, &mut wide, &mut rc, flags | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX);
    }
}

fn light_theme() -> bool {
    let (mut value, mut size) = (0u32, size_of::<u32>() as u32);
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut _ as _),
            Some(&mut size),
        )
        .is_ok()
            && value != 0
    }
}

fn autostart() -> bool {
    unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, w!("frun"), RRF_RT_REG_SZ, None, None, None).is_ok() }
}

fn set_autostart(on: bool) {
    unsafe {
        if on {
            let Ok(exe) = std::env::current_exe() else { return };
            let cmd: Vec<u16> = format!("\"{}\"", exe.display()).encode_utf16().chain([0]).collect();
            let _ = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                w!("frun"),
                REG_SZ.0,
                Some(cmd.as_ptr() as _),
                (cmd.len() * 2) as u32,
            );
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, w!("frun"));
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if APP.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let app = &*APP;
        match msg {
            WM_HOTKEY => {
                if IsWindowVisible(hwnd).as_bool() {
                    app.hide()
                } else {
                    app.show()
                }
            }
            WM_ACTIVATE if wp.0 & 0xFFFF == WA_INACTIVE as usize => app.hide(),
            WM_SETFOCUS => {
                let _ = SetFocus(Some(app.edit));
            }
            WM_PAINT => app.paint(),
            WM_ERASEBKGND => return LRESULT(1),
            WM_CTLCOLOREDIT => {
                let (dc, t) = (HDC(wp.0 as _), app.theme());
                SetTextColor(dc, COLORREF(t.text));
                SetBkColor(dc, COLORREF(t.bg));
                return LRESULT(app.bg().0 as isize);
            }
            WM_COMMAND => {
                if lp.0 == 0 {
                    app.command(wp.0 & 0xFFFF);
                } else if (wp.0 >> 16) & 0xFFFF == EN_CHANGE as usize {
                    app.query();
                }
            }
            WM_TRAY => match lp.0 as u32 & 0xFFFF {
                WM_LBUTTONUP => app.show(),
                WM_RBUTTONUP => app.menu(),
                _ => {}
            },
            WM_INDEX => {
                let items = *Box::from_raw(lp.0 as *mut Vec<Item>);
                app.s.borrow_mut().items = items;
                app.query();
            }
            WM_ICON => {
                let target = *Box::from_raw(lp.0 as *mut String);
                app.s.borrow_mut().icons.insert(target, wp.0 as isize);
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            WM_DESTROY => {
                app.tray(NIM_DELETE);
                PostQuitMessage(0);
            }
            // 资源管理器重启后托盘图标需要重新添加
            m if m == app.taskbar_created => app.tray(NIM_ADD),
            _ => return DefWindowProcW(hwnd, msg, wp, lp),
        }
        LRESULT(0)
    }
}

/// 输入框的子类过程：把导航键交给列表，其余按键留给编辑控件。
unsafe extern "system" fn editproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let app = &*APP;
        let down = |key: VIRTUAL_KEY| GetKeyState(key.0 as i32) < 0;
        match msg {
            WM_KEYDOWN => {
                match VIRTUAL_KEY(wp.0 as u16) {
                    VK_ESCAPE => app.hide(),
                    VK_RETURN => app.launch(down(VK_CONTROL)),
                    VK_DOWN => app.move_sel(1),
                    VK_UP => app.move_sel(-1),
                    VK_TAB => app.move_sel(if down(VK_SHIFT) { -1 } else { 1 }),
                    _ => return CallWindowProcW(transmute(EDIT_PROC), hwnd, msg, wp, lp),
                }
                LRESULT(0)
            }
            // 吞掉这些键产生的字符，否则单行编辑框会发出提示音
            WM_CHAR if matches!(wp.0, 9 | 10 | 13 | 27) => LRESULT(0),
            _ => CallWindowProcW(transmute(EDIT_PROC), hwnd, msg, wp, lp),
        }
    }
}
