fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "desktop_initialize",
            "app_get_status",
            "settings_get",
            "settings_update",
            "logs_get_status",
            "logs_clear",
            "overlay_get",
            "overlay_control",
            "framework_probe",
            "phase0_check_open_path",
            "ipc_fixture",
        ]),
    ))
    .expect("Tauri build configuration failed")
}
