import assert from "node:assert/strict";
import { writeFile } from "node:fs/promises";
import { resolve } from "node:path";

export async function verifyLoggingDesktop({ evaluate, until, report, cdp, output, debugPort, receive, replaceOverlaySocket }) {
  const invoke = (command, params = {}, apiVersion = 1) => `window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)}, {request:${JSON.stringify({apiVersion, params})}})`;
  const call = (command, params, version) => evaluate(invoke(command, params, version));
  const fixture = async action => assert.equal((await call("ipc_fixture", {action})).status, "ok");
  const getSettings = async () => (await call("settings_get", {scope: "global"})).data;
  await until(() => evaluate("document.querySelector('.main-area')?.dataset.page"), page => page === "home");
  const initial = await call("logs_get_status");
  assert.equal(initial.status, "ok");
  assert.equal(initial.data.enabled, true, "Isolated app_log_dir must support protected persistence");
  assert.equal(initial.data.auditRetentionDays, 90);
  assert.equal(initial.data.summaryCollectionEnabled, false);
  const secret = "synthetic-secret-DO-NOT-LOG";
  for (const [params, version, code] of [[{secret}, 1, "PA_IPC_INVALID_REQUEST"], [{}, 2, "PA_IPC_VERSION_UNSUPPORTED"]]) {
    const rejected = await call("logs_clear", params, version);
    assert.equal(rejected.error.code, code);
    assert.equal(JSON.stringify(rejected).includes(secret), false);
  }
  const target = await until(async () => (await (await fetch(`http://127.0.0.1:${debugPort}/json/list`)).json()).find(t => t.url.includes("window=overlay")), Boolean);
  const socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((done, reject) => { socket.addEventListener("open", done, {once: true}); socket.addEventListener("error", reject, {once: true}); });
  socket.addEventListener("message", receive);
  replaceOverlaySocket(socket);
  for (const command of ["logs_get_status", "logs_clear"]) {
    assert.equal(await evaluate(`${invoke(command)}.then(() => false, () => true)`, socket), true);
  }
  report.results.push("真实 WebView2 日志状态、严格请求/版本和 overlay 最小权限通过；摘要采集保持关闭");

  await evaluate("document.querySelector('.nav-item[data-page=settings]').click()");
  await until(() => evaluate("document.querySelector('[data-testid=settings-status]')?.dataset.phase"), phase => phase === "ready");
  const before = await getSettings();
  await fixture("audit_fail_write");
  const failed = await call("settings_update", {scope: "global", expectedRevision: before.revision, changes: [{key: "search_engine", value: "google"}, {key: "overlay_enabled", value: false}]});
  assert.equal(failed.error.code, "PA_AUDIT_WRITE_FAILED");
  assert.equal(JSON.stringify(failed).includes(secret), false);
  assert.deepEqual(await getSettings(), before);
  assert.equal((await call("overlay_get")).data.enabled, true);
  await fixture("audit_restore_write");
  const saved = await call("settings_update", {scope: "global", expectedRevision: before.revision, changes: [{key: "log_level", value: "debug"}]});
  assert.equal(saved.status, "ok");
  assert.equal((await call("logs_get_status")).data.level, "debug");
  report.results.push("真实审计 SQL 故障拒绝整批设置且常驻运行状态保留；修复后提交成功并立即应用日志级别");

  await until(() => evaluate("Boolean(document.querySelector('[data-testid=logs-clear]'))"), Boolean);
  await evaluate("document.querySelector('[data-testid=logs-clear]').click()");
  await until(() => evaluate("document.querySelector('[data-testid=logs-clear]')?.disabled"), disabled => disabled === false);
  const cleared = await call("logs_clear");
  assert.equal(cleared.status, "ok");
  assert.equal(cleared.data.fileCount, 0);
  assert.equal(cleared.data.totalBytes, 0);
  assert.equal((await getSettings()).revision, saved.data.revision);
  await evaluate("document.querySelector('.nav-item[data-page=diagnostics]').click()");
  await until(() => evaluate("document.querySelector('[data-testid=logging-status]')?.textContent"), Boolean);
  const text = await evaluate("document.querySelector('.main-area').textContent");
  assert.equal(text.includes(secret), false);
  assert.equal(text.includes("C:\\"), false);
  const screenshot = await cdp("Page.captureScreenshot", {format: "png"});
  await writeFile(resolve(output, "logging-diagnostics.png"), Buffer.from(screenshot.data, "base64"));
  report.results.push("设置页清理按钮和主窗口专用清理命令可用，数据库设置保留；诊断显示白名单汇总且不暴露路径或秘密");
}
