import type { Response, ProbeReply, DesktopReady } from "./contracts.ts";

// Transport failures never expose native/plugin error strings to UI or console.
export const TRANSPORT_ERROR = "PA_IPC_TRANSPORT_FAILED：本地服务调用失败，请重试；持续失败时重启应用。";
export const CONTRACT_ERROR = "PA_IPC_RESPONSE_INVALID：服务返回格式不兼容，请更新应用。";
export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
export function parseResponse<T>(value: unknown, accepts: (data: unknown) => data is T): Response<T> {
  if (!isRecord(value) || value.apiVersion !== 1 || typeof value.correlationId !== "string" || !/^ipc-[a-z0-9-]{1,96}$/.test(value.correlationId)) throw new Error(CONTRACT_ERROR);
  if (value.status === "ok" && !("error" in value) && accepts(value.data)) return value as unknown as Response<T>;
  if (value.status === "error" && !("data" in value) && isRecord(value.error)) {
    const e = value.error;
    if (typeof e.code === "string" && /^PA_[A-Z0-9_]+$/.test(e.code) &&
      typeof e.category === "string" && ["validation", "permission", "environment", "authentication", "compatibility", "conflict", "timeout", "cancelled", "external", "storage", "internal"].includes(e.category) &&
      typeof e.message === "string" && e.message.length <= 512 && typeof e.suggestion === "string" && e.suggestion.length <= 512 && typeof e.retryable === "boolean") return value as unknown as Response<T>;
  }
  throw new Error(CONTRACT_ERROR);
}
export function isProbeReply(value: unknown): value is ProbeReply {
  return isRecord(value) && typeof value.message === "string" && typeof value.platform === "string" && typeof value.architecture === "string";
}
export function isDesktopReady(value: unknown): value is DesktopReady {
  return isRecord(value) && Object.keys(value).length === 1 && ["normal", "phase0"].includes(String(value.mode));
}
export function unwrap<T>(response: Response<T>): T {
  if (response.status === "error") throw new Error(`${response.error.code}：${response.error.message} ${response.error.suggestion}（关联 ID：${response.correlationId}）`);
  return response.data;
}
