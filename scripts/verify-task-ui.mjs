import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, readdir, rename, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { randomUUID } from "node:crypto";
import { createServer } from "node:net";
import { setTimeout as delay } from "node:timers/promises";
import { controlledSite } from "./codex/controlled-site.mjs";
import { discoverCodex } from "./codex/environment.mjs";

assert.equal(process.platform, "win32");
assert.ok(process.env.npm_execpath, "Run with npm run verify:task-ui");
const root = resolve(import.meta.dirname, "..");
const output = join(root, "artifacts/phase-3-6");
await mkdir(output, {recursive: true});
const temp = await mkdtemp(join(tmpdir(), "pa-task-ui-"));
const secret = `synthetic-ui-${randomUUID()}`;
const report = {step: "3.6", date: new Date().toISOString(), timezone: "Asia/Shanghai", passed: false,
  syntheticDataOnly: true, officialSource: "reserved-not-tested", checks: [], desktop: [],
  limits: ["Real CLI and WebView with synthetic HTTPS replies/print-only approvals. No real Key/model usage.",
    "Diff/eviction/gap fixtures are explicit test-only injections, separate from real task evidence.",
    "Physical IME, tray, monitors and DPI remain manual; overlay task submission remains S3-07."]};
const npmArgs = args => [process.env.npm_execpath, ...args];
const children = new Set();
async function run(label, exe, args, env = {}) {
  let text = "";
  const child = spawn(exe, args, {cwd: root, windowsHide: true, env: {...process.env, CARGO_TERM_COLOR: "never", ...env}, stdio: ["ignore", "pipe", "pipe"]});
  children.add(child);
  for (const stream of [child.stdout, child.stderr]) stream.on("data", b => { text += b; });
  const code = await new Promise((done, reject) => { child.on("error", reject); child.on("close", done); });
  children.delete(child);
  assert.ok(!text.includes(secret), `${label}: Key leaked`);
  await writeFile(join(output, `${label}.log`), text);
  assert.equal(code, 0, `${label} failed; see its artifact log`);
  report.checks.push(label); console.log(`${label}: passed`);
  return text;
}
async function freePort() {
  const server = createServer(); await new Promise(done => server.listen(0, "127.0.0.1", done));
  const port = server.address().port; await new Promise(done => server.close(done)); return port;
}
async function until(read, predicate = Boolean, ms = 60000) {
  const deadline = Date.now() + ms;
  while (Date.now() < deadline) { const value = await read(); if (predicate(value)) return value; await delay(200); }
  throw new Error("Desktop condition timed out");
}
let site, vite;
async function desktop(mode, binary, cli, ca) {
  const dir = join(temp, mode); await mkdir(dir);
  const port = await freePort();
  let processOutput = "";
  const child = spawn(binary, ["task_desktop_acceptance", "--ignored", "--exact", "--nocapture"], {cwd: root, windowsHide: true, env: {
    ...process.env, CODEX_BIN: cli, PA_DESKTOP_MODE: "normal", PA_IPC_VERIFY: "1", PA_PROJECT_PICKER_FIXTURE: "1",
    PA_OVERLAY_VERIFY_DIR: dir, PA_LOG_VERIFY_DIR: join(dir, "logs"), PA_TASK_DESKTOP_FIXTURE_ENDPOINT: site.baseUrl, PA_TASK_DESKTOP_CA: ca,
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port} --remote-debugging-address=127.0.0.1`
  }, stdio: ["ignore", "pipe", "pipe"]});
  children.add(child);
  for (const stream of [child.stdout, child.stderr]) stream.on("data", b => { processOutput += b; });
  let socket, cleanupCredentials;
  const evidence = {mode, passed: false, checks: []};
  const record = message => { evidence.checks.push(message); console.log(`desktop-${mode}: ${message}`); };
  try {
    const target = await until(async () => {
      if (child.exitCode !== null) throw new Error(`Desktop exited ${child.exitCode}`);
      try { return (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find(t => t.type === "page" && !t.url.includes("window=")); } catch { return null; }
    });
    socket = new WebSocket(target.webSocketDebuggerUrl);
    await new Promise((done, reject) => { socket.addEventListener("open", done, {once: true}); socket.addEventListener("error", reject, {once: true}); });
    let nextId = 0;
    const pending = new Map();
    const receive = event => { const m = JSON.parse(event.data); const p = pending.get(m.id); if (p) { pending.delete(m.id); clearTimeout(p.timer); m.error ? p.reject(new Error("CDP request failed")) : p.done(m.result); } };
    socket.addEventListener("message", receive);
    const cdp = (method, params = {}, channel = socket) => new Promise((done, reject) => {
      const id = ++nextId, timer = setTimeout(() => { pending.delete(id); reject(new Error(`CDP timeout ${method}`)); }, 30000);
      pending.set(id, {done, reject, timer}); channel.send(JSON.stringify({id, method, params}));
    });
    const evaluate = async (expression, channel = socket) => {
      const value = await cdp("Runtime.evaluate", {expression, awaitPromise: true, returnByValue: true}, channel);
      if (value.exceptionDetails) throw new Error("WebView evaluation failed");
      return value.result.value;
    };
    const invoke = (command, params = {}) => `window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)},{request:${JSON.stringify({apiVersion: 1, params})}})`;
    const call = async (command, params = {}) => {
      const wire = await evaluate(invoke(command, params));
      assert.ok(!JSON.stringify(wire).includes(secret)); return wire;
    };
    const ok = async (command, params) => { const wire = await call(command, params); assert.equal(wire.status, "ok", `${command}: ${wire.error?.code}`); return wire.data; };
    const click = async selector => { await until(() => evaluate(`Boolean(document.querySelector(${JSON.stringify(selector)}) && !document.querySelector(${JSON.stringify(selector)}).disabled)`)); await evaluate(`document.querySelector(${JSON.stringify(selector)}).click()`); };
    const input = async (selector, text) => evaluate(`(() => {const e=document.querySelector(${JSON.stringify(selector)}); Object.getOwnPropertyDescriptor(${selector.includes("prompt") ? "HTMLTextAreaElement" : "HTMLInputElement"}.prototype,'value').set.call(e,${JSON.stringify(text)});e.dispatchEvent(new Event('input',{bubbles:true}));})()`);
    await until(() => evaluate("Boolean(document.querySelector('.app-shell'))"));
    await click(".nav-item[data-page=tasks]");
    await until(() => evaluate("document.querySelector('[data-testid=tasks-notice]')?.textContent"), t => t?.includes("还没有登记项目"));
    assert.equal(await evaluate("document.querySelector('[data-testid=task-send]').disabled"), true);
    record("No-context submission blocked in real WebView");
    const added = await ok("projects_add_from_picker", {}); const project = added.project;
    // Match the gateway acceptance fixture: an explicit synthetic repository boundary.
    await mkdir(join(project.absolutePath, ".git"));
    const saved = await ok("provider_save", {profileId: null, expectedRevision: null, name: "UI synthetic relay", endpoint: site.baseUrl, defaultModel: null, defaultEffort: null});
    cleanupCredentials = async () => {
      const state = await ok("providers_list", {}); const p = state.profiles.find(p => p.id === saved.id);
      if (p?.credential.revision > 0 && p.credential.status !== "missing") await ok("credential_remove", {profileId: p.id, expectedRevision: p.configRevision});
    };
    const put = await ok("credential_put", {profileId: saved.id, expectedRevision: saved.configRevision, secret});
    const models = await ok("provider_models", {profileId: saved.id, expectedRevision: put.configRevision, refresh: true});
    const cap = models.capabilities.find(c => c.modelId === "gpt-5.1-codex-mini") ?? models.capabilities[0];
    const selection = {profileId: saved.id, expectedRevision: put.configRevision, modelId: cap.modelId, effort: cap.efforts.includes("medium") ? "medium" : cap.efforts[0]};
    const switched = await ok("provider_switch", selection);
    const operation = await until(() => ok("operation_get", {operationId: switched.operationId}), v => ["completed", "failed", "cancelled"].includes(v.phase));
    assert.equal(operation.phase, "completed"); assert.equal(operation.result.outcome, "applied");
    await click(".nav-item[data-page=home]"); await click(".nav-item[data-page=tasks]");
    await click("[data-testid=session-create]");
    await until(() => evaluate("document.querySelector('[data-testid=task-session]')?.value"));
    await until(() => evaluate("document.querySelector('[data-testid=task-config]')?.textContent"), t => t?.includes(selection.modelId));
    let sessionId = await evaluate("document.querySelector('[data-testid=task-session]').value");
    let lastTask;
    async function submit(mode, prompt = "S3_UI_PROMPT_SENTINEL") {
      site.setMode(mode, project.absolutePath);
      await input("[data-testid=task-prompt]", prompt);
      if (!await evaluate("document.querySelector('[data-testid=task-confirm]').checked")) await click("[data-testid=task-confirm]");
      const before = new Set((await ok("tasks_list", {sessionId, limit: 100})).items.map(t => t.id));
      await click("[data-testid=task-send]");
      const page = await until(() => ok("tasks_list", {sessionId, limit: 100}), page => page.items.some(t => !before.has(t.id)));
      lastTask = page.items.find(t => !before.has(t.id)); return lastTask;
    }
    await input("[data-testid=task-prompt]", "S3_UI_IME_SENTINEL");
    await evaluate("document.querySelector('[data-testid=task-prompt]').dispatchEvent(new CompositionEvent('compositionstart',{bubbles:true,data:'中'}));document.querySelector('[data-testid=task-prompt]').dispatchEvent(new KeyboardEvent('keydown',{bubbles:true,key:'Enter',isComposing:true}));");
    assert.equal((await ok("tasks_list", {sessionId})).items.length, 0);
    await evaluate("document.querySelector('[data-testid=task-prompt]').dispatchEvent(new CompositionEvent('compositionend',{bubbles:true,data:'中文'}))");
    await submit("ok");
    await until(() => ok("task_get", {taskId: lastTask.id}), t => ["completed", "failed", "cancelled", "interrupted"].includes(t.status));
    { const final = await ok("task_get", {taskId: lastTask.id}); assert.equal(final.status, "completed", `real task failed: ${final.error?.code}; see desktop log`); }
    await until(() => evaluate("document.querySelector('[data-testid=task-result]')?.dataset.status"), s => s === "completed");
    assert.match(await evaluate("document.querySelector('[data-testid=task-reply]')?.textContent ?? ''"), /CONTROLLED_UI_OK/);
    record("Real UI create/session confirmation/IME guard/start/CLI reply and authoritative completed state");
    await cdp("Page.reload"); await until(() => evaluate("Boolean(document.querySelector('.app-shell'))")); await click(".nav-item[data-page=tasks]");
    await until(() => evaluate("document.querySelector('[data-testid=task-reply]')?.textContent"), t => t?.includes("CONTROLLED_UI_OK"));
    await ok("ipc_fixture", {action: "task_ui_gap"});
    await click(".nav-item[data-page=home]"); await click(".nav-item[data-page=tasks]");
    await until(() => evaluate("document.querySelector('[data-testid=task-result]')?.dataset.status"), t => t === "completed");
    record("WebView reconstruction and injected event gap recover authoritative state/content");
    await ok("ipc_fixture", {action: "task_ui_change"});
    await until(() => evaluate("document.querySelector('.task-change')?.textContent"), t => t?.includes("selected-through-ui.txt"));
    await click(".task-change button");
    await until(() => evaluate("document.querySelector('[data-testid=task-diff]')?.textContent"), t => t?.includes("S3_DIFF_SENTINEL"));
    const changes = await ok("task_changes_get", {taskId: lastTask.id});
    assert.equal((await call("task_change_get", {taskId: lastTask.id, changeId: randomUUID()})).status, "error");
    assert.equal((await call("task_change_get", {taskId: lastTask.id, changeId: changes.items[0].id, relativePath: "../outside"})).error.code, "PA_IPC_INVALID_REQUEST");
    assert.equal((await call("ipc_fixture", {action: "task_ui_escape"})).status, "error");
    record("Injected normalized change renders text-only diff; forged ID/path and out-of-scope change rejected");
    // Leaf replacement must invalidate cached diffs without suppressing replies.
    const leaf = join(project.absolutePath, "selected-through-ui.txt");
    await rename(leaf, leaf + ".old"); await writeFile(leaf, "replacement fixture");
    assert.equal((await ok("task_change_get", {taskId: lastTask.id, changeId: changes.items[0].id})).unavailableReason, "unsafe_path");
    assert.equal((await ok("task_changes_get", {taskId: lastTask.id})).items[0].diffAvailable, false);
    await rm(leaf); await rename(leaf + ".old", leaf);
    await ok("ipc_fixture", {action: "task_ui_evict"});
    await until(() => evaluate("document.querySelector('[data-testid=task-result]')?.textContent"), t => t?.includes("内容已过期"));
    record("Injected file replacement rejects stale diff; memory eviction preserves task metadata with explicit unavailable feedback");
    await submit("long");
    await until(() => ok("task_get", {taskId: lastTask.id}), t => ["completed", "failed"].includes(t.status));
    assert.equal((await ok("task_get", {taskId: lastTask.id})).status, "completed");
    await until(() => evaluate("document.querySelector('[data-testid=task-reply]')?.textContent.length"), length => length > 65536);
    assert.equal(await evaluate("document.querySelectorAll('[data-testid=task-reply] img').length"), 0);
    assert.equal(await evaluate("document.documentElement.scrollWidth <= innerWidth"), true);
    record("Real long CLI reply crosses content pages and renders markup as text without horizontal overflow");
    await submit("approval");
    await until(() => evaluate("document.querySelector('[data-testid=task-result]')?.dataset.status"), t => t === "approval_required");
    await click("[data-testid=approval-approve]");
    await until(() => ok("task_get", {taskId: lastTask.id}), t => ["completed", "failed", "cancelled", "interrupted"].includes(t.status));
    { const final = await ok("task_get", {taskId: lastTask.id}); assert.equal(final.status, "completed", `real task failed: ${final.error?.code}; see desktop log`); }
    const approvals = await ok("approvals_get", {taskId: lastTask.id});
    assert.equal(approvals.items[0].decision, "approved"); assert.equal(approvals.items[0].consumption, "acknowledged");
    record("UI approves one real CLI print-only command; service confirms acknowledgement");
    await submit("hold");
    await until(() => evaluate("document.querySelector('[data-testid=task-result]')?.dataset.status"), t => t === "running");
    await click("[data-testid=task-cancel]");
    await until(() => ok("task_get", {taskId: lastTask.id}), t => t.status === "cancelled"); site.release();
    await until(() => evaluate("document.querySelector('[data-testid=task-result]')?.dataset.status"), t => t === "cancelled");
    record("UI cancellation waits for real owned CLI authoritative cancelled state");
    await submit("unauthorized");
    await until(() => ok("task_get", {taskId: lastTask.id}), t => t.status === "failed");
    await until(() => evaluate("document.querySelector('[data-testid=task-result]')?.dataset.status"), t => t === "failed");
    assert.ok(await evaluate("Boolean(document.querySelector('[data-testid=task-result] [role=alert]'))")); site.setMode("ok");
    record("Upstream failure shown as fixed error with authoritative failed state");
    await click("[data-testid=session-end]");
    await until(() => ok("session_get", {sessionId}), s => s.status === "closed");
    assert.equal(await evaluate("document.querySelector('[data-testid=task-send]').disabled"), true);
    await until(() => evaluate("document.querySelector('[data-testid=session-resume]').disabled"), value => value === true);
    await click("[data-testid=session-create]");
    const previousSessionId = sessionId;
    sessionId = await until(() => evaluate("document.querySelector('[data-testid=task-session]').value"), id => id && id !== previousSessionId);
    await ok("ipc_fixture", {action: "task_ui_recover"});
    await until(() => ok("session_get", {sessionId}), s => s.status === "interrupted");
    await click("[data-testid=session-resume]");
    await until(() => ok("session_get", {sessionId}), s => s.status === "active");
    record("Real UI ends a session permanently; a new session interrupted by production recovery requires explicit resume without replay");
    const beforeAudit = (await ok("tasks_list", {sessionId})).items.length;
    await ok("ipc_fixture", {action: "audit_fail_write"});
    await input("[data-testid=task-prompt]", "S3_UI_PROMPT_SENTINEL");
    if (!await evaluate("document.querySelector('[data-testid=task-confirm]').checked")) await click("[data-testid=task-confirm]");
    await click("[data-testid=task-send]");
    await until(() => evaluate("document.querySelector('[data-testid=tasks-error]')?.textContent"), t => t?.includes("PA_AUDIT_WRITE_FAILED"));
    assert.equal((await ok("tasks_list", {sessionId})).items.length, beforeAudit);
    await ok("ipc_fixture", {action: "audit_restore_write"});
    await click("[data-testid=task-stop-wait]");
    record("Injected admission audit failure creates no task and shows fixed recoverable feedback without resubmission");
    for (const command of ["task_content_get", "task_changes_get", "task_change_get"]) {
      assert.equal((await call(command, {taskId: lastTask.id, raw: "S3_RAW_SENTINEL"})).error.code, "PA_IPC_INVALID_REQUEST");
    }
    const overlayTarget = await until(async () => (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find(t => t.url.includes("window=overlay")));
    const overlay = new WebSocket(overlayTarget.webSocketDebuggerUrl);
    await new Promise(done => overlay.addEventListener("open", done, {once: true})); overlay.addEventListener("message", receive);
    try { for (const command of ["task_content_get", "task_changes_get", "task_change_get", "task_start", "approval_decide"]) assert.equal(await evaluate(invoke(command) + ".then(()=>false,()=>true)", overlay), true); } finally { overlay.close(); }
    assert.equal(await evaluate("document.body.textContent.includes(" + JSON.stringify(secret) + ")"), false);
    const logFiles = await readdir(join(dir, "logs"));
    const logs = (await Promise.all(logFiles.filter(f => f.endsWith(".jsonl")).map(f => readFile(join(dir, "logs", f), "utf8")))).join("\n");
    const database = await readFile(join(dir, "project-assistant.sqlite3"));
    for (const sentinel of [secret, "S3_UI_PROMPT_SENTINEL", "CONTROLLED_UI_OK", "S3_LONG_REPLY_", "S3_DIFF_SENTINEL", "S3_RAW_SENTINEL"]) {
      assert.ok(!logs.includes(sentinel)); assert.ok(!processOutput.includes(sentinel));
      assert.ok(!database.includes(Buffer.from(sentinel)), "transcript must not persist");
    }
    record("Overlay ACL and raw-field denial; DOM Key/log/transcript-storage sentinel checks");
    await cleanupCredentials(); cleanupCredentials = null; evidence.credentialCleanup = true;
    await click(".exit-button");
    await until(() => child.exitCode, code => code !== null); assert.equal(child.exitCode, 0);
    children.delete(child);
    assert.equal((await readdir(join(dir, "owned-processes"))).length, 0);
    evidence.passed = true;
    console.log(`desktop-${mode}: passed`);
  } finally {
    if (cleanupCredentials && child.exitCode === null) {
      try { await cleanupCredentials(); evidence.credentialCleanup = true; } catch { evidence.credentialCleanup = false; }
    }
    socket?.close();
    if (child.exitCode === null) spawnSync("taskkill.exe", ["/pid", String(child.pid), "/t", "/f"], {windowsHide: true, stdio: "ignore"});
    children.delete(child);
    assert.ok(!processOutput.includes(secret)); await writeFile(join(output, `desktop-${mode}.log`), processOutput);
    report.desktop.push(evidence);
  }
}

try {
  report.environment = {node: process.version, npm: (await run("npm-version", process.execPath, npmArgs(["--version"]))).trim(), rust: (await run("rust-version", "rustc", ["--version"])).trim()};
  await run("frontend-build", process.execPath, npmArgs(["run", "build"]));
  await run("ipc-tests", process.execPath, npmArgs(["run", "test:ipc"]));
  await run("offline-tests", process.execPath, npmArgs(["run", "test:codex"]));
  await run("rust-format", "cargo", ["fmt", "--check", "--manifest-path", "src-tauri/Cargo.toml"]);
  const common = ["--locked", "--manifest-path", "src-tauri/Cargo.toml"];
  const compiled = await run("rust-tests", "cargo", ["test", ...common, "--lib"]);
  report.rustTestsPassed = Number(compiled.match(/test result: ok\. (\d+) passed/)?.[1]);
  const devBinary = resolve(root, compiled.match(/Running unittests[^\r\n]*\(([^)]+\.exe)\)/)?.[1] ?? "missing");
  await run("rust-release-check", "cargo", ["check", ...common, "--release"]);
  const cli = await discoverCodex(); report.cliVersion = (await run("cli-version", cli, ["--version"])).trim();
  const ca = join(temp, "ca.pem"), key = join(temp, "key.pem");
  const generated = spawnSync(join(process.env.ProgramFiles, "Git/usr/bin/openssl.exe"), ["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-sha256", "-days", "1", "-keyout", key, "-out", ca, "-subj", "/CN=Project Assistant UI", "-addext", "subjectAltName=IP:127.0.0.1", "-addext", "basicConstraints=critical,CA:TRUE"], {windowsHide: true, stdio: "ignore"});
  assert.equal(generated.status, 0);
  site = await controlledSite("UI", secret, {tls: {cert: await readFile(ca), key: await readFile(key)}});
  vite = spawn(process.execPath, npmArgs(["run", "dev"]), {cwd: root, windowsHide: true, stdio: "ignore"}); children.add(vite);
  await until(async () => { try { return (await fetch("http://localhost:1420")).ok; } catch { return false; } });
  await desktop("dev", devBinary, cli, ca);
  spawnSync("taskkill.exe", ["/pid", String(vite.pid), "/t", "/f"], {windowsHide: true, stdio: "ignore"}); children.delete(vite); vite = null;
  const built = await run("embedded-test-build", "cargo", ["test", ...common, "--lib", "--features", "tauri/custom-protocol", "--no-run"]);
  const builtBinary = resolve(root, built.match(/Executable unittests[^\r\n]*\(([^)]+\.exe)\)/)?.[1] ?? "missing");
  site.setMode("ok"); await desktop("built", builtBinary, cli, ca);
  report.routes = {requests: site.requests.length, allKeysBound: site.requests.every(r => r.keyMatches)};
  assert.ok(report.routes.allKeysBound);
  report.passed = true;
} catch (error) { report.error = error instanceof Error ? error.message : String(error); console.error(report.error); process.exitCode = 1; }
finally {
  for (const child of children) if (child.exitCode === null) spawnSync("taskkill.exe", ["/pid", String(child.pid), "/t", "/f"], {windowsHide: true, stdio: "ignore"});
  if (site) { site.release(); await site.close(); }
  await rm(temp, {recursive: true, force: true, maxRetries: 10, retryDelay: 200}); report.temporaryRemoved = true;
  await writeFile(join(output, "report.json"), JSON.stringify(report, null, 2) + "\n");
}
console.log(`Task UI verification: ${report.passed ? "passed" : "failed"}; artifacts/phase-3-6/report.json`);
