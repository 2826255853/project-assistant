// Step 1.2 acceptance: isolated synthetic state, both real desktop resource modes.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const output = resolve(root, "artifacts", "phase-1-2");
await mkdir(output, { recursive: true });
assert.ok(process.env.npm_execpath, "Run via npm run verify:state");
const report = { step: "1.2", date: new Date().toISOString(), timezone: "Asia/Shanghai", passed: false, checks: [], desktops: [] };
async function run(label, executable, args) {
  console.log(label);
  let stdout = "";
  await new Promise((done, reject) => {
    const child = spawn(executable, args, { cwd: root, windowsHide: true, shell: false, stdio: ["ignore", "pipe", "pipe"] });
    const timer = setTimeout(() => { child.kill(); reject(new Error(`${label}: timeout`)); }, 900000);
    child.stdout.on("data", chunk => { stdout += chunk; }); child.stderr.on("data", chunk => { stdout += chunk; });
    child.on("error", error => { clearTimeout(timer); reject(error); });
    child.on("close", code => { clearTimeout(timer); code === 0 ? done() : reject(new Error(`${label}: exit ${code}`)); });
  }).finally(() => writeFile(resolve(output, `${label}.log`), stdout));
  report.checks.push(label);
}
try {
  const npm = (...args) => [process.env.npm_execpath, ...args];
  await run("frontend-build", process.execPath, npm("run", "build"));
  await run("rust-tests", "cargo", ["test", "--locked", "--manifest-path", "src-tauri/Cargo.toml", "--lib"]);
  await run("ipc-tests", process.execPath, npm("run", "test:ipc"));
  await run("codex-offline-tests", process.execPath, npm("run", "test:codex"));
  for (const scenario of ["ipc", "shell-error", "phase0"]) await run(`desktop-dev-${scenario}`, process.execPath, npm("run", "verify:desktop", "--", "dev", scenario));
  await run("embedded-debug-build", process.execPath, npm("run", "tauri", "--", "build", "--debug", "--no-bundle"));
  for (const scenario of ["ipc", "shell-error", "phase0"]) await run(`desktop-built-${scenario}`, process.execPath, npm("run", "verify:desktop", "--", "built", scenario));
  for (const mode of ["dev", "built"]) {
    const desktop = JSON.parse(await readFile(resolve(root, "artifacts", `shell-${mode}-ipc`, "report.json"), "utf8"));
    assert.equal(desktop.passed, true); report.desktops.push(desktop);
    const log = await readFile(resolve(root, "artifacts", `shell-${mode}-ipc`, "process.log"), "utf8");
    for (const privateValue of ["synthetic-secret-DO-NOT-LOG", "中文草稿", "synthetic-project"]) assert.equal(log.includes(privateValue), false, "IPC logs exclude payload");
    const entries = log.split(/\r?\n/).filter(line => line.startsWith('{"apiVersion":')).map(line => JSON.parse(line));
    assert.ok(entries.some(entry => entry.errorCode === "PA_IPC_VERSION_UNSUPPORTED"));
    for (const entry of entries) {
      const allowed = entry.event === "logging_disabled"
        ? ["apiVersion", "occurredAt", "level", "event", "outcome", "errorCode"]
        : ["apiVersion", "occurredAt", "level", "command", "event", "correlationId", "outcome", "errorCode"];
      assert.deepEqual(Object.keys(entry).sort(), allowed.sort());
    }
  }
  report.checks.push("logs-allowlist-no-payload"); report.passed = true;
} catch (error) { report.error = String(error); process.exitCode = 1; console.error(report.error); }
finally { await writeFile(resolve(output, "report.json"), JSON.stringify(report, null, 2) + "\n"); }
