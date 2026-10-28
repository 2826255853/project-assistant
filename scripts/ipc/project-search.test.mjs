import assert from "node:assert/strict";
import test from "node:test";
import { ProjectSearchController } from "../../src/ipc/projectSearch.ts";

function fixture() {
  const requests = [], states = [];
  const controller = new ProjectSearchController((params, options) => new Promise((resolve, reject) => requests.push({params, options, resolve, reject})), state => states.push(state));
  return { requests, states, controller, current: () => states.at(-1) };
}
const page = (id, nextCursor = null) => ({ items: [{id}], nextCursor });

test("rapid input suppresses stale successes and errors even if native cancellation is ignored", async () => {
  const f = fixture();
  const old = f.controller.search("old");
  const middle = f.controller.search("middle");
  const latest = f.controller.search("最新");
  assert.equal(f.requests[0].options.signal.aborted, true);
  f.requests[2].resolve(page("latest")); await latest;
  f.requests[0].resolve(page("old")); await old;
  f.requests[1].reject(new Error("old failure")); await middle;
  assert.equal(f.current().query, "最新");
  assert.deepEqual(f.current().items, [{id:"latest"}]);
  assert.equal(f.current().error, null);
});

test("IME invalidates in-flight search immediately and sends only committed text", async () => {
  const f = fixture();
  const old = f.controller.search("before");
  await f.controller.search("zhong", true);
  f.requests[0].resolve(page("old")); await old;
  assert.equal(f.requests.length, 1);
  assert.deepEqual(f.current().items, []);
  const committed = f.controller.search("中文");
  assert.equal(f.requests[1].params.query, "中文");
  f.requests[1].resolve(page("chinese")); await committed;
  assert.equal(f.current().loading, false);
});

test("pagination is serialized and old next-page responses cannot replace a new query", async () => {
  const f = fixture();
  const first = f.controller.search(""); f.requests[0].resolve(page("1", "cursor")); await first;
  const next = f.controller.loadMore();
  await f.controller.loadMore();
  assert.deepEqual(f.requests[1].params, {query:"",cursor:"cursor"});
  assert.equal(f.requests.length, 2);
  const fresh = f.controller.search("new");
  f.requests[2].resolve(page("new")); await fresh;
  f.requests[1].resolve(page("2")); await next;
  assert.deepEqual(f.current().items, [{id:"new"}]);
});

test("append, empty results, fixed errors and disposal have explicit state", async () => {
  const f = fixture();
  let pending = f.controller.search(""); f.requests[0].resolve(page("1", "cursor")); await pending;
  pending = f.controller.loadMore(); f.requests[1].resolve(page("2")); await pending;
  assert.deepEqual(f.current().items, [{id:"1"},{id:"2"}]);
  pending = f.controller.search("none"); f.requests[2].resolve({items:[],nextCursor:null}); await pending;
  assert.deepEqual(f.current().items, []);
  const error = {code:"PA_PROJECT_CONFLICT_REVISION",correlationId:"safe-id"};
  pending = f.controller.search("fail"); f.requests[3].reject(error); await pending;
  assert.equal(f.current().error, error);
  pending = f.controller.search("dispose"); const count = f.states.length;
  f.controller.dispose(); f.requests[4].resolve(page("late")); await pending;
  await f.controller.search("after-dispose");
  assert.equal(f.states.length, count);
  assert.equal(f.requests.length, 5);
});
