import { useEffect, useRef, useState } from "react";
import { clearLogs, getLogsStatus } from "../ipc/client";
import type { LogStatus } from "../ipc/contracts";
import { errorForCode, toPublicError } from "../ipc/errors";
import type { PublicError } from "../ipc/errors";
import { ErrorNotice } from "./ErrorNotice";

export function LoggingPanel({ allowClear = false, streamId, sequence }: {
  allowClear?: boolean; streamId?: string; sequence?: number;
}) {
  const [status, setStatus] = useState<LogStatus | null>(null);
  const [phase, setPhase] = useState<"reading" | "ready" | "clearing" | "cleared" | "failure">("reading");
  const [error, setError] = useState<PublicError | null>(null);
  const [needsRefresh, setNeedsRefresh] = useState(false);
  const mounted = useRef(false);
  const generation = useRef(0);
  const clearing = useRef(false);
  async function read() {
    if (clearing.current) return;
    const id = ++generation.current;
    setPhase("reading"); setError(null);
    try {
      const next = await getLogsStatus();
      if (!mounted.current || id !== generation.current) return;
      setStatus(next); setNeedsRefresh(false); setPhase("ready");
    } catch (cause) {
      if (!mounted.current || id !== generation.current) return;
      setError(toPublicError(cause)); setPhase("failure");
    }
  }
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; ++generation.current; };
  }, []);
  useEffect(() => { void read(); }, [streamId, sequence]);
  async function clear() {
    if (!allowClear || clearing.current || !status || needsRefresh || phase === "reading") return;
    const id = ++generation.current;
    clearing.current = true;
    setPhase("clearing"); setError(null);
    try {
      // One user submission, one mutation. A timeout/stop does not cancel native
      // cleanup; require a fresh status query before offering another attempt.
      const next = await clearLogs();
      if (!mounted.current || id !== generation.current) return;
      setStatus(next); setPhase("cleared");
    } catch (cause) {
      if (!mounted.current || id !== generation.current) return;
      setError(toPublicError(cause)); setNeedsRefresh(true); setPhase("failure");
    } finally { clearing.current = false; }
  }
  const busy = phase === "reading" || phase === "clearing";
  return <section className="logging-panel section-block" aria-label="日志与留存" aria-busy={busy}>
    <h2>日志与留存</h2>
    <p role="status" data-testid="logging-status" data-phase={phase} data-enabled={status?.enabled}>
      {phase === "reading" ? "正在读取日志状态…" : phase === "clearing" ? "正在清理本应用日志…" : phase === "cleared" ? "日志清理请求已完成。" : phase === "failure" ? "日志操作失败，请刷新状态。" : "日志状态已更新。"}
      {status && ` 日志落盘${status.enabled ? "已启用" : "已禁用"} · 级别 ${status.level} · ${status.fileCount} 个文件 · ${status.totalBytes} 字节`}
    </p>
    {status && <>
      <dl className="logging-details">
        <dt>审计留存期限</dt><dd>{status.auditRetentionDays} 天</dd>
        <dt>摘要采集</dt><dd>关闭</dd>
      </dl>
      {!status.enabled && <p role="status" className="logging-warning" data-testid="logging-disabled">日志落盘已禁用；本次运行可能无法保存后续日志。请检查磁盘空间和应用日志目录权限，修复后重启应用。</p>}
      {(status.errorCode || !status.enabled) && <ErrorNotice error={errorForCode(status.errorCode ?? "PA_INTERNAL_UNEXPECTED")} testId="logging-error" />}
    </>}
    {error && <ErrorNotice error={error} testId="logs-operation-error" />}
    {needsRefresh && <p>请先刷新日志状态，确认清理结果后再决定是否重新清理。</p>}
    <div className="logging-actions">
      <button type="button" className="quiet-button" disabled={busy} onClick={() => void read()}>刷新日志状态</button>
      {allowClear && <button type="button" className="quiet-button" data-testid="logs-clear" disabled={busy || !status || needsRefresh} onClick={() => void clear()}>清理本应用日志</button>}
    </div>
    {allowClear && <p className="logging-help">清理本应用日志文件。审计按 90 天期限自动清理；摘要采集保持关闭。</p>}
  </section>;
}
