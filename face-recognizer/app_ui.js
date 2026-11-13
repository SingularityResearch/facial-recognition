// app_ui.js - the app window's tab page (app_ui.html, served by app.py): switching tabs, start-up status,
// Check for updates, and the Settings tab, which asks each program's page for its settings
// (settings_bridge.js) and sends changes back.
let current = "track";
function show(k) {
  current = k;
  for (const t of ["track", "label", "cams", "set"]) document.getElementById("tab-" + t).setAttribute("aria-selected", t === k);
  for (const t of ["track", "label", "cams"])
    document.getElementById("f-" + t).classList.toggle("on", t === k && !!document.getElementById("f-" + t).src);
  const f = document.getElementById("f-" + k);
  document.getElementById("settings").hidden = k !== "set";
  document.getElementById("wait").hidden = k === "set" || !!f.src;
  if (k === "set") askAll(); else if (f.src) f.focus();
  try { localStorage.setItem("tab", k); } catch {}
}
try { const t = localStorage.getItem("tab"); if (t === "label" || t === "set" || t === "cams") current = t; } catch {}
const errors = {};
async function poll() {                          // each program's URL once it's serving, or why it failed
  let st = {};
  try { st = await (await fetch("/status", { cache: "no-store" })).json(); } catch {}
  for (const k of ["track", "label"]) {
    const f = document.getElementById("f-" + k), s = st[k] || {};
    if (s.url && !f.src) { f.src = s.url; document.getElementById("st-" + k).textContent = ""; }
    // the Cameras tab: the tracker's page of every device's camera
    if (k === "track" && s.url && !document.getElementById("f-cams").src) document.getElementById("f-cams").src = new URL("cameras", s.url).href;
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
// This page runs: app.py's watchdog brings the window back when these stop (a frozen page renderer)
// An answer nobody reads has to be let go. Until then its body keeps a file descriptor open in the page's process,
// and nothing frees it while the page runs. On Linux (Qt WebEngine, 1024 descriptors) the unread answers of the
// every-second requests used them all in ten minutes and the window froze; the watchdog then restarted the app,
// about every 90 minutes after that. drop(answer) lets the body go; a test checks every fetch reads or drops.
const drop = r => { try { r?.body?.cancel().catch(() => {}); } catch {} return r; };
setInterval(() => { fetch("/alive", { cache: "no-store" }).then(drop, () => {}); }, 2000);
// Start again: a program that failed to start (the model download, say) or whose page says it stopped answering
// (its "Start again" button asks this page, over postMessage). app.py starts it again, with any other program that
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
    STATE[j] = null;
    document.getElementById("st-" + j).textContent = "";
  }
  poll();
}
addEventListener("keydown", e => {               // Ctrl+1 / Ctrl+2 / Ctrl+3 switch tabs, Ctrl+4 the alerts
  const t = { "1": "track", "2": "label", "3": "set" }[e.key];
  if (e.ctrlKey && t) { show(t); e.preventDefault(); }
  if (e.ctrlKey && e.key === "4") { viewAlerts(); e.preventDefault(); }
});
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

// ---------------------------------------------------------------- Settings tab
// The tracker's and the labeling tool's settings live in their own pages (their controls, their browser storage,
// their servers), so this tab asks each page for them (settings_bridge.js, over postMessage) and sends changes back.
const NAMES = { track: "Face tracking", label: "Labeling" }, STATE = { track: null, label: null }, waiting = {};
let rid = 0;
const frame = k => document.getElementById("f-" + k);
const originOf = k => { try { return new URL(frame(k).src).origin; } catch { return null; } };
function tell(k, msg) { const o = originOf(k); if (o && frame(k).contentWindow) frame(k).contentWindow.postMessage(msg, o); }
function askAll() { for (const k of ["track", "label"]) tell(k, { fs: "get" }); drawPrograms(); }
function act(k, id, args) {
  return new Promise(res => { const r = ++rid; waiting[r] = res; tell(k, { fs: "act", id, args, rid: r });
    setTimeout(() => { if (waiting[r]) { delete waiting[r]; res({ ok: false, error: "No answer" }); } }, 30000); });
}
addEventListener("message", e => {
  const k = ["track", "label"].find(k => frame(k).contentWindow === e.source && originOf(k) === e.origin);
  const m = e.data;
  if (!k || !m) return;
  if (m.fs === "state") { STATE[k] = m.items; drawPrograms(); }
  if (k === "track" && m.fs === "alerts") alerts(m);
  // a person forgotten in the labeling tool: the tracker page forgets what it learned about them (its browser storage)
  if (k === "label" && m.fs === "forgot" && typeof m.name === "string" && m.name) act("track", "forgetPerson", { name: m.name });
  if (m.fs === "result" && waiting[m.rid]) { waiting[m.rid](m); delete waiting[m.rid]; }
  // the camera won't start even after the tracker page loaded again: the app starts again (app.py, at most hourly)
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
    const inputs = (it.fields || []).map(f => el("input", { type: f.secret ? "password" : "text", placeholder: f.placeholder || "",
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
function drawPrograms() {
  const box = document.getElementById("sec-programs");
  for (const k of ["track", "label"]) {
    const items = STATE[k];
    if (!items) {                                  // not answered yet: loading, or the program failed
      const id = "sec-wait-" + k;
      const n = document.getElementById(id) || box.appendChild(el("section", { className: "sec", id }));
      n.replaceChildren(el("h2", { textContent: NAMES[k] }),
        el("p", { className: "note", textContent: errors[k] ? `${NAMES[k]} couldn't start, so its settings aren't available.` : "Loading…" }));
      continue;
    }
    document.getElementById("sec-wait-" + k)?.remove();
    for (const it of items) {
      const sid = "sec-" + k + "-" + it.section;
      let sec = document.getElementById(sid);
      if (!sec) {
        sec = box.appendChild(el("section", { className: "sec", id: sid }));
        const resetable = items.filter(x => x.section === it.section && x.def != null && !x.noReset && ["toggle", "range", "select"].includes(x.type));
        const h = el("h2", { textContent: it.section });
        if (resetable.length) h.append(el("button", { className: "b", textContent: "Reset to defaults", type: "button",
          onclick: () => { if (confirm(`Reset "${it.section}" to the default settings?`)) for (const x of STATE[k].filter(x => x.section === it.section && x.def != null && !x.noReset && ["toggle", "range", "select"].includes(x.type))) tell(k, { fs: "set", id: x.id, value: x.def }); } }));
        sec.append(h);
      }
      const key = k + ":" + it.id;
      const row = sec.querySelector(`[data-key="${CSS.escape(key)}"]`) || sec.appendChild(makeRow(k, it));
      update(row, it);
    }
  }
}
// ---------------------------------------------------------------- Alerts
// The tracker page raises them and keeps the inbox (alerts.py); it tells this page how many are new, and the text of
// one that just happened. Here: the bell's count on every tab, a banner over whichever tab is open, and the window's
// title and taskbar button (app.py /attention) so it's seen even when the window is behind others or minimized.
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
