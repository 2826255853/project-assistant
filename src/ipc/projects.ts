// Phase 2 project contract validators. No function in this module is wired to
// the IPC client until the matching Rust service and capability are shipped.
import type {
  ActiveProject, GitSummary, ProjectAvailability, ProjectAuthorization, ProjectListPage,
  ProjectSettingsView, ProjectView, CommandMap,
} from "./contracts.ts";
import { isRecord } from "./response.ts";

const keys = (value: Record<string, unknown>, expected: string[]) =>
  Object.keys(value).length === expected.length && expected.every(key => key in value);
const bytes = (value: string) => new TextEncoder().encode(value).length;
const uuid = (value: unknown): value is string => typeof value === "string" &&
  /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(value);
const id = (value: unknown): value is string => typeof value === "string" && /^[A-Za-z0-9_-]{1,128}$/.test(value);
const revision = (value: unknown): value is number => Number.isSafeInteger(value) && Number(value) >= 1;
const timestamp = (value: unknown): value is number => Number.isSafeInteger(value) && Number(value) >= 0;
const boundedText = (value: unknown, limit: number): value is string => typeof value === "string" && bytes(value) <= limit && !/[\u0000-\u001f]/.test(value);
const availability = (value: unknown): value is ProjectAvailability => ["available", "missing", "inaccessible", "identity_changed"].includes(String(value));
const authorization = (value: unknown): value is ProjectAuthorization => ["granted", "requires_reauthorization", "revoked"].includes(String(value));
function isAiSelection(value: unknown): boolean {
  return isRecord(value) && keys(value, ["profileId", "expectedRevision", "modelId", "effort"]) &&
    id(value.profileId) && revision(value.expectedRevision) &&
    typeof value.modelId === "string" && /^[A-Za-z0-9_.-]{1,128}$/.test(value.modelId) &&
    (value.effort === null || (typeof value.effort === "string" && /^[A-Za-z0-9_.-]{1,128}$/.test(value.effort)));
}

function isProjectSetting(value: unknown): boolean {
  if (!isRecord(value) || !keys(value, ["key", "value", "source"]) || !["project", "global"].includes(String(value.source))) return false;
  switch (value.key) {
    case "search_engine": return value.value === "bing" || value.value === "google";
    case "editor":
      return value.value === null || (isRecord(value.value) && keys(value.value, ["id", "executable"]) &&
        value.value.id === "custom" && typeof value.value.executable === "string" && value.value.executable.length <= 1024 &&
        /^[A-Za-z]:\\/.test(value.value.executable) && value.value.executable.toLowerCase().endsWith(".exe"));
    case "project_sort": return value.value === "recent" || value.value === "name";
    case "approval_policy": return value.value === "ask";
    case "log_level": return ["error", "warn", "info", "debug"].includes(String(value.value));
    case "summary_retention_days": return Number.isSafeInteger(value.value) && Number(value.value) >= 1 && Number(value.value) <= 365;
    case "ai_defaults": return value.value === null || isAiSelection(value.value);
    default: return false;
  }
}

export function isProjectView(value: unknown): value is ProjectView {
  if (!isRecord(value) || !keys(value, ["id", "name", "displayName", "absolutePath", "availability", "authorization", "revision", "createdAt", "lastUsedAt"])) return false;
  return uuid(value.id) && boundedText(value.name, 1024) && value.name.length > 0 &&
    (value.displayName === null || boundedText(value.displayName, 1024)) &&
    boundedText(value.absolutePath, 4096) && value.absolutePath.length > 0 &&
    availability(value.availability) && authorization(value.authorization) && revision(value.revision) &&
    timestamp(value.createdAt) && (value.lastUsedAt === null || timestamp(value.lastUsedAt));
}

export function isProjectListPage(value: unknown): value is ProjectListPage {
  return isRecord(value) && keys(value, ["items", "nextCursor"]) && Array.isArray(value.items) &&
    value.items.length <= 100 && value.items.every(isProjectView) && new Set(value.items.map(item => item.id)).size === value.items.length &&
    (value.nextCursor === null || (boundedText(value.nextCursor, 512) && value.nextCursor.length > 0));
}

export function isActiveProject(value: unknown): value is ActiveProject {
  return isRecord(value) && keys(value, ["projectId", "name"]) && uuid(value.projectId) && boundedText(value.name, 1024) && value.name.length > 0;
}

export function isGitSummary(value: unknown): value is GitSummary {
  if (!isRecord(value) || !keys(value, ["state", "isRepository", "branch", "detached", "workingTreeModified", "refreshedAt"])) return false;
  if (value.state === "available") {
    return value.isRepository === true && (value.branch === null || boundedText(value.branch, 512)) &&
      typeof value.detached === "boolean" && typeof value.workingTreeModified === "boolean" && timestamp(value.refreshedAt);
  }
  return ["not_repository", "git_unavailable", "timed_out", "failed"].includes(String(value.state)) &&
    (value.state === "not_repository" ? value.isRepository === false : value.isRepository === null) &&
    value.branch === null && value.detached === false && value.workingTreeModified === null &&
    (value.refreshedAt === null || timestamp(value.refreshedAt));
}

export function isProjectSettingsView(value: unknown): value is ProjectSettingsView {
  if (!isRecord(value) || !keys(value, ["projectId", "revision", "globalRevision", "values"]) ||
    !uuid(value.projectId) || !revision(value.revision) || !revision(value.globalRevision) || !Array.isArray(value.values) || value.values.length > 16) return false;
  return value.values.every(isProjectSetting) && new Set(value.values.map(setting => isRecord(setting) ? setting.key : null)).size === value.values.length;
}

export function isProjectPickerResult(value: unknown): value is CommandMap["projects_add_from_picker"]["data"] {
  if (!isRecord(value) || typeof value.outcome !== "string") return false;
  if (value.outcome === "cancelled") return keys(value, ["outcome"]);
  return value.outcome === "registered" && keys(value, ["outcome", "project"]) && isProjectView(value.project);
}

export function isSelectedProject(value: unknown): value is CommandMap["project_select"]["data"] {
  return isRecord(value) && keys(value, ["activeProject"]) && isActiveProject(value.activeProject);
}

export function isRemovedProject(value: unknown): value is CommandMap["project_remove"]["data"] {
  return isRecord(value) && keys(value, ["removed", "retainedForHistory"]) &&
    value.removed === true && typeof value.retainedForHistory === "boolean";
}

export function isProjectOpenAccepted(value: unknown): value is CommandMap["project_open_directory"]["data"] {
  return isRecord(value) && keys(value, ["systemAccepted"]) && value.systemAccepted === true;
}

export function isProjectFilePickerResult(value: unknown): value is CommandMap["project_pick_file"]["data"] {
  if (!isRecord(value)) return false;
  if (value.outcome === "cancelled") return keys(value, ["outcome"]);
  if (!keys(value, ["relativePath"]) || typeof value.relativePath !== "string" || bytes(value.relativePath) > 4096) return false;
  return validRelativePath(value.relativePath);
}

export function isProjectListParams(value: unknown): value is CommandMap["projects_list"]["params"] {
  if (!isRecord(value) || Object.keys(value).some(key => !["query", "cursor", "limit"].includes(key))) return false;
  if ("query" in value && (!boundedText(value.query, 256) || /[\u0000-\u001f]/.test(value.query))) return false;
  if ("cursor" in value && (!boundedText(value.cursor, 512) || value.cursor.length === 0)) return false;
  return !("limit" in value) || (Number.isSafeInteger(value.limit) && Number(value.limit) >= 1 && Number(value.limit) <= 100);
}

export function isProjectFileParams(value: unknown): value is CommandMap["project_open_file"]["params"] {
  return isRecord(value) && keys(value, ["projectId", "relativePath"]) && uuid(value.projectId) &&
    typeof value.relativePath === "string" && bytes(value.relativePath) <= 4096 && validRelativePath(value.relativePath);
}

function validRelativePath(value: string): boolean {
  return value.length > 0 && !/[\u0000-\u001f:]/.test(value) && !/^[\\/]/.test(value) &&
    value.split(/[\\/]/).every(part => part.length > 0 && part !== "." && part !== ".." && validWindowsComponent(part));
}

function validWindowsComponent(value: string): boolean {
  if (/[<>"|?*]/.test(value) || /[. ]$/.test(value)) return false;
  const base = value.split(".", 1)[0]!.toUpperCase();
  return !/^(CON|PRN|AUX|NUL|CONIN\$|CONOUT\$|COM[1-9¹²³]|LPT[1-9¹²³])$/.test(base);
}

export const projectCommands: readonly (keyof CommandMap)[] = [
  "projects_add_from_picker", "projects_list", "project_get", "project_select", "project_remove",
  "project_open_directory", "project_pick_file", "project_open_file", "project_git_summary",
];

export const projectErrorCodes = [
  "PA_PROJECT_NOT_FOUND", "PA_PROJECT_PICKER_FAILED", "PA_PROJECT_PATH_UNSAFE", "PA_PROJECT_PATH_UNAVAILABLE",
  "PA_PROJECT_PATH_CHANGED", "PA_PROJECT_FILE_OUTSIDE_PROJECT", "PA_PROJECT_FILE_MISSING", "PA_PROJECT_ACCESS_DENIED",
  "PA_PROJECT_OPEN_REJECTED", "PA_PROJECT_OPEN_TIMEOUT", "PA_PROJECT_CONFLICT_REVISION",
] as const;
