mod app_state;
mod desktop;
pub mod ipc;
mod overlay;
pub mod safety;
mod settings;
pub mod storage;
use tauri::Manager;

#[tauri::command]
async fn settings_get(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<settings::View> {
    let id = ipc::correlation_id();
    let result = (|| {
        ipc::authorize(&window, true)?;
        let scope: settings::Scope = ipc::parse_request(request)?;
        scope.authorize()?;
        settings::get(&app)
    })();
    complete("settings_get", id, result)
}

#[tauri::command]
async fn settings_update(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<settings::View> {
    let id = ipc::correlation_id();
    let result = (|| {
        ipc::authorize(&window, true)?;
        if serde_json::to_vec(&request)
            .map_err(|_| ipc::ErrorCode::InvalidRequest)?
            .len()
            > 8192
        {
            return Err(ipc::ErrorCode::InputTooLarge);
        }
        let params = ipc::parse_request(request)?;
        settings::update(&app, params, id.clone())
    })();
    complete("settings_update", id, result)
}

#[tauri::command]
async fn overlay_get(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<overlay::OverlayView> {
    let id = ipc::correlation_id();
    let result = (|| {
        ipc::authorize(&window, false)?;
        ipc::parse_request::<ipc::EmptyParams>(request)?;
        Ok(app_state::status(&app, window.label() == "overlay").overlay)
    })();
    complete("overlay_get", id, result)
}

#[tauri::command]
async fn overlay_control(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    request: serde_json::Value,
) -> ipc::Response<()> {
    let id = ipc::correlation_id();
    let result = (|| {
        ipc::authorize(&window, false)?;
        let action = overlay::validate_action(request, window.label() == "overlay")?;
        let shortcut_change = matches!(action, overlay::OverlayAction::SetShortcut { .. });
        overlay::apply_with_id(&app, action, id.clone())?;
        if shortcut_change && overlay::view(&app).shortcut_error == Some("PA_SHORTCUT_CONFLICT") {
            return Err(ipc::ErrorCode::ShortcutConflict);
        }
        Ok(())
    })();
    complete("overlay_control", id, result)
}

fn complete<T: serde::Serialize>(
    command: &'static str,
    id: String,
    result: Result<T, ipc::ErrorCode>,
) -> ipc::Response<T> {
    eprintln!(
        "{}",
        ipc::command_log(command, &id, result.as_ref().err().copied())
    );
    ipc::Response::from_result(id, result)
}

#[tauri::command]
async fn app_get_status(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<app_state::AppStatus> {
    let id = ipc::correlation_id();
    let result = (|| {
        ipc::authorize(&window, false)?;
        ipc::parse_request::<ipc::EmptyParams>(request)?;
        Ok(app_state::status(&app, window.label() == "overlay"))
    })();
    complete("app_get_status", id, result)
}

// Debug fixture is dynamically authorized only during this project's desktop verification.
#[cfg(debug_assertions)]
#[tauri::command]
async fn ipc_fixture(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<()> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Params {
        action: String,
    }
    let id = ipc::correlation_id();
    let result = (|| {
        ipc::authorize(&window, true)?;
        if std::env::var("PA_IPC_VERIFY").as_deref() != Ok("1") {
            return Err(ipc::ErrorCode::PermissionDenied);
        }
        let params: Params = ipc::parse_request(request)?;
        match params.action.as_str() {
            "tray_disable" => overlay::tray_action(&app, "disable"),
            "tray_enable" => overlay::tray_action(&app, "enable"),
            "settings_fail_write" | "settings_restore_write" => {
                let fail = params.action == "settings_fail_write";
                app.state::<storage::StorageState>().with_storage(|storage| {
                    storage.connection.execute_batch(if fail { "CREATE TRIGGER fail_settings_verify BEFORE UPDATE ON resident_preferences BEGIN SELECT RAISE(ABORT,'synthetic-secret-DO-NOT-LOG'); END;" } else { "DROP TRIGGER fail_settings_verify;" }).map_err(|_| storage::StorageError { code: "STORAGE_SQL_FAILED", backup_path: None })
                }).map_err(|_| ipc::ErrorCode::SettingsStorageFailed)
            }
            "create_denied_window" => {
                tauri::WebviewWindowBuilder::new(
                    &app,
                    "ipc-denied",
                    tauri::WebviewUrl::App("index.html?window=denied".into()),
                )
                .visible(false)
                .build()
                .map_err(|_| ipc::ErrorCode::OverlayControlFailed)?;
                Ok(())
            }
            "close_denied_window" => {
                if let Some(window) = app.get_webview_window("ipc-denied") {
                    window
                        .destroy()
                        .map_err(|_| ipc::ErrorCode::OverlayControlFailed)?;
                }
                Ok(())
            }
            "navigate_remote" => {
                let url = std::env::var("PA_IPC_REMOTE_URL")
                    .map_err(|_| ipc::ErrorCode::InvalidRequest)?;
                let url = url.parse().map_err(|_| ipc::ErrorCode::InvalidRequest)?;
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    let _ = window.navigate(url);
                });
                Ok(())
            }
            "destroy_overlay" => {
                if let Some(window) = app.get_webview_window("overlay") {
                    window
                        .destroy()
                        .map_err(|_| ipc::ErrorCode::OverlayControlFailed)?;
                }
                Ok(())
            }
            action => app_state::fixture(&app, action, id.clone()),
        }
    })();
    complete("ipc_fixture", id, result)
}

#[tauri::command]
fn desktop_initialize(
    request: serde_json::Value,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, desktop::DesktopState>,
) -> ipc::Response<desktop::DesktopReady> {
    let id = ipc::correlation_id();
    let result = ipc::authorize(&window, true).and_then(|_| state.initialize(request));
    complete("desktop_initialize", id, result)
}

#[tauri::command]
fn framework_probe(
    request: serde_json::Value,
    state: tauri::State<'_, desktop::DesktopState>,
) -> ipc::Response<ipc::ProbeReply> {
    let id = ipc::correlation_id();
    let result = if state.phase0 {
        ipc::probe(request)
    } else {
        Err(ipc::ErrorCode::InvalidRequest)
    };
    eprintln!("{}", ipc::probe_log(&id, result.as_ref().err().copied()));
    ipc::Response::from_result(id, result)
}

// Phase 0.4 only: preflight the intentionally narrow opener fixture scope.
#[tauri::command]
fn phase0_check_open_path(
    path: &str,
    state: tauri::State<'_, desktop::DesktopState>,
) -> Result<(), String> {
    if !state.phase0 {
        return Err("OPEN_PATH_DENIED".to_string());
    }
    let scope = std::env::temp_dir().join("phase-0-system-open");
    safety::existing_scoped_path(&scope, std::path::Path::new(path))
        .map(|_| ())
        .map_err(|e| {
            match e {
                safety::PathError::Denied => "OPEN_PATH_DENIED",
                safety::PathError::ScopeUnavailable => "OPEN_SCOPE_UNAVAILABLE",
                safety::PathError::Missing => "OPEN_PATH_MISSING",
                safety::PathError::AccessDenied => "OPEN_PATH_ACCESS_DENIED",
                safety::PathError::CheckFailed => "OPEN_PATH_CHECK_FAILED",
            }
            .to_string()
        })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(debug_assertions)]
    if let Some(code) = storage::probe::run_cli() {
        std::process::exit(code);
    }
    let state = desktop::DesktopState::from_environment();
    let phase0 = state.phase0;
    let result = tauri::Builder::default()
        .manage(state)
        .manage(app_state::AppState::default())
        .manage(overlay::OverlayState::default())
        .manage(storage::StorageState::default())
        .manage(settings::SettingsState::default())
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _, event| {
                    if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        let app = app.clone();
                        std::thread::spawn(move || {
                            let _ = overlay::apply(&app, overlay::OverlayAction::Expand);
                        });
                    }
                })
                .build(),
        )
        .setup(move |app| {
            #[cfg(debug_assertions)]
            if std::env::var("PA_IPC_VERIFY").as_deref() == Ok("1") {
                app.add_capability(r#"{"identifier":"ipc-verify","windows":["main"],"permissions":["allow-ipc-fixture"]}"#)?;
            }
            if !phase0 {
                let storage = app.state::<storage::StorageState>();
                let data_dir = app.path().app_data_dir();
                #[cfg(debug_assertions)]
                let data_dir = if std::env::var("PA_IPC_VERIFY").as_deref() == Ok("1") {
                    std::env::var_os("PA_OVERLAY_VERIFY_DIR").filter(|v| !v.is_empty()).map(std::path::PathBuf::from).map(Ok).unwrap_or(data_dir)
                } else { data_dir };
                match data_dir {
                    Ok(data_dir) => {
                        if let Err(failure) = storage.initialize(&data_dir, &ipc::correlation_id()) {
                            eprintln!("PA_STORAGE_STARTUP_FAILED:{}", failure.public_code());
                        }
                        settings::initialize(app.handle(), &data_dir);
                    }
                    Err(_) => {
                        storage.mark_unavailable("PA_STORAGE_DIRECTORY_FAILED");
                        eprintln!("PA_STORAGE_STARTUP_FAILED:PA_STORAGE_DIRECTORY_FAILED");
                    }
                }
            }
            if phase0 {
                app.add_capability(include_str!("../capabilities/phase0.json"))?;
            } else {
                overlay::setup(app.handle())?;
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            let app = window.app_handle();
            let phase0 = app.state::<desktop::DesktopState>().phase0;
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } if !phase0 => {
                    if window.label() == "overlay" {
                        api.prevent_close();
                        let app = app.clone();
                        std::thread::spawn(move || {
                            let _ = overlay::apply(&app, overlay::OverlayAction::Disable);
                        });
                    } else if overlay::view(app).enabled {
                        api.prevent_close();
                        let _ = window.hide();
                    } else {
                        app.exit(0);
                    }
                }
                tauri::WindowEvent::Destroyed if phase0 && window.label() == "main" => app.exit(0),
                tauri::WindowEvent::ScaleFactorChanged { .. } if window.label() == "overlay" => {
                    let app = app.clone();
                    std::thread::spawn(move || {
                        let _ = overlay::apply(&app, overlay::OverlayAction::RepairPosition);
                    });
                }
                tauri::WindowEvent::Moved { .. } if window.label() == "overlay" => {
                    let app = app.clone();
                    std::thread::spawn(move || { let _ = overlay::persist_preferences(&app); });
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            desktop_initialize,
            app_get_status,
            settings_get,
            settings_update,
            overlay_get,
            overlay_control,
            #[cfg(debug_assertions)]
            ipc_fixture,
            framework_probe,
            phase0_check_open_path
        ])
        .run(tauri::generate_context!());
    if result.is_err() {
        eprintln!("PA_INTERNAL_STARTUP_FAILED");
        std::process::exit(1);
    }
}
