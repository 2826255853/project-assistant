// No credentials or network model requests. Own subprocesses only.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";

assert.equal(process.platform, "win32", "Current acceptance baseline is Windows x64.");
assert.ok(process.env.npm_execpath, "Run with npm run verify:ipc.");
const root = resolve(import.meta.dirname, "..");
const output = resolve(root, "artifacts", "ipc");
await mkdir(output, { recursive: true });
const report = { step: "0.6", results: [], passed: false };
const env = { ...process.env, CARGO_TERM_COLOR: "never", CARGO_TERM_PROGRESS_WHEN: "never" };
async function run(name, executable, args, timeoutMs = 900000) {
  console.log(name);
  await new Promise((done, reject) => {
    const child = spawn(executable, args, { cwd: root, env, shell: false, windowsHide: true, stdio: ["ignore", "pipe", "pipe"] });
    const logs = [];
    child.stdout.on("data", (chunk) => { logs.push(chunk); process.stdout.write(chunk); });
    child.stderr.on("data", (chunk) => { logs.push(chunk); process.stderr.write(chunk); });
    const timer = setTimeout(() => {
      child.kill(); reject(new Error(`${name}: timeout`));
    }, timeoutMs);
    child.once("error", (error) => { clearTimeout(timer); reject(error); });
    child.once("close", async (code) => {
      clearTimeout(timer);
      try {
        await writeFile(resolve(output, `${name}.log`), Buffer.concat(logs));
        assert.equal(code, 0, `${name} failed`);
        report.results.push({ name, passed: true }); done();
      } catch (error) { reject(error); }
    });
  });
}
try {
  await run("rust-tests", "cargo", ["test", "--locked", "--manifest-path", "src-tauri/Cargo.toml", "--lib"]);
  await run("ipc-tests", process.execPath, [process.env.npm_execpath, "run", "test:ipc"]);
  await run("codex-offline-tests", process.execPath, [process.env.npm_execpath, "run", "test:codex"]);
  await run("desktop-build", process.execPath, [process.env.npm_execpath, "run", "tauri", "--", "build", "--debug", "--no-bundle"]);
  await run("desktop-built", process.execPath, [process.env.npm_execpath, "run", "verify:desktop", "--", "built"]);
  const desktop = JSON.parse(await readFile(resolve(root, "artifacts/desktop-built/report.json"), "utf8"));
  assert.equal(desktop.passed, true);
  report.desktopResults = desktop.results;
  report.passed = true;
} catch (error) {
  report.error = String(error); process.exitCode = 1;
} finally {
  await writeFile(resolve(output, "report.json"), JSON.stringify(report, null, 2) + "\n");
}
