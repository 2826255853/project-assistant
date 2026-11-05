//! Durable admission, owned workers, main-window approval decisions and cancellation.
#![allow(dead_code)]
use crate::{
    app_state,
    credentials::CredentialsState,
    ipc::ErrorCode,
    storage::{
        StorageError, StorageState,
        tasks::{ApprovalReceipt, ApprovalRequestInput, TaskReceipt, TaskRecord, TaskStartInput},
    },
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    path::Path,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tauri::Manager;
pub(crate) mod results;

#[derive(Default)]
pub(crate) struct TaskRuntimeState {
    workers: Mutex<HashMap<String, crate::owned_process::Cancel>>,
    approval_waiters: Mutex<HashMap<String, ApprovalWaiter>>,
    approval_summaries: Mutex<HashMap<String, (String, String)>>,
    outputs: Mutex<HashMap<String, TaskOutput>>,
    completed: Mutex<VecDeque<String>>,
    closing: AtomicBool,
}

struct ApprovalWaiter {
    task_id: String,
    deadline: Instant,
    sender: Sender<crate::owned_process::ApprovalReply>,
}

#[derive(Default)]
struct TaskOutput {
    reply: String,
    progress: String,
    changes: Vec<results::StoredChange>,
}

impl TaskRuntimeState {
    fn append(&self, task_id: &str, channel: &str, text: &str) -> Result<(), ErrorCode> {
        if text.len() > 64 * 1024 || !matches!(channel, "reply" | "progress") {
            return Err(ErrorCode::InputTooLarge);
        }
        let mut outputs = self
            .outputs
            .lock()
            .map_err(|_| ErrorCode::ProcessExitFailed)?;
        let output = outputs.entry(task_id.to_owned()).or_default();
        if output
            .reply
            .len()
            .saturating_add(output.progress.len())
            .saturating_add(text.len())
            > 2 * 1024 * 1024
        {
            return Err(ErrorCode::InputTooLarge);
        }
        match channel {
            "reply" => output.reply.push_str(text),
            "progress" => output.progress.push_str(text),
            _ => return Err(ErrorCode::InvalidRequest),
        }
        Ok(())
    }

    fn has_output(&self, task_id: &str) -> bool {
        self.outputs
            .lock()
            .map(|outputs| outputs.contains_key(task_id))
            .unwrap_or(false)
    }

    pub(crate) fn cancel_task(&self, task_id: &str) -> Result<(), ErrorCode> {
        let workers = self
            .workers
            .lock()
            .map_err(|_| ErrorCode::ProcessExitFailed)?;
        if let Some(cancel) = workers.get(task_id) {
            cancel.cancel();
        }
        Ok(())
    }

    pub(crate) fn cancel_all(&self) {
        self.closing.store(true, Ordering::Release);
        if let Ok(workers) = self.workers.lock() {
            for cancel in workers.values() {
                cancel.cancel();
            }
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if self
                .workers
                .lock()
                .map(|workers| workers.is_empty())
                .unwrap_or(true)
                || Instant::now() >= deadline
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub(crate) fn start_admitted(
        &self,
        app: &tauri::AppHandle,
        task_id: String,
        prompt: String,
        correlation: String,
    ) -> Result<(), ErrorCode> {
        let cancel = crate::owned_process::Cancel::default();
        {
            let mut workers = self
                .workers
                .lock()
                .map_err(|_| ErrorCode::ProcessExitFailed)?;
            if self.closing.load(Ordering::Acquire) {
                return Err(ErrorCode::ProcessExitFailed);
            }
            if workers.contains_key(&task_id) {
                return Err(ErrorCode::TaskConflictRevision);
            }
            workers.insert(task_id.clone(), cancel.clone());
        }
        let app = app.clone();
        let worker_task_id = task_id.clone();
        let worker_prompt = prompt.clone();
        let worker_correlation = correlation.clone();
        let spawned = std::thread::Builder::new()
            .name("pa-owned-task".into())
            .spawn(move || {
                let worker_cancel = cancel.clone();
                let result = run_admitted(
                    &app,
                    &worker_task_id,
                    &worker_correlation,
                    &worker_prompt,
                    cancel,
                );
                if let Err(error) = &result {
                    #[cfg(test)]
                    eprintln!("TASK_WORKER_FAILURE:{error:?}");
                    finish_task_failure(
                        &app,
                        &worker_task_id,
                        &worker_correlation,
                        if worker_cancel.is_cancelled()
                            && !matches!(
                                error,
                                ErrorCode::ProcessExitFailed
                                    | ErrorCode::AuditWriteFailed
                                    | ErrorCode::SettingsStorageFailed
                            )
                        {
                            ErrorCode::ProcessCancelled
                        } else {
                            *error
                        },
                    );
                }
                if let Ok(mut waiters) = app.state::<TaskRuntimeState>().approval_waiters.lock() {
                    waiters.retain(|_, waiter| waiter.task_id != worker_task_id);
                }
                if let Ok(mut completed) = app.state::<TaskRuntimeState>().completed.lock() {
                    completed.push_back(worker_task_id.clone());
                    while completed.len() > 16 {
                        if let Some(expired) = completed.pop_front() {
                            if let Ok(mut outputs) = app.state::<TaskRuntimeState>().outputs.lock()
                            {
                                outputs.remove(&expired);
                            }
                            if let Ok(mut summaries) =
                                app.state::<TaskRuntimeState>().approval_summaries.lock()
                            {
                                summaries.retain(|_, (task_id, _)| task_id != &expired);
                            }
                        }
                    }
                }
                if let Ok(mut workers) = app.state::<TaskRuntimeState>().workers.lock() {
                    workers.remove(&worker_task_id);
                }
            });
        if spawned.is_err() {
            if let Ok(mut workers) = self.workers.lock() {
                workers.remove(&task_id);
            }
            return Err(ErrorCode::ProcessExitFailed);
        }
        Ok(())
    }
}

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
pub struct ApprovalDecideRequest {
    pub approval_id: String,
    pub task_id: String,
    pub expected_revision: i64,
    pub expected_task_revision: i64,
    pub operation_fingerprint: String,
    pub decision: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskTransitionRequest {
    pub task_id: String,
    pub expected_revision: i64,
    pub status: String,
    pub error_code: Option<String>,
}

impl SessionCreateRequest {
    pub(crate) fn validate(&self) -> Result<(), ErrorCode> {
        if self.project_id.is_empty() || self.expected_project_revision < 1 {
            Err(ErrorCode::InvalidRequest)
        } else {
            Ok(())
        }
    }
}
impl SessionFindRequest {
    pub(crate) fn validate(&self) -> Result<(), ErrorCode> {
        if self.project_id.is_empty() || self.client_request_id.is_empty() {
            Err(ErrorCode::InvalidRequest)
        } else {
            Ok(())
        }
    }
}
impl SessionsListRequest {
    pub(crate) fn validate(&self) -> Result<(), ErrorCode> {
        if self.project_id.is_empty()
            || self.limit.unwrap_or(50) == 0
            || self.limit.unwrap_or(50) > 100
        {
            Err(ErrorCode::InvalidRequest)
        } else {
            Ok(())
        }
    }
}
impl SessionGetRequest {
    pub(crate) fn validate(&self) -> Result<(), ErrorCode> {
        if self.session_id.is_empty() {
            Err(ErrorCode::InvalidRequest)
        } else {
            Ok(())
        }
    }
}
impl SessionResumeRequest {
    pub(crate) fn validate(&self) -> Result<(), ErrorCode> {
        if self.session_id.is_empty()
            || self.expected_revision < 1
            || self.expected_project_revision < 1
        {
            Err(ErrorCode::InvalidRequest)
        } else {
            Ok(())
        }
    }
}
impl SessionEndRequest {
    pub(crate) fn validate(&self) -> Result<(), ErrorCode> {
        if self.session_id.is_empty() || self.expected_revision < 1 {
            Err(ErrorCode::InvalidRequest)
        } else {
            Ok(())
        }
    }
}
impl TaskStartRequest {
    pub(crate) fn validate(&self) -> Result<(), ErrorCode> {
        if self.project_id.is_empty()
            || self.session_id.is_empty()
            || self.client_request_id.is_empty()
            || self.prompt.trim().is_empty()
        {
            Err(ErrorCode::InvalidRequest)
        } else {
            Ok(())
        }
    }
}
impl TaskFindRequest {
    pub(crate) fn validate(&self) -> Result<(), ErrorCode> {
        if self.project_id.is_empty()
            || self.session_id.is_empty()
            || self.client_request_id.is_empty()
        {
            Err(ErrorCode::InvalidRequest)
        } else {
            Ok(())
        }
    }
}
impl TasksListRequest {
    pub(crate) fn validate(&self) -> Result<(), ErrorCode> {
        if self.session_id.is_empty()
            || self.limit.unwrap_or(50) == 0
            || self.limit.unwrap_or(50) > 100
        {
            Err(ErrorCode::InvalidRequest)
        } else {
            Ok(())
        }
    }
}
impl TaskGetRequest {
    pub(crate) fn validate(&self) -> Result<(), ErrorCode> {
        if self.task_id.is_empty() {
            Err(ErrorCode::InvalidRequest)
        } else {
            Ok(())
        }
    }
}
impl TaskCancelRequest {
    pub(crate) fn validate(&self) -> Result<(), ErrorCode> {
        if self.task_id.is_empty() || self.expected_revision < 1 {
            Err(ErrorCode::InvalidRequest)
        } else {
            Ok(())
        }
    }
}
impl TaskEventsRequest {
    pub(crate) fn validate(&self) -> Result<(), ErrorCode> {
        if self.task_id.is_empty()
            || self.limit.unwrap_or(50) == 0
            || self.limit.unwrap_or(50) > 100
        {
            Err(ErrorCode::InvalidRequest)
        } else {
            Ok(())
        }
    }
}
impl ApprovalsRequest {
    pub(crate) fn validate(&self) -> Result<(), ErrorCode> {
        if self.task_id.is_empty() {
            Err(ErrorCode::InvalidRequest)
        } else {
            Ok(())
        }
    }
}
impl ApprovalDecideRequest {
    pub(crate) fn validate(&self) -> Result<(), ErrorCode> {
        if self.approval_id.is_empty()
            || self.task_id.is_empty()
            || self.expected_revision < 1
            || self.expected_task_revision < 1
            || !matches!(self.decision.as_str(), "approve" | "deny")
            || self.operation_fingerprint.len() != 64
            || !self
                .operation_fingerprint
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            Err(ErrorCode::InvalidRequest)
        } else {
            Ok(())
        }
    }
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
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalPage {
    pub items: Vec<crate::storage::tasks::ApprovalRecord>,
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
    let prompt = request.prompt.clone();
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
    let receipt = app
        .state::<StorageState>()
        .with_storage(|storage| {
            Ok(storage.tasks().task_start(
                input,
                correlation,
                crate::credentials::verify_task_credential,
            ))
        })
        .map_err(map_storage)??;
    drop(_guard);
    if !receipt.duplicate {
        if let Err(error) = app.state::<TaskRuntimeState>().start_admitted(
            app,
            receipt.task.id.clone(),
            prompt,
            correlation.to_owned(),
        ) {
            let _ = app.state::<StorageState>().with_storage(|storage| {
                Ok(storage.tasks().task_transition(
                    &receipt.task.id,
                    receipt.task.revision,
                    "failed",
                    Some("PA_TASK_RECOVERY_BLOCKED"),
                    correlation,
                ))
            });
            return Err(error);
        }
    }
    Ok(receipt)
}
pub(crate) fn find(
    app: &tauri::AppHandle,
    request: TaskFindRequest,
) -> Result<Option<crate::storage::tasks::TaskRecord>, ErrorCode> {
    let found = app
        .state::<StorageState>()
        .with_storage(|storage| {
            Ok(storage.tasks().task_find_by_request(
                &request.project_id,
                &request.session_id,
                &request.client_request_id,
            ))
        })
        .map_err(map_storage)??;
    Ok(found.map(|mut task| {
        task.content_available = app.state::<TaskRuntimeState>().has_output(&task.id);
        task
    }))
}
pub(crate) fn list(
    app: &tauri::AppHandle,
    request: TasksListRequest,
) -> Result<TaskPage, ErrorCode> {
    let (mut items, next_cursor) = app
        .state::<StorageState>()
        .with_storage(|storage| {
            Ok(storage.tasks().tasks_list(
                &request.session_id,
                request.cursor.as_deref(),
                request.limit.unwrap_or(50),
            ))
        })
        .map_err(map_storage)??;
    for task in &mut items {
        task.content_available = app.state::<TaskRuntimeState>().has_output(&task.id);
    }
    Ok(TaskPage { items, next_cursor })
}
pub(crate) fn get(
    app: &tauri::AppHandle,
    request: TaskGetRequest,
) -> Result<crate::storage::tasks::TaskRecord, ErrorCode> {
    let mut task = app
        .state::<StorageState>()
        .with_storage(|storage| Ok(storage.tasks().task_get(&request.task_id)))
        .map_err(map_storage)??;
    task.content_available = app.state::<TaskRuntimeState>().has_output(&request.task_id);
    Ok(task)
}
pub(crate) fn cancel(
    app: &tauri::AppHandle,
    request: TaskCancelRequest,
    correlation: &str,
) -> Result<crate::storage::tasks::CancelReceipt, ErrorCode> {
    let runtime = app.state::<TaskRuntimeState>();
    let signal = runtime
        .workers
        .lock()
        .map_err(|_| ErrorCode::ProcessExitFailed)?
        .get(&request.task_id)
        .cloned()
        .unwrap_or_default();
    let receipt = signal.request_after(|| {
        let receipt = app
            .state::<StorageState>()
            .with_storage(|storage| {
                Ok(storage.tasks().task_cancel(
                    &request.task_id,
                    request.expected_revision,
                    correlation,
                ))
            })
            .map_err(map_storage)??;
        let accepted = receipt.accepted;
        Ok((receipt, accepted))
    })?;
    app_state::publish(app, correlation.to_owned());
    Ok(receipt)
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
) -> Result<ApprovalPage, ErrorCode> {
    let mut items = app
        .state::<StorageState>()
        .with_storage(|storage| Ok(storage.tasks().approvals_get(&request.task_id)))
        .map_err(map_storage)??;
    let runtime = app.state::<TaskRuntimeState>();
    let summaries = runtime
        .approval_summaries
        .lock()
        .map_err(|_| ErrorCode::ProcessExitFailed)?;
    for item in &mut items {
        if let Some((_, summary)) = summaries.get(&item.id) {
            item.display_summary = summary.clone();
        }
    }
    Ok(ApprovalPage { items })
}

pub(crate) fn approval_decide(
    app: &tauri::AppHandle,
    request: ApprovalDecideRequest,
    correlation: &str,
) -> Result<ApprovalReceipt, ErrorCode> {
    let runtime = app.state::<TaskRuntimeState>();
    let waiters = runtime
        .approval_waiters
        .lock()
        .map_err(|_| ErrorCode::ProcessExitFailed)?;
    let waiter = waiters
        .get(&request.approval_id)
        .ok_or(ErrorCode::ApprovalNotPending)?;
    if waiter.task_id != request.task_id {
        return Err(ErrorCode::PermissionDenied);
    }
    if Instant::now() >= waiter.deadline {
        return Err(ErrorCode::ApprovalExpired);
    }
    let sender = waiter.sender.clone();
    let decision = if request.decision == "approve" {
        crate::storage::tasks::ApprovalDecisionInput::Approve
    } else {
        crate::storage::tasks::ApprovalDecisionInput::Deny
    };
    let mut receipt = app
        .state::<StorageState>()
        .with_storage(|storage| {
            Ok(storage.tasks().approval_decide(
                &request.task_id,
                &request.approval_id,
                request.expected_revision,
                request.expected_task_revision,
                &request.operation_fingerprint,
                "main",
                decision,
                correlation,
            ))
        })
        .map_err(map_storage)??;
    drop(waiters);
    if let Some((_, summary)) = runtime
        .approval_summaries
        .lock()
        .map_err(|_| ErrorCode::ProcessExitFailed)?
        .get(&receipt.approval.id)
    {
        receipt.approval.display_summary = summary.clone();
    }
    let consume_request_id = receipt
        .approval
        .consume_request_id
        .clone()
        .ok_or(ErrorCode::TaskConflictRevision)?;
    let reply = crate::owned_process::ApprovalReply {
        approval_id: receipt.approval.id.clone(),
        consume_request_id,
        accepted: decision == crate::storage::tasks::ApprovalDecisionInput::Approve,
    };
    if sender.send(reply.clone()).is_err() {
        let _ = app.state::<StorageState>().with_storage(|storage| {
            Ok(storage.tasks().approval_acknowledge(
                &receipt.approval.id,
                &reply.consume_request_id,
                false,
                correlation,
            ))
        });
        return Err(ErrorCode::TaskConflictRevision);
    }
    app_state::publish(app, correlation.to_owned());
    Ok(receipt)
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

/// Start an admitted task in the application-owned Codex process. This is an
/// internal service boundary: UI commands cannot bypass admission and the journal.
pub(crate) fn run_admitted(
    app: &tauri::AppHandle,
    task_id: &str,
    correlation: &str,
    prompt: &str,
    cancel: crate::owned_process::Cancel,
) -> Result<crate::owned_process::TaskRun, ErrorCode> {
    if cancel.is_cancelled() {
        return Err(ErrorCode::ProcessCancelled);
    }
    crate::providers::config_state(app)?;
    let owned_state = app.state::<crate::owned_process::OwnedProcessState>();
    if owned_state.is_blocked() {
        return Err(ErrorCode::ProcessExitFailed);
    }
    let _serial_guard = owned_state.lock_serial_cancellable(&cancel)?;
    if owned_state.is_blocked() {
        return Err(ErrorCode::ProcessExitFailed);
    }
    if cancel.is_cancelled() {
        return Err(ErrorCode::ProcessCancelled);
    }
    crate::providers::config_state(app)?;
    let credentials = app.state::<CredentialsState>();
    if !credentials.health() {
        return Err(ErrorCode::CredentialsUnavailable);
    }
    let credential_guard = credentials
        .operation
        .lock()
        .map_err(|_| ErrorCode::CredentialsStorageFailed)?;
    let launch = app
        .state::<StorageState>()
        .with_storage(|storage| Ok(storage.tasks().task_launch(task_id)))
        .map_err(map_storage)??;
    let snapshot = launch.task.snapshot.clone();
    let origin = url::Url::parse(&snapshot.endpoint)
        .map_err(|_| ErrorCode::ProviderEndpointInvalid)?
        .origin()
        .ascii_serialization();
    let profile = app
        .state::<StorageState>()
        .with_storage(|storage| storage.profiles().process_profile(&snapshot.profile_id))
        .map_err(map_storage)?
        .ok_or(ErrorCode::CredentialProfileNotFound)?;
    if !profile.enabled
        || profile.adapter_kind != "self_managed"
        || profile.external_profile_id.is_some()
        || profile.source_kind != "relay"
        || profile.wire_api != "responses"
        || profile.auth_kind != "api_key"
        || profile.config_revision != snapshot.config_revision
        || profile.credential_revision != snapshot.credential_revision
        || profile.endpoint != snapshot.endpoint
        || profile.endpoint_origin != origin
    {
        return Err(ErrorCode::ProviderSharedRouteUnsafe);
    }
    let selection = crate::providers::Selection {
        profile_id: snapshot.profile_id.clone(),
        expected_revision: snapshot.config_revision,
        model_id: snapshot.model_id.clone(),
        effort: snapshot.effort.clone(),
    };
    app.state::<StorageState>()
        .with_storage(|storage| {
            Ok(crate::providers::validate_project_selection(
                &storage.connection,
                &selection,
            ))
        })
        .map_err(map_storage)??;
    let secret = crate::credentials::resolve_for_owned_locked(
        app,
        &snapshot.profile_id,
        snapshot.credential_revision,
        &origin,
    )?;
    drop(credential_guard);
    let root = app
        .state::<crate::owned_process::OwnedProcessState>()
        .root()?;
    let executable = crate::providers::executable()?;
    let mut task_revision = launch.task.revision;
    let mut task_status = launch.task.status.clone();
    let mut content_revision = launch.task.content_revision;
    let mut last_content_event = Instant::now() - Duration::from_secs(1);
    let mut dirty_channels = Vec::<&'static str>::new();
    let mut pending_approval: Option<(
        crate::storage::tasks::ApprovalRecord,
        Receiver<crate::owned_process::ApprovalReply>,
    )> = None;
    let cancel_signal = cancel.clone();
    let mut on_event = |event: crate::owned_process::TaskEvent| -> Result<Option<crate::owned_process::ApprovalReply>, ErrorCode> {
        match event {
            crate::owned_process::TaskEvent::Started {
                external_session_id,
                external_turn_id,
            } => {
                let updated = app
                    .state::<StorageState>()
                    .with_storage(|storage| {
                        Ok(storage.tasks().bind_external_context(
                            task_id,
                            task_revision,
                            &external_session_id,
                            &external_turn_id,
                            correlation,
                        ))
                    })
                    .map_err(map_storage)??;
                task_revision = updated.revision;
                app_state::publish(app, correlation.to_owned());
            }
            crate::owned_process::TaskEvent::ReplyDelta { text } => {
                app.state::<TaskRuntimeState>()
                    .append(task_id, "reply", &text)?;
                if !dirty_channels.contains(&"reply") {
                    dirty_channels.push("reply");
                }
                if last_content_event.elapsed() >= Duration::from_millis(250) {
                    flush_content_events(
                        app,
                        task_id,
                        &mut dirty_channels,
                        &mut content_revision,
                        &mut task_revision,
                        correlation,
                    )?;
                    last_content_event = Instant::now();
                }
            }
            crate::owned_process::TaskEvent::Progress { text } => {
                app.state::<TaskRuntimeState>()
                    .append(task_id, "progress", &text)?;
                if !dirty_channels.contains(&"progress") {
                    dirty_channels.push("progress");
                }
                if last_content_event.elapsed() >= Duration::from_millis(250) {
                    flush_content_events(
                        app,
                        task_id,
                        &mut dirty_channels,
                        &mut content_revision,
                        &mut task_revision,
                        correlation,
                    )?;
                    last_content_event = Instant::now();
                }
            }
            crate::owned_process::TaskEvent::Running => {
                if task_status == "preparing" {
                    let updated = app
                        .state::<StorageState>()
                        .with_storage(|storage| {
                            Ok(storage.tasks().task_transition(
                                task_id,
                                task_revision,
                                "running",
                                None,
                                correlation,
                            ))
                        })
                        .map_err(map_storage)??;
                    task_revision = updated.revision;
                    task_status = updated.status;
                    app_state::publish(app, correlation.to_owned());
                }
            }
            crate::owned_process::TaskEvent::ApprovalRequired {
                item_id: _,
                external_request_id,
                operation_kind,
                operation_fingerprint,
                display_summary,
            } => {
                let mut binding = Sha256::new();
                binding.update(b"project-assistant-approval-v1\0");
                binding.update(operation_fingerprint.as_bytes());
                binding.update([0]);
                binding.update(task_id.as_bytes());
                binding.update([0]);
                binding.update(launch.task.session_id.as_bytes());
                binding.update([0]);
                binding.update(launch.project_identity_value().as_bytes());
                let operation_fingerprint = format!("{:x}", binding.finalize());
                let approval = app
                    .state::<StorageState>()
                    .with_storage(|storage| {
                        Ok(storage.tasks().approval_create(
                            ApprovalRequestInput {
                                task_id: task_id.to_owned(),
                                expected_task_revision: task_revision,
                                external_request_id,
                                operation_kind,
                                operation_fingerprint,
                                window_label: "main",
                            },
                            correlation,
                        ))
                    })
                    .map_err(map_storage)??;
                let waiting = app
                    .state::<StorageState>()
                    .with_storage(|storage| Ok(storage.tasks().task_get_internal(task_id)))
                    .map_err(map_storage)??;
                task_revision = waiting.revision;
                task_status = waiting.status;
                app.state::<TaskRuntimeState>().approval_summaries.lock()
                    .map_err(|_| ErrorCode::ProcessExitFailed)?
                    .insert(approval.approval.id.clone(), (task_id.to_owned(), display_summary));
                if approval.approval.operation_kind == "unknown" {
                    let denied = app.state::<StorageState>().with_storage(|storage| Ok(storage.tasks().approval_decide(
                        task_id, &approval.approval.id, approval.approval.revision, task_revision,
                        &approval.approval.operation_fingerprint, "main", crate::storage::tasks::ApprovalDecisionInput::Deny, correlation,
                    ))).map_err(map_storage)??;
                    let current = app.state::<StorageState>().with_storage(|storage| Ok(storage.tasks().task_get_internal(task_id))).map_err(map_storage)??;
                    task_revision = current.revision;
                    task_status = current.status;
                    return Ok(Some(crate::owned_process::ApprovalReply {
                        approval_id: denied.approval.id, consume_request_id: denied.approval.consume_request_id.ok_or(ErrorCode::ApprovalNotPending)?, accepted: false,
                    }));
                }
                let (sender, receiver) = mpsc::channel();
                app.state::<TaskRuntimeState>().approval_waiters.lock()
                    .map_err(|_| ErrorCode::ProcessExitFailed)?
                    .insert(approval.approval.id.clone(), ApprovalWaiter {
                        task_id: task_id.to_owned(), deadline: Instant::now() + Duration::from_secs(5 * 60), sender,
                    });
                pending_approval = Some((approval.approval, receiver));
                app_state::publish(app, correlation.to_owned());
            }
            crate::owned_process::TaskEvent::ApprovalPoll => {
                let (approval, receiver) = pending_approval.as_ref().ok_or(ErrorCode::ApprovalNotPending)?;
                if crate::providers::now()? >= approval.expires_at {
                    app.state::<StorageState>().with_storage(|storage| Ok(storage.tasks().approval_expire(task_id, &approval.id, correlation))).map_err(map_storage)??;
                    return Err(ErrorCode::ApprovalExpired);
                }
                match receiver.try_recv() {
                    Ok(reply) => {
                        let current = app.state::<StorageState>().with_storage(|storage| {
                            Ok(storage.tasks().approval_verify_claim(task_id, &reply.approval_id, &reply.consume_request_id, correlation))
                        }).map_err(map_storage)??;
                        task_revision = current.revision;
                        task_status = current.status;
                        app.state::<TaskRuntimeState>().approval_waiters.lock()
                            .map_err(|_| ErrorCode::ProcessExitFailed)?.remove(&approval.id);
                        pending_approval = None;
                        return Ok(Some(reply));
                    }
                    Err(TryRecvError::Empty) => {},
                    Err(TryRecvError::Disconnected) => return Err(ErrorCode::ApprovalNotPending),
                }
            }
            crate::owned_process::TaskEvent::ApprovalExpired => {
                let (approval, _) = pending_approval.take().ok_or(ErrorCode::ApprovalNotPending)?;
                app.state::<TaskRuntimeState>().approval_waiters.lock()
                    .map_err(|_| ErrorCode::ProcessExitFailed)?.remove(&approval.id);
                app.state::<StorageState>().with_storage(|storage| Ok(storage.tasks().approval_expire(
                    task_id, &approval.id, correlation,
                ))).map_err(map_storage)??;
                let current = app.state::<StorageState>().with_storage(|storage| Ok(storage.tasks().task_get_internal(task_id))).map_err(map_storage)??;
                task_revision = current.revision;
                task_status = current.status;
                app_state::publish(app, correlation.to_owned());
            }
            crate::owned_process::TaskEvent::ApprovalAcknowledged { approval_id, consume_request_id } => {
                app.state::<StorageState>().with_storage(|storage| Ok(storage.tasks().approval_acknowledge(
                    &approval_id, &consume_request_id, true, correlation,
                ))).map_err(map_storage)??;
                app_state::publish(app, correlation.to_owned());
            }
            crate::owned_process::TaskEvent::OperationCompleted { .. } => {}
            crate::owned_process::TaskEvent::FileChanges { changes } => {
                results::capture(app, task_id, changes)?;
                app_state::publish(app, correlation.to_owned());
            }
            crate::owned_process::TaskEvent::ApprovalAbandoned { approval_id, consume_request_id } => {
                app.state::<StorageState>().with_storage(|storage| Ok(storage.tasks().approval_acknowledge(
                    &approval_id, &consume_request_id, false, correlation,
                ))).map_err(map_storage)??;
                app_state::publish(app, correlation.to_owned());
                return Err(ErrorCode::ApprovalUncertain);
            }
            crate::owned_process::TaskEvent::Completed
            | crate::owned_process::TaskEvent::Cancelled
            | crate::owned_process::TaskEvent::Failed => {
                flush_content_events(
                    app,
                    task_id,
                    &mut dirty_channels,
                    &mut content_revision,
                    &mut task_revision,
                    correlation,
                )?;
                last_content_event = Instant::now();
            }
        }
        Ok(None)
    };
    let result = crate::owned_process::run_task(
        &root,
        &executable,
        &snapshot.profile_id,
        snapshot.config_revision,
        snapshot.credential_revision,
        &snapshot.endpoint,
        &origin,
        &snapshot.model_id,
        snapshot
            .effort
            .as_deref()
            .ok_or(ErrorCode::ProviderSelectionInvalid)?,
        secret,
        Path::new(&launch.project_path),
        &launch.project_identity_value(),
        prompt,
        cancel,
        &mut on_event,
    );
    drop(on_event);
    match &result {
        Ok(run) => {
            let terminal = match run.outcome {
                crate::owned_process::TaskOutcome::Completed => "completed",
                crate::owned_process::TaskOutcome::Cancelled => "cancelled",
                crate::owned_process::TaskOutcome::Failed => "failed",
            };
            let error = (terminal == "failed").then_some("PA_TASK_PROTOCOL_INVALID");
            let current = app
                .state::<StorageState>()
                .with_storage(|storage| Ok(storage.tasks().task_get_internal(task_id)))
                .map_err(map_storage)??;
            if matches!(
                current.status.as_str(),
                "completed" | "failed" | "cancelled" | "interrupted"
            ) {
                app_state::publish(app, correlation.to_owned());
                return result;
            }
            let mut final_revision = current.revision;
            if terminal == "cancelled" && current.status != "cancelling" {
                let cancelling = app
                    .state::<StorageState>()
                    .with_storage(|storage| {
                        Ok(storage.tasks().task_transition(
                            task_id,
                            final_revision,
                            "cancelling",
                            None,
                            correlation,
                        ))
                    })
                    .map_err(map_storage)??;
                final_revision = cancelling.revision;
            }
            let _terminal_record = app
                .state::<StorageState>()
                .with_storage(|storage| {
                    Ok(storage.tasks().task_transition(
                        task_id,
                        final_revision,
                        terminal,
                        error,
                        correlation,
                    ))
                })
                .map_err(map_storage)??;
            app_state::publish(app, correlation.to_owned());
        }
        Err(error) => {
            if *error == ErrorCode::ProcessExitFailed {
                owned_state.block();
            }
            let current = app
                .state::<StorageState>()
                .with_storage(|storage| Ok(storage.tasks().task_get_internal(task_id)))
                .map_err(map_storage)??;
            if (*error == ErrorCode::ProcessCancelled || cancel_signal.is_cancelled())
                && !matches!(
                    error,
                    ErrorCode::ProcessExitFailed
                        | ErrorCode::AuditWriteFailed
                        | ErrorCode::SettingsStorageFailed
                )
            {
                let mut current_revision = current.revision;
                if current.status != "cancelling" {
                    let cancelling = app
                        .state::<StorageState>()
                        .with_storage(|storage| {
                            Ok(storage.tasks().task_transition(
                                task_id,
                                current_revision,
                                "cancelling",
                                None,
                                correlation,
                            ))
                        })
                        .map_err(map_storage)??;
                    current_revision = cancelling.revision;
                }
                let _ = app
                    .state::<StorageState>()
                    .with_storage(|storage| {
                        Ok(storage.tasks().task_transition(
                            task_id,
                            current_revision,
                            "cancelled",
                            None,
                            correlation,
                        ))
                    })
                    .map_err(map_storage)??;
                app_state::publish(app, correlation.to_owned());
                return result;
            }
            let code = match error {
                ErrorCode::ApprovalUncertain => "PA_APPROVAL_UNCERTAIN",
                ErrorCode::ApprovalExpired => "PA_APPROVAL_EXPIRED",
                ErrorCode::CodexRequestTimeout => "PA_CODEX_REQUEST_TIMEOUT",
                ErrorCode::CodexDisconnected => "PA_CODEX_DISCONNECTED",
                ErrorCode::ProcessCancelled => "PA_TASK_RECOVERY_BLOCKED",
                ErrorCode::ProcessExitFailed => "PA_PROCESS_EXIT_FAILED",
                ErrorCode::ProviderSharedRouteUnsafe => "PA_TASK_RECOVERY_BLOCKED",
                ErrorCode::InputTooLarge | ErrorCode::TaskLimitExceeded => "PA_TASK_LIMIT_EXCEEDED",
                ErrorCode::CodexVersionUnsupported | ErrorCode::ProviderTestFailed => {
                    "PA_TASK_PROTOCOL_INVALID"
                }
                _ => "PA_TASK_PROTOCOL_INVALID",
            };
            if !matches!(
                current.status.as_str(),
                "completed" | "failed" | "cancelled" | "interrupted"
            ) {
                let _ = app
                    .state::<StorageState>()
                    .with_storage(|storage| {
                        Ok(storage.tasks().task_transition(
                            task_id,
                            current.revision,
                            "failed",
                            Some(code),
                            correlation,
                        ))
                    })
                    .map_err(map_storage)??;
            }
            app_state::publish(app, correlation.to_owned());
        }
    }
    result
}

fn finish_task_failure(app: &tauri::AppHandle, task_id: &str, correlation: &str, error: ErrorCode) {
    if matches!(
        error,
        ErrorCode::ProcessExitFailed
            | ErrorCode::AuditWriteFailed
            | ErrorCode::SettingsStorageFailed
    ) {
        app.state::<crate::owned_process::OwnedProcessState>()
            .block();
    }
    let Ok(Ok(current)) = app
        .state::<StorageState>()
        .with_storage(|storage| Ok(storage.tasks().task_get_internal(task_id)))
        .map_err(map_storage)
    else {
        return;
    };
    if matches!(
        current.status.as_str(),
        "completed" | "failed" | "cancelled" | "interrupted"
    ) {
        return;
    }
    if error == ErrorCode::ProcessExitFailed {
        app.state::<crate::owned_process::OwnedProcessState>()
            .block();
    }
    if error == ErrorCode::ProcessCancelled
        || (current.status == "cancelling"
            && !matches!(
                error,
                ErrorCode::ProcessExitFailed
                    | ErrorCode::AuditWriteFailed
                    | ErrorCode::SettingsStorageFailed
            ))
    {
        let mut revision = current.revision;
        if current.status != "cancelling" {
            let Ok(Ok(cancelling)) = app
                .state::<StorageState>()
                .with_storage(|storage| {
                    Ok(storage.tasks().task_transition(
                        task_id,
                        revision,
                        "cancelling",
                        None,
                        correlation,
                    ))
                })
                .map_err(map_storage)
            else {
                return;
            };
            revision = cancelling.revision;
        }
        let _ = app.state::<StorageState>().with_storage(|storage| {
            Ok(storage
                .tasks()
                .task_transition(task_id, revision, "cancelled", None, correlation))
        });
        app_state::publish(app, correlation.to_owned());
        return;
    }
    let code = match error {
        ErrorCode::ApprovalUncertain => "PA_APPROVAL_UNCERTAIN",
        ErrorCode::ApprovalExpired => "PA_APPROVAL_EXPIRED",
        ErrorCode::CodexNotInstalled => "PA_CODEX_NOT_INSTALLED",
        ErrorCode::CodexRequestTimeout => "PA_CODEX_REQUEST_TIMEOUT",
        ErrorCode::CodexDisconnected => "PA_CODEX_DISCONNECTED",
        ErrorCode::ProcessExitFailed => "PA_PROCESS_EXIT_FAILED",
        ErrorCode::ProviderCapabilityInvalid => "PA_PROVIDER_CAPABILITY_INVALID",
        ErrorCode::ProviderSelectionInvalid => "PA_PROVIDER_SELECTION_INVALID",
        ErrorCode::CredentialInvalid => "PA_CREDENTIAL_INVALID",
        ErrorCode::CredentialsUnavailable => "PA_CREDENTIALS_UNAVAILABLE",
        ErrorCode::ProjectPathChanged => "PA_PROJECT_PATH_CHANGED",
        ErrorCode::ProjectPathUnavailable => "PA_PROJECT_PATH_UNAVAILABLE",
        ErrorCode::SettingsStorageFailed => "PA_SETTINGS_STORAGE_FAILED",
        ErrorCode::AuditWriteFailed => "PA_AUDIT_WRITE_FAILED",
        ErrorCode::InputTooLarge | ErrorCode::TaskLimitExceeded => "PA_TASK_LIMIT_EXCEEDED",
        ErrorCode::ProviderSharedRouteUnsafe | ErrorCode::ProviderRollbackFailed => {
            "PA_TASK_RECOVERY_BLOCKED"
        }
        _ => "PA_TASK_PROTOCOL_INVALID",
    };
    let _ = app.state::<StorageState>().with_storage(|storage| {
        Ok(storage.tasks().task_transition(
            task_id,
            current.revision,
            "failed",
            Some(code),
            correlation,
        ))
    });
    app_state::publish(app, correlation.to_owned());
}

fn flush_content_events(
    app: &tauri::AppHandle,
    task_id: &str,
    dirty_channels: &mut Vec<&'static str>,
    content_revision: &mut i64,
    task_revision: &mut i64,
    correlation: &str,
) -> Result<(), ErrorCode> {
    let channels = std::mem::take(dirty_channels);
    for channel in channels {
        *content_revision = content_revision
            .checked_add(1)
            .ok_or(ErrorCode::TaskConflictRevision)?;
        let updated = app
            .state::<StorageState>()
            .with_storage(|storage| {
                Ok(storage.tasks().content_changed(
                    task_id,
                    channel,
                    *content_revision,
                    correlation,
                ))
            })
            .map_err(map_storage)??;
        *task_revision = updated.revision;
        app_state::publish(app, correlation.to_owned());
    }
    Ok(())
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

    #[test]
    fn approval_request_cannot_supply_window_summary_operation_or_execution_arguments() {
        let request = serde_json::json!({"approvalId":"a","taskId":"t","expectedRevision":1,"expectedTaskRevision":3,"operationFingerprint":"f".repeat(64),"decision":"approve"});
        assert!(
            serde_json::from_value::<ApprovalDecideRequest>(request.clone())
                .unwrap()
                .validate()
                .is_ok()
        );
        for key in [
            "windowLabel",
            "displaySummary",
            "command",
            "consumeRequestId",
            "cwd",
        ] {
            let mut changed = request.clone();
            changed[key] = serde_json::json!("synthetic-private-value");
            assert!(serde_json::from_value::<ApprovalDecideRequest>(changed).is_err());
        }
        let mut missing_revision = request.clone();
        missing_revision
            .as_object_mut()
            .unwrap()
            .remove("expectedTaskRevision");
        assert!(serde_json::from_value::<ApprovalDecideRequest>(missing_revision).is_err());
        for decision in ["acceptForSession", "acceptWithExecpolicyAmendment", "allow"] {
            let mut changed = request.clone();
            changed["decision"] = serde_json::json!(decision);
            assert!(
                serde_json::from_value::<ApprovalDecideRequest>(changed)
                    .unwrap()
                    .validate()
                    .is_err()
            );
        }
    }
}
