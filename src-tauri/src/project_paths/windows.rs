use crate::ipc::ErrorCode;
use std::{
    ffi::{OsStr, OsString, c_void},
    fs::File,
    mem::size_of,
    os::windows::ffi::{OsStrExt, OsStringExt},
    os::windows::io::{AsRawHandle, FromRawHandle},
    path::{Component, Path, PathBuf},
};
use windows_sys::Win32::{
    Foundation::{GetLastError, INVALID_HANDLE_VALUE, SetLastError},
    Globalization::{LCMAP_UPPERCASE, LCMapStringEx, LOCALE_NAME_INVARIANT},
    Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_ATTRIBUTE_TAG_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_ID_INFO, FILE_NAME_NORMALIZED, FILE_READ_ATTRIBUTES, FILE_SHARE_READ,
        FILE_SHARE_WRITE, FileAttributeTagInfo, FileIdInfo, GetDriveTypeW,
        GetFileInformationByHandleEx, GetFinalPathNameByHandleW, GetVolumeInformationW,
        OPEN_EXISTING,
    },
    System::{
        Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize},
        WindowsProgramming::{DRIVE_FIXED, DRIVE_REMOVABLE},
    },
    UI::Shell::{
        BIF_NEWDIALOGSTYLE, BIF_NOTRANSLATETARGETS, BIF_RETURNONLYFSDIRS, BROWSEINFOW,
        SHBrowseForFolderW, SHGetNameFromIDList, SIGDN_FILESYSPATH,
    },
};

const MAX_WINDOWS_PATH_UNITS: usize = 32_767;

/// Stable, local file-system identity. Keep this in the application database;
/// it is never part of an IPC DTO, log, or audit summary.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DirectoryIdentity {
    pub volume_serial_number: u64,
    pub file_id: [u8; 16],
}

impl std::fmt::Debug for DirectoryIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("DirectoryIdentity([redacted])")
    }
}

impl DirectoryIdentity {
    /// Hex-encoded opaque value for the private projects table. Never expose it
    /// in DTOs or diagnostic output.
    pub fn storage_value(self) -> String {
        let mut value = String::with_capacity(48);
        use std::fmt::Write as _;
        let _ = write!(value, "{:016X}", self.volume_serial_number);
        for byte in self.file_id {
            let _ = write!(value, "{byte:02X}");
        }
        value
    }

    pub fn from_storage_value(value: &str) -> Option<Self> {
        if value.len() != 48 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let volume_serial_number = u64::from_str_radix(&value[..16], 16).ok()?;
        let mut file_id = [0; 16];
        for (index, byte) in file_id.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&value[16 + index * 2..18 + index * 2], 16).ok()?;
        }
        Some(Self {
            volume_serial_number,
            file_id,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DuplicateResolution {
    NewDirectory,
    ExistingDirectory,
    ExplicitReauthorizationRequired,
}

/// A verified path plus open handles for every directory in its path. The
/// handles retain the checked identities; they do not make later path-based
/// operations atomic, so callers must revalidate at the actual use boundary.
pub struct VerifiedDirectory {
    pub canonical_path: PathBuf,
    pub display_path: String,
    pub path_key: String,
    pub name: String,
    pub identity: DirectoryIdentity,
    _ancestor_handles: Vec<File>,
}

impl std::fmt::Debug for VerifiedDirectory {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VerifiedDirectory")
            .field("path", &"[redacted]")
            .field("identity", &"[redacted]")
            .finish_non_exhaustive()
    }
}

impl PartialEq for VerifiedDirectory {
    fn eq(&self, other: &Self) -> bool {
        self.canonical_path == other.canonical_path && self.identity == other.identity
    }
}

impl Eq for VerifiedDirectory {}

/// Opens the native Windows folder picker and validates its result before it
/// can become an authorization candidate. The returned handles must be kept
/// alive until the caller has persisted the authorization transactionally.
pub(super) fn choose_project_directory(
    owner_hwnd: isize,
) -> Result<Option<VerifiedDirectory>, ErrorCode> {
    let _apartment = ComApartment::initialize()?;
    let title: Vec<u16> = "选择要登记的项目目录"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut display_name = vec![0u16; MAX_WINDOWS_PATH_UNITS];
    let mut info = BROWSEINFOW {
        hwndOwner: owner_hwnd as *mut c_void,
        pidlRoot: std::ptr::null_mut(),
        pszDisplayName: display_name.as_mut_ptr(),
        lpszTitle: title.as_ptr(),
        ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE | BIF_NOTRANSLATETARGETS,
        lpfn: None,
        lParam: 0,
        iImage: 0,
    };
    unsafe { SetLastError(0) };
    let item = unsafe { SHBrowseForFolderW(&mut info) };
    if item.is_null() {
        // The API returns null for both cancellation and shell failure. A
        // cleared last-error slot distinguishes the usual cancellation path.
        return if unsafe { GetLastError() } == 0 {
            Ok(None)
        } else {
            Err(ErrorCode::ProjectPickerFailed)
        };
    }
    let _item = TaskMemory(item.cast());

    let mut selected_path = std::ptr::null_mut();
    let result = unsafe { SHGetNameFromIDList(item, SIGDN_FILESYSPATH, &mut selected_path) };
    if result < 0 || selected_path.is_null() {
        return Err(ErrorCode::ProjectPickerFailed);
    }
    let _selected_path = TaskMemory(selected_path.cast());
    let units = unsafe { read_nul_terminated(selected_path, MAX_WINDOWS_PATH_UNITS) }
        .ok_or(ErrorCode::ProjectPickerFailed)?;
    let path = PathBuf::from(OsString::from_wide(units));
    authorize_selected_directory(&path).map(Some)
}

pub(super) fn authorize_selected_directory(path: &Path) -> Result<VerifiedDirectory, ErrorCode> {
    inspect_directory(path, None)
}

pub(super) fn revalidate_registered_directory(
    canonical_path: &Path,
    expected_identity: DirectoryIdentity,
) -> Result<VerifiedDirectory, ErrorCode> {
    inspect_directory(canonical_path, Some(expected_identity))
}

fn inspect_directory(
    input_path: &Path,
    expected_identity: Option<DirectoryIdentity>,
) -> Result<VerifiedDirectory, ErrorCode> {
    let path = validate_windows_path(input_path, expected_identity.is_some())?;
    let text = path.to_str().ok_or(ErrorCode::ProjectPathUnsafe)?;
    let drive_root = format!("{}:\\", &text[..1]);
    let wide_drive_root: Vec<u16> = drive_root.encode_utf16().chain(Some(0)).collect();
    let drive_type = unsafe { GetDriveTypeW(wide_drive_root.as_ptr()) };
    if drive_type != DRIVE_FIXED && drive_type != DRIVE_REMOVABLE {
        return Err(ErrorCode::ProjectPathUnsafe);
    }
    ensure_supported_local_filesystem(wide_drive_root.as_ptr())?;

    let body = &text[3..];
    let components = body
        .strip_suffix('\\')
        .unwrap_or(body)
        .split('\\')
        .collect::<Vec<_>>();
    if components.is_empty() || components.iter().any(|value| value.is_empty()) {
        return Err(ErrorCode::ProjectPathUnsafe);
    }

    // Use the extended DOS prefix for filesystem opens so selected paths over
    // MAX_PATH remain available without changing the user-facing display path.
    let root = PathBuf::from(format!("\\\\?\\{}:\\", &text[..1]));
    let mut cursor = root.clone();
    let mut handles = Vec::with_capacity(components.len() + 1);
    let root_handle = open_directory_no_follow(&root)?;
    let root_identity = identity(&root_handle)?;
    let mut previous_final_path = final_path(&root_handle)?;
    handles.push(root_handle);
    for component in components {
        cursor.push(component);
        let handle = open_directory_no_follow(&cursor)?;
        let current_final_path = final_path(&handle)?;
        let current_identity = identity(&handle)?;
        let is_child = is_direct_child_path(&previous_final_path, &current_final_path);
        let same_volume =
            current_identity.volume_serial_number == root_identity.volume_serial_number;
        if !is_child || !same_volume {
            return Err(ErrorCode::ProjectPathUnsafe);
        }
        previous_final_path = current_final_path;
        handles.push(handle);
    }

    let final_handle = handles.last().ok_or(ErrorCode::ProjectPathUnavailable)?;
    let final_handle_path = previous_final_path;
    if !same_drive(&final_handle_path, &path) {
        return Err(ErrorCode::ProjectPathUnsafe);
    }
    let canonical_path = final_handle_path;
    let file_identity = identity(final_handle)?;
    if expected_identity.is_some_and(|expected| expected != file_identity) {
        return Err(ErrorCode::ProjectPathChanged);
    }

    let display_path = strip_extended_prefix(&canonical_path)
        .to_str()
        .ok_or(ErrorCode::ProjectPathUnsafe)?
        .to_owned();
    let name = canonical_path
        .file_name()
        .and_then(OsStr::to_str)
        .filter(|name| !name.is_empty())
        .ok_or(ErrorCode::ProjectPathUnsafe)?
        .to_owned();
    if display_path.len() > 4 * 1024 || name.len() > 1024 {
        return Err(ErrorCode::InputTooLarge);
    }
    let path_key = windows_path_key(&canonical_path)?;

    Ok(VerifiedDirectory {
        canonical_path,
        display_path,
        path_key,
        name,
        identity: file_identity,
        _ancestor_handles: handles,
    })
}

/// Validates a plain drive-absolute path. Extended DOS paths are accepted only
/// when re-reading a path already stored by the service; UNC/device namespaces,
/// ADS, dot segments, DOS devices and Win32-invalid names always fail closed.
pub(super) fn validate_windows_path(
    path: &Path,
    allow_extended: bool,
) -> Result<PathBuf, ErrorCode> {
    validate_windows_path_inner(path, allow_extended, false)
}

fn validate_windows_path_inner(
    path: &Path,
    allow_extended: bool,
    allow_drive_root: bool,
) -> Result<PathBuf, ErrorCode> {
    let raw = path.to_str().ok_or(ErrorCode::ProjectPathUnsafe)?;
    if raw.is_empty() || raw.encode_utf16().count() > MAX_WINDOWS_PATH_UNITS {
        return Err(ErrorCode::ProjectPathUnsafe);
    }
    let normalized = if let Some(rest) = raw.strip_prefix(r"\\?\") {
        if !allow_extended || rest.starts_with("UNC\\") {
            return Err(ErrorCode::ProjectPathUnsafe);
        }
        rest
    } else {
        raw
    };
    let bytes = normalized.as_bytes();
    if bytes.len() < 3
        || !bytes[0].is_ascii_alphabetic()
        || bytes[1] != b':'
        || bytes[2] != b'\\'
        || normalized.contains('/')
        || normalized[2..].contains(':')
    {
        return Err(ErrorCode::ProjectPathUnsafe);
    }
    let body = &normalized[3..];
    let body = if let Some(stripped) = body.strip_suffix('\\') {
        if stripped.ends_with('\\') {
            return Err(ErrorCode::ProjectPathUnsafe);
        }
        stripped
    } else {
        body
    };
    if body.is_empty() && !allow_drive_root {
        // A drive root is too broad to grant as a project directory.
        return Err(ErrorCode::ProjectPathUnsafe);
    }
    if !body.is_empty() {
        for component in body.split('\\') {
            if component.is_empty()
                || component == "."
                || component == ".."
                || component.ends_with(['.', ' '])
                || component
                    .chars()
                    .any(|ch| ch < ' ' || "<>\"|?*".contains(ch))
                || is_dos_device_name(component)
            {
                return Err(ErrorCode::ProjectPathUnsafe);
            }
        }
    }
    let parsed = Path::new(normalized);
    if !parsed.is_absolute()
        || !parsed.components().all(|component| {
            matches!(
                component,
                Component::Prefix(_) | Component::RootDir | Component::Normal(_)
            )
        })
    {
        return Err(ErrorCode::ProjectPathUnsafe);
    }
    Ok(PathBuf::from(normalized))
}

fn is_dos_device_name(component: &str) -> bool {
    let stem = component
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(['.', ' ']);
    let stem = stem.to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        stem.strip_prefix(prefix).is_some_and(|number| {
            matches!(number, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        })
    }) || ["COM", "LPT"].iter().any(|prefix| {
        stem.strip_prefix(prefix)
            .is_some_and(|number| matches!(number, "¹" | "²" | "³"))
    })
}

fn open_directory_no_follow(path: &Path) -> Result<File, ErrorCode> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let raw = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(map_native_error(std::io::Error::from_raw_os_error(
            unsafe { GetLastError() } as i32,
        )));
    }
    let handle = unsafe { File::from_raw_handle(raw.cast()) };

    let mut attributes = FILE_ATTRIBUTE_TAG_INFO::default();
    let ok = unsafe {
        GetFileInformationByHandleEx(
            handle.as_raw_handle().cast(),
            FileAttributeTagInfo,
            (&mut attributes as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    };
    if ok == 0 {
        return Err(map_native_error(std::io::Error::from_raw_os_error(
            unsafe { GetLastError() } as i32,
        )));
    }
    if attributes.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(ErrorCode::ProjectPathUnsafe);
    }
    if attributes.FileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
        return Err(ErrorCode::ProjectPathUnsafe);
    }
    Ok(handle)
}

fn identity(handle: &File) -> Result<DirectoryIdentity, ErrorCode> {
    let mut info = FILE_ID_INFO::default();
    let ok = unsafe {
        GetFileInformationByHandleEx(
            handle.as_raw_handle().cast(),
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if ok == 0 {
        return Err(map_native_error(std::io::Error::from_raw_os_error(
            unsafe { GetLastError() } as i32,
        )));
    }
    Ok(DirectoryIdentity {
        volume_serial_number: info.VolumeSerialNumber,
        file_id: info.FileId.Identifier,
    })
}

fn final_path(handle: &File) -> Result<PathBuf, ErrorCode> {
    let mut buffer = vec![0u16; 512];
    loop {
        let length = unsafe {
            GetFinalPathNameByHandleW(
                handle.as_raw_handle().cast(),
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                FILE_NAME_NORMALIZED,
            )
        } as usize;
        if length == 0 {
            return Err(map_native_error(std::io::Error::from_raw_os_error(
                unsafe { GetLastError() } as i32,
            )));
        }
        if length < buffer.len() {
            return Ok(PathBuf::from(OsString::from_wide(&buffer[..length])));
        }
        if length >= MAX_WINDOWS_PATH_UNITS {
            return Err(ErrorCode::ProjectPathUnsafe);
        }
        buffer.resize(length + 1, 0);
    }
}

fn same_drive(left: &Path, right: &Path) -> bool {
    let Some(left) = drive_letter(left) else {
        return false;
    };
    let Some(right) = drive_letter(right) else {
        return false;
    };
    left.eq_ignore_ascii_case(&right)
}

fn drive_letter(path: &Path) -> Option<char> {
    let text = path.to_str()?;
    let text = text.strip_prefix(r"\\?\").unwrap_or(text);
    let bytes = text.as_bytes();
    (bytes.len() >= 3 && bytes[1] == b':' && bytes[2] == b'\\').then(|| bytes[0] as char)
}

fn strip_extended_prefix(path: &Path) -> &Path {
    path.to_str()
        .and_then(|value| value.strip_prefix(r"\\?\"))
        .map(Path::new)
        .unwrap_or(path)
}

fn windows_path_key(path: &Path) -> Result<String, ErrorCode> {
    let value = strip_extended_prefix(path)
        .to_str()
        .ok_or(ErrorCode::ProjectPathUnsafe)?
        .replace('/', "\\");
    let value = value.trim_end_matches('\\');
    let source = value.encode_utf16().collect::<Vec<_>>();
    if source.len() > i32::MAX as usize {
        return Err(ErrorCode::ProjectPathUnsafe);
    }
    let required = unsafe {
        LCMapStringEx(
            LOCALE_NAME_INVARIANT,
            LCMAP_UPPERCASE,
            source.as_ptr(),
            source.len() as i32,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
            std::ptr::null(),
            0,
        )
    };
    if required <= 0 {
        return Err(ErrorCode::ProjectPathUnsafe);
    }
    let mut upper = vec![0u16; required as usize];
    let written = unsafe {
        LCMapStringEx(
            LOCALE_NAME_INVARIANT,
            LCMAP_UPPERCASE,
            source.as_ptr(),
            source.len() as i32,
            upper.as_mut_ptr(),
            upper.len() as i32,
            std::ptr::null(),
            std::ptr::null(),
            0,
        )
    };
    if written != required {
        return Err(ErrorCode::ProjectPathUnsafe);
    }
    String::from_utf16(&upper).map_err(|_| ErrorCode::ProjectPathUnsafe)
}

fn compare_ordinal_ignore_case(left: &str, right: &str) -> bool {
    use windows_sys::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal};
    let left: Vec<u16> = left.encode_utf16().collect();
    let right: Vec<u16> = right.encode_utf16().collect();
    if left.len() > i32::MAX as usize || right.len() > i32::MAX as usize {
        return false;
    }
    unsafe {
        CompareStringOrdinal(
            left.as_ptr(),
            left.len() as i32,
            right.as_ptr(),
            right.len() as i32,
            1,
        ) == CSTR_EQUAL
    }
}

pub fn same_or_child_path(root: &Path, candidate: &Path) -> bool {
    let Ok(root) = validate_windows_path_inner(root, true, true) else {
        return false;
    };
    let Ok(candidate) = validate_windows_path_inner(candidate, true, true) else {
        return false;
    };
    let root = strip_extended_prefix(&root);
    let candidate = strip_extended_prefix(&candidate);
    let root_text = match root.to_str() {
        Some(value) => value,
        None => return false,
    };
    let candidate_text = match candidate.to_str() {
        Some(value) => value,
        None => return false,
    };
    let root_components = root.components().collect::<Vec<_>>();
    let candidate_components = candidate.components().collect::<Vec<_>>();
    root_components.len() <= candidate_components.len()
        && root_components
            .iter()
            .zip(candidate_components.iter())
            .all(|(left, right)| {
                compare_ordinal_ignore_case(
                    left.as_os_str().to_str().unwrap_or(root_text),
                    right.as_os_str().to_str().unwrap_or(candidate_text),
                )
            })
}

fn is_direct_child_path(parent: &Path, candidate: &Path) -> bool {
    let parent_components = parent.components().collect::<Vec<_>>();
    let candidate_components = candidate.components().collect::<Vec<_>>();
    candidate_components.len() == parent_components.len() + 1
        && same_or_child_path(parent, candidate)
}

/// Resolves duplicate selection without silently granting a replacement at a
/// previously registered canonical path.
pub fn compare_registered_target(
    registered_path_key: &str,
    registered_identity: DirectoryIdentity,
    selected: &VerifiedDirectory,
) -> DuplicateResolution {
    if registered_identity == selected.identity {
        DuplicateResolution::ExistingDirectory
    } else if compare_ordinal_ignore_case(registered_path_key, &selected.path_key) {
        DuplicateResolution::ExplicitReauthorizationRequired
    } else {
        DuplicateResolution::NewDirectory
    }
}

pub(super) fn map_native_error(error: std::io::Error) -> ErrorCode {
    match error.raw_os_error().map(|code| code as u32) {
        Some(2 | 3 | 15 | 21 | 53) => ErrorCode::ProjectPathUnavailable,
        Some(5 | 32 | 33) => ErrorCode::ProjectAccessDenied,
        _ => ErrorCode::ProjectPathUnavailable,
    }
}

fn ensure_supported_local_filesystem(root: *const u16) -> Result<(), ErrorCode> {
    let mut name = [0u16; 32];
    let ok = unsafe {
        GetVolumeInformationW(
            root,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            name.as_mut_ptr(),
            name.len() as u32,
        )
    };
    if ok == 0 {
        return Err(map_native_error(std::io::Error::from_raw_os_error(
            unsafe { GetLastError() } as i32,
        )));
    }
    let length = name
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(name.len());
    let name = String::from_utf16(&name[..length]).map_err(|_| ErrorCode::ProjectPathUnsafe)?;
    if name.eq_ignore_ascii_case("NTFS") || name.eq_ignore_ascii_case("ReFS") {
        Ok(())
    } else {
        Err(ErrorCode::ProjectPathUnsafe)
    }
}

struct ComApartment;

impl ComApartment {
    fn initialize() -> Result<Self, ErrorCode> {
        let result = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
        if result < 0 {
            Err(ErrorCode::ProjectPickerFailed)
        } else {
            Ok(Self)
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct TaskMemory(*const c_void);

impl Drop for TaskMemory {
    fn drop(&mut self) {
        unsafe { CoTaskMemFree(self.0) };
    }
}

unsafe fn read_nul_terminated<'a>(pointer: *const u16, max_units: usize) -> Option<&'a [u16]> {
    if pointer.is_null() {
        return None;
    }
    let mut length = 0;
    while length < max_units {
        if unsafe { *pointer.add(length) } == 0 {
            return Some(unsafe { std::slice::from_raw_parts(pointer, length) });
        }
        length += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_path_key_discards_backend_extended_prefix() {
        assert_eq!(
            windows_path_key(Path::new(r"\\?\C:\Work\Project")).unwrap(),
            windows_path_key(Path::new(r"c:\work\project")).unwrap()
        );
    }

    #[test]
    fn handle_paths_must_remain_adjacent_children() {
        let parent = Path::new(r"\\?\C:\root\project");
        assert!(is_direct_child_path(
            parent,
            Path::new(r"\\?\C:\root\project\child")
        ));
        assert!(!is_direct_child_path(
            parent,
            Path::new(r"\\?\C:\root\project\child\nested")
        ));
        assert!(!is_direct_child_path(
            parent,
            Path::new(r"\\?\C:\root\outside\child")
        ));
        assert!(!is_direct_child_path(
            parent,
            Path::new(r"\\?\C:\root\project-old\child")
        ));
    }

    #[test]
    fn dos_device_aliases_are_rejected_case_insensitively() {
        for name in ["con", "NUL.txt", "COM1.log", "lpt9", "COM¹"] {
            assert!(is_dos_device_name(name));
        }
        for name in ["console", "com10", "project.txt"] {
            assert!(!is_dos_device_name(name));
        }
    }

    #[test]
    fn directory_identity_round_trips_only_fixed_hex_storage_values() {
        let identity = DirectoryIdentity {
            volume_serial_number: 0xDEADBEEF00112233,
            file_id: [0xAB; 16],
        };
        let encoded = identity.storage_value();
        assert_eq!(encoded.len(), 48);
        assert_eq!(
            DirectoryIdentity::from_storage_value(&encoded),
            Some(identity)
        );
        assert_eq!(DirectoryIdentity::from_storage_value("../private"), None);
        assert_eq!(DirectoryIdentity::from_storage_value(&"0".repeat(47)), None);
        assert_eq!(
            DirectoryIdentity::from_storage_value(&format!("{}G", "0".repeat(47))),
            None
        );
    }

    #[test]
    fn identity_debug_output_does_not_expose_storage_value() {
        let identity = DirectoryIdentity {
            volume_serial_number: 0xDEADBEEF00112233,
            file_id: [0xAB; 16],
        };
        assert_eq!(format!("{identity:?}"), "DirectoryIdentity([redacted])");
    }
}
