import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawn, spawnSync } from "node:child_process";
import { readFile, mkdir, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { discoverCodex, nonOfficialConnection } from "./codex/environment.mjs";
import { ProbeError } from "./codex/client.mjs";

// One-off read-only provenance diagnosis, not a cc-switch runtime integration API.
// Values and hashes stay in memory; persist only types, counts and comparisons.
const output = resolve(import.meta.dirname, "..", "artifacts", "phase-0-cc-switch-retest");
await mkdir(output, { recursive: true });
const report = { date: new Intl.DateTimeFormat("sv-SE", { timeZone: "Asia/Shanghai" }).format(new Date()),
  scope: "steps-0.1-0.3-cc-switch-configured-nonofficial-retest", checks: [], passed: false,
  officialSourceTests: "paused-per-current-plan", ccSwitchAutomaticIntegration: "blocked-no-supported-external-interface" };
const fingerprints = new Map();
async function remember(path) {
  try { return createHash("sha256").update(await readFile(path)).digest("hex"); }
  catch (error) { if (error.code === "ENOENT") return "missing"; throw error; }
}
async function check(name, run) {
  console.log(`Checking ${name}`);
  try { const evidence = await run(); report.checks.push({ name, status: "passed", evidence }); }
  catch (error) {
    report.checks.push({ name, status: "failed", code: error instanceof ProbeError ? error.code : "SUPPLEMENT_CHECK_FAILED" });
    throw error;
  }
}
function value(source, key) {
  return source.match(new RegExp(`^\\s*${key}\\s*=\\s*"([^"\\\\]*)"\\s*(?:#.*)?$`, "m"))?.[1];
}
async function runProbe(script, reportPath) {
  const exitCode = await new Promise((done, reject) => {
    const child = spawn(process.execPath, [join(import.meta.dirname, script)], { windowsHide: true, stdio: "inherit", shell: false });
    child.once("error", () => reject(new ProbeError("PROBE_START_FAILED")));
    child.once("close", done);
  });
  const evidence = JSON.parse(await readFile(resolve(import.meta.dirname, "..", reportPath), "utf8"));
  assert.equal(exitCode, 0);
  if (evidence.passed !== undefined) assert.equal(evidence.passed, true);
  return { exitCode, report: reportPath, passedChecks: evidence.checks?.filter(x => x.status === "passed").length,
    integrationStatus: evidence.status, tempRemoved: evidence.tempRemoved };
}
try {
  const ccDir = join(homedir(), ".cc-switch");
  const settings = JSON.parse(await readFile(join(ccDir, "settings.json"), "utf8"));
  const configuredHome = settings.codexConfigDir || join(homedir(), ".codex");
  const sourceHome = resolve(process.env.CODEX_HOME || join(homedir(), ".codex"));
  assert.equal(sourceHome.toLowerCase(), resolve(configuredHome).toLowerCase());
  for (const path of [join(sourceHome, "config.toml"), join(sourceHome, "auth.json"), join(ccDir, "settings.json"), join(ccDir, "live-state.json")]) {
    fingerprints.set(path, await remember(path));
  }
  await check("cc-switch-selected-provider-matches-codex", async () => {
    const config = await readFile(join(sourceHome, "config.toml"), "utf8");
    const active = nonOfficialConnection(config);
    const db = new DatabaseSync(join(ccDir, "cc-switch.db"), { readOnly: true });
    try {
      const rows = db.prepare("SELECT id, is_current, settings_config FROM providers WHERE app_type = ?").all("codex");
      const selected = rows.find(row => row.id === settings.currentProviderCodex);
      assert.ok(selected && selected.is_current);
      assert.equal(rows.filter(row => row.is_current).length, 1);
      const saved = JSON.parse(selected.settings_config);
      const expected = nonOfficialConnection(saved.config);
      assert.equal(active.endpoint, expected.endpoint);
      assert.equal(active.protocol, expected.protocol);
      const actualKey = value(config, "experimental_bearer_token");
      const expectedKey = value(saved.config, "experimental_bearer_token") || saved.auth?.OPENAI_API_KEY;
      assert.ok(actualKey && expectedKey && actualKey === expectedKey);
      return { selectedProviderFound: true, uniqueCurrentProvider: true, sharedUserHome: true,
        endpointMatches: true, protocolMatches: true,
        modelMatches: value(config, "model") === value(saved.config, "model"),
        effortMatches: value(config, "model_reasoning_effort") === value(saved.config, "model_reasoning_effort"),
        modelOverridePolicy: "revalidate-live-selection-via-model-list-and-real-inference", credentialMatches: true,
        provider: active.provider, providerKind: "nonofficial-https", authenticationKind: "bearer", totalCodexProfiles: rows.length,
        credentialSavedInReport: false, databaseAccess: "one-off-readonly-diagnosis" };
    } finally { db.close(); }
  });
  await check("cli-installation-and-authentication-baseline", async () => {
    const executable = await discoverCodex();
    const version = spawnSync(executable, ["--version"], { encoding: "utf8", windowsHide: true, timeout: 10000 });
    const login = spawnSync(executable, ["login", "status"], { encoding: "utf8", windowsHide: true, timeout: 10000 });
    assert.equal(version.status, 0);
    return { cliVersion: version.stdout.trim().match(/^codex-cli [\w.+-]+$/)?.[0] ?? "unrecognized",
      loginStatusExitCode: login.status, loginStatusScope: "local-status-only-no-official-request",
      customProviderAuthentication: "verified-by-real-request-below" };
  });
  await check("cc-switch-installation-survey", () => runProbe("inspect-cc-switch.mjs", "artifacts/cc-switch/report.json"));
  await check("real-configured-provider-lifecycle", () => runProbe("verify-codex.mjs", "artifacts/codex-real/report.json"));
  await check("isolated-provider-switch-key-and-recovery", () => runProbe("verify-codex-providers.mjs", "artifacts/codex-controlled/report.json"));
  report.passed = true;
} catch (error) {
  report.error = error instanceof ProbeError ? error.code : "SUPPLEMENT_CHECK_FAILED";
  process.exitCode = 1;
  console.error(report.error);
} finally {
  try {
    const unchanged = (await Promise.all([...fingerprints].map(async ([path, fingerprint]) => await remember(path) === fingerprint))).every(Boolean);
    report.sharedConfigurationUnchanged = unchanged;
    if (!unchanged) { report.passed = false; process.exitCode = 1; }
  } catch { report.sharedConfigurationUnchanged = false; report.passed = false; process.exitCode = 1; }
  await writeFile(join(output, "report.json"), JSON.stringify(report, null, 2) + "\n");
  console.log(`Sanitized evidence: ${join(output, "report.json")}`);
}
