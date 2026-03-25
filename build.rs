fn main() {
    winfsp::build::winfsp_link_delayload();

    // Copy CFS_Drive.ico next to the built binary so resolve_cfs_icon() finds it.
    // OUT_DIR is e.g. target/debug/build/<pkg>/out — walk up 3 levels to get target/debug.
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let profile_dir = std::path::PathBuf::from(&out_dir)
        .ancestors()
        .nth(3)
        .unwrap()
        .to_owned();

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let icon_src = std::path::PathBuf::from(&manifest_dir)
        .join("cfs-ui")
        .join("src-tauri")
        .join("icons")
        .join("CFS_Drive.ico");

    if icon_src.exists() {
        let icon_dst = profile_dir.join("CFS_Drive.ico");
        let _ = std::fs::copy(&icon_src, &icon_dst);
    }

    // Re-run if the icon changes
    println!("cargo:rerun-if-changed=cfs-ui/src-tauri/icons/CFS_Drive.ico");
}
