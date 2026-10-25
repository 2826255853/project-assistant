import assert from "node:assert/strict";
import { setTimeout as delay } from "node:timers/promises";

export async function verifyIpcDesktop({ evaluate, until, report, debugPort, receive, getOverlaySocket, replaceOverlaySocket, cdp, mainUrl }) {
  const invoke = (command, params = {}, apiVersion = 1) => `window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)}, { request: ${JSON.stringify({ apiVersion, params })} })`;
  const fixture = async action => {
    const response = await evaluate(invoke("ipc_fixture", { action }));
    assert.equal(response.status, "ok"); return response;
  };
  const overlayEval = expression => evaluate(expression, getOverlaySocket());
  const status = await evaluate(invoke("app_get_status"));
  assert.equal(status.status, "ok");
  assert.equal(status.data.services.find(s => s.id === "storage").state, "ready");
  assert.equal(status.data.services.find(s => s.id === "storage").reasonCode, null);
  assert.equal(status.data.services.find(s => s.id === "codex").state, "unavailable");
  assert.deepEqual(status.data.ccSwitch, { management: "user_manual", integration: "disabled", readSupported: false, switchSupported: false });
  assert.equal(status.data.overlay.draft, "");
  const ids = new Set([status.correlationId]);
  for (const [command, params, version, code] of [
    ["app_get_status", {}, 2, "PA_IPC_VERSION_UNSUPPORTED"],
    ["app_get_status", { secret: "synthetic-secret-DO-NOT-LOG" }, 1, "PA_IPC_INVALID_REQUEST"],
    ["overlay_get", { unknown: 1 }, 1, "PA_IPC_INVALID_REQUEST"],
    ["overlay_control", { action: "disable", unknown: "synthetic-secret-DO-NOT-LOG" }, 1, "PA_IPC_INVALID_REQUEST"],
    ["overlay_control", { action: "set_shortcut", shortcut: "x".repeat(81) }, 1, "PA_VALIDATION_INPUT_TOO_LARGE"],
    ["overlay_control", { action: "draft", draft: "private" }, 1, "PA_IPC_PERMISSION_DENIED"],
  ]) {
    const response = await evaluate(invoke(command, params, version));
    assert.equal(response.status, "error"); assert.equal(response.error.code, code);
    assert.equal(JSON.stringify(response).includes("synthetic-secret"), false);
    assert.equal("data" in response, false); assert.equal(ids.has(response.correlationId), false); ids.add(response.correlationId);
  }
  for (const params of [{ action: "exit" }, { action: "set_shortcut", shortcut: "Ctrl+A" }, { action: "repair_position" }]) {
    const response = await overlayEval(invoke("overlay_control", params));
    assert.equal(response.error.code, "PA_IPC_PERMISSION_DENIED");
  }
  const tooLarge = await overlayEval(invoke("overlay_control", { action: "draft", draft: "中".repeat(10923) }));
  assert.equal(tooLarge.error.code, "PA_VALIDATION_INPUT_TOO_LARGE");
  for (const command of ["settings_get", "settings_update"]) {
    assert.equal(await overlayEval(invoke(command, {scope:"global"}) + ".then(() => false, () => true)"),true);
  }
  const after = await evaluate(invoke("app_get_status"));
  assert.equal(after.data.sequence, status.data.sequence);
  assert.equal(after.data.overlay.enabled, true);
  for (const [windowName, run] of [["main", evaluate], ["overlay", overlayEval]]) {
    assert.equal(await run("window.__TAURI_INTERNALS__.invoke('plugin:event|emit', { event: 'project-assistant://event/v1', payload: {} }).then(() => false, () => true)"), true);
    const denied = windowName === "main"
      ? await run(invoke("credential_put", { secret: "synthetic-secret-DO-NOT-LOG" }) + ".then(result => result.status === 'error' && result.error.code === 'PA_IPC_INVALID_REQUEST', () => false)")
      : await run(invoke("credential_put", { secret: "synthetic-secret-DO-NOT-LOG" }) + ".then(() => false, () => true)");
    assert.equal(denied, true, `${windowName} must reject credential_put`);
  }
  report.results.push("正式状态/错误往返、严格字段/UTF-8 上限/版本、窗口职责及未实现命令/前端事件伪造均在副作用前拒绝，关联 ID 唯一");

  await evaluate(`(() => {
    window.__ipcStats = { queries: 0, listens: 0, unlistens: 0 };
    const original = window.fetch;
    window.fetch = (url, options) => {
      const command = decodeURIComponent(new URL(String(url), location.href).pathname.slice(1));
      if (command === 'app_get_status') window.__ipcStats.queries++;
      if (command === 'plugin:event|listen') window.__ipcStats.listens++;
      if (command === 'plugin:event|unlisten') window.__ipcStats.unlistens++;
      return original(url, options);
    };
    window.__ipcEvents = [];
    const handler = window.__TAURI_INTERNALS__.transformCallback(event => window.__ipcEvents.push(event.payload));
    return window.__TAURI_INTERNALS__.invoke('plugin:event|listen', { event: 'project-assistant://event/v1', target: {kind:'WebviewWindow',label:'main'}, handler }).then(id => { window.__ipcListener = {id,handler}; });
  })()`);
  const contextReceipt = await fixture("context");
  await until(() => evaluate("document.querySelector('.app-shell')?.dataset.context"), s => s === "synthetic-project");
  await until(() => overlayEval("document.querySelector('.overlay-root')?.dataset.context"), s => s === "synthetic-project");
  assert.equal(await evaluate("window.__ipcEvents.at(-1).correlationId"), contextReceipt.correlationId);
  assert.equal(await evaluate("JSON.stringify(window.__ipcEvents).includes('draft')"), false);
  const beforeReplay = await evaluate("window.__ipcStats.queries");
  await fixture("replay"); await delay(300);
  assert.equal(await evaluate("window.__ipcStats.queries"), beforeReplay);
  await fixture("gap");
  const gap = await evaluate(invoke("app_get_status"));
  await until(() => evaluate("document.querySelector('.app-shell')?.dataset.sequence"), s => Number(s) === gap.data.sequence);
  await until(() => overlayEval("document.querySelector('.overlay-root')?.dataset.sequence"), s => Number(s) === gap.data.sequence);
  await fixture("restart");
  const restarted = await evaluate(invoke("app_get_status"));
  assert.notEqual(restarted.data.streamId, gap.data.streamId);
  await until(() => evaluate("document.querySelector('.app-shell')?.dataset.stream"), s => s === restarted.data.streamId);
  await until(() => overlayEval("document.querySelector('.overlay-root')?.dataset.stream"), s => s === restarted.data.streamId);
  report.results.push("合成活动项目/会话/脱敏 AI 选择在 main/overlay 同步，重放不重复查询，事件缺口及 streamId 变化自动恢复权威快照");

  await fixture("destroy_overlay");
  const response = await evaluate(invoke("overlay_control", { action: "expand" })); assert.equal(response.status, "ok");
  const target = await until(async () => (await (await fetch(`http://127.0.0.1:${debugPort}/json/list`)).json()).find(t => t.url.includes("window=overlay")), Boolean);
  const next = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((done, reject) => { next.addEventListener("open", done, {once:true}); next.addEventListener("error", reject, {once:true}); });
  next.addEventListener("message", receive); replaceOverlaySocket(next);
  await until(() => overlayEval("document.querySelector('.overlay-root')?.dataset.context"), s => s === "synthetic-project");
  assert.equal(await overlayEval("document.querySelector('#overlay-draft')?.value"), "中文草稿\n第二行 & DO-NOT-LOG");
  assert.deepEqual((await evaluate("window.__TAURI_INTERNALS__.invoke('plugin:window|get_all_windows')")).sort(), ["main", "overlay"]);
  report.results.push("销毁后重建唯一 overlay，通过状态查询恢复合成上下文及私有草稿");

  const baseline = await evaluate("({...window.__ipcStats})");
  for (let count = 0; count < 3; count++) {
    await evaluate("document.querySelector('.nav-item[data-page=settings]').click()");
    await until(() => evaluate("document.querySelector('#resident-enabled')?.disabled"), s => s === false);
    await evaluate("document.querySelector('.nav-item[data-page=home]').click()");
    await until(() => evaluate("document.querySelector('.main-area').dataset.page"), s => s === "home");
  }
  await until(() => evaluate("window.__ipcStats.unlistens - " + baseline.unlistens), s => s >= 3);
  const stats = await evaluate("({...window.__ipcStats})");
  assert.equal(stats.listens - baseline.listens, stats.unlistens - baseline.unlistens);
  await evaluate(`(() => {
    const {id,handler} = window.__ipcListener;
    window.__TAURI_EVENT_PLUGIN_INTERNALS__.unregisterListener('project-assistant://event/v1', id);
    return window.__TAURI_INTERNALS__.invoke('plugin:event|unlisten', {event:'project-assistant://event/v1',eventId:id}).then(() => window.__TAURI_INTERNALS__.unregisterCallback(handler));
  })()`);
  const events = await evaluate("window.__ipcEvents.length"); await fixture("clear"); await delay(300);
  assert.equal(await evaluate("window.__ipcEvents.length"), events);
  report.results.push("真实页面重复挂载/卸载的监听与注销数量匹配，注销后不再接收事件");

  await fixture("create_denied_window");
  const deniedTarget = await until(async () => (await (await fetch(`http://127.0.0.1:${debugPort}/json/list`)).json()).find(t => t.url.includes("window=denied")), Boolean);
  const deniedSocket = new WebSocket(deniedTarget.webSocketDebuggerUrl);
  await new Promise(done => deniedSocket.addEventListener("open", done, {once:true})); deniedSocket.addEventListener("message", receive);
  try {
    await until(() => evaluate("Boolean(window.__TAURI_INTERNALS__)", deniedSocket), Boolean);
    assert.equal(await evaluate(invoke("app_get_status") + ".then(() => false, () => true)", deniedSocket), true);
    assert.equal(await evaluate(invoke("settings_get", {scope:"global"}) + ".then(() => false, () => true)", deniedSocket), true);
    assert.equal(await evaluate(invoke("overlay_control", { action: "disable" }) + ".then(() => false, () => true)", deniedSocket), true);
  } finally { deniedSocket.close(); await fixture("close_denied_window"); }
  report.results.push("动态创建的未授权本地 WebView2 窗口被真实 capability 拒绝");

  await fixture("navigate_remote");
  await until(() => evaluate("document.title"), s => s === "IPC remote fixture");
  await until(() => evaluate("Boolean(window.__TAURI_INTERNALS__)", undefined), Boolean);
  assert.equal(await evaluate(invoke("app_get_status") + ".then(() => false, () => true)"), true);
  assert.equal(await evaluate(invoke("settings_get", {scope:"global"}) + ".then(() => false, () => true)"), true);
  assert.equal(await evaluate(invoke("overlay_control", { action: "disable" }) + ".then(() => false, () => true)"), true);
  await cdp("Page.navigate", { url: mainUrl });
  await until(() => evaluate("Boolean(document.querySelector('.app-shell'))"), Boolean);
  report.results.push("同一 main 窗口导航到本机 HTTP 远端页面后无 IPC 权限，恢复本地页面后应用正常");
}
