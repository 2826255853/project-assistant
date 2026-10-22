//! S1-06: private, bounded stdio candidate adapter. No task/IPC entry is exposed.
//! The future profile service supplies a revision-bound application profile.
#![allow(dead_code)] // Internal integration surface is consumed by step 1.8.
mod home;
mod route;
#[cfg(test)]
mod tests;
#[cfg(windows)]
mod windows;

use crate::{
    credentials::{self, SecretBuffer},
    ipc::ErrorCode,
};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tauri::Manager;
use url::Url;

const PROVIDER: &str = "project_assistant_owned";
pub(crate) const ADAPTER_VERSION: &str = "owned-stdio-relay-v1";
const MAX_FRAME: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Failure {
    Unsafe,
    Endpoint,
    Executable,
    Protocol,
    Timeout,
    Disconnected,
    Cancelled,
    Cleanup,
    TestFailed,
}
type Result<T> = std::result::Result<T, Failure>;
impl Failure {
    fn code(self) -> ErrorCode {
        match self {
            Self::Unsafe => ErrorCode::ProviderSharedRouteUnsafe,
            Self::Endpoint => ErrorCode::ProviderEndpointInvalid,
            Self::Executable => ErrorCode::CodexNotInstalled,
            Self::Protocol => ErrorCode::CodexVersionUnsupported,
            Self::Timeout => ErrorCode::CodexRequestTimeout,
            Self::Disconnected => ErrorCode::CodexDisconnected,
            Self::Cancelled => ErrorCode::ProcessCancelled,
            Self::Cleanup => ErrorCode::ProcessExitFailed,
            Self::TestFailed => ErrorCode::ProviderTestFailed,
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct Cancel(Arc<AtomicBool>);
impl Cancel {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(Failure::Cancelled)
        } else {
            Ok(())
        }
    }
}
fn random_token() -> Result<String> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|_| Failure::Unsafe)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn file_hash(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|_| Failure::Executable)?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|_| Failure::Executable)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

/// Constructed by backend executable selection, never from a webview spawn
/// request or a PATH lookup. Hash changes require a new explicit selection.
pub(crate) struct Executable {
    path: PathBuf,
    hash: String,
    _pin: File,
}
impl Executable {
    pub fn inspect(path: &Path) -> std::result::Result<Self, ErrorCode> {
        let path = home::check_path(path).map_err(|_| ErrorCode::CodexNotInstalled)?;
        if !path.is_file() || path.extension().and_then(|v| v.to_str()) != Some("exe") {
            return Err(ErrorCode::CodexNotInstalled);
        }
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1); // read only: deny replacement/write through spawn and exit
        }
        let pin = options
            .open(&path)
            .map_err(|_| ErrorCode::CodexNotInstalled)?;
        let hash = file_hash(&path).map_err(Failure::code)?;
        Ok(Self {
            path,
            hash,
            _pin: pin,
        })
    }
    fn verify(&self) -> Result<()> {
        home::check_path(&self.path)?;
        if file_hash(&self.path)? != self.hash {
            return Err(Failure::Executable);
        }
        Ok(())
    }
}

#[derive(Clone, Serialize)]
struct Selection {
    profile_id: String,
    config_revision: i64,
    credential_revision: i64,
    endpoint: String,
    origin: String,
    model: String,
    effort: String,
    #[serde(skip)]
    fixture: bool,
    #[cfg(test)]
    #[serde(skip)]
    fixture_ca: Option<Vec<u8>>,
}
impl Selection {
    fn validate_endpoint(&self) -> Result<Url> {
        let url = Url::parse(&self.endpoint).map_err(|_| Failure::Endpoint)?;
        let host = url.host_str().ok_or(Failure::Endpoint)?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || self.endpoint.len() > 2048
            || url.origin().ascii_serialization() != self.origin
            || self.config_revision < 1
            || self.credential_revision < 1
            || self.profile_id.is_empty()
            || self.profile_id.len() > 128
            || !self
                .profile_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            || self.model.is_empty()
            || self.model.len() > 128
            || self.model.chars().any(char::is_control)
            || !matches!(
                self.effort.as_str(),
                "none" | "minimal" | "low" | "medium" | "high" | "xhigh"
            )
            || host == "openai.com"
            || host.ends_with(".openai.com")
        {
            return Err(Failure::Endpoint);
        }
        if self.fixture {
            // Only unit acceptance can construct this branch; no environment or
            // debug build switch can enable plaintext production destinations.
            if !cfg!(test) || host != "127.0.0.1" || !matches!(url.scheme(), "http" | "https") {
                return Err(Failure::Unsafe);
            }
        } else if url.scheme() != "https" {
            return Err(Failure::Endpoint);
        }
        Ok(url)
    }
    fn fingerprint(&self, executable: &Executable) -> Result<String> {
        let mut hash = Sha256::new();
        hash.update(ADAPTER_VERSION);
        hash.update(&executable.hash);
        hash.update(serde_json::to_vec(self).map_err(|_| Failure::Protocol)?);
        Ok(format!("{:x}", hash.finalize()))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProbeEvidence {
    pub fingerprint: String,
    pub requests: usize,
    pub verified_responses: usize,
    pub exit: ExitEvidence,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExitEvidence {
    pub pid: u32,
    pub code: Option<i32>,
    pub forced: bool,
    pub cleaned: bool,
}

#[derive(Default)]
pub(crate) struct OwnedProcessState {
    root: Mutex<Option<PathBuf>>,
    serial: Mutex<()>,
    blocked: AtomicBool,
}
impl OwnedProcessState {
    pub fn initialize(&self, app_data_dir: &Path) -> std::result::Result<(), ErrorCode> {
        let parent = home::check_path(app_data_dir).map_err(Failure::code)?;
        let root = parent.join("owned-processes");
        if !root.exists() {
            #[cfg(windows)]
            windows::private_directory(&root).map_err(Failure::code)?;
        }
        let root = home::check_path(&root).map_err(Failure::code)?;
        home::cleanup_stale(&root).map_err(Failure::code)?;
        *self.root.lock().map_err(|_| ErrorCode::ProcessExitFailed)? = Some(root);
        self.blocked.store(false, Ordering::Release);
        Ok(())
    }
}

/// Internal profile-service integration. The DB supplies every connection field;
/// a caller may only select the profile revision, model and effort. Holding the
/// existing credential-operation lock prevents replacement during the candidate.
pub(crate) fn probe_candidate(
    app: &tauri::AppHandle,
    executable: &Executable,
    profile_id: &str,
    expected_revision: i64,
    model: &str,
    effort: &str,
    cancel: Cancel,
) -> std::result::Result<ProbeEvidence, ErrorCode> {
    let state = app.state::<OwnedProcessState>();
    let _serial = state
        .serial
        .lock()
        .map_err(|_| ErrorCode::ProcessExitFailed)?;
    if state.blocked.load(Ordering::Acquire) {
        return Err(ErrorCode::ProcessExitFailed);
    }
    let credentials = app.state::<credentials::CredentialsState>();
    let _credential = credentials
        .operation
        .lock()
        .map_err(|_| ErrorCode::CredentialsStorageFailed)?;
    let selection = app
        .state::<crate::storage::StorageState>()
        .with_storage(|db| db.profiles().process_profile(profile_id))
        .map_err(|_| ErrorCode::SettingsStorageFailed)?
        .ok_or(ErrorCode::CredentialProfileNotFound)?;
    if selection.config_revision != expected_revision {
        return Err(ErrorCode::RevisionConflict);
    }
    if !selection.enabled
        || selection.adapter_kind != "self_managed"
        || selection.external_profile_id.is_some()
        || selection.source_kind != "relay"
        || selection.wire_api != "responses"
        || selection.auth_kind != "api_key"
    {
        return Err(ErrorCode::ProviderSharedRouteUnsafe);
    }
    let selected = Selection {
        profile_id: profile_id.into(),
        config_revision: selection.config_revision,
        credential_revision: selection.credential_revision,
        endpoint: selection.endpoint,
        origin: selection.endpoint_origin,
        model: model.into(),
        effort: effort.into(),
        fixture: false,
        #[cfg(test)]
        fixture_ca: None,
    };
    selected.validate_endpoint().map_err(Failure::code)?;
    // Lock already held, so use the locked resolver rather than recursively
    // acquiring it. Origin/revision are checked again by the credential service.
    let secret = credentials::resolve_for_owned_locked(
        app,
        profile_id,
        selected.credential_revision,
        &selected.origin,
    )?;
    let root = state
        .root
        .lock()
        .map_err(|_| ErrorCode::ProcessExitFailed)?
        .clone()
        .ok_or(ErrorCode::ProviderSharedRouteUnsafe)?;
    let result = run_probe(
        &root,
        executable,
        selected,
        secret,
        None,
        cancel,
        Duration::from_secs(90),
    );
    if matches!(result, Err(Failure::Cleanup)) {
        state.blocked.store(true, Ordering::Release);
    }
    result.map_err(Failure::code)
}

fn run_probe(
    root: &Path,
    executable: &Executable,
    selection: Selection,
    secret: SecretBuffer,
    project: Option<&Path>,
    cancel: Cancel,
    timeout: Duration,
) -> Result<ProbeEvidence> {
    cancel.check()?;
    selection.validate_endpoint()?;
    if let Some(project) = project {
        verify_project(project)?;
    }
    let mut candidate = Candidate::start(
        root, executable, selection, secret, project, cancel, timeout,
    )?;
    let result = candidate.probe();
    let exit = candidate.close()?;
    if result.is_ok() && (exit.forced || exit.code != Some(0)) {
        return Err(Failure::Cleanup);
    }
    result.map(
        |(fingerprint, requests, verified_responses)| ProbeEvidence {
            fingerprint,
            requests,
            verified_responses,
            exit,
        },
    )
}

/// Until project configuration support is separately verified, any .codex layer
/// in cwd or an ancestor is rejected, including on-request/reparse directories.
fn verify_project(project: &Path) -> Result<PathBuf> {
    let project = home::check_path(project)?;
    if !project.is_dir() {
        return Err(Failure::Unsafe);
    }
    for ancestor in project.ancestors() {
        match fs::symlink_metadata(ancestor.join(".codex")) {
            Ok(_) => return Err(Failure::Unsafe),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(Failure::Unsafe),
        }
        if fs::symlink_metadata(ancestor.join(".git")).is_ok() {
            break;
        }
    }
    Ok(project)
}

fn configuration(selection: &Selection, base_url: &str) -> String {
    // JSON string encoding is a subset of TOML basic strings for these validated
    // values. No external stanza, MCP, hook, plugin or trust data is imported.
    format!(
        "model_provider = \"{PROVIDER}\"\nmodel = {}\nmodel_reasoning_effort = {}\napproval_policy = \"on-request\"\nsandbox_mode = \"read-only\"\ncli_auth_credentials_store = \"file\"\nweb_search = \"disabled\"\n[history]\npersistence = \"none\"\n[analytics]\nenabled = false\n[feedback]\nenabled = false\n[model_providers.{PROVIDER}]\nname = \"Project Assistant owned route\"\nbase_url = {}\nwire_api = \"responses\"\nenv_key = \"PA_OWNED_ROUTE_TOKEN\"\nrequires_openai_auth = false\nsupports_websockets = false\nrequest_max_retries = 0\nstream_max_retries = 0\nstream_idle_timeout_ms = 10000\n",
        json!(selection.model),
        json!(selection.effort),
        json!(base_url)
    )
}

fn stdout_frames(stdout: impl Read, sender: mpsc::SyncSender<Result<Value>>) {
    let mut reader = BufReader::new(stdout);
    let mut total = 0;
    loop {
        let mut line = Vec::new();
        let read = (&mut reader)
            .take(MAX_FRAME + 1)
            .read_until(b'\n', &mut line);
        total += line.len() as u64;
        let value = match read {
            Ok(0) => break,
            Ok(_)
                if total <= MAX_FRAME
                    && line.len() as u64 <= MAX_FRAME
                    && line.ends_with(b"\n") =>
            {
                serde_json::from_slice(&line).map_err(|_| Failure::Protocol)
            }
            _ => Err(Failure::Protocol),
        };
        let failed = value.is_err();
        if sender.try_send(value).is_err() || failed {
            break;
        }
    }
}
fn stderr_drain(mut stderr: impl Read, sender: mpsc::SyncSender<Result<Value>>) {
    let mut bytes = [0; 8192];
    let mut total = 0;
    while let Ok(count) = stderr.read(&mut bytes) {
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > MAX_FRAME {
            let _ = sender.try_send(Err(Failure::Protocol));
            break;
        }
    }
}

struct Candidate {
    child: Child,
    stdin: Option<ChildStdin>,
    #[cfg(windows)]
    job: Option<windows::Job>,
    home: home::Home,
    route: route::Route,
    messages: Receiver<Result<Value>>,
    readers: Vec<JoinHandle<()>>,
    selection: Selection,
    cwd: PathBuf,
    config_hash: String,
    fingerprint: String,
    cancel: Cancel,
    deadline: Instant,
    next_id: u64,
    terminal: VecDeque<Value>,
    closed: bool,
    stage: &'static str,
    active_turn: Option<(String, String)>,
}
impl Candidate {
    fn start(
        root: &Path,
        executable: &Executable,
        selection: Selection,
        secret: SecretBuffer,
        project: Option<&Path>,
        cancel: Cancel,
        timeout: Duration,
    ) -> Result<Self> {
        executable.verify()?;
        let home = home::Home::create(root)?;
        let cwd = verify_project(project.unwrap_or(&home.path.join("project")))?;
        let route = route::Route::start(&selection, secret, cancel.clone(), timeout)?;
        let config = configuration(&selection, &route.base_url);
        let config_path = home.path.join("codex/config.toml");
        fs::write(&config_path, &config).map_err(|_| Failure::Unsafe)?;
        let config_hash = file_hash(&config_path)?;
        let fingerprint = selection.fingerprint(executable)?;
        let mut command = Command::new(&executable.path);
        command
            .args(["app-server", "--listen", "stdio://"])
            .current_dir(&cwd)
            .env_clear()
            .env("CODEX_HOME", home.path.join("codex"))
            .env("USERPROFILE", home.path.join("user"))
            .env("HOME", home.path.join("user"))
            .env("APPDATA", home.path.join("appdata"))
            .env("LOCALAPPDATA", home.path.join("localappdata"))
            .env("PROGRAMDATA", home.path.join("appdata"))
            .env("TEMP", home.path.join("temp"))
            .env("TMP", home.path.join("temp"))
            .env("PA_OWNED_ROUTE_TOKEN", &route.token)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // No inherited PATH, proxy, API key, provider, Rust logging, Node preload,
        // shell setup or cc-switch variable survives this explicit allowlist.
        let system = std::env::var_os("SystemRoot").ok_or(Failure::Unsafe)?;
        let system = home::check_path(Path::new(&system))?;
        command
            .env("SystemRoot", &system)
            .env("WINDIR", &system)
            .env("PATH", system.join("System32"));
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0004); // CREATE_NO_WINDOW | CREATE_SUSPENDED
        }
        let mut child = command.spawn().map_err(|_| Failure::Executable)?;
        #[cfg(windows)]
        let job = match windows::Job::attach_suspended(&child) {
            Ok(job) => Some(job),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().ok_or(Failure::Protocol)?;
        let stderr = child.stderr.take().ok_or(Failure::Protocol)?;
        let (sender, messages) = mpsc::sync_channel(64);
        let error_sender = sender.clone();
        let output_reader = thread::spawn(move || stdout_frames(stdout, sender));
        let error_reader = thread::spawn(move || stderr_drain(stderr, error_sender));
        Ok(Self {
            child,
            stdin,
            #[cfg(windows)]
            job,
            home,
            route,
            messages,
            readers: vec![output_reader, error_reader],
            selection,
            cwd,
            config_hash,
            fingerprint,
            cancel,
            deadline: Instant::now() + timeout,
            next_id: 0,
            terminal: VecDeque::new(),
            closed: false,
            stage: "created",
            active_turn: None,
        })
    }
    fn guard(&self) -> Result<()> {
        self.cancel.check()?;
        if self.route.evidence.unsafe_route.load(Ordering::Acquire) {
            return Err(Failure::Unsafe);
        }
        if Instant::now() >= self.deadline {
            return Err(Failure::Timeout);
        }
        verify_project(&self.cwd)?;
        if file_hash(&self.home.path.join("codex/config.toml"))? != self.config_hash
            || self.home.path.join("codex/auth.json").exists()
        {
            return Err(Failure::Unsafe);
        }
        Ok(())
    }
    fn send(&mut self, value: Value) -> Result<()> {
        self.guard()?;
        let input = self.stdin.as_mut().ok_or(Failure::Disconnected)?;
        serde_json::to_writer(&mut *input, &value).map_err(|_| Failure::Disconnected)?;
        input
            .write_all(b"\n")
            .and_then(|_| input.flush())
            .map_err(|_| Failure::Disconnected)
    }
    fn receive(&mut self) -> Result<Value> {
        self.receive_until(self.deadline)
    }
    fn receive_until(&mut self, deadline: Instant) -> Result<Value> {
        loop {
            self.guard()?;
            if Instant::now() >= deadline {
                return Err(Failure::Timeout);
            }
            match self.messages.recv_timeout(Duration::from_millis(25)) {
                Ok(value) => return value,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(Failure::Disconnected),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if self
                        .child
                        .try_wait()
                        .map_err(|_| Failure::Disconnected)?
                        .is_some()
                    {
                        return Err(Failure::Disconnected);
                    }
                }
            }
        }
    }
    fn event(&mut self, message: Value) -> Result<()> {
        if let Some(id) = message.get("id") {
            if message
                .get("method")
                .and_then(Value::as_str)
                .is_some_and(|v| v.ends_with("/requestApproval"))
            {
                self.send(json!({"id":id,"result":{"decision":"decline"}}))?;
            } else {
                self.send(
                    json!({"id":id,"error":{"code":-32601,"message":"Unsupported client request"}}),
                )?;
            }
        } else if message.get("method").and_then(Value::as_str) == Some("turn/completed") {
            if self.terminal.len() >= 8 {
                return Err(Failure::Protocol);
            }
            self.terminal.push_back(message);
        }
        Ok(())
    }
    fn rpc(&mut self, method: &str, params: Value) -> Result<Value> {
        let request_deadline = Instant::now() + Duration::from_secs(30);
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"id":id,"method":method,"params":params}))?;
        loop {
            if Instant::now() >= request_deadline {
                return Err(Failure::Timeout);
            }
            let value = self.receive_until(request_deadline)?;
            if value.get("method").is_some() {
                self.event(value)?;
                continue;
            }
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                return Err(Failure::Protocol);
            }
            if value.get("error").is_some() {
                return Err(Failure::TestFailed);
            }
            return value.get("result").cloned().ok_or(Failure::Protocol);
        }
    }
    fn probe(&mut self) -> Result<(String, usize, usize)> {
        self.stage = "initialize";
        self.rpc("initialize", json!({"clientInfo":{"name":"project_assistant_owned","version":"0.1.0"},"capabilities":{"experimentalApi":false}}))?;
        self.send(json!({"method":"initialized","params":{}}))?;
        self.stage = "config";
        let read = self.rpc("config/read", json!({"includeLayers":true,"cwd":self.cwd}))?;
        self.verify_effective_config(&read)?;
        self.stage = "thread";
        let started = self.rpc("thread/start", json!({"cwd":self.cwd,"ephemeral":true,
            "model":self.selection.model,"modelProvider":PROVIDER,"sandbox":"read-only","approvalPolicy":"on-request",
            "config":{"model_reasoning_effort":self.selection.effort}}))?;
        if started.get("model").and_then(Value::as_str) != Some(&self.selection.model)
            || started.get("modelProvider").and_then(Value::as_str) != Some(PROVIDER)
            || started.get("reasoningEffort").and_then(Value::as_str)
                != Some(&self.selection.effort)
            || started
                .get("cwd")
                .and_then(Value::as_str)
                .and_then(|v| Path::new(v).canonicalize().ok())
                .as_ref()
                != Some(&self.cwd)
        {
            return Err(Failure::Unsafe);
        }
        let thread_id = started
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .ok_or(Failure::Protocol)?
            .to_owned();
        // Guard is repeated immediately before the sole network-producing call.
        self.stage = "turn";
        let turn = self.rpc(
            "turn/start",
            json!({"threadId":thread_id,"model":self.selection.model,"effort":self.selection.effort,
            "input":[{"type":"text","text":"Do not use tools. Reply OK.","text_elements":[]}]}),
        )?;
        let turn_id = turn
            .pointer("/turn/id")
            .and_then(Value::as_str)
            .ok_or(Failure::Protocol)?
            .to_owned();
        self.active_turn = Some((thread_id.clone(), turn_id.clone()));
        loop {
            if let Some(index) = self.terminal.iter().position(|event| {
                event.pointer("/params/threadId").and_then(Value::as_str) == Some(&thread_id)
                    && event.pointer("/params/turn/id").and_then(Value::as_str) == Some(&turn_id)
            }) {
                let terminal = self.terminal.remove(index).ok_or(Failure::Protocol)?;
                self.active_turn = None;
                self.guard()?;
                if terminal
                    .pointer("/params/turn/status")
                    .and_then(Value::as_str)
                    != Some("completed")
                {
                    return Err(Failure::TestFailed);
                }
                let count = self.route.evidence.verified.load(Ordering::Acquire);
                if count == 0 {
                    return Err(Failure::Unsafe);
                }
                return Ok((
                    self.fingerprint.clone(),
                    self.route.evidence.requests.load(Ordering::Acquire),
                    count,
                ));
            }
            let value = self.receive()?;
            if value.get("method").is_some() {
                self.event(value)?;
            }
        }
    }
    fn verify_effective_config(&mut self, value: &Value) -> Result<()> {
        self.stage = "config-selection";
        let config = value.get("config").ok_or(Failure::Protocol)?;
        if config.get("model_provider").and_then(Value::as_str) != Some(PROVIDER)
            || config.get("model").and_then(Value::as_str) != Some(&self.selection.model)
            || config.get("model_reasoning_effort").and_then(Value::as_str)
                != Some(&self.selection.effort)
        {
            return Err(Failure::Unsafe);
        }
        self.stage = "config-provider";
        let provider = config
            .pointer(&format!("/model_providers/{PROVIDER}"))
            .ok_or(Failure::Unsafe)?;
        if provider.get("base_url").and_then(Value::as_str) != Some(&self.route.base_url)
            || provider.get("wire_api").and_then(Value::as_str) != Some("responses")
            || provider.get("env_key").and_then(Value::as_str) != Some("PA_OWNED_ROUTE_TOKEN")
            || provider
                .get("requires_openai_auth")
                .and_then(Value::as_bool)
                != Some(false)
        {
            return Err(Failure::Unsafe);
        }
        self.stage = "config-extensions";
        for key in ["mcp_servers", "plugins"] {
            if config
                .get(key)
                .is_some_and(|v| !v.is_null() && v.as_object().is_none_or(|o| !o.is_empty()))
            {
                return Err(Failure::Unsafe);
            }
        }
        if config
            .get("notify")
            .is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()))
        {
            return Err(Failure::Unsafe);
        }
        // Loaded system/requirements/project layers cannot silently inject a
        // second route or executable extension. Only our exact user file and
        // the empty command-line layer are accepted.
        self.stage = "config-layers";
        let layers = value
            .get("layers")
            .and_then(Value::as_array)
            .ok_or(Failure::Protocol)?;
        for layer in layers {
            let name = layer
                .pointer("/name/type")
                .and_then(Value::as_str)
                .ok_or(Failure::Protocol)?;
            match name {
                "user" => {
                    let file = layer
                        .pointer("/name/file")
                        .and_then(Value::as_str)
                        .ok_or(Failure::Protocol)?;
                    if Path::new(file).canonicalize().ok()
                        != self.home.path.join("codex/config.toml").canonicalize().ok()
                    {
                        return Err(Failure::Unsafe);
                    }
                }
                "sessionFlags" | "commandLine" | "system"
                    if layer
                        .get("config")
                        .and_then(Value::as_object)
                        .is_some_and(|o| o.is_empty()) => {}
                _ => return Err(Failure::Unsafe),
            }
        }
        Ok(())
    }
    fn close(&mut self) -> Result<ExitEvidence> {
        self.close_inspecting(|_| {})
    }
    fn close_inspecting(&mut self, inspect: impl FnOnce(&Path)) -> Result<ExitEvidence> {
        if self.closed {
            return Err(Failure::Cleanup);
        }
        if let (Some((thread_id, turn_id)), Some(stdin)) =
            (self.active_turn.take(), self.stdin.as_mut())
        {
            self.next_id += 1;
            let message = json!({"id":self.next_id,"method":"turn/interrupt","params":{"threadId":thread_id,"turnId":turn_id}});
            let _ = serde_json::to_writer(&mut *stdin, &message)
                .and_then(|_| stdin.write_all(b"\n").map_err(serde_json::Error::io));
        }
        self.route.close()?;
        self.stdin.take();
        let started = Instant::now();
        let mut forced = false;
        let status = loop {
            if let Some(status) = self.child.try_wait().map_err(|_| Failure::Cleanup)? {
                break status;
            }
            if started.elapsed() > Duration::from_secs(2) {
                forced = true;
                #[cfg(windows)]
                if let Some(job) = &self.job {
                    job.terminate();
                }
                let _ = self.child.kill();
                break self.child.wait().map_err(|_| Failure::Cleanup)?;
            }
            thread::sleep(Duration::from_millis(10));
        };
        // Close the Job even after a graceful main-process exit; no descendant
        // can retain stdout/stderr or an owned resource past this boundary.
        #[cfg(windows)]
        self.job.take();
        self.route.close()?;
        for reader in self.readers.drain(..) {
            reader.join().map_err(|_| Failure::Cleanup)?;
        }
        inspect(&self.home.path);
        self.home.cleanup()?;
        self.closed = true;
        Ok(ExitEvidence {
            pid: self.child.id(),
            code: status.code(),
            forced,
            cleaned: true,
        })
    }
}
impl Drop for Candidate {
    fn drop(&mut self) {
        if !self.closed {
            #[cfg(windows)]
            if let Some(job) = &self.job {
                job.terminate();
            }
            let _ = self.child.kill();
            let _ = self.close();
        }
    }
}
