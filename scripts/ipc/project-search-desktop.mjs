import assert from "node:assert/strict";

export async function verifyProjectSearchDesktop({evaluate, call, fixture, report}) {
  await fixture("project_search_seed");
  const list = async params => {
    const result = await call("projects_list", params);
    assert.equal(result.status, "ok");
    return result.data;
  };
  assert.equal((await list({})).items.length, 50);
  let page = await list({query:"项目助手"});
  assert.deepEqual(page.items.map(p=>p.name),["项目助手","工具项目助手扩展","项目桌面助手"]);
  assert.equal((await list({query:"中文工作台"})).items[0].displayName,"中文工作台");
  assert.equal((await list({query:"C:/SYNTHETIC-PRIVATE-PATH/Synthetic-0099"})).items[0].name,"Synthetic-0099");
  assert.equal((await list({query:"synth0099"})).items[0].name,"Synthetic-0099");
  for(const query of ["无匹配哨兵","助手项目","%_*","   "]) assert.equal((await list({query})).items.length,0);
  const seen = new Set(); let cursor;
  do {
    page = await list({limit:100,...(cursor?{cursor}:{})});
    for(const p of page.items) { assert.equal(seen.has(p.id),false); seen.add(p.id); }
    cursor=page.nextCursor;
  } while(cursor);
  assert.equal(seen.size,1000);
  for(const params of [{query:null},{cursor:null},{limit:null},{limit:0},{limit:101},{limit:1.5},{query:123},{sort:"name"},{cursor:"forged"},{query:"bad\nquery"}]) {
    assert.equal((await call("projects_list",params)).error.code,"PA_IPC_INVALID_REQUEST");
  }
  assert.equal((await call("projects_list",{query:"中".repeat(86)})).error.code,"PA_VALIDATION_INPUT_TOO_LARGE");
  assert.equal((await call("projects_list",{cursor:"x".repeat(513)})).error.code,"PA_VALIDATION_INPUT_TOO_LARGE");
  assert.equal((await call("projects_list",{},2)).error.code,"PA_IPC_VERSION_UNSUPPORTED");
  const previous = await list({limit:1});
  const settings = (await call("settings_get",{scope:"global"})).data;
  assert.equal((await call("settings_update",{scope:"global",expectedRevision:settings.revision,changes:[{key:"project_sort",value:"name"}]})).status,"ok");
  assert.equal((await call("projects_list",{limit:1,cursor:previous.nextCursor})).error.code,"PA_PROJECT_CONFLICT_REVISION");
  assert.equal((await list({limit:1})).items[0].name,"Synthetic-0004");
  // Timer lives in the real WebView, including invoke, Rust/SQLite, transport
  // deserialization and JS result checks. No CDP round-trip in the measurement.
  const performance = await evaluate(`(async()=>{
    const queries=["","项目助手","synthetic","synth0099","中文工作台","C:/synthetic-private-path","无匹配哨兵"];
    const samples=[];
    for(let i=0;i<105;i++){
      const started=performance.now();
      const response=await window.__TAURI_INTERNALS__.invoke("projects_list",{request:{apiVersion:1,params:{query:queries[i%queries.length],limit:50}}});
      if(response.status!=="ok" || response.data.items.length>50 || response.data.items.some(p=>typeof p.absolutePath!=="string" || typeof p.name!=="string")) throw new Error("Search verification failed");
      if(i>=5)samples.push(performance.now()-started);
    }
    samples.sort((a,b)=>a-b);
    return {records:1000,samples:samples.length,warmup:5,p50Ms:samples[49],p95Ms:samples[94],maxMs:samples.at(-1),targetMs:300,userAgent:navigator.userAgent,scope:"WebView invoke through Rust/SQLite and JS decode/check; no UI rendering"};
  })()`);
  report.searchPerformance=performance;
  assert.ok(performance.p95Ms<=300,`Search P95 ${performance.p95Ms.toFixed(2)} ms exceeds 300 ms`);
  report.results.push("1,000 条元数据：中文/路径/模糊匹配、稳定分页、持久排序设置、严格参数与失效游标通过；WebView IPC 搜索 P95 达标");
}
