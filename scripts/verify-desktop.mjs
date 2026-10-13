// Windows-only framework smoke test. CDP is enabled only for this child process.
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { createWriteStream } from "node:fs";
import { resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { createServer } from "node:net";
import { createServer as createHttpServer } from "node:http";
import { verifyIpcDesktop } from "./ipc/desktop.mjs";

assert.equal(process.platform, "win32", "This test requires Windows and WebView2.");
assert.ok(process.env.npm_execpath, "Run this script with npm run verify:desktop.");
const mode = process.argv[2] ?? "dev";
assert.ok(["dev", "built"].includes(mode), "Expected dev or built.");
const scenario = process.argv[3] ?? "phase0";
assert.ok(["phase0", "shell", "shell-error", "ipc"].includes(scenario));
const shell = scenario !== "phase0";
const root = resolve(import.meta.dirname, "..");
const output = resolve(root, "artifacts", shell ? `shell-${mode}-${scenario}` : `desktop-${mode}`);
await mkdir(output, { recursive: true });
const listener = createServer();
await new Promise((resolvePort, reject) => {
  listener.once("error", reject);
  listener.listen(0, "127.0.0.1", resolvePort);
});
const debugPort = listener.address().port;
await new Promise((done) => listener.close(done));
const overlayFixtureDirectory = scenario === "ipc" ? await mkdtemp(resolve(tmpdir(), "project-assistant-ipc-")) : "";
const remote = createHttpServer((_, response) => { response.end('<!doctype html><title>IPC remote fixture</title><p>Local verification fixture</p>'); });
if (scenario === "ipc") await new Promise(done => remote.listen(0, "127.0.0.1", done));
const env = {
  ...process.env,
  PA_DESKTOP_MODE: shell ? "normal" : "phase0",
  PA_SHELL_FAIL_ONCE: scenario === "shell-error" ? "1" : "0",
  PA_IPC_VERIFY: scenario === "ipc" ? "1" : "0",
  PA_OVERLAY_VERIFY_DIR: overlayFixtureDirectory,
  PA_IPC_REMOTE_URL: scenario === "ipc" ? `http://127.0.0.1:${remote.address().port}/` : "",
  CARGO_TERM_COLOR: "never",
  CARGO_TERM_PROGRESS_WHEN: "never",
  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${debugPort} --remote-debugging-address=127.0.0.1`,
};
const child = mode === "dev"
  ? spawn(process.execPath, [process.env.npm_execpath, "run", "tauri", "--", "dev", "--no-watch"],
    { cwd: root, env, windowsHide: true, stdio: ["ignore", "pipe", "pipe"] })
  : spawn(resolve(root, "src-tauri", "target", "debug", "project-assistant.exe"), [],
    { cwd: root, env, windowsHide: true, stdio: ["ignore", "pipe", "pipe"] });
const log = createWriteStream(resolve(output, "process.log"));
let processOutput = "";
let spawnError;
child.on("error", (error) => { spawnError = error; });
for (const stream of [child.stdout, child.stderr]) {
  stream.on("data", (chunk) => { log.write(chunk); processOutput += chunk.toString(); });
}
let socket;
let overlaySocket;
const report = { mode, scenario, date: new Date().toISOString(), timezone: "Asia/Shanghai", results: [], passed: false };

async function until(read, accepts, timeoutMs = 15000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (spawnError) throw spawnError;
    const value = await read();
    if (accepts(value)) return value;
    await delay(300);
  }
  throw new Error(`Timed out after ${timeoutMs}ms; see ${output}/process.log`);
}

try {
  console.log(`Starting real desktop (${mode}); logs: ${output}`);
  const target = await until(async () => {
    if (child.exitCode !== null) throw new Error(`Desktop launcher exited with ${child.exitCode}`);
    try {
      const response = await fetch(`http://127.0.0.1:${debugPort}/json/list`, { signal: AbortSignal.timeout(2000) });
      const targets = await response.json();
      return targets.find((item) => item.type === "page" && !item.url.includes("window=overlay") && item.url.includes(mode === "dev" ? "localhost:1420" : "tauri.localhost"));
    } catch { return undefined; }
  }, Boolean, 900000);

  socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((done, reject) => { socket.addEventListener("open", done, { once: true }); socket.addEventListener("error", reject, { once: true }); });
  let requestId = 0;
  const pending = new Map();
  const receive = (event) => {
    const message = JSON.parse(event.data);
    const request = pending.get(message.id);
    if (!request) return;
    pending.delete(message.id);
    clearTimeout(request.timer);
    if (message.error) request.reject(new Error(JSON.stringify(message.error)));
    else request.resolve(message.result);
  };
  socket.addEventListener("message", receive);
  function cdp(method, params = {}, targetSocket = socket) {
    return new Promise((resolveRequest, reject) => {
      const id = ++requestId;
      const timer = setTimeout(() => { pending.delete(id); reject(new Error(`CDP timeout: ${method}`)); }, 15000);
      pending.set(id, { resolve: resolveRequest, reject, timer });
      targetSocket.send(JSON.stringify({ id, method, params }));
    });
  }
  async function evaluate(expression, targetSocket = socket) {
    const result = await cdp("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true }, targetSocket);
    if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
    return result.result.value;
  }
  let wire;
  if (shell) {
    if (scenario === "shell-error") {
      const error = await until(() => evaluate("document.querySelector('[data-testid=startup-error]')?.textContent"), Boolean);
      assert.match(error, /PA_DESKTOP_INITIALIZATION_FAILED/);
      assert.equal(await evaluate("Boolean(document.querySelector('#probe-message'))"), false);
      const screenshot = await cdp("Page.captureScreenshot", { format: "png" });
      await writeFile(resolve(output, "startup-error.png"), Buffer.from(screenshot.data, "base64"));
      await evaluate("document.querySelector('.state-actions .primary-button').click()");
      report.results.push("真实 Rust 初始化失败显示固定恢复提示，重试恢复");
    }
    await until(() => evaluate("document.querySelector('.main-area')?.dataset.page"), (page) => page === "home");
    assert.equal(await evaluate("document.querySelector('h1').textContent"), "工作台");
    assert.equal(await evaluate("Boolean(document.querySelector('#probe-message'))"), false);
    report.results.push("正常启动进入工作台，不加载阶段 0 验证控件");
    wire = await evaluate("window.__TAURI_INTERNALS__.invoke('desktop_initialize', { request: { apiVersion: 1, params: {} } })");
    assert.equal(wire.status, "ok");
    assert.equal(wire.data.mode, "normal");
    for (const command of ["framework_probe", "phase0_check_open_path", "plugin:opener|open_url"]) {
      const deniedRequest = { request: { apiVersion: 1, params: { message: "fixture" } }, path: "C:\\", url: "http://127.0.0.1/" };
      const denied = await evaluate(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)}, ${JSON.stringify(deniedRequest)}).then(() => false, () => true)`);
      assert.equal(denied, true, `${command} must be denied in normal mode`);
    }
    report.results.push("正常模式 ACL 拒绝阶段 0 命令和系统打开插件");
    const screenshot = await cdp("Page.captureScreenshot", { format: "png" });
    await writeFile(resolve(output, "window.png"), Buffer.from(screenshot.data, "base64"));
    // Wait for mount focus before moving focus to the nav item.
    await until(() => evaluate("document.activeElement.id"), (id) => id === "page-heading");
    await cdp("Page.bringToFront");
    await evaluate("document.querySelector('[data-page=projects].nav-item').focus()");
    assert.equal(await evaluate("document.activeElement.dataset.page"), "projects");
    await cdp("Input.dispatchKeyEvent", { type: "rawKeyDown", key: "Enter", code: "Enter", windowsVirtualKeyCode: 13 });
    await cdp("Input.dispatchKeyEvent", { type: "keyUp", key: "Enter", code: "Enter", windowsVirtualKeyCode: 13 });
    await until(() => evaluate("document.querySelector('.main-area')?.dataset.page"), (page) => page === "projects");
    assert.equal(await evaluate("document.activeElement.id"), "page-heading");
    assert.match(await evaluate("document.querySelector('[data-testid=state-empty]').textContent"), /尚未启用/);
    report.results.push("键盘 Enter 可打开项目入口，焦点移至标题并显示明确空状态");
    for (const page of ["ai", "settings", "diagnostics", "home"]) {
      await evaluate(`document.querySelector('.nav-item[data-page=${page}]').click()`);
      await until(() => evaluate("document.querySelector('.main-area')?.dataset.page"), (current) => current === page);
      assert.equal(await evaluate("document.querySelector('nav [aria-current=page]').dataset.page"), page);
      assert.equal(await evaluate("document.activeElement.id"), "page-heading");
      if (page !== "home") assert.equal(await evaluate("Boolean(document.querySelector('[data-testid=state-unavailable]'))"), true);
      if (page === "ai") assert.equal(await evaluate("document.querySelectorAll('input').length"), 0);
      if (page === "settings") assert.equal(await evaluate("document.querySelector('.settings-form fieldset').disabled"), true);
    }
    assert.equal(await evaluate("document.documentElement.scrollWidth <= window.innerWidth"), true);
    report.results.push("项目/AI/设置/服务状态/概览导航和焦点正确；未就绪表单禁用，无秘密输入");
    const overlayTarget = await until(async () => {
      const response = await fetch(`http://127.0.0.1:${debugPort}/json/list`);
      return (await response.json()).find((item) => item.url.includes("window=overlay"));
    }, Boolean);
    overlaySocket = new WebSocket(overlayTarget.webSocketDebuggerUrl);
    await new Promise((done, reject) => { overlaySocket.addEventListener("open", done, { once: true }); overlaySocket.addEventListener("error", reject, { once: true }); });
    overlaySocket.addEventListener("message", receive);
    const overlayEval = (expression) => evaluate(expression, overlaySocket);
    if (await overlayEval("window.__TAURI_INTERNALS__.invoke('overlay_get', { request: { apiVersion: 1, params: {} } }).then(r => r.data).then(s => s.enabled)") === false) {
      await evaluate("window.__TAURI_INTERNALS__.invoke('overlay_control', { request: { apiVersion: 1, params: { action: 'enable' } } })");
      report.results.push("已恢复上次保存的停用状态，托盘/主窗口入口可重新启用");
    }
    await until(() => overlayEval("Boolean(document.querySelector('.floating-ball'))"), Boolean);
    assert.equal(await overlayEval("window.__TAURI_INTERNALS__.invoke('overlay_get', { request: { apiVersion: 1, params: {} } }).then(r => r.data).then(s => s.trayReady)"), true);
    report.results.push("系统托盘已创建，提供展开、启停、主窗口和退出菜单入口");
    assert.equal(await overlayEval("window.__TAURI_INTERNALS__.invoke('plugin:window|is_always_on_top', { label: 'overlay' })"), true);
    assert.equal(await overlayEval("window.__TAURI_INTERNALS__.invoke('plugin:window|is_visible', { label: 'overlay' })"), true);
    assert.equal(await overlayEval("window.__TAURI_INTERNALS__.invoke('desktop_initialize', { request: { apiVersion: 1, params: {} } }).then(() => false, () => true)"), true);
    report.results.push("首次默认创建独立置顶悬浮球，overlay capability 拒绝 main 初始化命令");
    const invalidShortcut = await evaluate("window.__TAURI_INTERNALS__.invoke('overlay_control', { request: { apiVersion: 1, params: { action: 'set_shortcut', shortcut: 'not-a-shortcut' } } })");
    assert.equal(invalidShortcut.error.code, "PA_SHORTCUT_INVALID");
    assert.equal(await overlayEval("window.__TAURI_INTERNALS__.invoke('overlay_get', { request: { apiVersion: 1, params: {} } }).then(r => r.data).then(s => s.shortcut)"), "Ctrl+Alt+Space");
    report.results.push("非法快捷键被拒绝并显示固定错误，原快捷键保留");
    const conflictShortcut = await evaluate("window.__TAURI_INTERNALS__.invoke('overlay_control', { request: { apiVersion: 1, params: { action: 'set_shortcut', shortcut: 'Ctrl+Alt+Delete' } } })");
    assert.equal(conflictShortcut.error.code, 'PA_SHORTCUT_CONFLICT');
    await until(() => overlayEval("window.__TAURI_INTERNALS__.invoke('overlay_get', { request: { apiVersion: 1, params: {} } }).then(r => r.data).then(s => s.shortcutError)"), (value) => value === "PA_SHORTCUT_CONFLICT");
    assert.equal(await overlayEval("window.__TAURI_INTERNALS__.invoke('overlay_get', { request: { apiVersion: 1, params: {} } }).then(r => r.data).then(s => s.shortcut)"), "Ctrl+Alt+Space");
    report.results.push("Windows 保留快捷键冲突被拒绝并提示更换组合，原快捷键保留");
    await evaluate("window.__TAURI_INTERNALS__.invoke('overlay_control', { request: { apiVersion: 1, params: { action: 'set_shortcut', shortcut: 'Ctrl+Alt+Space' } } })");
    await until(() => overlayEval("window.__TAURI_INTERNALS__.invoke('overlay_get', { request: { apiVersion: 1, params: {} } }).then(r => r.data).then(s => s.shortcutError)"), (value) => value === null);
    await overlayEval("document.querySelector('.floating-ball').click()");
    await until(() => overlayEval("document.activeElement.id"), (id) => id === "overlay-draft");
    const draft = "中文草稿\n第二行 & DO-NOT-LOG";
    await overlayEval(`(() => { const input = document.querySelector('#overlay-draft'); Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set.call(input, ${JSON.stringify(draft)}); input.dispatchEvent(new Event('input', { bubbles: true })); })()`);
    await until(() => overlayEval("window.__TAURI_INTERNALS__.invoke('overlay_get', { request: { apiVersion: 1, params: {} } }).then(r => r.data).then(s => s.draft)"), (value) => value === draft);
    assert.equal(await evaluate("window.__TAURI_INTERNALS__.invoke('overlay_get', { request: { apiVersion: 1, params: {} } }).then(r => r.data).then(s => s.draft)"), "");
    await overlayEval("document.querySelector('#overlay-draft').dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', isComposing: true, bubbles: true }))");
    assert.equal(await overlayEval("Boolean(document.querySelector('.overlay-error'))"), false);
    await overlayEval("document.querySelector('#overlay-draft').dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))");
    await until(() => overlayEval("document.querySelector('.overlay-error')?.textContent"), (value) => value?.includes("尚未启用"));
    assert.equal(await overlayEval("document.querySelector('footer button:last-child').disabled"), true);
    const panelShot = await cdp("Page.captureScreenshot", { format: "png" }, overlaySocket);
    await writeFile(resolve(output, "overlay-panel.png"), Buffer.from(panelShot.data, "base64"));
    await overlayEval("document.querySelector('#overlay-draft').dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))");
    await until(() => overlayEval("Boolean(document.querySelector('.floating-ball'))"), Boolean);
    await overlayEval("document.querySelector('.floating-ball').click()");
    await until(() => overlayEval("document.querySelector('#overlay-draft')?.value"), (value) => value === draft);
    report.results.push("悬浮球展开聚焦、中文组字 Enter 不误发送、换行草稿收起保留；真实发送禁用，main 不回显草稿");
    await evaluate("window.__TAURI_INTERNALS__.invoke('plugin:window|close', { label: 'main' })");
    await until(() => evaluate("window.__TAURI_INTERNALS__.invoke('plugin:window|is_visible', { label: 'main' })"), (visible) => visible === false);
    await overlayEval("document.querySelector('footer button:first-child').click()");
    await until(() => evaluate("window.__TAURI_INTERNALS__.invoke('plugin:window|is_visible', { label: 'main' })"), Boolean);
    await overlayEval("document.querySelector('[aria-label=关闭常驻入口]').click()");
    await until(() => evaluate("window.__TAURI_INTERNALS__.invoke('overlay_get', { request: { apiVersion: 1, params: {} } }).then(r => r.data).then(s => s.enabled)"), (enabled) => enabled === false);
    await evaluate("window.__TAURI_INTERNALS__.invoke('overlay_control', { request: { apiVersion: 1, params: { action: 'enable' } } })");
    await until(() => overlayEval("Boolean(document.querySelector('.floating-ball'))"), Boolean);
    const labels = await evaluate("window.__TAURI_INTERNALS__.invoke('plugin:window|get_all_windows')");
    assert.deepEqual(labels.sort(), ["main", "overlay"]);
    report.results.push("常驻开启时主窗口关闭仅隐藏，可从悬浮面板恢复；关闭入口停用，再启用复用唯一 overlay");
    if (scenario === "ipc") {
      await verifyIpcDesktop({ evaluate, until, report, debugPort, receive,
        getOverlaySocket: () => overlaySocket,
        replaceOverlaySocket: next => { overlaySocket?.close(); overlaySocket = next; },
        cdp, mainUrl: target.url,
      });
    }
    if (scenario === "shell-error") await evaluate("window.__TAURI_INTERNALS__.invoke('overlay_control', { request: { apiVersion: 1, params: { action: 'disable' } } })");
  } else {
  await until(() => evaluate("Boolean(document.querySelector('#probe-message'))"), Boolean);
  assert.equal(await evaluate("document.querySelector('h1').textContent"), "桌面框架验证");
  report.results.push("真实 WebView2 窗口加载 React 页面");

  async function submit(value) {
    await evaluate(`(() => {
      const input = document.querySelector('#probe-message');
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input, ${JSON.stringify(value)});
      input.dispatchEvent(new Event('input', { bubbles: true }));
    })()`);
    await evaluate("document.querySelector('button[type=submit]').click()");
  }
  await submit("阶段0 / 中文 space & < >");
  const success = await until(() => evaluate("document.querySelector('[data-testid=probe-success]')?.textContent"), Boolean);
  assert.ok(success.includes("Rust 已收到：阶段0 / 中文 space & < >"));
  assert.ok(success.includes("windows / x86_64"));
  report.results.push("React → Rust → React 往返成功（含中文、空格和特殊字符）");
  await submit("   ");
  const error = await until(() => evaluate("document.querySelector('[data-testid=probe-error]')?.textContent"), Boolean);
  assert.ok(error.includes("EMPTY_MESSAGE"));
  assert.equal(await evaluate("Boolean(document.querySelector('[data-testid=probe-success]'))"), false);
  report.results.push("Rust 错误正确返回并显示；旧成功状态已清除");
  await submit("错误后重试");
  const retry = await until(() => evaluate("document.querySelector('[data-testid=probe-success]')?.textContent"), Boolean);
  assert.ok(retry.includes("Rust 已收到：错误后重试"));
  assert.equal(await evaluate("Boolean(document.querySelector('[data-testid=probe-error]'))"), false);
  report.results.push("错误后可重试并恢复成功状态");
  // Real WebView -> registered Rust handler; no mocked invoke or UI-only assertion.
  wire = await evaluate(`window.__TAURI_INTERNALS__.invoke('framework_probe', { request: { apiVersion: 1, params: { message: 'wire-format-probe' } } })`);
  assert.equal(wire.apiVersion, 1);
  assert.equal(wire.status, "ok");
  assert.match(wire.correlationId, /^ipc-[a-z0-9-]+$/);
  assert.equal(wire.data.message, "Rust 已收到：wire-format-probe");
  assert.equal(wire.error, undefined);
  const incompatible = await evaluate(`window.__TAURI_INTERNALS__.invoke('framework_probe', { request: { apiVersion: 2, params: { message: 'synthetic-secret-DO-NOT-LOG' } } })`);
  assert.equal(incompatible.status, "error");
  assert.equal(incompatible.error.code, "PA_IPC_VERSION_UNSUPPORTED");
  assert.notEqual(incompatible.correlationId, wire.correlationId);
  assert.equal(incompatible.data, undefined);
  assert.ok(!JSON.stringify(incompatible).includes("synthetic-secret-DO-NOT-LOG"));
  report.results.push("IPC v1 成功/错误 envelope、版本拒绝及服务端唯一关联 ID 实测通过");
  const screenshot = await cdp("Page.captureScreenshot", { format: "png" });
  await writeFile(resolve(output, "window.png"), Buffer.from(screenshot.data, "base64"));
  }

  // Match the exact executable path, then send normal WM_CLOSE via CloseMainWindow.
  const exe = resolve(root, "src-tauri", "target", "debug", "project-assistant.exe");
  const psPath = `'${exe.replaceAll("'", "''")}'`;
  const useExitButton = shell && ["shell", "ipc"].includes(scenario);
  if (useExitButton) await evaluate("document.querySelector('.exit-button').click()");
  const close = spawnSync("powershell.exe", ["-NoProfile", "-Command", `
    $p = @(Get-Process -Name 'project-assistant' -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq ${psPath} });
    if (${useExitButton ? "$true" : "$false"} -and $p.Count -eq 0) { exit 0 };
    if ($p.Count -ne 1) { throw 'Expected exactly one test application' };
    $null = $p[0].Handle;
    if (${useExitButton ? "$false" : "$true"} -and -not $p[0].CloseMainWindow()) { throw 'Could not send window close' };
    if (-not $p[0].WaitForExit(10000)) { throw 'Application did not exit after window close' };
    $exitCode = $p[0].ExitCode;
    if ($null -eq $exitCode -or $exitCode -ne 0) { throw "Unexpected application exit code: $exitCode" };
  `], { encoding: "utf8", windowsHide: true, timeout: 15000 });
  assert.equal(close.status, 0, close.stderr || close.stdout);
  if (mode === "built") {
    await until(async () => child.exitCode, (code) => code !== null);
    assert.equal(child.exitCode, 0, "Embedded app must exit cleanly");
  }
  report.results.push(useExitButton ? "退出应用按钮关闭主窗口，应用进程退出" : "窗口正常关闭后应用进程退出，退出码 0");
  await delay(300);
  if (!shell) {
  assert.ok(processOutput.includes("PA_VALIDATION_EMPTY_MESSAGE"), "Expected sanitized Rust error output");
  assert.ok(processOutput.includes(wire.correlationId), "Response and log must share correlation ID");
  for (const forbidden of ["阶段0 / 中文 space & < >", "错误后重试", "wire-format-probe", "synthetic-secret-DO-NOT-LOG"]) assert.ok(!processOutput.includes(forbidden), "Payload must not enter process log");
  report.results.push("进程日志包含脱敏的 Rust 错误码");
  } else if (scenario === "shell-error") {
    assert.ok(processOutput.includes("PA_DESKTOP_INITIALIZATION_FAILED"));
  }
  report.passed = true;
  console.log(report.results.join("\n"));
} catch (error) {
  report.error = String(error);
  console.error(error);
  process.exitCode = 1;
} finally {
  socket?.close();
  overlaySocket?.close();
  if (remote.listening) await new Promise(done => remote.close(done));
  // Terminate only the launcher tree created by this test, including Vite.
  if (child.exitCode === null && child.pid) {
    spawnSync("taskkill.exe", ["/pid", String(child.pid), "/t", "/f"], { windowsHide: true, stdio: "ignore" });
  }
  if (overlayFixtureDirectory) await rm(overlayFixtureDirectory, { recursive: true, force: true });
  log.end();
  await writeFile(resolve(output, "report.json"), JSON.stringify(report, null, 2) + "\n");
}
