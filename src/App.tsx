import { useEffect, useRef, useState } from "react";
import "./App.css";
import { StatePanel } from "./components/StatePanel";
import { closeApplication, controlOverlay } from "./ipc/client";
import { useOverlay } from "./app/useOverlay";
import { useAppStatus } from "./app/useAppStatus";
import type { AppStatus, OverlayAction } from "./ipc/contracts";
type Page = "home" | "projects" | "ai" | "settings" | "diagnostics";
const pageMeta: Record<Page, {
    label: string;
    title: string;
    description: string;
}> = {
    home: { label: "概览", title: "工作台", description: "从这里进入项目、配置运行环境或查看最近状态。" },
    projects: { label: "项目", title: "项目", description: "近期项目、添加项目和项目搜索将在项目管理功能上线后开放。" },
    ai: { label: "AI 配置", title: "AI 配置", description: "服务源和 Key 管理将在凭据保护、连接隔离与配置服务就绪后开放。" },
    settings: { label: "设置", title: "设置", description: "设置保存功能正在准备中，当前显示默认值预览。" },
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
    <section className="welcome-panel"><div><p className="eyebrow">本机项目助手</p><h2>准备开始工作</h2><p>选择一个入口继续。项目管理、AI 配置和应用设置会在功能就绪后启用。</p></div><button className="primary-button" type="button" onClick={() => onSelect("projects")}>进入项目</button></section>
    <div className="status-grid"><StatusCard title="桌面运行状态" value="正常" detail="主窗口和前端资源已加载" tone="ready"/><StatusCard title="项目工作区" value="尚未选择" detail="下一阶段接入近期项目和搜索"/><StatusCard title="AI 服务源" value="未配置" detail="应用不会读取外部 cc-switch 配置" tone="pending"/></div>
    <section className="section-block"><div className="section-heading"><div><p className="eyebrow">快速入口</p><h3>常用模块</h3></div></div><div className="quick-grid"><button type="button" onClick={() => onSelect("projects")}><span>▦</span><b>项目</b><small>查找和打开本机项目</small></button><button type="button" onClick={() => onSelect("ai")}><span>✦</span><b>AI 配置</b><small>管理应用自管运行配置</small></button><button type="button" onClick={() => onSelect("settings")}><span>⚙</span><b>设置</b><small>调整应用行为和记录</small></button></div></section>
  </div>;
}
function Placeholder({ page, onSelect }: {
    page: Page;
    onSelect: (page: Page) => void;
}) {
    const meta = pageMeta[page];
    return <div className="page-content"><StatePanel kind={page === "projects" ? "empty" : "unavailable"} title={page === "projects" ? "项目列表尚未启用" : "AI 配置服务尚未启用"} actions={<><button className="primary-button" type="button" onClick={() => onSelect("home")}>返回概览</button><button className="quiet-button" type="button" onClick={() => onSelect("diagnostics")}>查看服务状态</button></>}>
    <p>{meta.description}</p>{page === "ai" && <p>cc-switch 由你在应用外手动管理；官方源仅作预留。</p>}
  </StatePanel></div>;
}
function SettingsPage() {
    return <div className="page-content"><ResidentControls /><StatePanel kind="unavailable" title="设置保存尚未启用"><p id="settings-help">以下仅为默认值预览。设置服务就绪后可修改并保存。</p></StatePanel>
    <form className="settings-form" aria-describedby="settings-help" onSubmit={(event) => event.preventDefault()}>
      <fieldset disabled><legend>常规设置</legend><label htmlFor="search-engine">默认搜索引擎</label><select id="search-engine" defaultValue="bing"><option value="bing">Bing</option><option value="google">Google</option></select>
      <label htmlFor="project-sort">项目排序</label><select id="project-sort" defaultValue="recent"><option value="recent">最近使用</option><option value="name">名称</option></select>
      <label htmlFor="approval-policy">审批策略</label><select id="approval-policy"><option>操作前询问</option></select><button type="submit" className="primary-button">保存设置（暂不可用）</button></fieldset>
    </form></div>;
}
function ResidentControls() {
    const view = useOverlay();
    const [shortcut, setShortcut] = useState("Ctrl+Alt+Space");
    const [error, setError] = useState("");
    useEffect(() => { if (view) setShortcut(view.shortcut); }, [view?.shortcut]);
    function act(action: OverlayAction["action"], params?: Record<string, string>) {
        setError("");
        void controlOverlay(action, params).catch((cause: Error) => setError(cause.message));
    }
    return <section className="resident-settings"><h2>常驻对话入口</h2><p>启停、快捷键、悬浮位置和显示器标识会保存到本机，重启后恢复并校正越界位置。</p>
      <button data-testid="resident-toggle" className="quiet-button" disabled={!view} onClick={() => act(view?.enabled ? "disable" : "enable")}>{view?.enabled ? "停用常驻入口" : "启用常驻入口"}</button>
      <button className="quiet-button" onClick={() => act("expand")}>展开对话</button>
      <form onSubmit={(event) => { event.preventDefault(); act("set_shortcut", { shortcut }); }}>
        <label htmlFor="resident-shortcut">全局快捷键</label><input id="resident-shortcut" value={shortcut} maxLength={80} onChange={(event) => setShortcut(event.currentTarget.value)} /><button className="primary-button" type="submit">应用快捷键</button>
      </form>
      {view?.shortcutError && <p role="alert" data-testid="shortcut-error">{view.shortcutError}：快捷键无效或已被占用，请更换组合；悬浮球仍可使用。</p>}
      {(error || view?.windowError) && <p role="alert">{error || "入口置顶或位置恢复失败，请从托盘重新展开。"}</p>}
    </section>;
}
function DiagnosticsPage({ status }: { status: AppStatus | null }) {
    return <div className="page-content"><div className="status-grid">{status?.services.map(service => <StatusCard key={service.id} title={service.id} value={service.state === "ready" ? "已就绪" : service.state === "degraded" ? "需要处理" : "尚未启用"} detail={service.reasonCode ?? "本地服务可用"} tone={service.state === "ready" ? "ready" : "pending"}/>)}</div>
    <StatePanel kind="unavailable" title="业务服务正在准备中"><p>项目、设置和 AI 配置功能上线后，将在这里显示服务状态。</p><p>cc-switch 为外部手动管理，自动集成已关闭。</p></StatePanel>
  </div>;
}
function App() {
    const { status, error: syncError, retry } = useAppStatus();
    const [page, setPage] = useState<Page>("home");
    const meta = pageMeta[page];
    const [closeError, setCloseError] = useState("");
    const heading = useRef<HTMLHeadingElement>(null);
    useEffect(() => { heading.current?.focus(); }, [page]);
    function navigate(next: Page) { setPage(next); }
    return <div className="app-shell" data-context={status?.activeContext?.projectId ?? ""} data-stream={status?.streamId} data-sequence={status?.sequence}><a className="skip-link" href="#page-heading">跳到页面内容</a><aside className="sidebar"><div className="brand"><div className="brand-mark">PA</div><div><b>Project Assistant</b><small>本机开发工作台</small></div></div><nav aria-label="主导航"><p className="nav-heading">工作区</p>{(["home", "projects", "ai"] as Page[]).map((item) => <NavButton key={item} page={item} current={page} onSelect={navigate}/>)}<p className="nav-heading">应用</p>{(["settings", "diagnostics"] as Page[]).map((item) => <NavButton key={item} page={item} current={page} onSelect={navigate}/>)}</nav><div className="sidebar-footer"><span className="online-dot"/>桌面框架已就绪</div><button className="exit-button" type="button" onClick={() => { void closeApplication().catch((error: Error) => setCloseError(error.message)); }}>退出应用</button>{closeError && <p role="alert">{closeError}</p>}</aside><main className="main-area" data-page={page}><header className="topbar"><div><p className="breadcrumb">工作区 / {meta.label}</p><h1 id="page-heading" ref={heading} tabIndex={-1}>{meta.title}</h1></div><span className="version-pill">本地优先</span></header>{syncError && <p role="alert">{syncError}<button onClick={retry}>重试同步</button></p>}{page === "home" ? <Home onSelect={navigate}/> : page === "diagnostics" ? <DiagnosticsPage status={status} /> : page === "settings" ? <SettingsPage /> : <Placeholder page={page} onSelect={navigate}/>}</main></div>;
}
export default App;
