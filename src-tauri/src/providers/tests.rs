use super::{operations::*, repository as repo, *};
use crate::{
    owned_process::{ADAPTER_VERSION, Cancel},
    storage::Storage,
};
use rusqlite::params;
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    path::PathBuf,
};
#[cfg(windows)]
struct Actual {
    fixture: Fixture,
    exe: crate::owned_process::Executable,
    root: PathBuf,
    endpoint: String,
    key: String,
    ca: Vec<u8>,
}
#[cfg(windows)]
impl Backend for Actual {
    fn db<T>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T, ErrorCode>,
    ) -> Result<T, ErrorCode> {
        self.fixture.db(f)
    }
    fn probe(&self, s: &Selection, effort: &str, cancel: Cancel) -> Result<(), ErrorCode> {
        let cr = self.db(|c| {
            repo::validate_selection(c, s, now()?).map(|v| v.profile.profile.credential.revision)
        })?;
        let secret = crate::credentials::provider_fixture_secret(
            &mut self.fixture.db.borrow_mut(),
            &s.profile_id,
            &self.key,
        );
        crate::owned_process::provider_acceptance_probe(
            &self.root,
            &self.exe,
            &s.profile_id,
            s.expected_revision,
            cr,
            &self.endpoint,
            &s.model_id,
            effort,
            secret,
            self.ca.clone(),
            cancel,
        )
    }
    fn notify(&self, id: &str) {
        self.fixture.notify(id)
    }
}
#[test]
#[cfg(windows)]
#[ignore = "invoked by verify:providers with synthetic TLS sites and an explicitly selected CLI"]
fn actual_cli_provider_service_acceptance() {
    let exe = crate::owned_process::Executable::inspect(std::path::Path::new(
        &std::env::var("PA_PROVIDER_CODEX_BIN").unwrap(),
    ))
    .unwrap();
    let root = PathBuf::from(std::env::var("PA_PROVIDER_ROOT").unwrap());
    let endpoint = std::env::var("PA_PROVIDER_ENDPOINT").unwrap();
    let ca = std::fs::read(std::env::var("PA_PROVIDER_CA").unwrap()).unwrap();
    let key = std::env::var("PA_PROVIDER_SYNTHETIC_KEY").unwrap();
    assert!(key.starts_with("synthetic-provider-"));
    let scenario = std::env::var("PA_PROVIDER_SCENARIO").unwrap();
    if scenario == "catalog" {
        let entries =
            crate::owned_process::provider_acceptance_catalog(&root, &exe, &endpoint, ca).unwrap();
        assert!(!entries.is_empty());
        println!(
            "PROVIDER_EVIDENCE:{}",
            serde_json::json!({"scenario":scenario,"models":entries.len(),"cleaned":std::fs::read_dir(&root).unwrap().next().is_none()})
        );
        return;
    }
    let fixture = Fixture::new();
    let mut selected = fixture.seed("Actual");
    selected.model_id = "gpt-5.1-codex-mini".into();
    selected.effort = Some("medium".into());
    fixture.db(|c|{c.execute("DELETE FROM model_capabilities",[]).map_err(sql)?;let at=now()?;c.execute("INSERT INTO model_capabilities VALUES(?1,?2,'medium',NULL,1,'cli_catalog',?3,2,?4,?5)",params![selected.profile_id,selected.model_id,format!("{ADAPTER_VERSION}:synthetic-acceptance"),at,at+CAPABILITY_TTL]).map_err(sql)?;Ok(())}).unwrap();
    let backend = Actual {
        fixture,
        exe,
        root,
        endpoint,
        key,
        ca,
    };
    let kind = if scenario == "test" { "test" } else { "switch" };
    let id = backend
        .db(|c| insert_job(c, &selected, kind, &crate::ipc::correlation_id()))
        .unwrap();
    let j = backend.db(|c| read_job(c, &id)).unwrap();
    let cancel = Cancel::default();
    if scenario == "cancel" {
        let flag = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(12));
            flag.cancel()
        });
    }
    if let Err(e) = execute(&backend, &j, &cancel) {
        backend.db(|c| finish_failure(c, &j, e)).unwrap();
    }
    let v = backend.db(|c| operation(c, &id)).unwrap();
    let mut snapshot = false;
    if scenario == "test" {
        assert_eq!(v.phase, "completed");
        assert!(backend.fixture.defaults().is_none());
    } else {
        let r = switch(v);
        assert_eq!(
            r.outcome,
            if matches!(scenario.as_str(), "redirect" | "cancel") {
                "unchanged"
            } else {
                "applied"
            }
        );
        if r.outcome == "applied" {
            let snap = backend
                .db(|c| {
                    let tx = c.transaction().map_err(sql)?;
                    let s = snapshots::capture_in_transaction(&tx, None, None)?;
                    tx.commit().map_err(sql)?;
                    Ok(s)
                })
                .unwrap();
            assert_eq!(snap.profile_id, selected.profile_id);
            assert_eq!(snap.credential_revision, 1);
            assert_eq!(snap.effort.as_deref(), Some("medium"));
            snapshot = true;
        }
    }
    let cleaned = std::fs::read_dir(&backend.root).unwrap().next().is_none();
    assert!(cleaned);
    let bytes = std::fs::read(
        backend
            .fixture
            .directory
            .join(crate::storage::DATABASE_NAME),
    )
    .unwrap();
    assert!(
        !bytes
            .windows(backend.key.len())
            .any(|v| v == backend.key.as_bytes())
    );
    println!(
        "PROVIDER_EVIDENCE:{}",
        serde_json::json!({"scenario":scenario,"snapshot":snapshot,"cleaned":cleaned,"credentialTargetRemoved":true})
    );
}

struct Fixture {
    db: RefCell<Storage>,
    directory: PathBuf,
    results: RefCell<VecDeque<Result<(), ErrorCode>>>,
    probes: Cell<usize>,
    changes: RefCell<Vec<String>>,
    change_revision: Cell<bool>,
    change_settings: Cell<bool>,
    release_idle: Cell<bool>,
    cancel_wait: RefCell<Option<Cancel>>,
}
impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!("pa-providers-{}", uuid().unwrap()));
        let mut db = Storage::open(&directory).unwrap();
        db.settings().initialize_defaults(&directory).unwrap();
        Self {
            db: RefCell::new(db),
            directory,
            results: RefCell::new(VecDeque::new()),
            probes: Cell::new(0),
            changes: RefCell::new(vec![]),
            change_revision: Cell::new(false),
            change_settings: Cell::new(false),
            release_idle: Cell::new(false),
            cancel_wait: RefCell::new(None),
        }
    }
    fn seed(&self, name: &str) -> Selection {
        self.db(|c|{
  let p=repo::mutate_save(c,Save{profile_id:None,expected_revision:None,name:name.into(),endpoint:format!("https://{}.invalid/v1",name.to_lowercase()),default_model:None,default_effort:None},&crate::ipc::correlation_id())?;
  c.execute("UPDATE provider_profiles SET credential_revision=1,credential_ref=?1,credential_status='configured_unverified',config_revision=2 WHERE id=?2",params![format!("credential:{}:1",p.id),p.id]).map_err(sql)?;
  let at=now()?;for effort in ["low","high"]{c.execute("INSERT INTO model_capabilities VALUES(?1,'synthetic-model',?2,NULL,?3,'cli_catalog',?4,2,?5,?6)",params![p.id,effort,effort=="low",format!("{ADAPTER_VERSION}:synthetic-version"),at,at+CAPABILITY_TTL]).map_err(sql)?;}
  Ok(Selection{profile_id:p.id,expected_revision:2,model_id:"synthetic-model".into(),effort:Some("high".into())})
 }).unwrap()
    }
    fn run(&self, s: &Selection, kind: &str, cancel: Cancel) -> OperationView {
        let id = self
            .db(|c| insert_job(c, s, kind, &crate::ipc::correlation_id()))
            .unwrap();
        let j = self.db(|c| read_job(c, &id)).unwrap();
        if let Err(e) = execute(self, &j, &cancel) {
            self.db(|c| finish_failure(c, &j, e)).unwrap();
        }
        self.db(|c| operation(c, &id)).unwrap()
    }
    fn outcome(&self, v: OperationView) -> String {
        match v.result.unwrap() {
            OperationResult::Switch(s) => s.outcome.into(),
            _ => panic!("not a switch"),
        }
    }
    fn defaults(&self) -> Option<Selection> {
        self.db(|c| default_selection(c, None)).unwrap()
    }
}
impl Backend for Fixture {
    fn db<T>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T, ErrorCode>,
    ) -> Result<T, ErrorCode> {
        f(&mut self.db.borrow_mut().connection)
    }
    fn probe(&self, s: &Selection, _: &str, cancel: Cancel) -> Result<(), ErrorCode> {
        self.probes.set(self.probes.get() + 1);
        if cancel.is_cancelled() {
            return Err(ErrorCode::ProcessCancelled);
        }
        if self.change_revision.replace(false) {
            self.db(|c| {
                c.execute(
                    "UPDATE provider_profiles SET config_revision=config_revision+1 WHERE id=?1",
                    [&s.profile_id],
                )
                .map_err(sql)?;
                Ok(())
            })?;
        }
        if self.change_settings.replace(false) {
            self.db(|c| {
                c.execute(
                    "UPDATE settings_revisions SET revision=revision+1 WHERE scope='global'",
                    [],
                )
                .map_err(sql)?;
                Ok(())
            })?;
        }
        self.results.borrow_mut().pop_front().unwrap_or(Ok(()))
    }
    fn notify(&self, _: &str) {
        self.db(|c| {
            let p: String = c
                .query_row(
                    "SELECT phase FROM provider_operations ORDER BY rowid DESC LIMIT 1",
                    [],
                    |r| r.get(0),
                )
                .map_err(sql)?;
            self.changes.borrow_mut().push(p);
            Ok(())
        })
        .unwrap();
    }
    fn wait(&self) {
        if self.release_idle.replace(false) {
            self.db(|c|{c.execute("UPDATE tasks SET status='interrupted',finished_at=?1 WHERE finished_at IS NULL",[now()?]).map_err(sql)?;Ok(())}).unwrap();
        }
        if let Some(c) = self.cancel_wait.borrow_mut().take() {
            c.cancel();
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let old = std::mem::replace(
            &mut self.db.get_mut().connection,
            Connection::open_in_memory().unwrap(),
        );
        drop(old);
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}
fn occupy(f: &Fixture, s: &Selection) {
    f.run(s, "test", Cancel::default());
    f.db(|c|{let tx=c.transaction().map_err(sql)?;let snapshot=snapshots::capture_in_transaction(&tx,Some(s),None)?;let at=now()?;
 tx.execute("INSERT OR IGNORE INTO projects(id,canonical_path,path_key,name,display_name,created_at,updated_at) VALUES('synthetic-project','C:/synthetic','synthetic','Synthetic',NULL,?1,?1)",[at]).map_err(sql)?;
 tx.execute("INSERT OR IGNORE INTO sessions VALUES('synthetic-session','synthetic-project',NULL,NULL,'active',?1,?1)",[at]).map_err(sql)?;
 tx.execute("INSERT INTO tasks(id,session_id,snapshot_id,status,created_at,started_at) VALUES('synthetic-task','synthetic-session',?1,'running',?2,?2)",params![snapshot.id,at]).map_err(sql)?;tx.commit().map_err(sql)
 }).unwrap();
}
#[test]
fn waiting_for_affected_task_idle_preserves_task_and_only_then_applies() {
    let f = Fixture::new();
    let a = f.seed("A");
    let b = f.seed("B");
    f.run(&a, "switch", Cancel::default());
    occupy(&f, &a);
    f.release_idle.set(true);
    let r = switch(f.run(&b, "switch", Cancel::default()));
    assert_eq!(r.outcome, "applied");
    assert!(f.changes.borrow().contains(&"waiting_for_idle".into()));
    let p=f.db(|c|c.query_row("SELECT s.profile_id FROM tasks t JOIN ai_config_snapshots s ON s.id=t.snapshot_id WHERE t.id='synthetic-task'",[],|r|r.get::<_,String>(0)).map_err(sql)).unwrap();
    assert_eq!(p, a.profile_id);
}
#[test]
fn cancelling_while_waiting_does_not_stop_task_or_change_defaults() {
    let f = Fixture::new();
    let a = f.seed("A");
    let b = f.seed("B");
    f.run(&a, "switch", Cancel::default());
    occupy(&f, &a);
    let cancel = Cancel::default();
    *f.cancel_wait.borrow_mut() = Some(cancel.clone());
    assert_eq!(f.outcome(f.run(&b, "switch", cancel)), "unchanged");
    assert_eq!(f.defaults(), Some(a));
    assert_eq!(
        f.db(|c| c
            .query_row(
                "SELECT status FROM tasks WHERE id='synthetic-task'",
                [],
                |r| r.get::<_, String>(0)
            )
            .map_err(sql))
            .unwrap(),
        "running"
    );
}
#[test]
fn selection_resolution_uses_complete_groups_without_fallback() {
    let f = Fixture::new();
    let a = f.seed("A");
    let b = f.seed("B");
    f.run(&a, "switch", Cancel::default());
    f.db(|c|{let at=now()?;c.execute("INSERT INTO projects(id,canonical_path,path_key,name,display_name,created_at,updated_at) VALUES('project','C:/synthetic','synthetic','Synthetic',NULL,?1,?1)",[at]).map_err(sql)?;c.execute("INSERT INTO settings(id,scope,project_id,key,profile_id,model_id,effort,revision,updated_at) VALUES('project-ai','project','project','ai_defaults',?1,?2,NULL,1,?3)",params![b.profile_id,b.model_id,at]).map_err(sql)?;c.execute("INSERT INTO ai_default_revisions VALUES('project-ai',2)",[]).map_err(sql)?;let p=snapshots::resolve(c,None,Some("project"))?;assert_eq!(p.profile_id,b.profile_id);assert_eq!(p.effort,None);assert_eq!(snapshots::resolve(c,Some(&a),Some("project"))?,a);c.execute("UPDATE ai_default_revisions SET profile_revision=99 WHERE setting_id='project-ai'",[]).map_err(sql)?;let stale=snapshots::resolve(c,None,Some("project"))?;assert_eq!(repo::validate_selection(c,&stale,at).err(),Some(ErrorCode::RevisionConflict));Ok(())}).unwrap();
}
#[test]
fn restart_removes_snapshot_admission_evidence_without_network() {
    let f = Fixture::new();
    let a = f.seed("A");
    f.run(&a, "test", Cancel::default());
    f.db(recover).unwrap();
    assert_eq!(
        f.db(|c| {
            let tx = c.transaction().map_err(sql)?;
            snapshots::capture_in_transaction(&tx, Some(&a), None)
        })
        .err(),
        Some(ErrorCode::ProviderTestFailed)
    );
}
#[test]
fn same_queue_switches_observe_previous_commit_instead_of_conflicting() {
    let f = Fixture::new();
    let a = f.seed("A");
    let b = f.seed("B");
    let first = f
        .db(|c| insert_job(c, &a, "switch", &crate::ipc::correlation_id()))
        .unwrap();
    let second = f
        .db(|c| insert_job(c, &b, "switch", &crate::ipc::correlation_id()))
        .unwrap();
    for id in [first, second] {
        let j = f.db(|c| read_job(c, &id)).unwrap();
        execute(&f, &j, &Cancel::default()).unwrap();
        assert_eq!(
            switch(f.db(|c| operation(c, &id)).unwrap()).outcome,
            "applied"
        );
    }
    assert_eq!(f.defaults(), Some(b));
}
#[test]
fn default_effort_change_invalidates_connection_evidence() {
    let f = Fixture::new();
    let mut a = f.seed("A");
    a.effort = None;
    f.run(&a, "test", Cancel::default());
    f.db(|c| {
        c.execute(
            "UPDATE model_capabilities SET is_default=(effort='high')",
            [],
        )
        .map_err(sql)?;
        Ok(())
    })
    .unwrap();
    assert_eq!(
        f.db(|c| {
            let tx = c.transaction().map_err(sql)?;
            snapshots::capture_in_transaction(&tx, Some(&a), None)
        })
        .err(),
        Some(ErrorCode::ProviderTestFailed)
    );
}
fn switch(v: OperationView) -> SwitchResult {
    match v.result.unwrap() {
        OperationResult::Switch(s) => s,
        _ => panic!("not a switch"),
    }
}

#[test]
fn profile_metadata_strict_endpoint_revision_and_audit_atomicity() {
    let f = Fixture::new();
    let s = f.seed("A");
    for bad in [
        "http://example.com/v1",
        "https://u:synthetic@example.com/v1",
        "https://example.com/v1?key=synthetic",
        "https://example.com/v1#secret",
        "https://api.openai.com/v1",
        "https://localhost/v1",
        "https://example.com/secret-token",
        "https://example.com/%76%31",
    ] {
        assert!(repo::endpoint(bad).is_err(), "{bad}")
    }
    assert!(
        repo::endpoint("https://127.0.0.1/v1").is_ok(),
        "endpoint syntax is distinct from route resolution"
    );
    let edit = || Save {
        profile_id: Some(s.profile_id.clone()),
        expected_revision: Some(2),
        name: "Renamed".into(),
        endpoint: "https://a.invalid/v1".into(),
        default_model: None,
        default_effort: None,
    };
    f.db(|c|{c.execute_batch("CREATE TEMP TRIGGER fail_audit BEFORE INSERT ON audit_events BEGIN SELECT RAISE(ABORT,'synthetic-private');END;").map_err(sql)}).unwrap();
    assert_eq!(
        f.db(|c| repo::mutate_save(c, edit(), &crate::ipc::correlation_id()))
            .err(),
        Some(ErrorCode::AuditWriteFailed)
    );
    assert_eq!(
        f.db(|c| repo::row(c, &s.profile_id).map(|r| r.profile.config_revision))
            .unwrap(),
        2
    );
    f.db(|c| c.execute_batch("DROP TRIGGER fail_audit").map_err(sql))
        .unwrap();
    assert_eq!(
        f.db(|c| repo::mutate_save(c, edit(), &crate::ipc::correlation_id()))
            .unwrap()
            .config_revision,
        3
    );
    assert_eq!(
        f.db(|c| repo::mutate_save(c, edit(), &crate::ipc::correlation_id()))
            .err(),
        Some(ErrorCode::RevisionConflict)
    );
    assert!(
        f.db(|c| repo::cached(c, &s.profile_id, 3, now()?))
            .unwrap()
            .is_empty()
    );
}
#[test]
fn capability_ttl_adapter_revision_and_effort_fail_closed() {
    let f = Fixture::new();
    let s = f.seed("A");
    let at = now().unwrap();
    assert!(f.db(|c| repo::validate_selection(c, &s, at)).is_ok());
    let mut null = s.clone();
    null.effort = None;
    assert_eq!(
        f.db(|c| repo::validate_selection(c, &null, at))
            .unwrap()
            .effort,
        "low"
    );
    let mut bad = s.clone();
    bad.effort = Some("unknown".into());
    assert_eq!(
        f.db(|c| repo::validate_selection(c, &bad, at)).err(),
        Some(ErrorCode::ProviderCapabilityInvalid)
    );
    assert_eq!(
        f.db(|c| repo::validate_selection(c, &s, at + CAPABILITY_TTL))
            .err(),
        Some(ErrorCode::ProviderCapabilityInvalid)
    );
    f.db(|c| {
        c.execute(
            "UPDATE model_capabilities SET source_version='old-adapter'",
            [],
        )
        .map_err(sql)?;
        Ok(())
    })
    .unwrap();
    assert_eq!(
        f.db(|c| repo::validate_selection(c, &s, at)).err(),
        Some(ErrorCode::ProviderCapabilityInvalid)
    );
}
#[test]
fn list_and_connection_test_never_apply_defaults() {
    let f = Fixture::new();
    let s = f.seed("A");
    assert_eq!(f.db(|c| repo::active(c, now()?)).unwrap().1, "unconfigured");
    assert_eq!(f.run(&s, "test", Cancel::default()).phase, "completed");
    assert!(f.defaults().is_none());
    assert_eq!(f.db(|c| repo::active(c, now()?)).unwrap().1, "unconfigured");
}
#[test]
fn switch_applies_atomically_and_advances_global_revision() {
    let f = Fixture::new();
    let s = f.seed("A");
    let before = f.db(|c| settings_revision(c)).unwrap();
    assert_eq!(f.outcome(f.run(&s, "switch", Cancel::default())), "applied");
    assert_eq!(f.defaults(), Some(s.clone()));
    assert_eq!(f.db(|c| settings_revision(c)).unwrap(), before + 1);
    assert_eq!(
        f.db(|c| repo::active(c, now()?)).unwrap(),
        (Some(s), "ready".into())
    );
    assert_eq!(*f.changes.borrow(), vec!["waiting_for_idle", "applying"]);
    assert!(f.db.borrow_mut().settings().read_view().is_ok());
}
#[test]
fn failed_candidate_is_unchanged_and_never_touches_defaults() {
    let f = Fixture::new();
    let a = f.seed("A");
    let b = f.seed("B");
    f.run(&a, "switch", Cancel::default());
    f.results
        .borrow_mut()
        .push_back(Err(ErrorCode::ProviderTestFailed));
    let r = switch(f.run(&b, "switch", Cancel::default()));
    assert_eq!(r.outcome, "unchanged");
    assert_eq!(r.active, Some(a.clone()));
    assert_eq!(f.defaults(), Some(a));
}
#[test]
fn applying_failure_restores_and_verifies_previous_selection() {
    let f = Fixture::new();
    let a = f.seed("A");
    let b = f.seed("B");
    f.run(&a, "switch", Cancel::default());
    f.db(|c|{c.execute("UPDATE provider_operations SET tested_at=?1 WHERE kind='switch' AND outcome='applied' AND profile_id=?2",rusqlite::params![now()?-TEST_TTL-1,&a.profile_id]).map_err(sql)?;Ok(())}).unwrap();
    f.results
        .borrow_mut()
        .extend([Ok(()), Err(ErrorCode::ProviderTestFailed), Ok(())]);
    let r = switch(f.run(&b, "switch", Cancel::default()));
    assert_eq!(r.outcome, "rolled_back");
    assert_eq!(r.active, Some(a.clone()));
    assert_eq!(f.defaults(), Some(a));
    assert!(f.changes.borrow().contains(&"rolling_back".into()));
    assert!(
        f.db(|c| {
            let tx = c.transaction().map_err(sql)?;
            snapshots::capture_in_transaction(&tx, None, None)
        })
        .is_ok()
    );
}
#[test]
fn rollback_failure_blocks_snapshots_and_explicit_switch_recovers() {
    let f = Fixture::new();
    let a = f.seed("A");
    let b = f.seed("B");
    f.run(&a, "switch", Cancel::default());
    f.results.borrow_mut().extend([
        Ok(()),
        Err(ErrorCode::ProviderTestFailed),
        Err(ErrorCode::ProviderTestFailed),
    ]);
    let r = switch(f.run(&b, "switch", Cancel::default()));
    assert_eq!(r.outcome, "rollback_failed");
    assert!(r.active.is_none());
    assert_eq!(
        f.db(|c| repo::active(c, now()?)).unwrap(),
        (None, "error".into())
    );
    assert_eq!(
        f.db(|c| {
            let tx = c.transaction().map_err(sql)?;
            snapshots::capture_in_transaction(&tx, Some(&a), None)
        })
        .err(),
        Some(ErrorCode::ProviderRollbackFailed)
    );
    assert_eq!(f.outcome(f.run(&b, "switch", Cancel::default())), "applied");
}
#[test]
fn cancelled_candidate_is_unchanged_without_probe() {
    let f = Fixture::new();
    let a = f.seed("A");
    let cancel = Cancel::default();
    cancel.cancel();
    assert_eq!(f.outcome(f.run(&a, "switch", cancel)), "unchanged");
    assert_eq!(f.probes.get(), 0);
    assert!(f.defaults().is_none());
}
#[test]
fn concurrent_profile_revision_is_rechecked_before_apply() {
    let f = Fixture::new();
    let a = f.seed("A");
    f.change_revision.set(true);
    let r = switch(f.run(&a, "switch", Cancel::default()));
    assert_eq!(r.outcome, "unchanged");
    assert_eq!(r.error.unwrap().code, ErrorCode::RevisionConflict);
    assert!(f.defaults().is_none());
}
#[test]
fn queue_has_eight_item_limit_and_fifo_order() {
    let f = Fixture::new();
    let a = f.seed("A");
    let mut ids = vec![];
    for _ in 0..8 {
        ids.push(
            f.db(|c| insert_job(c, &a, "test", &crate::ipc::correlation_id()))
                .unwrap(),
        )
    }
    assert_eq!(
        f.db(|c| insert_job(c, &a, "test", &crate::ipc::correlation_id()))
            .err(),
        Some(ErrorCode::ProviderQueueFull)
    );
    let mut q = Queue::default();
    for id in &ids {
        q.pending.push_back(id.clone())
    }
    for id in ids {
        assert_eq!(q.pending.pop_front(), Some(id))
    }
}
#[test]
fn snapshots_are_immutable_and_do_not_follow_key_or_defaults() {
    let f = Fixture::new();
    let a = f.seed("A");
    let b = f.seed("B");
    f.run(&a, "test", Cancel::default());
    let snapshot = f
        .db(|c| {
            let tx = c.transaction().map_err(sql)?;
            let s = snapshots::capture_in_transaction(&tx, Some(&a), None)?;
            tx.commit().map_err(sql)?;
            Ok(s)
        })
        .unwrap();
    f.run(&b, "switch", Cancel::default());
    f.db(|c|{c.execute("UPDATE provider_profiles SET credential_revision=2,credential_ref=?1,config_revision=3 WHERE id=?2",params![format!("credential:{}:2",a.profile_id),a.profile_id]).map_err(sql)?;assert!(c.execute("UPDATE ai_config_snapshots SET model_id='other' WHERE id=?1",[&snapshot.id]).is_err());Ok(())}).unwrap();
    let unchanged = f.db(|c| snapshots::get(c, &snapshot.id)).unwrap();
    assert_eq!(unchanged.credential_revision, 1);
    assert_eq!(unchanged.config_revision, 2);
    assert_eq!(unchanged.profile_id, a.profile_id);
    assert!(
        !serde_json::to_string(&unchanged)
            .unwrap()
            .contains("credential_ref")
    );
}
#[test]
fn evidence_expires_and_full_selection_is_required_for_snapshot() {
    let f = Fixture::new();
    let a = f.seed("A");
    f.run(&a, "test", Cancel::default());
    let mut changed = a.clone();
    changed.effort = Some("low".into());
    assert_eq!(
        f.db(|c| {
            let tx = c.transaction().map_err(sql)?;
            snapshots::capture_in_transaction(&tx, Some(&changed), None)
        })
        .err(),
        Some(ErrorCode::ProviderTestFailed)
    );
    f.db(|c| {
        c.execute(
            "UPDATE provider_operations SET tested_at=?1",
            [now()? - TEST_TTL - 1],
        )
        .map_err(sql)?;
        Ok(())
    })
    .unwrap();
    assert_eq!(
        f.db(|c| {
            let tx = c.transaction().map_err(sql)?;
            snapshots::capture_in_transaction(&tx, Some(&a), None)
        })
        .err(),
        Some(ErrorCode::ProviderTestFailed)
    );
}
#[test]
fn restart_never_replays_and_requires_fresh_verification() {
    let f = Fixture::new();
    let a = f.seed("A");
    f.run(&a, "switch", Cancel::default());
    let id = f
        .db(|c| insert_job(c, &a, "switch", &crate::ipc::correlation_id()))
        .unwrap();
    f.db(|c| {
        c.execute(
            "UPDATE provider_operations SET phase='applying' WHERE id=?1",
            [&id],
        )
        .map_err(sql)?;
        recover(c)
    })
    .unwrap();
    let r = switch(f.db(|c| operation(c, &id)).unwrap());
    assert_eq!(r.outcome, "rollback_failed");
    assert!(r.active.is_none());
    let before = f.probes.get();
    f.db(recover).unwrap();
    assert_eq!(before, f.probes.get());
    assert_eq!(f.db(|c| repo::active(c, now()?)).unwrap().1, "error");
}
#[test]
fn strict_ipc_rejects_unknown_fields_types_and_versions() {
    let s = serde_json::json!({"profileId":"abc","expectedRevision":1,"modelId":"synthetic","effort":"high"});
    assert!(
        crate::ipc::parse_request::<Selection>(serde_json::json!({"apiVersion":1,"params":s}))
            .is_ok()
    );
    let mut bad = s;
    bad["secret"] = serde_json::json!("synthetic-must-not-echo");
    assert_eq!(
        crate::ipc::parse_request::<Selection>(serde_json::json!({"apiVersion":1,"params":bad}))
            .err(),
        Some(ErrorCode::InvalidRequest)
    );
}

#[test]
fn required_nullable_fields_cannot_be_omitted() {
    for params in [
        serde_json::json!({"profileId":"a","expectedRevision":1,"modelId":"synthetic"}),
        serde_json::json!({"profileId":"a","expectedRevision":"1","modelId":"synthetic","effort":null}),
    ] {
        assert_eq!(
            crate::ipc::parse_request::<Selection>(
                serde_json::json!({"apiVersion":1,"params":params})
            )
            .err(),
            Some(ErrorCode::InvalidRequest)
        );
    }
    let valid = serde_json::json!({"profileId":null,"expectedRevision":null,"name":"A","endpoint":"https://a.invalid/v1","defaultModel":null,"defaultEffort":null});
    for key in [
        "profileId",
        "expectedRevision",
        "defaultModel",
        "defaultEffort",
    ] {
        let mut params = valid.clone();
        params.as_object_mut().unwrap().remove(key);
        assert!(
            serde_json::from_value::<Save>(params).is_err(),
            "missing {key}"
        );
    }
}
#[test]
fn global_default_clear_recovers_blocked_state_without_reusing_evidence() {
    let f = Fixture::new();
    let a = f.seed("A");
    f.run(&a, "switch", Cancel::default());
    f.db(|c| block(c)).unwrap();
    let rev = f.db(|c| settings_revision(c)).unwrap();
    f.db.borrow_mut()
        .settings()
        .update_view(
            rev,
            &[crate::settings::Change::AiDefaults(None)],
            |_| Ok(()),
        )
        .unwrap();
    assert!(f.defaults().is_none());
    assert_eq!(
        f.db(|c| repo::active(c, now()?)).unwrap(),
        (None, "unconfigured".into())
    );
}
#[test]
fn unrelated_settings_update_preserves_applied_ai_default_and_next_restart() {
    let f = Fixture::new();
    let a = f.seed("A");
    f.run(&a, "switch", Cancel::default());
    let rev = f.db(|c| settings_revision(c)).unwrap();
    f.db.borrow_mut()
        .settings()
        .update_view(
            rev,
            &[crate::settings::Change::SearchEngine("google".into())],
            |_| Ok(()),
        )
        .unwrap();
    assert_eq!(f.defaults(), Some(a.clone()));
    f.db.borrow_mut()
        .settings()
        .initialize_defaults(&f.directory)
        .unwrap();
    assert_eq!(f.defaults(), Some(a));
    f.db(recover).unwrap();
    assert_eq!(f.db(|c| repo::active(c, now()?)).unwrap().1, "degraded");
}
#[test]
fn legacy_external_and_official_profiles_remain_visible_but_cannot_activate() {
    let f = Fixture::new();
    let a = f.seed("A");
    f.db(|c|{c.execute("UPDATE provider_profiles SET adapter_kind='cc_switch',external_profile_id='legacy' WHERE id=?1",[&a.profile_id]).map_err(sql)?;let r=repo::row(c,&a.profile_id)?;assert!(!r.profile.available);assert_eq!(r.profile.unavailable_code,Some("PA_PROVIDER_ADAPTER_UNAVAILABLE"));assert_eq!(repo::ensure_owned(&r).err(),Some(ErrorCode::ProviderAdapterUnavailable));c.execute("UPDATE provider_profiles SET adapter_kind='self_managed',external_profile_id=NULL,source_kind='official' WHERE id=?1",[&a.profile_id]).map_err(sql)?;assert_eq!(repo::validate_selection(c,&a,now()?).err(),Some(ErrorCode::ProviderAdapterUnavailable));Ok(())}).unwrap();
}
#[test]
fn audit_failure_at_apply_never_commits_new_defaults() {
    let f = Fixture::new();
    let a = f.seed("A");
    f.db(|c|{c.execute_batch("CREATE TEMP TRIGGER reject_switch BEFORE INSERT ON audit_events WHEN NEW.event_kind='config_switch' BEGIN SELECT RAISE(ABORT,'synthetic');END;").map_err(sql)}).unwrap();
    let r = switch(f.run(&a, "switch", Cancel::default()));
    assert_eq!(r.outcome, "rolled_back");
    assert_eq!(r.error.unwrap().code, ErrorCode::AuditWriteFailed);
    assert!(f.defaults().is_none());
}

#[test]
fn concurrent_global_setting_change_is_not_overwritten_by_switch() {
    let f = Fixture::new();
    let a = f.seed("A");
    f.change_settings.set(true);
    let r = switch(f.run(&a, "switch", Cancel::default()));
    assert_eq!(r.outcome, "unchanged");
    assert_eq!(r.error.unwrap().code, ErrorCode::RevisionConflict);
    assert!(f.defaults().is_none());
}
#[test]
fn rollback_audit_failure_leaves_durable_applying_block_instead_of_ready() {
    let f = Fixture::new();
    let a = f.seed("A");
    f.run(&a, "switch", Cancel::default());
    let b = f.seed("B");
    f.results
        .borrow_mut()
        .extend([Ok(()), Err(ErrorCode::ProviderTestFailed), Ok(())]);
    let id = f
        .db(|c| insert_job(c, &b, "switch", &crate::ipc::correlation_id()))
        .unwrap();
    let j = f.db(|c| read_job(c, &id)).unwrap();
    f.db(|c|c.execute_batch("CREATE TEMP TRIGGER reject_rollback BEFORE INSERT ON audit_events WHEN NEW.event_kind='config_rollback' BEGIN SELECT RAISE(ABORT,'synthetic');END;").map_err(sql)).unwrap();
    assert_eq!(
        execute(&f, &j, &Cancel::default()).err(),
        Some(ErrorCode::AuditWriteFailed)
    );
    assert_eq!(
        f.db(|c| repo::active(c, now()?)).unwrap(),
        (None, "error".into())
    );
    assert_eq!(f.db(|c| operation(c, &id)).unwrap().phase, "rolling_back");
}
#[test]
fn snapshot_requires_caller_transaction_and_freezes_resolved_default_effort() {
    let f = Fixture::new();
    let mut a = f.seed("A");
    a.effort = None;
    f.run(&a, "test", Cancel::default());
    assert_eq!(
        f.db(|c| snapshots::capture_in_transaction(c, Some(&a), None))
            .err(),
        Some(ErrorCode::InvalidRequest)
    );
    let s = f
        .db(|c| {
            let tx = c.transaction().map_err(sql)?;
            let s = snapshots::capture_in_transaction(&tx, Some(&a), None)?;
            tx.commit().map_err(sql)?;
            Ok(s)
        })
        .unwrap();
    assert_eq!(s.effort.as_deref(), Some("low"));
}
