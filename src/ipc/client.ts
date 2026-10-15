import { invoke, transformCallback } from "@tauri-apps/api/core";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { ImplementedCommandMap, OverlayAction } from "./contracts";
import { API_VERSION } from "./contracts";
import { isDesktopReady, isProbeReply, parseResponse, unwrap } from "./response";
import { awaitTransport } from "./transport";
import { isAppStatus, isOverlayView } from "./state";
import { isSettingsView } from "./settings";

type Name = keyof ImplementedCommandMap;
const validators: { [K in Name]: (data: unknown) => data is ImplementedCommandMap[K]["data"] } = {
  framework_probe: isProbeReply, desktop_initialize: isDesktopReady,
  app_get_status: isAppStatus, overlay_get: isOverlayView,
  overlay_control: (data): data is null => data === null,
  settings_get: isSettingsView, settings_update: isSettingsView,
};
export async function call<K extends Name>(command: K, params: ImplementedCommandMap[K]["params"], options?: { signal?: AbortSignal; timeoutMs?: number }): Promise<ImplementedCommandMap[K]["data"]> {
  const wire = await awaitTransport(() => invoke<unknown>(command, { request: { apiVersion: API_VERSION, params } }), options);
  return unwrap(parseResponse(wire, validators[command]));
}

export async function frameworkProbe(message: string) {
  return call("framework_probe", { message });
}

export async function phase0OpenPath(path: string) {
  await invoke("phase0_check_open_path", { path });
  await openPath(path);
}

export async function phase0OpenUrl(url: URL) {
  await openUrl(url);
}

export async function initializeDesktop() {
  return call("desktop_initialize", {});
}

export async function closeApplication() {
  return call("overlay_control", { action: "exit" });
}

export const isOverlayWindow = () => "__TAURI_INTERNALS__" in window && getCurrentWindow().label === "overlay";
export async function getOverlay() { return call("overlay_get", {}); }
export async function controlOverlay(action: OverlayAction["action"], params: Record<string, string> = {}) {
  return call("overlay_control", { action, ...params } as OverlayAction);
}
export const getAppStatus = () => call("app_get_status", {});
export async function listenAppEvents(callback: (event: unknown) => void) {
  const event = "project-assistant://event/v1";
  const handler = transformCallback<{ payload: unknown }>(({ payload }) => callback(payload));
  const internals = (window as unknown as { __TAURI_INTERNALS__: { unregisterCallback: (id: number) => void }; __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: (event: string, id: number) => void } });
  const release = () => internals.__TAURI_INTERNALS__.unregisterCallback(handler);
  let active = true;
  const cleanup = async (eventId: number) => {
    if (!active) return;
    active = false;
    // Remove the JavaScript handler immediately, even if native unlisten fails.
    internals.__TAURI_EVENT_PLUGIN_INTERNALS__.unregisterListener(event, eventId);
    release();
    try { await awaitTransport(() => invoke("plugin:event|unlisten", { event, eventId })); } catch { /* fixed transport failure; component is already disposed */ }
  };
  const registration = invoke<number>("plugin:event|listen", { event, target: { kind: "WebviewWindow", label: getCurrentWindow().label }, handler });
  try {
    const eventId = await awaitTransport(() => registration);
    return () => { void cleanup(eventId); };
  } catch {
    release();
    void registration.then(eventId => cleanup(eventId), () => {});
    throw new Error("PA_IPC_TRANSPORT_FAILED：事件监听不可用，请重试同步。");
  }
}
export const dragOverlay = () => getCurrentWindow().startDragging();
