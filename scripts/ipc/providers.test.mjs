import test from "node:test";
import assert from "node:assert/strict";
import {readFile} from "node:fs/promises";
import {isSelection,isProviderList,isProviderProfile,isCapability,isOperation,isReceipt} from "../../src/ipc/providers.ts";
const selection={profileId:"profile-a",expectedRevision:2,modelId:"synthetic-model",effort:"high"};
const profile={id:"profile-a",name:"Synthetic",sourceKind:"relay",adapterKind:"self_managed",endpoint:"https://example.invalid/v1",wireApi:"responses",authKind:"api_key",defaultModel:null,defaultEffort:null,enabled:true,configRevision:2,credential:{status:"configured_unverified",revision:1},available:true,unavailableCode:null};
const error={category:"external",code:"PA_PROVIDER_TEST_FAILED",message:"fixed",suggestion:"fixed",retryable:true};
test("provider profiles and selections reject unknown fields secrets and invalid revisions",()=>{
 assert.equal(isSelection(selection),true);assert.equal(isProviderProfile(profile),true);
 for(const invalid of [{...selection,secret:"synthetic"},{...selection,expectedRevision:0},{...selection,modelId:""},{...selection,effort:undefined}])assert.equal(isSelection(invalid),false);
 for(const invalid of [{...profile,credentialRef:"credential:a:1"},{...profile,credential:{...profile.credential,secret:"synthetic"}},{...profile,endpoint:"https://u:secret@example.invalid/v1"},{...profile,credential:{...profile.credential,revision:-1}}])assert.equal(isProviderProfile(invalid),false);
 assert.equal(isProviderProfile({...profile,adapterKind:"cc_switch",available:false,unavailableCode:"PA_PROVIDER_ADAPTER_UNAVAILABLE"}),true);
});
test("provider list never equates list access with ready and capability TTL is bounded",()=>{
 assert.equal(isProviderList({profiles:[profile],active:null,configState:"unconfigured"}),true);
 assert.equal(isProviderList({profiles:[profile],active:null,configState:"ready"}),false);
 const cap={modelId:"synthetic",efforts:["low","high"],supportsDefaultEffort:true,contextWindow:null,source:"cli_catalog",sourceVersion:"owned-v1:hash",profileRevision:2,observedAt:1000,expiresAt:86401000};
 assert.equal(isCapability(cap),true);assert.equal(isCapability({...cap,expiresAt:86401001}),false);assert.equal(isCapability({...cap,efforts:["high","high"]}),false);
});
test("operation union enforces result error exclusivity and all recovery outcomes",()=>{
 const base={operationId:"op-a",phase:"checking",result:null,error:null};assert.equal(isOperation(base),true);assert.equal(isReceipt({operationId:"op-a",state:"checking"}),true);
 assert.equal(isOperation({...base,result:{}}),false);assert.equal(isOperation({...base,phase:"failed",error}),true);
 assert.equal(isOperation({...base,phase:"completed",result:{outcome:"applied",active:selection,configState:"ready"}}),true);
 for(const outcome of ["unchanged","rolled_back","rollback_failed"]){assert.equal(isOperation({...base,phase:"completed",result:{outcome,active:outcome==="rollback_failed"?null:selection,configState:outcome==="rollback_failed"?"error":"ready",error}}),true)}
 assert.equal(isOperation({...base,phase:"completed",result:{outcome:"rollback_failed",active:selection,configState:"error",error}}),false);
 assert.equal(isOperation({...base,phase:"completed",result:{testedRevision:2,testedAt:1000,authenticated:true,modelAccepted:true,effortAccepted:true,evidence:"connection_probe"}}),true);
});
test("all nine provider commands have explicit main-only permissions and no task entry",async()=>{
 const names=["providers_list","provider_save","provider_set_enabled","provider_delete","provider_models","provider_test","provider_switch","operation_get","operation_cancel"];
 const main=JSON.parse(await readFile(new URL("../../src-tauri/capabilities/default.json",import.meta.url),"utf8"));
 const overlay=JSON.parse(await readFile(new URL("../../src-tauri/capabilities/overlay.json",import.meta.url),"utf8"));
 const lib=await readFile(new URL("../../src-tauri/src/lib.rs",import.meta.url),"utf8");
 for(const n of names){assert.ok(main.permissions.includes("allow-"+n.replaceAll("_","-")));assert.ok(!overlay.permissions.includes("allow-"+n.replaceAll("_","-")));assert.match(lib,new RegExp("async fn "+n));}
 assert.ok(!main.permissions.some(p=>/task-start|snapshot/.test(p)));
});
