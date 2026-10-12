import assert from "node:assert/strict";
import { mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { join, resolve } from "node:path";
import { AppServerClient, ProbeError, listModels, normalizeEvent, selectModel, safePrintCommand } from "./codex/client.mjs";
import { discoverCodex, isolatedEnvironment } from "./codex/environment.mjs";

const output = resolve(import.meta.dirname, "..", "artifacts", "codex-real");
const timeoutMs = Number(process.env.CODEX_PROBE_TIMEOUT_MS ?? 90000);
if (!Number.isFinite(timeoutMs) || timeoutMs < 1000 || timeoutMs > 300000) throw new ProbeError("INVALID_TIMEOUT");
await mkdir(output, { recursive: true });
const report = { date: new Intl.DateTimeFormat("sv-SE", { timeZone: "Asia/Shanghai" }).format(new Date()), mode: "real-configured-provider", checks: [], events: {}, passed: false };
let isolated;
let client;

async function check(name, action) {
  console.log(`Checking ${name}`);
  try { const evidence = await action(); report.checks.push({ name, status: "passed", evidence }); return evidence; }
  catch (error) {
    report.checks.push({ name, status: "failed", code: error instanceof ProbeError ? error.code : "PROBE_ASSERTION_FAILED", rpcCode: error.rpcCode });
    throw error;
  }
}

async function newThread(selection, { writable = false, approvalPolicy = "untrusted" } = {}) {
  const response = await client.request("thread/start", { cwd: isolated.project, ephemeral: true,
    model: selection.model, sandbox: writable ? "workspace-write" : "read-only", approvalPolicy,
    config: { model_reasoning_effort: selection.effort },
    developerInstructions: "This is an isolated integration test. Follow only the test message. Do not access files outside cwd, use the network, or invoke other agents." });
  assert.equal(resolve(response.cwd).toLowerCase(), resolve(isolated.project).toLowerCase());
  assert.equal(response.model, selection.model);
  assert.equal(response.reasoningEffort, selection.effort);
  return response;
}

async function startTurn(thread, selection, text) {
  const after = client.sequence;
  const response = await client.request("turn/start", { threadId: thread.thread.id,
    input: [{ type: "text", text, text_elements: [] }], model: selection.model, effort: selection.effort });
  const completed = () => client.waitFor((event) => event.method === "turn/completed" &&
    event.params.threadId === thread.thread.id && event.params.turn.id === response.turn.id, { after, timeoutMs });
  return { response, after, completed };
}

try {
  const executable = await discoverCodex();
  const version = spawnSync(executable, ["--version"], { encoding: "utf8", windowsHide: true, timeout: 10000 });
  report.cliVersion = version.stdout.trim().match(/^codex-cli [\w.+-]+$/)?.[0] ?? "unrecognized";
  isolated = await isolatedEnvironment({ nonOfficialOnly: true });
  client = new AppServerClient({ executable, args: ["app-server", "--listen", "stdio://"],
    cwd: isolated.project, env: isolated.env });
  client.on("event", (event) => {
    report.events[event.method] = (report.events[event.method] ?? 0) + 1;
    const normalized = normalizeEvent(event);
    if (normalized?.type === "error") report.remoteErrors = (report.remoteErrors ?? 0) + 1;
  });
  // Default deny. Approval experiments below can opt in to one reviewed harmless command.
  let approvalHandler = () => "decline";
  client.on("serverRequest", (event) => {
    if (["item/commandExecution/requestApproval", "item/fileChange/requestApproval"].includes(event.method)) {
      const decision = approvalHandler(event);
      report.approvals ??= [];
      report.approvals.push({ method: event.method, fields: Object.keys(event.params), decision });
      client.respond(event.id, { decision });
    } else client.rejectRequest(event.id);
  });
  await check("initialize", async () => {
    const response = await client.initialize();
    assert.equal(resolve(response.codexHome).toLowerCase(), resolve(isolated.home).toLowerCase());
    return { platform: response.platformOs, isolatedHome: true, ownedPid: client.child.pid };
  });
  const account = await client.request("account/read", { refreshToken: false });
  report.authentication = { type: account.account?.type ?? "none", requiresOpenaiAuth: account.requiresOpenaiAuth };
  const config = (await client.request("config/read", { includeLayers: false, cwd: isolated.project })).config;
  const models = await check("model-list", async () => {
    const items = await listModels(client);
    assert.ok(items.length);
    return items.map((item) => ({ model: item.model, defaultReasoningEffort: item.defaultReasoningEffort,
      supportedReasoningEfforts: item.supportedReasoningEfforts.map((effort) => ({ reasoningEffort: effort.reasoningEffort })) }));
  });
  const modelName = process.env.CODEX_PROBE_MODEL ?? config.model ?? models[0].model;
  const selection = selectModel(models, modelName, process.env.CODEX_PROBE_EFFORT ?? config.model_reasoning_effort ?? undefined);
  report.selection = { provider: config.model_provider ?? "openai", ...selection };
  const shellDiscovery = spawnSync("powershell.exe", ["-NoProfile", "-Command", "Get-Command pwsh.exe,powershell.exe -ErrorAction SilentlyContinue | ForEach-Object { $_.Source }"],
    { encoding: "utf8", windowsHide: true, timeout: 10000 });
  const trustedExecutables = shellDiscovery.stdout.trim().split(/\r?\n/);
  await check("no-file-message", async () => {
    const thread = await newThread(selection);
    const turn = await startTurn(thread, selection, "Do not call any tools or read/write files. Reply with exactly PHASE_0_3_OK.");
    const event = await turn.completed();
    assert.equal(event.params.turn.status, "completed");
    const events = client.events.filter((item) => item.sequence > turn.after && item.params?.threadId === thread.thread.id);
    const deltas = events.filter((item) => item.method === "item/agentMessage/delta");
    assert.ok(deltas.length > 0);
    const messages = events.filter((item) => item.method === "item/completed" && item.params.item?.type === "agentMessage");
    assert.ok(messages.some((item) => item.params.item.text.includes("PHASE_0_3_OK")));
    assert.ok(!events.some((item) => item.params?.item && ["commandExecution", "fileChange", "mcpToolCall", "collabAgentToolCall"].includes(item.params.item.type)));
    assert.deepEqual(await readdir(isolated.project), []);
    return { status: event.params.turn.status, deltaCount: deltas.length, cwdMatches: true, configurationMatches: true, projectUnchanged: true };
  });
  await check("cancel", async () => {
    const thread = await newThread(selection);
    const turn = await startTurn(thread, selection, "Do not use any tools. Write a long detailed explanation of 200 sorting algorithms.");
    await client.request("turn/interrupt", { threadId: thread.thread.id, turnId: turn.response.turn.id });
    const event = await turn.completed();
    assert.equal(event.params.turn.status, "interrupted");
    return { protocolStatus: "interrupted", applicationStatus: "cancelled" };
  });
  await check("model-effort-revalidation", async () => {
    assert.throws(() => selectModel(models, selection.model, "invalid-effort"), { code: "EFFORT_UNSUPPORTED" });
    const next = models.find((item) => item.model !== selection.model) ?? models[0];
    const nextSelection = selectModel(models, next.model);
    const thread = await newThread(nextSelection);
    const turn = await startTurn(thread, nextSelection, "Do not use tools or access any files. Reply exactly SECOND_MODEL_OK.");
    assert.equal((await turn.completed()).params.turn.status, "completed");
    const finalMessage = client.events.find((event) => event.sequence > turn.after && event.params?.threadId === thread.thread.id &&
      event.method === "item/completed" && event.params.item?.type === "agentMessage" && event.params.item.text.includes("SECOND_MODEL_OK"));
    assert.ok(finalMessage);
    assert.deepEqual(await readdir(isolated.project), []);
    return { model: thread.model, effort: thread.reasoningEffort, invalidEffortRejected: true, inference: "completed", originalSelectionUnchanged: Object.isFrozen(selection) };
  });
  for (const decision of ["decline", "accept"]) {
    await check(`approval-${decision}`, async () => {
      const before = report.approvals?.length ?? 0;
      const thread = await newThread(selection);
      approvalHandler = (event) => {
        if (event.params.threadId !== thread.thread.id || event.method !== "item/commandExecution/requestApproval") return "decline";
        // Only an exact print-only PowerShell command, no session-wide grants or policy amendments.
        const command = event.params.command ?? "";
        const safe = safePrintCommand(command, trustedExecutables);
        const cwdMatches = event.params.cwd && resolve(event.params.cwd).toLowerCase() === resolve(isolated.project).toLowerCase();
        report.approvalCommandChecks ??= [];
        report.approvalCommandChecks.push({ requestedDecision: decision, safe, cwdMatches: Boolean(cwdMatches) });
        return safe && cwdMatches ? decision : "decline";
      };
      const turn = await startTurn(thread, selection,
        `Integration test: use the shell execution tool with the command exactly Get-Location and workdir exactly ${isolated.project}. Explicitly request escalated permission so the client can review it. Do not run any other command, use other tools, or change files. If declined, do not retry; reply DENIED. If approved, reply CWD_OK.`);
      const event = await turn.completed();
      approvalHandler = () => "decline";
      const approvals = (report.approvals ?? []).slice(before);
      assert.ok(approvals.some((item) => item.decision === decision));
      assert.equal(event.params.turn.status, "completed");
      assert.deepEqual(await readdir(isolated.project), []);
      const executed = client.events.filter((item) => item.sequence > turn.after && item.method === "item/completed" && item.params.item?.type === "commandExecution");
      if (decision === "accept") {
        assert.ok(executed.some((item) => item.params.item.exitCode === 0 && item.params.item.aggregatedOutput?.toLowerCase().includes(isolated.project.toLowerCase())));
      } else assert.ok(!executed.some((item) => item.params.item.exitCode === 0));
      return { decision, terminalStatus: event.params.turn.status, executedSuccessfully: decision === "accept", projectUnchanged: true };
    });
  }
  report.passed = true;
} catch (error) {
  report.error = error instanceof ProbeError ? error.code : "PROBE_ASSERTION_FAILED";
  process.exitCode = 1;
  console.error(report.error);
} finally {
  if (client) {
    report.processExit = await client.close();
    if (report.processExit.code !== 0 || report.processExit.forced) { report.passed = false; process.exitCode = 1; }
  }
  if (isolated) {
    try { await isolated.cleanup(); report.tempRemoved = true; }
    catch { report.tempRemoved = false; report.passed = false; process.exitCode = 1; }
  }
  await writeFile(join(output, "report.json"), JSON.stringify(report, null, 2) + "\n");
  console.log(`Sanitized evidence: ${join(output, "report.json")}`);
}
