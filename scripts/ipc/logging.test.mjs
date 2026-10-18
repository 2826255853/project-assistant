import assert from "node:assert/strict";
import test from "node:test";
import { isLogStatus } from "../../src/ipc/logging.ts";
import { parseResponse, unwrap } from "../../src/ipc/response.ts";

const status = {
  enabled: true, level: "info", errorCode: null, fileCount: 1, totalBytes: 128,
  auditRetentionDays: 90, summaryCollectionEnabled: false,
};

test("log status accepts healthy, disabled and post-cleanup whitelist summaries", () => {
  for (const value of [status, { ...status, enabled: false, errorCode: "PA_INTERNAL_UNEXPECTED" },
    { ...status, fileCount: 0, totalBytes: 0 }, ...["error", "warn", "info", "debug"].map(level => ({ ...status, level }))]) {
    assert.equal(isLogStatus(value), true);
    const wire = { apiVersion: 1, correlationId: "ipc-1-ab-0", status: "ok", data: value };
    assert.deepEqual(unwrap(parseResponse(wire, isLogStatus)), value);
  }
});

test("log status rejects extra data, invalid counters, retention and enabled summaries", () => {
  const secret = "synthetic-secret-DO-NOT-DISPLAY";
  const missing = { ...status }; delete missing.enabled;
  const invalid = [null, [], missing, { ...status, path: secret }, { ...status, rawError: secret },
    { ...status, level: "trace" }, { ...status, level: {} }, { ...status, enabled: "true" },
    { ...status, errorCode: secret }, { ...status, errorCode: "PA_LOG_CLEANUP_FAILED" },
    { ...status, auditRetentionDays: 30 }, { ...status, auditRetentionDays: "90" },
    { ...status, summaryCollectionEnabled: true }, { ...status, summaryCollectionEnabled: "false" }];
  for (const field of ["fileCount", "totalBytes"]) {
    for (const value of [-1, 0.5, NaN, Infinity, Number.MAX_SAFE_INTEGER + 1, "0", null, {}]) invalid.push({ ...status, [field]: value });
  }
  for (const value of invalid) {
    assert.equal(isLogStatus(value), false);
    assert.throws(() => parseResponse({ apiVersion: 1, correlationId: "ipc-1-ab-0", status: "ok", data: value }, isLogStatus), error => {
      assert.equal(error.code, "PA_IPC_RESPONSE_INVALID");
      assert.equal(error.message.includes(secret), false);
      return true;
    });
  }
});
