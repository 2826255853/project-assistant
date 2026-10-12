import assert from "node:assert/strict";
import test from "node:test";
import { nonOfficialConnection } from "./environment.mjs";

const source = (provider, endpoint, auth = "false") => `model_provider = "${provider}"\n[model_providers.${provider}]\nbase_url = "${endpoint}"\nwire_api = "responses"\nrequires_openai_auth = ${auth}\n`;
test("real probe fails closed for official or ambiguous configuration before starting CLI", () => {
  for (const config of ["", source("openai", "https://api.example.com"), source("custom", "https://api.openai.com/v1"),
    source("custom", "https://api.example.com", "true"), source("custom", "https://api.example.com").replace("requires_openai_auth = false", "")]) {
    assert.throws(() => nonOfficialConnection(config), { code: "OFFICIAL_SOURCE_TESTS_PAUSED" });
  }
  assert.equal(nonOfficialConnection(source("custom", "https://api.example.com")).provider, "custom");
  for (const endpoint of ["http://example.com", "https://secret@example.com", "https://example.com?key=secret"]) {
    assert.throws(() => nonOfficialConnection(source("custom", endpoint)), { code: "PROVIDER_ENDPOINT_INVALID" });
  }
});
