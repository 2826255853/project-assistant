//! Application-owned Windows Credential Manager integration.
//! The SQLite database stores only the logical reference and state; secret bytes
//! never enter a command response, log, audit row, or ordinary database.
use crate::{ipc::ErrorCode, storage};
use serde::{Deserialize, Deserializer, Serialize};
use std::sync::{
    Mutex,
    atomic::{Ordering, compiler_fence},
};
use std::time::{Duration, Instant};
use tauri::Manager;
use url::Url;

const MAX_PROFILE_ID: usize = 128;
// Windows generic credential blobs have a small documented size ceiling; leave
// room for the versioned profile/revision/origin envelope.
const MAX_SECRET_BYTES: usize = 1800;
const IDLE_WAIT_LIMIT: Duration = Duration::from_secs(5 * 60);
#[cfg(all(test, windows))]
static OS_TEST_LOCK: Mutex<()> = Mutex::new(());

pub(crate) struct SecretBuffer(Vec<u8>);
impl SecretBuffer {
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}
impl Clone for SecretBuffer {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl Drop for SecretBuffer {
    fn drop(&mut self) {
        wipe(&mut self.0);
    }
}

fn wipe(bytes: &mut [u8]) {
    for byte in bytes {
        unsafe {
            std::ptr::write_volatile(byte, 0);
        }
    }
    compiler_fence(Ordering::SeqCst);
}

pub(crate) struct SecretString(String);
impl SecretString {
    fn as_str(&self) -> &str {
        &self.0
    }
}
impl<'de> Deserialize<'de> for SecretString {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self)
    }
}
impl Drop for SecretString {
    fn drop(&mut self) {
        // The owned String is uniquely held here; overwrite before deallocation.
        unsafe {
            wipe(self.0.as_bytes_mut());
        }
    }
}

pub struct CredentialsState {
    pub operation: Mutex<()>,
    available: Mutex<bool>,
}

impl Default for CredentialsState {
    fn default() -> Self {
        Self {
            operation: Mutex::new(()),
            available: Mutex::new(platform::available()),
        }
    }
}

impl CredentialsState {
    fn set_available(&self, available: bool) {
        if let Ok(mut state) = self.available.lock() {
            *state = available;
        }
    }

    pub fn health(&self) -> bool {
        self.available.lock().map(|value| *value).unwrap_or(false)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialState {
    pub status: &'static str,
    pub revision: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationResult {
    pub credential: CredentialState,
    pub config_revision: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PutRequest {
    pub profile_id: String,
    pub expected_revision: i64,
    pub(crate) secret: SecretString,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoveRequest {
    pub profile_id: String,
    pub expected_revision: i64,
}

#[derive(Debug, Clone)]
struct Binding {
    profile_id: String,
    origin: String,
    credential_revision: i64,
    config_revision: i64,
    credential_ref: Option<String>,
    status: String,
}

fn valid_profile_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_PROFILE_ID
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The profile table already stores the origin, but normalize and validate it
/// again at the credential boundary so a malformed historical row cannot bind
/// a secret to an ambiguous site.
fn normalize_origin(value: &str) -> Option<String> {
    if value.len() > 520 {
        return None;
    }
    let parsed = Url::parse(value).ok()?;
    if parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return None;
    }
    Some(parsed.origin().ascii_serialization())
}

fn binding(profile: storage::CredentialProfile) -> Result<Binding, ErrorCode> {
    if !valid_profile_id(&profile.id)
        || profile.adapter_kind != "self_managed"
        || profile.auth_kind != "api_key"
        || !(0..=9007199254740991).contains(&profile.credential_revision)
        || !(1..=9007199254740991).contains(&profile.config_revision)
        || !matches!(
            profile.credential_status.as_str(),
            "missing" | "configured_unverified" | "verified" | "invalid" | "external_unavailable"
        )
    {
        return Err(ErrorCode::CredentialInvalid);
    }
    let origin = normalize_origin(&profile.endpoint_origin).ok_or(ErrorCode::CredentialInvalid)?;
    let expected_ref = format!("credential:{}:{}", profile.id, profile.credential_revision);
    if profile.credential_ref.as_deref() != Some(expected_ref.as_str()) {
        return Err(ErrorCode::CredentialInvalid);
    }
    Ok(Binding {
        profile_id: profile.id,
        origin,
        credential_revision: profile.credential_revision,
        config_revision: profile.config_revision,
        credential_ref: profile.credential_ref,
        status: profile.credential_status,
    })
}

fn read_binding(storage: &mut storage::Storage, profile_id: &str) -> Result<Binding, ErrorCode> {
    if !valid_profile_id(profile_id) {
        return Err(ErrorCode::CredentialInvalid);
    }
    storage
        .profiles()
        .credential_profile(profile_id)
        .map_err(map_storage_failure)
        .and_then(|profile| profile.ok_or(ErrorCode::CredentialProfileNotFound))
        .and_then(binding)
}

/// Resolve a secret only for an owned process that already knows the selected
/// profile, credential revision, and normalized HTTPS origin. This is not IPC.
#[allow(dead_code)] // Used by the owned process service introduced in step 1.7.
pub(crate) fn resolve(
    app: &tauri::AppHandle,
    profile_id: &str,
    credential_revision: i64,
    expected_origin: &str,
) -> Result<SecretBuffer, ErrorCode> {
    let state = app.state::<CredentialsState>();
    let _operation = state
        .operation
        .lock()
        .map_err(|_| ErrorCode::CredentialsStorageFailed)?;
    resolve_for_owned_locked(app, profile_id, credential_revision, expected_origin)
}

/// Caller holds CredentialsState.operation for the entire owned candidate.
pub(crate) fn resolve_for_owned_locked(
    app: &tauri::AppHandle,
    profile_id: &str,
    credential_revision: i64,
    expected_origin: &str,
) -> Result<SecretBuffer, ErrorCode> {
    let state = app.state::<CredentialsState>();
    let result = app
        .state::<storage::StorageState>()
        .with_storage(|storage| {
            Ok(resolve_in_storage(
                storage,
                profile_id,
                credential_revision,
                expected_origin,
                &SystemStore,
            ))
        })
        .map_err(map_storage_failure)?;
    match result {
        Ok(_) => state.set_available(true),
        Err(ErrorCode::CredentialsUnavailable | ErrorCode::CredentialsStorageFailed) => {
            state.set_available(false)
        }
        _ => {}
    }
    result
}

fn resolve_in_storage(
    storage: &mut storage::Storage,
    profile_id: &str,
    credential_revision: i64,
    expected_origin: &str,
    store: &impl CredentialStore,
) -> Result<SecretBuffer, ErrorCode> {
    let current = read_binding(storage, profile_id)?;
    if current.status != "configured_unverified" && current.status != "verified" {
        return Err(ErrorCode::CredentialsUnavailable);
    }
    if current.credential_revision != credential_revision
        || normalize_origin(expected_origin).as_deref() != Some(current.origin.as_str())
    {
        return Err(ErrorCode::CredentialInvalid);
    }
    let blob = match store.read(&current) {
        Ok(Some(blob)) => blob,
        Ok(None) | Err(_) => {
            return Err(ErrorCode::CredentialsUnavailable);
        }
    };
    platform::secret(&blob, &current).map_err(|_| ErrorCode::CredentialsUnavailable)
}

fn map_storage_failure(failure: storage::StorageError) -> ErrorCode {
    match failure.code {
        "STORAGE_CREDENTIAL_PROFILE_NOT_FOUND" => ErrorCode::CredentialProfileNotFound,
        "STORAGE_CREDENTIAL_REVISION_CONFLICT" => ErrorCode::CredentialRevisionConflict,
        "STORAGE_CREDENTIAL_BUSY" => ErrorCode::CredentialBusy,
        "STORAGE_CREDENTIAL_INVALID" => ErrorCode::CredentialInvalid,
        "STORAGE_CREDENTIAL_BACKEND_FAILED" => ErrorCode::CredentialsUnavailable,
        "STORAGE_CREDENTIAL_ROLLBACK_FAILED" => ErrorCode::CredentialsStorageFailed,
        "STORAGE_AUDIT_WRITE_FAILED" => ErrorCode::AuditWriteFailed,
        _ => ErrorCode::CredentialsStorageFailed,
    }
}

fn platform_storage_failure(failure: platform::Failure) -> storage::StorageError {
    let code = match failure {
        platform::Failure::Invalid => "STORAGE_CREDENTIAL_INVALID",
        platform::Failure::Unavailable => "STORAGE_CREDENTIAL_BACKEND_FAILED",
    };
    storage::StorageError {
        code,
        backup_path: None,
    }
}

fn wait_for_idle(app: &tauri::AppHandle, profile_id: &str) -> Result<(), ErrorCode> {
    let started = Instant::now();
    loop {
        let active = app
            .state::<storage::StorageState>()
            .with_storage(|storage| storage.profiles().has_active_credential_task(profile_id))
            .map_err(|_| ErrorCode::CredentialsStorageFailed)?;
        if !active {
            return Ok(());
        }
        if started.elapsed() >= IDLE_WAIT_LIMIT {
            return Err(ErrorCode::CredentialBusy);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn next_binding(current: &Binding, status: &str) -> Result<Binding, ErrorCode> {
    let revision = current
        .credential_revision
        .checked_add(1)
        .ok_or(ErrorCode::CredentialInvalid)?;
    Ok(Binding {
        credential_revision: revision,
        credential_ref: Some(logical_ref(&current.profile_id, revision)),
        status: status.to_owned(),
        ..current.clone()
    })
}

fn logical_ref(profile_id: &str, revision: i64) -> String {
    format!("credential:{profile_id}:{revision}")
}

/// Acceptance-only bridge: exercise the production SQLite/OS-store resolver
/// using a unique synthetic HTTPS binding, then remove its exact vault target.
/// Only cfg(test) permits the loopback HTTPS origin and temporary test CA.
#[cfg(all(test, windows))]
pub(crate) fn isolation_fixture_secret(
    profile_id: &str,
    endpoint: &str,
    secret: &str,
) -> SecretBuffer {
    let _lock = OS_TEST_LOCK.lock().unwrap();
    assert!(profile_id.starts_with("isolation-") && valid_profile_id(profile_id));
    let mut url = Url::parse(endpoint).unwrap();
    url.set_scheme("https").unwrap();
    let origin = url.origin().ascii_serialization();
    let directory = std::env::temp_dir().join(format!("pa-isolation-credential-{profile_id}"));
    let mut database = storage::Storage::open(&directory).unwrap();
    database.connection.execute(
        "INSERT INTO provider_profiles (id,name,source_kind,adapter_kind,endpoint,endpoint_origin,wire_api,auth_kind,credential_ref,credential_revision,enabled,config_revision,created_at,updated_at,credential_status) VALUES (?1,'Synthetic isolation','relay','self_managed',?2,?3,'responses','api_key',?4,0,1,1,1000,1000,'missing')",
        rusqlite::params![profile_id, url.as_str(), origin, logical_ref(profile_id, 0)],
    ).unwrap();
    struct Cleanup(Binding, std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = platform::delete(&self.0);
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }
    let current = read_binding(&mut database, profile_id).unwrap();
    let next = next_binding(&current, "configured_unverified").unwrap();
    let cleanup = Cleanup(next.clone(), directory.clone());
    mutate_in_storage(
        &mut database,
        profile_id,
        1,
        Some(secret),
        &crate::ipc::correlation_id(),
        &SystemStore,
    )
    .unwrap();
    assert!(
        resolve_in_storage(
            &mut database,
            profile_id,
            next.credential_revision,
            "https://wrong.invalid",
            &SystemStore
        )
        .is_err()
    );
    let value = resolve_in_storage(
        &mut database,
        profile_id,
        next.credential_revision,
        &origin,
        &SystemStore,
    )
    .unwrap();
    assert!(
        value.as_bytes() == secret.as_bytes(),
        "synthetic resolution mismatch"
    );
    platform::delete(&next).unwrap();
    assert!(platform::read(&next).unwrap().is_none());
    drop(database);
    std::fs::remove_dir_all(directory).unwrap();
    drop(cleanup);
    value
}

trait CredentialStore {
    fn read(&self, binding: &Binding) -> Result<Option<SecretBuffer>, platform::Failure>;
    fn write(&self, binding: &Binding, secret: &str) -> Result<(), platform::Failure>;
    fn write_blob(&self, binding: &Binding, blob: SecretBuffer) -> Result<(), platform::Failure>;
    fn delete(&self, binding: &Binding) -> Result<(), platform::Failure>;
}

struct SystemStore;
impl CredentialStore for SystemStore {
    fn read(&self, binding: &Binding) -> Result<Option<SecretBuffer>, platform::Failure> {
        platform::read(binding)
    }
    fn write(&self, binding: &Binding, secret: &str) -> Result<(), platform::Failure> {
        platform::write(binding, secret)
    }
    fn write_blob(&self, binding: &Binding, blob: SecretBuffer) -> Result<(), platform::Failure> {
        platform::write_blob(binding, blob)
    }
    fn delete(&self, binding: &Binding) -> Result<(), platform::Failure> {
        platform::delete(binding)
    }
}

#[cfg(windows)]
mod platform {
    use super::{Binding, SecretBuffer};
    use std::{ffi::c_void, ptr, slice};
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::Security::Credentials::{
        CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree,
        CredReadW, CredWriteW,
    };

    const ERROR_NOT_FOUND: u32 = 1168;
    const HEADER: &[u8] = b"PA-CREDENTIAL-V1\0";

    #[derive(Debug)]
    pub enum Failure {
        Unavailable,
        Invalid,
    }

    fn target(binding: &Binding) -> Vec<u16> {
        // Windows target names are case insensitive; encode the case sensitive
        // SQLite profile ID so "Profile" and "profile" cannot share a secret.
        let profile: String = binding
            .profile_id
            .bytes()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        format!(
            "ProjectAssistant/Credential/{}/{}",
            profile, binding.credential_revision
        )
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect()
    }

    fn field(out: &mut Vec<u8>, value: &[u8]) {
        out.extend_from_slice(&(value.len() as u32).to_le_bytes());
        out.extend_from_slice(value);
    }

    fn payload(binding: &Binding, secret: &str) -> Result<SecretBuffer, Failure> {
        if secret.is_empty() || secret.len() > super::MAX_SECRET_BYTES || secret.contains('\0') {
            return Err(Failure::Invalid);
        }
        let mut out = Vec::with_capacity(
            HEADER.len() + binding.profile_id.len() + binding.origin.len() + secret.len() + 32,
        );
        out.extend_from_slice(HEADER);
        field(&mut out, binding.profile_id.as_bytes());
        field(&mut out, binding.credential_revision.to_string().as_bytes());
        field(&mut out, binding.origin.as_bytes());
        field(&mut out, secret.as_bytes());
        if out.len() > 2560 {
            super::wipe(&mut out);
            return Err(Failure::Invalid);
        }
        Ok(SecretBuffer::new(out))
    }

    fn secret_range(blob: &[u8], binding: &Binding) -> Result<std::ops::Range<usize>, Failure> {
        if blob.len() > 64 * 1024 || !blob.starts_with(HEADER) {
            return Err(Failure::Invalid);
        }
        let mut cursor = HEADER.len();
        let next = |cursor: &mut usize| -> Result<&[u8], Failure> {
            let end = cursor.checked_add(4).ok_or(Failure::Invalid)?;
            if end > blob.len() {
                return Err(Failure::Invalid);
            }
            let len = u32::from_le_bytes(
                blob[*cursor..end]
                    .try_into()
                    .map_err(|_| Failure::Invalid)?,
            ) as usize;
            *cursor = end;
            let end = cursor.checked_add(len).ok_or(Failure::Invalid)?;
            if end > blob.len() {
                return Err(Failure::Invalid);
            }
            let value = &blob[*cursor..end];
            *cursor = end;
            Ok(value)
        };
        let profile = next(&mut cursor).map_err(|_| Failure::Invalid)?;
        let revision = next(&mut cursor).map_err(|_| Failure::Invalid)?;
        let origin = next(&mut cursor).map_err(|_| Failure::Invalid)?;
        let secret_start = cursor.checked_add(4).ok_or(Failure::Invalid)?;
        let secret = next(&mut cursor).map_err(|_| Failure::Invalid)?;
        if cursor != blob.len()
            || profile != binding.profile_id.as_bytes()
            || revision != binding.credential_revision.to_string().as_bytes()
            || origin != binding.origin.as_bytes()
            || secret.is_empty()
            || secret.len() > super::MAX_SECRET_BYTES
            || secret.contains(&0)
            || std::str::from_utf8(secret).is_err()
        {
            return Err(Failure::Invalid);
        }
        Ok(secret_start..cursor)
    }

    pub fn read(binding: &Binding) -> Result<Option<SecretBuffer>, Failure> {
        let target = target(binding);
        let mut raw: *mut CREDENTIALW = ptr::null_mut();
        let ok = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut raw) } != 0;
        if !ok {
            return if unsafe { GetLastError() } == ERROR_NOT_FOUND {
                Ok(None)
            } else {
                Err(Failure::Unavailable)
            };
        }
        let result = unsafe {
            if raw.is_null() || (*raw).CredentialBlob.is_null() {
                Err(Failure::Invalid)
            } else {
                let mut blob = slice::from_raw_parts(
                    (*raw).CredentialBlob,
                    (*raw).CredentialBlobSize as usize,
                )
                .to_vec();
                let result =
                    secret_range(&blob, binding).map(|_| Some(SecretBuffer::new(blob.clone())));
                super::wipe(&mut blob);
                result
            }
        };
        unsafe {
            if !raw.is_null() && !(*raw).CredentialBlob.is_null() {
                let blob = slice::from_raw_parts_mut(
                    (*raw).CredentialBlob,
                    (*raw).CredentialBlobSize as usize,
                );
                for byte in blob {
                    ptr::write_volatile(byte, 0);
                }
            }
        }
        unsafe {
            if !raw.is_null() {
                CredFree(raw as *const c_void);
            }
        }
        result
    }

    pub fn write_blob(binding: &Binding, mut blob: SecretBuffer) -> Result<(), Failure> {
        let mut target = target(binding);
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: target.as_mut_ptr(),
            CredentialBlobSize: blob.0.len() as u32,
            CredentialBlob: blob.0.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            ..Default::default()
        };
        if unsafe { CredWriteW(&credential, 0) } == 0 {
            Err(Failure::Unavailable)
        } else {
            Ok(())
        }
    }

    pub fn write(binding: &Binding, secret: &str) -> Result<(), Failure> {
        write_blob(binding, payload(binding, secret)?)
    }

    pub fn secret(blob: &SecretBuffer, binding: &Binding) -> Result<SecretBuffer, Failure> {
        let range = secret_range(blob.as_bytes(), binding)?;
        Ok(SecretBuffer::new(blob.as_bytes()[range].to_vec()))
    }

    pub fn available() -> bool {
        use std::ptr;
        let target: Vec<u16> = "ProjectAssistant/Credential/__availability_probe__/0"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut raw: *mut CREDENTIALW = ptr::null_mut();
        let ok = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut raw) } != 0;
        if ok {
            if !raw.is_null() {
                unsafe {
                    CredFree(raw as *const c_void);
                }
            }
            true
        } else {
            (unsafe { GetLastError() }) == ERROR_NOT_FOUND
        }
    }

    pub fn delete(binding: &Binding) -> Result<(), Failure> {
        let target = target(binding);
        let ok = unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } != 0;
        if ok || unsafe { GetLastError() } == ERROR_NOT_FOUND {
            Ok(())
        } else {
            Err(Failure::Unavailable)
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{Binding, SecretBuffer};
    #[derive(Debug)]
    pub enum Failure {
        Unavailable,
        Invalid,
    }
    pub fn read(_: &Binding) -> Result<Option<SecretBuffer>, Failure> {
        Err(Failure::Unavailable)
    }
    pub fn write_blob(_: &Binding, _: SecretBuffer) -> Result<(), Failure> {
        Err(Failure::Unavailable)
    }
    pub fn write(_: &Binding, _: &str) -> Result<(), Failure> {
        Err(Failure::Unavailable)
    }
    pub fn delete(_: &Binding) -> Result<(), Failure> {
        Err(Failure::Unavailable)
    }
    pub fn secret(_: &SecretBuffer, _: &Binding) -> Result<SecretBuffer, Failure> {
        Err(Failure::Unavailable)
    }
    pub fn available() -> bool {
        false
    }
}

fn profile_state(status: &str, revision: i64) -> CredentialState {
    CredentialState {
        status: match status {
            "missing" => "missing",
            "configured_unverified" => "configured_unverified",
            "verified" => "verified",
            "invalid" => "invalid",
            "external_unavailable" => "external_unavailable",
            _ => "invalid",
        },
        revision,
    }
}

// Shared by the IPC service and isolated acceptance processes. The injected
// store is private to Rust; production always supplies SystemStore.
fn mutate_in_storage(
    storage: &mut storage::Storage,
    profile_id: &str,
    expected_revision: i64,
    secret: Option<&str>,
    correlation_id: &str,
    store: &impl CredentialStore,
) -> Result<MutationResult, ErrorCode> {
    if secret.is_some_and(|value| {
        value.is_empty() || value.len() > MAX_SECRET_BYTES || value.contains('\0')
    }) {
        return Err(ErrorCode::CredentialInvalid);
    }
    let current = read_binding(storage, profile_id)?;
    if current.config_revision != expected_revision {
        return Err(ErrorCode::CredentialRevisionConflict);
    }
    if secret.is_none() && current.status == "missing" {
        return Ok(MutationResult {
            credential: profile_state("missing", current.credential_revision),
            config_revision: current.config_revision,
        });
    }
    let status = if secret.is_some() {
        "configured_unverified"
    } else {
        "missing"
    };
    let next = next_binding(&current, status)?;
    let result = storage.profiles().commit_credential_change(
        profile_id,
        expected_revision,
        next.credential_revision,
        next.credential_ref
            .as_deref()
            .ok_or(ErrorCode::CredentialInvalid)?,
        status,
        correlation_id,
        |old| {
            let old_binding = binding(old.clone()).map_err(|_| storage::StorageError {
                code: "STORAGE_CREDENTIAL_INVALID",
                backup_path: None,
            })?;
            let old_blob = store.read(&old_binding).map_err(platform_storage_failure)?;
            // A missing external entry is repairable by an explicit put/remove.
            // A malformed or incorrectly bound entry is rejected by read above.
            if let Some(secret) = secret {
                // Version targets are immutable. A leftover from an interrupted
                // operation is not overwritten or mistaken for a committed Key.
                if store
                    .read(&next)
                    .map_err(platform_storage_failure)?
                    .is_some()
                {
                    return Err(storage::StorageError {
                        code: "STORAGE_CREDENTIAL_INVALID",
                        backup_path: None,
                    });
                }
                if let Err(failure) = store.write(&next, secret) {
                    return Err(platform_storage_failure(failure));
                }
            }
            if old_blob.is_some() {
                if let Err(failure) = store.delete(&old_binding) {
                    return Err(if secret.is_none() || store.delete(&next).is_ok() {
                        platform_storage_failure(failure)
                    } else {
                        storage::StorageError {
                            code: "STORAGE_CREDENTIAL_ROLLBACK_FAILED",
                            backup_path: None,
                        }
                    });
                }
            }
            Ok((old_binding, old_blob))
        },
        |(old_binding, old_blob)| {
            // Restore the usable reference before removing the uncommitted one.
            let restored = old_blob
                .map(|blob| store.write_blob(&old_binding, blob).is_ok())
                .unwrap_or(true);
            let removed = secret.is_none() || store.delete(&next).is_ok();
            restored && removed
        },
        |_| {},
    );
    let committed = match result {
        Ok(value) => value,
        Err(failure) => {
            if failure.code == "STORAGE_CREDENTIAL_ROLLBACK_FAILED" {
                // Persist a fail-closed state when compensation cannot restore
                // consistency, including across a restart. If SQLite itself is
                // unavailable, resolve still validates the actual OS payload.
                let _ = storage.connection.execute(
                    "UPDATE provider_profiles SET credential_status = 'external_unavailable' WHERE id = ?1 AND config_revision = ?2",
                    rusqlite::params![profile_id, expected_revision],
                );
            }
            return Err(map_storage_failure(failure));
        }
    };
    Ok(MutationResult {
        credential: profile_state(status, committed.credential_revision),
        config_revision: committed.config_revision,
    })
}

fn mutate(
    app: &tauri::AppHandle,
    profile_id: &str,
    expected_revision: i64,
    secret: Option<&str>,
    correlation_id: &str,
) -> Result<MutationResult, ErrorCode> {
    if !valid_profile_id(profile_id) {
        return Err(ErrorCode::CredentialInvalid);
    }
    let state = app.state::<CredentialsState>();
    let operation = state
        .operation
        .lock()
        .map_err(|_| ErrorCode::CredentialsStorageFailed)?;
    if secret.is_some_and(|value| {
        value.is_empty() || value.len() > MAX_SECRET_BYTES || value.contains('\0')
    }) {
        return Err(ErrorCode::CredentialInvalid);
    }
    app.state::<storage::StorageState>()
        .with_storage(|storage| {
            Ok(read_binding(storage, profile_id).and_then(|current| {
                if current.config_revision == expected_revision {
                    Ok(())
                } else {
                    Err(ErrorCode::CredentialRevisionConflict)
                }
            }))
        })
        .map_err(map_storage_failure)??;
    // Do not hold the database lock while waiting: tasks must be able to finish.
    wait_for_idle(app, profile_id)?;
    let result = app
        .state::<storage::StorageState>()
        .with_storage(|storage| {
            Ok(mutate_in_storage(
                storage,
                profile_id,
                expected_revision,
                secret,
                correlation_id,
                &SystemStore,
            ))
        })
        .map_err(map_storage_failure)?;
    if result.is_ok() {
        state.set_available(true);
    }
    if matches!(
        result,
        Err(ErrorCode::CredentialsUnavailable | ErrorCode::CredentialsStorageFailed)
    ) {
        state.set_available(false);
    }
    drop(operation);
    crate::app_state::publish(app, correlation_id.to_owned());
    result
}

pub fn put(
    app: &tauri::AppHandle,
    request: PutRequest,
    correlation_id: &str,
) -> Result<MutationResult, ErrorCode> {
    mutate(
        app,
        &request.profile_id,
        request.expected_revision,
        Some(request.secret.as_str()),
        correlation_id,
    )
}

pub fn remove(
    app: &tauri::AppHandle,
    request: RemoveRequest,
    correlation_id: &str,
) -> Result<MutationResult, ErrorCode> {
    mutate(
        app,
        &request.profile_id,
        request.expected_revision,
        None,
        correlation_id,
    )
}

#[cfg(all(test, windows))]
mod acceptance_tests;

#[cfg(debug_assertions)]
mod fixture;
#[cfg(debug_assertions)]
pub(crate) use fixture::run as verification_fixture;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_normalization_rejects_ambiguous_endpoints() {
        assert_eq!(
            normalize_origin("https://EXAMPLE.invalid:443/"),
            Some("https://example.invalid".into())
        );
        for origin in [
            "http://example.invalid/",
            "https://user@example.invalid/",
            "https://example.invalid/v1",
            "https://example.invalid/?token=x",
            "https://example.invalid/#fragment",
        ] {
            assert_eq!(normalize_origin(origin), None, "{origin}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_credential_manager_round_trips_and_cleans_synthetic_versions() {
        let _guard = OS_TEST_LOCK.lock().unwrap();
        struct Cleanup(Vec<Binding>);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                for binding in &self.0 {
                    let _ = platform::delete(binding);
                }
            }
        }

        let profile_id = format!("test-{}", crate::ipc::correlation_id());
        let mut cleanup = Cleanup(Vec::new());
        let first = Binding {
            profile_id: profile_id.clone(),
            origin: "https://fixture.invalid".into(),
            credential_revision: 1,
            config_revision: 1,
            credential_ref: Some(logical_ref(&profile_id, 1)),
            status: "configured_unverified".into(),
        };
        cleanup.0.push(first.clone());
        platform::write(&first, "synthetic-credential-one").unwrap();
        let stored = platform::read(&first).unwrap().unwrap();
        assert_eq!(
            platform::secret(&stored, &first).unwrap().as_bytes(),
            b"synthetic-credential-one"
        );

        let second = Binding {
            credential_revision: 2,
            credential_ref: Some(logical_ref(&profile_id, 2)),
            ..first.clone()
        };
        cleanup.0.push(second.clone());
        platform::write(&second, "synthetic-credential-two").unwrap();
        platform::delete(&first).unwrap();
        assert!(platform::read(&first).unwrap().is_none());
        let stored = platform::read(&second).unwrap().unwrap();
        assert_eq!(
            platform::secret(&stored, &second).unwrap().as_bytes(),
            b"synthetic-credential-two"
        );
        platform::delete(&second).unwrap();
        assert!(platform::read(&second).unwrap().is_none());
    }
}
