//! S3-03 task and session service boundary. No IPC command is registered yet; S3-04
//! supplies the owned-process owner after this durable admission succeeds.
#![allow(dead_code)]
use crate::{
    credentials::CredentialsState,
    ipc::ErrorCode,
    storage::{
        StorageError, StorageState,
        tasks::{ApprovalReceipt, ApprovalRequestInput, TaskReceipt, TaskRecord, TaskStartInput},
    },
};
use serde::Deserialize;
use tauri::Manager;

fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionCreateRequest {
    pub project_id: String,
    pub expected_project_revision: i64,
    pub client_request_id: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionFindRequest {
    pub project_id: String,
    pub client_request_id: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionsListRequest {
    pub project_id: String,
    pub cursor: Option<String>,
    pub limit: Option<u16>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionGetRequest {
    pub session_id: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionResumeRequest {
    pub session_id: String,
    pub expected_revision: i64,
    pub expected_project_revision: i64,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionEndRequest {
    pub session_id: String,
    pub expected_revision: i64,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskStartRequest {
    pub project_id: String,
    pub session_id: String,
    pub client_request_id: String,
    pub expected_project_revision: i64,
    pub expected_session_revision: i64,
    pub expected_project_settings_revision: i64,
    pub expected_global_revision: i64,
    #[serde(deserialize_with = "required_option")]
    pub selection: Option<crate::providers::Selection>,
    pub prompt: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskFindRequest {
    pub project_id: String,
    pub session_id: String,
    pub client_request_id: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TasksListRequest {
    pub session_id: String,
    pub cursor: Option<String>,
    pub limit: Option<u16>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskGetRequest {
    pub task_id: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskCancelRequest {
    pub task_id: String,
    pub expected_revision: i64,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskEventsRequest {
    pub task_id: String,
    pub cursor: Option<String>,
    pub limit: Option<u16>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApprovalsRequest {
    pub task_id: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskTransitionRequest {
    pub task_id: String,
    pub expected_revision: i64,
    pub status: String,
    pub error_code: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPage {
    pub items: Vec<crate::storage::tasks::SessionRecord>,
    pub next_cursor: Option<String>,
}
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskPage {
    pub items: Vec<crate::storage::tasks::TaskRecord>,
    pub next_cursor: Option<String>,
}

fn map_storage(failure: StorageError) -> ErrorCode {
    match failure.code {
        "STORAGE_AUDIT_WRITE_FAILED" => ErrorCode::AuditWriteFailed,
        _ => ErrorCode::SettingsStorageFailed,
    }
}

pub(crate) fn session_create(
    app: &tauri::AppHandle,
    request: SessionCreateRequest,
    correlation: &str,
) -> Result<crate::storage::tasks::SessionReceipt, ErrorCode> {
    app.state::<StorageState>()
        .with_storage(|storage| {
            Ok(storage.tasks().session_create(
                &request.project_id,
                request.expected_project_revision,
                &request.client_request_id,
                correlation,
            ))
        })
        .map_err(map_storage)?
}
pub(crate) fn session_find(
    app: &tauri::AppHandle,
    request: SessionFindRequest,
) -> Result<Option<crate::storage::tasks::SessionRecord>, ErrorCode> {
    app.state::<StorageState>()
        .with_storage(|storage| {
            Ok(storage
                .tasks()
                .session_find_by_request(&request.project_id, &request.client_request_id))
        })
        .map_err(map_storage)?
}
pub(crate) fn sessions_list(
    app: &tauri::AppHandle,
    request: SessionsListRequest,
) -> Result<SessionPage, ErrorCode> {
    let (items, next_cursor) = app
        .state::<StorageState>()
        .with_storage(|storage| {
            Ok(storage.tasks().sessions_list(
                &request.project_id,
                request.cursor.as_deref(),
                request.limit.unwrap_or(50),
            ))
        })
        .map_err(map_storage)??;
    Ok(SessionPage { items, next_cursor })
}
pub(crate) fn session_get(
    app: &tauri::AppHandle,
    request: SessionGetRequest,
) -> Result<crate::storage::tasks::SessionRecord, ErrorCode> {
    app.state::<StorageState>()
        .with_storage(|storage| Ok(storage.tasks().session_get(&request.session_id)))
        .map_err(map_storage)?
}
pub(crate) fn session_resume(
    app: &tauri::AppHandle,
    request: SessionResumeRequest,
    correlation: &str,
) -> Result<crate::storage::tasks::SessionRecord, ErrorCode> {
    app.state::<StorageState>()
        .with_storage(|storage| {
            Ok(storage.tasks().session_resume(
                &request.session_id,
                request.expected_revision,
                request.expected_project_revision,
                correlation,
            ))
        })
        .map_err(map_storage)?
}
pub(crate) fn session_end(
    app: &tauri::AppHandle,
    request: SessionEndRequest,
    correlation: &str,
) -> Result<crate::storage::tasks::SessionRecord, ErrorCode> {
    app.state::<StorageState>()
        .with_storage(|storage| {
            Ok(storage.tasks().session_end(
                &request.session_id,
                request.expected_revision,
                correlation,
            ))
        })
        .map_err(map_storage)?
}
pub(crate) fn start(
    app: &tauri::AppHandle,
    request: TaskStartRequest,
    correlation: &str,
) -> Result<TaskReceipt, ErrorCode> {
    crate::providers::config_state(app)?;
    let credentials = app.state::<CredentialsState>();
    if !credentials.health() {
        return Err(ErrorCode::CredentialsUnavailable);
    }
    let _guard = credentials
        .operation
        .lock()
        .map_err(|_| ErrorCode::CredentialsStorageFailed)?;
    let input = TaskStartInput {
        project_id: request.project_id,
        session_id: request.session_id,
        client_request_id: request.client_request_id,
        expected_project_revision: request.expected_project_revision,
        expected_session_revision: request.expected_session_revision,
        expected_project_settings_revision: request.expected_project_settings_revision,
        expected_global_revision: request.expected_global_revision,
        selection: request.selection,
        prompt: request.prompt,
        origin_window: "main",
    };
    app.state::<StorageState>()
        .with_storage(|storage| {
            Ok(storage.tasks().task_start(
                input,
                correlation,
                crate::credentials::verify_task_credential,
            ))
        })
        .map_err(map_storage)?
}
pub(crate) fn find(
    app: &tauri::AppHandle,
    request: TaskFindRequest,
) -> Result<Option<crate::storage::tasks::TaskRecord>, ErrorCode> {
    app.state::<StorageState>()
        .with_storage(|storage| {
            Ok(storage.tasks().task_find_by_request(
                &request.project_id,
                &request.session_id,
                &request.client_request_id,
            ))
        })
        .map_err(map_storage)?
}
pub(crate) fn list(
    app: &tauri::AppHandle,
    request: TasksListRequest,
) -> Result<TaskPage, ErrorCode> {
    let (items, next_cursor) = app
        .state::<StorageState>()
        .with_storage(|storage| {
            Ok(storage.tasks().tasks_list(
                &request.session_id,
                request.cursor.as_deref(),
                request.limit.unwrap_or(50),
            ))
        })
        .map_err(map_storage)??;
    Ok(TaskPage { items, next_cursor })
}
pub(crate) fn get(
    app: &tauri::AppHandle,
    request: TaskGetRequest,
) -> Result<crate::storage::tasks::TaskRecord, ErrorCode> {
    app.state::<StorageState>()
        .with_storage(|storage| Ok(storage.tasks().task_get(&request.task_id)))
        .map_err(map_storage)?
}
pub(crate) fn cancel(
    app: &tauri::AppHandle,
    request: TaskCancelRequest,
    correlation: &str,
) -> Result<crate::storage::tasks::CancelReceipt, ErrorCode> {
    app.state::<StorageState>()
        .with_storage(|storage| {
            Ok(storage.tasks().task_cancel(
                &request.task_id,
                request.expected_revision,
                correlation,
            ))
        })
        .map_err(map_storage)?
}
pub(crate) fn events(
    app: &tauri::AppHandle,
    request: TaskEventsRequest,
) -> Result<crate::storage::tasks::EventPage, ErrorCode> {
    app.state::<StorageState>()
        .with_storage(|storage| {
            Ok(storage.tasks().task_events_get(
                &request.task_id,
                request.cursor.as_deref(),
                request.limit.unwrap_or(50),
            ))
        })
        .map_err(map_storage)?
}
pub(crate) fn approvals(
    app: &tauri::AppHandle,
    request: ApprovalsRequest,
) -> Result<Vec<crate::storage::tasks::ApprovalRecord>, ErrorCode> {
    app.state::<StorageState>()
        .with_storage(|storage| Ok(storage.tasks().approvals_get(&request.task_id)))
        .map_err(map_storage)?
}

pub(crate) fn transition(
    app: &tauri::AppHandle,
    request: TaskTransitionRequest,
    correlation: &str,
) -> Result<TaskRecord, ErrorCode> {
    app.state::<StorageState>()
        .with_storage(|storage| {
            Ok(storage.tasks().task_transition(
                &request.task_id,
                request.expected_revision,
                &request.status,
                request.error_code.as_deref(),
                correlation,
            ))
        })
        .map_err(map_storage)?
}

pub(crate) fn approval_create(
    app: &tauri::AppHandle,
    request: ApprovalRequestInput,
    correlation: &str,
) -> Result<ApprovalReceipt, ErrorCode> {
    app.state::<StorageState>()
        .with_storage(|storage| Ok(storage.tasks().approval_create(request, correlation)))
        .map_err(map_storage)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_dtos_reject_unknown_fields_and_missing_nullable_selection() {
        let unknown = serde_json::json!({"projectId":"p","expectedProjectRevision":1,"clientRequestId":"00000000-0000-4000-8000-000000000001","extra":true});
        assert!(serde_json::from_value::<SessionCreateRequest>(unknown).is_err());
        let missing_selection = serde_json::json!({
            "projectId":"p","sessionId":"s","clientRequestId":"00000000-0000-4000-8000-000000000001",
            "expectedProjectRevision":1,"expectedSessionRevision":1,"expectedProjectSettingsRevision":1,"expectedGlobalRevision":1,"prompt":"hello"
        });
        assert!(serde_json::from_value::<TaskStartRequest>(missing_selection).is_err());
        let null_selection = serde_json::json!({
            "projectId":"p","sessionId":"s","clientRequestId":"00000000-0000-4000-8000-000000000001",
            "expectedProjectRevision":1,"expectedSessionRevision":1,"expectedProjectSettingsRevision":1,"expectedGlobalRevision":1,"selection":null,"prompt":"hello"
        });
        assert!(serde_json::from_value::<TaskStartRequest>(null_selection).is_ok());
    }
}
