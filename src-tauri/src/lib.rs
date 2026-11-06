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
mod tasks;
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

fn task_request<T: serde::de::DeserializeOwned>(
    window: &tauri::WebviewWindow,
    request: serde_json::Value,
    max_bytes: usize,
) -> Result<T, ipc::ErrorCode> {
    ipc::authorize(window, true)?;
    if serde_json::to_vec(&request)
        .map_err(|_| ipc::ErrorCode::InvalidRequest)?
        .len()
        > max_bytes
    {
        return Err(ipc::ErrorCode::InputTooLarge);
    }
    ipc::parse_request(request)
}

fn overlay_task_request<T: serde::de::DeserializeOwned>(
    window: &tauri::WebviewWindow,
    request: serde_json::Value,
    max_bytes: usize,
) -> Result<T, ipc::ErrorCode> {
    ipc::authorize(window, false)?;
    if window.label() != "overlay" {
        return Err(ipc::ErrorCode::PermissionDenied);
    }
    if serde_json::to_vec(&request)
        .map_err(|_| ipc::ErrorCode::InvalidRequest)?
        .len()
        > max_bytes
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
fn session_create(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<storage::tasks::SessionReceipt> {
    let id = ipc::correlation_id();
    let result = project_request::<tasks::SessionCreateRequest>(&window, request).and_then(|p| {
        p.validate()?;
        tasks::session_create(&app, p, &id)
    });
    complete(&app, "session_create", id, result)
}

#[tauri::command]
fn session_find_by_request(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<Option<storage::tasks::SessionRecord>> {
    let id = ipc::correlation_id();
    let result = project_request::<tasks::SessionFindRequest>(&window, request).and_then(|p| {
        p.validate()?;
        tasks::session_find(&app, p)
    });
    complete(&app, "session_find_by_request", id, result)
}

#[tauri::command]
fn sessions_list(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<tasks::SessionPage> {
    let id = ipc::correlation_id();
    let result = project_request::<tasks::SessionsListRequest>(&window, request).and_then(|p| {
        p.validate()?;
        tasks::sessions_list(&app, p)
    });
    complete(&app, "sessions_list", id, result)
}

#[tauri::command]
fn session_get(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<storage::tasks::SessionRecord> {
    let id = ipc::correlation_id();
    let result = project_request::<tasks::SessionGetRequest>(&window, request).and_then(|p| {
        p.validate()?;
        tasks::session_get(&app, p)
    });
    complete(&app, "session_get", id, result)
}

#[tauri::command]
fn session_resume(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<storage::tasks::SessionRecord> {
    let id = ipc::correlation_id();
    let result = project_request::<tasks::SessionResumeRequest>(&window, request).and_then(|p| {
        p.validate()?;
        tasks::session_resume(&app, p, &id)
    });
    complete(&app, "session_resume", id, result)
}

#[tauri::command]
fn session_end(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<storage::tasks::SessionRecord> {
    let id = ipc::correlation_id();
    let result = project_request::<tasks::SessionEndRequest>(&window, request).and_then(|p| {
        p.validate()?;
        tasks::session_end(&app, p, &id)
    });
    complete(&app, "session_end", id, result)
}

#[tauri::command]
fn task_start(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<storage::tasks::TaskReceipt> {
    let id = ipc::correlation_id();
    let result =
        task_request::<tasks::TaskStartRequest>(&window, request, 80 * 1024).and_then(|p| {
            p.validate()?;
            tasks::start(&app, p, &id)
        });
    complete(&app, "task_start", id, result)
}

#[tauri::command]
fn task_find_by_request(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<Option<storage::tasks::TaskRecord>> {
    let id = ipc::correlation_id();
    let result = project_request::<tasks::TaskFindRequest>(&window, request).and_then(|p| {
        p.validate()?;
        tasks::find(&app, p)
    });
    complete(&app, "task_find_by_request", id, result)
}

#[tauri::command]
fn tasks_list(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<tasks::TaskPage> {
    let id = ipc::correlation_id();
    let result = project_request::<tasks::TasksListRequest>(&window, request).and_then(|p| {
        p.validate()?;
        tasks::list(&app, p)
    });
    complete(&app, "tasks_list", id, result)
}

#[tauri::command]
fn task_get(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<storage::tasks::TaskRecord> {
    let id = ipc::correlation_id();
    let result = project_request::<tasks::TaskGetRequest>(&window, request).and_then(|p| {
        p.validate()?;
        tasks::get(&app, p)
    });
    complete(&app, "task_get", id, result)
}

#[tauri::command]
fn task_cancel(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<storage::tasks::CancelReceipt> {
    let id = ipc::correlation_id();
    let result = project_request::<tasks::TaskCancelRequest>(&window, request).and_then(|p| {
        p.validate()?;
        tasks::cancel(&app, p, &id)
    });
    complete(&app, "task_cancel", id, result)
}

#[tauri::command]
fn task_events_get(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<storage::tasks::EventPage> {
    let id = ipc::correlation_id();
    let result = project_request::<tasks::TaskEventsRequest>(&window, request).and_then(|p| {
        p.validate()?;
        tasks::events(&app, p)
    });
    complete(&app, "task_events_get", id, result)
}

#[tauri::command]
fn approvals_get(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<tasks::ApprovalPage> {
    let id = ipc::correlation_id();
    let result = project_request::<tasks::ApprovalsRequest>(&window, request).and_then(|p| {
        p.validate()?;
        tasks::approvals(&app, p)
    });
    complete(&app, "approvals_get", id, result)
}

#[tauri::command]
fn approval_decide(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<storage::tasks::ApprovalReceipt> {
    let id = ipc::correlation_id();
    let result =
        task_request::<tasks::ApprovalDecideRequest>(&window, request, 8192).and_then(|p| {
            p.validate()?;
            tasks::approval_decide(&app, p, &id)
        });
    complete(&app, "approval_decide", id, result)
}

#[tauri::command]
fn task_content_get(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<tasks::results::Content> {
    let id = ipc::correlation_id();
    let result = project_request::<tasks::results::ContentRequest>(&window, request)
        .and_then(|p| tasks::results::content(&app, p));
    complete(&app, "task_content_get", id, result)
}

#[tauri::command]
fn task_changes_get(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<tasks::results::Changes> {
    let id = ipc::correlation_id();
    let result = project_request::<tasks::results::ChangesRequest>(&window, request)
        .and_then(|p| tasks::results::changes(&app, p));
    complete(&app, "task_changes_get", id, result)
}

#[tauri::command]
fn task_change_get(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<tasks::results::Diff> {
    let id = ipc::correlation_id();
    let result = project_request::<tasks::results::DiffRequest>(&window, request)
        .and_then(|p| tasks::results::diff(&app, p));
    complete(&app, "task_change_get", id, result)
}

#[tauri::command]
fn active_context_set(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<tasks::active::ContextState> {
    let id = ipc::correlation_id();
    let result = project_request::<tasks::active::ActiveContextSetRequest>(&window, request)
        .and_then(|p| {
            p.validate()?;
            tasks::active::set_active_context(&app, p)
        });
    complete(&app, "active_context_set", id, result)
}

#[tauri::command]
fn task_start_from_active(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<tasks::active::OverlayTaskReceipt> {
    let id = ipc::correlation_id();
    let result = (|| {
        if window.label() != "overlay" {
            return Err(ipc::ErrorCode::PermissionDenied);
        }
        let p = overlay_task_request::<tasks::active::OverlayTaskStartRequest>(
            &window,
            request,
            96 * 1024,
        )?;
        p.validate()?;
        tasks::active::start_from_active(&app, p, &id)
    })();
    complete(&app, "task_start_from_active", id, result)
}

#[tauri::command]
fn overlay_task_get(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<serde_json::Value> {
    let id = ipc::correlation_id();
    let result = (|| {
        if window.label() != "overlay" {
            return Err(ipc::ErrorCode::PermissionDenied);
        }
        ipc::authorize(&window, false)?;
        ipc::parse_request::<ipc::EmptyParams>(request)?;
        let state = tasks::active::overlay_get(&app)?;
        serde_json::to_value(state).map_err(|_| ipc::ErrorCode::InternalUnexpected)
    })();
    complete(&app, "overlay_task_get", id, result)
}

#[tauri::command]
fn overlay_task_find_by_request(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<Option<tasks::active::OverlayTaskView>> {
    let id = ipc::correlation_id();
    let result = (|| {
        if window.label() != "overlay" {
            return Err(ipc::ErrorCode::PermissionDenied);
        }
        let p = overlay_task_request::<tasks::active::OverlayTaskFindRequest>(
            &window,
            request,
            16 * 1024,
        )?;
        p.validate()?;
        tasks::active::overlay_find_by_request(&app, p)
    })();
    complete(&app, "overlay_task_find_by_request", id, result)
}

#[tauri::command]
fn overlay_task_cancel(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<storage::tasks::CancelReceipt> {
    let id = ipc::correlation_id();
    let result = (|| {
        if window.label() != "overlay" {
            return Err(ipc::ErrorCode::PermissionDenied);
        }
        let p = overlay_task_request::<tasks::active::OverlayTaskCancelRequest>(
            &window,
            request,
            16 * 1024,
        )?;
        p.validate()?;
        tasks::active::overlay_cancel(&app, p, &id)
    })();
    complete(&app, "overlay_task_cancel", id, result)
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
async fn project_settings_update(
    request: serde_json::Value,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ipc::Response<project_contract::ProjectSettingsView> {
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
        let params: project_contract::ProjectAiDefaultsUpdate = ipc::parse_request(request)?;
        projects::update_project_ai_defaults(&app, params, &id)
    })();
    complete(&app, "project_settings_update", id, result)
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
        if matches!(&action, overlay::OverlayAction::OpenMain) && window.label() == "overlay" {
            tasks::active::focus_current_task(&app)?;
        }
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
            #[cfg(all(test, windows))]
            action if action.starts_with("task_ui_") => {
                tasks::results::desktop_fixture(&app, action)
            }
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
            "ai_settings_sql_fail" | "ai_settings_sql_restore" => {
                let fail = params.action == "ai_settings_sql_fail";
                app.state::<storage::StorageState>().with_storage(|storage| {
                    storage.connection.execute_batch(if fail {
                        "CREATE TEMP TRIGGER fail_project_ai_setting BEFORE INSERT ON settings WHEN NEW.scope='project' AND NEW.key='ai_defaults' BEGIN SELECT RAISE(ABORT,'synthetic-secret-DO-NOT-LOG'); END;"
                    } else { "DROP TRIGGER fail_project_ai_setting;" }).map_err(|_| storage::StorageError { code: "STORAGE_SQL_FAILED", backup_path: None })
                }).map_err(|_| ipc::ErrorCode::SettingsStorageFailed)
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
    let builder = tauri::Builder::default();
    #[cfg(test)]
    let builder = builder.any_thread();
    let result = builder
        .manage(state)
        .manage(app_state::AppState::default())
        .manage(overlay::OverlayState::default())
        .manage(storage::StorageState::default())
        .manage(settings::SettingsState::default())
        .manage(logging::LoggingState::default())
        .manage(credentials::CredentialsState::default())
        .manage(owned_process::OwnedProcessState::default())
        .manage(providers::ProvidersState::default())
        .manage(tasks::TaskRuntimeState::default())
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
            session_create,
            session_find_by_request,
            sessions_list,
            session_get,
            session_resume,
            session_end,
            task_start,
            task_find_by_request,
            tasks_list,
            task_get,
            task_cancel,
            task_events_get,
            approvals_get,
            approval_decide,
            task_content_get,
            task_changes_get,
            task_change_get,
            active_context_set,
            task_start_from_active,
            overlay_task_get,
            overlay_task_find_by_request,
            overlay_task_cancel,
            desktop_initialize,
            app_get_status,
            settings_get,
            settings_update,
            project_settings_update,
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
        .build(tauri::generate_context!());
    match result {
        Ok(app) => app.run(|handle, event| {
            if matches!(
                event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                // Close both admissions before waiting: a provider candidate may
                // be queued behind a task on the owned-process serial lock.
                handle.state::<tasks::TaskRuntimeState>().begin_shutdown();
                handle.state::<providers::ProvidersState>().begin_shutdown();
                handle
                    .state::<owned_process::OwnedProcessState>()
                    .begin_shutdown();
                handle.state::<tasks::TaskRuntimeState>().cancel_all();
                handle
                    .state::<providers::ProvidersState>()
                    .wait_for_shutdown();
                handle
                    .state::<owned_process::OwnedProcessState>()
                    .wait_for_shutdown();
            }
        }),
        Err(_) => {
            eprintln!("PA_INTERNAL_STARTUP_FAILED");
            std::process::exit(1);
        }
    }
}

// Real WebView acceptance runs the same application in a test binary. Only
// this binary can trust the disposable synthetic HTTPS CA/loopback endpoint.
#[cfg(all(test, windows))]
#[test]
#[ignore = "launched by verify:task-ui in an isolated process"]
fn task_desktop_acceptance() {
    assert_eq!(std::env::var("PA_IPC_VERIFY").as_deref(), Ok("1"));
    assert!(std::env::var_os("PA_OVERLAY_VERIFY_DIR").is_some());
    run();
}

#[cfg(all(test, windows))]
#[test]
#[ignore = "isolated startup recovery and exact synthetic vault cleanup for verify:task-concurrency"]
fn task_desktop_recovery() {
    let directory = std::path::PathBuf::from(std::env::var_os("PA_TASK_RECOVERY_DIR").unwrap());
    let parent = directory
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy();
    assert!(parent.starts_with("pa-task-ui-"));
    assert!(matches!(
        directory.file_name().unwrap().to_str(),
        Some("dev" | "built")
    ));
    let mut db = storage::Storage::open(&directory).unwrap();
    let mode = std::env::var("PA_TASK_RECOVERY_MODE").unwrap();
    if mode != "cleanup" {
        if mode == "exit" {
            assert!(
                std::fs::read_dir(directory.join("owned-processes"))
                    .unwrap()
                    .next()
                    .is_none()
            );
            let unfinished: i64 = db.connection.query_row("SELECT count(*) FROM provider_operations WHERE phase IN ('checking','waiting_for_idle','applying','rolling_back')", [], |row| row.get(0)).unwrap();
            assert_eq!(unfinished, 0);
        }
        let before: i64 = db
            .connection
            .query_row("SELECT count(*) FROM tasks", [], |row| row.get(0))
            .unwrap();
        for _ in 0..2 {
            db.recover_on_startup(&ipc::correlation_id()).unwrap();
            providers::recover(&mut db.connection).unwrap();
            owned_process::OwnedProcessState::default()
                .initialize(&directory)
                .unwrap();
        }
        let status: String = db
            .connection
            .query_row(
                "SELECT status FROM tasks ORDER BY rowid DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            status,
            if mode == "exit" {
                "cancelled"
            } else {
                "interrupted"
            }
        );
        let after: i64 = db
            .connection
            .query_row("SELECT count(*) FROM tasks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(before, after);
        let pending: i64 = db
            .connection
            .query_row(
                "SELECT count(*) FROM approvals WHERE decision='pending'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(pending, 0);
        let operations: i64 = db.connection.query_row("SELECT count(*) FROM provider_operations WHERE phase IN ('checking','waiting_for_idle','applying','rolling_back')", [], |row| row.get(0)).unwrap();
        assert_eq!(operations, 0);
        assert!(
            std::fs::read_dir(directory.join("owned-processes"))
                .unwrap()
                .next()
                .is_none()
        );
        println!(
            "LIFECYCLE_EVIDENCE:{}",
            serde_json::json!({"mode":mode,"taskCountUnchanged":true,"pendingApprovals":pending,"ownedResourcesCleaned":true,"lastTaskStatus":status})
        );
    }
    credentials::cleanup_task_desktop_credentials(&mut db);
}
