//! 测试用的临时目录：建在 target/test-tmp/ 下，和代码在同一块盘，不用系统的 %TEMP%（在 C: 盘）。
//! 离开作用域就删掉，测试中途失败也一样。

use std::fs;
use std::ops::Deref;
use std::path::{Path, PathBuf};

pub struct TestDir(PathBuf);

impl TestDir {
    /// `name` 在同时跑的测试之间要唯一
    pub fn new(name: &str) -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        TestDir(root)
    }
}

impl Deref for TestDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
