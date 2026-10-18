import assert from "node:assert/strict";
import test from "node:test";
import { errorForCode, PublicError, toPublicError } from "../../src/ipc/errors.ts";
import { isDesktopReady, parseResponse, unwrap } from "../../src/ipc/response.ts";

const secret = "synthetic-secret-DO-NOT-DISPLAY";
const failure = (code = "PA_CONFLICT_REVISION") => ({
  apiVersion: 1, correlationId: "ipc-1-ab-0", status: "error",
  error: { category: "internal", code, message: secret, suggestion: secret, retryable: false },
});

test("wire error text, category and retry flag cannot replace local presentation", () => {
  let caught;
  try { unwrap(parseResponse(failure(), isDesktopReady)); } catch (error) { caught = error; }
  assert.ok(caught instanceof PublicError);
  assert.equal(caught.code, "PA_CONFLICT_REVISION");
  assert.equal(caught.category, "conflict");
  assert.equal(caught.retryable, true);
  assert.equal(caught.correlationId, "ipc-1-ab-0");
  assert.match(caught.suggestion, /重新读取/);
  assert.equal(JSON.stringify(caught).includes(secret), false);
  assert.equal(caught.message.includes(secret), false);
});

test("unknown codes and raw exceptions become fixed internal error without coercion", () => {
  const hostile = { toString() { assert.fail("must never stringify arbitrary errors"); } };
  for (const cause of [new Error(`PA_CONFLICT_REVISION：${secret}`), secret, { code: "PA_CONFLICT_REVISION", message: secret }, hostile, null]) {
    const safe = toPublicError(cause);
    assert.equal(safe.code, "PA_INTERNAL_UNEXPECTED");
    assert.equal(safe.correlationId, null);
    assert.equal(safe.message.includes(secret), false);
  }
  for (const code of ["PA_SYNTHETIC_SECRET", "constructor", "toString", "__proto__", secret, null, hostile]) {
    const safe = errorForCode(code, "ipc-1-ab-0");
    assert.equal(safe.code, "PA_INTERNAL_UNEXPECTED");
    assert.equal(safe.correlationId, "ipc-1-ab-0");
    assert.equal(safe.message.includes(String(code === hostile ? "unused" : code)), false);
  }
  assert.throws(() => unwrap(parseResponse(failure("PA_SYNTHETIC_SECRET"), isDesktopReady)), error => {
    assert.equal(error.code, "PA_INTERNAL_UNEXPECTED");
    assert.equal(error.correlationId, "ipc-1-ab-0");
    assert.equal(error.message.includes("PA_SYNTHETIC_SECRET"), false);
    return true;
  });
});

test("public error display is rebuilt locally and retains only validated correlation", () => {
  const mutated = errorForCode("PA_LOG_CLEANUP_FAILED", "ipc-1-ab-0");
  mutated.message = secret;
  Object.assign(mutated, { suggestion: secret, userMessage: secret });
  const safe = toPublicError(mutated);
  assert.equal(safe.code, "PA_LOG_CLEANUP_FAILED");
  assert.equal(safe.correlationId, "ipc-1-ab-0");
  assert.equal(JSON.stringify(safe).includes(secret), false);
  for (const id of [secret, "ipc-a\nsecret", "ipc-" + "a".repeat(97), 1, null]) {
    assert.equal(errorForCode("PA_AUDIT_WRITE_FAILED", id).correlationId, null);
  }
  assert.match(errorForCode("PA_AUDIT_WRITE_FAILED").userMessage, /审计/);
});

test("strict response envelope rejects extra fields without echoing rejected data", () => {
  for (const wire of [
    { ...failure(), rawError: secret },
    { ...failure(), error: { ...failure().error, rawError: secret } },
    { ...failure(), correlationId: secret },
  ]) {
    assert.throws(() => parseResponse(wire, isDesktopReady), error => {
      assert.equal(error.code, "PA_IPC_RESPONSE_INVALID");
      assert.equal(error.message.includes(secret), false);
      return true;
    });
  }
});
