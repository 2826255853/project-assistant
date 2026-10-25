// S1-06: real CLI requests through the production Rust adapter, never real keys.
import assert from "node:assert/strict";
import {spawn,spawnSync} from "node:child_process";
import {randomUUID, createHash} from "node:crypto";
import {mkdir, mkdtemp, readFile, writeFile, rm, readdir} from "node:fs/promises";
import {tmpdir} from "node:os";
import {resolve, join} from "node:path";
import {setTimeout as delay} from "node:timers/promises";
import {controlledSite} from "./codex/controlled-site.mjs";
import {discoverCodex} from "./codex/environment.mjs";

assert.equal(process.platform, "win32", "Isolation acceptance requires Windows");
assert.ok(process.env.npm_execpath, "Run via npm run verify:isolation");
const root = resolve(import.meta.dirname, "..");
const routesOnly = process.argv.includes("--routes-only");
const output = join(root, "artifacts", routesOnly ? "phase-1-7-routes-only" : "phase-1-7");
await mkdir(output, {recursive:true});
const temporary = await mkdtemp(join(tmpdir(), "pa-isolation-"));
const ownedRoot = join(temporary, "owned");
const external = join(temporary, "external");
await mkdir(ownedRoot); await mkdir(external);
const keyA = `synthetic-isolation-${randomUUID()}`;
const keyB = `synthetic-isolation-${randomUUID()}`;
// A throwaway CA/leaf is trusted only by the cfg(test) reqwest client. Nothing
// is installed into the user's Windows certificate stores.
const certificate = join(temporary,"fixture-ca.pem");
const privateKey = join(temporary,"fixture-key.pem");
const openssl = process.env.PA_ISOLATION_OPENSSL_BIN || join(process.env.ProgramFiles,"Git","usr","bin","openssl.exe");
const certificateResult = spawnSync(openssl,["req","-x509","-newkey","rsa:2048","-nodes","-sha256","-days","1",
  "-keyout",privateKey,"-out",certificate,"-subj","/CN=Project Assistant synthetic isolation",
  "-addext","subjectAltName=IP:127.0.0.1","-addext","basicConstraints=critical,CA:TRUE"],
  {windowsHide:true,timeout:15000,stdio:"ignore"});
assert.equal(certificateResult.status,0,"synthetic TLS certificate creation failed; install Git OpenSSL or set PA_ISOLATION_OPENSSL_BIN");
const tls = {cert:await readFile(certificate),key:await readFile(privateKey)};
const sites = {a:await controlledSite("OWNED_A", keyA,{tls}), b:await controlledSite("OWNED_B", keyB,{tls})};
const report = {step:"1.7", date:new Date().toISOString(), timezone:"Asia/Shanghai", passed:false,
  validation:routesOnly?"routes-only":"full",
  syntheticDataOnly:true, realOfficialAuthentication:"not-tested-reserved", checks:[], routes:[], desktops:[],
  environment:{node:process.version}, limits:[
    "Synthetic upstreams use cfg(test)-only loopback HTTPS and a temporary CA. Production permits normal system TLS trust and public pinned addresses only.",
    "No remote provider/model usage, real Key, cc-switch application, task concurrency or profile switching state machine is tested.",
    "The relay verifies the selected endpoint and on-wire model/effort; it cannot attest a provider's internal model implementation."]};
const externalFiles = ["config.toml","auth.json","cc-switch-version.txt"];
async function externalState(version) {
  await Promise.all(externalFiles.map((file,index) => writeFile(join(external,file), `${version}-${index}-synthetic-external-only\n`)));
}
async function hashes() {
  return Promise.all(externalFiles.map(async file => createHash("sha256").update(await readFile(join(external,file))).digest("hex")));
}
await externalState("absent");
const sentinel = spawn(process.execPath, ["-e", "setInterval(()=>{},1000)"], {windowsHide:true, stdio:"ignore"});
sentinel.on("error", ()=>{});

async function run(label, executable, args, extra={}) {
  console.log(label);
  let captured = "";
  await new Promise((done,reject) => {
    const child = spawn(executable,args,{cwd:root,windowsHide:true,env:{...process.env,CARGO_TERM_COLOR:"never",...extra},stdio:["ignore","pipe","pipe"]});
    const timer = setTimeout(()=>{child.kill();reject(new Error(`${label}: timeout`));},900000);
    for (const stream of [child.stdout,child.stderr]) stream.on("data",chunk=>{captured+=chunk;});
    child.on("error", error=>{clearTimeout(timer);reject(error);});
    child.on("close",code=>{clearTimeout(timer);code===0?done():reject(new Error(`${label}: exit ${code}`));});
  }).finally(async ()=>{
    assert.ok(!captured.includes(keyA) && !captured.includes(keyB), "secret sentinel in worker output");
    await writeFile(join(output,`${label}.log`),captured);
  });
  report.checks.push(label);
  return captured;
}
const npm = (label,args)=>run(label,process.execPath,[process.env.npm_execpath,...args]);
try {
  const executable = await discoverCodex();
  report.environment.cli = (await run("cli-version",executable,["--version"])).trim();
  report.environment.rust = (await run("rust-version","rustc",["--version"])).trim();
  report.environment.npm = (await npm("npm-version",["--version"])).trim();
  if (!routesOnly) {
    await npm("frontend-build",["run","build"]);
    await run("rust-format","cargo",["fmt","--check","--manifest-path","src-tauri/Cargo.toml"]);
    await npm("ipc-tests",["run","test:ipc"]);
    await npm("codex-offline-tests",["run","test:codex"]);
  }
  const rustTests = await run("rust-tests","cargo",["test","--locked","--manifest-path","src-tauri/Cargo.toml","--lib"]);
  const rustSummary = rustTests.match(/test result: ok\. (\d+) passed; 0 failed; (\d+) ignored/);
  assert.ok(rustSummary); report.rustTestsPassed=Number(rustSummary[1]); report.rustHelpersIgnored=Number(rustSummary[2]);
  const workerPath = rustTests.match(/Running unittests[^\r\n]*\(([^)]+\.exe)\)/)?.[1];
  assert.ok(workerPath,"missing Rust test executable");
  const testExecutable = resolve(root,workerPath);
  for (const scenario of ["a","b","external-live","project","shared","drift","redirect","tls-untrusted","unauthorized","echo-secret-error","echo-secret-success","timeout","cancel","a-restored"]) {
    sites.a.setMode("ok"); sites.b.setMode("ok");
    const site = scenario === "b" ? sites.b : sites.a;
    const key = scenario === "b" ? keyB : keyA;
    if (scenario === "redirect") sites.a.setMode("redirect",`${sites.b.baseUrl}/responses`);
    if (scenario === "unauthorized") sites.a.setMode("unauthorized");
    if (scenario.startsWith("echo-secret-")) sites.a.setMode(scenario);
    if (scenario === "timeout" || scenario === "cancel") sites.a.setMode("hang");
    if (scenario === "external-live") sites.a.setMode("hold");
    const before = {a:sites.a.requests.length,b:sites.b.requests.length};
    const beforeHashes = await hashes();
    const worker = run(`route-${scenario}`,testExecutable,[
      "owned_process::tests::actual_cli_route_acceptance","--ignored","--exact","--nocapture"],{
        PA_ISOLATION_CODEX_BIN:executable,PA_ISOLATION_ROOT:ownedRoot,PA_ISOLATION_ENDPOINT:site.baseUrl,
        PA_ISOLATION_SCENARIO:scenario,PA_ISOLATION_SYNTHETIC_KEY:key,
        PA_ISOLATION_CA:certificate,
        CODEX_HOME:external,USERPROFILE:external,HOME:external,APPDATA:external,LOCALAPPDATA:external,
        OPENAI_API_KEY:"synthetic-inherited-must-not-use",CODEX_API_KEY:"synthetic-inherited-must-not-use",
        OPENAI_BASE_URL:sites.b.baseUrl,OPENAI_API_BASE:sites.b.baseUrl,CODEX_MODEL_PROVIDER:"external",
        HTTP_PROXY:sites.b.baseUrl,HTTPS_PROXY:sites.b.baseUrl,ALL_PROXY:sites.b.baseUrl,NO_PROXY:"",
        http_proxy:sites.b.baseUrl,https_proxy:sites.b.baseUrl,all_proxy:sites.b.baseUrl,
        CC_SWITCH_HOME:external,CC_SWITCH_BIN:join(external,"missing-or-upgraded.exe"),
    });
    let expectedHashes = beforeHashes;
    if (scenario === "external-live") {
      // The controlled request is held while an external tool changes its own
      // config/auth/version fixtures. The application never reads those files.
      let settled = false;
      worker.then(()=>{settled=true;},()=>{settled=true;});
      const deadline = Date.now()+30000;
      while (sites.a.requests.length===before.a && !settled && Date.now()<deadline) await delay(30);
      if (sites.a.requests.length>before.a) {
        await externalState("upgraded-manually"); expectedHashes = await hashes();
        sites.a.release(); sites.a.setMode("ok");
      }
    }
    const captured = await worker;
    const line = captured.split(/\r?\n/).find(line=>line.startsWith("ISOLATION_EVIDENCE:"));
    assert.ok(line,"missing Rust evidence");
    const evidence = JSON.parse(line.slice("ISOLATION_EVIDENCE:".length));
    const requests = {a:sites.a.requests.slice(before.a),b:sites.b.requests.slice(before.b)};
    if (["project","shared","drift","tls-untrusted"].includes(scenario)) {
      assert.equal(requests.a.length+requests.b.length,0);
      assert.equal(evidence.errorCode,scenario === "tls-untrusted" ? "PA_PROVIDER_TEST_FAILED" : "PA_PROVIDER_SHARED_ROUTE_UNSAFE");
    } else {
      assert.ok(requests[scenario === "b" ? "b" : "a"].length>0,"no actual upstream request");
      assert.equal(requests[scenario === "b" ? "a" : "b"].length,0,"request reached other site");
      for (const request of [...requests.a,...requests.b]) {
        assert.equal(request.keyMatches,true); assert.equal(request.model,"gpt-5.1-codex-mini"); assert.equal(request.effort,"medium");
      }
    }
    assert.deepEqual(await hashes(),expectedHashes,"application changed external fixtures");
    assert.equal(sentinel.exitCode,null,"unrelated process was stopped");
    const remaining = await readdir(ownedRoot);
    assert.ok(remaining.every(name=>!name.startsWith("owned-")),"owned home was not removed");
    report.routes.push({...evidence,requests,externalUnchanged:true,unrelatedProcessAlive:true});
  }
  if (!routesOnly) {
  await run("release-check","cargo",["check","--locked","--release","--manifest-path","src-tauri/Cargo.toml"]);
  await npm("embedded-debug-build",["run","tauri","--","build","--debug","--no-bundle","--config",JSON.stringify({build:{beforeBuildCommand:""}})]);
  for (const mode of ["built","dev"]) {
    for (const scenario of ["shell","credentials"]) {
    await npm(`${mode}-${scenario}-desktop`,["run","verify:desktop","--",mode,scenario]);
    const desktop = JSON.parse(await readFile(join(root,"artifacts",`shell-${mode}-${scenario}`,"report.json"),"utf8"));
    assert.equal(desktop.passed,true); report.desktops.push(desktop);
    if (scenario === "credentials") assert.equal(desktop.credentialCleanupPassed,true);
    }
  }
  }
  report.passed=true;
} catch (error) {
  report.error=String(error); console.error(report.error); process.exitCode=1;
} finally {
  sites.a.release(); sites.b.release();
  await Promise.all([sites.a.close(),sites.b.close()]);
  sentinel.kill();
  await rm(temporary,{recursive:true,force:true,maxRetries:5,retryDelay:200});
  report.temporaryRemoved=true;
  await writeFile(join(output,"report.json"),JSON.stringify(report,null,2)+"\n");
}
console.log(`Isolation verification: ${report.passed?"passed":"failed"}; artifacts/${routesOnly?"phase-1-7-routes-only":"phase-1-7"}/report.json`);
