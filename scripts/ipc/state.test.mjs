import assert from "node:assert/strict";
import test from "node:test";
import { setTimeout as delay } from "node:timers/promises";
import { subscribeState, isStateEvent, isAppStatus } from "../../src/ipc/state.ts";
import { awaitTransport, TIMEOUT_ERROR, CANCELLED_ERROR } from "../../src/ipc/transport.ts";
import { TRANSPORT_ERROR } from "../../src/ipc/response.ts";

const event = (sequence, streamId = "stream-ipc-1-ab-0") => ({ apiVersion: 1, eventId: `${streamId}-${sequence}`, streamId, sequence, occurredAt: 1, correlationId: "ipc-1-ab-0", payload: { type: "app_state_changed" } });
const snapshot = (sequence = 0, streamId = "stream-ipc-1-ab-0") => ({ apiVersion: 1, streamId, sequence });

test("replay ignored, gaps and changed streams queried; never resend a mutation", async () => {
  let receive, queries = 0, disposed = 0, current = snapshot();
  const updates = [];
  const cleanup = subscribeState({ listen: async cb => { receive = cb; return () => disposed++; }, query: async () => { queries++; return current; } }, s => updates.push(s));
  await delay(0);
  assert.equal(queries, 1);
  current = snapshot(1); receive(event(1)); await delay(0);
  receive(event(1)); receive(event(1)); await delay(0);
  assert.equal(queries, 2);
  current = snapshot(4); receive(event(4)); await delay(0);
  assert.equal(updates.at(-1).sequence, 4);
  current = snapshot(1, "stream-ipc-2-cd-0"); receive(event(1, current.streamId)); await delay(0);
  assert.equal(updates.at(-1).streamId, current.streamId);
  receive(event(8)); await delay(0);
  assert.equal(queries, 4, "late replay from retired stream ignored");
  receive({ ...event(2), payload: { type: "future_event" } }); await delay(0);
  assert.equal(queries, 5);
  cleanup(); cleanup(); receive(event(3)); await delay(0);
  assert.equal(queries, 5); assert.equal(disposed, 1);
});

test("snapshot ahead of triggering event suppresses already incorporated events", async () => {
  let receive, queries = 0, current = snapshot();
  const cleanup = subscribeState({ listen: async cb => { receive = cb; return () => {}; }, query: async () => { queries++; return current; } }, () => {});
  await delay(0); current = snapshot(5); receive(event(1)); await delay(0);
  receive(event(2)); await delay(0); assert.equal(queries, 2); cleanup();
});

test("event during snapshot query cannot roll state back", async () => {
  let receive, finish;
  const updates = [], answers = [new Promise(resolve => finish = resolve), Promise.resolve(snapshot(3))];
  const cleanup = subscribeState({ listen: async cb => { receive = cb; return () => {}; }, query: () => answers.shift() }, s => updates.push(s));
  await delay(0);
  receive(event(3)); finish(snapshot(0)); await delay(0);
  assert.deepEqual(updates.map(s => s.sequence), [3]); cleanup();
});

test("unmount while registration pending still unregisters and performs no query", async () => {
  let finish, disposed = 0;
  const cleanup = subscribeState({ listen: () => new Promise(resolve => finish = resolve), query: async () => assert.fail("query after cleanup") }, () => assert.fail("update after cleanup"));
  cleanup(); finish(() => disposed++); await delay(0); assert.equal(disposed, 1);
});

test("disposed inflight snapshots cannot update; query failure is recoverable", async () => {
  let receive, finish;
  const updates = [], failures = [];
  let queries = 0;
  const cleanup = subscribeState({ listen: async cb => { receive = cb; return () => {}; }, query: () => ++queries === 1 ? Promise.reject(new Error("fixture")) : new Promise(resolve => finish = resolve) }, s => updates.push(s), e => failures.push(e));
  await delay(0); assert.equal(failures.length, 1);
  receive(event(1)); cleanup(); finish(snapshot(1)); await delay(0); assert.equal(updates.length, 0);
});

test("runtime event validation rejects incompatible, oversized or unknown payload", () => {
  assert.equal(isStateEvent(event(1)), true);
  for (const wire of [null, { ...event(1), apiVersion: 2 }, { ...event(1), sequence: -1 }, { ...event(1), sequence: 1.5 },
    { ...event(1), eventId: "forged" }, { ...event(1), payload: { type: "app_state_changed", draft: "secret" } },
    { ...event(1), extra: "x".repeat(16384) }]) assert.equal(isStateEvent(wire), false);
  assert.equal(isAppStatus(snapshot()), false);
});

test("app status carries a bounded process-only active project independently of task context", () => {
  const services = ["desktop","ipc","events","overlay","storage","settings","logging","credentials","providers","codex","projects","tasks"]
    .map(id => ({id,state:"ready",reasonCode:null}));
  const status = {
    apiVersion:1,streamId:"stream-ipc-1-ab-0",sequence:2,configState:"unconfigured",services,
    ccSwitch:{management:"user_manual",integration:"disabled",readSupported:false,switchSupported:false},
    activeContext:null,taskStatus:"idle",
    overlay:{enabled:true,expanded:false,shortcut:"Ctrl+Alt+Space",shortcutError:null,windowError:null,draft:"",trayReady:true},
    activeProject:{projectId:"c14e7b09-b84c-44bc-9b57-a7182e70b056",name:"synthetic-project"},
  };
  assert.equal(isAppStatus(status),true);
  assert.equal(isAppStatus({...status,activeProject:{projectId:status.activeProject.projectId,name:"x\u0000"}}),false);
  assert.equal(isAppStatus({...status,activeProject:{projectId:status.activeProject.projectId,name:"x".repeat(1025)}}),false);
});

test("transport timeout, AbortSignal and native exceptions have fixed errors and one send", async () => {
  let sends = 0;
  await assert.rejects(awaitTransport(() => { sends++; return new Promise(() => {}); }, { timeoutMs: 5 }), { message: TIMEOUT_ERROR });
  const controller = new AbortController();
  const waiting = awaitTransport(() => { sends++; return new Promise(() => {}); }, { signal: controller.signal });
  await delay(0); controller.abort(); await assert.rejects(waiting, { message: CANCELLED_ERROR });
  await assert.rejects(awaitTransport(async () => { sends++; throw new Error("synthetic-secret"); }), { message: TRANSPORT_ERROR });
  await assert.rejects(awaitTransport(async () => { assert.fail("pre-aborted request was sent"); }, { signal: controller.signal }), { message: CANCELLED_ERROR });
  assert.equal(sends, 3);
});
