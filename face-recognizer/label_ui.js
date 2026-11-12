// label_ui.js - the script of the labeling tool's page (label_ui.html, served by label_tool.py).
"use strict";
const $ = s => document.querySelector(s);
const esc = s => String(s ?? "").replace(/[&<>"]/g, c => ({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;"}[c]));
const IGN = "#ignore";
const label = n => n === IGN ? "Ignored" : n;
// Stored times are UTC (2026-11-12T01:35:37Z), shown in this computer's local time; older local-time text as it is.
const localTime = s => /^\d{4}-\d\d-\d\dT[\d:]+(\.\d+)?(Z|[+-]\d\d:\d\d)$/.test(s || "")
  ? new Date(s).toLocaleString([], { dateStyle: "medium", timeStyle: "short" }) : (s || "");
const plural = (n, w) => `${n.toLocaleString()} ${w}${n === 1 ? "" : "s"}`;

let STACKOF = {}, STACKN = -1, STACK = new Map();   // same picture again: face -> first face of its stack; tile -> its faces
let SUM = null, FIDS = [], view = { v: "suggested" }, showLow = false, WEBCAM = new Set(), VIDEO = new Set();
let sel = new Set(), order = [], secOf = new Map(), anchor = null;
const PAGE = 160;     // faces shown per section before "show more"

async function get(p) {
  const r = await fetch(p);
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(j.error || `HTTP ${r.status}`);
  return j;
}
async function post(p, body) {
  const r = await fetch(p, { method: "POST", headers: { "Content-Type": "application/json", "X-Face-Tool": "1" }, body: JSON.stringify(body || {}) });
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(j.error || `HTTP ${r.status}`);
  return j;
}
const thumb = i => `/thumb/${i}.jpg?f=${encodeURIComponent(FIDS[i] || "")}`;

// ------------------------------------------------------------------ summary + rail
async function loadSummary() {
  SUM = await get("/api/summary");
  if (SUM.faces !== FIDS.length) FIDS = (await get("/api/fids")).fids;
  if (SUM.faces !== STACKN) { const r = await get("/api/stacks"); STACKOF = r.of; STACKN = r.n; }
  const set = (id, n, hot) => { const e = $("#n-" + id); e.textContent = n ? n.toLocaleString() : ""; e.classList.toggle("hot", !!hot && n > 0); };
  set("suggested", SUM.suggested, true); set("unknown", SUM.unknown, true); set("possible", SUM.possible); set("auto", SUM.auto); set("checks", SUM.checks);
  set("tagged", SUM.tagged, true);
  WEBCAM = new Set(SUM.webcam || []);
  VIDEO = new Set(SUM.video || []);
  const few = SUM.settings.few || 5, needs = p => p.name !== IGN && p.usable < few;
  const nFew = SUM.people.filter(needs).length;
  $("#pfew").textContent = nFew ? `· ${nFew} need more faces` : "";
  // People list order (remembered): A-Z, most faces first, or the ones needing more faces first
  const sortBy = $("#psort").value, list = [...SUM.people];
  if (sortBy === "faces") list.sort((a, b) => b.faces - a.faces);
  else if (sortBy === "few") list.sort((a, b) => (needs(b) - needs(a)) || (a.usable - b.usable) || a.name.localeCompare(b.name));
  list.sort((a, b) => (a.name === IGN) - (b.name === IGN));       // Ignored stays last
  const ppl = $("#people");
  ppl.innerHTML = list.map(p => `<button class="nav" data-person="${esc(p.name)}">
      <img src="${thumb(p.cover)}" alt="">${esc(label(p.name))}
      ${p.name !== IGN && p.usable < 5 ? `<span class="warn" title="Only ${p.usable} usable faces: add more photos of ${esc(p.name)} for reliable recognition">!</span>` : ""}
      <span class="n">${p.faces}</span></button>`).join("") || `<p class="lede" style="padding:0 10px">Nobody named yet.</p>`;
  ppl.querySelectorAll("[data-person]").forEach(b => b.onclick = () => go({ v: "person", name: b.dataset.person }));
  findPeople();
  $("#undo").disabled = !SUM.canUndo;
  $("#undo").title = SUM.canUndo ? `Undo: ${SUM.undoText} (Z)` : "Nothing to undo";
  $("#folders").innerHTML = SUM.settings.folders.map(f => `<option value="${esc(f)}">`).join("");
  $("#peoplelist").innerHTML = names().map(n => `<option value="${esc(n)}">`).join("");
  markNav();
  showImport(SUM.import);
}
const names = () => (SUM?.people || []).filter(p => p.name !== IGN).map(p => p.name);
const person = n => (SUM?.people || []).find(p => p.name === n);

function markNav() {
  document.querySelectorAll("nav .nav").forEach(b => b.setAttribute("aria-current",
    b.dataset.v ? String(view.v === b.dataset.v) : String(view.v === "person" && view.name === b.dataset.person)));
}

// ------------------------------------------------------------------ views
// Each view has its own address (#suggested, #person/Name...), so Back, Forward and reloading work
const hashOf = v => "#" + (v.v === "person" ? "person/" + encodeURIComponent(v.name) : v.v);
function viewOf(hash) {
  const h = hash.replace(/^#/, "");
  if (h.startsWith("person/")) return { v: "person", name: decodeURIComponent(h.slice(7)) };
  return ["suggested", "unknown", "possible", "tagged", "auto", "checks"].includes(h) ? { v: h } : null;
}
function go(v) {
  view = v; clearSel(); render(true);
  if (location.hash !== hashOf(v)) history.pushState(null, "", hashOf(v));
  if (narrowNav.matches) setNav(false);             // narrow: the sidebar covers the faces, so close it
}
addEventListener("popstate", () => { const v = viewOf(location.hash); if (v) { view = v; clearSel(); render(true); } });

// Sidebar fly-out, people search, face size: remembered in this browser
const store = { get: k => { try { return localStorage.getItem("fl-" + k); } catch { return null; } },
                set: (k, v) => { try { localStorage.setItem("fl-" + k, v); } catch {} } };
const narrowNav = matchMedia("(max-width: 760px)");
function setNav(open, save = true) {
  $("#shell").classList.toggle("closed", !open);
  $("#sidebar").inert = !open;
  $("#navbtn").setAttribute("aria-expanded", open);
  if (save && !narrowNav.matches) store.set("nav", open ? "1" : "0");
}
function findPeople() {
  const q = $("#pfind").value.trim().toLowerCase();
  document.querySelectorAll("#people [data-person]").forEach(b => { b.hidden = !!q && !label(b.dataset.person).toLowerCase().includes(q); });
}
function setTile(px) { document.documentElement.style.setProperty("--tile", px + "px"); }

async function render(top) {
  const main = $("#main");
  const scroll = top ? 0 : main.scrollTop;
  markNav();
  let html;
  order = []; secOf = new Map(); STACK.clear();       // stacks are rebuilt by grid() for this view only
  try {
    const low = showLow ? "1" : "0";
    if (SUM && !SUM.faces && !view.name)              // a new copy: nothing added yet
      html = `<div class="empty"><b>No photos yet</b><p style="max-width:560px;margin:6px auto">Click <strong>Add photos</strong> and choose a folder
        of your photos or videos. The faces in them are found on this computer and listed under <strong>New faces</strong>; name a few
        faces of each person and the recognizer learns them, then suggests names for the rest. Your photos, faces and names stay on
        this computer.</p><p><button class="btn primary" id="addfirst">Add photos</button></p></div>`;
    else if (view.v === "suggested") html = viewSuggested(await get(`/api/suggested?low=${low}`));
    else if (view.v === "unknown") html = viewUnknown(await get(`/api/unknown?low=${low}${view.regroup ? "&regroup=1" : ""}`));
    else if (view.v === "possible") html = viewPossible(await get("/api/possiblefaces"));
    else if (view.v === "auto") html = viewAuto(await get("/api/auto"));
    else if (view.v === "tagged") html = viewTagged(await get(`/api/tagged?low=${low}`));
    else if (view.v === "checks") html = viewChecks(await get("/api/checks"));
    else html = viewPerson(await get(`/api/person?name=${encodeURIComponent(view.name)}`));
  } catch (e) { html = `<div class="empty"><b>Couldn't load this view</b>${esc(e.message)}. Is the Face Labels window still open?</div>`; }
  view.regroup = false;
  main.innerHTML = html;
  main.querySelectorAll(".grid").forEach(g => fillGrid(g));
  wire(main);
  main.scrollTop = scroll;
  pruneSel();
  const first = $("#addfirst");
  if (first) first.onclick = () => $("#add").click();
  const scan = $("#scanpossible");
  if (scan) scan.onclick = async () => {
    try { await post("/api/possible/scan", {}); scan.disabled = true; scan.textContent = "Looking… (progress at the top)"; pollImport(); }
    catch (err) { toast(err.message, true); }
  };
}

// sections hold their faces until drawn, so huge views stay fast
const SECTIONS = new Map();
let secId = 0;
function grid(faces, sec, opts = {}) {
  const id = "g" + (++secId);
  // the same picture again (a screenshot avatar, burst shots, copies) shows once, as a stack: acting on the
  // tile acts on every face in it (act() expands it)
  const firstOf = new Map(), out = [];
  for (const f of faces) {
    const k = STACKOF[f.i];
    if (k == null) { out.push(f); continue; }
    if (!firstOf.has(k)) { firstOf.set(k, f); out.push(f); STACK.set(f.i, [f.i]); }
    else STACK.get(firstOf.get(k).i).push(f.i);
  }
  faces = out;
  SECTIONS.set(id, { faces, sec, opts });
  return `<div class="grid" data-grid="${id}"></div>`;
}
function fillGrid(g, all) {
  const { faces, sec, opts } = SECTIONS.get(g.dataset.grid);
  const show = all ? faces : faces.slice(0, PAGE);
  g.innerHTML = show.map(f => tile(f, opts)).join("") +
    (show.length < faces.length ? `<button class="more">+${(faces.length - show.length).toLocaleString()} more</button>` : "");
  for (const f of show) { order.push(f.i); secOf.set(f.i, sec); }
  const more = g.querySelector(".more");
  if (more) more.onclick = () => { order = order.filter(i => !faces.some(f => f.i === i)); fillGrid(g, true); wireTiles(g); };
  if (all) wireTiles(g);
}
function tile(f, o) {
  const s = f.s, q = f.q;
  const lo = q != null && q < SUM.settings.qEnroll;
  let bar = "";
  if (s != null && o.score !== false) {
    const pct = Math.max(6, Math.min(100, (s - 0.25) / 0.5 * 100));
    bar = `<i class="sc ${s < SUM.settings.auto ? "mid" : ""}" style="width:${pct}%"></i>`;
  }
  const cam = WEBCAM.has(f.i), vid = VIDEO.has(f.i);
  const tags = [o.auto !== false && f.by === "auto" ? `<span class="tag">auto</span>` : "", cam ? `<span class="tag cam">webcam</span>` : "",
                vid ? `<span class="tag vid">video</span>` : "",
                lo ? `<span class="tag low">blurry</span>` : ""].join("");
  const tip = [s != null ? `score ${s.toFixed(2)}` : "", lo ? "too blurry to learn from" : "",
               cam ? (f.by === "auto" ? "webcam snapshot, named automatically: it teaches the recognizer once you confirm it" : "webcam snapshot") : "",
               vid ? "from a video" : ""]
    .filter(Boolean).join(", ");
  const k = (STACK.get(f.i) || []).length;
  return `<button class="face ${sel.has(f.i) ? "sel" : ""}" data-i="${f.i}" title="${tip}${k > 1 ? `${tip ? ", " : ""}the same picture in ${k} photos: acting on it acts on all ${k}` : ""}" aria-pressed="${sel.has(f.i)}">
    <img loading="lazy" src="${thumb(f.i)}" alt="">${bar}${tags ? `<span class="tags">${tags}</span>` : ""}${k > 1 ? `<span class="stackn">×${k}</span>` : ""}<span class="zoom" aria-hidden="true">⤢</span></button>`;
}
const F3 = a => ({ i: a[0], s: a[1], q: a[2], by: a[3] });

function head(title, count, lede) {
  return `<div class="head"><h2>${title}</h2>${count != null ? `<span class="cnt" style="color:var(--muted)">${count}</span>` : ""}</div>` + (lede ? `<p class="lede">${lede}</p>` : "");
}
function avatar(n) { const p = person(n); return p ? `<img src="${thumb(p.cover)}" alt="">` : ""; }

function viewSuggested(d) {
  SECTIONS.clear();
  const total = d.groups.reduce((a, g) => a + g.faces.length, 0);
  let h = head("Suggestions", total ? plural(total, "face") : null,
    `The model thinks it knows these faces. Click any that are wrong (or drag across them) and press <kbd>R</kbd> for "not this person", then accept the rest. Least certain faces come first; the bar under each face shows how sure it is.`);
  if (!d.groups.length) return h + `<div class="empty"><b>Nothing to confirm</b>New suggestions appear after you add photos or name more faces.</div>`;
  for (const g of d.groups) {
    const n = g.faces.length;
    h += `<section class="sec" data-sec="${esc(g.name)}"><div class="sechead">${avatar(g.name)}<h3>${esc(label(g.name))}?</h3><span class="cnt">${n}</span>
      <div class="acts"><button class="btn small" data-act="reject-sel">Not ${esc(label(g.name))}</button>
      <button class="btn small primary" data-act="accept-rest" title="Accept every face in this group that isn't selected (A)">Accept ${n === 1 ? "" : "all "}${n}</button></div></div>
      ${grid(g.faces.map(F3), g.name)}</section>`;
  }
  return h;
}

function viewTagged(d) {
  SECTIONS.clear();
  const total = d.groups.reduce((a, g) => a + g.faces.length, 0), st = SUM.facts || {};
  let h = head("From photo tags", total ? plural(total, "face") : null,
    `Faces the photos themselves name: face tags saved by photo apps (Windows Photo Gallery, Lightroom, Picasa, digiKam) or a Google Takeout <code>.json</code> next to the photo. Check them like suggestions: select any that are wrong and press <kbd>R</kbd>, then accept the rest. If a tag spells a name differently from yours, select the faces and press <kbd>N</kbd> instead.` +
    (st.running ? ` <b>Still reading photo tags: ${st.done.toLocaleString()} of ${st.total.toLocaleString()} photos.</b>` : ""));
  if (!d.groups.length) return h + `<div class="empty"><b>No tagged faces waiting</b>Add an export from a photo app that saves people tags (for example a Google Takeout of Google Photos, or photos tagged in Lightroom or digiKam) with <b>Add photos</b>.</div>`;
  for (const g of d.groups) {
    const n = g.faces.length;
    h += `<section class="sec" data-sec="${esc(g.name)}"><div class="sechead">${avatar(g.name)}<h3>${esc(g.name)}</h3>${g.known ? "" : `<span class="newp">new person</span>`}<span class="cnt">${n}</span>
      <div class="acts"><button class="btn small" data-act="reject-sel">Not ${esc(g.name)}</button>
      <button class="btn small primary" data-act="accept-rest" title="Accept every face here that isn't selected (A)">Accept ${n === 1 ? "" : "all "}${n}</button></div></div>
      ${grid(g.faces.map(f => ({ i: f[0], q: f[1] })), g.name)}</section>`;
  }
  return h;
}

function viewAuto(d) {
  SECTIONS.clear();
  const total = d.groups.reduce((a, g) => a + g.faces.length, 0);
  let h = head("Named automatically", total ? plural(total, "face") : null,
    `These faces were named because the model was very sure. You don't have to review them, but a quick look catches the rare mistake: select any that are wrong and press <kbd>R</kbd>. Least certain first.`);
  if (!d.groups.length) return h + `<div class="empty"><b>No automatic names to review</b>Faces get named automatically when you add photos of people you've already named.</div>`;
  for (const g of d.groups) {
    h += `<section class="sec" data-sec="${esc(g.name)}"><div class="sechead">${avatar(g.name)}<h3>${esc(label(g.name))}</h3><span class="cnt">${g.faces.length}</span>
      <div class="acts"><button class="btn small" data-act="reject-sel">Not ${esc(label(g.name))}</button>
      <button class="btn small" data-act="confirm-rest" title="Mark every face here that isn't selected as checked">Looks right</button></div></div>
      ${grid(g.faces.map(f => ({ i: f[0], s: f[1], q: f[2] })), g.name, { auto: false })}</section>`;
  }
  return h;
}

function viewUnknown(d) {
  SECTIONS.clear();
  const n = d.groups.reduce((a, g) => a + g.faces.length, 0) + d.singles.length;
  let h = head("New faces", n ? plural(n, "face") : null,
    `Faces that don't look like anyone you've named, grouped by look-alikes. Name a group, or select some of its faces first to name only those. Faces of strangers, artwork or tattoos: <b>Ignore</b>.`) +
    `<p style="margin:-6px 0 16px"><button class="btn small" id="regroup" title="Group the faces again, including faces that arrived since">Regroup</button></p>`;
  if (!n) return h + `<div class="empty"><b>No new faces</b>Everyone found so far is named, suggested or ignored.</div>`;
  for (const g of d.groups) {
    const sec = "u" + g.id;
    h += `<div class="card" data-sec="${sec}" data-group="1"><div class="sechead"><h3>${plural(g.faces.length, "face")}</h3>
      ${g.hint ? `<button class="hint" data-hint="${esc(g.hint.name)}" title="Average score ${g.hint.s.toFixed(2)}">Looks like ${esc(label(g.hint.name))}? Add to ${esc(label(g.hint.name))}</button>` : ""}
      <div class="acts"><button class="btn small" data-act="name-group">Name… <kbd>N</kbd></button><button class="btn small" data-act="ignore-group">Ignore</button></div></div>
      ${grid(g.faces.map(f => ({ i: f[0], q: f[1] })), sec)}</div>`;
  }
  if (d.singles.length) {
    h += `<section class="sec" data-sec="singles"><div class="sechead"><h3>Seen once</h3><span class="cnt">${d.singles.length}</span>
      <div class="acts" style="color:var(--muted);font-size:13px">Select faces, then <kbd>N</kbd> to name or <kbd>X</kbd> to ignore. Open one (double-click) for clues to who it is.</div></div>
      ${grid(d.singles.map(f => ({ i: f[0], q: f[1] })), "singles")}</section>`;
  }
  return h;
}

function viewPossible(d) {
  SECTIONS.clear();
  const likely = d.faces.filter(f => f[4]), unlikely = d.faces.filter(f => !f[4]);
  let h = head("Possible faces", d.faces.length ? plural(d.faces.length, "face") : null,
    `Faces the detector was less sure of (45% to 85%): blurry, turned, small or partly hidden faces, and many things that only look like faces. Sorted by how much each looks like a real face (learned from the faces you've named and ignored); only the first group gets dashed boxes in the photo view. Once one looks like someone you've named, it moves to <b>Suggestions</b>. Not a face: <b>Ignore</b> (that also teaches the sorting).`);
  if (d.left) h += `<p style="margin:-6px 0 16px"><button class="btn small" id="scanpossible" title="Looks at every photo added before possible faces were kept (videos and online-only cloud photos are skipped)">Look in ${plural(d.left, "photo")} added before</button></p>`;
  if (!d.faces.length) return h + `<div class="empty"><b>No possible faces${d.left ? " yet" : ""}</b>${d.left ? "Look in the photos added before to find them." : "Every one is named, suggested or ignored."}</div>`;
  const sec = (key, title, faces, note) => faces.length ? `<section class="sec" data-sec="${key}"><div class="sechead"><h3>${title}</h3><span class="cnt">${faces.length}</span>
    <div class="acts" style="color:var(--muted);font-size:13px">${note}</div></div>
    ${grid(faces.map(f => ({ i: f[0], q: f[1] })), key)}</section>` : "";
  h += sec("possible", "Look like faces", likely, "Select faces, then <kbd>N</kbd> to name or <kbd>X</kbd> to ignore. Double-click one to see it in its photo.");
  h += sec("unlikely", "Less likely to be faces", unlikely, "Mostly food, objects and textures; no dashed boxes for these.");
  return h;
}

function viewChecks(d) {
  SECTIONS.clear();
  let h = head("Checks", null, "Possible labeling mistakes and gaps the recognizer would trip over.");
  h += `<section class="sec"><div class="sechead"><h3>Might be mislabeled</h3><span class="cnt">${d.mislabeled.length}</span></div>`;
  if (!d.mislabeled.length) h += `<p class="lede">Every named face looks like the rest of its person.</p>`;
  for (const m of d.mislabeled) {
    order.push(m.i);
    h += `<div class="row" data-i="${m.i}">${tile({ i: m.i, q: m.q }, { score: false })}
      <div class="why">Named <b>${esc(m.name)}</b>${m.by === "auto" ? " automatically" : ""}
        <small>${m.other && m.os > m.s ? `Looks more like ${esc(label(m.other))} (${m.os.toFixed(2)} vs ${m.s.toFixed(2)})` : `Doesn't look much like ${esc(m.name)}'s other photos (${m.s.toFixed(2)})`}</small></div>
      <div class="chips"><button class="chip" data-row="ok">Keep ${esc(m.name)}</button>
      ${m.other && m.os > m.s ? `<button class="chip" data-row="move" data-to="${esc(m.other)}">${m.other === IGN ? "Ignore" : "It's " + esc(m.other)}</button>` : ""}
      <button class="chip" data-row="reject" data-name="${esc(m.name)}">Not ${esc(m.name)}</button>
      <button class="chip" data-row="photo">See photo</button></div></div>`;
  }
  h += `</section><section class="sec"><div class="sechead"><h3>Needs more photos</h3><span class="cnt">${d.few.length}</span></div>`;
  h += d.few.length ? `<p class="lede">People need about 5 clear faces to be recognized reliably from different angles and lighting.</p><div class="chips">` +
    d.few.map(f => `<button class="chip" data-person="${esc(f.name)}">${esc(f.name)}: ${f.usable} clear face${f.usable === 1 ? "" : "s"}</button>`).join("") + `</div>`
    : `<p class="lede">Everyone has enough clear faces.</p>`;
  h += `</section><section class="sec"><div class="sechead"><h3>Duplicate photos skipped</h3><span class="cnt">${d.duplicates.length}</span></div>`;
  h += d.duplicates.length ? `<details><summary>Show the copies that were skipped</summary><ul>${d.duplicates.slice(0, 500).map(x => `<li>${esc(x.path)}<br>copy of ${esc(x.of)}</li>`).join("")}</ul></details>`
    : `<p class="lede">None so far.</p>`;
  return h + `</section>`;
}

function viewPerson(d) {
  SECTIONS.clear();
  const n = d.name, p = person(n);
  if (!d.faces.length) return head(esc(label(n))) + `<div class="empty"><b>No faces named ${esc(label(n))}</b></div>`;
  const autoN = d.faces.filter(f => f[3] === "auto").length;
  const lede = n === IGN ? "Strangers, artwork and other faces that are never recognized. Select faces and press <kbd>N</kbd> to give them a name instead."
    : `${p ? plural(p.usable, "clear face") + " used for recognition" : ""}${autoN ? `, ${autoN} named automatically` : ""}. Faces that look least like ${esc(n)} come first, so mistakes surface at the top.`;
  return `<div class="head">${n !== IGN ? avatar(n).replace("<img", '<img style="width:44px;height:44px;border-radius:50%;object-fit:cover"') : ""}<h2>${esc(label(n))}</h2>
      <span style="color:var(--muted)">${plural(d.faces.length, "face")}</span>
      ${n !== IGN ? `<button class="btn small" id="rename">Rename or merge</button><button class="btn small" id="photosbtn">Photos…</button><button class="btn small" id="persondata" title="Export everything held about this person, or forget them">Their data…</button>` : ""}</div><p class="lede">${lede}</p>
    <section class="sec" data-sec="${esc(n)}">${grid(d.faces.map(F3), n)}</section>`;
}

// ------------------------------------------------------------------ wiring
function wire(root) {
  wireTiles(root);
  root.querySelectorAll("[data-act]").forEach(b => b.onclick = () => sectionAct(b.dataset.act, b.closest("[data-sec]")));
  root.querySelectorAll("[data-hint]").forEach(b => b.onclick = () => {
    const box = b.closest("[data-sec]"); act("name", facesIn(box, true), b.dataset.hint);
  });
  root.querySelectorAll("[data-person]").forEach(b => b.onclick = () => go({ v: "person", name: b.dataset.person }));
  root.querySelectorAll("[data-row]").forEach(b => b.onclick = () => {
    const i = +b.closest(".row").dataset.i, k = b.dataset.row;
    if (k === "ok") act("ok", [i]);
    if (k === "move") b.dataset.to === IGN ? act("ignore", [i]) : act("name", [i], b.dataset.to);
    if (k === "reject") act("reject", [i], b.dataset.name);
    if (k === "photo") openPhoto(i);
  });
  const rg = root.querySelector("#regroup"); if (rg) rg.onclick = () => { view.regroup = true; render(); };
  const rn = root.querySelector("#rename"); if (rn) rn.onclick = () => openRename(view.name);
  const pb = root.querySelector("#photosbtn"); if (pb) pb.onclick = () => openPhotos(view.name);
  const pd = root.querySelector("#persondata"); if (pd) pd.onclick = () => openPersonData(view.name);
}
function wireTiles(root) {
  root.querySelectorAll(".face").forEach(b => {
    const i = +b.dataset.i;
    b.onclick = e => {
      if (e.target.closest(".zoom") || !secOf.has(i)) return openPhoto(i);
      if (e.shiftKey && anchor != null) {
        const a = order.indexOf(anchor), z = order.indexOf(i);
        if (a >= 0 && z >= 0) for (let k = Math.min(a, z); k <= Math.max(a, z); k++) setSel(order[k], true);
      } else { setSel(i, !sel.has(i)); anchor = i; }
      selbar();
    };
    b.ondblclick = () => openPhoto(i);
  });
}
function facesIn(box, onlySelIfAny) {
  const all = [...box.querySelectorAll(".face")].map(b => +b.dataset.i);
  const grid = box.querySelector("[data-grid]");
  const full = grid ? SECTIONS.get(grid.dataset.grid).faces.map(f => f.i) : all;
  const picked = full.filter(i => sel.has(i));
  return onlySelIfAny && picked.length ? picked : full;
}
async function sectionAct(kind, box) {
  const sec = box.dataset.sec, full = facesIn(box, false), picked = full.filter(i => sel.has(i));
  if (kind === "reject-sel") {
    if (!picked.length) return toast(`Select the faces that aren't ${label(sec)} first`, true);
    return sec === IGN ? act("clear", picked) : act("reject", picked, sec);
  }
  if (kind === "accept-rest") {
    const rest = full.filter(i => !sel.has(i));
    return sec === IGN ? act("ignore", rest) : act("name", rest, sec);
  }
  if (kind === "confirm-rest") return act("confirm", full.filter(i => !sel.has(i)));
  if (kind === "ignore-group") return act("ignore", picked.length ? picked : full);
  if (kind === "name-group") {
    const faces = picked.length ? picked : full;
    const n = await pickName(`Name ${plural(faces.length, "face")}`);
    if (n) act(n === IGN ? "ignore" : "name", faces, n);
  }
}

// ------------------------------------------------------------------ selection
function setSel(i, on) {
  on ? sel.add(i) : sel.delete(i);
  document.querySelectorAll(`.face[data-i="${i}"]`).forEach(b => { b.classList.toggle("sel", on); b.setAttribute("aria-pressed", on); });
}
function clearSel() { [...sel].forEach(i => setSel(i, false)); selbar(); }
function pruneSel() { for (const i of [...sel]) if (!secOf.has(i)) sel.delete(i); selbar(); }
function selSections() { return new Set([...sel].map(i => secOf.get(i))); }
function personSec(s) { return s && !/^u\d+$/.test(s) && s !== "singles" ? s : null; }

function selbar() {
  const bar = $("#selbar");
  if (!sel.size) { bar.classList.remove("on"); return; }
  const secs = [...selSections()], one = secs.length === 1 ? personSec(secs[0]) : null;
  const named = secs.every(personSec);
  const inSugg = view.v === "suggested" || view.v === "tagged";
  bar.innerHTML = `<span class="count">${plural(expand([...sel]).length, "face")}</span>
    ${inSugg ? `<button class="go" data-k="a">Accept<kbd>A</kbd></button>` : ""}
    <button data-k="n">Name…<kbd>N</kbd></button>
    ${named ? `<button data-k="r">${one ? "Not " + esc(label(one)) : "Not this person"}<kbd>R</kbd></button>` : ""}
    <button data-k="x">Ignore<kbd>X</kbd></button>
    ${sel.size === 1 ? `<button data-k="p">See photo<kbd>Space</kbd></button>` : ""}
    <button data-k="esc" title="Clear selection (Esc)">✕</button>`;
  bar.querySelectorAll("button").forEach(b => b.onclick = () => key(b.dataset.k));
  bar.classList.add("on");
}

async function key(k) {
  const faces = [...sel];
  if (k === "esc") return clearSel();
  if (!faces.length) return;
  if (k === "p") return openPhoto(faces[0]);
  if (k === "x") return act("ignore", faces);
  if (k === "n") {
    const n = await pickName(`Name ${plural(faces.length, "face")}`);
    if (n) act(n === IGN ? "ignore" : "name", faces, n);
  }
  if (k === "a" && (view.v === "suggested" || view.v === "tagged")) {      // accept each selected face as its section's person
    const by = groupBy(faces);
    for (const [s, f] of by) await act(s === IGN ? "ignore" : "name", f, s, by.size > 1);
    if (by.size > 1) done(`Accepted ${plural(faces.length, "face")}`);
  }
  if (k === "r") {
    const by = groupBy(faces);
    if ([...by.keys()].some(s => !personSec(s))) return;
    for (const [s, f] of by) await act(s === IGN ? "clear" : "reject", f, s, by.size > 1);
    if (by.size > 1) done(`Marked ${plural(faces.length, "face")} as not who they were suggested as`);
  }
}
function groupBy(faces) { const m = new Map(); for (const i of faces) { const s = secOf.get(i); m.set(s, [...(m.get(s) || []), i]); } return m; }

const expand = faces => [...new Set(faces.flatMap(i => STACK.get(i) || [i]))];   // a stack tile stands for all its faces
async function act(action, faces, name, quiet) {
  if (!faces.length) return;
  faces = expand(faces);
  document.querySelectorAll(faces.map(i => `.face[data-i="${i}"]`).join(",")).forEach(b => b.classList.add("gone"));
  try {
    const r = await post("/api/act", { action, faces, name });
    faces.forEach(i => sel.delete(i));
    if (!quiet) done(r.undo);
  } catch (e) { toast(e.message, true); render(); }
}
async function done(msg) { await loadSummary(); await render(); toast(msg, false, true); }
async function undo() {
  try { const r = await post("/api/undo"); if (r.undone) { await loadSummary(); await render(); toast("Undone: " + r.undone); } }
  catch (e) { toast(e.message, true); }
}

// ------------------------------------------------------------------ rubber-band selection
(() => {
  const main = $("#main"), band = $("#band");
  let start = null, base = null;
  main.addEventListener("mousedown", e => {
    if (e.button !== 0 || e.target.closest("button, input, a, summary, .row")) return;
    start = { x: e.clientX, y: e.clientY }; base = e.shiftKey || e.ctrlKey ? new Set(sel) : new Set();
    e.preventDefault();
  });
  addEventListener("mousemove", e => {
    if (!start) return;
    const x1 = Math.min(start.x, e.clientX), y1 = Math.min(start.y, e.clientY), x2 = Math.max(start.x, e.clientX), y2 = Math.max(start.y, e.clientY);
    if (x2 - x1 < 4 && y2 - y1 < 4) return;
    Object.assign(band.style, { display: "block", left: x1 + "px", top: y1 + "px", width: x2 - x1 + "px", height: y2 - y1 + "px" });
    main.querySelectorAll(".grid .face").forEach(b => {
      const r = b.getBoundingClientRect(), i = +b.dataset.i;
      const hit = r.right > x1 && r.left < x2 && r.bottom > y1 && r.top < y2;
      setSel(i, hit || base.has(i));
    });
    selbar();
  });
  addEventListener("mouseup", () => {
    if (start && band.style.display !== "block" && !base.size) clearSel();   // plain click on empty space
    start = null; band.style.display = "none";
  });
})();

// ------------------------------------------------------------------ name picker
let pickResolve = null, pickItems = [], pickIdx = 0;
function pickName(title, start = "") {
  $("#namer .ttl").textContent = title;
  $("#nameinput").value = start;
  $("#namer").classList.add("on");
  $("#nameinput").focus();
  $("#nameinput").select();
  fillPicker();
  return new Promise(r => pickResolve = r);
}
function fillPicker() {
  const q = $("#nameinput").value.trim(), ql = q.toLowerCase();
  const ppl = (SUM.people || []).filter(p => p.name !== IGN);
  let items = ppl.filter(p => !ql || p.name.toLowerCase().includes(ql))
    .sort((a, b) => (b.name.toLowerCase().startsWith(ql) - a.name.toLowerCase().startsWith(ql)) || b.faces - a.faces)
    .map(p => ({ name: p.name, html: `<img src="${thumb(p.cover)}" alt="">${esc(p.name)}<small>${p.faces}</small>` }));
  if (q && !ppl.some(p => p.name.toLowerCase() === ql) && !q.startsWith("#"))
    items.push({ name: q, html: `<span class="ph">+</span>New person: <b>${esc(q)}</b>` });
  items.push({ name: IGN, html: `<span class="ph">✕</span>Ignore (stranger, artwork, not a face)` });
  pickItems = items; pickIdx = 0;
  $("#namelist").innerHTML = items.map((it, k) => `<li role="option" data-k="${k}" aria-selected="${k === 0}">${it.html}</li>`).join("");
  $("#namelist").querySelectorAll("li").forEach(li => li.onclick = () => closePicker(pickItems[+li.dataset.k].name));
}
function movePick(d) {
  pickIdx = (pickIdx + d + pickItems.length) % pickItems.length;
  $("#namelist").querySelectorAll("li").forEach((li, k) => { li.setAttribute("aria-selected", k === pickIdx); if (k === pickIdx) li.scrollIntoView({ block: "nearest" }); });
}
function closePicker(v) { $("#namer").classList.remove("on"); const r = pickResolve; pickResolve = null; r && r(v || null); }
$("#nameinput").oninput = fillPicker;
$("#nameinput").onkeydown = e => {
  if (e.key === "ArrowDown") { e.preventDefault(); movePick(1); }
  else if (e.key === "ArrowUp") { e.preventDefault(); movePick(-1); }
  else if (e.key === "Enter") { e.preventDefault(); closePicker(pickItems[pickIdx]?.name); }
  else if (e.key === "Escape") { e.preventDefault(); closePicker(null); }
};
$("#namer").onclick = e => { if (e.target.id === "namer") closePicker(null); };

// ------------------------------------------------------------------ rename
// Photos of a person, or of several people together; copy them to a folder (labels.py photos_with / export)
let phNames = [];
function phWanted(first) {
  return [first, ...$("#phalso").value.split(",").map(s => s.trim()).filter(Boolean)];
}
async function phRefresh() {
  const names = phNames, q = names.map(n => "with=" + encodeURIComponent(n)).join("&");
  $("#phtitle").textContent = names.length > 1 ? `Photos with ${names.join(" and ")} together` : `Photos with ${names[0]}`;
  try {
    const d = await get(`/api/photos?${q}`);
    const here = d.photos.filter(p => p.exists).length;
    $("#phcount").textContent = d.photos.length
      ? `${plural(d.photos.length, "photo")}${here < d.photos.length ? `, ${here} of them on this computer` : ""}. Click one to open it.`
      : "No photos with all of them named.";
    $("#phgrid").innerHTML = d.photos.slice(0, 120).map(p =>
      `<img src="${thumb(p.i)}" data-ph="${p.i}" alt="" title="${esc(p.date)} · ${esc(p.people.join(", "))}" style="width:56px;height:56px;border-radius:6px;cursor:pointer;opacity:${p.exists ? 1 : .4}">`).join("")
      + (d.photos.length > 120 ? `<span class="meta">and ${d.photos.length - 120} more</span>` : "");
    $("#phgrid").querySelectorAll("[data-ph]").forEach(img => img.onclick = () => { $("#photosdlg").classList.remove("on"); openPhoto(+img.dataset.ph); });
  } catch (e) { $("#phcount").textContent = String(e.message || e); }
}
function openPhotos(n) {
  $("#phalso").value = ""; $("#pherr").textContent = ""; phNames = [n];
  $("#photosdlg").classList.add("on");
  phRefresh();
  $("#phalso").onchange = () => { phNames = phWanted(n); phRefresh(); };
}
$("#phclose").onclick = () => $("#photosdlg").classList.remove("on");
$("#phform").onsubmit = async e => {
  e.preventDefault();
  const folder = $("#phfolder").value.trim();
  if (!folder) return ($("#pherr").textContent = "Type the folder to copy the photos to.");
  $("#pherr").textContent = "Copying…";
  try {
    const r = await post("/api/export", { names: phNames, folder });
    $("#pherr").textContent = "";
    toast(`Copied ${plural(r.copied, "photo")} to ${folder}${r.skipped ? ` (${r.skipped} already there)` : ""}${r.missing ? `; ${r.missing} not on this computer` : ""}`);
  } catch (err) { $("#pherr").textContent = String(err.message || err); }
};

// A person's data (persondata.py): export everything held about them, or forget them everywhere
let pdName = "";
function openPersonData(n) {
  pdName = n;
  $("#pdtitle").textContent = `${n}: their data`;
  $("#pdconfirm").value = ""; $("#pderr").textContent = "";
  $("#persondlg").classList.add("on");
}
$("#pdclose").onclick = () => $("#persondlg").classList.remove("on");
$("#pdexport").onclick = async () => {
  const folder = $("#pdfolder").value.trim();
  if (!folder) return ($("#pderr").textContent = "Type the folder to export into.");
  $("#pderr").textContent = "Exporting…";
  try {
    const r = await post("/api/person/export", { name: pdName, folder });
    $("#pderr").textContent = "";
    toast(`Exported ${plural(r.faces, "face")} of ${pdName} to ${r.folder}`);
  } catch (err) { $("#pderr").textContent = String(err.message || err); }
};
$("#pdforget").onclick = async () => {
  const typed = $("#pdconfirm").value.trim();
  if (typed !== pdName) return ($("#pderr").textContent = `Type ${pdName} exactly to confirm.`);
  if (!confirm(`Forget ${pdName} everywhere? This deletes their face prints, pictures and every record naming them, and can't be undone.`)) return;
  try {
    const r = await post("/api/person/forget", { name: pdName, confirm: typed });
    $("#persondlg").classList.remove("on");
    SettingsBridge.notify({ fs: "forgot", name: pdName });     // the app window's tracker tab: their learned normals
    view = { v: "suggested" };
    history.replaceState(null, "", hashOf(view));
    done(`Forgot ${pdName}: ${plural(r.removed.faces, "face")} kept only as Ignored places, ${plural(r.removed.history, "history line")} removed`);
  } catch (err) { $("#pderr").textContent = String(err.message || err); }
};

function openRename(n) {
  $("#rentitle").textContent = `Rename ${n}`;
  $("#renname").value = n;
  $("#renamer").classList.add("on");
  $("#renname").select();
}
$("#rencancel").onclick = () => $("#renamer").classList.remove("on");
$("#renform").onsubmit = async e => {
  e.preventDefault();
  const nw = $("#renname").value.trim(), old = view.name;
  if (!nw || nw === old) return $("#renamer").classList.remove("on");
  try {
    await post("/api/rename", { old, new: nw });
    $("#renamer").classList.remove("on");
    view = { v: "person", name: nw };
    history.replaceState(null, "", hashOf(view));      // the old name's address no longer exists
    done(names().includes(nw) ? `Merged ${old} into ${nw}` : `Renamed ${old} to ${nw}`);
  } catch (err) { toast(err.message, true); }
};

// ------------------------------------------------------------------ photo viewer
const clock = t => `${Math.floor(t / 60)}:${String(Math.floor(t % 60)).padStart(2, "0")}`;
let vIndex = null, vInfo = null;
async function openPhoto(i) {
  vIndex = i;
  $("#viewer").classList.add("on");
  const info = await get(`/api/photoinfo/${i}`).catch(() => null);
  if (vIndex !== i) return;
  const me = info?.faces.find(f => f.i === i);
  const nm = me?.name;
  $("#vwrap").innerHTML = info?.exists
    ? `<img id="vimg" src="/photo/${i}.jpg?f=${encodeURIComponent(FIDS[i] || "")}" alt="Photo">`
    : `<div class="nofile"><img src="${thumb(i)}" alt="" style="width:224px;height:224px;border-radius:12px"><p>The original ${info?.video != null ? "video" : "photo"} isn't on this computer any more,<br>so only the face is shown.</p></div>`;
  const img = $("#vimg");
  if (img) img.onload = () => {
    const s = img.clientWidth / img.naturalWidth;
    for (const f of info.faces) {
      if (f.shown === false && f.i !== i) continue;   // possible face that doesn't look enough like a real face
      const b = document.createElement("div");
      b.className = "box" + (f.i === i ? " me" : "") + (f.maybe != null && !f.name ? " maybe" : "");
      Object.assign(b.style, { left: f.box[0] * s + "px", top: f.box[1] * s + "px", width: f.box[2] * s + "px", height: f.box[3] * s + "px" });
      b.innerHTML = `<span>${esc(f.name ? label(f.name) : f.maybe === 0 ? "Drawn" : f.maybe != null ? `Face? ${Math.round(f.maybe * 100)}%` : "?")}</span>`;
      b.title = f.name ? label(f.name) : f.maybe === 0 ? "Drawn by hand, not named yet" : f.maybe != null ? `Possible face (the detector is ${Math.round(f.maybe * 100)}% sure), not named yet` : "Not named yet";
      b.onclick = () => openPhoto(f.i);
      $("#vwrap").appendChild(b);
    }
    if (store.get("possible") !== "0") showPossible(i, s);
  };
  const others = (info?.faces || []).filter(f => f.i !== i);
  vInfo = info;
  $("#vside").innerHTML = `
    <div><div class="who">${nm ? esc(label(nm)) : "Not named yet"}</div>
      <div class="meta">${info?.video != null ? `From a video, at ${clock(info.video)}<br>` : ""}${info?.date ? esc(info.date) + "<br>" : ""}${esc(info?.path || "")}</div></div>
    <div class="acts"><button class="btn primary" data-v="n">Name… <kbd>N</kbd></button>
      ${nm && nm !== IGN ? `<button class="btn" data-v="r">Not ${esc(nm)} <kbd>R</kbd></button>` : ""}
      ${nm !== IGN ? `<button class="btn" data-v="x">Ignore <kbd>X</kbd></button>` : ""}</div>
    ${others.length ? `<div class="meta">Also in this ${info?.video != null ? "frame" : "photo"}: ${others.map(f => esc(f.name ? label(f.name) : "someone not named yet")).join(", ")}. Click a box to switch to that face.</div>` : ""}
    ${info?.exists ? `<div class="clue" id="desc"><h4>The photo, described <small id="descwhere">(by a model on this computer)</small></h4>
      <div id="desctext"></div>
      <div class="acts" style="margin-top:6px"><button class="btn small" id="descgo" title="Asks the photo describer chosen in Settings (LM Studio on this computer unless you changed it, or the built-in model) to describe the photo and read any text in it">Describe photo <kbd>D</kbd></button>
      <label class="vtoggle"><input type="checkbox" id="descauto" ${store.get("describe") === "1" ? "checked" : ""}> Every photo</label></div>
      <div class="acts" style="margin-top:6px"><button class="btn small" id="descall" title="Describes every photo not described yet, in the background (progress at the top; Stop there pauses it, and it carries on from where it stopped)">Describe all photos</button> <small id="desccount"></small></div>
      <div class="acts" style="margin-top:6px;display:none" id="descget"><button class="btn small" id="descgetgo" title="Downloads Florence-2, a smaller vision model that runs inside this app when LM Studio isn't available (from Hugging Face; your photos aren't sent anywhere). Plainer descriptions than LM Studio's.">Download the built-in model</button> <small id="descgetmsg"></small></div></div>` : ""}
    ${nm ? "" : clues(info?.hints)}
    ${info?.exists ? `<label class="vtoggle" title="Dashed boxes: things the detector thinks may be faces (blurry, turned or partly hidden) that weren't kept when the photo was added. Nothing is saved until you click one."><input type="checkbox" id="possible" ${store.get("possible") !== "0" ? "checked" : ""}> Show possible faces <span id="possn"></span></label>` : ""}
    <div class="acts">${info?.exists && info?.video == null ? `<button class="btn" id="drawface" title="Drag a box around a face the detector missed (hidden, turned away, tiny), then name it">Draw a face <kbd>B</kbd></button>` : ""}
      ${info?.exists ? `<button class="btn" id="morefaces" style="display:none" title="Adds every dashed box to this ${info?.video != null ? "frame" : "photo"}, ready to name (Ignore any that aren't faces)">Add all possible faces</button>` : ""}
      <button class="btn" id="wsopen" title="Sends this face to Amazon and this photo to Google, and suggests who it is">Web search</button></div>
    <div class="keys"><kbd>←</kbd> <kbd>→</kbd> previous / next face<br><kbd>Esc</kbd> close</div>`;
  $("#vside").querySelectorAll("[data-v]").forEach(b => b.onclick = () => viewerKey(b.dataset.v));
  $("#vside").querySelectorAll("[data-nm]").forEach(b => b.onclick = async () => { await act("name", [i], b.dataset.nm); openPhoto(i); });
  $("#vside").querySelectorAll("[data-see]").forEach(b => b.onclick = () => openPhoto(+b.dataset.see));
  $("#wsopen").onclick = () => { $("#websearch").classList.add("on"); webSearch(false); };
  if ($("#drawface")) $("#drawface").onclick = () => drawMode(true);
  if ($("#desc")) {
    $("#descgo").onclick = () => describePhoto(i, true);
    $("#descall").onclick = async () => {
      try { await post("/api/describe/all", {}); toast("Describing every photo in the background: progress at the top"); pollImport(); }
      catch (err) { toast(err.message, true); }
    };
    describeStatus(i);
    $("#descgetgo").onclick = async () => {
      try { await post("/api/describe/getmodel", {}); describeStatus(i); } catch (err) { toast(err.message, true); }
    };
    $("#descauto").onchange = () => { store.set("describe", $("#descauto").checked ? "1" : "0"); if ($("#descauto").checked) describePhoto(i, false); };
    get(`/api/describe/${i}`).then(r => {
      if (vIndex !== i) return;
      if (r.saved) showDescription(r.saved);
      else if (store.get("describe") === "1") describePhoto(i, false);
    }).catch(() => {});
  }
  const more = $("#morefaces");
  if (more) more.onclick = () => addPossible(i, null);
  const pos = $("#possible");
  if (pos) pos.onchange = () => {
    store.set("possible", pos.checked ? "1" : "0");
    if (pos.checked) { const img = $("#vimg"); if (img?.naturalWidth) showPossible(i, img.clientWidth / img.naturalWidth); }
    else { document.querySelectorAll("#vwrap .box.cand").forEach(b => b.remove()); $("#possn").textContent = ""; $("#morefaces").style.display = "none"; }
  };
  const k = order.indexOf(i);
  $("#vprev").style.visibility = k > 0 ? "visible" : "hidden";
  $("#vnext").style.visibility = k >= 0 && k < order.length - 1 ? "visible" : "hidden";
}
// Possible faces: the detector run again on this photo with a much lower cut-off (labels.py
// Importer.possible_faces), shown as dashed boxes. Clicking one adds it as a face, ready to name.
async function showPossible(i, s) {
  $("#possn").textContent = "(looking…)";
  const r = await get(`/api/possible/${i}`).catch(() => null);
  if (vIndex !== i || !$("#possible")?.checked) return;
  document.querySelectorAll("#vwrap .box.cand").forEach(b => b.remove());
  const faces = r?.faces || [];
  $("#possn").textContent = r ? `(${faces.length || "none"})` : "(couldn't look)";
  $("#morefaces").style.display = faces.length > 1 ? "" : "none";
  for (const f of faces) {
    const b = document.createElement("div");
    b.className = "box maybe cand";          // cand: not saved yet (clicking adds it)
    Object.assign(b.style, { left: f.box[0] * s + "px", top: f.box[1] * s + "px", width: f.box[2] * s + "px", height: f.box[3] * s + "px" });
    b.innerHTML = `<span>Face? ${Math.round(f.score * 100)}%</span>`;
    b.title = `Possible face (the detector is ${Math.round(f.score * 100)}% sure; photos keep faces from 85%). Click to add it and name it.`;
    b.onclick = () => addPossible(i, f.box);
    $("#vwrap").appendChild(b);
  }
}
// Draw a face: drag a box on the photo; on release it's added as a face (labels.py Importer.add_drawn)
// and opens, ready to name. B starts it, Esc cancels.
function drawMode(on) {
  const wrap = $("#vwrap"), img = $("#vimg"), btn = $("#drawface");
  if (!img || !btn) return;
  wrap.classList.toggle("drawing", on);
  btn.classList.toggle("primary", on);
  btn.innerHTML = on ? "Drag around the face… <kbd>Esc</kbd>" : "Draw a face <kbd>B</kbd>";
  wrap.onpointerdown = !on ? null : e => {
    e.preventDefault();
    const r = img.getBoundingClientRect(), s = img.clientWidth / img.naturalWidth;
    const x0 = Math.min(Math.max(e.clientX - r.left, 0), r.width), y0 = Math.min(Math.max(e.clientY - r.top, 0), r.height);
    const b = document.createElement("div");
    b.className = "box drawn";
    wrap.appendChild(b);
    let box = null;
    const move = ev => {
      const x1 = Math.min(Math.max(ev.clientX - r.left, 0), r.width), y1 = Math.min(Math.max(ev.clientY - r.top, 0), r.height);
      box = [Math.min(x0, x1), Math.min(y0, y1), Math.abs(x1 - x0), Math.abs(y1 - y0)];
      Object.assign(b.style, { left: box[0] + "px", top: box[1] + "px", width: box[2] + "px", height: box[3] + "px" });
    };
    const up = async () => {
      removeEventListener("pointermove", move); removeEventListener("pointerup", up);
      drewAt = Date.now();
      drawMode(false);
      if (!box || box[2] < 8 || box[3] < 8) { b.remove(); return; }
      const i = vIndex;
      try {
        const res = await post("/api/drawface", { i, box: box.map(v => Math.round(v / s * 10) / 10) });
        await loadSummary();
        toast("Face added: name it now (N), or later under Possible faces");
        if (vIndex === i) openPhoto(res.i);
      } catch (err) { b.remove(); toast(err.message, true); }
    };
    addEventListener("pointermove", move); addEventListener("pointerup", up);
  };
}
// "Describe photo": a vision model (LM Studio on this computer by default; Settings can change it) describes the photo (describe.py); saved
// per photo outside the project, so each photo is described once. D asks (again), "Every photo" asks on opening.
function showDescription(d) {
  const t = $("#desctext");
  if (!t) return;
  t.innerHTML = `${esc(d.text)}<br><small>${esc(d.model || "")}${d.when ? ", " + esc(localTime(d.when)) : ""}${d.secs ? `, ${d.secs} s` : ""}</small>`;
  $("#descgo").innerHTML = "Describe again <kbd>D</kbd>";
}
// what describes photos (LM Studio, or the built-in model), how many are done, and the built-in model's download
async function describeStatus(i) {
  let r;
  try { r = await get("/api/describe/status"); } catch { return; }
  if (vIndex !== i || !$("#desccount")) return;
  const away = r.ok && r.model !== r.builtin?.name && r.config && r.config.where !== "this computer";
  $("#descwhere").textContent = away ? `(by a model on ${r.config.where === "your network" ? "another device on your network" : "a server on the internet"}, ${new URL(r.config.url).host})`
                                     : "(by a model on this computer)";
  const left = r.photos - r.described, b = r.builtin || {};
  $("#desccount").textContent = r.ok ? `${r.described.toLocaleString()} of ${r.photos.toLocaleString()} described` : r.why;
  if (left <= 0 || !r.ok) $("#descall").style.display = "none";
  else $("#descall").textContent = `Describe all photos (${left.toLocaleString()} to go, about ${Math.max(1, Math.round(left * 3.5 / 3600))} h)`;
  $("#descget").style.display = !r.ok || b.running ? "" : "none";
  $("#descgetgo").disabled = !!b.running;
  $("#descgetgo").textContent = `Download the built-in model (${(b.size_mb / 1000).toFixed(1)} GB)`;
  $("#descgetmsg").textContent = b.running ? `${Math.floor(b.done * 100 / (b.size_mb * 1e6))}% downloaded` : b.error || "";
  if (b.running) setTimeout(() => describeStatus(i), 2000);
}
async function describePhoto(i, refresh) {
  const t = $("#desctext"), go = $("#descgo");
  if (!t || go.disabled) return;
  go.disabled = true;
  t.innerHTML = `<small>Describing… (the first time, the model may take up to 40 seconds to load)</small>`;
  try {
    const d = await post("/api/describe", { i, refresh: !!refresh });
    if (vIndex === i) { showDescription(d); describeStatus(i); }
  } catch (err) { if (vIndex === i) t.innerHTML = `<small>${esc(err.message)}</small>`; }
  finally { go.disabled = false; }
}
async function addPossible(i, box) {
  try {
    const r = await post("/api/morefaces", box ? { i, box } : { i });
    if (!r.new.length) { toast("Nothing new to add"); return; }
    await loadSummary();                            // the new faces get indexes (FIDS) and show up in New faces
    toast(box ? "Face added: name it, or Ignore it if it isn't a face" : `Added ${plural(r.new.length, "face")}: click a box to name it`);
    if (vIndex === i) openPhoto(r.new[0]);
  } catch (err) { toast(err.message, true); }
}
// clues to who an unnamed face is; a chip names the face (Z undoes), ⤢ opens that person's face from the occasion
function clues(h) {
  if (!h) return "";
  const chip = (n, extra) => `<button class="chip" data-nm="${esc(n)}" title="Name this face ${esc(n)}">${avatar(n)}${esc(n)}${extra || ""}</button>`;
  let out = "";
  const live = h.live;                     // webcam snapshot: what the live tracker said when it saved it
  if (live) out += `<div class="clue"><h4>Live tracker (${esc(localTime(live.when))})</h4>${live.name !== "Unknown"
    ? `<div class="chips">${chip(live.name, ` <small>${Math.round(live.sim * 100)}%</small>`)}</div>`
    : `<div class="meta">Didn't recognize this face${live.sim > 0 ? ` (best match ${Math.round(live.sim * 100)}%)` : ""}.</div>`}</div>`;
  if (h.tag) out += `<div class="clue"><h4>The photo's own tags</h4><div class="chips">${chip(h.tag)}</div></div>`;
  else if (h.tags.length) out += `<div class="clue"><h4>The photo is tagged with</h4><div class="chips">${h.tags.map(n => chip(n)).join("")}</div></div>`;
  const web = h.web;
  if (web) out += `<div class="clue"><h4>Web search (${esc(localTime(web.when))})</h4>${web.suggestions.length
    ? `<div class="chips">${web.suggestions.map(s => chip(s.name, ` <small>${esc(s.level)}</small>`)).join("")}</div>`
    : `<div class="meta">No suggestions found.</div>`}</div>`;
  if (h.closest.length) out += `<div class="clue"><h4>Closest people (weak guesses)</h4><div class="chips">${h.closest.map(c => chip(c.name, ` <small>${c.s.toFixed(2)}</small>`)).join("")}</div></div>`;
  const o = h.occasion;
  if (o.people.length) out += `<div class="clue"><h4>Same occasion</h4><div class="meta">Named people in ${plural(o.photos, "photo")} ${esc(o.how)}:</div>
    <div class="chips">${o.people.slice(0, 8).map(p => `<span>${chip(p.name, ` <small>×${p.n}</small>`)}<button class="see" data-see="${p.i}" title="See ${esc(p.name)} in that photo">⤢</button></span>`).join("")}</div></div>`;
  else if (o.photos) out += `<div class="clue"><h4>Same occasion</h4><div class="meta">Nobody is named yet in the ${plural(o.photos, "photo")} ${esc(o.how)}.</div></div>`;
  return out;
}

// Web search: the server sends the face to Amazon and the photo to Google (websearch.py) and combines what
// they say into suggestions with a confidence level. Runs when you click Web search.
function wsPicture(what) {
  return new Promise((res, rej) => {
    const c = document.createElement("canvas"), img = $("#vimg"), me = vInfo?.faces.find(f => f.i === vIndex);
    const draw = (src, x, y, w, h, k) => { c.width = Math.round(w * k); c.height = Math.round(h * k);
      c.getContext("2d").drawImage(src, x, y, w, h, 0, 0, c.width, c.height); c.toBlob(b => b ? res(b) : rej(new Error("no picture")), "image/jpeg", 0.92); };
    if (!img || !img.naturalWidth) {                  // original photo gone: the stored face is all there is
      const t = new Image(); t.onload = () => draw(t, 0, 0, t.naturalWidth, t.naturalHeight, 1); t.onerror = () => rej(new Error("no picture")); t.src = thumb(vIndex); return;
    }
    if (what === "photo" || !me) return draw(img, 0, 0, img.naturalWidth, img.naturalHeight, 1);
    const [bx, by, bw, bh] = me.box, m = 0.5;          // the face with some hair, chin and background around it
    const x = Math.max(0, bx - bw * m), y = Math.max(0, by - bh * m);
    const w = Math.min(img.naturalWidth, bx + bw * (1 + m)) - x, h = Math.min(img.naturalHeight, by + bh * (1 + m)) - y;
    draw(img, x, y, w, h, Math.max(1, 400 / Math.max(w, h)));
  });
}
// the clipboard takes PNG only
const toPng = jpg => new Promise((res, rej) => {
  const im = new Image();
  im.onload = () => { const c = document.createElement("canvas"); c.width = im.naturalWidth; c.height = im.naturalHeight;
    c.getContext("2d").drawImage(im, 0, 0); URL.revokeObjectURL(im.src); c.toBlob(b => b ? res(b) : rej(new Error("no picture")), "image/png"); };
  im.onerror = rej; im.src = URL.createObjectURL(jpg);
});
const LEVEL_TEXT = { "very high": "Very high confidence", high: "High confidence", medium: "Medium confidence", low: "Low confidence" };
async function webSearch(refresh) {
  const i = vIndex, res = $("#wsres"), go = $("#wsgo");
  $("#wserr").textContent = ""; go.disabled = true;
  res.innerHTML = `<p class="meta" style="margin:0">Searching…</p>`;
  showServices();
  try {
    const r = await post("/api/websearch", { face: i, refresh });
    if (vIndex !== i) return;
    const s = r.suggestions;
    res.innerHTML = (s.length
      ? `<div><b>${s.length === 1 ? "Suggestion" : "Suggestions"}</b></div>` + s.map(x => `<div class="wssug">
          <div><b>${esc(x.name)}</b> <span class="lvl lvl-${x.level.replace(" ", "-")}">${LEVEL_TEXT[x.level]}</span></div>
          <div class="meta">${x.why.map(esc).join(". ")}.</div>
          <div class="acts"><button class="btn small primary" data-tag="${esc(x.name)}">Name this face ${esc(x.name)}</button>
            ${x.links.map(u => `<a href="${esc(u)}" target="_blank" rel="noopener noreferrer">${esc(new URL(u).hostname.replace(/^www\./, ""))} ↗</a>`).join(" ")}</div></div>`).join("")
      : `<div><b>No suggestions found.</b></div><div class="meta">${r.checked.length ? "Checked: " + r.checked.map(esc).join("; ") + "." : ""}
          Amazon only knows well-known people, and Google only helps when this same picture is posted online.</div>`) +
      (r.cached ? `<div><small>Saved answer from your search on ${esc(localTime(r.when))}: nothing was sent again. <b>Search again</b> asks anew.</small></div>` : "") +
      (r.cached ? "" : `<div><small>This month: ${Object.entries(r.usage).map(([k, u]) => `${k === "vision" ? "Google" : "Amazon"} ${u.used.toLocaleString()} of ${u.cap.toLocaleString()}`).join(", ")} searches (each stops before $10).</small></div>`);
    if (r.errors.length) $("#wserr").textContent = r.errors.join("; ");
    res.querySelectorAll("[data-tag]").forEach(b => b.onclick = async () => {
      const n = await pickName("Name this face", b.dataset.tag);
      if (n) { $("#websearch").classList.remove("on"); await act(n === IGN ? "ignore" : "name", [i], n); openPhoto(i); }
    });
  } catch (e) { res.innerHTML = `<div><b>Web search didn't work.</b></div>`; $("#wserr").textContent = e.message; }
  finally { go.disabled = false; }
}
$("#wsgo").onclick = () => webSearch(true);
async function showServices() {
  const r = await get("/api/websearch/key").catch(() => null);
  if (r) $("#wskeys").textContent = `Amazon: ${r.services.rekognition ? "key saved" : "no key"}. Google: ${r.services.vision ? "key saved" : "no key"}.`;
}
// Google Lens only accepts pictures from its own page: copy the face (while this page has focus), then open it
$("#wslens").onclick = async () => {
  const png = wsPicture("face").then(toPng);
  const copied = navigator.clipboard && window.ClipboardItem
    ? navigator.clipboard.write([new ClipboardItem({ "image/png": png })]) : Promise.reject(new Error("no clipboard"));
  const ok = await copied.then(() => true, () => false);
  window.open("https://images.google.com/?hl=en&gl=us", "_blank", "noopener");
  $("#wserr").textContent = ok ? "" : "Couldn't copy the face: use Save face picture and drag the file onto the camera icon.";
};
$("#wsawssave").onclick = async () => {
  try {
    await post("/api/websearch/aws", { id: $("#wsawsid").value, secret: $("#wsawssecret").value, region: $("#wsawsregion").value || "us-east-1" });
    $("#wsawsid").value = $("#wsawssecret").value = ""; showServices();
  } catch (e) { $("#wserr").textContent = e.message; }
};
$("#wskeysave").onclick = async () => {
  try { await post("/api/websearch/key", { key: $("#wskeyin").value }); $("#wskeyin").value = ""; showServices(); }
  catch (e) { $("#wserr").textContent = e.message; }
};
$("#wssave").onclick = async () => {
  try {
    const a = document.createElement("a");
    a.href = URL.createObjectURL(await wsPicture("face"));
    a.download = `face-${vIndex}.jpg`; a.click();
    setTimeout(() => URL.revokeObjectURL(a.href), 5000);
  } catch (e) { $("#wserr").textContent = "Couldn't make the picture: " + e.message; }
};
$("#wscancel").onclick = () => $("#websearch").classList.remove("on");
$("#websearch").onclick = e => { if (e.target.id === "websearch") $("#websearch").classList.remove("on"); };

function closePhoto() { $("#viewer").classList.remove("on"); vIndex = null; }
function stepPhoto(d) { const k = order.indexOf(vIndex); if (k >= 0 && order[k + d] != null) openPhoto(order[k + d]); }
async function viewerKey(k) {
  const i = vIndex, info = await get(`/api/photoinfo/${i}`), nm = info.faces.find(f => f.i === i)?.name;
  if (k === "n") { const n = await pickName("Name this face"); if (n) { await act(n === IGN ? "ignore" : "name", [i], n); openPhoto(i); } }
  if (k === "x") { await act("ignore", [i]); openPhoto(i); }
  if (k === "r" && nm && nm !== IGN) { await act("reject", [i], nm); openPhoto(i); }
}
$("#vprev").onclick = () => stepPhoto(-1);
$("#vnext").onclick = () => stepPhoto(1);
let drewAt = 0;                                  // a drag that ends off the photo mustn't close it
$("#viewer").onclick = e => { if (e.target.classList.contains("stage") && Date.now() - drewAt > 400) closePhoto(); };

// ------------------------------------------------------------------ import
$("#add").onclick = () => { $("#imperr").textContent = ""; $("#importer").classList.add("on"); $("#impfolder").focus(); };
$("#impcancel").onclick = () => $("#importer").classList.remove("on");
$("#impform").onsubmit = async e => {
  e.preventDefault();
  try { await post("/api/import", { folder: $("#impfolder").value }); $("#importer").classList.remove("on"); pollImport(); }
  catch (err) { $("#imperr").textContent = err.message; }
};
$("#stopimp").onclick = () => post("/api/import/stop").catch(() => {});
let polling = false;
async function pollImport() {
  if (polling) return; polling = true;
  let last = 0;
  try {
    while (true) {
      const st = await get("/api/import");
      showImport(st);
      if (!st.running) break;
      if (Date.now() - last > 6000) { last = Date.now(); loadSummary().catch(() => {}); }
      await new Promise(r => setTimeout(r, 1000));
    }
  } finally { polling = false; }
  await loadSummary(); await render();
}
function showImport(st) {
  const p = $("#progress");
  // A finished webcam import stays up for a minute, and only if it added something or failed: the
  // snapshots are re-checked every 20 seconds and mostly find nothing new. A folder you added stays up
  // for 5 minutes even when nothing was new: "all already added" is the answer you were waiting for.
  const ended = st && !st.running && st.started ? st.started + (st.elapsed || 0) : null;
  const cam = /[\\/]webcam$/i.test(st?.folder || "");
  const quiet = ended != null && !st.error && (cam ? !st.new || Date.now() / 1000 - ended > 60 : Date.now() / 1000 - ended > 300);
  if (!st || (!st.running && !st.stage) || quiet) { p.classList.remove("on"); return; }
  if (st.running && !polling) pollImport();
  p.classList.add("on");
  const pct = st.total ? st.done / st.total * 100 : 0;
  p.querySelector("i").style.width = pct + "%";
  const el = (Date.now() / 1000 - st.started);
  const rate = st.done - st.known > 0 ? (st.done - st.known) / el : 0;
  const left = rate > 0 ? Math.round((st.total - st.done) / rate) : null;
  const descr = st.folder === "photo descriptions";
  const parts = descr ? [`${st.done.toLocaleString()} of ${st.total.toLocaleString()} photos`, `${st.new.toLocaleString()} described`]
    : [`${st.done.toLocaleString()} of ${st.total.toLocaleString()} ${st.videos ? "photos and videos" : "photos"}`, plural(st.faces, "face") + " found",
       `${st.auto.toLocaleString()} named automatically`];
  if (st.known) parts.push(descr ? `${st.known.toLocaleString()} not on this computer now` : st.folder === "possible faces" ? `${st.known.toLocaleString()} not on this computer now` : `${st.known.toLocaleString()} already added`);
  if (st.duplicates) parts.push(plural(st.duplicates, "duplicate") + " skipped");
  if (st.errors) parts.push(`${st.errors} unreadable`);
  if (descr && st.by && Object.keys(st.by).length > 1)      // Describe all shared with other computers: who did how many
    parts.push(Object.entries(st.by).map(([h, n]) => `${h} ${n.toLocaleString()}`).join(", "));
  if (descr && st.left_out?.length) parts.push(`left out: ${st.left_out.join("; ")}`);
  // the folder's own name (the whole path is in the tooltip): the counts are what matter at a glance
  const where = cam ? "webcam snapshots" : (st.folder || "").split(/[\\/]/).filter(Boolean).pop() || "";
  parts.unshift(where);
  if (descr) parts.shift();                         // the stage already says what it is
  let txt = st.running && !st.total ? `${st.stage}: ${where}`
    : st.running ? `${st.stage}: ${parts.join(", ")}${left != null && /^(Reading|Describing|Looking)/.test(st.stage) ? `, about ${left < 90 ? left + " s" : left < 5400 ? Math.round(left / 60) + " min" : (left / 3600).toFixed(1) + " h"} left` : ""}`
    : st.error ? `Adding photos failed: ${st.error}` : `${st.stage === "Stopped" ? "Stopped" : "Finished"}: ${parts.join(", ")}.`;
  p.querySelector(".txt").textContent = txt;
  p.querySelector(".txt").title = st.folder && !cam ? `${txt}\n${st.folder}` : txt;
  $("#stopimp").style.display = st.running ? "" : "none";
  lastProblems = st.problems || [];
  $("#impwhy").style.display = st.errors ? "" : "none";
  $("#probsnote").textContent = st.errors > lastProblems.length
    ? `${st.errors.toLocaleString()} in all; the last ${lastProblems.length} are listed.` : "";
  if ($("#probs").classList.contains("on")) showProblems();
}
// Which files an import (or Describe all) couldn't read, and why: built with textContent only
let lastProblems = [];
function showProblems() {
  $("#probslist").replaceChildren(...lastProblems.slice().reverse().map(p => {
    const li = document.createElement("li"), why = document.createElement("small");
    li.textContent = p.file; why.textContent = p.why; li.append(why);
    return li;
  }));
}
$("#impwhy").onclick = () => { showProblems(); $("#probs").classList.add("on"); };
$("#probsclose").onclick = () => $("#probs").classList.remove("on");
$("#probs").onclick = e => { if (e.target.id === "probs") $("#probs").classList.remove("on"); };
// Downloads (models for LM Studio, CUDA support): their own bar beside the import's, every 2 s while one runs
function amountText(d) {
  const gb = n => (n / 1e9).toFixed(1), secs = Date.now() / 1000 - (d.started || Date.now() / 1000);
  if (!(d.bytes && d.total)) return d.pct != null ? `${d.pct}%` : "starting";
  const rate = d.bytes / Math.max(secs, 1), left = (d.total - d.bytes) / Math.max(rate, 1);
  return `${gb(d.bytes)} of ${gb(d.total)} GB (${Math.floor(d.bytes * 100 / d.total)}%), ${(rate / 1e6).toFixed(0)} MB/s, ` +
    `about ${left > 5400 ? (left / 3600).toFixed(1) + " h" : left > 90 ? Math.round(left / 60) + " min" : Math.max(1, Math.round(left)) + " s"} left`;
}
let dlTimer = null;
async function pollDownloads() {
  clearTimeout(dlTimer);
  let items = [];
  try { items = (await get("/api/downloads")).items || []; } catch { /* the server is busy or restarting */ }
  const p = $("#dlprogress");
  p.classList.toggle("on", items.length > 0);
  if (items.length) {
    const d = items[0];
    const pct = d.bytes && d.total ? d.bytes * 100 / d.total : d.pct || 0;
    p.querySelector("i").style.width = pct + "%";
    const how = d.how ? d.how[0].toUpperCase() + d.how.slice(1) : "Downloading";
    const txt = `${d.fallback ? `${d.fallback[0].toUpperCase() + d.fallback.slice(1)}: ` : ""}${how}: ${d.what}: ${amountText(d)}` +
                (items.length > 1 ? ` (and ${items.length - 1} more)` : "");
    p.querySelector(".txt").textContent = txt;
    p.title = txt;
  }
  dlTimer = setTimeout(pollDownloads, items.length ? 2000 : document.hidden ? 15000 : 6000);
}
pollDownloads();
// An import started anywhere else (the webcam check, another window) shows up here too.
setInterval(() => { if (!polling && !document.hidden) get("/api/import").then(showImport).catch(() => {}); }, 4000);

// ------------------------------------------------------------------ toast
let toastT = null;
function toast(msg, err, withUndo) {
  const t = $("#toast");
  t.querySelector(".msg").textContent = msg;
  t.classList.toggle("err", !!err);
  t.querySelector(".undo").style.display = withUndo ? "" : "none";
  t.classList.add("on");
  clearTimeout(toastT); toastT = setTimeout(() => t.classList.remove("on"), err ? 7000 : 5000);
}
$("#toast .undo").onclick = () => { $("#toast").classList.remove("on"); undo(); };

// ------------------------------------------------------------------ keys + start
addEventListener("keydown", e => {
  if (e.target.matches("input, textarea, select") || e.ctrlKey && e.key !== "z" || e.altKey || e.metaKey) return;
  const k = e.key.toLowerCase();
  if ($("#websearch").classList.contains("on")) { if (k === "escape") $("#websearch").classList.remove("on"); return; }
  if ($("#viewer").classList.contains("on")) {
    if (k === "escape" && $("#vwrap").classList.contains("drawing")) drawMode(false);
    else if (k === "escape") closePhoto();
    else if (k === "b") drawMode(!$("#vwrap").classList.contains("drawing"));
    else if (k === "d" && $("#descgo")) describePhoto(vIndex, true);
    else if (k === "arrowleft") stepPhoto(-1);
    else if (k === "arrowright") stepPhoto(1);
    else if (["n", "x", "r"].includes(k)) viewerKey(k);
    else return;
    return e.preventDefault();
  }
  if (document.querySelector(".modal.on")) { if (k === "escape") document.querySelectorAll(".modal.on").forEach(m => m.classList.remove("on")); return; }
  if (e.key === "?") { e.preventDefault(); return $("#keys").classList.add("on"); }
  if (k === "z") { e.preventDefault(); return undo(); }
  if (k === "escape") return clearSel();
  if (k === " " && sel.size === 1) { e.preventDefault(); return openPhoto([...sel][0]); }
  if (k === " ") { const h = document.querySelector(".face:hover"); if (h) { e.preventDefault(); return openPhoto(+h.dataset.i); } }
  if (["a", "n", "x", "r"].includes(k)) { e.preventDefault(); key(k); }
});
document.querySelectorAll("nav [data-v]").forEach(b => b.onclick = () => go({ v: b.dataset.v }));
$("#undo").onclick = undo;
$("#low").checked = showLow = store.get("low") === "1";       // remembered
$("#low").onchange = e => { showLow = e.target.checked; store.set("low", showLow ? "1" : "0"); render(); };
$("#navbtn").onclick = () => setNav($("#shell").classList.contains("closed"));
addEventListener("keydown", e => { if (e.ctrlKey && e.key === ".") { e.preventDefault(); $("#navbtn").click(); } });
setNav(narrowNav.matches ? false : store.get("nav") !== "0", false);
// Resizing across the narrow width: close it (it would cover the faces), or restore the remembered state
narrowNav.addEventListener("change", e => setNav(e.matches ? false : store.get("nav") !== "0", false));
new ResizeObserver(() => document.documentElement.style.setProperty("--hdr", $("header").offsetHeight + "px")).observe($("header"));
$("#pfind").oninput = findPeople;
$("#pfind").onkeydown = e => {                     // Enter opens the first person found; Esc clears the search
  if (e.key === "Enter") { const b = [...document.querySelectorAll("#people [data-person]")].find(x => !x.hidden); if (b) b.click(); }
  if (e.key === "Escape") { $("#pfind").value = ""; findPeople(); }
};
$("#keysbtn").onclick = () => $("#keys").classList.add("on");
$("#keysclose").onclick = () => $("#keys").classList.remove("on");
$("#keys").onclick = e => { if (e.target.id === "keys") $("#keys").classList.remove("on"); };   // click outside closes
$("#psort").value = store.get("psort") || "name";
$("#psort").onchange = () => { store.set("psort", $("#psort").value); loadSummary(); };
$("#tilesz").value = store.get("tile") || 88; setTile($("#tilesz").value);
$("#tilesz").oninput = e => { setTile(e.target.value); store.set("tile", e.target.value); };

// The app window's Settings tab (settings_bridge.js): this page's settings, Web search, Describe photo, LM Studio and
// this computer's hardware, each group in its own section, in that order
{
  const c = SettingsBridge.control, L = "Labeling", W = "Labeling: Web search", P = "Labeling: Describe photo",
        M = "LM Studio", H = "This computer", D = "Shared database";
  const flag = (section, id, key, label, def, hint) => ({ section, id, label, type: "toggle", def, hint,
    get: () => (store.get(key) ?? (def ? "1" : "0")) === "1",
    set: v => { store.set(key, v ? "1" : "0"); const el = $("#" + id); if (el) el.checked = !!v; } });
  let desc = {}, hw = {}, lms = {}, lmsOffer = false, sdb = {};
  SettingsBridge.init("label", [
    c("tilesz", L, { label: "Face picture size" }),
    c("psort", L, { label: "Order of the people list" }),
    c("low", L, { label: "Show blurry faces", hint: "Faces too blurry or small to learn from are hidden from the review lists unless this is on." }),
    flag(L, "possible", "possible", "Show possible faces in photos", true,
         "Dashed boxes on things the detector thinks may be faces (blurry, turned or partly hidden). Nothing is saved until you click one."),
    { section: L, id: "folders", label: "Photo folders added", type: "info",
      get: () => (SUM?.settings.folders || []).join("\n") || "None yet: use Add photos." },
    { section: W, id: "wskeys", label: "Keys", type: "info",
      hint: "Web search sends a face to Amazon (celebrity recognition) and its photo to Google (pages with the same picture), only when you click Web search. The keys are your own (how to make them: the Web search section of the README) and stay in %APPDATA%\\FaceLabels on this computer. Save an empty key to remove it.",
      get: async () => { const r = await get("/api/websearch/key");
        return `Amazon: ${r.services.rekognition ? "key saved" : "no key"}. Google: ${r.services.vision ? "key saved" : "no key"}.`; } },
    { section: W, id: "wsgoogle", label: "Google Cloud Vision API key", type: "form", button: "Save key",
      fields: [{ name: "key", placeholder: "API key", secret: true }] },
    { section: W, id: "wsaws", label: "Amazon Rekognition key", type: "form", button: "Save",
      fields: [{ name: "id", placeholder: "Access key ID" }, { name: "secret", placeholder: "Secret access key", secret: true },
               { name: "region", placeholder: "us-east-1", size: 9 }] },
    { section: P, id: "describer", label: "Describing with", type: "info",
      get: async () => {
        desc = await get("/api/describe/status");
        const b = desc.builtin || {};
        if (b.running) setTimeout(SettingsBridge.changed, 2000);
        return [desc.ok ? (desc.model === b.name ? desc.model : `${desc.model}, on ${desc.config.url}`) : desc.why,
                `${desc.described.toLocaleString()} of ${desc.photos.toLocaleString()} photos described.`,
                b.running ? `Built-in model: ${Math.floor(b.done * 100 / (b.size_mb * 1e6))}% downloaded.`
                  : b.ready ? "Built-in model: downloaded." : b.error ? `Built-in model: ${b.error}` : "Built-in model: not downloaded."].join("\n");
      } },
    flag(P, "descauto", "describe", "Describe every photo as it's opened", false, "Runs Describe photo on each photo you open in the photo viewer, with the describer set here."),
    { section: P, id: "describeUse", label: "Describer", type: "select", def: "auto",
      options: () => [["auto", "Automatic: the server, or the built-in model when it doesn't answer"],
                      ["server", "The server only"], ["builtin", "The built-in model only"]],
      get: () => desc.config?.use || "auto",
      set: v => post("/api/describe/config", { use: v }) },
    { section: P, id: "describeUrl", label: "Server address", type: "text", hidden: () => desc.config?.use === "builtin",
      hint: () => {
        const c = desc.config || {};
        const base = `LM Studio (its local server on), or another server with the same API: Ollama (port 11434), llama.cpp, vLLM, Jan. Empty for the default, ${c.default_url || "http://127.0.0.1:1234"}.`;
        if (c.where === "your network") return base + " This is another device on your network: each photo you describe is sent to it (in LM Studio there, turn on \"Serve on local network\").";
        if (c.where === "the internet") return base + ` This is a server on the internet: each photo you describe is sent to it${c.unencrypted ? ", unencrypted (http): anyone on the way could see the photos; use https" : " (encrypted, https)"}. Only use a server you control.`;
        return base;
      },
      get: () => desc.config?.url || "",
      set: v => post("/api/describe/config", { url: v }) },
    { section: P, id: "describeModel", label: "Model", type: "select", def: "", hidden: () => desc.config?.use === "builtin",
      hint: "Automatic: LM Studio's loaded vision model, else its first one. Other servers don't say which models read pictures: pick a vision model.",
      options: () => { const m = desc.models || [], pick = desc.config?.model;
        return [["", "Automatic"], ...m.map(x => [x, x]), ...(pick && !m.includes(pick) ? [[pick, pick + " (not on the server)"]] : [])]; },
      get: () => desc.config?.model || "",
      set: v => post("/api/describe/config", { model: v }) },
    { section: P, id: "describeTest", label: "Check the server", type: "button", button: "Test", hidden: () => desc.config?.use === "builtin",
      hint: "Asks the server above which models it has." },
    { section: P, id: "describeKey", label: "API key", type: "form", button: "Save key", hidden: () => desc.config?.use === "builtin",
      hint: () => `Only if the server asks for one (for example behind a proxy). ${desc.config?.key ? "A key is saved." : "None saved."} Save an empty key to remove it. Kept in %APPDATA%\\FaceLabels on this computer.`,
      fields: [{ name: "key", placeholder: "API key", secret: true }] },
    { section: P, id: "describeSpread", label: "Share Describe all with other computers", type: "toggle", def: false,
      hidden: () => desc.config?.use === "builtin",
      hint: "Describe all also sends photos to the other LM Studio servers on your network that have the same vision model, one photo at a time each: every computer takes the next photo as it finishes, so a faster one does more. Each photo sent there is seen by that computer. A computer that fails twice is left out and its photo goes back to the others. Describing a single photo uses only the server above.",
      get: () => !!desc.config?.spread,
      set: v => post("/api/describe/config", { spread: !!v }) },
    { section: P, id: "describeHelpers", label: "Shared with", type: "info",
      hidden: () => !desc.config?.spread || desc.config?.use === "builtin",
      get: () => { const h = desc.helpers || [];
        return h.length ? h.map(x => `${x.host} (${x.url})`).join("\n") + `\nDescribe all uses ${h.length + 1} computers.`
          : desc.ok ? `No other LM Studio server on your network has ${desc.model} (see LM Studio below): Describe all uses this server alone.`
          : "Nothing to share with until the describer works."; } },
    { section: P, id: "descget", label: "Built-in model", type: "button",
      button: () => `Download the built-in model (${((desc.builtin?.size_mb || 831) / 1000).toFixed(1)} GB)`,
      hidden: () => !!(desc.builtin?.ready || desc.builtin?.running),
      hint: "Florence-2 large, from Hugging Face; runs inside this app on the processor, so photos stay on this computer. Plainer descriptions than LM Studio's." },
    { section: M, id: "lmshere", label: "LM Studio on this computer", type: "info",
      hint: "While this app runs, LM Studio here is kept serving on all network interfaces and announced on the network (mDNS: LM Studio on <this computer>, with its port), so the other computers find it. FACE_LMSTUDIO=off turns that off.",
      get: async () => { const s = lms = await get("/api/lmstudio");
        if (!s.installed) return "Not installed (lmstudio.ai).";
        const lines = [s.running ? `Server running on port ${s.port}, ${s.on_network ? "on all network interfaces" : "on this computer only"}` : "Server not running"];
        if (s.bind) lines.push(`Changed: ${s.bind}`);
        if (s.announced) lines.push(s.announced[0].toUpperCase() + s.announced.slice(1));
        if (s.missing.length) lines.push("Missing vision models the other LM Studio servers have:",
          ...s.missing.map(m => `  ${m.variant} (on ${m.from}): ${m.in_share ? "will be copied from the shared folder; from the internet if that fails" : "will be downloaded from the internet (not in the shared folder)"}`));
        else lines.push("Has every vision model the other LM Studio servers on the network have.");
        const sh = s.shared || {};
        if (sh.share) lines.push(sh.reachable ? `Shared models folder: ${sh.models.length} model${sh.models.length === 1 ? "" : "s"} there${sh.models.length ? ` (${sh.models.join(", ")})` : ""}${sh.note ? `; ${sh.note}` : ""}`
                                            : `Shared models folder ${sh.share} can't be reached`);
        if (sh.note) setTimeout(SettingsBridge.changed, 10000);
        const g = s.get;
        if (g?.running) {
          const pct = g.bytes && g.total ? g.bytes * 100 / g.total : g.pct || 0, n = Math.round(pct / 5);
          lines.push(`${g.fallback ? `${g.fallback[0].toUpperCase() + g.fallback.slice(1)}: ` : ""}${g.how ? g.how[0].toUpperCase() + g.how.slice(1) : "Getting"}: ${g.now}`,
                     `[${"#".repeat(n)}${"-".repeat(20 - n)}] ${amountText(g)}` + (g.done.length ? `; done: ${g.done.join(", ")}` : ""));
          setTimeout(SettingsBridge.changed, 3000);
        }
        else if (g?.error) lines.push(`Download stopped: ${g.error}`);
        if (g?.notes?.length) lines.push(...g.notes);
        else if (g?.done?.length) lines.push(`Got: ${g.done.join(", ")}`);
        if (!!s.missing.length !== lmsOffer) { lmsOffer = !!s.missing.length; setTimeout(SettingsBridge.changed, 0); }
        return lines.join("\n"); } },
    { section: M, id: "lmsget", label: "Get the same models", type: "button",
      button: () => `Get ${lms.missing?.length || 0} model${lms.missing?.length === 1 ? "" : "s"} (shared folder first, else the internet)`,
      hidden: () => !lmsOffer || !!lms.get?.running,
      hint: "Gets the vision models listed above into LM Studio's models folder, in this order: 1. copied from the Shared models folder when they're there (the very same files, each checked by SHA-256; fast on a local network); 2. if that copy fails in any way, or the model isn't there, downloaded from the internet with LM Studio (the same model and quantization; much slower). The bar at the top of the Labeling tab shows which, how far and how long. Large: tens of GB." },
    { section: M, id: "lmsshare", label: "Shared models folder", type: "text",
      hint: "A folder every computer can reach, for example on a NAS (\\\\SERVER\\share\\lmstudio-models on Windows, /mnt/nas/share/lmstudio-models on Linux). Each computer with this app and LM Studio puts its vision models there, and gets the ones it lacks from there before downloading them. Empty: not used.",
      hidden: () => !lms.installed,
      get: () => lms.shared?.share || "",
      set: v => post("/api/lmstudio/share", { folder: v }) },
    { section: M, id: "hwnet", label: "LM Studio on the network", type: "info",
      hint: "Every LM Studio server on this computer's local network: each computer's name, the addresses and port to reach it at, and the models it serves (which read pictures, which are loaded now). Found two ways, only while Settings is open: servers that announce themselves on the network (mDNS, _lmstudio._tcp) say their own address and port; the rest are found by trying LM Studio's port (1234, this computer's own LM Studio port, and any port announced) on each address of this computer's networks; Describe photo can use any of them (put its address in Server address).",
      get: async () => { const s = (await get("/api/hardware/network")).servers || [];
        if (!s.length) return "None found on this network (LM Studio's server must listen on all interfaces: Serve on local network).";
        return s.map(e => {
          const ms = e.models.filter(m => m.kind !== "embeddings").map(m => `${m.id}${m.vision ? " (reads pictures)" : ""}${m.loaded ? ", loaded" : ""}`);
          return `${e.host}${e.this_computer ? " (this computer)" : ""}: ${e.urls.join(", ")}${e.announced ? " (announces itself)" : ""}\n  ${ms.length} model${ms.length === 1 ? "" : "s"}: ${ms.join("; ") || "none"}`;
        }).join("\n"); } },
    { section: M, id: "hwai", label: "AI model apps found", type: "info",
      hint: "Apps that can run a vision model for Describe photo: the usual ports on this computer (LM Studio, Ollama, Jan, llama.cpp, vLLM and others), the server set under Labeling: Describe photo (which may be another device), and apps installed but not running. Nothing else on the network is asked.",
      get: async () => { const a = (await get("/api/hardware")).ai || [];
        if (!a.length) return "None found: LM Studio (lmstudio.ai) with a vision model, or the built-in model under Labeling: Describe photo.";
        return a.map(e => {
          const state = e.running ? `running, ${e.models} model${e.models === 1 ? "" : "s"}` : "";
          const sees = e.vision.length ? `reads pictures: ${e.vision.join(", ")}` : "";
          const lan = e.lan?.length ? `other devices can use it at ${e.lan.join(", ")}` : (e.running && e.url?.includes("127.0.0.1") ? "this computer only" : "");
          return `${e.app}${e.url ? ` (${e.url})` : ""}: ${[state, e.note, sees, lan].filter(Boolean).join("; ") || "not running"}`;
        }).join("\n"); } },
    { section: H, id: "hwhas", label: "Hardware", type: "info",
      hint: "What the app found on this computer when it started.",
      get: async () => { const h = (await get("/api/hardware")).has;
        return [`Processor: ${h.cpu}, ${h.threads} threads`, h.memory_gb ? `Memory: ${h.memory_gb} GB` : null,
                `Graphics: ${h.gpus.length ? h.gpus.join("; ") : "none found"}`, `NPU: ${h.npu || "none found"}`,
                `ONNX Runtime can use: ${h.onnxruntime.join(", ") || "not installed"}`, `${h.system}, Python ${h.python}`]
          .filter(Boolean).join("\n"); } },
    { section: H, id: "hwuses", label: "Where the work runs", type: "info",
      hint: "The app picks the fastest way for each job by itself: a graphics processor (DirectML on Windows, CUDA on NVIDIA under Linux, CoreML on a Mac) is used for a model only after a check at start shows it gives the processor's answers, faster. To choose yourself, start the app with FACE_DEVICE set to gpu, cpu or npu.",
      get: async () => { const u = (await get("/api/hardware")).uses, d = u.devices;
        const was = !!hw.cuda_offer?.offer; hw = u;
        if (!!u.cuda_offer?.offer !== was) setTimeout(SettingsBridge.changed, 0);   // show or hide the CUDA button
        const job = (name, k) => d[k] ? `${name}: ${d[k]}` : null;
        const lines = [job("Face finding", "detection"), job("Recognition (face prints)", "recognition"),
                       job("Photo quality", "quality"), job("Expression", "expression"),
                       job("Face shape cues", "cues"), job("Body posture", "pose")].filter(Boolean);
        if (!lines.length) lines.push("Decided when the models first load (adding photos, or the face tracker).");
        if (u.gpu_note) lines.push(u.gpu_note);
        if (u.cuda) { lines.push(u.cuda); if (/downloading/.test(u.cuda)) setTimeout(SettingsBridge.changed, 3000); }
        lines.push(`Adding photos: ${u.import_workers} at once; photos ${u.photos}`, `Videos: ${u.videos}`,
                   `NPU: ${u.npu_note}`);
        const why = Object.entries(u.decisions).map(([m, w]) => `  ${m}: ${w}`);
        return lines.concat(why.length ? ["Checks at start:", ...why] : []).join("\n"); } },
    { section: D, id: "dbstate", label: "Shared database", type: "info",
      get: async () => {
        sdb = await get("/api/db");
        if (!sdb.available) return sdb.why;
        const c = sdb.config, st = sdb.sync || {};
        if (st.state === "joining" || st.state === "online" && st.waiting) setTimeout(SettingsBridge.changed, 3000);
        return [c ? `${c.user} on ${c.host}:${c.port}, database ${c.dbname}, schema ${c.schema}` +
                    (c.ca ? " (the server is checked against its own certificate authority)" : "") : "Not set up.",
                st.text || "", st.waiting ? `${st.waiting} changes on this device wait to be sent.` : "",
                `This device: ${sdb.device?.name || ""}`].filter(Boolean).join("\n");
      } },
    { section: D, id: "dbsetup", label: "Connect", type: "form", button: "Connect",
      hint: "Keeps this device's faces, names, labels, Web search answers and photo descriptions in a PostgreSQL database (with pgvector) that your other devices share, and takes theirs. The data goes to that server over an encrypted connection; the password is kept on this device only (protected by Windows for your user). While the server can't be reached the app works from this device's copy and sends changes later. Leave the server empty and Connect to stop using it (this device keeps its copy).",
      fields: [{ name: "host", placeholder: "Server (e.g. sentient-nas)" }, { name: "port", placeholder: "5432", size: 5 },
               { name: "dbname", placeholder: "Database (sentient)", size: 10 }, { name: "user", placeholder: "User", size: 9 },
               { name: "password", placeholder: "Password", secret: true }] },
    { section: H, id: "cudaget", label: "NVIDIA CUDA", type: "button",
      button: () => `Download CUDA support (${(hw.cuda_offer?.gb || 1.9).toFixed(1)} GB)`,
      hidden: () => !hw.cuda_offer?.offer,
      hint: "Lets this app run its models on the NVIDIA card. Downloads NVIDIA's own CUDA 12 and cuDNN 9 packages and Microsoft's ONNX Runtime with CUDA from PyPI (the public Python package index), for this app's Python 3.12, each checked by size and SHA-256, into the app's data folder (about 3.5 GB unpacked). Used from the next start; the app checks each model gives the same answers on the card before using it." },
  ], {
    dbsetup: a => post("/api/db/setup", { host: a.host || "", port: +a.port || 5432, dbname: a.dbname || "sentient",
                                         user: a.user || "", password: a.password || "" }),
    wsgoogle: a => post("/api/websearch/key", { key: a.key || "" }),
    wsaws: a => post("/api/websearch/aws", { id: a.id || "", secret: a.secret || "", region: a.region || "us-east-1" }),
    descget: () => post("/api/describe/getmodel", {}),
    cudaget: () => post("/api/cuda/download", {}),
    lmsget: () => post("/api/lmstudio/get", {}),
    describeTest: () => post("/api/describe/test", { url: desc.config?.url || "" }),
    describeKey: a => post("/api/describe/config", { key: a.key || "" }),
  });
}

(async () => {
  try {
    await loadSummary();
    view = viewOf(location.hash) || { v: SUM.suggested ? "suggested" : SUM.unknown ? "unknown" : "checks" };
    history.replaceState(null, "", hashOf(view));
    await render(true);
  } catch (e) {
    // In the app window Start again asks the window to start it again (app.py /restart); on its own, it reloads
    $("#main").innerHTML = `<div class="empty"><b>Couldn't reach Face Labels</b>${window.parent !== window
      ? "Click Start again (or close Face Recognizer and start it again)."
      : "Start label.bat again (label.sh on macOS and Linux, or python label_tool.py), then click Start again."}
      <p><button class="btn primary" id="againBtn" type="button">Start again</button></p></div>`;
    $("#againBtn").addEventListener("click", () => {
      if (window.parent !== window) SettingsBridge.notify({ fs: "restart" }); else location.reload();
    });
  }
})();
