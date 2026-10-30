import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { arch, cpus, release, totalmem, version } from "node:os";

assert.equal(process.platform,"win32");
assert.ok(process.env.npm_execpath,"Run via npm run verify:projects");
const root=resolve(import.meta.dirname,".."), output=join(root,"artifacts","phase-2-8");
await mkdir(output,{recursive:true});
const report={step:"2.8",date:new Date().toISOString(),timezone:"Asia/Shanghai",passed:false,syntheticDataOnly:true,checks:[],desktops:[],limits:["Git uses Windows-registered system Git installations with exact-path validation and a five-second in-memory summary cache. Porcelain v2 status includes submodule dirtiness; no diff body, history, project script, external helper, worktree or Git write operation is run in production.","Git absence, timeout, output overflow, malformed output, detached HEAD, empty repository, tracked/untracked/rename/unmerged and submodule status are covered by Rust adapter tests; actual IPC and built/dev desktop exercise authorized synthetic repositories.","Native picker first-use mouse/keyboard acceptance remains manual in 2.9; automated page testing does not claim physical Chinese IME validation.","Real directory/text OS delivery and a synthetic configured PE editor are checked; acceptance does not prove application loading.","Final project-open target requires rollback-only NTFS metadata protection and write-attribute permission. TxF-unavailable/ReFS/protected targets fail closed; no weaker fallback. ReFS was not tested.","Real preexisting attribute-handle/reparse mutation and rollback cases are tested. Timed-out delivery may complete later, never automatically retried.","Active project is a process-only state independent of ActiveContext; process restart clears it while recent-use records remain. No task/session/model is fabricated.","No real tasks, keys or model requests."]};
async function run(label,command,args) {
  console.log(label); let captured="";
  try {
    await new Promise((done,reject)=>{
      const child=spawn(command,args,{cwd:root,windowsHide:true,env:{...process.env,CARGO_TERM_COLOR:"never"},stdio:["ignore","pipe","pipe"]});
      const timer=setTimeout(()=>{child.kill();reject(new Error(`${label}: timeout`));},900000);
      for(const stream of [child.stdout,child.stderr]) stream.on("data",chunk=>captured+=chunk);
      child.on("error",error=>{clearTimeout(timer);reject(error);});
      child.on("close",code=>{clearTimeout(timer);code===0?done():reject(new Error(`${label}: exit ${code}`));});
    });
  } finally { await writeFile(join(output,`${label}.log`),captured); }
  report.checks.push(label);return captured;
}
const npm=(label,args)=>run(label,process.execPath,[process.env.npm_execpath,...args]);
try {
  report.environment={node:process.version,npm:(await npm("npm-version",["--version"])).trim(),rust:(await run("rust-version","rustc",["--version"])).trim(),os:version(),release:release(),arch:arch(),cpu:cpus()[0]?.model,logicalProcessors:cpus().length,memoryGiB:Math.round(totalmem()/1024**3)};
  await npm("frontend-build",["run","build"]);
  await run("rust-format","cargo",["fmt","--check","--manifest-path","src-tauri/Cargo.toml"]);
  const tests=await run("rust-tests","cargo",["test","--locked","--manifest-path","src-tauri/Cargo.toml","--lib"]);
  report.rustSummary=tests.match(/test result: ok\. \d+ passed; 0 failed; \d+ ignored/)?.[0];assert.ok(report.rustSummary);
  await npm("ipc-tests",["run","test:ipc"]);
  await npm("codex-offline-tests",["run","test:codex"]);
  await run("release-check","cargo",["check","--locked","--release","--manifest-path","src-tauri/Cargo.toml"]);
  await npm("storage-regression",["run","verify:storage"]);
  report.storage=JSON.parse(await readFile(join(root,"artifacts","storage","report.json"),"utf8"));assert.equal(report.storage.passed,true);
  for(const mode of ["built","dev"]) {
    await npm(`${mode}-projects-desktop`,["run","verify:desktop","--",mode,"projects"]);
    const result=JSON.parse(await readFile(join(root,"artifacts",`shell-${mode}-projects`,"report.json"),"utf8"));assert.equal(result.passed,true);report.desktops.push(result);
  }
  report.passed=true;
} catch(error) {report.error=String(error);console.error(report.error);process.exitCode=1;}
finally {await writeFile(join(output,"report.json"),JSON.stringify(report,null,2)+"\n");}
console.log(`Project page/storage/search/open/Git/active-project verification: ${report.passed?"passed":"failed"}; artifacts/phase-2-8/report.json`);
