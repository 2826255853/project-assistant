import type { LogStatus } from "./contracts.ts";
import { isRecord } from "./response.ts";

export function isLogStatus(value: unknown): value is LogStatus {
  return isRecord(value) && Object.keys(value).length === 7 &&
    typeof value.enabled === "boolean" &&
    typeof value.level === "string" && ["error", "warn", "info", "debug"].includes(value.level) &&
    (value.errorCode === null || value.errorCode === "PA_INTERNAL_UNEXPECTED") &&
    Number.isSafeInteger(value.fileCount) && Number(value.fileCount) >= 0 &&
    Number.isSafeInteger(value.totalBytes) && Number(value.totalBytes) >= 0 &&
    value.auditRetentionDays === 90 && value.summaryCollectionEnabled === false;
}
