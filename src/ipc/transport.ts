import { errorForCode } from "./errors.ts";

export const TIMEOUT_ERROR = errorForCode("PA_IPC_TIMEOUT").message;
export const CANCELLED_ERROR = errorForCode("PA_IPC_CANCELLED").message;
export async function awaitTransport<T>(send: () => Promise<T>, options: { timeoutMs?: number; signal?: AbortSignal } = {}): Promise<T> {
  if (options.signal?.aborted) throw errorForCode("PA_IPC_CANCELLED");
  let timer: ReturnType<typeof setTimeout> | undefined;
  let abort: (() => void) | undefined;
  try {
    return await Promise.race([
      Promise.resolve().then(send).catch(() => { throw errorForCode("PA_IPC_TRANSPORT_FAILED"); }),
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(errorForCode("PA_IPC_TIMEOUT")), options.timeoutMs ?? 8000);
        abort = () => reject(errorForCode("PA_IPC_CANCELLED"));
        options.signal?.addEventListener("abort", abort, { once: true });
      }),
    ]);
  } finally {
    clearTimeout(timer);
    if (abort) options.signal?.removeEventListener("abort", abort);
  }
}
