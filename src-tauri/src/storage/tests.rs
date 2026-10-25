use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "phase-0-sqlite-中文 空格-{}-{}-{}",
            std::process::id(),
            now_ms(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        )))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn fixture() -> (Fixture, Storage) {
    let directory = Fixture::new();
    let mut storage = Storage::open(&directory.0).unwrap();
    probe::seed(&mut storage.connection).unwrap();
    (directory, storage)
}
fn upgrade(sql: &'static str) -> Vec<Migration> {
    let mut migrations: Vec<_> = MIGRATIONS
        .iter()
        .map(|m| Migration {
            version: m.version,
            name: m.name,
            sql: m.sql,
        })
        .collect();
    migrations.push(Migration {
        version: SCHEMA_VERSION + 1,
        name: "test_only",
        sql,
    });
    migrations
}

#[test]
fn fresh_database_constraints_and_reopen() {
    let (directory, mut storage) = fixture();
    assert_eq!(probe::verify(&mut storage.connection).unwrap().len(), 9);
    drop(storage);
    let mut reopened = Storage::open(&directory.0).unwrap();
    probe::verify(&mut reopened.connection).unwrap();
    assert_eq!(
        reopened
            .connection
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        SCHEMA_VERSION
    );
    assert!(!directory.0.join("backups").exists());
}

#[test]
fn successful_upgrade_backs_up_committed_wal() {
    let (directory, storage) = fixture();
    // Keep this writer open: its committed data still lives in WAL.
    let upgraded = Storage::open_with(
        &directory.0,
        &upgrade("ALTER TABLE projects ADD COLUMN fixture_column TEXT;"),
    )
    .unwrap();
    assert_eq!(
        upgraded
            .connection
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        SCHEMA_VERSION + 1
    );
    let backup_path = fs::read_dir(directory.0.join("backups"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let backup =
        Connection::open_with_flags(backup_path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert_eq!(
        backup
            .query_row("SELECT count(*) FROM projects", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        backup
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        SCHEMA_VERSION
    );
    assert!(
        backup
            .prepare("SELECT fixture_column FROM projects")
            .is_err()
    );
    drop(storage);
}

#[test]
fn failed_upgrade_rolls_back_ddl_data_history_and_allows_retry() {
    let (directory, storage) = fixture();
    let failed = Storage::open_with(
        &directory.0,
        &upgrade(
            "CREATE TABLE should_rollback (id INTEGER); DELETE FROM project_usage; SELECT * FROM missing_fixture;",
        ),
    );
    let error = failed.err().unwrap();
    assert_eq!(error.to_string(), "STORAGE_MIGRATION_FAILED");
    let backup =
        Connection::open_with_flags(error.backup_path.unwrap(), OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    assert_eq!(
        backup
            .query_row("SELECT count(*) FROM project_usage", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(storage);
    let mut original = Storage::open(&directory.0).unwrap();
    probe::verify(&mut original.connection).unwrap();
    assert!(
        original
            .connection
            .prepare("SELECT * FROM should_rollback")
            .is_err()
    );
    assert_eq!(
        original
            .connection
            .query_row("SELECT count(*) FROM schema_migrations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        SCHEMA_VERSION
    );
    drop(original);
    Storage::open_with(&directory.0, &upgrade("CREATE TABLE retried (id INTEGER);")).unwrap();
}

#[test]
fn backup_failure_prevents_any_migration() {
    let (directory, storage) = fixture();
    fs::write(directory.0.join("backups"), "fixture blocking directory").unwrap();
    let failed = Storage::open_with(&directory.0, &upgrade("DELETE FROM project_usage;"));
    assert_eq!(failed.err().unwrap().code, "STORAGE_BACKUP_FAILED");
    assert_eq!(
        storage
            .connection
            .query_row("SELECT count(*) FROM project_usage", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn newer_schema_is_rejected_without_downgrade() {
    let (directory, storage) = fixture();
    storage
        .connection
        .pragma_update(None, "user_version", 99)
        .unwrap();
    assert_eq!(
        Storage::open(&directory.0).err().unwrap().code,
        "STORAGE_NEWER_SCHEMA"
    );
    assert_eq!(
        storage
            .connection
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        99
    );
    assert!(!directory.0.join("backups").exists());
}

#[test]
fn changed_migration_checksum_is_rejected() {
    let (directory, storage) = fixture();
    storage
        .connection
        .execute("UPDATE schema_migrations SET checksum = 'tampered'", [])
        .unwrap();
    assert_eq!(
        Storage::open(&directory.0).err().unwrap().code,
        "STORAGE_MIGRATION_HISTORY"
    );
    assert_eq!(
        storage
            .connection
            .query_row("SELECT count(*) FROM projects", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn corrupt_database_is_not_replaced() {
    let directory = Fixture::new();
    fs::create_dir_all(&directory.0).unwrap();
    let path = directory.0.join(DATABASE_NAME);
    fs::write(&path, "not a database fixture").unwrap();
    let before = fs::read(&path).unwrap();
    let failure = Storage::open(&directory.0).err().unwrap();
    assert_eq!(failure.public_code(), "PA_STORAGE_CORRUPT");
    let runtime = StorageState::default();
    assert_eq!(
        runtime
            .initialize(&directory.0, &crate::ipc::correlation_id())
            .unwrap_err()
            .public_code(),
        "PA_STORAGE_CORRUPT"
    );
    assert_eq!(runtime.health().reason_code, Some("PA_STORAGE_CORRUPT"));
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn query_only_migration_is_not_reported_as_busy() {
    let (directory, storage) = fixture();
    let mut readonly =
        Connection::open_with_flags(&storage.path, OpenFlags::SQLITE_OPEN_READ_WRITE).unwrap();
    readonly.pragma_update(None, "query_only", true).unwrap();
    let failure = Storage::migrate(&mut readonly, &directory.0, true, MIGRATIONS).unwrap_err();
    assert_eq!(failure.public_code(), "PA_STORAGE_SQL_FAILED");
}

#[test]
fn migration_write_lock_conflict_is_reported_as_busy_without_changes() {
    let (directory, mut storage) = fixture();
    let mut contender = Connection::open(&storage.path).unwrap();
    contender.busy_timeout(Duration::ZERO).unwrap();
    let transaction = storage
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let failure = Storage::migrate(&mut contender, &directory.0, true, MIGRATIONS).unwrap_err();
    assert_eq!(failure.public_code(), "PA_STORAGE_BUSY");
    assert!(!directory.0.join("backups").exists());
    drop(transaction);
    probe::verify(&mut contender).unwrap();
}

#[test]
fn unrecognized_unversioned_database_is_preserved() {
    let directory = Fixture::new();
    fs::create_dir_all(&directory.0).unwrap();
    let connection = Connection::open(directory.0.join(DATABASE_NAME)).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE unrelated (value TEXT); INSERT INTO unrelated VALUES ('fixture');",
        )
        .unwrap();
    assert_eq!(
        Storage::open(&directory.0).err().unwrap().code,
        "STORAGE_UNRECOGNIZED_SCHEMA"
    );
    assert_eq!(
        connection
            .query_row("SELECT value FROM unrelated", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "fixture"
    );
}

#[test]
fn defaults_uniqueness_and_setting_allowlist() {
    let (_directory, storage) = fixture();
    assert!(storage.connection.execute("INSERT INTO settings SELECT 'duplicate', scope, project_id, key, value_text, value_integer, profile_id, model_id, effort, revision, updated_at FROM settings WHERE id = 'global-ai'", []).is_err());
    assert!(storage.connection.execute("INSERT INTO settings VALUES ('bad', 'global', NULL, 'api_key', 'synthetic-secret', NULL, NULL, NULL, NULL, 1, 1000)", []).is_err());
    assert!(storage.connection.execute("UPDATE provider_profiles SET endpoint = 'https://user:synthetic-secret@fixture.invalid'", []).is_err());
    assert!(storage.connection.execute("UPDATE provider_profiles SET endpoint = 'https://fixture.invalid?api_key=synthetic-secret'", []).is_err());
}

#[test]
fn profile_edit_and_capability_expiry_do_not_change_snapshots() {
    let (_directory, storage) = fixture();
    storage
        .connection
        .execute(
            "UPDATE provider_profiles SET name = 'renamed', config_revision = 2",
            [],
        )
        .unwrap();
    storage
        .connection
        .execute("DELETE FROM model_capabilities", [])
        .unwrap();
    assert_eq!(
        storage
            .connection
            .query_row("SELECT profile_name FROM ai_config_snapshots", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
        "合成服务"
    );
    assert!(
        storage
            .connection
            .execute("DELETE FROM provider_profiles", [])
            .is_err()
    );
}

#[test]
fn capabilities_require_current_profile_revision_and_observation_window() {
    let (_directory, mut storage) = fixture();
    for (at, expected) in [(999, 0), (1000, 1), (1999, 1), (2000, 0)] {
        assert_eq!(
            storage.capabilities().count_valid("provider1", at).unwrap(),
            expected
        );
    }
    storage
        .connection
        .execute(
            "UPDATE provider_profiles SET config_revision = 2 WHERE id = 'provider1'",
            [],
        )
        .unwrap();
    assert_eq!(
        storage
            .capabilities()
            .count_valid("provider1", 1001)
            .unwrap(),
        0
    );
    assert_eq!(
        storage
            .connection
            .query_row(
                "SELECT config_revision FROM ai_config_snapshots WHERE id = 'snap1'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
}

#[test]
fn delete_history_detaches_audit_and_requires_dependency_order() {
    let (_directory, storage) = fixture();
    assert!(
        storage
            .connection
            .execute("DELETE FROM projects", [])
            .is_err()
    );
    storage.connection.execute_batch("BEGIN; DELETE FROM approvals; DELETE FROM tasks; DELETE FROM ai_config_snapshots; DELETE FROM sessions; DELETE FROM projects; DELETE FROM settings; DELETE FROM provider_profiles; COMMIT;").unwrap();
    assert_eq!(storage.connection.query_row("SELECT count(*) FROM audit_events WHERE task_id IS NULL AND approval_id IS NULL AND project_id IS NULL AND profile_id IS NULL", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
}

#[test]
fn foreign_key_corruption_blocks_open() {
    let (directory, storage) = fixture();
    storage
        .connection
        .pragma_update(None, "foreign_keys", false)
        .unwrap();
    storage
        .connection
        .execute("UPDATE project_usage SET project_id = 'missing'", [])
        .unwrap();
    assert_eq!(
        Storage::open(&directory.0).err().unwrap().code,
        "STORAGE_FOREIGN_KEY_CHECK"
    );
}

#[test]
fn approval_and_task_lifecycle_constraints() {
    let (_directory, storage) = fixture();
    assert!(
        storage
            .connection
            .execute("UPDATE approvals SET decision = 'pending'", [])
            .is_err()
    );
    assert!(
        storage
            .connection
            .execute("UPDATE approvals SET decided_at = NULL", [])
            .is_err()
    );
    assert!(
        storage
            .connection
            .execute("UPDATE tasks SET finished_at = NULL", [])
            .is_err()
    );
    assert!(
        storage
            .connection
            .execute("UPDATE tasks SET finished_at = 0", [])
            .is_err()
    );
    // Cancellation during preparation has no fabricated runtime configuration.
    storage.connection.execute("INSERT INTO tasks VALUES ('cancel-before-start', 's1', NULL, NULL, NULL, NULL, 'cancelled', NULL, 1000, NULL, 1001)", []).unwrap();
}

#[test]
fn checksum_is_stable_across_checkout_line_endings() {
    assert_eq!(
        checksum("SELECT 1;\nSELECT 2;\n"),
        checksum("SELECT 1;\r\nSELECT 2;\r\n")
    );
}

#[test]
fn real_v1_settings_upgrade_preserves_values_and_backs_up_original() {
    let directory = Fixture::new();
    let mut old = Storage::open_with(&directory.0, &MIGRATIONS[..1]).unwrap();
    old.settings()
        .put_text("v1-search", "search_engine", "google", 7)
        .unwrap();
    drop(old);
    let mut upgraded = Storage::open(&directory.0).unwrap();
    upgraded
        .settings()
        .initialize_defaults(&directory.0)
        .unwrap();
    let view = upgraded.settings().read_view().unwrap();
    assert_eq!(view.revision, 7);
    assert!(
        view.values
            .contains(&crate::settings::Change::SearchEngine("google".into()))
    );
    let backup = fs::read_dir(directory.0.join("backups"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let original = Connection::open_with_flags(backup, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert_eq!(
        original
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        original
            .query_row(
                "SELECT value_text FROM settings WHERE key='search_engine'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "google"
    );
}

#[test]
fn real_v2_credentials_upgrade_preserves_profiles_settings_audit_and_backup() {
    use crate::settings::Change;

    let directory = Fixture::new();
    let mut old = Storage::open_with(&directory.0, &MIGRATIONS[..2]).unwrap();
    old.settings().initialize_defaults(&directory.0).unwrap();
    let previous = old
        .settings()
        .update_view(
            1,
            &[
                Change::SearchEngine("google".into()),
                Change::OverlayEnabled(false),
            ],
            |_| Ok(()),
        )
        .unwrap();
    old.connection.execute_batch(
        "INSERT INTO provider_profiles VALUES ('legacy-key', 'synthetic', 'relay', 'self_managed', NULL,
            'https://fixture.invalid/v1', 'https://fixture.invalid', 'responses', 'api_key',
            'credential:legacy-key:7', 7, NULL, NULL, 1, 9, 1000, 1000);
         INSERT INTO provider_profiles VALUES ('legacy-login', 'synthetic', 'official', 'self_managed', NULL,
            'https://fixture.invalid/v1', 'https://fixture.invalid', 'responses', 'external_login',
            NULL, 0, NULL, NULL, 0, 1, 1000, 1000);
         INSERT INTO audit_events (id, occurred_at, correlation_id, event_kind, outcome, profile_id)
            VALUES ('legacy-audit', 1001, 'synthetic', 'config_read', 'success', 'legacy-key');"
    ).unwrap();
    let audit_count: i64 = old
        .connection
        .query_row("SELECT count(*) FROM audit_events", [], |row| row.get(0))
        .unwrap();
    // Keep the v2 connection open so the upgrade must preserve committed WAL data.
    let mut upgraded = Storage::open(&directory.0).unwrap();
    assert_eq!(upgraded.settings().read_view().unwrap(), previous);
    let configured = upgraded
        .profiles()
        .credential_profile("legacy-key")
        .unwrap()
        .unwrap();
    assert_eq!(
        configured.credential_ref.as_deref(),
        Some("credential:legacy-key:7")
    );
    assert_eq!(configured.credential_revision, 7);
    assert_eq!(configured.config_revision, 9);
    assert_eq!(configured.credential_status, "configured_unverified");
    assert_eq!(
        upgraded
            .profiles()
            .credential_profile("legacy-login")
            .unwrap()
            .unwrap()
            .credential_status,
        "missing"
    );
    assert_eq!(
        upgraded
            .connection
            .query_row("SELECT count(*) FROM audit_events", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        audit_count
    );
    assert_eq!(
        upgraded
            .connection
            .query_row(
                "SELECT profile_id FROM audit_events WHERE id = 'legacy-audit'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "legacy-key"
    );
    let backup_path = fs::read_dir(directory.0.join("backups"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut backup =
        Connection::open_with_flags(backup_path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert_eq!(
        backup
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert!(
        backup
            .prepare("SELECT credential_status FROM provider_profiles")
            .is_err()
    );
    assert_eq!(
        backup
            .query_row("SELECT count(*) FROM audit_events", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        audit_count
    );
    assert_eq!(
        SettingsRepository::new(&mut backup).read_view().unwrap(),
        previous
    );
    drop(upgraded);
    drop(old);
    let mut reopened = Storage::open(&directory.0).unwrap();
    assert_eq!(reopened.settings().read_view().unwrap(), previous);
    assert_eq!(
        reopened
            .connection
            .query_row("SELECT count(*) FROM schema_migrations", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        SCHEMA_VERSION
    );
}

#[test]
fn real_v3_provider_upgrade_preserves_defaults_and_old_schema_backup() {
    let directory = Fixture::new();
    let mut old = Storage::open_with(&directory.0, &MIGRATIONS[..3]).unwrap();
    probe::seed(&mut old.connection).unwrap();
    let before: i64 = old
        .connection
        .query_row("SELECT count(*) FROM ai_config_snapshots", [], |r| r.get(0))
        .unwrap();
    let upgraded = Storage::open(&directory.0).unwrap();
    assert_eq!(
        upgraded
            .connection
            .query_row("SELECT count(*) FROM ai_config_snapshots", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        before
    );
    let missing:i64=upgraded.connection.query_row("SELECT count(*) FROM settings s LEFT JOIN ai_default_revisions r ON r.setting_id=s.id WHERE s.key='ai_defaults' AND r.setting_id IS NULL",[],|r|r.get(0)).unwrap();
    assert_eq!(missing, 0);
    assert_eq!(
        upgraded
            .connection
            .query_row("SELECT blocked FROM ai_runtime", [], |r| r
                .get::<_, bool>(0))
            .unwrap(),
        false
    );
    let backup = fs::read_dir(directory.0.join("backups"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let backup = Connection::open_with_flags(backup, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert_eq!(
        backup
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert!(backup.prepare("SELECT * FROM provider_operations").is_err());
}

#[test]
fn failed_sql_errors_never_expose_query_values() {
    let (directory, storage) = fixture();
    let result = Storage::open_with(
        &directory.0,
        &upgrade("SELECT synthetic_secret_never_logged FROM missing_fixture;"),
    );
    let failure = result.err().unwrap();
    assert_eq!(failure.code, "STORAGE_MIGRATION_FAILED");
    assert!(!format!("{failure:?} {failure}").contains("synthetic_secret"));
    drop(storage);
}

#[test]
fn migration_sequence_has_no_gaps() {
    let directory = Fixture::new();
    let migrations = [Migration {
        version: 2,
        name: "gap",
        sql: "SELECT 1;",
    }];
    assert_eq!(
        Storage::open_with(&directory.0, &migrations)
            .err()
            .unwrap()
            .code,
        "STORAGE_MIGRATION_SEQUENCE"
    );
}

#[test]
fn startup_recovery_interrupts_work_and_is_idempotent() {
    let (_directory, mut storage) = fixture();
    storage
        .connection
        .execute_batch(
            "UPDATE tasks SET status = 'running', started_at = 1001, finished_at = NULL, snapshot_id = 'snap1' WHERE id = 't1'; UPDATE approvals SET decision = 'pending', decided_at = NULL WHERE id = 'a1';",
        )
        .unwrap();
    let first = storage.recover_on_startup("recovery-1").unwrap();
    assert_eq!(
        first,
        RecoverySummary {
            tasks: 1,
            approvals: 1
        }
    );
    assert_eq!(
        storage
            .connection
            .query_row("SELECT status FROM tasks WHERE id = 't1'", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "interrupted"
    );
    assert_eq!(
        storage
            .connection
            .query_row("SELECT decision FROM approvals WHERE id = 'a1'", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
        "interrupted"
    );
    assert_eq!(
        storage
            .connection
            .query_row(
                "SELECT count(*) FROM audit_events WHERE event_kind = 'storage_recovery'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(
        storage.recover_on_startup("recovery-2").unwrap(),
        RecoverySummary {
            tasks: 0,
            approvals: 0
        }
    );
    assert_eq!(
        storage
            .connection
            .query_row(
                "SELECT count(*) FROM audit_events WHERE event_kind = 'storage_recovery'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn typed_repositories_keep_business_access_out_of_raw_ipc() {
    let (_directory, mut storage) = fixture();
    let setting = storage.settings().get_global("ai_defaults").unwrap();
    assert!(setting.is_some());
    storage
        .settings()
        .put_text("global-search", "search_engine", "bing", 1)
        .unwrap();
    assert_eq!(
        storage
            .settings()
            .get_global("search_engine")
            .unwrap()
            .unwrap()
            .value_text
            .as_deref(),
        Some("bing")
    );
    assert!(
        storage
            .settings()
            .put_text("bad-setting", "api_key", "synthetic", 1)
            .is_err()
    );
    assert!(storage.profiles().exists("provider1").unwrap());
    assert_eq!(
        storage
            .capabilities()
            .count_valid("provider1", 1001)
            .unwrap(),
        1
    );
    assert!(storage.snapshots().exists("snap1").unwrap());
    storage
        .audit()
        .record(AuditEventInput {
            id: "typed-audit",
            correlation_id: "typed-correlation",
            event_kind: "config_read",
            outcome: "success",
            error_code: None,
            profile_id: None,
        })
        .unwrap();
    assert_eq!(
        storage
            .connection
            .query_row(
                "SELECT count(*) FROM audit_events WHERE id = 'typed-audit'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn credential_commit_updates_reference_invalidates_cache_and_audits_atomically() {
    let (_directory, mut storage) = fixture();
    let correlation_id = crate::ipc::correlation_id();
    let committed = storage
        .profiles()
        .commit_credential_change(
            "provider1",
            1,
            2,
            "credential:provider1:2",
            "configured_unverified",
            &correlation_id,
            |profile| {
                assert_eq!(profile.credential_revision, 1);
                Ok(())
            },
            |_| panic!("successful commit must not roll back"),
            |_| {},
        )
        .unwrap();

    assert_eq!(committed.credential_revision, 2);
    assert_eq!(
        committed.credential_ref.as_deref(),
        Some("credential:provider1:2")
    );
    assert_eq!(committed.config_revision, 2);
    assert_eq!(
        storage
            .capabilities()
            .count_valid("provider1", 1001)
            .unwrap(),
        0
    );
    assert_eq!(
        storage.connection.query_row(
            "SELECT count(*) FROM audit_events WHERE event_kind = 'credential_change' AND outcome = 'success' AND profile_id = 'provider1'",
            [],
            |row| row.get::<_, i64>(0),
        ).unwrap(),
        1
    );
}

#[test]
fn credential_audit_failure_rolls_back_metadata_and_invokes_compensation() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let (_directory, mut storage) = fixture();
    storage
        .connection
        .execute_batch(
            "CREATE TRIGGER reject_credential_audit BEFORE INSERT ON audit_events WHEN NEW.event_kind = 'credential_change' BEGIN SELECT RAISE(ABORT, 'fixture'); END;",
        )
        .unwrap();
    let rolled_back = AtomicBool::new(false);
    let correlation_id = crate::ipc::correlation_id();
    let result = storage.profiles().commit_credential_change(
        "provider1",
        1,
        2,
        "credential:provider1:2",
        "configured_unverified",
        &correlation_id,
        |_| Ok(()),
        |_| {
            rolled_back.store(true, Ordering::SeqCst);
            true
        },
        |_| panic!("failed audit must not commit"),
    );

    assert!(result.is_err());
    assert!(rolled_back.load(Ordering::SeqCst));
    let profile = storage
        .profiles()
        .credential_profile("provider1")
        .unwrap()
        .unwrap();
    assert_eq!(profile.credential_revision, 1);
    assert_eq!(profile.config_revision, 1);
    assert_eq!(profile.credential_status, "configured_unverified");
    assert_eq!(
        storage
            .capabilities()
            .count_valid("provider1", 1001)
            .unwrap(),
        1
    );
}

#[test]
fn credential_compensation_retains_write_lock_through_statement_and_commit_failure() {
    for commit_failure in [false, true] {
        let (_directory, mut storage) = fixture();
        let competing = Connection::open(&storage.path).unwrap();
        competing.busy_timeout(Duration::ZERO).unwrap();
        storage.connection.execute_batch(if commit_failure {
            "CREATE TEMP TABLE commit_guard (profile TEXT REFERENCES provider_profiles(id)); CREATE TEMP TRIGGER reject_commit AFTER UPDATE ON provider_profiles BEGIN INSERT INTO commit_guard VALUES ('absent-profile'); END; PRAGMA defer_foreign_keys = ON;"
        } else {
            "CREATE TEMP TRIGGER reject_audit BEFORE INSERT ON audit_events BEGIN SELECT RAISE(ABORT, 'fixture'); END;"
        }).unwrap();
        let compensated = std::cell::Cell::new(false);
        let result = storage.profiles().commit_credential_change(
            "provider1",
            1,
            2,
            "credential:provider1:2",
            "configured_unverified",
            &crate::ipc::correlation_id(),
            |_| Ok(()),
            |_| {
                let error = competing.execute_batch("BEGIN IMMEDIATE").unwrap_err();
                assert_eq!(
                    error.sqlite_error_code(),
                    Some(rusqlite::ErrorCode::DatabaseBusy)
                );
                compensated.set(true);
                true
            },
            |_| panic!("failed transaction must not commit"),
        );
        assert!(result.is_err());
        assert!(compensated.get());
        competing
            .execute_batch("BEGIN IMMEDIATE; ROLLBACK;")
            .unwrap();
        assert_eq!(
            storage
                .profiles()
                .credential_profile("provider1")
                .unwrap()
                .unwrap()
                .config_revision,
            1
        );
    }
}

#[test]
fn audit_retention_keeps_boundary_and_all_non_audit_data() {
    let (_directory, mut storage) = fixture();
    let at = now_ms();
    let cutoff = at - AUDIT_RETENTION_MS;
    storage
        .connection
        .execute("UPDATE audit_events SET occurred_at=?1", [cutoff - 1])
        .unwrap();
    storage.connection.execute(
        "INSERT INTO audit_events (id,occurred_at,correlation_id,event_kind,outcome) VALUES ('boundary',?1,'fixture','config_switch','success'),('recent',?2,'fixture','config_switch','success')",
        rusqlite::params![cutoff, at],
    ).unwrap();
    let approvals_before: (String, i64) = storage
        .connection
        .query_row(
            "SELECT decision,decided_at FROM approvals WHERE id='a1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(storage.audit().prune_expired(at).unwrap(), 1);
    let ids = storage
        .connection
        .prepare("SELECT id FROM audit_events ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(ids, ["boundary", "recent"]);
    let approvals_after: (String, i64) = storage
        .connection
        .query_row(
            "SELECT decision,decided_at FROM approvals WHERE id='a1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(approvals_after, approvals_before);
    for (table, expected) in [
        ("projects", 1),
        ("project_usage", 1),
        ("sessions", 1),
        ("tasks", 1),
        ("approvals", 1),
        ("provider_profiles", 1),
        ("model_capabilities", 1),
        ("ai_config_snapshots", 1),
        ("settings", 2),
        ("schema_migrations", SCHEMA_VERSION),
    ] {
        assert_eq!(
            storage
                .connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0),)
                .unwrap(),
            expected,
            "cleanup changed {table}"
        );
    }
}

#[test]
fn audit_retention_rejects_clock_anomalies_without_deleting() {
    let (_directory, mut storage) = fixture();
    for at in [
        -1,
        i64::MIN,
        i64::MAX,
        now_ms() + AUDIT_RETENTION_MS,
        now_ms() - AUDIT_RETENTION_MS,
    ] {
        assert_eq!(
            storage.audit().prune_expired(at).unwrap_err().code,
            "STORAGE_CLOCK_FAILED"
        );
        assert_eq!(
            storage
                .connection
                .query_row("SELECT count(*) FROM audit_events", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    let future = now_ms() + AUDIT_RETENTION_MS;
    storage.connection.execute(
        "INSERT INTO audit_events (id,occurred_at,correlation_id,event_kind,outcome) VALUES ('future',?1,'fixture','config_switch','success')", [future],
    ).unwrap();
    assert_eq!(
        storage.audit().prune_expired(now_ms()).unwrap_err().code,
        "STORAGE_CLOCK_FAILED"
    );
    assert_eq!(
        storage
            .connection
            .query_row("SELECT count(*) FROM audit_events", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn startup_prunes_only_audit_and_cleanup_failure_preserves_data() {
    let (directory, storage) = fixture();
    let runtime = StorageState::default();
    runtime
        .initialize(&directory.0, &crate::ipc::correlation_id())
        .unwrap();
    assert_eq!(runtime.health().state, "ready");
    assert_eq!(
        storage
            .connection
            .query_row("SELECT count(*) FROM audit_events", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        storage
            .connection
            .query_row("SELECT decision FROM approvals WHERE id='a1'", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
        "denied"
    );
    let (failed_directory, failed_storage) = fixture();
    failed_storage.connection.execute_batch(
        "CREATE TRIGGER fail_audit_delete BEFORE DELETE ON audit_events BEGIN SELECT RAISE(ABORT,'injected cleanup failure'); END;"
    ).unwrap();
    let failed_runtime = StorageState::default();
    let failure = failed_runtime
        .initialize(&failed_directory.0, &crate::ipc::correlation_id())
        .unwrap_err();
    assert_eq!(failure.public_code(), "PA_AUDIT_WRITE_FAILED");
    assert_eq!(failed_runtime.health().state, "unavailable");
    assert_eq!(
        failed_runtime.health().reason_code,
        Some("PA_AUDIT_WRITE_FAILED")
    );
    assert_eq!(
        failed_storage
            .connection
            .query_row("SELECT count(*) FROM audit_events", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        failed_storage
            .connection
            .query_row("SELECT decision FROM approvals WHERE id='a1'", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
        "denied"
    );
}

#[test]
fn rejected_audit_and_settings_secret_sentinels_never_reach_database_or_backup() {
    let directory = Fixture::new();
    let mut storage = Storage::open(&directory.0).unwrap();
    storage
        .settings()
        .initialize_defaults(&directory.0)
        .unwrap();
    let sentinel = "sk-audit-secret-sentinel-DoNotPersist-a1b2c3";
    let id = repositories::new_audit_id();
    let correlation = crate::ipc::correlation_id();
    let disguised_id = format!("audit-{sentinel}");
    let future_id = format!(
        "{}-18446744073709551615",
        correlation.rsplit_once('-').unwrap().0
    );
    for (event_id, correlation_id, event_kind, outcome, error_code) in [
        (
            sentinel,
            correlation.as_str(),
            "config_switch",
            "success",
            None,
        ),
        (
            disguised_id.as_str(),
            correlation.as_str(),
            "config_switch",
            "success",
            None,
        ),
        (id.as_str(), sentinel, "config_switch", "success", None),
        (
            id.as_str(),
            future_id.as_str(),
            "config_switch",
            "success",
            None,
        ),
        (id.as_str(), correlation.as_str(), sentinel, "success", None),
        (
            id.as_str(),
            correlation.as_str(),
            "config_switch",
            sentinel,
            None,
        ),
        (
            id.as_str(),
            correlation.as_str(),
            "config_switch",
            "failure",
            Some(sentinel),
        ),
        (
            id.as_str(),
            correlation.as_str(),
            "config_switch",
            "failure",
            Some("PA_NOT_A_REGISTERED_ERROR"),
        ),
    ] {
        let failure = storage
            .audit()
            .record(AuditEventInput {
                id: event_id,
                correlation_id,
                event_kind,
                outcome,
                error_code,
                profile_id: None,
            })
            .unwrap_err();
        assert_eq!(failure.public_code(), "PA_AUDIT_WRITE_FAILED");
        assert!(!format!("{failure:?} {failure}").contains(sentinel));
    }
    assert!(
        storage
            .settings()
            .put_text("global-search", "search_engine", sentinel, 1)
            .is_err()
    );
    assert!(matches!(
        storage.settings().update_view_with_id(
            1,
            &[crate::settings::Change::SearchEngine(sentinel.into())],
            &correlation,
            |_| panic!("rejected setting must precede prepare"),
        ),
        Err(crate::ipc::ErrorCode::SettingsInvalidValue)
    ));
    assert!(matches!(
        storage.settings().update_view_with_id(
            1,
            &[crate::settings::Change::OverlayEnabled(false)],
            sentinel,
            |_| panic!("rejected correlation must precede prepare"),
        ),
        Err(crate::ipc::ErrorCode::AuditWriteFailed)
    ));
    assert_eq!(
        storage.settings().read_view().unwrap(),
        crate::settings::View::defaults(1)
    );
    let backup_path = Storage::backup(&directory.0, SCHEMA_VERSION).unwrap();
    drop(storage);
    for path in [directory.0.join(DATABASE_NAME), backup_path] {
        let bytes = fs::read(path).unwrap();
        assert!(
            !bytes
                .windows(sentinel.len())
                .any(|window| window == sentinel.as_bytes())
        );
    }
}

#[test]
fn legacy_test_writer_cannot_commit_without_audit() {
    let directory = Fixture::new();
    let mut storage = Storage::open(&directory.0).unwrap();
    storage.connection.execute_batch(
        "CREATE TEMP TRIGGER fail_audit_insert BEFORE INSERT ON audit_events BEGIN SELECT RAISE(ABORT,'injected audit failure'); END;"
    ).unwrap();
    assert_eq!(
        storage
            .settings()
            .put_text("global-search", "search_engine", "bing", 1)
            .unwrap_err()
            .public_code(),
        "PA_AUDIT_WRITE_FAILED"
    );
    assert_eq!(
        storage
            .connection
            .query_row("SELECT count(*) FROM settings", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        storage
            .connection
            .query_row("SELECT count(*) FROM audit_events", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
