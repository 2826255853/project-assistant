mod desktop;
pub mod ipc;
mod overlay;
pub mod safety;
pub mod storage;
use tauri::Manager;

#[tauri::command]
fn overlay_get(app: tauri::AppHandle, window: tauri::WebviewWindow) -> overlay::OverlayView {
    let mut view = overlay::view(&app);
    if window.label() != "overlay" {
        view.draft.clear();
    }
    view
}

#[tauri::command]
async fn overlay_control(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    action: serde_json::Value,
) -> Result<(), &'static str> {
    let action: overlay::OverlayAction =
        serde_json::from_value(action).map_err(|_| "PA_OVERLAY_INVALID_REQUEST")?;
    if window.label() == "overlay"
        && matches!(
            action,
            overlay::OverlayAction::Exit | overlay::OverlayAction::SetShortcut { .. }
        )
    {
        return Err("PA_OVERLAY_PERMISSION_DENIED");
    }
    overlay::apply(&app, action)
}

#[tauri::command]
fn desktop_initialize(
    request: serde_json::Value,
    state: tauri::State<'_, desktop::DesktopState>,
) -> ipc::Response<desktop::DesktopReady> {
    let result = state.initialize(request);
    if let Err(code) = &result {
        eprintln!(
            "{}",
            serde_json::to_string(&code).expect("fixed error code")
        );
    }
    ipc::Response::from_result(ipc::correlation_id(), result)
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
        .manage(overlay::OverlayState::default())
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
                    let _ = overlay::persist_preferences(&app);
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            desktop_initialize,
            overlay_get,
            overlay_control,
            framework_probe,
            phase0_check_open_path
        ])
        .run(tauri::generate_context!());
    if result.is_err() {
        eprintln!("PA_INTERNAL_STARTUP_FAILED");
        std::process::exit(1);
    }
}
