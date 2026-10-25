//! S1-07 application-owned profiles, validated selections and immutable snapshots.
mod operations;
mod repository;
pub(crate) mod snapshots;
#[cfg(test)]
mod tests;
mod types;
pub(crate) use operations::*;
pub(crate) use repository::{delete, list, models, save, set_enabled};
pub(crate) use types::*;

use crate::{ipc::ErrorCode, storage};
use rusqlite::Connection;
use tauri::Manager;
pub(crate) const MAX_REVISION: i64 = 9_007_199_254_740_991;
pub(crate) const CAPABILITY_TTL: i64 = 24 * 60 * 60 * 1000;
pub(crate) const TEST_TTL: i64 = 5 * 60 * 1000;
pub fn config_state(app: &tauri::AppHandle) -> Result<String, ErrorCode> {
    if !app
        .state::<ProvidersState>()
        .initialized
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(ErrorCode::SettingsStorageFailed);
    }
    with_db(app, |c| repository::active(c, now()?).map(|v| v.1))
}
pub fn initialize(app: &tauri::AppHandle) -> Result<(), ErrorCode> {
    let result = app
        .state::<storage::StorageState>()
        .with_storage(|db| Ok(operations::recover(&mut db.connection)))
        .map_err(|_| ErrorCode::SettingsStorageFailed)
        .and_then(|v| v);
    app.state::<ProvidersState>()
        .initialized
        .store(result.is_ok(), std::sync::atomic::Ordering::Release);
    result
}
pub(crate) fn sql(_: rusqlite::Error) -> ErrorCode {
    ErrorCode::SettingsStorageFailed
}
pub(crate) fn now() -> Result<i64, ErrorCode> {
    storage::audit_time().map_err(|_| ErrorCode::SettingsStorageFailed)
}
pub(crate) fn with_db<T>(
    app: &tauri::AppHandle,
    f: impl FnOnce(&mut Connection) -> Result<T, ErrorCode>,
) -> Result<T, ErrorCode> {
    if !app
        .state::<ProvidersState>()
        .initialized
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(ErrorCode::SettingsStorageFailed);
    }
    app.state::<storage::StorageState>()
        .with_storage(|db| Ok(f(&mut db.connection)))
        .map_err(|_| ErrorCode::SettingsStorageFailed)?
}
pub(crate) fn uuid() -> Result<String, ErrorCode> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|_| ErrorCode::InternalUnexpected)?;
    b[6] = (b[6] & 15) | 64;
    b[8] = (b[8] & 63) | 128;
    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0],
        b[1],
        b[2],
        b[3],
        b[4],
        b[5],
        b[6],
        b[7],
        b[8],
        b[9],
        b[10],
        b[11],
        b[12],
        b[13],
        b[14],
        b[15]
    ))
}
