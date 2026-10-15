import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";

assert.ok(process.env.npm_execpath,"Run via npm run verify:settings");
const root=resolve(import.meta.dirname,"..");
const output=resolve(root,"artifacts","phase-1-4");
await mkdir(output,{recursive:true});
const report={step:"1.4",date:new Date().toISOString(),timezone:"Asia/Shanghai",passed:false,checks:[],desktops:[],limits:["Tray dispatch is exercised through the actual menu callback function; physical mouse interaction with the Windows notification area is not automated.","Monitor removal is injected using a missing monitor identifier and out-of-bounds position; physical monitor disconnect and live DPI change require hardware acceptance."]};
async function run(label,args,env={},executable=process.execPath) {
  console.log(label); let outputText="";
  await new Promise((done,reject)=>{
    const child=spawn(executable,executable===process.execPath?[process.env.npm_execpath,...args]:args,{cwd:root,env:{...process.env,...env},windowsHide:true,stdio:["ignore","pipe","pipe"]});
    const timer=setTimeout(()=>{child.kill();reject(new Error(`${label}: timeout`));},900000);
    child.stdout.on("data",c=>outputText+=c);child.stderr.on("data",c=>outputText+=c);
    child.on("error",e=>{clearTimeout(timer);reject(e);});
    child.on("close",code=>{clearTimeout(timer);code===0?done():reject(new Error(`${label}: exit ${code}`));});
  }).finally(()=>writeFile(resolve(output,`${label}.log`),outputText));
  report.checks.push(label);
}
try {
  await run("frontend-build",["run","build"]);
  await run("rust-format",["fmt","--check","--manifest-path","src-tauri/Cargo.toml"],{},"cargo");
  await run("rust-tests",["test","--locked","--manifest-path","src-tauri/Cargo.toml","--lib"],{},"cargo");
  await run("rust-check",["check","--locked","--manifest-path","src-tauri/Cargo.toml"],{},"cargo");
  await run("ipc-tests",["run","test:ipc"]);
  for (const mode of ["dev","built"]) {
    if(mode==="built") await run("embedded-debug-build",["run","tauri","--","build","--debug","--no-bundle"]);
    const fixture=await mkdtemp(resolve(tmpdir(),"pa-settings-desktop-"));
    try {
      for(const scenario of ["settings-write","settings-read","settings-enabled"]) {
        await run(`${mode}-${scenario}`,["run","verify:desktop","--",mode,scenario],{PA_SETTINGS_VERIFY_DIR:fixture});
        const dir=resolve(root,"artifacts",`shell-${mode}-${scenario}`);
        const desktop=JSON.parse(await readFile(resolve(dir,"report.json"),"utf8"));
        assert.equal(desktop.passed,true);report.desktops.push(desktop);
        const log=await readFile(resolve(dir,"process.log"),"utf8");
        assert.equal(log.includes("synthetic-secret-DO-NOT-LOG"),false);
        assert.equal(log.includes("synthetic-removed-monitor"),false);
      }
    } finally {await rm(fixture,{recursive:true,force:true});}
  }
  report.passed=true;
} catch(error) {report.error=String(error);process.exitCode=1;console.error(report.error);}
finally {await writeFile(resolve(output,"report.json"),JSON.stringify(report,null,2)+"\n");}
