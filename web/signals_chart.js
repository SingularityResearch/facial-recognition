// signals_chart.js - the Signals chart of the tracker page (tracker_ui.html) and of each camera on the Cameras page
// (cameras_ui.html): every sensor reading of one device over the last minutes, one expander a section.
//
//   This device   People in view, Camera, Device (what it is and how hard it works: its processor, memory,
//                 warmth and battery), and one section for each audio interface that is listened to
//                 (what the computer itself plays; what a microphone hears in the room)
//   One person    Recognition, Expression, Face, Voice, Body and hands, Their normal
//
// Closed, a section is one row: its name, a sparkline of its main signal and that signal's value now. Open, it
// has a small chart for each signal, all on one time axis. The green band is the person's own usual range (their
// average ± 2 SD, signals.js Baselines), orange a reading outside it, a gap a time with no reading. Pointing at one
// chart reads every chart at that moment.
//
// The readings are as fast as the device makes them: one each time its camera's picture is checked for faces (a
// sound interface: each time it was listened to). Another device sends every one of its readings, each with its own
// time, a few seconds' worth with each of its readouts (pack, unpack), so none is lost between them. They are kept
// here in memory only, for the last 15 minutes: nothing is stored. What another device sends is checked value by
// value (clean); names reach the page as text only. Evidence of how someone looks and sounds, never of what they
// feel.
//
// Model (the readings, the events made of them) and pack/unpack run in Node too, for the tests
// (tests/signals_chart.test.js); Panel is the page's part. Drawn on canvases with the page's own colours: the page
// loads no library.
(function (root) {
  "use strict";
  const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
  const KEEP_MS = 900000;                 // what is kept: the longest time the chart shows
  const MAX_ROWS = 30000;                 // readings kept of one person or device; beyond it the older half is thinned
  const GAP_MS = 2500;                    // readings further apart than this are not joined (out of view between)
  const LISTEN_GAP_MS = 20000;            // the same for a sound interface, listened to every few seconds
  const ARRIVE_GAP_MS = 10000;            // not seen for this long: left; seen again after it: in view again
  const ODD_SIGNALS = 3, ODD_FOR_MS = 5000, CALM_FOR_MS = 10000;   // "unusual signals": this many beyond 2 SD for this long
  const SENT_MS = 2500;                   // a device sends its readings of this long with each readout (one a second)
  const DRAW_EVERY_MS = 100;              // new readings are drawn at most this often
  const GUTTER = 96, ROW = 15, LANE = 18, MAX_LANES = 8;
  const LABELS = ["angry", "disgust", "fearful", "happy", "neutral", "sad", "surprised"];
  // what an interface heard, in a word (sound.rs `what`), and each word's colour (a custom property of .sg)
  const WHAT = ["quiet", "music", "speech", "television or radio", "steady background noise", "activity", "alert sounds",
                "animal sounds", "other sounds"];
  const WHAT_COLOR = ["line", "x3", "x5", "x2", "x4", "x1", "x0", "x6", "muted"];
  // a device's heat state as its system words it (a phone gives no degrees), coolest first
  const HEAT = ["nominal", "fair", "serious", "critical"];
  // what a device says of itself in words: what it is, and what its camera gives (each a short line of text)
  const FACTS = ["device", "system", "app", "processor", "memory", "models", "camera"];
  const S = (k, label, o = {}) => ({ k, label, ...o });

  // Each section with its signals. lo/hi: the chart's range (widened when a reading or the usual range lies
  // outside); mul: what the value is multiplied by to be shown; d: digits; base: a signal whose usual range is
  // learned for each person (signals.js B_KEYS); row: its name in Their normal.
  const DEVICE = [
    { key: "people", title: "People in view", head: "faces",
      about: "Who this camera sees, and when: every named person and unknown faces.",
      signals: [S("lanes", "in view", { kind: "lanes", wide: true }), S("faces", "faces in view", { lo: 0, hi: 4 })] },
    { key: "camera", title: "Camera", head: "checks",
      about: "How often this device checks its camera's picture for faces, and how long one check takes. Every check is one reading of everything below.",
      signals: [S("checks", "checks", { unit: "/s", lo: 0, hi: 30, d: 1 }), S("check", "one check", { unit: "ms", lo: 0, hi: 100 })] },
    { key: "host", title: "Device", head: "load",
      about: "The device itself: how much of its processor and memory is in use, how warm it is, and its battery. Each only where the device's own system says it: a phone gives a heat state and no degrees, a Mac and a Windows computer give no temperature at all. A reading the device doesn't give has no chart.",
      signals: [S("load", "processor in use", { unit: "%", lo: 0, hi: 100 }), S("memory", "memory in use", { unit: "%", lo: 0, hi: 100 }),
                S("temp", "temperature", { unit: "°C", lo: 20, hi: 100, d: 1 }), S("heat", "heat state", { lo: 0, hi: 3, words: HEAT }),
                S("battery", "battery", { unit: "%", lo: 0, hi: 100 }), S("charging", "on power", { kind: "flag" })] },
  ];
  // the Device section's numbers (a device's own readings), and its one yes or no
  const HOST = ["load", "memory", "temp", "heat", "battery"];
  const SOUND = {
    output: [
      S("what", "heard", { kind: "cat", wide: true }),
      S("level", "level", { unit: "dBFS", lo: -70, hi: 0 }),
      S("tempo", "tempo", { unit: "/min", lo: 60, hi: 180 }),
      S("pulse", "beat clarity", { lo: 0, hi: 1, d: 2 }),
      S("bright", "brightness", { unit: "Hz", lo: 500, hi: 4000 }),
      S("bass", "bass share", { lo: 0, hi: 0.6, d: 2 })],
    input: [
      S("what", "heard", { kind: "cat", wide: true }),
      S("level", "level", { unit: "dBFS", lo: -70, hi: 0 }),
      S("events", "events", { unit: "dB", lo: 0, hi: 40 }),
      S("steady", "steady", { lo: 0, hi: 1, d: 2 }),
      S("speechlike", "speech-like", { lo: 0, hi: 1, d: 2 }),
      S("hum", "mains hum", { unit: "dB", lo: 0, hi: 20 })],
  };
  const SOUND_KEYS = ["level", "floor", "events", "steady", "flat", "hum", "speechlike", "tempo", "pulse", "bright", "bass", "sure"];
  const PERSON = [
    { key: "recognition", title: "Recognition", head: "match",
      about: "Who the face is, and how sure the match is. The dashed line is the strictness a match has to pass.",
      signals: [
        S("match", "match", { unit: "%", lo: 0, hi: 1, mul: 100, mark: "strict" }),
        S("quality", "picture quality", { lo: 0, hi: 1, d: 2 }),
        S("size", "face size", { unit: "px", lo: 0, hi: 320 })] },
    { key: "expression", title: "Expression", head: "valence",
      about: "What the face looks like, check by check, and the reading made of it. A look, never a feeling.",
      signals: [
        S("probs", "share of each look", { kind: "stack", wide: true }),
        S("valence", "valence", { lo: -1, hi: 1, zero: true, d: 2 }),
        S("arousal", "arousal", { lo: -1, hi: 1, zero: true, d: 2 }),
        S("share", "share of votes", { unit: "%", lo: 0, hi: 1, mul: 100 })] },
    { key: "face", title: "Face", head: "pulse",
      about: "Movement, mouth, eyes, pulse and the face-shape cues, read from the face itself. A pulse is read only while the face is still.",
      signals: [
        S("motion", "moving", { row: "head moving", unit: "widths/s", lo: 0, hi: 0.4, d: 2, base: true }),
        S("pulse", "pulse", { unit: "/min", lo: 45, hi: 120, base: true }),
        S("blinkRate", "blinks", { unit: "/min", lo: 0, hi: 40, base: true }),
        S("gazeAway", "gaze away", { unit: "%", lo: 0, hi: 1, mul: 100, base: true }),
        S("smile", "smile", { lo: 0, hi: 1, d: 2, base: true }),
        S("jaw", "jaw", { lo: 0, hi: 0.1, d: 3, base: true }),
        S("cheek", "cheek raise", { lo: 0, hi: 1, d: 2 }),
        S("frown", "frown", { lo: 0, hi: 1, d: 2 }),
        S("browDown", "brow down", { lo: 0, hi: 1, d: 2 }),
        S("browInnerUp", "inner brow up", { lo: 0, hi: 1, d: 2 }),
        S("eyeWide", "eyes wide", { lo: 0, hi: 1, d: 2 })] },
    { key: "voice", title: "Voice", head: "loudness",
      about: "The microphone, analysed live and never stored. Readings only while this person speaks.",
      signals: [
        S("speaking", "speaking", { unit: "%", lo: 0, hi: 1, mul: 100 }),
        S("loudness", "loudness", { unit: "dB", lo: 0, hi: 30, base: true }),
        S("pitch", "pitch", { unit: "Hz", lo: 75, hi: 260, base: true }),
        S("pitchSd", "pitch range", { unit: "st", lo: 0, hi: 6, d: 1, base: true }),
        S("speechRate", "rate", { row: "speech rate", unit: "/s", lo: 0, hi: 8, d: 1, base: true }),
        S("voiceArousal", "arousal", { row: "voice arousal", lo: 0, hi: 1, d: 2, base: true }),
        S("voiceValence", "valence", { row: "voice valence", lo: 0, hi: 1, d: 2, base: true })] },
    { key: "body", title: "Body and hands", head: "bodyMotion",
      about: "Posture and movement from the body points: head height, shoulders, breathing, hands.",
      signals: [
        S("head", "head height", { lo: 0.3, hi: 1.1, d: 2, base: true }),
        S("neck", "neck", { lo: 0.1, hi: 0.7, d: 2, base: true }),
        S("tilt", "shoulder tilt", { unit: "°", lo: -20, hi: 20, zero: true, base: true }),
        S("bodyMotion", "movement", { row: "body movement", lo: 0, hi: 0.4, d: 2, base: true }),
        S("breathing", "breathing", { unit: "/min", lo: 6, hi: 30, base: true }),
        S("hand", "hand on face", { kind: "flag" }),
        S("armsCrossed", "arms crossed", { kind: "flag" })] },
    { key: "normal", title: "Their normal", head: "odd",
      about: "Each signal against this person's own usual level. Darker means further from usual; from 2 SD it counts as unusual.",
      signals: [S("z", "every signal against their usual", { kind: "heat", wide: true }), S("odd", "", { lo: 0, hi: 8, hidden: true })] },
  ];
  // Where each section and each reading is explained: a name in the chart is a button that asks the page around the
  // chart to show what each reading means (/readings, docs/READINGS.md) at that part, by a "readings" event; a page
  // that doesn't show it leaves the click at that. A sound interface's reading is "sound:" and its key, a section
  // "sec:" and its key.
  // the parts of docs/READINGS.md each section and reading is explained in (readings.rs holds every one to the page)
  const READ = {
    "sec:people": "people-in-view", "sec:camera": "camera", "sec:host": "device", "sec:sound": "sound-speakers-and-microphones",
    "sec:recognition": "recognition", "sec:expression": "expression", "sec:face": "face", "sec:voice": "voice",
    "sec:body": "body-and-hands", "sec:normal": "their-normal",
    lanes: "in-view", faces: "faces-in-view", checks: "checks", check: "one-check", load: "processor-in-use",
    memory: "memory-in-use", temp: "temperature", heat: "heat-state", battery: "battery", charging: "on-power",
    "sound:what": "heard", "sound:level": "level", "sound:tempo": "tempo", "sound:pulse": "beat-clarity",
    "sound:bright": "brightness", "sound:bass": "bass-share", "sound:events": "events", "sound:steady": "steady",
    "sound:speechlike": "speech-like", "sound:hum": "mains-hum",
    match: "match", quality: "picture-quality", size: "face-size", probs: "share-of-each-look", valence: "valence-face",
    arousal: "arousal-face", share: "share-of-votes", motion: "moving", pulse: "pulse", blinkRate: "blinks",
    gazeAway: "gaze-away", smile: "smile", jaw: "jaw", cheek: "cheek-raise", frown: "frown", browDown: "brow-down",
    browInnerUp: "inner-brow-up", eyeWide: "eyes-wide", speaking: "speaking", loudness: "loudness", pitch: "pitch",
    pitchSd: "pitch-range", speechRate: "speech-rate", voiceArousal: "voice-arousal", voiceValence: "voice-valence",
    head: "head-height", neck: "neck", tilt: "shoulder-tilt", bodyMotion: "body-movement", breathing: "breathing",
    hand: "hand-on-face", armsCrossed: "arms-crossed", z: "spreads-away",
};
  // Why a section has no readings: the option of the device's Face tracking tab that makes them is off (the device
  // says which are, with its readings), or the device has no model for them. [the section it is said in, the words]
  const OFF = {
    expression: ["expression", "No readings: Show expression is off"],
    voice: ["voice", "No readings: Listen to voice (microphone) is off"],
    nomic: ["voice", "No readings: there is no microphone of this device's own (a Mac leaves an iPhone's to the phone)"],
    body: ["body", "No readings: Body posture is off"],
    nopose: ["body", "No readings: there is no posture model"],
    pulse: ["face", "No pulse: Webcam pulse (experimental) is off"],
    nocues: ["face", "No face-shape cues (smile, jaw, blinks, gaze): MediaPipe isn't there"],
    normal: ["normal", "No usual ranges: Calibrate to each person is off"],
  };
  const PERSON_SIGNALS = PERSON.flatMap(sec => sec.signals);
  const BASE = PERSON_SIGNALS.filter(s => s.base);
  const SCALARS = PERSON_SIGNALS.filter(s => !s.kind && s.k !== "odd").map(s => s.k);
  const FLAGS = PERSON_SIGNALS.filter(s => s.kind === "flag").map(s => s.k);

  // ------------------------------------------------------------------ readings, checked
  const num = x => typeof x === "number" && isFinite(x) ? x : null;
  const flag = x => typeof x === "boolean" ? x : null;
  const text = (x, max) => typeof x === "string" ? x.slice(0, max) : "";
  const obj = x => x && typeof x === "object" && !Array.isArray(x) ? x : {};

  // One face's numbers as the chart keeps them: every value a finite number, a yes or no, or null.
  function cleanNums(n) {
    n = obj(n);
    const out = {};
    for (const k of SCALARS) out[k] = num(n[k]);
    for (const k of FLAGS) out[k] = flag(n[k]);
    out.probs = Array.isArray(n.probs) && n.probs.length >= 2 && n.probs.length <= 12 && n.probs.every(p => num(p) != null && p >= 0)
      ? n.probs.slice() : null;
    return out;
  }
  // A person's usual ranges, {signal: [average, spread]}: only the signals a usual range is learned for.
  function cleanUsual(u) {
    u = obj(u);
    const out = {};
    for (const s of BASE) { const r = u[s.k]; if (Array.isArray(r) && num(r[0]) != null && num(r[1]) != null && r[1] > 0) out[s.k] = [r[0], r[1]]; }
    return out;
  }
  function cleanSound(snd) {
    if (snd == null) return null;
    snd = obj(snd);
    if (!Array.isArray(snd.interfaces) || !(typeof snd.seq === "number" || typeof snd.seq === "string")) return null;
    return { seq: snd.seq, t: num(snd.t), interfaces: snd.interfaces.slice(0, 24).map(obj).filter(i => i.kind === "output" || i.kind === "input").map(i => {
      const v = { what: WHAT.includes(i.what) ? WHAT.indexOf(i.what) : i.what == null ? null : WHAT.length - 1 };
      for (const k of SOUND_KEYS) v[k] = num(i[k]);
      return { name: text(i.name, 120) || "audio interface", kind: i.kind, v };
    }) };
  }
  // One reading of the page (or of another device, as its readout brought it): {device, faces, sound}.
  function clean(sample) {
    const o = obj(sample), dev = obj(o.device);
    const labels = Array.isArray(dev.labels) && dev.labels.length >= 2 && dev.labels.length <= 12 && dev.labels.every(l => typeof l === "string")
      ? dev.labels.map(l => l.slice(0, 24)) : null;
    const faces = (Array.isArray(o.faces) ? o.faces.slice(0, 50) : []).map(obj)
      .map(f => ({ name: text(f.name, 80) || "Unknown", nums: cleanNums(f.nums), usual: cleanUsual(f.usual) }));
    const off = Array.isArray(dev.off) ? dev.off.filter(k => typeof k === "string" && Object.hasOwn(OFF, k)).slice(0, 12) : null;
    const device = { checks: num(dev.checks), check: num(dev.check), strict: num(dev.strict), labels, off,
                     charging: flag(dev.charging), facts: cleanFacts(dev.facts) };
    for (const k of HOST) device[k] = num(dev[k]);
    // (a heat state is one of the four; a share is a share)
    if (device.heat != null && !(Number.isInteger(device.heat) && device.heat >= 0 && device.heat < HEAT.length)) device.heat = null;
    for (const k of ["load", "memory", "battery"]) if (device[k] != null && (device[k] < 0 || device[k] > 100)) device[k] = null;
    return { device, faces, sound: cleanSound(o.sound) };
  }
  // What a device says of itself in words: only the lines there are names for, each as text of a line's length.
  // null when it said none.
  function cleanFacts(f) {
    f = obj(f);
    const out = {};
    for (const k of FACTS) { const line = text(f[k], 240).replace(/\s+/g, " ").trim(); if (line) out[k] = line; }
    return Object.keys(out).length ? out : null;
  }

  // What a device sends of its readings with a readout, for the other devices' charts: the readings themselves
  // (samples: [{t, device, faces: [{name, nums, usual}]}], oldest first, t by its own clock), each face's numbers
  // in the order of `keys`, a person's usual ranges once. now: its clock as it sends; sound: {seq, age, interfaces}.
  function pack(samples, now, sound) {
    const names = [], usual = [];
    const rows = samples.map(s => [s.t, s.device?.check ?? null, s.device?.checks ?? null, (s.faces || []).map(f => {
      let i = names.indexOf(f.name);
      if (i < 0) i = names.push(f.name) - 1;
      usual[i] = f.usual || {};
      return [i, SCALARS.map(k => f.nums?.[k] ?? null), FLAGS.map(k => f.nums?.[k] ?? null), f.nums?.probs || null];
    })]);
    const last = samples[samples.length - 1];
    // the device's own readings change slowly: the newest once, and what it says of itself in words
    const d = last?.device || {}, host = HOST.map(k => d[k] ?? null);
    return { v: 1, now, keys: SCALARS, flags: FLAGS, strict: last?.device?.strict ?? null, labels: last?.device?.labels || undefined,
             off: last?.device?.off || undefined,
             host: host.some(v => v != null) || d.charging != null ? [...host, d.charging ?? null] : undefined, facts: d.facts || undefined,
             people: names.map((name, i) => ({ name, usual: usual[i] })), rows, sound: sound || null };
  }
  // Another device's clock against this one's, from the gap between the time it wrote on a batch and the time the
  // batch arrived. A batch only ever arrives later than it was sent, never earlier: so the smallest gap seen is the
  // best word for how far the clocks are apart, and a larger one means only that this batch waited on the way (a
  // connection made again brings a minute of them at once). Each reading is then placed at its own time. A gap that
  // stays larger for half a minute means its clock was set back: taken anew then, and the caller is told (true) to
  // forget which readings it has taken, since their times mean something else now. state: {offset, late}.
  function settle(state, gap, now) {
    if (state.offset == null || gap < state.offset) { state.offset = gap; state.late = null; return false; }
    if (gap <= state.offset + 3000) { state.late = null; return false; }
    if (state.late == null) { state.late = now; return false; }
    if (now - state.late <= 30000) return false;
    state.offset = gap; state.late = null;
    return true;
  }

  // The same back into readings, [{t, raw}] with t by the sender's clock and raw as Model.sample takes it. What
  // doesn't have the shape is left out; every value is checked again where it is kept (clean).
  function unpack(sig) {
    sig = obj(sig);
    const keys = Array.isArray(sig.keys) ? sig.keys.slice(0, 200) : [], flags = Array.isArray(sig.flags) ? sig.flags.slice(0, 20) : [];
    const people = (Array.isArray(sig.people) ? sig.people.slice(0, 50) : []).map(obj);
    const host = {};
    if (Array.isArray(sig.host)) { HOST.forEach((k, i) => { host[k] = sig.host[i]; }); host.charging = sig.host[HOST.length]; }
    const out = [];
    for (const row of Array.isArray(sig.rows) ? sig.rows.slice(-2000) : []) {
      if (!Array.isArray(row) || num(row[0]) == null) continue;
      const faces = [];
      for (const f of Array.isArray(row[3]) ? row[3].slice(0, 50) : []) {
        if (!Array.isArray(f) || !people[f[0]]) continue;
        const nums = { probs: f[3] };
        (Array.isArray(f[1]) ? f[1] : []).forEach((v, i) => { if (typeof keys[i] === "string") nums[keys[i]] = v; });
        (Array.isArray(f[2]) ? f[2] : []).forEach((v, i) => { if (typeof flags[i] === "string") nums[flags[i]] = v; });
        faces.push({ name: people[f[0]].name, nums, usual: people[f[0]].usual });
      }
      out.push({ t: row[0], raw: { device: { check: row[1], checks: row[2], strict: sig.strict, labels: sig.labels, off: sig.off, ...host, facts: sig.facts }, faces } });
    }
    return out;
  }

  // The catalog: every section and signal of the chart as data, for what a node publishes of itself (its bus):
  // a source for each section, each signal with its label, unit, range and kind, and why a section can be
  // without readings. How a signal is drawn stays here; one that only helps the drawing is left out.
  function catalog() {
    const sig = s => {
      const o = { key: s.k, label: s.label, kind: s.kind || (s.words ? "word" : "number") };
      for (const key of ["unit", "lo", "hi", "mul", "d"]) if (s[key] != null) o[key] = s[key];
      if (s.base) o.usual = true;                       // (a usual range is learned of it for each person)
      const words = s.words || (s.kind === "cat" ? WHAT : s.kind === "stack" ? LABELS : null);
      if (words) o.words = words.slice();
      return o;
    };
    const sec = x => ({ key: x.key, title: x.title, about: x.about, signals: x.signals.filter(s => !s.hidden).map(sig) });
    return { v: 1, device: DEVICE.map(sec), sound: { output: SOUND.output.map(sig), input: SOUND.input.map(sig) }, person: PERSON.map(sec),
             off: Object.fromEntries(Object.entries(OFF).map(([key, [section, why]]) => [key, { section, why }])) };
  }

  // ------------------------------------------------------------------ what is kept, and the events made of it
  // Rows beyond what is kept of one source: every second one of the older half goes, so the newest stay as dense
  // as they came.
  function thin(rows, most = MAX_ROWS) {
    if (rows.length <= most) return;
    const half = rows.length >> 1;
    let w = 0;
    for (let i = 1; i < half; i += 2) rows[w++] = rows[i];
    rows.copyWithin(w, half);
    rows.length -= half - w;
  }

  class Model {
    constructor(keepMs = KEEP_MS) {
      this.keep = keepMs;
      this.device = { rows: [] };            // {t, v: {checks, check, strict, faces, known, unknown, and HOST's}}
      this.facts = null;                     // what the device says of itself in words (FACTS), as it last said it
      this.people = new Map();               // name -> {rows: [{t, v}], usual: {key: [mean, sd]}, last, ...}
      this.sound = new Map();                // "kind:name" -> {name, kind, n, rows: [{t, v}]}
      this.events = [];                      // {t, kind, text}, oldest first
      this.labels = LABELS.slice();
      this.off = [];                         // what the device says it has switched off (OFF's keys)
      this.soundSeq = null;
    }
    event(t, kind, text) {
      this.events.push({ t, kind, text });
      this.events.sort((a, b) => a.t - b.t);
    }
    // One reading at time t (ms). Returns true when a person or an interface was seen for the first time.
    sample(t, raw) {
      const s = clean(raw);
      let fresh = false;
      if (s.device.labels) this.labels = s.device.labels;
      if (s.device.off) this.off = s.device.off;
      if (s.device.facts) this.facts = s.device.facts;
      const unknown = s.faces.filter(f => f.name === "Unknown").length;
      const v = { checks: s.device.checks, check: s.device.check, strict: s.device.strict,
                  faces: s.faces.length, known: s.faces.length - unknown, unknown: unknown > 0, charging: s.device.charging };
      for (const k of HOST) v[k] = s.device[k];
      this.device.rows.push({ t, v });
      thin(this.device.rows);
      const seen = new Set();
      for (const f of s.faces) {
        if (seen.has(f.name)) continue;                        // one row a name: several unknown faces share a lane
        seen.add(f.name);
        let p = this.people.get(f.name);
        if (!p) { this.people.set(f.name, p = { rows: [], usual: {}, last: -Infinity, gone: true, oddSince: null, calmSince: null, said: false }); fresh = true; }
        if (p.gone) this.event(t, f.name === "Unknown" ? "unknown" : "arrived", f.name === "Unknown" ? "unknown face" : `${f.name} in view`);
        p.last = t; p.gone = false;
        p.usual = f.usual;                                     // their usual ranges as they are now
        const v = f.nums;
        v.z = {};                                              // how far each signal is from their usual, in spreads
        for (const b of BASE) { const u = p.usual[b.k]; v.z[b.k] = u && v[b.k] != null ? (v[b.k] - u[0]) / u[1] : null; }
        v.odd = BASE.filter(b => v.z[b.k] != null && Math.abs(v.z[b.k]) >= 2).length;
        p.rows.push({ t, v });
        thin(p.rows);
        // unusual signals: several beyond their usual range, and for some seconds on end
        if (v.odd >= ODD_SIGNALS) {
          p.calmSince = null; p.oddSince ??= t;
          if (!p.said && t - p.oddSince >= ODD_FOR_MS && f.name !== "Unknown") { p.said = true; this.event(t, "unusual", `${f.name}: unusual signals`); }
        } else {
          p.oddSince = null;
          if (v.odd === 0) { p.calmSince ??= t; if (t - p.calmSince >= CALM_FOR_MS) p.said = false; } else p.calmSince = null;
        }
      }
      for (const [name, p] of this.people) {
        if (!p.gone && t - p.last > ARRIVE_GAP_MS) {
          p.gone = true; p.oddSince = null;
          if (name !== "Unknown") this.event(p.last, "left", `${name} left`);
        }
      }
      if (s.sound && s.sound.seq !== this.soundSeq) {
        this.soundSeq = s.sound.seq;
        const at = s.sound.t ?? t;
        for (const i of s.sound.interfaces) {
          const key = `${i.kind}:${i.name}`;
          let src = this.sound.get(key);
          if (!src) {
            const n = [...this.sound.values()].filter(x => x.kind === i.kind).length + 1;
            this.sound.set(key, src = { name: i.name, kind: i.kind, n, rows: [] });
            fresh = true;
          }
          const was = src.rows.length ? src.rows[src.rows.length - 1].v.what : null;
          if (i.v.what === 1 && was !== 1) this.event(at, "song", "music heard");
          src.rows.push({ t: at, v: i.v });
        }
      }
      this.trim(t);
      return fresh;
    }
    trim(now) {
      const from = now - this.keep, cut = rows => { let n = 0; while (n < rows.length && rows[n].t < from) n++; if (n) rows.splice(0, n); };
      cut(this.device.rows);
      for (const [name, p] of this.people) { cut(p.rows); if (!p.rows.length && p.gone) this.people.delete(name); }
      for (const src of this.sound.values()) cut(src.rows);
      cut(this.events);
    }
    // The names to show: whoever was seen last first; Unknown last.
    names() {
      return [...this.people.keys()].sort((a, b) => (a === "Unknown") - (b === "Unknown") || this.people.get(b).last - this.people.get(a).last);
    }
  }

  // The first row at or after a time (its place in rows; rows.length when there is none).
  function firstFrom(rows, t) {
    let lo = 0, hi = rows.length;
    while (lo < hi) { const mid = (lo + hi) >> 1; if (rows[mid].t < t) lo = mid + 1; else hi = mid; }
    return lo;
  }
  // The row nearest to a time, within `within` ms of it; null when there is none.
  function rowNear(rows, t, within = 1500) {
    if (!rows.length) return null;
    const i = Math.min(firstFrom(rows, t), rows.length - 1), a = rows[i], b = rows[i - 1];
    const best = b && Math.abs(b.t - t) < Math.abs(a.t - t) ? b : a;
    return Math.abs(best.t - t) <= within ? best : null;
  }
  // The newest row at or before a time, within `within` ms; null when there is none (a sound interface's reading
  // stands until the next one).
  function rowBefore(rows, t, within = LISTEN_GAP_MS) {
    const i = firstFrom(rows, t + 1) - 1;
    return i >= 0 && t - rows[i].t <= within ? rows[i] : null;
  }
  const fmt = (s, v) => v == null ? "–" : s.kind === "flag" ? (v ? "yes" : "no") : s.words ? s.words[Math.round(v)] ?? "–"
    : (v * (s.mul || 1)).toFixed(s.d ?? 0) + (s.unit ? (s.unit === "%" || s.unit === "°" ? "" : " ") + s.unit : "");
  // A chart's range: its own, widened to hold every reading shown and the usual range.
  function range(s, values, usual) {
    let lo = s.lo, hi = s.hi;
    const take = v => { if (v != null && isFinite(v)) { if (v < lo) lo = v; if (v > hi) hi = v; } };
    for (const v of values) take(v);
    if (usual) { take(usual[0] - 2 * usual[1]); take(usual[0] + 2 * usual[1]); }
    if (lo < s.lo) lo -= (hi - lo) * 0.05;
    if (hi > s.hi) hi += (hi - lo) * 0.05;
    return [lo, hi];
  }
  // A signal's readings from a time on as the points of a line, one for each reading or, where several fall on
  // one pixel, one for the pixel with its lowest and highest: {x, t0, t1, first, last, lo, hi, hot}. Readings with
  // no value break the line (a point with `gap`).
  function points(rows, from, key, X, odd) {
    const out = [];
    let cur = null, broke = false;
    for (let i = from; i < rows.length; i++) {
      const r = rows[i], v = r.v[key];
      if (v == null) { broke = true; cur = null; continue; }
      const x = Math.round(X(r.t));
      if (cur && cur.x === x && !broke) {
        cur.t1 = r.t; cur.last = v;
        if (v < cur.lo) cur.lo = v;
        if (v > cur.hi) cur.hi = v;
        if (odd(r)) cur.hot = true;
      } else {
        out.push(cur = { x, t0: r.t, t1: r.t, first: v, last: v, lo: v, hi: v, hot: odd(r), gap: broke });
        broke = false;
      }
    }
    return out;
  }

  // ------------------------------------------------------------------ the page's part
  const el = (tag, props = {}, ...kids) => { const n = Object.assign(document.createElement(tag), props); n.append(...kids.filter(x => x != null)); return n; };
  const EMPTY = { rows: [], usual: {} };

  class Panel {
    // host: the element the chart is put into. o: {title, device (the first group's heading), open (the open
    // sections' keys, as remembered), onOpen(keys), window (ms, as remembered), onWindow(ms), turnOn(key): switches
    // one of this page's own options on (OFF's keys); without it the chart is another device's and only says so}.
    constructor(host, o = {}) {
      this.o = o;
      this.model = new Model();
      this.windowMs = [60000, 300000, 900000].includes(o.window) ? o.window : 300000;
      this.hoverT = null; this.who = null; this.picked = false;
      this.charts = []; this.T = {}; this.open = Array.isArray(o.open) ? o.open : null; this.seen = "";
      this.drawn = 0; this.due = false; this.offset = null; this.lastT = -Infinity; this.notes = {}; this.offSeen = null;
      this.abouts = {}; this.factsSeen = "";
      this.events = el("canvas", { className: "strip" });
      this.events.setAttribute("role", "img");
      this.deviceSecs = el("div", { className: "secs" });
      this.personSecs = el("div", { className: "secs" });
      this.whoSeg = el("div", { className: "seg" });
      this.whoSeg.setAttribute("role", "group"); this.whoSeg.setAttribute("aria-label", "Person");
      this.note = el("p", { className: "later", hidden: true });
      this.why = el("p", { className: "why", hidden: true });
      this.why.setAttribute("role", "status");
      this.nobody = el("p", { className: "later", textContent: "Nobody has been in view in the time shown." });
      const win = el("div", { className: "seg" });
      win.setAttribute("role", "group"); win.setAttribute("aria-label", "Time shown");
      this.seg(win, [["1 min", 60000], ["5 min", 300000], ["15 min", 900000]], this.windowMs, ms => { this.windowMs = ms; o.onWindow?.(ms); });
      this.root = el("div", { className: "sg" },
        el("div", { className: "top" },
          el("div", {}, el("h2", { className: "title", textContent: o.title || "Signals" }),
            el("p", { className: "sub", textContent: "Every sensor reading of this device over the last minutes: its camera, its sound interfaces, and each person it sees." })),
          win),
        this.why,
        el("section", { className: "panel" }, el("div", { className: "gl", textContent: "Events" }), this.events, this.axis()),
        el("section", { className: "group" },
          el("div", { className: "gh" }, el("h3", { textContent: o.device || "This device" }), el("span", { className: "gnote", textContent: "its camera and every audio interface" })),
          this.deviceSecs, this.note),
        el("section", { className: "group" },
          el("div", { className: "gh" }, el("h3", { textContent: "One person" }), this.whoSeg),
          this.nobody, this.personSecs),
        el("p", { className: "foot", textContent: "Green band: this person's usual range (their average ± 2 SD). Orange: a reading outside it. A gap: no reading then (not speaking, a moving face, out of view, silence). Dots: one reading each time a sound interface was listened to. Point at a chart to read every chart at that moment. Readings are evidence of how someone looks and sounds, never of what they feel." }));
      for (const sec of DEVICE) this.deviceSecs.append(this.section(sec));
      for (const sec of PERSON) this.personSecs.append(this.section({ ...sec, person: true }));
      host.append(this.root);
      if (window.ResizeObserver) { this.sizes = new ResizeObserver(() => this.draw()); this.sizes.observe(this.root); }
      this.people();
      this.draw();
    }
    // Something to say under the device's sections (why no sound is charted); "" hides it.
    say(text) { this.note.textContent = text || ""; this.note.hidden = !text; }
    // Something to say above everything (why there are no readings at all); "" hides it.
    tellWhy(text) { if (this.why.textContent !== (text || "")) this.why.textContent = text || ""; this.why.hidden = !text; }
    close() { this.sizes?.disconnect(); this.root.remove(); }

    // One reading of this device (see Model.sample), at this computer's time t.
    sample(t, raw) {
      if (this.model.sample(t, raw)) this.sounds();
      this.itself();
      this.whyNot();
      this.people();
      this.later();
    }
    // Another device's readings as its readout brought them (pack): those not taken yet, each at its own time,
    // told by this computer's clock. Its sound's reading says how old it was when it was sent.
    take(signals, now = Date.now()) {
      const sig = obj(signals), sent = num(sig.now);
      if (sent == null) return;
      // its clock against this one's (settle): which readings were taken is forgotten only when its clock was set
      if (settle(this, now - sent, now)) this.lastT = -Infinity;
      const snd = obj(sig.sound), sound = sig.sound ? { seq: snd.seq, t: now - (num(snd.age) ?? 0) * 1000, interfaces: snd.interfaces } : null;
      const news = unpack(sig).filter(r => r.t > this.lastT);
      if (!news.length && sound) news.push({ t: sent, raw: { device: { strict: sig.strict, labels: sig.labels }, faces: [] } });
      let fresh = false;
      news.forEach((r, i) => {
        this.lastT = Math.max(this.lastT, r.t);
        if (i === news.length - 1) r.raw.sound = sound;
        fresh = this.model.sample(r.t + this.offset, r.raw) || fresh;
      });
      if (!news.length) return;
      if (fresh) this.sounds();
      this.itself();
      this.whyNot();
      this.people();
      this.later();
    }
    // What the device says of itself in words, put under its own section's first line and its camera's: what it
    // is, and what its camera gives. Text only, each line as the chart's model checked it.
    itself() {
      // a reading the device's system doesn't give has no chart (a computer without a battery, a phone without
      // degrees); one it gave once keeps its chart, so that a gap is seen as a gap
      const last = this.model.device.rows.at(-1)?.v;
      if (last) for (const ch of this.charts) {
        if (ch.sec.key !== "host" || last[ch.sig.k] == null) continue;
        if (ch.fig?.hidden) ch.fig.hidden = false;
        if (ch.det.hidden) ch.det.hidden = false;
      }
      const f = this.model.facts, said = f ? JSON.stringify(f) : "";
      if (said === this.factsSeen) return;
      this.factsSeen = said;
      const lines = keys => keys.map(key => f?.[key]).filter(Boolean).join(" · ");
      const host = this.abouts.host, cam = this.abouts.camera, is = lines(["device", "system", "app", "processor", "memory", "models"]);
      if (host) host.el.textContent = (is ? is + ". " : "") + host.base;
      if (cam) cam.el.textContent = cam.base + (f?.camera ? " This camera: " + f.camera + "." : "");
    }
    // a section for each audio interface there are readings of
    sounds() {
      for (const [key, src] of this.model.sound) {
        if (this.charts.some(ch => ch.sec.sound === key)) continue;
        const speakers = src.kind === "output";
        this.deviceSecs.append(this.section({
          key: "sound:" + key, sound: key, head: "level", signals: SOUND[src.kind], kind: src.kind,
          title: `Sound · ${speakers ? "speakers" : "microphone"}${src.n > 1 ? " " + src.n : ""}`,
          about: speakers
            ? `${src.name}: what this computer itself plays through it, listened to a few seconds at a time and measured in memory. The sound is never kept.`
            : `${src.name}: what sounds in the room, the same few seconds, as numbers and one word only.` }));
      }
    }

    // a name that opens what it means (see READ)
    readButton(part, text, title, cls = "") {
      const b = el("button", { type: "button", className: "rd " + cls, textContent: text, title });
      b.onclick = () => b.dispatchEvent(new CustomEvent("readings", { bubbles: true, detail: part }));
      return b;
    }
    axis() { return el("div", { className: "axis" }, el("span", { className: "from" }), el("span", { className: "at" }), el("span", { textContent: "now" })); }
    seg(host, items, current, set) {
      host.replaceChildren(...items.map(([label, value]) => {
        const b = el("button", { type: "button", textContent: label });
        b.setAttribute("aria-pressed", String(value === current));
        b.onclick = () => { set(value); for (const x of host.children) x.setAttribute("aria-pressed", String(x === b)); this.draw(); };
        return b;
      }));
    }
    // Under a section's first line: why it has no readings, when the device said which of its options are off.
    whyNot() {
      const said = this.model.off.join(",");
      if (said === this.offSeen) return;
      this.offSeen = said;
      for (const [section, note] of Object.entries(this.notes)) {
        const mine = this.model.off.filter(k => OFF[k][0] === section);
        note.replaceChildren(...mine.map(k => {
          const line = el("span", { textContent: OFF[k][1] + (this.o.turnOn ? "." : " on that device.") });
          if (!this.o.turnOn || k === "nopose" || k === "nocues" || k === "nomic") return line;   // (nothing to switch on)
          const b = el("button", { type: "button", className: "on", textContent: "Turn it on" });
          b.onclick = () => this.o.turnOn(k);
          return el("span", {}, line, " ", b);
        }));
        note.hidden = !mine.length;
      }
    }
    // The people to choose from: whoever was in view in the time kept. The one shown stays the one picked while
    // there are readings of them; otherwise the person seen last.
    people() {
      const names = this.model.names();
      if (!names.includes(this.who)) { this.who = null; this.picked = false; }
      if (!this.picked && names.length && this.who !== names[0]) this.who = names[0];
      const key = [...names].sort().join("\n") + "\n\n" + this.who;
      if (key === this.seen) return;
      this.seen = key;
      this.seg(this.whoSeg, [...names].sort((a, b) => (a === "Unknown") - (b === "Unknown") || a.localeCompare(b)).map(n => [n, n]), this.who,
               n => { this.who = n; this.picked = true; this.seen = ""; });
      this.nobody.hidden = names.length > 0;
      this.personSecs.hidden = !names.length;
    }
    isOpen(sec) { return this.open ? this.open.includes(sec.key) : sec.key === "face" || sec.kind === "input"; }
    saveOpen() {
      this.open = [...this.root.querySelectorAll("details.sec[open]")].map(d => d.dataset.key);
      this.o.onOpen?.(this.open);
    }
    swatches(names, colors, values) {
      return el("div", { className: "legend" }, ...names.map((name, i) => {
        const sw = el("i");
        sw.style.background = `var(--${colors[i]})`;
        return el("span", {}, sw, name, values ? values[i] : null);
      }));
    }
    section(sec) {
      const spark = el("canvas", { className: "spark" }), now = el("span", { className: "now" });
      spark.setAttribute("aria-hidden", "true");
      const det = el("details", { className: "sec", open: this.isOpen(sec) });
      det.dataset.key = sec.key;
      const grid = el("div", { className: "grid" }, ...sec.signals.filter(s => !s.hidden).map(s => {
        const cv = el("canvas", { className: "plot " + (s.kind || "line") }), val = el("b");
        cv.setAttribute("role", "img");
        const ch = { cv, val, sec, sig: s, det };
        let legend = null;
        if (s.kind === "stack") { ch.legend = legend = el("div", { className: "legend" }); ch.labels = ""; ch.shares = []; }
        if (s.kind === "cat") legend = this.swatches(WHAT, WHAT_COLOR);
        if (s.kind === "heat") cv.style.height = BASE.length * ROW + "px";
        const x0 = s.kind === "heat" || s.kind === "lanes" ? GUTTER : 0;
        cv.onpointermove = e => { this.hoverT = Date.now() - this.windowMs + clamp((e.offsetX - x0) / (cv.clientWidth - x0), 0, 1) * this.windowMs; this.draw(); };
        cv.onpointerleave = () => { this.hoverT = null; this.draw(); };
        this.charts.push(ch);
        const part = READ[sec.sound ? "sound:" + s.k : s.k];
        ch.fig = el("figure", { className: "ch" + (s.wide ? " wide" : "") }, el("figcaption", {},
          part ? this.readButton(part, s.label, `What "${s.label}" means: what it measures, its values and why it matters`) : el("span", { textContent: s.label }), val), cv, legend);
        if (sec.key === "host") ch.fig.hidden = true;     // (shown from the first reading the device gives of it: itself())
        return ch.fig;
      }));
      this.charts.push({ cv: spark, val: now, sec, sig: sec.signals.find(s => s.k === sec.head), spark: true, det });
      const note = el("p", { className: "off", hidden: true });
      if (sec.person) this.notes[sec.key] = note;
      const aboutText = el("span", { textContent: sec.about }), secPart = READ["sec:" + (sec.sound ? "sound" : sec.key)];
      const about = el("p", { className: "about" }, aboutText, secPart ? " " : null,
                       secPart ? this.readButton(secPart, "What each reading means", "Every reading of this section explained: what it measures, its values and why it matters", "more") : null);
      this.abouts[sec.key] = { el: aboutText, base: sec.about };
      det.append(el("summary", {}, el("span", { className: "name", textContent: sec.title }), spark, now), about, note, grid, this.axis());
      det.ontoggle = () => { this.saveOpen(); this.draw(); };
      if (sec.key === "host") det.hidden = true;          // (a device that says nothing of itself has no such section)
      return det;
    }

    // ---- drawing
    from(ch) {
      const m = this.model;
      return (ch.sec.sound ? m.sound.get(ch.sec.sound) : ch.sec.person ? m.people.get(this.who) : m.device) || EMPTY;
    }
    tokens() {
      const cs = getComputedStyle(this.root);
      for (const k of ["fg", "muted", "line", "accent", "warn", "panel", "mono", "x0", "x1", "x2", "x3", "x4", "x5", "x6"]) this.T[k] = cs.getPropertyValue("--" + k).trim();
    }
    fit(cv) {
      const d = window.devicePixelRatio || 1, w = Math.max(10, cv.clientWidth), h = Math.max(10, cv.clientHeight);
      if (cv.width !== Math.round(w * d) || cv.height !== Math.round(h * d)) { cv.width = Math.round(w * d); cv.height = Math.round(h * d); }
      const c = cv.getContext("2d");
      c.setTransform(d, 0, 0, d, 0, 0);
      c.clearRect(0, 0, w, h);
      return [c, w, h];
    }
    pointed(ch, t1) { return this.hoverT != null && !ch.spark ? this.hoverT : t1; }
    // a signal's row at the time read: of a sound interface its latest reading before then
    rowFor(ch, t) { const rows = this.from(ch).rows; return ch.sec.sound ? rowBefore(rows, t) : rowNear(rows, t); }
    line(c, x0, y0, x1, y1, color, dash = []) {
      c.save(); c.strokeStyle = color; c.lineWidth = 1; c.setLineDash(dash);
      c.beginPath(); c.moveTo(x0, y0); c.lineTo(x1, y1); c.stroke(); c.restore();
    }
    eventColor(e) { return e.kind === "unusual" || e.kind === "unknown" ? this.T.warn : e.kind === "song" ? this.T.muted : this.T.accent; }
    backdrop(c, w, h, t0, x0 = 0) {                    // the quarter lines and the events
      for (let i = 1; i < 4; i++) { const x = x0 + Math.round((w - x0) * i / 4) + 0.5; this.line(c, x, 0, x, h, this.T.line); }
      for (const e of this.model.events) if (e.t >= t0) {
        const x = x0 + Math.round((e.t - t0) / this.windowMs * (w - x0)) + 0.5;
        c.globalAlpha = 0.7; this.line(c, x, 0, x, h, this.eventColor(e), [3, 3]); c.globalAlpha = 1;
      }
    }
    pointer(c, w, h, t0, x0 = 0) {                     // the time pointed at
      if (this.hoverT == null) return;
      const x = x0 + Math.round((this.hoverT - t0) / this.windowMs * (w - x0)) + 0.5;
      this.line(c, x, 0, x, h, this.T.muted);
    }
    tell(ch, text, hot, none) {
      if (ch.val.textContent !== text) ch.val.textContent = text;
      ch.val.classList.toggle("hot", !!hot);
      ch.val.classList.toggle("na", !!none);
      if (!ch.spark && ch.said !== text) { ch.said = text; ch.cv.setAttribute("aria-label", `${ch.sig.label}: ${text}`); }
    }
    // filled stretches of time where on(row) holds; rows further apart than `gap` are not joined
    runs(c, rows, t0, X, on, y, h, gap = GAP_MS) {
      let start = null, last = null;
      const end = until => { c.fillRect(X(start), y, Math.max(1, X(until) - X(start)), h); start = null; };
      for (let i = firstFrom(rows, t0 - gap); i < rows.length; i++) {
        const r = rows[i];
        if (start != null && r.t - last > gap) end(last + 1000);
        const o = on(r);
        if (o && start == null) start = r.t;
        if (!o && start != null) end(r.t);
        last = r.t;
      }
      if (start != null) end(last + 1000);
    }
    // one row for each pixel across (the first that falls on it), with how wide it is to be drawn
    columns(rows, t0, X, each) {
      let lastX = null;
      for (let i = firstFrom(rows, t0); i < rows.length; i++) {
        const r = rows[i], x = Math.floor(X(r.t));
        if (x === lastX) continue;
        lastX = x;
        const next = rows[i + 1], until = next && next.t - r.t <= GAP_MS ? Math.floor(X(next.t)) : Math.floor(X(r.t + 1000));
        each(r, x, Math.max(1, until - x));
      }
    }

    drawLine(ch, t1) {
      const [c, w, h] = this.fit(ch.cv), s = ch.sig, src = this.from(ch), T = this.T, t0 = t1 - this.windowMs, pad = ch.spark ? 3 : 6;
      const sparse = !!ch.sec.sound, gap = sparse ? LISTEN_GAP_MS : GAP_MS, usual = s.base ? src.usual?.[s.k] : null;
      const rows = src.rows, from = firstFrom(rows, t0 - gap);
      const X = t => (t - t0) / this.windowMs * w;
      const odd = r => !!s.base && r.v.z?.[s.k] != null && Math.abs(r.v.z[s.k]) >= 2;
      const pts = points(rows, from, s.k, X, odd);
      const [lo, hi] = range(s, pts.flatMap(p => [p.lo, p.hi]), usual);
      const Y = v => h - pad - (clamp(v, lo, hi) - lo) / (hi - lo) * (h - 2 * pad);
      if (!ch.spark) {
        if (usual) { c.fillStyle = T.accent; c.globalAlpha = 0.14; c.fillRect(0, Y(usual[0] + 2 * usual[1]), w, Y(usual[0] - 2 * usual[1]) - Y(usual[0] + 2 * usual[1])); c.globalAlpha = 1; }
        this.backdrop(c, w, h, t0);
        if (s.zero) this.line(c, 0, Math.round(Y(0)) + 0.5, w, Math.round(Y(0)) + 0.5, T.line);
        const mark = s.mark ? rowNear(this.model.device.rows, this.pointed(ch, t1), 5000)?.v[s.mark] : null;
        if (mark != null) this.line(c, 0, Math.round(Y(mark)) + 0.5, w, Math.round(Y(mark)) + 0.5, T.warn, [4, 4]);
      }
      const calm = new Path2D(), hot = new Path2D();
      let prev = null;
      for (const p of pts) {
        const path = p.hot ? hot : calm;
        if (prev && !p.gap && p.t0 - prev.t1 <= gap) { path.moveTo(prev.x, Y(prev.last)); path.lineTo(p.x, Y(p.first)); }
        if (p.lo !== p.hi) { path.moveTo(p.x, Y(p.lo)); path.lineTo(p.x, Y(p.hi)); }
        prev = p;
      }
      c.lineJoin = c.lineCap = "round"; c.lineWidth = ch.spark ? 1.25 : 1.6;
      c.strokeStyle = ch.spark ? T.muted : T.fg; c.stroke(calm);
      c.strokeStyle = T.warn; c.stroke(hot);
      if (sparse) { c.fillStyle = ch.spark ? T.muted : T.fg; for (const p of pts) { c.beginPath(); c.arc(p.x, Y(p.last), ch.spark ? 1.6 : 2.2, 0, 7); c.fill(); } }
      const at = this.rowFor(ch, this.pointed(ch, t1)), v = at ? at.v[s.k] ?? null : null;
      if (!ch.spark) {
        this.pointer(c, w, h, t0);
        if (v != null && !sparse) { c.fillStyle = odd(at) ? T.warn : T.fg; c.beginPath(); c.arc(clamp(X(at.t), 3, w - 3), Y(v), 2.6, 0, 7); c.fill(); }
      }
      if (s.k === "odd") this.tell(ch, v == null ? "not in view" : v >= ODD_SIGNALS ? `unusual · ${v} signals` : "usual", v >= ODD_SIGNALS, v == null);
      else this.tell(ch, ch.spark ? `${s.label} ${fmt(s, v)}` : fmt(s, v), at && odd(at), v == null);
    }

    drawFlag(ch, t1) {
      const [c, w, h] = this.fit(ch.cv), s = ch.sig, src = this.from(ch), t0 = t1 - this.windowMs;
      const X = t => clamp((t - t0) / this.windowMs, 0, 1) * w;
      this.backdrop(c, w, h, t0);
      c.fillStyle = this.T.warn; c.globalAlpha = 0.8;
      this.runs(c, src.rows, t0, X, r => r.v[s.k] === true, 4, h - 8);
      c.globalAlpha = 1;
      this.pointer(c, w, h, t0);
      const at = this.rowFor(ch, this.pointed(ch, t1)), v = at ? at.v[s.k] ?? null : null;
      this.tell(ch, fmt(s, v), v === true, v == null);
    }

    drawStack(ch, t1) {
      const [c, w, h] = this.fit(ch.cv), src = this.from(ch), t0 = t1 - this.windowMs, labels = this.model.labels;
      this.columns(src.rows, t0, t => (t - t0) / this.windowMs * w, (r, x, cw) => {
        if (!r.v.probs) return;
        const sum = r.v.probs.reduce((a, b) => a + b, 0) || 1;
        let y = h;
        r.v.probs.forEach((p, i) => { const part = p / sum * h; c.fillStyle = this.T["x" + i % 7]; c.fillRect(x, y - part, cw, part + 0.5); y -= part; });
      });
      this.backdrop(c, w, h, t0);
      this.pointer(c, w, h, t0);
      if (ch.labels !== labels.join("\n")) {                       // the looks the recognizer names
        ch.labels = labels.join("\n");
        ch.shares = labels.map(() => el("b"));
        const legend = this.swatches(labels, labels.map((l, i) => "x" + i % 7), ch.shares);
        ch.legend.replaceWith(legend);
        ch.legend = legend;
      }
      const at = this.rowFor(ch, this.pointed(ch, t1)), probs = at ? at.v.probs : null;
      const sum = probs ? probs.reduce((a, b) => a + b, 0) || 1 : 1, top = probs ? probs.indexOf(Math.max(...probs)) : -1;
      this.tell(ch, probs ? `${labels[top] ?? "look " + (top + 1)} ${Math.round(probs[top] / sum * 100)}%` : "–", false, !probs);
      ch.shares.forEach((b, i) => { const said = probs && probs[i] != null ? Math.round(probs[i] / sum * 100) + "%" : "–"; if (b.textContent !== said) b.textContent = said; });
    }

    drawHeat(ch, t1) {
      const [c, w, h] = this.fit(ch.cv), src = this.from(ch), T = this.T, t0 = t1 - this.windowMs, pw = w - GUTTER;
      c.font = `10px ${T.mono}`; c.textBaseline = "middle"; c.fillStyle = T.muted;
      BASE.forEach((s, i) => c.fillText(s.row || s.label, 0, i * ROW + ROW / 2));
      this.columns(src.rows, t0, t => GUTTER + clamp((t - t0) / this.windowMs, 0, 1) * pw, (r, x, cw) => {
        BASE.forEach((s, i) => {
          if (r.v[s.k] == null) return;                            // no reading: left empty
          c.globalAlpha = 1; c.fillStyle = T.line; c.fillRect(x, i * ROW + 1, cw, ROW - 2);
          const z = r.v.z[s.k], a = z == null ? 0 : clamp((Math.abs(z) - 1) / 2, 0, 1);
          if (a > 0) { c.globalAlpha = a; c.fillStyle = T.warn; c.fillRect(x, i * ROW + 1, cw, ROW - 2); }
        });
      });
      c.globalAlpha = 1;
      this.backdrop(c, w, h, t0, GUTTER);
      this.pointer(c, w, h, t0, GUTTER);
      const at = this.rowFor(ch, this.pointed(ch, t1)), n = at ? at.v.odd : null;
      const learned = at ? BASE.filter(s => at.v.z[s.k] != null).length : 0;
      this.tell(ch, n == null ? "–" : learned ? `${n} of ${learned} beyond 2 SD` : "their usual isn't learned yet", n >= ODD_SIGNALS, n == null || !learned);
    }

    // what an interface heard, in a word, as blocks of colour from one listening to the next
    drawCat(ch, t1) {
      const [c, w, h] = this.fit(ch.cv), s = ch.sig, src = this.from(ch), t0 = t1 - this.windowMs;
      const X = t => clamp((t - t0) / this.windowMs, 0, 1) * w;
      let cur = null;
      const block = until => {
        if (!cur || cur.v == null || until <= t0) return;
        c.fillStyle = this.T[WHAT_COLOR[cur.v]] || this.T.muted;
        c.fillRect(X(cur.t), 0, Math.max(1, X(until) - X(cur.t)), h);
      };
      for (let i = Math.max(0, firstFrom(src.rows, t0) - 1); i < src.rows.length; i++) {
        const r = src.rows[i];
        block(Math.min(r.t, cur ? cur.t + LISTEN_GAP_MS : r.t));
        cur = { t: r.t, v: r.v[s.k] };
      }
      if (cur) block(Math.min(t1, cur.t + LISTEN_GAP_MS / 2));
      this.backdrop(c, w, h, t0);
      this.pointer(c, w, h, t0);
      const at = this.rowFor(ch, this.pointed(ch, t1)), v = at ? at.v[s.k] : null;
      this.tell(ch, v == null ? "–" : WHAT[v], false, v == null);
    }

    // who is in view: one lane a person, and one for unknown faces
    drawLanes(ch, t1) {
      const names = this.model.names().slice(0, MAX_LANES).sort((a, b) => (a === "Unknown") - (b === "Unknown") || a.localeCompare(b));
      const tall = Math.max(1, names.length) * LANE + "px";
      if (ch.cv.style.height !== tall) ch.cv.style.height = tall;
      const [c, w, h] = this.fit(ch.cv), T = this.T, t0 = t1 - this.windowMs, pw = w - GUTTER;
      const X = t => GUTTER + clamp((t - t0) / this.windowMs, 0, 1) * pw;
      c.font = `10px ${T.mono}`; c.textBaseline = "middle";
      names.forEach((name, i) => {
        let shown = name;
        while (shown.length > 2 && c.measureText(shown).width > GUTTER - 8) shown = shown.slice(0, -2) + "…";
        c.fillStyle = T.muted; c.fillText(shown, 0, i * LANE + LANE / 2);
        c.fillStyle = name === "Unknown" ? T.warn : T.accent;
        this.runs(c, this.model.people.get(name).rows, t0, X, () => true, i * LANE + 4, LANE - 8);
      });
      this.backdrop(c, w, h, t0, GUTTER);
      this.pointer(c, w, h, t0, GUTTER);
      const at = rowNear(this.model.device.rows, this.pointed(ch, t1)), n = at ? at.v.faces : null;
      this.tell(ch, n == null ? "–" : `${n} in view`, false, n == null);
    }

    drawEvents(t1) {
      const [c, w, h] = this.fit(this.events), T = this.T, t0 = t1 - this.windowMs;
      this.line(c, 0, h - 6.5, w, h - 6.5, T.line);
      for (let i = 0; i <= 4; i++) { const x = clamp(Math.round(w * i / 4), 0, w - 1) + 0.5; this.line(c, x, h - 10, x, h - 3, T.line); }
      c.font = `11px ${T.mono}`; c.textBaseline = "top";
      const inView = this.model.events.filter(e => e.t >= t0);
      inView.forEach((e, i) => {                                   // labels on three rows in turn, so neighbours stay readable
        const x = Math.round((e.t - t0) / this.windowMs * w) + 0.5, y = 3 + (i % 3) * 14;
        this.line(c, x, y, x, h - 3, this.eventColor(e));
        c.fillStyle = this.eventColor(e);
        const tw = c.measureText(e.text).width;
        c.fillText(e.text, x + 5 + tw > w ? x - 5 - tw : x + 5, y);
      });
      this.pointer(c, w, h, t0);
      const said = inView.length ? "Events in the time shown: " + inView.map(e => e.text).join(", ") : "No events in the time shown";
      if (this.eventsSaid !== said) { this.eventsSaid = said; this.events.setAttribute("aria-label", said); }
    }

    // New readings are drawn soon, and not more often than the eye follows; a hidden chart isn't drawn at all,
    // its readings go on being kept.
    later() {
      if (this.due) return;
      this.due = true;
      setTimeout(() => requestAnimationFrame(() => { this.due = false; this.draw(); }), Math.max(0, DRAW_EVERY_MS - (performance.now() - this.drawn)));
    }
    // Everything that can be seen, drawn again.
    draw() {
      if (!this.root.isConnected || !this.root.offsetParent) return;
      this.drawn = performance.now();
      this.tokens();
      const t1 = Date.now();
      for (const ch of this.charts) {
        if (!ch.spark && !ch.det.open) continue;
        if (ch.det.hidden || ch.fig?.hidden) continue;
        if (ch.sec.person && !this.model.people.size) continue;
        ({ stack: this.drawStack, heat: this.drawHeat, flag: this.drawFlag, cat: this.drawCat, lanes: this.drawLanes }[ch.sig.kind] || this.drawLine).call(this, ch, t1);
      }
      this.drawEvents(t1);
      const ago = this.windowMs < 120000 ? `${this.windowMs / 1000} s ago` : `${this.windowMs / 60000} min ago`;
      const at = this.hoverT == null ? "" : new Date(this.hoverT).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
      for (const n of this.root.querySelectorAll(".axis .from")) if (n.textContent !== ago) n.textContent = ago;
      for (const n of this.root.querySelectorAll(".axis .at")) if (n.textContent !== at) n.textContent = at;
    }
  }

  // ------------------------------------------------------------------ the history's rows
  // What the signal history keeps (the owner, 2026-11-24: "Keep a history of ALL signals in the database", opt-in
  // on each device): not every reading, but every reading's part in one row for each stretch of HISTORY_MS, a
  // row for the device itself (name "") and a row for each named person in view. A row holds, for every number
  // that was read in the stretch, [average, lowest, highest, how many readings]; for a yes-or-no the same with
  // yes as 1 (so the average is the share of yes); for a word (the look that won, the heat state, what a sound
  // interface heard) how often each was read. For a person also "looks" (each look's average share), "z" (how far
  // from the person's own usual each signal was at its furthest, in spreads, with its sign) and "odd" (the most
  // signals that were unusual at one reading). A face nobody is named for has no row: it is counted in the
  // device's row ("unknown"). No row names another person, so a person's rows are that person's alone.
  // HISTORY_MS is a choice, not a measured value: short enough that a quick change shows as a step between two
  // rows or a wide lowest-to-highest inside one, long enough that a person in view all day is a few thousand rows.
  const HISTORY_MS = 10000;
  const r4 = x => Math.round(x * 1e4) / 1e4;
  class Stat {
    constructor() { this.n = 0; this.sum = 0; this.min = Infinity; this.max = -Infinity; }
    add(v) { this.n++; this.sum += v; if (v < this.min) this.min = v; if (v > this.max) this.max = v; }
    out() { return [r4(this.sum / this.n), r4(this.min), r4(this.max), this.n]; }
  }
  const statsOut = bag => Object.fromEntries(Object.entries(bag).map(([k, st]) => [k, st.out()]));
  class History {
    constructor(stretch = HISTORY_MS) { this.stretch = stretch; this.rows = []; this.at = null; this.begin(); }
    begin() { this.n = 0; this.dev = {}; this.words = {}; this.people = new Map(); this.sound = new Map(); this.heard = null; }
    // One reading (raw, as Model.sample takes it; a face may also say "look", the look its reading settled on, and
    // "state"), at its time t. A reading of a later stretch closes the one before.
    add(t, raw) {
      const start = Math.floor(t / this.stretch) * this.stretch;
      if (this.at !== null && start !== this.at) this.close();
      if (this.at === null) this.at = start;
      const s = clean(raw), given = Array.isArray(obj(raw).faces) ? raw.faces : [];
      const stat = (bag, k, v) => { if (v != null) (bag[k] ||= new Stat()).add(v); };
      const word = (bag, k, w) => { if (w) { const c = bag[k] ||= {}; c[w] = (c[w] || 0) + 1; } };
      this.n++;
      for (const k of ["checks", "check", ...HOST.filter(k => k !== "heat")]) stat(this.dev, k, s.device[k]);
      if (s.device.charging != null) stat(this.dev, "charging", s.device.charging ? 1 : 0);
      if (s.device.heat != null) word(this.words, "heat", HEAT[s.device.heat]);
      stat(this.dev, "faces", s.faces.length);
      stat(this.dev, "unknown", s.faces.filter(f => f.name === "Unknown").length);
      // a listening is one reading however many checks of the picture it lasts: taken once, by its number
      if (s.sound && s.sound.seq !== this.heard) {
        this.heard = s.sound.seq;
        for (const i of s.sound.interfaces) {
          const key = i.kind + "\n" + i.name;
          const io = this.sound.get(key) || this.sound.set(key, { name: i.name, kind: i.kind, nums: {}, words: {} }).get(key);
          for (const k of SOUND_KEYS) stat(io.nums, k, i.v[k]);
          if (i.v.what != null) word(io.words, "what", WHAT[i.v.what]);
        }
      }
      s.faces.forEach((f, i) => {
        if (f.name === "Unknown" || f.name.startsWith("#")) return;
        const p = this.people.get(f.name) || this.people.set(f.name, { n: 0, nums: {}, words: {}, looks: null, labels: null, z: {}, odd: 0 }).get(f.name);
        p.n++;
        for (const k of SCALARS) stat(p.nums, k, f.nums[k]);
        for (const k of FLAGS) if (f.nums[k] != null) stat(p.nums, k, f.nums[k] ? 1 : 0);
        const labels = s.device.labels;
        if (f.nums.probs && labels && labels.length === f.nums.probs.length && (!p.labels || p.labels.length === labels.length)) {
          p.labels = labels; p.looks ||= labels.map(() => new Stat());
          f.nums.probs.forEach((v, j) => p.looks[j].add(v));
        }
        let odd = 0;
        for (const b of BASE) {
          const u = f.usual[b.k], v = f.nums[b.k];
          if (!u || v == null) continue;
          const z = (v - u[0]) / u[1];
          if (Math.abs(z) >= 2) odd++;
          if (p.z[b.k] == null || Math.abs(z) > Math.abs(p.z[b.k])) p.z[b.k] = z;
        }
        if (odd > p.odd) p.odd = odd;
        const said = obj(given[i]);
        word(p.words, "look", text(said.look, 24));
        word(p.words, "state", text(said.state, 20));
      });
    }
    // The stretch is over: its rows are made. (Rows nobody took are not kept without end: the oldest go.)
    close() {
      if (this.at === null) return;
      const t = this.at, s = this.stretch / 1000;
      if (this.n) {
        const data = { ...statsOut(this.dev), ...this.words };
        if (this.sound.size) data.sound = [...this.sound.values()].slice(0, 24).map(io => ({ name: io.name, kind: io.kind, ...statsOut(io.nums), ...io.words }));
        this.rows.push({ t, s, name: "", n: this.n, data });
        for (const [name, p] of this.people) {
          const d = { ...statsOut(p.nums), ...p.words };
          if (p.looks) d.looks = Object.fromEntries(p.labels.map((l, j) => [l, r4(p.looks[j].sum / p.looks[j].n)]));
          const z = Object.fromEntries(Object.entries(p.z).map(([k, v]) => [k, Math.round(Math.max(-99, Math.min(99, v)) * 100) / 100]));
          if (Object.keys(z).length) d.z = z;
          if (p.odd) d.odd = p.odd;
          this.rows.push({ t, s, name, n: p.n, data: d });
        }
      }
      this.at = null; this.begin();
      if (this.rows.length > 2000) this.rows.splice(0, this.rows.length - 2000);
    }
    // The rows of the stretches that are over (now: this moment, which may end the current one), each once.
    take(now) {
      if (this.at !== null && now >= this.at + this.stretch) this.close();
      const rows = this.rows; this.rows = [];
      return rows;
    }
  }

  const api = { Panel, Model, clean, OFF: Object.keys(OFF), cleanNums, cleanUsual, cleanFacts, HOST, HEAT, FACTS, catalog, settle, History, HISTORY_MS,
                DEVICE: DEVICE.map(sec => ({ key: sec.key, signals: sec.signals.map(s => s.k) })), pack, unpack, thin, firstFrom, rowNear, rowBefore, points, fmt, range,
                WHAT, LABELS, SCALARS, FLAGS, BASE: BASE.map(s => s.k),
                C: { KEEP_MS, MAX_ROWS, GAP_MS, LISTEN_GAP_MS, ARRIVE_GAP_MS, ODD_SIGNALS, ODD_FOR_MS, CALM_FOR_MS, SENT_MS, DRAW_EVERY_MS } };
  if (typeof module === "object" && module.exports) module.exports = api;
  else root.SignalsChart = api;
})(typeof window !== "undefined" ? window : globalThis);
