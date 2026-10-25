use super::*;
use crate::{credentials::CredentialState, owned_process::ADAPTER_VERSION};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use url::Url;

pub(crate) struct Row {
    pub profile: Profile,
    pub origin: String,
    pub credential_ref: String,
    pub external: Option<String>,
}
pub(crate) fn endpoint(value: &str) -> Result<(String, String), ErrorCode> {
    if value.len() > 2048
        || value.trim() != value
        || value.bytes().any(|b| b.is_ascii_control())
        || value.contains(['\\', '%'])
    {
        return Err(ErrorCode::ProviderEndpointInvalid);
    }
    let u = Url::parse(value).map_err(|_| ErrorCode::ProviderEndpointInvalid)?;
    let host = u.host_str().ok_or(ErrorCode::ProviderEndpointInvalid)?;
    if u.scheme() != "https"
        || !u.username().is_empty()
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
        || host == "openai.com"
        || host.ends_with(".openai.com")
        || host == "localhost"
        || host.ends_with(".localhost")
        || !matches!(u.path(), "/" | "/v1" | "/v1/" | "/api/v1" | "/api/v1/")
    {
        return Err(ErrorCode::ProviderEndpointInvalid);
    }
    Ok((
        u.as_str().trim_end_matches('/').to_owned(),
        u.origin().ascii_serialization(),
    ))
}
pub(crate) fn row(c: &Connection, id: &str) -> Result<Row, ErrorCode> {
    c.query_row("SELECT id,name,source_kind,adapter_kind,endpoint,wire_api,auth_kind,default_model,default_effort,enabled,config_revision,credential_status,credential_revision,endpoint_origin,credential_ref,external_profile_id FROM provider_profiles WHERE id=?1",[id],|r|{
        let adapter:String=r.get(3)?; let source:String=r.get(2)?; let wire:String=r.get(5)?; let auth:String=r.get(6)?; let external:Option<String>=r.get(15)?; let enabled:bool=r.get(9)?;let credential_status:String=r.get(11)?;
        let owned=adapter=="self_managed"&&source=="relay"&&wire=="responses"&&auth=="api_key"&&external.is_none();
        let credential_configured=matches!(credential_status.as_str(),"configured_unverified"|"verified");
        let available=owned&&enabled&&credential_configured;
        Ok(Row{profile:Profile{id:r.get(0)?,name:r.get(1)?,source_kind:source,adapter_kind:adapter,endpoint:r.get(4)?,wire_api:wire,auth_kind:auth,default_model:r.get(7)?,default_effort:r.get(8)?,enabled,config_revision:r.get(10)?,credential:CredentialState{status:match credential_status.as_str(){"configured_unverified"=>"configured_unverified","verified"=>"verified","invalid"=>"invalid","external_unavailable"=>"external_unavailable",_=>"missing"},revision:r.get(12)?},available,unavailable_code:if !owned{Some("PA_PROVIDER_ADAPTER_UNAVAILABLE")}else if !enabled{Some("PA_PROVIDER_DISABLED")}else if !credential_configured{Some("PA_CREDENTIAL_INVALID")}else{None}},origin:r.get(13)?,credential_ref:r.get::<_,Option<String>>(14)?.unwrap_or_default(),external})
    }).optional().map_err(sql)?.ok_or(ErrorCode::CredentialProfileNotFound)
}
pub(crate) fn ensure_owned(r: &Row) -> Result<(), ErrorCode> {
    if r.external.is_some()
        || r.profile.adapter_kind != "self_managed"
        || r.profile.source_kind != "relay"
        || r.profile.auth_kind != "api_key"
        || r.profile.wire_api != "responses"
    {
        return Err(ErrorCode::ProviderAdapterUnavailable);
    }
    let (e, o) = endpoint(&r.profile.endpoint)?;
    if e != r.profile.endpoint.trim_end_matches('/') || o != r.origin {
        return Err(ErrorCode::ProviderEndpointInvalid);
    }
    Ok(())
}
pub(crate) fn revision(r: &Row, expected: i64) -> Result<(), ErrorCode> {
    if r.profile.config_revision != expected {
        Err(ErrorCode::RevisionConflict)
    } else {
        Ok(())
    }
}
pub(crate) fn audit(
    c: &Connection,
    id: &str,
    kind: &str,
    profile: Option<&str>,
    outcome: &str,
) -> Result<(), ErrorCode> {
    crate::storage::provider_audit(c, id, kind, profile, outcome)
        .map_err(|_| ErrorCode::AuditWriteFailed)
}
pub(crate) fn invalidate(c: &Connection, id: &str) -> Result<(), ErrorCode> {
    c.execute("DELETE FROM model_capabilities WHERE profile_id=?1", [id])
        .map_err(sql)?;
    c.execute("UPDATE ai_runtime SET profile_id=NULL,profile_revision=NULL,model_id=NULL,effort=NULL,credential_revision=NULL,adapter_version=NULL,tested_at=NULL WHERE profile_id=?1",[id]).map_err(sql)?;
    Ok(())
}
pub(crate) fn busy(c: &Connection, id: &str) -> Result<bool, ErrorCode> {
    c.query_row("SELECT EXISTS(SELECT 1 FROM tasks t JOIN ai_config_snapshots s ON s.id=t.snapshot_id WHERE s.profile_id=?1 AND t.status IN ('idle','preparing','running','approval_required','cancelling'))",[id],|r|r.get(0)).map_err(sql)
}
pub(crate) fn mutate_save(c: &mut Connection, v: Save, id: &str) -> Result<Profile, ErrorCode> {
    if v.name.trim().is_empty()
        || v.name != v.name.trim()
        || v.name.chars().count() > 128
        || v.name.chars().any(char::is_control)
        || v.default_model.as_ref().is_some_and(|m| !valid_model(m))
        || v.default_effort.as_ref().is_some_and(|e| !valid_model(e))
        || v.default_effort.is_some() && v.default_model.is_none()
    {
        return Err(ErrorCode::ProviderSelectionInvalid);
    }
    let (endpoint, origin) = endpoint(&v.endpoint)?;
    let tx = c
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    let profile_id = match (&v.profile_id, v.expected_revision) {
        (None, None) => {
            let p = uuid()?;
            tx.execute("INSERT INTO provider_profiles(id,name,source_kind,adapter_kind,endpoint,endpoint_origin,wire_api,auth_kind,credential_ref,credential_revision,default_model,default_effort,enabled,config_revision,created_at,updated_at) VALUES(?1,?2,'relay','self_managed',?3,?4,'responses','api_key',?5,0,?6,?7,1,1,?8,?8)",params![p,v.name,endpoint,origin,format!("credential:{p}:0"),v.default_model,v.default_effort,now()?]).map_err(sql)?;
            p
        }
        (Some(p), Some(rev)) if valid_id(p) && (1..MAX_REVISION).contains(&rev) => {
            let r = row(&tx, p)?;
            ensure_owned(&r)?;
            revision(&r, rev)?;
            if busy(&tx, p)? {
                return Err(ErrorCode::CredentialBusy);
            }
            // A site credential cannot silently move to another origin. Remove it first.
            if r.origin != origin && r.profile.credential.status != "missing" {
                return Err(ErrorCode::CredentialInvalid);
            }
            tx.execute("UPDATE provider_profiles SET name=?1,endpoint=?2,endpoint_origin=?3,default_model=?4,default_effort=?5,config_revision=config_revision+1,credential_status=CASE WHEN credential_status='verified' THEN 'configured_unverified' ELSE credential_status END,updated_at=?6 WHERE id=?7",params![v.name,endpoint,origin,v.default_model,v.default_effort,now()?,p]).map_err(sql)?;
            invalidate(&tx, p)?;
            p.clone()
        }
        _ => return Err(ErrorCode::ProviderSelectionInvalid),
    };
    // Metadata defaults are declarations, never capabilities or ready evidence.
    audit(&tx, id, "config_switch", Some(&profile_id), "success")?;
    let profile = row(&tx, &profile_id)?.profile;
    tx.commit().map_err(sql)?;
    Ok(profile)
}
pub fn save(app: &tauri::AppHandle, v: Save, id: &str) -> Result<Profile, ErrorCode> {
    let state = app.state::<crate::credentials::CredentialsState>();
    let _g = state
        .operation
        .lock()
        .map_err(|_| ErrorCode::CredentialsStorageFailed)?;
    let result = with_db(app, |c| mutate_save(c, v, id));
    crate::app_state::publish(app, id.into());
    result
}
pub fn set_enabled(app: &tauri::AppHandle, v: Enabled, id: &str) -> Result<Profile, ErrorCode> {
    ProfileRequest {
        profile_id: v.profile_id.clone(),
        expected_revision: v.expected_revision,
    }
    .validate()?;
    let state = app.state::<crate::credentials::CredentialsState>();
    let _g = state
        .operation
        .lock()
        .map_err(|_| ErrorCode::CredentialsStorageFailed)?;
    let result = with_db(app, |c| {
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let r = row(&tx, &v.profile_id)?;
        revision(&r, v.expected_revision)?;
        if v.enabled {
            ensure_owned(&r)?
        }
        if r.profile.enabled == v.enabled {
            return Ok(r.profile);
        }
        if v.expected_revision >= MAX_REVISION {
            return Err(ErrorCode::RevisionConflict);
        }
        tx.execute("UPDATE provider_profiles SET enabled=?1,config_revision=config_revision+1,updated_at=?2 WHERE id=?3",params![v.enabled,now()?,v.profile_id]).map_err(sql)?;
        invalidate(&tx, &v.profile_id)?;
        audit(&tx, id, "config_switch", Some(&v.profile_id), "success")?;
        let result = row(&tx, &v.profile_id)?.profile;
        tx.commit().map_err(sql)?;
        Ok(result)
    });
    crate::app_state::publish(app, id.into());
    result
}
pub fn delete(app: &tauri::AppHandle, v: ProfileRequest, id: &str) -> Result<Removed, ErrorCode> {
    v.validate()?;
    let state = app.state::<crate::credentials::CredentialsState>();
    let _g = state
        .operation
        .lock()
        .map_err(|_| ErrorCode::CredentialsStorageFailed)?;
    let result = with_db(app, |c| {
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let r = row(&tx, &v.profile_id)?;
        revision(&r, v.expected_revision)?;
        let referenced:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM ai_config_snapshots WHERE profile_id=?1) OR EXISTS(SELECT 1 FROM settings WHERE profile_id=?1) OR EXISTS(SELECT 1 FROM provider_operations WHERE profile_id=?1 AND phase NOT IN ('completed','failed','cancelled'))",[&v.profile_id],|r|r.get(0)).map_err(sql)?;
        if referenced {
            return Err(ErrorCode::ProviderReferenced);
        }
        if r.profile.credential.status != "missing" {
            return Err(ErrorCode::CredentialInvalid);
        }
        invalidate(&tx, &v.profile_id)?;
        audit(&tx, id, "config_switch", Some(&v.profile_id), "success")?;
        tx.execute("DELETE FROM provider_profiles WHERE id=?1", [&v.profile_id])
            .map_err(sql)?;
        tx.commit().map_err(sql)?;
        Ok(Removed { removed: true })
    });
    crate::app_state::publish(app, id.into());
    result
}
pub(crate) fn active(c: &Connection, at: i64) -> Result<(Option<Selection>, String), ErrorCode> {
    let (blocked,p,rev,m,e,cr,version,tested):(bool,Option<String>,Option<i64>,Option<String>,Option<String>,Option<i64>,Option<String>,Option<i64>)=c.query_row("SELECT blocked,profile_id,profile_revision,model_id,effort,credential_revision,adapter_version,tested_at FROM ai_runtime WHERE id=1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).map_err(sql)?;
    if blocked {
        return Ok((None, "error".into()));
    }
    if let (Some(p), Some(rev), Some(m), Some(cr), Some(t)) = (p, rev, m, cr, tested) {
        let s = Selection {
            profile_id: p,
            expected_revision: rev,
            model_id: m,
            effort: e,
        };
        if version.as_deref() == Some(ADAPTER_VERSION)
            && t <= at
            && at - t < TEST_TTL
            && validate_selection(c, &s, at)
                .is_ok_and(|v| v.profile.profile.credential.revision == cr && c.query_row("SELECT capability_version=?1 AND effective_effort=?2 FROM ai_runtime WHERE id=1",params![v.version,v.effort],|r|r.get::<_,bool>(0)).unwrap_or(false))
        {
            return Ok((Some(s), "ready".into()));
        }
    }
    let configured: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM settings WHERE scope='global' AND key='ai_defaults')",
            [],
            |r| r.get(0),
        )
        .map_err(sql)?;
    Ok((
        None,
        if configured {
            "degraded"
        } else {
            "unconfigured"
        }
        .into(),
    ))
}
pub fn list(app: &tauri::AppHandle) -> Result<ProviderList, ErrorCode> {
    with_db(app, |c| {
        let tx = c.transaction().map_err(sql)?;
        let ids = tx
            .prepare("SELECT id FROM provider_profiles ORDER BY created_at,id")
            .map_err(sql)?
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(sql)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql)?;
        let mut profiles = Vec::new();
        for p in ids {
            let mut r = row(&tx, &p)?.profile;
            if r.available && endpoint(&r.endpoint).is_err() {
                r.available = false;
                r.unavailable_code = Some("PA_PROVIDER_ENDPOINT_INVALID")
            }
            profiles.push(r)
        }
        let (active, config_state) = active(&tx, now()?)?;
        tx.commit().map_err(sql)?;
        Ok(ProviderList {
            profiles,
            active,
            config_state,
        })
    })
}
pub(crate) struct Validated {
    pub profile: Row,
    pub effort: String,
    pub source: String,
    pub version: String,
}
pub(crate) fn validate_selection(
    c: &Connection,
    s: &Selection,
    at: i64,
) -> Result<Validated, ErrorCode> {
    s.validate()?;
    let r = row(c, &s.profile_id)?;
    ensure_owned(&r)?;
    revision(&r, s.expected_revision)?;
    if !r.profile.enabled {
        return Err(ErrorCode::ProviderDisabled);
    }
    if !matches!(
        r.profile.credential.status,
        "configured_unverified" | "verified"
    ) || r.profile.credential.revision < 1
        || r.credential_ref
            != format!(
                "credential:{}:{}",
                s.profile_id, r.profile.credential.revision
            )
    {
        return Err(ErrorCode::CredentialInvalid);
    }
    let cap=c.query_row("SELECT effort,source_kind,source_version FROM model_capabilities WHERE profile_id=?1 AND model_id=?2 AND profile_revision=?3 AND observed_at<=?4 AND expires_at>?4 AND expires_at-observed_at<=?5 AND source_version LIKE ?6 AND ((?7 IS NOT NULL AND effort=?7) OR (?7 IS NULL AND is_default=1))",params![s.profile_id,s.model_id,s.expected_revision,at,CAPABILITY_TTL,format!("{ADAPTER_VERSION}:%"),s.effort],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional().map_err(sql)?.ok_or(ErrorCode::ProviderCapabilityInvalid)?;
    Ok(Validated {
        profile: r,
        effort: cap.0,
        source: cap.1,
        version: cap.2,
    })
}
pub(crate) fn cached(
    c: &Connection,
    p: &str,
    rev: i64,
    at: i64,
) -> Result<Vec<Capability>, ErrorCode> {
    let entries=c.prepare("SELECT model_id,effort,is_default,context_window,source_kind,source_version,observed_at,expires_at FROM model_capabilities WHERE profile_id=?1 AND profile_revision=?2 AND observed_at<=?3 AND expires_at>?3 AND expires_at-observed_at<=?4 AND source_version LIKE ?5 ORDER BY model_id,effort").map_err(sql)?.query_map(params![p,rev,at,CAPABILITY_TTL,format!("{ADAPTER_VERSION}:%")],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,bool>(2)?,r.get::<_,Option<i64>>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,i64>(6)?,r.get::<_,i64>(7)?))).map_err(sql)?.collect::<Result<Vec<_>,_>>().map_err(sql)?;
    let mut out: Vec<Capability> = Vec::new();
    for (model, effort, default, context, source, version, observed, expires) in entries {
        if let Some(last) = out.last_mut().filter(|v| v.model_id == model) {
            last.efforts.push(effort);
            last.supports_default_effort |= default
        } else {
            out.push(Capability {
                model_id: model,
                efforts: vec![effort],
                supports_default_effort: default,
                context_window: context,
                source,
                source_version: version,
                profile_revision: rev,
                observed_at: observed,
                expires_at: expires,
            })
        }
    }
    Ok(out)
}
pub fn models(app: &tauri::AppHandle, v: ModelsRequest, id: &str) -> Result<Models, ErrorCode> {
    ProfileRequest {
        profile_id: v.profile_id.clone(),
        expected_revision: v.expected_revision,
    }
    .validate()?;
    let existing = with_db(app, |c| {
        let r = row(c, &v.profile_id)?;
        ensure_owned(&r)?;
        revision(&r, v.expected_revision)?;
        if !r.profile.enabled {
            return Err(ErrorCode::ProviderDisabled);
        }
        cached(c, &v.profile_id, v.expected_revision, now()?)
    })?;
    if !v.refresh && !existing.is_empty() {
        return Ok(Models {
            capabilities: existing,
        });
    }
    let executable = super::operations::executable()?;
    let catalog = crate::owned_process::catalog_candidate(
        app,
        &executable,
        &v.profile_id,
        v.expected_revision,
    )?;
    let version = format!("{ADAPTER_VERSION}:{}", executable.identity());
    let capabilities = with_db(app, |c| {
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let r = row(&tx, &v.profile_id)?;
        ensure_owned(&r)?;
        revision(&r, v.expected_revision)?;
        let at = now()?;
        tx.execute(
            "DELETE FROM model_capabilities WHERE profile_id=?1",
            [&v.profile_id],
        )
        .map_err(sql)?;
        for entry in catalog {
            for effort in entry.efforts {
                tx.execute("INSERT INTO model_capabilities(profile_id,model_id,effort,is_default,context_window,source_kind,source_version,profile_revision,observed_at,expires_at) VALUES(?1,?2,?3,?4,NULL,'cli_catalog',?5,?6,?7,?8)",params![v.profile_id,entry.model_id,effort,entry.default_effort.as_deref()==Some(effort.as_str()),version,v.expected_revision,at,at+CAPABILITY_TTL]).map_err(sql)?;
            }
        }
        let result = cached(&tx, &v.profile_id, v.expected_revision, at)?;
        tx.commit().map_err(sql)?;
        Ok(result)
    })?;
    crate::app_state::publish(app, id.into());
    Ok(Models { capabilities })
}
