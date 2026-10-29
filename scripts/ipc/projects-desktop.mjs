import assert from "node:assert/strict";
import { readFile, stat } from "node:fs/promises";
import { join } from "node:path";
import { verifyProjectSearchDesktop } from "./project-search-desktop.mjs";
import { verifyProjectOpenDesktop } from "./project-open-desktop.mjs";

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

  const summary = (await call("project_git_summary", {projectId:id, refresh:true})).data;
  assert.deepEqual({state:summary.state, isRepository:summary.isRepository, branch:summary.branch, detached:summary.detached, workingTreeModified:summary.workingTreeModified}, {
    state:"available", isRepository:true, branch:"s2-06-fixture", detached:false, workingTreeModified:false,
  });
  assert.equal(Number.isSafeInteger(summary.refreshedAt), true);
  assert.equal((await call("project_get",{projectId:id})).data.lastUsedAt,null);
  await fixture("project_git_dirty");
  const cachedBeforeRefresh = (await call("project_git_summary", {projectId:id})).data;
  assert.equal(cachedBeforeRefresh.workingTreeModified,false);
  const dirty = (await call("project_git_summary", {projectId:id, refresh:true})).data;
  assert.equal(dirty.state,"available"); assert.equal(dirty.workingTreeModified,true);
  const cached = (await call("project_git_summary", {projectId:id})).data;
  assert.equal(cached.workingTreeModified,true);
  assert.equal((await call("project_get",{projectId:id})).data.lastUsedAt,null);
  report.results.push("正式 Rust Git IPC 识别分支、干净/已修改工作区和刷新缓存，不写项目使用时间");
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
  assert.equal((await call("project_git_summary", {projectId:id, refresh:"yes"})).error.code,"PA_IPC_INVALID_REQUEST");
  assert.equal((await call("project_git_summary", {projectId:id, unknown:true})).error.code,"PA_IPC_INVALID_REQUEST");
  report.results.push("main 三个项目命令的版本、未知字段、根路径伪造、UUID、大小和修订校验通过");
  await verifyProjectOpenDesktop({call, until, report, fixtureDirectory, id});

  await evaluate("document.querySelector('[data-page=projects].nav-item').click()");
  await until(() => evaluate("document.querySelectorAll('[data-testid=project-row]').length"), count => count >= 1);
  assert.equal(await evaluate("Boolean(document.querySelector('[data-testid=project-add]'))"), true);
  assert.equal(await evaluate("Boolean(document.querySelector('[data-testid=project-sort]'))"), true);
  await evaluate(`(() => { const input=document.querySelector('[data-testid=project-search]'); Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,'no-such-project'); input.dispatchEvent(new Event('input',{bubbles:true})); })()`);
  await until(() => evaluate("document.querySelector('[data-testid=state-empty] h2')?.textContent"), title => title === "没有匹配的项目");
  await evaluate("document.querySelector('[data-testid=state-empty] button').click()");
  await until(() => evaluate("document.querySelectorAll('[data-testid=project-row]').length"), count => count >= 1);
  await evaluate(`(() => { const input=document.querySelector('[data-testid=project-search]'); Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,'中文组合词'); input.dispatchEvent(new CompositionEvent('compositionstart',{bubbles:true,data:'中'})); input.dispatchEvent(new Event('input',{bubbles:true})); })()`);
  await until(() => evaluate("Boolean(document.querySelector('[data-testid=project-ime-status]'))"), Boolean);
  assert.equal(await evaluate("Boolean(document.querySelector('[data-testid=state-empty]'))"), false);
  await evaluate("document.querySelector('[data-testid=project-search]').dispatchEvent(new CompositionEvent('compositionend',{bubbles:true,data:'中文组合词'}))");
  await until(() => evaluate("document.querySelector('[data-testid=state-empty] h2')?.textContent"), title => title === "没有匹配的项目");
  await evaluate("document.querySelector('[data-testid=state-empty] button').click()");
  await until(() => evaluate("document.querySelectorAll('[data-testid=project-row]').length"), count => count >= 1);
  await evaluate("document.querySelector('[data-page=projects].nav-item').click()");
  await until(() => evaluate("document.querySelector('[data-testid=project-add]') && !document.querySelector('[data-testid=project-add]').disabled"), Boolean);
  await evaluate("document.querySelector('[data-testid=project-add]').click()");
  const added = await until(async () => (await call("projects_list",{query:"project-added-through-ui"})).data.items.find(project=>project.name==="project-added-through-ui"), Boolean);
  let stableRows = 0;
  await until(async () => {
    const stable = await evaluate("!document.querySelector('[data-testid=project-add]').disabled && document.querySelectorAll('[data-testid=project-row]').length >= 2 && !document.querySelector('.project-list-heading [role=status]')");
    stableRows = stable ? stableRows + 1 : 0;
    return stableRows >= 3;
  }, Boolean);
  const firstProjectId = await until(() => evaluate("document.querySelector('[data-testid=project-row]')?.id"), Boolean);
  await until(() => evaluate("Boolean(document.querySelector('[role=listbox].project-list'))"), Boolean);
  await evaluate("document.querySelector('[role=listbox].project-list')?.focus()");
  await cdp("Input.dispatchKeyEvent",{type:"rawKeyDown",key:"Home",code:"Home",windowsVirtualKeyCode:36});
  await cdp("Input.dispatchKeyEvent",{type:"keyUp",key:"Home",code:"Home",windowsVirtualKeyCode:36});
  await until(() => evaluate("document.querySelector('[aria-selected=true][data-testid=project-row]')?.id"), selectedId => selectedId === firstProjectId);
  await cdp("Input.dispatchKeyEvent",{type:"rawKeyDown",key:"ArrowDown",code:"ArrowDown",windowsVirtualKeyCode:40});
  await cdp("Input.dispatchKeyEvent",{type:"keyUp",key:"ArrowDown",code:"ArrowDown",windowsVirtualKeyCode:40});
  await until(() => evaluate("document.querySelector('[aria-selected=true][data-testid=project-row]')?.id"), selectedId => selectedId && selectedId !== firstProjectId);
  await evaluate("Array.from(document.querySelectorAll('[data-testid=project-row]')).find(row=>row.innerText.includes('project-added-through-ui')).click()");
  await until(() => evaluate("document.querySelector('[data-testid=project-detail] h3')?.textContent"), name => name === "project-added-through-ui");
  await evaluate("document.querySelector('[data-testid=project-open-directory]').click()");
  await until(async () => (await call("project_get",{projectId:added.id})).data.lastUsedAt, value => value !== null);
  await until(() => evaluate("Boolean(document.querySelector('[data-testid=project-pick-file]') && !document.querySelector('[data-testid=project-pick-file]').disabled)"), Boolean);
  await evaluate("document.querySelector('[data-testid=project-pick-file]').click()");
  await until(() => evaluate("document.querySelector('[data-testid=project-selected-file]')?.innerText.includes('selected-through-ui.txt')"), Boolean);
  await until(() => evaluate("Boolean(document.querySelector('[data-testid=project-open-file]') && !document.querySelector('[data-testid=project-open-file]').disabled)"), Boolean);
  await evaluate("document.querySelector('[data-testid=project-open-file]').click()");
  await until(() => evaluate("document.querySelector('[data-testid=project-action-status]')?.textContent"), message => message === "系统已接受指定文件打开请求。");
  await evaluate("document.querySelector('[data-page=home].nav-item').click()");
  await until(() => evaluate("Array.from(document.querySelectorAll('[data-testid=recent-project-row]')).some(row=>row.innerText.includes('project-added-through-ui'))"), Boolean);
  await evaluate("Array.from(document.querySelectorAll('[data-testid=recent-project-row]')).find(row=>row.innerText.includes('project-added-through-ui')).click()");
  await until(() => evaluate("document.querySelector('[data-testid=project-detail] h3')?.textContent"), name => name === "project-added-through-ui");
  await evaluate("document.querySelector('[data-testid=project-remove]').click()");
  assert.equal(await evaluate("document.querySelector('[data-testid=project-remove]').textContent.trim()"), "确认移除记录");
  await evaluate("document.querySelector('[data-testid=project-remove]').click()");
  await until(async () => (await call("projects_list",{query:"project-added-through-ui"})).data.items.length, count => count === 0);
  assert.equal((await stat(join(fixtureDirectory,"project-added-through-ui"))).isDirectory(), true);
  await evaluate("document.querySelector('[data-page=projects].nav-item').click()");
  await until(() => evaluate("Array.from(document.querySelectorAll('[data-testid=project-row]')).some(row=>row.innerText.includes('synthetic-project-private-path'))"), Boolean);
  await evaluate("Array.from(document.querySelectorAll('[data-testid=project-row]')).find(row=>row.innerText.includes('synthetic-project-private-path')).click()");
  await until(() => evaluate("document.querySelector('[data-testid=project-detail] h3')?.textContent"), name => name === "synthetic-project-private-path");
  await evaluate("Array.from(document.querySelectorAll('[data-testid=project-row]')).find(row=>row.innerText.includes('synthetic-project-private-path')).click()");
  await until(() => evaluate("Boolean(document.querySelector('[data-testid=project-open-directory]'))"), Boolean);
  await evaluate(`(() => { const select=document.querySelector('[data-testid=project-sort]'); Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,'value').set.call(select,'name'); select.dispatchEvent(new Event('change',{bubbles:true})); })()`);
  await until(async () => (await call("settings_get",{scope:"global"})).data.values.find(change=>change.key==="project_sort").value, value => value === "name");
  await evaluate(`(() => { const select=document.querySelector('[data-testid=project-sort]'); Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,'value').set.call(select,'recent'); select.dispatchEvent(new Event('change',{bubbles:true})); })()`);
  await until(async () => (await call("settings_get",{scope:"global"})).data.values.find(change=>change.key==="project_sort").value, value => value === "recent");
  await until(() => evaluate("document.querySelectorAll('[data-testid=project-row]').length"), count => count >= 1);
  await evaluate("document.querySelector('[data-testid=project-row]').click()");
  await until(() => evaluate("Boolean(document.querySelector('[data-testid=project-open-directory]') && !document.querySelector('[data-testid=project-open-directory]').disabled)"), Boolean);
  await evaluate("document.querySelector('[data-testid=project-open-directory]').click()");
  await until(async () => (await call("project_get",{projectId:id})).data.lastUsedAt, value => value !== null);
  await evaluate("document.querySelector('[data-page=home].nav-item').click()");
  await until(() => evaluate("Array.from(document.querySelectorAll('[data-testid=recent-project-row]')).some(row=>row.innerText.includes('synthetic-project-private-path'))"), Boolean);
  await evaluate("Array.from(document.querySelectorAll('[data-testid=recent-project-row]')).find(row=>row.innerText.includes('synthetic-project-private-path')).click()");
  await until(() => evaluate("document.querySelector('[data-testid=project-detail] h3')?.textContent"), name => name === "synthetic-project-private-path");
  await fixture("audit_fail_write");
  await evaluate("document.querySelector('[data-testid=project-remove]').click()");
  assert.equal(await evaluate("document.querySelector('[data-testid=project-remove]').textContent.trim()"), "确认移除记录");
  await evaluate("document.querySelector('[data-testid=project-remove]').click()");
  await until(() => evaluate("document.querySelector('[data-testid=project-action-error] p:first-child')?.textContent"), value => value?.includes("PA_AUDIT_WRITE_FAILED"));
  assert.match(await evaluate("document.querySelector('[data-testid=project-action-error] .error-correlation').textContent"), /ipc-[a-z0-9-]+/);
  assert.equal((await call("project_get",{projectId:id})).data.authorization,"granted");
  await fixture("audit_restore_write");
  report.results.push("真实项目页面完成添加→目录/指定文件打开→首页近期返回→确认移除，源目录保留；键盘选择、IME、搜索、排序、审计失败和关联 ID 通过");

  await fixture("audit_fail_write");
  assert.equal((await call("project_remove",{projectId:id,expectedRevision:1})).error.code,"PA_AUDIT_WRITE_FAILED");
  assert.equal((await call("project_get",{projectId:id})).data.authorization,"granted");
  await fixture("audit_restore_write");
  await fixture("project_missing");
  view=(await call("project_get",{projectId:id})).data;
  assert.equal(view.availability,"missing"); assert.equal(view.authorization,"granted"); assert.equal(view.revision,2);
  await until(() => evaluate("Boolean(document.querySelector('[data-testid=project-path-unavailable]'))"), Boolean);
  for (const command of ["project_open_directory","project_open_file","project_pick_file"]) {
    const params={projectId:id,...(command==="project_open_file"?{relativePath:"keep.txt"}:{})};
    assert.equal((await call(command,params)).error?.code,"PA_PROJECT_PATH_UNAVAILABLE");
  }
  assert.equal((await call("project_remove",{projectId:id,expectedRevision:1})).error.code,"PA_PROJECT_CONFLICT_REVISION");
  assert.deepEqual((await call("project_remove",{projectId:id,expectedRevision:2})).data,{removed:true,retainedForHistory:true});
  assert.equal((await call("project_get",{projectId:id})).error.code,"PA_PROJECT_NOT_FOUND");
  assert.equal(await readFile(join(fixtureDirectory,"moved-project","keep.txt"),"utf8"),"source-sentinel");
  report.results.push("真实 IPC 审计故障回滚、路径失效保留、修订冲突及撤权移除通过；源文件保持不变");
  await verifyProjectSearchDesktop({evaluate, call, fixture, report});
  for (const [query, create, close] of [["window=overlay",null,null],["window=denied","create_denied_window","close_denied_window"]]) {
    if(create) await fixture(create);
    const target=await until(async()=> (await (await fetch(`http://127.0.0.1:${debugPort}/json/list`)).json()).find(v=>v.url.includes(query)),Boolean);
    const socket=new WebSocket(target.webSocketDebuggerUrl);
    try {
      await new Promise((done,reject)=>{socket.addEventListener("open",done,{once:true});socket.addEventListener("error",reject,{once:true});});socket.addEventListener("message",receive);
      await until(()=>evaluate("Boolean(window.__TAURI_INTERNALS__)",socket),Boolean);
      for(const command of ["projects_add_from_picker","projects_list","project_get","project_git_summary","project_remove","project_open_directory","project_open_file","project_pick_file"]) assert.equal(await evaluate(`${invoke(command,{projectId:id,expectedRevision:2})}.then(()=>false,()=>true)`,socket),true);
    } finally {socket.close();}
    if(close) await fixture(close);
  }
  await fixture("navigate_remote");
  await until(()=>evaluate("document.title"),title=>title==="IPC remote fixture");
  for(const command of ["projects_add_from_picker","projects_list","project_get","project_git_summary","project_remove","project_open_directory","project_open_file","project_pick_file"]) assert.equal(await evaluate(`${invoke(command,{projectId:id})}.then(()=>false,()=>true)`),true);
  await cdp("Page.navigate",{url:mainUrl}); await until(()=>evaluate("Boolean(document.querySelector('.app-shell'))"),Boolean);
  report.results.push("overlay、未知本地窗口、远端 main 的项目 capability 拒绝通过；未自动打开原生选择器");
}
