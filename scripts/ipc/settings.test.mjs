import test from "node:test";
import assert from "node:assert/strict";
import { isSettingsView } from "../../src/ipc/settings.ts";

const defaults={revision:1,values:[
  {key:"search_engine",value:"bing"},{key:"editor",value:null},{key:"project_sort",value:"recent"},
  {key:"approval_policy",value:"ask"},{key:"log_level",value:"info"},{key:"summary_retention_days",value:30},
  {key:"ai_defaults",value:null},{key:"overlay_enabled",value:true},{key:"overlay_shortcut",value:"Ctrl+Alt+Space"},{key:"overlay_position",value:null},
]};
test("settings response rejects unknown/duplicate/secret values, malformed revision and coordinates",()=>{
  assert.equal(isSettingsView(defaults),true);
  for(const invalid of [
    {...defaults,revision:0},{...defaults,revision:1.5},{...defaults,secret:"synthetic"},
    {...defaults,values:[...defaults.values,{key:"api_key",value:"synthetic"}]},
    {...defaults,values:defaults.values.map((v,i)=>i===0?{key:"search_engine",value:"unknown"}:v)},
    {...defaults,values:defaults.values.map((v,i)=>i===0?defaults.values[1]:v)},
    {...defaults,values:defaults.values.map(v=>v.key==="overlay_position"?{...v,value:{x:0,y:0,monitor:null,scaleFactor:Infinity}}:v)},
  ]) assert.equal(isSettingsView(invalid),false);
});
