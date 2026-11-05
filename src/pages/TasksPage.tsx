import { useEffect, useRef, useState } from "react";
import { useAppStatus } from "../app/useAppStatus";
import { ErrorNotice } from "../components/ErrorNotice";
import { call } from "../ipc/client";
import type { AiSelection, ModelCapability, ProjectSettingsView, ProjectView, ProviderProfile } from "../ipc/contracts";
import type { ApprovalView, SessionView, TaskChange, TaskDiff, TaskStatus, TaskView } from "../ipc/taskContracts";
import { errorForCode, toPublicError, type PublicError } from "../ipc/errors";
import "./TasksPage.css";

const labels: Record<TaskStatus, string> = { preparing: "准备 / 排队中", running: "运行中", approval_required: "等待审批", cancelling: "取消中", completed: "成功", failed: "失败", cancelled: "已取消", interrupted: "已中断" };
const ended = (task: TaskView) => ["completed", "failed", "cancelled", "interrupted"].includes(task.status);
const sessionLabel = (session: SessionView) => ({ active: "活动", closed: "已结束", interrupted: "已中断" })[session.status];
type Pending = { kind: "session" | "task"; projectId: string; sessionId: string; clientRequestId: string };
type ResultView = { task: TaskView; reply: string; progress: string; available: boolean; approvals: ApprovalView[]; changes: TaskChange[] };

export function TasksPage({ onProjects, onConfigure }: { onProjects: () => void; onConfigure: () => void }) {
  const { status } = useAppStatus();
  const [projects, setProjects] = useState<ProjectView[]>([]);
  const [projectCursor, setProjectCursor] = useState<string | null>(null);
  const [projectId, setProjectId] = useState("");
  const [project, setProject] = useState<ProjectView | null>(null);
  const [settings, setSettings] = useState<ProjectSettingsView | null>(null);
  const [profiles, setProfiles] = useState<ProviderProfile[]>([]);
  const [sessions, setSessions] = useState<SessionView[]>([]);
  const [sessionId, setSessionId] = useState("");
  const [sessionCursor, setSessionCursor] = useState<string | null>(null);
  const [tasks, setTasks] = useState<TaskView[]>([]);
  const [taskCursor, setTaskCursor] = useState<string | null>(null);
  const [taskId, setTaskId] = useState("");
  const [result, setResult] = useState<ResultView | null>(null);
  const [diff, setDiff] = useState<TaskDiff | null>(null);
  const [override, setOverride] = useState(false);
  const [selection, setSelection] = useState<AiSelection | null>(null);
  const [capabilities, setCapabilities] = useState<ModelCapability[]>([]);
  const [prompt, setPrompt] = useState("");
  const [confirmed, setConfirmed] = useState(false);
  const [readError, setReadError] = useState<PublicError | null>(null);
  const [error, setError] = useState<PublicError | null>(null);
  const [notice, setNotice] = useState("正在读取项目与会话…");
  const [busy, setBusy] = useState(false);
  const [pending, setPending] = useState<Pending | null>(null);
  const [refresh, setRefresh] = useState(0);
  const mutating = useRef(false);
  const generation = useRef(0);
  const composing = useRef(false);
  const capabilityRequest = useRef(0);
  const currentTarget = useRef("");
  currentTarget.current = `${projectId}:${sessionId}:${taskId}`;
  const eventCursor = useRef<string | undefined>(undefined);
  const session = sessions.find(s => s.id === sessionId) ?? null;
  const inherited = settings?.values.find(v => v.key === "ai_defaults");
  const effective = override ? selection : inherited?.key === "ai_defaults" ? inherited.value : null;
  const profile = profiles.find(p => p.id === effective?.profileId);
  const source = override ? "本次任务" : inherited?.source === "project" ? "项目默认" : "全局默认";
  const validConfig = Boolean(effective && profile?.enabled && profile.available && profile.configRevision === effective.expectedRevision &&
    (!override || capabilities.some(c => c.modelId === effective.modelId && c.profileRevision === effective.expectedRevision && c.expiresAt > Date.now() &&
      (effective.effort === null ? c.supportsDefaultEffort : c.efforts.includes(effective.effort)))));
  const taskService = status?.services.find(s => s.id === "tasks");
  const allowed = project?.authorization === "granted" && project.availability === "available";
  const promptBytes = new TextEncoder().encode(prompt).length;
  const canSend = taskService?.state === "ready" && allowed && validConfig && confirmed && session?.status === "active" && Boolean(prompt.trim()) && promptBytes <= 65536 && !busy && !pending && !tasks.some(t => !ended(t));

  useEffect(() => { setConfirmed(false); }, [projectId, sessionId, override, effective?.profileId, effective?.expectedRevision, effective?.modelId, effective?.effort, settings?.revision, settings?.globalRevision, project?.revision]);
  useEffect(() => {
    let disposed = false;
    void Promise.all([call("projects_list", { limit: 100 }), call("providers_list", {})]).then(([p, ai]) => {
      if (disposed) return;
      setProjects(p.items); setProjectCursor(p.nextCursor); setProfiles(ai.profiles);
      setProjectId(old => old || status?.activeProject?.projectId || p.items[0]?.id || "");
      if (!p.items.length) setNotice("还没有登记项目，请先登记并授权项目目录。");
    }).catch(cause => { if (!disposed) setError(toPublicError(cause)); });
    return () => { disposed = true; };
  }, [refresh, status?.streamId, status?.sequence]);

  useEffect(() => {
    let disposed = false;
    if (!projectId) return;
    void Promise.all([call("project_get", { projectId }), call("settings_get", { scope: "project", projectId }), sessionId ? call("session_get", { sessionId }) : Promise.resolve(null)]).then(([p, s, current]) => {
      if (disposed) return;
      setProject(p); setSettings(s);
      if (current?.projectId === projectId) setSessions(previous => previous.map(item => item.id === current.id ? current : item));
    }).catch(cause => {
      if (disposed) return;
      setProject(null); setSettings(null); setResult(null); setDiff(null); setConfirmed(false); setError(toPublicError(cause));
    });
    return () => { disposed = true; };
  }, [projectId, sessionId, status?.streamId, status?.sequence]);

  useEffect(() => {
    const id = ++generation.current;
    setProject(null); setSettings(null); setResult(null); setDiff(null); setSessions([]); setTasks([]);
    if (!projectId) return;
    void Promise.all([call("project_get", { projectId }), call("settings_get", { scope: "project", projectId }), call("sessions_list", { projectId, limit: 50 })]).then(async ([p, s, page]) => {
      if (sessionId && !page.items.some(v => v.id === sessionId)) {
        const selected = await call("session_get", { sessionId });
        if (selected.projectId === projectId) page.items.push(selected);
      }
      if (id !== generation.current) return;
      setProject(p); setSettings(s); setSessions(page.items); setSessionCursor(page.nextCursor);
      setSessionId(old => page.items.some(v => v.id === old) ? old : page.items[0]?.id ?? "");
      setNotice(page.items.length ? "会话已读取。任务状态以本地服务为准。" : "还没有会话，请新建会话。");
    }).catch(cause => { if (id === generation.current) setError(toPublicError(cause)); });
    return () => { ++generation.current; };
  }, [projectId, refresh]);

  useEffect(() => {
    let disposed = false;
    setTasks([]); setResult(null); setDiff(null); eventCursor.current = undefined;
    if (!sessionId) { setTaskId(""); return; }
    void call("tasks_list", { sessionId, limit: 50 }).then(async page => {
      if (taskId && !page.items.some(v => v.id === taskId)) {
        const selected = await call("task_get", { taskId });
        if (selected.sessionId === sessionId) page.items.push(selected);
      }
      if (disposed) return;
      setTasks(page.items); setTaskCursor(page.nextCursor);
      setTaskId(old => page.items.some(t => t.id === old) ? old : page.items[0]?.id ?? "");
    }).catch(cause => { if (!disposed) setError(toPublicError(cause)); });
    return () => { disposed = true; };
  }, [sessionId, refresh]);

  // Listen-before-query is provided by useAppStatus. Notifications invalidate;
  // only pull responses supply content or approve a task state. A periodic read
  // also recovers when a final notification was lost without a later event.
  useEffect(() => {
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    eventCursor.current = undefined;
    async function readText(channel: "reply" | "progress") {
      let text = "", cursor: string | undefined;
      let available = true;
      for (let i = 0; i < 70; i++) {
        const page = await call("task_content_get", { taskId, channel, ...(cursor ? { cursor } : {}) });
        if (page.taskId !== taskId || page.channel !== channel) throw errorForCode("PA_IPC_RESPONSE_INVALID");
        text += page.text; available = page.availability === "memory";
        if (!page.nextCursor) return { text, available };
        if (page.nextCursor === cursor) throw errorForCode("PA_IPC_RESPONSE_INVALID");
        cursor = page.nextCursor;
      }
      throw errorForCode("PA_VALIDATION_INPUT_TOO_LARGE");
    }
    async function read() {
      if (!taskId) return;
      try {
        const task = await call("task_get", { taskId });
        const events = await call("task_events_get", { taskId, ...(eventCursor.current ? { cursor: eventCursor.current } : {}), limit: 100 });
        if (events.taskId !== taskId) throw errorForCode("PA_IPC_RESPONSE_INVALID");
        const [reply, progress, approvals, changes] = await Promise.all([readText("reply"), readText("progress"), call("approvals_get", { taskId }), call("task_changes_get", { taskId, limit: 100 })]);
        if (disposed) return;
        if (changes.taskId !== taskId || approvals.items.some(a => a.taskId !== taskId || a.sessionId !== task.sessionId)) throw errorForCode("PA_IPC_RESPONSE_INVALID");
        eventCursor.current = events.resetRequired ? undefined : events.nextCursor ?? undefined;
        setDiff(previous => previous && changes.items.some(c => c.id === previous.changeId && c.diffAvailable) ? previous : null);
        setReadError(null);
        setResult({ task, reply: reply.text, progress: progress.text, available: reply.available || progress.available, approvals: approvals.items, changes: changes.items });
        setTasks(old => [task, ...old.filter(t => t.id !== task.id)].sort((a, b) => b.createdAt - a.createdAt));
      } catch (cause) {
        if (disposed) return;
        setResult(null); setDiff(null); setReadError(toPublicError(cause)); eventCursor.current = undefined;
      }
      if (!disposed) timer = setTimeout(() => void read(), 1200);
    }
    void read();
    return () => { disposed = true; if (timer) clearTimeout(timer); };
  }, [taskId, sessionId, projectId, refresh, status?.streamId, status?.sequence]);

  async function mutate(action: () => Promise<void>, reload = true) {
    if (mutating.current) return;
    mutating.current = true; setBusy(true); setError(null);
    try { await action(); } catch (cause) { setError(toPublicError(cause)); setNotice("操作结果请重新查询；变更请求不会自动重发。"); }
    finally { mutating.current = false; setBusy(false); if (reload) setRefresh(v => v + 1); }
  }
  function newSession() {
    if (!project) return;
    void mutate(async () => {
      const p: Pending = { kind: "session", projectId, sessionId: "", clientRequestId: crypto.randomUUID() };
      setPending(p);
      const receipt = await call("session_create", { projectId, expectedProjectRevision: project.revision, clientRequestId: p.clientRequestId });
      setSessionId(receipt.session.id); setTaskId(""); setPending(null); setNotice("会话已新建。");
    });
  }
  function send() {
    if (!canSend || !project || !session || !settings) return;
    void mutate(async () => {
      const p: Pending = { kind: "task", projectId, sessionId, clientRequestId: crypto.randomUUID() };
      setPending(p);
      const receipt = await call("task_start", { projectId, sessionId, clientRequestId: p.clientRequestId, expectedProjectRevision: project.revision,
        expectedSessionRevision: session.revision, expectedProjectSettingsRevision: settings.revision, expectedGlobalRevision: settings.globalRevision,
        selection: override ? selection : null, prompt });
      setTaskId(receipt.task.id); setPrompt(""); setPending(null); setNotice("服务端已接收任务，正在查询执行状态。");
    });
  }
  async function recoverPending() {
    if (!pending) return;
    await mutate(async () => {
      if (pending.kind === "session") {
        const found = await call("session_find_by_request", { projectId: pending.projectId, clientRequestId: pending.clientRequestId });
        if (found) { setProjectId(pending.projectId); setSessionId(found.id); }
        setNotice(found ? "已找到已创建会话。" : "当前尚未查到；若原请求仍在处理，请稍后再次查询。");
        if (found) setPending(null);
      } else {
        const found = await call("task_find_by_request", { projectId: pending.projectId, sessionId: pending.sessionId, clientRequestId: pending.clientRequestId });
        if (found) { setProjectId(pending.projectId); setSessionId(pending.sessionId); setTaskId(found.id); setPrompt(""); setPending(null); }
        setNotice(found ? "已找到已接收任务，没有重复提交。" : "当前尚未查到；若原请求仍在处理，请稍后再次查询。");
      }
    });
  }
  async function chooseProfile(id: string) {
    const request = ++capabilityRequest.current;
    const p = profiles.find(v => v.id === id);
    setSelection(p ? { profileId: p.id, expectedRevision: p.configRevision, modelId: "", effort: null } : null); setCapabilities([]); setConfirmed(false);
    if (!p) return;
    const epoch = generation.current;
    try {
      const result = await call("provider_models", { profileId: p.id, expectedRevision: p.configRevision, refresh: false });
      if (epoch !== generation.current || request !== capabilityRequest.current) return;
      setCapabilities(result.capabilities); const cap = result.capabilities[0];
      setSelection(old => old?.profileId === id ? { ...old, modelId: cap?.modelId ?? "", effort: cap?.efforts[0] ?? null } : old);
    } catch (cause) { setError(toPublicError(cause)); }
  }
  function decide(approval: ApprovalView, decision: "approve" | "deny") {
    if (!result) return;
    void mutate(async () => {
      await call("approval_decide", { approvalId: approval.id, taskId: result.task.id, expectedRevision: approval.revision, expectedTaskRevision: result.task.revision, operationFingerprint: approval.operationFingerprint, decision });
      setNotice("决定已提交；消费结果以重新查询的审批状态为准。");
    });
  }
  async function showDiff(change: TaskChange) {
    const current = taskId, target = currentTarget.current; setDiff(null);
    try { const data = await call("task_change_get", { taskId: current, changeId: change.id });
      if (data.taskId !== current || data.changeId !== change.id) throw errorForCode("PA_IPC_RESPONSE_INVALID");
      if (target === currentTarget.current) setDiff(data);
    } catch (cause) { setError(toPublicError(cause)); }
  }

  return <div className="page-content tasks-page">
    <section className="task-toolbar"><h2>项目与会话</h2><button type="button" onClick={onProjects}>管理项目</button><button type="button" onClick={onConfigure}>AI 配置</button><button type="button" disabled={busy} onClick={() => { setError(null); setRefresh(v => v + 1); }}>重新查询</button></section>
    <div role="status" data-testid="tasks-notice">{notice}</div>{error && <ErrorNotice error={error} testId="tasks-error" />}{readError && <ErrorNotice error={readError} testId="task-read-error" />}
    {taskService && taskService.state !== "ready" && <ErrorNotice error={errorForCode(taskService.reasonCode ?? "PA_TASK_RECOVERY_BLOCKED")} />}
    {pending && <div className="task-warning">上次提交结果尚未确认。<button type="button" disabled={busy} onClick={() => void recoverPending()}>按请求标识查询结果</button><button type="button" data-testid="task-stop-wait" disabled={busy} onClick={() => { setPending(null); setNotice("已停止等待；原请求仍可能完成，请查看会话和任务历史后再决定是否提交。"); }}>停止等待</button></div>}
    <section className="task-panel task-context"><label>目标项目<select data-testid="task-project" disabled={busy || Boolean(pending)} value={projectId} onChange={e => { ++generation.current; setProjectId(e.target.value); setSessionId(""); setTaskId(""); setConfirmed(false); }}><option value="">请选择已授权项目</option>{projects.map(p => <option key={p.id} value={p.id}>{p.displayName ?? p.name} · {p.availability}</option>)}</select></label>
      {projectCursor && <button type="button" disabled={busy} onClick={() => void mutate(async () => { const page = await call("projects_list", { cursor: projectCursor, limit: 100 }); setProjects(old => [...old, ...page.items.filter(p => !old.some(v => v.id === p.id))]); setProjectCursor(page.nextCursor); }, false)}>更多项目</button>}
      {project && <p className="task-path">工作目录：{project.absolutePath} · 授权：{project.authorization}</p>}
      {!allowed && <p>请选择有效的已授权项目；目录失效时请到项目页重新授权。</p>}
      <div className="task-toolbar"><label>会话<select data-testid="task-session" disabled={busy || Boolean(pending)} value={sessionId} onChange={e => { setSessionId(e.target.value); setTaskId(""); }}><option value="">请选择或新建会话</option>{sessions.map(s => <option key={s.id} value={s.id}>{new Date(s.createdAt).toLocaleString()} · {sessionLabel(s)} · {s.id.slice(0, 8)}</option>)}</select></label>
        <button type="button" data-testid="session-create" disabled={busy || !allowed || Boolean(pending)} onClick={newSession}>新建会话</button>
        <button type="button" data-testid="session-resume" disabled={busy || !session || session.status !== "interrupted" || !allowed} onClick={() => void mutate(async () => { if (session && project) await call("session_resume", { sessionId, expectedRevision: session.revision, expectedProjectRevision: project.revision }); })}>恢复会话</button>
        <button type="button" data-testid="session-end" disabled={busy || !session || session.status !== "active"} onClick={() => void mutate(async () => { if (session) await call("session_end", { sessionId, expectedRevision: session.revision }); })}>结束会话</button>
        {sessionCursor && <button type="button" disabled={busy} onClick={() => void mutate(async () => { const page = await call("sessions_list", { projectId, cursor: sessionCursor, limit: 50 }); setSessions(old => [...old, ...page.items.filter(s => !old.some(v => v.id === s.id))]); setSessionCursor(page.nextCursor); })}>更多会话</button>}
      </div>
    </section>
    <div className="task-layout"><section className="task-panel task-composer"><h2>发起任务</h2><label><input type="checkbox" checked={override} disabled={busy} onChange={e => { setOverride(e.target.checked); setConfirmed(false); }} />为本次任务选择配置</label>
      {override && <div className="task-selection"><label>服务源<select value={selection?.profileId ?? ""} disabled={busy} onChange={e => void chooseProfile(e.target.value)}><option value="">请选择服务源</option>{profiles.map(p => <option key={p.id} value={p.id} disabled={!p.available || !p.enabled}>{p.name}</option>)}</select></label>
        <label>模型<select value={selection?.modelId ?? ""} disabled={busy || !capabilities.length} onChange={e => { const cap = capabilities.find(c => c.modelId === e.target.value); setSelection(s => s ? { ...s, modelId: e.target.value, effort: cap?.efforts[0] ?? null } : null); }}><option value="">请先读取服务源能力</option>{capabilities.map(c => <option key={c.modelId} value={c.modelId}>{c.modelId}</option>)}</select></label>
        <label>思考强度<select value={selection?.effort ?? ""} disabled={busy || !selection?.modelId} onChange={e => setSelection(s => s ? { ...s, effort: e.target.value || null } : null)}>{capabilities.find(c => c.modelId === selection?.modelId)?.efforts.map(e => <option key={e} value={e}>{e}</option>)}</select></label></div>}
      <div className="task-config" data-testid="task-config">{effective ? <><strong>{source}：{profile?.name ?? "失效服务源"} / {effective.modelId} / {effective.effort ?? "能力默认"}</strong><p>{profile?.endpoint} · 配置修订 {effective.expectedRevision}</p></> : "没有可用默认配置，请到 AI 配置页设置并测试，或选择本次任务配置。"}</div>
      {!validConfig && <p>配置缺失、已失效或能力未读取；请主动重新选择或核验配置。启动时还会复核连接证据和凭据版本。</p>}
      <form onSubmit={e => { e.preventDefault(); send(); }}><label htmlFor="task-prompt">任务指令</label><textarea id="task-prompt" data-testid="task-prompt" value={prompt} disabled={busy || Boolean(pending)} onChange={e => setPrompt(e.target.value)} onCompositionStart={() => { composing.current = true; }} onCompositionEnd={() => { composing.current = false; }} onKeyDown={e => { if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing && !composing.current && e.keyCode !== 229) { e.preventDefault(); send(); } }} placeholder="Enter 发送，Shift+Enter 换行" />
        <p>{promptBytes} / 65536 字节。指令和回复仅保留在当前进程内；重启后保留任务元数据。</p><label><input data-testid="task-confirm" type="checkbox" checked={confirmed} disabled={busy || !allowed || !validConfig || !session} onChange={e => setConfirmed(e.target.checked)} />确认目标项目、会话和上方配置</label>
        <button data-testid="task-send" className="primary-button" type="submit" disabled={!canSend}>发送任务</button></form>
    </section>
    <section className="task-panel task-history"><h2>任务历史</h2>{!tasks.length && <p>当前会话还没有任务。</p>}<div className="task-list">{tasks.map(t => <button type="button" key={t.id} data-task-id={t.id} aria-pressed={taskId === t.id} disabled={busy} onClick={() => { setTaskId(t.id); setResult(null); setDiff(null); }}><strong>{labels[t.status]}</strong><small>{new Date(t.createdAt).toLocaleString()} · {t.id.slice(0, 8)}</small></button>)}</div>
      {taskCursor && <button type="button" disabled={busy} onClick={() => void mutate(async () => { const page = await call("tasks_list", { sessionId, cursor: taskCursor, limit: 50 }); setTasks(old => [...old, ...page.items.filter(t => !old.some(v => v.id === t.id))]); setTaskCursor(page.nextCursor); })}>更多任务</button>}
    </section></div>
    {result && <section className="task-panel task-result" data-testid="task-result" data-task-id={result.task.id} data-status={result.task.status} data-revision={result.task.revision}><div className="task-toolbar"><h2>{labels[result.task.status]}</h2>{!ended(result.task) && <button type="button" data-testid="task-cancel" disabled={busy || result.task.status === "cancelling"} onClick={() => void mutate(async () => { const receipt = await call("task_cancel", { taskId, expectedRevision: result.task.revision }); setNotice(receipt.accepted ? "取消请求已接受，等待权威终态。" : "任务已进入终态，请查看最新状态。"); })}>取消任务</button>}</div>
      <p>实际快照：{result.task.snapshot.profileName} / {result.task.snapshot.modelId} / {result.task.snapshot.effort} · 来源 {result.task.selectionSource} · 修订 {result.task.snapshot.configRevision}</p>
      {result.task.error && <ErrorNotice error={errorForCode(result.task.error.code)} />}
      {result.approvals.map(a => <article className="task-approval" key={a.id} data-testid="task-approval"><h3>审批 · {a.operationKind}</h3><p>{a.displaySummary}</p><p>{a.decision} · {a.consumption} · 到期 {new Date(a.expiresAt).toLocaleTimeString()}</p>{a.decision === "pending" && <div><button type="button" data-testid="approval-approve" disabled={busy || a.operationKind === "unknown" || result.task.status !== "approval_required" || a.expiresAt <= Date.now()} onClick={() => decide(a, "approve")}>批准这一次</button><button type="button" data-testid="approval-deny" disabled={busy || result.task.status !== "approval_required"} onClick={() => decide(a, "deny")}>拒绝</button></div>}</article>)}
      <h3>回复</h3>{!result.available ? <p>{ended(result.task) ? "内容已过期或进程已重启；任务元数据仍可查询。" : "等待任务输出…"}</p> : <pre className="task-text" data-testid="task-reply">{result.reply || (ended(result.task) ? "本次任务没有文本回复。" : "等待回复…")}</pre>}
      <details><summary>执行进度</summary><pre className="task-text" data-testid="task-progress">{result.progress || "暂无执行输出"}</pre></details>
      <h3>任务变更文件</h3>{!result.changes.length && <p>{result.available ? "没有经核验的任务变更文件。" : "变更正文不可用。"}</p>}{result.changes.map(c => <div className="task-change" key={c.id}><span>{c.kind} · {c.relativePath}{!c.diffAvailable && " · 路径或文件身份已变化，差异不可用"}</span><button type="button" disabled={!c.diffAvailable} onClick={() => void showDiff(c)}>查看差异</button><button type="button" disabled={busy || !c.diffAvailable || c.kind === "deleted"} onClick={() => void mutate(async () => { await call("project_open_file", { projectId: result.task.projectId, relativePath: c.relativePath }); setNotice("系统已接受文件交付。"); })}>在编辑器中打开</button></div>)}
      {diff && <div data-testid="task-diff"><h3>文件差异</h3>{diff.text !== null ? <pre className="task-text">{diff.text}</pre> : <p>差异不可用：{({ expired: "内容已过期", binary: "二进制文件", too_large: "内容超限", unsafe_path: "文件路径或身份已变化" })[diff.unavailableReason ?? "expired"]}</p>}</div>}
    </section>}
  </div>;
}
