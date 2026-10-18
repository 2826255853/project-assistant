import assert from "node:assert/strict";
import { writeFile } from "node:fs/promises";
import { resolve } from "node:path";

export async function verifySettingsDesktop({ evaluate, until, report, scenario, cdp, output }) {
  const invoke = (command, params = {}, apiVersion = 1) => `window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)}, {request:${JSON.stringify({apiVersion,params})}})`;
  const get = async () => {
    const response = await evaluate(invoke("settings_get", {scope:"global"}));
    assert.equal(response.status,"ok",JSON.stringify(response)); return response.data;
  };
  const value = (view,key) => view.values.find(c=>c.key===key)?.value;
  const update = async changes => {
    const current = await get();
    const result = await evaluate(invoke("settings_update",{scope:"global",expectedRevision:current.revision,changes}));
    assert.equal(result.status,"ok",JSON.stringify(result)); return result.data;
  };
  const fixture = async action => assert.equal((await evaluate(invoke("ipc_fixture",{action}))).status,"ok");
  async function edit(selector, inputValue) {
    await evaluate(`(() => { const el=document.querySelector(${JSON.stringify(selector)}); const proto=el.tagName==='SELECT'?HTMLSelectElement.prototype:HTMLInputElement.prototype; Object.getOwnPropertyDescriptor(proto,'value').set.call(el,${JSON.stringify(inputValue)}); el.dispatchEvent(new Event(el.tagName==='SELECT'?'change':'input',{bubbles:true})); })()`);
    await until(()=>evaluate("document.querySelector('[data-testid=settings-status]')?.dataset.phase"),v=>v==="editing");
  }
  await until(()=>evaluate("document.querySelector('.main-area')?.dataset.page"),v=>v==="home");
  await evaluate("document.querySelector('.nav-item[data-page=settings]').click()");
  await until(()=>evaluate("document.querySelector('[data-testid=settings-status]')?.dataset.phase"),v=>v==="ready");
  const initial = await get();
  const status = await evaluate(invoke("app_get_status"));
  assert.equal(status.data.services.find(s=>s.id==="settings").state,"ready");
  assert.equal(status.data.overlay.expanded,false);
  assert.equal(status.data.overlay.trayReady,true);
  report.results.push("真实 WebView2 设置页读回服务状态，启动收起为悬浮球，系统托盘创建成功");
  if (scenario === "settings-write") {
    assert.equal(value(initial,"search_engine"),"bing");
    assert.equal(value(initial,"summary_retention_days"),30);
    assert.equal(value(initial,"editor"),null);
    assert.equal(value(initial,"ai_defaults"),null);
    assert.equal(value(initial,"overlay_enabled"),true);
    for (const [params,version,code] of [
      [{scope:"global",expectedRevision:initial.revision,changes:[{key:"api_key",value:"synthetic-secret-DO-NOT-LOG"}]},1,"PA_IPC_INVALID_REQUEST"],
      [{scope:"global",expectedRevision:initial.revision,changes:[{key:"search_engine",value:"google"},{key:"summary_retention_days",value:0}]},1,"PA_SETTINGS_INVALID_VALUE"],
      [{scope:"global",expectedRevision:initial.revision,changes:[{key:"approval_policy",value:"never"}]},1,"PA_SETTINGS_INVALID_VALUE"],
      [{scope:"global",expectedRevision:initial.revision,changes:[{key:"editor",value:{id:"custom",executable:"cmd.exe /c synthetic-secret"}}]},1,"PA_SETTINGS_INVALID_VALUE"],
      [{scope:"global",expectedRevision:initial.revision,changes:[{key:"log_level",value:"debug"}]},2,"PA_IPC_VERSION_UNSUPPORTED"],
    ]) {
      const result=await evaluate(invoke("settings_update",params,version));
      assert.equal(result.error.code,code); assert.equal(JSON.stringify(result).includes("synthetic-secret"),false);
    }
    assert.deepEqual(await get(),initial);
    assert.equal((await evaluate(invoke("settings_get",{scope:"project",projectId:"unregistered"}))).error.code,"PA_IPC_PERMISSION_DENIED");
    report.results.push("真实设置 IPC 默认值、非法键/值、类型/版本、整批拒绝和未授权项目范围通过");

    await edit("#search-engine","google");
    await edit("#summary-days","60");
    await edit("#resident-shortcut","Ctrl+Alt+F9");
    await evaluate("document.querySelector('[data-testid=settings-save]').click()");
    await until(()=>evaluate("document.querySelector('[data-testid=settings-status]')?.dataset.phase"),v=>v==="success" || v==="ready");
    assert.equal(value(await get(),"search_engine"),"google");
    assert.equal(value(await get(),"overlay_shortcut"),"Ctrl+Alt+F9");
    report.results.push("设置表单编辑、保存和服务返回值刷新通过");

    await edit("#project-sort","name");
    await fixture("tray_disable");
    await until(()=>evaluate(invoke("overlay_get")),r=>r.data.enabled===false);
    await evaluate("document.querySelector('[data-testid=settings-save]').click()");
    await until(()=>evaluate("document.querySelector('[data-testid=settings-error]')?.textContent"),v=>v?.includes("PA_CONFLICT_REVISION"));
    assert.equal(value(await get(),"project_sort"),"recent");
    assert.equal(await evaluate("document.querySelector('[data-testid=settings-save]').disabled"),true);
    await evaluate("[...document.querySelectorAll('button')].find(b=>b.textContent==='重新读取设置').click()");
    await until(()=>evaluate("document.querySelector('[data-testid=settings-status]')?.dataset.phase"),v=>v==="ready");
    assert.equal(await evaluate("document.querySelector('#resident-enabled').checked"),false);
    await fixture("tray_enable");
    await until(()=>evaluate("document.querySelector('#resident-enabled').checked"),v=>v===true);
    report.results.push("调用真实托盘菜单回调验证双向设置同步；编辑期间并发冲突要求重读再提交，不自动覆盖");

    const beforeConflict = await get();
    const conflict = await evaluate(invoke("settings_update",{scope:"global",expectedRevision:beforeConflict.revision,changes:[{key:"search_engine",value:"bing"},{key:"overlay_shortcut",value:"Ctrl+Alt+Delete"}]}));
    assert.equal(conflict.error.code,"PA_SHORTCUT_CONFLICT");
    assert.deepEqual(await get(),beforeConflict);
    assert.equal((await evaluate(invoke("overlay_get"))).data.shortcut,"Ctrl+Alt+F9");
    report.results.push("Windows 保留快捷键注册冲突使整个批次回滚，旧快捷键和普通设置保留");

    await fixture("settings_fail_write");
    const beforeFailure = await get();
    const failed = await evaluate(invoke("settings_update",{scope:"global",expectedRevision:beforeFailure.revision,changes:[{key:"search_engine",value:"bing"},{key:"overlay_enabled",value:false}]}));
    assert.equal(failed.error.code,"PA_SETTINGS_STORAGE_FAILED");
    assert.deepEqual(await get(),beforeFailure);
    assert.equal((await evaluate(invoke("overlay_get"))).data.enabled,true);
    await fixture("settings_restore_write");
    report.results.push("SQL 写入故障保留整批旧值和运行中的常驻开关，无秘密错误泄漏");

    // Monitor name is deliberately absent from the real current monitor list.
    await update([{key:"overlay_position",value:{x:50000,y:50000,monitor:"synthetic-removed-monitor",scaleFactor:2}}]);
    await until(async()=>value(await get(),"overlay_position"),p=>p?.x<50000 && p?.monitor!=="synthetic-removed-monitor");
    const monitorPosition=value(await get(),"overlay_position");
    await update([{key:"overlay_position",value:{...monitorPosition,x:160,y:180}}]);
    await until(async()=>value(await get(),"overlay_position"),p=>p?.x===160 && p?.y===180);
    await update([{key:"overlay_enabled",value:false}]);
    assert.equal(value(await get(),"summary_retention_days"),60);
    assert.equal(value(await get(),"ai_defaults"),null);
    const screenshot=await cdp("Page.captureScreenshot",{format:"png"});
    await writeFile(resolve(output,"settings.png"),Buffer.from(screenshot.data,"base64"));
    await writeFile(resolve(process.env.PA_SETTINGS_VERIFY_DIR,"expected.json"),JSON.stringify(await get()));
    report.results.push("注入已移除显示器及越界坐标，真实窗口回到当前可见工作区；保存停用用于下一独立进程恢复");
  } else if (scenario === "settings-read") {
    const { readFile }=await import("node:fs/promises");
    const expected=JSON.parse(await readFile(resolve(process.env.PA_SETTINGS_VERIFY_DIR,"expected.json"),"utf8"));
    assert.equal(value(initial,"search_engine"),value(expected,"search_engine"));
    assert.equal(value(initial,"summary_retention_days"),value(expected,"summary_retention_days"));
    assert.equal(value(initial,"overlay_enabled"),value(expected,"overlay_enabled"));
    assert.equal(value(initial,"overlay_shortcut"),value(expected,"overlay_shortcut"));
    const initialPosition=value(initial,"overlay_position");
    const expectedPosition=value(expected,"overlay_position");
    assert.equal(initialPosition?.monitor,expectedPosition?.monitor);
    assert.equal(initialPosition?.y,expectedPosition?.y);
    assert.equal(initialPosition?.x,expectedPosition?.x);
    assert.equal(status.data.overlay.enabled,false);
    assert.equal(status.data.overlay.shortcut,"Ctrl+Alt+F9");
    await update([{key:"overlay_enabled",value:true}]);
    await evaluate(invoke("overlay_control",{action:"expand"}));
    report.results.push("明确退出后的独立进程读回全部设置/停用/快捷键/位置/显示器；启用并展开后再退出");
  } else {
    assert.equal(value(initial,"overlay_enabled"),true);
    assert.equal(status.data.overlay.enabled,true);
    assert.equal(status.data.overlay.expanded,false);
    assert.equal(status.data.overlay.shortcut,"Ctrl+Alt+F9");
    assert.equal(status.data.overlay.draft,"");
    report.results.push("第三个独立进程恢复启用设置，启动为悬浮球而非面板，不持久化草稿");
  }
}
