import { useEffect, useRef, useState } from "react";
import { call, controlOverlay } from "../ipc/client";
import { useAppStatus } from "../app/useAppStatus";
import type { SettingChange, SettingsView } from "../ipc/contracts";
import { ErrorNotice } from "../components/ErrorNotice";
import { LoggingPanel } from "../components/LoggingPanel";
import { errorForCode, toPublicError } from "../ipc/errors";
import type { PublicError } from "../ipc/errors";

type Values = { search: "bing" | "google"; sort: "recent" | "name"; log: "error" | "warn" | "info" | "debug"; days: number; editor: string; enabled: boolean; shortcut: string };
function values(view: SettingsView): Values {
  const get = (key: SettingChange["key"]) => view.values.find(c => c.key === key)!.value;
  return { search: get("search_engine") as Values["search"], sort: get("project_sort") as Values["sort"], log: get("log_level") as Values["log"], days: get("summary_retention_days") as number, editor: (get("editor") as { executable: string } | null)?.executable ?? "", enabled: get("overlay_enabled") as boolean, shortcut: get("overlay_shortcut") as string };
}

export function SettingsPage() {
  const { status } = useAppStatus();
  const [view, setView] = useState<SettingsView | null>(null);
  const [draft, setDraft] = useState<Values | null>(null);
  const [phase, setPhase] = useState<"reading" | "ready" | "editing" | "saving" | "success" | "failure">("reading");
  const [error, setError] = useState<PublicError | null>(null);
  const [conflict, setConflict] = useState(false);
  const phaseRef = useRef(phase); phaseRef.current = phase;
  const mounted = useRef(false);
  const requestId = useRef(0);
  async function read() {
    const id = ++requestId.current;
    phaseRef.current = "reading";
    setPhase("reading"); setError(null);
    try {
      const next = await call("settings_get", { scope: "global" });
      if (!mounted.current || id !== requestId.current) return;
      setView(next); setDraft(values(next)); setConflict(false); setPhase("ready");
    } catch (cause) {
      if (mounted.current && id === requestId.current) { setError(toPublicError(cause)); setPhase("failure"); }
    }
  }
  useEffect(() => { mounted.current = true; void read(); return () => { mounted.current = false; ++requestId.current; }; }, []);
  // Clean forms follow tray/window changes. Edited forms retain their revision and
  // draft; a conflict requires an explicit reread and a new user submission.
  useEffect(() => {
    if (phaseRef.current === "ready" || phaseRef.current === "success") void read();
  }, [status?.streamId, status?.sequence]);
  function edit<K extends keyof Values>(key: K, value: Values[K]) {
    if (draft) { phaseRef.current = "editing"; setDraft({ ...draft, [key]: value }); setPhase("editing"); setError(null); }
  }
  async function save() {
    if (!view || !draft || conflict) return;
    phaseRef.current = "saving";
    setPhase("saving"); setError(null);
    const all: SettingChange[] = [
      { key: "search_engine", value: draft.search }, { key: "project_sort", value: draft.sort },
      { key: "approval_policy", value: "ask" }, { key: "log_level", value: draft.log },
      { key: "summary_retention_days", value: draft.days },
      { key: "editor", value: draft.editor.trim() ? { id: "custom", executable: draft.editor.trim() } : null },
      { key: "overlay_enabled", value: draft.enabled }, { key: "overlay_shortcut", value: draft.shortcut },
    ];
    const changes = all.filter(c => JSON.stringify(c.value) !== JSON.stringify(view.values.find(v => v.key === c.key)?.value));
    if (!changes.length) { setPhase("ready"); return; }
    try {
      const next = await call("settings_update", { scope: "global", expectedRevision: view.revision, changes });
      if (!mounted.current) return;
      setView(next); setDraft(values(next)); setPhase("success");
    } catch (cause) {
      if (!mounted.current) return;
      const safe = toPublicError(cause);
      setError(safe); setConflict(safe.code === "PA_CONFLICT_REVISION"); setPhase("failure");
    }
  }
  const busy = phase === "reading" || phase === "saving";
  return <div className="page-content">
    <p role="status" data-testid="settings-status" data-phase={phase}>{({ reading: "正在读取设置…", ready: "已读取设置", editing: "有未保存的修改", saving: "正在保存…", success: "设置已保存", failure: "设置操作失败" })[phase]}{view && ` · 修订 ${view.revision}`}</p>
    {error && <ErrorNotice error={error} testId="settings-error" />}
    <button type="button" className="quiet-button" disabled={busy} onClick={() => void read()}>重新读取设置</button>
    {conflict && <p>重新读取会加载最新值并丢弃当前草稿；请确认后重新编辑、提交。</p>}
    {draft && <form className="settings-form" onSubmit={event => { event.preventDefault(); void save(); }}>
      <fieldset disabled={busy || conflict}><legend>常规设置</legend>
        <label htmlFor="search-engine">默认搜索引擎</label><select id="search-engine" value={draft.search} onChange={e => edit("search", e.target.value as Values["search"])}><option value="bing">Bing</option><option value="google">Google</option></select>
        <label htmlFor="project-sort">项目排序</label><select id="project-sort" value={draft.sort} onChange={e => edit("sort", e.target.value as Values["sort"])}><option value="recent">最近使用</option><option value="name">名称</option></select>
        <label htmlFor="approval-policy">审批策略</label><select id="approval-policy" value="ask" disabled><option value="ask">操作前询问</option></select>
        <label htmlFor="log-level">日志级别</label><select id="log-level" value={draft.log} onChange={e => edit("log", e.target.value as Values["log"])}>{["error", "warn", "info", "debug"].map(v => <option key={v}>{v}</option>)}</select>
        <label htmlFor="summary-days">摘要留存期限（天）</label><input id="summary-days" type="number" min="1" max="365" step="1" value={draft.days} onChange={e => edit("days", Number(e.target.value))}/><p id="summary-help">摘要采集保持关闭；保存期限不会开启采集。</p>
        <label htmlFor="editor-path">编辑器可执行文件绝对路径（留空表示未配置）</label><input id="editor-path" maxLength={1024} value={draft.editor} onChange={e => edit("editor", e.target.value)}/>
      </fieldset>
      <fieldset className="resident-settings" disabled={busy || conflict}><legend>常驻对话入口</legend>
        <label htmlFor="resident-enabled"><input id="resident-enabled" type="checkbox" checked={draft.enabled} onChange={e => edit("enabled", e.target.checked)}/> 启用常驻入口</label>
        <label htmlFor="resident-shortcut">全局快捷键</label><input id="resident-shortcut" maxLength={80} value={draft.shortcut} onChange={e => edit("shortcut", e.target.value)}/>
        <p>停用后重启保持关闭；启用后重启显示悬浮球。位置和显示器随拖动自动保存，草稿仅保留在本次运行。</p>
        <p data-testid="resident-position">{(() => { const p = view?.values.find(c => c.key === "overlay_position")?.value as { x: number; y: number; monitor: string | null } | null; return p ? `位置：${p.x}, ${p.y} · 显示器：${p.monitor ?? "主显示器"}` : "位置：首次使用主显示器"; })()}</p>
        <button type="button" className="quiet-button" disabled={!status?.overlay.enabled} onClick={() => { void controlOverlay("expand").catch(cause => setError(toPublicError(cause))); }}>展开对话</button>
      </fieldset>
      <button type="submit" className="primary-button" data-testid="settings-save" disabled={busy || conflict || phase === "ready" || phase === "success"}>保存设置</button>
    </form>}
    {status?.overlay.shortcutError && <ErrorNotice error={errorForCode(status.overlay.shortcutError)} testId="shortcut-error" />}
    <LoggingPanel allowClear streamId={status?.streamId} sequence={status?.sequence} />
    <p>AI 默认选择请在 AI 配置页中测试并应用；设置页不会直接修改服务源或凭据。</p>
  </div>;
}
