// The page scripts no other test loads must at least parse: a syntax error there would only show up as a blank
// or dead page in the app. Also checks each page loads its scripts and each server serves them.
const test = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const dir = path.join(__dirname, "..");
const read = f => fs.readFileSync(path.join(dir, f), "utf8");

const SCRIPTS = ["tracker_ui.js", "label_ui.js", "app_ui.js", "expression.js", "signals.js", "settings_bridge.js", "page_store.js", "cameras_ui.js"];
for (const file of SCRIPTS) {
  test(`${file} parses`, () => assert.doesNotThrow(() => new vm.Script(read(file), { filename: file })));
}

test("no page keeps code inline (so the checks above see all of it)", () => {
  for (const f of ["tracker_ui.html", "label_ui.html", "app_ui.html"]) assert.doesNotMatch(read(f), /<script>/);
});

test("each page loads its scripts, in order, and each is served and packed into the .exe", () => {
  const order = (html, names) => names.map(n => html.indexOf(`<script src="/${n}"></script>`));
  const t = order(read("tracker_ui.html"), ["expression.js", "signals.js", "settings_bridge.js", "page_store.js", "tracker_ui.js"]);
  const l = order(read("label_ui.html"), ["settings_bridge.js", "page_store.js", "label_ui.js"]);
  for (const pos of [t, l]) assert.deepStrictEqual(pos.filter(p => p < 0), [], "a script tag is missing");
  for (const pos of [t, l]) assert.deepStrictEqual(pos, [...pos].sort((a, b) => a - b), "scripts out of order");
  assert.match(read("app_ui.html"), /<script src="\/app_ui\.js"><\/script>/);
  for (const n of ["expression.js", "signals.js", "settings_bridge.js", "page_store.js", "tracker_ui.js"]) assert.match(read("tracker.py"), new RegExp(`"/${n.replace(".", "\\.")}"`));
  for (const n of ["label_ui.js", "settings_bridge.js", "page_store.js"]) assert.match(read("label_tool.py"), new RegExp(`"/${n.replace(".", "\\.")}"`));
  assert.match(read("app.py"), /"\/app_ui\.js"/);
  const build = read("build_exe.py");                      // the .exe build (build_app.bat and CI both run it)
  for (const f of [...SCRIPTS, "tracker_ui.html", "label_ui.html", "app_ui.html"]) assert.ok(build.includes(`"${f}"`), `${f} not packed`);
  assert.match(read("build_app.bat"), /build_exe\.py/);
});

test("hidden overlays really hide (a display rule would otherwise override the hidden attribute)", () => {
  for (const f of ["app_ui.html", "tracker_ui.html", "label_ui.html"])
    assert.match(read(f), /\[hidden\] \{ display: none !important; \}/, f);
});

test("no inline event handlers either (the pages' Content-Security-Policy refuses inline script)", () => {
  for (const f of ["tracker_ui.html", "label_ui.html", "app_ui.html", ...SCRIPTS])
    assert.doesNotMatch(read(f), /\son[a-z]+\s*=\s*["']/i, f);
});

test("the tracker page uses the computer's own fonts, nothing from the internet", () => {
  assert.doesNotMatch(read("tracker_ui.html"), /fonts\.googleapis|<link[^>]+https?:/);
  assert.doesNotMatch(read("tracker_ui.js"), /Plex/);
});
