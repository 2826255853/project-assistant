import { useEffect, useState } from "react";
import { getAppStatus, listenAppEvents } from "../ipc/client";
import type { AppStatus } from "../ipc/contracts";
import { subscribeState } from "../ipc/state";
import { errorForCode, toPublicError } from "../ipc/errors";
import type { PublicError } from "../ipc/errors";

export function useAppStatus() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [error, setError] = useState<PublicError | null>(null);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    setError(null);
    return subscribeState({ listen: listenAppEvents, query: getAppStatus }, state => {
      setStatus(state); setError(null);
    }, cause => setError(cause instanceof Error ? toPublicError(cause) : errorForCode("PA_IPC_STATE_UNAVAILABLE")));
  }, [attempt]);
  return { status, error, retry: () => setAttempt(value => value + 1) };
}
