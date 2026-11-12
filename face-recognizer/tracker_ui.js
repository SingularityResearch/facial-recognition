// tracker_ui.js - the script of the live tracker's page (tracker_ui.html, served by tracker.py).
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
let calibration = null, vaFit = null;      // fitted by tools/expr_eval.py (data/expr_calibration.json, data/va_fit.json)
let voiceModel = false;                    // the recognizer has the voice model (models/wav2small.onnx)
let poseOn = false;                        // the recognizer can send body points (MediaPipe pose model installed)
let handsOn = false;                       // the recognizer can send finger points (MediaPipe hand model installed)
let meshTris = null;                       // face-mesh triangles (landmark indices, three per triangle) when MediaPipe is installed
let qEnroll = 0.3, captured = 0;           // "Collect faces": the quality a face needs to be learned from; snapshots saved
const CAP_EVERY = 5000, CAP_MAX = 12, CAP_SEEN = 3;   // per face: ms between snapshots, most per visit, checks seen first
let threshTouched = false;                 // the strictness slider was moved, so don't reset it to the model's default
let checks = 0, fpsT = performance.now(), lastErr = 0;

function setStatus(text, cls = "") { $("statusText").textContent = text; $("status").className = "status " + cls; }
const colorFor = name => name === "Unknown" ? UNKNOWN : COLORS[[...name].reduce((h, c) => (h * 31 + c.charCodeAt(0)) >>> 0, 7) % COLORS.length];

async function loadPeople() {
  try {
    const r = await (await fetch("/api/people")).json();
    people = r.people;
    setExpressions(r.expressions || null);
    cuesOn = !!r.cues; showDevices(r.devices, r.warmup_ms);
    calibration = r.calibration || null; vaFit = r.vaFit || null; voiceModel = !!r.voice;
    forgetForgotten(r.forgotten);
    poseOn = !!r.pose; $("optPose").disabled = $("optHandGate").disabled = $("optSkeleton").disabled = !poseOn;
    handsOn = !!r.hands; $("optFingers").disabled = !handsOn;
    meshTris = r.meshTriangles || null; $("optMesh").disabled = !meshTris;
    if (r.qEnroll != null) qEnroll = r.qEnroll;
    if (r.captureDir) $("optCollect").parentElement.title = `Saves snapshots to ${r.captureDir} for the labeling tool`;
    $("testCount").textContent = [r.sessions ? `${r.sessions} tests` : "", r.reports ? `${r.reports} reports` : ""].filter(Boolean).join(" · ");
    if (r.threshold) $("optThresh").defaultValue = r.threshold;   // "Reset to defaults" in the app's Settings tab
    if (!threshTouched && r.threshold) {         // start the slider at the active model's measured setting
      $("optThresh").value = r.threshold; $("threshVal").textContent = (+r.threshold).toFixed(2);
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
// notification if allowed, and it's kept in the alert inbox (alerts.py) until you acknowledge or archive it. In the
// app window the tab bar's bell shows it too, whichever tab is open (app_ui.js).
// In the app window its tab bar has the bell and the banner (app_ui.js), over every tab: this page then shows no
// bell or banner of its own, so an alert shows once. Opened on its own (recognize.bat), the page shows both.
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
    ? rows.map(a => `<li><span>${esc(a.text)}</span><span class="when">${esc(when(a.t))} · ${esc({ arrived: "arrived", left: "left", unusual: "unusual signals", unknown: "unknown face", moved: "moved between cameras" }[a.kind] || a.kind)}</span>
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
$("alertItems").addEventListener("click", e => {
  const d = e.target.closest("button[data-detail]");
  if (d) {
    openDetail = openDetail === d.dataset.detail ? null : d.dataset.detail;
    $("alertsPanel").classList.toggle("wide", !!openDetail);        // room for the two sets side by side
    renderAlerts(Object.values(alertRows).sort((a, b) => b.t - a.t));
    return;
  }
  const b = e.target.closest("button[data-id]");
  if (b && (b.dataset.to !== "delete" || confirm("Delete this alert for good?"))) setAlerts({ ids: [b.dataset.id], state: b.dataset.to });
});
$("alertsBulk").onclick = () => {
  const [label, to] = BULK[alertView];
  if (to !== "delete" || confirm(`${label}? This can't be undone.`)) setAlerts({ where: alertView, state: to });
};
addEventListener("keydown", e => { if (e.key === "Escape" && !$("alertsPanel").hidden) closeAlerts(); });
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
                           headers: { "Content-Type": "application/json", "X-Face-Tool": "1" } }).catch(() => {});
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
  } catch {}
}

async function startCamera() {
  if (stream) { stopCamera(); return; }
  try {
    const id = $("camSelect").value;
    const ask = id => navigator.mediaDevices.getUserMedia({ video: {
      width: { ideal: 1280 }, height: { ideal: 720 }, ...(id ? { deviceId: { exact: id } } : {}) } });
    try { stream = await ask(id); }
    catch (e) {
      // A remembered camera ID only fits the browser that saved it (the app window and each browser
      // get their own IDs), and cameras get unplugged: fall back to the default camera.
      if (!id || !["OverconstrainedError", "NotFoundError"].includes(e.name)) throw e;
      $("camSelect").value = ""; stream = await ask("");
    }
    video.srcObject = stream; video.removeAttribute("src");
    $("optMirror").checked = true; applyMirror();
    await video.play();
    const activeId = stream.getVideoTracks()[0]?.getSettings().deviceId;
    listCameras(activeId);
    remember("camera", activeId || ""); remember("autostart", "1");
    // unplugged, or taken by another program: stopped, so "Keep this camera running" starts it again
    stream.getVideoTracks()[0]?.addEventListener("ended", () => { if (stream) stopCamera(); });
    $("camBtn").textContent = "Stop camera";
    begin();
  } catch (e) {
    stream = null;
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
function pictureSpread() {
  const g = flatGrab.getContext("2d", { willReadFrequently: true });
  g.drawImage(video, 0, 0, 32, 18);
  const d = g.getImageData(0, 0, 32, 18).data;
  let lo = 255, hi = 0;
  for (let i = 0; i < d.length; i += 4) { const y = (d[i] + d[i + 1] + d[i + 2]) / 3; if (y < lo) lo = y; if (y > hi) hi = y; }
  return hi - lo;
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
  let spread;
  try { spread = pictureSpread(); } catch { return; }
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
  stream?.getTracks().forEach(t => t.stop());
  stream = null; video.srcObject = null; running = false;
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

// ---------------------------------------------------------------- recognition requests
async function sendLoop() {
  while (running) {
    if (video.readyState < 2 || busy) { await new Promise(r => setTimeout(r, 30)); continue; }
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
    const blob = await new Promise(r => grab.toBlob(r, "image/jpeg", 0.85));
    busy = true;
    try {
      const want = new URLSearchParams({ t: $("optThresh").value });
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
      const r = await fetch(`/api/recognize?${want}`, { method: "POST", body: blob, headers: { "Content-Type": "image/jpeg", "X-Face-Tool": "1" } });
      const res = await r.json();
      if (!r.ok) throw new Error(res.error || r.status);
      updateTracks(res.faces.map(f => ({ ...f, box: f.box.map(v => v / s), kps: f.kps.map(([x, y]) => [x / s, y / s]),
                                         pose: f.pose && f.pose.map(([x, y, v]) => [x / s, y / s, v]),
                                         body: f.body && f.body.map(([x, y, v]) => [x / s, y / s, v]),
                                         hands: f.hands && f.hands.map(h => h.map(([x, y]) => [x / s, y / s])),
                                         mesh: f.mesh && Float32Array.from(f.mesh, v => v / s) })));
      shown = sync && $("optMesh").checked ? hold : null;
      await identify(want.get("t"));
      trackPresence();
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
    .then(r => r.ok ? r.json() : Promise.reject(new Error(r.status)))
    .then(() => { captured += faces.length; $("collectHint").textContent = `${captured} face snapshot${captured === 1 ? "" : "s"} saved this session for the labeling tool.`; })
    .catch(() => { $("collectHint").textContent = "Couldn't save a snapshot. Is the tracker window still open?"; });
}

// Name each face from the average of its last 7 face prints, weighted by face quality, instead of
// the vote over 7 per-frame names that updateTracks keeps as a fallback. Measured with
// tools/webcam_eval.py on webcam faces: 98% named correctly against 93% for the vote, no stranger
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
    if (!r.ok) throw new Error(r.status);
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
// window to start it again (app.py /restart); opened on its own, start recognize.bat (Windows) or recognize.sh
// (macOS, Linux) again, and the button reloads the page.
function startAgain(what) {
  return window.parent !== window ? `${what}: click Start again (or close Face Recognizer and start it again).`
    : `${what}: start recognize.bat again (recognize.sh on macOS and Linux), then click Start again.`;
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
async function startVoice() {
  try {
    mic = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: false, autoGainControl: false } });
    audioCtx = new AudioContext({ sampleRate: Signals.S.V_RATE });
    const tap = "registerProcessor('tap', class extends AudioWorkletProcessor { process(i) { const c = i[0][0]; if (c) this.port.postMessage(c.slice(0)); return true; } });";
    await audioCtx.audioWorklet.addModule(URL.createObjectURL(new Blob([tap], { type: "text/javascript" })));
    const node = new AudioWorkletNode(audioCtx, "tap"), mute = audioCtx.createGain();
    mute.gain.value = 0;
    audioCtx.createMediaStreamSource(mic).connect(node); node.connect(mute); mute.connect(audioCtx.destination);
    voice = new Signals.VoiceSignals(audioCtx.sampleRate);
    node.port.onmessage = e => voice?.push(e.data, performance.now());
    voiceTimer = setInterval(scoreVoice, 1000);
  } catch (e) {
    stopVoice(); $("optVoice").checked = false;
    setStatus(e.name === "NotAllowedError" ? "Microphone blocked. Allow it for this page to use voice." : "Couldn't start the microphone.", "err");
  }
}
function stopVoice() {
  clearInterval(voiceTimer); voiceTimer = null;
  mic?.getTracks().forEach(t => t.stop()); mic = null;
  audioCtx?.close(); audioCtx = null; voice = null;
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
// 30 seconds before the question, for tools/expr_eval.py.
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
function draw() {
  if (!running) return;
  render();
  requestAnimationFrame(draw);
}
// While the page is hidden (window minimized, covered, or another tab of the app shown), the browser stops calling
// requestAnimationFrame: draw (and stream this camera) on a timer instead, so the names and boxes keep up.
setInterval(() => { if (running && document.hidden) render(); }, 100);
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

// Body lines: shoulders, arms, torso (points in tracker.py's order; only those seen)
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

// ---- This camera for the other devices (camstream.py, peer.py): the video with every marker drawn on it, and the
// live readout as data (drawn on the other side with text only). Sent with a shared database while "Stream this
// camera" is on, at full rate (about 10 frames a second) all the time, watched or not.
const comp = document.createElement("canvas"), cctx = comp.getContext("2d");
let streamBusy = false, streamNext = 0, streamOff = 0;
async function streamFrame() {
  const now = performance.now();
  if (!running || !$("optStream").checked || streamBusy || now < streamNext || now < streamOff || !video.videoWidth) return;
  streamBusy = true;
  try {
    const s = Math.min(1, 960 / video.videoWidth);
    comp.width = Math.round(video.videoWidth * s); comp.height = Math.round(video.videoHeight * s);
    cctx.save();
    if ($("optMirror").checked) { cctx.translate(comp.width, 0); cctx.scale(-1, 1); }   // as this page shows it
    cctx.drawImage(video, 0, 0, comp.width, comp.height);
    cctx.drawImage(canvas, 0, 0, comp.width, comp.height);                              // names, boxes, markers
    cctx.restore();
    const blob = await new Promise(r => comp.toBlob(r, "image/jpeg", 0.72));
    const r = await fetch("/api/stream/frame", { method: "POST", body: blob, headers: { "Content-Type": "image/jpeg", "X-Face-Tool": "1" } });
    const j = await r.json().catch(() => ({}));
    if (!r.ok || j.stream === false) { streamOff = now + 30000; return; }
    streamNext = now + 100;                         // full rate all the time (about 10 frames a second)
  } catch { streamOff = now + 30000; } finally { streamBusy = false; }
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
  let readout = [];
  try { readout = $("optStream").checked ? readoutData() : []; } catch {}
  fetch("/api/inview", { method: "POST", headers: { "Content-Type": "application/json", "X-Face-Tool": "1" },
                         body: JSON.stringify({ names, readout }) }).catch(() => {});
}, 1000);
// Alerts the app raised itself (someone moved between cameras, from any device): shown like this page's own
let alertsSeenT = Date.now();
setInterval(async () => {
  try {
    const r = await alertApi("/api/alerts");
    const fresh = r.alerts.filter(a => a.kind === "moved" && a.state === "new" && a.t > alertsSeenT);
    if (!fresh.length) return;
    alertsSeenT = Math.max(...fresh.map(a => a.t));
    for (const a of fresh) announce(a.text);
    showAlertCounts(r.counts, fresh[0].text);
    if (!$("alertsPanel").hidden) loadAlerts();
  } catch {}
}, 5000);

// "Keep this camera running": start it again within 10 s whenever it isn't running (not while a video file plays)
let keepStarting = false;
setInterval(async () => {
  if (!$("optKeepCam").checked || stream || keepStarting || video.src) return;
  keepStarting = true;
  try { await startCamera(); } catch {} finally { keepStarting = false; }
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
$("camSelect").onchange = async () => { if (stream) { stopCamera(); await startCamera(); } };
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
const narrow = matchMedia("(max-width: 900px)");
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
$("calibReset").onclick = () => {
  const who = $("forgetWho").value;
  if (!confirm(who ? `Forget everything learned about ${who} (usual look and normal signals)?` : "Forget everything learned about everyone?")) return;
  usualLooks.forget(who || undefined); baselines.forget(who || undefined);
  renderForget(); renderList();
};
renderForget();
addEventListener("pagehide", () => { baselines.save(); usualLooks.save(); });   // keep the last few seconds' learning
$("testBtn").onclick = runTest;
$("optVoice").onchange = () => { if ($("optVoice").checked) startVoice(); else stopVoice(); };
$("optState").onchange = $("optTalkGate").onchange = $("optHandGate").onchange = $("optPose").onchange = renderList;   // from the next check
$("feelBtn").onclick = openFeel;
$("feelSave").onclick = saveFeel;
$("feelSkip").onclick = closeFeel;
$("feelV").oninput = e => $("feelVOut").textContent = e.target.value;
$("feelA").oninput = e => $("feelAOut").textContent = e.target.value;
$("feelEvery").value = recall("feel") || "0";
$("feelEvery").onchange = () => { remember("feel", $("feelEvery").value); lastAsk = performance.now(); };
$("testDelete").onclick = async () => {
  if (!confirm("Delete all saved expression test sessions and feeling reports?")) return;
  try { await fetch("/api/expr-eval/delete", { method: "POST", headers: { "X-Face-Tool": "1" } }); } catch {}
  loadPeople();
};
$("optSmooth").oninput = e => $("smoothVal").textContent = (+e.target.value).toFixed(2);
$("optThresh").oninput = e => { threshTouched = true; $("threshVal").textContent = (+e.target.value).toFixed(2); };
navigator.mediaDevices?.addEventListener?.("devicechange", () => listCameras());
// Remember the camera and start it automatically next time (unless you pressed Stop camera)
function remember(k, v) { if (window.PageStore) PageStore.set("ft-" + k, String(v)); else try { localStorage.setItem("ft-" + k, v); } catch {} }
function recall(k) { try { return localStorage.getItem("ft-" + k); } catch { return null; } }
// The other options are remembered between visits too (the camera's mirror view follows the source instead)
const KEEP = ["optPoints", "optTrail", "optUnknown", "optBlur", "optExpr", "optCalib", "optEvidence", "optVoice", "optPulse",
              "optTalkGate", "optState", "optPose", "optHandGate", "optSkeleton", "optFingers", "optSmooth", "optThresh", "optMeshStrength",
              "optAlertArrive", "optAlertLeave", "optAlertMove", "optStream", "optKeepCam", "optAlertUnusual", "optArriveGap", "optLeaveGap", "optUnknownStay",
              "optUnusualZ", "optUnusualFor", "optUnusualRepeat", "optDetailsAfter", "optBannerFor"];
for (const id of KEEP) {
  const el = $(id), v = recall("o-" + id);
  if (v !== null) {
    if (el.type === "checkbox") el.checked = v === "1"; else { el.value = v; el.dispatchEvent(new Event("input")); }
  }
  el.addEventListener("change", () => remember("o-" + id, el.type === "checkbox" ? (el.checked ? "1" : "0") : el.value));
}
if ($("optVoice").checked) startVoice();
for (const id of ["alertWho", "optAlertUnknown"]) $(id).addEventListener("change", () => SettingsBridge.changed());   // the Settings tab's list
// The app window's Settings tab (settings_bridge.js): every option of this page, in sections
{
  const c = SettingsBridge.control, C = "Face tracking: camera", D = "Face tracking: display",
        E = "Face tracking: expression", S = "Face tracking: voice, pulse and posture", R = "Face tracking: recognition",
        A = "Face tracking: alerts", P = "Face tracking: presence log";
  SettingsBridge.init("track", [
    c("camSelect", C, { noReset: true }),
    c("optMirror", C, { hint: "Turned on for the webcam and off for video files when the source starts." }),
    c("optPoints", D), c("optMesh", D), c("optMeshStrength", D),
    c("optSkeleton", D, { hint: "The body points the posture model finds below the face, joined: shoulders, arms with rough hands, hips, legs and feet. Drawn only; nothing is kept." }),
    c("optFingers", D, { hint: "The joints of every finger, 21 points a hand, from a second model (about 10-20 ms more a frame). Drawn only; nothing is kept." }),
    c("optTrail", D), c("optUnknown", D), c("optBlur", D),
    c("optSmooth", D, { hint: "How much the boxes are smoothed between frames." }),
    c("optExpr", E), c("optCalib", E, { hint: $("exprHint").textContent }), c("optEvidence", E),
    c("optTalkGate", E), c("optState", E),
    c("optHandGate", E, { hint: "The two \"skip\" options and the state model change the expression reading and aren't measured yet: they stay off until the Expression test and tools/expr_eval.py show they help." }),
    c("feelEvery", E, { label: "Ask how I feel (feeling check)" }),
    c("optVoice", S, { hint: "Analysed live and never recorded: loudness, pitch, speaking rate, and the voice model's arousal and valence." }),
    c("optPulse", S, { hint: "Reads tiny skin-colour changes; needs a still face in steady light." }),
    c("optPose", S, { hint: "Head height, shoulders, lean, body movement, breathing, hand on face and crossed arms." }),
    c("optThresh", R, { hint: "Higher strictness means fewer wrong names but more \"Unknown\". The default is the measured best for the active model." }),
    c("optCollect", R, { hint: $("collectHint").textContent }),
    c("optKeepCam", R, { label: "Keep this camera running", hint: "Starts the camera when the app starts, and again within 10 s if it stops (unplugged, busy, an error), so this device's camera is always on in every device's Cameras tab. Stop camera turns this off." }),
    c("optStream", R, { label: "Stream this camera to my other devices", hint: "Your other devices can watch this camera live in their Cameras tab, as this page shows it (names, boxes and every marker that's on here), with its live readout. Sent all the time at full rate (about 10 frames a second), over your local network, encrypted, only to your own devices; never stored. Needs a shared database and Share my photos with my other devices." }),
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
    c("optAlertArrive", A), c("optAlertLeave", A), c("optAlertMove", A, { label: "They move between cameras", hint: "Someone this camera sees was on another device's camera in the last 10 minutes (with a shared database). Every device shows the alert." }), c("optAlertUnusual", A), c("optAlertUnknown", A, { label: "An unknown face stays" }),
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
  await listCameras(recall("camera"));
  if (recall("autostart") !== "0" || $("optKeepCam").checked) startCamera();
})();
// People forgotten on any device (shared database): forget what this page learned about them, once each
function forgetForgotten(names) {
  let done;
  try { done = new Set(JSON.parse(localStorage.getItem("ft-forgot-done") || "[]")); } catch { done = new Set(); }
  let any = false;
  for (const n of names || []) if (!done.has(n)) { try { forgetPerson({ name: n }); } catch {} done.add(n); any = true; }
  if (any) try { localStorage.setItem("ft-forgot-done", JSON.stringify([...done])); } catch {}
}
forgetForgotten(PAGE?.forgotten);
loadPeople();
loadAlerts();                              // the bell's count from the inbox, and the app window's tab bar told
setInterval(() => { if (!running) loadPeople(); }, 10000);   // pick up names saved in the labeling tool
