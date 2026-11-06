fn main() {
    // Unit-test binaries now host real Wry windows too. Tauri embeds this
    // dependency in its app resource, but Cargo's test executable needs its own.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
        );
        // The normal app already has a complete manifest in Tauri's resource
        // library. Do not generate a second resource with the same numeric ID.
        println!("cargo:rustc-link-arg-bin=project-assistant=/MANIFEST:NO");
    }
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "projects_add_from_picker",
            "project_get",
            "project_git_summary",
            "projects_list",
            "project_open_directory",
            "project_open_file",
            "project_pick_file",
            "project_remove",
            "project_select",
            "desktop_initialize",
            "app_get_status",
            "settings_get",
            "settings_update",
            "project_settings_update",
            "session_create",
            "session_find_by_request",
            "sessions_list",
            "session_get",
            "session_resume",
            "session_end",
            "task_start",
            "task_find_by_request",
            "tasks_list",
            "task_get",
            "task_cancel",
            "task_events_get",
            "approvals_get",
            "approval_decide",
            "task_content_get",
            "task_changes_get",
            "task_change_get",
            "active_context_set",
            "task_start_from_active",
            "overlay_task_get",
            "overlay_task_find_by_request",
            "overlay_task_cancel",
            "logs_get_status",
            "logs_clear",
            "providers_list",
            "provider_save",
            "provider_set_enabled",
            "provider_delete",
            "provider_models",
            "provider_test",
            "provider_switch",
            "operation_get",
            "operation_cancel",
            "credential_put",
            "credential_remove",
            "overlay_get",
            "overlay_control",
            "framework_probe",
            "phase0_check_open_path",
            "ipc_fixture",
        ]),
    ))
    .expect("Tauri build configuration failed")
}
