use super::{
    SettingsRepository,
    repositories::{
        audit_now_ms, prune_expired_in_transaction, record_settings_commit, validate_correlation_id,
    },
};
use crate::{
    ipc::ErrorCode,
    settings::{Change, Position, View},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

fn sql(_: rusqlite::Error) -> ErrorCode {
    ErrorCode::SettingsStorageFailed
}

fn read(connection: &Connection) -> Result<View, ErrorCode> {
    let revision: i64 = connection
        .query_row(
            "SELECT revision FROM settings_revisions WHERE scope='global'",
            [],
            |r| r.get(0),
        )
        .map_err(sql)?;
    if !(1..=9007199254740991).contains(&revision) {
        return Err(ErrorCode::SettingsCorrupt);
    }
    let mut view = View::defaults(revision);
    let mut statement = connection.prepare("SELECT key,value_text,value_integer,profile_id,model_id,effort,revision FROM settings WHERE scope='global'").map_err(sql)?;
    let mut rows = statement.query([]).map_err(sql)?;
    while let Some(row) = rows.next().map_err(sql)? {
        let key: String = row.get(0).map_err(sql)?;
        let text: Option<String> = row.get(1).map_err(sql)?;
        let integer: Option<i64> = row.get(2).map_err(sql)?;
        let profile: Option<String> = row.get(3).map_err(sql)?;
        let model: Option<String> = row.get(4).map_err(sql)?;
        let effort: Option<String> = row.get(5).map_err(sql)?;
        let row_revision: i64 = row.get(6).map_err(sql)?;
        if row_revision > revision
            || row_revision < 1
            || profile.is_some()
            || model.is_some()
            || effort.is_some()
        {
            return Err(ErrorCode::SettingsCorrupt);
        }
        let corrupt = || ErrorCode::SettingsCorrupt;
        let change = match key.as_str() {
            "search_engine" if integer.is_none() => Change::SearchEngine(text.ok_or_else(corrupt)?),
            "project_sort" if integer.is_none() => Change::ProjectSort(text.ok_or_else(corrupt)?),
            "approval_policy" if integer.is_none() => {
                Change::ApprovalPolicy(text.ok_or_else(corrupt)?)
            }
            "log_level" if integer.is_none() => Change::LogLevel(text.ok_or_else(corrupt)?),
            "summary_retention_days" if text.is_none() => {
                Change::SummaryRetentionDays(integer.ok_or_else(corrupt)?)
            }
            "editor" if integer.is_none() => Change::Editor(Some(
                serde_json::from_str(&text.ok_or_else(corrupt)?).map_err(|_| corrupt())?,
            )),
            _ => return Err(corrupt()),
        };
        change.validate().map_err(|_| corrupt())?;
        view.replace(change);
    }
    let resident = connection
        .query_row(
            "SELECT enabled,shortcut,x,y,monitor,scale_factor FROM resident_preferences WHERE id=1",
            [],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<i32>>(2)?,
                    r.get::<_, Option<i32>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<f64>>(5)?,
                ))
            },
        )
        .optional()
        .map_err(sql)?;
    if let Some((enabled, shortcut, x, y, monitor, scale)) = resident {
        if !matches!(enabled, 0 | 1) {
            return Err(ErrorCode::SettingsCorrupt);
        }
        let position = match (x, y, scale) {
            (Some(x), Some(y), Some(scale_factor)) => Some(Position {
                x,
                y,
                monitor,
                scale_factor,
            }),
            (None, None, None) if monitor.is_none() => None,
            _ => return Err(ErrorCode::SettingsCorrupt),
        };
        for change in [
            Change::OverlayEnabled(enabled == 1),
            Change::OverlayShortcut(shortcut),
            Change::OverlayPosition(position),
        ] {
            change.validate().map_err(|_| ErrorCode::SettingsCorrupt)?;
            view.replace(change);
        }
    }
    Ok(view)
}

fn write(connection: &Connection, view: &View) -> Result<(), ErrorCode> {
    for change in &view.values {
        let (text, integer) = match change {
            Change::SearchEngine(v)
            | Change::ProjectSort(v)
            | Change::ApprovalPolicy(v)
            | Change::LogLevel(v) => (Some(v.clone()), None),
            Change::SummaryRetentionDays(v) => (None, Some(*v)),
            Change::Editor(Some(v)) => (
                Some(serde_json::to_string(v).map_err(|_| ErrorCode::SettingsInvalidValue)?),
                None,
            ),
            Change::Editor(None) | Change::AiDefaults(None) => {
                connection
                    .execute(
                        "DELETE FROM settings WHERE scope='global' AND key=?1",
                        [change.key()],
                    )
                    .map_err(sql)?;
                continue;
            }
            _ => continue,
        };
        connection.execute("INSERT INTO settings(id,scope,project_id,key,value_text,value_integer,revision,updated_at) VALUES(?1,'global',NULL,?2,?3,?4,?5,?6) ON CONFLICT(key) WHERE scope='global' DO UPDATE SET value_text=excluded.value_text,value_integer=excluded.value_integer,revision=excluded.revision,updated_at=excluded.updated_at",
            params![format!("global-{}",change.key()),change.key(),text,integer,view.revision,super::now_ms()]).map_err(sql)?;
    }
    let (enabled, shortcut, position) = view.resident();
    connection.execute("INSERT INTO resident_preferences(id,enabled,shortcut,x,y,monitor,scale_factor) VALUES(1,?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET enabled=excluded.enabled,shortcut=excluded.shortcut,x=excluded.x,y=excluded.y,monitor=excluded.monitor,scale_factor=excluded.scale_factor",
        params![enabled,shortcut,position.as_ref().map(|p|p.x),position.as_ref().map(|p|p.y),position.as_ref().and_then(|p|p.monitor.as_deref()),position.as_ref().map(|p|p.scale_factor)]).map_err(sql)?;
    Ok(())
}

impl SettingsRepository<'_> {
    pub fn read_view(&mut self) -> Result<View, ErrorCode> {
        let transaction = self.connection.transaction().map_err(sql)?;
        let view = read(&transaction)?;
        transaction.commit().map_err(sql)?;
        Ok(view)
    }

    pub fn initialize_defaults(&mut self, data_dir: &std::path::Path) -> Result<(), ErrorCode> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let mut view = read(&transaction)?;
        let resident_exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM resident_preferences)",
                [],
                |r| r.get(0),
            )
            .map_err(sql)?;
        if !resident_exists {
            let path = data_dir.join("overlay-settings.json");
            // A present but unreadable/malformed legacy file is an error, not defaults.
            if path
                .try_exists()
                .map_err(|_| ErrorCode::SettingsStorageFailed)?
            {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct LegacyPosition {
                    x: i32,
                    y: i32,
                    monitor: Option<String>,
                }
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Legacy {
                    enabled: bool,
                    shortcut: String,
                    position: Option<LegacyPosition>,
                }
                let bytes = std::fs::read(path).map_err(|_| ErrorCode::SettingsStorageFailed)?;
                if bytes.len() > 4096 {
                    return Err(ErrorCode::SettingsCorrupt);
                }
                let legacy: Legacy =
                    serde_json::from_slice(&bytes).map_err(|_| ErrorCode::SettingsCorrupt)?;
                for change in [
                    Change::OverlayEnabled(legacy.enabled),
                    Change::OverlayShortcut(legacy.shortcut),
                    Change::OverlayPosition(legacy.position.map(|p| Position {
                        x: p.x,
                        y: p.y,
                        monitor: p.monitor,
                        scale_factor: 1.0,
                    })),
                ] {
                    change.validate().map_err(|_| ErrorCode::SettingsCorrupt)?;
                    view.replace(change);
                }
            }
        }
        let rows: i64 = transaction
            .query_row(
                "SELECT count(*) FROM settings WHERE scope='global'",
                [],
                |r| r.get(0),
            )
            .map_err(sql)?;
        let expected_rows = view
            .values
            .iter()
            .filter(|change| {
                matches!(
                    change,
                    Change::SearchEngine(_)
                        | Change::ProjectSort(_)
                        | Change::ApprovalPolicy(_)
                        | Change::LogLevel(_)
                        | Change::SummaryRetentionDays(_)
                        | Change::Editor(Some(_))
                )
            })
            .count() as i64;
        let at = audit_now_ms().map_err(|_| ErrorCode::AuditWriteFailed)?;
        prune_expired_in_transaction(&transaction, at).map_err(|_| ErrorCode::AuditWriteFailed)?;
        if !resident_exists || rows != expected_rows {
            record_settings_commit(&transaction, &crate::ipc::correlation_id(), at)
                .map_err(|_| ErrorCode::AuditWriteFailed)?;
            write(&transaction, &view)?;
        }
        transaction
            .commit()
            .map_err(|_| ErrorCode::AuditWriteFailed)
    }

    pub fn update_view(
        &mut self,
        expected: i64,
        changes: &[Change],
        prepare: impl FnOnce(&View) -> Result<(), ErrorCode>,
    ) -> Result<View, ErrorCode> {
        self.update_view_with_id(expected, changes, &crate::ipc::correlation_id(), prepare)
    }

    /// Commit one global/resident/position batch and its minimal audit fact under
    /// the command's server correlation ID. Audit INSERT/cleanup precede prepare,
    /// so audit rejection cannot apply runtime changes. The caller still rolls
    /// back prepared runtime state when a later transaction commit fails.
    pub fn update_view_with_id(
        &mut self,
        expected: i64,
        changes: &[Change],
        id: &str,
        prepare: impl FnOnce(&View) -> Result<(), ErrorCode>,
    ) -> Result<View, ErrorCode> {
        if changes.is_empty() || changes.len() > 10 {
            return Err(ErrorCode::InvalidRequest);
        }
        let mut keys = std::collections::HashSet::new();
        for change in changes {
            if !keys.insert(change.key()) {
                return Err(ErrorCode::InvalidRequest);
            }
            change.validate()?;
        }
        validate_correlation_id(id).map_err(|_| ErrorCode::AuditWriteFailed)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let mut next = read(&transaction)?;
        if next.revision != expected {
            return Err(ErrorCode::RevisionConflict);
        }
        if next.revision >= 9007199254740991 {
            return Err(ErrorCode::SettingsCorrupt);
        }
        next.revision += 1;
        for change in changes {
            next.replace(change.clone());
        }
        let at = audit_now_ms().map_err(|_| ErrorCode::AuditWriteFailed)?;
        prune_expired_in_transaction(&transaction, at).map_err(|_| ErrorCode::AuditWriteFailed)?;
        record_settings_commit(&transaction, id, at).map_err(|_| ErrorCode::AuditWriteFailed)?;
        write(&transaction, &next)?;
        transaction
            .execute(
                "UPDATE settings_revisions SET revision=?1 WHERE scope='global'",
                [next.revision],
            )
            .map_err(sql)?;
        prepare(&next)?;
        transaction
            .commit()
            .map_err(|_| ErrorCode::AuditWriteFailed)?;
        Ok(next)
    }
}
