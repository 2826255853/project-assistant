use super::{repository as repo, *};
use crate::owned_process::{self, ADAPTER_VERSION, Cancel, Executable};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct ProvidersState {
    pub(crate) queue: Mutex<Queue>,
    pub(crate) initialized: std::sync::atomic::AtomicBool,
}
#[derive(Default)]
pub(crate) struct Queue {
    pub(super) pending: VecDeque<String>,
    running: bool,
    cancels: HashMap<String, Cancel>,
}

pub(crate) fn executable() -> Result<Executable, ErrorCode> {
    if let Some(path) = std::env::var_os("CODEX_BIN") {
        let p = std::path::PathBuf::from(path);
        if !p.is_absolute() {
            return Err(ErrorCode::CodexNotInstalled);
        }
        return Executable::inspect(&p);
    }
    #[cfg(windows)]
    {
        use std::io::Read;
        use std::os::windows::process::CommandExt;
        let system = std::env::var_os("SystemRoot").ok_or(ErrorCode::CodexNotInstalled)?;
        let ps =
            std::path::PathBuf::from(system).join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let mut child=std::process::Command::new(ps).args(["-NoProfile","-NonInteractive","-Command","$p=Get-AppxPackage -Name OpenAI.Codex -ErrorAction Stop; if($p){Join-Path $p.InstallLocation 'app/resources/codex.exe'}"]).creation_flags(0x08000000).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().map_err(|_|ErrorCode::CodexNotInstalled)?;
        let started = Instant::now();
        while child
            .try_wait()
            .map_err(|_| ErrorCode::CodexNotInstalled)?
            .is_none()
        {
            if started.elapsed() > Duration::from_secs(10) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ErrorCode::CodexNotInstalled);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let mut out = String::new();
        child
            .stdout
            .take()
            .ok_or(ErrorCode::CodexNotInstalled)?
            .take(4096)
            .read_to_string(&mut out)
            .map_err(|_| ErrorCode::CodexNotInstalled)?;
        return Executable::inspect(std::path::Path::new(out.trim()));
    }
    #[cfg(not(windows))]
    Err(ErrorCode::CodexNotInstalled)
}
pub(crate) struct Job {
    pub(super) id: String,
    pub(super) kind: String,
    pub(super) selection: Selection,
    pub(super) correlation: String,
    pub(super) settings_revision: i64,
}
pub(super) fn read_job(c: &Connection, id: &str) -> Result<Job, ErrorCode> {
    c.query_row("SELECT kind,profile_id,profile_revision,model_id,effort,correlation_id,settings_revision FROM provider_operations WHERE id=?1",[id],|r|Ok(Job{id:id.into(),kind:r.get(0)?,selection:Selection{profile_id:r.get(1)?,expected_revision:r.get(2)?,model_id:r.get(3)?,effort:r.get(4)?},correlation:r.get(5)?,settings_revision:r.get(6)?})).optional().map_err(sql)?.ok_or(ErrorCode::OperationNotFound)
}
pub(crate) fn settings_revision(c: &Connection) -> Result<i64, ErrorCode> {
    c.query_row(
        "SELECT revision FROM settings_revisions WHERE scope='global'",
        [],
        |r| r.get(0),
    )
    .map_err(sql)
}
fn phase(c: &Connection, id: &str) -> Result<String, ErrorCode> {
    c.query_row(
        "SELECT phase FROM provider_operations WHERE id=?1",
        [id],
        |r| r.get(0),
    )
    .optional()
    .map_err(sql)?
    .ok_or(ErrorCode::OperationNotFound)
}
fn terminal(p: &str) -> bool {
    matches!(p, "completed" | "failed" | "cancelled")
}
pub(crate) fn insert_job(
    c: &mut Connection,
    s: &Selection,
    kind: &str,
    correlation: &str,
) -> Result<String, ErrorCode> {
    s.validate()?;
    let tx = c
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    repo::validate_selection(&tx, s, now()?)?;
    let count:i64=tx.query_row("SELECT count(*) FROM provider_operations WHERE phase NOT IN ('completed','failed','cancelled')",[],|r|r.get(0)).map_err(sql)?;
    if count >= 8 {
        return Err(ErrorCode::ProviderQueueFull);
    }
    let id = uuid()?;
    repo::audit(
        &tx,
        correlation,
        "config_read",
        Some(&s.profile_id),
        "success",
    )?;
    tx.execute("INSERT INTO provider_operations(id,kind,profile_id,profile_revision,model_id,effort,correlation_id,phase,settings_revision,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,'checking',?8,?9)",params![id,kind,s.profile_id,s.expected_revision,s.model_id,s.effort,correlation,settings_revision(&tx)?,now()?]).map_err(sql)?;
    tx.commit().map_err(sql)?;
    Ok(id)
}
pub fn submit(
    app: &tauri::AppHandle,
    s: Selection,
    kind: &str,
    correlation: &str,
) -> Result<Receipt, ErrorCode> {
    let state = app.state::<ProvidersState>();
    let mut q = state
        .queue
        .lock()
        .map_err(|_| ErrorCode::InternalUnexpected)?;
    let id = with_db(app, |c| insert_job(c, &s, kind, correlation))?;
    q.pending.push_back(id.clone());
    q.cancels.insert(id.clone(), Cancel::default());
    if !q.running {
        q.running = true;
        let app = app.clone();
        std::thread::spawn(move || drain(app));
    }
    drop(q);
    crate::app_state::publish(app, correlation.into());
    Ok(Receipt {
        operation_id: id,
        state: "checking",
    })
}
fn drain(app: tauri::AppHandle) {
    loop {
        let next = {
            let state = app.state::<ProvidersState>();
            let Ok(mut q) = state.queue.lock() else {
                return;
            };
            match q.pending.pop_front() {
                Some(id) => {
                    let cancel = q.cancels.get(&id).cloned().unwrap_or_default();
                    Some((id, cancel))
                }
                None => {
                    q.running = false;
                    None
                }
            }
        };
        let Some((id, cancel)) = next else { return };
        let job = with_db(&app, |c| {
            if terminal(&phase(c, &id)?) {
                return Ok(None);
            }
            read_job(c, &id).map(Some)
        });
        if let Ok(Some(job)) = job {
            let backend = Production { app: &app };
            if let Err(error) = execute(&backend, &job, &cancel) {
                // A durable applying marker remains blocked even if the final write
                // itself failed. Never invent a successful terminal result.
                let _ = with_db(&app, |c| {
                    if matches!(phase(c, &id)?.as_str(), "applying" | "rolling_back") {
                        block(c)?;
                        finish_switch(
                            c,
                            &job,
                            "rollback_failed",
                            None,
                            "error",
                            Some(ErrorCode::ProviderRollbackFailed),
                        )
                    } else {
                        finish_failure(c, &job, error)
                    }
                });
            }
            crate::app_state::publish(&app, job.correlation.clone());
        }
        if let Ok(mut q) = app.state::<ProvidersState>().queue.lock() {
            q.cancels.remove(&id);
        }
    }
}
pub(crate) trait Backend {
    fn db<T>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T, ErrorCode>,
    ) -> Result<T, ErrorCode>;
    fn probe(&self, s: &Selection, effort: &str, cancel: Cancel) -> Result<(), ErrorCode>;
    fn notify(&self, id: &str);
    fn wait(&self) {
        std::thread::sleep(Duration::from_millis(40))
    }
}
struct Production<'a> {
    app: &'a tauri::AppHandle,
}
impl Backend for Production<'_> {
    fn db<T>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T, ErrorCode>,
    ) -> Result<T, ErrorCode> {
        with_db(self.app, f)
    }
    fn probe(&self, s: &Selection, effort: &str, cancel: Cancel) -> Result<(), ErrorCode> {
        let exe = executable()?;
        with_db(self.app, |c| {
            let v = repo::validate_selection(c, s, now()?)?;
            if v.version != format!("{ADAPTER_VERSION}:{}", exe.identity()) || v.effort != effort {
                return Err(ErrorCode::ProviderCapabilityInvalid);
            }
            Ok(())
        })?;
        owned_process::probe_candidate(
            self.app,
            &exe,
            &s.profile_id,
            s.expected_revision,
            &s.model_id,
            effort,
            cancel,
        )
        .and_then(|e| {
            if e.requests > 0
                && e.verified_responses > 0
                && e.exit.cleaned
                && !e.exit.forced
                && e.exit.code == Some(0)
            {
                Ok(())
            } else {
                Err(ErrorCode::ProviderTestFailed)
            }
        })
    }
    fn notify(&self, id: &str) {
        crate::app_state::publish(self.app, id.into())
    }
}
fn set_phase<B: Backend>(b: &B, j: &Job, p: &str) -> Result<(), ErrorCode> {
    b.db(|c| {
        c.execute(
            "UPDATE provider_operations SET phase=?1 WHERE id=?2",
            params![p, j.id],
        )
        .map_err(sql)?;
        Ok(())
    })?;
    b.notify(&j.correlation);
    Ok(())
}
pub(crate) fn finish_failure(
    c: &mut Connection,
    j: &Job,
    error: ErrorCode,
) -> Result<(), ErrorCode> {
    if j.kind == "switch" {
        let (active, state) = repo::active(c, now()?)?;
        finish_switch(c, j, "unchanged", active, state.as_str(), Some(error))
    } else {
        let code = serde_json::to_value(error).map_err(|_| ErrorCode::InternalUnexpected)?;
        c.execute(
            "UPDATE provider_operations SET phase=?1,error_code=?2,finished_at=?3 WHERE id=?4",
            params![
                if error == ErrorCode::ProcessCancelled {
                    "cancelled"
                } else {
                    "failed"
                },
                code.as_str(),
                now()?,
                j.id
            ],
        )
        .map_err(sql)?;
        Ok(())
    }
}
fn finish_switch(
    c: &Connection,
    j: &Job,
    outcome: &str,
    active: Option<Selection>,
    state: &str,
    error: Option<ErrorCode>,
) -> Result<(), ErrorCode> {
    let code = error
        .map(serde_json::to_value)
        .transpose()
        .map_err(|_| ErrorCode::InternalUnexpected)?;
    c.execute("UPDATE provider_operations SET phase='completed',outcome=?1,error_code=?2,active_profile_id=?3,active_revision=?4,active_model_id=?5,active_effort=?6,config_state=?7,finished_at=?8 WHERE id=?9",params![outcome,code.as_ref().and_then(serde_json::Value::as_str),active.as_ref().map(|s|&s.profile_id),active.as_ref().map(|s|s.expected_revision),active.as_ref().map(|s|&s.model_id),active.as_ref().and_then(|s|s.effort.as_deref()),state,now()?,j.id]).map_err(sql)?;
    Ok(())
}

fn record_rollback_evidence(
    c: &Connection,
    j: &Job,
    s: &Selection,
    credential: i64,
    effort: &str,
    version: &str,
    source: &str,
    tested_at: i64,
) -> Result<(), ErrorCode> {
    let id = uuid()?;
    c.execute("INSERT INTO provider_operations(id,kind,profile_id,profile_revision,model_id,effort,correlation_id,phase,outcome,tested_at,credential_revision,adapter_version,capability_version,effective_effort,settings_revision,created_at,finished_at) VALUES(?1,'test',?2,?3,?4,?5,?6,'completed',NULL,?7,?8,?9,?10,?11,?12,?7,?7)",
      params![id,s.profile_id,s.expected_revision,s.model_id,s.effort,j.correlation,tested_at,credential,ADAPTER_VERSION,version,effort,settings_revision(c)?]).map_err(sql)?;
    repo::audit(
        c,
        &j.correlation,
        "connection_test",
        Some(&s.profile_id),
        "success",
    )?;
    let _ = source;
    Ok(())
}
pub(crate) fn block(c: &Connection) -> Result<(), ErrorCode> {
    c.execute("UPDATE ai_runtime SET blocked=1,profile_id=NULL,profile_revision=NULL,model_id=NULL,effort=NULL,credential_revision=NULL,adapter_version=NULL,tested_at=NULL WHERE id=1",[]).map_err(sql)?;
    Ok(())
}
fn set_runtime(c: &Connection, s: &Selection, cr: i64, at: i64) -> Result<(), ErrorCode> {
    c.execute("UPDATE ai_runtime SET blocked=0,profile_id=?1,profile_revision=?2,model_id=?3,effort=?4,credential_revision=?5,adapter_version=?6,tested_at=?7 WHERE id=1",params![s.profile_id,s.expected_revision,s.model_id,s.effort,cr,ADAPTER_VERSION,at]).map_err(sql)?;
    let v = repo::validate_selection(c, s, at)?;
    c.execute(
        "UPDATE ai_runtime SET effective_effort=?1,capability_version=?2 WHERE id=1",
        params![v.effort, v.version],
    )
    .map_err(sql)?;
    Ok(())
}
pub(crate) fn execute<B: Backend>(b: &B, j: &Job, cancel: &Cancel) -> Result<(), ErrorCode> {
    if cancel.is_cancelled() {
        return b.db(|c| finish_failure(c, j, ErrorCode::ProcessCancelled));
    }
    let initial = b.db(|c| repo::validate_selection(c, &j.selection, now()?))?;
    // Queued switches capture the global revision on execution, not enqueue;
    // earlier FIFO entries may legitimately have committed a new default.
    let current_revision = b.db(|c| settings_revision(c))?;
    let current_job = Job {
        id: j.id.clone(),
        kind: j.kind.clone(),
        selection: j.selection.clone(),
        correlation: j.correlation.clone(),
        settings_revision: current_revision,
    };
    let j = &current_job;
    let cr = initial.profile.profile.credential.revision;
    let capability_version = initial.version;
    let effort = initial.effort;
    if let Err(error) = b.probe(&j.selection, &effort, cancel.clone()) {
        return b.db(|c| finish_failure(c, j, error));
    }
    let tested_at = now()?;
    if j.kind == "test" {
        return b.db(|c|{let tx=c.transaction_with_behavior(TransactionBehavior::Immediate).map_err(sql)?;let current=repo::validate_selection(&tx,&j.selection,now()?)?;
            if cr!=current.profile.profile.credential.revision||current.effort!=effort||current.version!=capability_version{return Err(ErrorCode::RevisionConflict)}
            if cancel.is_cancelled(){return Err(ErrorCode::ProcessCancelled)}
            repo::audit(&tx,&j.correlation,"connection_test",Some(&j.selection.profile_id),"success")?;
            tx.execute("UPDATE provider_profiles SET credential_status='verified' WHERE id=?1 AND config_revision=?2",params![j.selection.profile_id,j.selection.expected_revision]).map_err(sql)?;
            tx.execute("UPDATE provider_operations SET phase='completed',tested_at=?1,finished_at=?1,credential_revision=?3,adapter_version=?4,effective_effort=?5,capability_version=?6 WHERE id=?2",params![tested_at,j.id,cr,ADAPTER_VERSION,effort,capability_version]).map_err(sql)?;tx.commit().map_err(sql)
        });
    }
    set_phase(b, j, "waiting_for_idle")?;
    let started = Instant::now();
    loop {
        if cancel.is_cancelled() {
            return b.db(|c| finish_failure(c, j, ErrorCode::ProcessCancelled));
        }
        let busy = b.db(|c| {
            repo::validate_selection(c, &j.selection, now()?)?;
            let old = default_selection(c, None)?;
            Ok(repo::busy(c, &j.selection.profile_id)?
                || old
                    .as_ref()
                    .map(|s| repo::busy(c, &s.profile_id))
                    .transpose()?
                    .unwrap_or(false))
        })?;
        if !busy {
            break;
        }
        if started.elapsed() > Duration::from_secs(5 * 60) {
            return b.db(|c| finish_failure(c, j, ErrorCode::CodexRequestTimeout));
        }
        b.wait();
    }
    // Evidence can expire while waiting. Re-probe the precise combination,
    // never promote an old list or another operation's test to ready.
    if now()? - tested_at >= TEST_TTL {
        if let Err(error) = b.probe(&j.selection, &effort, cancel.clone()) {
            return b.db(|c| finish_failure(c, j, error));
        }
    }
    let old = b.db(|c| {
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let valid = repo::validate_selection(&tx, &j.selection, now()?)?;
        if valid.profile.profile.credential.revision != cr
            || valid.effort != effort
            || valid.version != capability_version
            || settings_revision(&tx)? != j.settings_revision
        {
            return Err(ErrorCode::RevisionConflict);
        }
        if repo::busy(&tx, &j.selection.profile_id)? {
            return Err(ErrorCode::CredentialBusy);
        }
        let old = default_selection(&tx, None)?;
        if let Some(s) = &old {
            if repo::busy(&tx, &s.profile_id)? {
                return Err(ErrorCode::CredentialBusy);
            }
        }
        if cancel.is_cancelled() {
            return Err(ErrorCode::ProcessCancelled);
        }
        // Journal BEFORE applying: a crash cannot leave a false ready state.
        block(&tx)?;
        tx.execute(
            "UPDATE provider_operations SET phase='applying' WHERE id=?1",
            [&j.id],
        )
        .map_err(sql)?;
        tx.commit().map_err(sql)?;
        Ok(old)
    })?;
    b.notify(&j.correlation);
    // A second isolated candidate verifies the actual applying phase. Once
    // applying starts cancel cannot interrupt commit/compensating recovery.
    let applied=b.probe(&j.selection,&effort,Cancel::default()).and_then(|_|b.db(|c|{
        let tx=c.transaction_with_behavior(TransactionBehavior::Immediate).map_err(sql)?;let valid=repo::validate_selection(&tx,&j.selection,now()?)?;
        if valid.profile.profile.credential.revision!=cr||valid.effort!=effort||valid.version!=capability_version||settings_revision(&tx)?!=j.settings_revision||repo::busy(&tx,&j.selection.profile_id)?{return Err(ErrorCode::RevisionConflict)}
        if let Some(old)=&old{if repo::busy(&tx,&old.profile_id)?{return Err(ErrorCode::CredentialBusy)}}
        let revision=j.settings_revision.checked_add(1).filter(|v|*v<=MAX_REVISION).ok_or(ErrorCode::RevisionConflict)?;
        tx.execute("INSERT INTO settings(id,scope,key,profile_id,model_id,effort,revision,updated_at) VALUES('global-ai_defaults','global','ai_defaults',?1,?2,?3,?4,?5) ON CONFLICT(key) WHERE scope='global' DO UPDATE SET profile_id=excluded.profile_id,model_id=excluded.model_id,effort=excluded.effort,revision=excluded.revision,updated_at=excluded.updated_at",params![j.selection.profile_id,j.selection.model_id,j.selection.effort,revision,now()?]).map_err(sql)?;
        tx.execute("INSERT INTO ai_default_revisions(setting_id,profile_revision) SELECT id,?1 FROM settings WHERE scope='global' AND key='ai_defaults' ON CONFLICT(setting_id) DO UPDATE SET profile_revision=excluded.profile_revision",[j.selection.expected_revision]).map_err(sql)?;
        tx.execute("UPDATE settings_revisions SET revision=?1 WHERE scope='global'",[revision]).map_err(sql)?;set_runtime(&tx,&j.selection,cr,now()?)?;
        tx.execute("UPDATE provider_profiles SET credential_status='verified' WHERE id=?1",[&j.selection.profile_id]).map_err(sql)?;
        repo::audit(&tx,&j.correlation,"config_switch",Some(&j.selection.profile_id),"success")?;
        tx.execute("UPDATE provider_operations SET tested_at=?1,credential_revision=?2,adapter_version=?3,effective_effort=?5,capability_version=?6 WHERE id=?4",params![now()?,cr,ADAPTER_VERSION,j.id,effort,capability_version]).map_err(sql)?;
        tx.execute("UPDATE ai_runtime SET effective_effort=?1,capability_version=?2 WHERE id=1",params![effort,capability_version]).map_err(sql)?;
        finish_switch(&tx,j,"applied",Some(j.selection.clone()),"ready",None)?;tx.commit().map_err(sql)
    }));
    let Err(failure) = applied else { return Ok(()) };
    set_phase(b, j, "rolling_back")?;
    let recovered = match &old {
        Some(s) => b
            .db(|c| repo::validate_selection(c, s, now()?))
            .and_then(|v| {
                let tested_at = now()?;
                b.probe(s, &v.effort, Cancel::default()).map(|_| {
                    Some((
                        s.clone(),
                        v.profile.profile.credential.revision,
                        v.effort,
                        v.version,
                        tested_at,
                        v.source,
                    ))
                })
            }),
        None => Ok(None),
    };
    b.db(|c| {
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let restored = match recovered {
            Ok(Some((ref s, ref credential, ref effort, ref version, _, _))) => {
                default_selection(&tx, None)?.as_ref() == Some(s)
                    && repo::validate_selection(&tx, s, now()?).is_ok_and(|v| {
                        v.profile.profile.credential.revision == *credential
                            && v.effort == *effort
                            && v.version == *version
                    })
            }
            Ok(None) => default_selection(&tx, None)?.is_none(),
            Err(_) => false,
        };
        if restored {
            match &recovered {
                Ok(Some((s, cr, effort, version, tested_at, source))) => {
                    set_runtime(&tx, s, *cr, *tested_at)?;
                    record_rollback_evidence(&tx, j, s, *cr, effort, version, source, *tested_at)?;
                }
                _ => {
                    tx.execute("UPDATE ai_runtime SET blocked=0 WHERE id=1", [])
                        .map_err(sql)?;
                }
            }
            repo::audit(
                &tx,
                &j.correlation,
                "config_rollback",
                old.as_ref().map(|s| s.profile_id.as_str()),
                "success",
            )?;
            finish_switch(
                &tx,
                j,
                "rolled_back",
                old.clone(),
                if old.is_some() {
                    "ready"
                } else {
                    "unconfigured"
                },
                Some(failure),
            )?;
        } else {
            block(&tx)?;
            repo::audit(&tx, &j.correlation, "config_rollback", None, "failure")?;
            finish_switch(
                &tx,
                j,
                "rollback_failed",
                None,
                "error",
                Some(ErrorCode::ProviderRollbackFailed),
            )?;
        }
        tx.commit().map_err(sql)
    })
}
pub(crate) fn default_selection(
    c: &Connection,
    project: Option<&str>,
) -> Result<Option<Selection>, ErrorCode> {
    c.query_row("SELECT s.profile_id,r.profile_revision,s.model_id,s.effort FROM settings s LEFT JOIN ai_default_revisions r ON r.setting_id=s.id WHERE s.key='ai_defaults' AND ((?1 IS NULL AND s.scope='global') OR (?1 IS NOT NULL AND s.scope='project' AND s.project_id=?1))",[project],|r|Ok(Selection{profile_id:r.get(0)?,expected_revision:r.get(1)?,model_id:r.get(2)?,effort:r.get(3)?})).optional().map_err(sql)
}
pub(crate) fn recover(c: &mut Connection) -> Result<(), ErrorCode> {
    let tx = c
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    let applying:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM provider_operations WHERE phase IN ('applying','rolling_back'))",[],|r|r.get(0)).map_err(sql)?;
    let pending:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM provider_operations WHERE phase NOT IN ('completed','failed','cancelled'))",[],|r|r.get(0)).map_err(sql)?;
    if pending {
        repo::audit(
            &tx,
            &crate::ipc::correlation_id(),
            "storage_recovery",
            None,
            "interrupted",
        )?;
    }
    // Persisted operation results remain queryable history, but do not grant
    // task readiness after process restart or a clock reset.
    tx.execute(
        "UPDATE provider_operations SET adapter_version=NULL WHERE tested_at IS NOT NULL",
        [],
    )
    .map_err(sql)?;
    // No network replay on startup; require a fresh explicit user operation.
    if applying {
        block(&tx)?;
    } else {
        tx.execute("UPDATE ai_runtime SET profile_id=NULL,profile_revision=NULL,model_id=NULL,effort=NULL,credential_revision=NULL,adapter_version=NULL,tested_at=NULL WHERE id=1",[]).map_err(sql)?;
    }
    tx.execute("UPDATE provider_operations SET phase='completed',outcome='rollback_failed',config_state='error',error_code='PA_PROVIDER_ROLLBACK_FAILED',finished_at=?1 WHERE phase IN ('applying','rolling_back')",[now()?]).map_err(sql)?;
    tx.execute("UPDATE provider_operations SET phase='completed',outcome='unchanged',config_state=CASE WHEN EXISTS(SELECT 1 FROM settings WHERE key='ai_defaults' AND scope='global') THEN 'degraded' ELSE 'unconfigured' END,error_code='PA_CODEX_DISCONNECTED',finished_at=?1 WHERE kind='switch' AND phase IN ('checking','waiting_for_idle')",[now()?]).map_err(sql)?;
    tx.execute("UPDATE provider_operations SET phase='cancelled',error_code='PA_PROCESS_CANCELLED',finished_at=?1 WHERE phase IN ('checking','waiting_for_idle')",[now()?]).map_err(sql)?;
    tx.commit().map_err(sql)
}
pub(super) fn operation(c: &Connection, id: &str) -> Result<OperationView, ErrorCode> {
    let job = read_job(c, id)?;
    let (phase,outcome,error,tested,p,rev,m,e,state):(String,Option<String>,Option<String>,Option<i64>,Option<String>,Option<i64>,Option<String>,Option<String>,Option<String>)=c.query_row("SELECT phase,outcome,error_code,tested_at,active_profile_id,active_revision,active_model_id,active_effort,config_state FROM provider_operations WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).map_err(sql)?;
    let error = error.map(|v| {
        serde_json::from_value::<ErrorCode>(serde_json::Value::String(v))
            .unwrap_or(ErrorCode::InternalUnexpected)
            .public()
    });
    let mut view = OperationView {
        operation_id: id.into(),
        phase: phase.clone(),
        result: None,
        error: None,
    };
    match phase.as_str() {
        "completed" if job.kind == "test" => {
            view.result = Some(OperationResult::Test(ConnectionTest {
                tested_revision: job.selection.expected_revision,
                tested_at: tested.ok_or(ErrorCode::SettingsCorrupt)?,
                authenticated: true,
                model_accepted: true,
                effort_accepted: true,
                evidence: "connection_probe",
            }))
        }
        "completed" => {
            let outcome = match outcome.as_deref() {
                Some("applied") => "applied",
                Some("rolled_back") => "rolled_back",
                Some("rollback_failed") => "rollback_failed",
                Some("unchanged") => "unchanged",
                _ => return Err(ErrorCode::SettingsCorrupt),
            };
            let active = match (p, rev, m) {
                (Some(p), Some(r), Some(m)) => Some(Selection {
                    profile_id: p,
                    expected_revision: r,
                    model_id: m,
                    effort: e,
                }),
                _ => None,
            };
            view.result = Some(OperationResult::Switch(SwitchResult {
                outcome,
                active,
                config_state: state.ok_or(ErrorCode::SettingsCorrupt)?,
                error,
            }));
        }
        "failed" | "cancelled" => view.error = Some(error.ok_or(ErrorCode::SettingsCorrupt)?),
        _ => {}
    }
    Ok(view)
}
pub fn get_operation(
    app: &tauri::AppHandle,
    v: OperationRequest,
) -> Result<OperationView, ErrorCode> {
    if !valid_id(&v.operation_id) {
        return Err(ErrorCode::OperationNotFound);
    }
    with_db(app, |c| operation(c, &v.operation_id))
}
pub fn cancel_operation(
    app: &tauri::AppHandle,
    v: OperationRequest,
) -> Result<Accepted, ErrorCode> {
    if !valid_id(&v.operation_id) {
        return Err(ErrorCode::OperationNotFound);
    }
    let state = app.state::<ProvidersState>();
    let q = state
        .queue
        .lock()
        .map_err(|_| ErrorCode::InternalUnexpected)?;
    let accepted = with_db(app, |c| {
        let p = phase(c, &v.operation_id)?;
        if !matches!(p.as_str(), "checking" | "waiting_for_idle") {
            return Ok(false);
        }
        let Some(cancel) = q.cancels.get(&v.operation_id) else {
            return Ok(false);
        };
        cancel.cancel();
        Ok(true)
    })?;
    Ok(Accepted { accepted })
}
