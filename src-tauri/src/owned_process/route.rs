//! A per-candidate, application-owned relay. The CLI never receives the site
//! secret. No system/environment proxy, redirect, dynamic destination or CLI
//! supplied header can change the bound HTTPS origin.
use super::{Cancel, Failure, Result, Selection};
use crate::credentials::SecretBuffer;
use std::{
    io::{Read, Write},
    net::{IpAddr, SocketAddr, TcpListener, TcpStream, ToSocketAddrs},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use url::Url;

const MAX_BYTES: usize = 2 * 1024 * 1024;
#[derive(Default)]
pub struct Evidence {
    pub requests: AtomicUsize,
    pub verified: AtomicUsize,
    pub unsafe_route: AtomicBool,
}

pub struct Route {
    pub base_url: String,
    pub token: String,
    pub evidence: Arc<Evidence>,
    stop: Cancel,
    worker: Option<JoinHandle<()>>,
}

impl Route {
    pub fn start(
        selection: &Selection,
        secret: SecretBuffer,
        cancel: Cancel,
        timeout: Duration,
    ) -> Result<Self> {
        let url = selection.validate_endpoint()?;
        let host = url.host_str().ok_or(Failure::Unsafe)?.to_owned();
        let addresses = resolve(&url, selection.fixture, &cancel)?;
        let mut target = url.clone();
        target.set_path(&format!("{}/responses", url.path().trim_end_matches('/')));
        let builder = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(timeout)
            .resolve_to_addrs(&host, &addresses);
        #[cfg(test)]
        let builder = if selection.fixture {
            if let Some(pem) = &selection.fixture_ca {
                builder.add_root_certificate(
                    reqwest::Certificate::from_pem(pem).map_err(|_| Failure::Unsafe)?,
                )
            } else {
                builder
            }
        } else {
            builder
        };
        let client = builder.build().map_err(|_| Failure::Unsafe)?;
        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|_| Failure::Unsafe)?;
        listener
            .set_nonblocking(true)
            .map_err(|_| Failure::Unsafe)?;
        let address = listener.local_addr().map_err(|_| Failure::Unsafe)?;
        let token = super::random_token()?;
        let worker_token = token.clone();
        let stop = Cancel::default();
        let worker_stop = stop.clone();
        let evidence = Arc::new(Evidence::default());
        let observed = evidence.clone();
        let selection = selection.clone();
        let worker = thread::spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                observed.unsafe_route.store(true, Ordering::Release);
                return;
            };
            while !worker_stop.is_cancelled()
                && !cancel.is_cancelled()
                && !observed.unsafe_route.load(Ordering::Acquire)
            {
                match listener.accept() {
                    Ok((mut stream, peer)) => {
                        if !peer.ip().is_loopback() {
                            continue;
                        }
                        let result = receive(&mut stream, address, &worker_token, &selection, &worker_stop, &cancel, timeout)
                            .and_then(|body| {
                                runtime.block_on(async {
                                    tokio::select! {
                                    value = forward(&client, target.clone(), body, &secret, &observed, &selection.model) => value,
                                        _ = cancelled(&worker_stop, &cancel) => Err(Failure::Cancelled),
                                    }
                                })
                            });
                        match result {
                            Ok(bytes) => {
                                let header = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                    bytes.len()
                                );
                                let _ = stream
                                    .write_all(header.as_bytes())
                                    .and_then(|_| stream.write_all(&bytes));
                            }
                            Err(failure) => {
                                if matches!(failure, Failure::Unsafe) {
                                    observed.unsafe_route.store(true, Ordering::Release);
                                }
                                // Neither upstream text nor secret-bearing HTTP headers are returned.
                                let _ = stream.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => {
                        observed.unsafe_route.store(true, Ordering::Release);
                        break;
                    }
                }
            }
        });
        Ok(Self {
            base_url: format!("http://{address}/v1"),
            token,
            evidence,
            stop,
            worker: Some(worker),
        })
    }
    pub fn close(&mut self) -> Result<()> {
        self.stop.cancel();
        if let Some(worker) = self.worker.take() {
            worker.join().map_err(|_| Failure::Cleanup)?;
        }
        Ok(())
    }
}
impl Drop for Route {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
async fn cancelled(stop: &Cancel, cancel: &Cancel) {
    while !stop.is_cancelled() && !cancel.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn resolve(url: &Url, fixture: bool, cancel: &Cancel) -> Result<Vec<SocketAddr>> {
    let host = url.host_str().ok_or(Failure::Unsafe)?.to_owned();
    let port = url.port_or_known_default().ok_or(Failure::Unsafe)?;
    // A fixed resolution is used by the upstream client for the whole candidate.
    // No subsequent DNS change can redirect a running candidate to a local proxy.
    let (sender, receiver) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let result = (host.as_str(), port)
            .to_socket_addrs()
            .map(|addresses| addresses.collect::<Vec<_>>())
            .map_err(|_| Failure::Unsafe);
        let _ = sender.send(result);
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let addresses = loop {
        cancel.check()?;
        if Instant::now() >= deadline {
            return Err(Failure::Unsafe);
        }
        match receiver.recv_timeout(Duration::from_millis(25)) {
            Ok(result) => break result?,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => return Err(Failure::Unsafe),
        }
    };
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|a| !public_address(a.ip()) && !(fixture && a.ip().is_loopback()))
    {
        return Err(Failure::Unsafe);
    }
    Ok(addresses)
}
fn public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_broadcast()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && a != 0
                && a < 224
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 192 && b == 0 && (c == 0 || c == 2))
                && !(a == 198 && (b == 18 || b == 19 || b == 51))
                && !(a == 203 && b == 0 && c == 113)
        }
        IpAddr::V6(ip) => {
            // Accept global unicast only; reject mapped IPv4, ULA, link local,
            // transition/special-use and documentation ranges conservatively.
            let s = ip.segments();
            (s[0] & 0xe000) == 0x2000
                && s[0] != 0x2002
                && !(s[0] == 0x2001 && (s[1] < 0x200 || s[1] == 0xdb8))
        }
    }
}

fn receive(
    stream: &mut TcpStream,
    address: SocketAddr,
    token: &str,
    selection: &Selection,
    stop: &Cancel,
    cancel: &Cancel,
    timeout: Duration,
) -> Result<Vec<u8>> {
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .map_err(|_| Failure::Unsafe)?;
    stream
        .set_write_timeout(Some(Duration::from_millis(500)))
        .map_err(|_| Failure::Unsafe)?;
    let until = Instant::now() + timeout;
    let mut bytes = Vec::new();
    let mut expected = None;
    let mut header_end = None;
    loop {
        if stop.is_cancelled() || cancel.is_cancelled() {
            return Err(Failure::Cancelled);
        }
        if Instant::now() >= until {
            return Err(Failure::Timeout);
        }
        let mut chunk = [0; 8192];
        match stream.read(&mut chunk) {
            Ok(0) => return Err(Failure::Unsafe),
            Ok(len) => bytes.extend_from_slice(&chunk[..len]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(_) => return Err(Failure::Unsafe),
        }
        if bytes.len() > MAX_BYTES {
            return Err(Failure::Unsafe);
        }
        if header_end.is_none() {
            if let Some(index) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                if index > 16 * 1024 {
                    return Err(Failure::Unsafe);
                }
                let header = std::str::from_utf8(&bytes[..index]).map_err(|_| Failure::Unsafe)?;
                let mut lines = header.split("\r\n");
                if lines.next() != Some("POST /v1/responses HTTP/1.1") {
                    return Err(Failure::Unsafe);
                }
                let mut length = None;
                let mut auth = None;
                let mut host = None;
                for line in lines {
                    let (name, value) = line.split_once(':').ok_or(Failure::Unsafe)?;
                    let slot = match name.to_ascii_lowercase().as_str() {
                        "content-length" => &mut length,
                        "authorization" => &mut auth,
                        "host" => &mut host,
                        "transfer-encoding" => return Err(Failure::Unsafe),
                        _ => continue,
                    };
                    if slot.replace(value.trim().to_owned()).is_some() {
                        return Err(Failure::Unsafe);
                    }
                }
                if auth.as_deref() != Some(format!("Bearer {token}").as_str())
                    || host.as_deref() != Some(address.to_string().as_str())
                {
                    return Err(Failure::Unsafe);
                }
                let length = length
                    .ok_or(Failure::Unsafe)?
                    .parse::<usize>()
                    .map_err(|_| Failure::Unsafe)?;
                if length > MAX_BYTES - index - 4 {
                    return Err(Failure::Unsafe);
                }
                header_end = Some(index + 4);
                expected = Some(index + 4 + length);
            } else if bytes.len() > 16 * 1024 {
                return Err(Failure::Unsafe);
            }
        }
        if let Some(expected) = expected {
            if bytes.len() > expected {
                return Err(Failure::Unsafe);
            }
            if bytes.len() == expected {
                break;
            }
        }
    }
    let body = bytes.split_off(header_end.ok_or(Failure::Unsafe)?);
    let mut value: serde_json::Value =
        serde_json::from_slice(&body).map_err(|_| Failure::Unsafe)?;
    if value.get("model").and_then(|v| v.as_str()) != Some(selection.model.as_str())
        || value.pointer("/reasoning/effort").and_then(|v| v.as_str())
            != Some(selection.effort.as_str())
    {
        return Err(Failure::Unsafe);
    }
    // A connection probe never advertises executable tools to the upstream.
    value["tools"] = serde_json::json!([]);
    value["tool_choice"] = serde_json::json!("none");
    serde_json::to_vec(&value).map_err(|_| Failure::Unsafe)
}

async fn forward(
    client: &reqwest::Client,
    target: Url,
    body: Vec<u8>,
    secret: &SecretBuffer,
    evidence: &Evidence,
    model: &str,
) -> Result<Vec<u8>> {
    let mut header = reqwest::header::HeaderValue::from_bytes(
        &[b"Bearer ".as_slice(), secret.as_bytes()].concat(),
    )
    .map_err(|_| Failure::Unsafe)?;
    header.set_sensitive(true);
    evidence.requests.fetch_add(1, Ordering::AcqRel);
    let mut response = client
        .post(target.clone())
        .header(reqwest::header::AUTHORIZATION, header)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::ACCEPT, "text/event-stream")
        .body(body)
        .send()
        .await
        .map_err(|_| Failure::TestFailed)?;
    if response.url() != &target || response.status().is_redirection() {
        return Err(Failure::Unsafe);
    }
    if !response.status().is_success() {
        return Err(Failure::TestFailed);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Failure::TestFailed)? {
        if bytes.len() + chunk.len() > MAX_BYTES {
            return Err(Failure::TestFailed);
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes
        .windows(secret.as_bytes().len())
        .any(|w| w == secret.as_bytes())
    {
        return Err(Failure::TestFailed);
    }
    validate_probe_response(&bytes, model)?;
    evidence.verified.fetch_add(1, Ordering::AcqRel);
    Ok(bytes)
}

/// Only text/reasoning output can reach this configuration-only CLI session.
/// A malicious provider cannot smuggle a shell/function call into a probe.
fn validate_probe_response(bytes: &[u8], model: &str) -> Result<()> {
    fn safe_types(value: &serde_json::Value) -> bool {
        match value {
            serde_json::Value::Object(object) => {
                if let Some(kind) = object.get("type").and_then(|v| v.as_str()) {
                    if !matches!(
                        kind,
                        "response.created"
                            | "response.in_progress"
                            | "response.output_item.added"
                            | "response.content_part.added"
                            | "response.output_text.delta"
                            | "response.output_text.done"
                            | "response.content_part.done"
                            | "response.output_item.done"
                            | "response.completed"
                            | "response.reasoning_summary_part.added"
                            | "response.reasoning_summary_part.done"
                            | "response.reasoning_summary_text.delta"
                            | "response.reasoning_summary_text.done"
                            | "message"
                            | "reasoning"
                            | "output_text"
                            | "summary_text"
                    ) {
                        return false;
                    }
                }
                object.values().all(safe_types)
            }
            serde_json::Value::Array(array) => array.iter().all(safe_types),
            _ => true,
        }
    }
    let text = std::str::from_utf8(bytes).map_err(|_| Failure::TestFailed)?;
    let mut completed = false;
    for line in text.lines().filter_map(|line| line.strip_prefix("data:")) {
        let value: serde_json::Value =
            serde_json::from_str(line.trim()).map_err(|_| Failure::TestFailed)?;
        if !safe_types(&value) {
            return Err(Failure::TestFailed);
        }
        if let Some(actual) = value.pointer("/response/model").and_then(|v| v.as_str()) {
            if actual != model {
                return Err(Failure::Unsafe);
            }
        }
        if value.get("type").and_then(|v| v.as_str()) == Some("response.completed") {
            completed = true;
        }
    }
    if !completed {
        return Err(Failure::TestFailed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_private_shared_and_transition_addresses() {
        for text in [
            "127.0.0.1",
            "10.0.0.1",
            "192.168.1.1",
            "169.254.1.1",
            "100.64.0.1",
            "0.0.0.1",
            "::1",
            "::ffff:8.8.8.8",
            "2002:808:808::1",
            "2001:db8::1",
            "fc00::1",
        ] {
            assert!(
                !public_address(text.parse().unwrap()),
                "unsafe address allowed"
            );
        }
        assert!(public_address("8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn probe_response_cannot_execute_tools_or_claim_a_different_model() {
        let text = b"data: {\"type\":\"response.completed\",\"response\":{\"model\":\"selected\",\"output\":[{\"type\":\"message\"}]}}\n\n";
        assert!(validate_probe_response(text, "selected").is_ok());
        assert_eq!(
            validate_probe_response(text, "other").unwrap_err(),
            Failure::Unsafe
        );
        let tool = b"data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"function_call\",\"name\":\"shell\"}}\n\n";
        assert!(validate_probe_response(tool, "selected").is_err());
        assert!(validate_probe_response(b"data: not-json\n\n", "selected").is_err());
        assert!(validate_probe_response(b"", "selected").is_err());
    }

    #[test]
    fn relay_rejects_forged_auth_authority_model_effort_and_chunked_requests_before_network() {
        for mode in [
            "auth",
            "host",
            "path",
            "model",
            "effort",
            "chunked",
            "duplicate-length",
        ] {
            let upstream = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            upstream.set_nonblocking(true).unwrap();
            let endpoint = format!("http://{}/v1", upstream.local_addr().unwrap());
            let mut selected = super::super::tests::selection(&endpoint);
            selected.fixture = true;
            let mut route = Route::start(
                &selected,
                SecretBuffer::new(b"synthetic-isolation-test".to_vec()),
                Cancel::default(),
                Duration::from_secs(2),
            )
            .unwrap();
            let address = Url::parse(&route.base_url)
                .unwrap()
                .socket_addrs(|| None)
                .unwrap()[0];
            let token = if mode == "auth" {
                "forged"
            } else {
                route.token.as_str()
            };
            let host = if mode == "host" {
                "other.invalid".to_owned()
            } else {
                address.to_string()
            };
            let path = if mode == "path" {
                "/v1/other"
            } else {
                "/v1/responses"
            };
            let body = serde_json::json!({"model":if mode == "model" { "other" } else { selected.model.as_str() },
                "reasoning":{"effort":if mode == "effort" { "high" } else { "medium" }}}).to_string();
            let extra = if mode == "chunked" {
                "Transfer-Encoding: chunked\r\n"
            } else if mode == "duplicate-length" {
                "Content-Length: 1\r\n"
            } else {
                ""
            };
            let request = format!(
                "POST {path} HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n{extra}\r\n{body}",
                body.len()
            );
            let mut stream = TcpStream::connect(address).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream.write_all(request.as_bytes()).unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            assert!(response.starts_with("HTTP/1.1 502"));
            route.close().unwrap();
            assert_eq!(route.evidence.requests.load(Ordering::Acquire), 0);
            assert!(route.evidence.unsafe_route.load(Ordering::Acquire));
            assert!(
                matches!(upstream.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock)
            );
        }
    }
}
