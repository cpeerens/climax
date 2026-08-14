fn main() {
    // tauri-build only watches tauri.conf.json, so swapping the icon files
    // without touching the config leaves the .exe with the OLD embedded
    // resource (see: pinned taskbar tile reverting to a stale icon). Watch
    // the icon files explicitly so resource.lib regenerates on change.
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=icons/icon.png");
    println!("cargo:rerun-if-changed=icons/32x32.png");
    println!("cargo:rerun-if-changed=icons/128x128.png");
    println!("cargo:rerun-if-changed=icons/128x128@2x.png");
    tauri_build::build()
}
