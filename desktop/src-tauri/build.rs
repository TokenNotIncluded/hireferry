fn main() {
    // Application commands must participate in the capability ACL, too.
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(&["connect", "open_browser"])),
    )
    .expect("failed to build the desktop application");
}
