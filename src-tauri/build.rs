fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "desktop_initialize",
            "overlay_get",
            "overlay_control",
            "framework_probe",
            "phase0_check_open_path",
        ]),
    ))
    .expect("Tauri build configuration failed")
}
