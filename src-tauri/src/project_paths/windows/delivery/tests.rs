use super::*;
// TxF conflicts conservatively across concurrent metadata operations; the product
// also serializes opens. Keep these adversarial filesystem cases independent.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
use std::{
    fs,
    sync::{Arc, atomic::AtomicUsize},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("pa-s2-05-{}", crate::ipc::correlation_id()));
        fs::create_dir_all(root.join("项目 空格 & ()/nested")).unwrap();
        fs::write(
            root.join("项目 空格 & ()/nested/文本 & ().txt"),
            "source-sentinel",
        )
        .unwrap();
        Self(root)
    }
    fn project(&self) -> PathBuf {
        self.0.join("项目 空格 & ()")
    }
    fn pinned(&self) -> PinnedTarget {
        directory(&authorize_selected_directory(&self.project()).unwrap()).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn strong_pins_block_root_ancestor_leaf_rename_delete_and_write_until_acceptance() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let f = Fixture::new();
    let root = f.pinned();
    let target = file(&root, "nested/文本 & ().txt").unwrap();
    for path in [
        f.0.clone(),
        f.project(),
        f.project().join("nested"),
        f.project().join("nested/文本 & ().txt"),
    ] {
        assert!(
            fs::rename(&path, path.with_extension("moved")).is_err(),
            "identity pin allowed rename"
        );
    }
    assert!(fs::remove_file(f.project().join("nested/文本 & ().txt")).is_err());
    assert!(fs::write(f.project().join("nested/文本 & ().txt"), "replacement").is_err());
    // An attacker cannot acquire the write handle needed to set a reparse point.
    use std::os::windows::fs::OpenOptionsExt;
    assert!(
        fs::OpenOptions::new()
            .access_mode(0x40000000)
            .share_mode(7)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(f.project().join("nested"))
            .is_err()
    );
    assert_eq!(
        fs::read(f.project().join("nested/文本 & ().txt")).unwrap(),
        b"source-sentinel"
    );
    drop((target, root));
    fs::rename(f.project().join("nested"), f.project().join("renamed")).unwrap();
}

#[test]
fn replaced_root_is_rejected_before_delivery() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let f = Fixture::new();
    let verified = authorize_selected_directory(&f.project()).unwrap();
    fs::rename(f.project(), f.0.join("old")).unwrap();
    fs::create_dir(f.project()).unwrap();
    assert!(matches!(
        directory(&verified),
        Err(ErrorCode::ProjectPathChanged)
    ));
}

#[test]
fn preexisting_write_or_delete_handle_prevents_delivery_pin() {
    use std::os::windows::fs::OpenOptionsExt;
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let f = Fixture::new();
    let root = f.pinned();
    let path = f.project().join("nested").join("文本 & ().txt");
    for access in [0x40000000, 0x00010000] {
        let attacker = fs::OpenOptions::new()
            .access_mode(access)
            .share_mode(7)
            .open(&path)
            .unwrap();
        assert!(matches!(
            file(&root, "nested/文本 & ().txt"),
            Err(ErrorCode::ProjectAccessDenied)
        ));
        drop(attacker);
    }
    assert_eq!(file(&root, "nested/文本 & ().txt").err(), None);
}

#[test]
fn preexisting_attribute_handle_cannot_set_junction_while_delivery_is_pinned() {
    use std::os::windows::fs::OpenOptionsExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn DeviceIoControl(
            handle: *mut c_void,
            code: u32,
            input: *const c_void,
            input_size: u32,
            output: *mut c_void,
            output_size: u32,
            returned: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
    }
    fn set_junction(handle: &File, target: &Path) -> bool {
        let name = target.to_str().unwrap();
        let substitute: Vec<u16> = format!(r"\??\{name}").encode_utf16().collect();
        let display: Vec<u16> = name.encode_utf16().collect();
        let mut data = Vec::new();
        data.extend(0xA0000003u32.to_le_bytes());
        data.extend(((8 + (substitute.len() + display.len() + 2) * 2) as u16).to_le_bytes());
        for value in [
            0,
            0,
            (substitute.len() * 2) as u16,
            ((substitute.len() + 1) * 2) as u16,
            (display.len() * 2) as u16,
        ] {
            data.extend(value.to_le_bytes());
        }
        for unit in substitute
            .into_iter()
            .chain(Some(0))
            .chain(display)
            .chain(Some(0))
        {
            data.extend(unit.to_le_bytes());
        }
        let mut returned = 0;
        unsafe {
            DeviceIoControl(
                handle.as_raw_handle().cast(),
                0x000900A4,
                data.as_ptr().cast(),
                data.len() as u32,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            ) != 0
        }
    }
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let f = Fixture::new();
    let outside = f.0.join("outside");
    fs::create_dir(&outside).unwrap();
    let control = f.project().join("control");
    fs::create_dir(&control).unwrap();
    let write_handle = fs::OpenOptions::new()
        .access_mode(0x40000000)
        .share_mode(7)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&control)
        .unwrap();
    assert!(
        set_junction(&write_handle, &outside),
        "control reparse mutation must succeed"
    );
    drop(write_handle);
    fs::remove_dir(&control).unwrap();
    let victim = f.project().join("victim");
    fs::create_dir(&victim).unwrap();
    let attribute_handle = fs::OpenOptions::new()
        .access_mode(0x100)
        .share_mode(7)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&victim)
        .unwrap();
    match pin(&victim, true) {
        Ok(pinned) => {
            assert!(
                !set_junction(&attribute_handle, &outside),
                "pinned identity allowed reparse mutation"
            );
            drop(pinned);
        }
        Err(error) => assert!(matches!(
            error,
            ErrorCode::ProjectAccessDenied | ErrorCode::ProjectOpenRejected
        )),
    }
    // Intermediate directories retain a pinned child name and cannot become
    // empty. Even a previously obtained attribute handle cannot reparse them.
    let branch = f.project().join("branch");
    fs::create_dir(&branch).unwrap();
    fs::write(branch.join("anchor"), "anchor").unwrap();
    let ancestor_writer = fs::OpenOptions::new()
        .access_mode(FILE_WRITE_ATTRIBUTES)
        .share_mode(7)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&branch)
        .unwrap();
    let child = pin_names(&branch.join("anchor"), false).unwrap();
    assert!(!set_junction(&ancestor_writer, &outside));
    assert!(fs::remove_file(branch.join("anchor")).is_err());
    drop(child);
    fs::remove_file(branch.join("anchor")).unwrap();
    assert!(set_junction(&ancestor_writer, &outside));
    drop(ancestor_writer);
    fs::remove_dir(branch).unwrap();
}

#[test]
fn junction_inside_project_and_same_prefix_sibling_are_rejected() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let f = Fixture::new();
    fs::create_dir(f.0.join("outside")).unwrap();
    fs::write(f.0.join("outside/out.txt"), "outside-sentinel").unwrap();
    let junction = f.project().join("escape");
    let output = Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&junction)
        .arg(f.0.join("outside"))
        .creation_flags(0x08000000)
        .output()
        .unwrap();
    assert!(output.status.success());
    let root = f.pinned();
    assert!(matches!(
        file(&root, "escape/out.txt"),
        Err(ErrorCode::ProjectFileOutsideProject)
    ));
    assert!(matches!(
        picked_relative(&root, &f.0.join("项目 空格 & ()-old/out.txt")),
        Err(ErrorCode::ProjectFileOutsideProject)
    ));
    drop(root);
    fs::remove_dir(junction).unwrap();
}

#[test]
fn file_picker_result_is_relative_case_insensitive_and_never_authorizes_outside() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let f = Fixture::new();
    let root = f.pinned();
    let alias = PathBuf::from(
        f.project()
            .join("nested")
            .join("文本 & ().txt")
            .to_string_lossy()
            .to_uppercase(),
    );
    assert_eq!(
        picked_relative(&root, &alias).unwrap().to_lowercase(),
        "nested\\文本 & ().txt"
    );
    assert!(matches!(
        picked_relative(&root, &f.0.join("outside.txt")),
        Err(ErrorCode::ProjectFileOutsideProject)
    ));
    assert_eq!(
        file(&root, "nested/missing.txt").err(),
        Some(ErrorCode::ProjectFileMissing)
    );
    for relative in [
        "../out.txt",
        "C:\\outside.txt",
        "\\\\server\\share",
        "nested/a:stream",
        "nested/NUL.txt",
        "nested/trailing. ",
    ] {
        assert!(matches!(
            file(&root, relative),
            Err(ErrorCode::ProjectFileOutsideProject)
        ));
    }
}

#[test]
fn timeout_delivers_once_and_retains_pins_until_native_return() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let f = Fixture::new();
    let root = pin(&f.project(), true).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let (started, ready) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let (done, finished) = std::sync::mpsc::channel();
    assert_eq!(
        bounded(Duration::from_millis(20), move || {
            counter.fetch_add(1, Ordering::SeqCst);
            started.send(()).unwrap();
            wait.recv().unwrap();
            drop(root);
            done.send(()).unwrap();
            Ok(())
        }),
        Err(ErrorCode::ProjectOpenTimeout)
    );
    ready.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(fs::rename(f.project(), f.0.join("moved")).is_err());
    use std::os::windows::fs::OpenOptionsExt;
    assert!(
        fs::OpenOptions::new()
            .access_mode(FILE_WRITE_ATTRIBUTES)
            .share_mode(7)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(f.project())
            .is_err()
    );
    release.send(()).unwrap();
    finished.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    fs::rename(f.project(), f.0.join("moved")).unwrap();
}

#[test]
fn failed_associations_access_denial_and_invalid_editor_are_fixed_errors() {
    assert_eq!(native_open_error(1155), ErrorCode::ProjectOpenRejected);
    assert_eq!(native_open_error(5), ErrorCode::ProjectAccessDenied);
    assert_eq!(native_open_error(1460), ErrorCode::ProjectOpenTimeout);
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let f = Fixture::new();
    let root = f.pinned();
    let target = file(&root, "nested/文本 & ().txt").unwrap();
    for executable in [f.0.join("missing.exe"), f.0.join("invalid.exe")] {
        if executable.file_name().unwrap() == "invalid.exe" {
            fs::write(&executable, "invalid PE").unwrap();
        }
        assert_eq!(
            editor_open(
                &target,
                &crate::settings::Editor {
                    id: "custom".into(),
                    executable: executable.to_string_lossy().into()
                }
            ),
            Err(ErrorCode::ProjectOpenRejected)
        );
    }
}

#[test]
fn acquired_metadata_guard_denies_later_attribute_writers_and_rolls_back_unchanged() {
    use std::os::windows::fs::OpenOptionsExt;
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let f = Fixture::new();
    let empty = f.project().join("empty");
    fs::create_dir(&empty).unwrap();
    fn basic(path: &Path) -> FILE_BASIC_INFO {
        let handle = fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(7)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .unwrap();
        let mut info = FILE_BASIC_INFO::default();
        assert_ne!(
            unsafe {
                GetFileInformationByHandleEx(
                    handle.as_raw_handle().cast(),
                    FileBasicInfo,
                    (&mut info as *mut FILE_BASIC_INFO).cast(),
                    size_of::<FILE_BASIC_INFO>() as u32,
                )
            },
            0
        );
        info
    }
    for (path, is_directory) in [
        (empty, true),
        (f.project().join("nested").join("文本 & ().txt"), false),
    ] {
        let before = basic(&path);
        let pinned = pin(&path, is_directory).unwrap();
        assert!(
            fs::OpenOptions::new()
                .access_mode(FILE_WRITE_ATTRIBUTES)
                .share_mode(7)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                .open(&path)
                .is_err()
        );
        drop(pinned);
        let after = basic(&path);
        assert_eq!(
            (
                before.CreationTime,
                before.LastWriteTime,
                before.ChangeTime,
                before.FileAttributes
            ),
            (
                after.CreationTime,
                after.LastWriteTime,
                after.ChangeTime,
                after.FileAttributes
            )
        );
    }
}
