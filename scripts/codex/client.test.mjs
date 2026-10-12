import assert from "node:assert/strict";
import test from "node:test";
import { resolve } from "node:path";
import { AppServerClient, listModels, normalizeEvent, safePrintCommand, selectModel } from "./client.mjs";
import { connectionConfig, discoverCodex } from "./environment.mjs";

const fixture = resolve(import.meta.dirname, "fixture-server.mjs");
function client(t, mode = "normal") {
  const result = new AppServerClient({ executable: process.execPath, args: [fixture, mode], cwd: import.meta.dirname, timeoutMs: 1000 });
  t.after(() => result.close());
  return result;
}

test("correlates concurrent replies arriving out of order and decodes split UTF-8", async (t) => {
  const c = client(t);
  const results = await Promise.all([c.request("echo", { value: "first", delay: 15 }), c.request("echo", { value: "second" })]);
  assert.equal(results[0].value, "first");
  assert.equal(results[1].value, "second");
  assert.equal((await c.request("fragmented")).text, "中文");
  assert.equal(c.pending.size, 0);
});
test("replays early completion and separates server approval request IDs", async (t) => {
  const c = client(t);
  let approval;
  c.on("serverRequest", (request) => { approval = request; c.respond(request.id, { decision: "decline" }); });
  await c.request("approval");
  assert.equal(approval.id, "server-request");
  const after = c.sequence;
  await c.request("turn/start");
  const done = await c.waitFor((event) => event.method === "turn/completed", { after });
  assert.equal(normalizeEvent(done).type, "completed");
  assert.equal(normalizeEvent(c.events.find((event) => event.method === "item/agentMessage/delta")).bytes, 6);
  assert.equal(normalizeEvent(approval).type, "approval_required");
});
test("request timeout removes pending entries; event timeout removes listeners", async (t) => {
  const c = client(t, "hang");
  await assert.rejects(c.request("echo", {}, 30), { code: "CODEX_REQUEST_TIMEOUT" });
  assert.equal(c.pending.size, 0);
  await assert.rejects(c.waitFor(() => true, { timeoutMs: 30 }), { code: "CODEX_EVENT_TIMEOUT" });
  assert.equal(c.listenerCount("event"), 0);
  assert.equal(c.listenerCount("disconnected"), 0);
});
test("malformed frame fails pending calls without retaining payload", async (t) => {
  const c = client(t, "invalid");
  await assert.rejects(c.request("echo"), { code: "CODEX_INVALID_FRAME" });
});
test("disconnect rejects pending and future requests", async (t) => {
  const c = client(t, "exit");
  await assert.rejects(c.request("echo"), { code: "CODEX_DISCONNECTED" });
  await assert.rejects(c.request("echo"), { code: "CODEX_DISCONNECTED" });
});
test("remote errors expose numeric RPC code but discard sensitive message", async (t) => {
  const c = client(t);
  await assert.rejects(c.request("rpc-error"), (error) => error.rpcCode === -32600 && !String(error).includes("secret"));
});
test("missing executable produces actionable error and can close", async (t) => {
  const c = new AppServerClient({ executable: resolve(import.meta.dirname, "missing-codex.exe"), cwd: import.meta.dirname });
  t.after(() => c.close());
  await assert.rejects(c.initialize(), { code: "CODEX_NOT_INSTALLED" });
  await assert.rejects(discoverCodex("relative.exe"), { code: "CODEX_PATH_NOT_ABSOLUTE" });
});
test("model changes validate the new effort options and freeze selection", () => {
  const models = [{ model: "one", defaultReasoningEffort: "high", supportedReasoningEfforts: [{ reasoningEffort: "high" }] },
    { model: "two", defaultReasoningEffort: "low", supportedReasoningEfforts: [{ reasoningEffort: "low" }] }];
  assert.equal(selectModel(models, "two").effort, "low");
  assert.throws(() => selectModel(models, "two", "high"), { code: "EFFORT_UNSUPPORTED" });
  assert.throws(() => selectModel(models, "missing"), { code: "MODEL_UNAVAILABLE" });
  assert.ok(Object.isFrozen(selectModel(models, "one")));
});
test("model pagination collects all pages and rejects cursor loops", async () => {
  let calls = 0;
  const result = await listModels({ request: async () => ({ data: [{ model: ++calls }], nextCursor: calls < 2 ? "next" : null }) });
  assert.equal(result.length, 2);
  await assert.rejects(listModels({ request: async () => ({ data: [], nextCursor: "loop" }) }), { code: "MODEL_PAGINATION_LOOP" });
});
test("connection-only import drops external hooks, plugins, MCP and trust", () => {
  const source = 'model = "test"\nnotify = ["evil"]\n[desktop]\nsecret = "ignored"\n[mcp_servers.x]\ncommand = "evil"\n[model_providers.site]\nname = "site"\nhttp_headers = { Authorization = "private" }\n[projects.x]\ntrust_level = "trusted"\n';
  const config = connectionConfig(source);
  assert.ok(config.includes('model = "test"'));
  assert.ok(config.includes('http_headers = { Authorization = "private" }'));
  assert.ok(!/evil|desktop|trust_level|mcp_servers/.test(config));
  assert.throws(() => connectionConfig('model = """\nsecret\n"""'), { code: "CONFIG_IMPORT_UNSUPPORTED" });
});
test("approval whitelist accepts only the exact print operation with trusted shell", () => {
  const trusted = ['C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe'];
  assert.ok(safePrintCommand('"C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe" -Command Get-Location', trusted));
  assert.ok(safePrintCommand('"C:\\\\Windows\\\\System32\\\\WindowsPowerShell\\\\v1.0\\\\powershell.exe" -Command Get-Location', trusted));
  assert.ok(safePrintCommand("Get-Location"));
  for (const command of ["Get-Location; Remove-Item *", "Get-Location | Out-File x", "pwd && whoami", "powershell.exe -EncodedCommand abcd", '"C:\\evil\\powershell.exe" -Command Get-Location']) {
    assert.equal(safePrintCommand(command, trusted), false);
  }
});
test("interrupted and failed map to distinct terminal states", () => {
  assert.equal(normalizeEvent({ method: "turn/completed", params: { turn: { id: "r", status: "interrupted" } } }).type, "cancelled");
  assert.equal(normalizeEvent({ method: "turn/completed", params: { turn: { id: "r", status: "failed" } } }).type, "failed");
});
