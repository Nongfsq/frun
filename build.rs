use std::{env, fs, path::Path};

fn main() {
    // 把图标和版本信息编进 exe：资源管理器、任务管理器和托盘用图标；文件属性和代码签名用版本信息
    println!("cargo:rerun-if-changed=assets/frun.ico");
    let dir = env::var("CARGO_MANIFEST_DIR").unwrap().replace('\\', "/");
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let [major, minor, patch] = ["MAJOR", "MINOR", "PATCH"].map(|k| env::var(format!("CARGO_PKG_VERSION_{k}")).unwrap());
    let rc = format!(
        r#"1 ICON "{dir}/assets/frun.ico"
1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "Frank Menger"
      VALUE "FileDescription", "frun"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "frun"
      VALUE "LegalCopyright", "Copyright (c) 2026 Frank Menger. MIT License."
      VALUE "OriginalFilename", "frun.exe"
      VALUE "ProductName", "frun"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    );
    let out = Path::new(&env::var("OUT_DIR").unwrap()).join("frun.rc");
    fs::write(&out, rc).unwrap();
    embed_resource::compile(&out, embed_resource::NONE).manifest_optional().unwrap();
}
