import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

export async function verifyCredentialsDesktop({evaluate, until, report, debugPort, receive, cdp, mainUrl, fixtureDirectory}) {
  const secret = "synthetic-credential-desktop-DO-NOT-EXPORT";
  const profileId = `desktop-credential-${createHash("sha256").update(fixtureDirectory).digest("hex").slice(0, 24)}`;
  const invoke = (command, params = {}, apiVersion = 1) => `window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)}, {request:${JSON.stringify({apiVersion, params})}})`;
  const call = async (command, params, version) => {
    const wire = await evaluate(invoke(command, params, version));
    assert.ok(!JSON.stringify(wire).includes(secret), "Credential must not enter response");
    return wire;
  };
  const fixture = async action => assert.equal((await call("ipc_fixture", {action})).status, "ok");
  const sockets = [];
  await until(() => evaluate("document.querySelector('.main-area')?.dataset.page"), page => page === "home");
  try {
    await fixture("credential_seed");
    const params = {profileId, expectedRevision: 1, secret};
    for (const [command, data, version, code] of [
      ["credential_put", params, 2, "PA_IPC_VERSION_UNSUPPORTED"],
      ["credential_put", {...params, unknown: secret}, 1, "PA_IPC_INVALID_REQUEST"],
      ["credential_put", {...params, expectedRevision: "1"}, 1, "PA_IPC_INVALID_REQUEST"],
      ["credential_put", {...params, secret: "x".repeat(25 * 1024)}, 1, "PA_VALIDATION_INPUT_TOO_LARGE"],
      ["credential_put", {...params, secret: ""}, 1, "PA_CREDENTIAL_INVALID"],
      ["credential_put", {...params, profileId: "missing-profile"}, 1, "PA_CREDENTIAL_PROFILE_NOT_FOUND"],
      ["credential_remove", {profileId, expectedRevision: 1, secret}, 1, "PA_IPC_INVALID_REQUEST"],
    ]) assert.equal((await call(command, data, version)).error.code, code);
    const put = await call("credential_put", params);
    assert.deepEqual(put.data, {credential: {status: "configured_unverified", revision: 1}, configRevision: 2});
    await fixture("credential_assert");
    assert.equal((await call("credential_put", params)).error.code, "PA_CREDENTIAL_REVISION_CONFLICT");
    report.results.push("真实 main 凭据写入/读取绑定、严格请求/长度/修订校验及能力缓存失效通过；返回仅状态和版本");

    await fixture("audit_fail_write");
    assert.equal((await call("credential_put", {...params, expectedRevision: 2})).error.code, "PA_AUDIT_WRITE_FAILED");
    assert.equal((await call("credential_remove", {profileId, expectedRevision: 2})).error.code, "PA_AUDIT_WRITE_FAILED");
    await fixture("credential_assert");
    await fixture("audit_restore_write");
    report.results.push("真实 IPC 替换/删除遇审计 SQL 故障均拒绝，Windows 凭据和原引用补偿恢复");

    await fixture("credential_occupy");
    await evaluate(`window.__credentialPending = null; ${invoke("credential_put", {...params, expectedRevision: 2})}.then(value => {window.__credentialPending = value;}); true`);
    await delay(350);
    assert.equal(await evaluate("window.__credentialPending"), null);
    assert.equal((await call("app_get_status")).status, "ok");
    await fixture("credential_idle");
    const completed = await until(() => evaluate("window.__credentialPending"), Boolean);
    assert.deepEqual(completed.data, {credential: {status: "configured_unverified", revision: 2}, configRevision: 3});
    await fixture("credential_assert");
    report.results.push("合成任务占用期间凭据变更等待，状态查询仍响应，任务空闲后同一请求完成");

    for (const [query, create, close] of [["window=overlay", null, null], ["window=denied", "create_denied_window", "close_denied_window"]]) {
      if (create) await fixture(create);
      const target = await until(async () => (await (await fetch(`http://127.0.0.1:${debugPort}/json/list`)).json()).find(value => value.url.includes(query)), Boolean);
      const socket = new WebSocket(target.webSocketDebuggerUrl); sockets.push(socket);
      await new Promise((done, reject) => { socket.addEventListener("open", done, {once:true}); socket.addEventListener("error", reject, {once:true}); });
      socket.addEventListener("message", receive);
      await until(() => evaluate("Boolean(window.__TAURI_INTERNALS__)", socket), Boolean);
      for (const command of ["credential_put", "credential_remove"]) {
        assert.equal(await evaluate(`${invoke(command, params)}.then(() => false, () => true)`, socket), true);
      }
      socket.close();
      if (close) await fixture(close);
    }
    await fixture("navigate_remote");
    await until(() => evaluate("document.title"), title => title === "IPC remote fixture");
    for (const command of ["credential_put", "credential_remove"]) assert.equal(await evaluate(`${invoke(command, params)}.then(() => false, () => true)`), true);
    await cdp("Page.navigate", {url: mainUrl});
    await until(() => evaluate("Boolean(document.querySelector('.app-shell'))"), Boolean);
    report.results.push("overlay、未授权本地窗口和远端 main 页面均被真实 capability 拒绝凭据命令");

    const removed = await call("credential_remove", {profileId, expectedRevision: 3});
    assert.deepEqual(removed.data, {credential: {status:"missing", revision:3}, configRevision:4});
    assert.deepEqual((await call("credential_remove", {profileId, expectedRevision:4})).data, removed.data);
    for (const command of ["credential_get", "credential_read", "credential_resolve"]) assert.equal(await evaluate(`${invoke(command, {profileId})}.then(() => false, () => true)`), true);
    assert.ok(!JSON.stringify((await call("app_get_status")).data).includes(secret));
    async function scan(directory) {
      for (const entry of await readdir(directory, {withFileTypes:true})) {
        const path = join(directory, entry.name);
        if (entry.isDirectory()) await scan(path);
        else assert.equal((await readFile(path)).includes(Buffer.from(secret)), false, "Credential leaked to ordinary database, audit or file");
      }
    }
    await scan(fixtureDirectory);
    if (process.env.PA_LOG_VERIFY_DIR) await scan(process.env.PA_LOG_VERIFY_DIR);
    report.results.push("删除及重复删除通过；无 secret 读取 IPC；状态、SQLite/WAL/审计和持久日志无合成秘密");
  } finally {
    for (const socket of sockets) socket.close();
    // Navigate back before exact synthetic target cleanup, even after a failure.
    await cdp("Page.navigate", {url: mainUrl});
    await until(() => evaluate("Boolean(document.querySelector('.app-shell'))"), Boolean);
    await fixture("credential_idle");
    await fixture("credential_cleanup");
    report.credentialCleanupPassed = true;
  }
}
