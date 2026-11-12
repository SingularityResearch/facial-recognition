// cameras_ui.js - the Cameras page (cameras_ui.html, served by tracker.py): every device's live camera in a grid,
// as that device's tracker page shows it (names, boxes and every marker on), with its live readout beside it, and
// one list of events from every camera. Streams come through this device's own app (tracker.py /api/cameras/...),
// which fetches them from the other devices over the peer service (peer.py: TLS, signed requests). Readouts are
// data, drawn here with text only (no markup from another device).
"use strict";
const $ = id => document.getElementById(id);
const el = (tag, props = {}, ...kids) => { const n = Object.assign(document.createElement(tag), props); n.append(...kids.filter(x => x != null)); return n; };
const localTime = s => new Date(s).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
const KIND = { arrived: "arrived", left: "left", unusual: "unusual signals", unknown: "unknown face", moved: "moved between cameras" };
let devices = [], hidden = new Set(), solo = null;
const tiles = {};                          // device id -> {tile, img, st, msg, faces, seq, url, lastFrame, frames, d}
try { hidden = new Set(JSON.parse(localStorage.getItem("cams-hidden") || "[]")); } catch {}
const saveHidden = () => { try { localStorage.setItem("cams-hidden", JSON.stringify([...hidden])); } catch {} };

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
  const n = solo ? 1 : show.length;
  $("grid").style.gridTemplateColumns = n <= 1 ? "1fr" : n <= 4 ? "repeat(2, minmax(0, 1fr))" : "repeat(3, minmax(0, 1fr))";
  for (const [id, t] of Object.entries(tiles)) {
    t.tile.hidden = !!solo && solo !== id;
    t.tile.classList.toggle("solo", solo === id);
  }
}

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

function startTile(d) {
  const img = el("img", { alt: `${d.name}'s camera` }), st = el("span", { className: "st", textContent: "connecting" });
  const msg = el("div", { className: "msg", hidden: true });
  const faces = el("ul", { className: "faces", ariaLabel: `Faces on ${d.name}'s camera` }, el("li", { className: "none", textContent: "No faces in view" }));
  const vid = el("div", { className: "vid", title: "Double-click: this camera alone (again: all cameras)" }, img,
                 el("div", { className: "bar" }, el("span", { textContent: `${d.name}${d.me ? " (this device)" : ""}` }), st), msg);
  const tile = el("section", { className: "tile", ariaLabel: `${d.name}'s camera` }, vid, faces);
  const t = tiles[d.id] = { tile, img, st, msg, faces, seq: 0, url: null, lastFrame: 0, frames: 0, d };
  vid.ondblclick = () => { solo = solo === d.id ? null : d.id; layout(); };
  $("grid").append(tile);
}

// Every camera's frames through one request at a time (tracker.py /api/cameras/frames): it asks for each tile's
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
  t.msg.hidden = true; t.down = false;
}
async function pumpAll() {
  for (;;) {
    const ids = Object.keys(tiles);
    if (!ids.length) { await sleep(500); continue; }
    try {
      const r = await fetch(`/api/cameras/frames?after=${ids.map(id => `${id}:${tiles[id].seq}`).join(",")}`,
                            { cache: "no-store", signal: AbortSignal.timeout(15000) });
      if (!r.ok) { await sleep(1000); continue; }
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
        t.msg.hidden = false; t.msg.textContent = `Can't reach this camera now (${why}); trying again…`;
      }
    } catch { await sleep(1000); }
  }
}
pumpAll();

function stopTile(id) {
  const t = tiles[id];
  if (!t) return;
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
      if (stale) { t.st.className = "st off"; t.st.textContent = `paused · no new frame for ${Math.round(r.frameAge)} s`; t.paused = true; }
      else if (t.paused) { t.paused = false; }                       // frames again
      t.faces.replaceChildren(...(faces.length ? faces.map(faceBlock) : [el("li", { className: "none", textContent: r.error ? "Readout unavailable" : "No faces in view" })]));
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
setInterval(loadEvents, 3000);
setInterval(loadDevices, 10000);
loadDevices();
loadEvents();
