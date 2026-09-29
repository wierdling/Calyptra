fn main() {
    // Embed the icon in the .exe so Explorer and the taskbar show it.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=assets/icon.ico");
        embed_resource::compile("assets/icon.rc", embed_resource::NONE)
            .manifest_optional()
            .unwrap();
    }
}
