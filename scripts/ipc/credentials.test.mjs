import assert from "node:assert/strict";
import test from "node:test";
import { isCredentialMutation } from "../../src/ipc/credentials.ts";
import { parseResponse } from "../../src/ipc/response.ts";

test("credential responses accept only bounded state and revision, never Key material", () => {
  const valid = {credential: {status: "configured_unverified", revision: 1}, configRevision: 2};
  for (const status of ["missing", "configured_unverified", "verified", "invalid", "external_unavailable"]) {
    assert.equal(isCredentialMutation({...valid, credential: {...valid.credential, status}}), true);
  }
  for (const value of [null, [], {}, {...valid, secret: "synthetic-key"}, {...valid, credential: {...valid.credential, secret: "synthetic-key"}},
    {...valid, credential: {...valid.credential, status: "saved"}},
    ...[-1, 1.5, Infinity, Number.MAX_SAFE_INTEGER + 1, "1"].map(revision => ({...valid, credential: {...valid.credential, revision}})),
    ...[0, -1, 1.5, Infinity, Number.MAX_SAFE_INTEGER + 1, "1"].map(configRevision => ({...valid, configRevision}))]) {
    assert.equal(isCredentialMutation(value), false);
    assert.throws(() => parseResponse({apiVersion: 1, correlationId: "ipc-1-ab-0", status: "ok", data: value}, isCredentialMutation), error => error.code === "PA_IPC_RESPONSE_INVALID");
  }
});
