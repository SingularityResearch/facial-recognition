// app_ui.js - the app window's tab page (app_ui.html, served by the app): switching tabs, start-up status,
// Check for updates, and the Settings tab, which asks each program's page for its settings
// (settings_bridge.js) and sends changes back.
// The Cameras page's address: its own port in the tracker's process when it has one (each address is a process of
// its own in this window, so a busy Cameras page can't slow the tracker's page), else the tracker's own address.
const camsUrl = s => s.cameras || new URL("cameras", s.url).href;
const anaUrl = s => new URL("analyze", s.url).href;
// The setup page (setup.html, the labeling tool's): over every tab from the Setup button, and by itself at a new
// copy's first start, when the page, loaded hidden, says nothing is set up yet. It is not a tab: the watchdog
// doesn't ask it, and closing it shows the tab that was open.
const setupUrl = (s, auto) => new URL(auto ? "setup?auto=1" : "setup", s.url).href;
let labelAt = null, setupOpen = false;
function openSetup() {
  const f = document.getElementById("f-setup");
  if (labelAt) { const u = setupUrl(labelAt, false); if (f.src !== u) f.src = u; }
  f.classList.add("on");
  setupOpen = true;
  f.focus();
}
function closeSetup() {
  document.getElementById("f-setup").classList.remove("on");
  setupOpen = false;
  show(current);
}
let current = "track";
function show(k) {
  current = k;
  for (const t of ["track", "label", "cams", "ana", "set"]) document.getElementById("tab-" + t).setAttribute("aria-selected", t === k);
  for (const t of ["track", "label", "cams", "ana"])
    document.getElementById("f-" + t).classList.toggle("on", t === k && !!document.getElementById("f-" + t).src);
  const f = document.getElementById("f-" + k);
  document.getElementById("settings").hidden = k !== "set";
  document.getElementById("wait").hidden = k === "set" || !!f.src;
  // the keyboard follows the tab: left in a tab that is now hidden, typing would still act on that page
  if (k !== "set") { if (f.src) f.focus(); }
  else { askAll(); if (document.activeElement?.tagName === "IFRAME") document.getElementById("tab-set").focus(); }
  try { localStorage.setItem("tab", k); } catch {}
}
try { const t = localStorage.getItem("tab"); if (t === "label" || t === "set" || t === "cams" || t === "ana") current = t; } catch {}
const errors = {};
async function poll() {                          // each program's URL once it's serving, or why it failed
  let st = {};
  try { st = await (await fetch("/status", { cache: "no-store" })).json(); } catch {}
  for (const k of ["track", "label"]) {
    const f = document.getElementById("f-" + k), s = st[k] || {};
    if (s.url && !f.src) { f.src = s.url; document.getElementById("st-" + k).textContent = ""; }
    // the Cameras tab: the tracker's page of every device's camera
    if (k === "track" && s.url && !document.getElementById("f-cams").src) document.getElementById("f-cams").src = camsUrl(s);
    // the Analyze tab: a page of the labeling tool's (it asks that program's server)
    if (k === "label" && s.url && !document.getElementById("f-ana").src) document.getElementById("f-ana").src = anaUrl(s);
    // the setup page, hidden: it says whether a new copy wants it (see openSetup)
    if (k === "label" && s.url && !frame("setup").src) { labelAt = s; frame("setup").src = setupUrl(s, true); }
    if (s.error) { errors[k] = s.error; document.getElementById("st-" + k).textContent = "failed"; }
  }
  show(current);
  const w = document.getElementById("wait");
  if (current !== "set" && errors[current]) {
    w.className = "wait err";
    const again = Object.assign(document.createElement("button"), { className: "b", type: "button", textContent: "Start again" });
    const k = current;
    again.addEventListener("click", () => startAgain(k));
    w.replaceChildren(errors[current], document.createElement("br"), document.createElement("br"), again);
  }
  else {
    w.className = "wait";
    w.textContent = st.fetch   // first start of a downloaded copy: the models come from the project's GitHub Release
      ? `Downloading the face models (first start only): ${Math.round(st.fetch.done / 1e6)} of ${Math.round(st.fetch.total / 1e6)} MB`
      : "Loading the face models…";
  }
  if (!["track", "label"].every(k => document.getElementById("f-" + k).src || errors[k])) setTimeout(poll, 400);
}
poll();
// This page runs: the app's watchdog brings the window back when these stop (a frozen page renderer)
// An answer nobody reads has to be let go. Until then its body keeps a file descriptor open in the page's process,
// and nothing frees it while the page runs. On Linux (Qt WebEngine, 1024 descriptors) the unread answers of the
// every-second requests used them all in ten minutes and the window froze; the watchdog then restarted the app,
// about every 90 minutes after that. drop(answer) lets the body go; a test checks every fetch reads or drops.
const drop = r => { try { r?.body?.cancel().catch(() => {}); } catch {} return r; };
// Each tab's page runs in a process of its own (the servers' Origin-Agent-Cluster header), so that
// nothing one tab does can slow another: a Labeling view with thousands of faces took the camera to 1 frame a
// second while they shared one. A tab's page that hangs therefore no longer stops this page. So this page asks every
// loaded tab whether it runs (settings_bridge.js answers "pong") and tells the watchdog which haven't answered for
// ANSWER_IN: the window then counts as not running, as it did when a hung tab stopped everything.
// Only a page that has loaded is asked and counted, and only for the address it was loaded from: a tab still
// starting, or one being started again, has no page to answer (and a message to a frame whose page isn't there yet
// is an error line in the log).
const ANSWER_IN = 10000, answered = {};            // tab -> { src, at }: its page, and when that last answered or loaded
for (const k of ["track", "label", "cams", "ana"]) document.getElementById("f-" + k).addEventListener("load", e => {
  if (e.target.getAttribute("src")) answered[k] = { src: e.target.src, at: Date.now() };
});
const loadedTabs = () => ["track", "label", "cams", "ana"].filter(k => { const f = document.getElementById("f-" + k); return f.src && answered[k]?.src === f.src; });
function silentTabs(now) {
  return loadedTabs().filter(k => now - answered[k].at > ANSWER_IN);
}
// A tab that was given its address and never began to load it. Seen on Windows, about one start in
// twenty (and once in six on a real installation): the Labeling frame kept the empty page every frame begins with,
// its address set and nothing loading. No error and no load event, so nothing above noticed, and the tab stayed
// blank until the app was started again. Giving the frame its address again loads the page (tried on a window
// stuck that way: half a second). What drops the first try is not shown; it happens when a program is already up
// while the window is still being made (the labeling tool serves a second after the start since 1.25).
// A frame that still has that empty page START_IN after it was first seen with an address is given the address
// again, then after twice as long, and so on, AGAIN times at most; after that the tab counts as one that doesn't
// answer, which is the watchdog's business. A page that is merely slow is not touched: once its address has been
// taken up the frame no longer has the empty page (a page of another address can't be looked into at all).
const START_IN = 3000, AGAIN = 5, unbegun = {};    // tab -> { src, since, tries }: a frame seen with its empty first page
function emptyStill(f) {
  try { return !!f.getAttribute("src") && f.contentDocument?.URL === "about:blank"; } catch { return false; }
}
function lateTabs(now, empty) {                    // the tabs to give their address again now
  const late = [];
  for (const k of ["track", "label", "cams", "ana"]) {
    const f = document.getElementById("f-" + k);
    if (!empty(f)) { delete unbegun[k]; continue; }
    const w = unbegun[k]?.src === f.src ? unbegun[k] : (unbegun[k] = { src: f.src, since: now, tries: 0 });
    if (w.tries < AGAIN && now - w.since > START_IN * (w.tries + 1)) { w.tries++; w.since = now; late.push(k); }
  }
  return late;
}
const gaveUp = now => ["track", "label", "cams", "ana"].filter(k => unbegun[k]?.tries >= AGAIN && now - unbegun[k].since > START_IN * AGAIN);
setInterval(() => {
  for (const k of loadedTabs()) tell(k, { fs: "ping" });
  const now = Date.now(), again = lateTabs(now, emptyStill);
  for (const k of again) { const f = document.getElementById("f-" + k); f.src = f.src; }
  const silent = [...new Set([...silentTabs(now), ...gaveUp(now)])];
  const said = [silent.length ? `silent=${silent.join(",")}` : "", again.length ? `again=${again.join(",")}` : ""].filter(Boolean).join("&");
  fetch("/alive" + (said ? "?" + said : ""), { cache: "no-store" }).then(drop, () => {});
}, 2000);
// A program that was started again may be on another port than the one its tab was loaded from (the old one not
// free yet): every 5 s this page asks where the programs are, and loads a tab again whose address has changed.
setInterval(async () => {
  let st = {};
  try { st = await (await fetch("/status", { cache: "no-store" })).json(); } catch { return; }
  for (const k of ["track", "label"]) {
    const f = document.getElementById("f-" + k), url = st[k]?.url;
    const cams = document.getElementById("f-cams");
    if (k === "track" && url && cams.src && new URL(camsUrl(st[k])).origin !== new URL(cams.src).origin) cams.src = camsUrl(st[k]);
    const ana = document.getElementById("f-ana");
    if (k === "label" && url && ana.src && new URL(url).origin !== new URL(ana.src).origin) ana.src = anaUrl(st[k]);
    const setup = frame("setup");
    if (k === "label" && url && setup.src && new URL(url).origin !== new URL(setup.src).origin) { labelAt = st[k]; setup.src = setupUrl(st[k], !setupOpen); }
    if (!url || !f.src || new URL(url).origin === new URL(f.src).origin) continue;
    f.src = url;
  }
}, 5000);
// Start again: a program that failed to start (the model download, say) or whose page says it stopped answering
// (its "Start again" button asks this page, over postMessage). The app starts it again, with any other program that
// failed; one that still answers is left alone. Its frame is emptied until /status gives its (maybe new) address.
async function startAgain(k) {
  let r = {};
  try { r = await (await fetch(`/restart?what=${k}`, { cache: "no-store", headers: { "X-Face-Tool": "1" } })).json(); } catch {}
  if (r.now === "running") { frame(k).src = frame(k).src; return; }      // it answers: just reload its page
  for (const j of ["track", "label"]) {
    if (j !== k && !errors[j]) continue;
    delete errors[j];
    frame(j).removeAttribute("src");
    if (j === "track") frame("cams").removeAttribute("src");   // the Cameras page is the tracker's: its address may change too
    if (j === "label") frame("ana").removeAttribute("src");    // and Analyze is the labeling tool's
    STATE[j] = null;
    document.getElementById("st-" + j).textContent = "";
  }
  poll();
}
// Ctrl+1 / Ctrl+2 / Ctrl+3 / Ctrl+5 switch tabs, Ctrl+4 the alerts. A key pressed inside a tab goes to that tab's page, not
// to this one: those pages pass these four on (settings_bridge.js, { fs: "key" }; the message listener below).
function hotkey(key) {
  const t = { "1": "track", "2": "label", "3": "set", "5": "ana" }[key];
  if (t) show(t); else if (key === "4") viewAlerts();
  return !!t || key === "4";
}
addEventListener("keydown", e => { if (e.ctrlKey && hotkey(e.key)) e.preventDefault(); });
let newer = false;                               // a newer release was found: the buttons open its download page
async function checkUpdate() {
  if (newer) return fetch("/open-release", { headers: { "X-Face-Tool": "1" } }).then(drop);
  const bs = ["upd", "upd2"].map(id => document.getElementById(id)), say = t => bs.forEach(b => b.textContent = t);
  say("Checking…");
  try {
    const r = await (await fetch("/update", { cache: "no-store", headers: { "X-Face-Tool": "1" } })).json();
    if (r.error) say("Couldn't check");
    else if (r.newer) { say(`${r.latest} is available: open the download page`); newer = true; }
    else say("Up to date");
  } catch { say("Couldn't check"); }
}
// The page's buttons: no inline onclick attributes, so the page's Content-Security-Policy can refuse inline script
document.querySelectorAll("[data-tab]").forEach(b => b.addEventListener("click", () => show(b.dataset.tab)));
document.querySelectorAll("#upd, #upd2").forEach(b => b.addEventListener("click", checkUpdate));
document.querySelectorAll("[data-open]").forEach(b => b.addEventListener("click", () => openDir(b.dataset.open)));
document.querySelectorAll("#setupBtn, #setupBtn2").forEach(b => b.addEventListener("click", openSetup));

// ---------------------------------------------------------------- Settings tab
// The tracker's and the labeling tool's settings live in their own pages (their controls, their browser storage,
// their servers), so this tab asks each page for them (settings_bridge.js, over postMessage) and sends changes back.
const NAMES = { track: "Face tracking", label: "Labeling", ana: "Analyze" }, STATE = { track: null, label: null, ana: null }, waiting = {};
let rid = 0;
const frame = k => document.getElementById("f-" + k);
const originOf = k => { try { return new URL(frame(k).src).origin; } catch { return null; } };
function tell(k, msg) { const o = originOf(k); if (o && frame(k).contentWindow) frame(k).contentWindow.postMessage(msg, o); }
function askAll() { for (const k of ["track", "label", "ana"]) tell(k, { fs: "get" }); drawPrograms(); }
function act(k, id, args) {
  return new Promise(res => { const r = ++rid; waiting[r] = res; tell(k, { fs: "act", id, args, rid: r });
    setTimeout(() => { if (waiting[r]) { delete waiting[r]; res({ ok: false, error: "No answer" }); } }, 30000); });
}
addEventListener("message", e => {
  // the setup page: open itself (a new copy), close, go to a tab (Settings at a group), or open the private folder
  if (frame("setup").contentWindow === e.source && originOf("setup") === e.origin && e.data?.fs === "setup") {
    const m = e.data;
    if (m.want && !setupOpen) openSetup();
    if (m.close) closeSetup();
    if (["track", "label", "set"].includes(m.go)) {
      closeSetup();
      show(m.go);
      if (m.go === "set" && typeof m.group === "string" && /^[a-z]+$/.test(m.group)) document.getElementById("grp-" + m.group)?.scrollIntoView({ block: "start" });
    }
    if (m.open === "private" || m.open === "data") openDir(m.open);
    return;
  }
  const k = ["track", "label", "cams", "ana"].find(k => frame(k).contentWindow === e.source && originOf(k) === e.origin);
  const m = e.data;
  if (!k || !m) return;
  if (m.fs === "pong") { if (answered[k]) answered[k].at = Date.now(); return; }   // this tab's page runs (see silentTabs)
  if (m.fs === "key") { hotkey(String(m.key)); return; }                 // Ctrl+1 to Ctrl+4 pressed inside a tab
  if (k === "cams") return;                                               // the Cameras page says nothing else
  if (m.fs === "state") { STATE[k] = m.items; drawPrograms(); }
  if (k === "track" && m.fs === "alerts") alerts(m);
  // a person forgotten in the labeling tool: the tracker page forgets what it learned about them (its browser storage)
  if (k === "label" && m.fs === "forgot" && typeof m.name === "string" && m.name) act("track", "forgetPerson", { name: m.name });
  if (m.fs === "result" && waiting[m.rid]) { waiting[m.rid](m); delete waiting[m.rid]; }
  // the camera won't start even after the tracker page loaded again: the app starts again (at most hourly)
  if (k === "track" && m.fs === "camstuck") fetch("/restart-app?why=camera", { cache: "no-store", headers: { "X-Face-Tool": "1" } }).then(drop, () => {});
  if (m.fs === "restart") startAgain(k);                                  // the page's own Start again button
  // another device's settings were copied to this one: the tracker page loads them again (the labeling page itself)
  if (k === "label" && m.fs === "reload" && frame("track").src) frame("track").src = frame("track").src;
});
const el = (tag, props = {}, ...kids) => { const n = Object.assign(document.createElement(tag), props); n.append(...kids.filter(x => x != null)); return n; };
// One row per setting; rows are made once and then only updated, so typing and dragging aren't interrupted
function makeRow(k, it) {
  const key = k + ":" + it.id, row = el("div", { className: "row" }), ctl = el("div", { className: "ctl" });
  row.dataset.key = key;
  const lab = el("label", { textContent: it.label, htmlFor: "s-" + key });
  if (it.type === "toggle") {
    const c = el("input", { type: "checkbox", id: "s-" + key });
    c.onchange = () => tell(k, { fs: "set", id: it.id, value: c.checked });
    ctl.append(c);
  } else if (it.type === "range") {
    const r = el("input", { type: "range", id: "s-" + key, min: it.min, max: it.max, step: it.step }), o = el("output");
    const dp = String(it.step).includes(".") ? String(it.step).split(".")[1].length : 0;
    r.oninput = () => { o.value = (+r.value).toFixed(dp); tell(k, { fs: "set", id: it.id, value: +r.value }); };
    r.dataset.dp = dp; ctl.append(r, o);
  } else if (it.type === "select") {
    const sel = el("select", { id: "s-" + key });
    sel.onchange = () => tell(k, { fs: "set", id: it.id, value: sel.value });
    ctl.append(sel);
  } else if (it.type === "text") {
    const t = el("input", { type: "text", id: "s-" + key, autocomplete: "off" });
    t.onchange = () => tell(k, { fs: "set", id: it.id, value: t.value });
    ctl.append(t);
  } else if (it.type === "multi") {                // several of a list: a box of checkboxes with a filter
    const find = el("input", { type: "text", placeholder: "Find…", autocomplete: "off", ariaLabel: "Find in " + it.label });
    const count = el("span", { className: "msg" }), none = el("button", { className: "b", type: "button", textContent: "Clear" });
    const box = el("div", { className: "box", id: "s-" + key, role: "group", ariaLabel: it.label });
    const send = () => { const v = [...box.querySelectorAll("input:checked")].map(c => c.value);
      count.textContent = `${v.length} selected`; box.querySelectorAll("label").forEach(l => l.classList.toggle("sel", l.firstChild.checked));
      tell(k, { fs: "set", id: it.id, value: v }); };
    box.onchange = send;
    none.onclick = () => { box.querySelectorAll("input").forEach(c => c.checked = false); send(); };
    box._filter = () => {                          // as you type: only the names containing it
      const q = find.value.trim().toLowerCase();
      let shown = 0;
      box.querySelectorAll("label").forEach(l => { l.hidden = !!q && !l.firstChild.value.toLowerCase().includes(q); shown += !l.hidden; });
      box.querySelector(".none")?.remove();
      if (!shown) box.append(el("div", { className: "none", textContent: "Nobody by that name" }));
      box.scrollTop = 0;
    };
    find.oninput = box._filter;
    find.onkeydown = e => { if (e.key === "Escape") { find.value = ""; box._filter(); } };
    ctl.append(el("div", { className: "multi" }, el("div", { className: "top" }, find, count, none), box));
    box._count = count;
  } else if (it.type === "info") {
    ctl.append(el("span", { className: "info", id: "s-" + key }));
  } else if (it.type === "button" || it.type === "form") {
    // a field is typed into, or (with `options`, [[value, text]]) chosen from a list
    const inputs = (it.fields || []).map(f => f.options
      ? el("select", { ariaLabel: f.label || f.name })
      : el("input", { type: f.secret ? "password" : "text", placeholder: f.placeholder || "",
        autocomplete: "off", ariaLabel: f.label || f.placeholder || f.name, size: f.size || 20,
        value: f.value ?? "", oninput: e => { e.target.dataset.edited = "1"; } }));
    // several fields: each with its caption above it, wrapping to the next line rather than shrinking
    const parts = inputs.map((inp, j) => {
      const f = it.fields[j], part = f.secret ? secretBox(inp) : inp;
      if ((it.fields || []).length < 2) return part;
      const box = el("label", { className: "fld" }, el("span", { textContent: f.label || f.placeholder || f.name }), part);
      box.style.flex = `${f.size ? 0 : 1} 1 ${(f.size || 18) + 4}ch`;
      return box;
    });
    if ((it.fields || []).length >= 2) ctl.classList.add("fields");
    const b = el("button", { className: "b", id: "s-" + key, type: "button" }), msg = el("span", { className: "msg" });
    b.onclick = async () => {
      const now = row._it || it;                   // as the page describes it now: what the question says changes
      if (now.confirm && !(await askUser(now.confirm, now.confirmYes || "Start"))) return;
      b.disabled = true; msg.className = "msg"; msg.textContent = "…";
      const args = {}; (it.fields || []).forEach((f, j) => args[f.name] = inputs[j].value);
      const r = await act(k, it.id, args);
      b.disabled = false;
      msg.textContent = r.ok ? r.value?.message || (it.type === "form" ? "Saved" : "Started") : r.error; msg.className = r.ok ? "msg" : "msg err";
      if (r.ok) inputs.forEach(i => { delete i.dataset.edited; i.value = ""; });   // the saved values come back with the next state
    };
    ctl._inputs = inputs;
    ctl.append(...parts, b, msg);
  }
  if (["toggle", "range", "select", "text"].includes(it.type)) ctl.append(el("span", { className: "msg err rmsg" }));
  // a form's or button's row name is plain text: as a <label for> the button, a click on the name pressed it, and with
  // the fields empty that saved an empty key or disconnected the shared database
  row.append(["info", "button", "form"].includes(it.type) ? el("span", { className: "lab", textContent: it.label }) : lab, ctl);
  if (it.hint) row.append(el("div", { className: "hint", textContent: it.hint }));
  return row;
}
function update(row, it) {
  const c = row.querySelector("#" + CSS.escape("s-" + row.dataset.key));
  row._it = it;
  row.hidden = !!it.hidden;
  const m = row.querySelector(".rmsg"), h = row.querySelector(".hint");
  if (m) m.textContent = it.error || "";
  if (h && it.hint && h.textContent !== it.hint) h.textContent = it.hint;
  row.classList.toggle("off", !!it.disabled);
  if (!c) return;
  c.disabled = !!it.disabled && it.type !== "info";
  if (document.activeElement === c && (it.type === "text" || it.type === "range")) return;    // don't fight the user
  if (it.type === "toggle") c.checked = !!it.value;
  else if (it.type === "range") { c.value = it.value; c.nextSibling.value = (+it.value).toFixed(+c.dataset.dp); }
  else if (it.type === "select") {
    const opts = JSON.stringify(it.options || []);
    if (c.dataset.opts !== opts) { c.dataset.opts = opts; c.replaceChildren(...(it.options || []).map(([v, t]) => el("option", { value: v, textContent: t }))); }
    c.value = it.value;
  } else if (it.type === "text") c.value = it.value ?? "";
  else if (it.type === "multi") {
    const opts = JSON.stringify(it.options || []), on = new Set(it.value || []);
    if (c.dataset.opts !== opts) {
      c.dataset.opts = opts;
      c.replaceChildren(...(it.options || []).map(([v, t]) => el("label", {}, el("input", { type: "checkbox", value: v }), t)));
      c._filter();                                 // keep what's typed in Find applied
    }
    c.querySelectorAll("label").forEach(l => { l.firstChild.checked = on.has(l.firstChild.value); l.classList.toggle("sel", l.firstChild.checked); });
    c._count.textContent = `${on.size} selected`;
  }
  else if (it.type === "info") c.textContent = it.error ? `Couldn't read this: ${it.error}` : it.value ?? "";
  else if (it.type === "button") c.textContent = it.button;
  else if (it.type === "form") {
    c.textContent = it.button || "Save";
    // the saved values (keys masked): not over what's being typed
    (c.parentElement._inputs || []).forEach((inp, j) => {
      if (inp.tagName === "SELECT") {              // the list as it is now, with what was chosen kept while it is still there
        const opts = JSON.stringify(it.fields?.[j]?.options || []);
        if (inp.dataset.opts !== opts) {
          const chosen = inp.value;
          inp.dataset.opts = opts;
          inp.replaceChildren(...(it.fields?.[j]?.options || []).map(([v, t]) => el("option", { value: v, textContent: t })));
          if ([...inp.options].some(o => o.value === chosen)) inp.value = chosen;
        }
        return;
      }
      const v = it.fields?.[j]?.value;
      if (v !== undefined && document.activeElement !== inp && !inp.dataset.edited) inp.value = v ?? "";
      if (it.fields?.[j]?.placeholder !== undefined) inp.placeholder = it.fields[j].placeholder;
    });
  }
}
const EYE = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12z" fill="none" stroke="currentColor" stroke-width="1.8"/><circle cx="12" cy="12" r="3" fill="currentColor"/></svg>';
// A secret's field with an eye button: the secret shows only while the button is held (mouse, touch, or Space/Enter).
function secretBox(inp) {
  const eye = el("button", { type: "button", className: "eye", title: "Hold to show", ariaLabel: "Hold to show " + (inp.ariaLabel || "the secret") });
  eye.innerHTML = EYE;                           // fixed markup, nothing from outside
  const show = on => { inp.type = on ? "text" : "password"; eye.classList.toggle("on", on); };
  eye.addEventListener("pointerdown", e => { e.preventDefault(); eye.setPointerCapture?.(e.pointerId); show(true); });
  for (const ev of ["pointerup", "pointercancel", "lostpointercapture", "blur"]) eye.addEventListener(ev, () => show(false));
  eye.addEventListener("keydown", e => { if (e.key === " " || e.key === "Enter") { e.preventDefault(); show(true); } });
  eye.addEventListener("keyup", () => show(false));
  return el("span", { className: "secret" }, inp, eye);
}
// The tab's order (the owner, 2026-11-25: "reorganize it so that everything related is grouped together. Like all AI /
// LLM settings should go together and be easy to recognize what each one does"): groups of cards in a fixed order,
// whichever page answers first. A card is a section name the pages give their settings, and may hold settings of
// several pages (Location: the tracker's switch and the labeling tool's history of it), each page's in its own order.
// A section no group names goes under More, so a setting is never lost.
const LAYOUT = [
  { id: "app", title: "App", cards: [] },          // (the App card is in the page itself)
  { id: "camera", title: "Camera and recognition", cards: ["Camera", "Recognition", "On screen"] },
  { id: "readings", title: "Readings", note: "What is read from a face, a voice and a body while the camera runs.",
    cards: ["Expression", "Voice, pulse, posture and sound"] },
  { id: "alerts", title: "Alerts", cards: ["Alerts"] },
  { id: "history", title: "History and logs", note: "What is kept over time: in your shared database, or on this computer.",
    cards: ["Signal history", "The library's history", "Presence log"] },
  { id: "location", title: "Location", cards: ["Location"] },
  { id: "labeling", title: "Labeling", cards: ["Labeling", "Web search"] },
  { id: "ai", title: "AI models", note: "Describe photo and Analyze use a language model on a model server (LM Studio, Ollama, llama.cpp or vLLM), on this computer or another of yours. Describe photo can also use the built-in model, which runs inside this app.",
    cards: ["Model server", "Describe photo", "Analyze", "Model servers on this computer", "Model servers on your other devices"] },
  { id: "music", title: "Music", cards: ["Music"] },
  { id: "devices", title: "Devices and shared database", cards: ["Shared database", "Your devices", "Passphrase for keys"] },
  { id: "computer", title: "This computer", cards: ["This computer"] },
];
const MORE = { id: "more", title: "More", cards: [] };
const PROGRAMS = ["track", "label", "ana"];
const cardId = name => "card-" + String(name).toLowerCase().replace(/[^a-z0-9]+/g, "-");
const resetable = name => PROGRAMS.flatMap(k => (STATE[k] || []).filter(x => x.section === name && x.def != null && !x.noReset
  && ["toggle", "range", "select"].includes(x.type)).map(x => [k, x]));
// (a box goes before the first one placed later than it)
function placeIn(host, box, at) {
  box.dataset.at = at;
  host.insertBefore(box, [...host.children].find(n => n.dataset.at !== undefined && +n.dataset.at > at) || null);
}
function groupBox(g) {
  const id = "grp-" + g.id;
  let box = document.getElementById(id);
  if (box) return box;
  box = el("div", { className: "grp", id });
  // a group of one card of its own name needs no heading over the card's
  if (!(g.cards.length === 1 && g.cards[0] === g.title)) box.append(el("h2", { className: "gt", textContent: g.title }));
  if (g.note) box.append(el("p", { className: "gnote", textContent: g.note }));
  placeIn(document.getElementById("sec-programs"), box, g === MORE ? LAYOUT.length : LAYOUT.indexOf(g));
  return box;
}
function cardBox(name) {
  let sec = document.getElementById(cardId(name));
  if (!sec) {
    const g = LAYOUT.find(x => x.cards.includes(name)) || MORE;
    sec = el("section", { className: "sec", id: cardId(name) });
    sec.append(el("h3", { textContent: name }));
    placeIn(groupBox(g), sec, g === MORE ? 999 : g.cards.indexOf(name));
  }
  const h = sec.firstChild;
  if (!h.querySelector("button") && resetable(name).length) h.append(el("button", { className: "b", textContent: "Reset to defaults", type: "button",
    onclick: async () => { if (await askUser(`Reset "${name}" to the default settings?`, "Reset")) for (const [k, x] of resetable(name)) tell(k, { fs: "set", id: x.id, value: x.def }); } }));
  return sec;
}
function drawPrograms() {
  const waits = document.getElementById("sec-waits");
  PROGRAMS.forEach((k, p) => {
    const items = STATE[k];
    if (!items) {                                  // not answered yet: loading, or the program failed
      const id = "sec-wait-" + k;
      const n = document.getElementById(id) || waits.appendChild(el("p", { className: "note", id }));
      n.textContent = errors[k] ? `${NAMES[k]} couldn't start, so its settings aren't available.` : `${NAMES[k]}'s settings are loading…`;
      return;
    }
    document.getElementById("sec-wait-" + k)?.remove();
    items.forEach((it, i) => {
      const sec = cardBox(it.section), key = k + ":" + it.id;
      let row = sec.querySelector(`[data-key="${CSS.escape(key)}"]`);
      if (!row) {                                  // each page's settings in its own order, the pages in theirs
        row = makeRow(k, it);
        row.dataset.rank = p * 1000 + i;
        sec.insertBefore(row, [...sec.querySelectorAll(":scope > .row")].find(r => +r.dataset.rank > p * 1000 + i) || null);
      }
      update(row, it);
    });
  });
  // the jump list: a group shows once it has a card
  for (const b of document.querySelectorAll("#setnav button")) b.hidden = !document.getElementById(b.dataset.to);
}
// The jump list at the top of the tab: one button a group, in the tab's order.
for (const g of [...LAYOUT, MORE]) {
  const to = g.id === "app" ? "sec-app" : "grp-" + g.id;
  const b = el("button", { type: "button", textContent: g.title, hidden: to !== "sec-app",
    onclick: () => document.getElementById(to)?.scrollIntoView({ block: "start", behavior: "smooth" }) });
  b.dataset.to = to;
  document.getElementById("setnav").append(b);
}
// ---------------------------------------------------------------- Alerts
// The tracker page raises them and keeps the inbox; it tells this page how many are new, and the text of
// one that just happened. Here: the bell's count on every tab, a banner over whichever tab is open, and the window's
// title and taskbar button (/attention) so it's seen even when the window is behind others or minimized.
let unread = 0;
function alerts(m) {
  const n = Math.max(0, Math.floor(+m.unread || 0)), b = document.getElementById("bell"), c = document.getElementById("bellN");
  c.textContent = n; c.hidden = !n; b.classList.toggle("has", n > 0);
  b.setAttribute("aria-label", n ? `Alerts, ${n} new` : "Alerts");
  if (typeof m.latest === "string" && m.latest) {
    document.getElementById("bannerText").textContent = m.latest;
    document.getElementById("banner").hidden = false;
    clearTimeout(alerts.timer);                  // how long: the tracker's "Banner stays" setting (0: until dismissed)
    const ms = Number.isFinite(+m.banner) ? Math.max(0, Math.min(+m.banner, 3600)) * 1000 : 60000;
    if (ms) alerts.timer = setTimeout(() => { document.getElementById("banner").hidden = true; }, ms);
    b.classList.remove("ring"); void b.offsetWidth; b.classList.add("ring");
  }
  // (where the window is a browser's, as on Linux, the page's title is the window's)
  document.title = n ? `Face Recognizer · ${n} new alert${n === 1 ? "" : "s"}` : "Face Recognizer";
  if (n !== unread || m.latest) fetch(`/attention?unread=${n}&flash=${m.latest ? 1 : 0}`, { headers: { "X-Face-Tool": "1" } }).then(drop, () => {});
  unread = n;
}
function viewAlerts() {
  document.getElementById("banner").hidden = true;
  show("track");
  act("track", "openAlerts", {});
}
document.getElementById("bell").addEventListener("click", viewAlerts);
document.getElementById("bannerView").addEventListener("click", viewAlerts);
document.getElementById("bannerOk").addEventListener("click", () => { document.getElementById("banner").hidden = true; });

async function openDir(which) { try { drop(await fetch("/open?what=" + which, { headers: { "X-Face-Tool": "1" } })); } catch {} }
(async () => {
  try { const p = await (await fetch("/paths", { cache: "no-store" })).json();
        document.getElementById("p-data").textContent = p.data; document.getElementById("p-private").textContent = p.private; } catch {}
})();
setInterval(() => { if (current === "set") askAll(); }, 5000);   // statuses (keys, downloads) and the other tabs' changes
