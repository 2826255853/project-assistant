//! S2-03/04/05 main-window entry points. Active state follows in S2-08.
use crate::{
    ipc::ErrorCode,
    project_contract::ProjectView,
    storage::{Storage, StorageState},
};
use tauri::Manager;

pub use crate::project_contract::{FilePickerResult, OpenResult};

pub fn git_summary(
    app: &tauri::AppHandle,
    id: &str,
    refresh: bool,
    correlation: &str,
) -> Result<crate::project_contract::GitSummary, ErrorCode> {
    #[cfg(windows)]
    {
        // Obtain the short-lived authorized directory used as Git's cwd.
        // Persist invalidation when identity/access checks fail. Git reads never
        // create a project usage record.
        let prepared = with_repository(app, |storage| storage.projects().authorized(id));
        let (revision, verified) = match prepared {
            Ok(prepared) => prepared,
            Err(code) => {
                get(app, id, correlation)?;
                return Err(code);
            }
        };
        let summary = crate::project_git::summary(id, revision, &verified.canonical_path, refresh);
        summary.validate_wire()?;
        Ok(summary)
    }
    #[cfg(not(windows))]
    {
        let _ = (app, id, refresh, correlation);
        Err(ErrorCode::ProjectPathUnavailable)
    }
}

pub fn open(
    app: &tauri::AppHandle,
    id: &str,
    relative: Option<&str>,
    correlation: &str,
) -> Result<OpenResult, ErrorCode> {
    #[cfg(windows)]
    {
        // Persist invalidation and publish even when an operation must fail.
        let view = get(app, id, correlation)?;
        with_repository(app, |storage| {
            let editor = storage
                .settings()
                .read_view()?
                .values
                .into_iter()
                .find_map(|v| {
                    if let crate::settings::Change::Editor(editor) = v {
                        editor
                    } else {
                        None
                    }
                });
            storage.projects().record_usage(
                id,
                view.revision,
                crate::storage::projects::UsageSource::SystemOpen,
                |verified| {
                    let root = crate::project_paths::delivery::directory(verified)?;
                    let target = relative
                        .map(|r| crate::project_paths::delivery::file(&root, r))
                        .transpose()?;
                    crate::project_paths::delivery::open(root, target, editor)
                },
            )
        })?;
        crate::app_state::publish(app, correlation.into());
        Ok(OpenResult {
            system_accepted: true,
        })
    }
    #[cfg(not(windows))]
    {
        let _ = (app, id, relative, correlation);
        Err(ErrorCode::ProjectOpenRejected)
    }
}

pub fn pick_file(
    app: &tauri::AppHandle,
    owner: isize,
    id: &str,
    correlation: &str,
) -> Result<FilePickerResult, ErrorCode> {
    #[cfg(windows)]
    {
        #[cfg(debug_assertions)]
        if picker_fixture_enabled() {
            let (revision, verified) =
                with_repository(app, |storage| storage.projects().authorized(id))?;
            let root = crate::project_paths::delivery::directory(&verified)?;
            let relative_path = "selected-through-ui.txt";
            let _file = crate::project_paths::delivery::file(&root, relative_path)?;
            let _ = revision;
            return Ok(FilePickerResult::Selected {
                relative_path: relative_path.into(),
            });
        }
        let prepared = with_repository(app, |storage| {
            let (revision, verified) = storage.projects().authorized(id)?;
            Ok((
                revision,
                crate::project_paths::delivery::directory(&verified)?,
            ))
        });
        let (revision, pinned) = match prepared {
            Ok(prepared) => prepared,
            Err(code) => {
                get(app, id, correlation)?;
                return Err(code);
            }
        };
        // Native UI is outside the database mutex/transaction; cancel writes no usage.
        let Some(relative_path) = crate::project_paths::delivery::pick(owner, &pinned)? else {
            return Ok(FilePickerResult::Cancelled {
                outcome: "cancelled",
            });
        };
        with_repository(app, |storage| {
            let (current, verified) = storage.projects().authorized(id)?;
            if current != revision {
                return Err(ErrorCode::ProjectConflictRevision);
            }
            let root = crate::project_paths::delivery::directory(&verified)?;
            let _file = crate::project_paths::delivery::file(&root, &relative_path)?;
            Ok(())
        })?;
        Ok(FilePickerResult::Selected { relative_path })
    }
    #[cfg(not(windows))]
    {
        let _ = (app, owner, id, correlation);
        Err(ErrorCode::ProjectPickerFailed)
    }
}

#[cfg(all(debug_assertions, windows))]
fn picker_fixture_enabled() -> bool {
    std::env::var("PA_IPC_VERIFY").as_deref() == Ok("1")
        && std::env::var("PA_PROJECT_PICKER_FIXTURE").as_deref() == Ok("1")
}

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
        #[cfg(debug_assertions)]
        if picker_fixture_enabled() {
            return add_fixture(app, correlation);
        }
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

/// Synthetic picker result used only by the isolated debug desktop acceptance flow.
#[cfg(all(debug_assertions, windows))]
pub(crate) fn add_fixture(
    app: &tauri::AppHandle,
    correlation: &str,
) -> Result<PickerResult, ErrorCode> {
    use std::path::PathBuf;
    let directory = std::env::var_os("PA_OVERLAY_VERIFY_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or(ErrorCode::PermissionDenied)?;
    let project = with_repository(app, |storage| {
        if storage.path.parent() != Some(directory.as_path()) {
            return Err(ErrorCode::PermissionDenied);
        }
        let source = directory.join("project-added-through-ui");
        std::fs::create_dir_all(&source).map_err(|_| ErrorCode::ProjectPathUnavailable)?;
        std::fs::write(
            source.join("selected-through-ui.txt"),
            "synthetic file-picker acceptance",
        )
        .map_err(|_| ErrorCode::ProjectPathUnavailable)?;
        let candidate = crate::project_paths::authorize_fixture(&source)?;
        storage.projects().register(Some(candidate), correlation)
    })?
    .ok_or(ErrorCode::InternalUnexpected)?;
    crate::app_state::publish(app, correlation.into());
    Ok(PickerResult::Registered { project })
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
                let git =
                    crate::project_git::discover_git().ok_or(ErrorCode::ProjectPathUnavailable)?;
                let initialized = std::process::Command::new(&git)
                    .args(["init", "-b", "s2-06-fixture"])
                    .current_dir(&source)
                    .env("GIT_CONFIG_NOSYSTEM", "1")
                    .env("GIT_CONFIG_GLOBAL", "NUL")
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .ok()
                    .is_some_and(|status| status.success());
                if initialized {
                    let staged = std::process::Command::new(&git)
                        .args(["add", "--", "keep.txt"])
                        .current_dir(&source)
                        .env("GIT_CONFIG_NOSYSTEM", "1")
                        .env("GIT_CONFIG_GLOBAL", "NUL")
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .status()
                        .ok()
                        .is_some_and(|status| status.success());
                    if staged {
                        let _ = std::process::Command::new(&git)
                            .args([
                                "-c",
                                "user.name=Project Assistant Fixture",
                                "-c",
                                "user.email=fixture@example.invalid",
                                "commit",
                                "-m",
                                "synthetic fixture",
                            ])
                            .current_dir(&source)
                            .env("GIT_CONFIG_NOSYSTEM", "1")
                            .env("GIT_CONFIG_GLOBAL", "NUL")
                            .stdin(std::process::Stdio::null())
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .status();
                    }
                }
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
            "project_git_dirty" => {
                std::fs::write(source.join("git-dirty-fixture.txt"), "synthetic git change")
                    .map_err(|_| ErrorCode::ProjectPathUnavailable)?;
            }
            _ => return Err(ErrorCode::InvalidRequest),
        }
        Ok(())
    })
}
