import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { isStateEvent } from "../../src/ipc/state.ts";

const read = path => readFile(new URL(path, import.meta.url), "utf8");
const taskCommands = [
  "active_context_set",
  "session_create", "session_find_by_request", "sessions_list", "session_get", "session_resume", "session_end",
  "task_start", "task_find_by_request", "tasks_list", "task_get", "task_cancel",
  "task_content_get", "task_events_get", "approvals_get", "approval_decide",
  "task_changes_get", "task_change_get", "task_start_from_active", "overlay_task_get",
  "overlay_task_cancel", "overlay_task_find_by_request",
];

test("task IPC commands stay unregistered until their service and authorization gates ship", async () => {
  const [contracts, taskContracts, client, rust, manifest, main, overlay] = await Promise.all([
    read("../../src/ipc/contracts.ts"), read("../../src/ipc/taskContracts.ts"),
    read("../../src/ipc/client.ts"), read("../../src-tauri/src/lib.rs"),
    read("../../src-tauri/build.rs"), read("../../src-tauri/capabilities/default.json").then(JSON.parse),
    read("../../src-tauri/capabilities/overlay.json").then(JSON.parse),
  ]);
  assert.ok(contracts.includes('Extract<keyof ImplementedCommandMap, keyof TaskCommandMap> extends never ? true : false'));
  const declared = [...taskContracts.split("export interface TaskCommandMap {")[1].matchAll(/^  ([a-z_]+):/gm)].map(match => match[1]);
  assert.deepEqual(declared.sort(), [...taskCommands].sort(), "every future task command must be checked");
  for (const name of taskCommands) {
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
