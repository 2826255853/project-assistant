import { useEffect, useState } from "react";
import { getAppStatus, listenAppEvents } from "../ipc/client";
import type { AppStatus } from "../ipc/contracts";
import { subscribeState } from "../ipc/state";

export function useAppStatus() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [error, setError] = useState("");
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    setError("");
    return subscribeState({ listen: listenAppEvents, query: getAppStatus }, state => {
      setStatus(state); setError("");
    }, () => setError("PA_IPC_STATE_UNAVAILABLE：无法同步本地服务状态，请重试。"));
  }, [attempt]);
  return { status, error, retry: () => setAttempt(value => value + 1) };
}
