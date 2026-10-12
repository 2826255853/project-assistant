// Phase 0.5: Rust tests plus two independent processes using Tauri app_data_dir.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { mkdir, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const output = join(root, "artifacts", "storage");
await mkdir(output, { recursive: true });
const id = `probe-${randomUUID()}`;
const exe = join(root, "src-tauri", "target", "debug", process.platform === "win32" ? "project-assistant.exe" : "project-assistant");
const report = { date: new Date().toISOString(), timezone: "Asia/Shanghai", platform: process.platform,
  passed: false, results: [], syntheticDataOnly: true, processRestartVerified: false, cleanupPassed: false };

function execute(command, args, timeoutMs = 900000) {
  return new Promise((done, reject) => {
    const child = spawn(command, args, { cwd: root, windowsHide: true, stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "", stderr = "";
    const timer = setTimeout(() => { child.kill(); reject(new Error("STORAGE_VERIFY_TIMEOUT")); }, timeoutMs);
    child.stdout.on("data", (chunk) => { stdout += chunk; });
    child.stderr.on("data", (chunk) => { stderr += chunk; });
    child.on("error", () => { clearTimeout(timer); reject(new Error("STORAGE_VERIFY_SPAWN_FAILED")); });
    child.on("close", (code) => { clearTimeout(timer); done({ code, stdout, stderr }); });
  });
}

async function checkCommand(label, command, args, timeoutMs) {
  console.log(label);
  const result = await execute(command, args, timeoutMs);
  // Only synthetic storage queries and compilation output, never external config.
  await writeFile(join(output, `${label}.log`), result.stdout + result.stderr);
  assert.equal(result.code, 0, `${label} failed; see artifacts/storage/${label}.log`);
  report.results.push(label);
  return result;
}

let ownsFixture = false;
try {
  const npmCli = process.env.npm_execpath;
  assert.ok(npmCli, "Run via npm run verify:storage");
  await checkCommand("frontend-build", process.execPath, [npmCli, "run", "build"]);
  const tests = await checkCommand("rust-storage-tests", "cargo", ["test", "--locked", "--manifest-path", "src-tauri/Cargo.toml", "--lib", "storage::"]);
  const summary = tests.stdout.match(/test result: ok\. (\d+) passed/);
  assert.ok(summary, "Expected Rust test summary");
  report.rustTestsPassed = Number(summary[1]);
  await checkCommand("embedded-debug-build", "cargo", ["build", "--locked", "--manifest-path", "src-tauri/Cargo.toml", "--features", "tauri/custom-protocol"]);
  ownsFixture = true;
  const seed = await checkCommand("app-data-seed-process", exe, ["--phase0-storage-probe", "seed", id], 30000);
  report.seed = JSON.parse(seed.stdout.trim());
  assert.equal(report.seed.passed, true);
  const verify = await checkCommand("app-data-restart-process", exe, ["--phase0-storage-probe", "verify", id], 30000);
  report.restart = JSON.parse(verify.stdout.trim());
  assert.equal(report.restart.passed, true);
  assert.equal(report.restart.schemaVersion, 1);
  assert.equal(report.restart.journalMode, "wal");
  assert.equal(report.restart.usesTauriAppDataDir, true);
  report.processRestartVerified = true;
} catch (failure) {
  report.error = failure.message;
  console.error(failure.message);
} finally {
  if (ownsFixture) {
    try {
      const cleanup = await execute(exe, ["--phase0-storage-probe", "cleanup", id], 30000);
      report.cleanupPassed = cleanup.code === 0 && JSON.parse(cleanup.stdout.trim()).passed === true;
    } catch { report.cleanupPassed = false; }
  }
  report.passed = report.processRestartVerified && report.cleanupPassed && !report.error;
  await writeFile(join(output, "report.json"), JSON.stringify(report, null, 2) + "\n");
}
console.log(`Storage verification: ${report.passed ? "passed" : "failed"}; artifacts/storage/report.json`);
if (!report.passed) process.exitCode = 1;
