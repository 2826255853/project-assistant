//! Bounded, read-only Git summary adapter for authorized project directories.
//!
//! The adapter never invokes a shell, project command, pager, hook, or configured
//! external diff/helper. Git output and diagnostics stay local to this module.

use crate::project_contract::{GitSummary, GitSummaryState, MAX_SAFE_INTEGER};
use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Mutex, OnceLock},
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const CACHE_TTL: Duration = Duration::from_secs(5);
const CACHE_LIMIT: usize = 128;
const QUERY_BUDGET: Duration = Duration::from_secs(4);
const MAX_STDOUT: usize = 512 * 1024;
const MAX_STDERR: usize = 32 * 1024;
const READ_CHUNK: usize = 8 * 1024;

#[derive(Clone)]
struct CacheEntry {
    summary: GitSummary,
    cached_at: Instant,
}

static CACHE: OnceLock<Mutex<HashMap<(String, u64), CacheEntry>>> = OnceLock::new();

#[derive(Debug)]
enum RunFailure {
    NotFound,
    TimedOut,
    Failed,
}

struct GitOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

pub fn summary(project_id: &str, revision: u64, directory: &Path, refresh: bool) -> GitSummary {
    if !refresh {
        if let Some(cached) = cached(project_id, revision) {
            return cached;
        }
    }

    let deadline = Instant::now() + QUERY_BUDGET;
    let result = discover_git().map_or_else(
        || degraded(GitSummaryState::GitUnavailable),
        |git| query(&git, directory, deadline),
    );
    let result = with_refresh_time(result);
    cache(project_id, revision, result.clone());
    result
}

fn cached(project_id: &str, revision: u64) -> Option<GitSummary> {
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut entries = cache.lock().ok()?;
    let now = Instant::now();
    entries.retain(|_, entry| now.duration_since(entry.cached_at) <= CACHE_TTL);
    entries
        .get(&(project_id.to_owned(), revision))
        .map(|entry| entry.summary.clone())
}

fn cache(project_id: &str, revision: u64, summary: GitSummary) {
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut entries) = cache.lock() {
        if entries.len() >= CACHE_LIMIT {
            entries.clear();
        }
        entries.insert(
            (project_id.to_owned(), revision),
            CacheEntry {
                summary,
                cached_at: Instant::now(),
            },
        );
    }
}

fn query(git: &Path, directory: &Path, deadline: Instant) -> GitSummary {
    match run_git(
        git,
        directory,
        &["rev-parse", "--is-inside-work-tree"],
        deadline,
    ) {
        Ok(output) if output.status.success() && is_true(&output.stdout) => {}
        Ok(output) if is_not_repository(&output) => {
            return degraded(GitSummaryState::NotRepository);
        }
        Ok(_) => return degraded(GitSummaryState::Failed),
        Err(RunFailure::NotFound) => return degraded(GitSummaryState::GitUnavailable),
        Err(RunFailure::TimedOut) => return degraded(GitSummaryState::TimedOut),
        Err(RunFailure::Failed) => return degraded(GitSummaryState::Failed),
    }

    let branch_output = match run_git(
        git,
        directory,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        deadline,
    ) {
        Ok(output) => output,
        Err(RunFailure::NotFound) => return degraded(GitSummaryState::GitUnavailable),
        Err(RunFailure::TimedOut) => return degraded(GitSummaryState::TimedOut),
        Err(RunFailure::Failed) => return degraded(GitSummaryState::Failed),
    };
    let (branch, detached) = if branch_output.status.success() {
        match parse_branch(&branch_output.stdout) {
            Some(branch) => (Some(branch), false),
            None => return degraded(GitSummaryState::Failed),
        }
    } else if branch_output.status.code() == Some(1) && branch_output.stderr.is_empty() {
        (None, true)
    } else {
        return degraded(GitSummaryState::Failed);
    };

    let status_output = match run_git(
        git,
        directory,
        &[
            "status",
            "--porcelain=v2",
            "-z",
            "--untracked-files=normal",
            "--ignore-submodules=dirty",
        ],
        deadline,
    ) {
        Ok(output) if output.status.success() => output,
        Ok(_) => return degraded(GitSummaryState::Failed),
        Err(RunFailure::NotFound) => return degraded(GitSummaryState::GitUnavailable),
        Err(RunFailure::TimedOut) => return degraded(GitSummaryState::TimedOut),
        Err(RunFailure::Failed) => return degraded(GitSummaryState::Failed),
    };
    let Some(modified) = parse_status(&status_output.stdout) else {
        return degraded(GitSummaryState::Failed);
    };

    GitSummary {
        state: GitSummaryState::Available,
        is_repository: Some(true),
        branch,
        detached,
        working_tree_modified: Some(modified),
        refreshed_at: None,
    }
}

fn run_git(
    git: &Path,
    directory: &Path,
    args: &[&str],
    deadline: Instant,
) -> Result<GitOutput, RunFailure> {
    let mut command = Command::new(git);
    command
        .args([
            "--no-pager",
            "-c",
            "color.ui=false",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.untrackedCache=false",
            "-c",
            "core.pager=cat",
        ])
        .args(args)
        .current_dir(directory)
        .env_clear()
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_SYSTEM", "NUL")
        .env("GIT_CONFIG_GLOBAL", "NUL")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "Never")
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_git_environment(&mut command, git)?;

    run_command(command, deadline)
}

fn run_command(mut command: Command, deadline: Instant) -> Result<GitOutput, RunFailure> {
    #[cfg(windows)]
    let (mut child, job) = crate::owned_process::spawn_in_job(&mut command)
        .map(|(child, job)| (child, Some(job)))
        .map_err(map_spawn_error)?;
    #[cfg(not(windows))]
    let (mut child, job): (Child, Option<()>) = (command.spawn().map_err(map_spawn_error)?, None);

    let stdout = child.stdout.take().ok_or(RunFailure::Failed)?;
    let stderr = child.stderr.take().ok_or(RunFailure::Failed)?;
    let mut stdout_reader = Some(bounded_reader(stdout, MAX_STDOUT));
    let mut stderr_reader = Some(bounded_reader(stderr, MAX_STDERR));
    let mut stdout_data = None;
    let mut stderr_data = None;
    let mut status = None;

    loop {
        if Instant::now() >= deadline {
            terminate(&mut child, &job);
            let _ = child.wait();
            if let Some(reader) = stdout_reader.take() {
                let _ = join_reader(reader);
            }
            if let Some(reader) = stderr_reader.take() {
                let _ = join_reader(reader);
            }
            return Err(RunFailure::TimedOut);
        }
        if stdout_reader.as_ref().is_some_and(JoinHandle::is_finished) {
            match join_reader(stdout_reader.take().expect("reader present")) {
                Ok(data) => stdout_data = Some(data),
                Err(()) => {
                    terminate(&mut child, &job);
                    let _ = child.wait();
                    if let Some(reader) = stderr_reader.take() {
                        let _ = join_reader(reader);
                    }
                    return Err(RunFailure::Failed);
                }
            }
        }
        if stderr_reader.as_ref().is_some_and(JoinHandle::is_finished) {
            match join_reader(stderr_reader.take().expect("reader present")) {
                Ok(data) => stderr_data = Some(data),
                Err(()) => {
                    terminate(&mut child, &job);
                    let _ = child.wait();
                    if let Some(reader) = stdout_reader.take() {
                        let _ = join_reader(reader);
                    }
                    return Err(RunFailure::Failed);
                }
            }
        }
        if status.is_none() {
            match child.try_wait() {
                Ok(Some(exited)) => status = Some(exited),
                Ok(None) => {}
                Err(_) => {
                    terminate(&mut child, &job);
                    let _ = child.wait();
                    return Err(RunFailure::Failed);
                }
            }
        }
        if status.is_some() && stdout_data.is_some() && stderr_data.is_some() {
            return Ok(GitOutput {
                status: status.take().expect("status checked above"),
                stdout: stdout_data.take().expect("stdout checked above"),
                stderr: stderr_data.take().expect("stderr checked above"),
            });
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn bounded_reader<R: Read + Send + 'static>(
    mut reader: R,
    limit: usize,
) -> JoinHandle<Result<Vec<u8>, ()>> {
    thread::spawn(move || {
        let mut output = Vec::new();
        let mut buffer = [0; READ_CHUNK];
        loop {
            let count = reader.read(&mut buffer).map_err(|_| ())?;
            if count == 0 {
                return Ok(output);
            }
            if output.len().saturating_add(count) > limit {
                return Err(());
            }
            output.extend_from_slice(&buffer[..count]);
        }
    })
}

fn join_reader(reader: JoinHandle<Result<Vec<u8>, ()>>) -> Result<Vec<u8>, ()> {
    reader.join().map_err(|_| ())?
}

fn terminate(child: &mut Child, job: &Option<impl TerminateJob>) {
    if let Some(job) = job {
        job.terminate_job();
    } else {
        let _ = child.kill();
    }
}

trait TerminateJob {
    fn terminate_job(&self);
}

#[cfg(windows)]
impl TerminateJob for crate::owned_process::Job {
    fn terminate_job(&self) {
        self.terminate();
    }
}

#[cfg(not(windows))]
impl TerminateJob for () {
    fn terminate_job(&self) {}
}

#[cfg(windows)]
fn configure_git_environment(command: &mut Command, git: &Path) -> Result<(), RunFailure> {
    use std::os::windows::process::CommandExt;
    let system_root = std::env::var_os("SystemRoot").ok_or(RunFailure::Failed)?;
    let system_root = PathBuf::from(system_root);
    let install = git
        .parent()
        .and_then(Path::parent)
        .ok_or(RunFailure::Failed)?;
    let path = [
        install.join("cmd"),
        install.join("bin"),
        install.join("mingw64/bin"),
        install.join("usr/bin"),
        install.join("mingw64/libexec/git-core"),
        install.join("libexec/git-core"),
        system_root.join("System32"),
    ];
    let path = std::env::join_paths(path).map_err(|_| RunFailure::Failed)?;
    command
        .env("SystemRoot", &system_root)
        .env("WINDIR", &system_root)
        .env("PATH", path)
        .creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    Ok(())
}

#[cfg(not(windows))]
fn configure_git_environment(_command: &mut Command, _git: &Path) -> Result<(), RunFailure> {
    Ok(())
}

fn map_spawn_error(error: std::io::Error) -> RunFailure {
    if error.kind() == std::io::ErrorKind::NotFound {
        RunFailure::NotFound
    } else {
        RunFailure::Failed
    }
}

fn is_true(output: &[u8]) -> bool {
    matches!(output, b"true\n" | b"true\r\n")
}

fn is_not_repository(output: &GitOutput) -> bool {
    if output.status.code() != Some(128) {
        return false;
    }
    output
        .stderr
        .windows(b"not a git repository".len())
        .any(|window| window.eq_ignore_ascii_case(b"not a git repository"))
}

fn parse_branch(output: &[u8]) -> Option<String> {
    let line = output.strip_suffix(b"\n")?;
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    if line.is_empty() || line.len() > 512 || line.iter().any(|byte| *byte < 0x20 || *byte == 0x7f)
    {
        return None;
    }
    String::from_utf8(line.to_vec()).ok()
}

fn parse_status(output: &[u8]) -> Option<bool> {
    if output.is_empty() {
        return Some(false);
    }
    if output.last() != Some(&0) {
        return None;
    }
    let mut modified = false;
    let mut records = output.split(|byte| *byte == 0).peekable();
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        if record.starts_with(b"? ") {
            if record.len() == 2 {
                return None;
            }
            modified = true;
        } else if record.starts_with(b"1 ") {
            let (fields, path) = split_fields(record, 8)?;
            if !valid_xy(fields[1])
                || !valid_sub(fields[2])
                || !valid_mode(fields[3])
                || !valid_mode(fields[4])
                || !valid_mode(fields[5])
                || !valid_oid(fields[6])
                || !valid_oid(fields[7])
                || path.is_empty()
            {
                return None;
            }
            modified = true;
        } else if record.starts_with(b"2 ") {
            let (fields, path) = split_fields(record, 9)?;
            if !valid_xy(fields[1])
                || !valid_sub(fields[2])
                || !valid_mode(fields[3])
                || !valid_mode(fields[4])
                || !valid_mode(fields[5])
                || !valid_oid(fields[6])
                || !valid_oid(fields[7])
                || !valid_score(fields[8])
                || path.is_empty()
                || records.next().is_none_or(|old_path| old_path.is_empty())
            {
                return None;
            }
            modified = true;
        } else if record.starts_with(b"u ") {
            let (fields, path) = split_fields(record, 10)?;
            if !valid_xy(fields[1])
                || !valid_sub(fields[2])
                || !valid_mode(fields[3])
                || !valid_mode(fields[4])
                || !valid_mode(fields[5])
                || !valid_mode(fields[6])
                || !valid_oid(fields[7])
                || !valid_oid(fields[8])
                || !valid_oid(fields[9])
                || path.is_empty()
            {
                return None;
            }
            modified = true;
        } else {
            return None;
        }
    }
    Some(modified)
}

fn split_fields(record: &[u8], count: usize) -> Option<(Vec<&[u8]>, &[u8])> {
    let mut fields = Vec::with_capacity(count);
    let mut remaining = record;
    for _ in 0..count {
        let separator = remaining.iter().position(|byte| *byte == b' ')?;
        if separator == 0 {
            return None;
        }
        fields.push(&remaining[..separator]);
        remaining = &remaining[separator + 1..];
    }
    Some((fields, remaining))
}

fn valid_xy(value: &[u8]) -> bool {
    value.len() == 2 && value.iter().all(|byte| b".MADRCTU".contains(byte)) && value != b".."
}

fn valid_sub(value: &[u8]) -> bool {
    value == b"N..."
        || (value.len() == 4
            && value[0] == b'S'
            && value[1..].iter().all(|byte| b".CMU".contains(byte)))
}

fn valid_mode(value: &[u8]) -> bool {
    value.len() == 6 && value.iter().all(|byte| (b'0'..=b'7').contains(byte))
}

fn valid_oid(value: &[u8]) -> bool {
    (40..=64).contains(&value.len()) && value.iter().all(u8::is_ascii_hexdigit)
}

fn valid_score(value: &[u8]) -> bool {
    value.len() >= 2 && matches!(value[0], b'R' | b'C') && value[1..].iter().all(u8::is_ascii_digit)
}

fn degraded(state: GitSummaryState) -> GitSummary {
    GitSummary {
        state,
        is_repository: (state == GitSummaryState::NotRepository).then_some(false),
        branch: None,
        detached: false,
        working_tree_modified: None,
        refreshed_at: None,
    }
}

fn with_refresh_time(mut summary: GitSummary) -> GitSummary {
    summary.refreshed_at = Some(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u128::from(MAX_SAFE_INTEGER)) as u64,
    );
    summary
}

pub(crate) fn discover_git() -> Option<PathBuf> {
    #[cfg(windows)]
    let roots = windows_program_files_roots()
        .into_iter()
        .map(|root| root.join("Git"))
        .collect::<Vec<_>>();
    #[cfg(not(windows))]
    let roots = std::env::var_os("PATH")
        .map(|value| {
            std::env::split_paths(&value)
                .map(PathBuf::from)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    for root in roots {
        for relative in ["cmd/git.exe", "bin/git.exe"] {
            let candidate = root.join(relative);
            let canonical = candidate.canonicalize().ok();
            if trusted_git_file(&candidate)
                && canonical.as_deref().is_some_and(trusted_git_file)
                && canonical
                    .as_deref()
                    .is_some_and(|path| crate::project_paths::same_or_child_path(&root, path))
            {
                return canonical;
            }
        }
        #[cfg(not(windows))]
        {
            let candidate = root.join("git");
            if candidate.is_file() {
                return candidate.canonicalize().ok();
            }
        }
    }
    None
}

#[cfg(windows)]
fn windows_program_files_roots() -> Vec<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Registry::{
        HKEY_LOCAL_MACHINE, RRF_NOEXPAND, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6464KEY, RegGetValueW,
    };
    const KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion";
    let key = KEY.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let mut roots = Vec::new();
    for name in ["ProgramFilesDir", "ProgramFilesDir (x86)"] {
        let name = name.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let flags = RRF_RT_REG_SZ | RRF_NOEXPAND | RRF_SUBKEY_WOW6464KEY;
        let mut kind = 0;
        let mut bytes = 0u32;
        let size = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                name.as_ptr(),
                flags,
                &mut kind,
                std::ptr::null_mut(),
                &mut bytes,
            )
        };
        if size != 0 || bytes == 0 || bytes > 32 * 1024 {
            continue;
        }
        let mut value = vec![0u16; (bytes as usize).div_ceil(size_of::<u16>())];
        let mut capacity = bytes;
        let read = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                name.as_ptr(),
                flags,
                &mut kind,
                value.as_mut_ptr().cast(),
                &mut capacity,
            )
        };
        if read != 0 || kind != 1 {
            continue;
        }
        let length = value
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(value.len());
        if length == 0 {
            continue;
        }
        let path = PathBuf::from(std::ffi::OsString::from_wide(&value[..length]));
        if path.is_absolute()
            && !path.to_string_lossy().contains('%')
            && !roots.iter().any(|existing| existing == &path)
        {
            roots.push(path);
        }
    }
    roots
}

#[cfg(windows)]
fn trusted_git_file(candidate: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    if !candidate.is_absolute()
        || candidate
            .extension()
            .is_none_or(|ext| !ext.eq_ignore_ascii_case("exe"))
    {
        return false;
    }
    for path in candidate.ancestors() {
        let Ok(metadata) = std::fs::symlink_metadata(path) else {
            return false;
        };
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return false;
        }
    }
    candidate.is_file()
}

#[cfg(not(windows))]
fn trusted_git_file(_candidate: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_v2_clean_changed_untracked_and_rename_records() {
        assert_eq!(parse_status(b""), Some(false));
        assert_eq!(parse_status(b"? new-file\0"), Some(true));
        let oid = b"0123456789012345678901234567890123456789";
        let one = [
            b"1 .M N... 100644 100644 100644 ".as_slice(),
            oid,
            b" ".as_slice(),
            oid,
            b" file\0".as_slice(),
        ]
        .concat();
        let two = [
            b"2 R. N... 100644 100644 100644 ".as_slice(),
            oid,
            b" ".as_slice(),
            oid,
            b" R100 new\0old\0".as_slice(),
        ]
        .concat();
        let unmerged = [
            b"u UU N... 100644 100644 100644 100644 ".as_slice(),
            oid,
            b" ".as_slice(),
            oid,
            b" ".as_slice(),
            oid,
            b" file\0".as_slice(),
        ]
        .concat();
        assert_eq!(parse_status(&one), Some(true));
        assert_eq!(parse_status(&two), Some(true));
        assert_eq!(parse_status(&unmerged), Some(true));
        assert_eq!(parse_status(b"unsupported\0"), None);
        assert_eq!(parse_status(b"? missing-terminator"), None);
        assert_eq!(parse_status(b"2 R. truncated\0"), None);
        assert_eq!(
            parse_status(b"1 .M N... 100644 100644 100644 fake fake path\0"),
            None
        );
    }

    #[test]
    fn branch_and_repository_diagnostic_parsers_reject_malformed_values() {
        assert_eq!(parse_branch(b"main\n").as_deref(), Some("main"));
        assert_eq!(
            parse_branch("feature/中文\r\n".as_bytes()).as_deref(),
            Some("feature/中文")
        );
        assert_eq!(parse_branch(b"bad\nextra\n"), None);
        assert_eq!(parse_branch(&[b'a'; 513]), None);
        assert!(is_true(b"true\n"));
        assert!(!is_true(b" true\n"));
        assert!(is_not_repository(&GitOutput {
            status: fake_status(128),
            stdout: Vec::new(),
            stderr: b"fatal: not a git repository (or any parent)".to_vec(),
        }));
        assert!(!is_not_repository(&GitOutput {
            status: fake_status(128),
            stdout: Vec::new(),
            stderr: b"fatal: unsafe repository".to_vec(),
        }));
    }

    #[test]
    fn owned_child_timeout_and_output_limit_are_enforced() {
        let executable = std::env::current_exe().unwrap();
        let root = std::env::temp_dir();
        let mut sleeper = Command::new(&executable);
        sleeper
            .args([
                "--exact",
                "project_git::tests::owned_process_fixture",
                "--nocapture",
            ])
            .env("PA_GIT_TEST_MODE", "sleep")
            .current_dir(&root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let timed = run_command(sleeper, Instant::now() + Duration::from_millis(750));
        assert!(matches!(timed, Err(RunFailure::TimedOut)));

        let mut noisy = Command::new(executable);
        noisy
            .args([
                "--exact",
                "project_git::tests::owned_process_fixture",
                "--nocapture",
            ])
            .env("PA_GIT_TEST_MODE", "output")
            .current_dir(root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let overflow = run_command(noisy, Instant::now() + Duration::from_secs(5));
        assert!(matches!(overflow, Err(RunFailure::Failed)));
    }

    #[test]
    fn owned_process_fixture() {
        match std::env::var("PA_GIT_TEST_MODE").as_deref() {
            Ok("sleep") => thread::sleep(Duration::from_secs(20)),
            Ok("output") => {
                use std::io::Write;
                let bytes = vec![b'x'; MAX_STDOUT + 1];
                let _ = std::io::stdout().lock().write_all(&bytes);
            }
            _ => {}
        }
    }

    #[cfg(unix)]
    fn fake_status(code: i32) -> ExitStatus {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw(code << 8)
    }

    #[cfg(windows)]
    fn fake_status(code: i32) -> ExitStatus {
        use std::os::windows::process::ExitStatusExt;
        ExitStatus::from_raw(code as u32)
    }

    #[test]
    fn real_git_summary_handles_empty_repository_changes_non_repository_and_missing_git() {
        use std::{fs, path::PathBuf, process::Command};

        let root = std::env::temp_dir().join(format!("pa-git-summary-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let repo = root.join("repo 中文 & special");
        let non_repo = root.join("not a repo");
        fs::create_dir(&repo).unwrap();
        fs::create_dir(&non_repo).unwrap();

        let git = discover_git().or_else(|| {
            #[cfg(windows)]
            {
                Some(PathBuf::from(r"C:\Program Files\Git\cmd\git.exe"))
            }
            #[cfg(not(windows))]
            {
                std::env::var_os("PATH")
                    .and_then(|path| std::env::split_paths(&path).next())
                    .map(|path| path.join("git"))
            }
        });
        let Some(git) = git else {
            fs::remove_dir_all(&root).unwrap();
            return;
        };
        let init = Command::new(&git)
            .args(["init", "-b", "main"])
            .current_dir(&repo)
            .output()
            .unwrap();
        assert!(init.status.success());
        assert!(
            Command::new(&git)
                .args([
                    "config",
                    "core.fsmonitor",
                    "echo invoked > git-helper-marker",
                ])
                .current_dir(&repo)
                .output()
                .unwrap()
                .status
                .success()
        );

        let empty = query(&git, &repo, Instant::now() + QUERY_BUDGET);
        assert_eq!(empty.state, GitSummaryState::Available);
        assert_eq!(empty.branch.as_deref(), Some("main"));
        assert_eq!(empty.detached, false);
        assert_eq!(empty.working_tree_modified, Some(false));
        assert!(!repo.join("git-helper-marker").exists());
        assert!(
            Command::new(&git)
                .args(["config", "--unset", "core.fsmonitor"])
                .current_dir(&repo)
                .output()
                .unwrap()
                .status
                .success()
        );

        fs::write(repo.join("untracked 中文.txt"), "synthetic").unwrap();
        let changed = query(&git, &repo, Instant::now() + QUERY_BUDGET);
        assert_eq!(changed.state, GitSummaryState::Available);
        assert_eq!(changed.working_tree_modified, Some(true));
        fs::remove_file(repo.join("untracked 中文.txt")).unwrap();

        fs::write(repo.join("tracked.txt"), "initial\n").unwrap();
        let add = Command::new(&git)
            .args(["add", "--", "tracked.txt"])
            .current_dir(&repo)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "NUL")
            .output()
            .unwrap();
        assert!(add.status.success());
        let commit = Command::new(&git)
            .args([
                "-c",
                "user.name=Project Assistant Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-m",
                "synthetic",
            ])
            .current_dir(&repo)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "NUL")
            .output()
            .unwrap();
        assert!(commit.status.success());
        let clean = query(&git, &repo, Instant::now() + QUERY_BUDGET);
        assert_eq!(clean.state, GitSummaryState::Available);
        assert_eq!(clean.working_tree_modified, Some(false));

        let detached = Command::new(&git)
            .args(["checkout", "--detach", "HEAD"])
            .current_dir(&repo)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "NUL")
            .output()
            .unwrap();
        assert!(detached.status.success());
        let detached_summary = query(&git, &repo, Instant::now() + QUERY_BUDGET);
        assert_eq!(detached_summary.state, GitSummaryState::Available);
        assert_eq!(detached_summary.branch, None);
        assert!(detached_summary.detached);

        fs::write(repo.join("tracked.txt"), "modified\n").unwrap();
        let tracked_change = query(&git, &repo, Instant::now() + QUERY_BUDGET);
        assert_eq!(tracked_change.working_tree_modified, Some(true));
        assert!(
            Command::new(&git)
                .args(["checkout", "--", "tracked.txt"])
                .current_dir(&repo)
                .output()
                .unwrap()
                .status
                .success()
        );

        let submodule = root.join("submodule");
        fs::create_dir(&submodule).unwrap();
        let submodule_init = Command::new(&git)
            .args(["init", "-q", "-b", "main"])
            .current_dir(&submodule)
            .output()
            .unwrap();
        assert!(submodule_init.status.success());
        for args in [
            vec!["config", "user.name", "Project Assistant Fixture"],
            vec!["config", "user.email", "fixture@example.invalid"],
        ] {
            assert!(
                Command::new(&git)
                    .args(args)
                    .current_dir(&submodule)
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        }
        fs::write(submodule.join("nested.txt"), "committed").unwrap();
        assert!(
            Command::new(&git)
                .args(["add", "--", "nested.txt"])
                .current_dir(&submodule)
                .output()
                .unwrap()
                .status
                .success()
        );
        assert!(
            Command::new(&git)
                .args(["commit", "-qm", "submodule base"])
                .current_dir(&submodule)
                .output()
                .unwrap()
                .status
                .success()
        );
        assert!(
            Command::new(&git)
                .args(["-c", "protocol.file.allow=always", "submodule", "add", "-q"])
                .arg(&submodule)
                .arg("dep")
                .current_dir(&repo)
                .output()
                .unwrap()
                .status
                .success()
        );
        assert!(
            Command::new(&git)
                .args(["add", "--", ".gitmodules", "dep"])
                .current_dir(&repo)
                .output()
                .unwrap()
                .status
                .success()
        );
        assert!(
            Command::new(&git)
                .args([
                    "-c",
                    "user.name=Project Assistant Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "commit",
                    "-m",
                    "submodule fixture",
                ])
                .current_dir(&repo)
                .output()
                .unwrap()
                .status
                .success()
        );
        assert_eq!(
            query(&git, &repo, Instant::now() + QUERY_BUDGET).working_tree_modified,
            Some(false)
        );
        fs::write(repo.join("dep/nested.txt"), "dirty but ignored").unwrap();
        assert_eq!(
            query(&git, &repo, Instant::now() + QUERY_BUDGET).working_tree_modified,
            Some(false)
        );
        assert!(
            Command::new(&git)
                .args(["add", "--", "nested.txt"])
                .current_dir(repo.join("dep"))
                .output()
                .unwrap()
                .status
                .success()
        );
        assert!(
            Command::new(&git)
                .args([
                    "-c",
                    "user.name=Project Assistant Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "commit",
                    "-qm",
                    "submodule second commit",
                ])
                .current_dir(repo.join("dep"))
                .output()
                .unwrap()
                .status
                .success()
        );
        let submodule_head_change = query(&git, &repo, Instant::now() + QUERY_BUDGET);
        assert_eq!(submodule_head_change.state, GitSummaryState::Available);
        assert_eq!(submodule_head_change.working_tree_modified, Some(true));

        let not_repo = query(&git, &non_repo, Instant::now() + QUERY_BUDGET);
        assert_eq!(not_repo.state, GitSummaryState::NotRepository);
        assert_eq!(not_repo.is_repository, Some(false));

        let absent = query(
            &root.join("git-not-installed.exe"),
            &repo,
            Instant::now() + QUERY_BUDGET,
        );
        assert_eq!(absent.state, GitSummaryState::GitUnavailable);

        let malformed = query(
            &std::env::current_exe().unwrap(),
            &repo,
            Instant::now() + QUERY_BUDGET,
        );
        assert_eq!(malformed.state, GitSummaryState::Failed);
        fs::remove_dir_all(&root).unwrap();
    }
}
