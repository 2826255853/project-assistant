//! Business storage lifecycle, schema migrations, typed repositories and restart recovery.
use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use sha2::{Digest, Sha256};
use std::{
    fmt, fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub mod probe;
#[cfg(windows)]
pub mod projects;
mod repositories;
pub(crate) mod settings;
pub use repositories::{
    AUDIT_RETENTION_MS, AuditEventInput, AuditRepository, CapabilityRepository, CredentialProfile,
    ProfileRepository, SettingRecord, SettingsRepository, SnapshotRepository,
};

pub const DATABASE_NAME: &str = "project-assistant.sqlite3";
pub const SCHEMA_VERSION: i64 = 5;

struct Migration {
    version: i64,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    // Append only; released SQL/checksums below remain unchanged.
    Migration {
        version: 1,
        name: "initial",
        sql: include_str!("../../migrations/0001_initial.sql"),
    },
    Migration {
        version: 2,
        name: "settings",
        sql: include_str!("../../migrations/0002_settings.sql"),
    },
    Migration {
        version: 3,
        name: "credentials",
        sql: include_str!("../../migrations/0003_credentials.sql"),
    },
    Migration {
        version: 4,
        name: "providers",
        sql: include_str!("../../migrations/0004_providers.sql"),
    },
    Migration {
        version: 5,
        name: "projects",
        sql: include_str!("../../migrations/0005_projects.sql"),
    },
];

pub(crate) fn audit_time() -> Result<i64, StorageError> {
    repositories::audit_now_ms()
}
pub(crate) fn provider_audit(
    c: &Connection,
    id: &str,
    kind: &str,
    profile: Option<&str>,
    outcome: &str,
) -> Result<(), StorageError> {
    let at = repositories::audit_now_ms()?;
    repositories::prune_expired_in_transaction(c, at)?;
    repositories::record_in_transaction(
        c,
        AuditEventInput {
            id: &repositories::new_audit_id(),
            correlation_id: id,
            event_kind: kind,
            outcome,
            error_code: None,
            profile_id: profile,
        },
        at,
    )
}

#[derive(Debug)]
pub struct StorageError {
    pub code: &'static str,
    /// Only a local recovery reference, never SQLite's raw error/query/value.
    pub backup_path: Option<PathBuf>,
}

impl StorageError {
    pub fn public_code(&self) -> &'static str {
        match self.code {
            "STORAGE_BUSY" => "PA_STORAGE_BUSY",
            "STORAGE_DIRECTORY_FAILED" => "PA_STORAGE_DIRECTORY_FAILED",
            "STORAGE_OPEN_FAILED" => "PA_STORAGE_OPEN_FAILED",
            "STORAGE_SQL_FAILED" => "PA_STORAGE_SQL_FAILED",
            "STORAGE_CLOCK_FAILED" => "PA_STORAGE_CLOCK_FAILED",
            "STORAGE_AUDIT_WRITE_FAILED" | "STORAGE_AUDIT_INVALID_INPUT" => "PA_AUDIT_WRITE_FAILED",
            "STORAGE_CREDENTIAL_ROLLBACK_FAILED" => "PA_CREDENTIALS_STORAGE_FAILED",
            "STORAGE_BACKUP_FAILED" => "PA_STORAGE_BACKUP_FAILED",
            "STORAGE_MIGRATION_FAILED" => "PA_STORAGE_MIGRATION_FAILED",
            "STORAGE_NEWER_SCHEMA" => "PA_STORAGE_NEWER_SCHEMA",
            "STORAGE_CORRUPT" => "PA_STORAGE_CORRUPT",
            "STORAGE_UNRECOGNIZED_SCHEMA" => "PA_STORAGE_UNRECOGNIZED_SCHEMA",
            "STORAGE_MIGRATION_HISTORY" => "PA_STORAGE_MIGRATION_HISTORY",
            "STORAGE_MIGRATION_SEQUENCE" => "PA_STORAGE_MIGRATION_SEQUENCE",
            "STORAGE_FOREIGN_KEY_CHECK" => "PA_STORAGE_FOREIGN_KEY_CHECK",
            _ => "PA_STORAGE_UNEXPECTED",
        }
    }
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code)
    }
}
impl std::error::Error for StorageError {}

fn error(code: &'static str) -> StorageError {
    StorageError {
        code,
        backup_path: None,
    }
}
fn database_error(failure: rusqlite::Error, fallback: &'static str) -> StorageError {
    use rusqlite::ErrorCode;
    let code = match failure.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => "STORAGE_BUSY",
        Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) => "STORAGE_CORRUPT",
        _ => fallback,
    };
    error(code)
}
fn sql_error(failure: rusqlite::Error) -> StorageError {
    database_error(failure, "STORAGE_SQL_FAILED")
}
fn checksum(sql: &str) -> String {
    format!("{:x}", Sha256::digest(sql.replace("\r\n", "\n").as_bytes()))
}

pub struct Storage {
    pub connection: Connection,
    pub path: PathBuf,
}

impl Storage {
    /// Caller supplies Tauri's app_data_dir (or an owned test subdirectory).
    pub fn open(data_dir: &Path) -> Result<Self, StorageError> {
        Self::open_with(data_dir, MIGRATIONS)
    }

    /// Mark all local work that could have been active when this process stopped.
    /// The update and its minimal audit fact are one transaction; no external work is replayed.
    pub fn recover_on_startup(
        &mut self,
        correlation_id: &str,
    ) -> Result<RecoverySummary, StorageError> {
        repositories::validate_correlation_id(correlation_id)?;
        let transaction = self.connection.transaction().map_err(sql_error)?;
        let finished_at = repositories::audit_now_ms()?;
        let tasks = transaction
            .execute(
                "UPDATE tasks SET status = 'interrupted', error_code = 'PA_STORAGE_RESTART_INTERRUPTED', finished_at = ?1 WHERE status IN ('idle', 'preparing', 'running', 'approval_required', 'cancelling')",
                [finished_at],
            )
            .map_err(sql_error)?;
        let approvals = transaction
            .execute(
                "UPDATE approvals SET decision = 'interrupted', decided_at = ?1 WHERE decision = 'pending'",
                [finished_at],
            )
            .map_err(sql_error)?;
        if tasks > 0 || approvals > 0 {
            repositories::record_in_transaction(
                &transaction,
                AuditEventInput {
                    id: &repositories::new_audit_id(),
                    correlation_id,
                    event_kind: "storage_recovery",
                    outcome: "interrupted",
                    error_code: Some("PA_STORAGE_RESTART_INTERRUPTED"),
                    profile_id: None,
                },
                finished_at,
            )?;
        }
        transaction
            .commit()
            .map_err(repositories::audit_write_error)?;
        Ok(RecoverySummary { tasks, approvals })
    }

    pub fn settings(&mut self) -> SettingsRepository<'_> {
        SettingsRepository::new(&mut self.connection)
    }

    #[cfg(windows)]
    pub fn projects(&mut self) -> projects::ProjectRepository<'_> {
        projects::ProjectRepository::new(&mut self.connection)
    }

    pub fn profiles(&mut self) -> ProfileRepository<'_> {
        ProfileRepository::new(&mut self.connection)
    }

    pub fn capabilities(&mut self) -> CapabilityRepository<'_> {
        CapabilityRepository::new(&mut self.connection)
    }

    pub fn snapshots(&mut self) -> SnapshotRepository<'_> {
        SnapshotRepository::new(&mut self.connection)
    }

    pub fn audit(&mut self) -> AuditRepository<'_> {
        AuditRepository::new(&mut self.connection)
    }

    fn open_with(data_dir: &Path, migrations: &[Migration]) -> Result<Self, StorageError> {
        fs::create_dir_all(data_dir).map_err(|_| error("STORAGE_DIRECTORY_FAILED"))?;
        let path = data_dir.join(DATABASE_NAME);
        let existing = path
            .try_exists()
            .map_err(|_| error("STORAGE_OPEN_FAILED"))?;
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | if existing {
                OpenFlags::empty()
            } else {
                OpenFlags::SQLITE_OPEN_CREATE
            };
        let mut connection = Connection::open_with_flags(&path, flags)
            .map_err(|failure| database_error(failure, "STORAGE_OPEN_FAILED"))?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(sql_error)?;
        connection
            .pragma_update(None, "foreign_keys", true)
            .map_err(sql_error)?;
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(sql_error)?;
        connection
            .pragma_update(None, "trusted_schema", false)
            .map_err(sql_error)?;
        Self::migrate(&mut connection, data_dir, existing, migrations)?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(sql_error)?;
        Ok(Self { connection, path })
    }

    fn migrate(
        connection: &mut Connection,
        data_dir: &Path,
        existing: bool,
        migrations: &[Migration],
    ) -> Result<(), StorageError> {
        if migrations
            .iter()
            .enumerate()
            .any(|(i, m)| m.version != i as i64 + 1)
        {
            return Err(error("STORAGE_MIGRATION_SEQUENCE"));
        }
        // Serializes version check, backup and every pending migration against writers.
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        let version: i64 = transaction
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(sql_error)?;
        if version > migrations.len() as i64 {
            return Err(error("STORAGE_NEWER_SCHEMA"));
        }
        let integrity: String = transaction
            .query_row("PRAGMA quick_check", [], |row| row.get(0))
            .map_err(sql_error)?;
        if integrity != "ok" {
            return Err(error("STORAGE_CORRUPT"));
        }
        if version == 0 {
            let tables: i64 = transaction
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
                    [],
                    |r| r.get(0),
                )
                .map_err(sql_error)?;
            if tables != 0 {
                return Err(error("STORAGE_UNRECOGNIZED_SCHEMA"));
            }
        } else {
            let mut statement = transaction
                .prepare("SELECT version, name, checksum FROM schema_migrations ORDER BY version")
                .map_err(|failure| database_error(failure, "STORAGE_MIGRATION_HISTORY"))?;
            let history = statement
                .query_map([], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })
                .map_err(sql_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql_error)?;
            if history.len() != version as usize
                || history.iter().enumerate().any(|(i, (v, name, hash))| {
                    let m = &migrations[i];
                    *v != m.version || name != m.name || hash != &checksum(m.sql)
                })
            {
                return Err(error("STORAGE_MIGRATION_HISTORY"));
            }
        }
        let has_fk_error = transaction
            .prepare("PRAGMA foreign_key_check")
            .map_err(sql_error)?
            .query([])
            .map_err(sql_error)?
            .next()
            .map_err(sql_error)?
            .is_some();
        if has_fk_error {
            return Err(error("STORAGE_FOREIGN_KEY_CHECK"));
        }
        if version == migrations.len() as i64 {
            return transaction.commit().map_err(sql_error);
        }

        let backup_path = if existing {
            // Read a separate connection while the write lock is held. SQLite backup
            // copies a coherent committed image, including outstanding WAL pages.
            Some(Self::backup(data_dir, version)?)
        } else {
            None
        };
        let result = (|| -> rusqlite::Result<()> {
            if version == 0 {
                transaction.execute_batch("CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, name TEXT NOT NULL, checksum TEXT NOT NULL, applied_at INTEGER NOT NULL) STRICT;")?;
            }
            for migration in migrations.iter().skip(version as usize) {
                transaction.execute_batch(migration.sql)?;
                transaction.execute(
                    "INSERT INTO schema_migrations VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![
                        migration.version,
                        migration.name,
                        checksum(migration.sql),
                        now_ms()
                    ],
                )?;
                transaction.pragma_update(None, "user_version", migration.version)?;
            }
            if transaction
                .prepare("PRAGMA foreign_key_check")?
                .query([])?
                .next()?
                .is_some()
            {
                return Err(rusqlite::Error::InvalidQuery);
            }
            transaction.commit()
        })();
        result.map_err(|_| StorageError {
            code: "STORAGE_MIGRATION_FAILED",
            backup_path,
        })
    }

    fn backup(data_dir: &Path, version: i64) -> Result<PathBuf, StorageError> {
        let directory = data_dir.join("backups");
        fs::create_dir_all(&directory).map_err(|_| error("STORAGE_BACKUP_FAILED"))?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| error("STORAGE_CLOCK_FAILED"))?
            .as_nanos();
        let path = directory.join(format!(
            "before-v{version}-{nonce}-{}.sqlite3",
            std::process::id()
        ));
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| error("STORAGE_BACKUP_FAILED"))?;
        let result = (|| -> rusqlite::Result<()> {
            let source = Connection::open_with_flags(
                data_dir.join(DATABASE_NAME),
                OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
            let mut destination = Connection::open(&path)?;
            let backup = rusqlite::backup::Backup::new(&source, &mut destination)?;
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                if Instant::now() >= deadline {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                match backup.step(128)? {
                    rusqlite::backup::StepResult::Done => break,
                    rusqlite::backup::StepResult::More => continue,
                    _ => std::thread::sleep(Duration::from_millis(10)),
                }
            }
            drop(backup);
            let check: String =
                destination.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
            if check != "ok" {
                return Err(rusqlite::Error::InvalidQuery);
            }
            Ok(())
        })();
        if result.is_err() {
            // Retain partial backups for diagnosis; do not touch the original.
            return Err(error("STORAGE_BACKUP_FAILED"));
        }
        // Windows FlushFileBuffers requires a handle opened with write access.
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .and_then(|file| file.sync_all())
            .map_err(|_| error("STORAGE_BACKUP_FAILED"))?;
        Ok(path)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoverySummary {
    pub tasks: usize,
    pub approvals: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageHealth {
    pub state: &'static str,
    pub reason_code: Option<&'static str>,
}

struct StorageRuntime {
    storage: Option<Storage>,
    health: StorageHealth,
}

pub struct StorageState(Mutex<StorageRuntime>);

impl Default for StorageState {
    fn default() -> Self {
        Self(Mutex::new(StorageRuntime {
            storage: None,
            health: StorageHealth {
                state: "unavailable",
                reason_code: Some("PA_STORAGE_NOT_INITIALIZED"),
            },
        }))
    }
}

impl StorageState {
    pub fn mark_unavailable(&self, reason_code: &'static str) {
        if let Ok(mut runtime) = self.0.lock() {
            runtime.storage = None;
            runtime.health = StorageHealth {
                state: "unavailable",
                reason_code: Some(reason_code),
            };
        }
    }

    pub fn initialize(
        &self,
        data_dir: &Path,
        correlation_id: &str,
    ) -> Result<RecoverySummary, StorageError> {
        let mut runtime = self.0.lock().map_err(|_| error("STORAGE_OPEN_FAILED"))?;
        match Storage::open(data_dir).and_then(|mut storage| {
            storage
                .audit()
                .prune_expired(repositories::audit_now_ms()?)?;
            let recovery = storage.recover_on_startup(correlation_id)?;
            #[cfg(windows)]
            storage.projects().recover(correlation_id).map_err(|code| {
                error(if code == crate::ipc::ErrorCode::AuditWriteFailed {
                    "STORAGE_AUDIT_WRITE_FAILED"
                } else {
                    "STORAGE_SQL_FAILED"
                })
            })?;
            Ok((storage, recovery))
        }) {
            Ok((storage, recovery)) => {
                runtime.storage = Some(storage);
                runtime.health = StorageHealth {
                    state: "ready",
                    reason_code: None,
                };
                Ok(recovery)
            }
            Err(failure) => {
                runtime.storage = None;
                runtime.health = StorageHealth {
                    state: "unavailable",
                    reason_code: Some(failure.public_code()),
                };
                Err(failure)
            }
        }
    }

    pub fn health(&self) -> StorageHealth {
        self.0
            .lock()
            .map(|runtime| runtime.health)
            .unwrap_or(StorageHealth {
                state: "unavailable",
                reason_code: Some("PA_STORAGE_UNEXPECTED"),
            })
    }

    pub fn with_storage<T>(
        &self,
        operation: impl FnOnce(&mut Storage) -> Result<T, StorageError>,
    ) -> Result<T, StorageError> {
        let mut runtime = self.0.lock().map_err(|_| error("STORAGE_SQL_FAILED"))?;
        let storage = runtime
            .storage
            .as_mut()
            .ok_or_else(|| error("STORAGE_OPEN_FAILED"))?;
        operation(storage)
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests;
