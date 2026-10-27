//! Windows directory picker and identity-bound project authorization.
//!
//! This module is intentionally not registered as an IPC command. The picker
//! returns a short-lived verified directory so the project repository can
//! persist its canonical path and identity in the same transaction (step 2.3).

use crate::ipc::ErrorCode;

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::{
    DirectoryIdentity, DuplicateResolution, VerifiedDirectory, compare_registered_target,
    same_or_child_path,
};

#[cfg(all(any(test, debug_assertions), windows))]
pub(crate) fn authorize_fixture(path: &std::path::Path) -> Result<VerifiedDirectory, ErrorCode> {
    windows::authorize_selected_directory(path)
}

/// Starts the native Windows folder picker, owned by the local main window.
/// `None` means the user cancelled; it is not an error.
#[cfg(windows)]
pub fn choose_project_directory(owner_hwnd: isize) -> Result<Option<VerifiedDirectory>, ErrorCode> {
    windows::choose_project_directory(owner_hwnd)
}

/// Non-Windows builds fail closed. Project authorization is currently a
/// Windows-only product capability.
#[cfg(not(windows))]
pub fn choose_project_directory(_owner_hwnd: isize) -> Result<Option<()>, ErrorCode> {
    Err(ErrorCode::ProjectPickerFailed)
}

/// Reopens a stored canonical path and verifies its file-system identity.
/// Callers must keep the returned identity snapshot available and still
/// revalidate at the actual use boundary; directory handles do not make
/// path-based operations atomic.
#[cfg(windows)]
pub fn revalidate_registered_directory(
    canonical_path: &std::path::Path,
    expected_identity: DirectoryIdentity,
) -> Result<VerifiedDirectory, ErrorCode> {
    windows::revalidate_registered_directory(canonical_path, expected_identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::Path, path::PathBuf, process::Command};

    fn temporary_root(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "project-assistant-s2-02-{label}-{}",
            crate::ipc::correlation_id()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    #[cfg(windows)]
    fn identity_supports_unicode_spaces_and_windows_case_aliases() {
        let root = temporary_root("unicode");
        let project = root.join("中文 项目 & Source");
        fs::create_dir(&project).unwrap();

        let first = windows::authorize_selected_directory(&project).unwrap();
        let case_alias = PathBuf::from(project.to_string_lossy().to_uppercase());
        let second = windows::authorize_selected_directory(&case_alias).unwrap();

        assert_eq!(first.identity, second.identity);
        assert_eq!(first.path_key, second.path_key);
        assert_eq!(
            windows::compare_registered_target(&first.path_key, first.identity, &second),
            windows::DuplicateResolution::ExistingDirectory
        );
        assert_eq!(
            windows::compare_registered_target("C:\\OLD\\PROJECT", first.identity, &second),
            windows::DuplicateResolution::ExistingDirectory
        );
        let replacement = windows::DirectoryIdentity {
            volume_serial_number: first.identity.volume_serial_number.wrapping_add(1),
            file_id: first.identity.file_id,
        };
        assert_eq!(
            windows::compare_registered_target(&first.path_key, replacement, &second),
            windows::DuplicateResolution::ExplicitReauthorizationRequired
        );
        assert!(first.display_path.contains("中文 项目 & Source"));
        assert_eq!(first.name, "中文 项目 & Source");
        let debug = format!("{first:?}");
        assert!(!debug.contains(&first.display_path));
        assert!(!debug.contains(&first.identity.storage_value()));

        drop((first, second));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(windows)]
    fn path_boundary_compares_components_not_string_prefixes() {
        let root = Path::new(r"C:\work\app");
        assert!(windows::same_or_child_path(
            root,
            Path::new(r"c:\WORK\app\src")
        ));
        assert!(!windows::same_or_child_path(
            root,
            Path::new(r"C:\work\app-old")
        ));
        assert!(!windows::same_or_child_path(
            root,
            Path::new(r"C:\work\other")
        ));
        assert!(!windows::same_or_child_path(
            root,
            Path::new(r"C:\work\app\..\outside")
        ));
        assert!(!windows::same_or_child_path(
            root,
            Path::new(r"\\server\share\app\src")
        ));
    }

    #[test]
    #[cfg(windows)]
    fn missing_file_and_unsafe_windows_path_shapes_fail_closed() {
        let root = temporary_root("invalid");
        let file = root.join("not-a-directory.txt");
        fs::write(&file, "fixture").unwrap();

        assert_eq!(
            windows::authorize_selected_directory(&root.join("missing")),
            Err(ErrorCode::ProjectPathUnavailable)
        );
        assert_eq!(
            windows::authorize_selected_directory(&file),
            Err(ErrorCode::ProjectPathUnsafe)
        );
        for path in [
            r"\\server\share\project",
            r"\\?\C:\project",
            r"C:\project:stream",
            r"C:\work\..\outside",
            r"C:\work\CON",
            r"C:\work\trailing.",
        ] {
            assert_eq!(
                windows::validate_windows_path(Path::new(path), false),
                Err(ErrorCode::ProjectPathUnsafe),
                "unsafe path shape was accepted"
            );
        }

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(windows)]
    fn replacing_a_registered_directory_requires_reauthorization() {
        let root = temporary_root("replacement");
        let project = root.join("project");
        let moved = root.join("project-old");
        fs::create_dir(&project).unwrap();
        fs::write(project.join("identity-anchor"), "old directory identity").unwrap();
        let authorized = windows::authorize_selected_directory(&project).unwrap();
        let canonical_path = authorized.canonical_path.clone();
        let identity = authorized.identity;
        // A directory handle is an identity snapshot, not a path lock; the
        // next operation must re-open the stored name and compare File IDs.
        fs::rename(&project, &moved).unwrap();
        drop(authorized);

        fs::create_dir(&project).unwrap();
        fs::write(project.join("identity-anchor"), "replacement identity").unwrap();

        assert_eq!(
            windows::revalidate_registered_directory(&canonical_path, identity),
            Err(ErrorCode::ProjectPathChanged)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(windows)]
    fn selected_junction_is_rejected_before_it_can_grant_an_identity() {
        let root = temporary_root("junction");
        let project = root.join("project");
        let outside = root.join("outside");
        let junction = project.join("escape");
        fs::create_dir(&project).unwrap();
        fs::create_dir(&outside).unwrap();
        let output = Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&junction)
            .arg(&outside)
            .output()
            .unwrap();
        assert!(output.status.success(), "junction fixture creation failed");

        assert_eq!(
            windows::authorize_selected_directory(&junction),
            Err(ErrorCode::ProjectPathUnsafe)
        );
        fs::remove_dir(&junction).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn access_denied_and_missing_native_errors_have_fixed_public_mappings() {
        #[cfg(windows)]
        {
            assert_eq!(
                windows::map_native_error(std::io::Error::from_raw_os_error(5)),
                ErrorCode::ProjectAccessDenied
            );
            assert_eq!(
                windows::map_native_error(std::io::Error::from_raw_os_error(2)),
                ErrorCode::ProjectPathUnavailable
            );
        }
    }
}
