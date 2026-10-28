import assert from "node:assert/strict";
import { readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

export async function verifyProjectOpenDesktop({call, until, report, fixtureDirectory, id}) {
  const source = join(fixtureDirectory,"synthetic-project-private-path");
  const relativePath = `文本 & (指定)-${id}.txt`;
  await writeFile(join(source,relativePath),"system-open-source-sentinel");
  const get = async () => (await call("project_get",{projectId:id})).data;
  assert.equal((await get()).lastUsedAt,null);
  for (const [command,params,code] of [
    ["project_open_directory",{projectId:id,absolutePath:"synthetic-private-input"},"PA_IPC_INVALID_REQUEST"],
    ["project_pick_file",{projectId:id,url:"https://invalid.example"},"PA_IPC_INVALID_REQUEST"],
    ["project_open_file",{projectId:id,relativePath,program:"cmd.exe"},"PA_IPC_INVALID_REQUEST"],
    ["project_open_directory",{projectId:"c14e7b09-b84c-44bc-9b57-a7182e70b056"},"PA_PROJECT_NOT_FOUND"],
    ["project_open_file",{projectId:id,relativePath:"../outside.txt"},"PA_PROJECT_FILE_OUTSIDE_PROJECT"],
    ["project_open_file",{projectId:id,relativePath:"C:\\outside.txt"},"PA_PROJECT_FILE_OUTSIDE_PROJECT"],
    ["project_open_file",{projectId:id,relativePath:"keep.txt:secret"},"PA_PROJECT_FILE_OUTSIDE_PROJECT"],
    ["project_open_file",{projectId:id,relativePath:"COM¹.txt"},"PA_PROJECT_FILE_OUTSIDE_PROJECT"],
    ["project_open_file",{projectId:id,relativePath:"missing.txt"},"PA_PROJECT_FILE_MISSING"],
    ["project_open_file",{projectId:id,relativePath:"absent/missing.txt"},"PA_PROJECT_FILE_MISSING"],
  ]) assert.equal((await call(command,params)).error?.code,code);
  assert.equal((await get()).lastUsedAt,null);
  assert.deepEqual((await call("project_open_directory",{projectId:id})).data,{systemAccepted:true});
  assert.ok((await get()).lastUsedAt > 0);
  assert.deepEqual((await call("project_open_file",{projectId:id,relativePath})).data,{systemAccepted:true});
  report.results.push("S2-05：真实 Windows 默认程序接受授权目录和中文/空格/特殊字符文本文件；成功才更新使用时间");

  const beforeFailures=(await get()).lastUsedAt;
  await writeFile(join(source,"redirect.url"),"[InternetShortcut]\r\nURL=https://invalid.example\r\n");
  assert.equal((await call("project_open_file",{projectId:id,relativePath:"redirect.url"})).error?.code,"PA_PROJECT_OPEN_REJECTED");
  const unassociated=`unknown.${id}`;
  await writeFile(join(source,unassociated),"synthetic-unassociated");
  assert.equal((await call("project_open_file",{projectId:id,relativePath:unassociated})).error?.code,"PA_PROJECT_OPEN_REJECTED");

  const saveEditor=async executable=> {
    const settings=(await call("settings_get",{scope:"global"})).data;
    assert.equal((await call("settings_update",{scope:"global",expectedRevision:settings.revision,changes:[{key:"editor",value:executable?{id:"custom",executable}:null}]})).status,"ok");
  };
  const invalid=join(fixtureDirectory,"invalid-editor.exe");
  await writeFile(invalid,"invalid executable");
  await saveEditor(invalid);
  assert.equal((await call("project_open_file",{projectId:id,relativePath})).error?.code,"PA_PROJECT_OPEN_REJECTED");
  await saveEditor(join(fixtureDirectory,"missing-editor.exe"));
  assert.equal((await call("project_open_file",{projectId:id,relativePath})).error?.code,"PA_PROJECT_OPEN_REJECTED");
  assert.equal((await get()).lastUsedAt,beforeFailures);

  // A short-lived real PE executable records only a fixed receipt. It verifies
  // one literal argument and exits; no shell or GUI process is impersonated.
  const helper=join(fixtureDirectory,"editor-probe.exe");
  const helperSource=join(fixtureDirectory,"editor-probe.rs");
  await writeFile(helperSource,`fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    assert_eq!(args.len(), 2);
    let path = std::path::Path::new(&args[1]);
    assert_eq!(path.file_name().unwrap().to_str().unwrap(), ${JSON.stringify(relativePath)});
    assert!(path.is_absolute());
    std::fs::write(path.parent().unwrap().join("editor-receipt"), "literal-argument-accepted").unwrap();
  }`);
  const compiled=spawnSync("rustc",["--edition=2024",helperSource,"-o",helper],{windowsHide:true,encoding:"utf8",timeout:60000});
  assert.equal(compiled.status,0,"Synthetic editor helper must compile");
  await saveEditor(helper);
  assert.deepEqual((await call("project_open_file",{projectId:id,relativePath})).data,{systemAccepted:true});
  await until(async()=>readFile(join(source,"editor-receipt"),"utf8").catch(()=>""),value=>value==="literal-argument-accepted");
  await saveEditor(null);
  assert.equal(await readFile(join(source,relativePath),"utf8"),"system-open-source-sentinel");
  report.results.push("S2-05：无默认关联、Shell 转向文件和失效编辑器固定失败且不增加使用；真实配置编辑器收到单一字面参数并退出");
}
