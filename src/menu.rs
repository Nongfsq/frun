//! 菜单：和面板同一种底、同一套行与选中样式的小窗口。托盘图标和结果行的右键共用。
use crate::{
    app::*,
    gfx::{Align, Gfx},
    index::Kind,
    platform::monitor,
};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::{Dwm::*, Gdi::*},
        System::LibraryLoader::GetModuleHandleW,
        UI::{Controls::MARGINS, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    },
    core::*,
};

const WIDTH: f32 = 208.0;
const ITEM: f32 = 36.0;

#[derive(Clone, Copy, PartialEq)]
enum Cmd {
    Settings,
    Reindex,
    Quit,
    Open,
    Admin,
    Folder,
    Hide,
}

#[derive(Default)]
pub struct Menu {
    hwnd: Option<HWND>,
    open: bool,
    sel: usize,
    k: f32,
    width: f32,
    /// (文字, 按键提示, 命令)
    items: Vec<(String, &'static str, Cmd)>,
    /// 结果行的菜单：针对第几行
    row: Option<usize>,
}

impl Menu {
    /// 结果行的菜单开着：面板此时不响应鼠标，也不因失去焦点而收起。
    pub fn on_row(&self) -> bool {
        self.open && self.row.is_some()
    }
}

impl App {
    /// 托盘菜单：右下角对着鼠标。
    pub fn open_menu(&mut self) {
        self.hide();
        let items = vec![
            (self.s.tray_settings.clone(), "Ctrl ,", Cmd::Settings),
            (self.s.tray_reindex.clone(), "Ctrl R", Cmd::Reindex),
            (self.s.tray_quit.clone(), "", Cmd::Quit),
        ];
        let (work, dpi) = monitor(false);
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
        self.popup(items, None, pt, true, work, dpi);
    }

    /// 结果行的菜单：把只有快捷键的操作摆出来。`at` 为空时贴着这一行弹出（键盘触发）。
    pub fn open_row_menu(&mut self, i: usize, at: Option<POINT>) {
        let Some(row) = self.rows.get(i) else { return };
        let entry = &self.index.groups[row.group].members[row.member];
        let mut items = vec![(self.s.menu_open.clone(), "↵", Cmd::Open)];
        if entry.kind != Kind::Store && !self.cfg.admin {
            items.push((self.s.admin.clone(), "Ctrl Shift ↵", Cmd::Admin));
        }
        if location(entry).is_some() {
            items.push((self.s.menu_folder.clone(), "Ctrl ↵", Cmd::Folder));
        }
        items.push((self.s.menu_hide.clone(), "Ctrl Del", Cmd::Hide));
        self.sel = i;
        let at = at.unwrap_or_else(|| {
            let mut rc = RECT::default();
            unsafe {
                let _ = GetWindowRect(self.hwnd, &mut rc);
            }
            let k = self.k();
            POINT { x: rc.left + (60.0 * k) as i32, y: rc.top + ((BAR + (i as f32 + 0.8) * ROW) * k) as i32 }
        });
        let (work, dpi) = (self.work, self.dpi);
        self.popup(items, Some(i), at, false, work, dpi);
        self.invalidate();
    }

    fn popup(
        &mut self,
        items: Vec<(String, &'static str, Cmd)>,
        row: Option<usize>,
        at: POINT,
        above: bool,
        work: RECT,
        dpi: f32,
    ) {
        let hwnd = match self.menu.hwnd {
            Some(h) => h,
            None => unsafe {
                let inst: HINSTANCE = GetModuleHandleW(None).unwrap_or_default().into();
                RegisterClassW(&WNDCLASSW {
                    lpfnWndProc: Some(menuproc),
                    hInstance: inst,
                    lpszClassName: w!("frun.menu"),
                    hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                    ..Default::default()
                });
                let Ok(h) = CreateWindowExW(
                    WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                    w!("frun.menu"),
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
                ) else {
                    return;
                };
                let set = |attr: DWMWINDOWATTRIBUTE, v: i32| {
                    let _ = DwmSetWindowAttribute(h, attr, &v as *const _ as _, 4);
                };
                set(DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_TRANSIENTWINDOW.0);
                let margins = MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 };
                let _ = DwmExtendFrameIntoClientArea(h, &margins);
                self.menu.hwnd = Some(h);
                h
            },
        };
        self.refresh_theme();
        // 宽度跟着最长的一项走
        let mut width = WIDTH;
        if let Some(g) = &mut self.gfx {
            for (label, key, _) in &items {
                let kw = if key.is_empty() { 0.0 } else { g.measure(key, 12.0) + 24.0 };
                width = width.max(g.measure(label, 14.0) + kw + 34.0);
            }
        }
        let dark = self.theme_is_dark() as i32;
        let (w, h) = ((width * dpi).round() as i32, ((items.len() as f32 * ITEM + 12.0) * dpi).round() as i32);
        self.menu.width = width;
        self.menu.items = items;
        self.menu.row = row;
        self.menu.k = dpi;
        self.menu.sel = 0;
        self.menu.open = true;
        unsafe {
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as _, 4);
            let corner = self.corner_preference();
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &corner as *const _ as _, 4);
            // 整个菜单留在工作区内（不盖住任务栏）
            let (x, y) = if above { (at.x - w, at.y - h) } else { (at.x, at.y) };
            let x = x.clamp(work.left, (work.right - w).max(work.left));
            let y = y.clamp(work.top, (work.bottom - h).max(work.top));
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_SHOWWINDOW);
            let _ = SetForegroundWindow(hwnd);
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }

    /// 收起菜单。`back` 为真时把焦点还给面板（结果行的菜单）。
    fn close_menu(&mut self, back: bool) {
        if !self.menu.open {
            return;
        }
        self.menu.open = false;
        let on_row = self.menu.row.take().is_some();
        if let Some(h) = self.menu.hwnd {
            unsafe {
                let _ = ShowWindow(h, SW_HIDE);
            }
        }
        if let Some(g) = &mut self.gfx {
            g.release();
        }
        if on_row {
            if back {
                unsafe {
                    let _ = SetForegroundWindow(self.hwnd);
                }
                self.invalidate();
            } else {
                self.hide();
            }
        }
    }

    fn run_menu(&mut self, item: usize) {
        let Some(&(_, _, cmd)) = self.menu.items.get(item) else { return };
        let row = self.menu.row.unwrap_or(0);
        self.close_menu(true);
        match cmd {
            Cmd::Settings => self.show_settings(),
            Cmd::Reindex => self.rebuild_index(),
            Cmd::Quit => unsafe {
                let _ = DestroyWindow(self.hwnd);
            },
            Cmd::Open => self.launch(row, false),
            Cmd::Admin => self.launch(row, true),
            Cmd::Folder => self.reveal(row),
            Cmd::Hide => {
                self.sel = row;
                self.hide_selected();
            }
        }
        self.invalidate();
    }

    fn draw_menu(&mut self, g: &mut Gfx) {
        let th = &self.theme;
        let (t1, t2, tint, sel, kbd) = (th.t1, th.t2, th.tint, th.sel, th.kbd);
        let width = self.menu.width;
        let h = self.menu.items.len() as f32 * ITEM + 12.0;
        g.fill(0.0, 0.0, width, h + 2.0, 0.0, tint);
        let r = (self.radius() - 2.0).max(2.0);
        for (i, (label, key, _)) in self.menu.items.iter().enumerate() {
            let y = 6.0 + i as f32 * ITEM;
            if i == self.menu.sel {
                g.fill(6.0, y, width - 12.0, ITEM, r, sel);
            }
            let mut right = width - 16.0;
            if !key.is_empty() {
                let kw = g.measure(key, 12.0) + 12.0;
                g.fill(right - kw, y + 7.0, kw, 22.0, 5.0, kbd);
                g.text(key, right - kw, y + 7.0, kw, 22.0, 12.0, t2, Align::Center);
                right -= kw + 12.0;
            }
            g.text(label, 16.0, y, right - 16.0, ITEM, 14.0, t1, Align::Left);
        }
    }

    fn menu_message(&mut self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let n = self.menu.items.len().max(1);
        match msg {
            WM_PAINT => {
                if self.menu.open
                    && let Some(mut g) = self.gfx.take()
                {
                    g.k = self.menu.k;
                    if g.begin(hwnd).is_ok() {
                        self.draw_menu(&mut g);
                        g.end();
                    }
                    self.gfx = Some(g);
                }
                unsafe {
                    let _ = ValidateRect(Some(hwnd), None);
                }
            }
            WM_ERASEBKGND => return Some(LRESULT(1)),
            WM_MOUSEMOVE | WM_LBUTTONUP => {
                let y = ((lp.0 >> 16) as i16) as f32 / self.menu.k;
                let item = (((y - 6.0) / ITEM).floor().max(0.0) as usize).min(n - 1);
                if msg == WM_LBUTTONUP {
                    self.run_menu(item);
                } else if item != self.menu.sel {
                    self.menu.sel = item;
                    unsafe {
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                }
            }
            WM_KEYDOWN => {
                let vk = VIRTUAL_KEY(wp.0 as u16);
                if vk == VK_ESCAPE {
                    self.close_menu(true);
                } else if vk == VK_RETURN || vk == VK_SPACE {
                    self.run_menu(self.menu.sel);
                } else if vk == VK_DOWN || vk == VK_UP {
                    let step = if vk == VK_DOWN { 1 } else { n - 1 };
                    self.menu.sel = (self.menu.sel + step) % n;
                    unsafe {
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                }
            }
            // 点到别处就收起；点回面板则面板留着
            WM_ACTIVATE if wp.0 & 0xFFFF == WA_INACTIVE as usize => self.close_menu(lp.0 == self.hwnd.0 as isize),
            _ => return None,
        }
        Some(LRESULT(0))
    }
}

unsafe extern "system" fn menuproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match with_app(|app| app.menu_message(hwnd, msg, wp, lp)) {
        Some(r) => r,
        None => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}
