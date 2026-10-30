mod app_state;
mod credentials;
mod desktop;
pub mod ipc;
mod logging;
mod overlay;
mod owned_process;
pub mod project_contract;
mod project_git;
pub mod project_paths;
mod projects;
mod providers;
pub mod safety;
mod settings;
pub mod storage;
use tauri::Manager;

fn project_request<T: serde::de::DeserializeOwned>(
    window: &tauri::WebviewWindow,
    request: serde_json::Value,
) -> Result<T, ipc::ErrorCode> {
    ipc::authorize(window, true)?;
    if serde_json::to_vec(&request)
        .map_err(|_| ipc::ErrorCode::InvalidRequest)?
        .len()
        > 8192
    {
        return Err(ipc::ErrorCode::InputTooLarge);
    }
    ipc::parse_request(request)
}

#[tauri::command]
async fn projects_add_from_picker(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<projects::PickerResult> {
    let id = ipc::correlation_id();
    let result = match project_request::<ipc::EmptyParams>(&window, request) {
        Ok(_) => {
            #[cfg(windows)]
            let owner = window
                .hwnd()
                .map(|h| h.0 as isize)
                .map_err(|_| ipc::ErrorCode::ProjectPickerFailed);
            #[cfg(not(windows))]
            let owner: Result<isize, ipc::ErrorCode> = Err(ipc::ErrorCode::ProjectPickerFailed);
            match owner {
                Ok(owner) => {
                    let a = app.clone();
                    let i = id.clone();
                    tauri::async_runtime::spawn_blocking(move || projects::add(&a, owner, &i))
                        .await
                        .unwrap_or(Err(ipc::ErrorCode::InternalUnexpected))
                }
                Err(e) => Err(e),
            }
        }
        Err(e) => Err(e),
    };
    complete(&app, "projects_add_from_picker", id, result)
}

#[tauri::command]
async fn project_open_directory(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<projects::OpenResult> {
    let id = ipc::correlation_id();
    let parsed =
        project_request::<project_contract::ProjectIdRequest>(&window, request).and_then(|p| {
            p.validate()?;
            Ok(p)
        });
    let result = match parsed {
        Ok(p) => {
            let a = app.clone();
            let i = id.clone();
            tauri::async_runtime::spawn_blocking(move || {
                projects::open(&a, &p.project_id, None, &i)
            })
            .await
            .unwrap_or(Err(ipc::ErrorCode::InternalUnexpected))
        }
        Err(e) => Err(e),
    };
    complete(&app, "project_open_directory", id, result)
}

#[tauri::command]
async fn project_open_file(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<projects::OpenResult> {
    let id = ipc::correlation_id();
    let parsed =
        project_request::<project_contract::OpenFileRequest>(&window, request).and_then(|p| {
            p.validate()?;
            Ok(p)
        });
    let result = match parsed {
        Ok(p) => {
            let a = app.clone();
            let i = id.clone();
            tauri::async_runtime::spawn_blocking(move || {
                projects::open(&a, &p.project_id, Some(&p.relative_path), &i)
            })
            .await
            .unwrap_or(Err(ipc::ErrorCode::InternalUnexpected))
        }
        Err(e) => Err(e),
    };
    complete(&app, "project_open_file", id, result)
}

#[tauri::command]
async fn project_pick_file(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<projects::FilePickerResult> {
    let id = ipc::correlation_id();
    let parsed =
        project_request::<project_contract::ProjectIdRequest>(&window, request).and_then(|p| {
            p.validate()?;
            Ok(p)
        });
    let result = match parsed {
        Ok(p) => {
            #[cfg(windows)]
            let owner = window
                .hwnd()
                .map(|h| h.0 as isize)
                .map_err(|_| ipc::ErrorCode::ProjectPickerFailed);
            #[cfg(not(windows))]
            let owner = Err::<isize, _>(ipc::ErrorCode::ProjectPickerFailed);
            match owner {
                Ok(owner) => {
                    let a = app.clone();
                    let i = id.clone();
                    tauri::async_runtime::spawn_blocking(move || {
                        projects::pick_file(&a, owner, &p.project_id, &i)
                    })
                    .await
                    .unwrap_or(Err(ipc::ErrorCode::InternalUnexpected))
                }
                Err(e) => Err(e),
            }
        }
        Err(e) => Err(e),
    };
    complete(&app, "project_pick_file", id, result)
}

#[tauri::command]
async fn projects_list(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<project_contract::ProjectListPage> {
    let id = ipc::correlation_id();
    let parsed = project_request::<project_contract::ListRequest>(&window, request).and_then(|p| {
        p.validate()?;
        Ok(p)
    });
    let result = match parsed {
        Ok(p) => {
            let a = app.clone();
            tauri::async_runtime::spawn_blocking(move || projects::list(&a, &p))
                .await
                .unwrap_or(Err(ipc::ErrorCode::InternalUnexpected))
        }
        Err(e) => Err(e),
    };
    complete(&app, "projects_list", id, result)
}

#[tauri::command]
async fn project_get(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<project_contract::ProjectView> {
    let id = ipc::correlation_id();
    let parsed =
        project_request::<project_contract::ProjectIdRequest>(&window, request).and_then(|p| {
            p.validate()?;
            Ok(p)
        });
    let result = match parsed {
        Ok(p) => {
            let a = app.clone();
            let i = id.clone();
            tauri::async_runtime::spawn_blocking(move || projects::get(&a, &p.project_id, &i))
                .await
                .unwrap_or(Err(ipc::ErrorCode::InternalUnexpected))
        }
        Err(e) => Err(e),
    };
    complete(&app, "project_get", id, result)
}

#[tauri::command]
async fn project_git_summary(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<project_contract::GitSummary> {
    let id = ipc::correlation_id();
    let parsed =
        project_request::<project_contract::GitSummaryRequest>(&window, request).and_then(|p| {
            p.validate()?;
            Ok(p)
        });
    let result = match parsed {
        Ok(p) => {
            let a = app.clone();
            let i = id.clone();
            tauri::async_runtime::spawn_blocking(move || {
                projects::git_summary(&a, &p.project_id, p.refresh.unwrap_or(false), &i)
            })
            .await
            .unwrap_or(Err(ipc::ErrorCode::InternalUnexpected))
        }
        Err(e) => Err(e),
    };
    complete(&app, "project_git_summary", id, result)
}

#[tauri::command]
async fn project_remove(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<project_contract::RemoveResult> {
    let id = ipc::correlation_id();
    let parsed =
        project_request::<project_contract::RemoveRequest>(&window, request).and_then(|p| {
            p.validate()?;
            Ok(p)
        });
    let result = match parsed {
        Ok(p) => {
            let a = app.clone();
            let i = id.clone();
            tauri::async_runtime::spawn_blocking(move || {
                projects::remove(&a, &p.project_id, p.expected_revision, &i)
            })
            .await
            .unwrap_or(Err(ipc::ErrorCode::InternalUnexpected))
        }
        Err(e) => Err(e),
    };
    complete(&app, "project_remove", id, result)
}

#[tauri::command]
async fn project_select(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<serde_json::Value> {
    let id = ipc::correlation_id();
    let result =
        project_request::<project_contract::ProjectIdRequest>(&window, request).and_then(|p| {
            p.validate()?;
            let active = projects::select(&app, &p.project_id, &id)?;
            serde_json::to_value(serde_json::json!({"activeProject": active}))
                .map_err(|_| ipc::ErrorCode::InternalUnexpected)
        });
    complete(&app, "project_select", id, result)
}

#[tauri::command]
async fn providers_list(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<providers::ProviderList> {
    let id = ipc::correlation_id();
    let parsed = (|| {
        ipc::authorize(&window, true)?;
        if serde_json::to_vec(&request)
            .map_err(|_| ipc::ErrorCode::InvalidRequest)?
            .len()
            > 8192
        {
            return Err(ipc::ErrorCode::InputTooLarge);
        }
        ipc::parse_request::<ipc::EmptyParams>(request)
    })();
    let result = match parsed {
        Ok(p) => {
            let a = app.clone();
            let i = id.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let _ = (&p, &i);
                providers::list(&a)
            })
            .await
            .unwrap_or(Err(ipc::ErrorCode::InternalUnexpected))
        }
        Err(e) => Err(e),
    };
    complete(&app, "providers_list", id, result)
}

#[tauri::command]
async fn provider_save(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<providers::Profile> {
    let id = ipc::correlation_id();
    let parsed = (|| {
        ipc::authorize(&window, true)?;
        if serde_json::to_vec(&request)
            .map_err(|_| ipc::ErrorCode::InvalidRequest)?
            .len()
            > 8192
        {
            return Err(ipc::ErrorCode::InputTooLarge);
        }
        ipc::parse_request::<providers::Save>(request)
    })();
    let result = match parsed {
        Ok(p) => {
            let a = app.clone();
            let i = id.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let _ = (&p, &i);
                providers::save(&a, p, &i)
            })
            .await
            .unwrap_or(Err(ipc::ErrorCode::InternalUnexpected))
        }
        Err(e) => Err(e),
    };
    complete(&app, "provider_save", id, result)
}

#[tauri::command]
async fn provider_set_enabled(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<providers::Profile> {
    let id = ipc::correlation_id();
    let parsed = (|| {
        ipc::authorize(&window, true)?;
        if serde_json::to_vec(&request)
            .map_err(|_| ipc::ErrorCode::InvalidRequest)?
            .len()
            > 8192
        {
            return Err(ipc::ErrorCode::InputTooLarge);
        }
        ipc::parse_request::<providers::Enabled>(request)
    })();
    let result = match parsed {
        Ok(p) => {
            let a = app.clone();
            let i = id.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let _ = (&p, &i);
                providers::set_enabled(&a, p, &i)
            })
            .await
            .unwrap_or(Err(ipc::ErrorCode::InternalUnexpected))
        }
        Err(e) => Err(e),
    };
    complete(&app, "provider_set_enabled", id, result)
}

#[tauri::command]
async fn provider_delete(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<providers::Removed> {
    let id = ipc::correlation_id();
    let parsed = (|| {
        ipc::authorize(&window, true)?;
        if serde_json::to_vec(&request)
            .map_err(|_| ipc::ErrorCode::InvalidRequest)?
            .len()
            > 8192
        {
            return Err(ipc::ErrorCode::InputTooLarge);
        }
        ipc::parse_request::<providers::ProfileRequest>(request)
    })();
    let result = match parsed {
        Ok(p) => {
            let a = app.clone();
            let i = id.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let _ = (&p, &i);
                providers::delete(&a, p, &i)
            })
            .await
            .unwrap_or(Err(ipc::ErrorCode::InternalUnexpected))
        }
        Err(e) => Err(e),
    };
    complete(&app, "provider_delete", id, result)
}

#[tauri::command]
async fn provider_models(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<providers::Models> {
    let id = ipc::correlation_id();
    let parsed = (|| {
        ipc::authorize(&window, true)?;
        if serde_json::to_vec(&request)
            .map_err(|_| ipc::ErrorCode::InvalidRequest)?
            .len()
            > 8192
        {
            return Err(ipc::ErrorCode::InputTooLarge);
        }
        ipc::parse_request::<providers::ModelsRequest>(request)
    })();
    let result = match parsed {
        Ok(p) => {
            let a = app.clone();
            let i = id.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let _ = (&p, &i);
                providers::models(&a, p, &i)
            })
            .await
            .unwrap_or(Err(ipc::ErrorCode::InternalUnexpected))
        }
        Err(e) => Err(e),
    };
    complete(&app, "provider_models", id, result)
}

#[tauri::command]
async fn provider_test(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<providers::Receipt> {
    let id = ipc::correlation_id();
    let parsed = (|| {
        ipc::authorize(&window, true)?;
        if serde_json::to_vec(&request)
            .map_err(|_| ipc::ErrorCode::InvalidRequest)?
            .len()
            > 8192
        {
            return Err(ipc::ErrorCode::InputTooLarge);
        }
        ipc::parse_request::<providers::Selection>(request)
    })();
    let result = match parsed {
        Ok(p) => {
            let a = app.clone();
            let i = id.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let _ = (&p, &i);
                providers::submit(&a, p, "test", &i)
            })
            .await
            .unwrap_or(Err(ipc::ErrorCode::InternalUnexpected))
        }
        Err(e) => Err(e),
    };
    complete(&app, "provider_test", id, result)
}

#[tauri::command]
async fn provider_switch(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<providers::Receipt> {
    let id = ipc::correlation_id();
    let parsed = (|| {
        ipc::authorize(&window, true)?;
        if serde_json::to_vec(&request)
            .map_err(|_| ipc::ErrorCode::InvalidRequest)?
            .len()
            > 8192
        {
            return Err(ipc::ErrorCode::InputTooLarge);
        }
        ipc::parse_request::<providers::Selection>(request)
    })();
    let result = match parsed {
        Ok(p) => {
            let a = app.clone();
            let i = id.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let _ = (&p, &i);
                providers::submit(&a, p, "switch", &i)
            })
            .await
            .unwrap_or(Err(ipc::ErrorCode::InternalUnexpected))
        }
        Err(e) => Err(e),
    };
    complete(&app, "provider_switch", id, result)
}

#[tauri::command]
async fn operation_get(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<providers::OperationView> {
    let id = ipc::correlation_id();
    let parsed = (|| {
        ipc::authorize(&window, true)?;
        if serde_json::to_vec(&request)
            .map_err(|_| ipc::ErrorCode::InvalidRequest)?
            .len()
            > 8192
        {
            return Err(ipc::ErrorCode::InputTooLarge);
        }
        ipc::parse_request::<providers::OperationRequest>(request)
    })();
    let result = match parsed {
        Ok(p) => {
            let a = app.clone();
            let i = id.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let _ = (&p, &i);
                providers::get_operation(&a, p)
            })
            .await
            .unwrap_or(Err(ipc::ErrorCode::InternalUnexpected))
        }
        Err(e) => Err(e),
    };
    complete(&app, "operation_get", id, result)
}

#[tauri::command]
async fn operation_cancel(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<providers::Accepted> {
    let id = ipc::correlation_id();
    let parsed = (|| {
        ipc::authorize(&window, true)?;
        if serde_json::to_vec(&request)
            .map_err(|_| ipc::ErrorCode::InvalidRequest)?
            .len()
            > 8192
        {
            return Err(ipc::ErrorCode::InputTooLarge);
        }
        ipc::parse_request::<providers::OperationRequest>(request)
    })();
    let result = match parsed {
        Ok(p) => {
            let a = app.clone();
            let i = id.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let _ = (&p, &i);
                providers::cancel_operation(&a, p)
            })
            .await
            .unwrap_or(Err(ipc::ErrorCode::InternalUnexpected))
        }
        Err(e) => Err(e),
    };
    complete(&app, "operation_cancel", id, result)
}

#[tauri::command]
async fn credential_put(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<credentials::MutationResult> {
    let id = ipc::correlation_id();
    let parsed = (|| {
        ipc::authorize(&window, true)?;
        let secret_size = request
            .get("params")
            .and_then(|params| params.get("secret"))
            .and_then(serde_json::Value::as_str)
            .map(str::len)
            .unwrap_or(0);
        if secret_size > 24 * 1024 {
            return Err(ipc::ErrorCode::InputTooLarge);
        }
        ipc::parse_request::<credentials::PutRequest>(request)
    })();
    let result = match parsed {
        Ok(request) => {
            let worker_app = app.clone();
            let worker_id = id.clone();
            tauri::async_runtime::spawn_blocking(move || {
                credentials::put(&worker_app, request, &worker_id)
            })
            .await
            .unwrap_or(Err(ipc::ErrorCode::CredentialsStorageFailed))
        }
        Err(error) => Err(error),
    };
    complete(&app, "credential_put", id, result)
}

#[tauri::command]
async fn credential_remove(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<credentials::MutationResult> {
    let id = ipc::correlation_id();
    let parsed = (|| {
        ipc::authorize(&window, true)?;
        ipc::parse_request::<credentials::RemoveRequest>(request)
    })();
    let result = match parsed {
        Ok(request) => {
            let worker_app = app.clone();
            let worker_id = id.clone();
            tauri::async_runtime::spawn_blocking(move || {
                credentials::remove(&worker_app, request, &worker_id)
            })
            .await
            .unwrap_or(Err(ipc::ErrorCode::CredentialsStorageFailed))
        }
        Err(error) => Err(error),
    };
    complete(&app, "credential_remove", id, result)
}

#[tauri::command]
async fn logs_get_status(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<logging::LogStatus> {
    let id = ipc::correlation_id();
    let result = (|| {
        ipc::authorize(&window, true)?;
        ipc::parse_request::<ipc::EmptyParams>(request)?;
        Ok(app.state::<logging::LoggingState>().status())
    })();
    complete(&app, "logs_get_status", id, result)
}

#[tauri::command]
async fn logs_clear(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<logging::LogStatus> {
    let id = ipc::correlation_id();
    let result = (|| {
        ipc::authorize(&window, true)?;
        ipc::parse_request::<ipc::EmptyParams>(request)?;
        app.state::<logging::LoggingState>().clear()
    })();
    complete(&app, "logs_clear", id, result)
}

#[tauri::command]
async fn settings_get(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<serde_json::Value> {
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
        let scope: settings::Scope = ipc::parse_request(request)?;
        if scope.scope == "project" {
            let project_id = scope
                .project_id
                .as_deref()
                .ok_or(ipc::ErrorCode::InvalidRequest)?;
            return projects::project_settings(&app, project_id, &id)
                .map(|v| serde_json::to_value(v).map_err(|_| ipc::ErrorCode::InternalUnexpected))
                .and_then(|v| v);
        }
        scope.authorize()?;
        settings::get(&app)
            .and_then(|v| serde_json::to_value(v).map_err(|_| ipc::ErrorCode::InternalUnexpected))
    })();
    complete(&app, "settings_get", id, result)
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
    complete(&app, "settings_update", id, result)
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
    complete(&app, "overlay_get", id, result)
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
    complete(&app, "overlay_control", id, result)
}

fn complete<T: serde::Serialize>(
    app: &tauri::AppHandle,
    command: &'static str,
    id: String,
    result: Result<T, ipc::ErrorCode>,
) -> ipc::Response<T> {
    let failure = result.as_ref().err().copied();
    eprintln!("{}", ipc::command_log(command, &id, failure));
    app.state::<logging::LoggingState>()
        .record(command, &id, failure);
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
    complete(&app, "app_get_status", id, result)
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
            #[cfg(windows)]
            action if action.starts_with("project_") => projects::fixture(&app, action),
            action if action.starts_with("credential_") => {
                credentials::verification_fixture(&app, action)
            }
            "tray_disable" => overlay::tray_action(&app, "disable"),
            "tray_enable" => overlay::tray_action(&app, "enable"),
            "audit_fail_write" | "audit_restore_write" => {
                let fail = params.action == "audit_fail_write";
                app.state::<storage::StorageState>().with_storage(|storage| {
                    storage.connection.execute_batch(if fail {
                        "CREATE TEMP TRIGGER fail_audit_verify BEFORE INSERT ON audit_events BEGIN SELECT RAISE(ABORT,'synthetic-secret-DO-NOT-LOG'); END;"
                    } else { "DROP TRIGGER fail_audit_verify;" }).map_err(|_| storage::StorageError { code: "STORAGE_SQL_FAILED", backup_path: None })
                }).map_err(|_| ipc::ErrorCode::AuditWriteFailed)
            }
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
    complete(&app, "ipc_fixture", id, result)
}

#[tauri::command]
fn desktop_initialize(
    app: tauri::AppHandle,
    request: serde_json::Value,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, desktop::DesktopState>,
) -> ipc::Response<desktop::DesktopReady> {
    let id = ipc::correlation_id();
    let result = ipc::authorize(&window, true).and_then(|_| state.initialize(request));
    complete(&app, "desktop_initialize", id, result)
}

#[tauri::command]
fn framework_probe(
    app: tauri::AppHandle,
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
    app.state::<logging::LoggingState>().record(
        "framework_probe",
        &id,
        result.as_ref().err().copied(),
    );
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
        .manage(logging::LoggingState::default())
        .manage(credentials::CredentialsState::default())
        .manage(owned_process::OwnedProcessState::default())
        .manage(providers::ProvidersState::default())
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
                        } else if let Err(code) = app.state::<owned_process::OwnedProcessState>().initialize(&data_dir) {
                            eprintln!("{}", serde_json::to_string(&code).unwrap_or_default());
                        }
                        if let Err(code)=providers::initialize(app.handle()){
                            eprintln!("{}",serde_json::to_string(&code).unwrap_or_default());
                        }
                        settings::initialize(app.handle(), &data_dir);
                        let level = settings::get(app.handle())
                            .ok()
                            .and_then(|view| view.values.into_iter().find_map(|change| match change {
                                settings::Change::LogLevel(value) => Some(value),
                                _ => None,
                            }))
                            .unwrap_or_else(|| "info".to_string());
                        let log_dir = if cfg!(debug_assertions) {
                            std::env::var_os("PA_LOG_VERIFY_DIR")
                                .map(std::path::PathBuf::from)
                                .map(Ok)
                                .unwrap_or_else(|| app.path().app_log_dir())
                        } else {
                            app.path().app_log_dir()
                        };
                        match log_dir {
                            Ok(path) => app.state::<logging::LoggingState>().initialize(&path, &level),
                            Err(_) => app.state::<logging::LoggingState>().initialize(std::path::Path::new(""), &level),
                        }
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
            projects_add_from_picker,
            project_get,
            project_git_summary,
            projects_list,
            project_open_directory,
            project_open_file,
            project_pick_file,
            project_remove,
            project_select,
            desktop_initialize,
            app_get_status,
            settings_get,
            settings_update,
            logs_get_status,
            logs_clear,
            providers_list,
            provider_save,
            provider_set_enabled,
            provider_delete,
            provider_models,
            provider_test,
            provider_switch,
            operation_get,
            operation_cancel,
            credential_put,
            credential_remove,
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
