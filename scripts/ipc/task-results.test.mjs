import assert from "node:assert/strict";
import test from "node:test";
import { isTaskContent, isTaskChangePage, isTaskDiff } from "../../src/ipc/taskContracts.ts";
const id = "00000000-0000-4000-8000-000000000001";
test("task bodies enforce UTF-8 limits and exclude raw protocol fields", () => {
  const content = {taskId: id, contentRevision: 1, channel: "reply", text: "中文🙂", nextCursor: null, truncated: false, availability: "memory"};
  assert.ok(isTaskContent(content));
  for (const extra of [{raw: "private"}, {channel: "stderr"}, {text: "中".repeat(11000)}, {text: "\u001b[31m"}, {availability: "unavailable"}]) assert.equal(isTaskContent({...content, ...extra}), false);
  assert.ok(isTaskContent({...content, text: "", availability: "unavailable"}));
});
test("task changes and diffs reject forged locators, unknown fields and contradictory availability", () => {
  const item = {id, kind: "modified", relativePath: "src/中文.txt", diffAvailable: true};
  const page = {taskId: id, items: [item], nextCursor: null, contentAvailable: true};
  assert.ok(isTaskChangePage(page));
  for (const relativePath of ["../outside", "/outside", "C:/secret", "src/../secret", "src\\file", "a:stream", "src/CON.txt", "src/a.", "src//a"]) assert.equal(isTaskChangePage({...page, items: [{...item, relativePath}]}), false);
  assert.equal(isTaskChangePage({...page, items: [item, item]}), false);
  const diff = {taskId: id, changeId: id, text: "@@ -1 +1 @@\n-old\n+new", unavailableReason: null};
  assert.ok(isTaskDiff(diff));
  for (const extra of [{raw: "private"}, {text: "x".repeat(65537)}, {text: null}, {unavailableReason: "expired"}]) assert.equal(isTaskDiff({...diff, ...extra}), false);
  assert.ok(isTaskDiff({...diff, text: null, unavailableReason: "unsafe_path"}));
});
