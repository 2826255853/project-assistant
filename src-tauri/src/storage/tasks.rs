//! Typed session/task persistence used by S3-03. This module never stores
//! prompt, reply, protocol frames, credentials, paths, or arbitrary summaries.
use super::{audit_time, repositories};
use crate::{
    ipc::{AppError, ErrorCode},
    providers,
    providers::PublicSnapshot,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

const MAX_SAFE: i64 = 9_007_199_254_740_991;
const MAX_PAGE: u16 = 100;
const TASK_LIMIT: i64 = 8;
const APPROVAL_TTL_MS: i64 = 5 * 60 * 1000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecord {
    pub id: String,
    pub project_id: String,
    pub status: String,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRecord {
    pub id: String,
    pub session_id: String,
    pub project_id: String,
    pub client_request_id: String,
    pub status: String,
    pub revision: i64,
    pub snapshot: PublicSnapshot,
    pub selection_source: String,
    pub error: Option<AppError>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub last_event_sequence: i64,
    pub content_revision: i64,
    pub content_available: bool,
}

/// Internal handoff used by the owned-process gateway. The verified project
/// path is never serialized to IPC or written to audit records.
#[derive(Debug)]
pub(crate) struct TaskLaunchRecord {
    pub task: TaskRecord,
    pub project_path: PathBuf,
    pub(crate) _proof: DirectoryProof,
}
impl TaskLaunchRecord {
    #[cfg(windows)]
    pub(crate) fn project_identity_value(&self) -> String {
        self._proof.identity.storage_value()
    }
    #[cfg(not(windows))]
    pub(crate) fn project_identity_value(&self) -> Result<String, ErrorCode> {
        Err(ErrorCode::ProjectPathUnavailable)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionReceipt {
    pub session: SessionRecord,
    pub duplicate: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskReceipt {
    pub task: TaskRecord,
    pub duplicate: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelReceipt {
    pub accepted: bool,
    pub status: String,
    pub revision: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRecord {
    pub id: String,
    pub task_id: String,
    pub session_id: String,
    pub revision: i64,
    pub operation_kind: String,
    pub operation_fingerprint: String,
    pub display_summary: String,
    pub decision: String,
    pub consumption: String,
    pub requested_at: i64,
    pub expires_at: i64,
    pub decided_at: Option<i64>,
    #[serde(skip)]
    pub(crate) window_label: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalReceipt {
    pub approval: ApprovalRecord,
    pub duplicate: bool,
}

#[derive(Debug, Clone)]
pub struct ApprovalRequestInput {
    pub task_id: String,
    pub expected_task_revision: i64,
    pub external_request_id: String,
    pub operation_kind: String,
    pub operation_fingerprint: String,
    pub window_label: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventRecord {
    pub task_id: String,
    pub sequence: i64,
    pub occurred_at: i64,
    pub data: EventData,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventData {
    Status {
        status: String,
        revision: i64,
    },
    ContentChanged {
        channel: String,
        content_revision: i64,
    },
    Approval {
        approval_id: String,
    },
    Completed {
        status: String,
        revision: i64,
    },
    Failed {
        code: String,
        revision: i64,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventPage {
    pub task_id: String,
    pub events: Vec<EventRecord>,
    pub next_cursor: Option<String>,
    pub through_sequence: i64,
    pub reset_required: bool,
}

pub struct TaskStartInput {
    pub project_id: String,
    pub session_id: String,
    pub client_request_id: String,
    pub expected_project_revision: i64,
    pub expected_session_revision: i64,
    pub expected_project_settings_revision: i64,
    pub expected_global_revision: i64,
    pub selection: Option<providers::Selection>,
    /// Transient request text. It is validated here and never persisted or audited.
    pub prompt: String,
    pub origin_window: &'static str,
}

pub struct TaskRepository<'a> {
    pub(crate) connection: &'a mut Connection,
}

impl<'a> TaskRepository<'a> {
    pub(crate) fn new(connection: &'a mut Connection) -> Self {
        Self { connection }
    }

    pub fn session_create(
        &mut self,
        project_id: &str,
        expected_project_revision: i64,
        client_request_id: &str,
        correlation: &str,
    ) -> Result<SessionReceipt, ErrorCode> {
        validate_request_id(client_request_id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let proof = authorize_project(&tx, project_id)?;
        let project = project_state(&tx, project_id)?;
        ensure_project_usable(&project)?;
        if let Some(session) = session_by_client_request(&tx, client_request_id)? {
            if session.project_id != project_id {
                return Err(ErrorCode::PermissionDenied);
            }
            return Ok(SessionReceipt {
                session,
                duplicate: true,
            });
        }
        if project.0 != expected_project_revision {
            return Err(ErrorCode::ProjectConflictRevision);
        }
        let id = new_id()?;
        let at = now()?;
        tx.execute(
            "INSERT INTO sessions(id,project_id,external_session_id,summary_redacted,status,created_at,updated_at,revision,client_request_id) VALUES(?1,?2,NULL,NULL,'active',?3,?3,1,?4)",
            params![id, project_id, at, client_request_id],
        )
        .map_err(sql)?;
        record_audit(&tx, correlation, project_id, None, "success", None, at)?;
        let session = session_get_tx(&tx, &id)?.ok_or(ErrorCode::SessionNotFound)?;
        tx.commit().map_err(sql)?;
        drop(proof);
        Ok(SessionReceipt {
            session,
            duplicate: false,
        })
    }

    pub fn session_find_by_request(
        &mut self,
        project_id: &str,
        client_request_id: &str,
    ) -> Result<Option<SessionRecord>, ErrorCode> {
        validate_request_id(client_request_id)?;
        let _proof = authorized_project(self.connection, project_id)?;
        session_by_request(self.connection, project_id, client_request_id)
    }

    pub fn sessions_list(
        &mut self,
        project_id: &str,
        cursor: Option<&str>,
        limit: u16,
    ) -> Result<(Vec<SessionRecord>, Option<String>), ErrorCode> {
        validate_page(cursor, limit)?;
        let _proof = authorized_project(self.connection, project_id)?;
        let scope = format!("sessions:{project_id}");
        let after = parse_page_cursor(cursor, &scope)?;
        let mut statement = self
            .connection
            .prepare("SELECT id,updated_at FROM sessions WHERE project_id=?1 AND (?2 IS NULL OR updated_at<?2 OR (updated_at=?2 AND id<?3)) ORDER BY updated_at DESC,id DESC LIMIT ?4")
            .map_err(sql)?;
        let ids = statement
            .query_map(
                params![
                    project_id,
                    after.as_ref().map(|v| v.0),
                    after.as_ref().map(|v| v.1.as_str()),
                    i64::from(limit) + 1
                ],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .map_err(sql)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql)?;
        let has_more = ids.len() > usize::from(limit);
        let ids: Vec<_> = ids.into_iter().take(usize::from(limit)).collect();
        let mut out = Vec::with_capacity(ids.len());
        for (id, _) in &ids {
            out.push(session_get(self.connection, id)?.ok_or(ErrorCode::SessionNotFound)?);
        }
        Ok((
            out,
            has_more.then(|| {
                ids.last()
                    .map(|(id, at)| page_cursor(&scope, *at, id))
                    .unwrap_or_default()
            }),
        ))
    }

    pub fn session_get(&mut self, id: &str) -> Result<SessionRecord, ErrorCode> {
        let session = session_get(self.connection, id)?.ok_or(ErrorCode::SessionNotFound)?;
        let _proof = authorized_project(self.connection, &session.project_id)?;
        Ok(session)
    }

    pub fn session_resume(
        &mut self,
        id: &str,
        expected_revision: i64,
        expected_project_revision: i64,
        correlation: &str,
    ) -> Result<SessionRecord, ErrorCode> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let current = session_get_tx(&tx, id)?.ok_or(ErrorCode::SessionNotFound)?;
        if current.revision != expected_revision {
            return Err(ErrorCode::TaskConflictRevision);
        }
        if expected_revision >= MAX_SAFE {
            return Err(ErrorCode::TaskConflictRevision);
        }
        let proof = authorize_project(&tx, &current.project_id)?;
        let project = project_state(&tx, &current.project_id)?;
        if project.0 != expected_project_revision {
            return Err(ErrorCode::ProjectConflictRevision);
        }
        ensure_project_usable(&project)?;
        if current.status == "closed" {
            return Err(ErrorCode::SessionClosed);
        }
        if current.status == "active" {
            return Ok(current);
        }
        if has_unfinished_task(&tx, id)? {
            return Err(ErrorCode::SessionBusy);
        }
        let at = now()?;
        tx.execute("UPDATE sessions SET status='active',revision=revision+1,updated_at=?1 WHERE id=?2 AND revision=?3", params![at,id,expected_revision]).map_err(sql)?;
        record_audit(
            &tx,
            correlation,
            &current.project_id,
            None,
            "success",
            None,
            at,
        )?;
        let result = session_get_tx(&tx, id)?.ok_or(ErrorCode::SessionNotFound)?;
        tx.commit().map_err(sql)?;
        drop(proof);
        Ok(result)
    }

    pub fn session_end(
        &mut self,
        id: &str,
        expected_revision: i64,
        correlation: &str,
    ) -> Result<SessionRecord, ErrorCode> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let current = session_get_tx(&tx, id)?.ok_or(ErrorCode::SessionNotFound)?;
        if current.revision != expected_revision {
            return Err(ErrorCode::TaskConflictRevision);
        }
        if expected_revision >= MAX_SAFE {
            return Err(ErrorCode::TaskConflictRevision);
        }
        let _proof = authorized_project(&tx, &current.project_id)?;
        if current.status == "closed" {
            return Ok(current);
        }
        if has_unfinished_task(&tx, id)? {
            return Err(ErrorCode::SessionBusy);
        }
        let at = now()?;
        tx.execute("UPDATE sessions SET status='closed',revision=revision+1,updated_at=?1 WHERE id=?2 AND revision=?3", params![at,id,expected_revision]).map_err(sql)?;
        record_audit(
            &tx,
            correlation,
            &current.project_id,
            None,
            "success",
            None,
            at,
        )?;
        let result = session_get_tx(&tx, id)?.ok_or(ErrorCode::SessionNotFound)?;
        tx.commit().map_err(sql)?;
        Ok(result)
    }

    pub fn task_start(
        &mut self,
        input: TaskStartInput,
        correlation: &str,
        verify_credential: impl FnOnce(&Connection, &PublicSnapshot) -> Result<(), ErrorCode>,
    ) -> Result<TaskReceipt, ErrorCode> {
        validate_request_id(&input.client_request_id)?;
        if input.origin_window != "main" {
            return Err(ErrorCode::PermissionDenied);
        }
        validate_prompt(&input.prompt)?;
        if input.expected_project_revision < 1
            || input.expected_project_revision > MAX_SAFE
            || input.expected_session_revision < 1
            || input.expected_session_revision > MAX_SAFE
            || input.expected_project_settings_revision < 1
            || input.expected_project_settings_revision > MAX_SAFE
            || input.expected_global_revision < 1
            || input.expected_global_revision > MAX_SAFE
        {
            return Err(ErrorCode::InvalidRequest);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let proof = authorize_project(&tx, &input.project_id)?;
        let project = project_state(&tx, &input.project_id)?;
        if project.0 != input.expected_project_revision {
            return Err(ErrorCode::ProjectConflictRevision);
        }
        ensure_project_usable(&project)?;
        let session = session_get_tx(&tx, &input.session_id)?.ok_or(ErrorCode::SessionNotFound)?;
        if session.project_id != input.project_id {
            return Err(ErrorCode::SessionProjectMismatch);
        }
        if let Some(existing) = task_by_client_request(&tx, &input.client_request_id)? {
            if existing.session_id != input.session_id || existing.project_id != input.project_id {
                return Err(ErrorCode::SessionProjectMismatch);
            }
            return Ok(TaskReceipt {
                task: existing,
                duplicate: true,
            });
        }
        if session.revision != input.expected_session_revision {
            return Err(ErrorCode::TaskConflictRevision);
        }
        if session.status != "active" {
            return Err(ErrorCode::SessionClosed);
        }
        if has_unfinished_task(&tx, &input.session_id)? {
            return Err(ErrorCode::SessionBusy);
        }
        let active: i64 = tx
            .query_row("SELECT count(*) FROM tasks WHERE status IN ('preparing','running','approval_required','cancelling')", [], |r| r.get(0))
            .map_err(sql)?;
        if active >= TASK_LIMIT {
            return Err(ErrorCode::TaskQueueFull);
        }
        let project_settings_revision: i64 = tx
            .query_row(
                "SELECT revision FROM project_settings_revisions WHERE project_id=?1",
                [&input.project_id],
                |r| r.get(0),
            )
            .map_err(sql)?;
        if project_settings_revision != input.expected_project_settings_revision {
            return Err(ErrorCode::ProjectConflictRevision);
        }
        let global_revision: i64 = tx
            .query_row(
                "SELECT revision FROM settings_revisions WHERE scope='global'",
                [],
                |r| r.get(0),
            )
            .map_err(sql)?;
        if global_revision != input.expected_global_revision {
            return Err(ErrorCode::RevisionConflict);
        }
        let (snapshot, source) = providers::capture_snapshot_in_transaction(
            &tx,
            input.selection.as_ref(),
            Some(&input.project_id),
        )?;
        verify_credential(&tx, &snapshot)?;
        let id = providers::uuid()?;
        let at = now()?;
        tx.execute(
            "INSERT INTO tasks(id,session_id,external_turn_id,snapshot_id,prompt_summary_redacted,result_summary_redacted,status,error_code,created_at,started_at,finished_at,revision,client_request_id,selection_source,project_revision,project_settings_revision,global_settings_revision,session_revision,origin_window,last_event_sequence,content_revision) VALUES(?1,?2,NULL,?3,NULL,NULL,'preparing',NULL,?4,NULL,NULL,1,?5,?6,?7,?8,?9,?10,?11,0,0)",
            params![id,input.session_id,snapshot.id,at,input.client_request_id,source,input.expected_project_revision,input.expected_project_settings_revision,input.expected_global_revision,input.expected_session_revision,input.origin_window],
        )
        .map_err(sql)?;
        tx.execute(
            "UPDATE sessions SET updated_at=max(updated_at,?1) WHERE id=?2",
            params![at, input.session_id],
        )
        .map_err(sql)?;
        insert_event(
            &tx,
            &id,
            "status",
            Some("preparing"),
            Some(1),
            None,
            None,
            None,
            None,
            at,
        )?;
        record_audit(
            &tx,
            correlation,
            &input.project_id,
            Some(&id),
            "success",
            None,
            at,
        )?;
        let task = task_get_tx(&tx, &id)?.ok_or(ErrorCode::TaskNotFound)?;
        tx.commit().map_err(sql)?;
        drop(proof);
        Ok(TaskReceipt {
            task,
            duplicate: false,
        })
    }

    pub fn task_find_by_request(
        &mut self,
        project_id: &str,
        session_id: &str,
        client_request_id: &str,
    ) -> Result<Option<TaskRecord>, ErrorCode> {
        validate_request_id(client_request_id)?;
        let _proof = authorized_project(self.connection, project_id)?;
        let found = task_by_client_request(self.connection, client_request_id)?;
        if let Some(task) = &found {
            if task.project_id != project_id || task.session_id != session_id {
                return Err(ErrorCode::SessionProjectMismatch);
            }
        }
        Ok(found)
    }

    pub fn task_get(&mut self, id: &str) -> Result<TaskRecord, ErrorCode> {
        let task = task_get(self.connection, id)?.ok_or(ErrorCode::TaskNotFound)?;
        let _proof = authorized_project(self.connection, &task.project_id)?;
        Ok(task)
    }

    pub(crate) fn task_launch(&mut self, id: &str) -> Result<TaskLaunchRecord, ErrorCode> {
        let task = self.task_get(id)?;
        if !matches!(task.status.as_str(), "preparing" | "running") {
            return Err(ErrorCode::TaskConflictRevision);
        }
        #[cfg(windows)]
        {
            let (revision, verified) = authorized_project(self.connection, &task.project_id)?;
            let admitted_revision: i64 = self
                .connection
                .query_row(
                    "SELECT project_revision FROM tasks WHERE id=?1",
                    [&task.id],
                    |row| row.get(0),
                )
                .map_err(sql)?;
            if revision != admitted_revision as u64 {
                return Err(ErrorCode::ProjectConflictRevision);
            }
            return Ok(TaskLaunchRecord {
                task,
                project_path: verified.canonical_path.clone(),
                _proof: verified,
            });
        }
        #[cfg(not(windows))]
        {
            Err(ErrorCode::ProjectPathUnavailable)
        }
    }

    /// Publish a bounded content revision without storing the content itself.
    /// The actual reply/progress bytes remain in the gateway's authorized
    /// runtime buffer until a later content-read service is attached.
    pub(crate) fn content_changed(
        &mut self,
        id: &str,
        channel: &str,
        content_revision: i64,
        correlation: &str,
    ) -> Result<TaskRecord, ErrorCode> {
        if !matches!(channel, "reply" | "progress") || !(1..=MAX_SAFE).contains(&content_revision) {
            return Err(ErrorCode::InvalidRequest);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let current = task_get_tx(&tx, id)?.ok_or(ErrorCode::TaskNotFound)?;
        let _proof = authorized_project(&tx, &current.project_id)?;
        if is_terminal(&current.status) || content_revision <= current.content_revision {
            return Err(ErrorCode::TaskConflictRevision);
        }
        let at = now()?;
        tx.execute(
            "UPDATE tasks SET content_revision=?1 WHERE id=?2",
            params![content_revision, id],
        )
        .map_err(sql)?;
        insert_event(
            &tx,
            id,
            "content_changed",
            None,
            None,
            None,
            None,
            Some(channel),
            Some(content_revision),
            at,
        )?;
        record_audit(
            &tx,
            correlation,
            &current.project_id,
            Some(id),
            "success",
            None,
            at,
        )?;
        let result = task_get_tx(&tx, id)?.ok_or(ErrorCode::TaskNotFound)?;
        tx.commit().map_err(sql)?;
        Ok(result)
    }

    pub(crate) fn bind_external_context(
        &mut self,
        id: &str,
        expected_revision: i64,
        external_session_id: &str,
        external_turn_id: &str,
        correlation: &str,
    ) -> Result<TaskRecord, ErrorCode> {
        if external_session_id.is_empty()
            || external_session_id.len() > 128
            || !external_session_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            || external_turn_id.is_empty()
            || external_turn_id.len() > 128
            || !external_turn_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        {
            return Err(ErrorCode::InvalidRequest);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let current = task_get_tx(&tx, id)?.ok_or(ErrorCode::TaskNotFound)?;
        let _proof = authorized_project(&tx, &current.project_id)?;
        if current.revision != expected_revision
            || !matches!(current.status.as_str(), "preparing" | "running")
        {
            return Err(ErrorCode::TaskConflictRevision);
        }
        let existing: Option<String> = tx
            .query_row(
                "SELECT external_turn_id FROM tasks WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .map_err(sql)?;
        if existing
            .as_deref()
            .is_some_and(|value| value != external_turn_id)
        {
            return Err(ErrorCode::TaskConflictRevision);
        }
        let at = now()?;
        tx.execute(
            "UPDATE sessions SET updated_at=max(updated_at,?1) WHERE id=?2",
            params![at, current.session_id],
        )
        .map_err(sql)?;
        tx.execute(
            "UPDATE sessions SET external_session_id=?1 WHERE id=?2",
            params![external_session_id, current.session_id],
        )
        .map_err(sql)?;
        tx.execute(
            "UPDATE tasks SET external_turn_id=?1 WHERE id=?2",
            params![external_turn_id, id],
        )
        .map_err(sql)?;
        record_audit(
            &tx,
            correlation,
            &current.project_id,
            Some(id),
            "success",
            None,
            at,
        )?;
        let result = task_get_tx(&tx, id)?.ok_or(ErrorCode::TaskNotFound)?;
        tx.commit().map_err(sql)?;
        Ok(result)
    }

    /// Until the approval-decision service is available, every external
    /// high-risk request is recorded and denied atomically before the gateway
    /// sends the denial back to the owned CLI.
    pub(crate) fn approval_deny_by_default(
        &mut self,
        task_id: &str,
        approval_id: &str,
        expected_task_revision: i64,
        correlation: &str,
    ) -> Result<TaskRecord, ErrorCode> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let current = task_get_tx(&tx, task_id)?.ok_or(ErrorCode::TaskNotFound)?;
        let _proof = authorized_project(&tx, &current.project_id)?;
        if current.revision != expected_task_revision || current.status != "approval_required" {
            return Err(ErrorCode::TaskConflictRevision);
        }
        let approval = approval_by_id(&tx, approval_id)?.ok_or(ErrorCode::InvalidRequest)?;
        if approval.task_id != task_id || approval.decision != "pending" {
            return Err(ErrorCode::InvalidRequest);
        }
        let task_revision = expected_task_revision
            .checked_add(1)
            .filter(|value| *value <= MAX_SAFE)
            .ok_or(ErrorCode::TaskConflictRevision)?;
        let approval_revision = approval
            .revision
            .checked_add(1)
            .filter(|value| *value <= MAX_SAFE)
            .ok_or(ErrorCode::TaskConflictRevision)?;
        let at = now()?;
        tx.execute(
            "UPDATE approvals SET decision='denied',decided_at=?1,revision=?2 WHERE id=?3 AND decision='pending' AND revision=?4",
            params![at, approval_revision, approval_id, approval.revision],
        )
        .map_err(sql)?;
        tx.execute(
            "UPDATE tasks SET status='running',revision=?1 WHERE id=?2 AND revision=?3 AND status='approval_required'",
            params![task_revision, task_id, expected_task_revision],
        )
        .map_err(sql)?;
        insert_event(
            &tx,
            task_id,
            "status",
            Some("running"),
            Some(task_revision),
            None,
            None,
            None,
            None,
            at,
        )?;
        record_audit(
            &tx,
            correlation,
            &current.project_id,
            Some(task_id),
            "failure",
            None,
            at,
        )?;
        let result = task_get_tx(&tx, task_id)?.ok_or(ErrorCode::TaskNotFound)?;
        tx.commit().map_err(sql)?;
        Ok(result)
    }

    /// The gateway is the only caller allowed to advance durable task state.
    /// Every accepted transition, journal sequence and audit fact is atomic.
    pub fn task_transition(
        &mut self,
        id: &str,
        expected_revision: i64,
        next_status: &str,
        error_code: Option<&str>,
        correlation: &str,
    ) -> Result<TaskRecord, ErrorCode> {
        if !valid_status(next_status)
            || error_code.is_some_and(|code| !valid_task_error(code))
            || (next_status == "failed") != error_code.is_some()
        {
            return Err(ErrorCode::InvalidRequest);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let current = task_get_tx(&tx, id)?.ok_or(ErrorCode::TaskNotFound)?;
        if current.revision != expected_revision {
            return Err(ErrorCode::TaskConflictRevision);
        }
        if !transition_allowed(&current.status, next_status) {
            return Err(ErrorCode::TaskConflictRevision);
        }
        let revision = expected_revision
            .checked_add(1)
            .filter(|value| *value <= MAX_SAFE)
            .ok_or(ErrorCode::TaskConflictRevision)?;
        let at = now()?;
        let started = (next_status == "running" && current.started_at.is_none()).then_some(at);
        let finished = is_terminal(next_status).then_some(at);
        tx.execute("UPDATE tasks SET status=?1,error_code=?2,revision=?3,started_at=coalesce(started_at,?4),finished_at=?5 WHERE id=?6 AND revision=?7", params![next_status,error_code,revision,started,finished,id,expected_revision]).map_err(sql)?;
        tx.execute(
            "UPDATE sessions SET updated_at=max(updated_at,?1) WHERE id=?2",
            params![at, current.session_id],
        )
        .map_err(sql)?;
        let (event_kind, event_status) = if next_status == "failed" {
            ("failed", Some("failed"))
        } else if is_terminal(next_status) {
            ("completed", Some(next_status))
        } else {
            ("status", Some(next_status))
        };
        insert_event(
            &tx,
            id,
            event_kind,
            event_status,
            Some(revision),
            error_code,
            None,
            None,
            None,
            at,
        )?;
        let outcome = if next_status == "interrupted" {
            "interrupted"
        } else if next_status == "failed" {
            "failure"
        } else {
            "success"
        };
        record_audit(
            &tx,
            correlation,
            &current.project_id,
            Some(id),
            outcome,
            error_code,
            at,
        )?;
        let task = task_get_tx(&tx, id)?.ok_or(ErrorCode::TaskNotFound)?;
        tx.commit().map_err(sql)?;
        Ok(task)
    }

    pub fn tasks_list(
        &mut self,
        session_id: &str,
        cursor: Option<&str>,
        limit: u16,
    ) -> Result<(Vec<TaskRecord>, Option<String>), ErrorCode> {
        validate_page(cursor, limit)?;
        let session =
            session_get(self.connection, session_id)?.ok_or(ErrorCode::SessionNotFound)?;
        let _proof = authorized_project(self.connection, &session.project_id)?;
        let scope = format!("tasks:{session_id}");
        let after = parse_page_cursor(cursor, &scope)?;
        let mut statement = self
            .connection
            .prepare("SELECT id,created_at FROM tasks WHERE session_id=?1 AND client_request_id IS NOT NULL AND snapshot_id IS NOT NULL AND (?2 IS NULL OR created_at<?2 OR (created_at=?2 AND id<?3)) ORDER BY created_at DESC,id DESC LIMIT ?4")
            .map_err(sql)?;
        let ids = statement
            .query_map(
                params![
                    session_id,
                    after.as_ref().map(|v| v.0),
                    after.as_ref().map(|v| v.1.as_str()),
                    i64::from(limit) + 1
                ],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .map_err(sql)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql)?;
        let has_more = ids.len() > usize::from(limit);
        let ids: Vec<_> = ids.into_iter().take(usize::from(limit)).collect();
        let mut out = Vec::with_capacity(ids.len());
        for (id, _) in &ids {
            out.push(task_get(self.connection, id)?.ok_or(ErrorCode::TaskNotFound)?);
        }
        Ok((
            out,
            has_more.then(|| {
                ids.last()
                    .map(|(id, at)| page_cursor(&scope, *at, id))
                    .unwrap_or_default()
            }),
        ))
    }

    pub fn task_cancel(
        &mut self,
        id: &str,
        expected_revision: i64,
        correlation: &str,
    ) -> Result<CancelReceipt, ErrorCode> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let current = task_get_tx(&tx, id)?.ok_or(ErrorCode::TaskNotFound)?;
        if current.revision != expected_revision {
            return Err(ErrorCode::TaskConflictRevision);
        }
        if is_terminal(&current.status) {
            return Ok(CancelReceipt {
                accepted: false,
                status: current.status,
                revision: current.revision,
            });
        }
        if current.status == "cancelling" {
            return Ok(CancelReceipt {
                accepted: true,
                status: current.status,
                revision: current.revision,
            });
        }
        let next_revision = expected_revision
            .checked_add(1)
            .filter(|value| *value <= MAX_SAFE)
            .ok_or(ErrorCode::TaskConflictRevision)?;
        let at = now()?;
        tx.execute(
            "UPDATE tasks SET status='cancelling',revision=?1 WHERE id=?2 AND revision=?3",
            params![next_revision, id, expected_revision],
        )
        .map_err(sql)?;
        tx.execute(
            "UPDATE sessions SET updated_at=max(updated_at,?1) WHERE id=?2",
            params![at, current.session_id],
        )
        .map_err(sql)?;
        insert_event(
            &tx,
            id,
            "status",
            Some("cancelling"),
            Some(next_revision),
            None,
            None,
            None,
            None,
            at,
        )?;
        record_audit(
            &tx,
            correlation,
            &current.project_id,
            Some(id),
            "success",
            None,
            at,
        )?;
        let result = task_get_tx(&tx, id)?.ok_or(ErrorCode::TaskNotFound)?;
        tx.commit().map_err(sql)?;
        Ok(CancelReceipt {
            accepted: true,
            status: result.status,
            revision: result.revision,
        })
    }

    pub fn approvals_get(&mut self, task_id: &str) -> Result<Vec<ApprovalRecord>, ErrorCode> {
        let _task = self.task_get(task_id)?;
        let mut statement = self
            .connection
            .prepare("SELECT a.id,a.task_id,t.session_id,a.revision,a.operation_kind,coalesce(a.operation_fingerprint,''),'',a.decision,a.consumption,a.requested_at,coalesce(a.expires_at,a.requested_at+?2),a.decided_at,a.window_label FROM approvals a JOIN tasks t ON t.id=a.task_id WHERE a.task_id=?1 ORDER BY a.requested_at,a.id")
            .map_err(sql)?;
        statement
            .query_map(params![task_id, APPROVAL_TTL_MS], |r| {
                Ok(ApprovalRecord {
                    id: r.get(0)?,
                    task_id: r.get(1)?,
                    session_id: r.get(2)?,
                    revision: r.get(3)?,
                    operation_kind: r.get(4)?,
                    operation_fingerprint: r.get(5)?,
                    display_summary: r.get(6)?,
                    decision: r.get(7)?,
                    consumption: r.get(8)?,
                    requested_at: r.get(9)?,
                    expires_at: r.get(10)?,
                    decided_at: r.get(11)?,
                    window_label: r.get(12)?,
                })
            })
            .map_err(sql)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql)
    }

    /// Store only the normalized approval binding and fingerprint. Display text
    /// stays unavailable until a later step can produce a validated summary.
    pub fn approval_create(
        &mut self,
        input: ApprovalRequestInput,
        correlation: &str,
    ) -> Result<ApprovalReceipt, ErrorCode> {
        if input.window_label != "main"
            || !matches!(
                input.operation_kind.as_str(),
                "command" | "file_change" | "unknown"
            )
            || input.external_request_id.is_empty()
            || input.external_request_id.len() > 96
            || !input
                .external_request_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            || input.operation_fingerprint.len() != 64
            || !input
                .operation_fingerprint
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(ErrorCode::InvalidRequest);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let current = task_get_tx(&tx, &input.task_id)?.ok_or(ErrorCode::TaskNotFound)?;
        let _proof = authorized_project(&tx, &current.project_id)?;
        if let Some(existing) =
            approval_by_external_request(&tx, &input.task_id, &input.external_request_id)?
        {
            if existing.operation_fingerprint != input.operation_fingerprint
                || existing.operation_kind != input.operation_kind
                || existing.window_label.as_deref() != Some(input.window_label)
            {
                return Err(ErrorCode::InvalidRequest);
            }
            return Ok(ApprovalReceipt {
                approval: existing,
                duplicate: true,
            });
        }
        if current.revision != input.expected_task_revision {
            return Err(ErrorCode::TaskConflictRevision);
        }
        let origin_window: String = tx
            .query_row(
                "SELECT origin_window FROM tasks WHERE id=?1",
                [&input.task_id],
                |r| r.get(0),
            )
            .map_err(sql)?;
        if current.status != "running" || origin_window != "main" {
            return Err(ErrorCode::TaskConflictRevision);
        }
        let revision = current
            .revision
            .checked_add(1)
            .filter(|v| *v <= MAX_SAFE)
            .ok_or(ErrorCode::TaskConflictRevision)?;
        let at = now()?;
        let expires = at
            .checked_add(APPROVAL_TTL_MS)
            .ok_or(ErrorCode::SettingsStorageFailed)?;
        let id = providers::uuid()?;
        tx.execute("INSERT INTO approvals(id,task_id,external_request_id,operation_kind,operation_summary_redacted,decision,requested_at,decided_at,expires_at,revision,operation_fingerprint,window_label,consumption,consume_request_id) VALUES(?1,?2,?3,?4,NULL,'pending',?5,NULL,?6,1,?7,?8,'unconsumed',NULL)",params![id,input.task_id,input.external_request_id,input.operation_kind,at,expires,input.operation_fingerprint,input.window_label]).map_err(sql)?;
        tx.execute(
            "UPDATE tasks SET status='approval_required',revision=?1 WHERE id=?2 AND revision=?3",
            params![revision, input.task_id, input.expected_task_revision],
        )
        .map_err(sql)?;
        tx.execute(
            "UPDATE sessions SET updated_at=max(updated_at,?1) WHERE id=?2",
            params![at, current.session_id],
        )
        .map_err(sql)?;
        insert_event(
            &tx,
            &input.task_id,
            "status",
            Some("approval_required"),
            Some(revision),
            None,
            None,
            None,
            None,
            at,
        )?;
        insert_event(
            &tx,
            &input.task_id,
            "approval",
            None,
            None,
            None,
            Some(&id),
            None,
            None,
            at,
        )?;
        record_audit(
            &tx,
            correlation,
            &current.project_id,
            Some(&input.task_id),
            "success",
            None,
            at,
        )?;
        let approval = approval_by_id(&tx, &id)?.ok_or(ErrorCode::InvalidRequest)?;
        tx.commit().map_err(sql)?;
        Ok(ApprovalReceipt {
            approval,
            duplicate: false,
        })
    }

    pub fn task_events_get(
        &mut self,
        task_id: &str,
        cursor: Option<&str>,
        limit: u16,
    ) -> Result<EventPage, ErrorCode> {
        validate_page(cursor, limit)?;
        let after = parse_event_cursor(cursor, task_id)?;
        let task = self.task_get(task_id)?;
        let earliest: Option<i64> = self
            .connection
            .query_row(
                "SELECT min(sequence) FROM task_events WHERE task_id=?1",
                [task_id],
                |r| r.get(0),
            )
            .map_err(sql)?;
        if after > task.last_event_sequence {
            return Err(ErrorCode::InvalidRequest);
        }
        let mut statement = self.connection.prepare("SELECT task_id,sequence,occurred_at,event_kind,status,revision,error_code,approval_id,channel,content_revision FROM task_events WHERE task_id=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3").map_err(sql)?;
        let events = statement
            .query_map(params![task_id, after, i64::from(limit) + 1], |r| {
                event_from_row(r)
            })
            .map_err(sql)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql)?;
        let has_more = events.len() > usize::from(limit);
        let mut events: Vec<_> = events.into_iter().take(usize::from(limit)).collect();
        let reset_required = if earliest.is_some_and(|first| after < first - 1)
            || (earliest.is_none() && task.last_event_sequence > after)
        {
            events.clear();
            true
        } else {
            false
        };
        let through = events.last().map_or(after, |e| e.sequence);
        Ok(EventPage {
            task_id: task.id,
            events,
            next_cursor: has_more.then(|| {
                format!(
                    "v1.{}.{}",
                    cursor_hash(&format!("events:{task_id}")),
                    through
                )
            }),
            through_sequence: task.last_event_sequence,
            reset_required,
        })
    }
}

fn sql(_: rusqlite::Error) -> ErrorCode {
    ErrorCode::SettingsStorageFailed
}
fn now() -> Result<i64, ErrorCode> {
    audit_time().map_err(|_| ErrorCode::SettingsStorageFailed)
}

fn new_id() -> Result<String, ErrorCode> {
    providers::uuid()
}

fn validate_request_id(value: &str) -> Result<(), ErrorCode> {
    if !valid_uuid_v4(value) {
        return Err(ErrorCode::InvalidRequest);
    }
    Ok(())
}
fn validate_prompt(value: &str) -> Result<(), ErrorCode> {
    if value.trim().is_empty() {
        return Err(ErrorCode::EmptyMessage);
    }
    if value.len() > 64 * 1024 {
        return Err(ErrorCode::InputTooLarge);
    }
    if value
        .chars()
        .any(|ch| ch.is_control() && ch != '\n' && ch != '\r' && ch != '\t')
    {
        return Err(ErrorCode::InvalidRequest);
    }
    Ok(())
}
fn valid_uuid_v4(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            14 => b == b'4',
            19 => matches!(b, b'8' | b'9' | b'a' | b'b'),
            _ => b.is_ascii_hexdigit() && !b.is_ascii_uppercase(),
        })
}
fn validate_page(cursor: Option<&str>, limit: u16) -> Result<(), ErrorCode> {
    if limit == 0
        || limit > MAX_PAGE
        || cursor.is_some_and(|v| v.len() > 512 || v.chars().any(char::is_control))
    {
        Err(ErrorCode::InvalidRequest)
    } else {
        Ok(())
    }
}
fn parse_event_cursor(cursor: Option<&str>, task_id: &str) -> Result<i64, ErrorCode> {
    let Some(value) = cursor else { return Ok(0) };
    let mut parts = value.split('.');
    let expected = cursor_hash(&format!("events:{task_id}"));
    if parts.next() != Some("v1") || parts.next() != Some(expected.as_str()) {
        return Err(ErrorCode::InvalidRequest);
    }
    let sequence = parts.next().ok_or(ErrorCode::InvalidRequest)?;
    if parts.next().is_some() {
        return Err(ErrorCode::InvalidRequest);
    }
    sequence
        .parse::<i64>()
        .ok()
        .filter(|v| *v >= 0 && *v <= MAX_SAFE)
        .ok_or(ErrorCode::InvalidRequest)
}
fn cursor_hash(scope: &str) -> String {
    format!("{:x}", Sha256::digest(scope.as_bytes()))
}
fn page_cursor(scope: &str, at: i64, id: &str) -> String {
    format!("v1.{}.{}.{}", cursor_hash(scope), at, id)
}
fn parse_page_cursor(
    cursor: Option<&str>,
    scope: &str,
) -> Result<Option<(i64, String)>, ErrorCode> {
    let Some(value) = cursor else { return Ok(None) };
    let mut parts = value.split('.');
    if parts.next() != Some("v1") || parts.next() != Some(cursor_hash(scope).as_str()) {
        return Err(ErrorCode::InvalidRequest);
    }
    let at = parts.next().ok_or(ErrorCode::InvalidRequest)?;
    let id = parts.next().ok_or(ErrorCode::InvalidRequest)?;
    if parts.next().is_some() {
        return Err(ErrorCode::InvalidRequest);
    }
    let at = at
        .parse::<i64>()
        .ok()
        .filter(|v| *v >= 0 && *v <= MAX_SAFE)
        .ok_or(ErrorCode::InvalidRequest)?;
    if !valid_uuid_v4(id) {
        return Err(ErrorCode::InvalidRequest);
    }
    Ok(Some((at, id.to_owned())))
}

fn project_state(c: &Connection, id: &str) -> Result<(i64, String, String, bool), ErrorCode> {
    c.query_row("SELECT revision,authorization,availability,removed_at IS NOT NULL FROM projects WHERE id=?1", [id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(sql)?.ok_or(ErrorCode::ProjectNotFound)
}
#[cfg(windows)]
type DirectoryProof = crate::project_paths::VerifiedDirectory;
#[cfg(not(windows))]
type DirectoryProof = ();
#[cfg(windows)]
fn authorize_project(c: &Connection, id: &str) -> Result<(u64, DirectoryProof), ErrorCode> {
    crate::storage::projects::authorize_for_task(c, id)
}
#[cfg(not(windows))]
fn authorize_project(_: &Connection, _: &str) -> Result<(u64, DirectoryProof), ErrorCode> {
    Err(ErrorCode::ProjectPathUnavailable)
}
fn authorized_project(c: &Connection, id: &str) -> Result<(u64, DirectoryProof), ErrorCode> {
    let (revision, proof) = authorize_project(c, id)?;
    let state = project_state(c, id)?;
    ensure_project_usable(&state)?;
    if revision != state.0 as u64 {
        return Err(ErrorCode::ProjectConflictRevision);
    }
    Ok((revision, proof))
}
fn ensure_project_usable(p: &(i64, String, String, bool)) -> Result<(), ErrorCode> {
    if p.3 || p.1 == "revoked" {
        return Err(ErrorCode::ProjectNotFound);
    }
    if p.1 != "granted" || p.2 != "available" {
        return Err(ErrorCode::ProjectPathChanged);
    }
    Ok(())
}
fn is_terminal(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "cancelled" | "interrupted")
}
fn has_unfinished_task(c: &Connection, session_id: &str) -> Result<bool, ErrorCode> {
    c.query_row("SELECT EXISTS(SELECT 1 FROM tasks WHERE session_id=?1 AND status IN ('preparing','running','approval_required','cancelling'))", [session_id], |r| r.get(0)).map_err(sql)
}
fn session_by_request(
    c: &Connection,
    project_id: &str,
    request: &str,
) -> Result<Option<SessionRecord>, ErrorCode> {
    c.query_row("SELECT id,project_id,status,revision,created_at,updated_at FROM sessions WHERE project_id=?1 AND client_request_id=?2", params![project_id,request], |r| Ok(SessionRecord{id:r.get(0)?,project_id:r.get(1)?,status:r.get(2)?,revision:r.get(3)?,created_at:r.get(4)?,updated_at:r.get(5)?})).optional().map_err(sql)
}
fn session_by_client_request(
    c: &Connection,
    request: &str,
) -> Result<Option<SessionRecord>, ErrorCode> {
    c.query_row("SELECT id,project_id,status,revision,created_at,updated_at FROM sessions WHERE client_request_id=?1", [request], |r| Ok(SessionRecord{id:r.get(0)?,project_id:r.get(1)?,status:r.get(2)?,revision:r.get(3)?,created_at:r.get(4)?,updated_at:r.get(5)?})).optional().map_err(sql)
}
fn session_get(c: &Connection, id: &str) -> Result<Option<SessionRecord>, ErrorCode> {
    session_get_tx(c, id)
}
fn session_get_tx(c: &Connection, id: &str) -> Result<Option<SessionRecord>, ErrorCode> {
    c.query_row(
        "SELECT id,project_id,status,revision,created_at,updated_at FROM sessions WHERE id=?1",
        [id],
        |r| {
            Ok(SessionRecord {
                id: r.get(0)?,
                project_id: r.get(1)?,
                status: r.get(2)?,
                revision: r.get(3)?,
                created_at: r.get(4)?,
                updated_at: r.get(5)?,
            })
        },
    )
    .optional()
    .map_err(sql)
}
fn task_by_client_request(c: &Connection, request: &str) -> Result<Option<TaskRecord>, ErrorCode> {
    c.query_row(
        "SELECT id FROM tasks WHERE client_request_id=?1",
        [request],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map_err(sql)?
    .map_or(Ok(None), |id| task_get(c, &id))
}
fn task_get(c: &Connection, id: &str) -> Result<Option<TaskRecord>, ErrorCode> {
    task_get_tx(c, id)
}
fn task_get_tx(c: &Connection, id: &str) -> Result<Option<TaskRecord>, ErrorCode> {
    c.query_row("SELECT t.id,t.session_id,s.project_id,t.client_request_id,t.status,t.revision,t.selection_source,t.error_code,t.created_at,t.started_at,t.finished_at,t.last_event_sequence,t.content_revision,a.id,a.profile_id,a.profile_name,a.source_kind,a.endpoint,a.model_id,a.effort,a.config_revision,a.credential_revision,a.effective_at,a.routing_scope FROM tasks t JOIN sessions s ON s.id=t.session_id JOIN ai_config_snapshots a ON a.id=t.snapshot_id WHERE t.id=?1 AND t.client_request_id IS NOT NULL AND t.selection_source IS NOT NULL", [id], |r| {
        let error_code: Option<String> = r.get(7)?;
        let error = error_code.and_then(|code| {
            serde_json::from_value::<ErrorCode>(serde_json::Value::String(code))
                .ok()
                .map(ErrorCode::public)
        });
        let snapshot = PublicSnapshot{id:r.get(13)?,profile_id:r.get(14)?,profile_name:r.get(15)?,source_kind:r.get(16)?,endpoint:r.get(17)?,model_id:r.get(18)?,effort:r.get(19)?,config_revision:r.get(20)?,credential_revision:r.get(21)?,effective_at:r.get(22)?,routing_scope:"owned_process"};
        Ok(TaskRecord{id:r.get(0)?,session_id:r.get(1)?,project_id:r.get(2)?,client_request_id:r.get(3)?,status:r.get(4)?,revision:r.get(5)?,selection_source:r.get(6)?,error,created_at:r.get(8)?,started_at:r.get(9)?,finished_at:r.get(10)?,last_event_sequence:r.get(11)?,content_revision:r.get(12)?,snapshot,content_available:false})
    }).optional().map_err(sql)
}

fn approval_by_id(c: &Connection, id: &str) -> Result<Option<ApprovalRecord>, ErrorCode> {
    c.query_row("SELECT a.id,a.task_id,t.session_id,a.revision,a.operation_kind,coalesce(a.operation_fingerprint,''),'',a.decision,a.consumption,a.requested_at,coalesce(a.expires_at,a.requested_at+?2),a.decided_at,a.window_label FROM approvals a JOIN tasks t ON t.id=a.task_id WHERE a.id=?1",params![id,APPROVAL_TTL_MS],|r| Ok(ApprovalRecord{id:r.get(0)?,task_id:r.get(1)?,session_id:r.get(2)?,revision:r.get(3)?,operation_kind:r.get(4)?,operation_fingerprint:r.get(5)?,display_summary:r.get(6)?,decision:r.get(7)?,consumption:r.get(8)?,requested_at:r.get(9)?,expires_at:r.get(10)?,decided_at:r.get(11)?,window_label:r.get(12)?})).optional().map_err(sql)
}
fn approval_by_external_request(
    c: &Connection,
    task_id: &str,
    request_id: &str,
) -> Result<Option<ApprovalRecord>, ErrorCode> {
    c.query_row("SELECT a.id,a.task_id,t.session_id,a.revision,a.operation_kind,coalesce(a.operation_fingerprint,''),'',a.decision,a.consumption,a.requested_at,coalesce(a.expires_at,a.requested_at+?3),a.decided_at,a.window_label FROM approvals a JOIN tasks t ON t.id=a.task_id WHERE a.task_id=?1 AND a.external_request_id=?2",params![task_id,request_id,APPROVAL_TTL_MS],|r| Ok(ApprovalRecord{id:r.get(0)?,task_id:r.get(1)?,session_id:r.get(2)?,revision:r.get(3)?,operation_kind:r.get(4)?,operation_fingerprint:r.get(5)?,display_summary:r.get(6)?,decision:r.get(7)?,consumption:r.get(8)?,requested_at:r.get(9)?,expires_at:r.get(10)?,decided_at:r.get(11)?,window_label:r.get(12)?})).optional().map_err(sql)
}

fn insert_event(
    c: &Connection,
    task_id: &str,
    kind: &str,
    status: Option<&str>,
    revision: Option<i64>,
    error_code: Option<&str>,
    approval_id: Option<&str>,
    channel: Option<&str>,
    content_revision: Option<i64>,
    at: i64,
) -> Result<(), ErrorCode> {
    let sequence: i64 = c
        .query_row(
            "SELECT last_event_sequence+1 FROM tasks WHERE id=?1",
            [task_id],
            |r| r.get(0),
        )
        .map_err(sql)?;
    c.execute("INSERT INTO task_events(task_id,sequence,occurred_at,event_kind,status,revision,error_code,approval_id,channel,content_revision) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)", params![task_id,sequence,at,kind,status,revision,error_code,approval_id,channel,content_revision]).map_err(sql)?;
    prune_task_events(c, task_id, at)?;
    Ok(())
}
fn event_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<EventRecord> {
    let task_id: String = r.get(0)?;
    let sequence: i64 = r.get(1)?;
    let occurred_at: i64 = r.get(2)?;
    let kind: String = r.get(3)?;
    let status: Option<String> = r.get(4)?;
    let revision: Option<i64> = r.get(5)?;
    let code: Option<String> = r.get(6)?;
    let approval: Option<String> = r.get(7)?;
    let channel: Option<String> = r.get(8)?;
    let content_revision: Option<i64> = r.get(9)?;
    let data = match kind.as_str() {
        "status" => EventData::Status {
            status: status.unwrap_or_default(),
            revision: revision.unwrap_or_default(),
        },
        "approval" => EventData::Approval {
            approval_id: approval.unwrap_or_default(),
        },
        "completed" => EventData::Completed {
            status: status.unwrap_or_default(),
            revision: revision.unwrap_or_default(),
        },
        "failed" => EventData::Failed {
            code: code.unwrap_or_default(),
            revision: revision.unwrap_or_default(),
        },
        "content_changed" => EventData::ContentChanged {
            channel: channel.unwrap_or_else(|| "reply".into()),
            content_revision: content_revision.unwrap_or_default(),
        },
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(EventRecord {
        task_id,
        sequence,
        occurred_at,
        data,
    })
}
fn record_audit(
    c: &Connection,
    correlation: &str,
    project_id: &str,
    task_id: Option<&str>,
    outcome: &str,
    error_code: Option<&str>,
    at: i64,
) -> Result<(), ErrorCode> {
    repositories::validate_correlation_id(correlation).map_err(|_| ErrorCode::AuditWriteFailed)?;
    repositories::prune_expired_in_transaction(c, at).map_err(|_| ErrorCode::AuditWriteFailed)?;
    c.execute("INSERT INTO audit_events(id,occurred_at,correlation_id,event_kind,outcome,error_code,project_id,task_id,approval_id,profile_id,model_id,effort) VALUES(?1,?2,?3,'task_transition',?4,?5,?6,?7,NULL,NULL,NULL,NULL)", params![repositories::new_audit_id(),at,correlation,outcome,error_code,project_id,task_id]).map_err(|_| ErrorCode::AuditWriteFailed)?;
    Ok(())
}

pub(super) fn recover_in_transaction(
    c: &Connection,
    correlation: &str,
    at: i64,
) -> Result<(usize, usize, usize), super::StorageError> {
    repositories::validate_correlation_id(correlation)?;
    let claimed: usize = c
        .query_row(
            "SELECT count(*) FROM approvals WHERE consumption='claimed'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map_err(super::sql_error)? as usize;
    let approval_count: usize = c
        .query_row(
            "SELECT count(*) FROM approvals WHERE decision='pending' OR consumption='claimed'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map_err(super::sql_error)? as usize;
    let approvals = c.execute("UPDATE approvals SET decision='interrupted',decided_at=?1,revision=revision+1,consumption=CASE WHEN consumption='claimed' THEN 'abandoned' ELSE consumption END WHERE decision='pending'", [at]).map_err(super::sql_error)?;
    c.execute(
        "UPDATE approvals SET consumption='abandoned',revision=revision+1 WHERE consumption='claimed' AND decision!='interrupted'",
        [],
    )
    .map_err(super::sql_error)?;

    let mut statement = c.prepare("SELECT id,last_event_sequence,revision,created_at FROM tasks WHERE status IN ('idle','preparing','running','approval_required','cancelling') ORDER BY created_at,id").map_err(super::sql_error)?;
    let rows = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })
        .map_err(super::sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(super::sql_error)?;
    drop(statement);
    for (id, sequence, revision, created_at) in &rows {
        let next_revision = revision
            .checked_add(1)
            .filter(|v| *v <= MAX_SAFE)
            .ok_or_else(|| super::error("STORAGE_SQL_FAILED"))?;
        let finished = at.max(*created_at);
        c.execute("UPDATE tasks SET status='interrupted',error_code='PA_STORAGE_RESTART_INTERRUPTED',finished_at=?1,revision=?2 WHERE id=?3 AND revision=?4", params![finished,next_revision,id,revision]).map_err(super::sql_error)?;
        let next_sequence = sequence
            .checked_add(1)
            .filter(|v| *v <= MAX_SAFE)
            .ok_or_else(|| super::error("STORAGE_SQL_FAILED"))?;
        c.execute("INSERT INTO task_events(task_id,sequence,occurred_at,event_kind,status,revision,error_code,approval_id,channel) VALUES(?1,?2,?3,'completed','interrupted',?4,'PA_STORAGE_RESTART_INTERRUPTED',NULL,NULL)", params![id,next_sequence,finished,next_revision]).map_err(super::sql_error)?;
        prune_task_events(c, id, finished).map_err(|_| super::error("STORAGE_SQL_FAILED"))?;
    }
    let sessions = c.execute("UPDATE sessions SET status='interrupted',revision=revision+1,updated_at=max(updated_at,?1) WHERE status='active'", [at]).map_err(super::sql_error)?;
    let _ = (correlation, claimed);
    Ok((rows.len(), approval_count.max(approvals), sessions))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{project_contract::ProjectView, project_paths::authorize_fixture};
    use std::{cell::Cell, fs, path::PathBuf};

    fn uuid(seed: u8) -> String {
        format!("00000000-0000-4000-8000-{:012x}", seed)
    }

    struct Fixture {
        root: PathBuf,
        db: crate::storage::Storage,
        project: ProjectView,
        selection: providers::Selection,
        keep_root: bool,
    }
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("pa-s3-03-{}", providers::uuid().unwrap()));
            let source = root.join("project 中文");
            fs::create_dir_all(&source).unwrap();
            let mut db = crate::storage::Storage::open(&root.join("data")).unwrap();
            let project = db
                .projects()
                .register(
                    Some(authorize_fixture(&source).unwrap()),
                    &crate::ipc::correlation_id(),
                )
                .unwrap()
                .unwrap();
            let at = providers::now().unwrap();
            let capability_version = format!(
                "{}:task-storage-fixture",
                crate::owned_process::ADAPTER_VERSION
            );
            db.connection.execute("INSERT INTO provider_profiles(id,name,source_kind,adapter_kind,external_profile_id,endpoint,endpoint_origin,wire_api,auth_kind,credential_ref,credential_revision,default_model,default_effort,enabled,config_revision,created_at,updated_at,credential_status) VALUES('task-profile','Task fixture','relay','self_managed',NULL,'https://task-fixture.invalid/v1','https://task-fixture.invalid','responses','api_key','credential:task-profile:1',1,NULL,NULL,1,1,?1,?1,'configured_unverified')", params![at]).unwrap();
            db.connection.execute("INSERT INTO model_capabilities(profile_id,model_id,effort,context_window,is_default,source_kind,source_version,profile_revision,observed_at,expires_at) VALUES('task-profile','task-model','high',NULL,0,'cli_catalog',?1,1,?2,?3)", params![capability_version,at,at+providers::CAPABILITY_TTL]).unwrap();
            db.connection.execute("INSERT INTO provider_operations(id,kind,profile_id,profile_revision,model_id,effort,correlation_id,phase,outcome,tested_at,credential_revision,adapter_version,capability_version,effective_effort,settings_revision,created_at,finished_at) VALUES('task-test-operation','test','task-profile',1,'task-model','high','fixture-correlation','completed',NULL,?1,1,?2,?3,'high',1,?1,?1)", params![at,crate::owned_process::ADAPTER_VERSION,capability_version]).unwrap();
            let selection = providers::Selection {
                profile_id: "task-profile".into(),
                expected_revision: 1,
                model_id: "task-model".into(),
                effort: Some("high".into()),
            };
            Self {
                root,
                db,
                project,
                selection,
                keep_root: false,
            }
        }
        fn session(&mut self, seed: u8) -> SessionRecord {
            self.db
                .tasks()
                .session_create(
                    &self.project.id,
                    self.project.revision as i64,
                    &uuid(seed),
                    &crate::ipc::correlation_id(),
                )
                .unwrap()
                .session
        }
        fn input(&self, session: &SessionRecord, request: u8) -> TaskStartInput {
            TaskStartInput {
                project_id: self.project.id.clone(),
                session_id: session.id.clone(),
                client_request_id: uuid(request),
                expected_project_revision: self.project.revision as i64,
                expected_session_revision: session.revision,
                expected_project_settings_revision: 1,
                expected_global_revision: 1,
                selection: Some(self.selection.clone()),
                prompt: "private prompt sentinel".into(),
                origin_window: "main",
            }
        }
        fn count(&self, table: &str) -> i64 {
            self.db
                .connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let replacement = crate::storage::Storage {
                connection: Connection::open_in_memory().unwrap(),
                path: PathBuf::new(),
            };
            drop(std::mem::replace(&mut self.db, replacement));
            if !self.keep_root {
                let _ = fs::remove_dir_all(&self.root);
            }
        }
    }

    #[test]
    fn request_ids_and_prompts_are_bounded_without_retaining_text() {
        assert!(validate_request_id(&uuid(1)).is_ok());
        assert!(validate_request_id("plain-request").is_err());
        assert!(validate_prompt("hello\nworld").is_ok());
        assert!(validate_prompt("\0").is_err());
        assert!(validate_prompt(" ").is_err());
    }

    #[test]
    fn transitions_are_one_way_and_terminal() {
        assert!(transition_allowed("preparing", "running"));
        assert!(transition_allowed("running", "approval_required"));
        assert!(transition_allowed("cancelling", "cancelled"));
        assert!(!transition_allowed("completed", "running"));
        assert!(!transition_allowed("closed", "active"));
    }

    #[test]
    fn task_admission_snapshot_idempotency_transitions_and_audit_failure_are_atomic() {
        let mut f = Fixture::new();
        let session = f.session(1);
        let prompt = "private prompt sentinel";
        let input = f.input(&session, 2);
        let receipt =
            f.db.tasks()
                .task_start(input, &crate::ipc::correlation_id(), |_, _| Ok(()))
                .unwrap();
        assert!(!receipt.duplicate);
        assert_eq!(receipt.task.status, "preparing");
        assert_eq!(receipt.task.selection_source, "task");
        assert_eq!(receipt.task.last_event_sequence, 1);
        assert_eq!(f.count("tasks"), 1);
        assert_eq!(f.count("ai_config_snapshots"), 1);
        let body_rows: i64 = f.db.connection.query_row("SELECT count(*) FROM tasks WHERE prompt_summary_redacted IS NOT NULL OR result_summary_redacted IS NOT NULL", [], |r|r.get(0)).unwrap();
        assert_eq!(body_rows, 0);
        let duplicate_input = TaskStartInput {
            prompt: "changed duplicate prompt".into(),
            ..f.input(&session, 2)
        };
        let duplicate =
            f.db.tasks()
                .task_start(duplicate_input, &crate::ipc::correlation_id(), |_, _| {
                    panic!("duplicate must not revalidate or launch")
                })
                .unwrap();
        assert!(duplicate.duplicate);
        assert_eq!(duplicate.task.id, receipt.task.id);
        assert_eq!(f.count("tasks"), 1);
        assert_eq!(f.count("ai_config_snapshots"), 1);
        let running =
            f.db.tasks()
                .task_transition(
                    &receipt.task.id,
                    1,
                    "running",
                    None,
                    &crate::ipc::correlation_id(),
                )
                .unwrap();
        assert_eq!(running.revision, 2);
        assert_eq!(running.last_event_sequence, 2);
        assert_eq!(
            f.db.tasks()
                .session_end(&session.id, session.revision, &crate::ipc::correlation_id())
                .err(),
            Some(ErrorCode::SessionBusy)
        );
        let approval_input = ApprovalRequestInput {
            task_id: receipt.task.id.clone(),
            expected_task_revision: 2,
            external_request_id: "external-request-1".into(),
            operation_kind: "command".into(),
            operation_fingerprint: "a".repeat(64),
            window_label: "main",
        };
        let approval =
            f.db.tasks()
                .approval_create(approval_input.clone(), &crate::ipc::correlation_id())
                .unwrap();
        assert!(!approval.duplicate);
        assert_eq!(approval.approval.decision, "pending");
        assert!(approval.approval.display_summary.is_empty());
        let duplicate_approval =
            f.db.tasks()
                .approval_create(approval_input, &crate::ipc::correlation_id())
                .unwrap();
        assert!(duplicate_approval.duplicate);
        assert_eq!(
            f.db.tasks().approvals_get(&receipt.task.id).unwrap().len(),
            1
        );
        let resumed =
            f.db.tasks()
                .task_transition(
                    &receipt.task.id,
                    3,
                    "running",
                    None,
                    &crate::ipc::correlation_id(),
                )
                .unwrap();
        assert_eq!(resumed.revision, 4);
        let finished =
            f.db.tasks()
                .task_transition(
                    &receipt.task.id,
                    4,
                    "completed",
                    None,
                    &crate::ipc::correlation_id(),
                )
                .unwrap();
        assert_eq!(finished.revision, 5);
        assert_eq!(finished.status, "completed");
        assert!(
            f.db.connection
                .execute(
                    "UPDATE ai_config_snapshots SET model_id='replacement' WHERE id=?1",
                    [&receipt.task.snapshot.id]
                )
                .is_err()
        );
        assert!(
            f.db.connection
                .execute(
                    "UPDATE tasks SET snapshot_id=NULL WHERE id=?1",
                    [&receipt.task.id]
                )
                .is_err()
        );
        let before_tasks = f.count("tasks");
        let before_snapshots = f.count("ai_config_snapshots");
        f.db.connection.execute_batch("CREATE TEMP TRIGGER fail_task_audit BEFORE INSERT ON audit_events WHEN NEW.event_kind='task_transition' BEGIN SELECT RAISE(ABORT,'synthetic-task-audit-secret'); END;").unwrap();
        let verify_calls = Cell::new(0);
        let input = f.input(&session, 3);
        let failed =
            f.db.tasks()
                .task_start(input, &crate::ipc::correlation_id(), |_, _| {
                    verify_calls.set(verify_calls.get() + 1);
                    Ok(())
                });
        assert_eq!(failed.err(), Some(ErrorCode::AuditWriteFailed));
        assert_eq!(verify_calls.get(), 1);
        assert_eq!(f.count("tasks"), before_tasks);
        assert_eq!(f.count("ai_config_snapshots"), before_snapshots);
        let page =
            f.db.tasks()
                .task_events_get(&receipt.task.id, None, 10)
                .unwrap();
        assert_eq!(page.events.len(), 6);
        assert_eq!(page.through_sequence, 6);
        assert!(!page.reset_required);
        assert!(!format!("{:?}", page.events).contains(prompt));
        f.db.connection
            .execute(
                "DELETE FROM task_events WHERE task_id=?1 AND sequence=1",
                [&receipt.task.id],
            )
            .unwrap();
        let reset =
            f.db.tasks()
                .task_events_get(&receipt.task.id, None, 10)
                .unwrap();
        assert!(reset.reset_required);
        assert!(reset.events.is_empty());
    }

    #[test]
    fn unavailable_capability_and_revoked_project_reject_before_record_creation() {
        let mut f = Fixture::new();
        let session = f.session(4);
        f.db.connection
            .execute(
                "DELETE FROM model_capabilities WHERE profile_id='task-profile'",
                [],
            )
            .unwrap();
        let before = f.count("tasks");
        let input = f.input(&session, 5);
        assert_eq!(
            f.db.tasks()
                .task_start(input, &crate::ipc::correlation_id(), |_, _| Ok(()))
                .err(),
            Some(ErrorCode::ProviderCapabilityInvalid)
        );
        assert_eq!(f.count("tasks"), before);
        f.db.projects()
            .remove(
                &f.project.id,
                f.project.revision,
                &crate::ipc::correlation_id(),
            )
            .unwrap();
        let input = f.input(&session, 6);
        assert_eq!(
            f.db.tasks()
                .task_start(input, &crate::ipc::correlation_id(), |_, _| Ok(()))
                .err(),
            Some(ErrorCode::ProjectNotFound)
        );
        assert_eq!(f.count("tasks"), before);
    }

    #[test]
    #[ignore = "invoked as an independent process by the task persistence test"]
    fn session_task_reopen_worker() {
        let root = PathBuf::from(std::env::var_os("PA_TASK_STORAGE_TEST_ROOT").unwrap());
        let session_id = std::env::var("PA_TASK_STORAGE_SESSION").unwrap();
        let task_id = std::env::var("PA_TASK_STORAGE_TASK").unwrap();
        let mut db = crate::storage::Storage::open(&root.join("data")).unwrap();
        let session = db.tasks().session_get(&session_id).unwrap();
        assert_eq!(session.status, "active");
        let task = db.tasks().task_get(&task_id).unwrap();
        assert_eq!(task.status, "completed");
        assert_eq!(task.snapshot.profile_id, "task-profile");
        assert_eq!(task.selection_source, "task");
    }

    #[test]
    fn admitted_session_and_task_survive_independent_process_reopen() {
        use std::process::Command;
        let mut f = Fixture::new();
        let session = f.session(20);
        let input = f.input(&session, 21);
        let admitted =
            f.db.tasks()
                .task_start(input, &crate::ipc::correlation_id(), |_, _| Ok(()))
                .unwrap();
        f.db.tasks()
            .task_transition(
                &admitted.task.id,
                1,
                "running",
                None,
                &crate::ipc::correlation_id(),
            )
            .unwrap();
        f.db.tasks()
            .task_transition(
                &admitted.task.id,
                2,
                "completed",
                None,
                &crate::ipc::correlation_id(),
            )
            .unwrap();
        let root = f.root.clone();
        let session_id = session.id;
        let task_id = admitted.task.id;
        f.keep_root = true;
        drop(f);
        let result = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "storage::tasks::tests::session_task_reopen_worker",
            ])
            .env("PA_TASK_STORAGE_TEST_ROOT", &root)
            .env("PA_TASK_STORAGE_SESSION", session_id)
            .env("PA_TASK_STORAGE_TASK", task_id)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "task persistence worker failed: {}",
            String::from_utf8_lossy(&result.stdout)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pagination_cursors_are_scoped_and_stable() {
        let mut f = Fixture::new();
        let first = f.session(30);
        let second = f.session(31);
        let (items, cursor) = f.db.tasks().sessions_list(&f.project.id, None, 1).unwrap();
        assert_eq!(items.len(), 1);
        let cursor = cursor.unwrap();
        assert!(cursor.starts_with("v1."));
        let (next, _) =
            f.db.tasks()
                .sessions_list(&f.project.id, Some(&cursor), 1)
                .unwrap();
        assert_eq!(next.len(), 1);
        assert_ne!(items[0].id, next[0].id);
        assert_eq!(
            f.db.tasks()
                .sessions_list(&second.project_id, Some("v1.bad.0:bad"), 1)
                .err(),
            Some(ErrorCode::InvalidRequest)
        );
        let input = f.input(&first, 32);
        let task =
            f.db.tasks()
                .task_start(input, &crate::ipc::correlation_id(), |_, _| Ok(()))
                .unwrap();
        let events =
            f.db.tasks()
                .task_events_get(&task.task.id, None, 1)
                .unwrap();
        assert!(events.next_cursor.is_none());
        assert_eq!(
            f.db.tasks()
                .task_events_get(&task.task.id, Some("1"), 1)
                .err(),
            Some(ErrorCode::InvalidRequest)
        );
    }
}

fn prune_task_events(c: &Connection, task_id: &str, now: i64) -> Result<(), ErrorCode> {
    c.execute("DELETE FROM task_events WHERE task_id=?1 AND (occurred_at < ?2 OR sequence <= (SELECT max(sequence)-10000 FROM task_events WHERE task_id=?1))", params![task_id,now.saturating_sub(24*60*60*1000)]).map_err(sql)?;
    Ok(())
}

fn valid_status(status: &str) -> bool {
    matches!(
        status,
        "preparing"
            | "running"
            | "approval_required"
            | "cancelling"
            | "completed"
            | "failed"
            | "cancelled"
            | "interrupted"
    )
}
fn valid_task_error(code: &str) -> bool {
    matches!(
        code,
        "PA_STORAGE_RESTART_INTERRUPTED"
            | "PA_TASK_PROTOCOL_INVALID"
            | "PA_TASK_LIMIT_EXCEEDED"
            | "PA_TASK_RECOVERY_BLOCKED"
            | "PA_TASK_OUTPUT_UNAVAILABLE"
            | "PA_CODEX_NOT_INSTALLED"
            | "PA_CODEX_VERSION_UNSUPPORTED"
            | "PA_CODEX_REQUEST_TIMEOUT"
            | "PA_CODEX_DISCONNECTED"
            | "PA_PROCESS_EXIT_FAILED"
            | "PA_PROVIDER_TEST_FAILED"
            | "PA_PROVIDER_CAPABILITY_INVALID"
            | "PA_PROVIDER_SELECTION_INVALID"
            | "PA_CREDENTIAL_INVALID"
            | "PA_CREDENTIALS_UNAVAILABLE"
            | "PA_PROJECT_PATH_CHANGED"
            | "PA_PROJECT_PATH_UNAVAILABLE"
            | "PA_SETTINGS_STORAGE_FAILED"
            | "PA_AUDIT_WRITE_FAILED"
    )
}
fn transition_allowed(old: &str, new: &str) -> bool {
    match old {
        "preparing" => matches!(new, "running" | "cancelling" | "failed" | "interrupted"),
        "running" => matches!(
            new,
            "approval_required" | "cancelling" | "completed" | "failed" | "interrupted"
        ),
        "approval_required" => matches!(new, "running" | "cancelling" | "failed" | "interrupted"),
        "cancelling" => matches!(new, "completed" | "failed" | "cancelled" | "interrupted"),
        _ => false,
    }
}
