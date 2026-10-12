// Step 1.1: both actual desktop resource modes, failure recovery and clean restart.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const output = resolve(root, "artifacts", "phase-1-1");
await mkdir(output, { recursive: true });
assert.ok(process.env.npm_execpath, "Run via npm run verify:shell");
const report = { date: new Date().toISOString(), timezone: "Asia/Shanghai", platform: process.platform, passed: false, checks: [], desktops: [] };

async function run(label, command, args) {
  console.log(label);
  let stdout = "";
  await new Promise((done, reject) => {
    const child = spawn(command, args, { cwd: root, windowsHide: true, stdio: ["ignore", "pipe", "pipe"] });
    const timer = setTimeout(() => { child.kill(); reject(new Error(`${label}: timeout`)); }, 900000);
    child.stdout.on("data", (chunk) => { stdout += chunk; });
    child.stderr.on("data", (chunk) => { stdout += chunk; });
    child.on("error", (error) => { clearTimeout(timer); reject(error); });
    child.on("close", (code) => { clearTimeout(timer); code === 0 ? done() : reject(new Error(`${label}: exit ${code}`)); });
  }).finally(() => writeFile(resolve(output, `${label}.log`), stdout));
  report.checks.push(label);
}

try {
  const npm = (...args) => [process.env.npm_execpath, ...args];
  await run("frontend-build", process.execPath, npm("run", "build"));
  await run("rust-check", "cargo", ["check", "--locked", "--manifest-path", "src-tauri/Cargo.toml"]);
  await run("rust-tests", "cargo", ["test", "--locked", "--manifest-path", "src-tauri/Cargo.toml", "--lib"]);
  await run("ipc-tests", process.execPath, npm("run", "test:ipc"));
  await run("codex-offline-tests", process.execPath, npm("run", "test:codex"));
  // Dev first; the subsequent custom-protocol build must embed dist for built mode.
  for (const scenario of ["shell-error", "shell", "phase0"]) {
    await run(`desktop-dev-${scenario}`, process.execPath, npm("run", "verify:desktop", "--", "dev", scenario));
  }
  await run("embedded-debug-build", process.execPath, npm("run", "tauri", "--", "build", "--debug", "--no-bundle"));
  for (const scenario of ["shell-error", "shell", "phase0"]) {
    await run(`desktop-built-${scenario}`, process.execPath, npm("run", "verify:desktop", "--", "built", scenario));
  }
  await run("phase0-system-open", process.execPath, npm("run", "verify:system-open"));
  for (const mode of ["dev", "built"]) {
    for (const scenario of ["shell-error", "shell", "phase0"]) {
      const directory = scenario === "phase0" ? `desktop-${mode}` : `shell-${mode}-${scenario}`;
      const desktop = JSON.parse(await readFile(resolve(root, "artifacts", directory, "report.json"), "utf8"));
      assert.equal(desktop.passed, true);
      report.desktops.push(desktop);
    }
  }
  report.passed = true;
} catch (error) {
  report.error = String(error);
  process.exitCode = 1;
  console.error(report.error);
} finally {
  await writeFile(resolve(output, "report.json"), JSON.stringify(report, null, 2) + "\n");
}
