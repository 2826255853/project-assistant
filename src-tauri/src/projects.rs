//! S2-03/04 main-window entry points. Selection/OS open/active state follow later.
use crate::{
    ipc::ErrorCode,
    project_contract::ProjectView,
    storage::{Storage, StorageState},
};
use tauri::Manager;

pub fn list(
    app: &tauri::AppHandle,
    request: &crate::project_contract::ListRequest,
) -> Result<crate::project_contract::ProjectListPage, ErrorCode> {
    #[cfg(windows)]
    {
        with_repository(app, |storage| storage.projects().list(request))
    }
    #[cfg(not(windows))]
    {
        let _ = (app, request);
        Err(ErrorCode::ProjectPathUnavailable)
    }
}

#[derive(serde::Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum PickerResult {
    Registered { project: ProjectView },
    Cancelled,
}

pub(crate) fn with_repository<T>(
    app: &tauri::AppHandle,
    f: impl FnOnce(&mut Storage) -> Result<T, ErrorCode>,
) -> Result<T, ErrorCode> {
    app.state::<StorageState>()
        .with_storage(|storage| Ok(f(storage)))
        .map_err(|_| ErrorCode::SettingsStorageFailed)?
}

pub fn add(
    app: &tauri::AppHandle,
    owner: isize,
    correlation: &str,
) -> Result<PickerResult, ErrorCode> {
    #[cfg(windows)]
    {
        // No database lock while the native modal picker is visible.
        let candidate = crate::project_paths::choose_project_directory(owner)?;
        let Some(candidate) = candidate else {
            return Ok(PickerResult::Cancelled);
        };
        let project = with_repository(app, |storage| {
            storage.projects().register(Some(candidate), correlation)
        })?
        .ok_or(ErrorCode::InternalUnexpected)?;
        crate::app_state::publish(app, correlation.into());
        Ok(PickerResult::Registered { project })
    }
    #[cfg(not(windows))]
    {
        let _ = (app, owner, correlation);
        Err(ErrorCode::ProjectPickerFailed)
    }
}

pub fn get(app: &tauri::AppHandle, id: &str, correlation: &str) -> Result<ProjectView, ErrorCode> {
    #[cfg(windows)]
    {
        let (view, changed) = with_repository(app, |storage| {
            storage.projects().get_with_refresh(id, correlation)
        })?;
        if changed {
            crate::app_state::publish(app, correlation.into());
        }
        Ok(view)
    }
    #[cfg(not(windows))]
    {
        let _ = (app, id, correlation);
        Err(ErrorCode::ProjectPathUnavailable)
    }
}

pub fn remove(
    app: &tauri::AppHandle,
    id: &str,
    revision: u64,
    correlation: &str,
) -> Result<crate::project_contract::RemoveResult, ErrorCode> {
    #[cfg(windows)]
    {
        let result = with_repository(app, |storage| {
            storage.projects().remove(id, revision, correlation)
        })?;
        crate::app_state::publish(app, correlation.into());
        Ok(crate::project_contract::RemoveResult {
            removed: result.removed,
            retained_for_history: result.retained_for_history,
        })
    }
    #[cfg(not(windows))]
    {
        let _ = (app, id, revision, correlation);
        Err(ErrorCode::ProjectNotFound)
    }
}

/// Fixed synthetic source only; guarded by ipc_fixture's debug main capability.
#[cfg(all(debug_assertions, windows))]
pub(crate) fn fixture(app: &tauri::AppHandle, action: &str) -> Result<(), ErrorCode> {
    use std::path::PathBuf;
    let directory = std::env::var_os("PA_OVERLAY_VERIFY_DIR")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .ok_or(ErrorCode::PermissionDenied)?;
    with_repository(app, |storage| {
        if storage.path.parent() != Some(directory.as_path()) {
            return Err(ErrorCode::PermissionDenied);
        }
        let source = directory.join("synthetic-project-private-path");
        match action {
            "project_search_seed" => {
                crate::storage::projects::search_fixture::seed(&mut storage.connection)?
            }
            "project_seed" => {
                std::fs::create_dir_all(&source).map_err(|_| ErrorCode::ProjectPathUnavailable)?;
                std::fs::write(source.join("keep.txt"), "source-sentinel")
                    .map_err(|_| ErrorCode::ProjectPathUnavailable)?;
                let candidate = crate::project_paths::authorize_fixture(&source)?;
                let view = storage
                    .projects()
                    .register(Some(candidate), &crate::ipc::correlation_id())?
                    .ok_or(ErrorCode::InternalUnexpected)?;
                std::fs::write(directory.join("project-fixture-id"), view.id)
                    .map_err(|_| ErrorCode::SettingsStorageFailed)?;
            }
            "project_missing" => {
                std::fs::rename(&source, directory.join("moved-project"))
                    .map_err(|_| ErrorCode::ProjectPathUnavailable)?;
            }
            _ => return Err(ErrorCode::InvalidRequest),
        }
        Ok(())
    })
}
