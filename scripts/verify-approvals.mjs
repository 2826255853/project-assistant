import assert from "node:assert/strict";
import {spawn, spawnSync} from "node:child_process";
import {randomUUID} from "node:crypto";
import {mkdir, mkdtemp, readFile, readdir, rm, writeFile} from "node:fs/promises";
import {tmpdir} from "node:os";
import {join, resolve} from "node:path";
import {controlledSite} from "./codex/controlled-site.mjs";
import {discoverCodex} from "./codex/environment.mjs";

assert.equal(process.platform, "win32", "approval acceptance requires Windows");
assert.ok(process.env.npm_execpath, "Run via npm run verify:approvals");
const root = resolve(import.meta.dirname, "..");
const artifacts = join(root, "artifacts", "phase-3-5");
await mkdir(artifacts, {recursive: true});
const temporary = await mkdtemp(join(tmpdir(), "pa-task-gateway-"));
const owned = join(temporary, "owned");
await mkdir(owned);
const keys = {a: `synthetic-task-${randomUUID()}`, b: `synthetic-task-${randomUUID()}`};
const report = {
  step: "3.5",
  date: new Date().toISOString(),
  timezone: "Asia/Shanghai",
  passed: false,
  syntheticDataOnly: true,
  officialSource: "reserved-not-tested",
  checks: [],
  routes: [],
  limits: [
    "Synthetic HTTPS and print-only commands; no real provider, model usage or Key.",
    "File-change grantRoot callbacks lack a bound patch and remain denied.",
    "Desktop task controls and broader lifecycle regression remain in S3-06 through S3-09."
  ]
};

async function run(label, executable, args, env = {}) {
  let output = "";
  const sanitized = () => output.replaceAll(keys.a, "[redacted]").replaceAll(keys.b, "[redacted]");
  const save = async () => writeFile(join(artifacts, `${label}.log`), sanitized());
  await new Promise((resolveRun, reject) => {
    const child = spawn(executable, args, {
      cwd: root,
      windowsHide: true,
      env: {...process.env, CARGO_TERM_COLOR: "never", ...env},
      stdio: ["ignore", "pipe", "pipe"]
    });
    const timer = setTimeout(async () => { child.kill(); await save(); reject(new Error(`${label}: timeout`)); }, 900000);
    for (const stream of [child.stdout, child.stderr]) stream.on("data", chunk => { output += chunk; });
    child.on("error", async error => { clearTimeout(timer); await save(); reject(error); });
    child.on("close", async code => {
      clearTimeout(timer);
      await save();
      if (output.includes(keys.a) || output.includes(keys.b)) {
        reject(new Error(`${label}: synthetic key reached process output`));
      } else if (code === 0) resolveRun();
      else reject(new Error(`${label}: exit ${code}\n${sanitized().slice(-4000)}`));
    });
  });
  report.checks.push(label);
  return output;
}

let sites;
let external;
try {
  const npm = (label, args) => run(label, process.execPath, [process.env.npm_execpath, ...args]);
  await run("rust-format", "cargo", ["fmt", "--check", "--manifest-path", "src-tauri/Cargo.toml"]);
  await npm("frontend-build", ["run", "build"]);
  await npm("ipc-tests", ["run", "test:ipc"]);
  await npm("codex-offline-tests", ["run", "test:codex"]);
  await run("rust-check", "cargo", ["check", "--locked", "--manifest-path", "src-tauri/Cargo.toml"]);
  const compiled = await run("rust-tests", "cargo", ["test", "--locked", "--manifest-path", "src-tauri/Cargo.toml", "--lib"]);
  const summary = compiled.match(/test result: ok\. (\d+) passed; 0 failed; (\d+) ignored/);
  assert.ok(summary);
  report.rustTestsPassed = Number(summary[1]); report.rustHelpersIgnored = Number(summary[2]);
  const worker = resolve(root, compiled.match(/Running unittests[^\r\n]*\(([^)]+\.exe)\)/)?.[1] || "missing");
  assert.ok(worker.endsWith(".exe"));
  external = spawn(process.execPath, ["-e", "setInterval(()=>{},1000)"], {windowsHide: true, stdio: "ignore"});
  const cert = join(temporary, "ca.pem");
  const key = join(temporary, "key.pem");
  const openssl = process.env.PA_ISOLATION_OPENSSL_BIN || join(process.env.ProgramFiles, "Git", "usr", "bin", "openssl.exe");
  const generated = spawnSync(openssl, [
    "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-sha256", "-days", "1",
    "-keyout", key, "-out", cert, "-subj", "/CN=Project Assistant task gateway",
    "-addext", "subjectAltName=IP:127.0.0.1", "-addext", "basicConstraints=critical,CA:TRUE"
  ], {windowsHide: true, timeout: 15000, stdio: "ignore"});
  assert.equal(generated.status, 0, "synthetic TLS certificate creation failed");
  const tls = {cert: await readFile(cert), key: await readFile(key)};
  sites = {
    a: await controlledSite("TASK_A", keys.a, {tls}),
    b: await controlledSite("TASK_B", keys.b, {tls})
  };
  const executable = await discoverCodex();
  report.cliVersion = (await run("cli-version", executable, ["--version"])).trim();
  await run("protocol-schema", executable, ["app-server", "generate-ts", "--out", join(artifacts, "types")]);
  const scenarios = ["approve", "deny", "expire", "cancel", "exit", "audit"];
  for (const scenario of scenarios) {
    sites.a.setMode("ok"); sites.b.setMode("ok");
    sites.a.setMode("approval", join(owned, "approval project 中文"));
    const selected = scenario === "b" ? sites.b : sites.a;
    const selectedKey = scenario === "b" ? keys.b : keys.a;
    const before = {a: sites.a.requests.length, b: sites.b.requests.length};
    const output = await run(`scenario-${scenario}`, worker, [
      "owned_process::tests::actual_cli_approval_acceptance", "--ignored", "--exact", "--nocapture"
    ], {
      PA_TASK_CODEX_BIN: executable,
      PA_TASK_GATEWAY_ROOT: owned,
      PA_TASK_ENDPOINT: selected.baseUrl,
      PA_TASK_SYNTHETIC_KEY: selectedKey,
      PA_TASK_CA: cert,
      PA_TASK_SCENARIO: scenario
    });
    const line = output.split(/\r?\n/).find(value => value.startsWith("APPROVAL_EVIDENCE:"));
    assert.ok(line, `${scenario}: missing gateway evidence`);
    const evidence = JSON.parse(line.slice("APPROVAL_EVIDENCE:".length));
    const requests = {
      a: sites.a.requests.slice(before.a),
      b: sites.b.requests.slice(before.b)
    };
    assert.ok(requests.a.length > 0, `${scenario}: no upstream request`);
    assert.equal(requests.b.length, 0, `${scenario}: crossed route`);
    assert.ok(requests.a.every(request => request.keyMatches));
    assert.ok(requests.a[0].toolNames.includes("exec_command"));
    assert.equal(evidence.executedSuccessfully, Number(scenario === "approve"));
    assert.equal(external.exitCode, null, `${scenario}: external fixture was stopped`);
    assert.equal(evidence.projectUnchanged, true);
    assert.equal((await readdir(owned)).length, 0, `${scenario}: owned resources remained`);
    report.routes.push({...evidence, requests});
  }
  report.passed = true;
} catch (error) {
  report.error = String(error);
  console.error(report.error);
  process.exitCode = 1;
} finally {
  if (sites) {
    sites.a.release(); sites.b.release();
    await Promise.all([sites.a.close(), sites.b.close()]);
  }
  if (external) external.kill();
  await rm(temporary, {recursive: true, force: true, maxRetries: 5, retryDelay: 200});
  report.temporaryRemoved = true;
  await writeFile(join(artifacts, "report.json"), `${JSON.stringify(report, null, 2)}\n`);
}
console.log(`Approval verification: ${report.passed ? "passed" : "failed"}; artifacts/phase-3-5/report.json`);
