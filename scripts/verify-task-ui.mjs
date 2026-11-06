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
const phase37 = process.argv.includes("--step-3-7");
const overlaySmoke = process.argv.includes("--overlay-smoke") || phase37;
const step = phase37 ? "3.7" : "3.6";
const output = join(root, phase37 ? "artifacts/phase-3-7" : "artifacts/phase-3-6");
await mkdir(output, {recursive: true});
const temp = await mkdtemp(join(tmpdir(), "pa-task-ui-"));
const secret = `synthetic-ui-${randomUUID()}`;
const report = {step, date: new Date().toISOString(), timezone: "Asia/Shanghai", passed: false,
  syntheticDataOnly: true, officialSource: "reserved-not-tested", checks: [], desktop: [],
  limits: ["Real CLI and WebView with synthetic HTTPS replies/print-only approvals. No real Key/model usage.",
    "Diff/eviction/gap fixtures are explicit test-only injections, separate from real task evidence.",
    ...(phase37 ? ["Physical Chinese IME, tray, monitors and DPI remain manual."] : ["Physical IME, tray, monitors and DPI remain manual; overlay task submission remains S3-07."])]};
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
  throw new Error(`Desktop condition timed out during ${typeof verifyStage === "string" ? verifyStage : "desktop startup"}`);
}
let site, vite, verifyStage = "desktop setup";
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
  let socket, overlaySocket, overlayEval, overlayOk, overlayClick, cleanupCredentials;
  const evidence = {mode, passed: false, checks: []};
  const record = message => { evidence.checks.push(message); console.log(`desktop-${mode}: ${message}`); };
  try {
    const target = await until(async () => {
      if (child.exitCode !== null) throw new Error(`Desktop exited ${child.exitCode}`);
      try { return (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find(t => t.type === "page" && !t.url.includes("window=overlay")); } catch { return null; }
    });
    socket = new WebSocket(target.webSocketDebuggerUrl);
    await new Promise((done, reject) => { socket.addEventListener("open", done, {once: true}); socket.addEventListener("error", reject, {once: true}); });
    const channelStates = new Map();
    const attach = channel => {
      const state = {nextId: 0, pending: new Map()}; channelStates.set(channel, state);
      channel.addEventListener("message", event => { const m = JSON.parse(event.data); const p = state.pending.get(m.id); if (p) { state.pending.delete(m.id); clearTimeout(p.timer); m.error ? p.reject(new Error("CDP request failed")) : p.done(m.result); } });
    };
    attach(socket);
    const cdp = (method, params = {}, channel = socket) => new Promise((done, reject) => {
      const state = channelStates.get(channel); assert.ok(state, "CDP channel must be attached");
      const id = ++state.nextId, timer = setTimeout(() => { state.pending.delete(id); reject(new Error(`CDP timeout ${method}`)); }, 30000);
      state.pending.set(id, {done, reject, timer}); channel.send(JSON.stringify({id, method, params}));
    });
    const evaluate = async (expression, channel = socket) => {
      const value = await cdp("Runtime.evaluate", {expression, awaitPromise: true, returnByValue: true}, channel);
      if (value.exceptionDetails) throw new Error(`WebView evaluation failed during ${verifyStage}`);
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
    verifyStage = "main WebView bootstrap";
    await until(() => evaluate("Boolean(document.querySelector('.app-shell'))"));
    await click(".nav-item[data-page=tasks]");
    await until(() => evaluate("document.querySelector('[data-testid=tasks-notice]')?.textContent"), t => t?.includes("还没有登记项目"));
    assert.equal(await evaluate("document.querySelector('[data-testid=task-send]').disabled"), true);
    record("No-context submission blocked in real WebView");
    const overlayTarget = await until(async () => (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find(t => t.url.includes("window=overlay")));
    overlaySocket = new WebSocket(overlayTarget.webSocketDebuggerUrl);
    await new Promise((done, reject) => { overlaySocket.addEventListener("open", done, {once: true}); overlaySocket.addEventListener("error", reject, {once: true}); });
    attach(overlaySocket);
    overlayEval = expression => evaluate(expression, overlaySocket);
    overlayOk = async (command, params = {}) => { const wire = await overlayEval(invoke(command, params)); assert.equal(wire.status, "ok", `${command}: ${wire.error?.code}`); return wire.data; };
    overlayClick = async selector => { await until(() => overlayEval(`Boolean(document.querySelector(${JSON.stringify(selector)}) && !document.querySelector(${JSON.stringify(selector)}).disabled)`)); await overlayEval(`document.querySelector(${JSON.stringify(selector)}).click()`); };
    const initialOverlay = await overlayOk("overlay_task_get");
    assert.equal(initialOverlay.context, null);
    await overlayOk("overlay_control", {action: "expand"});
    await until(() => overlayEval("Boolean(document.querySelector('.overlay-panel'))"));
    assert.equal(await overlayEval("document.querySelector('.overlay-panel button.primary-button').disabled"), true);
    record("Overlay has no active context, denies submission and links to the main window");
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
    if (overlaySmoke) {
      await click(".nav-item[data-page=tasks]");
      verifyStage = "main-window selection of session for overlay context";
      await until(() => evaluate("Boolean(document.querySelector('[data-testid=session-create]') && !document.querySelector('[data-testid=session-create]').disabled)"));
      await click("[data-testid=session-create]");
      const sessionId = await until(() => evaluate("document.querySelector('[data-testid=task-session]')?.value"), value => Boolean(value));
      await until(() => ok("app_get_status", {}), value => value.activeContext?.sessionId === sessionId && value.activeContext?.projectId === project.id);
      verifyStage = "overlay context authority read";
      let context = await until(() => overlayOk("overlay_task_get"), value => value.context?.sessionId === sessionId);
      const baseline = (await ok("tasks_list", {sessionId, limit: 100})).items.length;
      const setOverlayPrompt = async value => overlayEval(`(() => {const e=document.querySelector('#overlay-draft');Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event('input',{bubbles:true}));})()`);
      const sendFromOverlay = async prompt => { await setOverlayPrompt(prompt); await overlayClick(".overlay-panel button.primary-button"); };
      site.setMode("ok", project.absolutePath);
      verifyStage = "overlay IME protection and first task send";
      await setOverlayPrompt("S3_OVERLAY_PROMPT_SENTINEL");
      await overlayEval("document.querySelector('#overlay-draft').dispatchEvent(new CompositionEvent('compositionstart',{bubbles:true,data:'中'}));document.querySelector('#overlay-draft').dispatchEvent(new KeyboardEvent('keydown',{bubbles:true,key:'Enter',isComposing:true}));");
      assert.equal((await ok("tasks_list", {sessionId, limit: 100})).items.length, baseline);
      await overlayEval("document.querySelector('#overlay-draft').dispatchEvent(new CompositionEvent('compositionend',{bubbles:true,data:'中文'}))");
      const mainTaskBefore = context.task?.taskId;
      const submitState = await overlayEval("JSON.stringify({disabled:document.querySelector('.overlay-panel button.primary-button')?.disabled,error:document.querySelector('.overlay-error')?.textContent})");
      assert.equal(JSON.parse(submitState).disabled, false, `overlay send unavailable: ${submitState}`);
      await overlayClick(".overlay-panel button.primary-button");
      context = await until(() => overlayOk("overlay_task_get"), value => value.task && value.task.taskId !== mainTaskBefore);
      const replyTaskId = context.task.taskId;
      await until(() => ok("task_get", {taskId: replyTaskId}), value => ["completed", "failed", "cancelled", "interrupted"].includes(value.status));
      assert.equal((await ok("task_get", {taskId: replyTaskId})).status, "completed");
      assert.match(await overlayEval("document.querySelector('[data-testid=overlay-task]')?.textContent ?? ''"), /CONTROLLED_UI_OK/);
      record("Overlay UI IME guard, real active-context CLI submission, and minimized reply");

      site.setMode("approval", project.absolutePath);
      verifyStage = "overlay approval task admission";
      await sendFromOverlay("S3_OVERLAY_APPROVAL_SENTINEL");
      context = await until(() => overlayOk("overlay_task_get"), value => value.task && value.task.taskId !== replyTaskId);
      const approvalTaskId = context.task.taskId;
      verifyStage = "overlay approval task status";
      const approvalState = await until(() => ok("task_get", {taskId: approvalTaskId}), value => value.status === "approval_required" || ["failed", "cancelled", "interrupted"].includes(value.status));
      assert.equal(approvalState.status, "approval_required", `overlay task did not request approval: ${approvalState.status}/${approvalState.error?.code ?? "none"}`);
      verifyStage = "overlay approval notice";
      await until(() => overlayEval("document.querySelector('.overlay-task .overlay-notice')?.textContent"), value => value?.includes("需要审批"));
      verifyStage = "overlay approval main-window handoff";
      assert.equal(await overlayEval("Boolean(document.querySelector('[aria-label=\"查看主窗口详情\"]'))"), true);
      await overlayOk("overlay_control", {action: "open_main"});
      verifyStage = "overlay focus task id publication";
      const focusStatus = await until(() => ok("app_get_status", {}), value => value.focusTaskId === approvalTaskId);
      verifyStage = "main navigation to task page";
      await until(() => evaluate("document.querySelector('.main-area')?.dataset.page"), value => value === "tasks");
      verifyStage = "main window matching overlay task";
      const mainTaskState = await until(() => evaluate("JSON.stringify({page:document.querySelector('.main-area')?.dataset.page,result:document.querySelector('[data-testid=task-result]')?.dataset.taskId,session:document.querySelector('[data-testid=task-session]')?.value,selectedTask:document.querySelector('.task-history [aria-pressed=true]')?.dataset.taskId,taskError:document.querySelector('[data-testid=tasks-error]')?.textContent,readError:document.querySelector('[data-testid=task-read-error]')?.textContent})"), value => { const state = JSON.parse(value); return state.result === approvalTaskId || Boolean(state.taskError || state.readError); });
      const observedMainTask = JSON.parse(mainTaskState);
      assert.equal(observedMainTask.result, approvalTaskId, `main window failed to load the selected overlay task: ${JSON.stringify(observedMainTask)}`);
      assert.equal(await evaluate("document.querySelector('[data-testid=task-approval]')?.textContent.includes('pending')"), true);
      verifyStage = "main-window decision for overlay task approval";
      const approvalTask = await ok("task_get", {taskId: approvalTaskId});
      const approvalPage = await ok("approvals_get", {taskId: approvalTaskId});
      const pendingApproval = approvalPage.items.find(item => item.decision === "pending");
      assert.ok(pendingApproval, "matching overlay task must expose a pending main-window approval");
      const denied = await ok("approval_decide", {approvalId: pendingApproval.id, taskId: approvalTaskId, expectedRevision: pendingApproval.revision,
        expectedTaskRevision: approvalTask.revision, operationFingerprint: pendingApproval.operationFingerprint, decision: "deny"});
      assert.equal(denied.approval.decision, "denied");
      await until(() => ok("task_get", {taskId: approvalTaskId}), value => ["completed", "failed", "cancelled", "interrupted"].includes(value.status));
      record("Overlay approval prompt opens the matching main-window task detail");

      site.setMode("hold", project.absolutePath);
      verifyStage = "overlay cancellation task admission";
      await sendFromOverlay("S3_OVERLAY_CANCEL_SENTINEL");
      context = await until(() => overlayOk("overlay_task_get"), value => value.task && value.task.taskId !== approvalTaskId);
      const cancelTaskId = context.task.taskId;
      await until(() => ok("task_get", {taskId: cancelTaskId}), value => value.status === "running");
      await until(() => overlayEval("document.querySelector('[data-testid=overlay-task]')?.dataset.status"), value => value === "running");
      await overlayClick(".overlay-task button:first-of-type");
      verifyStage = "overlay task cancellation result";
      await until(() => ok("task_get", {taskId: cancelTaskId}), value => value.status === "cancelled");
      record("Overlay cancel button reaches authoritative cancelled state");

      const oldRevision = context.contextRevision;
      verifyStage = "overlay context switch and stale revision rejection";
      const status = await ok("app_get_status", {});
      await ok("active_context_set", {projectId: null, sessionId: null, selection: null, expectedContextRevision: status.activeContextRevision});
      context = await until(() => overlayOk("overlay_task_get"), value => value.context === null && value.contextRevision > oldRevision);
      const stale = await overlayEval(invoke("task_start_from_active", {expectedContextRevision: oldRevision, clientRequestId: randomUUID(), prompt: "S3_STALE_CONTEXT_SENTINEL"}));
      assert.equal(stale.status, "error"); assert.equal(stale.error.code, "PA_TASK_CONTEXT_STALE");
      assert.equal(await overlayEval("document.querySelector('.overlay-panel button.primary-button').disabled"), true);
      await overlayClick(".overlay-panel footer button:first-child");
      for (const command of ["task_content_get", "task_changes_get", "task_change_get", "task_start", "task_get", "task_cancel", "active_context_set", "approval_decide"]) assert.equal(await overlayEval(invoke(command) + ".then(()=>false,()=>true)"), true);
      assert.equal(await overlayEval("document.body.textContent.includes(" + JSON.stringify(secret) + ")"), false);
      const reportFiles = await readdir(join(dir, "logs"));
      const smokeLogs = (await Promise.all(reportFiles.filter(f => f.endsWith(".jsonl")).map(f => readFile(join(dir, "logs", f), "utf8")))).join("\n");
      const smokeDb = await readFile(join(dir, "project-assistant.sqlite3"));
      for (const marker of [secret, "S3_OVERLAY_PROMPT_SENTINEL", "S3_OVERLAY_APPROVAL_SENTINEL", "S3_OVERLAY_CANCEL_SENTINEL", "S3_STALE_CONTEXT_SENTINEL", "CONTROLLED_UI_OK"]) {
        assert.ok(!smokeLogs.includes(marker)); assert.ok(!processOutput.includes(marker)); assert.ok(!smokeDb.includes(Buffer.from(marker)));
      }
      record("Context-switch rejection, no-context UI block, main-window link, overlay ACL and secret/transcript sentinels");
      await cleanupCredentials(); cleanupCredentials = null; evidence.credentialCleanup = true;
      await click(".exit-button"); await until(() => child.exitCode, code => code !== null); assert.equal(child.exitCode, 0);
      children.delete(child); assert.equal((await readdir(join(dir, "owned-processes"))).length, 0);
      evidence.passed = true;
      console.log(`desktop-${mode}: passed`);
      return;
    }
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
    for (const command of ["task_content_get", "task_changes_get", "task_change_get", "task_start", "task_get", "task_cancel", "active_context_set", "approval_decide"]) assert.equal(await overlayEval(invoke(command) + ".then(()=>false,()=>true)"), true);
    if (phase37) {
      verifyStage = "overlay active context read";
      site.setMode("ok", project.absolutePath);
      const currentMainStatus = await ok("app_get_status", {});
      await ok("active_context_set", {projectId: project.id, sessionId, selection: null, expectedContextRevision: currentMainStatus.activeContextRevision});
      const context = await until(() => overlayOk("overlay_task_get"), value => value.context?.sessionId === sessionId);
      assert.equal(Object.keys(context).sort().join(","), "context,contextRevision,pendingRequestId,task");
      assert.equal(context.context.projectId, project.id);
      assert.ok(!JSON.stringify(context).includes(project.absolutePath));
      await overlayOk("overlay_control", {action: "expand"});
      await until(() => overlayEval("Boolean(document.querySelector('.overlay-panel'))"));
      const beforeOverlayTask = context.task?.taskId;
      const overlayPrompt = "S3_OVERLAY_PROMPT_SENTINEL";
      const taskCountBeforeIme = (await ok("tasks_list", {sessionId, limit: 100})).items.length;
      const setOverlayPrompt = async value => overlayEval(`(() => {const e=document.querySelector('#overlay-draft');Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event('input',{bubbles:true}));})()`);
      verifyStage = "overlay prompt and IME handling";
      await setOverlayPrompt(overlayPrompt);
      await overlayEval("document.querySelector('#overlay-draft').dispatchEvent(new CompositionEvent('compositionstart',{bubbles:true,data:'中'}));document.querySelector('#overlay-draft').dispatchEvent(new KeyboardEvent('keydown',{bubbles:true,key:'Enter',isComposing:true}));");
      assert.equal((await ok("tasks_list", {sessionId, limit: 100})).items.length, taskCountBeforeIme);
      await overlayEval("document.querySelector('#overlay-draft').dispatchEvent(new CompositionEvent('compositionend',{bubbles:true,data:'中文'}))");
      verifyStage = "overlay first task submission";
      const beforeSubmitTaskId = context.task?.taskId;
      const controlState = await overlayEval("JSON.stringify({context:document.querySelector('.overlay-context')?.textContent,notice:document.querySelector('.overlay-notice')?.textContent,error:document.querySelector('.overlay-error')?.textContent,draft:document.querySelector('#overlay-draft')?.value,disabled:document.querySelector('.overlay-panel button.primary-button')?.disabled})");
      console.log(`desktop-${mode}: overlay-submit-state ${controlState}`);
      await overlayClick(".overlay-panel button.primary-button");
      let overlayTask = await until(() => overlayOk("overlay_task_get"), value => value.task && value.task.taskId !== beforeSubmitTaskId);
      const overlayTaskId = overlayTask.task.taskId;
      verifyStage = "overlay first task result";
      await until(() => ok("task_get", {taskId: overlayTaskId}), value => ["completed", "failed", "cancelled", "interrupted"].includes(value.status));
      assert.equal((await ok("task_get", {taskId: overlayTaskId})).status, "completed");
      await until(() => overlayEval("document.querySelector('[data-testid=overlay-task]')?.dataset.status"), value => value === "completed");
      assert.match(await overlayEval("document.querySelector('[data-testid=overlay-task]')?.textContent ?? ''"), /CONTROLLED_UI_OK/);
      record("Overlay IME guard, active-context submission, owned CLI reply and minimized feedback");

      site.setMode("approval", project.absolutePath);
      const approvalPrompt = "S3_OVERLAY_APPROVAL_SENTINEL";
      verifyStage = "overlay approval submission";
      await setOverlayPrompt(approvalPrompt);
      const approvalContext = await overlayOk("overlay_task_get");
      const approvalReceipt = await overlayOk("task_start_from_active", {expectedContextRevision: approvalContext.contextRevision, clientRequestId: randomUUID(), prompt: approvalPrompt});
      overlayTask = {task: approvalReceipt.task};
      const approvalTaskId = overlayTask.task.taskId;
      verifyStage = "overlay approval prompt and main-window focus";
      await until(() => ok("task_get", {taskId: approvalTaskId}), value => value.status === "approval_required");
      await until(() => overlayEval("document.querySelector('.overlay-task .overlay-notice')?.textContent"), value => value?.includes("需要审批"));
      await overlayClick(".overlay-task button:last-child");
      await until(() => evaluate("document.querySelector('.app-shell')?.dataset.page"), value => value === "tasks");
      await until(() => evaluate("document.querySelector('[data-testid=task-result]')?.dataset.taskId"), value => value === approvalTaskId);
      await click("[data-testid=approval-approve]");
      await until(() => ok("task_get", {taskId: approvalTaskId}), value => value.status === "completed");
      record("Overlay approval prompt sends the user to the matching main-window task detail");

      site.setMode("hold", project.absolutePath);
      const cancelPrompt = "S3_OVERLAY_CANCEL_SENTINEL";
      verifyStage = "overlay cancellation submission";
      await setOverlayPrompt(cancelPrompt);
      const cancelContext = await overlayOk("overlay_task_get");
      const cancelReceipt = await overlayOk("task_start_from_active", {expectedContextRevision: cancelContext.contextRevision, clientRequestId: randomUUID(), prompt: cancelPrompt});
      overlayTask = {task: cancelReceipt.task};
      const cancelTaskId = overlayTask.task.taskId;
      await until(() => overlayOk("task_get", {taskId: cancelTaskId}), value => value.status === "running");
      const cancelState = await overlayOk("overlay_task_get");
      await overlayOk("overlay_task_cancel", {expectedContextRevision: cancelState.contextRevision, expectedTaskRevision: overlayTask.task.revision});
      await until(() => ok("task_get", {taskId: cancelTaskId}), value => value.status === "cancelled");
      await until(() => overlayEval("document.querySelector('[data-testid=overlay-task]')?.dataset.status"), value => value === "cancelled");
      record("Overlay cancellation receipt resolves through authoritative cancelled state");
      const staleContextRevision = context.contextRevision;
      const mainStatus = await ok("app_get_status", {});
      await ok("active_context_set", {projectId: null, sessionId: null, selection: null, expectedContextRevision: mainStatus.activeContextRevision});
      const cleared = await until(() => overlayOk("overlay_task_get"), value => value.context === null && value.contextRevision > staleContextRevision);
      const staleStart = await overlayEval(invoke("task_start_from_active", {expectedContextRevision: staleContextRevision, clientRequestId: randomUUID(), prompt: "S3_STALE_CONTEXT_SENTINEL"}));
      assert.equal(staleStart.status, "error"); assert.equal(staleStart.error.code, "PA_TASK_CONTEXT_STALE");
      await overlayClick(".overlay-panel footer button:first-child");
      await until(() => evaluate("Boolean(document.querySelector('.app-shell'))"));
      assert.ok(cleared.contextRevision > staleContextRevision);
      record("Context switch invalidates overlay submission; stale context is rejected and main-window link remains available");
      await click(".nav-item[data-page=tasks]");
    }
    assert.equal(await overlayEval("document.body.textContent.includes(" + JSON.stringify(secret) + ")"), false);
    assert.equal(await evaluate("document.body.textContent.includes(" + JSON.stringify(secret) + ")"), false);
    const logFiles = await readdir(join(dir, "logs"));
    const logs = (await Promise.all(logFiles.filter(f => f.endsWith(".jsonl")).map(f => readFile(join(dir, "logs", f), "utf8")))).join("\n");
    const database = await readFile(join(dir, "project-assistant.sqlite3"));
    for (const sentinel of [secret, "S3_UI_PROMPT_SENTINEL", "S3_OVERLAY_PROMPT_SENTINEL", "S3_OVERLAY_APPROVAL_SENTINEL", "S3_OVERLAY_CANCEL_SENTINEL", "CONTROLLED_UI_OK", "S3_LONG_REPLY_", "S3_DIFF_SENTINEL", "S3_RAW_SENTINEL"]) {
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
    overlaySocket?.close();
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
console.log(`Task UI verification: ${report.passed ? "passed" : "failed"}; artifacts/phase-${phase37 ? "3-7" : "3-6"}/report.json`);
