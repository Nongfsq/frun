#![windows_subsystem = "windows"]

mod app;
mod config;
mod gfx;
mod i18n;
mod index;
mod menu;
mod platform;
mod scan;
mod search;
mod textfield;
mod view;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--scan") {
        scan::run(&args[1..]);
    } else if args.first().is_some_and(|a| a == "--icons") {
        scan::run_icons(&args[1..]);
    } else if args.first().is_some_and(|a| a == "--pick") {
        pick(args.get(1).and_then(|a| a.parse::<isize>().ok()).unwrap_or(0));
    } else {
        let _ = app::run();
    }
}

/// `frun --pick <hwnd>`：打开文件夹选择框，把选中的路径写到标准输出。
fn pick(owner: isize) {
    use windows::Win32::{Foundation::HWND, System::Com::*};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    if let Some(path) = platform::pick_folder(HWND(owner as _)) {
        print!("{path}");
    }
}
