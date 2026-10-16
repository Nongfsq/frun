use std::{fs, path::Path};

/// Writes a temporary file and renames it over `path`, so another process never reads a half-written file.
pub fn write_atomic(path: &Path, bytes: &[u8]) {
    let tmp = path.with_extension(format!("tmp{:?}", std::thread::current().id()));
    if fs::write(&tmp, bytes).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}
