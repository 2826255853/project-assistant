use super::{StorageError, error, now_ms, sql_error};
use rusqlite::{Connection, params};

pub struct SettingsRepository<'a> {
    connection: &'a mut Connection,
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
        let transaction = self
            .connection
            .transaction()
            .map_err(|_| error("STORAGE_BUSY"))?;
        transaction
            .execute(
                "INSERT INTO settings (id, scope, project_id, key, value_text, value_integer, profile_id, model_id, effort, revision, updated_at) VALUES (?1, 'global', NULL, ?2, ?3, NULL, NULL, NULL, NULL, ?4, ?5) ON CONFLICT(key) WHERE scope = 'global' DO UPDATE SET value_text = excluded.value_text, value_integer = NULL, revision = excluded.revision, updated_at = excluded.updated_at",
                params![id, key, value, revision, now_ms()],
            )
            .map_err(sql_error)?;
        transaction.commit().map_err(sql_error)
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

impl<'a> AuditRepository<'a> {
    pub(super) fn new(connection: &'a mut Connection) -> Self {
        Self { connection }
    }

    pub fn record(&mut self, event: AuditEventInput<'_>) -> Result<(), StorageError> {
        let transaction = self
            .connection
            .transaction()
            .map_err(|_| error("STORAGE_BUSY"))?;
        transaction
            .execute(
                "INSERT INTO audit_events (id, occurred_at, correlation_id, event_kind, outcome, error_code, project_id, task_id, approval_id, profile_id, model_id, effort) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL, NULL, NULL, NULL, NULL)",
                params![event.id, now_ms(), event.correlation_id, event.event_kind, event.outcome, event.error_code],
            )
            .map_err(sql_error)?;
        transaction.commit().map_err(sql_error)
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
