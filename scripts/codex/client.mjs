// Phase-0 experiment only. Business code must not depend on this wire protocol.
import { spawn } from "node:child_process";
import { EventEmitter } from "node:events";
import { win32 } from "node:path";

export class ProbeError extends Error {
  constructor(code) { super(code); this.code = code; }
}

export class AppServerClient extends EventEmitter {
  constructor({ executable, args = [], cwd, env = process.env, timeoutMs = 15000 }) {
    super();
    this.timeoutMs = timeoutMs;
    this.pending = new Map();
    this.events = [];
    this.sequence = 0;
    this.nextId = 0;
    this.stderrBytes = 0;
    this.child = spawn(executable, args, { cwd, env, windowsHide: true, shell: false,
      stdio: ["pipe", "pipe", "pipe"] });
    this.exit = new Promise((done) => {
      this.child.once("error", (error) => {
        this.fail(new ProbeError(error.code === "ENOENT" ? "CODEX_NOT_INSTALLED" : "CODEX_START_FAILED"));
        done({ code: null, signal: null, spawnFailed: true });
      });
      this.child.once("close", (code, signal) => {
        this.fail(new ProbeError("CODEX_DISCONNECTED"));
        done({ code, signal });
      });
    });
    this.child.stderr.on("data", (data) => { this.stderrBytes += data.length; });
    this.child.stdin.on("error", () => this.fail(new ProbeError("CODEX_DISCONNECTED")));
    let buffer = "";
    this.child.stdout.setEncoding("utf8");
    this.child.stdout.on("data", (data) => {
      buffer += data;
      if (Buffer.byteLength(buffer) > 8 * 1024 * 1024) {
        this.fail(new ProbeError("CODEX_FRAME_TOO_LARGE"));
        this.child.kill();
        return;
      }
      let end;
      while ((end = buffer.indexOf("\n")) !== -1) {
        const line = buffer.slice(0, end).trim();
        buffer = buffer.slice(end + 1);
        if (!line) continue;
        try { this.receive(JSON.parse(line)); }
        catch { this.fail(new ProbeError("CODEX_INVALID_FRAME")); this.child.kill(); }
      }
    });
  }

  fail(error) {
    this.failure ??= error;
    for (const request of this.pending.values()) { clearTimeout(request.timer); request.reject(error); }
    this.pending.clear();
    this.emit("disconnected", error);
  }

  receive(message) {
    if (message.method) {
      const event = { ...message, sequence: ++this.sequence };
      this.events.push(event);
      // Bounded in-memory replay prevents the response/notification race. Never persist raw events.
      if (this.events.length > 10000) this.events.shift();
      this.emit("event", event);
      if (Object.hasOwn(message, "id")) this.emit("serverRequest", event);
      return;
    }
    const pending = this.pending.get(message.id);
    if (!pending) return; // Late response after timeout.
    this.pending.delete(message.id);
    clearTimeout(pending.timer);
    if (message.error) {
      const error = new ProbeError("CODEX_RPC_ERROR");
      error.rpcCode = Number.isInteger(message.error.code) ? message.error.code : null;
      pending.reject(error); // Remote error text may contain a key, endpoint or prompt.
    } else pending.resolve(message.result);
  }

  write(message) {
    if (this.failure) throw this.failure;
    this.child.stdin.write(JSON.stringify(message) + "\n");
  }

  request(method, params = {}, timeoutMs = this.timeoutMs) {
    if (this.failure) return Promise.reject(this.failure);
    return new Promise((resolve, reject) => {
      const id = ++this.nextId;
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new ProbeError("CODEX_REQUEST_TIMEOUT"));
      }, timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      try { this.write({ id, method, params }); }
      catch (error) { clearTimeout(timer); this.pending.delete(id); reject(error); }
    });
  }

  notify(method, params = {}) { this.write({ method, params }); }
  respond(id, result) { this.write({ id, result }); }
  rejectRequest(id) { this.write({ id, error: { code: -32601, message: "Unsupported client request" } }); }

  waitFor(predicate, { after = 0, timeoutMs = this.timeoutMs } = {}) {
    const existing = this.events.find((event) => event.sequence > after && predicate(event));
    if (existing) return Promise.resolve(existing);
    if (this.failure) return Promise.reject(this.failure);
    return new Promise((resolve, reject) => {
      const cleanup = () => {
        clearTimeout(timer);
        this.off("event", onEvent);
        this.off("disconnected", onDisconnect);
      };
      const onEvent = (event) => {
        if (event.sequence > after && predicate(event)) { cleanup(); resolve(event); }
      };
      const onDisconnect = (error) => { cleanup(); reject(error); };
      const timer = setTimeout(() => { cleanup(); reject(new ProbeError("CODEX_EVENT_TIMEOUT")); }, timeoutMs);
      this.on("event", onEvent);
      this.on("disconnected", onDisconnect);
    });
  }

  async initialize() {
    const result = await this.request("initialize", {
      clientInfo: { name: "project_assistant_phase0", title: "Phase 0 probe", version: "0.1.0" },
      capabilities: { experimentalApi: false },
    });
    this.notify("initialized");
    return result;
  }

  async close() {
    this.child.stdin.end();
    let timer;
    const graceful = await Promise.race([this.exit, new Promise((done) => {
      timer = setTimeout(() => done(null), 3000);
    })]);
    clearTimeout(timer);
    if (graceful) return { ...graceful, forced: false, stderrBytes: this.stderrBytes };
    // Only this client's process tree. Never attach to or restart the Desktop daemon.
    if (process.platform === "win32" && this.child.pid) {
      const killer = spawn("taskkill.exe", ["/pid", String(this.child.pid), "/t", "/f"],
        { windowsHide: true, stdio: "ignore" });
      await new Promise((done) => { killer.once("close", done); killer.once("error", done); });
    } else this.child.kill("SIGKILL");
    const result = await this.exit;
    return { ...result, forced: true, stderrBytes: this.stderrBytes };
  }
}

export function normalizeEvent(event) {
  const params = event.params ?? {};
  const common = { sequence: event.sequence, threadId: params.threadId, turnId: params.turnId };
  switch (event.method) {
    case "turn/started": return { ...common, type: "running", turnId: params.turn?.id };
    case "item/agentMessage/delta": return { ...common, type: "message_delta", bytes: Buffer.byteLength(params.delta ?? "") };
    case "item/commandExecution/requestApproval":
    case "item/fileChange/requestApproval": return { ...common, type: "approval_required", itemId: params.itemId };
    case "turn/completed": return { ...common, type: ({ completed: "completed", interrupted: "cancelled", failed: "failed" })[params.turn?.status] ?? "unknown", turnId: params.turn?.id };
    case "error": return { ...common, type: "error", retrying: params.willRetry === true };
    default: return null;
  }
}

export function selectModel(models, modelName, effort) {
  const model = models.find((item) => item.model === modelName);
  if (!model) throw new ProbeError("MODEL_UNAVAILABLE");
  const selected = effort ?? model.defaultReasoningEffort;
  if (!model.supportedReasoningEfforts.some((item) => item.reasoningEffort === selected)) {
    throw new ProbeError("EFFORT_UNSUPPORTED");
  }
  return Object.freeze({ model: model.model, effort: selected });
}

export async function listModels(client) {
  const models = [];
  const cursors = new Set();
  let cursor;
  do {
    const page = await client.request("model/list", { cursor, limit: 100, includeHidden: false });
    models.push(...page.data);
    cursor = page.nextCursor;
    if (cursor && cursors.has(cursor)) throw new ProbeError("MODEL_PAGINATION_LOOP");
    cursors.add(cursor);
  } while (cursor);
  return models;
}

export function safePrintCommand(command, trustedExecutables = []) {
  if (/^(Get-Location|pwd)$/i.test(command.trim())) return true;
  const match = command.trim().match(/^(?:"([^"\r\n]+)"|'([^'\r\n]+)'|(\S+))\s+((?:-(?:NoProfile|NoLogo|NonInteractive)\s+)*)-Command\s+(?:"Get-Location"|'Get-Location'|Get-Location)$/i);
  if (!match) return false;
  // This Windows server displays the executable with doubled path separators.
  const executable = win32.normalize(match[1] ?? match[2] ?? match[3]).toLowerCase();
  return trustedExecutables.some((path) => win32.normalize(path).toLowerCase() === executable);
}
