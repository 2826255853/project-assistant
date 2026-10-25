import assert from "node:assert/strict";
import {isProviderProfile,isProviderList} from "../../src/ipc/providers.ts";
export async function verifyProvidersDesktop({evaluate,until,report,debugPort,receive,cdp,mainUrl}){
 const secret="synthetic-provider-desktop-NO-EXPORT";
 const invoke=(command,params={},apiVersion=1)=>"window.__TAURI_INTERNALS__.invoke("+JSON.stringify(command)+",{request:"+JSON.stringify({apiVersion,params})+"})";
 const call=async(c,p,v)=>{const wire=await evaluate(invoke(c,p,v));assert.ok(!JSON.stringify(wire).includes(secret));return wire;};
 const fixture=async action=>assert.equal((await call("ipc_fixture",{action})).status,"ok");
 await until(()=>evaluate("document.querySelector('.main-area')?.dataset.page"),p=>p==="home");
 let profile;
 try{
  let list=await call("providers_list");assert.equal(list.status,"ok");assert.equal(isProviderList(list.data),true);assert.equal(list.data.configState,"unconfigured");
  assert.equal((await call("providers_list",{},2)).error.code,"PA_IPC_VERSION_UNSUPPORTED");
  assert.equal((await call("providers_list",{unknown:secret})).error.code,"PA_IPC_INVALID_REQUEST");
  const input={profileId:null,expectedRevision:null,name:"Synthetic desktop",endpoint:"https://fixture.invalid/v1",defaultModel:null,defaultEffort:null};
  for(const endpoint of ["http://fixture.invalid/v1","https://user:secret@fixture.invalid/v1","https://api.openai.com/v1","https://fixture.invalid/v1?key=secret"]){assert.equal((await call("provider_save",{...input,endpoint})).error.code,"PA_PROVIDER_ENDPOINT_INVALID");}
  let saved=await call("provider_save",input);assert.equal(saved.status,"ok");assert.equal(isProviderProfile(saved.data),true);profile=saved.data;
  assert.equal(profile.credential.status,"missing");
  const put=await call("credential_put",{profileId:profile.id,expectedRevision:profile.configRevision,secret});assert.equal(put.status,"ok");profile.configRevision=put.data.configRevision;
  assert.equal((await call("provider_save",{...input,profileId:profile.id,expectedRevision:1})).error.code,"PA_CONFLICT_REVISION");
  assert.equal((await call("provider_test",{profileId:profile.id,expectedRevision:profile.configRevision,modelId:"unverified",effort:"high"})).error.code,"PA_PROVIDER_CAPABILITY_INVALID");
  assert.equal((await call("operation_get",{operationId:"not-owned"})).error.code,"PA_OPERATION_NOT_FOUND");
  assert.equal((await call("provider_save",{...input,profileId:profile.id,expectedRevision:profile.configRevision,endpoint:"https://other.invalid/v1"})).error.code,"PA_CREDENTIAL_INVALID");
  await fixture("audit_fail_write");assert.equal((await call("provider_set_enabled",{profileId:profile.id,expectedRevision:profile.configRevision,enabled:false})).error.code,"PA_AUDIT_WRITE_FAILED");await fixture("audit_restore_write");
  profile=(await call("provider_set_enabled",{profileId:profile.id,expectedRevision:profile.configRevision,enabled:false})).data;assert.equal(profile.enabled,false);
  profile=(await call("provider_set_enabled",{profileId:profile.id,expectedRevision:profile.configRevision,enabled:true})).data;assert.equal(profile.enabled,true);
  profile=(await call("provider_save",{...input,profileId:profile.id,expectedRevision:profile.configRevision,name:"Edited synthetic"})).data;
  const commands=["providers_list","provider_save","provider_set_enabled","provider_delete","provider_models","provider_test","provider_switch","operation_get","operation_cancel"];
  for(const [query,create,close] of [["window=overlay",null,null],["window=denied","create_denied_window","close_denied_window"]]){
   if(create)await fixture(create);
   const target=await until(async()=>(await(await fetch("http://127.0.0.1:"+debugPort+"/json/list")).json()).find(v=>v.url.includes(query)),Boolean);
   const socket=new WebSocket(target.webSocketDebuggerUrl);await new Promise((done,reject)=>{socket.addEventListener("open",done,{once:true});socket.addEventListener("error",reject,{once:true})});socket.addEventListener("message",receive);
   try{for(const c of commands)assert.equal(await evaluate(invoke(c)+".then(()=>false,()=>true)",socket),true);}finally{socket.close();if(close)await fixture(close);}
  }
  await fixture("navigate_remote");await until(()=>evaluate("document.title"),v=>v==="IPC remote fixture");for(const c of commands)assert.equal(await evaluate(invoke(c)+".then(()=>false,()=>true)"),true);
  await cdp("Page.navigate",{url:mainUrl});await until(()=>evaluate("Boolean(document.querySelector('.app-shell'))"),Boolean);
  const removed=await call("credential_remove",{profileId:profile.id,expectedRevision:profile.configRevision});assert.equal(removed.status,"ok");profile.configRevision=removed.data.configRevision;
  assert.equal((await call("provider_delete",{profileId:profile.id,expectedRevision:profile.configRevision})).data.removed,true);profile=null;
  assert.equal((await call("providers_list")).data.profiles.length,0);report.providerCleanupPassed=true;
  report.results.push("实际 main profile 新建/编辑/启停/删除、Windows 合成凭据、修订/能力/审计失败与九命令窗口/远端权限通过");
 }finally{if(profile){const state=await call("providers_list");const p=state.data?.profiles.find(v=>v.id===profile.id);if(p){const removed=await call("credential_remove",{profileId:p.id,expectedRevision:p.configRevision});if(removed.status==="ok")await call("provider_delete",{profileId:p.id,expectedRevision:removed.data.configRevision});}}}
}
