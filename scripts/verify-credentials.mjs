// Step 1.6: OS-backed synthetic credentials, failure compensation and WebView ACL.
import assert from "node:assert/strict";
import {spawn} from "node:child_process";
import {mkdir, mkdtemp, readFile, writeFile, rm, readdir} from "node:fs/promises";
import {tmpdir} from "node:os";
import {resolve, join} from "node:path";

assert.equal(process.platform, "win32", "Credential acceptance requires Windows");
assert.ok(process.env.npm_execpath, "Run via npm run verify:credentials");
const root = resolve(import.meta.dirname, "..");
const output = resolve(root, "artifacts", "phase-1-6");
await mkdir(output, {recursive:true});
const logDirectory = await mkdtemp(resolve(tmpdir(), "pa-credentials-logs-"));
const report = {step:"1.6", date:new Date().toISOString(), timezone:"Asia/Shanghai", platform:process.platform,
  syntheticDataOnly:true, passed:false, checks:[], desktops:[], environment:{node:process.version},
  limits:["Windows service/access failures are injected at the private OS adapter boundary; no service is disabled and no user ACL is changed.",
    "No real Key, remote model, official source, production process isolation or task execution is tested.",
    "AT-AI-007 routing, temporary authentication files and future exports require checks when implemented."]};
async function run(label, executable, args) {
  console.log(label);
  let captured = "";
  await new Promise((done, reject) => {
    const child = spawn(executable, args, {cwd:root, windowsHide:true, env:{...process.env, PA_LOG_VERIFY_DIR:logDirectory, CARGO_TERM_COLOR:"never"}, stdio:["ignore","pipe","pipe"]});
    const timer = setTimeout(() => {child.kill(); reject(new Error(`${label}: timeout`));}, 900000);
    for (const stream of [child.stdout, child.stderr]) stream.on("data", chunk => {captured += chunk;});
    child.on("error", error => {clearTimeout(timer); reject(error);});
    child.on("close", code => {clearTimeout(timer); code === 0 ? done() : reject(new Error(`${label}: exit ${code}`));});
  }).finally(() => writeFile(join(output, `${label}.log`), captured));
  report.checks.push(label);
  return captured;
}
const npm = (label, args) => run(label, process.execPath, [process.env.npm_execpath, ...args]);
try {
  report.environment.rust = (await run("rust-version", "rustc", ["--version"])).trim();
  report.environment.npm = (await npm("npm-version", ["--version"])).trim();
  await npm("frontend-build", ["run","build"]);
  await run("rust-format", "cargo", ["fmt","--check","--manifest-path","src-tauri/Cargo.toml"]);
  const tests = await run("rust-tests", "cargo", ["test","--locked","--manifest-path","src-tauri/Cargo.toml","--lib"]);
  const summary = tests.match(/test result: ok\. (\d+) passed; 0 failed; (\d+) ignored/);
  assert.ok(summary); report.rustTestsPassed = Number(summary[1]); report.rustHelpersIgnored = Number(summary[2]);
  assert.ok(tests.includes("independent_process_write_read_replace_remove_and_cleanup ... ok"));
  report.independentProcessesVerified = true;
  await npm("ipc-tests", ["run","test:ipc"]);
  await npm("codex-offline-tests", ["run","test:codex"]);
  await run("release-check", "cargo", ["check","--locked","--release","--manifest-path","src-tauri/Cargo.toml"]);
  await npm("embedded-debug-build", ["run","tauri","--","build","--debug","--no-bundle","--config",JSON.stringify({build:{beforeBuildCommand:""}})]);
  for (const mode of ["built","dev"]) {
    await npm(`${mode}-credentials-desktop`, ["run","verify:desktop","--",mode,"credentials"]);
    const desktop = JSON.parse(await readFile(join(root,"artifacts",`shell-${mode}-credentials`,"report.json"),"utf8"));
    assert.equal(desktop.passed,true); assert.equal(desktop.credentialCleanupPassed,true); report.desktops.push(desktop);
  }
  for (const entry of await readdir(logDirectory)) {
    const bytes = await readFile(join(logDirectory,entry));
    assert.equal(bytes.includes(Buffer.from("synthetic-credential-desktop-DO-NOT-EXPORT")),false);
  }
  report.passed = true;
} catch (error) {
  report.error = String(error); console.error(report.error); process.exitCode = 1;
} finally {
  // mkdtemp returned an exact task-owned directory; no user paths are removed.
  await rm(logDirectory, {recursive:true,force:true,maxRetries:5,retryDelay:200});
  await writeFile(join(output,"report.json"), JSON.stringify(report,null,2)+"\n");
}
console.log(`Credential verification: ${report.passed ? "passed" : "failed"}; artifacts/phase-1-6/report.json`);
