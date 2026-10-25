import { useEffect, useRef, useState } from "react";
import "./App.css";
import { StatePanel } from "./components/StatePanel";
import { closeApplication } from "./ipc/client";
import { SettingsPage } from "./pages/SettingsPage";
import { AiConfigPage } from "./pages/AiConfigPage";
import { useAppStatus } from "./app/useAppStatus";
import type { AppStatus, ServiceStatus } from "./ipc/contracts";
import { ErrorNotice } from "./components/ErrorNotice";
import { LoggingPanel } from "./components/LoggingPanel";
import { errorForCode, toPublicError } from "./ipc/errors";
import type { PublicError } from "./ipc/errors";
type Page = "home" | "projects" | "ai" | "settings" | "diagnostics";
const pageMeta: Record<Page, {
    label: string;
    title: string;
    description: string;
}> = {
    home: { label: "概览", title: "工作台", description: "从这里进入项目、配置运行环境或查看最近状态。" },
    projects: { label: "项目", title: "项目", description: "近期项目、添加项目和项目搜索将在项目管理功能上线后开放。" },
    ai: { label: "AI 配置", title: "AI 配置", description: "管理应用自管服务源、凭据、模型能力和默认选择。" },
    settings: { label: "设置", title: "设置", description: "修改并保存应用设置，重启后恢复。" },
    diagnostics: { label: "诊断", title: "服务状态", description: "查看当前可用功能。" },
};
function NavButton({ page, current, onSelect }: {
    page: Page;
    current: Page;
    onSelect: (page: Page) => void;
}) {
    const icons: Record<Page, string> = { home: "⌂", projects: "▦", ai: "✦", settings: "⚙", diagnostics: "⌁" };
    return <button data-page={page} aria-label={pageMeta[page].label} className={`nav-item${page === current ? " active" : ""}`} type="button" onClick={() => onSelect(page)} onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); onSelect(page); } }} aria-current={page === current ? "page" : undefined}>
    <span className="nav-icon" aria-hidden="true">{icons[page]}</span>{pageMeta[page].label}
  </button>;
}
function StatusCard({ title, value, detail, tone = "neutral" }: {
    title: string;
    value: string;
    detail: string;
    tone?: "neutral" | "ready" | "pending";
}) {
    return <article className={`status-card ${tone}`}><span className="status-mark" aria-hidden="true"/><div><p className="card-label">{title}</p><strong>{value}</strong><p className="card-detail">{detail}</p></div></article>;
}
function Home({ onSelect }: {
    onSelect: (page: Page) => void;
}) {
    return <div className="page-content">
    <section className="welcome-panel"><div><p className="eyebrow">本机项目助手</p><h2>准备开始工作</h2><p>应用设置和 AI 配置已可使用；项目登记、搜索与打开将在后续阶段接入。</p></div><button className="primary-button" type="button" onClick={() => onSelect("projects")}>进入项目</button></section>
    <div className="status-grid"><StatusCard title="桌面运行状态" value="正常" detail="主窗口和前端资源已加载" tone="ready"/><StatusCard title="项目工作区" value="尚未选择" detail="下一阶段接入近期项目和搜索"/><StatusCard title="AI 服务源" value="未配置" detail="应用不会读取外部 cc-switch 配置" tone="pending"/></div>
    <section className="section-block"><div className="section-heading"><div><p className="eyebrow">快速入口</p><h3>常用模块</h3></div></div><div className="quick-grid"><button type="button" onClick={() => onSelect("projects")}><span>▦</span><b>项目</b><small>查找和打开本机项目</small></button><button type="button" onClick={() => onSelect("ai")}><span>✦</span><b>AI 配置</b><small>管理应用自管运行配置</small></button><button type="button" onClick={() => onSelect("settings")}><span>⚙</span><b>设置</b><small>调整应用行为和记录</small></button></div></section>
  </div>;
}
function Placeholder({ page, onSelect }: {
    page: Page;
    onSelect: (page: Page) => void;
}) {
    const meta = pageMeta[page];
    return <div className="page-content"><StatePanel kind="empty" title="项目列表尚未启用" actions={<><button className="primary-button" type="button" onClick={() => onSelect("home")}>返回概览</button><button className="quiet-button" type="button" onClick={() => onSelect("diagnostics")}>查看服务状态</button></>}>
    <p>{meta.description}</p>
  </StatePanel></div>;
}
function DiagnosticsPage({ status }: { status: AppStatus | null }) {
    const labels: Record<ServiceStatus["id"], string> = { desktop: "桌面", ipc: "本地通信", events: "状态同步", overlay: "常驻入口", storage: "本地存储", settings: "设置", logging: "日志", credentials: "凭据", providers: "服务源", codex: "任务进程", projects: "项目", tasks: "任务" };
    return <div className="page-content"><div className="status-grid">{status?.services.map(service => <article className={`status-card ${service.state === "ready" ? "ready" : "pending"}`} key={service.id}>
      <span className="status-mark" aria-hidden="true" /><div><p className="card-label">{labels[service.id]}</p><strong>{service.state === "ready" ? "已就绪" : service.state === "degraded" ? "需要处理" : "尚未启用"}</strong>
        {service.reasonCode ? <ErrorNotice error={errorForCode(service.reasonCode)} /> : <p className="card-detail">本地服务可用</p>}
      </div>
    </article>)}</div>
    <LoggingPanel streamId={status?.streamId} sequence={status?.sequence} />
    <StatePanel kind="unavailable" title="部分业务服务仍在准备中"><p>项目管理和真实任务入口将在后续阶段接入。</p><p>AI 配置页只管理应用自有 profile，不会读取或修改外部 cc-switch。</p></StatePanel>
  </div>;
}
function App() {
    const { status, error: syncError, retry } = useAppStatus();
    const [page, setPage] = useState<Page>("home");
    const meta = pageMeta[page];
    const [closeError, setCloseError] = useState<PublicError | null>(null);
    const heading = useRef<HTMLHeadingElement>(null);
    useEffect(() => { heading.current?.focus(); }, [page]);
    function navigate(next: Page) { setPage(next); }
    return <div className="app-shell" data-context={status?.activeContext?.projectId ?? ""} data-stream={status?.streamId} data-sequence={status?.sequence}><a className="skip-link" href="#page-heading">跳到页面内容</a><aside className="sidebar"><div className="brand"><div className="brand-mark">PA</div><div><b>Project Assistant</b><small>本机开发工作台</small></div></div><nav aria-label="主导航"><p className="nav-heading">工作区</p>{(["home", "projects", "ai"] as Page[]).map((item) => <NavButton key={item} page={item} current={page} onSelect={navigate}/>)}<p className="nav-heading">应用</p>{(["settings", "diagnostics"] as Page[]).map((item) => <NavButton key={item} page={item} current={page} onSelect={navigate}/>)}</nav><div className="sidebar-footer"><span className="online-dot"/>桌面框架已就绪</div><button className="exit-button" type="button" onClick={() => { void closeApplication().catch(cause => setCloseError(toPublicError(cause))); }}>退出应用</button>{closeError && <ErrorNotice error={closeError} />}</aside><main className="main-area" data-page={page}><header className="topbar"><div><p className="breadcrumb">工作区 / {meta.label}</p><h1 id="page-heading" ref={heading} tabIndex={-1}>{meta.title}</h1></div><span className="version-pill">本地优先</span></header>{syncError && <ErrorNotice error={syncError} actions={<button onClick={retry}>重试同步</button>} />}{page === "home" ? <Home onSelect={navigate}/> : page === "diagnostics" ? <DiagnosticsPage status={status} /> : page === "settings" ? <SettingsPage /> : page === "ai" ? <AiConfigPage /> : <Placeholder page={page} onSelect={navigate}/>}</main></div>;
}
export default App;
