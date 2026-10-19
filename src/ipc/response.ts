import type { Response, ProbeReply, DesktopReady } from "./contracts.ts";
import { errorForCode, isCorrelationId } from "./errors.ts";

// Transport failures never expose native/plugin error strings to UI or console.
export const TRANSPORT_ERROR = errorForCode("PA_IPC_TRANSPORT_FAILED").message;
export const CONTRACT_ERROR = errorForCode("PA_IPC_RESPONSE_INVALID").message;
export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
export function parseResponse<T>(value: unknown, accepts: (data: unknown) => data is T): Response<T> {
  if (!isRecord(value) || Object.keys(value).length !== 4 || value.apiVersion !== 1 || !isCorrelationId(value.correlationId)) throw errorForCode("PA_IPC_RESPONSE_INVALID");
  if (value.status === "ok" && !("error" in value) && accepts(value.data)) return value as unknown as Response<T>;
  if (value.status === "error" && !("data" in value) && isRecord(value.error)) {
    const e = value.error;
    if (Object.keys(e).length === 5 && typeof e.code === "string" && /^PA_[A-Z0-9_]{1,96}$/.test(e.code) &&
      typeof e.category === "string" && ["validation", "permission", "environment", "authentication", "compatibility", "conflict", "timeout", "cancelled", "external", "storage", "internal"].includes(e.category) &&
      typeof e.message === "string" && e.message.length <= 512 && typeof e.suggestion === "string" && e.suggestion.length <= 512 && typeof e.retryable === "boolean") return value as unknown as Response<T>;
  }
  throw errorForCode("PA_IPC_RESPONSE_INVALID");
}
export function isProbeReply(value: unknown): value is ProbeReply {
  return isRecord(value) && typeof value.message === "string" && typeof value.platform === "string" && typeof value.architecture === "string";
}
export function isDesktopReady(value: unknown): value is DesktopReady {
  return isRecord(value) && Object.keys(value).length === 1 && ["normal", "phase0"].includes(String(value.mode));
}
export function unwrap<T>(response: Response<T>): T {
  if (response.status === "error") throw errorForCode(response.error.code, response.correlationId);
  return response.data;
}
