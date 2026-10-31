import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { arch, cpus, release, totalmem, version } from "node:os";

assert.equal(process.platform,"win32");
assert.ok(process.env.npm_execpath,"Run via npm run verify:projects");
const root=resolve(import.meta.dirname,".."), output=join(root,"artifacts","phase-2");
await mkdir(output,{recursive:true});
const report={step:"2.9",task:"S2-09",date:new Date().toISOString(),timezone:"Asia/Shanghai",passed:false,syntheticDataOnly:true,checks:[],desktops:[],stage1Regression:[],manualAcceptance:[],handoff:{},risks:[],limits:["Native picker first-use mouse/keyboard acceptance and real-device Chinese IME remain manual; automated page testing does not claim physical interaction.","Real directory/text OS delivery and a synthetic configured PE editor are checked; acceptance does not prove external application loading.","Final project-open target requires rollback-only NTFS metadata protection and write-attribute permission. TxF-unavailable/ReFS/protected targets fail closed; ReFS was not tested.","Active project is process-only; process restart clears it while recent-use records remain.","No real tasks, keys or model requests."]};
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
  report.checks.push("project-path-search-git-adapter-unit-coverage");
  for(const mode of ["built","dev"]) {
    await npm(`${mode}-projects-desktop`,["run","verify:desktop","--",mode,"projects"]);
    const result=JSON.parse(await readFile(join(root,"artifacts",`shell-${mode}-projects`,"report.json"),"utf8"));assert.equal(result.passed,true);report.desktops.push(result);
  }
  const phase1Checks=[
    {name:"settings-regression",script:"verify:settings",report:"phase-1-4/report.json"},
    {name:"logging-regression",script:"verify:logging",report:"phase-1-5/report.json"},
    {name:"credentials-regression",script:"verify:credentials",report:"phase-1-6/report.json"},
    {name:"isolation-regression",script:"verify:isolation",report:"phase-1-7/report.json"},
    {name:"providers-regression",script:"verify:providers",report:"phase-1-8/report.json"},
  ];
  for(const suite of phase1Checks){
    await npm(suite.name,["run",suite.script,...(suite.args??[])]);
    const result=JSON.parse(await readFile(join(root,"artifacts",suite.report),"utf8"));
    const passed=result.passed===true;
    assert.equal(passed,true,`${suite.name} report did not confirm its scoped pass`);
    report.stage1Regression.push({name:suite.name,sourceReport:`artifacts/${suite.report}`,passed:true});
  }
  report.manualAcceptance=[
    {item:"Native project directory and file picker first use: physical mouse/keyboard, selection and cancellation",status:"requires attended Windows desktop acceptance"},
    {item:"Project search composition using an actual Chinese IME",status:"requires attended device acceptance"},
    {item:"External default program/editor loading after systemAccepted",status:"not asserted by system handoff result"},
    {item:"Phase 1 tray mouse, monitor hotplug/movement and live DPI",status:"remains separate step 1.10 hardware acceptance"},
  ];
  report.handoff={stage3MustRevalidate:["Project remains registered and authorized at task creation.","Authorized directory identity still matches the intended working directory and cwd.","Project, global and per-request configuration precedence and validity are checked; Stage 2 selection/opening is not task authorization."],notImplemented:["Task/session creation and restoration","Real task submission, cancellation and event streaming","Approval decision/consumption atomicity","Project settings writes and project AI default application"],futureScope:["Git diff body/history and Git write operations remain outside Stage 2.","Real HTTPS search remains Stage 4; reserved official-source authentication is excluded."]};
  report.risks=[{id:"R-019",status:"partially open",detail:"Path authorization and NTFS delivery evidence covers tested Windows x64. ReFS/TxF-unavailable delivery is rejected; ReFS and other Windows versions are not validated."},{id:"R-014",status:"stage 1 configuration-layer gate only",detail:"Stage 3 real-task gateway and runtime concurrency remain open."},{id:"R-016",status:"regression rerun",detail:"This does not broaden supported providers or validate real task use."}];
  report.passed=true;
} catch(error) {report.error=String(error);console.error(report.error);process.exitCode=1;}
finally {await writeFile(join(output,"report.json"),JSON.stringify(report,null,2)+"\n");}
console.log(`Project stage 2 regression and stage 3 handoff verification: ${report.passed?"passed":"failed"}; artifacts/phase-2/report.json`);
