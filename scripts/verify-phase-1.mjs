import assert from "node:assert/strict";
import {spawn} from "node:child_process";
import {mkdir, readFile, writeFile} from "node:fs/promises";
import {resolve} from "node:path";

assert.equal(process.platform, "win32", "Phase 1 acceptance requires Windows");
assert.ok(process.env.npm_execpath, "Run via npm run verify:phase-1");

const root = resolve(import.meta.dirname, "..");
const output = resolve(root, "artifacts", "phase-1");
await mkdir(output, {recursive: true});

const suites = [
  {name: "step-1.1-shell", script: "verify:shell", report: "phase-1-1/report.json"},
  {name: "step-1.2-ipc-state", script: "verify:state", report: "phase-1-2/report.json"},
  {name: "step-1.3-storage", script: "verify:storage", report: "storage/report.json"},
  {name: "step-1.4-settings", script: "verify:settings", report: "phase-1-4/report.json"},
  {name: "step-1.5-logging", script: "verify:logging", report: "phase-1-5/report.json"},
  {name: "step-1.6-credentials", script: "verify:credentials", report: "phase-1-6/report.json"},
  {name: "step-1.7-isolation", script: "verify:isolation", report: "phase-1-7/report.json"},
  {name: "step-1.8-1.9-providers", script: "verify:providers", report: "phase-1-8/report.json"},
];

const report = {
  step: "1.10",
  task: "S1-09",
  date: new Date().toISOString(),
  timezone: "Asia/Shanghai",
  platform: process.platform,
  environment: {node: process.version},
  passed: false,
  suites: [],
  limits: [
    "Real tray mouse interaction, two-monitor movement, physical monitor disconnect and live DPI changes require attended hardware acceptance.",
    "AI configuration page controls require manual WebView2 interaction; provider desktop suites verify IPC and state, not each page control.",
    "Official OpenAI source/authentication is reserved and excluded from acceptance. No paid model or real API key is used.",
    "Stage 3 gateway behavior, real task submission/cancellation, approval atomicity and concurrent-task isolation remain future gates.",
  ],
};

async function runSuite(suite) {
  console.log(`Running ${suite.name} (${suite.script})`);
  let captured = "";
  const startedAt = new Date().toISOString();
  const child = spawn(process.execPath, [process.env.npm_execpath, "run", suite.script], {
    cwd: root,
    env: {...process.env, CARGO_TERM_COLOR: "never"},
    windowsHide: true,
    stdio: ["ignore", "pipe", "pipe"],
  });
  const timer = setTimeout(() => child.kill(), 3_600_000);
  for (const stream of [child.stdout, child.stderr]) stream.on("data", chunk => { captured += chunk; });
  const exitCode = await new Promise((resolveExit, reject) => {
    child.on("error", reject);
    child.on("close", code => resolveExit(code));
  }).finally(() => clearTimeout(timer));
  await writeFile(resolve(output, `${suite.name}.log`), captured);

  const entry = {name: suite.name, script: suite.script, startedAt, exitCode, passed: exitCode === 0};
  try {
    const result = JSON.parse(await readFile(resolve(root, "artifacts", suite.report), "utf8"));
    entry.sourceReport = `artifacts/${suite.report}`;
    entry.reportPassed = result.passed === true;
    entry.reportStep = result.step;
    entry.validation = result.validation;
  } catch (error) {
    entry.reportError = String(error);
  }
  report.suites.push(entry);
  await writeFile(resolve(output, "report.json"), JSON.stringify(report, null, 2) + "\n");
  assert.equal(exitCode, 0, `${suite.name} failed; see artifacts/phase-1/${suite.name}.log`);
  assert.equal(entry.reportPassed, true, `${suite.name} report did not confirm passed=true`);
}

try {
  report.environment.npm = (await new Promise((done, reject) => {
    const child = spawn(process.execPath, [process.env.npm_execpath, "--version"], {cwd: root, windowsHide: true, stdio: ["ignore", "pipe", "pipe"]});
    let outputText = "";
    child.stdout.on("data", chunk => { outputText += chunk; });
    child.on("error", reject);
    child.on("close", code => code === 0 ? done(outputText.trim()) : reject(new Error(`npm --version exited ${code}`)));
  }));
  report.environment.rust = (await new Promise((done, reject) => {
    const child = spawn("rustc", ["--version"], {cwd: root, windowsHide: true, stdio: ["ignore", "pipe", "pipe"]});
    let outputText = "";
    child.stdout.on("data", chunk => { outputText += chunk; });
    child.on("error", reject);
    child.on("close", code => code === 0 ? done(outputText.trim()) : reject(new Error(`rustc --version exited ${code}`)));
  }));
  for (const suite of suites) await runSuite(suite);
  report.passed = true;
} catch (error) {
  report.error = String(error);
  process.exitCode = 1;
  console.error(report.error);
} finally {
  report.finishedAt = new Date().toISOString();
  await writeFile(resolve(output, "report.json"), JSON.stringify(report, null, 2) + "\n");
}

console.log(`Phase 1 regression: ${report.passed ? "passed" : "failed"}; artifacts/phase-1/report.json`);
