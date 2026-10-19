use super::{
    FILE_NAMES, Failure, LOCK_NAME, MAX_FILE_BYTES, MAX_FILES, MAX_TOTAL_BYTES, RETENTION_MS,
};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub struct Writer {
    dir: PathBuf,
    _lock: File,
}

impl Drop for Writer {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.dir.join(LOCK_NAME));
    }
}

fn safe_dir(path: &Path) -> Result<PathBuf, Failure> {
    fs::create_dir_all(path).map_err(|_| Failure::Io)?;
    let meta = fs::symlink_metadata(path).map_err(|_| Failure::Identity)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(Failure::Identity);
    }
    let canonical = fs::canonicalize(path).map_err(|_| Failure::Identity)?;
    let check = fs::symlink_metadata(&canonical).map_err(|_| Failure::Identity)?;
    if !check.is_dir() || check.file_type().is_symlink() {
        return Err(Failure::Identity);
    }
    Ok(canonical)
}
fn file_path(dir: &Path, name: &str) -> Result<PathBuf, Failure> {
    let p = dir.join(name);
    if let Ok(meta) = fs::symlink_metadata(&p) {
        if meta.file_type().is_symlink() || !meta.is_file() {
            return Err(Failure::Identity);
        }
    }
    Ok(p)
}
fn measure(dir: &Path, now: u64) -> Result<(usize, u64), Failure> {
    let mut files = Vec::new();
    let mut bytes: u64 = 0;
    for name in FILE_NAMES {
        let p = file_path(dir, name)?;
        if let Ok(meta) = fs::metadata(&p) {
            if meta.len() > MAX_FILE_BYTES
                || now.saturating_sub(
                    meta.modified()
                        .map_err(|_| Failure::Io)?
                        .duration_since(UNIX_EPOCH)
                        .map_err(|_| Failure::Clock)?
                        .as_millis() as u64,
                ) > RETENTION_MS
            {
                fs::remove_file(&p).map_err(|_| Failure::Io)?;
            } else {
                files.push(p);
                bytes = bytes.saturating_add(meta.len());
            }
        }
    }
    while files.len() > MAX_FILES {
        let p = files.remove(0);
        fs::remove_file(p).map_err(|_| Failure::Io)?;
    }
    while bytes > MAX_TOTAL_BYTES {
        let Some(p) = files.first().cloned() else {
            break;
        };
        let len = fs::metadata(&p).map_err(|_| Failure::Io)?.len();
        fs::remove_file(&p).map_err(|_| Failure::Io)?;
        files.remove(0);
        bytes = bytes.saturating_sub(len);
    }
    Ok((files.len(), bytes))
}
impl Writer {
    pub fn open(path: &Path) -> Result<Self, Failure> {
        let dir = safe_dir(path)?;
        let lock_path = dir.join(LOCK_NAME);
        use std::os::windows::fs::OpenOptionsExt;
        let lock = OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(0x0000_0007)
            .custom_flags(0x0400_0000)
            .open(lock_path)
            .map_err(|_| Failure::Security)?;
        let mut writer = Self { dir, _lock: lock };
        let _ = writer.maintain(now_ms()?)?;
        Ok(writer)
    }
    pub fn maintain(&mut self, now: u64) -> Result<(usize, u64), Failure> {
        measure(&self.dir, now)
    }
    pub fn append(&mut self, line: &[u8], now: u64) -> Result<(usize, u64), Failure> {
        let path = file_path(&self.dir, FILE_NAMES[0])?;
        let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if size.saturating_add(line.len() as u64) > MAX_FILE_BYTES {
            for index in (1..MAX_FILES).rev() {
                let src = file_path(&self.dir, FILE_NAMES[index - 1])?;
                let dst = file_path(&self.dir, FILE_NAMES[index])?;
                if dst.exists() {
                    fs::remove_file(&dst).map_err(|_| Failure::Io)?;
                }
                if src.exists() {
                    fs::rename(src, dst).map_err(|_| Failure::Io)?;
                }
            }
        }
        use std::os::windows::fs::OpenOptionsExt;
        let mut options = OpenOptions::new();
        options.create(true).append(true).custom_flags(0x0020_0000);
        let mut file = options.open(&path).map_err(|_| Failure::Io)?;
        file.write_all(line).map_err(|_| Failure::Io)?;
        file.flush().map_err(|_| Failure::Io)?;
        self.maintain(now)
    }
    pub fn clear(&mut self) -> Result<(), Failure> {
        for name in FILE_NAMES {
            let p = file_path(&self.dir, name)?;
            if p.exists() {
                fs::remove_file(p).map_err(|_| Failure::Io)?;
            }
        }
        Ok(())
    }
}
fn now_ms() -> Result<u64, Failure> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|v| u64::try_from(v.as_millis()).ok())
        .ok_or(Failure::Clock)
}
