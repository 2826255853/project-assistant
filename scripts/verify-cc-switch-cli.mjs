// Phase-0 alternative-interface experiment. Never give the community CLI the user's database.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { access, mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { isAbsolute, join, resolve } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { isolatedEnvironment } from "./codex/environment.mjs";
import { ProbeError } from "./codex/client.mjs";

const output = resolve(import.meta.dirname, "..", "artifacts", "cc-switch-cli");
const executable = process.env.CC_SWITCH_CLI_BIN ?? resolve(import.meta.dirname, "..", "artifacts", "cc-switch-followup", "cli-bin", "cc-switch.exe");
const report = { date: new Intl.DateTimeFormat("sv-SE", { timeZone: "Asia/Shanghai" }).format(new Date()),
  scope: "portable-community-cli-with-synthetic-providers-only", supportedVersion: "5.11.0",
  desktopVersion: "4.0.6", desktopIntegration: "not-verified", sharedDesktopProxy: "not-tested",
  officialSourceTests: "reserved-excluded-from-acceptance", checks: [], passed: false,
  integrationStatus: "blocked", credentialsSavedInReport: false, liveDatabaseGivenToCli: false };
await mkdir(output, { recursive: true });
let isolated;
const fingerprints = new Map();
async function fingerprint(path) {
  try { return createHash("sha256").update(await readFile(path)).digest("hex"); }
  catch (error) { if (error.code === "ENOENT") return "missing"; throw error; }
}
async function check(name, run) {
  console.log(`Checking ${name}`);
  try { report.checks.push({ name, status: "passed", evidence: await run() }); }
  catch (error) { report.checks.push({ name, status: "failed", code: error instanceof ProbeError ? error.code : "CLI_PROBE_ASSERTION_FAILED" }); throw error; }
}
async function command(args, { expected = 0, env = isolated.env, timeoutMs = 15000 } = {}) {
  const result = await new Promise((done, reject) => {
    const child = spawn(executable, args, { env, cwd: isolated.project, shell: false, windowsHide: true,
      stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "", stderr = "", bytes = 0, timedOut = false;
    const timer = setTimeout(() => { timedOut = true; child.kill(); }, timeoutMs);
    const capture = (which) => (data) => {
      bytes += data.length;
      if (bytes > 2 * 1024 * 1024) { child.kill(); return; }
      if (which === "out") stdout += data.toString("utf8"); else stderr += data.toString("utf8");
    };
    child.stdout.on("data", capture("out")); child.stderr.on("data", capture("err"));
    child.once("error", () => { clearTimeout(timer); reject(new ProbeError("CC_SWITCH_CLI_START_FAILED")); });
    child.once("close", (code) => {
      clearTimeout(timer);
      if ((timedOut && expected !== "unavailable") || bytes > 2 * 1024 * 1024) reject(new ProbeError("CC_SWITCH_CLI_LIMIT_EXCEEDED"));
      else done({ code, stdout, stderr, timedOut });
    });
  });
  // Raw output stays in memory; even current/show/export may contain a credential.
  if (expected === "failure" || expected === "unavailable") assert.notEqual(result.code, 0);
  else if (result.code !== expected) {
    report.commandFailure = { operation: args.slice(0, 4).join(" "), exitCode: result.code,
      terminalRequired: /terminal|tty|prompt/i.test(result.stderr),
      configError: /config|toml|parse/i.test(result.stderr), ioError: /os error|file|directory/i.test(result.stderr) };
    throw new ProbeError("CC_SWITCH_CLI_COMMAND_FAILED");
  }
  return result;
}
const codex = (...args) => command(["--app", "codex", ...args]);
function configEndpoint(config) { return config.match(/^\s*base_url\s*=\s*"([^"\n]+)"/m)?.[1]; }

try {
  if (process.platform !== "win32") throw new ProbeError("CC_SWITCH_CLI_PROBE_WINDOWS_ONLY");
  if (!isAbsolute(executable)) throw new ProbeError("CC_SWITCH_CLI_PATH_NOT_ABSOLUTE");
  try { await access(executable); } catch { throw new ProbeError("CC_SWITCH_CLI_NOT_PREPARED"); }
  const ccDir = join(homedir(), ".cc-switch");
  const liveHome = process.env.CODEX_HOME || join(homedir(), ".codex");
  for (const path of [join(liveHome, "config.toml"), join(liveHome, "auth.json"),
    join(ccDir, "settings.json"), join(ccDir, "live-state.json")]) fingerprints.set(path, await fingerprint(path));
  isolated = await isolatedEnvironment({ inheritConnection: false, config: "" });
  const cliRoot = join(isolated.root, "cc-switch");
  await mkdir(cliRoot);
  Object.assign(isolated.env, { CC_SWITCH_CONFIG_DIR: cliRoot, CC_SWITCH_TEST_HOME: isolated.root,
    CLAUDE_CONFIG_DIR: join(isolated.root, ".claude"), HERMES_HOME: join(isolated.root, ".hermes"),
    XDG_CONFIG_HOME: join(isolated.root, ".config") });
  for (const key of Object.keys(isolated.env)) {
    if (/^(OPENAI_API_KEY|CODEX_API_KEY|ANTHROPIC_API_KEY|GEMINI_API_KEY|GOOGLE_API_KEY|CC_SWITCH_DAEMON_SOCKET)$/i.test(key)) delete isolated.env[key];
  }
  await writeFile(join(cliRoot, "settings.json"), JSON.stringify({ codexConfigDir: isolated.home, language: "en" }));
  await check("portable-version-and-isolated-path", async () => {
    const version = await command(["--version"]);
    assert.match(version.stdout, /\b5\.11\.0\b/);
    const paths = await codex("config", "path");
    assert.ok(paths.stdout.includes(cliRoot));
    return { version: "5.11.0", executableSha256: await fingerprint(executable),
      explicitConfigRoot: true, explicitTestHome: true, globalInstall: false };
  });
  await check("schema-version-compatibility-prerequisite", async () => {
    // One-off PRAGMA only, not a runtime dependency on desktop internal tables.
    const liveDb = new DatabaseSync(join(ccDir, "cc-switch.db"), { readOnly: true });
    const tempDb = new DatabaseSync(join(cliRoot, "cc-switch.db"), { readOnly: true });
    try {
      const desktop = liveDb.prepare("PRAGMA user_version").get().user_version;
      const community = tempDb.prepare("PRAGMA user_version").get().user_version;
      assert.equal(community, 20); assert.equal(desktop, community);
      return { desktop, community, matchingVersionIsNotFullCompatibility: true, liveAccess: "readonly-pragma-only" };
    } finally { liveDb.close(); tempDb.close(); }
  });
  const profiles = ["a", "b"].map(id => ({ id: `phase0_${id}`, name: `PHASE0_${id.toUpperCase()}`,
    endpoint: `https://phase0-${id}.invalid/v1`, key: `synthetic-${randomUUID()}` }));
  for (const profile of profiles) {
    // .invalid domains are reserved and never contacted. Real credentials are never imported.
    const file = join(isolated.root, `${profile.id}.json`);
    const config = `model_provider = "custom"\nmodel = "gpt-5.1-codex-mini"\nmodel_reasoning_effort = "medium"\n\n[model_providers.custom]\nname = "custom"\nbase_url = "${profile.endpoint}"\nwire_api = "responses"\nrequires_openai_auth = false\nexperimental_bearer_token = "${profile.key}"\n`;
    await writeFile(file, JSON.stringify({ config, auth: { OPENAI_API_KEY: profile.key } }), { mode: 0o600 });
    await check(`noninteractive-add-${profile.id}`, async () => {
      await codex("provider", "add", "--id", profile.id, "--name", profile.name, "--config-file", file);
      const list = await codex("provider", "list");
      assert.ok(list.stdout.includes(profile.id));
      return { added: true, listed: true, credentialTransport: "restricted-temp-config-file" };
    });
  }
  async function assertCurrent(profile) {
    const current = await codex("provider", "current");
    if (!current.stdout.includes(profile.id)) throw new ProbeError("CLI_CURRENT_ID_MISMATCH");
    const config = await readFile(join(isolated.home, "config.toml"), "utf8");
    if (configEndpoint(config) !== profile.endpoint) throw new ProbeError("CLI_ENDPOINT_MISMATCH");
    const bearer = config.match(/^\s*experimental_bearer_token\s*=\s*"([^"\n]+)"/m)?.[1];
    if (bearer && bearer !== profile.key) throw new ProbeError("CLI_BEARER_MISMATCH");
    let auth = {};
    try { auth = JSON.parse(await readFile(join(isolated.home, "auth.json"), "utf8")); }
    catch (error) { if (error.code !== "ENOENT") throw error; }
    if (auth.OPENAI_API_KEY && auth.OPENAI_API_KEY !== profile.key) throw new ProbeError("CLI_AUTH_KEY_MISMATCH");
    if (bearer !== profile.key && auth.OPENAI_API_KEY !== profile.key) throw new ProbeError("CLI_CREDENTIAL_MISSING");
  }
  await check("switch-a-b-a-and-key-binding", async () => {
    for (const index of [0, 1, 0]) { await codex("provider", "switch", profiles[index].id); await assertCurrent(profiles[index]); }
    return { sequence: ["A", "B", "A"], endpointMatches: true, keyMatches: true, remoteAuthentication: "not-tested" };
  });
  await check("invalid-switch-preserves-active-configuration", async () => {
    const before = await fingerprint(join(isolated.home, "config.toml"));
    await command(["--app", "codex", "provider", "switch", "phase0_missing"], { expected: "failure" });
    assert.equal(await fingerprint(join(isolated.home, "config.toml")), before);
    await assertCurrent(profiles[0]);
    return { nonzeroExit: true, configUnchanged: true, activeProviderUnchanged: true };
  });
  await check("backup-and-export-available", async () => {
    await codex("config", "backup", "--name", "phase0-probe");
    const backups = await readdir(join(cliRoot, "backups"));
    assert.ok(backups.some(name => name.endsWith(".sql")));
    const exported = join(isolated.root, "export.sql");
    await codex("config", "export", exported);
    assert.ok((await readFile(exported)).length > 0);
    return { sqlBackupCreated: true, sqlExportCreated: true, storedOnlyInRestrictedTemp: true };
  });
  await check("automatic-edit-import-restore-boundary", async () => {
    const before = await fingerprint(join(isolated.home, "config.toml"));
    const boundaries = [];
    for (const args of [["provider", "edit", profiles[0].id],
      ["config", "import", join(isolated.root, "export.sql")],
      ["config", "restore", "--file", join(isolated.root, "export.sql")]]) {
      const result = await command(["--app", "codex", ...args], { expected: "unavailable", timeoutMs: 3000 });
      if (!result.timedOut) assert.match(result.stderr, /terminal|tty|input|prompt/i);
      boundaries.push({ operation: args.slice(0, 2).join(" "),
        outcome: result.timedOut ? "timeout-owned-child-terminated" : "terminal-prompt-rejected", exitCode: result.code });
    }
    assert.equal(await fingerprint(join(isolated.home, "config.toml")), before);
    await assertCurrent(profiles[0]);
    return { edit: "requires-interactive-terminal", import: "requires-interactive-confirmation",
      restore: "requires-interactive-confirmation", automaticRoundTrip: "blocked", configUnchanged: true, boundaries };
  });
  await check("windows-launch-and-proxy-supervisor-boundary", async () => {
    const result = await command(["start", "codex", profiles[0].id], { expected: "failure" });
    assert.match(result.stderr, /unrecognized subcommand.*start/is);
    const help = await command(["--help"]);
    assert.doesNotMatch(help.stdout, /^\s+daemon\s/m);
    return { perProviderStart: "unavailable-on-windows", daemonSupervisor: "unavailable-on-windows",
      foregroundProxy: "not-tested-not-equivalent-to-desktop-routing" };
  });
  report.passed = true;
  report.remainingBlockers = ["no-verified-desktop-external-contract", "noninteractive-edit-and-restore-unavailable",
    "shared-desktop-proxy-task-impact-unverified", "desktop-cli-roundtrip-compatibility-unverified"];
} catch (error) {
  report.error = error instanceof ProbeError ? error.code : "CLI_PROBE_ASSERTION_FAILED";
  process.exitCode = 1;
  console.error(report.error);
} finally {
  try {
    report.sharedConfigurationUnchanged = (await Promise.all([...fingerprints].map(async ([path, value]) => await fingerprint(path) === value))).every(Boolean);
    if (!report.sharedConfigurationUnchanged) { report.passed = false; process.exitCode = 1; }
  } catch { report.passed = false; process.exitCode = 1; report.configurationCheckFailed = true; }
  try { if (isolated) { await isolated.cleanup(); report.tempRemoved = true; } }
  catch { report.passed = false; process.exitCode = 1; report.cleanupFailed = true; }
  await writeFile(join(output, "report.json"), JSON.stringify(report, null, 2) + "\n");
  console.log(`Community CLI probe passed=${report.passed}; integration=${report.integrationStatus}; evidence: ${join(output, "report.json")}`);
}
