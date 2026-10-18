import type { ActiveContext, AppEvent, AppStatus, OverlayView } from "./contracts.ts";
import { isRecord } from "./response.ts";
import { errorForCode } from "./errors.ts";

const bytes = (value: string) => new TextEncoder().encode(value).length;
const identifier = (value: unknown): value is string => typeof value === "string" && /^[a-zA-Z0-9_-]{1,128}$/.test(value);
const cursorId = (value: unknown): value is string => typeof value === "string" && /^stream-ipc-[a-z0-9-]{1,128}$/.test(value);
const counter = (value: unknown): value is number => Number.isSafeInteger(value) && Number(value) >= 0;
export function isOverlayView(value: unknown): value is OverlayView {
  return isRecord(value) && typeof value.enabled === "boolean" && typeof value.expanded === "boolean" &&
    typeof value.shortcut === "string" && bytes(value.shortcut) <= 80 && typeof value.draft === "string" && bytes(value.draft) <= 32768 &&
    typeof value.trayReady === "boolean" && [value.shortcutError, value.windowError].every(v => v === null || (typeof v === "string" && /^PA_[A-Z0-9_]{1,96}$/.test(v)));
}
function isContext(value: unknown): value is ActiveContext {
  return isRecord(value) && [value.projectId, value.sessionId, value.profileId].every(identifier) &&
    typeof value.modelId === "string" && bytes(value.modelId) > 0 && bytes(value.modelId) <= 128 && !/[\u0000-\u001f]/.test(value.modelId) &&
    (value.effort === null || identifier(value.effort));
}
export function isAppStatus(value: unknown): value is AppStatus {
  const ids = ["desktop", "ipc", "events", "overlay", "storage", "settings", "logging", "credentials", "providers", "codex", "projects", "tasks"];
  if (!isRecord(value) || value.apiVersion !== 1 || !cursorId(value.streamId) || !counter(value.sequence) || value.configState !== "unconfigured" ||
    value.taskStatus !== "idle" || !isOverlayView(value.overlay) || !(value.activeContext === null || isContext(value.activeContext)) ||
    !Array.isArray(value.services) || value.services.length !== ids.length || !isRecord(value.ccSwitch)) return false;
  const cc = value.ccSwitch;
  return cc.management === "user_manual" && cc.integration === "disabled" && cc.readSupported === false && cc.switchSupported === false &&
    new Set(value.services.map(s => isRecord(s) ? s.id : null)).size === ids.length && value.services.every(s => isRecord(s) && ids.includes(String(s.id)) &&
      ["ready", "unavailable", "degraded"].includes(String(s.state)) &&
      (s.state === "ready" ? s.reasonCode === null : typeof s.reasonCode === "string" && /^PA_[A-Z0-9_]{1,96}$/.test(s.reasonCode)));
}
export function isStateEvent(value: unknown): value is AppEvent {
  return isRecord(value) && value.apiVersion === 1 && cursorId(value.streamId) && counter(value.sequence) && value.sequence > 0 &&
    typeof value.eventId === "string" && value.eventId === `${value.streamId}-${value.sequence}` && counter(value.occurredAt) &&
    typeof value.correlationId === "string" && /^ipc-[a-z0-9-]{1,96}$/.test(value.correlationId) &&
    isRecord(value.payload) && Object.keys(value.payload).length === 1 && value.payload.type === "app_state_changed" && bytes(JSON.stringify(value)) <= 16384;
}

export interface StateTransport {
  listen: (callback: (event: unknown) => void) => Promise<() => void>;
  query: () => Promise<AppStatus>;
}

// Subscribe before querying. Invalidation events always fetch authoritative state;
// this also recovers gaps and process restarts without replaying a mutation.
export function subscribeState(transport: StateTransport, update: (status: AppStatus) => void, failed: (error: unknown) => void = () => {}) {
  let active = true;
  let unlisten: (() => void) | undefined;
  let current: AppStatus | undefined;
  let pending = false;
  let querying = false;
  let generation = 0;
  let expected: { streamId: string; sequence: number } | undefined;
  const retiredStreams = new Set<string>();
  async function refresh() {
    pending = true;
    if (querying || !active) return;
    querying = true;
    try {
      while (active && pending) {
        pending = false;
        const token = generation;
        const status = await transport.query();
        if (!active) break;
        if (token !== generation) { pending = true; continue; }
        if (expected && (status.streamId !== expected.streamId || status.sequence < expected.sequence)) {
          throw errorForCode("PA_IPC_STATE_STALE");
        }
        if (!current || current.streamId !== status.streamId || status.sequence >= current.sequence) {
          if (current && current.streamId !== status.streamId) {
            retiredStreams.add(current.streamId);
            if (retiredStreams.size > 8) retiredStreams.delete(retiredStreams.values().next().value!);
          }
          current = status; update(status);
        }
      }
    } catch (error) { if (active) failed(error); }
    finally { querying = false; }
  }
  void transport.listen(event => {
    if (!active) return;
    if (!isStateEvent(event)) { generation++; void refresh(); return; }
    if (retiredStreams.has(event.streamId)) return;
    const seen = Math.max(current?.streamId === event.streamId ? current.sequence : -1,
      expected?.streamId === event.streamId ? expected.sequence : -1);
    if (event.sequence <= seen) return;
    expected = { streamId: event.streamId, sequence: event.sequence };
    generation++; void refresh();
  }).then(cleanup => {
    if (!active) { cleanup(); return; }
    unlisten = cleanup; void refresh();
  }).catch(error => { if (active) failed(error); });
  return () => { active = false; pending = false; unlisten?.(); unlisten = undefined; };
}
