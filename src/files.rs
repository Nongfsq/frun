use std::{fs, io::Write, path::Path};

/// Writes a temporary file and renames it over `path`, so another process never reads a half-written file.
pub fn write_atomic(path: &Path, bytes: &[u8]) {
    let tmp = tmp_path(path);
    if fs::write(&tmp, bytes).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}

/// `write_atomic` that also flushes to disk before the rename, so a power cut leaves either the old file or
/// the new one. The flush costs a few milliseconds per file; keep it off paths that run on every launch.
pub fn write_durable(path: &Path, bytes: &[u8]) {
    let tmp = tmp_path(path);
    let written = fs::File::create(&tmp).and_then(|mut f| {
        f.write_all(bytes)?;
        f.sync_all()
    });
    if written.and_then(|()| fs::rename(&tmp, path)).is_err() {
        let _ = fs::remove_file(&tmp);
    }
}

fn tmp_path(path: &Path) -> std::path::PathBuf {
    path.with_extension(format!("tmp{:?}", std::thread::current().id()))
}
