fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "request",
            "select_restic_executable",
            "export_repository_password",
            "get_desktop_settings",
            "save_desktop_settings",
            "quit",
        ]),
    ))
    .expect("Could not build the desktop command capability manifest");
}
