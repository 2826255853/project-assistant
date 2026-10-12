import assert from "node:assert/strict";
import test from "node:test";
import { parseResponse, isProbeReply, unwrap, CONTRACT_ERROR } from "../../src/ipc/response.ts";

test("v1 success and typed error preserve safe correlation without transport details", () => {
  const success = { apiVersion: 1, correlationId: "ipc-1-ab-0", status: "ok", data: { message: "中文 & <>", platform: "windows", architecture: "x86_64" } };
  assert.equal(unwrap(parseResponse(success, isProbeReply)).message, "中文 & <>");
  const failure = { apiVersion: 1, correlationId: "ipc-1-ab-1", status: "error", error: { category: "validation", code: "PA_VALIDATION_EMPTY_MESSAGE", message: "测试消息不能为空。", suggestion: "输入后重试。", retryable: false } };
  assert.throws(() => unwrap(parseResponse(failure, isProbeReply)), /PA_VALIDATION_EMPTY_MESSAGE.*ipc-1-ab-1/);
});

test("malformed or incompatible response fails closed with fixed public error", () => {
  const secret = "synthetic-secret-DO-NOT-DISPLAY";
  for (const wire of [null, secret, { apiVersion: 2 }, { apiVersion: 1, correlationId: secret, status: "ok", data: {} },
    { apiVersion: 1, correlationId: "ipc-1-ab-0", status: "ok", data: { message: secret } },
    { apiVersion: 1, correlationId: "ipc-1-ab-0", status: "error", error: { code: secret } },
    { apiVersion: 1, correlationId: "ipc-1-ab-0", status: "ok", data: { message: secret, platform: "windows", architecture: "x86_64" }, error: {} }]) {
    assert.throws(() => parseResponse(wire, isProbeReply), { message: CONTRACT_ERROR });
  }
});

test("OS process arguments preserve injection-looking input as a single literal", async () => {
  const { spawnSync } = await import("node:child_process");
  const payload = '中文 & echo injected; $(echo injected) | < > " \\ ../';
  const result = spawnSync(process.execPath, ["-e", "process.stdout.write(JSON.stringify(process.argv.slice(1)))", "--", payload], { shell: false, windowsHide: true, encoding: "utf8", timeout: 5000 });
  assert.equal(result.status, 0);
  assert.deepEqual(JSON.parse(result.stdout), [payload]);
});

test("IPC permissions are limited to the local main window and explicit commands", async () => {
  const { readFile } = await import("node:fs/promises");
  const config = JSON.parse(await readFile(new URL("../../src-tauri/capabilities/default.json", import.meta.url), "utf8"));
  assert.deepEqual(config.windows, ["main"]);
  assert.equal(config.remote, undefined);
  assert.notEqual(config.local, false);
  assert.ok(config.permissions.includes("allow-desktop-initialize"));
  assert.ok(!config.permissions.includes("allow-framework-probe"));
  assert.ok(!config.permissions.includes("allow-phase0-check-open-path"));
  assert.ok(!config.permissions.some((p) => JSON.stringify(p).includes("opener:")));
  const phase0 = JSON.parse(await readFile(new URL("../../src-tauri/capabilities/phase0.json", import.meta.url), "utf8"));
  assert.deepEqual(phase0.windows, ["main"]);
  assert.equal(phase0.remote, undefined);
  assert.ok(phase0.permissions.includes("allow-framework-probe"));
  assert.ok(phase0.permissions.includes("allow-phase0-check-open-path"));
  const tauri = JSON.parse(await readFile(new URL("../../src-tauri/tauri.conf.json", import.meta.url), "utf8"));
  assert.deepEqual(tauri.app.security.capabilities, ["default", "overlay"]);
  const overlay = JSON.parse(await readFile(new URL("../../src-tauri/capabilities/overlay.json", import.meta.url), "utf8"));
  assert.deepEqual(overlay.windows, ["overlay"]);
  assert.ok(!overlay.permissions.includes("allow-desktop-initialize"));
  assert.ok(!overlay.permissions.some((p) => /credential|probe|opener/.test(p)));
  const build = await readFile(new URL("../../src-tauri/build.rs", import.meta.url), "utf8");
  assert.ok(build.includes("AppManifest::new().commands"));
  assert.ok(!config.permissions.some((p) => typeof p === "string" && /shell|fs:|allow-all/.test(p)));
});
