// Phase 0.4 Git CLI experiment. All writes are confined to disposable fixtures.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const output = join(root, "artifacts", "git-system");
const gitBin = process.env.GIT_BIN || "git";
const report = { platform: process.platform, gitVersion: null, checks: [], passed: false };
await mkdir(output, { recursive: true });
let fixture;

function run(program, args, cwd, timeoutMs = 5000) {
  return new Promise((resolveRun) => {
    const child = spawn(program, args, {
      cwd, windowsHide: true, shell: false, stdio: ["ignore", "pipe", "pipe"],
    });
    const stdout = [];
    const stderr = [];
    let spawnError;
    let timedOut = false;
    const timer = setTimeout(() => { timedOut = true; child.kill(); }, timeoutMs);
    child.stdout?.on("data", (chunk) => stdout.push(chunk));
    child.stderr?.on("data", (chunk) => stderr.push(chunk));
    child.on("error", (error) => { spawnError = error; });
    child.on("close", (exitCode, signal) => {
      clearTimeout(timer);
      const code = timedOut ? "COMMAND_TIMEOUT"
        : spawnError?.code === "ENOENT" ? "GIT_NOT_FOUND"
          : spawnError ? "SPAWN_FAILED"
            : exitCode === 0 ? null : "NONZERO_EXIT";
      resolveRun({ code, exitCode, signal, stdout: Buffer.concat(stdout).toString("utf8"),
        stderr: Buffer.concat(stderr).toString("utf8") });
    });
  });
}

function git(cwd, args, options = {}) {
  return run(options.bin || gitBin, ["--no-pager", "-c", "color.ui=false", ...args], cwd,
    options.timeoutMs ?? 5000);
}

async function mustGit(cwd, args) {
  const result = await git(cwd, args);
  assert.equal(result.code, null, `${args.join(" ")} failed: ${result.code} (${result.exitCode}) ${result.stderr}`);
  return result.stdout;
}

function record(id, detail) { report.checks.push({ id, detail }); }

function splitFields(line, count) {
  const fields = [];
  let rest = line;
  for (let i = 0; i < count; i++) {
    const at = rest.indexOf(" ");
    assert.ok(at > 0, `Malformed porcelain record: ${line}`);
    fields.push(rest.slice(0, at));
    rest = rest.slice(at + 1);
  }
  return { fields, path: rest };
}

function parseStatus(outputText) {
  const parts = outputText.split("\0");
  const entries = [];
  let branch;
  for (let i = 0; i < parts.length; i++) {
    const part = parts[i];
    if (!part) continue;
    if (part.startsWith("# branch.head ")) { branch = part.slice(14); continue; }
    if (part.startsWith("# ")) continue;
    const kind = part[0];
    if (kind === "?" || kind === "!") {
      entries.push({ kind, path: part.slice(2) });
    } else if (kind === "1" || kind === "2") {
      const { fields, path } = splitFields(part, kind === "1" ? 8 : 9);
      const entry = { kind, xy: fields[1], path };
      if (kind === "2") entry.origPath = parts[++i];
      entries.push(entry);
    } else {
      throw new Error(`Unsupported porcelain record kind: ${kind}`);
    }
  }
  return { branch, entries };
}

function parseRawDiff(outputText) {
  const parts = outputText.split("\0");
  const entries = [];
  for (let i = 0; i < parts.length; i++) {
    const header = parts[i];
    if (!header) continue;
    const match = /^:([0-7]{6}) ([0-7]{6}) ([0-9a-f]+) ([0-9a-f]+) ([A-Z?][0-9]*)$/.exec(header);
    assert.ok(match, `Malformed raw diff header: ${header}`);
    const status = match[5];
    const firstPath = parts[++i];
    assert.ok(firstPath, "Missing raw diff path");
    const entry = { oldMode: match[1], newMode: match[2], oldOid: match[3],
      newOid: match[4], status, path: firstPath };
    if (/^[RC]/.test(status)) { entry.oldPath = firstPath; entry.path = parts[++i]; }
    entries.push(entry);
  }
  return entries;
}

try {
  const version = await run(gitBin, ["--version"], root);
  assert.equal(version.code, null, `Git availability: ${version.code}`);
  report.gitVersion = version.stdout.trim();
  record("available", report.gitVersion);

  const base = await mkdtemp(join(tmpdir(), "phase-0-git-"));
  fixture = base;
  const repo = join(base, "项目 中文 & mkdir INJECTED");
  const outside = join(base, "不是仓库 & echo BAD");
  await mkdir(repo);
  await mkdir(outside);
  await mustGit(repo, ["init", "-b", "phase-0-main"]);
  const edited = "源 文件 α & [x].txt";
  const renamedFrom = "rename 源.txt";
  const renamedTo = "rename 目标 β & (x).txt";
  const untracked = "未跟踪 & ! ^.txt";
  await writeFile(join(repo, edited), "before\n");
  await writeFile(join(repo, renamedFrom), "rename me\n");
  await mustGit(repo, ["add", "--", edited, renamedFrom]);
  await mustGit(repo, ["-c", "user.name=Phase Zero", "-c", "user.email=phase0@example.invalid",
    "commit", "-m", "fixture"]);
  await writeFile(join(repo, edited), "before\nafter\n");
  await writeFile(join(repo, untracked), "not tracked\n");
  await mustGit(repo, ["mv", "--", renamedFrom, renamedTo]);

  const inside = await git(repo, ["rev-parse", "--is-inside-work-tree"]);
  assert.equal(inside.code, null);
  assert.equal(inside.stdout.trim(), "true");
  const branch = (await mustGit(repo, ["symbolic-ref", "--quiet", "--short", "HEAD"])).trim();
  assert.equal(branch, "phase-0-main");
  record("repository-and-branch", "worktree=true; branch=phase-0-main");

  const status = parseStatus(await mustGit(repo, ["status", "--porcelain=v2", "--branch", "-z", "--untracked-files=all"]));
  assert.equal(status.branch, branch);
  assert.ok(status.entries.some((e) => e.kind === "1" && e.xy === ".M" && e.path === edited));
  assert.ok(status.entries.some((e) => e.kind === "2" && e.xy === "R." && e.path === renamedTo && e.origPath === renamedFrom));
  assert.ok(status.entries.some((e) => e.kind === "?" && e.path === untracked));
  record("porcelain-v2-z", "modified, staged rename and untracked Unicode/metacharacter paths parsed exactly");

  const unstaged = parseRawDiff(await mustGit(repo, ["diff", "--raw", "-z", "--no-ext-diff", "--no-textconv", "--find-renames", "--"]));
  const staged = parseRawDiff(await mustGit(repo, ["diff", "--cached", "--raw", "-z", "--no-ext-diff", "--no-textconv", "--find-renames", "--"]));
  assert.ok(unstaged.some((e) => e.status === "M" && e.path === edited));
  assert.ok(staged.some((e) => e.status.startsWith("R") && e.oldPath === renamedFrom && e.path === renamedTo));
  record("raw-z-diff", "unstaged modification and staged rename parsed with modes, object IDs and exact paths");

  const nonRepo = await git(outside, ["rev-parse", "--is-inside-work-tree"]);
  assert.equal(nonRepo.exitCode, 128);
  assert.match(nonRepo.stderr, /not a git repository/i);
  record("not-repository", `NOT_REPOSITORY; exit=${nonRepo.exitCode}`);

  const absent = await git(repo, ["--version"], { bin: join(base, "git-missing.exe") });
  assert.equal(absent.code, "GIT_NOT_FOUND");
  record("git-missing", "GIT_NOT_FOUND (ENOENT)");

  const timeout = await run(process.execPath, ["-e", "setTimeout(() => {}, 10000)"], repo, 150);
  assert.equal(timeout.code, "COMMAND_TIMEOUT");
  record("timeout", "COMMAND_TIMEOUT; child terminated");

  const failed = await git(repo, ["status", "--phase-0-invalid-option"]);
  assert.equal(failed.code, "NONZERO_EXIT");
  assert.notEqual(failed.exitCode, 0);
  record("nonzero", `NONZERO_EXIT; exit=${failed.exitCode}`);

  const injected = await readFile(join(repo, edited), "utf8");
  assert.match(injected, /after/);
  const { stat } = await import("node:fs/promises");
  await assert.rejects(stat(join(base, "INJECTED")), { code: "ENOENT" });
  record("argument-safety", "shell=false and argument arrays; metacharacters remained literal; no injection marker");
  report.passed = true;
} catch (error) {
  report.error = String(error);
  process.exitCode = 1;
  console.error(error);
} finally {
  if (fixture) await rm(fixture, { recursive: true, force: true });
  await writeFile(join(output, "git-report.json"), JSON.stringify(report, null, 2) + "\n");
  console.log(`Git 0.4: ${report.passed ? "PASS" : "FAIL"}; ${report.checks.length} checks; ${join(output, "git-report.json")}`);
}
