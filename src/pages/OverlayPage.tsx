import { useEffect, useRef, useState } from "react";
import { useAppStatus } from "../app/useAppStatus";
import type { OverlayAction } from "../ipc/contracts";
import { call, controlOverlay, dragOverlay } from "../ipc/client";
import { ErrorNotice } from "../components/ErrorNotice";
import { errorForCode, toPublicError } from "../ipc/errors";
import type { PublicError } from "../ipc/errors";
import type { OverlayTaskView } from "../ipc/taskContracts";
import "./OverlayPage.css";

export default function OverlayPage() {
  const { status, error: syncError, retry } = useAppStatus();
  const view = status?.overlay;
  const input = useRef<HTMLTextAreaElement>(null);
  const [draft, setDraft] = useState("");
  const [task, setTask] = useState<OverlayTaskView | null>(null);
  const [contextName, setContextName] = useState<string | null>(null);
  const [contextRevision, setContextRevision] = useState(0);
  const [pendingRequestId, setPendingRequestId] = useState<string | null>(null);
  const [error, setError] = useState<PublicError | null>(null);
  const [busy, setBusy] = useState(false);
  const dragStart = useRef<{ x: number; y: number } | null>(null);
  const dragged = useRef(false);

  useEffect(() => { document.body.classList.add("overlay-body"); return () => document.body.classList.remove("overlay-body"); }, []);
  useEffect(() => { if (view) setDraft(view.draft); if (view?.expanded) requestAnimationFrame(() => input.current?.focus()); }, [view?.expanded, view?.draft]);
  useEffect(() => {
    let stopped = false;
    async function read() {
      try {
        const state = await call("overlay_task_get", {});
        if (stopped) return;
        setTask(state.task); setContextName(state.context?.projectName ?? null); setContextRevision(state.contextRevision); setPendingRequestId(state.pendingRequestId);
      } catch (cause) { if (!stopped) setError(toPublicError(cause)); }
    }
    void read(); const timer = window.setInterval(() => void read(), 900);
    return () => { stopped = true; window.clearInterval(timer); };
  }, [status?.streamId, status?.sequence, view?.expanded]);
  function report(cause: unknown) { setError(toPublicError(cause)); }
  function act(action: OverlayAction["action"]) { setError(null); void controlOverlay(action).catch(report); }
  async function send() {
    if (!draft.trim() || !contextRevision || busy || pendingRequestId) return;
    const requestId = crypto.randomUUID(); setPendingRequestId(requestId);
    setBusy(true); setError(null);
    try { const receipt = await call("task_start_from_active", { expectedContextRevision: contextRevision, clientRequestId: requestId, prompt: draft }); setTask(receipt.task); setPendingRequestId(null); setDraft(""); await controlOverlay("draft", { draft: "" }); }
    catch (cause) { const e = toPublicError(cause); report(cause); if (!["PA_IPC_TRANSPORT_FAILED", "PA_IPC_TIMEOUT", "PA_IPC_CANCELLED"].includes(e.code)) setPendingRequestId(null); } finally { setBusy(false); }
  }
  async function recoverPending() {
    if (!pendingRequestId || !contextRevision || busy) return;
    setBusy(true); setError(null);
    try { const found = await call("overlay_task_find_by_request", { expectedContextRevision: contextRevision, clientRequestId: pendingRequestId }); if (found) { setTask(found); setPendingRequestId(null); setDraft(""); await controlOverlay("draft", { draft: "" }); } }
    catch (cause) { report(cause); } finally { setBusy(false); }
  }
  async function cancel() {
    if (!task || !contextRevision || busy) return;
    setBusy(true); setError(null);
    try { const receipt = await call("overlay_task_cancel", { expectedContextRevision: contextRevision, expectedTaskRevision: task.revision }); setTask(old => old ? { ...old, status: receipt.status, revision: receipt.revision } : old); }
    catch (cause) { report(cause); } finally { setBusy(false); }
  }
  if (!view) return <div className="overlay-loading">{syncError ? <ErrorNotice error={syncError} actions={<button onClick={retry}>重试同步</button>} /> : "正在同步…"}</div>;
  return <div className="overlay-root" data-expanded={view.expanded} data-context={status?.activeProject?.projectId ?? ""} data-stream={status?.streamId} data-sequence={status?.sequence}>
    {!view.expanded ? <button className="floating-ball" aria-label="展开对话" title="点击展开，按住拖动" onPointerDown={(event) => { dragStart.current = { x: event.clientX, y: event.clientY }; dragged.current = false; }} onPointerMove={(event) => { const start = dragStart.current; if (start && event.buttons === 1 && Math.hypot(event.clientX - start.x, event.clientY - start.y) > 5) { dragStart.current = null; dragged.current = true; void dragOverlay().catch(report); } }} onPointerUp={() => { dragStart.current = null; }} onClick={() => { if (!dragged.current) act("expand"); dragged.current = false; }} onContextMenu={(event) => { event.preventDefault(); void dragOverlay().catch(report); }}>PA</button>
      : <section className="overlay-panel" aria-label="常驻对话"><header onPointerDown={(event) => { if ((event.target as HTMLElement).closest("button")) return; void dragOverlay().catch(report); }}><strong>本机项目助手</strong><button aria-label="收起" onClick={() => act("collapse")}>−</button><button aria-label="关闭常驻入口" onClick={() => act("disable")}>×</button></header>
      <p className="overlay-context">{contextName ? `活动项目：${contextName}` : "尚未准备项目、会话和配置"}</p>{!contextRevision && <p className="overlay-notice">请打开主窗口选择项目、会话和 AI 配置后再发送。</p>}
      {pendingRequestId && <div className="overlay-notice" data-testid="overlay-pending">提交结果尚未确认。可按原请求标识查询状态，不会重复发送。<button disabled={busy} onClick={() => void recoverPending()}>查询提交状态</button></div>}
      {task && <div className="overlay-task" data-testid="overlay-task" data-status={task.status}><strong>{task.status}</strong>{task.replyPreview && <p>{task.replyPreview}</p>}{task.approvalPending && <p className="overlay-notice">任务需要审批，请打开主窗口处理。</p>}{task.errorCode && <ErrorNotice error={errorForCode(task.errorCode)} />}{!["completed", "failed", "cancelled", "interrupted"].includes(task.status) && <button disabled={busy} onClick={() => void cancel()}>取消任务</button>}<button aria-label="查看主窗口详情" onClick={() => act("open_main")}>查看主窗口详情</button></div>}
      <label htmlFor="overlay-draft">任务草稿</label><textarea id="overlay-draft" ref={input} value={draft} maxLength={8000} placeholder="输入任务；Enter 发送，Shift+Enter 换行" onChange={(event) => { const value = event.currentTarget.value; setDraft(value); void controlOverlay("draft", { draft: value }).catch(report); }} onKeyDown={(event) => { if (event.nativeEvent.isComposing || event.keyCode === 229) return; if (event.key === "Escape") { event.preventDefault(); act("collapse"); } else if (event.key === "Enter" && !event.shiftKey) { event.preventDefault(); void send(); } }} />
      <p className="overlay-hint">Shift+Enter 换行 · Esc 收起 · 草稿仅保留在本次运行</p><footer><button onClick={() => act("open_main")}>打开主窗口</button><button className="primary-button" disabled={busy || Boolean(pendingRequestId) || !contextRevision || !draft.trim()} onClick={() => void send()}>发送任务</button></footer>
      {(error || syncError || view.windowError) && <ErrorNotice error={error || syncError || errorForCode(view.windowError)} actions={syncError ? <button onClick={retry}>重试同步</button> : undefined} />}
    </section>}
  </div>;
}
