use super::*;
use std::{cell::Cell, path::PathBuf, process::Command};

const FIRST: &str = "synthetic-credential-acceptance-FIRST-NO-EXPORT";
const SECOND: &str = "synthetic-credential-acceptance-SECOND-NO-EXPORT";

struct Fixture {
    id: String,
    directory: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        Self::named(&format!("acceptance-{}", crate::ipc::correlation_id()))
    }
    fn named(id: &str) -> Self {
        assert!(id.starts_with("acceptance-ipc-") && id.len() < 100);
        assert!(id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
        Self {
            id: id.to_owned(),
            directory: std::env::temp_dir().join(format!("pa-credentials-{id}")),
        }
    }
    fn open(&self) -> storage::Storage {
        storage::Storage::open(&self.directory).unwrap()
    }
    fn seed(&self, storage: &mut storage::Storage) {
        storage.connection.execute(
            "INSERT INTO provider_profiles (id,name,source_kind,adapter_kind,endpoint,endpoint_origin,wire_api,auth_kind,credential_ref,credential_revision,enabled,config_revision,created_at,updated_at,credential_status) VALUES (?1,'Synthetic credentials','relay','self_managed','https://fixture.invalid/v1','https://fixture.invalid','responses','api_key',?2,0,1,1,1000,1000,'missing')",
            rusqlite::params![self.id, logical_ref(&self.id, 0)],
        ).unwrap();
        storage.connection.execute("INSERT INTO model_capabilities VALUES (?1,'synthetic','',NULL,1,'connection_probe','fixture',1,1000,2000)", [&self.id]).unwrap();
    }
    fn mutate(
        &self,
        storage: &mut storage::Storage,
        revision: i64,
        secret: Option<&str>,
        store: &impl CredentialStore,
    ) -> Result<MutationResult, ErrorCode> {
        mutate_in_storage(
            storage,
            &self.id,
            revision,
            secret,
            &crate::ipc::correlation_id(),
            store,
        )
    }
    fn verify_secret(&self, storage: &mut storage::Storage, revision: i64, expected: &str) {
        let resolved = resolve_in_storage(
            storage,
            &self.id,
            revision,
            "https://FIXTURE.invalid:443/",
            &SystemStore,
        )
        .unwrap();
        assert!(
            resolved.as_bytes() == expected.as_bytes(),
            "synthetic secret mismatch"
        );
    }
    fn assert_no_secret_on_disk(&self) {
        fn scan(path: &std::path::Path) {
            for entry in std::fs::read_dir(path).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    scan(&entry.path());
                } else {
                    let bytes = std::fs::read(entry.path()).unwrap();
                    for secret in [FIRST, SECOND] {
                        assert!(
                            !bytes
                                .windows(secret.len())
                                .any(|window| window == secret.as_bytes()),
                            "secret found in ordinary storage"
                        );
                    }
                }
            }
        }
        scan(&self.directory);
    }
    fn cleanup(&self) {
        // Delete exact synthetic targets only; never enumerate the user's vault.
        for revision in 0..=8 {
            let binding = Binding {
                profile_id: self.id.clone(),
                origin: "https://fixture.invalid".into(),
                credential_revision: revision,
                config_revision: 1,
                credential_ref: Some(logical_ref(&self.id, revision)),
                status: "missing".into(),
            };
            platform::delete(&binding).unwrap();
            assert!(platform::read(&binding).unwrap().is_none());
        }
        let target = self.directory.canonicalize().ok();
        if let Some(target) = target {
            let root = std::env::temp_dir().canonicalize().unwrap();
            assert_eq!(target.parent(), Some(root.as_path()));
            assert_eq!(target.file_name(), self.directory.file_name());
            std::fs::remove_dir_all(target).unwrap();
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            self.cleanup();
        } else {
            // Best-effort exact synthetic target cleanup after an assertion failure.
            for revision in 0..=8 {
                let binding = Binding {
                    profile_id: self.id.clone(),
                    origin: "https://fixture.invalid".into(),
                    credential_revision: revision,
                    config_revision: 1,
                    credential_ref: Some(logical_ref(&self.id, revision)),
                    status: "missing".into(),
                };
                let _ = platform::delete(&binding);
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Action {
    Read,
    Write,
    Delete,
    Restore,
}
struct FaultStore {
    action: Action,
    occurrence: usize,
    calls: Cell<usize>,
}
impl FaultStore {
    fn new(action: Action, occurrence: usize) -> Self {
        Self {
            action,
            occurrence,
            calls: Cell::new(0),
        }
    }
    fn check(&self, action: Action) -> Result<(), platform::Failure> {
        if self.action == action {
            self.calls.set(self.calls.get() + 1);
            if self.calls.get() == self.occurrence {
                return Err(platform::Failure::Unavailable);
            }
        }
        Ok(())
    }
}
impl CredentialStore for FaultStore {
    fn read(&self, binding: &Binding) -> Result<Option<SecretBuffer>, platform::Failure> {
        self.check(Action::Read)?;
        SystemStore.read(binding)
    }
    fn write(&self, binding: &Binding, secret: &str) -> Result<(), platform::Failure> {
        self.check(Action::Write)?;
        SystemStore.write(binding, secret)
    }
    fn write_blob(&self, binding: &Binding, blob: SecretBuffer) -> Result<(), platform::Failure> {
        self.check(Action::Restore)?;
        SystemStore.write_blob(binding, blob)
    }
    fn delete(&self, binding: &Binding) -> Result<(), platform::Failure> {
        self.check(Action::Delete)?;
        SystemStore.delete(binding)
    }
}

fn metadata(storage: &mut storage::Storage, id: &str) -> storage::CredentialProfile {
    storage.profiles().credential_profile(id).unwrap().unwrap()
}
fn fail_audit(storage: &storage::Storage) {
    storage.connection.execute_batch("CREATE TEMP TRIGGER reject_audit BEFORE INSERT ON audit_events BEGIN SELECT RAISE(ABORT, 'synthetic-credential-acceptance-FIRST-NO-EXPORT'); END;").unwrap();
}
fn fail_commit(storage: &storage::Storage) {
    // A real SQLite COMMIT failure, after both the metadata and audit writes.
    storage.connection.execute_batch("CREATE TEMP TABLE commit_guard (profile TEXT REFERENCES provider_profiles(id)); CREATE TEMP TRIGGER reject_commit AFTER UPDATE ON provider_profiles BEGIN INSERT INTO commit_guard VALUES ('absent-profile'); END; PRAGMA defer_foreign_keys = ON;").unwrap();
}

#[test]
fn first_put_replace_remove_and_repair_missing_external_entry() {
    let _guard = OS_TEST_LOCK.lock().unwrap();
    let fixture = Fixture::new();
    let mut db = fixture.open();
    fixture.seed(&mut db);
    let put = fixture
        .mutate(&mut db, 1, Some(FIRST), &SystemStore)
        .unwrap();
    assert_eq!(put.credential.revision, 1);
    assert_eq!(put.config_revision, 2);
    fixture.verify_secret(&mut db, 1, FIRST);
    assert_eq!(db.capabilities().count_valid(&fixture.id, 1001).unwrap(), 0);
    let put = fixture
        .mutate(&mut db, 2, Some(SECOND), &SystemStore)
        .unwrap();
    fixture.verify_secret(&mut db, 2, SECOND);
    assert_eq!(put.config_revision, 3);
    let old = Binding {
        credential_revision: 1,
        ..read_binding(&mut db, &fixture.id).unwrap()
    };
    assert!(platform::read(&old).unwrap().is_none());
    platform::delete(&read_binding(&mut db, &fixture.id).unwrap()).unwrap();
    assert!(matches!(
        resolve_in_storage(
            &mut db,
            &fixture.id,
            2,
            "https://fixture.invalid",
            &SystemStore
        ),
        Err(ErrorCode::CredentialsUnavailable)
    ));
    fixture
        .mutate(&mut db, 3, Some(FIRST), &SystemStore)
        .unwrap();
    fixture.verify_secret(&mut db, 3, FIRST);
    let removed = fixture.mutate(&mut db, 4, None, &SystemStore).unwrap();
    assert_eq!(removed.credential.status, "missing");
    assert_eq!(removed.config_revision, 5);
    assert_eq!(
        fixture
            .mutate(&mut db, 5, None, &SystemStore)
            .unwrap()
            .config_revision,
        5
    );
    assert!(matches!(
        resolve_in_storage(
            &mut db,
            &fixture.id,
            4,
            "https://fixture.invalid",
            &SystemStore
        ),
        Err(ErrorCode::CredentialsUnavailable)
    ));
    fixture.assert_no_secret_on_disk();
}

#[test]
fn wrong_origin_revision_profile_and_invalid_payload_fail_closed() {
    let _guard = OS_TEST_LOCK.lock().unwrap();
    let fixture = Fixture::new();
    let mut db = fixture.open();
    fixture.seed(&mut db);
    fixture
        .mutate(&mut db, 1, Some(FIRST), &SystemStore)
        .unwrap();
    for (id, revision, origin) in [
        (&fixture.id[..], 1, "https://wrong.invalid"),
        (&fixture.id, 0, "https://fixture.invalid"),
        ("nonexistent", 1, "https://fixture.invalid"),
    ] {
        assert!(resolve_in_storage(&mut db, id, revision, origin, &SystemStore).is_err());
    }
    let blob = platform::read(&read_binding(&mut db, &fixture.id).unwrap())
        .unwrap()
        .unwrap();
    for changed in [
        Binding {
            origin: "https://wrong.invalid".into(),
            ..read_binding(&mut db, &fixture.id).unwrap()
        },
        Binding {
            credential_revision: 2,
            ..read_binding(&mut db, &fixture.id).unwrap()
        },
        Binding {
            profile_id: "wrong".into(),
            ..read_binding(&mut db, &fixture.id).unwrap()
        },
    ] {
        assert!(platform::secret(&blob, &changed).is_err());
    }
    let before = metadata(&mut db, &fixture.id);
    assert!(matches!(
        fixture.mutate(&mut db, 1, Some(SECOND), &SystemStore),
        Err(ErrorCode::CredentialRevisionConflict)
    ));
    for secret in ["", "bad\0secret", &"x".repeat(MAX_SECRET_BYTES + 1)] {
        assert!(matches!(
            fixture.mutate(&mut db, 2, Some(secret), &SystemStore),
            Err(ErrorCode::CredentialInvalid)
        ));
    }
    assert_eq!(metadata(&mut db, &fixture.id), before);
    fixture.verify_secret(&mut db, 1, FIRST);
}

#[test]
fn unavailable_read_write_or_delete_preserves_metadata_and_old_secret() {
    let _guard = OS_TEST_LOCK.lock().unwrap();
    for (action, occurrence) in [(Action::Read, 1), (Action::Write, 1), (Action::Delete, 1)] {
        for remove in [false, true] {
            let fixture = Fixture::new();
            let mut db = fixture.open();
            fixture.seed(&mut db);
            fixture
                .mutate(&mut db, 1, Some(FIRST), &SystemStore)
                .unwrap();
            let before = metadata(&mut db, &fixture.id);
            let store = FaultStore::new(action, occurrence);
            if remove && action == Action::Write {
                continue;
            }
            let result =
                fixture.mutate(&mut db, 2, if remove { None } else { Some(SECOND) }, &store);
            assert!(matches!(result, Err(ErrorCode::CredentialsUnavailable)));
            assert_eq!(metadata(&mut db, &fixture.id), before);
            fixture.verify_secret(&mut db, 1, FIRST);
            let next = next_binding(
                &read_binding(&mut db, &fixture.id).unwrap(),
                "configured_unverified",
            )
            .unwrap();
            assert!(platform::read(&next).unwrap().is_none());
        }
    }
}

#[test]
fn real_os_changes_are_compensated_on_metadata_audit_and_commit_failure() {
    let _guard = OS_TEST_LOCK.lock().unwrap();
    for failure in ["metadata", "audit", "commit"] {
        for operation in ["first", "replace", "remove"] {
            let fixture = Fixture::new();
            let mut db = fixture.open();
            fixture.seed(&mut db);
            let revision = if operation == "first" {
                1
            } else {
                fixture
                    .mutate(&mut db, 1, Some(FIRST), &SystemStore)
                    .unwrap();
                2
            };
            let before = metadata(&mut db, &fixture.id);
            match failure {
                "metadata" => db.connection.execute_batch("CREATE TEMP TRIGGER reject_metadata BEFORE UPDATE ON provider_profiles BEGIN SELECT RAISE(ABORT, 'synthetic-credential-acceptance-FIRST-NO-EXPORT'); END;").unwrap(),
                "audit" => fail_audit(&db),
                _ => fail_commit(&db),
            }
            let result = fixture.mutate(
                &mut db,
                revision,
                if operation == "remove" {
                    None
                } else {
                    Some(SECOND)
                },
                &SystemStore,
            );
            assert!(result.is_err(), "failure must reject mutation");
            assert_eq!(metadata(&mut db, &fixture.id), before);
            if operation != "first" {
                fixture.verify_secret(&mut db, 1, FIRST);
            }
            let next = next_binding(
                &read_binding(&mut db, &fixture.id).unwrap(),
                "configured_unverified",
            )
            .unwrap();
            assert!(platform::read(&next).unwrap().is_none());
            let audits: i64 = db
                .connection
                .query_row(
                    "SELECT count(*) FROM audit_events WHERE event_kind = 'credential_change'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(audits, if operation == "first" { 0 } else { 1 });
            fixture.assert_no_secret_on_disk();
        }
    }
}

#[test]
fn compensation_failure_persists_unavailable_state_across_reopen() {
    let _guard = OS_TEST_LOCK.lock().unwrap();
    for action in [Action::Restore, Action::Delete] {
        let fixture = Fixture::new();
        let mut db = fixture.open();
        fixture.seed(&mut db);
        fixture
            .mutate(&mut db, 1, Some(FIRST), &SystemStore)
            .unwrap();
        fail_audit(&db);
        let store = FaultStore::new(action, if action == Action::Delete { 2 } else { 1 });
        assert!(matches!(
            fixture.mutate(&mut db, 2, Some(SECOND), &store),
            Err(ErrorCode::CredentialsStorageFailed)
        ));
        assert_eq!(
            metadata(&mut db, &fixture.id).credential_status,
            "external_unavailable"
        );
        drop(db);
        let mut db = fixture.open();
        assert!(matches!(
            resolve_in_storage(
                &mut db,
                &fixture.id,
                1,
                "https://fixture.invalid",
                &SystemStore
            ),
            Err(ErrorCode::CredentialsUnavailable)
        ));
    }
}

#[test]
fn interrupted_next_target_and_malformed_os_payload_are_never_overwritten() {
    let _guard = OS_TEST_LOCK.lock().unwrap();
    let fixture = Fixture::new();
    let mut db = fixture.open();
    fixture.seed(&mut db);
    fixture
        .mutate(&mut db, 1, Some(FIRST), &SystemStore)
        .unwrap();
    let current = read_binding(&mut db, &fixture.id).unwrap();
    let next = next_binding(&current, "configured_unverified").unwrap();
    platform::write(&next, SECOND).unwrap();
    assert!(matches!(
        fixture.mutate(&mut db, 2, Some(FIRST), &SystemStore),
        Err(ErrorCode::CredentialInvalid)
    ));
    assert!(
        platform::secret(&platform::read(&next).unwrap().unwrap(), &next)
            .unwrap()
            .as_bytes()
            == SECOND.as_bytes()
    );
    platform::delete(&next).unwrap();
    platform::write_blob(
        &current,
        SecretBuffer::new(b"malformed-synthetic-payload".to_vec()),
    )
    .unwrap();
    assert!(
        resolve_in_storage(
            &mut db,
            &fixture.id,
            1,
            "https://fixture.invalid",
            &SystemStore
        )
        .is_err()
    );
    assert!(matches!(
        fixture.mutate(&mut db, 2, Some(SECOND), &SystemStore),
        Err(ErrorCode::CredentialInvalid)
    ));
    assert_eq!(metadata(&mut db, &fixture.id).config_revision, 2);
}

#[test]
fn occupied_task_and_stale_revision_reject_before_os_access() {
    let _guard = OS_TEST_LOCK.lock().unwrap();
    let fixture = Fixture::new();
    let mut db = fixture.open();
    storage::probe::seed(&mut db.connection).unwrap();
    db.connection
        .execute("UPDATE tasks SET status='running',finished_at=NULL", [])
        .unwrap();
    let store = FaultStore::new(Action::Read, 1);
    assert!(matches!(
        mutate_in_storage(
            &mut db,
            "provider1",
            1,
            Some(FIRST),
            &crate::ipc::correlation_id(),
            &store
        ),
        Err(ErrorCode::CredentialBusy)
    ));
    assert_eq!(store.calls.get(), 0);
    assert!(matches!(
        mutate_in_storage(
            &mut db,
            "provider1",
            2,
            Some(FIRST),
            &crate::ipc::correlation_id(),
            &store
        ),
        Err(ErrorCode::CredentialRevisionConflict)
    ));
    assert_eq!(store.calls.get(), 0);
}

#[test]
fn profile_ids_differing_only_in_case_do_not_share_os_targets() {
    let _guard = OS_TEST_LOCK.lock().unwrap();
    let first = Fixture::new();
    // Use a case difference in the suffix while retaining the cleanup namespace.
    let upper_id = format!("{}-Case", first.id);
    let lower_id = format!("{}-case", first.id);
    let upper = Fixture::named(&upper_id);
    let lower = Fixture::named(&lower_id);
    let mut a = upper.open();
    upper.seed(&mut a);
    let mut b = lower.open();
    lower.seed(&mut b);
    upper.mutate(&mut a, 1, Some(FIRST), &SystemStore).unwrap();
    lower.mutate(&mut b, 1, Some(SECOND), &SystemStore).unwrap();
    upper.verify_secret(&mut a, 1, FIRST);
    lower.verify_secret(&mut b, 1, SECOND);
    drop(a);
    drop(b);
    upper.cleanup();
    lower.cleanup();
}

#[test]
fn independent_process_write_read_replace_remove_and_cleanup() {
    let _guard = OS_TEST_LOCK.lock().unwrap();
    let fixture = Fixture::new();
    for mode in [
        "seed",
        "read-first",
        "replace",
        "read-second",
        "remove",
        "read-missing",
        "cleanup",
    ] {
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "credentials::acceptance_tests::independent_process_worker",
                "--ignored",
                "--nocapture",
            ])
            .env("PA_CREDENTIAL_TEST_ID", &fixture.id)
            .env("PA_CREDENTIAL_TEST_MODE", mode)
            .output()
            .unwrap();
        // Worker output includes only test names and fixed errors; never secrets.
        assert!(
            child.status.success(),
            "independent credential worker failed: {mode}; {}",
            String::from_utf8_lossy(&child.stderr)
        );
        for secret in [FIRST, SECOND] {
            assert!(
                !child
                    .stdout
                    .windows(secret.len())
                    .any(|value| value == secret.as_bytes())
            );
            assert!(
                !child
                    .stderr
                    .windows(secret.len())
                    .any(|value| value == secret.as_bytes())
            );
        }
    }
    assert!(!fixture.directory.exists());
}

#[test]
#[ignore = "Invoked only by the parent independent-process acceptance test"]
fn independent_process_worker() {
    let id = std::env::var("PA_CREDENTIAL_TEST_ID").unwrap();
    let fixture = std::mem::ManuallyDrop::new(Fixture::named(&id));
    let mode = std::env::var("PA_CREDENTIAL_TEST_MODE").unwrap();
    if mode == "cleanup" {
        fixture.cleanup();
        return;
    }
    let mut db = fixture.open();
    match mode.as_str() {
        "seed" => {
            fixture.seed(&mut db);
            fixture
                .mutate(&mut db, 1, Some(FIRST), &SystemStore)
                .unwrap();
        }
        "read-first" => fixture.verify_secret(&mut db, 1, FIRST),
        "replace" => {
            fixture
                .mutate(&mut db, 2, Some(SECOND), &SystemStore)
                .unwrap();
        }
        "read-second" => fixture.verify_secret(&mut db, 2, SECOND),
        "remove" => {
            fixture.mutate(&mut db, 3, None, &SystemStore).unwrap();
        }
        "read-missing" => {
            assert_eq!(metadata(&mut db, &fixture.id).credential_status, "missing");
            assert!(
                resolve_in_storage(
                    &mut db,
                    &fixture.id,
                    3,
                    "https://fixture.invalid",
                    &SystemStore
                )
                .is_err()
            );
        }
        _ => panic!("unknown synthetic mode"),
    }
    let backup_path = fixture
        .directory
        .join(format!("synthetic-backup-{mode}.sqlite3"));
    let mut backup = rusqlite::Connection::open(backup_path).unwrap();
    rusqlite::backup::Backup::new(&db.connection, &mut backup)
        .unwrap()
        .run_to_completion(64, Duration::from_millis(1), None)
        .unwrap();
    fixture.assert_no_secret_on_disk();
}
