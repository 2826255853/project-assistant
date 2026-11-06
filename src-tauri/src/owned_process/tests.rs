use super::*;

#[test]
fn task_change_paths_are_task_relative_and_reject_windows_escape_shapes() {
    let root = Path::new(r"C:\work\项目");
    for (input, expected) in [
        (r"C:\work\项目\src\a.txt", "src/a.txt"),
        ("src/a.txt", "src/a.txt"),
    ] {
        assert_eq!(relative_change_path(root, input).unwrap(), expected);
    }
    for input in [
        r"C:\work\项目-other\a.txt",
        "../outside",
        "/outside",
        r"\\server\share\file",
        "a:stream",
        "src/../file",
        "CON",
        "src/trailing.",
    ] {
        assert!(relative_change_path(root, input).is_err());
    }
}

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
        task_tools: false,
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
fn task_accumulator_enforces_total_output_bounds_and_exposes_only_normalized_events() {
    let mut accumulator = TaskAccumulator::new();
    let mut received = Vec::new();
    let mut callback = |event| {
        received.push(event);
        Ok(None)
    };
    accumulator
        .push(
            TaskEvent::ReplyDelta {
                text: "safe reply".into(),
            },
            &mut callback,
        )
        .unwrap();
    accumulator
        .push(
            TaskEvent::Progress {
                text: "bounded progress".into(),
            },
            &mut callback,
        )
        .unwrap();
    drop(callback);
    assert_eq!(accumulator.reply, "safe reply");
    assert_eq!(accumulator.progress, "bounded progress");
    assert_eq!(received.len(), 2);
    assert_eq!(accumulator.events, received);

    let large = "x".repeat(MAX_TASK_CONTENT);
    let mut callback = |_| Ok(None);
    assert_eq!(
        accumulator.push(TaskEvent::ReplyDelta { text: large }, &mut callback,),
        Err(Failure::Protocol)
    );
}

#[test]
fn queued_task_cancellation_and_failed_cancel_commit_do_not_start_or_cancel_other_work() {
    let owned = Arc::new(OwnedProcessState::default());
    let holder = owned.lock_serial().unwrap();
    let cancel = Cancel::default();
    let worker_owned = owned.clone();
    let worker_cancel = cancel.clone();
    let worker = thread::spawn(move || worker_owned.lock_serial_cancellable(&worker_cancel).err());
    assert_eq!(
        cancel.request_after::<()>(|| Err(ErrorCode::AuditWriteFailed)),
        Err(ErrorCode::AuditWriteFailed)
    );
    assert!(!cancel.is_cancelled());
    let accepted = cancel.request_after(|| Ok((true, true))).unwrap();
    assert!(accepted);
    assert_eq!(worker.join().unwrap(), Some(ErrorCode::ProcessCancelled));
    assert!(!owned.is_blocked());
    drop(holder);
    assert!(owned.lock_serial_cancellable(&Cancel::default()).is_ok());
}

#[test]
fn shutdown_cancels_registered_candidates_waiting_for_serial_and_closes_admission() {
    use std::sync::{Arc, Barrier};
    let owned = Arc::new(OwnedProcessState::default());
    let barrier = Arc::new(Barrier::new(3));
    let held = owned.lock_serial().unwrap();
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let owned = owned.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let cancel = Cancel::default();
                let _registration = owned.register_operation(cancel.clone()).unwrap();
                barrier.wait();
                owned.lock_serial_cancellable(&cancel).err()
            })
        })
        .collect();
    barrier.wait();
    owned.begin_shutdown();
    for worker in workers {
        assert_eq!(worker.join().unwrap(), Some(ErrorCode::ProcessCancelled));
    }
    assert!(owned.register_operation(Cancel::default()).is_err());
    owned.wait_for_shutdown();
    drop(held);
}

#[test]
fn approval_summary_rejects_controls_oversize_and_known_secret_markers() {
    assert!(safe_approval_text(
        "pwsh.exe -NoProfile -Command Get-Location",
        1800
    ));
    for text in [
        "x\nRun-Other",
        "Authorization: Bearer value",
        "sk-sensitive",
        "PA_OWNED_ROUTE_TOKEN",
        "synthetic-sentinel",
        "\u{1b}[2J",
    ] {
        assert!(!safe_approval_text(text, 1800));
    }
    assert!(!safe_approval_text(&"x".repeat(1801), 1800));
}

/// Invoked by `npm run verify:task-gateway` with an explicitly selected real
/// Codex CLI and two controlled HTTPS sites. The site key is always synthetic.
#[test]
#[cfg(windows)]
#[ignore = "requires real Codex CLI and two controlled synthetic HTTPS sites"]
fn actual_cli_task_gateway_acceptance() {
    let executable =
        Executable::inspect(Path::new(&std::env::var("PA_TASK_CODEX_BIN").unwrap())).unwrap();
    let root = PathBuf::from(std::env::var("PA_TASK_GATEWAY_ROOT").unwrap());
    let project = root.join("project 中文");
    fs::create_dir_all(project.join(".git")).unwrap();
    let endpoint = std::env::var("PA_TASK_ENDPOINT").unwrap();
    let profile_id = format!("task-{}", random_token().unwrap());
    let key = std::env::var("PA_TASK_SYNTHETIC_KEY").unwrap();
    assert!(key.starts_with("synthetic-task-"));
    let scenario = std::env::var("PA_TASK_SCENARIO").unwrap();
    let mut selected = selection(&endpoint);
    selected.profile_id = profile_id;
    selected.task_tools = true;
    selected.model = "gpt-5.1-codex-mini".into();
    selected.effort = "medium".into();
    selected.fixture = true;
    if scenario == "project" {
        fs::create_dir(project.join(".codex")).unwrap();
    }
    if scenario != "tls-untrusted" {
        selected.fixture_ca = Some(fs::read(std::env::var("PA_TASK_CA").unwrap()).unwrap());
    }
    let secret = crate::credentials::SecretBuffer::new(key.into_bytes());
    let cancel = Cancel::default();
    let timeout = if scenario == "timeout" {
        Duration::from_secs(4)
    } else {
        Duration::from_secs(25)
    };
    let mut candidate = match Candidate::start(
        &root,
        &executable,
        selected,
        secret,
        Some(&project),
        cancel.clone(),
        timeout,
    ) {
        Ok(candidate) => candidate,
        Err(error) if scenario == "project" => {
            assert_eq!(error, Failure::Unsafe);
            fs::remove_dir_all(&project).unwrap();
            assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
            println!(
                "TASK_GATEWAY_EVIDENCE:{}",
                json!({"scenario":scenario,"noChildStarted":true,"cleaned":true})
            );
            return;
        }
        Err(error) => panic!("candidate startup failed: {error:?}"),
    };
    if scenario == "drift" {
        fs::write(
            candidate.home.path.join("codex/config.toml"),
            "model_provider = \"external\"",
        )
        .unwrap();
    }
    if scenario == "cancel" {
        let cancel = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(2));
            cancel.cancel();
        });
    }
    let mut event_kinds = Vec::new();
    let mut result = candidate.task(
        "S3_04_PROMPT_SENTINEL: report the controlled response only.",
        &mut |event| {
            event_kinds.push(match &event {
                TaskEvent::Started { .. } => "started",
                TaskEvent::Running => "running",
                TaskEvent::ReplyDelta { .. } => "reply_delta",
                TaskEvent::Progress { .. } => "progress",
                TaskEvent::FileChanges { .. } => "file_changes",
                TaskEvent::ApprovalRequired { .. } => "approval_required",
                TaskEvent::ApprovalPoll => "approval_poll",
                TaskEvent::ApprovalExpired => "approval_expired",
                TaskEvent::OperationCompleted { .. } => "operation_completed",
                TaskEvent::ApprovalAcknowledged { .. } => "approval_acknowledged",
                TaskEvent::ApprovalAbandoned { .. } => "approval_abandoned",
                TaskEvent::Completed => "completed",
                TaskEvent::Cancelled => "cancelled",
                TaskEvent::Failed => "failed",
            });
            Ok(None)
        },
    );
    let requests = candidate.route.evidence.requests.load(Ordering::Acquire);
    let verified = candidate.route.evidence.verified.load(Ordering::Acquire);
    if result
        .as_ref()
        .is_ok_and(|run| run.outcome == TaskOutcome::Completed && (requests == 0 || verified == 0))
    {
        result = Err(Failure::Unsafe);
    }
    let reply_bytes = result.as_ref().map_or(0, |run| run.reply.len());
    let reply_marker = result
        .as_ref()
        .is_ok_and(|run| run.reply.contains("CONTROLLED_"));
    let exit = candidate.close().unwrap();
    assert!(exit.cleaned);
    assert!(!exit.forced);
    if scenario == "a" || scenario == "b" {
        assert_eq!(result.as_ref().unwrap().outcome, TaskOutcome::Completed);
        assert!(reply_marker);
        assert!(requests > 0 && verified > 0);
        assert!(event_kinds.contains(&"running"));
        assert!(event_kinds.contains(&"reply_delta"));
        assert!(event_kinds.contains(&"completed"));
    }
    if scenario == "tls-untrusted" {
        assert_eq!(verified, 0);
        assert!(
            !result
                .as_ref()
                .is_ok_and(|run| run.outcome == TaskOutcome::Completed)
        );
    }
    if matches!(scenario.as_str(), "drift" | "timeout" | "cancel") {
        assert!(result.is_err());
    }
    fs::remove_dir_all(&project).unwrap();
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    println!(
        "TASK_GATEWAY_EVIDENCE:{}",
        json!({"scenario":scenario,"requests":requests,"verifiedResponses":verified,
            "replyBytes":reply_bytes,"replyMarker":reply_marker,"eventKinds":event_kinds,
            "exit":exit,"errorCode":result.err().map(Failure::code)})
    );
}

#[test]
#[ignore = "invoked by verify:approvals with a real CLI and synthetic HTTPS"]
fn actual_cli_approval_acceptance() {
    use crate::storage::tasks::{ApprovalDecisionInput, ApprovalRequestInput};
    let executable =
        Executable::inspect(Path::new(&std::env::var("PA_TASK_CODEX_BIN").unwrap())).unwrap();
    let root = PathBuf::from(std::env::var("PA_TASK_GATEWAY_ROOT").unwrap());
    let project = root.join("approval project 中文");
    fs::create_dir_all(project.join(".git")).unwrap();
    let scenario = std::env::var("PA_TASK_SCENARIO").unwrap();
    let mut fixture = crate::storage::tasks::tests::Fixture::new_with_source(Some(project.clone()));
    let session = fixture.session(71);
    let input = fixture.input(&session, 72);
    let task = fixture
        .db
        .tasks()
        .task_start(input, &crate::ipc::correlation_id(), |_, _| Ok(()))
        .unwrap()
        .task;
    let mut revision = 1;
    let mut selected = selection(&std::env::var("PA_TASK_ENDPOINT").unwrap());
    selected.task_tools = true;
    selected.fixture = true;
    selected.fixture_ca = Some(fs::read(std::env::var("PA_TASK_CA").unwrap()).unwrap());
    let key = std::env::var("PA_TASK_SYNTHETIC_KEY").unwrap();
    assert!(key.starts_with("synthetic-task-"));
    let cancel = Cancel::default();
    let mut candidate = Candidate::start(
        &root,
        &executable,
        selected,
        crate::credentials::SecretBuffer::new(key.into_bytes()),
        Some(&project),
        cancel.clone(),
        Duration::from_secs(35),
    )
    .unwrap();
    if scenario == "expire" {
        candidate.approval_ttl = Duration::from_millis(100);
    }
    let mut approval_id = None;
    let mut approved = 0;
    let mut acknowledged = 0;
    let mut resolved = 0;
    let mut executed = 0;
    let owned_pid = candidate.child.id();
    let result = candidate.task("S3_04_PROMPT_SENTINEL: synthetic approval command.", &mut |event| {
        match event {
            TaskEvent::Running if revision == 1 => {
                revision = fixture.db.tasks().task_transition(&task.id, revision, "running", None, &crate::ipc::correlation_id())?.revision;
            }
            TaskEvent::ApprovalRequired { external_request_id, operation_kind, operation_fingerprint, display_summary, .. } => {
                assert_eq!(operation_kind, "command");
                assert!(display_summary.contains("Get-Location"));
                let approval = fixture.db.tasks().approval_create(ApprovalRequestInput {task_id: task.id.clone(), expected_task_revision: revision, external_request_id, operation_kind, operation_fingerprint: operation_fingerprint.clone(), window_label: "main"}, &crate::ipc::correlation_id())?.approval;
                revision += 1;
                approval_id = Some(approval.id.clone());
                if scenario == "exit" {
                    use windows_sys::Win32::{Foundation::CloseHandle, System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE}};
                    let process = unsafe { OpenProcess(PROCESS_TERMINATE, 0, owned_pid) };
                    assert!(!process.is_null());
                    assert_ne!(unsafe { TerminateProcess(process, 1) }, 0);
                    unsafe { CloseHandle(process); }
                    return Ok(None);
                }
                if scenario == "cancel" {
                    fixture.db.tasks().task_cancel(&task.id, revision, &crate::ipc::correlation_id())?;
                    revision += 1;
                    cancel.cancel();
                    return Ok(None);
                }
                if scenario == "expire" { return Ok(None); }
                if scenario == "audit" {
                    fixture.db.connection.execute_batch("CREATE TEMP TRIGGER fail_approval BEFORE INSERT ON audit_events WHEN NEW.event_kind='approval_decision' BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
                }
                let decision = if scenario == "deny" { ApprovalDecisionInput::Deny } else { ApprovalDecisionInput::Approve };
                let result = fixture.db.tasks().approval_decide(&task.id, &approval.id, 1, revision, &operation_fingerprint, "main", decision, &crate::ipc::correlation_id());
                if scenario == "audit" {
                    assert_eq!(result.unwrap_err(), ErrorCode::AuditWriteFailed);
                    return Err(ErrorCode::AuditWriteFailed);
                }
                let decided = result?.approval;
                revision += 1;
                approved += usize::from(decided.decision == "approved");
                assert!(fixture.db.tasks().approval_decide(&task.id, &approval.id, 1, revision - 1, &operation_fingerprint, "main", decision, &crate::ipc::correlation_id()).is_err());
                return Ok(Some(ApprovalReply {approval_id: decided.id, consume_request_id: decided.consume_request_id.unwrap(), accepted: decision == ApprovalDecisionInput::Approve}));
            }
            TaskEvent::ApprovalAcknowledged { approval_id, consume_request_id } => {
                fixture.db.tasks().approval_acknowledge(&approval_id, &consume_request_id, true, &crate::ipc::correlation_id())?;
                acknowledged += 1;
                resolved += 1;
            }
            TaskEvent::ApprovalAbandoned { .. } => return Err(ErrorCode::ApprovalUncertain),
            TaskEvent::OperationCompleted { succeeded } => executed += usize::from(succeeded),
            TaskEvent::ApprovalExpired => {
                fixture.db.tasks().approval_expire(&task.id, approval_id.as_ref().unwrap(), &crate::ipc::correlation_id())?;
                revision += 1;
            }
            _ => {},
        }
        Ok(None)
    });
    let exit = candidate.close().unwrap();
    assert!(exit.cleaned && !exit.forced);
    if scenario != "audit" {
        let current = fixture.db.tasks().task_get(&task.id).unwrap();
        let (status, error) = match scenario.as_str() {
            "cancel" => ("cancelled", None),
            "exit" => ("failed", Some("PA_CODEX_DISCONNECTED")),
            _ => ("completed", None),
        };
        fixture
            .db
            .tasks()
            .task_transition(
                &task.id,
                current.revision,
                status,
                error,
                &crate::ipc::correlation_id(),
            )
            .unwrap();
    }
    let approvals = fixture.db.tasks().approvals_get(&task.id).unwrap();
    assert_eq!(approvals.len(), 1, "must exercise a real CLI approval");
    if scenario == "approve" || scenario == "deny" {
        assert_eq!(result.as_ref().unwrap().outcome, TaskOutcome::Completed);
        assert_eq!(acknowledged, 1);
        assert_eq!(approvals[0].consumption, "acknowledged");
    }
    if scenario == "cancel" || scenario == "audit" || scenario == "exit" {
        assert!(result.is_err());
    }
    if scenario == "expire" {
        assert_eq!(approvals[0].decision, "expired");
    }
    if scenario == "approve" {
        assert_eq!(approved, 1);
    }
    assert_eq!(executed, usize::from(scenario == "approve"));
    let clean = fs::read_dir(&project)
        .unwrap()
        .all(|entry| entry.unwrap().file_name() == ".git");
    assert!(clean);
    fs::remove_dir_all(&project).unwrap();
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    println!(
        "APPROVAL_EVIDENCE:{}",
        json!({"scenario":scenario,"taskStatus":fixture.db.tasks().task_get_internal(&task.id).unwrap().status,"approved":approved,"executedSuccessfully":executed,"acknowledged":acknowledged,"resolved":resolved,"decision":approvals[0].decision,"consumption":approvals[0].consumption,"projectUnchanged":clean,"exit":exit,"error":result.err().map(Failure::code)})
    );
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
