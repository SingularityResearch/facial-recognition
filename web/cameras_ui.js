// cameras_ui.js - the Cameras page (cameras_ui.html, served by the tracker): every device's live camera in a grid,
// as that device's tracker page shows it (names, boxes and every marker on), with its live readout beside it, and
// one list of events from every camera. Streams come through this device's own app (/api/cameras/...),
// which fetches them from the other devices over the peer service (TLS, signed requests). Readouts are
// data, drawn here with text only (no markup from another device). Under each camera, its Signals chart
// (signals_chart.js): every reading of that device as it sends them with its readout, each value checked there.
// One camera can be looked at alone (its Focus button, or a double-click on its picture): the page is then laid out
// as that device's own Face tracking tab is, its picture large, its live readout beside it and its Signals chart
// in a drawer under the picture; All cameras (or Esc) brings the grid back. The other cameras' frames and readings
// go on arriving meanwhile.
"use strict";
const $ = id => document.getElementById(id);
const el = (tag, props = {}, ...kids) => { const n = Object.assign(document.createElement(tag), props); n.append(...kids.filter(x => x != null)); return n; };
const localTime = s => new Date(s).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
const KIND = { arrived: "arrived", left: "left", unusual: "unusual signals", unknown: "unknown face", moved: "moved between cameras", joined: "new device" };
let devices = [], hidden = new Set(), solo = null;
const tiles = {};                          // device id -> {tile, img, st, msg, faces, seq, url, lastFrame, frames, d}
try { hidden = new Set(JSON.parse(localStorage.getItem("cams-hidden") || "[]")); } catch {}
const saveHidden = () => { try { localStorage.setItem("cams-hidden", JSON.stringify([...hidden])); } catch {} };
// the Signals charts: which cameras' are open, which of their sections, and the time they show (remembered)
const kept = (key, none) => { try { const v = JSON.parse(localStorage.getItem(key)); return v ?? none; } catch { return none; } };
const keep = (key, v) => { try { localStorage.setItem(key, JSON.stringify(v)); } catch {} };
const sigOpen = new Set(kept("cams-sig", []));

async function loadDevices() {
  let r = { devices: [], why: "" };
  try { r = await (await fetch("/api/cameras", { cache: "no-store" })).json(); } catch { r.why = "This device's tracker isn't answering."; }
  devices = r.devices || [];
  for (const d of devices) d.seen = !!d.streamingAt;          // has streamed at some time: keep its tile
  $("why").textContent = r.why || (devices.some(d => d.streaming) ? "" : "No camera is streaming now: a device streams while its Face tracking tab has the camera on (Stream this camera to my other devices).");
  drawPicker();
  layout();
}

function drawPicker() {
  $("pick").replaceChildren(...devices.map(d => {
    const c = el("input", { type: "checkbox", checked: !hidden.has(d.id) });
    c.onchange = () => { if (c.checked) hidden.delete(d.id); else hidden.add(d.id); saveHidden(); layout(); };
    return el("label", { className: d.streaming ? "" : "off", title: d.streaming ? "Streaming" : "Not streaming now" }, c,
              `${d.name}${d.me ? " (this device)" : ""}`);
  }));
}

function layout() {
  // every ticked camera stays in the grid: one that stops streaming shows "offline" instead of disappearing
  const show = devices.filter(d => !hidden.has(d.id) && (d.streaming || tiles[d.id] || d.seen));
  // a tile marked offline still asks for frames now and then: the first frame brings it back at once
  for (const id of Object.keys(tiles)) if (!show.some(d => d.id === id)) { stopTile(id); }
  if (!show.length) {
    $("grid").replaceChildren(el("div", { className: "empty", textContent: devices.length ? "No camera to show." : "Looking for cameras…" }));
    return;
  }
  $("grid").querySelector(".empty")?.remove();
  for (const d of show) if (!tiles[d.id]) startTile(d);
  for (const d of show) offline(tiles[d.id], d);
  if (solo && !tiles[solo]) solo = null;                       // the camera looked at alone is no longer shown
  const n = solo ? 1 : show.length;
  $("grid").style.gridTemplateColumns = n <= 1 ? "1fr" : n <= 4 ? "repeat(2, minmax(0, 1fr))" : "repeat(3, minmax(0, 1fr))";
  for (const [id, t] of Object.entries(tiles)) {
    t.tile.hidden = !!solo && solo !== id;
    t.tile.classList.toggle("solo", solo === id);
    t.focus.textContent = solo === id ? "All cameras" : "Focus";
    t.focus.setAttribute("aria-pressed", String(solo === id));
  }
  // one camera alone: the header names it and offers the way back
  $("grid").classList.toggle("focus", !!solo);
  document.querySelector("header").classList.toggle("focus", !!solo);
  $("allBtn").hidden = !solo;
  $("focusName").textContent = solo ? `${tiles[solo].d.name}${tiles[solo].d.me ? " (this device)" : ""}` : "";
  if (solo) tiles[solo].panel.draw();
}
// This camera alone, or (null) all cameras again.
function focusOn(id) { solo = id; layout(); if (!id) $("grid").scrollTop = 0; }

function offline(t, d) {
  if (!t) return;
  t.d = d;
  // frames arriving win over the database's "last streamed" time, which a busy device may update late
  if (d.streaming || (t.lastFrame && Date.now() - t.lastFrame < 10000)) {
    if (t.down) { t.down = false; t.msg.hidden = true; }   // back: its pump picks the frames up again
    return;
  }
  t.down = true;
  t.st.className = "st off";
  const ago = d.streamingAt ? ` since ${new Date(d.streamingAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}` : "";
  t.st.textContent = `offline${ago}`;
  t.msg.hidden = false;
  t.msg.textContent = `${d.name}'s camera is offline${ago}: its app or camera isn't running, or it can't be reached. It comes back here by itself.`;
}

// A camera whose picture hasn't come yet shows that it is being connected to (a turning ring and a line of words),
// never an empty black box: from the moment it is put on the page, and again whenever its picture stops coming
// without a reason that is said in its place.
function waiting(t) {
  t.st.className = "st"; t.st.textContent = "connecting";
  t.msg.className = "msg wait";
  t.msg.textContent = `Connecting to ${t.d.name}${t.d.me ? " (this device)" : ""}'s camera…`;
  t.msg.hidden = false;
}

function startTile(d) {
  const img = el("img", { alt: `${d.name}'s camera` }), st = el("span", { className: "st", textContent: "connecting" });
  const msg = el("div", { className: "msg", hidden: true });
  const faces = el("ul", { className: "faces", ariaLabel: `Faces on ${d.name}'s camera` }, el("li", { className: "none", textContent: "No faces in view" }));
  const focus = el("button", { className: "fbtn", type: "button", textContent: "Focus", title: "This camera alone, laid out as its own Face tracking tab: large picture, live readout, Signals under it" });
  focus.setAttribute("aria-pressed", "false");
  const vid = el("div", { className: "vid", title: "Double-click: this camera alone (again: all cameras)" }, img,
                 el("div", { className: "bar" }, el("span", { className: "nm", textContent: `${d.name}${d.me ? " (this device)" : ""}` }), el("span", { className: "right" }, st, focus)), msg);
  // beside the picture: its live readout; with the camera alone also the three counts its own tab shows
  const stat = word => { const b = el("b", { textContent: "–" }); return [b, el("div", { className: "stat" }, b, el("span", { textContent: word }))]; };
  const [nFaces, sFaces] = stat("faces"), [nKnown, sKnown] = stat("recognized"), [nChecks, sChecks] = stat("checks / sec");
  const side = el("div", { className: "side" }, el("div", { className: "label sidel", textContent: "Live readout" }), el("div", { className: "stats" }, sFaces, sKnown, sChecks), faces);
  // its Signals chart: the readings are taken from the first readout on, shown or not
  const sigBody = el("div", { className: "body" });
  const sigs = el("details", { className: "sigs", open: sigOpen.has(d.id) },
    el("summary", {}, el("span", { className: "label", textContent: "Signals" }), el("span", { className: "signote", textContent: "every reading of this device, as charts" })), sigBody);
  const tile = el("section", { className: "tile", ariaLabel: `${d.name}'s camera` }, vid, side, sigs);
  const panel = new SignalsChart.Panel(sigBody, {
    device: d.me ? "This device" : String(d.name ?? ""), open: kept("cams-sig-open", null), onOpen: keys => keep("cams-sig-open", keys),
    window: kept("cams-sig-window", undefined), onWindow: ms => keep("cams-sig-window", ms) });
  tile.classList.toggle("charted", sigs.open);
  sigs.ontoggle = () => {
    if (sigs.open) sigOpen.add(d.id); else sigOpen.delete(d.id);
    keep("cams-sig", [...sigOpen]);
    tile.classList.toggle("charted", sigs.open);              // the whole row while its chart is open
    panel.draw();
  };
  const t = tiles[d.id] = { tile, img, st, msg, faces, panel, focus, counts: [nFaces, nKnown, nChecks], seq: 0, url: null, lastFrame: 0, frames: 0, d };
  vid.ondblclick = () => focusOn(solo === d.id ? null : d.id);
  focus.onclick = () => focusOn(solo === d.id ? null : d.id);
  focus.ondblclick = e => e.stopPropagation();                  // (a quick second click on the button is not the picture's double-click)
  waiting(t);
  $("grid").append(tile);
}

// Every camera's frames through one request at a time (/api/cameras/frames): it asks for each tile's
// next frame after the last one shown and is answered as soon as any camera has one (or after 3 s with none), with
// every camera's new frame in one answer. A browser allows six connections at a time to one address, so the page's
// requests must not grow with the number of cameras: frames take one, readouts another, now and then.
const sleep = ms => new Promise(r => setTimeout(r, ms));
function showFrame(t, seq, jpeg) {
  t.seq = seq;
  const url = URL.createObjectURL(jpeg);
  t.img.src = url;
  if (t.url) URL.revokeObjectURL(t.url);
  t.url = url; t.lastFrame = Date.now(); t.frames++;
  t.msg.hidden = true; t.msg.className = "msg"; t.down = false;
}
async function pumpAll() {
  for (;;) {
    const ids = Object.keys(tiles);
    if (!ids.length) { await sleep(500); continue; }
    try {
      const r = await fetch(`/api/cameras/frames?after=${ids.map(id => `${id}:${tiles[id].seq}`).join(",")}`,
                            { cache: "no-store", signal: AbortSignal.timeout(15000) });
      if (!r.ok) { r.body?.cancel().catch(() => {}); await sleep(1000); continue; }   // an unread answer keeps a descriptor (see drop in tracker_ui.js)
      const buf = await r.arrayBuffer(), n = new DataView(buf).getUint32(0);
      const head = JSON.parse(new TextDecoder().decode(new Uint8Array(buf, 4, n)));
      let off = 4 + n;
      for (const f of head.frames || []) {
        const t = tiles[f.id], part = buf.slice(off, off + f.size);
        off += f.size;
        if (t) showFrame(t, f.seq, new Blob([part], { type: "image/jpeg" }));
      }
      for (const [id, why] of Object.entries(head.errors || {})) {
        const t = tiles[id];
        if (!t || t.down || (t.lastFrame && Date.now() - t.lastFrame < 3000)) continue;
        t.st.className = "st off"; t.st.textContent = "can't reach it";
        t.msg.hidden = false; t.msg.className = "msg"; t.msg.textContent = `Can't reach this camera now (${why}); trying again…`;
      }
    } catch { await sleep(1000); }
  }
}
pumpAll();

function stopTile(id) {
  const t = tiles[id];
  if (!t) return;
  t.panel.close();
  t.tile.remove();
  if (t.url) URL.revokeObjectURL(t.url);
  delete tiles[id];                          // the pump stops asking for it
  if (solo === id) solo = null;
}

// status: live with frames per second, or why not
setInterval(() => {
  const now = Date.now();
  for (const t of Object.values(tiles)) {
    const fps = t.frames; t.frames = 0;
    if (t.down || t.paused) continue;
    if (t.lastFrame && now - t.lastFrame < 4000) { t.st.className = "st live"; t.st.textContent = `live · ${fps} fps`; }
    // no picture yet, or none for a while, and nothing said about why: it is being connected to, and says so
    else if (t.msg.hidden) waiting(t);
  }
}, 1000);

// each camera's live readout: one block a face, as the tracker shows it (text only)
function faceBlock(f) {
  const unk = f.name === "Unknown";
  const li = el("li", { className: "face" + (f.lost ? " lost" : "") },
    el("div", { className: "fhead" }, el("span", { className: "sw", style: `background:${/^#[0-9a-f]{3,8}$|^hsl\([\d.,% ]+\)$/i.test(f.color) ? f.color : "#888"}` }),
       el("span", { className: "who" + (unk ? " unk" : ""), textContent: String(f.name ?? "") }),
       el("span", { className: "match", textContent: f.sim == null ? "" : `${f.sim}%` })));
  if (f.sim != null) li.append(el("div", { className: "meter" }, el("i", { style: `width:${Math.max(0, Math.min(100, +f.sim || 0))}%` })));
  li.append(el("div", { className: "fsub" }, el("span", { className: f.exprState === "change" ? "change" : "", textContent: String(f.expr || "") }),
                                             el("span", { textContent: `${+f.size || 0} px${f.lost ? " · lost" : ""}` })));
  for (const [title, note, cells] of Array.isArray(f.groups) ? f.groups : []) {
    const g = el("div", { className: "grp" }, el("div", { className: "gl", textContent: String(title) }, note ? el("em", { textContent: " " + note }) : null));
    g.append(el("div", { className: "cells" }, ...(Array.isArray(cells) ? cells : []).map(([k, v, hot, wide]) =>
      el("div", { className: "cell" + (wide ? " wide" : "") }, el("span", { textContent: String(k) }),
         el("b", { className: v == null ? "na" : hot ? "hot" : "", textContent: v == null ? "–" : String(v), title: v == null ? "no reading" : String(v) })))));
    li.append(g);
  }
  return li;
}
// Each camera's readings as its device publishes them: this page asks its own server for every shown device's
// bus in ONE waiting question, and is answered the moment any of them publishes (its server follows those
// devices for as long as this page asks). So a camera's chart moves when its device takes a reading, not once a
// second. One question, however many cameras: a browser gives a page a handful of connections to one address, and
// a question for each camera took them from the pictures. The readings that come with a readout are taken as
// before; a chart keeps each reading once, by its own time, whichever way it came first. A device that can't be
// followed (away, or an earlier version) is named in an answer only when nothing else happened for the whole
// wait, and its chart lives on the readout meanwhile.
const busPlace = {};                   // device id -> this page's place in that device's bus, "<number>:<run>"
let busFollowing = false;
async function followAll() {
  if (busFollowing) return;
  busFollowing = true;
  let again = 500;
  for (;;) {
    const ids = Object.keys(tiles);
    for (const id of Object.keys(busPlace)) if (!tiles[id]) delete busPlace[id];
    if (!ids.length) { await sleep(1000); continue; }
    try {
      const nodes = ids.map(id => busPlace[id] ? `${id}:${busPlace[id]}` : id).join(",");
      const r = await fetch(`/api/bus/many?nodes=${nodes}&paths=readings&wait=20`, { cache: "no-store" });
      const j = await r.json();
      if (!r.ok) throw new Error("no answer");
      for (const [id, a] of Object.entries(j.nodes && typeof j.nodes === "object" ? j.nodes : {})) {
        const t = tiles[id];
        if (!t || !a || typeof a !== "object") continue;
        if (typeof a.epoch === "string" && /^[A-Za-z0-9]{1,64}$/.test(a.epoch) && Number.isInteger(a.seq)) busPlace[id] = `${a.seq}:${a.epoch}`;
        for (const e of Array.isArray(a.events) ? a.events : []) {
          if (e && e.path === "readings" && e.value && typeof e.value === "object") { try { t.panel.take(e.value); } catch {} }
        }
      }
      again = 500;
    } catch { await sleep(again); again = Math.min(again * 2, 15000); }
  }
}
followAll();
const howLong = s => s < 90 ? `${Math.round(s)} s` : s < 5400 ? `${Math.round(s / 60)} min` : `${Math.round(s / 3600)} h`;
let readoutsBusy = false;
async function loadReadouts() {                  // every camera's readout in one request
  const ids = Object.keys(tiles);
  if (!ids.length || readoutsBusy) return;
  readoutsBusy = true;
  try {
    const all = await (await fetch(`/api/cameras/readouts?ids=${ids.join(",")}`, { cache: "no-store" })).json();
    for (const [id, r] of Object.entries(all)) {
      const t = tiles[id];
      if (!t || !r) continue;
      const faces = Array.isArray(r.faces) ? r.faces : [];
      // frozen picture: the camera's device sent no new frame lately (its camera stopped, or its window was asleep)
      const stale = typeof r.frameAge === "number" && r.frameAge > 5 && !(t.lastFrame && Date.now() - t.lastFrame < 2000);
      // still picture: its frames arrive, each the same one, because its camera sends no new picture (on Windows a
      // locked screen does this; its tracker page says so with the readout)
      const still = typeof r.paused === "number" && r.paused > 0;
      if (still) {
        t.st.className = "st off"; t.st.textContent = `paused · no new picture for ${howLong(r.paused)}`; t.paused = true;
        t.st.title = "Its camera is on but sends no new picture. On Windows a locked screen does this.";
      } else if (stale) { t.st.className = "st off"; t.st.textContent = `paused · no new frame for ${howLong(r.frameAge)}`; t.st.title = ""; t.paused = true; }
      else if (t.paused) { t.paused = false; t.st.title = ""; }      // its picture again
      t.faces.replaceChildren(...(faces.length ? faces.map(faceBlock) : [el("li", { className: "none", textContent: r.error ? "Readout unavailable" : "No faces in view" })]));
      if (r.signals && !r.error) { try { t.panel.take(r.signals); } catch {} }      // its readings since the last readout, for its chart
      // a chart with nothing in it says why: the device can't be reached, or its app is older than the chart
      // (a device sends its readings with its readout while Stream this camera is on there)
      t.panel.tellWhy(r.signals ? "" : r.error || t.down ? "No readings: this device can't be reached now. What it sent before stays in the chart."
        : "This device doesn't send its readings yet. Its app has to be up to date, with Stream this camera to my other devices on.");
      // the three counts of its own tab, for the camera looked at alone (text only)
      const inView = faces.filter(f => !f.lost), rows = t.panel.model.device.rows, checks = rows.length ? rows[rows.length - 1].v.checks : null;
      const fresh = rows.length && Date.now() - rows[rows.length - 1].t < 5000;
      [inView.length, inView.filter(f => f.name !== "Unknown").length, fresh && checks != null ? Math.round(checks) : "–"]
        .forEach((v, i) => { const said = String(v); if (t.counts[i].textContent !== said) t.counts[i].textContent = said; });
    }
  } catch {} finally { readoutsBusy = false; }
}
setInterval(loadReadouts, 1000);

// events from every camera (the alert inbox, which every device shares through the database)
// The events panel is a fly-out: closed, the cameras take the whole width and its button counts what came since.
let evOpen = true, evSeenT = 0, evNewest = 0;
try { evOpen = localStorage.getItem("cams-events") !== "0"; } catch {}
function setEvents(open) {
  evOpen = open;
  document.querySelector("main").classList.toggle("noev", !open);
  $("events").hidden = !open;
  $("evBtn").setAttribute("aria-expanded", open);
  $("evBtnText").textContent = open ? "Hide events" : "Events";
  if (open) evSeenT = evNewest;
  evCount(0);
  try { localStorage.setItem("cams-events", open ? "1" : "0"); } catch {}
}
function evCount(n) { $("evN").textContent = n; $("evN").hidden = evOpen || !n; }
$("evBtn").onclick = () => setEvents(!evOpen);
$("evClose").onclick = () => { setEvents(false); $("evBtn").focus(); };
setEvents(evOpen);

async function loadEvents() {
  try {
    const r = await (await fetch("/api/alerts", { cache: "no-store" })).json();
    const all = r.alerts || [];
    evNewest = Math.max(evNewest, ...all.map(a => +a.t || 0));
    if (evOpen || !evSeenT) evSeenT = evNewest;
    evCount(all.filter(a => +a.t > evSeenT).length);
    $("evlist").replaceChildren(...all.slice(0, 60).map(a =>
      el("li", { className: `${a.kind} ${a.state === "new" ? "new" : ""}` },
         el("span", { className: "t", textContent: `${localTime(a.t)} · ${KIND[a.kind] || a.kind}` }), String(a.text || ""))));
  } catch {}
}
$("allBtn").onclick = () => focusOn(null);
addEventListener("keydown", e => { if (e.key === "Escape" && solo) focusOn(null); });
setInterval(loadEvents, 3000);
setInterval(loadDevices, 10000);
loadDevices();
loadEvents();
