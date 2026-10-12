import { invoke } from "@tauri-apps/api/core";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";
import type { OverlayView } from "./contracts";
import { API_VERSION } from "./contracts";
import { isDesktopReady, isProbeReply, parseResponse, TRANSPORT_ERROR, unwrap } from "./response";

export async function frameworkProbe(message: string) {
  let wire: unknown;
  try {
    wire = await invoke<unknown>("framework_probe", { request: { apiVersion: API_VERSION, params: { message } } });
  } catch {
    throw new Error(TRANSPORT_ERROR);
  }
  return unwrap(parseResponse(wire, isProbeReply));
}

export async function phase0OpenPath(path: string) {
  await invoke("phase0_check_open_path", { path });
  await openPath(path);
}

export async function phase0OpenUrl(url: URL) {
  await openUrl(url);
}

export async function initializeDesktop() {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let wire: unknown;
  try {
    wire = await Promise.race([
      invoke<unknown>("desktop_initialize", { request: { apiVersion: API_VERSION, params: {} } }),
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error(TRANSPORT_ERROR)), 8000);
      }),
    ]);
  } catch {
    throw new Error(TRANSPORT_ERROR);
  } finally {
    clearTimeout(timer);
  }
  return unwrap(parseResponse(wire, isDesktopReady));
}

export async function closeApplication() {
  try { await invoke("overlay_control", { action: { action: "exit" } }); }
  catch { throw new Error("PA_DESKTOP_CLOSE_FAILED：无法关闭窗口，请使用标题栏关闭按钮。"); }
}

export const isOverlayWindow = () => "__TAURI_INTERNALS__" in window && getCurrentWindow().label === "overlay";
export async function getOverlay() { return invoke<OverlayView>("overlay_get"); }
export async function controlOverlay(action: string, params: Record<string, string> = {}) {
  try { await invoke("overlay_control", { action: { action, ...params } }); }
  catch { throw new Error("PA_OVERLAY_CONTROL_FAILED：入口操作失败，请从托盘重试。"); }
}
export const listenOverlay = (callback: (view: OverlayView) => void) => listen<OverlayView>("overlay-state", ({ payload }) => callback(payload), { target: { kind: "WebviewWindow", label: getCurrentWindow().label } });
export const dragOverlay = () => getCurrentWindow().startDragging();
