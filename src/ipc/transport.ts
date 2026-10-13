import { TRANSPORT_ERROR } from "./response.ts";

export const TIMEOUT_ERROR = "PA_IPC_TIMEOUT：本地服务响应超时，请查询状态；变更请求不会自动重发。";
export const CANCELLED_ERROR = "PA_IPC_CANCELLED：已停止等待，请查询状态确认操作结果。";
export async function awaitTransport<T>(send: () => Promise<T>, options: { timeoutMs?: number; signal?: AbortSignal } = {}): Promise<T> {
  if (options.signal?.aborted) throw new Error(CANCELLED_ERROR);
  let timer: ReturnType<typeof setTimeout> | undefined;
  let abort: (() => void) | undefined;
  try {
    return await Promise.race([
      Promise.resolve().then(send).catch(() => { throw new Error(TRANSPORT_ERROR); }),
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error(TIMEOUT_ERROR)), options.timeoutMs ?? 8000);
        abort = () => reject(new Error(CANCELLED_ERROR));
        options.signal?.addEventListener("abort", abort, { once: true });
      }),
    ]);
  } finally {
    clearTimeout(timer);
    if (abort) options.signal?.removeEventListener("abort", abort);
  }
}
