//! Synthetic fixtures for step 0.5 only; never imports Codex or cc-switch data.
use super::{Storage, StorageError, error, sql_error};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::path::Path;

pub fn seed(connection: &mut Connection) -> Result<(), StorageError> {
    let transaction = connection.transaction().map_err(sql_error)?;
    transaction.execute_batch(r#"
        INSERT INTO projects VALUES ('p1', 'C:\\阶段 0 项目 & 空格', 'fixture:p1', '阶段 0 项目', NULL, 1000, 1000);
        INSERT INTO project_usage VALUES ('p1', 1000, 1, 'codex');
        INSERT INTO provider_profiles (id, name, source_kind, adapter_kind, external_profile_id,
          endpoint, endpoint_origin, wire_api, auth_kind, credential_ref, credential_revision,
          default_model, default_effort, enabled, config_revision, created_at, updated_at, credential_status)
          VALUES ('provider1', '合成服务', 'relay', 'self_managed', NULL,
          'https://fixture.invalid/v1', 'https://fixture.invalid', 'responses', 'api_key',
          'credential:provider1:1', 1, 'fixture-model', 'fixture-effort', 1, 1, 1000, 1000, 'configured_unverified');
        INSERT INTO model_capabilities VALUES ('provider1', 'fixture-model', 'fixture-effort', 4096, 1,
          'connection_probe', 'fixture-v1', 1, 1000, 2000);
        INSERT INTO sessions VALUES ('s1', 'p1', 'fixture-thread', NULL, 'active', 1000, 1000);
        INSERT INTO ai_config_snapshots VALUES ('snap1', 'provider1', '合成服务', 'relay',
          'https://fixture.invalid/v1', 'https://fixture.invalid', 'responses', 'api_key',
          'credential:provider1:1', 1, 'fixture-model', 'fixture-effort', 1, 'connection_probe',
          'fixture-v1', 'fixture-adapter-v1', 'owned_process', 1000, 1001);
        INSERT INTO tasks VALUES ('t1', 's1', 'fixture-turn', 'snap1', NULL, NULL, 'completed', NULL, 1000, 1001, 1002);
        INSERT INTO approvals VALUES ('a1', 't1', 'fixture-request', 'command', NULL, 'denied', 1001, 1002);
        INSERT INTO settings VALUES ('global-ai', 'global', NULL, 'ai_defaults', NULL, NULL, 'provider1', 'fixture-model', 'fixture-effort', 1, 1000);
        INSERT INTO settings VALUES ('project-ai', 'project', 'p1', 'ai_defaults', NULL, NULL, 'provider1', 'project-model', NULL, 1, 1000);
        INSERT INTO audit_events VALUES ('e1', 1002, 'fixture-correlation', 'approval_decision', 'denied', NULL, 'p1', 't1', 'a1', 'provider1', 'fixture-model', 'fixture-effort');
    "#).map_err(sql_error)?;
    transaction.commit().map_err(sql_error)
}

pub fn verify(connection: &mut Connection) -> Result<Vec<&'static str>, StorageError> {
    let mut checks = Vec::new();
    for table in [
        "projects",
        "project_usage",
        "sessions",
        "tasks",
        "approvals",
        "provider_profiles",
        "model_capabilities",
        "ai_config_snapshots",
        "audit_events",
    ] {
        let count: i64 = connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .map_err(sql_error)?;
        if count != 1 {
            return Err(error("STORAGE_PROBE_ROWS"));
        }
    }
    checks.push("ten_entities_persisted");
    let name: String = connection
        .query_row("SELECT name FROM projects WHERE id = 'p1'", [], |r| {
            r.get(0)
        })
        .map_err(sql_error)?;
    if name != "阶段 0 项目" {
        return Err(error("STORAGE_PROBE_UNICODE"));
    }
    checks.push("unicode_roundtrip");
    let default: String = connection.query_row("SELECT model_id FROM settings WHERE scope = 'project' AND project_id = 'p1' AND key = 'ai_defaults'", [], |r| r.get(0)).map_err(sql_error)?;
    if default != "project-model" {
        return Err(error("STORAGE_PROBE_DEFAULTS"));
    }
    let count: i64 = connection
        .query_row("SELECT count(*) FROM settings", [], |r| r.get(0))
        .map_err(sql_error)?;
    if count != 2 {
        return Err(error("STORAGE_PROBE_DEFAULTS"));
    }
    checks.push("global_project_defaults");
    if connection
        .execute(
            "UPDATE ai_config_snapshots SET model_id = 'changed' WHERE id = 'snap1'",
            [],
        )
        .is_ok()
    {
        return Err(error("STORAGE_PROBE_IMMUTABILITY"));
    }
    checks.push("snapshot_immutable");
    if connection
        .execute("UPDATE tasks SET snapshot_id = NULL WHERE id = 't1'", [])
        .is_ok()
    {
        return Err(error("STORAGE_PROBE_TASK_BINDING"));
    }
    checks.push("task_snapshot_fixed");
    if connection
        .execute("UPDATE project_usage SET project_id = 'missing'", [])
        .is_ok()
    {
        return Err(error("STORAGE_PROBE_FOREIGN_KEY"));
    }
    checks.push("foreign_keys_enforced");
    if connection
        .execute(
            "UPDATE provider_profiles SET credential_ref = 'synthetic-secret-must-be-rejected'",
            [],
        )
        .is_ok()
    {
        return Err(error("STORAGE_PROBE_CREDENTIAL"));
    }
    checks.push("credential_payload_rejected");
    {
        let transaction = connection.transaction().map_err(sql_error)?;
        transaction
            .execute("UPDATE project_usage SET open_count = 99", [])
            .map_err(sql_error)?;
        // Drop without commit must roll back all writes.
    }
    let count: i64 = connection
        .query_row("SELECT open_count FROM project_usage", [], |r| r.get(0))
        .map_err(sql_error)?;
    if count != 1 {
        return Err(error("STORAGE_PROBE_ROLLBACK"));
    }
    checks.push("transaction_rollback");
    let problems = connection
        .prepare("PRAGMA foreign_key_check")
        .map_err(sql_error)?
        .query([])
        .map_err(sql_error)?
        .next()
        .map_err(sql_error)?
        .is_some();
    if problems {
        return Err(error("STORAGE_PROBE_FOREIGN_KEY"));
    }
    checks.push("foreign_key_check_clean");
    Ok(checks)
}

pub fn run(data_dir: &Path, mode: &str) -> Result<Value, StorageError> {
    let mut storage = Storage::open(data_dir)?;
    if mode == "seed" {
        seed(&mut storage.connection)?;
    } else if mode != "verify" {
        return Err(error("STORAGE_PROBE_MODE"));
    }
    let checks = verify(&mut storage.connection)?;
    let sqlite_version: String = storage
        .connection
        .query_row("SELECT sqlite_version()", [], |r| r.get(0))
        .map_err(sql_error)?;
    let schema_version: i64 = storage
        .connection
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(sql_error)?;
    let journal: String = storage
        .connection
        .pragma_query_value(None, "journal_mode", |r| r.get(0))
        .map_err(sql_error)?;
    Ok(
        json!({"passed": true, "mode": mode, "sqliteVersion": sqlite_version,
        "schemaVersion": schema_version, "journalMode": journal, "checks": checks,
        "usesTauriAppDataDir": true, "syntheticDataOnly": true}),
    )
}

/// Debug executable only. The test owns a fresh child directory of app_data_dir;
/// normal startup never creates a database during phase 0.
#[cfg(debug_assertions)]
pub fn run_cli() -> Option<i32> {
    use tauri::Manager;
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some("--phase0-storage-probe") {
        return None;
    }
    let outcome = (|| -> Result<Value, StorageError> {
        let mode = args.get(2).ok_or_else(|| error("STORAGE_PROBE_ARGS"))?;
        let id = args.get(3).ok_or_else(|| error("STORAGE_PROBE_ARGS"))?;
        if args.len() != 4
            || !["seed", "verify", "cleanup"].contains(&mode.as_str())
            || !id.starts_with("probe-")
            || id.len() > 80
            || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(error("STORAGE_PROBE_ARGS"));
        }
        let mut context = tauri::generate_context!();
        context.config_mut().app.windows.clear();
        let app = tauri::Builder::default()
            .build(context)
            .map_err(|_| error("STORAGE_PROBE_TAURI"))?;
        let app_data = app
            .path()
            .app_data_dir()
            .map_err(|_| error("STORAGE_PROBE_APP_DATA"))?;
        let directory = app_data.join("phase-0-storage").join(id);
        if mode == "cleanup" {
            if directory.exists() {
                // MSIX can virtualize children of an existing app_data_dir while
                // leaving the parent unvirtualized. Anchor to this probe's dedicated
                // directory and require an exact direct child, never a broad prefix.
                let probe_root = app_data.join("phase-0-storage");
                let metadata = std::fs::symlink_metadata(&probe_root)
                    .map_err(|_| error("STORAGE_PROBE_CLEANUP_ROOT"))?;
                if metadata.file_type().is_symlink() {
                    return Err(error("STORAGE_PROBE_CLEANUP_BOUNDARY"));
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if metadata.file_attributes() & 0x400 != 0 {
                        return Err(error("STORAGE_PROBE_CLEANUP_BOUNDARY"));
                    }
                }
                let resolved_root = probe_root
                    .canonicalize()
                    .map_err(|_| error("STORAGE_PROBE_CLEANUP_ROOT"))?;
                let resolved = directory
                    .canonicalize()
                    .map_err(|_| error("STORAGE_PROBE_CLEANUP_TARGET"))?;
                if resolved.parent() != Some(resolved_root.as_path())
                    || resolved.file_name() != Some(std::ffi::OsStr::new(id))
                {
                    return Err(error("STORAGE_PROBE_CLEANUP_BOUNDARY"));
                }
                std::fs::remove_dir_all(&resolved)
                    .map_err(|_| error("STORAGE_PROBE_CLEANUP_DELETE"))?;
            }
            return Ok(json!({"passed": true, "mode": "cleanup", "usesTauriAppDataDir": true}));
        }
        if mode == "seed" && directory.exists() {
            return Err(error("STORAGE_PROBE_EXISTS"));
        }
        if mode == "verify" && !directory.join(super::DATABASE_NAME).is_file() {
            return Err(error("STORAGE_PROBE_MISSING"));
        }
        run(&directory, mode)
    })();
    Some(match outcome {
        Ok(report) => {
            println!("{report}");
            0
        }
        Err(failure) => {
            eprintln!("{}", failure.code);
            1
        }
    })
}
