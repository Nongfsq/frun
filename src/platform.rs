//! 与系统打交道的零散部分：主题、强调色、自启、托盘图标、目录监视、拖放、文件夹选择。
use crate::gfx::Color;
use std::{mem::size_of, os::windows::process::CommandExt, process::Command};
use windows::{
    Win32::{
        Foundation::*,
        Globalization::GetUserDefaultUILanguage,
        Graphics::Gdi::*,
        Storage::FileSystem::*,
        System::{
            Com::*,
            IO::CancelIoEx,
            Ole::*,
            Registry::*,
            SystemServices::MODIFIERKEYS_FLAGS,
        },
        UI::{
            HiDpi::*,
            Input::{Ime::*, KeyboardAndMouse::GetKeyState},
            Shell::*,
            WindowsAndMessaging::*,
        },
    },
    core::*,
};

pub const WM_DIRTY: u32 = WM_APP + 4;
pub const WM_DROP_HOVER: u32 = WM_APP + 5;
pub const WM_DROP_PATH: u32 = WM_APP + 6;
pub const WM_SCAN_DONE: u32 = WM_APP + 7;
pub const WM_PICKED: u32 = WM_APP + 9;

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");

pub fn key_down(vk: u32) -> bool {
    unsafe { GetKeyState(vk as i32) < 0 }
}

pub fn system_is_chinese() -> bool {
    unsafe { GetUserDefaultUILanguage() & 0x3FF == 0x04 }
}

pub fn system_light_theme() -> bool {
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

/// 系统强调色：深色界面用色板里的浅二档，浅色界面用深一档，与系统控件一致。
pub fn system_accent(dark: bool) -> Option<Color> {
    let (mut palette, mut size) = ([0u8; 32], 32u32);
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent"),
            w!("AccentPalette"),
            RRF_RT_REG_BINARY,
            None,
            Some(palette.as_mut_ptr() as _),
            Some(&mut size),
        )
        .is_ok()
    };
    let at = if dark { 4 } else { 16 };
    (ok && size == 32).then(|| {
        Color(palette[at] as f32 / 255.0, palette[at + 1] as f32 / 255.0, palette[at + 2] as f32 / 255.0, 1.0)
    })
}

/// 系统是否关闭了窗口内动画（设置 > 辅助功能 > 视觉效果 > 动画效果）。
pub fn system_reduced_motion() -> bool {
    let mut on = BOOL(1);
    unsafe {
        let _ = SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(&mut on as *mut _ as _), Default::default());
    }
    !on.as_bool()
}

pub fn autostart() -> bool {
    unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, w!("frun"), RRF_RT_REG_SZ, None, None, None).is_ok() }
}

pub fn set_autostart(on: bool) {
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

/// 面板要出现的显示器：工作区和缩放比例。
pub fn monitor(primary: bool) -> (RECT, f32) {
    unsafe {
        let mut pt = POINT::default();
        if !primary {
            let _ = GetCursorPos(&mut pt);
        }
        let mon = MonitorFromPoint(pt, if primary { MONITOR_DEFAULTTOPRIMARY } else { MONITOR_DEFAULTTONEAREST });
        let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        (mi.rcWork, dx as f32 / 96.0)
    }
}

/// 把输入法切到英文模式（用户仍可随时按 Shift 切回）。
pub fn ime_english(hwnd: HWND) {
    unsafe {
        let ime = ImmGetDefaultIMEWnd(hwnd);
        if !ime.is_invalid() {
            SendMessageW(ime, WM_IME_CONTROL, Some(WPARAM(IMC_SETCONVERSIONMODE as usize)), Some(LPARAM(0)));
        }
    }
}

/// 让输入法的组字窗口跟着光标（物理像素坐标）。
pub fn ime_follow(hwnd: HWND, x: i32, y: i32) {
    unsafe {
        let himc = ImmGetContext(hwnd);
        if !himc.is_invalid() {
            let form = COMPOSITIONFORM { dwStyle: CFS_POINT, ptCurrentPos: POINT { x, y }, ..Default::default() };
            let _ = ImmSetCompositionWindow(himc, &form);
            let _ = ImmReleaseContext(hwnd, himc);
        }
    }
}

/// 启动扫描进程；结束时向窗口发 WM_SCAN_DONE。
pub fn spawn_scan(hwnd: HWND, icon_px: u32, force: bool, only: &[String]) -> bool {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let Ok(exe) = std::env::current_exe() else { return false };
    let mut cmd = Command::new(exe);
    cmd.arg("--scan").arg((hwnd.0 as isize).to_string()).arg(icon_px.to_string());
    if force {
        cmd.arg("force");
    }
    cmd.args(only);
    let Ok(mut child) = cmd.creation_flags(CREATE_NO_WINDOW).spawn() else { return false };
    let hwnd = hwnd.0 as isize;
    std::thread::spawn(move || {
        let _ = child.wait();
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as _)), WM_SCAN_DONE, WPARAM(0), LPARAM(0));
        }
    });
    true
}

/// 启动一个只取指定图标的子进程，取完向窗口发 WM_ICONS。
pub fn spawn_icons(hwnd: HWND, icon_px: u32, targets: &[String]) {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    if let Ok(exe) = std::env::current_exe() {
        let _ = Command::new(exe)
            .arg("--icons")
            .arg((hwnd.0 as isize).to_string())
            .arg(icon_px.to_string())
            .args(targets)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }
}

/// 目录监视：目录里增删文件时向窗口发 WM_DIRTY，由主线程合并后触发一次增量扫描。
#[derive(Default)]
pub struct Watchers {
    handles: Vec<isize>,
}

impl Watchers {
    pub fn restart(&mut self, dirs: &[String], exts: &[String], hwnd: HWND) {
        self.stop();
        // (目录, 关心的扩展名)
        let mut all: Vec<(String, Vec<String>)> = dirs.iter().map(|d| (d.clone(), exts.to_vec())).collect();
        // 开始菜单的两个位置：装了新的桌面程序会在这里放快捷方式
        for var in ["APPDATA", "ProgramData"] {
            if let Ok(base) = std::env::var(var) {
                all.push((format!(r"{base}\Microsoft\Windows\Start Menu\Programs"), vec!["lnk".into()]));
            }
        }
        for (dir, exts) in all {
            let handle = unsafe {
                CreateFileW(
                    &HSTRING::from(&dir),
                    FILE_LIST_DIRECTORY.0,
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    None,
                    OPEN_EXISTING,
                    FILE_FLAG_BACKUP_SEMANTICS,
                    None,
                )
            };
            let Ok(handle) = handle else { continue };
            let (raw, target) = (handle.0 as isize, hwnd.0 as isize);
            self.handles.push(raw);
            std::thread::spawn(move || unsafe {
                let handle = HANDLE(raw as _);
                let mut buf = [0u32; 1024];
                let mut got = 0u32;
                while ReadDirectoryChangesW(
                    handle,
                    buf.as_mut_ptr() as _,
                    size_of_val(&buf) as u32,
                    true,
                    FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME,
                    Some(&mut got),
                    None,
                    None,
                )
                .is_ok()
                {
                    // 整个盘被加进来时变化非常频繁；只有会影响索引的文件才值得触发扫描
                    // lParam 是装箱的路径：哪个子目录变了；通知溢出时为空，表示整个重查
                    if got == 0 {
                        let _ = PostMessageW(Some(HWND(target as _)), WM_DIRTY, WPARAM(0), LPARAM(0));
                    } else if let Some(changed) = relevant(&buf, got as usize, &exts, &dir) {
                        let boxed = Box::into_raw(Box::new(changed)) as isize;
                        let _ = PostMessageW(Some(HWND(target as _)), WM_DIRTY, WPARAM(0), LPARAM(boxed));
                    }
                }
                let _ = CloseHandle(handle);
            });
        }
    }

    fn stop(&mut self) {
        for h in self.handles.drain(..) {
            // 打断阻塞中的读取，监视线程随即退出并关闭句柄
            unsafe {
                let _ = CancelIoEx(HANDLE(h as _), None);
            }
        }
    }
}

/// 一批目录变化通知里，有没有扩展名在索引范围内、又不在被跳过的目录里的文件。
/// 有的话返回它所在的目录。
fn relevant(buf: &[u32], len: usize, exts: &[String], root: &str) -> Option<String> {
    let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, len.min(size_of_val(buf))) };
    let mut at = 0;
    loop {
        // FILE_NOTIFY_INFORMATION：下一条的偏移、动作、文件名字节数、文件名
        let head = bytes.get(at..at + 12)?;
        let field = |i: usize| u32::from_le_bytes(head[i..i + 4].try_into().unwrap()) as usize;
        let (next, name_len) = (field(0), field(8));
        let raw = bytes.get(at + 12..at + 12 + name_len)?;
        let name: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).collect();
        let name = String::from_utf16_lossy(&name);
        let ext = name.rsplit_once('.').map_or(String::new(), |(_, e)| e.to_lowercase());
        if exts.contains(&ext) && !skipped(root, &name) {
            let file = std::path::Path::new(root).join(&name);
            return file.parent().map(|d| d.to_string_lossy().into_owned());
        }
        if next == 0 {
            return None;
        }
        at += next;
    }
}

/// 扫描不会进去的目录（依赖、缓存、隐藏目录）里的变化不算数，否则编译一次就要重扫一次。
fn skipped(root: &str, name: &str) -> bool {
    let mut dir = std::path::PathBuf::from(root);
    let mut parts: Vec<&str> = name.split('\\').collect();
    parts.pop();
    parts.into_iter().any(|part| {
        dir.push(part);
        crate::scan::skip_dir(part) || dir.join(crate::scan::CACHE_TAG).exists()
    })
}

/// 在子进程里打开文件夹选择框，选完向窗口发 WM_PICKED（lParam 是装箱的 Option<String>）。
///
/// 选择框会把资源管理器的一大堆组件载入所在进程（上百 MB、几十个线程，且不再释放），
/// 所以和扫描一样放在用完即退的子进程里。
pub fn spawn_pick(hwnd: HWND) -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let raw = hwnd.0 as isize;
    let Ok(child) = Command::new(exe).arg("--pick").arg(raw.to_string()).stdout(std::process::Stdio::piped()).spawn()
    else {
        return false;
    };
    std::thread::spawn(move || {
        let path = child
            .wait_with_output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|p| !p.is_empty());
        let boxed = Box::into_raw(Box::new(path));
        unsafe {
            let _ = PostMessageW(Some(HWND(raw as _)), WM_PICKED, WPARAM(0), LPARAM(boxed as isize));
        }
    });
    true
}

/// 系统的文件夹选择框（在 `--pick` 子进程里调用）。
pub fn pick_folder(owner: HWND) -> Option<String> {
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let options = dialog.GetOptions().ok()?;
        dialog.SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM).ok()?;
        dialog.Show(Some(owner)).ok()?;
        let path = dialog.GetResult().ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = path.to_string().ok();
        CoTaskMemFree(Some(path.0 as _));
        s
    }
}

/// 接收从资源管理器拖来的文件夹。
#[implement(IDropTarget)]
struct DropTarget {
    hwnd: isize,
}

impl DropTarget {
    fn hover(&self, on: bool) {
        unsafe {
            let _ = PostMessageW(Some(HWND(self.hwnd as _)), WM_DROP_HOVER, WPARAM(on as usize), LPARAM(0));
        }
    }
}

/// 拖动的内容里第一个文件夹的路径。
unsafe fn dropped_folder(data: &IDataObject) -> Option<String> {
    unsafe {
        let format = FORMATETC {
            cfFormat: CF_HDROP.0,
            ptd: std::ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT.0,
            lindex: -1,
            tymed: TYMED_HGLOBAL.0 as u32,
        };
        let mut medium = data.GetData(&format).ok()?;
        let drop = HDROP(medium.u.hGlobal.0);
        let mut found = None;
        for i in 0..DragQueryFileW(drop, u32::MAX, None) {
            let mut buf = [0u16; 520];
            let len = DragQueryFileW(drop, i, Some(&mut buf)) as usize;
            let path = String::from_utf16_lossy(&buf[..len]);
            if std::path::Path::new(&path).is_dir() {
                found = Some(path);
                break;
            }
        }
        ReleaseStgMedium(&mut medium);
        found
    }
}

impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(
        &self,
        data: Ref<IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> Result<()> {
        let ok = data.as_ref().is_some_and(|d| unsafe { dropped_folder(d).is_some() });
        self.hover(ok);
        unsafe { *effect = if ok { DROPEFFECT_LINK } else { DROPEFFECT_NONE } };
        Ok(())
    }

    fn DragOver(&self, _keys: MODIFIERKEYS_FLAGS, _pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe { *effect = DROPEFFECT_LINK };
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        self.hover(false);
        Ok(())
    }

    fn Drop(&self, data: Ref<IDataObject>, _keys: MODIFIERKEYS_FLAGS, _pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        self.hover(false);
        if let Some(path) = data.as_ref().and_then(|d| unsafe { dropped_folder(d) }) {
            let boxed = Box::into_raw(Box::new(path));
            unsafe {
                let _ = PostMessageW(Some(HWND(self.hwnd as _)), WM_DROP_PATH, WPARAM(0), LPARAM(boxed as isize));
                *effect = DROPEFFECT_LINK;
            }
        }
        Ok(())
    }
}

pub fn register_drop(hwnd: HWND) {
    let target: IDropTarget = DropTarget { hwnd: hwnd.0 as isize }.into();
    unsafe {
        let _ = RegisterDragDrop(hwnd, &target);
    }
}

/// 托盘图标：编进 exe 的那个图标，按托盘的实际像素取。
pub fn tray_icon() -> HICON {
    unsafe {
        let inst = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default();
        let size = GetSystemMetrics(SM_CXSMICON);
        // 资源编号 1，见 build.rs
        LoadImageW(Some(inst.into()), PCWSTR(1 as _), IMAGE_ICON, size, size, LR_DEFAULTCOLOR)
            .map(|h| HICON(h.0))
            .unwrap_or_else(|_| LoadIconW(None, IDI_APPLICATION).unwrap_or_default())
    }
}
