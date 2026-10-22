use super::{Failure, Result};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

const MARKER: &str = "project-assistant-owned-process-v1";

pub fn check_path(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(Failure::Unsafe);
    }
    // Check each ancestor, including Windows junctions (not only symbolic links).
    for ancestor in path.ancestors() {
        let metadata = fs::symlink_metadata(ancestor).map_err(|_| Failure::Unsafe)?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(Failure::Unsafe);
            }
        }
        if metadata.file_type().is_symlink() {
            return Err(Failure::Unsafe);
        }
    }
    path.canonicalize().map_err(|_| Failure::Unsafe)
}

fn lock(path: &Path, create: bool) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    if create {
        options.create_new(true);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0).custom_flags(0x0020_0000);
    }
    options.open(path)
}

pub struct Home {
    pub path: PathBuf,
    parent: PathBuf,
    pub(super) lock: Option<File>,
}
impl Home {
    pub fn create(parent: &Path) -> Result<Self> {
        let parent = check_path(parent)?;
        let path = parent.join(format!("owned-{}", super::random_token()?));
        #[cfg(windows)]
        super::windows::private_directory(&path)?;
        #[cfg(not(windows))]
        return Err(Failure::Unsafe);
        let mut home = Self {
            path,
            parent,
            lock: None,
        };
        home.lock = Some(lock(&home.path.join(".lock"), true).map_err(|_| Failure::Cleanup)?);
        fs::write(home.path.join(".owner"), MARKER).map_err(|_| Failure::Cleanup)?;
        for name in [
            "codex",
            "user",
            "appdata",
            "localappdata",
            "temp",
            "project",
        ] {
            fs::create_dir(home.path.join(name)).map_err(|_| Failure::Cleanup)?;
        }
        // A fixed project-root marker prevents config discovery from walking
        // into the real user's ancestors when app_data_dir is under their home.
        fs::create_dir(home.path.join("project/.git")).map_err(|_| Failure::Cleanup)?;
        Ok(home)
    }
    pub fn cleanup(&mut self) -> Result<()> {
        if !self.path.exists() {
            return Ok(());
        }
        self.verify()?;
        verify_tree(&self.path)?;
        self.lock.take();
        // Only a canonical direct child of the backend-owned root is removable.
        fs::remove_dir_all(&self.path).map_err(|_| Failure::Cleanup)
    }
    fn verify(&self) -> Result<()> {
        let canonical = check_path(&self.path)?;
        if canonical.parent() != Some(self.parent.as_path())
            || !canonical
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(valid_name)
            || fs::read(self.path.join(".owner")).map_err(|_| Failure::Cleanup)?
                != MARKER.as_bytes()
        {
            return Err(Failure::Cleanup);
        }
        Ok(())
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}
fn valid_name(name: &str) -> bool {
    name.strip_prefix("owned-")
        .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}
fn verify_tree(path: &Path) -> Result<()> {
    for entry in fs::read_dir(path).map_err(|_| Failure::Cleanup)? {
        let entry = entry.map_err(|_| Failure::Cleanup)?;
        check_path(&entry.path())?;
        if entry.file_type().map_err(|_| Failure::Cleanup)?.is_dir() {
            verify_tree(&entry.path())?;
        }
    }
    Ok(())
}

/// Startup cleanup only touches marked, unlocked direct children. Live owners
/// keep the exclusive file handle; unexpected entries are never adopted.
pub fn cleanup_stale(parent: &Path) -> Result<usize> {
    let parent = check_path(parent)?;
    let mut count = 0;
    for entry in fs::read_dir(&parent).map_err(|_| Failure::Cleanup)? {
        let entry = entry.map_err(|_| Failure::Cleanup)?;
        if !entry.file_name().to_str().is_some_and(valid_name) {
            continue;
        }
        let path = entry.path();
        check_path(&path)?;
        if fs::read(path.join(".owner")).map_err(|_| Failure::Cleanup)? != MARKER.as_bytes() {
            return Err(Failure::Cleanup);
        }
        let Ok(handle) = lock(&path.join(".lock"), false) else {
            continue;
        };
        let mut home = Home {
            path,
            parent: parent.clone(),
            lock: Some(handle),
        };
        home.cleanup()?;
        count += 1;
    }
    Ok(count)
}
