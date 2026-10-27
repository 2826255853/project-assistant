import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { join } from "node:path";

export async function verifyProjectsDesktop({evaluate, until, report, debugPort, receive, cdp, mainUrl, fixtureDirectory}) {
  const invoke = (command, params = {}, apiVersion = 1) => `window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)}, {request:${JSON.stringify({apiVersion, params})}})`;
  const call = async (command, params, version) => evaluate(invoke(command, params, version));
  const fixture = async action => assert.equal((await call("ipc_fixture", {action})).status, "ok");
  await until(() => evaluate("document.querySelector('.main-area')?.dataset.page"), page => page === "home");
  await fixture("project_seed");
  const id = await readFile(join(fixtureDirectory, "project-fixture-id"), "utf8");
  let view = (await call("project_get", {projectId:id})).data;
  assert.equal(view.authorization, "granted"); assert.equal(view.lastUsedAt, null);
  assert.equal(view.availability, "available"); assert.equal(view.revision, 1);
  assert.equal("directoryIdentity" in view, false);
  // No path-containing screenshots, responses, logs or report fields are saved.
  for (const [command, params, version, code] of [
    ["projects_add_from_picker", {absolutePath:"synthetic-private-input"}, 1, "PA_IPC_INVALID_REQUEST"],
    ["projects_add_from_picker", {}, 2, "PA_IPC_VERSION_UNSUPPORTED"],
    ["project_get", {projectId:id, authorization:"granted"}, 1, "PA_IPC_INVALID_REQUEST"],
    ["project_get", {projectId:"not-a-uuid"}, 1, "PA_IPC_INVALID_REQUEST"],
    ["project_get", {projectId:"c14e7b09-b84c-44bc-9b57-a7182e70b056"}, 1, "PA_PROJECT_NOT_FOUND"],
    ["project_remove", {projectId:id, expectedRevision:2}, 1, "PA_PROJECT_CONFLICT_REVISION"],
    ["project_remove", {projectId:id, expectedRevision:0}, 1, "PA_IPC_INVALID_REQUEST"],
    ["project_remove", {projectId:id, expectedRevision:1, unknown:"x".repeat(8192)}, 1, "PA_VALIDATION_INPUT_TOO_LARGE"],
  ]) assert.equal((await call(command,params,version)).error.code,code);
  report.results.push("main 三个项目命令的版本、未知字段、根路径伪造、UUID、大小和修订校验通过");
  await fixture("audit_fail_write");
  assert.equal((await call("project_remove",{projectId:id,expectedRevision:1})).error.code,"PA_AUDIT_WRITE_FAILED");
  assert.equal((await call("project_get",{projectId:id})).data.authorization,"granted");
  await fixture("audit_restore_write");
  await fixture("project_missing");
  view=(await call("project_get",{projectId:id})).data;
  assert.equal(view.availability,"missing"); assert.equal(view.authorization,"granted"); assert.equal(view.revision,2);
  assert.equal((await call("project_remove",{projectId:id,expectedRevision:1})).error.code,"PA_PROJECT_CONFLICT_REVISION");
  assert.deepEqual((await call("project_remove",{projectId:id,expectedRevision:2})).data,{removed:true,retainedForHistory:true});
  assert.equal((await call("project_get",{projectId:id})).error.code,"PA_PROJECT_NOT_FOUND");
  assert.equal(await readFile(join(fixtureDirectory,"moved-project","keep.txt"),"utf8"),"source-sentinel");
  report.results.push("真实 IPC 审计故障回滚、路径失效保留、修订冲突及撤权移除通过；源文件保持不变");
  for (const [query, create, close] of [["window=overlay",null,null],["window=denied","create_denied_window","close_denied_window"]]) {
    if(create) await fixture(create);
    const target=await until(async()=> (await (await fetch(`http://127.0.0.1:${debugPort}/json/list`)).json()).find(v=>v.url.includes(query)),Boolean);
    const socket=new WebSocket(target.webSocketDebuggerUrl);
    try {
      await new Promise((done,reject)=>{socket.addEventListener("open",done,{once:true});socket.addEventListener("error",reject,{once:true});});socket.addEventListener("message",receive);
      await until(()=>evaluate("Boolean(window.__TAURI_INTERNALS__)",socket),Boolean);
      for(const command of ["projects_add_from_picker","project_get","project_remove"]) assert.equal(await evaluate(`${invoke(command,{projectId:id,expectedRevision:2})}.then(()=>false,()=>true)`,socket),true);
    } finally {socket.close();}
    if(close) await fixture(close);
  }
  await fixture("navigate_remote");
  await until(()=>evaluate("document.title"),title=>title==="IPC remote fixture");
  for(const command of ["projects_add_from_picker","project_get","project_remove"]) assert.equal(await evaluate(`${invoke(command,{projectId:id})}.then(()=>false,()=>true)`),true);
  await cdp("Page.navigate",{url:mainUrl}); await until(()=>evaluate("Boolean(document.querySelector('.app-shell'))"),Boolean);
  report.results.push("overlay、未知本地窗口、远端 main 的项目 capability 拒绝通过；未自动打开原生选择器");
}
