use super::{StorageError, error, sql_error};
use rusqlite::{Connection, params};

pub struct SettingsRepository<'a> {
    pub(crate) connection: &'a mut Connection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingRecord {
    pub id: String,
    pub scope: String,
    pub project_id: Option<String>,
    pub key: String,
    pub value_text: Option<String>,
    pub value_integer: Option<i64>,
    pub profile_id: Option<String>,
    pub model_id: Option<String>,
    pub effort: Option<String>,
    pub revision: i64,
    pub updated_at: i64,
}

impl<'a> SettingsRepository<'a> {
    pub(super) fn new(connection: &'a mut Connection) -> Self {
        Self { connection }
    }

    pub fn get_global(&mut self, key: &str) -> Result<Option<SettingRecord>, StorageError> {
        self.connection
            .query_row(
                "SELECT id, scope, project_id, key, value_text, value_integer, profile_id, model_id, effort, revision, updated_at FROM settings WHERE scope = 'global' AND key = ?1",
                [key],
                map_setting,
            )
            .optional()
            .map_err(sql_error)
    }

    /// Historical schema experiments only. Production writes use update_view_with_id.
    #[cfg(test)]
    pub fn put_text(
        &mut self,
        id: &str,
        key: &str,
        value: &str,
        revision: i64,
    ) -> Result<(), StorageError> {
        if !matches!(
            key,
            "search_engine" | "editor" | "project_sort" | "approval_policy" | "log_level"
        ) {
            return Err(error("STORAGE_SETTING_NOT_ALLOWED"));
        }
        let valid = match key {
            "search_engine" => matches!(value, "bing" | "google"),
            "project_sort" => matches!(value, "recent" | "name"),
            "approval_policy" => value == "ask",
            "log_level" => matches!(value, "error" | "warn" | "info" | "debug"),
            _ => false,
        };
        if !valid || !safe_legacy_setting_id(id) || !(1..=9007199254740991).contains(&revision) {
            return Err(error("STORAGE_SETTING_NOT_ALLOWED"));
        }
        let at = audit_now_ms()?;
        let correlation_id = crate::ipc::correlation_id();
        let transaction = self
            .connection
            .transaction()
            .map_err(|_| error("STORAGE_BUSY"))?;
        prune_expired_in_transaction(&transaction, at)?;
        record_settings_commit(&transaction, &correlation_id, at)?;
        transaction
            .execute(
                "INSERT INTO settings (id, scope, project_id, key, value_text, value_integer, profile_id, model_id, effort, revision, updated_at) VALUES (?1, 'global', NULL, ?2, ?3, NULL, NULL, NULL, NULL, ?4, ?5) ON CONFLICT(key) WHERE scope = 'global' DO UPDATE SET value_text = excluded.value_text, value_integer = NULL, revision = excluded.revision, updated_at = excluded.updated_at",
                params![id, key, value, revision, at],
            )
            .map_err(sql_error)?;
        transaction.commit().map_err(audit_write_error)
    }
}

pub struct ProfileRepository<'a> {
    connection: &'a mut Connection,
}

impl<'a> ProfileRepository<'a> {
    pub(super) fn new(connection: &'a mut Connection) -> Self {
        Self { connection }
    }

    pub fn exists(&mut self, profile_id: &str) -> Result<bool, StorageError> {
        self.connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM provider_profiles WHERE id = ?1)",
                [profile_id],
                |row| row.get(0),
            )
            .map_err(sql_error)
    }
}

pub struct CapabilityRepository<'a> {
    connection: &'a mut Connection,
}

impl<'a> CapabilityRepository<'a> {
    pub(super) fn new(connection: &'a mut Connection) -> Self {
        Self { connection }
    }

    pub fn count_valid(&mut self, profile_id: &str, at: i64) -> Result<i64, StorageError> {
        self.connection
            .query_row(
                "SELECT count(*) FROM model_capabilities WHERE profile_id = ?1 AND expires_at > ?2",
                params![profile_id, at],
                |row| row.get(0),
            )
            .map_err(sql_error)
    }
}

pub struct SnapshotRepository<'a> {
    connection: &'a mut Connection,
}

impl<'a> SnapshotRepository<'a> {
    pub(super) fn new(connection: &'a mut Connection) -> Self {
        Self { connection }
    }

    pub fn exists(&mut self, snapshot_id: &str) -> Result<bool, StorageError> {
        self.connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM ai_config_snapshots WHERE id = ?1)",
                [snapshot_id],
                |row| row.get(0),
            )
            .map_err(sql_error)
    }
}

pub struct AuditRepository<'a> {
    connection: &'a mut Connection,
}

pub struct AuditEventInput<'a> {
    pub id: &'a str,
    pub correlation_id: &'a str,
    pub event_kind: &'a str,
    pub outcome: &'a str,
    pub error_code: Option<&'a str>,
}

/// Milliseconds; this policy never deletes approval/task history or backups.
pub const AUDIT_RETENTION_MS: i64 = 90 * 24 * 60 * 60 * 1000;
const CLOCK_TOLERANCE_MS: u64 = 60 * 1000;

/// A process monotonic anchor catches wall-clock jumps while the service is running.
/// A new process cannot establish trusted elapsed time across a machine clock change.
pub(super) fn audit_now_ms() -> Result<i64, StorageError> {
    use std::sync::OnceLock;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};
    static ANCHOR: OnceLock<(i64, Instant)> = OnceLock::new();
    let wall = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| error("STORAGE_CLOCK_FAILED"))?
            .as_millis(),
    )
    .map_err(|_| error("STORAGE_CLOCK_FAILED"))?;
    let (initial, monotonic) = ANCHOR.get_or_init(|| (wall, Instant::now()));
    let elapsed = i64::try_from(monotonic.elapsed().as_millis())
        .map_err(|_| error("STORAGE_CLOCK_FAILED"))?;
    let expected = initial
        .checked_add(elapsed)
        .ok_or_else(|| error("STORAGE_CLOCK_FAILED"))?;
    if wall.abs_diff(expected) > CLOCK_TOLERANCE_MS {
        return Err(error("STORAGE_CLOCK_FAILED"));
    }
    Ok(wall)
}

pub(super) fn audit_write_error(_: rusqlite::Error) -> StorageError {
    error("STORAGE_AUDIT_WRITE_FAILED")
}

/// IDs must have the current server instance prefix and an already issued sequence.
/// This is deliberately stricter than a general ASCII/text identifier allowlist.
pub(super) fn validate_correlation_id(id: &str) -> Result<(), StorageError> {
    #[cfg(test)]
    if matches!(id, "typed-correlation" | "recovery-1" | "recovery-2") {
        return Ok(());
    }
    let reference = crate::ipc::correlation_id();
    let valid = id.len() <= 96
        && id.rsplit_once('-').is_some_and(|(prefix, sequence)| {
            let (server, issued) = reference.rsplit_once('-').expect("server ID format");
            prefix == server
                && !sequence.is_empty()
                && sequence.bytes().all(|b| b.is_ascii_digit())
                && sequence.parse::<u64>().ok().is_some_and(|value| {
                    value.to_string() == sequence
                        && issued.parse::<u64>().is_ok_and(|last| value <= last)
                })
        });
    if valid {
        Ok(())
    } else {
        Err(error("STORAGE_AUDIT_INVALID_INPUT"))
    }
}

pub(super) fn new_audit_id() -> String {
    format!("audit-{}", crate::ipc::correlation_id())
}

fn validate_event(event: &AuditEventInput<'_>) -> Result<(), StorageError> {
    let valid_id = event
        .id
        .strip_prefix("audit-")
        .is_some_and(|id| validate_correlation_id(id).is_ok());
    #[cfg(test)]
    let valid_id = valid_id || event.id == "typed-audit";
    validate_correlation_id(event.correlation_id)?;
    if !valid_id
        || !matches!(
            event.event_kind,
            "config_read"
                | "config_switch"
                | "connection_test"
                | "config_rollback"
                | "approval_decision"
                | "task_transition"
                | "storage_recovery"
        )
        || !matches!(
            event.outcome,
            "success" | "failure" | "denied" | "interrupted"
        )
        || event.error_code.is_some_and(|code| {
            !matches!(
                code,
                "PA_STORAGE_RESTART_INTERRUPTED"
                    | "PA_AUDIT_WRITE_FAILED"
                    | "PA_SETTINGS_STORAGE_FAILED"
                    | "PA_SETTINGS_RECOVERY_FAILED"
                    | "PA_CONFLICT_REVISION"
                    | "PA_OVERLAY_CONTROL_FAILED"
                    | "PA_SHORTCUT_INVALID"
                    | "PA_SHORTCUT_CONFLICT"
                    | "PA_INTERNAL_UNEXPECTED"
            )
        })
        || (event.outcome == "success" && event.error_code.is_some())
    {
        return Err(error("STORAGE_AUDIT_INVALID_INPUT"));
    }
    Ok(())
}

pub(super) fn record_in_transaction(
    connection: &Connection,
    event: AuditEventInput<'_>,
    at: i64,
) -> Result<(), StorageError> {
    validate_event(&event)?;
    if at < 0 {
        return Err(error("STORAGE_CLOCK_FAILED"));
    }
    connection.execute(
        "INSERT INTO audit_events (id, occurred_at, correlation_id, event_kind, outcome, error_code, project_id, task_id, approval_id, profile_id, model_id, effort) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL, NULL, NULL, NULL, NULL)",
        params![event.id, at, event.correlation_id, event.event_kind, event.outcome, event.error_code],
    ).map_err(audit_write_error)?;
    Ok(())
}

/// Schema v2's config_switch/success means one committed settings batch, including
/// resident enablement, shortcut and position. No setting keys or values are copied.
pub(super) fn record_settings_commit(
    connection: &Connection,
    correlation_id: &str,
    at: i64,
) -> Result<(), StorageError> {
    record_in_transaction(
        connection,
        AuditEventInput {
            id: &new_audit_id(),
            correlation_id,
            event_kind: "config_switch",
            outcome: "success",
            error_code: None,
        },
        at,
    )
}

pub(super) fn prune_expired_in_transaction(
    connection: &Connection,
    at: i64,
) -> Result<usize, StorageError> {
    let current = audit_now_ms()?;
    // Reject arbitrary/future caller timestamps and arithmetic overflow before DELETE.
    if at < 0 || at > current || at.abs_diff(current) > CLOCK_TOLERANCE_MS {
        return Err(error("STORAGE_CLOCK_FAILED"));
    }
    let latest: Option<i64> = connection
        .query_row("SELECT max(occurred_at) FROM audit_events", [], |r| {
            r.get(0)
        })
        .map_err(audit_write_error)?;
    if latest.is_some_and(|latest| latest > at) {
        return Err(error("STORAGE_CLOCK_FAILED"));
    }
    let Some(cutoff) = at
        .checked_sub(AUDIT_RETENTION_MS)
        .filter(|cutoff| *cutoff >= 0)
    else {
        return Ok(0);
    };
    connection
        .execute("DELETE FROM audit_events WHERE occurred_at < ?1", [cutoff])
        .map_err(audit_write_error)
}

#[cfg(test)]
fn safe_legacy_setting_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

impl<'a> AuditRepository<'a> {
    pub(super) fn new(connection: &'a mut Connection) -> Self {
        Self { connection }
    }

    pub fn record(&mut self, event: AuditEventInput<'_>) -> Result<(), StorageError> {
        validate_event(&event)?;
        let transaction = self.connection.transaction().map_err(audit_write_error)?;
        let at = audit_now_ms()?;
        record_in_transaction(&transaction, event, at)?;
        transaction.commit().map_err(audit_write_error)
    }

    /// The caller passes current UTC Unix milliseconds. Clock jumps/rollback and
    /// future audit facts stop deletion. No approval history is authorized here.
    pub fn prune_expired(&mut self, at: i64) -> Result<usize, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(audit_write_error)?;
        let removed = prune_expired_in_transaction(&transaction, at)?;
        transaction.commit().map_err(audit_write_error)?;
        Ok(removed)
    }
}

fn map_setting(row: &rusqlite::Row<'_>) -> rusqlite::Result<SettingRecord> {
    Ok(SettingRecord {
        id: row.get(0)?,
        scope: row.get(1)?,
        project_id: row.get(2)?,
        key: row.get(3)?,
        value_text: row.get(4)?,
        value_integer: row.get(5)?,
        profile_id: row.get(6)?,
        model_id: row.get(7)?,
        effort: row.get(8)?,
        revision: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

trait OptionalRow<T> {
    fn optional(self) -> rusqlite::Result<Option<T>>;
}

impl<T> OptionalRow<T> for rusqlite::Result<T> {
    fn optional(self) -> rusqlite::Result<Option<T>> {
        match self {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error),
        }
    }
}
