use super::*;
use crate::{
    ipc::correlation_id,
    project_paths::authorize_fixture,
    storage::{DATABASE_NAME, MIGRATIONS, Storage},
};
use std::{fs, path::PathBuf, process::Command};

pub(super) struct Fixture {
    pub(super) root: PathBuf,
}
impl Fixture {
    pub(super) fn new() -> Self {
        let root = std::env::temp_dir().join(format!("pa-s2-03-{}", correlation_id()));
        fs::create_dir_all(root.join("source 中文 & 空格")).unwrap();
        fs::write(root.join("source 中文 & 空格/keep.txt"), "source-sentinel").unwrap();
        Self { root }
    }
    pub(super) fn source(&self) -> PathBuf {
        self.root.join("source 中文 & 空格")
    }
    pub(super) fn db(&self) -> Storage {
        Storage::open(&self.root.join("data")).unwrap()
    }
    pub(super) fn register(&self, db: &mut Storage) -> ProjectView {
        db.projects()
            .register(
                Some(authorize_fixture(&self.source()).unwrap()),
                &correlation_id(),
            )
            .unwrap()
            .unwrap()
    }
    pub(super) fn unchanged(&self) {
        assert_eq!(
            fs::read(self.source().join("keep.txt")).unwrap(),
            b"source-sentinel"
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
pub(super) fn count(db: &Storage, table: &str) -> i64 {
    db.connection
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

#[test]
fn cancellation_dedup_detail_and_reads_do_not_count_as_usage() {
    let f = Fixture::new();
    let mut db = f.db();
    assert!(
        db.projects()
            .register(None, &correlation_id())
            .unwrap()
            .is_none()
    );
    assert_eq!(count(&db, "projects"), 0);
    assert_eq!(count(&db, "audit_events"), 0);
    let first = f.register(&mut db);
    let alias = PathBuf::from(f.source().to_string_lossy().to_uppercase());
    let again = db
        .projects()
        .register(Some(authorize_fixture(&alias).unwrap()), &correlation_id())
        .unwrap()
        .unwrap();
    assert_eq!(first, again);
    assert_eq!(count(&db, "audit_events"), 1);
    let read = db.projects().get(&first.id, &correlation_id()).unwrap();
    assert_eq!(read.last_used_at, None);
    assert_eq!(count(&db, "project_usage"), 0);
    assert_eq!(read.authorization, Authorization::Granted);
    f.unchanged();
}

#[test]
fn usage_requires_valid_identity_revision_and_successful_acceptance() {
    let f = Fixture::new();
    let mut db = f.db();
    let p = f.register(&mut db);
    let failed = db
        .projects()
        .record_usage(&p.id, p.revision, UsageSource::SystemOpen, |_| {
            Err::<(), _>(ErrorCode::ProjectOpenRejected)
        });
    assert_eq!(failed, Err(ErrorCode::ProjectOpenRejected));
    assert_eq!(count(&db, "project_usage"), 0);
    assert_eq!(
        db.projects()
            .record_usage(&p.id, p.revision + 1, UsageSource::Selection, |_| Ok(())),
        Err(ErrorCode::ProjectConflictRevision)
    );
    db.projects()
        .record_usage(&p.id, p.revision, UsageSource::Selection, |v| {
            assert_eq!(v.name, p.name);
            Ok(())
        })
        .unwrap();
    db.projects()
        .record_usage(&p.id, p.revision, UsageSource::SystemOpen, |_| Ok(()))
        .unwrap();
    let (n, source): (i64, String) = db
        .connection
        .query_row(
            "SELECT open_count,last_source FROM project_usage",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(n, 2);
    assert_eq!(source, "system_open");
    let view = db.projects().get(&p.id, &correlation_id()).unwrap();
    assert!(view.last_used_at.is_some());
    assert_eq!(view.revision, p.revision);
    f.unchanged();
}

#[test]
fn grant_remove_and_usage_sql_audit_failures_roll_back() {
    let f = Fixture::new();
    let mut db = f.db();
    db.connection.execute_batch("CREATE TRIGGER reject_audit BEFORE INSERT ON audit_events BEGIN SELECT RAISE(ABORT,'private-path-secret'); END;").unwrap();
    assert_eq!(
        db.projects()
            .register(
                Some(authorize_fixture(&f.source()).unwrap()),
                &correlation_id()
            )
            .err(),
        Some(ErrorCode::AuditWriteFailed)
    );
    assert_eq!(count(&db, "projects"), 0);
    db.connection
        .execute_batch("DROP TRIGGER reject_audit")
        .unwrap();
    let p = f.register(&mut db);
    db.projects()
        .record_usage(&p.id, p.revision, UsageSource::Selection, |_| Ok(()))
        .unwrap();
    db.connection.execute_batch("CREATE TRIGGER reject_audit BEFORE INSERT ON audit_events BEGIN SELECT RAISE(ABORT,'private-path-secret'); END;").unwrap();
    assert_eq!(
        db.projects()
            .remove(&p.id, p.revision, &correlation_id())
            .err(),
        Some(ErrorCode::AuditWriteFailed)
    );
    assert_eq!(
        record(&db.connection, &p.id).unwrap().view.authorization,
        Authorization::Granted
    );
    assert_eq!(count(&db, "project_usage"), 1);
    assert_eq!(count(&db, "audit_events"), 1);
    db.connection.execute_batch("DROP TRIGGER reject_audit; CREATE TRIGGER reject_usage BEFORE UPDATE ON project_usage BEGIN SELECT RAISE(ABORT,'private-path-secret'); END;").unwrap();
    assert_eq!(
        db.projects()
            .record_usage(&p.id, p.revision, UsageSource::Selection, |_| Ok(())),
        Err(ErrorCode::SettingsStorageFailed)
    );
    let n: i64 = db
        .connection
        .query_row("SELECT open_count FROM project_usage", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
    db.connection.execute_batch("DROP TRIGGER reject_usage; CREATE TRIGGER reject_project BEFORE UPDATE ON projects BEGIN SELECT RAISE(ABORT,'private-path-secret'); END;").unwrap();
    assert_eq!(
        db.projects()
            .remove(&p.id, p.revision, &correlation_id())
            .err(),
        Some(ErrorCode::SettingsStorageFailed)
    );
    assert_eq!(count(&db, "audit_events"), 1);
    f.unchanged();
}

#[test]
fn history_is_retained_revocation_hides_project_and_does_not_delete_source() {
    let f = Fixture::new();
    let mut db = f.db();
    let p = f.register(&mut db);
    db.connection.execute("INSERT INTO sessions(id,project_id,status,created_at,updated_at) VALUES('history',?1,'closed',1,1)",[&p.id]).unwrap();
    db.connection.execute_batch("INSERT INTO tasks(id,session_id,status,created_at,finished_at) VALUES('history-task','history','completed',1,2);").unwrap_err();
    db.connection.execute_batch("INSERT INTO tasks(id,session_id,status,created_at,finished_at) VALUES('history-task','history','cancelled',1,2);").unwrap();
    db.projects()
        .record_usage(&p.id, p.revision, UsageSource::Selection, |_| Ok(()))
        .unwrap();
    assert_eq!(
        db.projects()
            .remove(&p.id, p.revision + 1, &correlation_id())
            .err(),
        Some(ErrorCode::ProjectConflictRevision)
    );
    let removed = db
        .projects()
        .remove(&p.id, p.revision, &correlation_id())
        .unwrap();
    assert!(removed.retained_for_history);
    assert_eq!(count(&db, "sessions"), 1);
    assert_eq!(count(&db, "tasks"), 1);
    assert_eq!(count(&db, "project_usage"), 0);
    assert_eq!(
        db.projects().get(&p.id, &correlation_id()).err(),
        Some(ErrorCode::ProjectNotFound)
    );
    assert_eq!(
        db.projects()
            .record_usage(&p.id, p.revision, UsageSource::Selection, |_| Ok(())),
        Err(ErrorCode::ProjectNotFound)
    );
    let again = f.register(&mut db);
    assert_eq!(again.id, p.id);
    assert_eq!(again.revision, p.revision + 2);
    assert_eq!(again.last_used_at, None);
    f.unchanged();
}

#[test]
fn missing_replacement_restart_and_explicit_reauthorization() {
    let f = Fixture::new();
    let mut db = f.db();
    let p = f.register(&mut db);
    fs::rename(f.source(), f.root.join("old-source")).unwrap();
    db.projects().recover(&correlation_id()).unwrap();
    let missing = db.projects().get(&p.id, &correlation_id()).unwrap();
    assert_eq!(missing.availability, Availability::Missing);
    assert_eq!(missing.authorization, Authorization::Granted);
    assert_eq!(
        db.projects()
            .record_usage(&p.id, missing.revision, UsageSource::Selection, |_| Ok(())),
        Err(ErrorCode::ProjectPathUnavailable)
    );
    fs::create_dir(f.source()).unwrap();
    fs::write(f.source().join("keep.txt"), "source-sentinel").unwrap();
    // A picker alone cannot silently overwrite a grant to the old identity.
    assert_eq!(
        db.projects()
            .register(
                Some(authorize_fixture(&f.source()).unwrap()),
                &correlation_id()
            )
            .err(),
        Some(ErrorCode::ProjectPathChanged)
    );
    drop(db);
    let mut db = f.db();
    db.projects().recover(&correlation_id()).unwrap();
    let changed = db.projects().get(&p.id, &correlation_id()).unwrap();
    assert_eq!(changed.availability, Availability::IdentityChanged);
    assert_eq!(
        changed.authorization,
        Authorization::RequiresReauthorization
    );
    let reauthorized = f.register(&mut db);
    assert_eq!(reauthorized.id, p.id);
    assert!(reauthorized.revision > changed.revision);
    f.unchanged();
}

#[test]
fn replacement_between_picker_and_commit_cannot_create_a_grant() {
    let f = Fixture::new();
    let mut db = f.db();
    let candidate = authorize_fixture(&f.source()).unwrap();
    fs::rename(f.source(), f.root.join("old-source")).unwrap();
    fs::create_dir(f.source()).unwrap();
    assert_eq!(
        db.projects()
            .register(Some(candidate), &correlation_id())
            .err(),
        Some(ErrorCode::ProjectPathChanged)
    );
    assert_eq!(count(&db, "projects"), 0);
    assert_eq!(count(&db, "audit_events"), 0);
}

#[test]
fn v4_upgrade_preserves_history_and_backup_without_granting_legacy_paths() {
    let f = Fixture::new();
    let dir = f.root.join("data");
    let mut old = Storage::open_with(&dir, &MIGRATIONS[..4]).unwrap();
    crate::storage::probe::seed(&mut old.connection).unwrap();
    drop(old);
    let mut db = f.db();
    assert_eq!(count(&db, "tasks"), 1);
    assert_eq!(count(&db, "audit_events"), 1);
    let auth: String = db
        .connection
        .query_row("SELECT authorization FROM projects", [], |r| r.get(0))
        .unwrap();
    assert_eq!(auth, "requires_reauthorization");
    // Startup tolerates historical IDs that predate the UUID service.
    db.projects().recover(&correlation_id()).unwrap();
    let backup = fs::read_dir(dir.join("backups"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let c = Connection::open(backup).unwrap();
    let version: i64 = c
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 4);
    assert!(dir.join(DATABASE_NAME).exists());
    f.unchanged();
}

#[test]
fn competing_connections_deduplicate_and_reject_stale_remove_after_revocation() {
    let f = Fixture::new();
    let mut first = f.db();
    let mut second = f.db();
    let a = f.register(&mut first);
    let b = f.register(&mut second);
    assert_eq!(a.id, b.id);
    first
        .projects()
        .remove(&a.id, a.revision, &correlation_id())
        .unwrap();
    assert_eq!(
        second
            .projects()
            .record_usage(&a.id, b.revision, UsageSource::Selection, |_| Ok(())),
        Err(ErrorCode::ProjectNotFound)
    );
    let new = f.register(&mut first);
    assert_eq!(
        second
            .projects()
            .remove(&a.id, b.revision, &correlation_id())
            .err(),
        Some(ErrorCode::ProjectConflictRevision)
    );
    assert!(new.revision > b.revision);
    assert_eq!(count(&second, "projects"), 1);
    f.unchanged();
}

#[test]
fn audit_contains_only_fixed_facts_and_server_ids() {
    let f = Fixture::new();
    let mut db = f.db();
    let p = f.register(&mut db);
    let json:String=db.connection.query_row("SELECT json_object('kind',event_kind,'project',project_id,'correlation',correlation_id,'outcome',outcome) FROM audit_events",[],|r|r.get(0)).unwrap();
    assert!(json.contains(&p.id));
    assert!(!json.contains("source"));
    assert!(!json.contains("keep.txt"));
    assert!(!json.contains(&f.source().to_string_lossy().to_string()));
    let log = crate::ipc::command_log(
        "project_get",
        &correlation_id(),
        Some(ErrorCode::ProjectPathChanged),
    );
    assert!(!log.contains(&p.id));
    assert!(!log.contains("source"));
}

#[test]
#[ignore = "invoked by cross_process_persistence parent"]
fn persistence_worker() {
    let root = PathBuf::from(std::env::var_os("PA_PROJECT_TEST_ROOT").unwrap());
    let f = Fixture { root };
    let mut db = f.db();
    match std::env::var("PA_PROJECT_TEST_ACTION").unwrap().as_str() {
        "seed" => {
            let p = f.register(&mut db);
            db.projects()
                .record_usage(&p.id, p.revision, UsageSource::Selection, |_| Ok(()))
                .unwrap();
            fs::write(f.root.join("id"), p.id).unwrap();
        }
        "verify" => {
            db.projects().recover(&correlation_id()).unwrap();
            let id = fs::read_to_string(f.root.join("id")).unwrap();
            let p = db.projects().get(&id, &correlation_id()).unwrap();
            assert_eq!(p.authorization, Authorization::Granted);
            assert_eq!(p.availability, Availability::Available);
            assert!(p.last_used_at.is_some());
            let r = db
                .projects()
                .remove(&id, p.revision, &correlation_id())
                .unwrap();
            assert!(r.retained_for_history);
            f.unchanged();
        }
        "removed" => {
            db.projects().recover(&correlation_id()).unwrap();
            let id = fs::read_to_string(f.root.join("id")).unwrap();
            assert_eq!(
                db.projects().get(&id, &correlation_id()).err(),
                Some(ErrorCode::ProjectNotFound)
            );
            assert_eq!(count(&db, "project_usage"), 0);
            f.unchanged();
        }
        _ => panic!("unknown fixture action"),
    }
    drop(db);
    std::mem::forget(f); // Parent owns cleanup after all child processes.
}

#[test]
fn cross_process_persistence() {
    let f = Fixture::new();
    for action in ["seed", "verify", "removed"] {
        let result = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "storage::projects::tests::persistence_worker",
            ])
            .env("PA_PROJECT_TEST_ROOT", &f.root)
            .env("PA_PROJECT_TEST_ACTION", action)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "project persistence worker failed: {}",
            String::from_utf8_lossy(&result.stdout)
        );
    }
    f.unchanged();
}

#[test]
fn final_commit_failure_rolls_back_grant_revoke_and_usage() {
    let f = Fixture::new();
    let mut db = f.db();
    db.connection.execute_batch("CREATE TEMP TABLE commit_guard(project TEXT REFERENCES projects(id) DEFERRABLE INITIALLY DEFERRED); CREATE TEMP TRIGGER fail_grant_commit AFTER INSERT ON projects BEGIN INSERT INTO commit_guard VALUES('absent-project'); END;").unwrap();
    assert_eq!(
        db.projects()
            .register(
                Some(authorize_fixture(&f.source()).unwrap()),
                &correlation_id()
            )
            .err(),
        Some(ErrorCode::SettingsStorageFailed)
    );
    assert_eq!(count(&db, "projects"), 0);
    assert_eq!(count(&db, "audit_events"), 0);
    db.connection
        .execute_batch("DROP TRIGGER fail_grant_commit")
        .unwrap();
    let p = f.register(&mut db);
    db.connection.execute_batch("CREATE TEMP TRIGGER fail_usage_commit AFTER INSERT ON project_usage BEGIN INSERT INTO commit_guard VALUES('absent-project'); END;").unwrap();
    assert_eq!(
        db.projects()
            .record_usage(&p.id, p.revision, UsageSource::Selection, |_| Ok(())),
        Err(ErrorCode::SettingsStorageFailed)
    );
    assert_eq!(count(&db, "project_usage"), 0);
    db.connection.execute_batch("DROP TRIGGER fail_usage_commit; CREATE TEMP TRIGGER fail_revoke_commit AFTER UPDATE ON projects BEGIN INSERT INTO commit_guard VALUES('absent-project'); END;").unwrap();
    assert_eq!(
        db.projects()
            .remove(&p.id, p.revision, &correlation_id())
            .err(),
        Some(ErrorCode::SettingsStorageFailed)
    );
    assert_eq!(
        record(&db.connection, &p.id).unwrap().view.authorization,
        Authorization::Granted
    );
    assert_eq!(count(&db, "audit_events"), 1);
    f.unchanged();
}

#[test]
fn simultaneous_registrations_share_one_id_and_usage_lock_blocks_removal() {
    let f = Fixture::new();
    drop(f.db());
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let root = f.root.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut db = Storage::open(&root.join("data")).unwrap();
                let selected = authorize_fixture(&root.join("source 中文 & 空格")).unwrap();
                barrier.wait();
                db.projects()
                    .register(Some(selected), &correlation_id())
                    .unwrap()
                    .unwrap()
                    .id
            })
        })
        .collect();
    let ids: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(ids[0], ids[1]);
    let mut db = f.db();
    let mut other = f.db();
    let p = db.projects().get(&ids[0], &correlation_id()).unwrap();
    other
        .connection
        .busy_timeout(std::time::Duration::ZERO)
        .unwrap();
    db.projects()
        .record_usage(&p.id, p.revision, UsageSource::Selection, |_| {
            assert_eq!(
                other
                    .projects()
                    .remove(&p.id, p.revision, &correlation_id())
                    .err(),
                Some(ErrorCode::SettingsStorageFailed)
            );
            Ok(())
        })
        .unwrap();
    other
        .projects()
        .remove(&p.id, p.revision, &correlation_id())
        .unwrap();
    f.unchanged();
}

#[test]
fn restart_audit_failure_rolls_back_invalidations_and_preserves_grant() {
    let f = Fixture::new();
    let mut db = f.db();
    let p = f.register(&mut db);
    fs::rename(f.source(), f.root.join("moved")).unwrap();
    db.connection.execute_batch("CREATE TEMP TRIGGER fail_recovery BEFORE INSERT ON audit_events BEGIN SELECT RAISE(ABORT,'private'); END;").unwrap();
    assert_eq!(
        db.projects().recover(&correlation_id()),
        Err(ErrorCode::AuditWriteFailed)
    );
    let old = record(&db.connection, &p.id).unwrap();
    assert_eq!(old.view.revision, p.revision);
    assert_eq!(old.view.availability, Availability::Available);
    // Even stale cached availability cannot authorize use when persistence fails.
    assert_eq!(
        db.projects()
            .record_usage(&p.id, p.revision, UsageSource::Selection, |_| Ok(())),
        Err(ErrorCode::ProjectPathUnavailable)
    );
}

#[test]
fn legacy_remove_keeps_the_new_revocation_audit_reference() {
    let f = Fixture::new();
    let mut db = f.db();
    let id = crate::providers::uuid().unwrap();
    db.connection.execute("INSERT INTO projects(id,canonical_path,path_key,name,created_at,updated_at) VALUES(?1,?2,'legacy-key','Legacy',1,1)",params![id,f.source().to_string_lossy()]).unwrap();
    let result = db.projects().remove(&id, 1, &correlation_id()).unwrap();
    assert!(result.retained_for_history);
    assert_eq!(count(&db, "projects"), 1);
    let project: Option<String> = db
        .connection
        .query_row("SELECT project_id FROM audit_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(project, Some(id));
    f.unchanged();
}
