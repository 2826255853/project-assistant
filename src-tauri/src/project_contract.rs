//! Phase 2 project DTOs. S2-03/04/05 wire register/list/get/remove/open/pick; other types remain
//! future contracts until their implementation step is ready.
use serde::{Deserialize, Serialize};

pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
pub const MAX_QUERY_BYTES: usize = 256;
pub const MAX_CURSOR_BYTES: usize = 512;
pub const MAX_RELATIVE_PATH_BYTES: usize = 4 * 1024;
pub const MAX_PAGE_SIZE: u16 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Available,
    Missing,
    Inaccessible,
    IdentityChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Authorization {
    Granted,
    RequiresReauthorization,
    Revoked,
}

/// `absolute_path` is display data for the local main window, never an
/// authorization token or an argument accepted by a project command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectView {
    pub id: String,
    pub name: String,
    pub display_name: Option<String>,
    pub absolute_path: String,
    pub availability: Availability,
    pub authorization: Authorization,
    pub revision: u64,
    pub created_at: u64,
    pub last_used_at: Option<u64>,
}

impl ProjectView {
    /// Checks the wire shape and public bounds only. It deliberately does not
    /// treat a path string as proof of filesystem authorization.
    pub fn validate_wire(&self) -> Result<(), crate::ipc::ErrorCode> {
        if !valid_uuid(&self.id)
            || !bounded_text(&self.name, 1024)
            || self.name.is_empty()
            || self
                .display_name
                .as_deref()
                .is_some_and(|value| !bounded_text(value, 1024))
            || !bounded_text(&self.absolute_path, 4096)
            || self.absolute_path.is_empty()
            || self.revision == 0
            || self.revision > MAX_SAFE_INTEGER
            || self.created_at > MAX_SAFE_INTEGER
            || self
                .last_used_at
                .is_some_and(|value| value > MAX_SAFE_INTEGER)
        {
            return Err(crate::ipc::ErrorCode::InvalidRequest);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectListPage {
    pub items: Vec<ProjectView>,
    pub next_cursor: Option<String>,
}

impl ProjectListPage {
    pub fn validate_wire(&self) -> Result<(), crate::ipc::ErrorCode> {
        let mut ids = std::collections::HashSet::new();
        if self.items.len() > usize::from(MAX_PAGE_SIZE)
            || self
                .items
                .iter()
                .any(|project| project.validate_wire().is_err() || !ids.insert(&project.id))
            || self
                .next_cursor
                .as_deref()
                .is_some_and(|value| !bounded_text(value, MAX_CURSOR_BYTES))
        {
            return Err(crate::ipc::ErrorCode::InvalidRequest);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActiveProject {
    pub project_id: String,
    pub name: String,
}

impl ActiveProject {
    pub fn validate_wire(&self) -> Result<(), crate::ipc::ErrorCode> {
        if !valid_uuid(&self.project_id) || !bounded_text(&self.name, 1024) || self.name.is_empty()
        {
            return Err(crate::ipc::ErrorCode::InvalidRequest);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitSummaryState {
    Available,
    NotRepository,
    GitUnavailable,
    TimedOut,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitSummary {
    pub state: GitSummaryState,
    pub is_repository: Option<bool>,
    pub branch: Option<String>,
    pub detached: bool,
    pub working_tree_modified: Option<bool>,
    pub refreshed_at: Option<u64>,
}

impl GitSummary {
    pub fn validate_wire(&self) -> Result<(), crate::ipc::ErrorCode> {
        let valid = match self.state {
            GitSummaryState::Available => {
                self.is_repository == Some(true)
                    && self
                        .branch
                        .as_deref()
                        .is_none_or(|value| bounded_text(value, 512))
                    && self.working_tree_modified.is_some()
                    && self
                        .refreshed_at
                        .is_some_and(|value| value <= MAX_SAFE_INTEGER)
            }
            GitSummaryState::NotRepository => {
                self.is_repository == Some(false)
                    && self.branch.is_none()
                    && !self.detached
                    && self.working_tree_modified.is_none()
                    && self
                        .refreshed_at
                        .is_some_and(|value| value <= MAX_SAFE_INTEGER)
            }
            GitSummaryState::GitUnavailable
            | GitSummaryState::TimedOut
            | GitSummaryState::Failed => {
                self.is_repository.is_none()
                    && self.branch.is_none()
                    && !self.detached
                    && self.working_tree_modified.is_none()
                    && self
                        .refreshed_at
                        .is_none_or(|value| value <= MAX_SAFE_INTEGER)
            }
        };
        if valid {
            Ok(())
        } else {
            Err(crate::ipc::ErrorCode::InvalidRequest)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingSource {
    Project,
    Global,
}

/// The value is serialized from the existing typed settings value after the
/// project-read allow-list is applied. It does not create a settings writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchEngine {
    Bing,
    Google,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectSort {
    Recent,
    Name,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPolicy {
    Ask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectEditor {
    pub id: String,
    pub executable: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectAiSelection {
    pub profile_id: String,
    pub expected_revision: u64,
    pub model_id: String,
    pub effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "key", content = "value", rename_all = "snake_case")]
pub enum ProjectSettingKind {
    SearchEngine(SearchEngine),
    Editor(Option<ProjectEditor>),
    ProjectSort(ProjectSort),
    ApprovalPolicy(ApprovalPolicy),
    LogLevel(LogLevel),
    SummaryRetentionDays(u16),
    AiDefaults(Option<ProjectAiSelection>),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSetting {
    #[serde(flatten)]
    pub setting: ProjectSettingKind,
    pub source: SettingSource,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSettingsView {
    pub project_id: String,
    pub revision: u64,
    pub global_revision: u64,
    pub values: Vec<ProjectSetting>,
}

impl ProjectSettingsView {
    pub fn validate_wire(&self) -> Result<(), crate::ipc::ErrorCode> {
        if !valid_uuid(&self.project_id)
            || self.revision == 0
            || self.global_revision == 0
            || self.revision > MAX_SAFE_INTEGER
            || self.global_revision > MAX_SAFE_INTEGER
            || self.values.len() > 16
            || self
                .values
                .iter()
                .any(|setting| setting.validate().is_err())
            || self.values.iter().enumerate().any(|(index, setting)| {
                self.values[..index]
                    .iter()
                    .any(|previous| previous.setting.key() == setting.setting.key())
            })
        {
            return Err(crate::ipc::ErrorCode::InvalidRequest);
        }
        Ok(())
    }
}

impl ProjectSettingKind {
    fn key(&self) -> &'static str {
        match self {
            Self::SearchEngine(_) => "search_engine",
            Self::Editor(_) => "editor",
            Self::ProjectSort(_) => "project_sort",
            Self::ApprovalPolicy(_) => "approval_policy",
            Self::LogLevel(_) => "log_level",
            Self::SummaryRetentionDays(_) => "summary_retention_days",
            Self::AiDefaults(_) => "ai_defaults",
        }
    }
}

impl ProjectSetting {
    fn validate(&self) -> Result<(), crate::ipc::ErrorCode> {
        let valid = match &self.setting {
            ProjectSettingKind::Editor(Some(editor)) => {
                let path = std::path::Path::new(&editor.executable);
                editor.id == "custom"
                    && editor.executable.len() <= 1024
                    && path.is_absolute()
                    && editor.executable.to_ascii_lowercase().ends_with(".exe")
                    && !editor
                        .executable
                        .chars()
                        .any(|c| c.is_control() || matches!(c, '"' | '<' | '>' | '|' | '*' | '?'))
                    && !path.components().any(|component| {
                        matches!(
                            component,
                            std::path::Component::ParentDir | std::path::Component::CurDir
                        )
                    })
            }
            ProjectSettingKind::AiDefaults(Some(selection)) => {
                crate::providers::valid_id(&selection.profile_id)
                    && (1..=MAX_SAFE_INTEGER).contains(&selection.expected_revision)
                    && crate::providers::valid_model(&selection.model_id)
                    && selection
                        .effort
                        .as_ref()
                        .is_none_or(|value| crate::providers::valid_model(value))
            }
            ProjectSettingKind::SummaryRetentionDays(value) => (1..=365).contains(value),
            _ => true,
        };
        if valid {
            Ok(())
        } else {
            Err(crate::ipc::ErrorCode::InvalidRequest)
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EmptyRequest {}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListRequest {
    #[serde(default, deserialize_with = "present_value")]
    pub query: Option<String>,
    #[serde(default, deserialize_with = "present_value")]
    pub cursor: Option<String>,
    #[serde(default, deserialize_with = "present_value")]
    pub limit: Option<u16>,
}

fn present_value<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

impl ListRequest {
    pub fn validate(&self) -> Result<(), crate::ipc::ErrorCode> {
        if self
            .query
            .as_deref()
            .is_some_and(|value| value.len() > MAX_QUERY_BYTES)
            || self
                .cursor
                .as_deref()
                .is_some_and(|value| value.len() > MAX_CURSOR_BYTES)
        {
            return Err(crate::ipc::ErrorCode::InputTooLarge);
        }
        if self
            .query
            .as_deref()
            .is_some_and(|value| value.chars().any(char::is_control))
            || self
                .cursor
                .as_deref()
                .is_some_and(|value| value.is_empty() || value.chars().any(char::is_control))
        {
            return Err(crate::ipc::ErrorCode::InvalidRequest);
        }
        if self
            .limit
            .is_some_and(|value| !(1..=MAX_PAGE_SIZE).contains(&value))
        {
            return Err(crate::ipc::ErrorCode::InvalidRequest);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectIdRequest {
    pub project_id: String,
}

impl ProjectIdRequest {
    pub fn validate(&self) -> Result<(), crate::ipc::ErrorCode> {
        if !valid_uuid(&self.project_id) {
            return Err(crate::ipc::ErrorCode::InvalidRequest);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoveRequest {
    pub project_id: String,
    pub expected_revision: u64,
}

impl RemoveRequest {
    pub fn validate(&self) -> Result<(), crate::ipc::ErrorCode> {
        ProjectIdRequest {
            project_id: self.project_id.clone(),
        }
        .validate()?;
        if self.expected_revision == 0 || self.expected_revision > MAX_SAFE_INTEGER {
            return Err(crate::ipc::ErrorCode::InvalidRequest);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenFileRequest {
    pub project_id: String,
    pub relative_path: String,
}

impl OpenFileRequest {
    pub fn validate(&self) -> Result<(), crate::ipc::ErrorCode> {
        ProjectIdRequest {
            project_id: self.project_id.clone(),
        }
        .validate()?;
        if self.relative_path.len() > MAX_RELATIVE_PATH_BYTES {
            return Err(crate::ipc::ErrorCode::InputTooLarge);
        }
        if !valid_relative_path(&self.relative_path) {
            return Err(crate::ipc::ErrorCode::ProjectFileOutsideProject);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitSummaryRequest {
    pub project_id: String,
    #[serde(default, deserialize_with = "present_value")]
    pub refresh: Option<bool>,
}

impl GitSummaryRequest {
    pub fn validate(&self) -> Result<(), crate::ipc::ErrorCode> {
        ProjectIdRequest {
            project_id: self.project_id.clone(),
        }
        .validate()
    }
}

fn bounded_text(value: &str, max_bytes: usize) -> bool {
    value.len() <= max_bytes && !value.chars().any(char::is_control)
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            14 => byte == b'4',
            19 => matches!(byte, b'8' | b'9' | b'a' | b'b' | b'A' | b'B'),
            _ => byte.is_ascii_hexdigit(),
        })
}

pub(crate) fn valid_relative_path(value: &str) -> bool {
    if value.is_empty()
        || !bounded_text(value, MAX_RELATIVE_PATH_BYTES)
        || value.starts_with('\\')
        || value.starts_with('/')
        || value.contains(':')
        || value.split(['\\', '/']).any(|part| {
            part == ".." || part == "." || part.is_empty() || !valid_windows_component(part)
        })
    {
        return false;
    }
    !std::path::Path::new(value).components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        )
    })
}

fn valid_windows_component(value: &str) -> bool {
    if value.ends_with('.')
        || value.ends_with(' ')
        || value
            .chars()
            .any(|character| matches!(character, '<' | '>' | '"' | '|' | '?' | '*'))
    {
        return false;
    }
    let base = value
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    !matches!(
        base.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) && !["COM", "LPT"].iter().any(|prefix| {
        base.strip_prefix(prefix).is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum PickerOutcome {
    Registered { project: ProjectView },
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveResult {
    pub removed: bool,
    pub retained_for_history: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenResult {
    pub system_accepted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum FilePickerResult {
    Selected {
        #[serde(rename = "relativePath")]
        relative_path: String,
    },
    Cancelled {
        outcome: &'static str,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn project_dto_json_is_camel_case_and_does_not_expose_identity() {
        let view = ProjectView {
            id: "c14e7b09-b84c-44bc-9b57-a7182e70b056".into(),
            name: "sample".into(),
            display_name: None,
            absolute_path: "C:\\work\\sample".into(),
            availability: Availability::Available,
            authorization: Authorization::Granted,
            revision: 1,
            created_at: 10,
            last_used_at: None,
        };
        assert_eq!(
            serde_json::to_value(view).unwrap(),
            json!({
                "id": "c14e7b09-b84c-44bc-9b57-a7182e70b056",
                "name": "sample",
                "displayName": null,
                "absolutePath": "C:\\work\\sample",
                "availability": "available",
                "authorization": "granted",
                "revision": 1,
                "createdAt": 10,
                "lastUsedAt": null
            })
        );
        assert!(
            ProjectView {
                id: "c14e7b09-b84c-44bc-9b57-a7182e70b056".into(),
                name: "sample".into(),
                display_name: None,
                absolute_path: "C:\\work\\sample".into(),
                availability: Availability::Available,
                authorization: Authorization::Granted,
                revision: 1,
                created_at: 10,
                last_used_at: None,
            }
            .validate_wire()
            .is_ok()
        );
    }

    #[test]
    fn requests_reject_frontend_paths_and_unknown_authorization_fields() {
        assert!(serde_json::from_value::<EmptyRequest>(json!({ "path": "C:\\anywhere" })).is_err());
        assert!(
            serde_json::from_value::<ProjectIdRequest>(json!({
                "projectId": "synthetic-id",
                "authorized": true
            }))
            .is_err()
        );
        let id = "c14e7b09-b84c-44bc-9b57-a7182e70b056";
        assert_eq!(
            serde_json::from_value::<GitSummaryRequest>(json!({ "projectId": id }))
                .unwrap()
                .refresh,
            None
        );
        assert_eq!(
            serde_json::from_value::<GitSummaryRequest>(json!({
                "projectId": id,
                "refresh": false
            }))
            .unwrap()
            .refresh,
            Some(false)
        );
        assert!(
            serde_json::from_value::<GitSummaryRequest>(json!({
                "projectId": id,
                "refresh": null
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<GitSummaryRequest>(json!({
                "projectId": id,
                "refresh": "true"
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<GitSummaryRequest>(json!({
                "projectId": id,
                "rawOutput": true
            }))
            .is_err()
        );
    }

    #[test]
    fn request_bounds_reject_bad_paging_ids_revisions_and_relative_paths() {
        let id = "c14e7b09-b84c-44bc-9b57-a7182e70b056".to_owned();
        assert!(
            ListRequest {
                query: Some(String::new()),
                cursor: None,
                limit: None,
            }
            .validate()
            .is_ok()
        );
        assert!(matches!(
            ListRequest {
                query: Some("x".repeat(MAX_QUERY_BYTES + 1)),
                cursor: None,
                limit: None,
            }
            .validate(),
            Err(crate::ipc::ErrorCode::InputTooLarge)
        ));
        assert!(
            ListRequest {
                query: None,
                cursor: None,
                limit: Some(0),
            }
            .validate()
            .is_err()
        );
        assert!(
            ProjectIdRequest {
                project_id: "not-a-uuid".into(),
            }
            .validate()
            .is_err()
        );
        assert!(
            RemoveRequest {
                project_id: id.clone(),
                expected_revision: MAX_SAFE_INTEGER + 1,
            }
            .validate()
            .is_err()
        );
        assert!(
            OpenFileRequest {
                project_id: id.clone(),
                relative_path: "src/main.rs".into(),
            }
            .validate()
            .is_ok()
        );
        for path in [
            "../outside.txt",
            "src/../../outside.txt",
            "C:\\outside.txt",
            "\\\\server\\share\\file.txt",
            "src/secret.txt:stream",
            "src//file.txt",
            "src/./file.txt",
            "src/NUL.txt",
            "src/con",
            "src/trailing.",
            "src/bad?.txt",
        ] {
            assert!(matches!(
                OpenFileRequest {
                    project_id: id.clone(),
                    relative_path: path.into(),
                }
                .validate(),
                Err(crate::ipc::ErrorCode::ProjectFileOutsideProject)
            ));
        }
    }

    #[test]
    fn picker_cancel_has_no_project_payload() {
        assert_eq!(
            serde_json::to_value(PickerOutcome::Cancelled).unwrap(),
            json!({ "outcome": "cancelled" })
        );
    }

    #[test]
    fn project_settings_report_the_source_of_each_value() {
        let setting = ProjectSetting {
            setting: ProjectSettingKind::SearchEngine(SearchEngine::Bing),
            source: SettingSource::Global,
        };
        assert_eq!(
            serde_json::to_value(setting).unwrap(),
            json!({ "key": "search_engine", "value": "bing", "source": "global" })
        );
        let settings = ProjectSettingsView {
            project_id: "c14e7b09-b84c-44bc-9b57-a7182e70b056".into(),
            revision: 1,
            global_revision: 1,
            values: vec![ProjectSetting {
                setting: ProjectSettingKind::SearchEngine(SearchEngine::Bing),
                source: SettingSource::Global,
            }],
        };
        assert!(settings.validate_wire().is_ok());
        let duplicate = ProjectSettingsView {
            values: vec![settings.values[0].clone(), settings.values[0].clone()],
            ..settings
        };
        assert!(duplicate.validate_wire().is_err());
    }

    #[test]
    fn git_degradation_is_data_and_has_no_raw_output_field() {
        let summary = GitSummary {
            state: GitSummaryState::GitUnavailable,
            is_repository: None,
            branch: None,
            detached: false,
            working_tree_modified: None,
            refreshed_at: None,
        };
        let wire = serde_json::to_value(&summary).unwrap();
        assert_eq!(wire["state"], "git_unavailable");
        assert_eq!(wire["branch"], serde_json::Value::Null);
        assert!(wire.get("stderr").is_none());
        assert!(summary.validate_wire().is_ok());
        assert!(
            GitSummary {
                state: GitSummaryState::Available,
                is_repository: Some(true),
                branch: None,
                detached: false,
                working_tree_modified: None,
                refreshed_at: Some(1),
            }
            .validate_wire()
            .is_err()
        );
    }

    #[test]
    fn project_errors_are_fixed_and_never_include_path_details() {
        let codes = [
            crate::ipc::ErrorCode::ProjectNotFound,
            crate::ipc::ErrorCode::ProjectPickerFailed,
            crate::ipc::ErrorCode::ProjectPathUnsafe,
            crate::ipc::ErrorCode::ProjectPathUnavailable,
            crate::ipc::ErrorCode::ProjectPathChanged,
            crate::ipc::ErrorCode::ProjectFileOutsideProject,
            crate::ipc::ErrorCode::ProjectFileMissing,
            crate::ipc::ErrorCode::ProjectAccessDenied,
            crate::ipc::ErrorCode::ProjectOpenRejected,
            crate::ipc::ErrorCode::ProjectOpenTimeout,
            crate::ipc::ErrorCode::ProjectConflictRevision,
        ];
        for code in codes {
            let wire = serde_json::to_value(code).unwrap();
            let public = code.public();
            assert!(wire.as_str().unwrap().starts_with("PA_PROJECT_"));
            assert!(!public.message.contains("C:\\private"));
            assert!(!public.suggestion.contains("C:\\private"));
        }
    }
}
