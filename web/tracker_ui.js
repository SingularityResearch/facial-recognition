// tracker_ui.js - the script of the live tracker's page (tracker_ui.html, served by the tracker).
// First: this device's settings and the people's learned signals from the server into localStorage (page_store.js).
const PAGE = window.PageStore ? PageStore.restore("track", "ft-") : null;
const $ = id => document.getElementById(id);
const esc = s => String(s ?? "").replace(/[&<>"]/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
const video = $("video"), canvas = $("overlay"), ctx = canvas.getContext("2d");
const COLORS = ["#34d399", "#60a5fa", "#f472b6", "#fbbf24", "#a78bfa", "#2dd4bf", "#fb923c", "#a3e635"];
const UNKNOWN = getComputedStyle(document.documentElement).getPropertyValue("--unknown").trim() || "#e0574a";
const SEND_MAX = 960;                      // frames are shrunk to this before being sent for recognition
const grab = document.createElement("canvas"), gctx = grab.getContext("2d");
// With the face mesh on, the page shows the frame that was analysed (not the live video) so the
// mesh sits exactly on the face: the live video runs one round trip ahead of any result, and a
// tight mesh makes that lag obvious. Two full-size canvases: one being sent, one on screen.
const holds = [document.createElement("canvas"), document.createElement("canvas")];
let shown = null;                          // the analysed frame on screen, or null to show the live video

let stream = null, running = false, busy = false, tracks = [], people = [];
let expressions = null;                    // label list from the recognizer, null when its expression model isn't installed
let cuesOn = false;                        // the recognizer can send face-shape cues (MediaPipe installed)
let test = null;                           // running expression test: { session, label, talk, prompt }
let calibration = null, vaFit = null;      // fitted on posed sessions and feeling reports (data/expr_calibration.json, data/va_fit.json)
let voiceModel = false;                    // the tracker has the voice model (models/melos_1.onnx)
let poseOn = false;                        // the recognizer can send body points (MediaPipe pose model installed)
let handsOn = false;                       // the recognizer can send finger points (MediaPipe hand model installed)
let meshTris = null;                       // face-mesh triangles (landmark indices, three per triangle) when MediaPipe is installed
// MediaPipe's parts are this page's: the server names a module for them (`parts` in /api/people), which runs
// them beside the page and puts into each answer of the server what they found.
let parts = null, partsAsked = false;
async function loadParts(info) {
  try {
    if (typeof info.script !== "string" || !/^\/[a-z_/]+\.js$/.test(info.script)) throw new Error("no module named");
    // beside the page, in workers, where the module can (loadAny: the page itself where it can't)
    const module = await import(info.script);
    parts = await (module.loadAny || module.load)(info);
    // Where the server says these parts cost the device its frames (a phone: a face's cues take a tenth of a
    // second there), the evidence that asks for them with every frame starts switched off. Once the owner has
    // switched it either way, that is what counts.
    if (info.costly && recall("o-optEvidence") === null) { $("optEvidence").checked = false; renderList(); }
    loadPeople();
  } catch (e) { pageNote(`MediaPipe's parts didn't start in the page: ${e.message || e}`); }
}
let qEnroll = 0.3, captured = 0;           // "Collect faces": the quality a face needs to be learned from; snapshots saved
const CAP_EVERY = 5000, CAP_MAX = 12, CAP_SEEN = 3;   // per face: ms between snapshots, most per visit, checks seen first
let threshTouched = false;                 // the strictness slider was moved, so don't reset it to the model's default
let checks = 0, fpsT = performance.now(), lastErr = 0;
let checkMs = null;                        // for the Signals chart: how long the last check took,
const checkTimes = [];                     // and when each check of the last second ended

function setStatus(text, cls = "") { $("statusText").textContent = text; $("status").className = "status " + cls; }
const colorFor = name => name === "Unknown" ? UNKNOWN : COLORS[[...name].reduce((h, c) => (h * 31 + c.charCodeAt(0)) >>> 0, 7) % COLORS.length];

async function loadPeople() {
  try {
    const r = await (await fetch("/api/people")).json();
    if (!Array.isArray(r.people)) throw new Error(r.error || "no list of people");   // an error answer: keep the list we have
    people = r.people;
    setExpressions(r.expressions || null);
    if (r.parts && !partsAsked) { partsAsked = true; loadParts(r.parts); }
    if (parts && r.devices) Object.assign(r.devices, parts.devices);
    cuesOn = !!r.cues || !!parts?.cues; showDevices(r.devices, r.warmup_ms);
    // where each model runs, as one line for the Signals chart's Device section
    const runOn = new Map();
    for (const [job, at] of Object.entries(r.devices || {})) if (typeof at === "string" && at) runOn.set(at, [...(runOn.get(at) || []), job]);
    modelsRun = [...runOn].map(([at, jobs]) => `${jobs.join(", ")} on the ${at}`).join("; ").slice(0, 240);
    calibration = r.calibration || null; vaFit = r.vaFit || null; voiceModel = !!r.voice;
    allSignals = r.signals === "all";
    if (r.location === "on" && !locationAsked) locationOn = locationAsked = true;
    if (r.history === "on" && !keepAsked) keepOn = keepAsked = true;
    streamAsked = ["transfer", "page"].includes(r.stream) ? r.stream : "";
    forgetForgotten(r.forgotten);
    // a server with a page of its own for joining a shared database says where it is (a phone's app has no Settings tab)
    if (typeof r.join === "string" && /^\/[a-z_]+$/.test(r.join)) {
      $("joinLink").href = r.join; $("joinLink").hidden = false;
      // a device that is in a shared database lets others in; one that isn't joins
      $("joinLink").textContent = r.joined ? "Join a new device…" : "Join a shared database…";
    }
    poseOn = !!r.pose || !!parts?.pose; $("optPose").disabled = $("optHandGate").disabled = $("optSkeleton").disabled = !poseOn;
    handsOn = !!r.hands || !!parts?.hands; $("optFingers").disabled = !handsOn;
    meshTris = r.meshTriangles || parts?.meshTriangles || null; $("optMesh").disabled = !meshTris;
    if (r.qEnroll != null) qEnroll = r.qEnroll;
    if (r.captureDir) $("optCollect").parentElement.title = `Saves snapshots to ${r.captureDir} for the labeling tool`;
    $("testCount").textContent = [r.sessions ? `${r.sessions} tests` : "", r.reports ? `${r.reports} reports` : ""].filter(Boolean).join(" · ");
    if (r.threshold) $("optStrict").defaultValue = r.threshold;   // "Reset to defaults" in the app's Settings tab
    if (!threshTouched && r.threshold) {         // start the slider at the active model's measured setting
      $("optStrict").value = r.threshold; $("threshVal").textContent = (+r.threshold).toFixed(2);
    }
    const where = r.device ? ` · ${r.device.split(" ")[0]}` : "";
    if (!running) setStatus(people.length ? `Ready · knows ${people.length} people${where}` : "Ready · nobody named yet (use the labeling tool)");
    renderPeople();
    $("againBtn").hidden = true;
    return true;
  } catch { setStatus(startAgain("The face tracker isn't answering"), "err"); $("againBtn").hidden = false; return false; }
}

// ---------------------------------------------------------------- alerts and presence log
// A visit is one person's time in view: it starts when they're named (from the averaged prints, after
// 3 checks, so a passing misread doesn't count) and ends when they've been out of view for VISIT_GAP.
// Finished visits go to the opt-in presence log. The chosen people's events (arrive, leave, unusual signals) and
// unknown faces staying raise alerts, each switched on, and timed, in the alert settings.
const VISIT_GAP = 60_000;
const visitsOpen = {}, lastLeft = {};       // name -> {start, last} (Date.now ms); name -> when they last left
const leaving = {}, lastUnusual = {};       // name -> {start, last} of a visit that ended; name -> last unusual alert
const sec = id => (+$(id).value || 0) * 1000;  // an alert timing setting, in ms
function alertNames() { return new Set($("alertWho").value.split(",").map(s => s.trim().toLowerCase()).filter(Boolean)); }
// An alert: a banner on the video (it stays 30 s, or until dismissed), the bell rings, a sound, a Windows
// notification if allowed, and it's kept in the alert inbox until you acknowledge or archive it. In the
// app window the tab bar's bell shows it too, whichever tab is open (app_ui.js).
// In the app window its tab bar has the bell and the banner (app_ui.js), over every tab: this page then shows no
// bell or banner of its own, so an alert shows once. Opened on its own, the page shows both.
const IN_APP = window.parent !== window;
if (IN_APP) $("bellBtn").hidden = true;
async function alertMe(text, kind = "arrived", name = "", track = null) {
  announce(text);
  let details = null;
  try { details = track ? alertDetails(track) : null; } catch (e) { details = { error: String(e.message || e).slice(0, 200) }; }
  return postAlert(text, kind, name, details, track);
}
// The banner, bell, sound and notification of an alert (alertMe stores it too; the app's own alerts are stored already)
function announce(text) {
  let el = $("toast");
  if (IN_APP) el = { hidden: true, firstChild: {} };       // the app window's banner shows it instead
  else if (!el) {
    el = document.createElement("div"); el.id = "toast"; el.className = "toast"; el.setAttribute("role", "alert");
    const msg = document.createElement("span"), view = document.createElement("button"), ok = document.createElement("button");
    view.textContent = "View alerts"; ok.textContent = "Dismiss"; ok.setAttribute("aria-label", "Dismiss this banner");
    view.onclick = () => { el.hidden = true; openAlerts(); };
    ok.onclick = () => { el.hidden = true; };
    el.append(msg, view, ok); $("stage").appendChild(el);
  }
  el.firstChild.textContent = text; el.hidden = false;
  clearTimeout(alertMe.timer);
  if (sec("optBannerFor")) alertMe.timer = setTimeout(() => { el.hidden = true; }, sec("optBannerFor"));
  if (!IN_APP) { $("bellBtn").classList.remove("ring"); void $("bellBtn").offsetWidth; $("bellBtn").classList.add("ring"); }
  if ($("optAlertSound").checked) beep();
  try { if (window.Notification && Notification.permission === "granted") new Notification("Face tracker", { body: text, silent: true }); } catch {}
}
async function postAlert(text, kind, name, details, track) {
  try {
    const r = await alertApi("/api/alerts", { kind, name, text, t: Date.now(), details });
    showAlertCounts(r.counts, text);
    if (!$("alertsPanel").hidden) loadAlerts();
    // measure again later (optDetailsAfter): pulse, breathing, blink rate and the 30-second averages need time to fill in
    if (track) setTimeout(async () => {
      if (!tracks.includes(track) || track.missed) return;
      try {
        await alertApi("/api/alerts/details", { id: r.alert.id, which: "later",
                                                details: { ...alertDetails(track), afterSeconds: sec("optDetailsAfter") / 1000 } });
        if (!$("alertsPanel").hidden) loadAlerts();
      } catch {}
    }, sec("optDetailsAfter"));
  } catch { showAlertCounts(null, text); }
}

// What the tracker measured about this face when the alert was raised: numbers and short labels only (no picture,
// no sound), the same signals the page already computes (signals.js, expression.js), each beside the person's normal.
function alertDetails(t) {
  const now = performance.now(), v = voiceFor(t, now), named = t.name && t.name !== "Unknown";
  const snap = t.fs.snapshot(now, v), sum = t.fs.summary(now, v);
  const z = named ? baselines.z(t.name, snap, t.ctx) : {};
  const vsNormal = {};
  for (const [k, x] of Object.entries(snap)) {
    const b = named && x != null ? baselines.normal(t.name, k, t.ctx) : null;
    vsNormal[k] = { now: x, normal: b ? b.m : null, spread: b ? b.sd : null, z: z[k] ?? null };
  }
  const shares = {}, last30s = {};
  for (const [k, x] of Object.entries(sum)) (k.startsWith("p_") ? shares : last30s)[k.replace(/^p_/, "")] = x ?? null;
  // The evidence view, exactly as the tracker shows it under the face with "Show expression evidence" on (kept
  // whether or not that's switched on), plus the expression reading behind it as numbers
  const e = t.expr, line = s => String(s || "").slice(0, 500);
  const evidence = {
    expression: line(e ? Expression.evidence(e) : t.xt?.skip ? `no reading: ${t.xt.skip}` : "no reading yet"),
    signals: line(Signals.evidence(t.fs, now, v, t.talking)),
    body: line(Signals.bodyEvidence(t.fs, now)),
    unusual: line(t.snap ? Signals.unusualEvidence(baselines, t.name, t.snap, t.ctx) : ""),
    devices: line($("devices")?.textContent.replace(/\s+/g, " ").trim()),
  };
  const va = e ? (e.vaFit || e.va) : null;
  const reading = e ? {
    label: e.label, state: e.state, share: e.share, lead: e.lead, votes: e.votes, calibratedConfidence: e.cal ?? null,
    valence: va ? va[0] : null, arousal: va ? va[1] : null, valenceArousalFrom: e.vaFit ? "your fitted model" : "textbook placements",
    stateModelSays: e.belief ? e.belief.label : null, stateModelShare: e.belief ? e.belief.p : null,
    flags: (e.flags || []).join(", ") || null, usualLookLearned: e.learning ?? null,
  } : null;
  const latest = t.xt?.votes?.length ? t.xt.votes[t.xt.votes.length - 1].probs : null;
  const asLabels = arr => arr && expressions ? Object.fromEntries(expressions.map((l, i) => [l, arr[i] ?? null])) : null;
  let usual = null;
  try { usual = named && usualLooks.usual(t.name); } catch {}
  return {
    evidence,
    expressionReading: reading,
    expressionCues: e?.cues || null,
    latestFrameScores: asLabels(latest),
    usualLook: asLabels(usual || null),
    recognition: { similarity: t.sim ?? null, faceQuality: t.quality ?? null, faceWidthPx: Math.round(t.tw || t.w || 0),
                   namedFromAveragedPrints: !!t.byAverage, setting: t.ctx || null,
                   facesInView: tracks.filter(x => !x.missed).length },
    expression: t.expr ? { reading: Expression.text(t.expr), state: t.expr.state, label: t.expr.label,
                           evidence: Expression.evidence(t.expr).slice(0, 200) }
                       : { reading: t.xt?.skip ? `no reading: ${t.xt.skip}` : "no reading yet" },
    expressionShares30s: shares,
    vsNormal,
    last30s,
    sensors: { camera: !!running, microphone: !!voice, voiceUsedForThisFace: !!v, faceShapeCues: !!(cuesOn && needCues()),
               bodyPosture: $("optPose").checked, normalLearned: named ? baselines.progress(t.name) : null },
  };
}

// ---------------------------------------------------------------- the alert inbox
let alertView = "new", alertCounts = { new: 0, ack: 0, archived: 0 };
// An answer nobody reads has to be let go. Until then its body keeps a file descriptor open in the page's process,
// and nothing frees it while the page runs. On Linux (Qt WebEngine, 1024 descriptors) the unread answers of the
// every-second requests used them all in ten minutes and the window froze; the watchdog then restarted the app,
// about every 90 minutes after that. drop(answer) lets the body go; a test checks every fetch reads or drops.
const drop = r => { try { r?.body?.cancel().catch(() => {}); } catch {} return r; };
async function alertApi(path, body) {
  const r = await fetch(path, body === undefined ? { cache: "no-store" }
    : { method: "POST", body: JSON.stringify(body), headers: { "Content-Type": "application/json", "X-Face-Tool": "1" } });
  const j = await r.json();
  if (!r.ok) throw new Error(j.error || `HTTP ${r.status}`);
  return j;
}
// The bell's count, and the app window's tab bar told (latest: the text of an alert that just happened)
function showAlertCounts(c, latest = null) {
  if (c) alertCounts = c;
  const n = alertCounts.new || 0;
  $("bellCount").textContent = n; $("bellCount").hidden = !n;
  $("bellBtn").classList.toggle("has", n > 0);
  $("bellBtn").setAttribute("aria-label", n ? `Alerts, ${n} new` : "Alerts");
  for (const s of ["new", "ack", "archived"]) $("cnt-" + s).textContent = alertCounts[s] ? `(${alertCounts[s]})` : "";
  SettingsBridge.notify({ fs: "alerts", unread: n, latest, banner: +$("optBannerFor").value });
}
async function loadAlerts() {
  try {
    const r = await alertApi("/api/alerts");
    showAlertCounts(r.counts);
    renderAlerts(r.alerts);
  } catch (e) { $("alertItems").innerHTML = `<li class="none">Couldn't read the alerts: ${esc(e.message)}</li>`; }
}
const BULK = { new: ["Acknowledge all", "ack"], ack: ["Archive all", "archived"], archived: ["Delete all archived", "delete"] };
const ROW_ACTS = { new: [["Acknowledge", "ack"], ["Archive", "archived"]], ack: [["Archive", "archived"], ["Mark new", "new"]],
                   archived: [["Restore", "ack"], ["Delete", "delete"]] };
// ---- an alert's details: what was measured, laid out as tables
const SIGNAL_NAMES = { head: "Head height", neck: "Neck angle", tilt: "Shoulder tilt", bodyMotion: "Body movement",
  motion: "Face movement", smile: "Smile", jaw: "Jaw open", blinkRate: "Blink rate", gazeAway: "Looking away",
  pitch: "Voice pitch", pitchSd: "Pitch variation", loudness: "Loudness", speechRate: "Speaking rate",
  voiceArousal: "Voice arousal", voiceValence: "Voice valence", voiceDominance: "Voice dominance", pulse: "Pulse",
  breathing: "Breathing", cheek: "Cheek raise", frown: "Frown", browDown: "Brows down", browInnerUp: "Inner brows up",
  eyeWide: "Eyes wide", approach: "Coming closer", talking: "Talking", smileAsym: "Smile asymmetry",
  speaking: "Voiced share", headHeight: "Head height", shoulderTilt: "Shoulder tilt", handOnFace: "Hand on face",
  armsCrossed: "Arms crossed", similarity: "Match to their face prints", faceQuality: "Face picture quality",
  faceWidthPx: "Face size", namedFromAveragedPrints: "Named from averaged prints", setting: "Alone or with others",
  facesInView: "Faces in view", camera: "Camera", microphone: "Microphone", voiceUsedForThisFace: "Voice counted for this face",
  faceShapeCues: "Face-shape cues", bodyPosture: "Body posture", normalLearned: "Their normal learned",
  reading: "Reading", state: "State", label: "Label", evidence: "Evidence" };
const UNITS = { pulse: " bpm", breathing: " /min", blinkRate: " /min", pitch: " Hz", loudness: " dB", faceWidthPx: " px",
  speechRate: " /s" };
const SHARES = new Set(["gazeAway", "talking", "speaking", "handOnFace", "armsCrossed", "similarity", "faceQuality",
  "normalLearned", "share", "lead", "calibratedConfidence", "stateModelShare", "usualLookLearned"]);
Object.assign(SIGNAL_NAMES, { share: "Share of the votes", lead: "Lead over the next label", votes: "Votes (frames)",
  calibratedConfidence: "Calibrated confidence", valence: "Valence", arousal: "Arousal", valenceArousalFrom: "Valence/arousal from",
  stateModelSays: "State model says", stateModelShare: "State model share", flags: "Why it isn't clear",
  usualLookLearned: "Their usual look learned" });
const EVIDENCE_NAMES = { expression: "Expression", signals: "Signals", body: "Body", unusual: "Unusual for them",
  devices: "Running on" };
function barList(o, cls = "") {
  const rows = Object.entries(o || {}).filter(([, v]) => v != null).sort((a, b) => b[1] - a[1]);
  return rows.length ? `<div class="bars ${cls}">${rows.map(([k, v]) => `<div><span>${esc(k)}</span><i style="width:${Math.max(0, Math.min(100, Math.round(v * 100)))}%"></i><b>${Math.round(v * 100)}%</b></div>`).join("")}</div>` : "";
}
const GROUPS = [["Face", ["smile", "cheek", "frown", "browDown", "browInnerUp", "eyeWide", "jaw", "smileAsym", "blinkRate", "gazeAway", "talking"]],
  ["Voice", ["speaking", "loudness", "pitch", "pitchSd", "speechRate", "voiceArousal", "voiceValence", "voiceDominance"]],
  ["Body", ["motion", "approach", "head", "headHeight", "neck", "tilt", "shoulderTilt", "bodyMotion", "handOnFace", "armsCrossed"]],
  ["Body rhythms (from the camera)", ["pulse", "breathing"]]];
function fmtValue(k, x) {
  if (x == null) return '<span class="na">—</span>';
  if (typeof x === "boolean") return x ? "yes" : "no";
  if (typeof x === "string") return esc(x);
  if (SHARES.has(k)) return `${Math.round(x * 100)}%`;
  const a = Math.abs(x), s = a >= 100 ? x.toFixed(0) : a >= 10 ? x.toFixed(1) : x.toFixed(2);
  return esc(s + (UNITS[k] || ""));
}
const nameOf = k => SIGNAL_NAMES[k] || k.replace(/([A-Z])/g, " $1").replace(/^./, c => c.toUpperCase());
function kvTable(o) {
  return `<table class="kv">${Object.entries(o || {}).map(([k, v]) => `<tr><th>${esc(nameOf(k))}</th><td>${fmtValue(k, v)}</td></tr>`).join("")}</table>`;
}
function detailSet(d) {
  if (!d) return "";
  if (d.error) return `<p class="na">Couldn't read the signals: ${esc(d.error)}</p>`;
  const shares = Object.entries(d.expressionShares30s || {}).filter(([, v]) => v != null).sort((a, b) => b[1] - a[1]);
  const vs = Object.entries(d.vsNormal || {}).filter(([, r]) => r && r.now != null);
  const zcell = z => z == null ? '<span class="na">learning</span>'
    : `<span class="${Math.abs(z) >= 2 ? "far" : ""}">${z >= 0 ? "+" : ""}${z.toFixed(1)}</span>`;
  const l30 = d.last30s || {};
  const ev = Object.entries(d.evidence || {}).filter(([, s]) => s);
  return `${ev.length ? `<h4>Evidence (as the tracker shows it)</h4><ul class="ev">${ev.map(([k, s]) =>
      `<li><b>${esc(EVIDENCE_NAMES[k] || k)}</b> ${esc(s)}</li>`).join("")}</ul>` : ""}
    <h4>Recognition</h4>${kvTable(d.recognition)}
    <h4>Expression</h4>${kvTable(d.expression)}
    ${d.expressionReading ? kvTable(d.expressionReading) : ""}
    ${d.latestFrameScores ? `<h5>Latest frame's scores</h5>${barList(d.latestFrameScores)}` : ""}
    ${d.usualLook ? `<h5>Their usual look (what the reading is compared with)</h5>${barList(d.usualLook, "dim")}` : ""}
    ${d.expressionCues ? `<h5>Face-shape cues for this label</h5>${kvTable(d.expressionCues)}` : ""}
    ${shares.length ? `<div class="bars">${shares.map(([k, v]) => `<div><span>${esc(k)}</span><i style="width:${Math.round(v * 100)}%"></i><b>${Math.round(v * 100)}%</b></div>`).join("")}</div>
      <p class="hint">Expression shares: the recognizer's scores averaged over the last 30 seconds.</p>` : ""}
    <h4>Now, against their normal</h4>
    ${vs.length ? `<table class="vs"><tr><th>Signal</th><th>Now</th><th>Their normal</th><th>Spreads away</th></tr>
      ${vs.map(([k, r]) => `<tr><td>${esc(nameOf(k))}</td><td>${fmtValue(k, r.now)}</td><td>${r.normal == null ? '<span class="na">—</span>' : fmtValue(k, r.normal)}</td><td>${zcell(r.z)}</td></tr>`).join("")}</table>
      <p class="hint">"Spreads away": how unusual this is for them (2 or more is marked); "learning" until their normal for it is learned.</p>`
      : '<p class="na">No signals yet (they need a few seconds of the face in view).</p>'}
    <h4>Last 30 seconds (averages)</h4>
    ${GROUPS.map(([g, keys]) => { const have = keys.filter(k => k in l30); return have.length
      ? `<h5>${esc(g)}</h5><table class="kv">${have.map(k => `<tr><th>${esc(nameOf(k))}</th><td>${fmtValue(k, l30[k])}</td></tr>`).join("")}</table>` : ""; }).join("")}
    <h4>Sensors</h4>${kvTable(d.sensors)}`;
}
function detailsHtml(a) {
  const d = a.details || {};
  if (!d.first) return '<p class="na">No details were recorded for this alert (raised before details were kept).</p>';
  return `<div class="dsets"><section><h3>When the alert was raised</h3>${detailSet(d.first)}</section>
    <section><h3>${d.later ? `${Math.round(d.later.afterSeconds || 15)} seconds later` : "Later"}</h3>${d.later ? detailSet(d.later) : '<p class="na">Not measured: the face had left the view (or it was a leave alert).</p>'}</section></div>
    <p class="hint">Numbers only: no picture or sound is kept with an alert.</p>`;
}
let alertRows = {}, openDetail = null;

function renderAlerts(all) {
  const when = ms => new Date(ms).toLocaleString([], { weekday: "short", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
  alertRows = Object.fromEntries(all.map(a => [a.id, a]));
  const rows = all.filter(a => a.state === alertView);
  document.querySelectorAll("[data-astate]").forEach(b => b.setAttribute("aria-selected", b.dataset.astate === alertView));
  $("alertItems").innerHTML = rows.length
    ? rows.map(a => `<li><span>${esc(a.text)}</span><span class="when">${esc(when(a.t))} · ${esc({ arrived: "arrived", left: "left", unusual: "unusual signals", unknown: "unknown face", moved: "moved between cameras", joined: "new device" }[a.kind] || a.kind)}</span>
        <span class="acts"><button data-detail="${esc(a.id)}" aria-expanded="${openDetail === a.id}">Details</button>${ROW_ACTS[alertView].map(([t, s]) => `<button data-id="${esc(a.id)}" data-to="${s}">${t}</button>`).join("")}</span>
        ${openDetail === a.id ? `<div class="details">${detailsHtml(a)}</div>` : ""}</li>`).join("")
    : `<li class="none">${{ new: "No new alerts", ack: "Nothing acknowledged", archived: "Nothing archived" }[alertView]}</li>`;
  const [label] = BULK[alertView];
  $("alertsBulk").textContent = label; $("alertsBulk").disabled = !rows.length;
}
async function setAlerts(body) {
  try { showAlertCounts((await alertApi("/api/alerts/state", body)).counts); } catch (e) { setStatus(`Alerts: ${e.message}`, "err"); }
  loadAlerts();
}
function openAlerts(view) {
  if (view) alertView = view;
  else if (!alertCounts.new && $("alertsPanel").hidden) alertView = alertCounts.ack ? "ack" : "new";
  $("alertsPanel").hidden = false; $("bellBtn").setAttribute("aria-expanded", "true");
  try { if (window.Notification && Notification.permission === "default") Notification.requestPermission(); } catch {}
  loadAlerts();
  return { message: "Alerts shown on the Face tracking tab" };
}
function closeAlerts() { $("alertsPanel").hidden = true; $("bellBtn").setAttribute("aria-expanded", "false"); $("bellBtn").focus(); }
$("bellBtn").onclick = () => ($("alertsPanel").hidden ? openAlerts() : closeAlerts());
$("alertsClose").onclick = closeAlerts;
document.querySelectorAll("[data-astate]").forEach(b => b.addEventListener("click", () => { alertView = b.dataset.astate; loadAlerts(); }));
$("alertItems").addEventListener("click", async e => {
  const d = e.target.closest("button[data-detail]");
  if (d) {
    openDetail = openDetail === d.dataset.detail ? null : d.dataset.detail;
    $("alertsPanel").classList.toggle("wide", !!openDetail);        // room for the two sets side by side
    renderAlerts(Object.values(alertRows).sort((a, b) => b.t - a.t));
    return;
  }
  const b = e.target.closest("button[data-id]");
  // askUser (ask.js), not confirm(): that would stop the camera and its stream until it's answered
  if (b && (b.dataset.to !== "delete" || await askUser("Delete this alert for good?", "Delete"))) setAlerts({ ids: [b.dataset.id], state: b.dataset.to });
});
$("alertsBulk").onclick = async () => {
  const where = alertView, [label, to] = BULK[where];          // the list shown when it was asked
  if (to !== "delete" || await askUser(`${label}? This can't be undone.`, "Delete")) setAlerts({ where, state: to });
};
addEventListener("keydown", e => { if (e.key === "Escape" && !$("alertsPanel").hidden) closeAlerts(); });
// The privacy policy is read inside the app (the server's own page, /privacy), in a sheet over this page: the page
// stays loaded behind it, so the camera and its checks go on while it is read.
function closePrivacy() { $("privacySheet").hidden = true; $("privacyFrame").removeAttribute("src"); }
$("privacyLink").onclick = e => {
  e.preventDefault();
  $("privacyFrame").src = "/privacy";
  $("privacySheet").hidden = false;
  $("privacyClose").focus();
};
$("privacyClose").onclick = closePrivacy;
addEventListener("keydown", e => { if (e.key === "Escape" && !$("privacySheet").hidden) closePrivacy(); });
// What each reading means (the server's own page, /readings, from docs/READINGS.md), in a sheet over this page like
// the policy: from its link at the top, or at the part a name in the Signals chart asks for (its "readings" event).
const PART = /^[a-z0-9-]{1,60}$/;
function openReadings(part) {
  $("readingsFrame").src = "/readings" + (PART.test(part || "") ? "#" + part : "");
  $("readingsSheet").hidden = false;
  $("readingsClose").focus();
}
function closeReadings() { $("readingsSheet").hidden = true; $("readingsFrame").removeAttribute("src"); }
$("readingsLink").onclick = e => { e.preventDefault(); openReadings(""); };
$("readingsClose").onclick = closeReadings;
document.addEventListener("readings", e => openReadings(e.detail));
addEventListener("keydown", e => { if (e.key === "Escape" && !$("readingsSheet").hidden) closeReadings(); });
function beep() {
  try {
    const ac = beep.ac ||= new AudioContext(), t = ac.currentTime;
    for (const [f, at] of [[880, 0], [1320, 0.16]]) {
      const o = ac.createOscillator(), g = ac.createGain();
      o.frequency.value = f; g.gain.setValueAtTime(0.0001, t + at);
      g.gain.exponentialRampToValueAtTime(0.25, t + at + 0.02); g.gain.exponentialRampToValueAtTime(0.0001, t + at + 0.14);
      o.connect(g).connect(ac.destination); o.start(t + at); o.stop(t + at + 0.15);
    }
  } catch {}
}
const hm = ms => new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
function trackPresence() {
  const now = Date.now(), want = alertNames();
  for (const t of tracks) {
    if (t.missed || !t.byAverage) continue;
    if (t.name === "Unknown") {
      t.seenSince ||= now;
      const stay = sec("optUnknownStay");
      if ($("optAlertUnknown").checked && !t.unknownAlerted && now - t.seenSince >= stay) {
        t.unknownAlerted = true; alertMe(`An unknown face has been in view for ${Math.round(stay / 1000)} seconds`, "unknown", "", t);
      }
      continue;
    }
    if ((t.prints?.length || 0) < 3) continue;
    const chosen = want.has(t.name.toLowerCase());
    if (chosen) checkUnusual(t, now);
    delete leaving[t.name];                  // back in view: not leaving after all
    const v = visitsOpen[t.name];
    if (v) { v.last = now; continue; }
    visitsOpen[t.name] = { start: now, last: now };
    const away = now - (lastLeft[t.name] ?? -Infinity);
    if (chosen && $("optAlertArrive").checked && away >= sec("optArriveGap"))
      alertMe(`${t.name} arrived · ${hm(now)}`, "arrived", t.name, t);
  }
  for (const [name, v] of Object.entries(visitsOpen)) if (now - v.last > VISIT_GAP) closeVisit(name);
  // "Leave": out of view for the set time (the visit itself ends after VISIT_GAP)
  for (const [name, v] of Object.entries(leaving)) {
    if (now - v.last < sec("optLeaveGap")) continue;
    delete leaving[name];
    if ($("optAlertLeave").checked && want.has(name.toLowerCase()))
      alertMe(`${name} left · last seen ${hm(v.last)} (in view from ${hm(v.start)})`, "left", name, null);
  }
}
// "Unusual": any of a chosen person's signals at least optUnusualZ spreads from their own normal (signals.js
// Baselines) for optUnusualFor, at most one alert a person per optUnusualRepeat. Thresholds are guesses.
function checkUnusual(t, now) {
  if (!$("optAlertUnusual").checked || !t.snap) { t.unusualSince = null; return; }
  const z = baselines.z(t.name, t.snap, t.ctx), lim = +$("optUnusualZ").value;
  const far = Object.entries(z).filter(([, v]) => Math.abs(v) >= lim).sort((a, b) => Math.abs(b[1]) - Math.abs(a[1]));
  if (!far.length) { t.unusualSince = null; return; }
  t.unusualSince ||= now;
  if (now - t.unusualSince < sec("optUnusualFor") || now - (lastUnusual[t.name] ?? -Infinity) < sec("optUnusualRepeat")) return;
  lastUnusual[t.name] = now; t.unusualSince = null;
  const what = far.slice(0, 3).map(([k, v]) => `${nameOf(k).toLowerCase()} ${v > 0 ? "+" : ""}${v.toFixed(1)}`).join(", ");
  alertMe(`${t.name}: unusual for them · ${what}`, "unusual", t.name, t);
}
function closeVisit(name, keepalive = false) {
  const v = visitsOpen[name];
  delete visitsOpen[name]; lastLeft[name] = v.last; leaving[name] = v;
  if (!$("optPresence").checked) return;
  fetch("/api/presence", { method: "POST", keepalive, body: JSON.stringify({ name, start: v.start, end: v.last }),
                           headers: { "Content-Type": "application/json", "X-Face-Tool": "1" } }).then(drop, () => {});
}
async function showPresence() {
  const list = $("presenceList");
  try {
    const r = await (await fetch("/api/presence")).json();
    const hm = ms => new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    const rows = [...r.visits, ...Object.entries(visitsOpen).map(([name, v]) => ({ name, start: v.start, end: v.last, now: true }))];
    list.innerHTML = rows.length
      ? rows.map(v => `<li><span>${esc(v.name)}</span><span class="num">${hm(v.start)}–${v.now ? "now" : hm(v.end)}</span></li>`).join("")
      : `<li class="none">${$("optPresence").checked ? "Nobody logged yet today" : "The presence log is off"}</li>`;
  } catch { list.innerHTML = '<li class="none">Couldn\'t read the log</li>'; }
  list.hidden = false;
}

function renderPeople() {
  const opts = people.map(p => `<option value="${esc(p)}">`).join("");
  if ($("peopleList").innerHTML !== opts) $("peopleList").innerHTML = opts;   // only when the people change
  const here = new Set(tracks.filter(t => t.missed === 0).map(t => t.name));
  $("peopleCount").textContent = people.length ? `${here.size ? here.size + " of " : ""}${people.length}` : "";
  $("people").innerHTML = people.length
    ? people.map(p => `<span class="chip${here.has(p) ? " on" : ""}">${esc(p)}</span>`).join("")
    : '<span class="chip">Nobody named yet</span>';
}

// ---------------------------------------------------------------- cameras
async function listCameras(activeId) {
  try {
    const cams = (await navigator.mediaDevices.enumerateDevices()).filter(d => d.kind === "videoinput");
    const sel = $("camSelect"), keep = activeId || sel.value;
    sel.innerHTML = '<option value="">Default camera</option>' +
      cams.map((c, i) => `<option value="${esc(c.deviceId)}">${esc(c.label || "Camera " + (i + 1))}</option>`).join("");
    if (keep) sel.value = keep;
    camCount = cams.length; showCamSwitch();
  } catch {}
}

// A phone has a camera on each side. It starts with the front one, the one that sees the person holding it (a
// camera chosen before is kept, like on a computer), and a small round button on the picture goes to the other
// one. The camera in use says which side it is on (facingMode); where it doesn't, or where the phone answers with
// the same camera again, the next camera of the list is taken. A phone is told by its pointer being a finger: a
// laptop with a touch screen has a mouse as its first pointer and keeps the camera list alone.
const phoneLike = () => navigator.maxTouchPoints > 0 && matchMedia("(pointer: coarse)").matches;
let camCount = 0, wantFacing = "", switching = false;
// The camera this device is set to (chosenCam): the one its owner picked in the list, or at the very first start
// the one the system gave. It stays the device's camera until the owner picks another. A start at which it isn't
// there (unplugged, or the system not ready with it yet) uses the default camera meanwhile (camFellBack) and does
// NOT take that one for the choice: on a computer with a second camera that shows nothing (a phone offered as a
// webcam while the phone is away gives a black picture) the page once opened that one at a start, remembered it,
// and showed a blank picture from then on, at every start, until the camera was picked by hand again
// (2026-11-24). The chosen camera is gone back to as soon as it is there again (backToChosen).
// camAsIs: the phone's switch button asks for the other side or the next camera, not for the remembered one.
let chosenCam = "", camFellBack = false, camAsIs = false, backTries = 0;
function showCamSwitch() {
  $("camSwitch").hidden = !(phoneLike() && camCount > 1 && (stream || switching));
  $("camSwitch").disabled = switching;
}
async function switchCamera() {
  if (switching || !stream) return;
  const before = stream.getVideoTracks()[0]?.getSettings() || {};
  const sel = $("camSelect"), cams = [...sel.options].filter(o => o.value);
  const other = before.facingMode === "environment" ? "user" : before.facingMode === "user" ? "environment" : "";
  const next = cams[(cams.findIndex(o => o.value === before.deviceId) + 1) % Math.max(1, cams.length)];
  switching = true;
  try {
    stopCamera();                                // a phone opens one camera at a time: this one first
    camAsIs = true;
    if (other) { wantFacing = other; sel.value = ""; } else if (next) sel.value = next.value;
    await startCamera();
    wantFacing = "";
    const after = stream?.getVideoTracks()[0]?.getSettings() || {};
    if (other && stream && after.deviceId === before.deviceId && next && next.value !== before.deviceId) {
      stopCamera(); sel.value = next.value; await startCamera();
    }
  } finally { wantFacing = ""; camAsIs = false; switching = false; showCamSwitch(); }
}

// Starts can overlap (the page's own start, the Start button, a change of camera, Keep this camera running): the
// newest wins, and an earlier one that gets its camera afterwards hands it straight back. Before, that camera stayed
// open with nothing showing it (its light on after Stop camera), and the two fought over the picture.
let camGen = 0, camError = "";             // camError: why the last start failed (an error's name), "" once it runs
async function startCamera() {
  if (stream) { stopCamera(); return; }
  const mine = ++camGen;
  let got = null, fellBack = false;
  try {
    // the camera picked in the list; with none picked there, the one this device is set to (the list doesn't
    // always have it yet when the page starts). A phone keeps to its list: its cameras' IDs change.
    const id = $("camSelect").value || (camAsIs || phoneLike() ? "" : chosenCam);
    // no camera chosen: on a phone the side that was asked for, and the front one when none was
    const facing = wantFacing || (phoneLike() ? "user" : "");
    const ask = id => navigator.mediaDevices.getUserMedia({ video: {
      width: { ideal: 1280 }, height: { ideal: 720 },
      ...(id ? { deviceId: { exact: id } } : facing ? { facingMode: { ideal: facing } } : {}) } });
    try { got = await ask(id); }
    catch (e) {
      // A remembered camera ID only fits the browser that saved it (the app window and each browser
      // get their own IDs), and cameras get unplugged: the default camera for the time being.
      if (!id || !["OverconstrainedError", "NotFoundError"].includes(e.name)) throw e;
      $("camSelect").value = ""; got = await ask(""); fellBack = true;
    }
    if (mine !== camGen) { got.getTracks().forEach(t => t.stop()); return; }
    stream = got;
    video.srcObject = stream; video.removeAttribute("src");
    // a camera that faces the person is shown as a mirror; one that looks away from them as it is
    $("optMirror").checked = stream.getVideoTracks()[0]?.getSettings().facingMode !== "environment"; applyMirror();
    await video.play();
    const activeId = stream.getVideoTracks()[0]?.getSettings().deviceId;
    listCameras(activeId);
    // The camera that was asked for is the one this device is set to from now on. One that was only fallen back
    // to is not: the choice stays as it was.
    camFellBack = fellBack && !!chosenCam && !phoneLike() && activeId !== chosenCam;
    if (!camFellBack) { chosenCam = activeId || ""; remember("camera", chosenCam); }
    else pageNote("The camera this device is set to isn't there now: the default camera is used until it is back.");
    remember("autostart", "1");
    // unplugged, or taken by another program: stopped, so "Keep this camera running" starts it again
    stream.getVideoTracks()[0]?.addEventListener("ended", () => { if (stream) stopCamera(); });
    $("camBtn").textContent = "Stop camera";
    camError = "";
    begin();
  } catch (e) {
    got?.getTracks().forEach(t => t.stop());
    if (mine !== camGen) return;
    stream = null;
    camError = e?.name || "Error";                        // why, for "Keep this camera running" and the log
    setStatus(e.name === "NotReadableError" ? "That camera is busy in another app. Close it there, or pick another camera."
            : e.name === "NotAllowedError" ? "Camera blocked. Allow camera access for this page and try again."
            : "Couldn't start that camera. Try another one from the list.", "err");
  }
}

// A camera that sends a flat picture: a laptop camera with the lid shut, a closed privacy shutter, a covered lens.
// Every 2 s the live picture is shrunk to 32x18 and its brightest and darkest points compared; three flat ones in a
// row (6 s) and the Source card says so, with a button for the next camera. Any real scene, even a dim room, differs
// by far more than FLAT_SPREAD (a closed lid measured a single value, 8 of 255, everywhere).
const FLAT_SPREAD = 12, FLAT_CHECKS = 3, FLAT_EVERY = 2000;
let flatRun = 0, flatShown = false;
const flatGrab = Object.assign(document.createElement("canvas"), { width: 32, height: 18 });
function pictureSpread() {                 // [brightest minus darkest, a number that differs when any point does]
  const g = flatGrab.getContext("2d", { willReadFrequently: true });
  g.drawImage(video, 0, 0, 32, 18);
  const d = g.getImageData(0, 0, 32, 18).data;
  let lo = 255, hi = 0, sum = 0;
  for (let i = 0; i < d.length; i += 4) {
    const y = (d[i] + d[i + 1] + d[i + 2]) / 3; if (y < lo) lo = y; if (y > hi) hi = y;
    sum = (Math.imul(sum, 31) + d[i] + 3 * d[i + 1] + 7 * d[i + 2]) | 0;
  }
  return [hi - lo, sum];
}

// A camera that is on but sends no new picture. Windows does this to every program while the screen is locked, and
// the video keeps its last picture: on 2026-11-15 one device's camera was shown as live for an hour that way, its
// last face still "in view". It is told two ways: the browser says the camera is muted, or the picture sampled
// above is the very same STILL_CHECKS times in a row (6 s) while the browser counts no new frame. A real camera's
// picture never repeats exactly (its noise changes it); a flat one can, and is left to the check above; a made-up
// camera showing one still picture keeps delivering frames, which the browser counts. (Measured in Edge 157:
// totalVideoFrames follows the frames delivered, shown or hidden; currentTime follows the clock and "muted" wasn't
// set by a stream whose frames stopped.) While it lasts no face is read from the still picture, the other devices
// are told (paused, with the readout) and the log says so.
const STILL_CHECKS = 3;
let stillSum = null, stillFrames = null, stillRun = 0, pausedAt = 0;      // pausedAt: when it was noticed (0: its picture arrives)
const camPaused = () => pausedAt > 0;
function stillCheck(spread, sum) {
  let frames = null;
  try { frames = video.getVideoPlaybackQuality?.().totalVideoFrames ?? null; } catch {}
  const same = sum === stillSum && spread >= FLAT_SPREAD && (frames === null || frames === stillFrames);
  stillRun = same ? stillRun + 1 : 0;
  stillSum = sum; stillFrames = frames;
  setPaused(stream.getVideoTracks()[0]?.muted === true || stillRun >= STILL_CHECKS);
}
function setPaused(on) {
  if (on === camPaused()) return;
  pausedAt = on ? performance.now() : 0;
  if (on) {
    for (const name of Object.keys(visitsOpen)) closeVisit(name);        // nobody is seen in a picture that stands still
    tracks = []; shown = null; renderList(); renderPeople();
    setStatus("The camera is on but sends no new picture. On Windows a locked screen does this.", "err");
    pageNote("The camera is on but sends no new picture (on Windows a locked screen does this): no faces are read until it does.");
  } else {
    setStatus("Tracking", "live");
    pageNote("The camera's picture arrives again.");
  }
}
function showFlat(on) {
  flatShown = on; $("camFlat").hidden = !on;
  if (!on) return;
  const sel = $("camSelect"), cams = [...sel.options].filter(o => o.value);
  const now = stream?.getVideoTracks()[0]?.getSettings().deviceId;
  const next = cams[(cams.findIndex(o => o.value === now) + 1) % Math.max(1, cams.length)];
  $("camFlatText").textContent = "This camera is sending a blank picture: its lens may be covered, its privacy shutter " +
    "closed or the laptop lid shut." + (next && next.value !== now ? "" : " No other camera was found.");
  const btn = $("camFlatTry");
  btn.hidden = !next || next.value === now;
  if (!btn.hidden) { btn.textContent = `Use ${next.textContent}`; btn.dataset.id = next.value; }
}
setInterval(() => {
  if (!stream || video.readyState < 2 || !video.videoWidth) { flatRun = 0; if (flatShown) showFlat(false); return; }
  let spread, sum;
  try { [spread, sum] = pictureSpread(); } catch { return; }
  stillCheck(spread, sum);
  flatRun = spread < FLAT_SPREAD ? flatRun + 1 : 0;
  if (flatRun >= FLAT_CHECKS && !flatShown) showFlat(true);
  else if (flatRun === 0 && flatShown) showFlat(false);
}, FLAT_EVERY);
$("camFlatTry").onclick = () => {
  const sel = $("camSelect"); sel.value = $("camFlatTry").dataset.id || "";
  showFlat(false); flatRun = 0; sel.dispatchEvent(new Event("change"));
};

function stopCamera() {
  flatRun = 0; showFlat(false);
  pausedAt = 0; stillRun = 0; stillSum = null;                    // stopped, not paused
  stream?.getTracks().forEach(t => t.stop());
  stream = null; video.srcObject = null; running = false;
  showCamSwitch();
  $("camBtn").textContent = "Start camera";
  for (const name of Object.keys(visitsOpen)) closeVisit(name);   // the camera's off: they're out of view
  $("empty").hidden = false; tracks = []; shown = null; ctx.clearRect(0, 0, canvas.width, canvas.height);
  renderList(); renderPeople(); loadPeople();
}

function openFile(file) {
  if (!file) return;
  if (stream) stopCamera();
  video.srcObject = null; video.src = URL.createObjectURL(file); video.loop = true;
  $("optMirror").checked = false; applyMirror();
  video.play().then(begin);
}

function begin() {
  $("empty").hidden = true; tracks = []; shown = null;
  setStatus("Tracking", "live");
  if (!running) { running = true; requestAnimationFrame(draw); sendLoop(); }
}

// ---------------------------------------------------------------- JPEGs, made off the page's main thread
// canvas.toBlob makes its picture when the page has idle time, and with none only after a second. Measured
// 2026-11-19 (WebView2 157, a 1280x720 JPEG): 13 ms with the page idle, 26 ms with its main thread half busy,
// 101 ms at 80%, 1,100 ms from 96%. So a page busy enough read faces once a second and sent its camera to the
// other devices at 1 frame a second, with the computer far from busy. A worker (jpeg_worker.js) has no such wait:
// 10, 13, 24 and 25 ms at the same loads. Where there is no worker or no OffscreenCanvas: toBlob as before. When
// the worker fails (three times in a row, or its script doesn't load): toBlob for a minute, then a new worker is
// tried, and after each further failure twice as long later (at most half an hour). Not "off for good": a graphics
// driver that is installed or reset under the app takes the worker's canvas with it for a moment (2026-11-19), and
// the page would have kept toBlob's wait until its next start. The log says each change (pageNote).
// Only in Chromium's browsers (WebView2 on Windows, Qt WebEngine on Linux: both measured, the same wait). WebKit
// (the macOS window) makes toBlob's picture at once however busy the page is (4 ms on the Mac, 2026-11-19), and
// the worker there gave the same picture later (23 ms with the page 80% busy): there, toBlob stays.
// off: 0 while the worker is used, else when (performance.now()) it is tried again; Infinity: never.
// out: stream frames handed to the worker and not yet reported back (streamByWorker), outAt: when the last was.
// sOff, sWait: the same for the stream's frames, which the worker also sends where the page makes its own JPEGs.
const jpegs = { worker: null, n: 0, waiting: new Map(), fails: 0, off: 0, wait: 60000, okAt: 0, said: false, out: 0, outAt: 0, told: null,
                sOff: 0, sWait: 60000,
                waits: typeof navigator !== "undefined" && /\bChrome\//.test(navigator.userAgent) };
const toBlobJpeg = (c, q) => new Promise(r => c.toBlob(r, "image/jpeg", q));
function jpegsOff(why, again) {
  if (jpegs.off) return;
  const now = performance.now();
  if (again && jpegs.okAt && now - jpegs.okAt > 10 * 60000) jpegs.wait = 60000;    // it had worked for a while: start over
  jpegs.off = again ? now + jpegs.wait : Infinity;
  if (again) jpegs.sOff = jpegs.off;                        // a worker that failed: not for the stream either
  try { jpegs.worker?.terminate(); } catch {}
  jpegs.worker = null; jpegs.fails = 0; jpegs.said = false; jpegs.out = 0;
  for (const [, [, no]] of jpegs.waiting) no(new Error(why));
  jpegs.waiting.clear();
  pageNote(`JPEGs are made on the page itself (toBlob)${again ? `, the worker is tried again in ${Math.round(jpegs.wait / 60000)} min` : ""}: ${why}`);
  if (again) jpegs.wait = Math.min(jpegs.wait * 2, 30 * 60000);
}
function jpegWorker() {
  if (jpegs.worker) return jpegs.worker;
  const w = jpegs.worker = new Worker("/jpeg_worker.js");
  w.onmessage = e => {
    if (e.data.stream) {                                     // how a stream frame went (streamByWorker)
      jpegs.out = Math.max(0, jpegs.out - 1);
      if (!jpegs.said && e.data.ok) { jpegs.said = true; jpegs.okAt = performance.now(); pageNote("JPEGs are made in a worker"); }
      if (jpegs.told) jpegs.told(e.data);
      return;
    }
    const wait = jpegs.waiting.get(e.data.n);
    if (!wait) return;
    jpegs.waiting.delete(e.data.n);
    if (e.data.blob) wait[0](e.data.blob); else wait[1](new Error(e.data.error || "no picture"));
  };
  w.onerror = e => { try { e.preventDefault(); } catch {} if (jpegs.worker === w) workerFailed(`the worker stopped (${e.message || "its script didn't load"})`); };
  return w;
}
// The worker failed. Where it makes the page's JPEGs: toBlob for a while (jpegsOff), and the stream with them.
// Where the page makes its own (WebKit) the worker only sends the stream: the page sends it for a while.
function workerFailed(why) {
  if (jpegs.waits && !jpegs.off) { jpegsOff(why, true); return; }
  if (jpegs.sOff) return;
  const now = performance.now();
  jpegs.sOff = now + jpegs.sWait;
  try { jpegs.worker?.terminate(); } catch {}
  jpegs.worker = null; jpegs.fails = 0; jpegs.out = 0;
  pageNote(`This camera's frames are sent by the page itself, the worker is tried again in ${Math.round(jpegs.sWait / 60000)} min: ${why}`);
  jpegs.sWait = Math.min(jpegs.sWait * 2, 30 * 60000);
}
async function jpegOf(c, q) {
  if (jpegs.off && performance.now() >= jpegs.off) jpegs.off = 0;          // time to try the worker again
  if (jpegs.off || !jpegs.waits || !window.Worker || !window.OffscreenCanvas || !window.createImageBitmap) {
    if (!jpegs.off) jpegsOff(jpegs.waits ? "this browser has no worker canvas" : "this browser makes them at once", false);
    return toBlobJpeg(c, q);
  }
  let bmp = null;
  try {
    const w = jpegWorker();
    bmp = await createImageBitmap(c);                       // the canvas as it is now; the worker gets it, not a copy
    const blob = await new Promise((yes, no) => {
      const n = ++jpegs.n;
      jpegs.waiting.set(n, [yes, no]);
      w.postMessage({ n, bmp, q }, [bmp]);
      setTimeout(() => { if (jpegs.waiting.delete(n)) no(new Error("no answer in 5 s")); }, 5000);
    });
    jpegs.fails = 0;
    if (!jpegs.said) { jpegs.said = true; jpegs.okAt = performance.now(); pageNote("JPEGs are made in a worker"); }
    return blob;
  } catch (e) {
    try { bmp?.close(); } catch {}
    if (++jpegs.fails >= 3) jpegsOff(`the worker failed three times (${e.message})`, true);
    return toBlobJpeg(c, q);
  }
}
// This camera's picture for the other devices, made AND sent by the worker, where the worker is in use. The page
// only hands the picture over, in one turn of its thread; the JPEG and the request to this page's own server are
// the worker's. Made and sent from the page, a frame took four turns of the page's thread (the picture, the JPEG,
// the request, its answer), each waiting behind whatever held the thread: with a face in view and MediaPipe's
// parts on (they hold it 100 to 150 ms a check), the stream fell from 20 frames a second to about 10 (measured
// 2026-11-23, with every part on: 9.4 before, and after, see streamFrame). A camera's stream is not to slow down
// because the page is busy reading faces. picture(): the frame as an ImageBitmap, which the page no longer has
// afterwards; made only when it is handed over. told(report): how each frame went ({ok}, {off}: not shared now,
// {error}). Answers "sent", "wait" (two frames are still on their way: the worker or the network is behind, and
// a frame is left out rather than queued) or "no" (no worker now: the page makes and sends it itself).
function streamByWorker(picture, q, failed, told) {
  const now = performance.now();
  if (jpegs.off && now >= jpegs.off) jpegs.off = 0;                        // time to try the worker again
  if (jpegs.sOff && now >= jpegs.sOff) jpegs.sOff = 0;
  // (where the worker makes the page's JPEGs and is off for them, it is off for the stream; where the page makes
  // its own, WebKit, the worker is there for the stream alone)
  if (!window.Worker || !window.OffscreenCanvas || jpegs.sOff || (jpegs.waits && jpegs.off)) return "no";
  if (jpegs.out >= 2) {
    // (a worker that says nothing of its frames is one that stopped: the page takes over, as for a JPEG)
    if (now - jpegs.outAt > 6000) { workerFailed("the worker said nothing of a stream frame for 6 s"); return "no"; }
    return "wait";
  }
  let bmp = null;
  try {
    const w = jpegWorker();
    bmp = picture();
    jpegs.told = told;
    w.postMessage({ stream: ++jpegs.n, bmp, q, failed: failed || "" }, [bmp]);
    jpegs.out++; jpegs.outAt = performance.now();
    return "sent";
  } catch (e) {
    try { bmp?.close(); } catch {}
    if (++jpegs.fails >= 3) workerFailed(`the worker failed three times (${e.message})`);
    return "no";
  }
}

// ---------------------------------------------------------------- recognition requests
async function sendLoop() {
  while (running) {
    if (video.readyState < 2 || busy || camPaused()) { await new Promise(r => setTimeout(r, 30)); continue; }
    const vw = video.videoWidth, vh = video.videoHeight, s = Math.min(1, SEND_MAX / Math.max(vw, vh));
    grab.width = Math.round(vw * s); grab.height = Math.round(vh * s);
    const sync = !!meshTris && $("optMesh").checked;
    const hold = holds[0] === shown ? holds[1] : holds[0];
    if (sync) {                            // keep the full-size frame to show with its result
      if (hold.width !== vw || hold.height !== vh) { hold.width = vw; hold.height = vh; }
      hold.getContext("2d").drawImage(video, 0, 0);
      gctx.drawImage(hold, 0, 0, grab.width, grab.height);
    } else {
      shown = null;
      gctx.drawImage(video, 0, 0, grab.width, grab.height);
    }
    const blob = await jpegOf(grab, 0.85);
    busy = true;
    try {
      const want = new URLSearchParams({ t: $("optStrict").value });
      if (cuesOn && needCues()) want.set("c", "1");
      if (poseOn && ($("optPose").checked || $("optHandGate").checked)) want.set("p", "1");
      if (meshTris && $("optMesh").checked) want.set("m", "1");
      if (poseOn && $("optSkeleton").checked) want.set("b", "1");
      if (handsOn && $("optFingers").checked) want.set("f", "1");
      if (test?.label) {
        want.set("rec", test.label); want.set("session", test.session);
        if (test.talk) want.set("talk", "1");
        if (test.hand) want.set("hand", "1");
        const big = largestTrack(), now = performance.now(), k = big?.fs.talking(now);
        if (k) want.set("tv", k.score.toFixed(4));
        if (voice) want.set("vad", voice.active(now) ? "1" : "0");
      }
      const asked = performance.now();
      // (while a face is in view, the parts that need no face's box begin with the frame: parts.js begin)
      const begun = parts?.begin && tracks.length ? parts.begin(grab, want) : null;
      const r = await fetch(`/api/recognize?${want}`, { method: "POST", body: blob, headers: { "Content-Type": "image/jpeg", "X-Face-Tool": "1" } });
      const res = await r.json();
      if (!r.ok) throw new Error(res.error || r.status);
      if (parts) await parts.fill(res, grab, want, begun);   // cues, mesh, posture and fingers, from the frame just sent
      checkMs = performance.now() - asked; checkTimes.push(performance.now());
      if (camPaused()) continue;           // the picture stopped while this was asked: not a face in view
      updateTracks(res.faces.map(f => ({ ...f, box: f.box.map(v => v / s), kps: f.kps.map(([x, y]) => [x / s, y / s]),
                                         pose: f.pose && f.pose.map(([x, y, v]) => [x / s, y / s, v]),
                                         body: f.body && f.body.map(([x, y, v]) => [x / s, y / s, v]),
                                         hands: f.hands && f.hands.map(h => h.map(([x, y]) => [x / s, y / s])),
                                         mesh: f.mesh && Float32Array.from(f.mesh, v => v / s) })));
      shown = sync && $("optMesh").checked ? hold : null;
      await identify(want.get("t"));
      trackPresence();
      chartSample();                        // one reading for the Signals chart, with every check
      if ($("optCollect").checked) collect(blob, s);
      checks++;
      if ($("status").classList.contains("err")) setStatus("Tracking", "live");
    } catch (e) {
      shown = null;                        // back to the live video rather than a frozen frame
      if (performance.now() - lastErr > 2000) setStatus("Lost the recognizer. Is the tracker window still open?", "err");
      lastErr = performance.now();
      await new Promise(r => setTimeout(r, 500));
    } finally { busy = false; }
  }
}

// "Collect faces": when a face has been in view for a few checks, is sharp enough to learn from, and hasn't
// been saved in the last few seconds, send the frame just checked with its box and the tracker's verdict (voted
// name and match), in frame pixels of the JPEG. The tracker keeps only that face with some room around it, never
// the whole frame, and greys out the other faces in view (`others`) so nobody else is saved.
function collect(blob, s) {
  const now = performance.now();
  const due = tracks.filter(t => t.matched && t.votes.length >= CAP_SEEN && (t.quality ?? 0) >= qEnroll
                                 && (t.caps || 0) < CAP_MAX && now - (t.capAt || -Infinity) >= CAP_EVERY);
  if (!due.length) return;
  for (const t of due) { t.capAt = now; t.caps = (t.caps || 0) + 1; }
  const boxOf = t => [t.tx, t.ty, t.tw, t.th].map(v => +(v * s).toFixed(1));
  const faces = due.map(t => ({ box: boxOf(t), name: t.name, sim: +t.sim.toFixed(3) }));
  const others = tracks.filter(t => t.matched && !due.includes(t)).slice(0, 20).map(boxOf);
  fetch(`/api/capture?${new URLSearchParams({ faces: JSON.stringify(faces), others: JSON.stringify(others) })}`,
        { method: "POST", body: blob, headers: { "Content-Type": "image/jpeg", "X-Face-Tool": "1" } })
    .then(r => r.ok ? r.json() : (drop(r), Promise.reject(new Error(r.status))))
    .then(() => { captured += faces.length; $("collectHint").textContent = `${captured} face snapshot${captured === 1 ? "" : "s"} saved this session for the labeling tool.`; })
    .catch(() => { $("collectHint").textContent = "Couldn't save a snapshot. Is the tracker window still open?"; });
}

// Name each face from the average of its last 7 face prints, weighted by face quality, instead of
// the vote over 7 per-frame names that updateTracks keeps as a fallback. Measured on
// webcam faces: 98% named correctly against 93% for the vote, no stranger
// named either way. One small request for all faces in view.
function printOf(b64) {
  const s = atob(b64), u = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) u[i] = s.charCodeAt(i);
  return new Float32Array(u.buffer);
}
function b64Of(f32) {
  const u = new Uint8Array(f32.buffer, f32.byteOffset, f32.byteLength);
  let s = "";
  for (let i = 0; i < u.length; i += 8192) s += String.fromCharCode(...u.subarray(i, i + 8192));
  return btoa(s);
}
async function identify(threshold) {
  const live = tracks.filter(t => t.matched && t.prints?.length);
  if (!live.length) return;
  const prints = live.map(t => {
    const m = new Float32Array(t.prints[0][0].length);
    for (const [p, w] of t.prints) for (let i = 0; i < m.length; i++) m[i] += p[i] * w;
    return b64Of(m);
  });
  try {
    const r = await fetch("/api/who", { method: "POST", body: JSON.stringify({ t: +threshold, prints }),
                                        headers: { "Content-Type": "application/json", "X-Face-Tool": "1" } });
    if (!r.ok) { drop(r); throw new Error(r.status); }
    (await r.json()).who.forEach(([name, sim], k) => Object.assign(live[k], { name, sim, byAverage: true }));
  } catch { for (const t of live) t.byAverage = false; }     // fall back to the vote
}

// Match new detections to existing tracks by nearest centre; vote on names over recent checks.
function updateTracks(dets) {
  const used = new Set();
  for (const t of tracks) t.matched = false;
  for (const d of dets) {
    const [x, y, w, h] = d.box, cx = x + w / 2, cy = y + h / 2;
    let best = null, bestDist = Infinity;
    for (const t of tracks) {
      if (used.has(t)) continue;
      const dist = Math.hypot(t.tcx - cx, t.tcy - cy);
      if (dist < Math.max(t.tw, w) * 0.9 && dist < bestDist) { best = t; bestDist = dist; }
    }
    if (!best) {
      best = { x, y, w, h, kp: d.kps, trail: [], votes: [], xt: new Expression.Track(), fs: new Signals.FaceSignals(),
               expr: null, name: d.name, sim: d.sim, talking: false };
      tracks.push(best);
    }
    used.add(best); best.matched = true; best.missed = 0;
    Object.assign(best, { tx: x, ty: y, tw: w, th: h, tcx: cx, tcy: cy, tkp: d.kps, score: d.score });
    best.tmesh = d.mesh || null; best.quality = d.quality;
    if (d.emb) {                                  // recent face prints, for naming by their average (identify)
      (best.prints ||= []).push([printOf(d.emb), d.quality == null ? 0.5 : Math.max(0.05, d.quality)]);
      if (best.prints.length > 7) best.prints.shift();
    }
    if (!best.tmesh || best.mesh?.length !== best.tmesh.length) best.mesh = best.tmesh && best.tmesh.slice();
    // Each frame votes with a weight from its face quality, so sharp frames outvote blurry ones
    const qw = d.quality == null ? 0.5 : Math.max(0.05, d.quality);
    best.votes.push([d.name, d.sim, qw]); if (best.votes.length > 7) best.votes.shift();
    const tally = {};
    for (const [n, s, w] of best.votes) tally[n] = (tally[n] || 0) + w * (1 + s);   // quality-weighted; similarity breaks ties
    if (!best.byAverage) {                        // identify() names it from the averaged prints once it can
      best.name = Object.entries(tally).sort((a, b) => b[1] - a[1])[0][0];
      const mine = best.votes.filter(v => v[0] === best.name);
      const wsum = mine.reduce((a, v) => a + v[2], 0);
      best.sim = mine.reduce((a, v) => a + v[1] * v[2], 0) / wsum;
    }
    // Other signals (signals.js): motion, face-shape cues over time, expression shares for reports
    const now = performance.now();
    best.fs.addPosition(now, d.box); best.fs.addCues(now, d.cues); best.fs.addExpr(now, d.expr);
    best.fs.addPose(now, d.pose, d.hand); best.pose = d.pose; best.body = d.body || null; best.hands = d.hands || null;
    best.talking = isTalking(best, now, dets.length);
    // Expression votes the same way (expression.js). Frames the recognizer didn't score (blurry,
    // turned away; or talking / a hand on the face, with those options on) cast no vote; after ten
    // in a row the reading is dropped.
    if (best.talking && d.expr && $("optTalkGate").checked) best.xt.add(null, d.quality, "talking", d.cues);
    else if (d.hand && d.expr && $("optHandGate").checked) best.xt.add(null, d.quality, "hand", d.cues);
    else best.xt.add(d.expr, d.quality, d.xskip, d.cues);
    best.expr = readExpression(best);
    best.fs.addLabel(now, best.expr && best.expr.state !== "unsure" ? best.expr.label : null);
    // This person's normal, alone or in company: learn it, and keep the current signals to compare with it
    best.ctx = dets.length > 1 ? "company" : "alone";
    if ($("optCalib").checked && best.name !== "Unknown") {
      best.snap = best.fs.snapshot(now, voiceFor(best, now));
      const known = best.name in baselines.people;
      if (baselines.learn(best.name, best.snap, best.ctx, now) && !known) renderForget();   // a new name to list
    } else best.snap = null;
    if (best.expr && vaFit) best.expr.vaFit = Signals.applyFit(vaFit, best.name, best.fs.summary(now, voiceFor(best, now)));
  }
  for (const t of tracks) if (!t.matched) t.missed++;
  tracks = tracks.filter(t => t.missed < 4);
  renderList(); renderPeople();
}

// ---------------------------------------------------------------- expression
// Voting, confidence and each person's usual look are in expression.js (with tests); this part
// connects them to the page.
const store = window.PageStore ? PageStore.signals : (() => { try { return localStorage; } catch { return null; } })();
const usualLooks = new Expression.UsualLooks(store);
const baselines = new Signals.Baselines(store);     // each person's normal for every signal (signals.js)

// People something has been learned about, for "Forget what's learned"
function renderForget() {
  const names = [...new Set([...baselines.names(), ...Object.keys(usualLooks.looks)])].sort();
  const keep = $("forgetWho").value;
  $("forgetWho").innerHTML = `<option value="">Everyone</option>` + names.map(n => `<option>${esc(n)}</option>`).join("");
  if (names.includes(keep)) $("forgetWho").value = keep;
}

// What to do when this page's program stopped or needs a restart. In the app window the Start again button asks the
// window to start it again (/restart); opened on its own, the program is started again by hand,
// and the button reloads the page.
function startAgain(what) {
  return window.parent !== window ? `${what}: click Start again (or close Face Recognizer and start it again).`
    : `${what}: start the tracker again, then click Start again.`;
}
$("againBtn").addEventListener("click", () => {
  if (window.parent !== window) SettingsBridge.notify({ fs: "restart" }); else location.reload();
});

function setExpressions(list) {
  const was = expressions; expressions = list;
  for (const id of ["optExpr", "optEvidence", "testBtn", "optTalkGate", "optState"]) $(id).disabled = !list;
  $("exprHint").textContent = list ? $("exprHint").dataset.on
    : startAgain("Expression model not installed: put facial_expression_recognition_mobilefacenet_2022july.onnx (OpenCV Zoo) in the models folder");
  if (was && !list) for (const t of tracks) { t.xt.clear(); t.expr = null; }
}

function readExpression(t) {
  const temperature = calibration?.people?.[t.name] ?? calibration?.global ?? null;
  return Expression.read(t.xt, expressions, t.name, $("optCalib").checked ? usualLooks : null,
                         { stateModel: $("optState").checked, temperature });
}

// ---------------------------------------------------------------- other signals (signals.js)
const needCues = () => $("optEvidence").checked || !!test?.label || $("optTalkGate").checked || +$("feelEvery").value > 0 || !!feel;
const largestTrack = () => tracks.filter(t => t.missed === 0).sort((a, b) => b.tw * b.th - a.tw * a.th)[0];

// Talking: the jaw is moving (face-shape cues), confirmed by the microphone when it's on. Without
// cues, the microphone alone decides, and only when one face is in view (otherwise whose voice?).
function isTalking(t, now, faces) {
  const k = t.fs.talking(now), vad = voice ? voice.active(now) : null;
  if (k) return k.talking && vad !== false;
  return vad === true && faces === 1;
}

// The voice belongs to whoever is talking, or to the only face in view.
function voiceFor(t, now) {
  if (!voice) return null;
  const live = tracks.filter(x => x.missed === 0);
  if (!(t.talking || live.length === 1)) return null;
  return { stats: voice.stats(now), avd: voice.currentAvd(now) };
}

// Microphone: 16 kHz audio into VoiceSignals (in the page); every second with speech in it, the last
// 2 seconds go to the recognizer's voice model. Nothing is recorded.
let voice = null, mic = null, audioCtx = null, voiceTimer = null, voiceBusy = false;
// The microphone runs while voice is listened to, or while this page hears the device's sound for the Signals chart
// (soundByPage: a device whose program can't listen to its audio interfaces itself, a phone). ear is what it hears
// for either; voice is the same thing only while Listen to voice is on, so that nothing is read of a voice the
// owner didn't ask to be listened to.
let ear = null, soundByPage = false, micStarting = false, micFailed = false;
function micSync() {
  const need = $("optVoice").checked || soundByPage;
  if (need && !mic && !micStarting && !micFailed) startVoice();
  else if (!need && (mic || ear)) stopVoice();
  voice = $("optVoice").checked ? ear : null;
}
// Switched off while it was still starting (the permission prompt, a quick off and on), the microphone used to open
// anyway and stay open with the switch off: each start has a number, and one that was overtaken gives its microphone back.
let voiceGen = 0;
// The microphone for voice is this device's own. A Mac offers a nearby iPhone or iPad as a microphone of its own,
// and opening it takes the phone over (it shows "Pause or Disconnect" and its own app stops): the owner's decision,
// 2026-11-22, is that the phone listens for itself and the Mac never does through it. The page can tell such a
// microphone only by its name, so that is what it goes by, on a Mac alone (a phone's own page keeps its microphone).
const ON_A_MAC = /Mac/i.test(navigator.platform || "") && !navigator.maxTouchPoints;
const aPhones = label => ON_A_MAC && /iphone|ipad/i.test(label || "");
let voiceWhy = "";                          // why voice has no microphone here ("nomic": none of this device's own)
async function ownMicrophone(not = "") {
  const audio = { echoCancellation: true, noiseSuppression: false, autoGainControl: false };
  if (!ON_A_MAC) return audio;
  let mics = [];
  try { mics = (await navigator.mediaDevices.enumerateDevices()).filter(d => d.kind === "audioinput"); } catch {}
  // (before the microphone was ever allowed the names are empty: then the one the system gives is looked at after)
  if (!not && !mics.some(d => aPhones(d.label))) return audio;
  const own = mics.find(d => d.label && !aPhones(d.label) && d.deviceId && d.deviceId !== "default" && d.deviceId !== not);
  return own ? { ...audio, deviceId: { exact: own.deviceId } } : null;
}
async function startVoice() {
  const mine = ++voiceGen;
  micStarting = true;
  try {
    voiceWhy = "";
    let audio = await ownMicrophone(), got = audio && await navigator.mediaDevices.getUserMedia({ audio });
    // the system's own choice turned out to be a phone's: let go of it at once and take one of this device's own
    const track = got?.getAudioTracks()[0];
    if (track && aPhones(track.label)) {
      got.getTracks().forEach(t => t.stop());
      audio = await ownMicrophone(track.getSettings().deviceId || "default");
      got = audio && await navigator.mediaDevices.getUserMedia({ audio });
      if (got && aPhones(got.getAudioTracks()[0]?.label)) { got.getTracks().forEach(t => t.stop()); got = null; }
    }
    if (!got) { voiceWhy = "nomic"; return; }            // no microphone of this device's own: voice has no readings
    if (mine !== voiceGen) { got.getTracks().forEach(t => t.stop()); return; }
    mic = got;
    audioCtx = new AudioContext({ sampleRate: Signals.S.V_RATE });
    const tap = "registerProcessor('tap', class extends AudioWorkletProcessor { process(i) { const c = i[0][0]; if (c) this.port.postMessage(c.slice(0)); return true; } });";
    const tapUrl = URL.createObjectURL(new Blob([tap], { type: "text/javascript" }));
    try { await audioCtx.audioWorklet.addModule(tapUrl); } finally { URL.revokeObjectURL(tapUrl); }
    if (mine !== voiceGen) return;
    const node = new AudioWorkletNode(audioCtx, "tap"), mute = audioCtx.createGain();
    mute.gain.value = 0;
    audioCtx.createMediaStreamSource(mic).connect(node); node.connect(mute); mute.connect(audioCtx.destination);
    ear = new Signals.VoiceSignals(audioCtx.sampleRate);
    voice = $("optVoice").checked ? ear : null;
    node.port.onmessage = e => ear?.push(e.data, performance.now());
    voiceTimer = setInterval(scoreVoice, 1000);
  } catch (e) {
    if (mine !== voiceGen) return;
    stopVoice(); micFailed = true;                       // not tried again until an option that wants it is changed
    if ($("optVoice").checked) {
      $("optVoice").checked = false;
      setStatus(e.name === "NotAllowedError" ? "Microphone blocked. Allow it for this page to use voice." : "Couldn't start the microphone.", "err");
    }
  } finally { if (mine === voiceGen) micStarting = false; }
}
function stopVoice() {
  voiceGen++;
  clearInterval(voiceTimer); voiceTimer = null;
  mic?.getTracks().forEach(t => t.stop()); mic = null;
  audioCtx?.close(); audioCtx = null; voice = ear = null; micStarting = false;
}
async function scoreVoice() {
  const now = performance.now(), s = voice?.stats(now, 2000);
  if (!voiceModel || voiceBusy || !s || s.speaking < 0.3) return;
  voiceBusy = true;
  try {
    const r = await fetch("/api/voice", { method: "POST", body: voice.clip(2).buffer,
                                          headers: { "Content-Type": "application/octet-stream", "X-Face-Tool": "1" } });
    const res = await r.json();
    if (r.ok && res.avd && voice) voice.setAvd(res.avd, performance.now());
  } catch {} finally { voiceBusy = false; }
}

// Pulse: mean skin colour of each face's forehead and cheeks, 30 times a second (the rate the pulse
// is worked out at; sampling every animation frame cost a lot on fast displays). Each sample copies
// one small picture per face, the area around its three skin regions, off the video and averages
// the regions from it: copying video pixels back from the GPU is the costly step, so it's done once.
const skin = document.createElement("canvas"), sctx = skin.getContext("2d", { willReadFrequently: true });
const SKIN_EVERY = 1000 / 30, SKIN_PX = 48;   // ms between samples; size of the copied picture's longer side
let skinAt = -Infinity;
function samplePulse(now) {
  if (now - skinAt < SKIN_EVERY) return;
  skinAt = now;
  const vw = video.videoWidth, vh = video.videoHeight;
  for (const t of tracks) {
    if (t.missed) continue;
    const regions = Signals.skinRegions(t.kp);
    if (!regions.length) continue;
    const x0 = Math.max(0, Math.min(...regions.map(r => r.x))), y0 = Math.max(0, Math.min(...regions.map(r => r.y)));
    const x1 = Math.min(vw, Math.max(...regions.map(r => r.x + r.w))), y1 = Math.min(vh, Math.max(...regions.map(r => r.y + r.h)));
    if (x1 - x0 < 2 || y1 - y0 < 2) continue;
    const k = SKIN_PX / Math.max(x1 - x0, y1 - y0), w = Math.max(1, Math.round((x1 - x0) * k)), h = Math.max(1, Math.round((y1 - y0) * k));
    if (skin.width !== w || skin.height !== h) { skin.width = w; skin.height = h; }
    sctx.drawImage(video, x0, y0, x1 - x0, y1 - y0, 0, 0, w, h);
    const px = sctx.getImageData(0, 0, w, h).data;
    const sum = [0, 0, 0];
    let n = 0;
    for (const r of regions) {
      const ax = Math.max(0, Math.floor((r.x - x0) * k)), bx = Math.min(w, Math.ceil((r.x + r.w - x0) * k));
      const ay = Math.max(0, Math.floor((r.y - y0) * k)), by = Math.min(h, Math.ceil((r.y + r.h - y0) * k));
      for (let y = ay; y < by; y++) for (let x = ax; x < bx; x++) {
        const i = (y * w + x) * 4;
        sum[0] += px[i]; sum[1] += px[i + 1]; sum[2] += px[i + 2]; n++;
      }
    }
    if (n) t.fs.addColor(now, sum.map(v => v / n));
  }
}

// Feeling check: you say how you feel (two 1-7 scales); the page saves that with a summary of the
// 30 seconds before the question, for the fit of valence/arousal.
let feel = null, lastAsk = performance.now();
function openFeel() {
  if (!running) { setStatus("Start the camera first, then report how you feel.", "err"); return; }
  const now = performance.now(), live = tracks.filter(t => t.missed === 0).sort((a, b) => b.tw * b.th - a.tw * a.th);
  const snap = {};
  for (const t of live) if (!snap[t.name]) snap[t.name] = { ...t.fs.summary(now, voiceFor(t, now)), withOthers: live.length > 1 ? 1 : 0 };
  const names = Object.keys(snap);
  if (!names.length) { setStatus("No face in view to report for.", "err"); return; }
  feel = snap;
  const first = names.find(n => n !== "Unknown") || names[0];
  $("feelWho").innerHTML = names.map(n => `<option${n === first ? " selected" : ""}>${esc(n)}</option>`).join("");
  $("feelV").value = $("feelA").value = 4; $("feelVOut").textContent = $("feelAOut").textContent = "4";
  $("feelCard").hidden = false; $("feelCard").scrollIntoView({ block: "nearest" }); $("feelV").focus();
}
function closeFeel() { feel = null; $("feelCard").hidden = true; lastAsk = performance.now(); }
async function saveFeel() {
  const name = $("feelWho").value, rep = { name, valence: +$("feelV").value, arousal: +$("feelA").value, features: feel?.[name] || {} };
  try {
    const r = await fetch("/api/expr-eval/report", { method: "POST", body: JSON.stringify(rep),
                                                     headers: { "Content-Type": "application/json", "X-Face-Tool": "1" } });
    if (!r.ok) throw new Error((await r.json()).error);
    drop(r);
  } catch (e) { setStatus(`Couldn't save the report: ${e.message}`, "err"); }
  closeFeel(); loadPeople();
}
setInterval(() => {
  const every = +$("feelEvery").value;
  if (every && running && !feel && performance.now() - lastAsk >= every * 60000) openFeel();
}, 15000);

function exprText(t) {
  return t.expr && $("optExpr").checked ? Expression.text(t.expr) : "";
}

function showDevices(devices, warmup) {
  const short = v => v.split(" ")[0];
  $("devices").textContent = devices ? "Runs on: " + Object.entries(devices).filter(([, v]) => v)
    .map(([k, v]) => `${k} ${short(v)}${warmup?.[k] != null ? ` (${warmup[k]} ms first)` : ""}`).join(" · ") : "";
}

// Posed expression test: asks for each expression in turn while frames are sent with rec=<label>,
// then to talk (with a neutral face, then smiling) so the talking detector and the talking gate can
// be measured: frames are sent with talk=1 and the page's talking signals.
const pause = ms => new Promise(r => setTimeout(r, ms));
const TALK_STEPS = [{ label: "neutral", talk: true, prompt: "Talk about your day, neutral face" },
                    { label: "happy", talk: true, prompt: "Keep talking, now smiling" },
                    { label: "neutral", hand: true, prompt: "Chin on your hand, neutral face" }];
async function runTest() {
  if (test) { test = null; return; }                       // pressing again stops it
  if (!stream) { setStatus("Start the camera first, then start the test.", "err"); return; }
  const d = new Date(), z = v => String(v).padStart(2, "0");    // UTC, like every stored time
  const session = `${d.getUTCFullYear()}${z(d.getUTCMonth() + 1)}${z(d.getUTCDate())}-${z(d.getUTCHours())}${z(d.getUTCMinutes())}${z(d.getUTCSeconds())}`;
  $("testBtn").textContent = "Stop test";
  test = { session };
  for (const label of expressions) {
    if (test?.session !== session) break;
    test = { session, label: null, prompt: `Next: look ${label}` }; await pause(2500);
    if (test?.session !== session) break;
    test = { session, label, prompt: `Look ${label}: hold it` }; await pause(4000);
  }
  for (const step of TALK_STEPS) {
    if (test?.session !== session) break;
    test = { session, label: null, prompt: `Next: ${step.prompt.toLowerCase()}` }; await pause(2500);
    if (test?.session !== session) break;
    test = { session, label: step.label, talk: !!step.talk, hand: !!step.hand, prompt: step.prompt }; await pause(6000);
  }
  test = null; $("testBtn").textContent = "Start test";
  loadPeople();
}

// ---------------------------------------------------------------- drawing (every animation frame)
// Frames a second sent to the other devices' Cameras tab: 20 (once 10), the owner's choice on my
// recommendation, about 10 Mbit/s a camera on the local network. A choice, not a measured limit.
const STREAM_FPS = 20, STREAM_EVERY = 1000 / STREAM_FPS;
// The names, boxes and markers are drawn at most 60 times a second, whatever the display's rate. On a 240 Hz display,
// drawing at its rate left the page no time for anything else: encoding one stream frame took 93 ms instead of 12
// (measured 2026-11-12), and the stream reached 13 frames a second instead of 20.
const DRAW_EVERY = 1000 / 60;
let drawnAt = 0;                                    // when requestAnimationFrame last drew
function draw() {
  if (!running) return;
  const now = performance.now();
  if (now - drawnAt >= DRAW_EVERY - 1) { drawnAt = now; render(); }
  requestAnimationFrame(draw);
}
// While the page is hidden (window minimized, covered, or another tab of the app shown), the browser stops calling
// requestAnimationFrame, and in a desktop session nobody is connected to it calls it only now and then (the NAS
// sent 4 frames a second): whenever it hasn't drawn for two frames, draw (and stream this camera) on a timer
// instead, so the names and boxes keep up and the stream keeps its pace.
setInterval(() => { if (running && (document.hidden || performance.now() - drawnAt > 2 * STREAM_EVERY)) render(); }, STREAM_EVERY / 2);
function render() {
  if (video.videoWidth && canvas.width !== video.videoWidth) { canvas.width = video.videoWidth; canvas.height = video.videoHeight; }
  if (shown && !$("optMesh").checked) shown = null;
  // Easing toward the latest detection; none while showing the analysed frame, whose results fit it exactly
  const a = shown ? 1 : 1 - parseFloat($("optSmooth").value) * 0.9;
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  if (shown) ctx.drawImage(shown, 0, 0, canvas.width, canvas.height);
  const lw = Math.max(2, canvas.width / 450), mirrored = $("optMirror").checked;
  if ($("optPulse").checked && video.videoWidth) samplePulse(performance.now());
  for (const t of tracks) {
    if (t.tx === undefined) continue;
    t.x += (t.tx - t.x) * a; t.y += (t.ty - t.y) * a; t.w += (t.tw - t.w) * a; t.h += (t.th - t.h) * a;
    t.kp = t.kp.map(([x, y], i) => [x + (t.tkp[i][0] - x) * a, y + (t.tkp[i][1] - y) * a]);
    if (t.mesh && t.tmesh) for (let i = 0; i < t.mesh.length; i++) t.mesh[i] += (t.tmesh[i] - t.mesh[i]) * a;
    const unknown = t.name === "Unknown";
    if (unknown && !$("optUnknown").checked) continue;
    const col = colorFor(t.name);
    t.trail.push([t.x + t.w / 2, t.y + t.h / 2]); if (t.trail.length > 40) t.trail.shift();
    ctx.globalAlpha = t.missed ? 0.45 : 1;
    if (unknown && $("optBlur").checked) {
      ctx.save(); ctx.beginPath(); ctx.rect(t.x, t.y, t.w, t.h); ctx.clip();
      ctx.filter = `blur(${Math.round(t.w / 9)}px)`; ctx.drawImage(shown || video, 0, 0, canvas.width, canvas.height);
      ctx.restore(); ctx.filter = "none";
    }
    if ($("optTrail").checked && t.trail.length > 1) {
      ctx.strokeStyle = col; ctx.lineWidth = lw; ctx.globalAlpha *= 0.5;
      ctx.beginPath(); t.trail.forEach(([x, y], i) => i ? ctx.lineTo(x, y) : ctx.moveTo(x, y)); ctx.stroke();
      ctx.globalAlpha = t.missed ? 0.45 : 1;
    }
    ctx.strokeStyle = col; ctx.lineWidth = lw * 1.5;
    const c = Math.min(t.w, t.h) * 0.22;
    ctx.beginPath();
    [[t.x, t.y, 1, 1], [t.x + t.w, t.y, -1, 1], [t.x, t.y + t.h, 1, -1], [t.x + t.w, t.y + t.h, -1, -1]].forEach(([x, y, dx, dy]) => {
      ctx.moveTo(x + dx * c, y); ctx.lineTo(x, y); ctx.lineTo(x, y + dy * c);
    });
    ctx.stroke();
    if (t.mesh && meshTris && $("optMesh").checked && !t.missed) drawMesh(t.mesh, col, lw);
    if ($("optPoints").checked) {
      ctx.fillStyle = col;
      for (const [x, y] of t.kp) { ctx.beginPath(); ctx.arc(x, y, lw * 1.4, 0, Math.PI * 2); ctx.fill(); }
      if (t.pose && $("optPose").checked && !t.missed && !(t.body && $("optSkeleton").checked)) drawBody(t.pose, col, lw);
    }
    if (t.body && $("optSkeleton").checked && !t.missed) drawSkeleton(t.body, col, lw);
    if (t.hands && $("optFingers").checked && !t.missed) for (const h of t.hands) drawHand(h, col, lw);
    // Name overlaid on the face itself: centred band across the top of the head
    const cx = t.x + t.w / 2;
    const size = Math.max(14, Math.min(t.w / 5.5, canvas.width / 30));
    ctx.save();
    if (mirrored) { ctx.translate(cx, 0); ctx.scale(-1, 1); ctx.translate(-cx, 0); }   // keep text readable when mirrored
    ctx.textAlign = "center"; ctx.textBaseline = "middle";
    ctx.font = `700 ${size}px system-ui, "Segoe UI", sans-serif`;
    const name = unknown ? "Unknown" : t.name;
    const pct = unknown ? "" : `${Math.round(t.sim * 100)}%`;
    const nw = ctx.measureText(name).width;
    ctx.font = `600 ${size * 0.62}px ui-monospace, Consolas, monospace`;
    const pw = pct ? ctx.measureText(pct).width : 0;
    const xt = exprText(t), xw = xt ? ctx.measureText(xt).width : 0;
    const bw = Math.max(nw, pw, xw) + size * 1.1, bh = size * (1.45 + (pct ? 0.6 : 0) + (xt ? 0.7 : 0));
    const by = Math.max(0, t.y - bh * 0.45);        // across the forehead / top of the head
    ctx.fillStyle = "rgba(8, 19, 15, 0.62)";
    ctx.beginPath(); ctx.roundRect ? ctx.roundRect(cx - bw / 2, by, bw, bh, size * 0.35) : ctx.rect(cx - bw / 2, by, bw, bh); ctx.fill();
    ctx.fillStyle = col; ctx.fillRect(cx - bw / 2, by, bw, Math.max(2, size * 0.12));
    ctx.font = `700 ${size}px system-ui, "Segoe UI", sans-serif`;
    ctx.fillStyle = "#ffffff"; ctx.fillText(name, cx, by + size * 0.78);
    if (pct) { ctx.font = `600 ${size * 0.62}px ui-monospace, Consolas, monospace`; ctx.fillStyle = col; ctx.fillText(pct, cx, by + size * 1.6); }
    if (xt) {   // expression under the name: bright when it departs from the person's usual look, dim otherwise
      ctx.font = `600 ${size * 0.62}px ui-monospace, Consolas, monospace`;
      ctx.fillStyle = t.expr.state === "change" || t.expr.state === "plain" ? "#ffffff" : "rgba(255,255,255,0.6)";
      ctx.fillText(xt, cx, by + size * (pct ? 2.3 : 1.6));
    }
    ctx.restore();
  }
  ctx.globalAlpha = 1;
  if (test?.prompt) {
    const size = Math.max(20, canvas.width / 24);
    ctx.save();
    if (mirrored) { ctx.translate(canvas.width, 0); ctx.scale(-1, 1); }
    ctx.font = `700 ${size}px system-ui, "Segoe UI", sans-serif`; ctx.textAlign = "center"; ctx.textBaseline = "middle";
    const w = ctx.measureText(test.prompt).width + size * 1.2;
    ctx.fillStyle = test.label ? "rgba(14, 159, 110, 0.85)" : "rgba(8, 19, 15, 0.75)";
    ctx.fillRect((canvas.width - w) / 2, size * 0.5, w, size * 1.6);
    ctx.fillStyle = "#ffffff"; ctx.fillText(test.prompt, canvas.width / 2, size * 1.3);
    ctx.restore();
  }
  const now = performance.now();
  if (now - fpsT > 1000) { $("fps").textContent = Math.round(checks * 1000 / (now - fpsT)); checks = 0; fpsT = now; }
  streamFrame();
}

// Face mesh: MediaPipe's points joined into triangles. Each triangle is shaded by the angle it
// faces (its 3D normal, from the points' depth): facing the camera is clear, turned away is darker,
// so the shading follows the shape and turn of the head. Then all the edges in the person's colour.
// The triangles are drawn in MESH_SHADES groups of similar shade, one fill per group: a fill per
// triangle (852 a face) made the page crawl with several people in view.
const MESH_SHADES = 8;
function drawMesh(m, col, lw) {
  ctx.save();
  const base = ctx.globalAlpha;
  const groups = Array.from({ length: MESH_SHADES }, () => new Path2D());
  for (let k = 0; k < meshTris.length; k += 3) {
    const a = meshTris[k] * 3, b = meshTris[k + 1] * 3, c = meshTris[k + 2] * 3;
    const ux = m[b] - m[a], uy = m[b + 1] - m[a + 1], uz = m[b + 2] - m[a + 2];
    const vx = m[c] - m[a], vy = m[c + 1] - m[a + 1], vz = m[c + 2] - m[a + 2];
    const nx = uy * vz - uz * vy, ny = uz * vx - ux * vz, nz = ux * vy - uy * vx;
    const len = Math.hypot(nx, ny, nz);
    if (!len) continue;
    const shade = (1 - Math.abs(nz) / len) ** 1.5;     // 0 = faces the camera, 1 = edge-on
    const g = Math.round(shade * (MESH_SHADES - 1));
    if (!g) continue;                                  // facing the camera: clear, nothing to fill
    const p = groups[g];
    p.moveTo(m[a], m[a + 1]); p.lineTo(m[b], m[b + 1]); p.lineTo(m[c], m[c + 1]); p.closePath();
  }
  // Mesh strength (Light / Normal / Strong) scales the shading and the lines. Normal was made stronger on
  // 2026-11-09: at 55% and under a pixel wide the lines vanished on some screens (seen on a 1280x720 camera).
  const k = Math.max(0.3, Math.min(2, +$("optMeshStrength").value || 1));
  ctx.fillStyle = "#08130f";
  for (let g = 1; g < MESH_SHADES; g++) { ctx.globalAlpha = Math.min(1, base * 0.5 * k * g / (MESH_SHADES - 1)); ctx.fill(groups[g]); }
  ctx.globalAlpha = Math.min(1, base * 0.8 * k); ctx.strokeStyle = col;
  ctx.lineWidth = Math.max(1, canvas.width / 900) * Math.max(0.75, k);
  ctx.beginPath();
  for (let k = 0; k < meshTris.length; k += 3) {
    const a = meshTris[k] * 3, b = meshTris[k + 1] * 3, c = meshTris[k + 2] * 3;
    ctx.moveTo(m[a], m[a + 1]); ctx.lineTo(m[b], m[b + 1]); ctx.lineTo(m[c], m[c + 1]); ctx.closePath();
  }
  ctx.stroke(); ctx.restore();
}

// Body lines: shoulders, arms, torso (points in POSE_POINTS' order; only those seen)
const BONES = [[3, 4], [3, 5], [5, 7], [7, 9], [4, 6], [6, 8], [8, 10], [3, 11], [4, 12], [11, 12]];
function drawBody(p, col, lw) {
  ctx.save(); ctx.strokeStyle = col; ctx.lineWidth = lw; ctx.globalAlpha *= 0.7; ctx.beginPath();
  for (const [a, b] of BONES) if (p[a][2] >= 0.5 && p[b][2] >= 0.5) { ctx.moveTo(p[a][0], p[a][1]); ctx.lineTo(p[b][0], p[b][1]); }
  ctx.stroke(); ctx.restore();
}

// Full body skeleton: MediaPipe's 33 pose points (face, arms with rough hands, torso, legs and feet); only seen ones
// The face's points (0-10: nose, eyes, ears, mouth) aren't drawn: the face has its own keypoints and mesh, and the
// owner found lines and dots over it clutter.
const SKELETON_FROM = 11;
const SKELETON = [[11, 12], [11, 23], [12, 24], [23, 24],                                                             // torso
  [11, 13], [13, 15], [15, 17], [15, 19], [15, 21], [17, 19], [12, 14], [14, 16], [16, 18], [16, 20], [16, 22], [18, 20],
  [23, 25], [25, 27], [27, 29], [29, 31], [27, 31], [24, 26], [26, 28], [28, 30], [30, 32], [28, 32]];   // legs, feet
function drawSkeleton(p, col, lw) {
  const seen = i => p[i] && p[i][2] >= 0.5;
  ctx.save(); ctx.strokeStyle = col; ctx.fillStyle = col; ctx.lineWidth = lw; ctx.globalAlpha *= 0.8; ctx.beginPath();
  for (const [a, b] of SKELETON) if (seen(a) && seen(b)) { ctx.moveTo(p[a][0], p[a][1]); ctx.lineTo(p[b][0], p[b][1]); }
  ctx.stroke();
  for (let i = SKELETON_FROM; i < p.length; i++) if (seen(i)) { ctx.beginPath(); ctx.arc(p[i][0], p[i][1], lw * 1.1, 0, Math.PI * 2); ctx.fill(); }
  ctx.restore();
}

// Hand skeleton: 21 points (wrist, then each finger from the palm out: thumb, index, middle, ring, pinky)
const HAND = [[0, 1], [1, 2], [2, 3], [3, 4], [0, 5], [5, 6], [6, 7], [7, 8], [5, 9], [9, 10], [10, 11], [11, 12],
  [9, 13], [13, 14], [14, 15], [15, 16], [13, 17], [0, 17], [17, 18], [18, 19], [19, 20]];
function drawHand(p, col, lw) {
  if (p.length < 21) return;
  ctx.save(); ctx.strokeStyle = col; ctx.fillStyle = "#ffffff"; ctx.lineWidth = Math.max(1.5, lw * 0.75); ctx.beginPath();
  for (const [a, b] of HAND) { ctx.moveTo(p[a][0], p[a][1]); ctx.lineTo(p[b][0], p[b][1]); }
  ctx.stroke();
  for (const [x, y] of p) { ctx.beginPath(); ctx.arc(x, y, Math.max(1.5, lw * 0.6), 0, Math.PI * 2); ctx.fill(); }
  ctx.restore();
}

function renderList() {
  const live = tracks.filter(t => t.missed === 0);
  $("count").textContent = live.length;
  $("known").textContent = live.filter(t => t.name !== "Unknown").length;
  const ul = $("faces");
  if (!tracks.length) { ul.innerHTML = '<li class="none">No faces tracked</li>'; return; }
  const showEv = $("optEvidence").checked && $("optExpr").checked, now = performance.now();
  // One block a face: name and match on top, expression and size below; with evidence on, the readings in
  // labeled cells that keep their place (a dash when there's no reading), so changing numbers are easy to follow.
  ul.innerHTML = tracks.map(t => {
    const unk = t.name === "Unknown", pct = Math.round(t.sim * 100);
    const xt = exprText(t);
    let html = `<li class="face${t.missed ? " lost" : ""}">` +
      `<div class="fhead"><span class="sw" style="background:${colorFor(t.name)}"></span>` +
      `<span class="who${unk ? " unk" : ""}">${esc(t.name)}</span>` +
      `<span class="match">${unk ? "" : pct + "%"}</span></div>` +
      (unk ? "" : `<div class="meter"><i style="width:${Math.max(0, Math.min(100, pct))}%"></i></div>`) +
      `<div class="fsub"><span class="${xt ? t.expr.state : "none"}">${xt ? esc(xt) : $("optExpr").checked ? "no expression yet" : ""}</span>` +
      `<span>${Math.round(t.tw || t.w)} px${t.missed ? " · lost" : ""}</span></div>`;
    if (showEv) {
      const sig = Signals.fields(t.fs, now, voiceFor(t, now), t.talking);
      const skip = !t.expr && t.xt.skip ? `no reading: ${t.xt.skip}` : "";
      // each group is shown by its setting, never by whether it has a reading right now, so the blocks keep their height
      html += group("Expression", Expression.fields(t.expr), skip) + group("Face", sig.face) +
              ($("optVoice").checked ? group("Voice", sig.voice) : "") +
              ($("optPose").checked ? group("Body", Signals.bodyFields(t.fs, now)) : "") +
              ($("optCalib").checked ? group("Their normal", Signals.normalFields(baselines, t.name, t.snap, t.ctx)) : "");
    }
    return html + "</li>";
  }).join("");
  $("devices").hidden = !showEv;
}

// A labeled group of readout cells: [label, value, hot, wide]; value null shows a dash, wide spans the row.
function group(title, cells, note) {
  return `<div class="grp"><div class="gl">${esc(title)}${note ? ` <em>${esc(note)}</em>` : ""}</div><div class="cells">` +
    cells.map(([k, v, hot, wide]) => `<div class="cell${wide ? " wide" : ""}"><span>${esc(k)}</span>` +
      `<b class="${v == null ? "na" : hot ? "hot" : ""}" title="${esc(v ?? "no reading")}">${v == null ? "–" : esc(v)}</b></div>`).join("") +
    "</div></div>";
}

// ---- This camera for the other devices (the peer service): the video with every marker drawn on it, and the
// live readout as data (drawn on the other side with text only). Sent with a shared database while "Stream this
// camera" is on, at full rate (STREAM_FPS frames a second) all the time, watched or not.
const comp = document.createElement("canvas"), cctx = comp.getContext("2d");
let ocomp = null, octx = null;             // the same picture on a canvas whose picture can be handed to the worker
let streamBusy = false, streamNext = 0, streamOff = 0, streamWhy = "";
const streamFailed = () => streamWhy.replace(/[^\x20-\x7e]/g, "?").slice(0, 200);
// the camera's picture with every marker on it, as this page shows it, on a canvas of the stream's size
function streamPicture(c, x) {
  const s = Math.min(1, 960 / video.videoWidth), w = Math.round(video.videoWidth * s), h = Math.round(video.videoHeight * s);
  if (c.width !== w || c.height !== h) { c.width = w; c.height = h; } else x.clearRect(0, 0, w, h);
  x.save();
  if ($("optMirror").checked) { x.translate(w, 0); x.scale(-1, 1); }                    // as this page shows it
  x.drawImage(video, 0, 0, w, h);
  x.drawImage(canvas, 0, 0, w, h);                                                      // names, boxes, markers
  x.restore();
}
// the canvas whose picture is handed to the worker, where a browser has one (false: it has none)
function streamCanvas() {
  if (octx === null) {
    try { ocomp = new OffscreenCanvas(2, 2); octx = (ocomp.transferToImageBitmap && ocomp.getContext("2d")) || false; } catch { octx = false; }
  }
  return octx;
}
// how a frame the worker sent went (streamByWorker): what the request's answer said
function streamTold(r) {
  const now = performance.now();
  if (r.off) streamOff = now + 2000;                                  // not shared now: ask again in 2 s
  else if (r.error) { streamWhy = String(r.error).slice(0, 200); streamOff = now + 500; }
}
// The way a frame leaves the page. "transfer": the picture is handed to the worker in the same turn, and the
// worker makes and sends the JPEG. "page": made and sent by the page itself, four turns of its thread. By the
// worker wherever there is one with a canvas, measured with a face in view and every MediaPipe part still in the
// page: 9.4 frames a second from the page and 20 by the worker in Chromium's window (2026-11-23), 15.3 and 19.3
// in WebKit's on a Mac (2026-11-24). The server names the page's way where its device was measured to lose by
// the worker's (an iPhone: /api/people's `stream`), and either way to compare the two on a device (FACE_STREAM).
let streamAsked = "";
const streamWay = () => streamAsked || "transfer";
async function streamFrame() {
  const now = performance.now();
  if (!running || !$("optStream").checked || streamBusy || now < streamNext || now < streamOff || !video.videoWidth) return;
  const way = streamWay(), paced = () => { streamWhy = ""; streamNext = Math.max(streamNext + STREAM_EVERY, now - STREAM_EVERY); };
  // where the worker makes and sends the frame, the page is done with it in this one turn
  if (way === "transfer" && streamCanvas()) {
    const how = streamByWorker(() => { streamPicture(ocomp, octx); return ocomp.transferToImageBitmap(); }, 0.72, streamWhy && streamFailed(), streamTold);
    if (how === "sent") paced();
    if (how !== "no") return;
  }
  streamBusy = true;
  try {
    streamPicture(comp, cctx);
    const blob = await jpegOf(comp, 0.72);
    if (!blob) throw new Error("couldn't make the JPEG");
    const headers = { "Content-Type": "image/jpeg", "X-Face-Tool": "1" };
    if (streamWhy) headers["X-Stream-Failed"] = streamFailed();  // logged
    // never wait long: a request that doesn't come back must not stop the stream
    const r = await fetch("/api/stream/frame", { method: "POST", body: blob, headers, signal: AbortSignal.timeout(5000) });
    const j = await r.json().catch(() => ({}));
    streamWhy = "";
    if (j.stream === false) { streamOff = now + 2000; return; }       // not shared now: ask again in 2 s
    if (!r.ok) { streamWhy = `HTTP ${r.status} ${j.error || ""}`; streamOff = now + 500; return; }
    // full rate all the time. The next frame is due one period after this one was due (not after it began plus
    // whatever it took), so the pace holds whatever calls this: the display's refresh or the timer above
    streamNext = Math.max(streamNext + STREAM_EVERY, now - STREAM_EVERY);
  } catch (e) { streamWhy = `${e.name}: ${e.message}`; streamOff = now + 500; } finally { streamBusy = false; }
}
// Each face as the live readout shows it: name, match, colour, expression, size and the labeled groups of cells
function readoutData() {
  const showEv = $("optEvidence").checked && $("optExpr").checked, now = performance.now();
  return tracks.map(t => {
    const xt = exprText(t), unk = t.name === "Unknown";
    const f = { name: t.name, sim: unk ? null : Math.round(t.sim * 100), color: colorFor(t.name), lost: !!t.missed,
                expr: xt || "", exprState: xt ? t.expr.state : "", size: Math.round(t.tw || t.w), groups: [] };
    if (showEv) {
      const sig = Signals.fields(t.fs, now, voiceFor(t, now), t.talking);
      f.groups.push(["Expression", !t.expr && t.xt.skip ? `no reading: ${t.xt.skip}` : "", Expression.fields(t.expr)], ["Face", "", sig.face]);
      if ($("optVoice").checked) f.groups.push(["Voice", "", sig.voice]);
      if ($("optPose").checked) f.groups.push(["Body", "", Signals.bodyFields(t.fs, now)]);
      if ($("optCalib").checked) f.groups.push(["Their normal", "", Signals.normalFields(baselines, t.name, t.snap, t.ctx)]);
    }
    return f;
  });
}
setInterval(() => {                       // who this camera sees, and its readout: every second while it runs
  if (!running) return;
  const names = tracks.filter(t => !t.missed && t.name !== "Unknown").map(t => t.name);
  let readout = [], signals = null;
  try { if ($("optStream").checked) { readout = readoutData(); signals = signalsToSend(); } } catch {}
  const paused = camPaused() ? Math.max(1, Math.round((performance.now() - pausedAt) / 1000)) : 0;   // no new picture for
  fetch("/api/inview", { method: "POST", headers: { "Content-Type": "application/json", "X-Face-Tool": "1" },
                         body: JSON.stringify({ names, readout, paused, signals }) }).then(drop, () => {});
}, 1000);

// ---- The Signals chart (signals_chart.js): every sensor reading of this device over the last minutes, in the
// drawer the header's Signals button opens. Its readings are this page's own, one with every check of the camera's
// picture (as fast as this device checks), taken whether the drawer is open or not, and kept in the page's memory
// only (15 minutes). With Stream this camera on, every reading also goes to the other devices with the readout.
const r4 = x => x == null || !isFinite(x) ? null : Math.round(x * 1e4) / 1e4;
// One face's readings as numbers (signals.js numbers, plus recognition and expression), and the person's own usual
// range for each signal that has one.
function faceNumbers(t, now) {
  const n = Signals.numbers(t.fs, now, voiceFor(t, now), t.snap);
  const e = $("optExpr").checked ? t.expr : null, va = e ? e.vaFit || e.va : null, unk = t.name === "Unknown";
  const last = $("optExpr").checked && t.xt?.votes?.length ? t.xt.votes[t.xt.votes.length - 1].probs : null;
  const usual = {};
  if (!unk && $("optCalib").checked) for (const k of Object.keys(Signals.B_KEYS)) { const b = baselines.normal(t.name, k, t.ctx); if (b) usual[k] = [r4(b.m), r4(b.sd)]; }
  for (const k of Object.keys(n)) if (typeof n[k] === "number") n[k] = r4(n[k]);
  return { usual, nums: { ...n, match: unk ? null : r4(t.sim), quality: r4(t.quality), size: Math.round(t.tw || t.w),
                          probs: last ? Array.from(last, p => Math.round(p * 1000) / 1000) : null,
                          valence: va ? r4(va[0]) : null, arousal: va ? r4(va[1]) : null, share: e ? r4(e.share) : null } };
}
// The options of this page that make a section's readings, by the chart's word for each: a section without readings
// says which is off, and can switch it on.
const CHART_OPTIONS = { expression: "optExpr", voice: "optVoice", body: "optPose", pulse: "optPulse", normal: "optCalib" };
// A test device that is to send every signal it has (its app started with FACE_SIGNALS=all: the server says so):
// every option that makes readings is switched on as if by hand, so it is remembered too, and kept on. One whose
// model isn't on the device is left alone. Not for a device in everyday use: the microphone is listened to.
const ALL_SIGNALS = ["optExpr", "optCalib", "optEvidence", "optStream", "optVoice", "optPulse", "optPose", "optSoundChart"];
let allSignals = false;
// A test device whose location is to be shown and kept (its app started with FACE_LOCATION=on: the server says
// so): the location's switch is switched on once after the page loaded, as if by hand, so it is remembered too.
// The system still asks whoever is at the device for its own permission.
let locationOn = false, locationAsked = false;
// The same for a test device whose signal history is to be kept (FACE_HISTORY=on).
let keepOn = false, keepAsked = false;
setInterval(() => {
  if (locationOn) { locationOn = false; const box = $("optLocation"); if (box && !box.checked) box.click(); }
  if (keepOn) { keepOn = false; const box = $("optHistory"); if (box && !box.checked) box.click(); }
  if (!allSignals) return;
  for (const id of ALL_SIGNALS) { const box = $(id); if (box && !box.disabled && !box.checked) box.click(); }
}, 5000);
const chart = new SignalsChart.Panel($("chartPanel"), {
  turnOn: key => { const box = $(CHART_OPTIONS[key] || ""); if (box && !box.checked) box.click(); },
  open: (() => { try { const v = JSON.parse(recall("chart-open")); return Array.isArray(v) ? v : null; } catch { return null; } })(),
  onOpen: keys => remember("chart-open", JSON.stringify(keys)),
  window: +recall("chart-window") || undefined,
  onWindow: ms => remember("chart-window", ms),
});
function setChart(open, save = true) {
  $("chartPanel").hidden = !open;
  $("wrap").classList.toggle("charted", open);
  $("chartBtn").setAttribute("aria-expanded", open);
  if (save) remember("chart", open ? "1" : "0");
  if (open) chart.draw();
}
$("chartBtn").onclick = () => setChart($("chartPanel").hidden);
setChart(recall("chart") === "1", false);
// One reading: what this device itself measures (checks in the last second, how long the last one took, the
// strictness a match has to pass, the looks the recognizer names), every face in view, and what its audio interfaces
// heard while that is charted. Taken after each check; once a second when none is made (a stopped camera reads as a
// gap, and time goes on).
let recent = [], sampledAt = 0, soundHeard = null;   // recent: the last seconds' readings, as they go to the other devices
// The device itself: what it is, and how much of its processor and memory is in use, how warm it is, its battery.
// Asked of this page's own server every few seconds (which asks the system then, and keeps nothing); what the
// camera gives is the page's own to say. Each reading goes into the chart with the next check of the picture.
const HOST_EVERY = 3000;
let hostNow = {}, hostFacts = null, modelsRun = "";
function cameraFact() {
  const s = stream?.getVideoTracks?.()[0]?.getSettings?.();
  if (!s || !s.width || !s.height) return "";
  const facing = { user: "facing you", environment: "facing away" }[s.facingMode] || "";
  return [`${s.width}×${s.height}` + (s.frameRate ? ` at ${Math.round(s.frameRate)} pictures a second` : ""), facing].filter(Boolean).join(", ");
}
async function hostStep() {
  try {
    const j = await (await fetch("/api/device/live", { headers: { "X-Face-Tool": "1" } })).json();
    if (j && j.reading && typeof j.reading === "object") hostNow = j.reading;
    if (j && j.facts && typeof j.facts === "object") hostFacts = j.facts;
  } catch { hostNow = {}; }                     // (no answer: a gap, not the last reading for ever)
}
setInterval(hostStep, HOST_EVERY);
hostStep();
// What this node publishes of itself on its bus (its own server's /api/bus): every source and signal there is
// (the chart's own catalog), what is on and what is not, and every setting of this page with its value. Each is
// sent when it changes, and again when the server says it no longer has it (it began again). The readings go
// with the readout as they always did; the server publishes them from there, each with its own time.
const BUS_EVERY = 2000, BUS_ASK_EVERY = 15;
const busSaid = {};
let busSteps = 0;
function busState() {
  const off = Object.keys(CHART_OPTIONS).filter(key => !$(CHART_OPTIONS[key]).checked);
  if (!poseOn) off.push("nopose");
  if (voiceWhy && $("optVoice").checked) off.push(voiceWhy);
  if (!cuesOn) off.push("nocues");
  return { running, paused: running && camPaused(), off, sound: $("optSoundChart").checked, streaming: $("optStream").checked };
}
async function busSettings() {
  const out = {};
  for (const s of await SettingsBridge.describe()) {
    if (!s.id || ["button", "form", "info"].includes(s.type) || s.secret) continue;     // (settings, not actions; never a secret)
    const o = { section: s.section, label: s.label, type: s.type, value: s.value ?? null };
    for (const key of ["def", "options", "min", "max", "step", "hint"]) if (s[key] != null) o[key] = s[key];
    if (s.disabled) o.disabled = true;
    out[s.id] = o;
  }
  return out;
}
async function busStep() {
  const now = {};
  try {
    now.catalog = SignalsChart.catalog();
    now.state = busState();
    now.settings = await busSettings();
  } catch { return; }
  const set = {}, text = {};
  for (const [key, value] of Object.entries(now)) { text[key] = JSON.stringify(value); if (busSaid[key] !== text[key]) set[key] = value; }
  // (nothing changed: now and then an empty question, whose answer says what the server still has)
  if (!Object.keys(set).length && ++busSteps % BUS_ASK_EVERY) return;
  try {
    const r = await fetch("/api/bus", { method: "POST", headers: { "Content-Type": "application/json", "X-Face-Tool": "1" }, body: JSON.stringify({ set }) });
    const j = await r.json();
    if (!r.ok) return;
    for (const key of Object.keys(set)) busSaid[key] = text[key];
    for (const key of Object.keys(now)) if (Array.isArray(j.has) && !j.has.includes(key)) delete busSaid[key];
  } catch {}
}
setInterval(busStep, BUS_EVERY);
// The readings themselves, published as they are taken: every BUS_READINGS_EVERY the readings since the last
// time, each with its own time (the same packing as with a readout). A subscriber is told the moment they are
// published, so another device's chart moves with this device and not once a second. A batch that doesn't
// arrive is a gap of a moment: the readout still carries the last seconds for a device that reads those.
const BUS_READINGS_EVERY = 250, BUS_READINGS_MOST = 400;
const busFresh = [];
let busSending = false, busSoundSeq = null;
async function busReadings() {
  // (the readings say who is in view and how they look: they go out only while this camera is streamed to the
  // other devices, as the readout does; what was taken meanwhile is not kept for later)
  if (!$("optStream").checked) { busFresh.length = 0; return; }
  if (busSending || !busFresh.length) return;
  busSending = true;
  try {
    const heard = soundHeard && $("optSoundChart").checked && soundHeard.seq !== busSoundSeq ? soundHeard : null;
    const sound = heard ? { seq: heard.seq, age: Math.round((Date.now() - heard.t) / 100) / 10, interfaces: heard.interfaces } : null;
    const readings = SignalsChart.pack(busFresh.splice(0), Date.now(), sound);
    const r = await fetch("/api/bus", { method: "POST", headers: { "Content-Type": "application/json", "X-Face-Tool": "1" }, body: JSON.stringify({ set: { readings } }) });
    if (r.ok && heard) busSoundSeq = heard.seq;
    r.body?.cancel();                                   // (the answer says nothing this needs: let go, not left unread)
  } catch {} finally { busSending = false; }
}
setInterval(busReadings, BUS_READINGS_EVERY);
function chartSample() {
  const now = performance.now(), at = Date.now();
  while (checkTimes.length && now - checkTimes[0] > 1000) checkTimes.shift();
  let faces = [];
  // (look and state: what the face's reading settled on, for the history's rows; the chart takes no notice of them)
  try {
    if (running && !camPaused()) faces = tracks.filter(t => !t.missed).map(t => ({ name: t.name, ...faceNumbers(t, now),
      look: $("optExpr").checked ? t.expr?.label : undefined, state: $("optExpr").checked ? t.expr?.state : undefined }));
  } catch {}
  // what is switched off here, or has no model on this device: the sections it leaves empty say so
  const off = Object.keys(CHART_OPTIONS).filter(k => !$(CHART_OPTIONS[k]).checked);
  if (!poseOn) off.push("nopose");
  if (voiceWhy && $("optVoice").checked) off.push(voiceWhy);
  if (!cuesOn) off.push("nocues");
  const device = { checks: running ? checkTimes.length : null, check: running ? r4(checkMs) : null, strict: +$("optStrict").value, labels: expressions || undefined, off,
                   load: hostNow.load, memory: hostNow.memory, temp: hostNow.temp, heat: hostNow.heat, battery: hostNow.battery, charging: hostNow.charging,
                   facts: hostFacts ? { ...hostFacts, models: modelsRun, camera: running ? cameraFact() : "" } : undefined };
  const sound = soundHeard && $("optSoundChart").checked ? soundHeard : null;
  chart.sample(at, { device, faces, sound });
  if (historyOn()) { try { signalRows.add(at, { device, faces, sound }); } catch {} }   // (measured: millionths of a second a reading)
  recent.push({ t: at, device, faces });
  while (recent.length && at - recent[0].t > SignalsChart.C.SENT_MS) recent.shift();
  busFresh.push({ t: at, device, faces });                          // (for the bus: published within a moment)
  if (busFresh.length > BUS_READINGS_MOST) busFresh.splice(0, busFresh.length - BUS_READINGS_MOST);
  sampledAt = now;
}
setInterval(() => { if (performance.now() - sampledAt > 900) chartSample(); }, 1000);
// The signal history (the owner, 2026-11-24: "Keep a history of ALL signals in the database", opt-in on each device,
// and: "This should NOT slow down the FPS at all"). While its switch is on, every reading above also goes into the
// history's rows (SignalsChart.History: one row every 10 s for the device and for each named person in view). The
// page looks every HISTORY_SEND_MS whether a row is finished and hands it to its own server at once, which checks it
// and gives it to its writer; the writer has a thread and a database connection of its own and paces itself: at once
// with nobody in view, and with a face in view as often as costs the camera nothing that can be measured (the owner,
// 2026-11-25: "get to / from the databases as fast as possible while slowing down the FPS the leasts amount"). Here
// that is a few millionths of a second a reading and one small request every 10 s (measured: the guide has the
// numbers). Nothing else in the app reads the history. Off, nothing of it runs here at all.
const HISTORY_SEND_MS = 1000;              // a choice: a row is finished every 10 s, and is on its way within this (a look that finds none is a comparison)
const HISTORY_ASK_MS = 30000;              // with nothing to send: how often the writer is asked what it did, for the line here
const signalRows = new SignalsChart.History();   // (not "history": that is the browser's own word)
function historyOn() { return $("optHistory").checked; }
let historyState = "", historyAsked = 0;
function historyShow() {
  const says = $("historyState"), text = historyOn() ? historyState : "";
  says.hidden = !text;
  if (says.textContent !== text) says.textContent = text;
}
async function historyStep() {
  if (!historyOn()) { signalRows.take(Date.now()); historyState = ""; historyAsked = 0; historyShow(); return; }      // (rows made before it was switched off are let go)
  const rows = signalRows.take(Date.now()), now = performance.now();
  if (!rows.length && historyAsked && now - historyAsked < HISTORY_ASK_MS) return;      // nothing is finished, and the line is fresh
  try {
    // (one request: the rows, answered with what the writer did; or only the question, when there are none)
    const r = rows.length
      ? await fetch("/api/signal-log", { method: "POST", headers: { "Content-Type": "application/json", "X-Face-Tool": "1" }, body: JSON.stringify({ rows }) })
      : await fetch("/api/signal-log", { headers: { "X-Face-Tool": "1" } });
    const s = await r.json();
    historyAsked = now;
    const n = v => (typeof v === "number" && isFinite(v) ? v : 0).toLocaleString();
    historyState = !s.joined ? "Nothing is kept yet: this device has no shared database."
      : s.trouble ? `The shared database can't be written to now; the rows wait on this device (${n(s.waited)} so far).`
      : `Since the app started: ${n(s.written)} rows kept${s.batches ? `; they reach the database ${n(Math.round(s.averageLagMs / 1000))} s after they are made, and a writing takes ${n(s.averageMs)} ms on average` : ""}. Kept for ${n(s.days)} days.`;
  } catch { /* the next look */ }
  historyShow();
}
setInterval(historyStep, HISTORY_SEND_MS);
for (const id of ["optHistory", "optHistoryDays"]) $(id).addEventListener("change", () => { historyShow(); setTimeout(historyStep, 1500); });
// The last seconds' readings for the other devices' charts, each with its own time (they overlap from one readout
// to the next, so a readout that is lost on the way loses no reading); thinned when there are too many to send.
function signalsToSend() {
  const sound = soundHeard && $("optSoundChart").checked
    ? { seq: soundHeard.seq, age: Math.round((Date.now() - soundHeard.t) / 100) / 10, interfaces: soundHeard.interfaces } : null;
  const sig = SignalsChart.pack(recent, Date.now(), sound);
  while (sig.rows.length > 4 && JSON.stringify(sig).length > 120000) sig.rows = sig.rows.filter((r, i) => i % 2);
  return sig;
}
// This device's sound for the chart: only while its switch is on. The page asks its server every few seconds; for
// as long as it is asked the server listens to every audio interface, a few seconds at a time, one stretch after
// another, and stops by itself when the asking stops (the switch off, or this page closed). Numbers and one word
// come back; the sound is never kept.
// Where the program can't listen to the device's audio interfaces itself (a phone), its server says so and this
// page hears instead: every few seconds the last seconds of the microphone go to the page's own server, on the
// device itself, which measures them and answers with the same numbers and one word. Nothing of the sound is kept.
const HEARD_SECONDS = 4;
let heardN = 0, heardAt = -Infinity, heardBusy = false;
async function soundByMic() {
  if (!soundByPage) { soundByPage = true; micFailed = false; }
  micSync();
  if (micFailed || voiceWhy) { soundHeard = null; chart.say("Sound isn't charted: this device's microphone can't be used."); return; }
  const now = performance.now();
  if (!soundHeard) chart.say("Listening to this device's sound: the first reading comes in a few seconds.");
  if (!ear || heardBusy || now - heardAt < HEARD_SECONDS * 1000) return;
  const clip = ear.clip(HEARD_SECONDS);
  if (clip.length < ear.rate * 2) return;                 // (the measures want two seconds)
  heardBusy = true; heardAt = now;
  try {
    const r = await fetch(`/api/sound/heard?rate=${ear.rate}`, { method: "POST", body: clip.buffer,
                                                                headers: { "Content-Type": "application/octet-stream", "X-Face-Tool": "1" } });
    const j = await r.json();
    if (r.ok && j.reading) { soundHeard = { seq: "page-" + (++heardN), t: Date.now(), interfaces: [j.reading] }; chart.say(""); }
  } catch { /* the next stretch is tried */ } finally { heardBusy = false; }
}
async function soundStep() {
  const hint = $("soundChartHint");
  if (!$("optSoundChart").checked) {
    soundHeard = null; chart.say(""); hint.hidden = true;
    if (soundByPage) { soundByPage = false; micSync(); }   // (the microphone is let go of, unless voice still wants it)
    return;
  }
  try {
    const r = await fetch("/api/sound/live", { method: "POST", headers: { "Content-Type": "application/json", "X-Face-Tool": "1" }, body: "{}" });
    const j = await r.json();
    if (j.available === false && j.page) { hint.hidden = true; return soundByMic(); }
    const why = !r.ok || j.available === false ? (j.why || j.error || "This device can't listen to its sound.") : "";
    if (why) { soundHeard = null; chart.say(`Sound isn't charted: ${why}`); hint.textContent = why; hint.hidden = false; return; }
    hint.hidden = true;
    if (Array.isArray(j.interfaces) && j.seq) soundHeard = { seq: j.seq, t: +j.at || Date.now(), interfaces: j.interfaces };
    chart.say(j.seq ? (j.interfaces.length ? "" : "No audio interface of this device could be listened to.") : "Listening to this device's sound: the first reading comes in a few seconds.");
  } catch { /* the server will be asked again */ }
}
setInterval(soundStep, 2000);
$("optSoundChart").addEventListener("change", soundStep);
// The expression log: what each named face looks like, every EXPR_LOG_EVERY, sent to this page's
// server, which keeps it in the shared database for reports (nothing is kept without one). The owner's decision of
// 2026-11-12; until then readings were shown and never stored. How a face looks, not what the person feels.
const EXPR_LOG_EVERY = 5000;               // ms between a person's readings: a choice, not a measured value
function exprLogRow(t) {
  const e = t.expr, va = e.vaFit || e.va, last = t.xt?.votes?.length ? t.xt.votes[t.xt.votes.length - 1].probs : null;
  return { t: Date.now(), name: t.name, label: e.label, share: e.share ?? null, state: e.state || "",
           data: { lead: e.lead ?? null, votes: e.votes ?? null, calibratedConfidence: e.cal ?? null,
                   valence: va ? va[0] : null, arousal: va ? va[1] : null, flags: (e.flags || []).join(", ") || null,
                   scores: last && expressions ? Object.fromEntries(expressions.map((l, i) => [l, last[i] ?? null])) : null,
                   setting: t.ctx || null, match: t.sim ?? null, quality: t.quality ?? null } };
}
setInterval(() => {
  if (!running || !$("optExprLog").checked || !$("optExpr").checked) return;
  let readings = [];
  try { readings = tracks.filter(t => !t.missed && t.name && t.name !== "Unknown" && t.expr?.label).map(exprLogRow); } catch {}
  if (readings.length) fetch("/api/expression-log", { method: "POST", headers: { "Content-Type": "application/json", "X-Face-Tool": "1" },
                                                      body: JSON.stringify({ readings }) }).then(drop, () => {});
}, EXPR_LOG_EVERY);
// Alerts the app raised itself (someone moved between cameras, from any device; a new device joined the shared
// database): shown like this page's own
let alertsSeenT = Date.now();
setInterval(async () => {
  try {
    const r = await alertApi("/api/alerts");
    const fresh = r.alerts.filter(a => (a.kind === "moved" || a.kind === "joined") && a.state === "new" && a.t > alertsSeenT);
    if (!fresh.length) return;
    alertsSeenT = Math.max(...fresh.map(a => a.t));
    for (const a of fresh) announce(a.text);
    showAlertCounts(r.counts, fresh[0].text);
    if (!$("alertsPanel").hidden) loadAlerts();
  } catch {}
}, 5000);

// "Keep this camera running": start it again within 10 s whenever it isn't running (not while a video file plays).
// Running means its picture arrives. Each of these is a failed try, and what the page does about it goes to the
// app's log (pageNote), since nobody may be looking at this page:
//   - a start that never finishes (seen on Linux: the window's camera service didn't start): failed after 30 s;
//   - a camera that was opened but never played, or is on and sends no picture for 30 s. An opened
//     camera once counted as running, so one that never played was left like that for good (seen on the NAS after its
//     page ran out of memory and was loaded again: no picture for six hours, and nothing in the log);
//   - a start that is refused at once (busy, or an error), six times in a row: a minute.
// After two of the first kinds, or a minute of the last, the page loads again; if the camera still isn't running
// after that, the app is asked to start again (it does at most hourly; asked again every STUCK_EVERY). Not for
// a camera that isn't there or isn't allowed (camAbsent: nothing here can change that), and a camera busy in
// another program gets the page reload only.
//   - a camera that is on but sends no new picture for 30 s (camPaused) is started again, and no more than that:
//     its next start counts as any start does.
// A locked screen (screenLocked; Windows): about 20 s after the lock Windows stops every running camera, and the
// video keeps its last picture. Every Remote Desktop disconnect locks the session too. What helps depends on the
// lock (all seen on the NAS, 2026-11-15):
//   - locked by a disconnect: a start from this page fails at once, but the app started again has the camera
//     within seconds, and it runs as ever;
//   - locked at the computer's own screen: no start finishes, in this app or a new one, until the unlock.
// The page can't tell the two apart, so: a camera paused under a lock is stopped and started at once (no 30 s
// wait: Windows stopped it for good); when that start fails or doesn't finish, the app is asked to start again,
// once (lockAsked; the app does it once per lock); after that one start is left open, and nothing more is loaded
// or started against the lock. Unlocked, the camera is started within 10 s as always.
const STUCK_EVERY = 600000;
async function screenLocked() {
  try { return (await (await fetch("/api/screen", { cache: "no-store", signal: AbortSignal.timeout(3000) })).json()).locked === true; }
  catch { return false; }
}
let keepStarting = false, keepHung = 0, keepFailed = 0, noPicture = 0, camNoted = "", stuckAt = 0;
let startOut = null;                       // the start this loop made that hasn't finished yet
let lockAsked = false;                     // the app was asked to start again under this lock
// No camera to start: the system refuses it to this page, or says there is none and lists none. "Not found" while
// one is listed is the camera service gone wrong, which loading the page or starting the app again does cure.
async function camAbsent() {
  if (camError === "NotAllowedError") return true;
  if (camError !== "NotFoundError" && camError !== "OverconstrainedError") return false;
  try { return !(await navigator.mediaDevices.enumerateDevices()).some(d => d.kind === "videoinput"); } catch { return true; }
}
function pageNote(text) {
  fetch("/api/page-note", { method: "POST", headers: { "Content-Type": "application/json", "X-Face-Tool": "1" },
                            body: JSON.stringify({ text }) }).then(drop, () => {});
}
setInterval(async () => {
  if (!$("optKeepCam").checked || keepStarting || video.src) return;
  let hung = false;
  if (stream && running && video.readyState >= 2 && !camPaused()) {   // its picture arrives: all is well
    if (camNoted) { pageNote("The camera is running again."); camNoted = ""; }
    noPicture = keepHung = keepFailed = 0;
    try { sessionStorage.removeItem("camHung"); } catch {}
    return;
  }
  keepStarting = true;                                   // one round at a time, also while the question is out
  const locked = await screenLocked();
  keepStarting = false;
  if (!locked) lockAsked = false;
  if (stream) {
    const still = camPaused();
    if (!(still && locked) && ++noPicture < 3) return;   // under a lock a paused camera doesn't come back by itself
    noPicture = 0;
    const what = still ? locked ? "The screen is locked and the camera's picture stopped: starting the camera again."
                                : "The camera sent no new picture for 30 s: starting it again."
               : running ? "The camera was on but sent no picture for 30 s: starting it again."
                         : "The camera was opened but didn't start playing in 30 s: starting it again.";
    if (what !== camNoted) pageNote(camNoted = what);    // once, not every round while it lasts
    camGen++;                                            // a start still waiting gives up quietly
    stopCamera();
    if (still) return;                                   // started again next round; no more than that for this
    hung = true;
  } else {
    if (locked && lockAsked && startOut) return;         // the one start left open: it finishes when Windows lets it
    keepStarting = true;
    try {
      const start = startOut = startCamera();
      start.finally(() => { if (startOut === start) startOut = null; });
      await Promise.race([start, new Promise((_, no) => setTimeout(() => { hung = true; no(new Error("hung")); }, 30000))]);
    } catch {} finally { keepStarting = false; }
    if (stream) return;                                  // opened: the next rounds look for its picture
    const why = hung ? "Starting the camera didn't finish in 30 s." : `The camera wouldn't start (${camError || "no reason given"}).`;
    if (locked) {
      if (!lockAsked) {                                  // once: the app decides (once per lock)
        lockAsked = true;
        pageNote(camNoted = `${why} The screen is locked: asking the app to start again, which it does once per lock.`);
        SettingsBridge.notify({ fs: "camstuck" });
      }
    } else if (why !== camNoted) pageNote((camNoted = why) + " Trying again every 10 s.");
  }
  if (locked) { keepHung = keepFailed = 0; return; }     // no page reload against a lock, and no second app restart
  if (hung ? ++keepHung < 2 : await camAbsent() || ++keepFailed < 6) return;
  let reloaded = false;
  try { reloaded = sessionStorage.getItem("camHung") === "1"; sessionStorage.setItem("camHung", "1"); } catch {}
  if (!reloaded) { pageNote("The camera still isn't running: loading the Face tracking page again."); location.reload(); }
  else if ((hung || camError !== "NotReadableError") && Date.now() - stuckAt >= STUCK_EVERY) {
    stuckAt = Date.now();                                // the app window starts the app again
    pageNote("The camera isn't running after the page was loaded again: asking the app to start again.");
    SettingsBridge.notify({ fs: "camstuck" });
  }
}, 10000);

function applyMirror() { $("stage").classList.toggle("mirror", $("optMirror").checked); }

$("camBtn").onclick = () => {
  if (stream && $("optKeepCam").checked) {           // stopping by hand: no longer kept running (Settings turns it on)
    $("optKeepCam").checked = false; $("optKeepCam").dispatchEvent(new Event("change"));
  }
  if (stream) remember("autostart", "0");
  startCamera();
};
$("fileBtn").onclick = () => $("fileInput").click();
$("fileInput").onchange = e => openFile(e.target.files[0]);
// A camera picked in the list is the owner's choice: what this device is set to from now on ("Default camera":
// none in particular, so whichever the system gives at the next start).
$("camSelect").onchange = async () => {
  chosenCam = $("camSelect").value; camFellBack = false; backTries = 0;
  remember("camera", chosenCam);
  if (stream) { stopCamera(); await startCamera(); }
};
// The camera this device is set to wasn't there when the camera was started, so the default one runs: as soon as
// the chosen one is listed again, it is gone back to. Looked for when the system says its cameras changed and
// every 10 s meanwhile; tried three times at the most for one page (a camera that is listed and still can't be
// opened must not have the running one started over and over).
async function backToChosen() {
  if (!camFellBack || !chosenCam || !stream || switching || backTries >= 3) return;
  let there = false;
  try { there = (await navigator.mediaDevices.enumerateDevices()).some(d => d.kind === "videoinput" && d.deviceId === chosenCam); } catch {}
  if (!there || !camFellBack || !stream) return;
  backTries++;
  pageNote("The camera this device is set to is there again: going back to it.");
  await listCameras(chosenCam);
  stopCamera(); await startCamera();
}
setInterval(backToChosen, 10000);
$("camSwitch").onclick = switchCamera;
$("optMirror").onchange = applyMirror;
// Controls panel: a fly-out on the right; closed, the video takes the whole window. Remembered;
// narrow windows start with it closed, as it covers the video there.
function setSide(open, save = true) {
  $("wrap").classList.toggle("closed", !open);
  $("side").inert = !open;                                    // hidden controls can't take keyboard focus
  $("sideBtn").setAttribute("aria-expanded", open);
  $("sideBtnText").textContent = open ? "Hide controls" : "Controls";
  if (save) remember("side", open ? "1" : "0");
}
const narrow = matchMedia("(max-width: 900px), (max-height: 500px) and (pointer: coarse)");   // a small window, or a phone held sideways
setSide(narrow.matches ? false : recall("side") !== "0", false);
$("sideBtn").onclick = () => setSide($("wrap").classList.contains("closed"));
addEventListener("keydown", e => {
  if (e.ctrlKey && e.key === ".") { e.preventDefault(); $("sideBtn").click(); }
  else if (e.key === "Escape" && narrow.matches && !$("wrap").classList.contains("closed")) setSide(false);
});
$("optMesh").checked = recall("mesh") === "1";                // remembered between visits
$("optMesh").onchange = () => remember("mesh", $("optMesh").checked ? "1" : "0");
// Alerts and presence log: remembered between visits
$("alertWho").value = recall("alertWho") || "";
$("alertWho").onchange = () => {
  remember("alertWho", $("alertWho").value);
  try { if ($("alertWho").value.trim() && window.Notification && Notification.permission === "default") Notification.requestPermission(); } catch {}
};
for (const [id, key] of [["optAlertUnknown", "alertUnknown"], ["optAlertSound", "alertSound"], ["optPresence", "presence"]]) {
  const v = recall(key);
  if (v !== null) $(id).checked = v === "1";
  $(id).addEventListener("change", () => remember(key, $(id).checked ? "1" : "0"));
}
$("presenceShow").onclick = showPresence;
addEventListener("pagehide", () => { for (const name of Object.keys(visitsOpen)) closeVisit(name, true); });   // log visits still open
$("optCollect").checked = recall("collect") === "1";
$("optCollect").onchange = () => remember("collect", $("optCollect").checked ? "1" : "0");
$("peopleCard").open = recall("people") === "1";             // collapsed unless you opened it last time
$("peopleCard").ontoggle = () => remember("people", $("peopleCard").open ? "1" : "0");
$("exprHint").dataset.on = $("exprHint").textContent;
$("optExpr").onchange = $("optCalib").onchange = $("optEvidence").onchange = renderList;
$("calibReset").onclick = async () => {
  const who = $("forgetWho").value;
  if (!await askUser(who ? `Forget everything learned about ${who} (usual look and normal signals)?` : "Forget everything learned about everyone?", "Forget")) return;
  usualLooks.forget(who || undefined); baselines.forget(who || undefined);
  renderForget(); renderList();
};
renderForget();
addEventListener("pagehide", () => { baselines.save(); usualLooks.save(); });   // keep the last few seconds' learning
$("testBtn").onclick = runTest;
$("optVoice").onchange = () => { micFailed = false; micSync(); };
$("optState").onchange = $("optTalkGate").onchange = $("optHandGate").onchange = $("optPose").onchange = renderList;   // from the next check
$("feelBtn").onclick = openFeel;
$("feelSave").onclick = saveFeel;
$("feelSkip").onclick = closeFeel;
$("feelV").oninput = e => $("feelVOut").textContent = e.target.value;
$("feelA").oninput = e => $("feelAOut").textContent = e.target.value;
$("feelEvery").value = recall("feel") || "0";
$("feelEvery").onchange = () => { remember("feel", $("feelEvery").value); lastAsk = performance.now(); };
$("testDelete").onclick = async () => {
  if (!await askUser("Delete all saved expression test sessions and feeling reports?", "Delete")) return;
  try { drop(await fetch("/api/expr-eval/delete", { method: "POST", headers: { "X-Face-Tool": "1" } })); } catch {}
  loadPeople();
};
$("optSmooth").oninput = e => $("smoothVal").textContent = (+e.target.value).toFixed(2);
$("optStrict").oninput = e => { threshTouched = true; $("threshVal").textContent = (+e.target.value).toFixed(2); };
navigator.mediaDevices?.addEventListener?.("devicechange", () => { listCameras(); backToChosen(); });
// Remember the camera and start it automatically next time (unless you pressed Stop camera)
function remember(k, v) { if (window.PageStore) PageStore.set("ft-" + k, String(v)); else try { localStorage.setItem("ft-" + k, v); } catch {} }
function recall(k) { try { return localStorage.getItem("ft-" + k); } catch { return null; } }
// The other options are remembered between visits too (the camera's mirror view follows the source instead)
const KEEP = ["optPoints", "optTrail", "optUnknown", "optBlur", "optExpr", "optCalib", "optEvidence", "optExprLog", "optVoice", "optPulse",
              "optTalkGate", "optState", "optPose", "optHandGate", "optSoundChart", "optSkeleton", "optFingers", "optSmooth", "optStrict", "optMeshStrength",
              "optAlertArrive", "optAlertLeave", "optAlertMove", "optStream", "optKeepCam", "optAlertUnusual", "optArriveGap", "optLeaveGap", "optUnknownStay",
              "optUnusualZ", "optUnusualFor", "optUnusualRepeat", "optDetailsAfter", "optBannerFor", "optLocation", "optPlace", "optHistory", "optHistoryDays"];
for (const id of KEEP) {
  const el = $(id), v = recall("o-" + id);
  if (v !== null) {
    if (el.type === "checkbox") el.checked = v === "1"; else { el.value = v; el.dispatchEvent(new Event("input")); }
  }
  el.addEventListener("change", () => remember("o-" + id, el.type === "checkbox" ? (el.checked ? "1" : "0") : el.value));
}
micSync();
// This device's location (place.js), while its owner has switched that on (off for a new copy). The page asks its
// own server every few seconds: the server asks the system's location service, or reads the place typed here, and
// answers with where the device is or why it can't say. Shown as one line of text. The server publishes the
// location to the owner's other devices and writes the history's rows; this page only shows it.
const PLACE_EVERY = 5000;
let placeNow = null, placeWhy = "", placeNote = "";
function placeText() { return placeNow ? Place.words(placeNow) : placeWhy; }
function placeShow() {
  const on = $("optLocation").checked, text = on ? placeText() : "", line = $("placeLine");
  line.hidden = !text;
  if (line.textContent !== text) line.textContent = text;
  line.classList.toggle("why", !!text && !placeNow);
  // a device that is placed by Wi-Fi says so with its place (its server's words, put in as text)
  const note = on && placeNow ? placeNote : "", says = $("placeNote");
  says.hidden = !note;
  if (says.textContent !== note) says.textContent = note;
  SettingsBridge.changed();                                    // the Settings tab's "This device is"
}
// (while the switch is off the server is not asked every few seconds: only when the switch or the typed place
// changes, so that it hears of the change at once)
async function placeStep(changed) {
  if (changed !== true && !$("optLocation").checked) { placeNow = null; placeWhy = placeNote = ""; placeShow(); return; }
  let j = null;
  try { j = await (await fetch("/api/location", { headers: { "X-Face-Tool": "1" } })).json(); } catch {}
  placeNow = j && j.on ? Place.clean(j.place) : null;
  placeWhy = j && j.on && !placeNow && typeof j.why === "string" ? j.why.slice(0, 400) : "";
  placeNote = j && j.on && placeNow && typeof j.note === "string" ? j.note.slice(0, 400) : "";
  placeShow();
}
setInterval(() => placeStep(), PLACE_EVERY);
// (a change reaches the server with the page's other settings, a moment later: asked then)
for (const id of ["optLocation", "optPlace"]) $(id).addEventListener("change", () => { placeShow(); setTimeout(() => placeStep(true), 800); });
placeStep();
for (const id of ["alertWho", "optAlertUnknown"]) $(id).addEventListener("change", () => SettingsBridge.changed());   // the Settings tab's list
// The app window's Settings tab (settings_bridge.js): every option of this page, in sections, each a card of the tab's
// groups (app_ui.js LAYOUT, which names every section here)
{
  const c = SettingsBridge.control, C = "Camera", D = "On screen", E = "Expression", S = "Voice, pulse, posture and sound",
        R = "Recognition", A = "Alerts", P = "Presence log", L = "Location", H = "Signal history";
  SettingsBridge.init("track", [
    c("camSelect", C, { noReset: true }),
    c("optMirror", C, { hint: "Turned on for the webcam and off for video files when the source starts." }),
    c("optPoints", D), c("optMesh", D), c("optMeshStrength", D),
    c("optSkeleton", D, { hint: "The body points the posture model finds below the face, joined: shoulders, arms with rough hands, hips, legs and feet. Drawn only; nothing is kept." }),
    c("optFingers", D, { hint: "The joints of every finger, 21 points a hand, from a second model (about 10-20 ms more a frame). Drawn only; nothing is kept." }),
    c("optTrail", D), c("optUnknown", D), c("optBlur", D),
    c("optSmooth", D, { hint: "How much the boxes are smoothed between frames." }),
    c("optExpr", E), c("optCalib", E, { hint: $("exprHint").textContent }), c("optEvidence", E),
    c("optExprLog", E, { hint: $("optExprLog").closest("label").title }),
    c("optTalkGate", E), c("optState", E),
    c("optHandGate", E, { hint: "The two \"skip\" options and the state model change the expression reading and aren't measured yet: they stay off until the Expression test shows they help." }),
    c("feelEvery", E, { label: "Ask how I feel (feeling check)" }),
    c("optVoice", S, { hint: "Analysed live and never recorded: loudness, pitch, speaking rate, and the voice model's arousal and valence." }),
    c("optPulse", S, { hint: "Reads tiny skin-colour changes; needs a still face in steady light." }),
    c("optPose", S, { hint: "Head height, shoulders, lean, body movement, breathing, hand on face and crossed arms." }),
    c("optSoundChart", S, { hint: "For the Signals chart: while this is on, every audio interface of this computer (each output: what it plays; each microphone: what sounds in the room) is listened to, 4 seconds at a time, one stretch after another, and measured in memory. Only numbers and one word (music, speech, quiet...) are kept, for 15 minutes; the sound itself is never kept. Turned off, or this page closed, the listening stops within seconds. With Stream this camera on, your other devices' Cameras tab charts the same numbers." }),
    c("optStrict", R, { hint: "Higher strictness means fewer wrong names but more \"Unknown\". The default is the measured best for the active model." }),
    c("optCollect", R, { hint: $("collectHint").textContent }),
    c("optKeepCam", C, { label: "Keep this camera running", hint: "Starts the camera when the app starts, and again within 10 s if it stops (unplugged, busy, an error), so this device's camera is always on in every device's Cameras tab. Stop camera turns this off." }),
    c("optStream", C, { label: "Stream this camera to my other devices", hint: "Your other devices can watch this camera live in their Cameras tab, as this page shows it (names, boxes and every marker that's on here), with its live readout. Sent all the time at full rate (about 20 frames a second), over your local network, encrypted, only to your own devices; never stored. Needs a shared database and Share my photos with my other devices." }),
    c("optHistory", H, { hint: $("historyHint").textContent }),
    c("optHistoryDays", H, { label: "Keep the history for (days)", hint: "Rows older than this many days are deleted from your shared database, once a day: this device's signal history and its location history. From 1 to 3650 days; 90 unless you set another number. A shorter time deletes the older rows at the next daily tidying." }),
    { section: H, id: "historyNow", label: "The history on this device", type: "info",
      get: () => historyOn() ? historyState || "Switched on: the first rows are sent within half a minute." : "Not kept: the switch above is off." },
    c("optLocation", L, { hint: $("placeHint").textContent }),
    // (secret: a typed place is never among the settings this node publishes. Where the device is goes out only
    // as its location, and only while the switch is on.)
    c("optPlace", L, { label: "This device's place, typed", secret: true, hint: "Leave it empty and the device's own location service is asked. Type a place and nothing is asked of anyone: a town (Springfield, or Springfield, Illinois, or Paris, France), or latitude and longitude (39.8017, -89.6437). Best for a computer that stays where it is, which its system often places only to within a few kilometres." }),
    { section: L, id: "placeNow", label: "This device is", type: "info",
      get: () => $("optLocation").checked ? [placeText() || "Not known yet.", placeNow ? placeNote : ""].filter(Boolean).join("\n") : "Not shown: the switch above is off." },
    { section: A, id: "alertList", label: "Alert me about these people", type: "multi",
      hint: "Pick the people to be told about (their arrive, leave and unusual-signal events, as switched on below). Unknown: a face the recognizer can't name stays the set time.",
      options: () => {
        const known = [...people].sort((a, b) => a.localeCompare(b)), low = new Set(known.map(p => p.toLowerCase()));
        const typed = $("alertWho").value.split(",").map(s => s.trim()).filter(s => s && !low.has(s.toLowerCase()));
        return [["Unknown", "Unknown (a face nobody is named for)"], ...known.map(p => [p, p]), ...typed.map(p => [p, p + " (not a known person now)"])];
      },
      get: () => {
        const want = alertNames(), typed = $("alertWho").value.split(",").map(s => s.trim()).filter(Boolean);
        const v = [...people.filter(p => want.has(p.toLowerCase())), ...typed.filter(s => !people.some(p => p.toLowerCase() === s.toLowerCase()))];
        return $("optAlertUnknown").checked ? ["Unknown", ...v] : v;
      },
      set: v => {
        $("alertWho").value = v.filter(x => x !== "Unknown").join(", ");
        $("alertWho").dispatchEvent(new Event("change"));
        $("optAlertUnknown").checked = v.includes("Unknown");
        $("optAlertUnknown").dispatchEvent(new Event("change"));
      } },
    c("optAlertArrive", A), c("optAlertLeave", A), c("optAlertMove", A, { label: "They move between cameras", hint: "Someone on your alert list was on another device's camera in the last 10 minutes and is now on this one (with a shared database). Only the people on the list are shared with your other devices for this, and only for 20 minutes. Every device shows the alert." }), c("optAlertUnusual", A), c("optAlertUnknown", A, { label: "An unknown face stays" }),
    c("optArriveGap", A), c("optLeaveGap", A), c("optUnknownStay", A), c("optUnusualZ", A), c("optUnusualFor", A),
    c("optUnusualRepeat", A), c("optDetailsAfter", A, { label: "Alert details: measure again after" }), c("optBannerFor", A),
    c("optAlertSound", A, { hint: $("alertHint").textContent }),
    c("optPresence", P, { hint: "Names and times only, no pictures, in %APPDATA%\\FaceLabels\\presence\\ on this computer." }),
  ], { forgetPerson, openAlerts });
}
// A person forgotten in the labeling tool (its "Their data…"; the app window's tab sends this): what this page
// learned about them lives only in its browser storage, so the page forgets it itself.
function forgetPerson({ name } = {}) {
  if (typeof name !== "string" || !name) throw new Error("No name given");
  usualLooks.forget(name); baselines.forget(name);
  const was = $("alertWho").value, left = was.split(",").map(s => s.trim()).filter(s => s && s !== name).join(", ");
  if (left !== was.split(",").map(s => s.trim()).filter(Boolean).join(", ")) {
    $("alertWho").value = left;
    $("alertWho").dispatchEvent(new Event("change"));
  }
  renderForget(); renderList(); loadAlerts();               // (the server already dropped their alerts)
  return { message: `Forgot what this page learned about ${name}` };
}
(async () => {
  chosenCam = recall("camera") || "";
  await listCameras(chosenCam);
  if (recall("autostart") !== "0" || $("optKeepCam").checked) startCamera();
})();
// People forgotten on any device (shared database): forget what this page learned about them, once each
function forgetForgotten(names) {
  let done;
  try { done = new Set(JSON.parse(localStorage.getItem("ft-forgot-done") || "[]")); } catch { done = new Set(); }
  let any = false;
  if (Array.isArray(names))                  // a name given to someone again leaves the list: a later forget counts anew
    for (const n of [...done]) if (!names.includes(n)) { done.delete(n); any = true; }
  for (const n of names || []) if (!done.has(n)) { try { forgetPerson({ name: n }); } catch {} done.add(n); any = true; }
  if (any) try { localStorage.setItem("ft-forgot-done", JSON.stringify([...done])); } catch {}
}
forgetForgotten(PAGE?.forgotten);
// Every half minute, camera running or not: what other devices learned about people since this page loaded, and who
// was forgotten there (with a shared database the sync puts both into this page's server). Without this the page
// kept saving what it had at load: other devices' learning was overwritten and a forgotten person written back.
if (window.PageStore) setInterval(async () => {
  const r = await PageStore.signals.refresh((key, name, data) => (key === "ft-usual" ? usualLooks : baselines).take(name, data));
  if (!r || r.same) return;
  forgetForgotten(r.forgotten);
  renderForget();
}, 30000);
loadPeople();
loadAlerts();                              // the bell's count from the inbox, and the app window's tab bar told
setInterval(() => { if (!running) loadPeople(); }, 10000);   // pick up names saved in the labeling tool
