import { Component, lazy, Suspense, useEffect, useState } from "react";
import type { ReactNode } from "react";
import App from "../App";
import { StatePanel } from "../components/StatePanel";
import { closeApplication, initializeDesktop } from "../ipc/client";
import type { DesktopReady } from "../ipc/contracts";
const Phase0Page = lazy(() => import("../pages/Phase0Page"));
// Share the read-only startup request across StrictMode effect remounts.
let initialRequest: ReturnType<typeof initializeDesktop> | undefined;
type StartupState = {
    status: "loading";
} | {
    status: "ready";
    ready: DesktopReady;
} | {
    status: "error";
    message: string;
};
export default function Bootstrap() {
    const [attempt, setAttempt] = useState(0);
    const [state, setState] = useState<StartupState>({ status: "loading" });
    const [closeError, setCloseError] = useState("");
    useEffect(() => {
        let active = true;
        setState({ status: "loading" });
        const request = attempt === 0 ? (initialRequest ??= initializeDesktop()) : initializeDesktop();
        void request.then((ready) => { if (active)
            setState({ status: "ready", ready }); }, (error: unknown) => { if (active)
            setState({ status: "error", message: error instanceof Error ? error.message : "PA_DESKTOP_INITIALIZATION_FAILED：请重新启动应用。" }); });
        return () => { active = false; };
    }, [attempt]);
    if (state.status === "loading")
        return <div className="startup"><StatePanel kind="loading" title="正在启动本机项目助手"><p>正在连接本地桌面服务…</p></StatePanel></div>;
    if (state.status === "error")
        return <div className="startup"><StatePanel kind="error" title="桌面服务暂时不可用" actions={<><button className="primary-button" onClick={() => setAttempt((value) => value + 1)}>重试初始化</button><button className="quiet-button" onClick={() => { void closeApplication().catch((error: Error) => setCloseError(error.message)); }}>退出应用</button></>}>
    <p data-testid="startup-error">{state.message}</p>{closeError && <p role="alert">{closeError}</p>}
  </StatePanel></div>;
    if (state.ready.mode === "phase0")
        return <Suspense fallback={<StatePanel kind="loading" title="正在加载验证页面"><p>阶段 0 显式验证模式</p></StatePanel>}><Phase0Page /></Suspense>;
    return <App />;
}
export class AppBoundary extends Component<{
    children: ReactNode;
}, {
    failed: boolean;
    revision: number;
}> {
    state = { failed: false, revision: 0 };
    static getDerivedStateFromError() { return { failed: true }; }
    render() {
        if (this.state.failed)
            return <div className="startup"><StatePanel kind="error" title="页面加载失败" actions={<button className="primary-button" onClick={() => this.setState(({ revision }) => ({ failed: false, revision: revision + 1 }))}>重新加载页面</button>}>
      <p>PA_DESKTOP_RENDER_FAILED：请重试；持续失败时关闭并重新启动应用。</p>
    </StatePanel></div>;
        return <div key={this.state.revision}>{this.props.children}</div>;
    }
}
