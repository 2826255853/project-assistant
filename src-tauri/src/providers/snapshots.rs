//! Backend-only: stage 3 must call capture_in_transaction in the SAME task transaction.
//! There is deliberately no task/snapshot mutation IPC in stage 1.8.
#![allow(dead_code)]
use super::{repository as repo, *};
use crate::owned_process::ADAPTER_VERSION;
use rusqlite::{OptionalExtension, params};

/// Complete groups only; an invalid project default never falls back globally.
pub(crate) fn resolve(
    c: &Connection,
    once: Option<&Selection>,
    project: Option<&str>,
) -> Result<Selection, ErrorCode> {
    if let Some(s) = once {
        return Ok(s.clone());
    }
    if let Some(id) = project {
        if !valid_id(id) {
            return Err(ErrorCode::PermissionDenied);
        }
        let exists: bool = c
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)",
                [id],
                |r| r.get(0),
            )
            .map_err(sql)?;
        if !exists {
            return Err(ErrorCode::PermissionDenied);
        }
        if let Some(s) = super::default_selection(c, Some(id))? {
            return Ok(s);
        }
    }
    super::default_selection(c, None)?.ok_or(ErrorCode::ProviderSelectionInvalid)
}
pub(crate) fn capture_in_transaction(
    c: &Connection,
    once: Option<&Selection>,
    authorized_project: Option<&str>,
) -> Result<PublicSnapshot, ErrorCode> {
    if c.is_autocommit() {
        return Err(ErrorCode::InvalidRequest);
    }
    let blocked: bool = c
        .query_row("SELECT blocked FROM ai_runtime WHERE id=1", [], |r| {
            r.get(0)
        })
        .map_err(sql)?;
    if blocked {
        return Err(ErrorCode::ProviderRollbackFailed);
    }
    let s = resolve(c, once, authorized_project)?;
    let at = now()?;
    let v = repo::validate_selection(c, &s, at)?;
    let tested:Option<i64>=c.query_row("SELECT max(tested_at) FROM provider_operations WHERE phase='completed' AND ((kind='test') OR (kind='switch' AND outcome='applied')) AND profile_id=?1 AND profile_revision=?2 AND model_id=?3 AND effort IS ?4 AND credential_revision=?5 AND adapter_version=?6 AND tested_at<=?7 AND tested_at>?8 AND capability_version=?9 AND effective_effort=?10",params![s.profile_id,s.expected_revision,s.model_id,s.effort,v.profile.profile.credential.revision,ADAPTER_VERSION,at,at-TEST_TTL,v.version,v.effort],|r|r.get(0)).map_err(sql)?;
    let tested = tested.ok_or(ErrorCode::ProviderTestFailed)?;
    let id = uuid()?;
    let p = &v.profile.profile;
    c.execute("INSERT INTO ai_config_snapshots(id,profile_id,profile_name,source_kind,endpoint,endpoint_origin,wire_api,auth_kind,credential_ref,credential_revision,model_id,effort,config_revision,capability_source,capability_version,adapter_version,routing_scope,effective_at,captured_at) VALUES(?1,?2,?3,'relay',?4,?5,'responses','api_key',?6,?7,?8,?9,?10,?11,?12,?13,'owned_process',?14,?15)",params![id,p.id,p.name,p.endpoint,v.profile.origin,v.profile.credential_ref,p.credential.revision,s.model_id,Some(v.effort.clone()),s.expected_revision,v.source,v.version,ADAPTER_VERSION,tested,at]).map_err(sql)?;
    Ok(PublicSnapshot {
        id,
        profile_id: p.id.clone(),
        profile_name: p.name.clone(),
        source_kind: p.source_kind.clone(),
        endpoint: p.endpoint.clone(),
        model_id: s.model_id,
        effort: Some(v.effort),
        config_revision: s.expected_revision,
        credential_revision: p.credential.revision,
        effective_at: tested,
        routing_scope: "owned_process",
    })
}
pub(crate) fn get(c: &Connection, id: &str) -> Result<PublicSnapshot, ErrorCode> {
    if !valid_id(id) {
        return Err(ErrorCode::InvalidRequest);
    }
    c.query_row("SELECT id,profile_id,profile_name,source_kind,endpoint,model_id,effort,config_revision,credential_revision,effective_at FROM ai_config_snapshots WHERE id=?1 AND routing_scope='owned_process'",[id],|r|Ok(PublicSnapshot{id:r.get(0)?,profile_id:r.get(1)?,profile_name:r.get(2)?,source_kind:r.get(3)?,endpoint:r.get(4)?,model_id:r.get(5)?,effort:r.get(6)?,config_revision:r.get(7)?,credential_revision:r.get(8)?,effective_at:r.get(9)?,routing_scope:"owned_process"})).optional().map_err(sql)?.ok_or(ErrorCode::InvalidRequest)
}
