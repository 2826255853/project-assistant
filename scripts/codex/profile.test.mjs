import assert from "node:assert/strict";
import test from "node:test";
import { profileSnapshot } from "./profile.mjs";

test("profile snapshots reject another site's key before any network request", () => {
  const profile = { id: "a", endpoint: "https://a.example/v1" };
  const selection = { model: "one", effort: "high" };
  assert.throws(() => profileSnapshot(profile, { profileId: "b", origin: "https://a.example" }, selection), { code: "KEY_SITE_MISMATCH" });
  assert.throws(() => profileSnapshot(profile, { profileId: "a", origin: "https://b.example" }, selection), { code: "KEY_SITE_MISMATCH" });
  const snapshot = profileSnapshot(profile, { profileId: "a", origin: "https://a.example", reference: "credential:a" }, selection);
  profile.endpoint = "https://changed.example";
  assert.equal(snapshot.endpoint, "https://a.example/v1");
  assert.ok(Object.isFrozen(snapshot));
  assert.deepEqual(Object.keys(snapshot).sort(), ["credentialReference", "effort", "endpoint", "model", "profileId"]);
});
test("endpoints reject inline credentials, query tokens and insecure remote transport", () => {
  const credential = { profileId: "a", origin: "https://a.example", reference: "ref" };
  for (const endpoint of ["http://a.example", "https://secret@a.example", "https://a.example?key=private", "https://a.example#private", "invalid"]) {
    assert.throws(() => profileSnapshot({ id: "a", endpoint }, credential, { model: "one", effort: "high" }), { code: "PROVIDER_ENDPOINT_INVALID" });
  }
});
