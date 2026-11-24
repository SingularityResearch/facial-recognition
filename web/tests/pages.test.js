// The page scripts no other test loads must at least parse: a syntax error there would only show up as a blank
// or dead page in the app. Also checks each page loads its scripts and each server serves them.
const test = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const dir = path.join(__dirname, "..");
const read = f => fs.readFileSync(path.join(dir, f), "utf8");

const SCRIPTS = ["tracker_ui.js", "label_ui.js", "app_ui.js", "expression.js", "signals.js", "signals_chart.js", "settings_bridge.js", "page_store.js", "cameras_ui.js", "ask.js", "jpeg_worker.js", "place.js"];
for (const file of SCRIPTS) {
  test(`${file} parses`, () => assert.doesNotThrow(() => new vm.Script(read(file), { filename: file })));
}

test("no page keeps code inline (so the checks above see all of it)", () => {
  for (const f of ["tracker_ui.html", "label_ui.html", "app_ui.html", "cameras_ui.html"]) assert.doesNotMatch(read(f), /<script>|<script\s[^>]*>\s*[^\s<]/);
});

test("each page loads its scripts, in order, and each is served and packed into the .exe", () => {
  const order = (html, names) => names.map(n => html.indexOf(`<script src="/${n}"></script>`));
  const t = order(read("tracker_ui.html"), ["expression.js", "signals.js", "settings_bridge.js", "page_store.js", "ask.js", "place.js", "signals_chart.js", "tracker_ui.js"]);
  const l = order(read("label_ui.html"), ["settings_bridge.js", "page_store.js", "ask.js", "label_ui.js"]);
  const a = order(read("app_ui.html"), ["ask.js", "app_ui.js"]);
  const c = order(read("cameras_ui.html"), ["settings_bridge.js", "place.js", "signals_chart.js", "cameras_ui.js"]);   // the bridge: Ctrl+1 to Ctrl+4 from this tab
  for (const pos of [t, l, a, c]) assert.deepStrictEqual(pos.filter(p => p < 0), [], "a script tag is missing");
  for (const pos of [t, l, a, c]) assert.deepStrictEqual(pos, [...pos].sort((a, b) => a - b), "scripts out of order");
  // each server names the files it sends (crates/face-server), and the programs carry every one of them (pagefiles.rs)
  const server = f => read(`../crates/face-server/src/${f}`);
  for (const n of ["expression.js", "signals.js", "signals_chart.js", "signals_chart.css", "settings_bridge.js", "page_store.js", "tracker_ui.js", "cameras_ui.js", "ask.js", "jpeg_worker.js", "place.js"]) assert.match(server("tracker.rs"), new RegExp(`"/${n.replace(".", "\\.")}"`));
  assert.match(read("tracker_ui.js"), /new Worker\("\/jpeg_worker\.js"\)/);          // the worker is the file the server sends
  for (const n of ["label_ui.js", "settings_bridge.js", "page_store.js", "ask.js"]) assert.match(server("label.rs"), new RegExp(`"/${n.replace(".", "\\.")}"`));
  for (const n of ["app_ui.js", "ask.js"]) assert.match(server("app.rs"), new RegExp(`"/${n.replace(".", "\\.")}"`));
  const carried = server("pagefiles.rs");
  for (const f of [...SCRIPTS, "signals_chart.css", "tracker_ui.html", "label_ui.html", "app_ui.html", "cameras_ui.html"]) assert.ok(carried.includes(`"${f}"`), `${f} not carried by the programs`);
});

test("the Cameras page: one camera can be looked at alone, and every camera goes on being fetched meanwhile", () => {
  const code = read("cameras_ui.js"), page = read("cameras_ui.html");
  assert.match(code, /function focusOn\(id\)/);
  assert.match(page, /id="allBtn"[^>]*hidden/);                       // the way back, there only while one is looked at
  assert.match(code, /e\.key === "Escape" && solo/);
  // the frames of every camera are asked for whatever is shown (cameras are never paused): the pump never looks at solo
  const pump = code.slice(code.indexOf("async function pumpAll"), code.indexOf("pumpAll();"));
  assert.ok(pump.length > 200 && !/\bsolo\b|\.tile\.hidden/.test(pump), "the frame pump must not depend on what is shown");
  const readouts = code.slice(code.indexOf("async function loadReadouts"), code.indexOf("setInterval(loadReadouts"));
  assert.ok(readouts.length > 200 && !/solo/.test(readouts), "nor the readouts");
});

test("every signal option is switched on only where the device's own server says it is a test device for that", () => {
  const code = read("tracker_ui.js");
  assert.match(code, /allSignals = r\.signals === "all";/);
  assert.match(code, /if \(!allSignals\) return;\r?\n  for \(const id of ALL_SIGNALS\)/);
  assert.match(read("../crates/face-server/src/tracker.rs"), /std::env::var\("FACE_SIGNALS"\)\.ok\(\)\.filter\(\|all\| all == "all"\)/);
  assert.match(read("../docs/SETTINGS.md"), /`FACE_SIGNALS` \| `all`/);
});

test("a phone starts with its front camera and has one button on the picture for the other one", () => {
  const page = read("tracker_ui.html"), code = read("tracker_ui.js");
  // the button: on the picture, bottom right, hidden until a phone with two cameras runs one
  assert.match(page, /<button id="camSwitch" class="camswitch" type="button" hidden aria-label="Switch between the front and the back camera"/);
  assert.match(page, /\.camswitch \{[^}]*right: calc\(14px \+ env\(safe-area-inset-right\)\); bottom: calc\(14px \+ env\(safe-area-inset-bottom\)\);/);
  assert.match(code, /\$\("camSwitch"\)\.hidden = !\(phoneLike\(\) && camCount > 1 && \(stream \|\| switching\)\);/);
  // a phone is told by a finger being its pointer, not by a touch screen alone
  assert.match(code, /const phoneLike = \(\) => navigator\.maxTouchPoints > 0 && matchMedia\("\(pointer: coarse\)"\)\.matches;/);
  // with no camera chosen a phone asks for the front one; the button asks for the other side
  assert.match(code, /const facing = wantFacing \|\| \(phoneLike\(\) \? "user" : ""\);/);
  assert.match(code, /id \? \{ deviceId: \{ exact: id \} \} : facing \? \{ facingMode: \{ ideal: facing \} \} : \{\}/);
  assert.match(code, /const other = before\.facingMode === "environment" \? "user" : before\.facingMode === "user" \? "environment" : "";/);
  // the camera in use is let go of before the other is opened, and a back camera's picture is not mirrored
  const sw = code.slice(code.indexOf("async function switchCamera()"));
  assert.ok(sw.indexOf("stopCamera();") < sw.indexOf("await startCamera();"));
  assert.match(code, /\$\("optMirror"\)\.checked = stream\.getVideoTracks\(\)\[0\]\?\.getSettings\(\)\.facingMode !== "environment";/);
});

test("the privacy policy is read inside the app, and the body readings say they are no medical device's", () => {
  const page = read("tracker_ui.html"), code = read("tracker_ui.js");
  // the policy: the server's own page, in a sheet over the tracker page (the camera goes on behind it)
  assert.match(page, /<a id="privacyLink" href="\/privacy"/);
  assert.match(page, /<div id="privacySheet"[^>]*hidden>/);
  assert.match(page, /<iframe id="privacyFrame" title="Privacy policy"><\/iframe>/);          // loaded only when opened
  assert.match(code, /\$\("privacyFrame"\)\.src = "\/privacy";/);
  assert.match(code, /function closePrivacy\(\) \{ \$\("privacySheet"\)\.hidden = true; \$\("privacyFrame"\)\.removeAttribute\("src"\); \}/);
  assert.match(read("../crates/face-server/src/tracker.rs"), /"\/privacy" => Response::bytes\("text\/html; charset=utf-8", crate::privacy::page\(\)/);
  // it is the repository's one text, carried in the program
  assert.match(read("../crates/face-server/src/privacy.rs"), /include_str!\("\.\.\/\.\.\/\.\.\/PRIVACY\.md"\)/);
  // the words the stores ask of an app that reads a pulse: on the page, in the policy and in the listing
  const words = /not a medical device and does not diagnose, treat, cure or prevent any medical condition/;
  assert.match(page, words);
  assert.match(read("../PRIVACY.md").replace(/\s+/g, " "), words);
  assert.match(read("../docs/store/google-play.md").replace(/[>\s]+/g, " "), words);
});

test("a phone hears for itself: the page's microphone, measured by its own server, and no voice read unless asked", () => {
  const code = read("tracker_ui.js");
  // the page takes over only where its server says it can't listen itself
  assert.match(code, /if \(j\.available === false && j\.page\) \{ hint\.hidden = true; return soundByMic\(\); \}/);
  assert.match(code, /fetch\(`\/api\/sound\/heard\?rate=\$\{ear\.rate\}`/);
  // the microphone running for the chart's sound gives no voice readings: voice is ear only with Listen to voice on
  assert.match(code, /voice = \$\("optVoice"\)\.checked \? ear : null;/);
  assert.match(code, /node\.port\.onmessage = e => ear\?\.push\(/);
  // and it is let go of when the chart's sound is switched off
  assert.match(code, /if \(soundByPage\) \{ soundByPage = false; micSync\(\); \}/);
  const server = read("../crates/face-server/src/tracker.rs");
  assert.match(server, /"\/api\/sound\/heard" =>/);
});

test("on a Mac the page never takes an iPhone's or iPad's microphone for voice, and a phone's own page keeps its own", () => {
  const code = read("tracker_ui.js");
  assert.match(code, /const ON_A_MAC = \/Mac\/i\.test\(navigator\.platform \|\| ""\) && !navigator\.maxTouchPoints;/);
  assert.match(code, /const aPhones = label => ON_A_MAC && \/iphone\|ipad\/i\.test\(label \|\| ""\);/);
  const start = code.slice(code.indexOf("async function startVoice"), code.indexOf("function stopVoice"));
  // what the system gave is looked at before anything is made of it, and a phone's is let go of
  assert.ok(start.indexOf("aPhones(track.label)") > 0 && start.indexOf("aPhones(track.label)") < start.indexOf("new AudioContext"));
  assert.match(start, /if \(!got\) \{ voiceWhy = "nomic"; return; \}/);
});

test("a camera whose picture hasn't come says it is being connected to, from the start (never an empty black box)", () => {
  const code = read("cameras_ui.js"), start = code.slice(code.indexOf("function startTile"), code.indexOf("const sleep"));
  assert.match(code, /function waiting\(t\)/);
  assert.match(start, /waiting\(t\);/);                              // as soon as it is on the page
  assert.match(code, /else if \(t\.msg\.hidden\) waiting\(t\);/);       // and whenever its picture stops with nothing said
  assert.match(read("cameras_ui.html"), /\.vid \.msg\.wait::before \{[^}]*animation: turn/);
});

test("the Cameras grid scrolls instead of squeezing its rows into the window (an opened Signals chart was cut off)", () => {
  assert.match(read("cameras_ui.html"), /#grid \{[^}]*grid-auto-rows: max-content;[^}]*overflow: auto;/);
});

test("the Signals chart: both pages that show it link its style sheet, and it brings nothing from anywhere else", () => {
  for (const f of ["tracker_ui.html", "cameras_ui.html"]) assert.ok(read(f).includes(`<link rel="stylesheet" href="/signals_chart.css">`), f);
  for (const f of ["signals_chart.css", "signals_chart.js"]) assert.doesNotMatch(read(f), /https?:\/\/|@import|url\(/, f);
  // a name or a word from another device is only ever put into the page as text
  assert.doesNotMatch(read("signals_chart.js"), /innerHTML|insertAdjacentHTML|outerHTML|document\.write/);
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
  const servers = ["http.rs", "guard.rs", "label.rs", "tracker.rs", "app.rs"].map(f => read(`../crates/face-server/src/${f}`));
  assert.match(servers[0], /CACHE_CONTROL, HeaderValue::from_static\("no-store"\)/);
  assert.doesNotMatch(servers.join(""), /max-age|immutable/);
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
  // Each tab's page runs in a process of its own (Origin-Agent-Cluster), so a hung page no longer
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
  assert.match(code, /silent\.length \? `silent=\$\{silent\.join\(","\)\}` : ""/);
  assert.match(code, /fetch\("\/alive" \+ \(said \? "\?" \+ said : ""\)/);
  assert.match(code, /m\.fs === "pong"\) \{ if \(answered\[k\]\) answered\[k\]\.at = Date\.now\(\)/);
  assert.match(code, /addEventListener\("load", e => \{\r?\n\s*if \(e\.target\.getAttribute\("src"\)\) answered\[k\] = \{ src: e\.target\.src, at: Date\.now\(\) \}/);
});

test("the tab page gives a tab its address again when its page never began to load", () => {
  // Seen on Windows: now and then the Labeling frame kept the empty page it began with, address set,
  // nothing loading, no error and no load event: blank until the app was started again. Its address given again
  // loads it. A page that has begun to load (slow or not) is never touched.
  const code = read("app_ui.js");
  const head = code.match(/const START_IN = (\d+), AGAIN = (\d+), unbegun = \{\};/);
  const still = code.match(/function emptyStill\(f\) \{\r?\n[\s\S]*?\n\}/);
  const fn = code.match(/function lateTabs\(now, empty\) \{[\s\S]*?\n\}\r?\nconst gaveUp = [^\n]*/);
  assert.ok(head && still && fn, "lateTabs, gaveUp, emptyStill or START_IN isn't written the way this test reads it");
  const START = +head[1], AGAIN = +head[2], L = "http://127.0.0.1:8765/";
  const frames = { "f-track": { src: "" }, "f-label": { src: L }, "f-cams": { src: "" } };
  const ctx = { document: { getElementById: id => frames[id] } };
  vm.createContext(ctx);
  vm.runInContext(`const START_IN = ${START}, AGAIN = ${AGAIN}, unbegun = {}; ${still[0]}; ${fn[0]};
    this.lateTabs = lateTabs; this.gaveUp = gaveUp; this.emptyStill = emptyStill;`, ctx);
  // the frame as the browser shows it: its empty first page can be looked into, a page of another address can't
  const frame = (src, doc) => ({ getAttribute: () => src, get contentDocument() { if (doc === "throws") throw new Error("SecurityError"); return doc; } });
  assert.equal(ctx.emptyStill(frame(L, { URL: "about:blank" })), true);
  assert.equal(ctx.emptyStill(frame(L, null)), false);                 // its page is there (another address: nothing to see)
  assert.equal(ctx.emptyStill(frame(L, "throws")), false);
  assert.equal(ctx.emptyStill(frame("", { URL: "about:blank" })), false);   // no address yet: the program is still starting
  let stuck = true;
  const late = now => [...ctx.lateTabs(now, f => f === frames["f-label"] && stuck)], gone = now => [...ctx.gaveUp(now)];
  const t0 = 1e6;
  assert.deepStrictEqual(late(t0), []);                                // first seen: it gets START_IN to begin
  assert.deepStrictEqual(late(t0 + START - 1000), []);
  assert.deepStrictEqual(late(t0 + START + 1), ["label"]);             // its address again
  assert.deepStrictEqual(late(t0 + START + 2001), []);                 // and twice as long before the next
  assert.deepStrictEqual(late(t0 + START + 1 + 2 * START + 1), ["label"]);
  stuck = false;                                                       // it began to load: left alone from here on
  assert.deepStrictEqual([late(t0 + 60000), gone(t0 + 60000)], [[], []]);
  stuck = true;                                                        // one that never begins: AGAIN tries, then given up
  let t = t0 + 100000, tries = 0;
  for (let i = 0; i < 400; i++, t += 2000) tries += late(t).length;
  assert.equal(tries, AGAIN);
  assert.deepStrictEqual(gone(t), ["label"]);                          // now it counts as a tab that doesn't answer
  frames["f-label"].src = "http://127.0.0.1:9765/";                    // started again at another address: counted anew
  assert.deepStrictEqual([late(t), gone(t)], [[], []]);
  assert.ok(START >= 2000 && START * (AGAIN * (AGAIN + 1) / 2) <= 90000, "long enough for a busy start, short enough to matter");
  assert.match(code, /for \(const k of again\) \{ const f = document\.getElementById\("f-" \+ k\); f\.src = f\.src; \}/);
  assert.match(code, /new Set\(\[\.\.\.silentTabs\(now\), \.\.\.gaveUp\(now\)\]\)/);
  assert.match(code, /again\.length \? `again=\$\{again\.join\(","\)\}` : ""/);
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

test("where the worker makes the JPEGs it also sends this camera's frames, and the page takes over where it can't", () => {
  // Made and sent from the page, a frame waited four times for the page's thread; MediaPipe's parts hold it 100 to
  // 150 ms a check with a face in view, and the stream fell from 20 frames a second to about 10 (2026-11-23).
  const code = read("tracker_ui.js");
  const block = code.match(/const jpegs = \{[\s\S]*?\n(?=\/\/ -+ recognition requests)/);
  assert.ok(block, "the JPEG block isn't where this test reads it");
  const CHROMIUM = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/157.0.0.0 Safari/537.36 Edg/157.0.0.0";
  const WEBKIT = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)";
  const make = ({ worker = true, browser = CHROMIUM, refuses = false } = {}) => {
    const notes = [], posted = [], made = [], clock = { t: 5000 };
    class Worker { constructor(url) { this.url = url; made.push(this); }
      postMessage(m, moved) { if (refuses) throw new Error("no room"); posted.push([m, moved]); }
      terminate() { this.ended = true; } }
    const window = worker ? { Worker, OffscreenCanvas: class {}, createImageBitmap: true } : {};
    const ctx = { window, Worker, navigator: { userAgent: browser }, pageNote: t => notes.push(t),
                  performance: { now: () => clock.t }, setTimeout: (fn, ms) => { const t = setTimeout(fn, ms); t.unref(); return t; } };
    vm.createContext(ctx);
    vm.runInContext(block[0] + "\nthis.streamByWorker = streamByWorker; this.jpegs = jpegs;", ctx);
    const pictures = [], heard = [];
    const picture = () => { const b = { closed: false, close() { this.closed = true; } }; pictures.push(b); return b; };
    return { ctx, notes, posted, made, clock, pictures, heard, frame: (failed = "") => ctx.streamByWorker(picture, 0.72, failed, r => heard.push(r)) };
  };
  let p = make();
  assert.strictEqual(p.frame(), "sent");
  // the picture itself is handed over (the page no longer has it), with what the last frame failed of
  const [m, moved] = p.posted[0];
  assert.deepStrictEqual([m.stream, m.bmp === p.pictures[0], m.q, m.failed, moved[0] === p.pictures[0]], [1, true, 0.72, "", true]);
  assert.strictEqual(p.frame("HTTP 500 no"), "sent");
  assert.strictEqual(p.posted[1][0].failed, "HTTP 500 no");
  // two on their way: a third is left out, not queued, and no picture is made for it
  assert.strictEqual(p.frame(), "wait");
  assert.deepStrictEqual([p.pictures.length, p.posted.length, p.ctx.jpegs.out], [2, 2, 2]);
  // the worker says how each went: the page hears it and the next frame may go
  p.made[0].onmessage({ data: { stream: 1, ok: true } });
  assert.deepStrictEqual([JSON.stringify(p.heard), p.ctx.jpegs.out, p.notes.join()], ['[{"stream":1,"ok":true}]', 1, "JPEGs are made in a worker"]);
  assert.strictEqual(p.frame(), "sent");
  p.made[0].onmessage({ data: { stream: 2, off: true } });               // not shared now: the page is told
  assert.deepStrictEqual(p.heard.at(-1), { stream: 2, off: true });
  // a worker that says nothing of two frames for 6 s is one that stopped: the page takes over and a new worker is
  // tried later, as for a JPEG
  assert.strictEqual(p.frame(), "sent");
  p.clock.t += 5000;
  assert.strictEqual(p.frame(), "wait");
  p.clock.t += 2000;
  assert.strictEqual(p.frame(), "no");
  assert.deepStrictEqual([p.made[0].ended, p.ctx.jpegs.out, p.ctx.jpegs.off], [true, 0, p.clock.t + 60000]);
  assert.match(p.notes.at(-1), /said nothing of a stream frame for 6 s/);
  assert.strictEqual(p.frame(), "no");                                   // while the worker is off: the page's own way
  p.clock.t += 61000;
  assert.deepStrictEqual([p.frame(), p.made.length], ["sent", 2]);        // and a new worker when it is tried again
  // a worker that can't be handed the picture: the picture is let go, the page sends this frame itself, and
  // after three such the worker is off for a while
  p = make({ refuses: true });
  assert.deepStrictEqual([p.frame(), p.frame()], ["no", "no"]);
  assert.deepStrictEqual([p.pictures.every(b => b.closed), p.ctx.jpegs.out, p.ctx.jpegs.off], [true, 0, 0]);
  assert.strictEqual(p.frame(), "no");
  assert.strictEqual(p.ctx.jpegs.off, 5000 + 60000);
  // Where the page makes its own JPEGs (WebKit's window: toBlob doesn't wait there) the worker is there for the
  // stream alone: 15.3 frames a second from the page, 19.3 by the worker, on a Mac with a face in view (2026-11-24)
  p = make({ browser: WEBKIT });
  assert.deepStrictEqual([p.frame(), p.made.length, p.pictures.length, p.posted[0][0].stream], ["sent", 1, 1, 1]);
  assert.deepStrictEqual([p.frame(), p.frame()], ["sent", "wait"]);
  // and when it stops there, the page sends for a while and says so; the page's own JPEGs are not touched
  p.clock.t += 7000;
  assert.deepStrictEqual([p.frame(), p.made[0].ended, p.ctx.jpegs.out, p.ctx.jpegs.off, p.ctx.jpegs.sOff], ["no", true, 0, 0, p.clock.t + 60000]);
  assert.match(p.notes.at(-1), /^This camera's frames are sent by the page itself, the worker is tried again in 1 min: the worker said nothing/);
  assert.strictEqual(p.frame(), "no");
  p.clock.t += 61000;
  assert.deepStrictEqual([p.frame(), p.made.length], ["sent", 2]);
  p.made[1].onerror({ message: "gone", preventDefault() {} });               // its script stops: twice as long before the next
  assert.deepStrictEqual([p.frame(), p.ctx.jpegs.sOff], ["no", p.clock.t + 120000]);
  p = make({ worker: false });                                               // a browser without a worker canvas
  assert.deepStrictEqual([p.frame(), p.made.length, p.pictures.length], ["no", 0, 0]);
  // the page: the worker's way first, its own only on "no", and a frame that was handed over keeps the pace
  assert.match(code, /const how = streamByWorker\(\(\) => \{ streamPicture\(ocomp, octx\); return ocomp\.transferToImageBitmap\(\); \}, 0\.72, streamWhy && streamFailed\(\), streamTold\);/);
  assert.match(code, /const way = streamWay\(\), paced = \(\) => \{ streamWhy = ""; streamNext = Math\.max\(streamNext \+ STREAM_EVERY, now - STREAM_EVERY\); \};/);
  assert.match(code, /if \(way === "transfer" && streamCanvas\(\)\) \{\s+const how = [^\n]+\n\s+if \(how === "sent"\) paced\(\);\s+if \(how !== "no"\) return;/);
  // the page's way where the server names it (an iPhone, measured to lose by the worker's), and nothing but the
  // two ways is taken from the server
  assert.match(code, /streamAsked = \["transfer", "page"\]\.includes\(r\.stream\) \? r\.stream : "";/);
  assert.match(code, /const streamWay = \(\) => streamAsked \|\| "transfer";/);
  const server = fs.readFileSync(path.join(dir, "..", "crates", "face-server", "src", "tracker.rs"), "utf8");
  assert.match(server, /asked\.or_else\(\|\| cfg!\(target_os = "ios"\)\.then\(\|\| "page"\.to_string\(\)\)\)/);
  // MediaPipe's parts beside the page: the page asks the module for that way, and begins the body's and the
  // hands' runs with the frame only while a face is in view
  assert.match(code, /parts = await \(module\.loadAny \|\| module\.load\)\(info\);/);
  assert.match(code, /const begun = parts\?\.begin && tracks\.length \? parts\.begin\(grab, want\) : null;/);
  assert.match(code, /if \(parts\) await parts\.fill\(res, grab, want, begun\);/);
});

test("the worker sends a frame to the page's own server only, in the order the frames were taken", async () => {
  global.OffscreenCanvas = class { constructor(w, h) { this.width = w; this.height = h; }
    getContext(kind) { return kind === "bitmaprenderer" ? { transferFromImageBitmap() {} } : null; }
    convertToBlob(o) { return Promise.resolve({ jpeg: this.width + "x" + this.height, quality: o.quality }); } };
  const W = require("../jpeg_worker.js");
  const bmp = () => ({ width: 960, height: 540, close() {} });
  const asked = [];
  const answer = (status, body, after = 0) => async (url, o) => {
    asked.push([url, o]);
    if (after) await new Promise(r => setTimeout(r, after));
    return { ok: status < 400, status, json: async () => { if (body === null) throw new Error("no JSON"); return body; } };
  };
  // the one address, as the page sends it: its own header, the JPEG as the body, what the last frame failed of
  assert.deepStrictEqual(await W.send(bmp(), 0.72, "TimeoutError: it t\u00f6ok long", answer(200, {})), { ok: true });
  const [url, o] = asked[0];
  assert.deepStrictEqual([url, W.STREAM_TO, o.method, o.body.jpeg, o.body.quality], ["/api/stream/frame", "/api/stream/frame", "POST", "960x540", 0.72]);
  assert.deepStrictEqual(o.headers, { "Content-Type": "image/jpeg", "X-Face-Tool": "1", "X-Stream-Failed": "TimeoutError: it t?ok long" });
  assert.ok(o.signal, "a request that doesn't come back must not stop the stream");
  assert.strictEqual("X-Stream-Failed" in (await W.send(bmp(), 0.72, "", answer(200, {})), asked.at(-1)[1].headers), false);
  // what the server says is what the page acts on
  assert.deepStrictEqual(await W.send(bmp(), 0.72, "", answer(200, { stream: false })), { off: true });
  assert.deepStrictEqual(await W.send(bmp(), 0.72, "", answer(500, { error: "no" })), { error: "HTTP 500 no" });
  assert.deepStrictEqual(await W.send(bmp(), 0.72, "", answer(502, null)), { error: "HTTP 502 " });
  const src = read("jpeg_worker.js");
  assert.deepStrictEqual(src.match(/["'`]\/api\/[^"'`]*["'`]/g), ['"/api/stream/frame"'], "the worker has one address");
  assert.doesNotMatch(src, /https?:\/\//);
  // one after the other: a first frame whose request takes longer is still told of first
  const told = [];
  W.stream(1, bmp(), 0.72, "", r => told.push(r), answer(200, {}, 30));
  W.stream(2, bmp(), 0.72, "", r => told.push(r), answer(200, { stream: false }));
  W.stream(3, bmp(), 0.72, "", r => told.push(r), async () => { const e = new Error("signal timed out"); e.name = "TimeoutError"; throw e; });
  await W.stream(4, bmp(), 0.72, "", r => told.push(r), answer(200, {}));
  assert.deepStrictEqual(told, [{ stream: 1, ok: true }, { stream: 2, off: true }, { stream: 3, error: "TimeoutError: signal timed out" }, { stream: 4, ok: true }]);
  delete global.OffscreenCanvas;
});

test("a device's location is switched on by itself only where the device's own server says it is a test device for that", () => {
  const code = read("tracker_ui.js");
  assert.match(code, /if \(r\.location === "on" && !locationAsked\) locationOn = locationAsked = true;/);
  assert.match(code, /if \(locationOn\) \{ locationOn = false; const box = \$\("optLocation"\); if \(box && !box\.checked\) box\.click\(\); \}/);
  assert.match(read("../crates/face-server/src/tracker.rs"), /"location": std::env::var\("FACE_LOCATION"\)\.ok\(\)\.filter\(\|on\| on == "on"\)/);
  assert.match(read("../docs/SETTINGS.md"), /`FACE_LOCATION` \| `on`/);
  // the options a test device for signals keeps on have no location among them: that is a switch of its own
  assert.doesNotMatch(code.match(/const ALL_SIGNALS = \[([^\]]*)\]/)[1], /optLocation|optPlace/);
});

test("a device's location: off as the page comes, asked of the page's own server only, shown as text, and a typed place never published", () => {
  const html = read("tracker_ui.html"), js = read("tracker_ui.js");
  // off for a new copy: the switch has no "checked", and the server takes anything but "1" for off
  assert.match(html, /<input type="checkbox" id="optLocation">/);
  assert.match(read("../crates/face-server/src/location.rs"), /store\.setting\("track", OPTION\)\.as_deref\(\) == Some\("1"\)/);
  // remembered like the page's other options, so the server reads them
  const keep = js.match(/const KEEP = \[([^\]]*)\]/)[1];
  for (const id of ["optLocation", "optPlace"]) assert.ok(keep.includes(`"${id}"`), id);
  // the page asks nobody but its own server, and never a location service itself
  assert.match(js, /fetch\("\/api\/location", \{ headers: \{ "X-Face-Tool": "1" \} \}\)/);
  for (const f of ["tracker_ui.js", "cameras_ui.js", "place.js"]) assert.doesNotMatch(read(f), /navigator\.geolocation/, f);
  // what the server says is checked and put into the page as text
  assert.match(js, /placeNow = j && j\.on \? Place\.clean\(j\.place\) : null;/);
  assert.match(js, /if \(line\.textContent !== text\) line\.textContent = text;/);
  // the typed place is marked secret: it is not among the settings this node publishes (busSettings leaves those out)
  assert.match(js, /c\("optPlace", L, \{ label: "This device's place, typed", secret: true,/);
  assert.match(js, /\|\| s\.secret\) continue;/);
  // the Cameras tab shows each device's place from its own server's list, as text
  const cams = read("cameras_ui.js");
  assert.match(cams, /const text = Place\.words\(d\.place\);/);
  assert.match(cams, /t\.place\.textContent = text;/);
  // the phones ask for the location only while the app is open: no background location anywhere
  assert.doesNotMatch(read("../apps/android/AndroidManifest.xml"), /ACCESS_BACKGROUND_LOCATION/);
  assert.doesNotMatch(read("../apps/ios/project.sh"), /NSLocationAlways/);
});
