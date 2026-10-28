//! S2-05. Read handles pin names; a rollback-only NTFS metadata transaction also
//! excludes attribute-only reparse mutation throughout synchronous OS acceptance.
use super::*;
use std::os::windows::{io::OwnedHandle, process::CommandExt};
use std::{
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::CloseHandle,
    Storage::FileSystem::{
        CreateFileTransactedW, CreateTransaction, FILE_BASIC_INFO, FILE_GENERIC_READ,
        FILE_WRITE_ATTRIBUTES, FileBasicInfo, RollbackTransaction, SetFileInformationByHandle,
    },
    UI::{
        Controls::Dialogs::{
            CommDlgExtendedError, GetOpenFileNameW, OFN_DONTADDTORECENT, OFN_EXPLORER,
            OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_NODEREFERENCELINKS, OFN_PATHMUSTEXIST,
            OPENFILENAMEW,
        },
        Shell::{
            ASSOCF_INIT_IGNOREUNKNOWN, ASSOCSTR_EXECUTABLE, AssocQueryStringW, SEE_MASK_FLAG_NO_UI,
            SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
        },
        WindowsAndMessaging::SW_SHOWNORMAL,
    },
};

pub(crate) struct PinnedTarget {
    path: PathBuf,
    identity: DirectoryIdentity,
    is_directory: bool,
    _handles: Vec<File>,
    _metadata: Option<MetadataGuard>,
}

struct MetadataTransaction(OwnedHandle);
impl Drop for MetadataTransaction {
    fn drop(&mut self) {
        unsafe { RollbackTransaction(self.0.as_raw_handle().cast()) };
    }
}
struct MetadataGuard {
    _file: File,
    _transaction: MetadataTransaction,
}

fn metadata_guard(path: &Path, expected: DirectoryIdentity) -> Result<MetadataGuard, ErrorCode> {
    // Zero means infinite timeout: caller timeout must never release the guard
    // while ShellExecute still runs. This transaction is NEVER committed.
    let raw = unsafe {
        CreateTransaction(
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            std::ptr::null(),
        )
    };
    if raw == INVALID_HANDLE_VALUE || raw.is_null() {
        return Err(ErrorCode::ProjectOpenRejected);
    }
    let transaction = MetadataTransaction(unsafe { OwnedHandle::from_raw_handle(raw.cast()) });
    let name = wide(path);
    let raw = unsafe {
        CreateFileTransactedW(
            name.as_ptr(),
            FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES,
            FILE_SHARE_READ,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
            transaction.0.as_raw_handle().cast(),
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(match unsafe { GetLastError() } {
            5 | 32 | 33 => ErrorCode::ProjectAccessDenied,
            _ => ErrorCode::ProjectOpenRejected,
        });
    }
    let file = unsafe { File::from_raw_handle(raw.cast()) };
    let mut basic = FILE_BASIC_INFO::default();
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle().cast(),
            FileBasicInfo,
            (&mut basic as *mut FILE_BASIC_INFO).cast(),
            size_of::<FILE_BASIC_INFO>() as u32,
        )
    } == 0
    {
        return Err(ErrorCode::ProjectOpenRejected);
    }
    if basic.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 || identity(&file)? != expected {
        return Err(ErrorCode::ProjectFileOutsideProject);
    }
    // Stage the unchanged basic information to take the transactional metadata
    // write lock. Nontransactional readers still see the original information;
    // writers conflict. Rollback preserves all original attributes/timestamps.
    if unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle().cast(),
            FileBasicInfo,
            (&basic as *const FILE_BASIC_INFO).cast(),
            size_of::<FILE_BASIC_INFO>() as u32,
        )
    } == 0
    {
        return Err(ErrorCode::ProjectOpenRejected);
    }
    Ok(MetadataGuard {
        _file: file,
        _transaction: transaction,
    })
}

fn pin(path: &Path, directory: bool) -> Result<PinnedTarget, ErrorCode> {
    let mut pinned = pin_names(path, directory)?;
    pinned._metadata = Some(metadata_guard(&pinned.path, pinned.identity)?);
    Ok(pinned)
}

fn pin_names(path: &Path, directory: bool) -> Result<PinnedTarget, ErrorCode> {
    let plain = validate_windows_path(path, true)?;
    let text = plain.to_str().ok_or(ErrorCode::ProjectPathUnsafe)?;
    let drive: Vec<u16> = text[..3].encode_utf16().chain(Some(0)).collect();
    if !matches!(
        unsafe { GetDriveTypeW(drive.as_ptr()) },
        DRIVE_FIXED | DRIVE_REMOVABLE
    ) {
        return Err(ErrorCode::ProjectPathUnsafe);
    }
    ensure_supported_local_filesystem(drive.as_ptr())?;
    let mut cursor = PathBuf::from(format!(r"\\?\{}", &text[..3]));
    let mut handles = Vec::new();
    let names = std::iter::once("")
        .chain(text[3..].split('\\'))
        .collect::<Vec<_>>();
    let mut previous: Option<PathBuf> = None;
    let mut volume = None;
    for (index, name) in names.iter().enumerate() {
        if !name.is_empty() {
            cursor.push(name);
        }
        let wide = wide(&cursor);
        let raw = unsafe {
            CreateFileW(
                wide.as_ptr(),
                FILE_GENERIC_READ,
                FILE_SHARE_READ,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                std::ptr::null_mut(),
            )
        };
        if raw == INVALID_HANDLE_VALUE {
            let error = unsafe { GetLastError() };
            return Err(if !directory && matches!(error, 2 | 3) {
                ErrorCode::ProjectFileMissing
            } else {
                map_native_error(std::io::Error::from_raw_os_error(error as i32))
            });
        }
        let handle = unsafe { File::from_raw_handle(raw.cast()) };
        let mut attr = FILE_ATTRIBUTE_TAG_INFO::default();
        if unsafe {
            GetFileInformationByHandleEx(
                handle.as_raw_handle().cast(),
                FileAttributeTagInfo,
                (&mut attr as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
                size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
            )
        } == 0
        {
            return Err(ErrorCode::ProjectAccessDenied);
        }
        if attr.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(ErrorCode::ProjectFileOutsideProject);
        }
        let wants_directory = index != names.len() - 1 || directory;
        if (attr.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0) != wants_directory {
            return Err(if directory {
                ErrorCode::ProjectPathUnsafe
            } else {
                ErrorCode::ProjectFileMissing
            });
        }
        let current = final_path(&handle)?;
        let id = identity(&handle)?;
        if previous
            .as_ref()
            .is_some_and(|p| !is_direct_child_path(p, &current))
            || volume.is_some_and(|v| v != id.volume_serial_number)
        {
            return Err(ErrorCode::ProjectFileOutsideProject);
        }
        volume = Some(id.volume_serial_number);
        previous = Some(current);
        handles.push(handle);
    }
    let last = handles.last().ok_or(ErrorCode::ProjectPathUnavailable)?;
    let path = previous.ok_or(ErrorCode::ProjectPathUnavailable)?;
    let expected = identity(last)?;
    Ok(PinnedTarget {
        path,
        identity: expected,
        is_directory: directory,
        _handles: handles,
        _metadata: None,
    })
}

pub(crate) fn directory(verified: &VerifiedDirectory) -> Result<PinnedTarget, ErrorCode> {
    // This root is only a preparation snapshot. For file delivery each ancestor
    // has a nondeletable child, so it cannot be converted to a reparse point.
    // Directory delivery additionally acquires a final metadata guard in open.
    let pinned = pin_names(&verified.canonical_path, true).map_err(|error| match error {
        ErrorCode::ProjectFileOutsideProject | ErrorCode::ProjectPathUnsafe => {
            ErrorCode::ProjectPathChanged
        }
        other => other,
    })?;
    if pinned.identity != verified.identity {
        return Err(ErrorCode::ProjectPathChanged);
    }
    Ok(pinned)
}

pub(crate) fn file(root: &PinnedTarget, relative: &str) -> Result<PinnedTarget, ErrorCode> {
    // Reuse the exact IPC relative-path rules, even for native picker output.
    if relative.len() > crate::project_contract::MAX_RELATIVE_PATH_BYTES {
        return Err(ErrorCode::InputTooLarge);
    }
    if !crate::project_contract::valid_relative_path(relative) {
        return Err(ErrorCode::ProjectFileOutsideProject);
    }
    let target = root.path.join(relative.replace('/', "\\"));
    let pinned = pin(&target, false)?;
    if !same_or_child_path(&root.path, &pinned.path) {
        return Err(ErrorCode::ProjectFileOutsideProject);
    }
    Ok(pinned)
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

pub(crate) fn pick(owner: isize, root: &PinnedTarget) -> Result<Option<String>, ErrorCode> {
    let _apartment = ComApartment::initialize()?;
    let initial = wide(&strip_extended_prefix(&root.path));
    let mut selected = vec![0u16; MAX_WINDOWS_PATH_UNITS];
    let mut dialog = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner as *mut c_void,
        lpstrFile: selected.as_mut_ptr(),
        nMaxFile: selected.len() as u32,
        lpstrInitialDir: initial.as_ptr(),
        Flags: OFN_EXPLORER
            | OFN_FILEMUSTEXIST
            | OFN_PATHMUSTEXIST
            | OFN_NOCHANGEDIR
            | OFN_DONTADDTORECENT
            | OFN_NODEREFERENCELINKS,
        ..Default::default()
    };
    if unsafe { GetOpenFileNameW(&mut dialog) } == 0 {
        return if unsafe { CommDlgExtendedError() } == 0 {
            Ok(None)
        } else {
            Err(ErrorCode::ProjectPickerFailed)
        };
    }
    let length = selected
        .iter()
        .position(|v| *v == 0)
        .ok_or(ErrorCode::ProjectPickerFailed)?;
    let selected = PathBuf::from(OsString::from_wide(&selected[..length]));
    picked_relative(root, &selected).map(Some)
}

fn picked_relative(root: &PinnedTarget, selected: &Path) -> Result<String, ErrorCode> {
    let root_plain = strip_extended_prefix(&root.path);
    if !same_or_child_path(&root_plain, selected) {
        return Err(ErrorCode::ProjectFileOutsideProject);
    }
    let relative = selected
        .components()
        .skip(root_plain.components().count())
        .collect::<PathBuf>();
    let relative = relative
        .to_str()
        .ok_or(ErrorCode::ProjectFileOutsideProject)?
        .to_owned();
    let _target = file(root, &relative)?;
    Ok(relative)
}

// A stuck native association retains its pins and admission permit. Repeated
// caller timeouts cannot accumulate unbounded native workers or redeliver.
static BUSY: AtomicBool = AtomicBool::new(false);
struct Permit;
impl Drop for Permit {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::Release);
    }
}

pub(crate) fn open(
    mut root: PinnedTarget,
    target: Option<PinnedTarget>,
    editor: Option<crate::settings::Editor>,
) -> Result<(), ErrorCode> {
    if BUSY.swap(true, Ordering::AcqRel) {
        return Err(ErrorCode::ProjectOpenRejected);
    }
    let permit = Permit;
    bounded(Duration::from_secs(10), move || {
        let _permit = permit;
        if target.is_none() {
            root._metadata = Some(metadata_guard(&root.path, root.identity)?);
        }
        let path = target.as_ref().unwrap_or(&root);
        // Root and leaf pins stay alive until the synchronous native call returns.
        let result = if let Some(editor) = editor {
            editor_open(path, &editor)
        } else {
            system_open(path)
        };
        drop((target, root));
        result
    })
}

fn bounded(
    timeout: Duration,
    work: impl FnOnce() -> Result<(), ErrorCode> + Send + 'static,
) -> Result<(), ErrorCode> {
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("project-open".into())
        .spawn(move || {
            let _ = send.send(work());
        })
        .map_err(|_| ErrorCode::ProjectOpenRejected)?;
    receive.recv_timeout(timeout).map_err(|error| match error {
        std::sync::mpsc::RecvTimeoutError::Timeout => ErrorCode::ProjectOpenTimeout,
        _ => ErrorCode::ProjectOpenRejected,
    })?
}

fn system_open(target: &PinnedTarget) -> Result<(), ErrorCode> {
    // Shell links are ordinary files but dispatch to a different target/URL.
    if !target.is_directory
        && target
            .path
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|ext| {
                [
                    "lnk",
                    "url",
                    "website",
                    "appref-ms",
                    "search-ms",
                    "library-ms",
                ]
                .iter()
                .any(|blocked| ext.eq_ignore_ascii_case(blocked))
            })
    {
        return Err(ErrorCode::ProjectOpenRejected);
    }
    let _apartment = ComApartment::initialize().map_err(|_| ErrorCode::ProjectOpenRejected)?;
    // ShellExecute can report acceptance of Open-With for an unassociated
    // extension. That is not acceptance by a default application.
    if !target.is_directory {
        require_association(&target.path)?;
    }
    let path = wide(&strip_extended_prefix(&target.path));
    let verb: Vec<u16> = "open\0".encode_utf16().collect();
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI | SEE_MASK_NOCLOSEPROCESS,
        lpVerb: verb.as_ptr(),
        lpFile: path.as_ptr(),
        nShow: SW_SHOWNORMAL,
        ..Default::default()
    };
    let accepted = unsafe { ShellExecuteExW(&mut info) };
    let code = unsafe { GetLastError() };
    if !info.hProcess.is_null() {
        unsafe { CloseHandle(info.hProcess) };
    }
    if accepted == 0 {
        Err(native_open_error(code))
    } else {
        Ok(())
    }
}

fn require_association(path: &Path) -> Result<(), ErrorCode> {
    let extension = path
        .extension()
        .and_then(OsStr::to_str)
        .filter(|s| !s.is_empty())
        .ok_or(ErrorCode::ProjectOpenRejected)?;
    let extension: Vec<u16> = format!(".{extension}\0").encode_utf16().collect();
    let verb: Vec<u16> = "open\0".encode_utf16().collect();
    let mut length = 0;
    let result = unsafe {
        AssocQueryStringW(
            ASSOCF_INIT_IGNOREUNKNOWN,
            ASSOCSTR_EXECUTABLE,
            extension.as_ptr(),
            verb.as_ptr(),
            std::ptr::null_mut(),
            &mut length,
        )
    };
    if result < 0 || !(2..=32767).contains(&length) {
        return Err(ErrorCode::ProjectOpenRejected);
    }
    let mut value = vec![0u16; length as usize];
    if unsafe {
        AssocQueryStringW(
            ASSOCF_INIT_IGNOREUNKNOWN,
            ASSOCSTR_EXECUTABLE,
            extension.as_ptr(),
            verb.as_ptr(),
            value.as_mut_ptr(),
            &mut length,
        )
    } < 0
        || value[0] == 0
    {
        return Err(ErrorCode::ProjectOpenRejected);
    }
    Ok(())
}

fn native_open_error(code: u32) -> ErrorCode {
    match code {
        5 => ErrorCode::ProjectAccessDenied,
        1460 => ErrorCode::ProjectOpenTimeout,
        _ => ErrorCode::ProjectOpenRejected,
    }
}

fn editor_open(target: &PinnedTarget, editor: &crate::settings::Editor) -> Result<(), ErrorCode> {
    crate::settings::Change::Editor(Some(editor.clone()))
        .validate()
        .map_err(|_| ErrorCode::ProjectOpenRejected)?;
    let executable =
        pin(Path::new(&editor.executable), false).map_err(|_| ErrorCode::ProjectOpenRejected)?;
    // Check the DOS signature; CreateProcess subsequently validates the PE image.
    use std::io::Read;
    let mut signature = [0u8; 2];
    let mut reader = executable
        ._handles
        .last()
        .ok_or(ErrorCode::ProjectOpenRejected)?;
    reader
        .read_exact(&mut signature)
        .map_err(|_| ErrorCode::ProjectOpenRejected)?;
    if signature != *b"MZ" {
        return Err(ErrorCode::ProjectOpenRejected);
    }
    let system = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .ok_or(ErrorCode::ProjectOpenRejected)?
        .join("System32");
    let child = Command::new(&executable.path)
        .arg(strip_extended_prefix(&target.path))
        .current_dir(system)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(0x08000000)
        .spawn()
        .map_err(|e| native_open_error(e.raw_os_error().unwrap_or(0) as u32))?;
    // Acceptance is successful CreateProcess, not successful loading/editing.
    drop(child);
    Ok(())
}

#[cfg(test)]
mod tests;
