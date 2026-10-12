fn main() {
    // 把图标编进 exe：资源管理器、任务管理器和托盘都用它
    println!("cargo:rerun-if-changed=assets/frun.rc");
    println!("cargo:rerun-if-changed=assets/frun.ico");
    embed_resource::compile("assets/frun.rc", embed_resource::NONE).manifest_optional().unwrap();
}
