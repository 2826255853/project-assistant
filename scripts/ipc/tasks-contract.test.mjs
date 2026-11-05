import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { isStateEvent } from "../../src/ipc/state.ts";
import { isApprovalReceipt, isApprovalPage, isCancelReceipt } from "../../src/ipc/taskContracts.ts";
import { errorForCode } from "../../src/ipc/errors.ts";

const read = path => readFile(new URL(path, import.meta.url), "utf8");
const taskCommands = [
  "active_context_set",
  "session_create", "session_find_by_request", "sessions_list", "session_get", "session_resume", "session_end",
  "task_start", "task_find_by_request", "tasks_list", "task_get", "task_cancel",
  "task_content_get", "task_events_get", "approvals_get", "approval_decide",
  "task_changes_get", "task_change_get", "task_start_from_active", "overlay_task_get",
  "overlay_task_cancel", "overlay_task_find_by_request",
];

test("approval receipts omit internal consumer/window authority and cancellation is acceptance only", () => {
  const id = "00000000-0000-4000-8000-000000000001";
  const approval = {id, taskId: id, sessionId: id, revision: 2, operationKind: "command", operationFingerprint: "f".repeat(64),
    displaySummary: "单次执行命令：Get-Location", decision: "approved", consumption: "claimed", requestedAt: 1, expiresAt: 300001, decidedAt: 2};
  assert.equal(isApprovalReceipt({approval, duplicate: false}), true);
  assert.equal(isApprovalPage({items: [approval]}), true);
  for (const key of ["windowLabel", "consumeRequestId", "command", "secret", "externalRequestId"]) {
    assert.equal(isApprovalReceipt({approval: {...approval, [key]: "private"}, duplicate: false}), false);
  }
  assert.equal(isApprovalReceipt({approval: {...approval, displaySummary: "x".repeat(2049)}, duplicate: false}), false);
  assert.equal(isCancelReceipt({accepted: true, status: "cancelling", revision: 3}), true);
  assert.equal(isCancelReceipt({accepted: false, status: "completed", revision: 4}), true);
  assert.equal(isCancelReceipt({accepted: true, status: "cancelling", revision: 3, terminated: true}), false);
  for (const code of ["PA_APPROVAL_NOT_PENDING", "PA_APPROVAL_EXPIRED", "PA_APPROVAL_BINDING_MISMATCH", "PA_APPROVAL_UNCERTAIN"]) {
    assert.equal(errorForCode(code).code, code);
    assert.equal(errorForCode(code).retryable, false);
  }
});

test("S3-06 exposes session/task gateway and approval decisions only to main", async () => {
  const [contracts, taskContracts, client, rust, manifest, main, overlay] = await Promise.all([
    read("../../src/ipc/contracts.ts"), read("../../src/ipc/taskContracts.ts"),
    read("../../src/ipc/client.ts"), read("../../src-tauri/src/lib.rs"),
    read("../../src-tauri/build.rs"), read("../../src-tauri/capabilities/default.json").then(JSON.parse),
    read("../../src-tauri/capabilities/overlay.json").then(JSON.parse),
  ]);
  assert.ok(contracts.includes('Extract<keyof ImplementedCommandMap, "active_context_set"'));
  const declared = [...taskContracts.split("export interface TaskCommandMap {")[1].matchAll(/^  ([a-z_]+):/gm)].map(match => match[1]);
  assert.deepEqual(declared.sort(), [...taskCommands].sort(), "every future task command must be checked");
  const implementedCommands = ["session_create", "session_find_by_request", "sessions_list", "session_get", "session_resume", "session_end",
    "task_start", "task_find_by_request", "tasks_list", "task_get", "task_cancel", "task_events_get", "approvals_get", "approval_decide", "task_content_get", "task_changes_get", "task_change_get"];
  for (const name of implementedCommands) {
    assert.ok(contracts.match(/export interface ImplementedCommandMap \{([\s\S]*?)^\}/m)?.[1].includes(name), `${name} must be in the callable map`);
    assert.match(client, new RegExp(`\\b${name}:`), `${name} must have a runtime validator`);
    assert.match(rust, new RegExp(`#\\[tauri::command\\]\\s*(?:async\\s+)?fn\\s+${name}\\s*\\(`));
    assert.ok(manifest.includes(`"${name}"`));
    assert.ok(main.permissions.includes(`allow-${name.replaceAll("_", "-")}`));
    assert.ok(!overlay.permissions.includes(`allow-${name.replaceAll("_", "-")}`));
  }
  for (const name of taskCommands.filter(name => !implementedCommands.includes(name))) {
    assert.match(taskContracts, new RegExp(`\\b${name}:`));
    assert.ok(!contracts.match(/export interface ImplementedCommandMap \{([\s\S]*?)^\}/m)?.[1].includes(name), `${name} must stay unimplemented`);
    assert.ok(!client.includes(`"${name}"`), `${name} must not have an invocation path`);
    assert.doesNotMatch(rust, new RegExp(`#\\[tauri::command\\]\\s*(?:async\\s+)?fn\\s+${name}\\s*\\(`), `${name} must not have a Rust IPC handler`);
    assert.ok(!manifest.includes(`"${name}"`), `${name} must not be registered`);
    assert.ok(!main.permissions.some(p => p === `allow-${name.replaceAll("_", "-")}`));
    assert.ok(!overlay.permissions.some(p => p === `allow-${name.replaceAll("_", "-")}`));
  }
});

test("task body, deltas and approval data remain outside the invalidation event contract", async () => {
  const contracts = await read("../../src/ipc/contracts.ts");
  const payload = contracts.match(/export type EventPayload =([\s\S]*?);/m)?.[1] ?? "";
  assert.match(payload, /app_state_changed/);
  assert.doesNotMatch(payload, /task_delta|approval_required|reply|prompt|displaySummary/);
  const event = { apiVersion: 1, streamId: "stream-ipc-1-ab-0", sequence: 1,
    eventId: "stream-ipc-1-ab-0-1", occurredAt: 1, correlationId: "ipc-1-ab-0",
    payload: { type: "app_state_changed" } };
  assert.equal(isStateEvent(event), true);
  for (const field of ["prompt", "reply", "text", "displaySummary", "operationFingerprint", "absolutePath", "secret"]) {
    assert.equal(isStateEvent({ ...event, payload: { ...event.payload, [field]: "synthetic-private-value" } }), false);
  }
  for (const type of ["task_delta", "approval_required", "task_status"]) {
    assert.equal(isStateEvent({ ...event, payload: { type } }), false);
  }
});
