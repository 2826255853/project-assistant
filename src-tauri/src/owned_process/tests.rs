use super::*;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let directory =
            std::env::temp_dir().join(format!("pa-owned-test-{}", random_token().unwrap()));
        windows::private_directory(&directory).unwrap();
        Self(directory)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
pub(super) fn selection(endpoint: &str) -> Selection {
    Selection {
        profile_id: "isolation-profile".into(),
        config_revision: 2,
        credential_revision: 1,
        endpoint: endpoint.into(),
        origin: Url::parse(endpoint).unwrap().origin().ascii_serialization(),
        model: "gpt-5.1-codex-mini".into(),
        effort: "medium".into(),
        fixture: false,
        fixture_ca: None,
    }
}
#[test]
fn endpoint_policy_cannot_be_enabled_by_runtime_environment() {
    for endpoint in [
        "http://example.com/v1",
        "https://key@example.com/v1",
        "https://example.com/v1?key=secret",
        "https://example.com/v1#secret",
        "https://api.openai.com/v1",
    ] {
        assert!(selection(endpoint).validate_endpoint().is_err());
    }
    let mut selected = selection("https://example.com/v1");
    assert!(selected.validate_endpoint().is_ok());
    selected.origin = "https://wrong.invalid".into();
    assert!(selected.validate_endpoint().is_err());
    selected = selection("http://127.0.0.1:9123/v1");
    assert!(selected.validate_endpoint().is_err());
    selected.fixture = true;
    assert!(selected.validate_endpoint().is_ok());
}
#[test]
fn homes_are_exclusive_and_cleanup_does_not_adopt_foreign_resources() {
    let fixture = Fixture::new();
    let foreign = fixture.0.join("user-data");
    fs::create_dir(&foreign).unwrap();
    fs::write(foreign.join("keep"), "unrelated").unwrap();
    let mut home = home::Home::create(&fixture.0).unwrap();
    let path = home.path.clone();
    assert_eq!(home::cleanup_stale(&fixture.0).unwrap(), 0);
    assert!(path.exists());
    home.cleanup().unwrap();
    assert!(!path.exists());
    assert!(foreign.join("keep").exists());
}
#[test]
fn stale_owned_home_is_cleaned_without_following_junctions() {
    let fixture = Fixture::new();
    let mut home = home::Home::create(&fixture.0).unwrap();
    let path = home.path.clone();
    home.lock.take();
    assert_eq!(home::cleanup_stale(&fixture.0).unwrap(), 1);
    assert!(!path.exists());
    let mut home = home::Home::create(&fixture.0).unwrap();
    let destination = Fixture::new();
    fs::write(destination.0.join("keep"), "foreign").unwrap();
    let junction = home.path.join("project/link");
    let status = Command::new("C:\\Windows\\System32\\cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&destination.0)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    assert!(home.cleanup().is_err());
    assert!(destination.0.join("keep").exists());
    // Remove only the exact junction itself, then the now-safe owned tree.
    fs::remove_dir(&junction).unwrap();
    home.cleanup().unwrap();
}
#[test]
fn project_layers_and_reparse_paths_fail_closed() {
    let fixture = Fixture::new();
    let project = fixture.0.join("project & 中文");
    fs::create_dir(&project).unwrap();
    fs::create_dir(fixture.0.join(".git")).unwrap();
    assert!(verify_project(&project).is_ok());
    fs::create_dir(fixture.0.join(".codex")).unwrap();
    assert!(matches!(verify_project(&project), Err(Failure::Unsafe)));
}
#[test]
fn configuration_fingerprint_binds_every_selection_revision_and_executable() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("binary"), b"fingerprint-only").unwrap();
    let executable = Executable {
        path: PathBuf::new(),
        hash: "binary-a".into(),
        _pin: File::open(fixture.0.join("binary")).unwrap(),
    };
    let selected = selection("https://example.com/v1");
    let original = selected.fingerprint(&executable).unwrap();
    for index in 0..7 {
        let mut changed = selected.clone();
        match index {
            0 => changed.profile_id.push('x'),
            1 => changed.config_revision += 1,
            2 => changed.credential_revision += 1,
            3 => changed.endpoint.push('/'),
            4 => changed.origin.push('x'),
            5 => changed.model.push('x'),
            _ => changed.effort = "high".into(),
        }
        assert_ne!(original, changed.fingerprint(&executable).unwrap());
    }
    let other = Executable {
        hash: "binary-b".into(),
        ..executable
    };
    assert_ne!(original, selected.fingerprint(&other).unwrap());
    assert!(!configuration(&selected, "http://127.0.0.1:1/v1").contains("auth.json"));
}

#[test]
fn executable_selection_prevents_replacement_until_handle_is_released() {
    let fixture = Fixture::new();
    let file = fixture.0.join("controlled.exe");
    fs::write(&file, b"synthetic-selection-only").unwrap();
    let executable = Executable::inspect(&file).unwrap();
    assert!(fs::OpenOptions::new().write(true).open(&file).is_err());
    assert!(fs::rename(&file, fixture.0.join("replaced.exe")).is_err());
    drop(executable);
    assert!(fs::OpenOptions::new().write(true).open(&file).is_ok());
}

#[test]
fn protocol_frames_and_streams_have_independent_limits_and_raw_errors_never_escape() {
    let mut large_frame = vec![b'x'; MAX_FRAME as usize + 1];
    large_frame.push(b'\n');
    for bytes in [
        b"not-json-secret\n".to_vec(),
        b"{\"id\":1}".to_vec(),
        vec![255, 10],
        large_frame,
    ] {
        let (sender, receiver) = mpsc::sync_channel(64);
        stdout_frames(std::io::Cursor::new(bytes), sender);
        assert_eq!(receiver.recv().unwrap().unwrap_err(), Failure::Protocol);
    }
    let frame = json!({"id":1,"result":"x".repeat(600_000)}).to_string() + "\n";
    let (sender, receiver) = mpsc::sync_channel(64);
    stdout_frames(std::io::Cursor::new(frame.repeat(2).into_bytes()), sender);
    assert!(receiver.recv().unwrap().is_ok());
    assert_eq!(receiver.recv().unwrap().unwrap_err(), Failure::Protocol);
    let (sender, receiver) = mpsc::sync_channel(64);
    stderr_drain(
        std::io::Cursor::new(vec![b'x'; MAX_FRAME as usize + 1]),
        sender,
    );
    assert_eq!(receiver.recv().unwrap().unwrap_err(), Failure::Protocol);
}

#[test]
#[ignore = "private Job lifetime worker, invoked by its parent test"]
fn job_fixture() {
    let Ok(role) = std::env::var("PA_JOB_FIXTURE_ROLE") else {
        return;
    };
    if role == "parent" {
        use std::os::windows::process::CommandExt;
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["owned_process::tests::job_fixture", "--ignored", "--exact"])
            .env("PA_JOB_FIXTURE_ROLE", "descendant")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x0800_0000)
            .spawn()
            .unwrap();
        println!("JOB_DESCENDANT:{}", child.id());
        std::io::stdout().flush().unwrap();
    }
    loop {
        thread::park();
    }
}

#[test]
fn job_owns_early_descendants_and_closing_its_handle_terminates_them() {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0},
        System::Threading::{OpenProcess, WaitForSingleObject},
    };
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "owned_process::tests::job_fixture",
            "--ignored",
            "--exact",
            "--nocapture",
        ])
        .env("PA_JOB_FIXTURE_ROLE", "parent")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(0x0800_0004)
        .spawn()
        .unwrap();
    let job = windows::Job::attach_suspended(&child).unwrap();
    let stdout = child.stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout)
            .lines()
            .map_while(std::result::Result::ok)
        {
            if let Some(id) = line
                .strip_prefix("JOB_DESCENDANT:")
                .and_then(|v| v.parse::<u32>().ok())
            {
                let _ = sender.send(id);
                break;
            }
        }
    });
    let id = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    let descendant = unsafe { OpenProcess(0x0010_0000, 0, id) }; // SYNCHRONIZE
    assert!(!descendant.is_null());
    drop(job); // KILL_ON_JOB_CLOSE, no global PID/process-name termination.
    child.wait().unwrap();
    assert_eq!(
        unsafe { WaitForSingleObject(descendant, 5000) },
        WAIT_OBJECT_0
    );
    unsafe {
        CloseHandle(descendant);
    }
    reader.join().unwrap();
}

/// Invoked explicitly by verify:isolation, not by ordinary offline cargo test.
/// Real CLI + the production Rust adapter + OS-resolved synthetic secrets.
#[test]
#[ignore = "requires two controlled HTTP sites and an explicitly selected real CLI"]
fn actual_cli_route_acceptance() {
    let executable =
        Executable::inspect(Path::new(&std::env::var("PA_ISOLATION_CODEX_BIN").unwrap())).unwrap();
    let root = PathBuf::from(std::env::var("PA_ISOLATION_ROOT").unwrap());
    let endpoint = std::env::var("PA_ISOLATION_ENDPOINT").unwrap();
    let scenario = std::env::var("PA_ISOLATION_SCENARIO").unwrap();
    let key = std::env::var("PA_ISOLATION_SYNTHETIC_KEY").unwrap();
    assert!(key.starts_with("synthetic-isolation-"));
    let mut selected = selection(&endpoint);
    selected.profile_id = format!("isolation-{}", random_token().unwrap());
    selected.fixture = true;
    if scenario != "tls-untrusted" {
        selected.fixture_ca = Some(fs::read(std::env::var("PA_ISOLATION_CA").unwrap()).unwrap());
    }
    let secret = credentials::isolation_fixture_secret(&selected.profile_id, &endpoint, &key);
    let cancel = Cancel::default();
    let mut project = None;
    let project_dir = root.join("project & 中文");
    if scenario == "project" {
        fs::create_dir_all(project_dir.join(".codex")).unwrap();
        fs::write(
            project_dir.join(".codex/config.toml"),
            "model_provider = \"external\"",
        )
        .unwrap();
        project = Some(project_dir.as_path());
    }
    if scenario == "shared" {
        selected.fixture = false;
        selected.endpoint = endpoint.replacen("http:", "https:", 1);
        selected.origin = selected.endpoint.trim_end_matches("/v1").into();
    }
    let timeout = if scenario == "timeout" {
        Duration::from_secs(3)
    } else {
        Duration::from_secs(25)
    };
    let mut candidate = match Candidate::start(
        &root,
        &executable,
        selected,
        secret,
        project,
        cancel.clone(),
        timeout,
    ) {
        Ok(candidate) => candidate,
        Err(error) if scenario == "project" || scenario == "shared" => {
            assert_eq!(error, Failure::Unsafe);
            println!(
                "ISOLATION_EVIDENCE:{}",
                json!({"scenario":scenario,"errorCode":error.code(),"noChildStarted":true,"cleaned":true})
            );
            return;
        }
        Err(error) => panic!("candidate creation failed: {error:?}"),
    };
    if scenario == "cancel" {
        let cancel = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(3));
            cancel.cancel();
        });
    }
    if scenario == "drift" {
        fs::write(
            candidate.home.path.join("codex/config.toml"),
            "model_provider = \"external\"",
        )
        .unwrap();
    }
    let result = candidate.probe();
    let requests = candidate.route.evidence.requests.load(Ordering::Acquire);
    let verified = candidate.route.evidence.verified.load(Ordering::Acquire);
    let fingerprint = candidate.fingerprint.clone();
    fn assert_no_site_key(path: &Path, key: &[u8]) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            assert!(!entry.file_type().unwrap().is_symlink());
            if entry.file_type().unwrap().is_dir() {
                assert_no_site_key(&entry.path(), key);
            } else if entry.file_name() != ".lock" {
                let bytes = fs::read(entry.path()).unwrap();
                assert!(
                    !bytes.windows(key.len()).any(|window| window == key),
                    "site key persisted in owned home"
                );
            }
        }
    }
    // The CLI owns SQLite byte-range locks while alive. Scan after its process
    // and Job exit, immediately before the production cleanup removes the home.
    let closed = candidate
        .close_inspecting(|path| assert_no_site_key(path, key.as_bytes()))
        .unwrap();
    println!(
        "ISOLATION_STATUS:{}",
        json!({"stage":candidate.stage,"exit":closed,"error":result.as_ref().err().map(|e| e.code())})
    );
    match scenario.as_str() {
        "redirect" | "drift" => assert!(
            matches!(result, Err(Failure::Unsafe)),
            "expected unsafe: {:?}",
            result.as_ref().err()
        ),
        "timeout" => assert!(
            matches!(result, Err(Failure::Timeout)),
            "expected timeout: {:?}",
            result.as_ref().err()
        ),
        "cancel" => assert!(
            matches!(result, Err(Failure::Cancelled)),
            "expected cancel: {:?}",
            result.as_ref().err()
        ),
        "unauthorized" | "tls-untrusted" | "echo-secret-error" | "echo-secret-success" => {
            assert!(result.is_err())
        }
        _ => {
            assert!(result.is_ok(), "probe failed: {:?}", result.as_ref().err());
            assert!(verified > 0);
        }
    }
    println!(
        "ISOLATION_EVIDENCE:{}",
        json!({"scenario":scenario,"errorCode":result.err().map(Failure::code),
        "requests":requests,"verifiedResponses":verified,"fingerprint":fingerprint,"exit":closed,"credentialCleanup":true})
    );
}
