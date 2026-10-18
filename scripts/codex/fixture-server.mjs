// Deterministic transport fixture, launched only by node:test. No network or credentials.
import { createInterface } from "node:readline";
const mode = process.argv[2] ?? "normal";
const send = (value) => process.stdout.write(JSON.stringify(value) + "\n");
const input = createInterface({ input: process.stdin });
input.on("line", (line) => {
  const message = JSON.parse(line);
  if (!Object.hasOwn(message, "id") || !message.method) return;
  if (mode === "hang") return;
  if (mode === "invalid") { process.stdout.write("not json\n"); return; }
  if (mode === "exit") { process.exit(3); }
  if (message.method === "rpc-error") { send({ id: message.id, error: { code: -32600, message: "secret-do-not-log" } }); return; }
  if (message.method === "approval") {
    send({ id: "server-request", method: "item/commandExecution/requestApproval", params: { threadId: "t", turnId: "r", itemId: "i", command: "Get-Location" } });
    send({ id: message.id, result: {} });
    return;
  }
  if (message.method === "turn/start") {
    // Notifications can precede the turn/start response, including the terminal event.
    send({ method: "item/agentMessage/delta", params: { threadId: "t", turnId: "r", delta: "中文" } });
    send({ method: "turn/completed", params: { threadId: "t", turn: { id: "r", status: "completed" } } });
    send({ id: message.id, result: { turn: { id: "r", status: "inProgress" } } });
    return;
  }
  if (message.method === "echo") {
    setTimeout(() => send({ id: message.id, result: message.params }), message.params.delay ?? 0);
    return;
  }
  if (message.method === "fragmented") {
    const frame = Buffer.from(JSON.stringify({ id: message.id, result: { text: "中文" } }) + "\n");
    const split = frame.indexOf(Buffer.from("中文")) + 1;
    process.stdout.write(frame.subarray(0, split));
    setTimeout(() => process.stdout.write(frame.subarray(split)), 5);
    return;
  }
  send({ id: message.id, result: {} });
});
