// The page scripts no other test loads must at least parse: a syntax error there would only show up as a blank
// or dead page in the app. Also checks each page loads its scripts and each server serves them.
const test = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const dir = path.join(__dirname, "..");
const read = f => fs.readFileSync(path.join(dir, f), "utf8");

const SCRIPTS = ["tracker_ui.js", "label_ui.js", "app_ui.js", "expression.js", "signals.js", "settings_bridge.js", "page_store.js", "cameras_ui.js", "ask.js"];
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
  for (const n of ["expression.js", "signals.js", "settings_bridge.js", "page_store.js", "tracker_ui.js", "cameras_ui.js", "ask.js"]) assert.match(read("tracker.py"), new RegExp(`"/${n.replace(".", "\\.")}"`));
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
