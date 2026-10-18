import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { AppServerClient, ProbeError, listModels, selectModel } from "./codex/client.mjs";
import { discoverCodex, isolatedEnvironment } from "./codex/environment.mjs";
import { controlledSite, controlledConfig } from "./codex/controlled-site.mjs";
import { profileSnapshot } from "./codex/profile.mjs";

const output = resolve(import.meta.dirname, "..", "artifacts", "codex-controlled");
await mkdir(output, { recursive: true });
const report = { date: new Intl.DateTimeFormat("sv-SE", { timeZone: "Asia/Shanghai" }).format(new Date()),
  mode: "real-cli-synthetic-http-providers", realOfficialAuthentication: "not-tested",
  ccSwitchIntegration: "not-tested", checks: [], passed: false };
const sites = {};
const clients = new Set();
const exits = [];
let isolated;
let executable;
let lastEphemeralThreadId;

async function check(name, run) {
  console.log(`Checking ${name}`);
  try { const evidence = await run(); report.checks.push({ name, status: "passed", evidence }); return evidence; }
  catch (error) { report.checks.push({ name, status: "failed", code: error instanceof ProbeError ? error.code : "PROBE_ASSERTION_FAILED", rpcCode: error.rpcCode }); throw error; }
}
async function startClient() {
  const client = new AppServerClient({ executable, args: ["app-server", "--listen", "stdio://"], cwd: isolated.project, env: isolated.env });
  clients.add(client);
  client.on("serverRequest", (event) => {
    if (event.method.endsWith("/requestApproval")) client.respond(event.id, { decision: "decline" });
    else client.rejectRequest(event.id);
  });
  await client.initialize();
  return client;
}
async function stop(client) { exits.push(await client.close()); clients.delete(client); }
async function turn(client, provider) {
  const models = await listModels(client);
  const selection = selectModel(models, models.some((item) => item.model === "gpt-5.1-codex-mini") ? "gpt-5.1-codex-mini" : models[0].model, "medium");
  const thread = await client.request("thread/start", { cwd: isolated.project, model: selection.model,
    modelProvider: provider, config: { model_reasoning_effort: selection.effort }, ephemeral: true, sandbox: "read-only", approvalPolicy: "untrusted" });
  assert.equal(thread.modelProvider, provider);
  lastEphemeralThreadId = thread.thread.id;
  assert.equal(thread.reasoningEffort, selection.effort);
  const after = client.sequence;
  const result = await client.request("turn/start", { threadId: thread.thread.id, model: selection.model,
    effort: selection.effort, input: [{ type: "text", text: "Do not use tools. Reply OK.", text_elements: [] }] });
  const completed = (timeoutMs = 30000) => client.waitFor((event) => event.method === "turn/completed" &&
    event.params.threadId === thread.thread.id && event.params.turn.id === result.turn.id, { after, timeoutMs });
  return { after, threadId: thread.thread.id, turnId: result.turn.id, selection, completed };
}
async function complete(client, provider) {
  const started = await turn(client, provider);
  const event = await started.completed();
  assert.equal(event.params.turn.status, "completed");
  return { model: started.selection.model, effort: started.selection.effort, status: "completed" };
}

try {
  executable = await discoverCodex();
  const keyA = `synthetic-${randomUUID()}`;
  const keyB = `synthetic-${randomUUID()}`;
  sites.probe_a = await controlledSite("A", keyA);
  sites.probe_b = await controlledSite("B", keyB);
  const configA = controlledConfig("probe_a", sites);
  const configB = controlledConfig("probe_b", sites);
  isolated = await isolatedEnvironment({ inheritConnection: false, config: configA });
  // Remove inherited generic auth. Only synthetic per-site keys are accessible here.
  delete isolated.env.OPENAI_API_KEY;
  delete isolated.env.CODEX_API_KEY;
  isolated.env.PHASE_0_3_PROBE_A_KEY = keyA;
  isolated.env.PHASE_0_3_PROBE_B_KEY = keyB;
  let active = await startClient();
  await check("key-owner-binding-before-send", async () => {
    const profile = { id: "probe_a", endpoint: sites.probe_a.baseUrl };
    assert.throws(() => profileSnapshot(profile,
      { profileId: "probe_b", origin: new URL(sites.probe_b.baseUrl).origin, reference: "synthetic:b" },
      { model: "test", effort: "medium" }, { allowLoopback: true }), { code: "KEY_SITE_MISMATCH" });
    assert.equal(sites.probe_a.requests.length, 0);
    return { error: "KEY_SITE_MISMATCH", networkRequests: 0 };
  });
  await check("site-a-and-effective-parameters", async () => {
    const result = await complete(active, "probe_a");
    const request = sites.probe_a.requests.at(-1);
    assert.ok(request.keyMatches);
    assert.equal(request.model, result.model);
    assert.equal(request.effort, result.effort);
    assert.equal(sites.probe_b.requests.length, 0);
    return { ...result, siteKeyMatches: true, httpPayloadMatches: true };
  });
  await check("active-turn-config-snapshot", async () => {
    sites.probe_a.setMode("hold");
    const running = await turn(active, "probe_a");
    await active.waitFor((event) => event.method === "item/agentMessage/delta" && event.params.threadId === running.threadId, { after: running.after, timeoutMs: 30000 });
    await writeFile(join(isolated.home, "config.toml"), configB, { mode: 0o600 });
    sites.probe_a.release();
    sites.probe_a.setMode("ok");
    assert.equal((await running.completed()).params.turn.status, "completed");
    assert.equal(sites.probe_b.requests.length, 0);
    let catalogAfterChange;
    try { await listModels(active); catalogAfterChange = "returned"; }
    catch (error) { catalogAfterChange = error instanceof ProbeError ? `${error.code}:${error.rpcCode}` : "failed"; }
    const observedConfig = (await active.request("config/read", { includeLayers: false, cwd: isolated.project })).config;
    const newThread = await active.request("thread/start", { cwd: isolated.project, ephemeral: true, sandbox: "read-only", approvalPolicy: "untrusted" });
    return { runningTurnStayedOnA: true, catalogAfterChange, configReadProviderAfterChange: observedConfig.model_provider,
      defaultNewThreadProviderBeforeRestart: newThread.modelProvider,
      policy: "wait-for-owned-process-idle-then-restart-and-create-new-thread" };
  });
  const oldEphemeralThreadId = lastEphemeralThreadId;
  await stop(active);
  active = await startClient();
  await check("ephemeral-thread-restart-recovery-limit", async () => {
    await assert.rejects(active.request("thread/resume", { threadId: oldEphemeralThreadId }), { code: "CODEX_RPC_ERROR" });
    return { resumeAfterProcessRestart: "rejected", strategy: "create-new-thread", persistentThreadResume: "not-tested" };
  });
  await check("restart-site-b", async () => {
    const result = await complete(active, "probe_b");
    assert.ok(sites.probe_b.requests.at(-1).keyMatches);
    return { ...result, siteKeyMatches: true };
  });
  await check("failed-candidate-keeps-prior-profile", async () => {
    sites.probe_a.setMode("unauthorized");
    // Candidate gets its own home and process; the active B process never changes.
    const candidateHome = await isolatedEnvironment({ inheritConnection: false, config: configA });
    let candidate;
    try {
      candidate = new AppServerClient({ executable, args: ["app-server", "--listen", "stdio://"], cwd: candidateHome.project,
        env: { ...isolated.env, CODEX_HOME: candidateHome.home } });
      clients.add(candidate);
      candidate.on("serverRequest", (event) => candidate.rejectRequest(event.id));
      await candidate.initialize();
      const started = await turn(candidate, "probe_a");
      const failed = await started.completed();
      assert.equal(failed.params.turn.status, "failed");
      assert.equal(await readFile(join(isolated.home, "config.toml"), "utf8"), configB);
      await complete(active, "probe_b");
      return { candidateStatus: "failed", originalConfigPreserved: true, previousProviderStillCompletes: true };
    } finally { if (candidate) await stop(candidate); await candidateHome.cleanup(); sites.probe_a.setMode("ok"); }
  });
  await stop(active);
  await writeFile(join(isolated.home, "config.toml"), configA, { mode: 0o600 });
  active = await startClient();
  await check("roundtrip-back-to-site-a", async () => {
    const result = await complete(active, "probe_a");
    assert.ok(sites.probe_a.requests.at(-1).keyMatches);
    return { ...result, siteKeyMatches: true };
  });
  await check("event-timeout", async () => {
    sites.probe_a.setMode("hang");
    const running = await turn(active, "probe_a");
    await assert.rejects(() => running.completed(500), { code: "CODEX_EVENT_TIMEOUT" });
    await active.request("turn/interrupt", { threadId: running.threadId, turnId: running.turnId });
    assert.equal((await running.completed()).params.turn.status, "interrupted");
    sites.probe_a.setMode("ok");
    return { code: "CODEX_EVENT_TIMEOUT", cancellationStatus: "interrupted" };
  });
  await check("cross-origin-redirect-credential-behavior", async () => {
    sites.probe_a.setMode("redirect", `${sites.probe_b.baseUrl}/responses`);
    const before = sites.probe_b.requests.length;
    const running = await turn(active, "probe_a");
    const terminal = await running.completed();
    const redirected = sites.probe_b.requests.slice(before);
    assert.equal(terminal.params.turn.status, "failed");
    assert.ok(redirected.every((item) => !item.credentialPresent));
    sites.probe_a.setMode("ok");
    return { terminalStatus: "failed", redirectedRequests: redirected.length, credentialForwarded: false,
      scope: "synthetic-loopback-different-port-only" };
  });
  await stop(active);
  await check("empty-home-authentication-state", async () => {
    const empty = await isolatedEnvironment({ inheritConnection: false });
    let signedOut;
    try {
      signedOut = new AppServerClient({ executable, args: ["app-server", "--listen", "stdio://"], cwd: empty.project,
        env: { ...isolated.env, CODEX_HOME: empty.home } });
      clients.add(signedOut);
      await signedOut.initialize();
      const account = await signedOut.request("account/read", { refreshToken: false });
      assert.equal(account.account, null);
      assert.equal(account.requiresOpenaiAuth, true);
      return { account: "none", requiresOpenaiAuth: true, userError: "CODEX_AUTH_REQUIRED", remoteOfficialRequest: "not-sent" };
    } finally { if (signedOut) await stop(signedOut); await empty.cleanup(); }
  });
  report.passed = true;
} catch (error) {
  report.error = error instanceof ProbeError ? error.code : "PROBE_ASSERTION_FAILED";
  console.error(report.error);
  process.exitCode = 1;
} finally {
  for (const client of clients) await stop(client);
  report.processExits = exits;
  if (exits.some((exit) => exit.code !== 0 || exit.forced)) { report.passed = false; process.exitCode = 1; }
  report.sites = Object.fromEntries(Object.entries(sites).map(([id, site]) => [id, site.requests]));
  for (const site of Object.values(sites)) await site.close();
  try { if (isolated) await isolated.cleanup(); report.tempRemoved = true; }
  catch { report.tempRemoved = false; report.passed = false; process.exitCode = 1; }
  await writeFile(join(output, "report.json"), JSON.stringify(report, null, 2) + "\n");
  console.log(`Sanitized evidence: ${join(output, "report.json")}`);
}
