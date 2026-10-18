import { useEffect, useRef, useState } from "react";
import { useAppStatus } from "../app/useAppStatus";
import type { OverlayAction } from "../ipc/contracts";
import { controlOverlay, dragOverlay } from "../ipc/client";
import "./OverlayPage.css";
import { ErrorNotice } from "../components/ErrorNotice";
import { errorForCode, toPublicError } from "../ipc/errors";
import type { PublicError } from "../ipc/errors";

export default function OverlayPage() {
  const { status, error: syncError, retry } = useAppStatus();
  const view = status?.overlay;
  const input = useRef<HTMLTextAreaElement>(null);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<PublicError | null>(null);
  const dragStart = useRef<{ x: number; y: number } | null>(null);
  const dragged = useRef(false);
  useEffect(() => { document.body.classList.add("overlay-body"); return () => document.body.classList.remove("overlay-body"); }, []);
  useEffect(() => { if (view) setDraft(view.draft); if (view?.expanded) requestAnimationFrame(() => input.current?.focus()); }, [view?.expanded]);
  function report(cause: unknown) { setError(toPublicError(cause)); }
  function drag() { void dragOverlay().catch(report); }
  function act(action: OverlayAction["action"]) { setError(null); void controlOverlay(action).catch(report); }
  if (!view) return <div className="overlay-loading">{syncError ? <ErrorNotice error={syncError} actions={<button onClick={retry}>重试同步</button>} /> : "…"}</div>;
  return <div className="overlay-root" data-expanded={view.expanded} data-context={status?.activeContext?.projectId ?? ""} data-stream={status?.streamId} data-sequence={status?.sequence}>
    {!view.expanded ? <button className="floating-ball" aria-label="展开对话" title="点击展开，按住拖动" onPointerDown={(event) => { dragStart.current = { x: event.clientX, y: event.clientY }; dragged.current = false; }} onPointerMove={(event) => {
      const start = dragStart.current;
      if (start && event.buttons === 1 && Math.hypot(event.clientX - start.x, event.clientY - start.y) > 5) { dragStart.current = null; dragged.current = true; drag(); }
    }} onPointerUp={() => { dragStart.current = null; }} onClick={() => { if (!dragged.current) act("expand"); dragged.current = false; }} onContextMenu={(event) => { event.preventDefault(); drag(); }}>PA</button>
      : <section className="overlay-panel" aria-label="常驻对话"><header onPointerDown={(event) => { if ((event.target as HTMLElement).closest("button")) return; drag(); }}><strong>本机项目助手</strong><button aria-label="收起" onClick={() => act("collapse")}>−</button><button aria-label="关闭常驻入口" onClick={() => act("disable")}>×</button></header>
      <p className="overlay-context">{status?.activeContext ? `${status.activeContext.projectId} · ${status.activeContext.modelId}` : "尚未选择项目或会话"}</p><p className="overlay-notice">任务服务尚未启用。请打开主窗口补充上下文。</p>
      <label htmlFor="overlay-draft">任务草稿</label><textarea id="overlay-draft" ref={input} value={draft} maxLength={8000} placeholder="先记下要做的事…" onChange={(event) => { const value = event.currentTarget.value; setDraft(value); void controlOverlay("draft", { draft: value }).catch(report); }} onKeyDown={(event) => {
        if (event.nativeEvent.isComposing || event.keyCode === 229) return;
        if (event.key === "Escape") { event.preventDefault(); act("collapse"); }
        else if (event.key === "Enter" && !event.shiftKey) { event.preventDefault(); setError(errorForCode("PA_TASKS_NOT_IMPLEMENTED")); }
      }} />
      <p className="overlay-hint">Shift+Enter 换行 · Esc 收起 · 草稿仅保留在本次运行</p><footer><button onClick={() => act("open_main")}>打开主窗口</button><button disabled>发送（暂不可用）</button></footer>
      {(error || syncError || view.windowError) && <ErrorNotice error={error || syncError || errorForCode(view.windowError)} actions={syncError ? <button onClick={retry}>重试同步</button> : undefined} />}
    </section>}
  </div>;
}
