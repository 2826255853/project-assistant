import type { ImplementedCommandMap } from "./contracts.ts";
import { isRecord } from "./response.ts";

export function isCredentialMutation(value: unknown): value is ImplementedCommandMap["credential_put"]["data"] {
  if (!isRecord(value) || Object.keys(value).length !== 2 || !isRecord(value.credential)) return false;
  const credential = value.credential;
  return Object.keys(credential).length === 2 &&
    typeof credential.revision === "number" && Number.isSafeInteger(credential.revision) && credential.revision >= 0 &&
    typeof credential.status === "string" && ["missing", "configured_unverified", "verified", "invalid", "external_unavailable"].includes(credential.status) &&
    typeof value.configRevision === "number" && Number.isSafeInteger(value.configRevision) && value.configRevision >= 1;
}
