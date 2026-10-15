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
    assert!(Storage::open(&directory.0).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
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
