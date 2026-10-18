import type { SettingsView } from "./contracts.ts";
import { isRecord } from "./response.ts";

export function isSettingsView(value: unknown): value is SettingsView {
  if (!isRecord(value) || Object.keys(value).length !== 2 || !Number.isSafeInteger(value.revision) || Number(value.revision) < 1 || !Array.isArray(value.values) || value.values.length !== 10) return false;
  const keys = new Set<string>();
  for (const change of value.values) {
    if (!isRecord(change) || Object.keys(change).length !== 2 || typeof change.key !== "string" || keys.has(change.key)) return false;
    keys.add(change.key);
    const v = change.value;
    switch (change.key) {
      case "search_engine": if (v !== "bing" && v !== "google") return false; break;
      case "project_sort": if (v !== "recent" && v !== "name") return false; break;
      case "approval_policy": if (v !== "ask") return false; break;
      case "log_level": if (!["error", "warn", "info", "debug"].includes(String(v))) return false; break;
      case "summary_retention_days": if (!Number.isInteger(v) || Number(v) < 1 || Number(v) > 365) return false; break;
      case "editor": if (v !== null && (!isRecord(v) || Object.keys(v).length !== 2 || v.id !== "custom" || typeof v.executable !== "string" || v.executable.length > 1024)) return false; break;
      case "ai_defaults": if (v !== null) return false; break;
      case "overlay_enabled": if (typeof v !== "boolean") return false; break;
      case "overlay_shortcut": if (typeof v !== "string" || v.length < 1 || v.length > 80) return false; break;
      case "overlay_position":
        if (v !== null && (!isRecord(v) || Object.keys(v).length !== 4 || !Number.isInteger(v.x) || !Number.isInteger(v.y) || Number(v.x) < -2147483648 || Number(v.x) > 2147483647 || Number(v.y) < -2147483648 || Number(v.y) > 2147483647 || typeof v.scaleFactor !== "number" || !Number.isFinite(v.scaleFactor) || v.scaleFactor < 0.25 || v.scaleFactor > 8 || (v.monitor !== null && (typeof v.monitor !== "string" || v.monitor.length > 256)))) return false;
        break;
      default: return false;
    }
  }
  return true;
}
