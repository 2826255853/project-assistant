//! Typed settings service. No arbitrary configuration, credentials or shell arguments.
use crate::{ipc::ErrorCode, overlay, storage};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::Manager;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Position {
    pub x: i32,
    pub y: i32,
    pub monitor: Option<String>,
    pub scale_factor: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Editor {
    pub id: String,
    pub executable: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "key",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Change {
    SearchEngine(String),
    Editor(Option<Editor>),
    ProjectSort(String),
    ApprovalPolicy(String),
    LogLevel(String),
    SummaryRetentionDays(i64),
    // AI configuration becomes writable only through the validated profile service.
    AiDefaults(Option<serde_json::Value>),
    OverlayEnabled(bool),
    OverlayShortcut(String),
    OverlayPosition(Option<Position>),
}

impl Change {
    pub fn key(&self) -> &'static str {
        match self {
            Self::SearchEngine(_) => "search_engine",
            Self::Editor(_) => "editor",
            Self::ProjectSort(_) => "project_sort",
            Self::ApprovalPolicy(_) => "approval_policy",
            Self::LogLevel(_) => "log_level",
            Self::SummaryRetentionDays(_) => "summary_retention_days",
            Self::AiDefaults(_) => "ai_defaults",
            Self::OverlayEnabled(_) => "overlay_enabled",
            Self::OverlayShortcut(_) => "overlay_shortcut",
            Self::OverlayPosition(_) => "overlay_position",
        }
    }

    pub fn validate(&self) -> Result<(), ErrorCode> {
        let valid = match self {
            Self::SearchEngine(value) => matches!(value.as_str(), "bing" | "google"),
            Self::ProjectSort(value) => matches!(value.as_str(), "recent" | "name"),
            Self::ApprovalPolicy(value) => value == "ask",
            Self::LogLevel(value) => matches!(value.as_str(), "error" | "warn" | "info" | "debug"),
            Self::SummaryRetentionDays(value) => (1..=365).contains(value),
            Self::Editor(None)
            | Self::AiDefaults(None)
            | Self::OverlayEnabled(_)
            | Self::OverlayPosition(None) => true,
            Self::AiDefaults(Some(_)) => return Err(ErrorCode::SettingsAiUnavailable),
            Self::Editor(Some(value)) => {
                let path = std::path::Path::new(&value.executable);
                value.id == "custom"
                    && value.executable.len() <= 1024
                    && path.is_absolute()
                    && value.executable.to_ascii_lowercase().ends_with(".exe")
                    && !value
                        .executable
                        .chars()
                        .any(|c| c.is_control() || matches!(c, '"' | '<' | '>' | '|' | '*' | '?'))
                    && !path.components().any(|c| {
                        matches!(
                            c,
                            std::path::Component::ParentDir | std::path::Component::CurDir
                        )
                    })
            }
            Self::OverlayShortcut(value) => {
                if value.is_empty()
                    || value.len() > 80
                    || value
                        .parse::<tauri_plugin_global_shortcut::Shortcut>()
                        .is_err()
                {
                    return Err(ErrorCode::ShortcutInvalid);
                }
                true
            }
            Self::OverlayPosition(Some(value)) => {
                value.scale_factor.is_finite()
                    && (0.25..=8.0).contains(&value.scale_factor)
                    && value
                        .monitor
                        .as_ref()
                        .is_none_or(|name| name.len() <= 256 && !name.chars().any(char::is_control))
            }
        };
        if valid {
            Ok(())
        } else {
            Err(ErrorCode::SettingsInvalidValue)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct View {
    pub revision: i64,
    pub values: Vec<Change>,
}

impl View {
    pub fn defaults(revision: i64) -> Self {
        Self {
            revision,
            values: vec![
                Change::SearchEngine("bing".into()),
                Change::Editor(None),
                Change::ProjectSort("recent".into()),
                Change::ApprovalPolicy("ask".into()),
                Change::LogLevel("info".into()),
                Change::SummaryRetentionDays(30),
                Change::AiDefaults(None),
                Change::OverlayEnabled(true),
                Change::OverlayShortcut(overlay::DEFAULT_SHORTCUT.into()),
                Change::OverlayPosition(None),
            ],
        }
    }
    pub fn replace(&mut self, change: Change) {
        let slot = self
            .values
            .iter_mut()
            .find(|value| value.key() == change.key())
            .expect("known key");
        *slot = change;
    }
    pub fn resident(&self) -> (bool, String, Option<Position>) {
        let mut enabled = true;
        let mut shortcut = overlay::DEFAULT_SHORTCUT.to_owned();
        let mut position = None;
        for value in &self.values {
            match value {
                Change::OverlayEnabled(v) => enabled = *v,
                Change::OverlayShortcut(v) => shortcut = v.clone(),
                Change::OverlayPosition(v) => position = v.clone(),
                _ => {}
            }
        }
        (enabled, shortcut, position)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scope {
    pub scope: String,
    pub project_id: Option<String>,
}
impl Scope {
    pub fn authorize(&self) -> Result<(), ErrorCode> {
        match self.scope.as_str() {
            "global" if self.project_id.is_none() => Ok(()),
            // Project-scoped writes remain unavailable during phase 2.
            "project"
                if self
                    .project_id
                    .as_ref()
                    .is_some_and(|id| !id.is_empty() && id.len() <= 128) =>
            {
                Err(ErrorCode::PermissionDenied)
            }
            _ => Err(ErrorCode::InvalidRequest),
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Update {
    pub scope: String,
    pub project_id: Option<String>,
    pub expected_revision: i64,
    pub changes: Vec<Change>,
}
impl Update {
    pub fn validate(&self) -> Result<(), ErrorCode> {
        Scope {
            scope: self.scope.clone(),
            project_id: self.project_id.clone(),
        }
        .authorize()?;
        if !(1..9007199254740991).contains(&self.expected_revision)
            || self.changes.is_empty()
            || self.changes.len() > 10
        {
            return Err(ErrorCode::InvalidRequest);
        }
        let mut keys = std::collections::HashSet::new();
        for change in &self.changes {
            if !keys.insert(change.key()) {
                return Err(ErrorCode::InvalidRequest);
            }
            change.validate()?;
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct SettingsState {
    pub operation: Mutex<()>,
    pub failure: Mutex<Option<ErrorCode>>,
    pub initialized: std::sync::atomic::AtomicBool,
}

pub fn get(app: &tauri::AppHandle) -> Result<View, ErrorCode> {
    let state = app.state::<SettingsState>();
    if !state.initialized.load(std::sync::atomic::Ordering::Acquire) {
        return Err(state
            .failure
            .lock()
            .unwrap()
            .unwrap_or(ErrorCode::SettingsStorageFailed));
    }
    app.state::<storage::StorageState>()
        .with_storage(|storage| Ok(storage.settings().read_view()))
        .map_err(|_| ErrorCode::SettingsStorageFailed)?
}

pub(crate) fn project_read_view(
    connection: &rusqlite::Connection,
    project_id: &str,
) -> Result<Vec<(String, crate::project_contract::ProjectSetting)>, ErrorCode> {
    use crate::project_contract::{
        ApprovalPolicy, LogLevel, ProjectAiSelection, ProjectEditor, ProjectSetting,
        ProjectSettingKind, ProjectSort, SearchEngine, SettingSource,
    };

    fn from_global(change: Change) -> Result<ProjectSettingKind, ErrorCode> {
        use crate::project_contract::*;
        Ok(match change {
            Change::SearchEngine(v) => ProjectSettingKind::SearchEngine(match v.as_str() {
                "bing" => SearchEngine::Bing,
                "google" => SearchEngine::Google,
                _ => return Err(ErrorCode::SettingsCorrupt),
            }),
            Change::Editor(v) => ProjectSettingKind::Editor(v.map(|v| ProjectEditor {
                id: v.id,
                executable: v.executable,
            })),
            Change::ProjectSort(v) => ProjectSettingKind::ProjectSort(match v.as_str() {
                "recent" => ProjectSort::Recent,
                "name" => ProjectSort::Name,
                _ => return Err(ErrorCode::SettingsCorrupt),
            }),
            Change::ApprovalPolicy(v) => ProjectSettingKind::ApprovalPolicy(if v == "ask" {
                ApprovalPolicy::Ask
            } else {
                return Err(ErrorCode::SettingsCorrupt);
            }),
            Change::LogLevel(v) => ProjectSettingKind::LogLevel(match v.as_str() {
                "error" => LogLevel::Error,
                "warn" => LogLevel::Warn,
                "info" => LogLevel::Info,
                "debug" => LogLevel::Debug,
                _ => return Err(ErrorCode::SettingsCorrupt),
            }),
            Change::SummaryRetentionDays(v) => ProjectSettingKind::SummaryRetentionDays(
                v.try_into().map_err(|_| ErrorCode::SettingsCorrupt)?,
            ),
            Change::AiDefaults(value) => ProjectSettingKind::AiDefaults(
                value
                    .map(serde_json::from_value::<ProjectAiSelection>)
                    .transpose()
                    .map_err(|_| ErrorCode::SettingsCorrupt)?,
            ),
            Change::OverlayEnabled(_) | Change::OverlayShortcut(_) | Change::OverlayPosition(_) => {
                return Err(ErrorCode::SettingsCorrupt);
            }
        })
    }

    fn project_override(
        connection: &rusqlite::Connection,
        project_id: &str,
        key: &str,
    ) -> Result<Option<ProjectSettingKind>, ErrorCode> {
        let row: Option<(Option<String>, Option<i64>, Option<String>, Option<String>, Option<String>, Option<i64>)> = connection
            .query_row("SELECT s.value_text,s.value_integer,s.profile_id,s.model_id,s.effort,r.profile_revision FROM settings s LEFT JOIN ai_default_revisions r ON r.setting_id=s.id WHERE s.scope='project' AND s.project_id=?1 AND s.key=?2", rusqlite::params![project_id,key], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))
            .optional().map_err(|_| ErrorCode::SettingsStorageFailed)?;
        let Some((text, integer, profile, model, effort, ai_revision)) = row else {
            return Ok(None);
        };
        let corrupt = || ErrorCode::SettingsCorrupt;
        let value = match key {
            "search_engine" => ProjectSettingKind::SearchEngine(match text.as_deref() {
                Some("bing") => SearchEngine::Bing,
                Some("google") => SearchEngine::Google,
                _ => return Err(corrupt()),
            }),
            "editor" => ProjectSettingKind::Editor(Some(
                serde_json::from_str::<ProjectEditor>(&text.ok_or_else(corrupt)?)
                    .map_err(|_| corrupt())?,
            )),
            "project_sort" => ProjectSettingKind::ProjectSort(match text.as_deref() {
                Some("recent") => ProjectSort::Recent,
                Some("name") => ProjectSort::Name,
                _ => return Err(corrupt()),
            }),
            "approval_policy" => {
                ProjectSettingKind::ApprovalPolicy(if text.as_deref() == Some("ask") {
                    ApprovalPolicy::Ask
                } else {
                    return Err(corrupt());
                })
            }
            "log_level" => ProjectSettingKind::LogLevel(match text.as_deref() {
                Some("error") => LogLevel::Error,
                Some("warn") => LogLevel::Warn,
                Some("info") => LogLevel::Info,
                Some("debug") => LogLevel::Debug,
                _ => return Err(corrupt()),
            }),
            "summary_retention_days" => ProjectSettingKind::SummaryRetentionDays(
                integer
                    .ok_or_else(corrupt)?
                    .try_into()
                    .map_err(|_| corrupt())?,
            ),
            "ai_defaults" => ProjectSettingKind::AiDefaults(Some(ProjectAiSelection {
                profile_id: profile.ok_or_else(corrupt)?,
                expected_revision: ai_revision
                    .ok_or_else(corrupt)?
                    .try_into()
                    .map_err(|_| corrupt())?,
                model_id: model.ok_or_else(corrupt)?,
                effort,
            })),
            _ => return Err(corrupt()),
        };
        Ok(Some(value))
    }

    let mut global = storage::settings::read_for_project(connection)?;
    global.values.retain(|v| {
        !matches!(
            v,
            Change::OverlayEnabled(_) | Change::OverlayShortcut(_) | Change::OverlayPosition(_)
        )
    });
    global
        .values
        .into_iter()
        .map(|change| {
            let key = change.key().to_owned();
            let inherited = from_global(change)?;
            let (setting, source) = match project_override(connection, project_id, &key)? {
                Some(value) => (value, SettingSource::Project),
                None => (inherited, SettingSource::Global),
            };
            Ok((key, ProjectSetting { setting, source }))
        })
        .collect()
}

pub fn update(app: &tauri::AppHandle, request: Update, id: String) -> Result<View, ErrorCode> {
    request.validate()?;
    let state = app.state::<SettingsState>();
    let _operation = state
        .operation
        .lock()
        .map_err(|_| ErrorCode::SettingsStorageFailed)?;
    let old = get(app)?;
    let expanded = overlay::view(app).expanded;
    let resident_changed = request.changes.iter().any(|v| {
        matches!(
            v,
            Change::OverlayEnabled(_) | Change::OverlayShortcut(_) | Change::OverlayPosition(_)
        )
    });
    let mut runtime_touched = false;
    let result = app
        .state::<storage::StorageState>()
        .with_storage(|storage| {
            Ok(storage.settings().update_view_with_id(
                request.expected_revision,
                &request.changes,
                &id,
                |next| {
                    if resident_changed {
                        runtime_touched = true;
                        overlay::apply_settings(app, next, expanded)?;
                    }
                    Ok(())
                },
            ))
        })
        .map_err(|_| ErrorCode::SettingsStorageFailed)
        .and_then(|result| result);
    if result.is_err() && runtime_touched && overlay::apply_settings(app, &old, expanded).is_err() {
        *state.failure.lock().unwrap() = Some(ErrorCode::SettingsRecoveryFailed);
        crate::app_state::publish(app, id);
        return Err(ErrorCode::SettingsRecoveryFailed);
    }
    if result.is_ok() {
        *state.failure.lock().unwrap() = None;
        if let Ok(view) = &result {
            for value in &view.values {
                if let Change::LogLevel(level) = value {
                    app.state::<crate::logging::LoggingState>().set_level(level);
                }
            }
        }
    }
    if matches!(result, Err(ErrorCode::ShortcutConflict)) {
        app.state::<overlay::OverlayState>()
            .0
            .lock()
            .unwrap()
            .shortcut_error = Some("PA_SHORTCUT_CONFLICT");
    }
    crate::app_state::publish(app, id);
    result
}

pub fn save_position(app: &tauri::AppHandle) -> Result<(), ErrorCode> {
    let id = crate::ipc::correlation_id();
    let state = app.state::<SettingsState>();
    let _operation = state
        .operation
        .lock()
        .map_err(|_| ErrorCode::SettingsStorageFailed)?;
    if !state.initialized.load(std::sync::atomic::Ordering::Acquire) {
        return Err(ErrorCode::SettingsStorageFailed);
    }
    let Some(position) = overlay::sample_position(app) else {
        return Ok(());
    };
    let result = app
        .state::<storage::StorageState>()
        .with_storage(|storage| {
            let repository = &mut storage.settings();
            let current = repository.read_view();
            Ok(current.and_then(|view| {
                if view.resident().2.as_ref() == Some(&position) {
                    return Ok(false);
                }
                repository
                    .update_view_with_id(
                        view.revision,
                        &[Change::OverlayPosition(Some(position.clone()))],
                        &id,
                        |_| Ok(()),
                    )
                    .map(|_| true)
            }))
        })
        .map_err(|_| ErrorCode::SettingsStorageFailed)
        .and_then(|r| r);
    match result {
        Ok(changed) => {
            if changed {
                overlay::sync_position(app, position);
                crate::app_state::publish(app, id);
            }
            Ok(())
        }
        Err(code) => {
            *state.failure.lock().unwrap() = Some(code);
            crate::app_state::publish(app, id);
            Err(code)
        }
    }
}

pub fn change_resident(app: &tauri::AppHandle, changes: Vec<Change>) -> Result<(), ErrorCode> {
    change_resident_with_id(app, changes, crate::ipc::correlation_id())
}

pub fn change_resident_with_id(
    app: &tauri::AppHandle,
    changes: Vec<Change>,
    id: String,
) -> Result<(), ErrorCode> {
    // Trusted tray/window operations use the same optimistic commit path; retry only
    // their fresh revision, never an edited user form or a failed write.
    for _ in 0..3 {
        let current = get(app)?;
        let request = Update {
            scope: "global".into(),
            project_id: None,
            expected_revision: current.revision,
            changes: changes.clone(),
        };
        match update(app, request, id.clone()) {
            Err(ErrorCode::RevisionConflict) => continue,
            result => return result.map(|_| ()),
        }
    }
    Err(ErrorCode::RevisionConflict)
}

pub fn initialize(app: &tauri::AppHandle, data_dir: &std::path::Path) {
    let result = app
        .state::<storage::StorageState>()
        .with_storage(|storage| Ok(storage.settings().initialize_defaults(data_dir)))
        .map_err(|_| ErrorCode::SettingsStorageFailed)
        .and_then(|r| r);
    app.state::<SettingsState>()
        .initialized
        .store(result.is_ok(), std::sync::atomic::Ordering::Release);
    *app.state::<SettingsState>().failure.lock().unwrap() = result.err();
}

#[cfg(test)]
mod tests;
