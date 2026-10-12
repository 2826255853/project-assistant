import { useEffect, useRef, useState } from "react";
import { useOverlay } from "../app/useOverlay";
import { controlOverlay, dragOverlay } from "../ipc/client";
import "./OverlayPage.css";

export default function OverlayPage() {
  const view = useOverlay();
  const input = useRef<HTMLTextAreaElement>(null);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState("");
  const dragStart = useRef<{ x: number; y: number } | null>(null);
  const dragged = useRef(false);
  useEffect(() => { document.body.classList.add("overlay-body"); return () => document.body.classList.remove("overlay-body"); }, []);
  useEffect(() => { if (view) setDraft(view.draft); if (view?.expanded) requestAnimationFrame(() => input.current?.focus()); }, [view?.expanded]);
  function act(action: string) { void controlOverlay(action).catch((cause: Error) => setError(cause.message)); }
  if (!view) return <div className="overlay-loading">…</div>;
  return <div className="overlay-root" data-expanded={view.expanded}>
    {!view.expanded ? <button className="floating-ball" aria-label="展开对话" title="点击展开，按住拖动" onPointerDown={(event) => { dragStart.current = { x: event.clientX, y: event.clientY }; dragged.current = false; }} onPointerMove={(event) => {
      const start = dragStart.current;
      if (start && event.buttons === 1 && Math.hypot(event.clientX - start.x, event.clientY - start.y) > 5) { dragStart.current = null; dragged.current = true; void dragOverlay(); }
    }} onPointerUp={() => { dragStart.current = null; }} onClick={() => { if (!dragged.current) act("expand"); dragged.current = false; }} onContextMenu={(event) => { event.preventDefault(); void dragOverlay(); }}>PA</button>
      : <section className="overlay-panel" aria-label="常驻对话"><header onPointerDown={(event) => { if ((event.target as HTMLElement).closest("button")) return; void dragOverlay(); }}><strong>本机项目助手</strong><button aria-label="收起" onClick={() => act("collapse")}>−</button><button aria-label="关闭常驻入口" onClick={() => act("disable")}>×</button></header>
      <p className="overlay-context">尚未选择项目或会话</p><p className="overlay-notice">任务服务尚未启用。请打开主窗口补充上下文。</p>
      <label htmlFor="overlay-draft">任务草稿</label><textarea id="overlay-draft" ref={input} value={draft} maxLength={8000} placeholder="先记下要做的事…" onChange={(event) => { const value = event.currentTarget.value; setDraft(value); void controlOverlay("draft", { draft: value }); }} onKeyDown={(event) => {
        if (event.nativeEvent.isComposing || event.keyCode === 229) return;
        if (event.key === "Escape") { event.preventDefault(); act("collapse"); }
        else if (event.key === "Enter" && !event.shiftKey) { event.preventDefault(); setError("任务服务尚未启用，草稿已保留。"); }
      }} />
      <p className="overlay-hint">Shift+Enter 换行 · Esc 收起 · 草稿仅保留在本次运行</p><footer><button onClick={() => act("open_main")}>打开主窗口</button><button disabled>发送（暂不可用）</button></footer>
      {(error || view.windowError) && <p role="alert" className="overlay-error">{error || "置顶或窗口恢复失败，请从托盘重新展开。"}</p>}
    </section>}
  </div>;
}
