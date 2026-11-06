//! Process-only authority for the resident entry. Lock order is credentials,
//! app state, then storage, matching project selection's state/storage order.
use super::*;
use serde::{Deserialize, Serialize};

const MAX_REVISION: i64 = 9_007_199_254_740_991;

#[derive(Default)]
pub(crate) struct ActiveState {
    pub revision: i64,
    context: Option<Context>,
    task_id: Option<String>,
    request_id: Option<String>,
}

#[derive(Clone, PartialEq)]
struct Context {
    project_id: String,
    session_id: String,
    project_name: String,
    selection: crate::providers::Selection,
    requested_selection: Option<crate::providers::Selection>,
    revisions: (i64, i64, i64, i64),
    evidence: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActiveTaskContextView {
    context_revision: i64,
    project_id: String,
    project_name: String,
    session_id: String,
    selection: crate::providers::Selection,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContextState {
    pub context_revision: i64,
    pub context: Option<ActiveTaskContextView>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OverlayTaskView {
    task_id: String,
    status: String,
    revision: i64,
    reply_preview: String,
    content_available: bool,
    approval_pending: bool,
    error_code: Option<ErrorCode>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OverlayStateView {
    context_revision: i64,
    context: Option<ActiveTaskContextView>,
    task: Option<OverlayTaskView>,
    pending_request_id: Option<String>,
}
#[derive(Serialize)]
pub(crate) struct OverlayTaskReceipt {
    task: OverlayTaskView,
    duplicate: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ActiveContextSetRequest {
    #[serde(deserialize_with = "required_option")]
    project_id: Option<String>,
    #[serde(deserialize_with = "required_option")]
    session_id: Option<String>,
    #[serde(deserialize_with = "required_option")]
    selection: Option<crate::providers::Selection>,
    expected_context_revision: i64,
}
impl ActiveContextSetRequest {
    pub(crate) fn validate(&self) -> Result<(), ErrorCode> {
        if !(0..=MAX_REVISION).contains(&self.expected_context_revision)
            || self.project_id.is_some() != self.session_id.is_some()
            || self
                .project_id
                .as_ref()
                .is_some_and(|v| validate_id(v).is_err())
            || self
                .session_id
                .as_ref()
                .is_some_and(|v| validate_id(v).is_err())
            || (self.project_id.is_none() && self.selection.is_some())
        {
            return Err(ErrorCode::InvalidRequest);
        }
        if let Some(selection) = &self.selection {
            selection.validate()?;
        }
        Ok(())
    }
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OverlayTaskStartRequest {
    expected_context_revision: i64,
    client_request_id: String,
    prompt: String,
}
impl OverlayTaskStartRequest {
    pub(crate) fn validate(&self) -> Result<(), ErrorCode> {
        validate_revision(self.expected_context_revision)?;
        validate_id(&self.client_request_id)?;
        crate::storage::tasks::validate_prompt_for_ipc(&self.prompt)
    }
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OverlayTaskFindRequest {
    expected_context_revision: i64,
    client_request_id: String,
}
impl OverlayTaskFindRequest {
    pub(crate) fn validate(&self) -> Result<(), ErrorCode> {
        validate_revision(self.expected_context_revision)?;
        validate_id(&self.client_request_id)
    }
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OverlayTaskCancelRequest {
    expected_context_revision: i64,
    expected_task_revision: i64,
}
impl OverlayTaskCancelRequest {
    pub(crate) fn validate(&self) -> Result<(), ErrorCode> {
        validate_revision(self.expected_context_revision)?;
        validate_revision(self.expected_task_revision)
    }
}
fn validate_revision(value: i64) -> Result<(), ErrorCode> {
    if (1..=MAX_REVISION).contains(&value) {
        Ok(())
    } else {
        Err(ErrorCode::InvalidRequest)
    }
}
fn validate_id(value: &str) -> Result<(), ErrorCode> {
    crate::project_contract::ProjectIdRequest {
        project_id: value.to_owned(),
    }
    .validate()
}

fn checked_context(
    app: &tauri::AppHandle,
    project: &str,
    session: &str,
    requested: Option<&crate::providers::Selection>,
) -> Result<Context, ErrorCode> {
    if !app.state::<CredentialsState>().health()
        || app
            .state::<crate::owned_process::OwnedProcessState>()
            .is_blocked()
    {
        return Err(ErrorCode::TaskRecoveryBlocked);
    }
    app.state::<StorageState>()
        .with_storage(|storage| {
            Ok((|| {
                let (p, s, ps, g, name) = storage.tasks().context_state(project, session)?;
                let (selection, preview, evidence) =
                    crate::providers::snapshots::preview(&storage.connection, requested, project)?;
                crate::credentials::verify_task_credential(&storage.connection, &preview)?;
                Ok(Context {
                    project_id: project.into(),
                    session_id: session.into(),
                    project_name: name,
                    selection,
                    requested_selection: requested.cloned(),
                    revisions: (p, s, ps, g),
                    evidence,
                })
            })())
        })
        .map_err(map_storage)?
}
fn view(context: &Context, revision: i64) -> ActiveTaskContextView {
    ActiveTaskContextView {
        context_revision: revision,
        project_id: context.project_id.clone(),
        project_name: context.project_name.clone(),
        session_id: context.session_id.clone(),
        selection: context.selection.clone(),
    }
}
fn current(state: &app_state::StateData, expected: i64) -> Result<Context, ErrorCode> {
    if state.task_context.revision != expected {
        return Err(ErrorCode::TaskContextStale);
    }
    let context = state
        .task_context
        .context
        .clone()
        .ok_or(ErrorCode::TaskContextStale)?;
    if state.active_project.as_ref().map(|p| p.project_id.as_str())
        != Some(context.project_id.as_str())
    {
        return Err(ErrorCode::TaskContextStale);
    }
    Ok(context)
}

pub(crate) fn set_active_context(
    app: &tauri::AppHandle,
    request: ActiveContextSetRequest,
) -> Result<ContextState, ErrorCode> {
    let credentials = app.state::<CredentialsState>();
    let _guard = credentials
        .operation
        .lock()
        .map_err(|_| ErrorCode::CredentialsStorageFailed)?;
    app_state::with_state(app, |state| {
        if state.task_context.revision != request.expected_context_revision {
            return Err(ErrorCode::TaskContextStale);
        }
        let next = match (&request.project_id, &request.session_id) {
            (Some(project), Some(session)) => Some(checked_context(
                app,
                project,
                session,
                request.selection.as_ref(),
            )?),
            (None, None) => None,
            _ => return Err(ErrorCode::InvalidRequest),
        };
        if state.task_context.context != next
            || next.as_ref().is_some_and(|c| {
                state.active_project.as_ref().map(|p| &p.project_id) != Some(&c.project_id)
            })
        {
            state.task_context.revision = state
                .task_context
                .revision
                .checked_add(1)
                .filter(|r| *r <= MAX_REVISION)
                .ok_or(ErrorCode::TaskContextStale)?;
            state.task_context.task_id = None;
            state.task_context.request_id = None;
            state.task_context.context = next;
            state.focus_task_id = None;
            if let Some(context) = &state.task_context.context {
                state.task_context.task_id = app
                    .state::<StorageState>()
                    .with_storage(|storage| {
                        Ok(storage.tasks().tasks_list(&context.session_id, None, 1))
                    })
                    .map_err(map_storage)??
                    .0
                    .first()
                    .map(|task| task.id.clone());
            }
            state.active_context =
                state
                    .task_context
                    .context
                    .as_ref()
                    .map(|context| app_state::ActiveContext {
                        context_revision: state.task_context.revision,
                        project_id: context.project_id.clone(),
                        session_id: context.session_id.clone(),
                        profile_id: context.selection.profile_id.clone(),
                        model_id: context.selection.model_id.clone(),
                        effort: context.selection.effort.clone(),
                    });
            if let Some(context) = &state.task_context.context {
                state.active_project = Some(crate::project_contract::ActiveProject {
                    project_id: context.project_id.clone(),
                    name: context.project_name.clone(),
                });
            }
            app_state::publish_locked(app, state, crate::ipc::correlation_id());
        }
        Ok(ContextState {
            context_revision: state.task_context.revision,
            context: state
                .task_context
                .context
                .as_ref()
                .map(|c| view(c, state.task_context.revision)),
        })
    })
}

pub(crate) fn invalidate_for_project(state: &mut app_state::StateData) {
    if state.task_context.context.as_ref().is_some_and(|c| {
        state.active_project.as_ref().map(|p| &p.project_id) != Some(&c.project_id)
    }) {
        state.task_context.revision = state.task_context.revision.saturating_add(1);
        state.task_context.context = None;
        state.task_context.task_id = None;
        state.task_context.request_id = None;
        state.active_context = None;
        state.focus_task_id = None;
    }
}

pub(crate) fn bind_task(state: &mut app_state::StateData, task: &TaskRecord) {
    if state
        .task_context
        .context
        .as_ref()
        .is_some_and(|c| c.project_id == task.project_id && c.session_id == task.session_id)
    {
        state.task_context.task_id = Some(task.id.clone());
    }
}

pub(crate) fn focus_current_task(app: &tauri::AppHandle) -> Result<(), ErrorCode> {
    app_state::with_state(app, |state| {
        let Some(task_id) = state.task_context.task_id.clone() else {
            return Ok(());
        };
        state.focus_task_id = Some(task_id);
        state.focus_task_revision = state
            .focus_task_revision
            .checked_add(1)
            .filter(|r| *r <= MAX_REVISION)
            .ok_or(ErrorCode::TaskContextStale)?;
        app_state::publish_locked(app, state, crate::ipc::correlation_id());
        Ok(())
    })
}
fn task_view(
    app: &tauri::AppHandle,
    context: &Context,
    task_id: &str,
) -> Result<OverlayTaskView, ErrorCode> {
    let task = get(
        app,
        TaskGetRequest {
            task_id: task_id.into(),
        },
    )?;
    if task.session_id != context.session_id || task.project_id != context.project_id {
        return Err(ErrorCode::TaskContextStale);
    }
    let runtime = app.state::<TaskRuntimeState>();
    let outputs = runtime
        .outputs
        .lock()
        .map_err(|_| ErrorCode::TaskRecoveryBlocked)?;
    let reply_preview = outputs
        .get(task_id)
        .map(|o| bounded_preview(&o.reply))
        .unwrap_or_default();
    Ok(OverlayTaskView {
        task_id: task.id,
        approval_pending: task.status == "approval_required",
        status: task.status,
        revision: task.revision,
        reply_preview,
        content_available: task.content_available,
        error_code: task.error.map(|e| e.code),
    })
}
fn bounded_preview(text: &str) -> String {
    let mut end = text.len().min(512);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}
pub(crate) fn overlay_get(app: &tauri::AppHandle) -> Result<OverlayStateView, ErrorCode> {
    let credentials = app.state::<CredentialsState>();
    let _guard = credentials
        .operation
        .lock()
        .map_err(|_| ErrorCode::CredentialsStorageFailed)?;
    app_state::with_state(app, |state| {
        let context = current(state, state.task_context.revision).ok();
        let available = context.as_ref().filter(|c| {
            checked_context(
                app,
                &c.project_id,
                &c.session_id,
                c.requested_selection.as_ref(),
            )
            .is_ok_and(|fresh| fresh == **c)
        });
        let task = match (&context, &state.task_context.task_id) {
            (Some(c), Some(id)) => task_view(app, c, id).ok(),
            _ => None,
        };
        Ok(OverlayStateView {
            context_revision: state.task_context.revision,
            context: available.map(|c| view(c, state.task_context.revision)),
            task,
            pending_request_id: state.task_context.request_id.clone(),
        })
    })
}
pub(crate) fn start_from_active(
    app: &tauri::AppHandle,
    request: OverlayTaskStartRequest,
    correlation: &str,
) -> Result<OverlayTaskReceipt, ErrorCode> {
    let credentials = app.state::<CredentialsState>();
    let _guard = credentials
        .operation
        .lock()
        .map_err(|_| ErrorCode::CredentialsStorageFailed)?;
    app_state::with_state(app, |state| {
        let context = current(state, request.expected_context_revision)?;
        let fresh = checked_context(
            app,
            &context.project_id,
            &context.session_id,
            context.requested_selection.as_ref(),
        )
        .map_err(|_| ErrorCode::TaskContextStale)?;
        if fresh != context {
            return Err(ErrorCode::TaskContextStale);
        }
        let existing = find(
            app,
            TaskFindRequest {
                project_id: context.project_id.clone(),
                session_id: context.session_id.clone(),
                client_request_id: request.client_request_id.clone(),
            },
        )?;
        if existing.is_some()
            && state.task_context.request_id.as_ref() != Some(&request.client_request_id)
        {
            return Err(ErrorCode::TaskContextStale);
        }
        state.task_context.request_id = Some(request.client_request_id.clone());
        let (p, s, ps, g) = context.revisions;
        let request_id = request.client_request_id.clone();
        let result = admit(
            app,
            TaskStartRequest {
                project_id: context.project_id.clone(),
                session_id: context.session_id.clone(),
                client_request_id: request_id.clone(),
                expected_project_revision: p,
                expected_session_revision: s,
                expected_project_settings_revision: ps,
                expected_global_revision: g,
                selection: context.requested_selection.clone(),
                prompt: request.prompt,
            },
            "overlay",
            correlation,
        );
        let receipt = match result {
            Ok(receipt) => receipt,
            Err(error) => {
                let committed = find(
                    app,
                    TaskFindRequest {
                        project_id: context.project_id.clone(),
                        session_id: context.session_id.clone(),
                        client_request_id: request_id,
                    },
                )?
                .is_some();
                if !committed {
                    state.task_context.request_id = None;
                }
                return Err(error);
            }
        };
        state.task_context.request_id = None;
        bind_task(state, &receipt.task);
        // The draft is private process memory. Clear it at acceptance too, so a
        // lost IPC reply followed by WebView reconstruction cannot offer resend.
        app.state::<crate::overlay::OverlayState>()
            .0
            .lock()
            .map_err(|_| ErrorCode::TaskRecoveryBlocked)?
            .draft
            .clear();
        app_state::publish_locked(app, state, correlation.into());
        Ok(OverlayTaskReceipt {
            task: task_view(app, &context, &receipt.task.id)?,
            duplicate: receipt.duplicate,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overlay_requests_require_explicit_nullable_context_and_bound_inputs() {
        let request: ActiveContextSetRequest = serde_json::from_str(
            r#"{"projectId":null,"sessionId":null,"selection":null,"expectedContextRevision":0}"#,
        )
        .unwrap();
        assert!(request.validate().is_ok());
        let incomplete: Result<ActiveContextSetRequest, _> = serde_json::from_str(
            r#"{"projectId":null,"sessionId":null,"expectedContextRevision":0}"#,
        );
        assert!(incomplete.is_err());
        let bad_pair: ActiveContextSetRequest = serde_json::from_str(r#"{"projectId":"00000000-0000-4000-8000-000000000001","sessionId":null,"selection":null,"expectedContextRevision":0}"#).unwrap();
        assert_eq!(bad_pair.validate(), Err(ErrorCode::InvalidRequest));
        let start: OverlayTaskStartRequest = serde_json::from_str(r#"{"expectedContextRevision":1,"clientRequestId":"00000000-0000-4000-8000-000000000001","prompt":"hello"}"#).unwrap();
        assert!(start.validate().is_ok());
        assert_eq!(start.prompt, "hello");
    }
    #[test]
    fn short_preview_never_splits_utf8_or_exceeds_limit() {
        let value = "界".repeat(300);
        let preview = bounded_preview(&value);
        assert!(preview.len() <= 512);
        assert!(preview.is_char_boundary(preview.len()));
    }
}
pub(crate) fn overlay_find_by_request(
    app: &tauri::AppHandle,
    request: OverlayTaskFindRequest,
) -> Result<Option<OverlayTaskView>, ErrorCode> {
    app_state::with_state(app, |state| {
        let context = current(state, request.expected_context_revision)?;
        if state.task_context.request_id.as_ref() != Some(&request.client_request_id) {
            return Err(ErrorCode::TaskContextStale);
        }
        let task = find(
            app,
            TaskFindRequest {
                project_id: context.project_id.clone(),
                session_id: context.session_id.clone(),
                client_request_id: request.client_request_id,
            },
        )?;
        task.map(|task| {
            let origin: String = app
                .state::<StorageState>()
                .with_storage(|storage| {
                    storage
                        .connection
                        .query_row(
                            "SELECT origin_window FROM tasks WHERE id=?1",
                            [&task.id],
                            |r| r.get(0),
                        )
                        .map_err(|_| StorageError {
                            code: "STORAGE_SQL_FAILED",
                            backup_path: None,
                        })
                })
                .map_err(map_storage)?;
            if origin != "overlay" {
                return Err(ErrorCode::PermissionDenied);
            }
            bind_task(state, &task);
            task_view(app, &context, &task.id)
        })
        .transpose()
    })
}
pub(crate) fn overlay_cancel(
    app: &tauri::AppHandle,
    request: OverlayTaskCancelRequest,
    correlation: &str,
) -> Result<crate::storage::tasks::CancelReceipt, ErrorCode> {
    // Resolve the binding under the same state lock. The normal cancellation
    // service publishes after commit, so release before invoking it; the task ID
    // remains the one explicitly observed in this context, never a new task.
    let task_id = app_state::with_state(app, |state| {
        let context = current(state, request.expected_context_revision)?;
        let id = state
            .task_context
            .task_id
            .clone()
            .ok_or(ErrorCode::TaskNotFound)?;
        task_view(app, &context, &id)?;
        Ok(id)
    })?;
    cancel(
        app,
        TaskCancelRequest {
            task_id,
            expected_revision: request.expected_task_revision,
        },
        correlation,
    )
}
