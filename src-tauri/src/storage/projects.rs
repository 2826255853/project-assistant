//! S2-03 typed project and usage transactions. Paths never enter audit facts.
use super::repositories::{
    audit_now_ms, audit_write_error, new_audit_id, prune_expired_in_transaction,
    validate_correlation_id,
};
use crate::{
    ipc::ErrorCode,
    project_contract::{Authorization, Availability, MAX_SAFE_INTEGER, ProjectView},
    project_paths::{DirectoryIdentity, VerifiedDirectory, revalidate_registered_directory},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::path::Path;

mod search;
#[cfg(any(test, debug_assertions))]
pub(crate) mod search_fixture;

pub struct ProjectRepository<'a> {
    connection: &'a mut Connection,
}

// Deliberately no Debug/Serialize: this record holds private filesystem identity.
struct Record {
    view: ProjectView,
    identity: Option<String>,
    path_key: String,
    removed: bool,
}

const COLUMNS: &str = "p.id,p.name,p.display_name,p.canonical_path,p.availability,p.authorization,p.revision,p.created_at,u.last_opened_at,p.directory_identity,p.path_key,p.removed_at";

fn sql(_: rusqlite::Error) -> ErrorCode {
    ErrorCode::SettingsStorageFailed
}
fn clock() -> Result<i64, ErrorCode> {
    audit_now_ms().map_err(|_| ErrorCode::SettingsStorageFailed)
}
fn audit_error(_: super::StorageError) -> ErrorCode {
    ErrorCode::AuditWriteFailed
}

fn map_record(r: &rusqlite::Row<'_>) -> rusqlite::Result<Record> {
    let availability: String = r.get(4)?;
    let authorization: String = r.get(5)?;
    Ok(Record {
        view: ProjectView {
            id: r.get(0)?,
            name: r.get(1)?,
            display_name: r.get(2)?,
            absolute_path: r.get(3)?,
            availability: match availability.as_str() {
                "available" => Availability::Available,
                "missing" => Availability::Missing,
                "inaccessible" => Availability::Inaccessible,
                _ => Availability::IdentityChanged,
            },
            authorization: match authorization.as_str() {
                "granted" => Authorization::Granted,
                "revoked" => Authorization::Revoked,
                _ => Authorization::RequiresReauthorization,
            },
            revision: r.get(6)?,
            created_at: r.get(7)?,
            last_used_at: r.get(8)?,
        },
        identity: r.get(9)?,
        path_key: r.get(10)?,
        removed: r.get::<_, Option<i64>>(11)?.is_some(),
    })
}

fn records(c: &Connection) -> Result<Vec<Record>, ErrorCode> {
    let mut statement = c.prepare(&format!("SELECT {COLUMNS} FROM projects p LEFT JOIN project_usage u ON u.project_id=p.id ORDER BY p.id")).map_err(sql)?;
    statement
        .query_map([], map_record)
        .map_err(sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql)
}

fn record(c: &Connection, id: &str) -> Result<Record, ErrorCode> {
    crate::project_contract::ProjectIdRequest {
        project_id: id.into(),
    }
    .validate()?;
    c.query_row(&format!("SELECT {COLUMNS} FROM projects p LEFT JOIN project_usage u ON u.project_id=p.id WHERE p.id=?1 AND p.removed_at IS NULL"), [id], map_record)
        .optional().map_err(sql)?.ok_or(ErrorCode::ProjectNotFound)
}

fn audit(
    c: &Connection,
    correlation: &str,
    project: &str,
    kind: &'static str,
    at: i64,
) -> Result<(), ErrorCode> {
    validate_correlation_id(correlation).map_err(audit_error)?;
    prune_expired_in_transaction(c, at).map_err(audit_error)?;
    c.execute("INSERT INTO audit_events(id,occurred_at,correlation_id,event_kind,outcome,project_id) VALUES(?1,?2,?3,?4,'success',?5)", params![new_audit_id(),at,correlation,kind,project])
        .map_err(audit_write_error).map_err(audit_error)?;
    Ok(())
}

fn verify(r: &Record) -> Result<VerifiedDirectory, ErrorCode> {
    if r.removed || r.view.authorization == Authorization::Revoked {
        return Err(ErrorCode::ProjectNotFound);
    }
    if r.view.authorization != Authorization::Granted {
        return Err(ErrorCode::ProjectPathChanged);
    }
    let identity = r
        .identity
        .as_deref()
        .and_then(DirectoryIdentity::from_storage_value)
        .ok_or(ErrorCode::ProjectPathChanged)?;
    revalidate_registered_directory(Path::new(&r.view.absolute_path), identity)
}

fn refresh_record(tx: &Connection, r: &Record, correlation: &str) -> Result<bool, ErrorCode> {
    let (availability, authorization) = match verify(r) {
        Ok(_) => ("available", "granted"),
        Err(ErrorCode::ProjectPathUnavailable) => ("missing", "granted"),
        Err(ErrorCode::ProjectAccessDenied) => ("inaccessible", "granted"),
        Err(_) => ("identity_changed", "requires_reauthorization"),
    };
    let changed: bool = tx
        .query_row(
            "SELECT availability!=?1 OR authorization!=?2 FROM projects WHERE id=?3",
            params![availability, authorization, r.view.id],
            |row| row.get(0),
        )
        .map_err(sql)?;
    if changed {
        if r.view.revision >= MAX_SAFE_INTEGER {
            return Err(ErrorCode::ProjectConflictRevision);
        }
        let at = clock()?;
        tx.execute("UPDATE projects SET availability=?1,authorization=?2,revision=revision+1,updated_at=max(updated_at,?3) WHERE id=?4",
            params![availability,authorization,at,r.view.id]).map_err(sql)?;
        audit(tx, correlation, &r.view.id, "project_invalidated", at)?;
    }
    Ok(changed)
}

/// A successful explicit selection or an OS-accepted open; never list/search/Git.
#[derive(Clone, Copy)]
pub enum UsageSource {
    Selection,
    SystemOpen,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovedProject {
    pub removed: bool,
    pub retained_for_history: bool,
}

impl<'a> ProjectRepository<'a> {
    pub(crate) fn authorized(&mut self, id: &str) -> Result<(u64, VerifiedDirectory), ErrorCode> {
        let r = record(self.connection, id)?;
        Ok((r.view.revision, verify(&r)?))
    }
    pub(super) fn new(connection: &'a mut Connection) -> Self {
        Self { connection }
    }

    /// None is picker cancellation. Both the candidate and a fresh set of identity
    /// handles live through COMMIT; no caller path can directly create a grant.
    pub fn register(
        &mut self,
        selected: Option<VerifiedDirectory>,
        correlation: &str,
    ) -> Result<Option<ProjectView>, ErrorCode> {
        let Some(selected) = selected else {
            return Ok(None);
        };
        validate_correlation_id(correlation).map_err(audit_error)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let verified =
            revalidate_registered_directory(&selected.canonical_path, selected.identity)?;
        let identity = verified.identity.storage_value();
        let all = records(&tx)?;
        // A replacement at an existing name is never silently granted, including
        // a target whose identity happens to be registered under another name.
        if all.iter().any(|r| {
            !r.removed
                && r.view.authorization == Authorization::Granted
                && r.path_key == verified.path_key
                && r.identity.as_deref().is_some_and(|v| v != identity)
        }) {
            return Err(ErrorCode::ProjectPathChanged);
        }
        let existing = all
            .iter()
            .find(|r| !r.removed && r.identity.as_deref() == Some(&identity));
        if let Some(existing) = existing {
            if existing.view.availability == Availability::Available && verify(existing).is_ok() {
                return Ok(Some(existing.view.clone()));
            }
        }
        let restore = existing.or_else(|| all.iter().find(|r| r.path_key == verified.path_key));
        let id = if let Some(r) = restore {
            r.view.id.clone()
        } else {
            crate::providers::uuid()?
        };
        let at = clock()?;
        if let Some(old) = restore {
            if old.view.revision >= MAX_SAFE_INTEGER {
                return Err(ErrorCode::ProjectConflictRevision);
            }
            tx.execute("UPDATE projects SET canonical_path=?1,path_key=?2,name=?3,directory_identity=?4,authorization='granted',availability='available',removed_at=NULL,revision=revision+1,updated_at=max(updated_at,?5) WHERE id=?6 AND revision=?7",
                params![verified.display_path,verified.path_key,verified.name,identity,at,id,old.view.revision]).map_err(sql)?;
        } else {
            tx.execute("INSERT INTO projects(id,canonical_path,path_key,name,created_at,updated_at,directory_identity,authorization,availability) VALUES(?1,?2,?3,?4,?5,?5,?6,'granted','available')",
                params![id,verified.display_path,verified.path_key,verified.name,at,identity]).map_err(sql)?;
        }
        audit(
            &tx,
            correlation,
            &id,
            if restore.is_some() {
                "project_reauthorize"
            } else {
                "project_grant"
            },
            at,
        )?;
        let view = record(&tx, &id)?.view;
        view.validate_wire()?;
        tx.commit().map_err(sql)?;
        Ok(Some(view))
    }

    /// Revalidates known paths only. Missing/inaccessible rows stay visible;
    /// changed identity permanently needs a new explicit picker grant.
    pub fn recover(&mut self, correlation: &str) -> Result<(), ErrorCode> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        for r in records(&tx)?.into_iter().filter(|r| !r.removed) {
            refresh_record(&tx, &r, correlation)?;
        }
        tx.commit().map_err(sql)
    }

    pub fn get(&mut self, id: &str, correlation: &str) -> Result<ProjectView, ErrorCode> {
        self.get_with_refresh(id, correlation).map(|(view, _)| view)
    }

    pub(crate) fn get_with_refresh(
        &mut self,
        id: &str,
        correlation: &str,
    ) -> Result<(ProjectView, bool), ErrorCode> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let r = record(&tx, id)?;
        let changed = refresh_record(&tx, &r, correlation)?;
        let view = record(&tx, id)?.view;
        view.validate_wire()?;
        tx.commit().map_err(sql)?;
        Ok((view, changed))
    }
    /// Serializes the explicit use with removal/reauthorization across processes.
    /// For SystemOpen, accept must return Ok only after the OS accepts delivery;
    /// the system adapter binds delivery using name pins and a rollback-only
    /// final-target metadata guard, including after the caller stops waiting.
    /// An OS side effect cannot be rolled back if the subsequent DB commit fails.
    pub fn record_usage<T>(
        &mut self,
        id: &str,
        expected_revision: u64,
        source: UsageSource,
        accept: impl FnOnce(&VerifiedDirectory) -> Result<T, ErrorCode>,
    ) -> Result<T, ErrorCode> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let r = record(&tx, id)?;
        if r.view.revision != expected_revision {
            return Err(ErrorCode::ProjectConflictRevision);
        }
        let verified = verify(&r)?;
        let result = accept(&verified)?;
        let at = clock()?;
        let source = match source {
            UsageSource::Selection => "project_list",
            UsageSource::SystemOpen => "system_open",
        };
        tx.execute("INSERT INTO project_usage(project_id,last_opened_at,open_count,last_source) VALUES(?1,?2,1,?3) ON CONFLICT(project_id) DO UPDATE SET last_opened_at=max(last_opened_at,excluded.last_opened_at),open_count=open_count+1,last_source=excluded.last_source", params![id,at,source]).map_err(sql)?;
        tx.commit().map_err(sql)?;
        Ok(result)
    }

    pub fn remove(
        &mut self,
        id: &str,
        expected_revision: u64,
        correlation: &str,
    ) -> Result<RemovedProject, ErrorCode> {
        crate::project_contract::RemoveRequest {
            project_id: id.into(),
            expected_revision,
        }
        .validate()?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let r = record(&tx, id)?;
        if r.view.revision != expected_revision || expected_revision >= MAX_SAFE_INTEGER {
            return Err(ErrorCode::ProjectConflictRevision);
        }
        let at = clock()?;
        // Revocation itself creates an audit reference, even for a legacy row.
        // Retain the tombstone so every audit fact keeps its project ID.
        tx.execute("UPDATE projects SET authorization='revoked',removed_at=max(created_at,?1),revision=revision+1,updated_at=max(updated_at,?1) WHERE id=?2",params![at,id]).map_err(sql)?;
        tx.execute("DELETE FROM project_usage WHERE project_id=?1", [id])
            .map_err(sql)?;
        audit(&tx, correlation, id, "project_revoke", at)?;
        tx.commit().map_err(sql)?;
        Ok(RemovedProject {
            removed: true,
            retained_for_history: true,
        })
    }

    pub fn project_settings(
        &mut self,
        id: &str,
        correlation: &str,
    ) -> Result<crate::project_contract::ProjectSettingsView, ErrorCode> {
        let tx = self.connection.transaction().map_err(sql)?;
        let r = record(&tx, id)?;
        verify(&r)?;
        let global_revision: u64 = tx
            .query_row(
                "SELECT revision FROM settings_revisions WHERE scope='global'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(sql)?
            .try_into()
            .map_err(|_| ErrorCode::SettingsCorrupt)?;
        let view = project_settings_view(&tx, id, global_revision)?;
        let at = clock()?;
        audit(&tx, correlation, id, "config_read", at)?;
        tx.commit().map_err(sql)?;
        Ok(view)
    }

    /// Save one whole project AI selection or clear it to inherit globally.
    /// Project/global revisions, provider validation, the setting and audit
    /// fact share one IMMEDIATE transaction.
    pub fn update_ai_defaults(
        &mut self,
        request: crate::project_contract::ProjectAiDefaultsUpdate,
        correlation: &str,
    ) -> Result<crate::project_contract::ProjectSettingsView, ErrorCode> {
        request.validate()?;
        super::repositories::validate_correlation_id(correlation)
            .map_err(|_| ErrorCode::AuditWriteFailed)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let r = record(&tx, &request.project_id)?;
        verify(&r)?;
        let project_revision: u64 = tx
            .query_row(
                "SELECT revision FROM project_settings_revisions WHERE project_id=?1",
                [&request.project_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(sql)?
            .try_into()
            .map_err(|_| ErrorCode::SettingsCorrupt)?;
        if project_revision != request.expected_revision {
            return Err(ErrorCode::ProjectConflictRevision);
        }
        let global_revision: u64 = tx
            .query_row(
                "SELECT revision FROM settings_revisions WHERE scope='global'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(sql)?
            .try_into()
            .map_err(|_| ErrorCode::SettingsCorrupt)?;
        if global_revision != request.expected_global_revision {
            return Err(ErrorCode::RevisionConflict);
        }
        let next_revision = project_revision
            .checked_add(1)
            .filter(|value| *value <= MAX_SAFE_INTEGER)
            .ok_or(ErrorCode::ProjectConflictRevision)?;
        if let Some(selection) = &request.selection {
            crate::providers::validate_project_selection(&tx, selection)?;
        }
        let at = clock()?;
        match &request.selection {
            Some(selection) => {
                let setting_id = format!("project-{}-ai_defaults", request.project_id);
                tx.execute(
                    "INSERT INTO settings(id,scope,project_id,key,profile_id,model_id,effort,revision,updated_at) VALUES(?1,'project',?2,'ai_defaults',?3,?4,?5,?6,?7) ON CONFLICT(project_id,key) WHERE scope='project' DO UPDATE SET profile_id=excluded.profile_id,model_id=excluded.model_id,effort=excluded.effort,revision=excluded.revision,updated_at=excluded.updated_at",
                    params![setting_id,request.project_id,selection.profile_id,selection.model_id,selection.effort,next_revision,at],
                ).map_err(sql)?;
                tx.execute(
                    "INSERT INTO ai_default_revisions(setting_id,profile_revision) SELECT id,?1 FROM settings WHERE scope='project' AND project_id=?2 AND key='ai_defaults' ON CONFLICT(setting_id) DO UPDATE SET profile_revision=excluded.profile_revision",
                    params![selection.expected_revision,request.project_id],
                ).map_err(sql)?;
            }
            None => {
                tx.execute(
                    "DELETE FROM settings WHERE scope='project' AND project_id=?1 AND key='ai_defaults'",
                    [&request.project_id],
                )
                .map_err(sql)?;
            }
        }
        tx.execute(
            "UPDATE project_settings_revisions SET revision=?1 WHERE project_id=?2 AND revision=?3",
            params![next_revision, request.project_id, project_revision],
        )
        .map_err(sql)?;
        if tx.changes() != 1 {
            return Err(ErrorCode::ProjectConflictRevision);
        }
        audit(
            &tx,
            correlation,
            &request.project_id,
            "project_ai_defaults_update",
            at,
        )?;
        let view = project_settings_view(&tx, &request.project_id, global_revision)?;
        view.validate_wire()?;
        tx.commit().map_err(sql)?;
        Ok(view)
    }
}

fn project_settings_view(
    connection: &Connection,
    id: &str,
    global_revision: u64,
) -> Result<crate::project_contract::ProjectSettingsView, ErrorCode> {
    let revision: u64 = connection
        .query_row(
            "SELECT revision FROM project_settings_revisions WHERE project_id=?1",
            [id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(sql)?
        .try_into()
        .map_err(|_| ErrorCode::SettingsCorrupt)?;
    let values = crate::settings::project_read_view(connection, id)?
        .into_iter()
        .map(|(_, setting)| setting)
        .collect();
    let view = crate::project_contract::ProjectSettingsView {
        project_id: id.to_owned(),
        revision,
        global_revision,
        values,
    };
    view.validate_wire()?;
    Ok(view)
}

#[cfg(test)]
mod search_tests;
#[cfg(test)]
mod tests;
