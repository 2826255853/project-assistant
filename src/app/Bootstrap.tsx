import { Component, lazy, Suspense, useEffect, useState } from "react";
import type { ReactNode } from "react";
import App from "../App";
import { StatePanel } from "../components/StatePanel";
import { closeApplication, initializeDesktop } from "../ipc/client";
import type { DesktopReady } from "../ipc/contracts";
import { ErrorNotice } from "../components/ErrorNotice";
import { errorForCode, toPublicError } from "../ipc/errors";
import type { PublicError } from "../ipc/errors";
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
    error: PublicError;
};
export default function Bootstrap() {
    const [attempt, setAttempt] = useState(0);
    const [state, setState] = useState<StartupState>({ status: "loading" });
    const [closeError, setCloseError] = useState<PublicError | null>(null);
    useEffect(() => {
        let active = true;
        setState({ status: "loading" });
        const request = attempt === 0 ? (initialRequest ??= initializeDesktop()) : initializeDesktop();
        void request.then((ready) => { if (active)
            setState({ status: "ready", ready }); }, (error: unknown) => { if (active)
            setState({ status: "error", error: toPublicError(error) }); });
        return () => { active = false; };
    }, [attempt]);
    if (state.status === "loading")
        return <div className="startup"><StatePanel kind="loading" title="正在启动本机项目助手"><p>正在连接本地桌面服务…</p></StatePanel></div>;
    if (state.status === "error")
        return <div className="startup"><StatePanel kind="error" title="桌面服务暂时不可用" actions={<><button className="primary-button" onClick={() => { setCloseError(null); setAttempt((value) => value + 1); }}>重试初始化</button><button className="quiet-button" onClick={() => { void closeApplication().catch(cause => setCloseError(toPublicError(cause))); }}>退出应用</button></>}>
    <ErrorNotice error={state.error} testId="startup-error" />{closeError && <ErrorNotice error={closeError} />}
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
      <ErrorNotice error={errorForCode("PA_DESKTOP_RENDER_FAILED")} />
    </StatePanel></div>;
        return <div key={this.state.revision}>{this.props.children}</div>;
    }
}
