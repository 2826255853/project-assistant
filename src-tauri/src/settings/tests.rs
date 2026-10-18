use super::*;
use crate::storage::Storage;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(std::env::temp_dir().join(format!(
            "pa-settings-{}-{}-{}",
            std::process::id(),
            storage::now_ms(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn defaults_missing_fields_and_reopen() {
    let dir = Fixture::new();
    let mut db = Storage::open(&dir.0).unwrap();
    db.settings().initialize_defaults(&dir.0).unwrap();
    assert_eq!(db.settings().read_view().unwrap(), View::defaults(1));
    let changes = [
        Change::SearchEngine("google".into()),
        Change::SummaryRetentionDays(60),
        Change::OverlayEnabled(false),
        Change::OverlayShortcut("Ctrl+Alt+F9".into()),
        Change::OverlayPosition(Some(Position {
            x: 120,
            y: 130,
            monitor: Some("test-monitor".into()),
            scale_factor: 1.5,
        })),
    ];
    let written = db.settings().update_view(1, &changes, |_| Ok(())).unwrap();
    drop(db);
    let mut reopened = Storage::open(&dir.0).unwrap();
    reopened.settings().initialize_defaults(&dir.0).unwrap();
    assert_eq!(reopened.settings().read_view().unwrap(), written);
    reopened
        .connection
        .execute("DELETE FROM settings WHERE key='log_level'", [])
        .unwrap();
    assert!(
        reopened
            .settings()
            .read_view()
            .unwrap()
            .values
            .contains(&Change::LogLevel("info".into()))
    );
}

#[test]
fn illegal_batch_duplicate_key_and_secret_are_rejected_without_writes() {
    let dir = Fixture::new();
    let mut db = Storage::open(&dir.0).unwrap();
    db.settings().initialize_defaults(&dir.0).unwrap();
    for change in [
        Change::ApprovalPolicy("never".into()),
        Change::SummaryRetentionDays(0),
        Change::SummaryRetentionDays(366),
        Change::Editor(Some(Editor {
            id: "custom".into(),
            executable: "powershell.exe -Command secret".into(),
        })),
        Change::LogLevel("trace".into()),
    ] {
        assert!(
            db.settings()
                .update_view(1, &[Change::SearchEngine("google".into()), change], |_| Ok(
                    ()
                ))
                .is_err()
        );
        assert_eq!(db.settings().read_view().unwrap(), View::defaults(1));
    }
    for value in [
        serde_json::json!({"key":"api_key","value":"synthetic-secret"}),
        serde_json::json!({"key":"overlay_enabled","value":"true"}),
        serde_json::json!({"key":"search_engine","value":"bing","secret":"synthetic-secret"}),
    ] {
        assert!(serde_json::from_value::<Change>(value).is_err());
    }
    assert!(
        db.settings()
            .update_view(
                1,
                &[
                    Change::SearchEngine("bing".into()),
                    Change::SearchEngine("google".into())
                ],
                |_| Ok(())
            )
            .is_err()
    );
    let error = crate::ipc::Response::<View>::from_result(
        crate::ipc::correlation_id(),
        Err(ErrorCode::SettingsInvalidValue),
    );
    assert!(
        !serde_json::to_string(&error)
            .unwrap()
            .contains("synthetic-secret")
    );
}

#[test]
fn concurrent_connections_conflict_and_failed_sql_or_prepare_rolls_back_everything() {
    let dir = Fixture::new();
    let mut db = Storage::open(&dir.0).unwrap();
    db.settings().initialize_defaults(&dir.0).unwrap();
    let mut second = Storage::open(&dir.0).unwrap();
    let first = db
        .settings()
        .update_view(1, &[Change::SearchEngine("google".into())], |_| Ok(()))
        .unwrap();
    assert!(matches!(
        second
            .settings()
            .update_view(1, &[Change::OverlayEnabled(false)], |_| panic!(
                "conflict must precede runtime changes"
            )),
        Err(ErrorCode::RevisionConflict)
    ));
    assert_eq!(second.settings().read_view().unwrap(), first);
    assert!(matches!(
        db.settings().update_view(
            2,
            &[
                Change::SearchEngine("bing".into()),
                Change::OverlayShortcut("Ctrl+Alt+F8".into())
            ],
            |_| Err(ErrorCode::ShortcutConflict)
        ),
        Err(ErrorCode::ShortcutConflict)
    ));
    assert_eq!(db.settings().read_view().unwrap(), first);
    db.connection.execute_batch("CREATE TRIGGER fail_settings BEFORE UPDATE ON resident_preferences BEGIN SELECT RAISE(ABORT,'synthetic-secret'); END;").unwrap();
    assert!(matches!(
        db.settings().update_view(
            2,
            &[
                Change::SearchEngine("bing".into()),
                Change::OverlayEnabled(false)
            ],
            |_| Ok(())
        ),
        Err(ErrorCode::SettingsStorageFailed)
    ));
    assert_eq!(db.settings().read_view().unwrap(), first);
}

#[test]
fn corrupted_values_are_errors_and_are_preserved() {
    let dir = Fixture::new();
    let mut db = Storage::open(&dir.0).unwrap();
    db.settings().initialize_defaults(&dir.0).unwrap();
    db.connection
        .execute(
            "UPDATE settings SET value_text='synthetic-secret' WHERE key='search_engine'",
            [],
        )
        .unwrap();
    assert!(matches!(
        db.settings().read_view(),
        Err(ErrorCode::SettingsCorrupt)
    ));
    assert!(matches!(
        db.settings().initialize_defaults(&dir.0),
        Err(ErrorCode::SettingsCorrupt)
    ));
    assert_eq!(
        db.connection
            .query_row(
                "SELECT value_text FROM settings WHERE key='search_engine'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "synthetic-secret"
    );
}

#[test]
fn legacy_disabled_preferences_import_once_and_corrupt_legacy_is_not_defaulted() {
    let dir = Fixture::new();
    let mut db = Storage::open(&dir.0).unwrap();
    fs::write(dir.0.join("overlay-settings.json"),r#"{"enabled":false,"shortcut":"Ctrl+Alt+F9","position":{"x":12,"y":20,"monitor":"legacy"}}"#).unwrap();
    db.settings().initialize_defaults(&dir.0).unwrap();
    assert!(!db.settings().read_view().unwrap().resident().0);
    fs::write(dir.0.join("overlay-settings.json"), "bad obsolete file").unwrap();
    db.settings().initialize_defaults(&dir.0).unwrap();
    assert!(!db.settings().read_view().unwrap().resident().0);
    let corrupt_dir = Fixture::new();
    let mut corrupt_db = Storage::open(&corrupt_dir.0).unwrap();
    fs::write(
        corrupt_dir.0.join("overlay-settings.json"),
        "synthetic-secret",
    )
    .unwrap();
    assert!(matches!(
        corrupt_db.settings().initialize_defaults(&corrupt_dir.0),
        Err(ErrorCode::SettingsCorrupt)
    ));
    assert_eq!(
        corrupt_db
            .connection
            .query_row("SELECT count(*) FROM settings", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn strict_scopes_revisions_and_ai_gate() {
    for scope in [
        Scope {
            scope: "project".into(),
            project_id: Some("registered-but-no-grant".into()),
        },
        Scope {
            scope: "global".into(),
            project_id: Some("unexpected".into()),
        },
    ] {
        assert!(scope.authorize().is_err());
    }
    let good = serde_json::json!({"apiVersion":1,"params":{"scope":"global","expectedRevision":1,"changes":[{"key":"log_level","value":"debug"}]}});
    crate::ipc::parse_request::<Update>(good)
        .unwrap()
        .validate()
        .unwrap();
    assert!(matches!(
        Change::AiDefaults(Some(serde_json::json!({"secret":"synthetic"}))).validate(),
        Err(ErrorCode::SettingsAiUnavailable)
    ));
    Change::Editor(Some(Editor {
        id: "custom".into(),
        executable: r"C:\Program Files\Editor\editor.exe".into(),
    }))
    .validate()
    .unwrap();
}

#[test]
fn audit_insert_failure_prevents_prepare_and_rolls_back_all_setting_tables() {
    use std::cell::Cell;
    let dir = Fixture::new();
    let mut db = Storage::open(&dir.0).unwrap();
    db.settings().initialize_defaults(&dir.0).unwrap();
    let before = db.settings().read_view().unwrap();
    let count: i64 = db
        .connection
        .query_row("SELECT count(*) FROM audit_events", [], |r| r.get(0))
        .unwrap();
    db.connection.execute_batch(
        "CREATE TEMP TRIGGER fail_audit BEFORE INSERT ON audit_events BEGIN SELECT RAISE(ABORT,'injected audit failure'); END;"
    ).unwrap();
    let prepared = Cell::new(false);
    let changes = [
        Change::SearchEngine("google".into()),
        Change::OverlayEnabled(false),
        Change::OverlayShortcut("Ctrl+Alt+F8".into()),
        Change::OverlayPosition(Some(Position {
            x: 44,
            y: 55,
            monitor: None,
            scale_factor: 1.25,
        })),
    ];
    let result = db.settings().update_view_with_id(
        before.revision,
        &changes,
        &crate::ipc::correlation_id(),
        |_| {
            prepared.set(true);
            Ok(())
        },
    );
    assert!(matches!(result, Err(ErrorCode::AuditWriteFailed)));
    assert!(!prepared.get());
    assert_eq!(db.settings().read_view().unwrap(), before);
    assert_eq!(
        db.connection
            .query_row("SELECT count(*) FROM audit_events", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        count
    );
}

#[test]
fn settings_resident_and_position_commits_use_command_id_and_prepare_rollback_removes_audit() {
    let dir = Fixture::new();
    let mut db = Storage::open(&dir.0).unwrap();
    db.settings().initialize_defaults(&dir.0).unwrap();
    let mut expected = 1;
    for changes in [
        vec![Change::LogLevel("warn".into())],
        vec![
            Change::OverlayEnabled(false),
            Change::OverlayShortcut("Ctrl+Alt+F8".into()),
        ],
        vec![Change::OverlayPosition(Some(Position {
            x: 12,
            y: 34,
            monitor: Some("local-monitor".into()),
            scale_factor: 1.0,
        }))],
    ] {
        let id = crate::ipc::correlation_id();
        let written = db
            .settings()
            .update_view_with_id(expected, &changes, &id, |_| Ok(()))
            .unwrap();
        assert_eq!(written.revision, expected + 1);
        assert_eq!(db.connection.query_row(
            "SELECT count(*) FROM audit_events WHERE correlation_id=?1 AND event_kind='config_switch' AND outcome='success' AND error_code IS NULL AND project_id IS NULL AND task_id IS NULL AND approval_id IS NULL AND profile_id IS NULL AND model_id IS NULL AND effort IS NULL",
            [&id], |r| r.get::<_, i64>(0),
        ).unwrap(), 1);
        expected += 1;
    }
    let before = db.settings().read_view().unwrap();
    let id = crate::ipc::correlation_id();
    assert!(matches!(
        db.settings()
            .update_view_with_id(expected, &[Change::OverlayEnabled(true)], &id, |_| Err(
                ErrorCode::ShortcutConflict
            ),),
        Err(ErrorCode::ShortcutConflict)
    ));
    assert_eq!(db.settings().read_view().unwrap(), before);
    assert_eq!(
        db.connection
            .query_row(
                "SELECT count(*) FROM audit_events WHERE correlation_id=?1",
                [&id],
                |r| r.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
}

#[test]
fn audit_cleanup_failure_is_atomic_and_prevents_prepare() {
    let dir = Fixture::new();
    let mut db = Storage::open(&dir.0).unwrap();
    db.settings().initialize_defaults(&dir.0).unwrap();
    db.connection.execute_batch(
        "UPDATE audit_events SET occurred_at=1; CREATE TEMP TRIGGER fail_prune BEFORE DELETE ON audit_events BEGIN SELECT RAISE(ABORT,'injected cleanup failure'); END;"
    ).unwrap();
    assert!(matches!(
        db.settings().update_view_with_id(
            1,
            &[Change::OverlayEnabled(false)],
            &crate::ipc::correlation_id(),
            |_| panic!("cleanup failure must precede runtime changes"),
        ),
        Err(ErrorCode::AuditWriteFailed)
    ));
    assert_eq!(db.settings().read_view().unwrap(), View::defaults(1));
    assert_eq!(
        db.connection
            .query_row(
                "SELECT count(*) FROM audit_events WHERE occurred_at=1",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
}

#[test]
fn settings_commit_prunes_expired_audit_and_initialization_is_audited_once() {
    let dir = Fixture::new();
    let mut db = Storage::open(&dir.0).unwrap();
    db.settings().initialize_defaults(&dir.0).unwrap();
    db.settings().initialize_defaults(&dir.0).unwrap();
    assert_eq!(
        db.connection
            .query_row("SELECT count(*) FROM audit_events", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    db.connection
        .execute("UPDATE audit_events SET occurred_at=1", [])
        .unwrap();
    let id = crate::ipc::correlation_id();
    db.settings()
        .update_view_with_id(1, &[Change::OverlayEnabled(false)], &id, |_| Ok(()))
        .unwrap();
    assert_eq!(
        db.connection
            .query_row("SELECT count(*) FROM audit_events", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.connection
            .query_row("SELECT correlation_id FROM audit_events", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        id
    );
}
