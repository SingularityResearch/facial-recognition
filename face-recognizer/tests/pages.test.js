// The page scripts no other test loads must at least parse: a syntax error there would only show up as a blank
// or dead page in the app. Also checks each page loads its scripts and each server serves them.
const test = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const dir = path.join(__dirname, "..");
const read = f => fs.readFileSync(path.join(dir, f), "utf8");

const SCRIPTS = ["tracker_ui.js", "label_ui.js", "app_ui.js", "expression.js", "signals.js", "settings_bridge.js", "page_store.js", "cameras_ui.js", "ask.js", "jpeg_worker.js"];
for (const file of SCRIPTS) {
  test(`${file} parses`, () => assert.doesNotThrow(() => new vm.Script(read(file), { filename: file })));
}

test("no page keeps code inline (so the checks above see all of it)", () => {
  for (const f of ["tracker_ui.html", "label_ui.html", "app_ui.html", "cameras_ui.html"]) assert.doesNotMatch(read(f), /<script>|<script\s[^>]*>\s*[^\s<]/);
});

test("each page loads its scripts, in order, and each is served and packed into the .exe", () => {
  const order = (html, names) => names.map(n => html.indexOf(`<script src="/${n}"></script>`));
  const t = order(read("tracker_ui.html"), ["expression.js", "signals.js", "settings_bridge.js", "page_store.js", "ask.js", "tracker_ui.js"]);
  const l = order(read("label_ui.html"), ["settings_bridge.js", "page_store.js", "ask.js", "label_ui.js"]);
  const a = order(read("app_ui.html"), ["ask.js", "app_ui.js"]);
  const c = order(read("cameras_ui.html"), ["settings_bridge.js", "cameras_ui.js"]);   // the bridge: Ctrl+1 to Ctrl+4 from this tab
  for (const pos of [t, l, a, c]) assert.deepStrictEqual(pos.filter(p => p < 0), [], "a script tag is missing");
  for (const pos of [t, l, a, c]) assert.deepStrictEqual(pos, [...pos].sort((a, b) => a - b), "scripts out of order");
  for (const n of ["expression.js", "signals.js", "settings_bridge.js", "page_store.js", "tracker_ui.js", "cameras_ui.js", "ask.js", "jpeg_worker.js"]) assert.match(read("tracker.py"), new RegExp(`"/${n.replace(".", "\\.")}"`));
  assert.match(read("tracker_ui.js"), /new Worker\("\/jpeg_worker\.js"\)/);          // the worker is the file the server sends
  for (const n of ["label_ui.js", "settings_bridge.js", "page_store.js", "ask.js"]) assert.match(read("label_tool.py"), new RegExp(`"/${n.replace(".", "\\.")}"`));
  for (const n of ["app_ui.js", "ask.js"]) assert.match(read("app.py"), new RegExp(`"/${n.replace(".", "\\.")}"`));
  const build = read("build_exe.py");                      // the .exe build (build_app.bat and CI both run it)
  for (const f of [...SCRIPTS, "tracker_ui.html", "label_ui.html", "app_ui.html", "cameras_ui.html"]) assert.ok(build.includes(`"${f}"`), `${f} not packed`);
  assert.match(read("build_app.bat"), /build_exe\.py/);
});

test("hidden overlays really hide (a display rule would otherwise override the hidden attribute)", () => {
  for (const f of ["app_ui.html", "tracker_ui.html", "label_ui.html"])
    assert.match(read(f), /\[hidden\] \{ display: none !important; \}/, f);
});

test("every fetch reads its answer or lets it go (an unread answer keeps a descriptor: the Linux window froze)", () => {
  // one line with fetch( and the three after it must read the body (json/text/arrayBuffer/blob) or drop/cancel it
  for (const f of SCRIPTS) {
    const lines = read(f).split(/\r?\n/);
    lines.forEach((line, i) => {
      if (!/\bfetch\(/.test(line) || /^\s*\/\//.test(line)) return;
      const near = lines.slice(i, i + 4).join("\n");
      assert.match(near, /\.(json|text|arrayBuffer|blob)\(\)|\bdrop\b|body\?\.cancel\(\)/, `${f}:${i + 1} ${line.trim().slice(0, 80)}`);
    });
  }
});

test("the labeling page keeps the pictures it showed in memory, and lets go of them with a forget", () => {
  // The server answers every picture with no-store (a browser's cache kept forgotten faces on disk), so the page
  // keeps what it showed as blob: URLs. Every picture goes through thumb()/photoSrc(), and what the page keeps is
  // dropped when faces are acted on, a person is forgotten, or an action is undone.
  const code = read("label_ui.js");
  assert.doesNotMatch(code, /src="\/(thumb|photo)\//, "a picture address written into the page directly");
  assert.match(code, /post\("\/api\/person\/forget"[^\n]*\n\s*forgetPics\(\)/);
  assert.match(code, /post\("\/api\/undo"\); if \(r\.undone\) \{ forgetPics\(\)/);
  assert.match(code, /post\("\/api\/act"[^\n]*\n[^\n]*dropPic\(thumbUrl\(i\)\)/);
  assert.match(read("localguard.py"), /"Cache-Control", "no-store"\)/);
  assert.doesNotMatch(read("localguard.py") + read("label_tool.py") + read("tracker.py") + read("app.py"), /max-age|immutable/);
});

test("no page stops to ask (confirm, alert and prompt stop every tab's scripts: the camera, and the watchdog's signal)", () => {
  // In the app window the tab page and its tabs run in one process. A confirm() left open stopped the camera and
  // the tab page's /alive requests; after 30 s the watchdog took the window for frozen and started the app again.
  for (const f of SCRIPTS) {
    const code = read(f).split(/\r?\n/).filter(l => !/^\s*\/\//.test(l)).join("\n");
    assert.doesNotMatch(code, /(?<![.\w])(confirm|alert|prompt)\(/, `${f}: ask with askUser (ask.js)`);
  }
});

test("askUser asks in the page and answers yes or no", async () => {
  const made = [];
  const node = tag => { const n = { tag, style: {}, kids: [], on: {}, append: (...k) => n.kids.push(...k), setAttribute() {}, focus() {},
    addEventListener: (t, fn) => { n.on[t] = fn; }, remove() { n.gone = true; },
    showModal() { n.open = true; }, close(v) { n.returnValue = v; n.on.close(); } }; made.push(n); return n; };
  const ctx = { document: { createElement: node, body: { append() {} } } };
  vm.createContext(ctx);
  vm.runInContext(read("ask.js") + "\nthis.askUser = askUser;", ctx);
  for (const [button, answer] of [["Delete", true], ["Cancel", false]]) {
    made.length = 0;
    const asked = ctx.askUser("Delete this alert for good?", "Delete");
    const dialog = made.find(n => n.tag === "dialog");
    assert.ok(dialog.open && made.some(n => n.textContent === "Delete this alert for good?"));
    made.find(n => n.tag === "button" && n.textContent === button).on.click();
    assert.strictEqual(await asked, answer);
    assert.ok(dialog.gone);
  }
  made.length = 0;
  const asked = ctx.askUser("Sure?");                       // Esc closes the dialog with no answer: no
  made.find(n => n.tag === "dialog").close("");
  assert.strictEqual(await asked, false);
});

test("no inline event handlers either (the pages' Content-Security-Policy refuses inline script)", () => {
  for (const f of ["tracker_ui.html", "label_ui.html", "app_ui.html", "cameras_ui.html", ...SCRIPTS])
    assert.doesNotMatch(read(f), /\son[a-z]+\s*=\s*["']/i, f);
});

test("the tracker page uses the computer's own fonts, nothing from the internet", () => {
  assert.doesNotMatch(read("tracker_ui.html"), /fonts\.googleapis|<link[^>]+https?:/);
  assert.doesNotMatch(read("tracker_ui.js"), /Plex/);
});

test("the tab page names a tab whose page stopped answering, and only such a tab", () => {
  // Each tab's page runs in a process of its own (localguard.py: Origin-Agent-Cluster), so a hung page no longer
  // stops the tab page and its /alive: the tab page asks every tab and tells the watchdog which are silent.
  const code = read("app_ui.js");
  const head = code.match(/const ANSWER_IN = (\d+), answered = \{\};/), fn = code.match(/const loadedTabs = [^\n]*\nfunction silentTabs\(now\) \{\r?\n[\s\S]*?\n\}/);
  assert.ok(head && fn, "silentTabs, loadedTabs or ANSWER_IN isn't written the way this test reads it");
  const T = "http://127.0.0.1:8766/", L = "http://127.0.0.1:8765/", C = "http://127.0.0.1:8766/cameras";
  const frames = { "f-track": { src: T }, "f-label": { src: "" }, "f-cams": { src: C } };
  const ctx = { document: { getElementById: id => frames[id] } };
  vm.createContext(ctx);
  vm.runInContext(`const ANSWER_IN = ${head[1]}, answered = {}; ${fn[0]}; this.silentTabs = silentTabs; this.loadedTabs = loadedTabs; this.answered = answered;`, ctx);
  const named = now => [...ctx.silentTabs(now)], asked = () => [...ctx.loadedTabs()];
  assert.deepStrictEqual([named(1e6), asked()], [[], []]);             // loaded pages only: nothing has loaded yet
  Object.assign(ctx.answered, { track: { src: T, at: 1e6 }, label: { src: L, at: 1e6 - 60000 }, cams: { src: C, at: 1e6 } });
  assert.deepStrictEqual(asked(), ["track", "cams"]);                  // Labeling has no page now (being started again)
  assert.deepStrictEqual(named(1e6 + 9000), []);                       // answered 9 s ago
  assert.deepStrictEqual(named(1e6 + 11000), ["track", "cams"]);
  ctx.answered.cams.at = 1e6 + 10000;
  assert.deepStrictEqual(named(1e6 + 11000), ["track"]);
  frames["f-label"].src = L;
  assert.deepStrictEqual(named(1e6 + 11000), ["track", "label"]);
  frames["f-track"].src = "http://127.0.0.1:9766/";                    // started again at another address: not loaded yet
  assert.deepStrictEqual([named(1e6 + 11000), asked()], [["label"], ["label", "cams"]]);
  assert.ok(+head[1] >= 6000, "a page busy for a few seconds (a large Labeling view) must not count as hung");
  assert.match(code, /for \(const k of loadedTabs\(\)\) tell\(k, \{ fs: "ping" \}\)/);
  assert.match(code, /fetch\("\/alive" \+ \(silent\.length \? `\?silent=\$\{silent\.join\(","\)\}` : ""\)/);
  assert.match(code, /m\.fs === "pong"\) \{ if \(answered\[k\]\) answered\[k\]\.at = Date\.now\(\)/);
  assert.match(code, /addEventListener\("load", e => \{\r?\n\s*if \(e\.target\.getAttribute\("src"\)\) answered\[k\] = \{ src: e\.target\.src, at: Date\.now\(\) \}/);
});

test("the tracker page makes its JPEGs in a worker, and by itself where that fails", async () => {
  // canvas.toBlob waits for the page's idle time (a second when there is none: the camera at 1 frame a second).
  const code = read("tracker_ui.js");
  const block = code.match(/const jpegs = \{[\s\S]*?\n(?=\/\/ -+ recognition requests)/);
  assert.ok(block, "the JPEG block isn't where this test reads it");
  assert.doesNotMatch(code.replace(block[0], ""), /\.toBlob\(/, "a JPEG made with toBlob outside jpegOf");
  const CHROMIUM = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/157.0.0.0 Safari/537.36 Edg/157.0.0.0";
  const WEBKIT = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)";
  const make = ({ worker = true, reply = n => ({ n, blob: "from the worker" }), browser = CHROMIUM } = {}) => {
    const notes = [], posted = [], made = [], clock = { t: 5000 };
    class Worker { constructor(url) { this.url = url; made.push(this); }
      postMessage(m) { posted.push(m); const r = reply(m.n); if (r) Promise.resolve().then(() => this.onmessage({ data: r })); }
      terminate() { this.ended = true; } }
    const window = worker ? { Worker, OffscreenCanvas: class {}, createImageBitmap: true } : {};
    const ctx = { window, Worker, navigator: { userAgent: browser }, pageNote: t => notes.push(t), createImageBitmap: async () => ({ close() {} }),
                  performance: { now: () => clock.t }, setTimeout: (fn, ms) => { const t = setTimeout(fn, ms); t.unref(); return t; } };
    vm.createContext(ctx);
    vm.runInContext(block[0] + "\nthis.jpegOf = jpegOf; this.jpegs = jpegs;", ctx);
    const canvas = { toBlob: (done, type, q) => done(`toBlob ${type} ${q}`) };
    return { ctx, notes, posted, made, clock, jpeg: q => ctx.jpegOf(canvas, q) };
  };
  let p = make();
  assert.strictEqual(await p.jpeg(0.72), "from the worker");
  assert.strictEqual(await p.jpeg(0.85), "from the worker");
  assert.deepStrictEqual([p.made.length, p.made[0].url, p.posted.map(m => m.q).join()], [1, "/jpeg_worker.js", "0.72,0.85"]);
  assert.deepStrictEqual(p.notes, ["JPEGs are made in a worker"]);                 // said once
  p = make({ worker: false });                                                     // no worker canvas in this browser
  assert.strictEqual(await p.jpeg(0.72), "toBlob image/jpeg 0.72");
  assert.strictEqual(await p.jpeg(0.72), "toBlob image/jpeg 0.72");
  assert.strictEqual(p.notes.length, 1);
  assert.match(p.notes[0], /on the page itself \(toBlob\)/);
  p.clock.t += 3600e3;                                                             // and that is for good
  assert.strictEqual(await p.jpeg(0.72), "toBlob image/jpeg 0.72");
  assert.deepStrictEqual([p.ctx.jpegs.off, p.notes.length], [Infinity, 1]);
  p = make({ browser: WEBKIT });                                                   // the macOS window: toBlob doesn't wait there (measured)
  assert.strictEqual(await p.jpeg(0.72), "toBlob image/jpeg 0.72");
  assert.deepStrictEqual([p.made.length, p.notes.length, p.ctx.jpegs.off], [0, 1, Infinity]);
  assert.match(p.notes[0], /makes them at once/);
  // The worker can't (a graphics driver reset under the app takes its canvas for a moment): three tries, then
  // toBlob for a minute, then a new worker; not off until the page's next start.
  let works = false;
  p = make({ reply: n => works ? { n, blob: "from the worker" } : { n, error: "no canvas" } });
  for (let i = 0; i < 5; i++) assert.strictEqual(await p.jpeg(0.72), "toBlob image/jpeg 0.72");
  assert.deepStrictEqual([p.posted.length, p.made.length, p.made[0].ended, p.ctx.jpegs.off], [3, 1, true, 5000 + 60000]);
  assert.match(p.notes.at(-1), /tried again in 1 min: the worker failed three times \(no canvas\)/);
  p.clock.t += 59000;
  assert.strictEqual(await p.jpeg(0.72), "toBlob image/jpeg 0.72");
  assert.strictEqual(p.made.length, 1);                                            // not yet
  p.clock.t += 2000;
  for (let i = 0; i < 4; i++) assert.strictEqual(await p.jpeg(0.72), "toBlob image/jpeg 0.72");
  assert.deepStrictEqual([p.made.length, p.posted.length, p.made[1].ended], [2, 6, true]);   // a new worker, which failed too
  assert.match(p.notes.at(-1), /tried again in 2 min/);                            // twice as long later
  works = true;
  p.clock.t += 121000;
  assert.strictEqual(await p.jpeg(0.72), "from the worker");
  assert.deepStrictEqual([p.made.length, p.ctx.jpegs.off, p.notes.at(-1)], [3, 0, "JPEGs are made in a worker"]);
  p = make();                                                                      // a failure among good ones doesn't end it
  await p.jpeg(0.72);
  p.ctx.jpegs.fails = 2;
  assert.strictEqual(await p.jpeg(0.72), "from the worker");
  assert.strictEqual(p.ctx.jpegs.fails, 0);
  p = make({ reply: () => null });                                                 // its script doesn't load: the picture waiting is made here
  const waiting = p.jpeg(0.72);
  await new Promise(r => setTimeout(r, 5));
  p.made[0].onerror({ message: "", preventDefault() {} });
  assert.strictEqual(await waiting, "toBlob image/jpeg 0.72");
  assert.strictEqual(p.ctx.jpegs.off, 5000 + 60000);                               // tried again in a minute, too
});

test("the worker makes a JPEG of the picture it is handed, and keeps a canvas per size", async () => {
  const made = [];
  global.OffscreenCanvas = class { constructor(w, h) { this.width = w; this.height = h; this.log = []; made.push(this); }
    getContext(kind) { if (kind === "bitmaprenderer" && !global.noRenderer) return { transferFromImageBitmap: b => this.log.push(["shown", b.width]) };
                       return kind === "2d" ? { drawImage: b => this.log.push(["drawn", b.width]) } : null; }
    convertToBlob(o) { return Promise.resolve({ ...o, of: this.width + "x" + this.height }); } };
  const W = require("../jpeg_worker.js");
  const bmp = (w, h) => ({ width: w, height: h, closed: false, close() { this.closed = true; } });
  assert.deepStrictEqual(await W.jpeg(bmp(960, 540), 0.72), { type: "image/jpeg", quality: 0.72, of: "960x540" });
  await W.jpeg(bmp(960, 540), 0.72);
  await W.jpeg(bmp(640, 360), 0.85);
  assert.deepStrictEqual([made.length, made[0].log.length, made[1].log[0][0]], [2, 2, "shown"]);
  global.noRenderer = true;                                            // a browser without that canvas: drawn, and the picture let go
  const b = bmp(320, 180);
  await W.jpeg(b, 0.8);
  assert.deepStrictEqual([made.at(-1).log[0][0], b.closed], ["drawn", true]);
  for (let i = 0; i < 12; i++) await W.jpeg(bmp(100 + i, 50), 0.8);    // sizes come and go: the canvases kept stay few
  assert.ok(W.canvases.size <= 8);
  delete global.OffscreenCanvas; delete global.noRenderer;
});
