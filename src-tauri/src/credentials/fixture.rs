//! Debug-only synthetic controls. No caller supplied profile, path or secret.
use super::*;
use sha2::{Digest, Sha256};

pub(crate) fn run(app: &tauri::AppHandle, action: &str) -> Result<(), ErrorCode> {
    let directory =
        std::env::var("PA_OVERLAY_VERIFY_DIR").map_err(|_| ErrorCode::PermissionDenied)?;
    if directory.is_empty() {
        return Err(ErrorCode::PermissionDenied);
    }
    let digest = format!("{:x}", Sha256::digest(directory.as_bytes()));
    let id = format!("desktop-credential-{}", &digest[..24]);
    let state = app.state::<CredentialsState>();
    // Finishing the task must not acquire the operation lock held by the waiter.
    if action == "credential_idle" || action == "credential_occupy" {
        return app.state::<storage::StorageState>().with_storage(|storage| {
            storage.connection.execute(
                "UPDATE tasks SET status = ?1, finished_at = ?2 WHERE id = 'credential-task'",
                rusqlite::params![if action == "credential_idle" { "completed" } else { "running" }, if action == "credential_idle" { Some(1002) } else { None }],
            ).map(|_| ()).map_err(|_| storage::StorageError { code: "STORAGE_SQL_FAILED", backup_path: None })
        }).map_err(map_storage_failure);
    }
    let _operation = state
        .operation
        .lock()
        .map_err(|_| ErrorCode::CredentialsStorageFailed)?;
    if action == "credential_cleanup" {
        for revision in 0..=8 {
            let target = Binding {
                profile_id: id.clone(),
                origin: "https://fixture.invalid".into(),
                credential_revision: revision,
                config_revision: 1,
                credential_ref: Some(logical_ref(&id, revision)),
                status: "missing".into(),
            };
            platform::delete(&target).map_err(|_| ErrorCode::CredentialsUnavailable)?;
            if platform::read(&target)
                .map_err(|_| ErrorCode::CredentialsUnavailable)?
                .is_some()
            {
                return Err(ErrorCode::CredentialsStorageFailed);
            }
        }
        return Ok(());
    }
    app.state::<storage::StorageState>().with_storage(|storage| {
        if action == "credential_seed" {
            storage::probe::seed(&mut storage.connection)?;
            storage.connection.execute(
                "INSERT INTO provider_profiles (id,name,source_kind,adapter_kind,endpoint,endpoint_origin,wire_api,auth_kind,credential_ref,credential_revision,enabled,config_revision,created_at,updated_at,credential_status) VALUES (?1,'合成凭据验收','relay','self_managed','https://fixture.invalid/v1','https://fixture.invalid','responses','api_key',?2,0,1,1,1000,1000,'missing')",
                rusqlite::params![id, logical_ref(&id, 0)],
            ).map_err(|_| storage::StorageError { code: "STORAGE_SQL_FAILED", backup_path: None })?;
            storage.connection.execute("INSERT INTO model_capabilities VALUES (?1,'synthetic','',NULL,1,'connection_probe','fixture',1,1000,2000)", [&id]).map_err(|_| storage::StorageError { code: "STORAGE_SQL_FAILED", backup_path: None })?;
            storage.connection.execute("INSERT INTO ai_config_snapshots SELECT 'credential-snap',?1,profile_name,source_kind,endpoint,endpoint_origin,wire_api,auth_kind,?2,0,model_id,effort,config_revision,capability_source,capability_version,adapter_version,routing_scope,effective_at,captured_at FROM ai_config_snapshots WHERE id='snap1'", rusqlite::params![id, logical_ref(&id, 0)]).map_err(|_| storage::StorageError { code: "STORAGE_SQL_FAILED", backup_path: None })?;
            storage.connection.execute_batch("INSERT INTO tasks SELECT 'credential-task',session_id,NULL,'credential-snap',NULL,NULL,'completed',NULL,1000,1001,1002 FROM tasks WHERE id='t1';").map_err(|_| storage::StorageError { code: "STORAGE_SQL_FAILED", backup_path: None })?;
            return Ok(Ok(()));
        }
        if action == "credential_assert" {
            let result = (|| {
                let current = read_binding(storage, &id)?;
                let secret = resolve_in_storage(storage, &id, current.credential_revision, "https://fixture.invalid", &SystemStore)?;
                if secret.as_bytes() != b"synthetic-credential-desktop-DO-NOT-EXPORT" { return Err(ErrorCode::CredentialsStorageFailed); }
                if storage.capabilities().count_valid(&id, 1001).map_err(map_storage_failure)? != 0 { return Err(ErrorCode::CredentialsStorageFailed); }
                Ok(())
            })();
            return Ok(result);
        }
        Ok(Err(ErrorCode::InvalidRequest))
    }).map_err(map_storage_failure)?
}
