// expression.js - reading a face's expression over time, for the live tracker page (tracker_ui.html).
//
// The recognizer scores each sharp, camera-facing face with 7 expression probabilities. Here they are
// averaged over the last ten checks (about a second), weighted by face quality, and only a clear
// winner is reported. For a named person the tracker also learns their usual look (a slow running
// average, kept in this browser) so that a face that always reads a bit "sad" or always smiles isn't
// reported as news: a reading is highlighted when it departs from that person's usual, and tagged
// "usual" when it matches it.
//
// Each reading also carries its evidence (votes, share, lead, why frames were skipped, how far the
// usual look has been learned, face-shape cues, a rough valence/arousal) for the evidence view and
// for tuning. The cues and valence/arousal are shown only; they never change the label.
//
// Runs in the browser (window.Expression) and in Node for the tests (tests/expression.test.js).
(function (root) {
  "use strict";

  // Measured on stored faces (95% give a clear winner): don't change without re-measuring.
  const X_SURE = 0.45, X_MARGIN = 0.12, X_MIN_VOTES = 3;   // winner needs this share, this lead, this many frames
  const X_WINDOW = 10;              // scored checks that vote (about a second)
  const X_STALE = 10;               // unscored checks in a row before the reading is dropped rather than left stale
  const X_FLOOR = 0.08;             // a person's usual share of a label is never taken below this (rare labels can't be inflated)
  const X_RATE = 0.005, X_SETTLED = 100;   // usual look: running-average rate per check, checks before it's trusted
  const X_LEARN_Q = 0.3;            // the usual look learns only from frames at least this sharp
  const X_CHANGE_SHARE = 0.25;      // a departure from the usual must also hold this much of the plain vote
  const STORE_KEY = "ft-usual";

  // Rough place of each label on the valence (unpleasant..pleasant) / arousal (calm..excited) circle.
  // Textbook placements, not measured: shown in the evidence view only.
  const VA = {
    angry: [-0.6, 0.7], disgust: [-0.7, 0.3], fearful: [-0.6, 0.8], happy: [0.8, 0.5],
    neutral: [0, 0], sad: [-0.7, -0.4], surprised: [0.3, 0.8],
  };

  // Face-shape cues that usually go with each label (FACS-style: happy = cheek raise + lip-corner
  // pull, surprise = brows up + eyes wide + jaw drop...). Shown as evidence next to the label.
  const CUE_SIGNS = {
    angry: ["browDown", "eyeSquint", "press"], disgust: ["sneer", "upperLip"],
    fearful: ["browInnerUp", "eyeWide", "stretch"], happy: ["smile", "cheek"], neutral: [],
    sad: ["browInnerUp", "frown"], surprised: ["browOuterUp", "eyeWide", "jaw"],
  };

  // The recent expression checks of one tracked face.
  class Track {
    constructor() { this.clear(); }
    clear() { this.votes = []; this.cues = []; this.miss = 0; this.skip = null; this.fresh = false; }

    // One check of this face: probs is null when the recognizer couldn't read it, and skip says why
    // ("blurry", "turned"). Such checks cast no vote; after X_STALE of them in a row the reading is dropped.
    add(probs, quality, skip, cues) {
      this.fresh = !!probs;
      this.skip = probs ? null : skip || null;
      if (probs) {
        const weight = quality == null ? 0.5 : Math.max(0.05, quality);   // sharp frames outvote blurry ones
        this.votes.push({ probs, weight, quality });
        if (this.votes.length > X_WINDOW) this.votes.shift();
        if (cues) { this.cues.push(cues); if (this.cues.length > X_WINDOW) this.cues.shift(); }
        this.miss = 0;
      } else if (++this.miss > X_STALE) {
        this.votes = []; this.cues = [];
      }
    }
  }

  // Each named person's usual look: a running average of their expression probabilities.
  class UsualLooks {
    constructor(store) {        // store: localStorage, or anything with getItem/setItem/removeItem
      this.store = store; this.saved = 0;
      try { this.looks = JSON.parse(store?.getItem(STORE_KEY) || "{}"); } catch { this.looks = {}; }
    }
    learn(name, probs, quality) {
      if (quality != null && quality < X_LEARN_Q) return;            // learn only from clear frames
      const b = this.looks[name] || (this.looks[name] = { p: probs.slice(), n: 0 });
      const rate = Math.max(X_RATE, 1 / (b.n + 1));     // plain average at first, slow running average once settled
      for (let i = 0; i < probs.length; i++) b.p[i] += (probs[i] - b.p[i]) * rate;
      b.n++;
      if (Date.now() - this.saved > 5000) this.save();
    }
    save() { this.saved = Date.now(); try { this.store?.setItem(STORE_KEY, JSON.stringify(this.looks)); } catch {} }
    progress(name) { return Math.min(1, (this.looks[name]?.n || 0) / X_SETTLED); }   // 1 = trusted
    usual(name) { const b = this.looks[name]; return b && b.n >= X_SETTLED ? b.p : null; }
    forget() { this.looks = {}; try { this.store?.removeItem(STORE_KEY); } catch {} }
  }

  const rank = q => q.map((v, i) => i).sort((i, j) => q[j] - q[i]);
  const mean = (rows, key) => rows.reduce((a, r) => a + (r[key] || 0), 0) / rows.length;

  // The current reading of a track, or null when there's nothing to read.
  //   labels      expression names, in the recognizer's order
  //   name        the person's name ("Unknown" is never calibrated)
  //   usualLooks  a UsualLooks to learn from and compare with, or null when calibration is off
  // Returns { label, share, lead, votes, state, flags, va, cues, learning }:
  //   state  "plain" clear winner | "change" departs from this person's usual | "usual" matches it |
  //          "unsure" no clear winner (shown dimmed with "?")
  //   flags  why it isn't clear, or what to keep in mind: few-votes, low-share, low-lead, blurry,
  //          turned (the latest frame couldn't be read), learning (usual look not trusted yet)
  function read(track, labels, name, usualLooks) {
    if (!labels || !track.votes.length) return null;
    const n = labels.length, p = new Array(n).fill(0);
    let ws = 0;
    for (const { probs, weight } of track.votes) { for (let i = 0; i < n; i++) p[i] += probs[i] * weight; ws += weight; }
    for (let i = 0; i < n; i++) p[i] /= ws;
    const votes = track.votes.length;
    const flagsFor = (q, top, second) => [
      ...(votes < X_MIN_VOTES ? ["few-votes"] : []),
      ...(q[top] < X_SURE ? ["low-share"] : []),
      ...(q[top] - q[second] < X_MARGIN ? ["low-lead"] : []),
    ];
    const reading = (q, top, second, state, extra) => {
      const label = labels[top];
      const cueRows = track.cues;
      return {
        label, share: p[top], lead: q[top] - q[second], votes, state,
        flags: [...(state === "unsure" ? flagsFor(q, top, second) : []), ...extra],
        va: p.reduce((a, v, i) => [a[0] + v * (VA[labels[i]]?.[0] || 0), a[1] + v * (VA[labels[i]]?.[1] || 0)], [0, 0]),
        cues: cueRows.length ? Object.fromEntries((CUE_SIGNS[label] || []).map(c => [c, mean(cueRows, c)])) : null,
      };
    };
    const [r0, r1] = rank(p);
    const extra = track.skip ? [track.skip] : [];
    const calibrating = usualLooks && name && name !== "Unknown";
    let usual = null, learning = null;
    if (calibrating) {
      const latest = track.votes[votes - 1];
      if (track.fresh) usualLooks.learn(name, latest.probs, latest.quality);   // each check counts once
      usual = usualLooks.usual(name);
      if (!usual) { learning = usualLooks.progress(name); extra.push("learning"); }
    }
    let out;
    if (usual) {
      // How much each label stands out from this person's usual: p / usual. With nothing changed every
      // ratio is about 1 and no label is clear; a real change makes one ratio large.
      const q = p.map((v, i) => v / Math.max(usual[i], X_FLOOR)), qs = q.reduce((a, v) => a + v, 0);
      for (let i = 0; i < n; i++) q[i] /= qs;
      const [c0, c1] = rank(q);
      if (flagsFor(q, c0, c1).length === 0 && p[c0] >= X_CHANGE_SHARE) out = reading(q, c0, c1, "change", extra);
      else out = reading(p, r0, r1, flagsFor(p, r0, r1).length === 0 ? "usual" : "unsure", extra);
    } else {
      out = reading(p, r0, r1, flagsFor(p, r0, r1).length === 0 ? "plain" : "unsure", extra);
    }
    out.learning = learning;
    return out;
  }

  // Short text drawn under the name: "happy", "sad?" (unsure), "neutral · usual".
  function text(e) {
    if (!e) return "";
    return e.state === "unsure" ? e.label + "?" : e.state === "usual" ? e.label + " · usual" : e.label;
  }

  // One-line evidence for the readout: "8 votes · 62% · lead 21 · V+0.5 A+0.4 · smile .82 cheek .61".
  function evidence(e) {
    if (!e) return "";
    const sign = v => (v >= 0 ? "+" : "") + v.toFixed(1);
    const parts = [`${e.votes} vote${e.votes === 1 ? "" : "s"}`, `${Math.round(e.share * 100)}%`,
                   `lead ${Math.round(e.lead * 100)}`, `V${sign(e.va[0])} A${sign(e.va[1])}`];
    if (e.cues && Object.keys(e.cues).length)
      parts.push(Object.entries(e.cues).map(([c, v]) => `${c} ${v.toFixed(2).replace(/^0/, "")}`).join(" "));
    if (e.learning != null) parts.push(`usual look ${Math.round(e.learning * 100)}% learned`);
    const flags = e.flags.filter(f => f !== "learning");
    if (flags.length) parts.push(flags.join(", "));
    return parts.join(" · ");
  }

  const api = { Track, UsualLooks, read, text, evidence, VA, CUE_SIGNS,
                X: { X_SURE, X_MARGIN, X_MIN_VOTES, X_WINDOW, X_STALE, X_FLOOR, X_RATE, X_SETTLED, X_LEARN_Q, X_CHANGE_SHARE } };
  if (typeof module === "object" && module.exports) module.exports = api;
  else root.Expression = api;
})(typeof window !== "undefined" ? window : globalThis);
