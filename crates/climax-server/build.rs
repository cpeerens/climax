// Guarantee the SPA build dir exists before rust-embed's #[folder] macro
// validates it, so a bare `cargo check --workspace` compiles on a tree where
// `npm run build` hasn't run yet (the embed is just empty then - the server
// serves 404s for UI assets until a real frontend build lands). Release
// packaging must still run `npm run build` FIRST so real assets get embedded.
fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let build_dir = std::path::Path::new(&manifest).join("../../build");
    let _ = std::fs::create_dir_all(&build_dir);
    // Re-run when the SPA build output changes, so release builds re-embed
    // fresh assets without a manual cargo clean.
    println!("cargo:rerun-if-changed={}", build_dir.display());
}
