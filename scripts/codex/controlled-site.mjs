// A synthetic Responses endpoint, never a real provider or official authentication test.
import { createServer } from "node:http";
import { createServer as createTlsServer } from "node:https";
import { randomUUID } from "node:crypto";

export async function controlledSite(label, key, {tls} = {}) {
  const requests = [];
  const held = [];
  let mode = "ok";
  let redirect;
  const handler = async (request, response) => {
    let bytes = 0;
    const chunks = [];
    for await (const chunk of request) {
      bytes += chunk.length;
      if (bytes > 2 * 1024 * 1024) { response.writeHead(413).end(); return; }
      chunks.push(chunk);
    }
    let payload;
    try { payload = JSON.parse(Buffer.concat(chunks).toString("utf8")); } catch { payload = {}; }
    const keyMatches = request.headers.authorization === `Bearer ${key}`;
    requests.push({ method: request.method, path: request.url?.split("?")[0], keyMatches,
      credentialPresent: Boolean(request.headers.authorization), model: payload.model,
      effort: payload.reasoning?.effort });
    if (mode === "redirect") { response.writeHead(307, { location: redirect }).end(); return; }
    if (!keyMatches || mode === "unauthorized" || mode === "echo-secret-error") {
      response.writeHead(401, { "Content-Type": "application/json" }).end(JSON.stringify({ error: { message: mode === "echo-secret-error" ? key : "Synthetic authentication failure", type: "invalid_request_error" } }));
      return;
    }
    if (!request.url?.endsWith("/responses")) { response.writeHead(404).end(); return; }
    if (mode === "hang") return; // Used only to verify client event timeouts and process cleanup.
    response.writeHead(200, { "Content-Type": "text/event-stream", "Cache-Control": "no-cache", Connection: "keep-alive" });
    const responseId = `resp_${randomUUID()}`;
    const itemId = `msg_${randomUUID()}`;
    const text = mode === "echo-secret-success" ? key : `CONTROLLED_${label}_OK`;
    const item = { id: itemId, type: "message", role: "assistant", status: "in_progress", content: [] };
    const base = { id: responseId, object: "response", created_at: Math.floor(Date.now() / 1000), model: payload.model, status: "in_progress", output: [] };
    let sequence = 0;
    const event = (type, data) => response.write(`event: ${type}\ndata: ${JSON.stringify({ type, sequence_number: sequence++, ...data })}\n\n`);
    event("response.created", { response: base });
    event("response.output_item.added", { output_index: 0, item });
    event("response.content_part.added", { item_id: itemId, output_index: 0, content_index: 0,
      part: { type: "output_text", text: "", annotations: [] } });
    event("response.output_text.delta", { item_id: itemId, output_index: 0, content_index: 0, delta: text });
    const finish = () => {
      if (response.destroyed || response.writableEnded) return;
      const part = { type: "output_text", text, annotations: [] };
      event("response.output_text.done", { item_id: itemId, output_index: 0, content_index: 0, text });
      event("response.content_part.done", { item_id: itemId, output_index: 0, content_index: 0, part });
      const completed = { ...item, status: "completed", content: [part] };
      event("response.output_item.done", { output_index: 0, item: completed });
      event("response.completed", { response: { ...base, status: "completed", output: [completed],
        usage: { input_tokens: 10, output_tokens: 5, total_tokens: 15, input_tokens_details: { cached_tokens: 0 }, output_tokens_details: { reasoning_tokens: 0 } } } });
      response.end();
    };
    if (mode === "hold") held.push(finish); else finish();
  };
  const server = tls ? createTlsServer(tls, handler) : createServer(handler);
  await new Promise((done, reject) => { server.once("error", reject); server.listen(0, "127.0.0.1", done); });
  const baseUrl = `${tls ? "https" : "http"}://127.0.0.1:${server.address().port}/v1`;
  return { baseUrl, requests, setMode: (value, target) => { mode = value; redirect = target; },
    release: () => { held.splice(0).forEach((finish) => finish()); },
    close: async () => { server.closeAllConnections(); await new Promise((done) => server.close(done)); } };
}

export function controlledConfig(provider, sites) {
  return `model_provider = ${JSON.stringify(provider)}\nmodel = "gpt-5.1-codex-mini"\nmodel_reasoning_effort = "medium"\n` +
    Object.entries(sites).map(([id, site]) => `\n[model_providers.${id}]\nname = ${JSON.stringify(id)}\nbase_url = ${JSON.stringify(site.baseUrl)}\nwire_api = "responses"\nenv_key = ${JSON.stringify(`PHASE_0_3_${id.toUpperCase()}_KEY`)}\nrequires_openai_auth = false\nsupports_websockets = false\nrequest_max_retries = 0\nstream_max_retries = 0\nstream_idle_timeout_ms = 5000\n`).join("");
}
