import assert from 'node:assert/strict';
import {spawn,spawnSync} from 'node:child_process';
import {randomUUID} from 'node:crypto';
import {mkdir,mkdtemp,readFile,writeFile,rm,readdir} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {controlledSite} from './codex/controlled-site.mjs';
import {discoverCodex} from './codex/environment.mjs';
assert.equal(process.platform,'win32');assert.ok(process.env.npm_execpath);
const root=resolve(import.meta.dirname,'..'),output=join(root,'artifacts','phase-1-8');await mkdir(output,{recursive:true});
const temporary=await mkdtemp(join(tmpdir(),'pa-providers-acceptance-')),owned=join(temporary,'owned');await mkdir(owned);
const keys={a:'synthetic-provider-'+randomUUID(),b:'synthetic-provider-'+randomUUID()};
const report={step:'1.8',date:new Date().toISOString(),timezone:'Asia/Shanghai',passed:false,syntheticDataOnly:true,checks:[],routes:[],desktops:[],limits:['AI page remains 1.9; no task IPC.','Rollback/idle faults: production state machine, injected candidate results and synthetic tasks.','Official sources reserved; no paid models or real API keys.']};
async function run(label,exe,args,extra={}){
 console.log(label);let captured='';
 try{await new Promise((done,reject)=>{const child=spawn(exe,args,{cwd:root,windowsHide:true,env:{...process.env,CARGO_TERM_COLOR:'never',...extra},stdio:['ignore','pipe','pipe']});const timer=setTimeout(()=>{child.kill();reject(new Error(label+': timeout'))},900000);for(const s of [child.stdout,child.stderr])s.on('data',v=>captured+=v);child.on('error',e=>{clearTimeout(timer);reject(e)});child.on('close',code=>{clearTimeout(timer);code===0?done():reject(new Error(label+': exit '+code))});});}
 finally{assert.ok(Object.values(keys).every(k=>!captured.includes(k)),'secret sentinel in output');await writeFile(join(output,label+'.log'),captured);}
 report.checks.push(label);return captured;
}
const npm=(label,args)=>run(label,process.execPath,[process.env.npm_execpath,...args]);
let sites;
try{
 const cert=join(temporary,'ca.pem'),key=join(temporary,'key.pem'),openssl=process.env.PA_ISOLATION_OPENSSL_BIN||join(process.env.ProgramFiles,'Git','usr','bin','openssl.exe');
 assert.equal(spawnSync(openssl,['req','-x509','-newkey','rsa:2048','-nodes','-sha256','-days','1','-keyout',key,'-out',cert,'-subj','/CN=Project Assistant synthetic providers','-addext','subjectAltName=IP:127.0.0.1','-addext','basicConstraints=critical,CA:TRUE'],{windowsHide:true,timeout:15000,stdio:'ignore'}).status,0);
 const tls={cert:await readFile(cert),key:await readFile(key)};sites={a:await controlledSite('PROVIDER_A',keys.a,{tls}),b:await controlledSite('PROVIDER_B',keys.b,{tls})};
 const exe=await discoverCodex();report.environment={node:process.version,npm:(await npm('npm-version',['--version'])).trim(),rust:(await run('rust-version','rustc',['--version'])).trim(),cli:(await run('cli-version',exe,['--version'])).trim()};
 await npm('frontend-build',['run','build']);await run('rust-format','cargo',['fmt','--check','--manifest-path','src-tauri/Cargo.toml']);
 const tests=await run('rust-tests','cargo',['test','--locked','--manifest-path','src-tauri/Cargo.toml','--lib']);
 const worker=resolve(root,tests.match(/Running unittests[^\r\n]*\(([^)]+\.exe)\)/)?.[1]||'missing');
 report.rustSummary=tests.match(/test result: ok\. (\d+) passed; 0 failed; (\d+) ignored/)?.[0];assert.ok(report.rustSummary);
 await npm('ipc-tests',['run','test:ipc']);await npm('codex-offline-tests',['run','test:codex']);
 for(const scenario of ['catalog','test','a','b','redirect','cancel']){
  sites.a.setMode('ok');sites.b.setMode('ok');if(scenario==='redirect')sites.a.setMode('redirect',sites.b.baseUrl+'/responses');if(scenario==='cancel')sites.a.setMode('hang');
  const selected=scenario==='b'?'b':'a',before={a:sites.a.requests.length,b:sites.b.requests.length};
  const result=await run('service-'+scenario,worker,['providers::tests::actual_cli_provider_service_acceptance','--ignored','--exact','--nocapture'],{PA_PROVIDER_CODEX_BIN:exe,PA_PROVIDER_ROOT:owned,PA_PROVIDER_ENDPOINT:sites[selected].baseUrl,PA_PROVIDER_CA:cert,PA_PROVIDER_SYNTHETIC_KEY:keys[selected],PA_PROVIDER_SCENARIO:scenario});
  const line=result.split(/\r?\n/).find(v=>v.startsWith('PROVIDER_EVIDENCE:'));assert.ok(line);const evidence=JSON.parse(line.slice('PROVIDER_EVIDENCE:'.length));
  const requests={a:sites.a.requests.slice(before.a),b:sites.b.requests.slice(before.b)};
  if(scenario==='catalog')assert.equal(requests.a.length+requests.b.length,0);else{if(scenario!=='cancel')assert.ok(requests[selected].length>0);assert.equal(requests[selected==='a'?'b':'a'].length,0);for(const r of requests[selected]){assert.equal(r.keyMatches,true);assert.equal(r.model,'gpt-5.1-codex-mini');assert.equal(r.effort,'medium')}}
  assert.equal(evidence.cleaned,true);assert.equal((await readdir(owned)).length,0);report.routes.push({...evidence,requests});
 }
 await npm('isolation-regression',['run','verify:isolation','--','--routes-only']);
 await run('release-check','cargo',['check','--locked','--release','--manifest-path','src-tauri/Cargo.toml']);
 await npm('embedded-debug-build',['run','tauri','--','build','--debug','--no-bundle','--config',JSON.stringify({build:{beforeBuildCommand:''}})]);
 for(const mode of ['built','dev']){for(const scenario of ['providers','credentials']){await npm(mode+'-'+scenario+'-desktop',['run','verify:desktop','--',mode,scenario]);const d=JSON.parse(await readFile(join(root,'artifacts','shell-'+mode+'-'+scenario,'report.json'),'utf8'));assert.equal(d.passed,true);report.desktops.push(d)}}
 report.passed=true;
}catch(error){report.error=String(error);console.error(report.error);process.exitCode=1;}
finally{if(sites){sites.a.release();sites.b.release();await Promise.all([sites.a.close(),sites.b.close()])}await rm(temporary,{recursive:true,force:true,maxRetries:5,retryDelay:200});report.temporaryRemoved=true;await writeFile(join(output,'report.json'),JSON.stringify(report,null,2)+'\n');}
console.log('Provider verification: '+(report.passed?'passed':'failed')+'; artifacts/phase-1-8/report.json');
