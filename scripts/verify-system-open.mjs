// Phase 0.4 real WebView2 -> Tauri opener -> Windows default handler experiment.
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { createServer as createHttpServer } from "node:http";
import { createServer as createPortServer } from "node:net";
import { createWriteStream } from "node:fs";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

assert.equal(process.platform, "win32", "This experiment requires an interactive Windows desktop.");
const root = resolve(import.meta.dirname, "..");
const output = join(root, "artifacts", "git-system");
const exe = join(root, "src-tauri", "target", "debug", "project-assistant.exe");
await mkdir(output, { recursive: true });
const scopeRoot = join(tmpdir(), "phase-0-system-open");
await mkdir(scopeRoot, { recursive: true });
const fixture = await mkdtemp(join(scopeRoot, "probe-"));
const directory = join(fixture, "目录 中文 & ^ (x)");
const file = join(directory, "文件 ü 空格 & ! ^.txt");
await mkdir(directory);
await writeFile(file, "Phase 0.4 opener fixture\n");
const report = { platform: process.platform, checks: [], passed: false };

const portProbe = createPortServer();
await new Promise((done, reject) => {
  portProbe.once("error", reject);
  portProbe.listen(0, "127.0.0.1", done);
});
const debugPort = portProbe.address().port;
await new Promise((done) => portProbe.close(done));

let searchRequest;
const http = createHttpServer((request, response) => {
  if (request.url?.startsWith("/search?")) searchRequest = request.url;
  response.writeHead(200, { "content-type": "text/html; charset=utf-8" });
  response.end("<!doctype html><title>Phase 0.4 opener probe</title><p>Local search URL received.</p>");
});
await new Promise((done, reject) => {
  http.once("error", reject);
  http.listen(0, "127.0.0.1", done);
});
const searchBase = `http://127.0.0.1:${http.address().port}/search`;
const query = "中文 & 空格 / ? # ü";

const env = { ...process.env, PA_DESKTOP_MODE: "phase0", PA_SHELL_FAIL_ONCE: "0",
  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${debugPort} --remote-debugging-address=127.0.0.1` };
const child = spawn(exe, [], { cwd: root, env, windowsHide: true, stdio: ["ignore", "pipe", "pipe"] });
const log = createWriteStream(join(output, "system-open-process.log"));
let spawnError;
child.on("error", (error) => { spawnError = error; });
for (const stream of [child.stdout, child.stderr]) stream.on("data", (chunk) => log.write(chunk));
let socket;

async function until(read, accepts, timeoutMs = 15000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (spawnError) throw spawnError;
    const value = await read();
    if (accepts(value)) return value;
    await delay(250);
  }
  throw new Error(`Timed out after ${timeoutMs}ms`);
}

try {
  const target = await until(async () => {
    if (child.exitCode !== null) throw new Error(`Desktop app exited: ${child.exitCode}`);
    try {
      const response = await fetch(`http://127.0.0.1:${debugPort}/json/list`, { signal: AbortSignal.timeout(1500) });
      return (await response.json()).find((item) => item.type === "page" && item.url.includes("tauri.localhost"));
    } catch { return undefined; }
  }, Boolean, 60000);
  socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((done, reject) => {
    socket.addEventListener("open", done, { once: true });
    socket.addEventListener("error", reject, { once: true });
  });
  let nextId = 0;
  const pending = new Map();
  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    const request = pending.get(message.id);
    if (!request) return;
    pending.delete(message.id);
    clearTimeout(request.timer);
    if (message.error) request.reject(new Error(JSON.stringify(message.error)));
    else request.resolve(message.result);
  });
  function cdp(method, params = {}) {
    return new Promise((resolveRequest, reject) => {
      const id = ++nextId;
      const timer = setTimeout(() => { pending.delete(id); reject(new Error(`CDP timeout: ${method}`)); }, 15000);
      pending.set(id, { resolve: resolveRequest, reject, timer });
      socket.send(JSON.stringify({ id, method, params }));
    });
  }
  async function evaluate(expression) {
    const result = await cdp("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
    return result.result.value;
  }
  async function setInput(selector, value) {
    await evaluate(`(() => {
      const input = document.querySelector(${JSON.stringify(selector)});
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input, ${JSON.stringify(value)});
      input.dispatchEvent(new Event('input', { bubbles: true }));
    })()`);
    const attribute = selector === "#system-path" ? "systemPath"
      : selector === "#search-base" ? "searchBase" : "searchQuery";
    await until(() => evaluate(`document.querySelector('.open-probe').dataset.${attribute}`),
      (current) => current === value);
  }
  async function clickAndRead(buttonIndex) {
    try {
      await until(() => evaluate(`!document.querySelectorAll('.open-probe button')[${buttonIndex}].disabled`), Boolean);
      const before = await evaluate("Number(document.querySelector('[data-testid=open-result]').dataset.attempt)");
      await evaluate(`document.querySelectorAll('.open-probe button')[${buttonIndex}].click()`);
      return await until(() => evaluate(`(() => {
        const p = document.querySelector('[data-testid=open-result]');
        return Number(p.dataset.attempt) > ${before}
          && !document.querySelectorAll('.open-probe button')[${buttonIndex}].disabled
          && p.textContent ? p.textContent : '';
      })()`), Boolean);
    } catch (error) {
      const state = await evaluate(`(() => ({
        attempt: document.querySelector('[data-testid=open-result]').dataset.attempt,
        result: document.querySelector('[data-testid=open-result]').textContent,
        path: document.querySelector('.open-probe').dataset.systemPath,
        disabled: document.querySelectorAll('.open-probe button')[${buttonIndex}].disabled
      }))()`);
      throw new Error(`${error}; UI state: ${JSON.stringify(state)}`);
    }
  }
  await until(() => evaluate("Boolean(document.querySelector('#system-path'))"), Boolean);
  report.checks.push({ id: "real-webview", detail: "WebView2 loaded embedded React app and opener controls" });

  await setInput("#system-path", directory);
  assert.match(await clickAndRead(0), /路径：系统已接受打开请求/);
  report.checks.push({ id: "open-directory", detail: "scoped Unicode/metacharacter directory accepted by Windows opener" });

  await setInput("#system-path", file);
  assert.match(await clickAndRead(0), /路径：系统已接受打开请求/);
  report.checks.push({ id: "open-file", detail: "scoped Unicode/metacharacter file accepted by Windows opener" });
  console.log("Opener: directory and file accepted");

  await setInput("#system-path", join(directory, "不存在.txt"));
  assert.match(await clickAndRead(0), /OPEN_PATH_MISSING/);
  report.checks.push({ id: "missing-path", detail: "existing scope but missing file returned opener error" });

  await setInput("#system-path", join(root, "README.md"));
  assert.match(await clickAndRead(0), /OPEN_PATH_DENIED/);
  report.checks.push({ id: "path-preflight", detail: "existing out-of-scope project file denied before dispatch" });

  const rawDeny = await evaluate(`(async () => {
    try {
      await window.__TAURI_INTERNALS__.invoke('plugin:opener|open_path',
        { path: ${JSON.stringify(join(root, "README.md"))}, with: null });
      return 'UNEXPECTED_ALLOWED';
    } catch (error) { return String(error); }
  })()`);
  assert.match(rawDeny, /not allowed to open path/i);
  report.checks.push({ id: "path-plugin-scope", detail: "Tauri opener itself denied existing out-of-scope path" });
  console.log("Opener: path errors and scope verified");

  await setInput("#search-base", searchBase);
  await setInput("#search-query", query);
  assert.match(await clickAndRead(1), /搜索 URL：系统已接受打开请求/);
  const requested = await until(async () => searchRequest, Boolean, 15000);
  const parsed = new URL(requested, searchBase);
  assert.equal(parsed.pathname, "/search");
  assert.equal(parsed.searchParams.get("q"), query);
  assert.match(requested, /%E4%B8%AD%E6%96%87/);
  assert.match(requested, /%26/);
  report.checks.push({ id: "encoded-search-url", detail: "default browser requested local URL; UTF-8 query round-tripped without shell interpolation" });

  await setInput("#search-base", "https://example.com/search");
  assert.match(await clickAndRead(1), /OPEN_URL_DENIED/);
  report.checks.push({ id: "url-scope", detail: "out-of-scope HTTPS URL denied before system dispatch" });
  console.log("Opener: encoded local URL and URL scope verified");

  report.passed = true;
} catch (error) {
  report.error = String(error);
  process.exitCode = 1;
  console.error(error);
} finally {
  socket?.close();
  if (child.exitCode === null && child.pid) {
    spawnSync("taskkill.exe", ["/pid", String(child.pid), "/t", "/f"], { windowsHide: true, stdio: "ignore" });
  }
  // Explorer may reuse an existing process; close only windows pointing at this unique fixture.
  spawnSync("powershell.exe", ["-NoProfile", "-Command", `
    $shell = New-Object -ComObject Shell.Application
    foreach ($window in $shell.Windows()) {
      try {
        if ($window.LocationURL.Contains('/phase-0-system-open/' + $env:PHASE0_FIXTURE_TOKEN + '/')) {
          $window.Quit()
        }
      } catch {}
    }
  `], { env: { ...process.env, PHASE0_FIXTURE_TOKEN: basename(fixture) },
    windowsHide: true, stdio: "ignore", timeout: 5000 });
  http.closeAllConnections();
  await Promise.race([new Promise((done) => http.close(done)), delay(3000)]);
  log.end();
  await rm(fixture, { recursive: true, force: true, maxRetries: 3, retryDelay: 300 });
  await writeFile(join(output, "system-open-report.json"), JSON.stringify(report, null, 2) + "\n");
  console.log(`System open 0.4: ${report.passed ? "PASS" : "FAIL"}; ${report.checks.length} checks; ${join(output, "system-open-report.json")}`);
}
