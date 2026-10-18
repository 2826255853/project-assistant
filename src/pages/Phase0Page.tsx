import { useState } from "react";
import "./Phase0Page.css";
import { frameworkProbe, phase0OpenPath, phase0OpenUrl } from "../ipc/client";
import type { ProbeReply } from "../ipc/contracts";
import { ErrorNotice } from "../components/ErrorNotice";
import { toPublicError } from "../ipc/errors";
import type { PublicError } from "../ipc/errors";

function describeOpenError(kind: "path" | "url", cause: unknown): string {
  const detail = typeof cause === "string" ? cause : String(cause);
  if (detail === "OPEN_PATH_DENIED" || /forbidden path|not allowed to open path/i.test(detail)) return "OPEN_PATH_DENIED：路径不在已授权的测试范围内。";
  if (detail === "OPEN_PATH_MISSING") return "OPEN_PATH_MISSING：目录或文件不存在，请检查路径。";
  if (detail === "OPEN_PATH_ACCESS_DENIED") return "OPEN_PATH_ACCESS_DENIED：当前用户无权访问该路径。";
  if (detail === "OPEN_SCOPE_UNAVAILABLE") return "OPEN_SCOPE_UNAVAILABLE：测试目录尚未创建。";
  if (/forbidden url|not allowed to open url/i.test(detail)) return "OPEN_URL_DENIED：地址不在已授权的测试范围内。";
  if (kind === "path" && /os error 2|cannot find|not found|找不到/i.test(detail)) {
    return "OPEN_PATH_MISSING：目录或文件不存在，请检查路径。";
  }
  if (kind === "url" && /invalid url|无效的 url/i.test(detail)) {
    return "OPEN_INVALID_URL：请输入有效的 HTTP(S) 地址。";
  }
  return `OPEN_FAILED：系统未能打开${kind === "path" ? "路径" : "地址"}，请检查默认关联和系统权限后重试。`;
}

function Phase0Page() {
  const [message, setMessage] = useState("你好，Rust！");
  const [reply, setReply] = useState<ProbeReply | null>(null);
  const [error, setError] = useState<PublicError | null>(null);
  const [pending, setPending] = useState(false);
  const [systemPath, setSystemPath] = useState("");
  const [searchBase, setSearchBase] = useState("");
  const [searchQuery, setSearchQuery] = useState("中文 & 空格 / ? #");
  const [openResult, setOpenResult] = useState("");
  const [openPending, setOpenPending] = useState(false);
  const [openAttempt, setOpenAttempt] = useState(0);

  async function runProbe() {
    setPending(true);
    setReply(null);
    setError(null);
    try {
      setReply(await frameworkProbe(message));
    } catch (cause) {
      setError(toPublicError(cause));
    } finally {
      setPending(false);
    }
  }

  async function runOpen(kind: "path" | "url") {
    setOpenAttempt((value) => value + 1);
    setOpenPending(true);
    setOpenResult("");
    try {
      if (kind === "path") {
        await phase0OpenPath(systemPath);
      } else {
        const url = new URL(searchBase);
        if (!["http:", "https:"].includes(url.protocol)) throw new Error("Invalid URL protocol");
        url.searchParams.set("q", searchQuery);
        await phase0OpenUrl(url);
      }
      setOpenResult(`${kind === "path" ? "路径" : "搜索 URL"}：系统已接受打开请求`);
    } catch (cause) {
      setOpenResult(describeOpenError(kind, cause));
    } finally {
      setOpenPending(false);
    }
  }

  return (
    <main className="phase0-page">
      <p className="eyebrow">阶段 0 · 步骤 0.2</p>
      <h1>桌面框架验证</h1>
      <p>发送一条消息，确认 React → Tauri IPC → Rust → React 的往返调用。</p>
      <form onSubmit={(event) => { event.preventDefault(); void runProbe(); }}>
        <label htmlFor="probe-message">测试消息（留空可验证错误返回）</label>
        <input id="probe-message" value={message} disabled={pending}
          onChange={(event) => setMessage(event.currentTarget.value)} />
        <button type="submit" disabled={pending}>
          {pending ? "调用中…" : "调用 Rust"}
        </button>
      </form>
      <section aria-live="polite" aria-busy={pending}>
        {reply && <div className="result" data-testid="probe-success">
          <h2>调用成功</h2>
          <p>{reply.message}</p>
          <p>运行平台：{reply.platform} / {reply.architecture}</p>
        </div>}
        {error && <ErrorNotice error={error} testId="probe-error" />}
      </section>
      <p className="hint">点击窗口关闭按钮退出应用。这里只验证框架链路。</p>
      <section className="open-probe" data-system-path={systemPath}
        data-search-base={searchBase} data-search-query={searchQuery}>
        <p className="eyebrow">阶段 0 · 步骤 0.4</p>
        <h2>系统打开能力验证</h2>
        <p>路径仅限系统临时目录下的 phase-0-system-open 测试区；搜索地址仅限 127.0.0.1。本区用于验证插件和权限，不作为正式文件浏览入口。</p>
        <label htmlFor="system-path">测试目录或文件的绝对路径</label>
        <input id="system-path" value={systemPath} disabled={openPending}
          onChange={(event) => setSystemPath(event.currentTarget.value)} />
        <button type="button" disabled={openPending} onClick={() => void runOpen("path")}>系统打开路径</button>
        <label htmlFor="search-base">本机搜索测试地址</label>
        <input id="search-base" value={searchBase} disabled={openPending}
          onChange={(event) => setSearchBase(event.currentTarget.value)} />
        <label htmlFor="search-query">搜索词</label>
        <input id="search-query" value={searchQuery} disabled={openPending}
          onChange={(event) => setSearchQuery(event.currentTarget.value)} />
        <button type="button" disabled={openPending} onClick={() => void runOpen("url")}>系统打开编码后的搜索 URL</button>
        <p data-testid="open-result" data-attempt={openAttempt} role="status">{openResult}</p>
      </section>
    </main>
  );
}

export default Phase0Page;
