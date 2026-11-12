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
const tiles = {};                          // device id -> {tile, img, st, msg, faces, lastFrame, retryAt, d}
try { hidden = new Set(JSON.parse(localStorage.getItem("cams-hidden") || "[]")); } catch {}
const saveHidden = () => { try { localStorage.setItem("cams-hidden", JSON.stringify([...hidden])); } catch {} };

async function loadDevices() {
  let r = { devices: [], why: "" };
  try { r = await (await fetch("/api/cameras", { cache: "no-store" })).json(); } catch { r.why = "This device's tracker isn't answering."; }
  devices = r.devices || [];
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
  const show = devices.filter(d => d.streaming && !hidden.has(d.id));
  for (const id of Object.keys(tiles)) if (!show.some(d => d.id === id)) { stopTile(id); }
  if (!show.length) {
    $("grid").replaceChildren(el("div", { className: "empty", textContent: devices.length ? "No camera to show." : "Looking for cameras…" }));
    return;
  }
  $("grid").querySelector(".empty")?.remove();
  for (const d of show) if (!tiles[d.id]) startTile(d);
  const n = solo ? 1 : show.length;
  $("grid").style.gridTemplateColumns = n <= 1 ? "1fr" : n <= 4 ? "repeat(2, minmax(0, 1fr))" : "repeat(3, minmax(0, 1fr))";
  for (const [id, t] of Object.entries(tiles)) {
    t.tile.hidden = !!solo && solo !== id;
    t.tile.classList.toggle("solo", solo === id);
  }
}

function startTile(d) {
  const img = el("img", { alt: `${d.name}'s camera` }), st = el("span", { className: "st", textContent: "connecting" });
  const msg = el("div", { className: "msg", hidden: true });
  const faces = el("ul", { className: "faces", ariaLabel: `Faces on ${d.name}'s camera` }, el("li", { className: "none", textContent: "No faces in view" }));
  const vid = el("div", { className: "vid", title: "Double-click: this camera alone (again: all cameras)" }, img,
                 el("div", { className: "bar" }, el("span", { textContent: `${d.name}${d.me ? " (this device)" : ""}` }), st), msg);
  const tile = el("section", { className: "tile", ariaLabel: `${d.name}'s camera` }, vid, faces);
  const t = tiles[d.id] = { tile, img, st, msg, faces, lastFrame: 0, retryAt: 0, d };
  vid.ondblclick = () => { solo = solo === d.id ? null : d.id; layout(); };
  img.onload = () => { t.lastFrame = Date.now(); msg.hidden = true; };
  img.onerror = () => { st.className = "st off"; st.textContent = "offline"; msg.hidden = false;
                        msg.textContent = "Can't reach this camera now; trying again…"; t.retryAt = Date.now() + 5000; };
  connect(t);
  $("grid").append(tile);
}

function connect(t) {
  t.st.className = "st"; t.st.textContent = "connecting";
  t.img.src = `/api/cameras/${t.d.id}/stream?n=${Date.now()}`;                 // each answer lasts 2 minutes
  t.connectedAt = Date.now();
}

function stopTile(id) {
  const t = tiles[id];
  if (!t) return;
  t.img.removeAttribute("src");
  t.tile.remove();
  delete tiles[id];
  if (solo === id) solo = null;
}

// status and reconnecting: a stream answer lasts 2 minutes (peer.STREAM_FOR), so each tile asks again a little
// before; an <img> fires "load" for a multipart stream's first frame only, so "live" means it started and hasn't failed
setInterval(() => {
  const now = Date.now();
  for (const t of Object.values(tiles)) {
    if (t.retryAt && now > t.retryAt) { t.retryAt = 0; connect(t); continue; }
    if (!t.retryAt && now - (t.connectedAt || 0) > 110000) connect(t);
    else if (t.lastFrame && !t.retryAt && !t.paused) { t.st.className = "st live"; t.st.textContent = "live"; }
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
async function loadReadouts() {
  await Promise.all(Object.entries(tiles).map(async ([id, t]) => {
    try {
      const r = await (await fetch(`/api/cameras/${id}/readout`, { cache: "no-store" })).json();
      const faces = Array.isArray(r.faces) ? r.faces : [];
      // frozen picture: the camera's device sent no new frame lately (its camera stopped, or its window was asleep)
      const stale = typeof r.frameAge === "number" && r.frameAge > 5;
      if (stale && !t.retryAt) { t.st.className = "st off"; t.st.textContent = `paused · no new frame for ${Math.round(r.frameAge)} s`; t.paused = true; }
      else if (!stale && t.paused) { t.paused = false; connect(t); }          // frames again: pick the stream up
      t.faces.replaceChildren(...(faces.length ? faces.map(faceBlock) : [el("li", { className: "none", textContent: r.error ? "Readout unavailable" : "No faces in view" })]));
    } catch {}
  }));
}
setInterval(loadReadouts, 1000);

// events from every camera (the alert inbox, which every device shares through the database)
async function loadEvents() {
  try {
    const r = await (await fetch("/api/alerts", { cache: "no-store" })).json();
    $("evlist").replaceChildren(...(r.alerts || []).slice(0, 60).map(a =>
      el("li", { className: `${a.kind} ${a.state === "new" ? "new" : ""}` },
         el("span", { className: "t", textContent: `${localTime(a.t)} · ${KIND[a.kind] || a.kind}` }), String(a.text || ""))));
  } catch {}
}
setInterval(loadEvents, 3000);
setInterval(loadDevices, 10000);
loadDevices();
loadEvents();
