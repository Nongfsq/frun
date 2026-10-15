//! 自动更新：`frun --update <hwnd>`。
//!
//! 主进程每天让它跑一次，用完即退，不占常驻内存。它问发布页有没有新版本，有就下载新的 frun.exe，
//! 用发布时附上的 SHA-256 核对，放进缓存目录，再告诉主进程。主进程在面板收起时换上新版本并重启。
//! 发布页取自 Cargo.toml 的 `repository`（GitHub 仓库）；没填时不检查。`FRUN_UPDATE_FEED` 可以换成别的地址，测试用。
use crate::{config, scan};
use std::{fs, path::PathBuf};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        Networking::WinHttp::*,
        Security::Cryptography::{BCRYPT_SHA256_ALG_HANDLE, BCryptHash},
        UI::WindowsAndMessaging::PostMessageW,
    },
    core::{HSTRING, PCWSTR},
};

/// 新版本已经下载并核对好：主进程收到后在合适的时候换上。
pub const WM_UPDATE: u32 = 0x8000 + 12;
/// 两次检查至少隔多久
pub const EVERY: u64 = 24 * 3600;

/// 发布页的地址；没有时不检查更新。
pub fn feed() -> Option<String> {
    if let Ok(feed) = std::env::var("FRUN_UPDATE_FEED") {
        return Some(feed);
    }
    let repo = env!("CARGO_PKG_REPOSITORY").strip_prefix("https://github.com/")?.trim_end_matches('/');
    (!repo.is_empty()).then(|| format!("https://api.github.com/repos/{repo}/releases/latest"))
}

fn dir() -> PathBuf {
    config::cache_dir().join("update")
}

/// 上次检查的时间（秒）。
pub fn last_check() -> u64 {
    fs::read_to_string(dir().join("checked")).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0)
}

/// 已经下载好、等着换上的新版本：(版本号, 文件)。
pub fn ready() -> Option<(String, PathBuf)> {
    let text = fs::read_to_string(dir().join("ready")).ok()?;
    let (version, file) = text.trim().split_once('\t')?;
    let file = PathBuf::from(file);
    (newer(version, env!("CARGO_PKG_VERSION")) && file.is_file()).then(|| (version.to_string(), file))
}

/// 换上之后清掉下载的文件和旧版本。
pub fn clean() {
    let _ = fs::remove_dir_all(dir().join("files"));
    let _ = fs::remove_file(dir().join("ready"));
    // 旧版本刚退出时它的文件可能还没放开：在后台多试几次
    if let Ok(exe) = std::env::current_exe() {
        let old = exe.with_file_name("frun.old.exe");
        std::thread::spawn(move || {
            for _ in 0..50 {
                if fs::remove_file(&old).is_ok() || !old.exists() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        });
    }
}

/// `a` 是否比 `b` 新。版本号是 0.2.0 这样的数字，可以带 v 前缀。
pub fn newer(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> Vec<u32> { v.trim().trim_start_matches('v').split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    parse(a) > parse(b)
}

pub fn run(args: &[String]) {
    let hwnd = args.first().and_then(|a| a.parse::<isize>().ok()).unwrap_or(0);
    let _ = fs::create_dir_all(dir().join("files"));
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let _ = fs::write(dir().join("checked"), now.to_string());
    match check() {
        Ok(Some(version)) => {
            scan::log(&format!("update {version} downloaded"));
            if hwnd != 0 {
                unsafe {
                    let _ = PostMessageW(Some(HWND(hwnd as _)), WM_UPDATE, WPARAM(0), LPARAM(0));
                }
            }
        }
        Ok(None) => {}
        Err(e) => scan::log(&format!("update check failed: {e}")),
    }
}

/// 有新版本就下载并核对，返回它的版本号。
fn check() -> Result<Option<String>, String> {
    let Some(feed) = feed() else { return Ok(None) };
    let json = String::from_utf8_lossy(&get(&feed)?).into_owned();
    let version = strings(&json, "tag_name").into_iter().next().ok_or("no tag_name")?;
    if !newer(&version, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    let urls = strings(&json, "browser_download_url");
    let exe = urls.iter().find(|u| u.ends_with("/frun.exe")).ok_or("no frun.exe in the release")?;
    let sum = urls.iter().find(|u| u.ends_with("/frun.exe.sha256")).ok_or("no frun.exe.sha256 in the release")?;
    let expected = String::from_utf8_lossy(&get(sum)?).split_whitespace().next().unwrap_or("").to_lowercase();
    let bytes = get(exe)?;
    if sha256(&bytes) != expected {
        return Err("checksum mismatch".into());
    }
    let file = dir().join("files").join(format!("frun-{}.exe", version.trim_start_matches('v')));
    fs::write(&file, &bytes).map_err(|e| e.to_string())?;
    fs::write(dir().join("ready"), format!("{version}\t{}", file.display())).map_err(|e| e.to_string())?;
    Ok(Some(version))
}

/// JSON 里某个键的所有字符串值。发布页的结构固定，不值得为它带一个 JSON 库。
fn strings(json: &str, key: &str) -> Vec<String> {
    let pat = format!("\"{key}\"");
    let mut out = Vec::new();
    let mut rest = json;
    while let Some(i) = rest.find(&pat) {
        rest = rest[i + pat.len()..].trim_start();
        let Some(after) = rest.strip_prefix(':') else { continue };
        let Some(value) = after.trim_start().strip_prefix('"') else { continue };
        let end = value.find('"').unwrap_or(value.len());
        out.push(value[..end].replace("\\/", "/"));
        rest = &value[end..];
    }
    out
}

fn sha256(bytes: &[u8]) -> String {
    let mut out = [0u8; 32];
    let status = unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, bytes, &mut out) };
    if status.is_err() {
        return String::new();
    }
    out.iter().map(|b| format!("{b:02x}")).collect()
}

/// 用系统的 WinHTTP 发一个 GET，跟随跳转，返回 200 的内容。
fn get(url: &str) -> Result<Vec<u8>, String> {
    let (secure, rest) = match url.split_once("://") {
        Some(("https", rest)) => (true, rest),
        Some(("http", rest)) => (false, rest),
        _ => return Err(format!("bad url {url}")),
    };
    let (host, path) = rest.split_once('/').map_or((rest, "/".to_string()), |(h, p)| (h, format!("/{p}")));
    let (host, port) = match host.split_once(':') {
        Some((h, p)) => (h, p.parse().unwrap_or(0)),
        None => (host, if secure { 443 } else { 80 }),
    };
    unsafe {
        let agent = HSTRING::from(format!("frun/{}", env!("CARGO_PKG_VERSION")));
        let session = WinHttpOpen(&agent, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0);
        if session.is_null() {
            return Err("WinHttpOpen failed".into());
        }
        let _ = WinHttpSetTimeouts(session, 10_000, 10_000, 30_000, 30_000);
        let result = (|| {
            let connect = WinHttpConnect(session, &HSTRING::from(host), port, 0);
            if connect.is_null() {
                return Err(format!("cannot connect to {host}"));
            }
            let flags = if secure { WINHTTP_FLAG_SECURE } else { WINHTTP_OPEN_REQUEST_FLAGS(0) };
            let request = WinHttpOpenRequest(connect, &HSTRING::from("GET"), &HSTRING::from(path), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), flags);
            let body = (|| {
                if request.is_null() {
                    return Err("cannot open the request".to_string());
                }
                WinHttpSendRequest(request, None, None, 0, 0, 0).map_err(|e| e.to_string())?;
                WinHttpReceiveResponse(request, std::ptr::null_mut()).map_err(|e| e.to_string())?;
                let (mut status, mut size) = (0u32, 4u32);
                WinHttpQueryHeaders(
                    request,
                    WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                    PCWSTR::null(),
                    Some(&mut status as *mut _ as _),
                    &mut size,
                    std::ptr::null_mut(),
                )
                .map_err(|e| e.to_string())?;
                if status != 200 {
                    return Err(format!("HTTP {status} from {host}"));
                }
                let mut body = Vec::new();
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    let mut read = 0u32;
                    WinHttpReadData(request, buf.as_mut_ptr() as _, buf.len() as u32, &mut read).map_err(|e| e.to_string())?;
                    if read == 0 {
                        break;
                    }
                    body.extend_from_slice(&buf[..read as usize]);
                }
                Ok(body)
            })();
            if !request.is_null() {
                let _ = WinHttpCloseHandle(request);
            }
            let _ = WinHttpCloseHandle(connect);
            body
        })();
        let _ = WinHttpCloseHandle(session);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_numbers() {
        assert!(newer("v0.10.0", "0.9.3"));
        assert!(newer("0.2.0", "0.1.9"));
        assert!(!newer("v0.1.0", "0.1.0"));
        assert!(!newer("0.1.0", "0.2.0"));
    }

    #[test]
    fn reads_the_release_fields() {
        let json = r#"{"tag_name": "v0.2.0", "assets": [
            {"name": "frun.exe", "browser_download_url": "https://github.com/a/frun/releases/download/v0.2.0/frun.exe"},
            {"name": "frun.exe.sha256", "browser_download_url":"https://github.com/a/frun/releases/download/v0.2.0/frun.exe.sha256"}]}"#;
        assert_eq!(strings(json, "tag_name"), ["v0.2.0"]);
        assert_eq!(strings(json, "browser_download_url").len(), 2);
    }

    #[test]
    fn hashes_like_sha256sum() {
        assert_eq!(sha256(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
