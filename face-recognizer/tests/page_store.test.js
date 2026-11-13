// page_store.js in a stub page: each person's learned signals go to the server per person (never the whole map), and
// what other devices learned or forgot is taken in by refresh().
const test = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

// server: {settings, signals, forgotten} the page's server answers with; ok: whether its POSTs succeed
function page(server = {}, { ok = true, local = {} } = {}) {
  const posts = [], state = { settings: {}, signals: { "ft-usual": {}, "ft-baselines": {} }, forgotten: [], ...server };
  const storage = { ...local };
  for (const [name, fn] of Object.entries({
    getItem: k => (k in storage ? storage[k] : null), setItem: (k, v) => { storage[k] = String(v); },
    removeItem: k => { delete storage[k]; } })) Object.defineProperty(storage, name, { value: fn, enumerable: false });
  const ctx = {
    setTimeout, clearTimeout, console, localStorage: storage, addEventListener() {},
    XMLHttpRequest: class {
      open() {} setRequestHeader() {}
      send() { this.status = 200; this.responseText = JSON.stringify(state); }
    },
    fetch: async (url, opts = {}) => {
      if (opts.method === "POST") {
        posts.push({ url, body: JSON.parse(opts.body) });
        if (!ctx.ok) throw new Error("server away");
        return { ok: true, json: async () => ({ ok: true }) };
      }
      return { ok: true, json: async () => ctx.answer };
    },
    ok, answer: null,
  };
  ctx.window = ctx;
  vm.createContext(ctx);
  vm.runInContext(fs.readFileSync(path.join(__dirname, "..", "page_store.js"), "utf8"), ctx);
  ctx.PageStore.restore("track", "ft-");
  return { S: ctx.PageStore, posts, storage, ctx, signalPosts: () => posts.filter(p => p.url === "/api/signals").map(p => p.body) };
}
const tick = () => new Promise(r => setTimeout(r, 0));
const ann = { p: [0.5, 0.5], n: 10 }, bo = { p: [0.1, 0.9], n: 200 };

test("only the people who changed are sent, never the whole map", async () => {
  const { S, signalPosts, storage } = page({ signals: { "ft-usual": { Ann: ann, Bo: bo }, "ft-baselines": {} } });
  assert.deepStrictEqual(JSON.parse(storage["ft-usual"]), { Ann: ann, Bo: bo });          // the server's copy, restored
  S.signals.setItem("ft-usual", JSON.stringify({ Ann: ann, Bo: bo }));
  assert.deepStrictEqual(signalPosts(), []);                                              // nothing changed: nothing sent
  S.signals.setItem("ft-usual", JSON.stringify({ Ann: { ...ann, n: 11 }, Bo: bo, Cy: ann }));
  await tick();
  assert.deepStrictEqual(signalPosts(), [{ key: "ft-usual", changed: { Ann: { ...ann, n: 11 }, Cy: ann }, gone: [] }]);
});

test("a person the page removed is sent as gone; someone it never knew is never touched", async () => {
  const { S, signalPosts } = page({ signals: { "ft-usual": { Ann: ann, Bo: bo }, "ft-baselines": {} } });
  S.signals.setItem("ft-usual", JSON.stringify({ Bo: bo }));                              // Forget Ann
  S.signals.removeItem("ft-usual");                                                       // Forget everyone (it knows)
  await tick();
  assert.deepStrictEqual(signalPosts(), [{ key: "ft-usual", changed: {}, gone: ["Ann"] }, { key: "ft-usual", changed: {}, gone: ["Bo"] }]);
});

test("a browser's own learning is sent once when the server has none yet", async () => {
  const { signalPosts } = page({}, { local: { "ft-baselines": JSON.stringify({ Ann: { all: {} } }) } });
  await tick();
  assert.deepStrictEqual(signalPosts(), [{ key: "ft-baselines", changed: { Ann: { all: {} } }, gone: [] }]);
});

test("a change that didn't arrive is sent again with the next save", async () => {
  const p = page({ signals: { "ft-usual": { Ann: ann, Bo: bo }, "ft-baselines": {} } }, { ok: false });
  p.S.signals.setItem("ft-usual", JSON.stringify({ Ann: { ...ann, n: 11 } }));            // Ann changed, Bo removed
  await tick(); await tick();
  p.ctx.ok = true;
  p.S.signals.setItem("ft-usual", JSON.stringify({ Ann: { ...ann, n: 11 } }));
  await tick();
  assert.deepStrictEqual(p.signalPosts()[1], { key: "ft-usual", changed: { Ann: { ...ann, n: 11 } }, gone: ["Bo"] });
});

test("refresh takes what other devices learned or forgot, and it isn't sent back", async () => {
  const p = page({ signals: { "ft-usual": { Ann: ann, Bo: bo }, "ft-baselines": {} } });
  const taken = [];
  p.ctx.answer = { v: "2", signals: { "ft-usual": { Bo: { ...bo, n: 300 }, Cy: ann }, "ft-baselines": {} }, forgotten: ["Ann"] };
  const r = await p.S.signals.refresh((key, name, data) => taken.push([key, name, data]));
  assert.deepStrictEqual(r.forgotten, ["Ann"]);
  assert.deepStrictEqual(taken, [["ft-usual", "Bo", { ...bo, n: 300 }], ["ft-usual", "Cy", ann], ["ft-usual", "Ann", null]]);
  assert.deepStrictEqual(JSON.parse(p.storage["ft-usual"]), { Bo: { ...bo, n: 300 }, Cy: ann });
  p.S.signals.setItem("ft-usual", p.storage["ft-usual"]);                                 // the page's next save
  await tick();
  assert.deepStrictEqual(p.signalPosts(), []);
  p.ctx.answer = { v: "2", same: true };
  assert.deepStrictEqual(await p.S.signals.refresh(() => assert.fail("nothing changed")), { v: "2", same: true });
});

test("refresh waits while a change of this page's is on its way", async () => {
  const p = page({ signals: { "ft-usual": { Ann: ann }, "ft-baselines": {} } });
  p.ctx.answer = { v: "2", signals: { "ft-usual": {}, "ft-baselines": {} }, forgotten: [] };   // read before the change landed
  p.S.signals.setItem("ft-usual", JSON.stringify({ Ann: ann, Cy: ann }));
  assert.strictEqual(await p.S.signals.refresh(() => assert.fail("must not apply")), null);
  assert.deepStrictEqual(Object.keys(JSON.parse(p.storage["ft-usual"])), ["Ann", "Cy"]);
});
