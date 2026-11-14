// settings_bridge.js in a stub page: which windows it answers, and what "set" and "act" do.
const test = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

function page({ ancestor = "http://127.0.0.1:8764", alone = false } = {}) {
  const listeners = {}, posted = [];
  const parent = { postMessage: (msg, origin) => posted.push({ msg, origin }) };
  const ctx = {
    setTimeout, clearTimeout, console, Event: class { constructor(t) { this.type = t; } },
    location: { ancestorOrigins: ancestor === undefined ? undefined : [ancestor] },
    document: { addEventListener() {}, getElementById: () => null, querySelector: () => null },
    addEventListener: (type, fn) => { listeners[type] = fn; },
    parent,
  };
  ctx.window = ctx;
  if (alone) ctx.parent = ctx;
  vm.createContext(ctx);
  vm.runInContext(fs.readFileSync(path.join(__dirname, "..", "settings_bridge.js"), "utf8") + "\nthis.SettingsBridge = SettingsBridge;", ctx);
  let value = false;
  ctx.SettingsBridge.init("test", [
    { section: "S", id: "flag", label: "Flag", type: "toggle", get: () => value, set: v => { value = v; } },
  ], { ping: async a => ({ message: "pong " + a.x }) });
  const send = async (data, origin = "http://127.0.0.1:8764", source = parent) => {
    await listeners.message({ data, origin, source });
    await new Promise(r => setTimeout(r, 0));
  };
  const key = (key, more = {}) => { const e = { key, ctrlKey: true, prevented: false, preventDefault() { e.prevented = true; }, ...more }; listeners.keydown(e); return e.prevented; };
  return { send, posted, key, get value() { return value; }, parent, bridge: ctx.SettingsBridge };
}

test("Ctrl+1 to Ctrl+4 pressed inside a tab go to the app window's tab page, and only there", () => {
  const p = page();
  assert.deepStrictEqual(["1", "2", "3", "4"].map(k => p.key(k)), [true, true, true, true]);
  assert.deepStrictEqual(JSON.parse(JSON.stringify(p.posted.map(m => [m.msg.fs, m.msg.key, m.origin]))),
                         ["1", "2", "3", "4"].map(k => ["key", k, "http://127.0.0.1:8764"]));
  assert.deepStrictEqual([p.key("5"), p.key("a"), p.key("1", { ctrlKey: false }), p.key("1", { altKey: true })], [false, false, false, false]);
  assert.strictEqual(p.posted.length, 4);                           // other keys stay with the page
  const alone = page({ alone: true });                              // opened on its own: the browser's shortcut
  assert.strictEqual(alone.key("1"), false);
  const framed = page({ ancestor: "https://evil.example" });        // framed by another site: nothing is told
  assert.strictEqual(framed.key("1"), false);
  assert.strictEqual(alone.posted.length + framed.posted.length, 0);
});

test("answers the app window: state, set and actions", async () => {
  const p = page();
  await p.send({ fs: "get" });
  const state = p.posted.at(-1);
  assert.strictEqual(state.origin, "http://127.0.0.1:8764");
  assert.deepStrictEqual(JSON.parse(JSON.stringify(state.msg.items.map(i => [i.id, i.value]))), [["flag", false]]);   // from the page's realm
  assert.ok(!("set" in state.msg.items[0]) && !("get" in state.msg.items[0]));
  await p.send({ fs: "set", id: "flag", value: true });
  assert.strictEqual(p.value, true);
  await p.send({ fs: "act", id: "ping", args: { x: 1 }, rid: 7 });
  const reply = p.posted.find(m => m.msg.fs === "result");
  assert.deepStrictEqual([reply.msg.rid, reply.msg.ok, reply.msg.value.message], [7, true, "pong 1"]);
});

test("ignores other websites, other local pages and other windows", async () => {
  const p = page();
  await p.send({ fs: "set", id: "flag", value: true }, "https://evil.example");
  await p.send({ fs: "set", id: "flag", value: true }, "http://127.0.0.1:3000");      // not the real parent
  await p.send({ fs: "set", id: "flag", value: true }, "http://127.0.0.1:8764", {});  // not the parent window
  assert.strictEqual(p.value, false);
  assert.strictEqual(p.posted.length, 0);
});

test("a failed action is reported, not thrown", async () => {
  const p = page();
  await p.send({ fs: "act", id: "missing", rid: 1 });
  assert.ok(!p.posted.some(m => m.msg.fs === "result"));
  assert.ok(p.posted.some(m => m.msg.fs === "state"));
});

test("notify tells only the window around the page, and only one on this computer", () => {
  const p = page();
  p.bridge.notify({ fs: "forgot", name: "Ann" });
  assert.deepStrictEqual(JSON.parse(JSON.stringify(p.posted)), [{ msg: { fs: "forgot", name: "Ann", app: "test" }, origin: "http://127.0.0.1:8764" }]);
  const q = page({ ancestor: "https://evil.example" });
  q.bridge.notify({ fs: "forgot", name: "Ann" });
  assert.strictEqual(q.posted.length, 0);
  const alone = page({ alone: true });                              // opened on its own: no window around it
  alone.bridge.notify({ fs: "forgot", name: "Ann" });
  assert.strictEqual(alone.posted.length, 0);
});
