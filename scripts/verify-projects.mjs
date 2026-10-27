import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";

assert.equal(process.platform,"win32");
assert.ok(process.env.npm_execpath,"Run via npm run verify:projects");
const root=resolve(import.meta.dirname,".."), output=join(root,"artifacts","phase-2-3");
await mkdir(output,{recursive:true});
const report={step:"2.3",date:new Date().toISOString(),timezone:"Asia/Shanghai",passed:false,syntheticDataOnly:true,checks:[],desktops:[],limits:["Native picker mouse/keyboard flow remains manual acceptance in 2.9.","Usage selection/OS acceptance uses repository callbacks; active project is 2.8 and system delivery is 2.5.","No real tasks, keys or model requests."]};
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
  report.environment={node:process.version,npm:(await npm("npm-version",["--version"])).trim(),rust:(await run("rust-version","rustc",["--version"])).trim()};
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
console.log(`Project storage verification: ${report.passed?"passed":"failed"}; artifacts/phase-2-3/report.json`);
