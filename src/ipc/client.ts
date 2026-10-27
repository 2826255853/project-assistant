import { invoke, transformCallback } from "@tauri-apps/api/core";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { ImplementedCommandMap, OverlayAction } from "./contracts";
import { API_VERSION } from "./contracts";
import { isDesktopReady, isProbeReply, parseResponse, unwrap } from "./response";
import { awaitTransport } from "./transport";
import { isAppStatus, isOverlayView } from "./state";
import { isSettingsView } from "./settings";
import { isLogStatus } from "./logging";
import { isCredentialMutation } from "./credentials";
import { isProviderList, isProviderProfile, isModels, isReceipt, isOperation, isRemoved, isAccepted } from "./providers";
import { errorForCode } from "./errors";
import { isProjectView, isProjectPickerResult, isRemovedProject } from "./projects";

type Name = keyof ImplementedCommandMap;
const validators: { [K in Name]: (data: unknown) => data is ImplementedCommandMap[K]["data"] } = {
  projects_add_from_picker: isProjectPickerResult, project_get: isProjectView, project_remove: isRemovedProject,
  providers_list: isProviderList, provider_save: isProviderProfile, provider_set_enabled: isProviderProfile,
  provider_delete: isRemoved, provider_models: isModels, provider_test: isReceipt, provider_switch: isReceipt,
  operation_get: isOperation, operation_cancel: isAccepted,
  framework_probe: isProbeReply, desktop_initialize: isDesktopReady,
  app_get_status: isAppStatus, overlay_get: isOverlayView,
  overlay_control: (data): data is null => data === null,
  settings_get: isSettingsView, settings_update: isSettingsView,
  logs_get_status: isLogStatus, logs_clear: isLogStatus,
  credential_put: isCredentialMutation, credential_remove: isCredentialMutation,
};
export async function call<K extends Name>(command: K, params: ImplementedCommandMap[K]["params"], options?: { signal?: AbortSignal; timeoutMs?: number }): Promise<ImplementedCommandMap[K]["data"]> {
  if ((command === "logs_get_status" || command === "logs_clear" || command === "credential_put" || command === "credential_remove" || command.startsWith("project") || command.startsWith("provider") || command.startsWith("operation_")) && !isMainWindow()) throw errorForCode("PA_IPC_PERMISSION_DENIED");
  const wire = await awaitTransport(() => invoke<unknown>(command, { request: { apiVersion: API_VERSION, params } }), options);
  const result = unwrap(parseResponse(wire, validators[command]));
  // Nested errors inside successful operation envelopes obey the same local
  // presentation rule as envelope errors: never render wire/remote text.
  if (command === "operation_get") {
    const op = result as ImplementedCommandMap["operation_get"]["data"];
    if (op.error) op.error = errorForCode(op.error.code);
    if (op.result && "outcome" in op.result && op.result.outcome !== "applied") op.result.error = errorForCode(op.result.error.code);
  }
  return result;
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
export const isMainWindow = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window && getCurrentWindow().label === "main";
export const getLogsStatus = () => call("logs_get_status", {});
export const clearLogs = () => call("logs_clear", {});
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
    throw errorForCode("PA_IPC_TRANSPORT_FAILED");
  }
}
export const dragOverlay = () => getCurrentWindow().startDragging();
