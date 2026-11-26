// cameras_ui.js - the Cameras page (cameras_ui.html, served by the tracker): every device's live camera in a grid,
// as that device's tracker page shows it (names, boxes and every marker on), with its live readout beside it, and
// one list of events from every camera. Streams come through this device's own app (/api/cameras/...),
// which fetches them from the other devices over the peer service (TLS, signed requests). Readouts are
// data, drawn here with text only (no markup from another device). Under each camera, its Signals chart
// (signals_chart.js): every reading of that device as it sends them with its readout, each value checked there.
// Over each picture's lower edge, where that device is (place.js), for a device whose owner switched its location on.
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

// The cameras to show: a fly-out list under its button (like the events'), with a tick for each camera and one for
// all of them. Its rows are kept and changed in place as the device list comes again, so a tick being clicked or a
// row with the keyboard's focus is never replaced under the hand.
const rows = new Map();                  // device id -> {row, box, dot, nm, sub}
function drawPicker() {
  const list = $("pick");
  for (const [id, r] of rows) if (!devices.some(d => d.id === id)) { r.row.remove(); rows.delete(id); }
  devices.forEach((d, i) => {
    let r = rows.get(d.id);
    if (!r) {
      const box = el("input", { type: "checkbox" }), dot = el("span", { className: "dot" });
      const nm = el("span", { className: "nm" }), sub = el("span", { className: "sub" });
      box.onchange = () => { if (box.checked) hidden.delete(d.id); else hidden.add(d.id); picked(); };
      r = { row: el("label", {}, box, dot, el("span", { className: "txt" }, nm, sub)), box, dot, nm, sub };
      rows.set(d.id, r);
    }
    if (list.children[i] !== r.row) list.insertBefore(r.row, list.children[i] || null);
    r.box.checked = !hidden.has(d.id);
    r.row.classList.toggle("off", !d.streaming);
    r.dot.classList.toggle("live", !!d.streaming);
    r.nm.textContent = `${d.name}${d.me ? " (this device)" : ""}`;
    const where = Place.words(d.place);
    r.sub.textContent = (d.streaming ? "streaming" : "not streaming now") + (where ? " · " + where : "");
    r.row.title = r.nm.textContent + "\n" + r.sub.textContent;
  });
  list.querySelector(".none")?.remove();
  if (!devices.length) list.append(el("div", { className: "none", textContent: "No cameras found yet." }));
  pickSummary();
}
// The button says how many of the cameras are shown; the tick for all is ticked, half-ticked or not.
function pickSummary() {
  const n = devices.length, on = devices.filter(d => !hidden.has(d.id)).length;
  $("camBtnText").textContent = !n ? "Cameras" : on === n ? `Cameras: all ${n}` : on ? `Cameras: ${on} of ${n}` : "Cameras: none";
  const all = $("camAll");
  all.checked = n > 0 && on === n;
  all.indeterminate = on > 0 && on < n;
  all.disabled = !n;
}
function picked() { saveHidden(); pickSummary(); layout(); }
$("camAll").onchange = () => {
  const on = $("camAll").checked;
  for (const d of devices) { if (on) hidden.delete(d.id); else hidden.add(d.id); }
  for (const r of rows.values()) r.box.checked = on;
  picked();
};
const menuOpen = () => !$("camMenu").hidden;
function setMenu(open) {
  $("camMenu").hidden = !open;
  $("camBtn").setAttribute("aria-expanded", String(open));
  if (open) ($("camAll").disabled ? $("camClose") : $("camAll")).focus();
}
$("camBtn").onclick = () => setMenu(!menuOpen());
$("camClose").onclick = () => { setMenu(false); $("camBtn").focus(); };
// a click anywhere else closes the list
document.addEventListener("pointerdown", e => { if (menuOpen() && !$("pickWrap").contains(e.target)) setMenu(false); });

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
  for (const d of show) { offline(tiles[d.id], d); showPlace(tiles[d.id], d); }
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
function focusOn(id) { solo = id; setMenu(false); layout(); if (!id) $("grid").scrollTop = 0; }

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

// Where the camera's device is, as this page's server says it with the list of devices (the device's own word while
// this page follows it, else the newest row of its history): one line of text over the picture's lower edge, and
// nothing for a device whose owner has not switched its location on.
function showPlace(t, d) {
  if (!t) return;
  const text = Place.words(d.place);
  t.place.hidden = !text;
  if (t.place.textContent !== text) { t.place.textContent = text; t.place.title = text; }
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
  const msg = el("div", { className: "msg", hidden: true }), place = el("div", { className: "place", hidden: true });
  const faces = el("ul", { className: "faces", ariaLabel: `Faces on ${d.name}'s camera` }, el("li", { className: "none", textContent: "No faces in view" }));
  const focus = el("button", { className: "fbtn", type: "button", textContent: "Focus", title: "This camera alone, laid out as its own Face tracking tab: large picture, live readout, Signals under it" });
  focus.setAttribute("aria-pressed", "false");
  const vid = el("div", { className: "vid", title: "Double-click: this camera alone (again: all cameras)" }, img,
                 el("div", { className: "bar" }, el("span", { className: "nm", textContent: `${d.name}${d.me ? " (this device)" : ""}` }), el("span", { className: "right" }, st, focus)), msg, place);
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
  const t = tiles[d.id] = { tile, img, st, msg, place, faces, panel, focus, counts: [nFaces, nKnown, nChecks], seq: 0, url: null, lastFrame: 0, frames: 0, d };
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

// Each camera's live readout: one block a face, made from the groups of readings its device sends (text only).
// First a summary a person can take in at a glance: the name and how sure, the expression, what stands out now
// (talking, looking away, a hand on the face, unusual for them) and the key numbers. Then each group of readings
// as a section of its own, closed until it is opened (which sections are open is remembered), with a preview of
// its first readings and how many deserve a look. Every reading has its fuller name, and what it says on
// pointing at it (docs/READINGS.md has them all). A group or a reading a device sends that isn't named here is
// shown as it comes: devices of other versions send their own.
const GROUP_ABOUT = {
  "Expression": "How the face looks, and how sure the reading is.",
  "Face": "Movement, eyes, mouth and pulse, from the face itself.",
  "Voice": "This person's voice, while they speak.",
  "Body": "Posture, movement, breathing and hands.",
  "Their normal": "These readings against this person's own usual.",
};
const ABOUT = {
  "Expression": {
    votes: ["votes", "How many of the latest checks (about a second) the expression is made from. A clear reading needs 3 or more."],
    share: ["share of votes", "How much of the votes went to the expression shown. A clear reading needs 45% or more."],
    lead: ["lead", "How far the expression shown is ahead of the next one. A clear reading needs 12% or more; a small lead means two looks are nearly tied."],
    valence: ["valence", "How pleasant (+) or unpleasant (−) the face looks, from −1 to +1. Around 0 is neutral."],
    "valence (fit)": ["valence (fit)", "How pleasant (+) or unpleasant (−) the face looks, from −1 to +1, fitted to your own feeling reports."],
    arousal: ["arousal", "How calm (−) or energetic and worked up (+) the face looks, from −1 to +1."],
    "arousal (fit)": ["arousal (fit)", "How calm (−) or energetic (+) the face looks, from −1 to +1, fitted to your own feeling reports."],
    calibrated: ["calibrated confidence", "How often a reading this sure was right in earlier posed sessions."],
    "state model": ["state model", "A second way of reading the expression, which carries its belief from check to check. Highlighted when it disagrees."],
    "usual look": ["their usual look", "Whether this person's usual look is learned. A change from it is what is highlighted."],
    flags: ["why it isn't clear", "Few votes, low share or low lead; or why the last check wasn't read: blurry, turned, talking, hand, learning."],
    cues: ["face-shape cues", "The movements of the face that go with this expression (smile, brows, eyes...), each from 0 to 1."],
  },
  "Face": {
    talking: ["talking", "Whether this person is talking now: the jaw moves as in speech and the microphone, where it is listened to, hears a voice."],
    moving: ["head movement", "How fast the head moves, in face widths a second. Under .05 is still. \"in\" or \"out\": coming closer or moving away."],
    jaw: ["jaw", "Whether the jaw moves as in speech (.035 or more) or is still."],
    blinks: ["blinks", "Blinks a minute, over the last minute. Most people blink 10 to 20 times a minute: fewer while concentrating, more when tired."],
    "gaze away": ["gaze away", "How much of the last 5 seconds the eyes looked away from the camera."],
    pulse: ["pulse", "Heart rate estimated from the skin's colour, in beats a minute, read only while the face is still. \"?\": weak. Not a medical reading."],
    "smile rise": ["smile rise", "How quickly the latest smile came on (shown when it is quick)."],
    "smile asym": ["smile asymmetry", "How lopsided the smiles of the last 30 seconds were: 0 even, 1 one side only."],
    held: ["held", "How long the expression shown has lasted."],
  },
  "Voice": {
    speaking: ["speaking", "How much of the last 3 seconds had this person's voice in it."],
    loudness: ["loudness", "How far the voice is above the room's own noise, in dB. Most speech reads 10 to 25."],
    pitch: ["pitch", "How high the voice is, in Hz. Each person has their own: only a change from theirs says anything."],
    "pitch range": ["pitch range", "How much the pitch goes up and down, in semitones. Under 2 is flat; 3 to 6 is ordinary lively speech."],
    rate: ["speech rate", "About syllables a second. Ordinary speech is 3 to 6."],
    arousal: ["voice arousal", "How energetic the voice sounds, about 0 to 1. Most speech reads .45 to .75."],
    valence: ["voice valence", "How pleasant the voice sounds, about 0 to 1. Most speech reads .2 to .4."],
  },
  "Body": {
    head: ["head height", "How high the head is above the shoulders, in shoulder widths. Lower: head down or slumped."],
    neck: ["neck", "How far the ears are above the shoulders, in shoulder widths. Smaller: shoulders raised, as in a shrug or tension."],
    tilt: ["shoulder tilt", "How far the shoulders lean from level, in degrees."],
    lean: ["leaning", "Toward the camera (in), away from it (back), or upright."],
    movement: ["body movement", "How much the shoulders and hands move, in shoulder widths a second."],
    breathing: ["breathing", "Breaths a minute, from the shoulders rising and falling, read only while the body is still. \"?\": weak. Not a medical reading."],
    "hand on face": ["hand on face", "A wrist or a fingertip over the face. It also hides part of the face from the expression reading."],
    "arms crossed": ["arms crossed", "Both arms folded across the body."],
  },
  "Their normal": {
    now: ["right now", "Whether this person's readings are usual for them (alone or in company), unusual, or still being learned."],
    "most unusual": ["most unusual", "The reading furthest from their usual, in spreads (SD): 2 or more is unusual for them."],
    next: ["next", "The next readings furthest from their usual."],
  },
};
// the signals a person's normal is learned for (signals.js B_KEYS) by their fuller names, for "Their normal"'s
// words ("gazeAway +2.6 SD" is shown "gaze away +2.6 SD")
const NORMAL_NAMES = {
  head: "head height", neck: "neck", tilt: "shoulder tilt", bodyMotion: "body movement", motion: "head movement",
  smile: "smile", jaw: "jaw", blinkRate: "blinks", gazeAway: "gaze away", pitch: "pitch", pitchSd: "pitch range",
  loudness: "loudness", speechRate: "speech rate", voiceArousal: "voice arousal", voiceValence: "voice valence",
  pulse: "pulse", breathing: "breathing",
};
const normalWords = v => v == null ? v : v.replace(/^[A-Za-z]+(?= [+-]?\d)/, k => NORMAL_NAMES[k] || k);
const openGroups = new Set(kept("cams-groups", []));
// a group's cells as [label, value, hot, wide], whatever else a device sent
const cellsOf = cells => (Array.isArray(cells) ? cells : []).filter(Array.isArray).map(([k, v, hot, wide]) =>
  [String(k ?? ""), v == null ? null : String(v), !!hot, !!wide]);
// the parts of docs/READINGS.md each group and reading of a readout is explained in (readings.rs holds every one to
// the page): a group by its title, a reading by its group and its name
const READ_AT = {
  "Expression": "expression", "Face": "face", "Voice": "voice", "Body": "body-and-hands", "Their normal": "their-normal",
  "Expression/votes": "votes", "Expression/share": "share-of-votes", "Expression/lead": "lead",
  "Expression/valence": "valence-face", "Expression/valence (fit)": "valence-face", "Expression/arousal": "arousal-face",
  "Expression/arousal (fit)": "arousal-face", "Expression/calibrated": "calibrated-confidence",
  "Expression/state model": "state-model", "Expression/usual look": "their-usual-look",
  "Expression/flags": "why-it-isnt-clear", "Expression/cues": "cues-for-this-look",
  "Face/talking": "talking", "Face/moving": "moving", "Face/jaw": "jaw", "Face/blinks": "blinks", "Face/gaze away": "gaze-away",
  "Face/pulse": "pulse", "Face/smile rise": "smile-rise", "Face/smile asym": "smile-asymmetry", "Face/held": "held",
  "Voice/speaking": "speaking", "Voice/loudness": "loudness", "Voice/pitch": "pitch", "Voice/pitch range": "pitch-range",
  "Voice/rate": "speech-rate", "Voice/arousal": "voice-arousal", "Voice/valence": "voice-valence",
  "Body/head": "head-height", "Body/neck": "neck", "Body/tilt": "shoulder-tilt", "Body/lean": "leaning",
  "Body/movement": "body-movement", "Body/breathing": "breathing", "Body/hand on face": "hand-on-face",
  "Body/arms crossed": "arms-crossed",
  "Their normal/now": "their-usual", "Their normal/most unusual": "most-unusual", "Their normal/next": "most-unusual",
};
function cellEl(group, [k, v, hot, wide]) {
  const [name, about] = ABOUT[group]?.[k] || [k, ""], part = READ_AT[`${group}/${k}`];
  wide ||= group === "Their normal";                     // its words ("unusual (company)", "head movement +2.6 SD") take a row
  const label = part ? el("button", { type: "button", className: "rdn", textContent: name }) : el("span", { textContent: name });
  if (part) { label.dataset.k = part; label.onclick = () => openReadings(part); }
  return el("div", { className: "cell" + (wide ? " wide" : ""), title: (about ? `${name}: ${about}` : name) + (part ? " (Click the name for more.)" : "") },
    label, el("b", { className: v == null ? "na" : hot ? "hot" : "", textContent: v == null ? "–" : v }));
}
// a group's "what these mean"
function readMore(part) {
  const b = el("button", { type: "button", className: "rdmore", textContent: "What these mean" });
  b.dataset.k = part;
  b.onclick = () => openReadings(part);
  return b;
}
// A camera's faces, changed in place: the readout comes every second, and a block made anew each time would take a
// click from under the pointer (the button pressed is gone before it is let go) and the keyboard's place with it.
// What is the same element in the same place is kept, its words, classes and titles brought up to date; anything
// else is put in anew.
function morph(old, nu) {
  if (old.nodeType !== nu.nodeType || old.nodeName !== nu.nodeName || old.childNodes.length !== nu.childNodes.length
      || (old.dataset?.k ?? "") !== (nu.dataset?.k ?? "") || (old.dataset?.g ?? "") !== (nu.dataset?.g ?? "")) { old.replaceWith(nu); return; }
  if (old.nodeType === 3) { if (old.nodeValue !== nu.nodeValue) old.nodeValue = nu.nodeValue; return; }
  for (const a of ["class", "title", "style", "aria-expanded"]) {
    const v = nu.getAttribute(a);
    if (v == null) old.removeAttribute(a); else if (old.getAttribute(a) !== v) old.setAttribute(a, v);
  }
  const a = [...old.childNodes], b = [...nu.childNodes];
  a.forEach((o, i) => morph(o, b[i]));
}
function setFaces(list, blocks) {
  if (list.children.length !== blocks.length) { list.replaceChildren(...blocks); return; }
  [...list.children].forEach((o, i) => morph(o, blocks[i]));
}
function faceBlock(f) {
  const unk = f.name === "Unknown";
  const li = el("li", { className: "face" + (f.lost ? " lost" : "") },
    el("div", { className: "fhead" }, el("span", { className: "sw", style: `background:${/^#[0-9a-f]{3,8}$|^hsl\([\d.,% ]+\)$/i.test(f.color) ? f.color : "#888"}` }),
       el("span", { className: "who" + (unk ? " unk" : ""), textContent: String(f.name ?? "") }),
       el("span", { className: "match", textContent: f.sim == null ? "" : `${f.sim}%`, title: "Match: how alike this face is to the prints of the person it is named as" })));
  li.dataset.k = "face:" + String(f.name ?? "");          // another person in this place is a block of its own
  if (f.sim != null) li.append(el("div", { className: "meter" }, el("i", { style: `width:${Math.max(0, Math.min(100, +f.sim || 0))}%` })));
  li.append(el("div", { className: "fsub" }, el("span", { className: f.exprState === "change" ? "change" : "", textContent: String(f.expr || ""),
                                                          title: f.exprState === "change" ? "A change from this person's usual look" : "How the face looks" }),
                                             el("span", { textContent: `${+f.size || 0} px${f.lost ? " · lost" : ""}`, title: "Face size in the picture" })));
  const groups = (Array.isArray(f.groups) ? f.groups : []).filter(Array.isArray)
    .map(([title, note, cells]) => ({ title: String(title ?? ""), note: note ? String(note) : "", cells: cellsOf(cells) }));
  for (const g of groups) if (g.title === "Their normal") g.cells = g.cells.map(([k, v, hot, wide]) => [k, normalWords(v), hot, wide]);
  const cell = (g, k) => groups.find(x => x.title === g)?.cells.find(c => c[0] === k) || null;
  const val = (g, k) => cell(g, k)?.[1] ?? null;
  // what stands out now
  const chips = [];
  const chip = (text, cls = "", title = "") => chips.push(el("span", { className: "chip " + cls, textContent: text, title }));
  const now = val("Their normal", "now");
  if (now?.startsWith("unusual")) {
    const most = val("Their normal", "most unusual");
    chip(most ? `unusual for them: ${most}` : "unusual for them", "hot", "Several of their readings are 2 or more spreads from their own usual");
  } else if (now?.startsWith("usual")) chip("usual for them", "ok", "Their readings are within their own usual range");
  else if (now?.startsWith("learning")) chip(`learning their normal ${now.slice(9)}`.trim(), "muted", "Not enough of this person has been seen yet to know their usual");
  if (val("Face", "talking") === "yes") chip("talking", "", ABOUT.Face.talking[1]);
  if (cell("Face", "gaze away")?.[2]) chip(`looking away ${val("Face", "gaze away")}`, "hot", ABOUT.Face["gaze away"][1]);
  if (val("Body", "hand on face") === "yes") chip("hand on face", "hot", ABOUT.Body["hand on face"][1]);
  if (val("Body", "arms crossed") === "yes") chip("arms crossed", "", ABOUT.Body["arms crossed"][1]);
  const lean = val("Body", "lean");
  if (lean === "in" || lean === "back") chip(`leaning ${lean}`, "", ABOUT.Body.lean[1]);
  if (chips.length) li.append(el("div", { className: "chips" }, ...chips));
  // the key numbers, where there are readings
  const keys = [["Face", "pulse"], ["Body", "breathing"], ["Face", "blinks"], ["Voice", "speaking"], ["Voice", "pitch"], ["Voice", "loudness"]]
    .map(([g, k]) => [g, cell(g, k)]).filter(([, c]) => c && c[1] != null).slice(0, 6);
  if (keys.length) li.append(el("div", { className: "keys" }, ...keys.map(([g, c]) => cellEl(g, [c[0], c[1], c[2], false]))));
  // every group, one by one: in the grid each opens at its heading (the same group under every face at once); with
  // the camera looked at alone (Focus) every group is open, side by side, as the full view of the readings. Which it
  // is, is the page's style alone (.tile.solo), so the readouts are made the same way whatever is shown.
  const all = el("div", { className: "groups" });
  for (const g of groups) {
    const shown = g.cells.filter(c => c[1] != null), hot = g.cells.filter(c => c[2] && c[1] != null).length;
    const preview = shown.slice(0, 3).map(([k, v]) => `${ABOUT[g.title]?.[k]?.[0] || k} ${v}`).join(" · ");
    const open = openGroups.has(g.title);
    const head = el("button", { type: "button", className: "gh", title: GROUP_ABOUT[g.title] || g.title },
      el("span", { className: "gl", textContent: g.title }, g.note ? el("em", { textContent: " " + g.note }) : null),
      hot ? el("span", { className: "gp hot", textContent: `${hot} to note` }) : null,
      el("span", { className: "gp pre", textContent: shown.length ? preview : "no readings yet" }));
    head.setAttribute("aria-expanded", String(open));
    const body = el("div", { className: "gbody" },
      GROUP_ABOUT[g.title] ? el("p", { className: "gabout" }, GROUP_ABOUT[g.title], READ_AT[g.title] ? " " : null,
        READ_AT[g.title] ? readMore(READ_AT[g.title]) : null) : null,
      el("div", { className: "cells" }, ...g.cells.map(c => cellEl(g.title, c))));
    const box = el("div", { className: "grp" + (open ? "" : " shut") }, head, body);
    box.dataset.g = g.title;
    head.onclick = () => {
      const now = !openGroups.has(g.title);
      if (now) openGroups.add(g.title); else openGroups.delete(g.title);
      keep("cams-groups", [...openGroups]);
      for (const o of document.querySelectorAll(".grp")) if (o.dataset.g === g.title) {
        o.classList.toggle("shut", !now);
        o.querySelector(".gh")?.setAttribute("aria-expanded", String(now));
      }
    };
    all.append(box);
  }
  if (groups.length) li.append(all);
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
      setFaces(t.faces, faces.length ? faces.map(faceBlock) : [el("li", { className: "none", textContent: r.error ? "Readout unavailable" : "No faces in view" })]);
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
// Esc closes the list of cameras first, then leaves the camera looked at alone
addEventListener("keydown", e => {
  if (e.key !== "Escape") return;
  if (!$("readingsSheet").hidden) closeReadings();
  else if (menuOpen()) { setMenu(false); $("camBtn").focus(); } else if (solo) focusOn(null);
});
// What each reading means (this server's own page, /readings, from docs/READINGS.md), in a sheet over the page:
// every camera goes on behind it. From the header's link, a reading's name in a readout, a group's "what these
// mean", or a name in a Signals chart (its "readings" event).
const PART = /^[a-z0-9-]{1,60}$/;
function openReadings(part) {
  $("readingsFrame").src = "/readings" + (PART.test(part || "") ? "#" + part : "");
  $("readingsSheet").hidden = false;
  $("readingsClose").focus();
}
function closeReadings() { $("readingsSheet").hidden = true; $("readingsFrame").removeAttribute("src"); }
$("readingsBtn").onclick = () => openReadings("");
$("readingsClose").onclick = closeReadings;
document.addEventListener("readings", e => openReadings(e.detail));
setInterval(loadEvents, 3000);
setInterval(loadDevices, 10000);
loadDevices();
loadEvents();
