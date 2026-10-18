// One coordinated pass for step 1.5; historical acceptance suites are not rerun.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, writeFile, readFile } from "node:fs/promises";
import { resolve } from "node:path";

assert.ok(process.env.npm_execpath, "Run via npm run verify:logging");
const root = resolve(import.meta.dirname, "..");
const output = resolve(root, "artifacts", "phase-1-5");
await mkdir(output, {recursive: true});
const report = {step: "1.5", date: new Date().toISOString(), timezone: "Asia/Shanghai", passed: false, checks: [], desktops: [], limits: ["Disk-full and access-denied failures use controlled I/O injection; no real disk is filled and no user ACL is changed.", "Approval decision and consumption atomicity require separate phase 3 acceptance.", "Physical tray, monitor and DPI acceptance remains in step 1.10."]};
async function run(label, args, executable = process.execPath) {
  console.log(label);
  let captured = "";
  await new Promise((done, reject) => {
    const child = spawn(executable, executable === process.execPath ? [process.env.npm_execpath, ...args] : args,
      {cwd: root, env: {...process.env, CARGO_TERM_COLOR: "never"}, windowsHide: true, stdio: ["ignore", "pipe", "pipe"]});
    const timer = setTimeout(() => { child.kill(); reject(new Error(`${label}: timeout`)); }, 900000);
    child.stdout.on("data", chunk => { captured += chunk; });
    child.stderr.on("data", chunk => { captured += chunk; });
    child.on("error", error => { clearTimeout(timer); reject(error); });
    child.on("close", code => { clearTimeout(timer); code === 0 ? done() : reject(new Error(`${label}: exit ${code}`)); });
  }).finally(() => writeFile(resolve(output, `${label}.log`), captured));
  report.checks.push(label);
}
try {
  await run("frontend-build", ["run", "build"]);
  await run("rust-format", ["fmt", "--check", "--manifest-path", "src-tauri/Cargo.toml"], "cargo");
  await run("rust-tests", ["test", "--locked", "--manifest-path", "src-tauri/Cargo.toml", "--lib"], "cargo");
  await run("ipc-tests", ["run", "test:ipc"]);
  await run("embedded-debug-build", ["run", "tauri", "--", "build", "--debug", "--no-bundle", "--config", JSON.stringify({build: {beforeBuildCommand: ""}})]);
  await run("built-logging-desktop", ["run", "verify:desktop", "--", "built", "logging"]);
  const desktop = JSON.parse(await readFile(resolve(root, "artifacts", "shell-built-logging", "report.json"), "utf8"));
  assert.equal(desktop.passed, true);
  report.desktops.push(desktop);
  report.passed = true;
} catch (error) {
  report.error = String(error);
  process.exitCode = 1;
  console.error(report.error);
} finally {
  await writeFile(resolve(output, "report.json"), JSON.stringify(report, null, 2) + "\n");
}
