// signals.js - signals besides the expression label, for the live tracker page (tracker_ui.html).
//
// The expression model reads what one frame of a face looks like. A feeling, if it shows at all,
// shows in several channels that fail in different ways, so these are measured separately:
//
//   motion     how fast the head moves (face widths per second) and whether it's coming closer
//   talking    jaw movement over the last 1.5 s; speech moves the mouth in ways the expression model
//              misreads as "surprised" or "happy". With the microphone on, voice activity confirms it.
//   blinks     blinks per minute (rough: the tracker checks a face about 10 times a second and a
//              blink lasts 100-300 ms, so some are missed)
//   gaze       how often the eyes look well away from straight ahead
//   timing     how fast a smile rises, how symmetric it is, how long the current reading has held
//              (spontaneous smiles tend to rise smoothly and be symmetric; posed ones less so)
//   pulse      heart rate from tiny skin-colour changes in the forehead and cheeks (remote
//              photoplethysmography, the POS method of Wang et al. 2017). Needs a still face and
//              steady light; reported with its signal-to-noise ratio, and only when the face is still.
//   voice      from the microphone, analysed here and never stored: loudness above the room's noise,
//              pitch and its variability, speaking rate, plus the voice model's arousal/valence
//              (tracker.py scores short clips with Wav2Small).
//
// All of these are evidence: shown, never used to change the expression label. The one exception is
// the page's "skip expression while talking" option (off by default). None of the thresholds below
// are measured yet: tools/expr_eval.py measures the talking detector on posed test sessions and fits
// valence/arousal from these signals to your own feeling reports (applyFit uses that fit).
//
// Runs in the browser (window.Signals) and in Node for the tests (tests/signals.test.js).
(function (root) {
  "use strict";

  // Not measured yet: change them only from tools/expr_eval.py results.
  const TALK_JAW_SD = 0.035, TALK_MS = 1500, TALK_MIN = 4;   // jaw-opening spread over 1.5 s that counts as talking
  const MOTION_MS = 1000;
  const BLINK_ON = 0.5, BLINK_MIN_MS = 20000, BLINK_KEEP_MS = 60000;
  const GAZE_AWAY = 0.35;
  const SMILE_ON = 0.3;
  const SUMMARY_MS = 30000;                       // a feeling report summarizes this much before the question
  const PULSE_MS = 10000, PULSE_MIN_MS = 6000, PULSE_HZ = 30, PULSE_BPM = [42, 180], PULSE_MAX_MOTION = 0.15;
  const V_RATE = 16000, V_FRAME = 320, V_KEEP_S = 10;   // voice: 20 ms frames at 16 kHz, 10 s kept
  const V_ON_DB = 9;                              // a frame this much louder than the room's noise counts as voice
  const V_PITCH = [75, 400], V_PITCH_R = 0.5;     // pitch range (Hz) and the correlation needed to call it voiced

  // Features of a feeling report, in a fixed order (tools/expr_eval.py fits valence/arousal on them).
  const LABELS = ["angry", "disgust", "fearful", "happy", "neutral", "sad", "surprised"];
  const FACE_CUES = ["smile", "cheek", "frown", "browDown", "browInnerUp", "eyeWide", "jaw"];
  const FEATURES = [...LABELS.map(l => "p_" + l), ...FACE_CUES,
    "motion", "approach", "talking", "blinkRate", "gazeAway", "smileAsym",
    "voiceArousal", "voiceDominance", "voiceValence", "speaking", "loudness", "pitchSd", "speechRate", "pulse"];

  const mean = a => a.length ? a.reduce((s, v) => s + v, 0) / a.length : null;
  const sd = a => { const m = mean(a); return a.length > 1 ? Math.sqrt(a.reduce((s, v) => s + (v - m) ** 2, 0) / (a.length - 1)) : 0; };
  const trim = (rows, now, ms) => { while (rows.length && now - rows[0].t > ms) rows.shift(); };
  const since = (rows, now, ms) => rows.filter(r => r.t <= now && now - r.t <= ms);

  // ------------------------------------------------------------------ one tracked face
  class FaceSignals {
    constructor() { this.pos = []; this.cues = []; this.rgb = []; this.exprs = []; this.labels = []; }

    addPosition(t, box) {
      this.pos.push({ t, cx: box[0] + box[2] / 2, cy: box[1] + box[3] / 2, w: box[2] });
      trim(this.pos, t, SUMMARY_MS);
    }
    addCues(t, cues) {
      if (!cues) return;
      this.cues.push({ t, ...cues });
      trim(this.cues, t, Math.max(SUMMARY_MS, BLINK_KEEP_MS));
    }
    addExpr(t, probs) {
      if (!probs) return;
      this.exprs.push({ t, probs });
      trim(this.exprs, t, SUMMARY_MS);
    }
    addLabel(t, label) {                         // the reading shown (null: none)
      const last = this.labels[this.labels.length - 1];
      if (!last || last.label !== label) this.labels.push({ t, label });
      while (this.labels.length > 1 && t - this.labels[1].t > SUMMARY_MS) this.labels.shift();
    }
    addColor(t, rgb) {                           // mean skin colour this animation frame
      this.rgb.push({ t, r: rgb[0], g: rgb[1], b: rgb[2] });
      trim(this.rgb, t, PULSE_MS);
    }

    // { speed: face widths per second, approach: relative size change per second (> 0 coming closer) }
    motion(now) {
      const p = since(this.pos, now, MOTION_MS);
      if (p.length < 2) return null;
      const dt = (p[p.length - 1].t - p[0].t) / 1000;
      if (!(dt > 0)) return null;
      let d = 0;
      for (let i = 1; i < p.length; i++) d += Math.hypot(p[i].cx - p[i - 1].cx, p[i].cy - p[i - 1].cy) / ((p[i].w + p[i - 1].w) / 2);
      return { speed: d / dt, approach: Math.log(p[p.length - 1].w / p[0].w) / dt };
    }

    // { score: spread of jaw opening, talking } or null without enough face-shape cues
    talking(now) {
      const jaw = since(this.cues, now, TALK_MS).map(c => c.jaw).filter(v => v != null);
      if (jaw.length < TALK_MIN) return null;
      const score = sd(jaw);
      return { score, talking: score >= TALK_JAW_SD };
    }

    blinkRate(now) {                              // per minute, after 20 s of cues
      const c = since(this.cues, now, BLINK_KEEP_MS).filter(x => x.blink != null);
      if (c.length < 2 || c[c.length - 1].t - c[0].t < BLINK_MIN_MS) return null;
      let n = 0;
      for (let i = 1; i < c.length; i++) if (c[i].blink >= BLINK_ON && c[i - 1].blink < BLINK_ON) n++;
      return n * 60000 / (c[c.length - 1].t - c[0].t);
    }

    gazeAway(now, ms = SUMMARY_MS) {              // share of checks with the eyes well off-centre
      const g = since(this.cues, now, ms).map(c => c.gaze).filter(v => v != null);
      return g.length >= 5 ? g.filter(v => v >= GAZE_AWAY).length / g.length : null;
    }

    // { smileRise: fastest smile increase per second in the last 2 s, smileAsym: mean left/right
    //   difference while smiling, held: seconds the current reading has been shown }
    timing(now) {
      const c = since(this.cues, now, 2000).filter(x => x.smile != null);
      let rise = null;
      for (let i = 1; i < c.length; i++) {
        const dt = (c[i].t - c[i - 1].t) / 1000;
        if (dt > 0) rise = Math.max(rise ?? 0, (c[i].smile - c[i - 1].smile) / dt);
      }
      const smiling = since(this.cues, now, SUMMARY_MS).filter(x => x.smile >= SMILE_ON && x.smileAsym != null);
      const last = this.labels[this.labels.length - 1];
      return { smileRise: rise, smileAsym: mean(smiling.map(x => x.smileAsym)),
               held: last && last.label ? (now - last.t) / 1000 : null };
    }

    pulse(now) {
      const m = this.motion(now);
      if (m && m.speed > PULSE_MAX_MOTION) return null;
      return pulseFrom(this.rgb);
    }

    // Numbers for a feeling report: averages over the last 30 s, named as in FEATURES.
    // voice: { stats, avd } from VoiceSignals / the voice model, or null.
    summary(now, voice) {
      const f = {};
      const ex = since(this.exprs, now, SUMMARY_MS);
      LABELS.forEach((l, i) => { f["p_" + l] = ex.length ? mean(ex.map(e => e.probs[i])) : null; });
      const cues = since(this.cues, now, SUMMARY_MS);
      for (const k of FACE_CUES) f[k] = cues.length ? mean(cues.map(c => c[k] ?? 0)) : null;
      const pos = since(this.pos, now, SUMMARY_MS);
      const speeds = [], approach = [];
      for (let t = now; t > now - SUMMARY_MS && pos.length > 1; t -= MOTION_MS) {
        const m = this.motion(t);
        if (m) { speeds.push(m.speed); approach.push(m.approach); }
        if (pos[0].t > t) break;
      }
      f.motion = mean(speeds); f.approach = mean(approach);
      const jaw = [];
      for (let t = now; t > now - SUMMARY_MS; t -= TALK_MS) { const k = this.talking(t); if (k) jaw.push(k.talking ? 1 : 0); }
      f.talking = mean(jaw);
      f.blinkRate = this.blinkRate(now);
      f.gazeAway = this.gazeAway(now);
      f.smileAsym = this.timing(now).smileAsym;
      const s = voice?.stats, avd = voice?.avd;
      f.voiceArousal = avd ? avd[0] : null; f.voiceDominance = avd ? avd[1] : null; f.voiceValence = avd ? avd[2] : null;
      f.speaking = s ? s.speaking : null; f.loudness = s ? s.loudness : null;
      f.pitchSd = s ? s.pitchSd : null; f.speechRate = s ? s.rate : null;
      const p = this.pulse(now);
      f.pulse = p && p.snr > 0 ? p.bpm : null;
      for (const k of Object.keys(f)) if (f[k] != null) f[k] = Math.round(f[k] * 1000) / 1000;
      return f;
    }
  }

  // Skin regions for the pulse, from the 5 face keypoints (eyes, nose, mouth corners): the
  // forehead and the cheeks under each eye, sized by the distance between the eyes.
  function skinRegions(kps) {
    const [e1, e2] = kps;
    const ex = (e1[0] + e2[0]) / 2, ey = (e1[1] + e2[1]) / 2, d = Math.hypot(e2[0] - e1[0], e2[1] - e1[1]);
    if (!(d > 12)) return [];
    return [
      { x: ex - 0.5 * d, y: ey - 0.85 * d, w: d, h: 0.4 * d },
      { x: e1[0] - 0.25 * d, y: ey + 0.35 * d, w: 0.5 * d, h: 0.35 * d },
      { x: e2[0] - 0.25 * d, y: ey + 0.35 * d, w: 0.5 * d, h: 0.35 * d },
    ];
  }

  // Heart rate from mean skin colour over time: rows [{ t ms, r, g, b }]. Returns { bpm, snr } (snr in
  // dB: power at the pulse and its harmonic over the rest of 42-180 bpm), or null with too little data.
  function pulseFrom(rows) {
    if (rows.length < 30 || rows[rows.length - 1].t - rows[0].t < PULSE_MIN_MS) return null;
    const t0 = rows[0].t, n = Math.floor((rows[rows.length - 1].t - t0) / 1000 * PULSE_HZ);
    const R = new Float64Array(n), G = new Float64Array(n), B = new Float64Array(n);
    for (let k = 0, j = 0; k < n; k++) {             // resample evenly at PULSE_HZ
      const t = t0 + k * 1000 / PULSE_HZ;
      while (j < rows.length - 2 && rows[j + 1].t < t) j++;
      const a = rows[j], b = rows[j + 1], u = b.t > a.t ? Math.min(1, Math.max(0, (t - a.t) / (b.t - a.t))) : 0;
      R[k] = a.r + (b.r - a.r) * u; G[k] = a.g + (b.g - a.g) * u; B[k] = a.b + (b.b - a.b) * u;
    }
    // POS: in overlapping 1.6 s windows, normalise each colour by its mean, project onto the plane
    // orthogonal to skin tone, and combine the two projections so motion/lighting changes cancel.
    const L = Math.round(1.6 * PULSE_HZ), h = new Float64Array(n);
    for (let s = 0; s + L <= n; s++) {
      let mr = 0, mg = 0, mb = 0;
      for (let i = s; i < s + L; i++) { mr += R[i]; mg += G[i]; mb += B[i]; }
      mr /= L; mg /= L; mb /= L;
      if (!(mr > 0 && mg > 0 && mb > 0)) continue;
      const s1 = new Float64Array(L), s2 = new Float64Array(L);
      for (let i = 0; i < L; i++) {
        const r = R[s + i] / mr, g = G[s + i] / mg, b = B[s + i] / mb;
        s1[i] = g - b; s2[i] = g + b - 2 * r;
      }
      const a = sd([...s1]) / (sd([...s2]) || 1);
      let m = 0;
      const w = new Float64Array(L);
      for (let i = 0; i < L; i++) { w[i] = s1[i] + a * s2[i]; m += w[i]; }
      m /= L;
      for (let i = 0; i < L; i++) h[s + i] += w[i] - m;
    }
    let hm = 0;
    for (let i = 0; i < n; i++) hm += h[i];
    hm /= n;
    const power = bpm => {
      const f = bpm / 60 / PULSE_HZ * 2 * Math.PI;
      let c = 0, s = 0;
      for (let i = 0; i < n; i++) { const v = (h[i] - hm) * (0.5 - 0.5 * Math.cos(2 * Math.PI * i / (n - 1))); c += v * Math.cos(f * i); s += v * Math.sin(f * i); }
      return c * c + s * s;
    };
    const spec = [];
    for (let bpm = PULSE_BPM[0]; bpm <= PULSE_BPM[1]; bpm++) spec.push(power(bpm));
    let peak = 0;
    for (let i = 1; i < spec.length; i++) if (spec[i] > spec[peak]) peak = i;
    const bpm = PULSE_BPM[0] + peak;
    let sig = 0, rest = 0;
    spec.forEach((p, i) => {
      const b = PULSE_BPM[0] + i;
      if (Math.abs(b - bpm) <= 6 || Math.abs(b - 2 * bpm) <= 6) sig += p; else rest += p;
    });
    return { bpm, snr: 10 * Math.log10((sig + 1e-12) / (rest + 1e-12)) };
  }

  // ------------------------------------------------------------------ voice (one microphone)
  // Fed with 16 kHz mono samples as they arrive. Keeps the last 10 s (for the voice model) and a
  // 20 ms frame analysis: loudness, whether it's voice, pitch.
  class VoiceSignals {
    constructor(rate = V_RATE) {
      this.rate = rate; this.buf = new Float32Array(rate * V_KEEP_S); this.len = 0;
      this.cur = new Float32Array(V_FRAME); this.fill = 0; this.prev = new Float32Array(V_FRAME);
      this.frames = []; this.floor = null; this.avd = null; this.avdT = 0;
    }
    push(samples, t) {
      for (let i = 0; i < samples.length; i++) {
        this.buf[this.len % this.buf.length] = samples[i]; this.len++;
        this.cur[this.fill++] = samples[i];
        if (this.fill === V_FRAME) { this._frame(t); this.prev.set(this.cur); this.fill = 0; }
      }
    }
    _frame(t) {
      let e = 0;
      for (let i = 0; i < V_FRAME; i++) e += this.cur[i] * this.cur[i];
      const db = 10 * Math.log10(e / V_FRAME + 1e-10);
      // Noise floor: follows quiet frames quickly and drifts up slowly (about 1.5 dB a second),
      // so it settles on the room's background level between words.
      if (this.floor == null) this.floor = db;
      else this.floor = db < this.floor ? this.floor + (db - this.floor) * 0.3 : this.floor + 0.03;
      const rel = db - this.floor, voiced = rel >= V_ON_DB && db > -70;
      this.frames.push({ t, db, rel, voiced, pitch: voiced ? this._pitch() : null });
      trim(this.frames, t, V_KEEP_S * 1000);
    }
    _pitch() {                                     // autocorrelation over the last 40 ms
      const x = new Float32Array(2 * V_FRAME);
      x.set(this.prev); x.set(this.cur, V_FRAME);
      const lo = Math.floor(this.rate / V_PITCH[1]), hi = Math.ceil(this.rate / V_PITCH[0]);
      let best = 0, lag = 0;
      for (let k = lo; k <= hi && k < x.length / 2; k++) {
        let c = 0, a = 0, b = 0;
        for (let i = 0; i + k < x.length; i++) { c += x[i] * x[i + k]; a += x[i] * x[i]; b += x[i + k] * x[i + k]; }
        const r = c / (Math.sqrt(a * b) + 1e-12);
        if (r > best) { best = r; lag = k; }
      }
      return best >= V_PITCH_R && lag > lo && lag < hi ? this.rate / lag : null;   // a peak at the edge is outside the range
    }
    active(now, ms = 300) {                        // talking right now?
      const f = since(this.frames, now, ms);
      return f.length > 0 && f.filter(x => x.voiced).length / f.length >= 0.5;
    }
    // { speaking: share of voiced frames, loudness: dB above noise while voiced, pitch: median Hz,
    //   pitchSd: pitch variability in semitones, rate: loudness peaks per voiced second (≈ syllables) }
    stats(now, ms = 3000) {
      const f = since(this.frames, now, ms);
      if (!f.length) return null;
      const v = f.filter(x => x.voiced), out = { speaking: v.length / f.length, loudness: null, pitch: null, pitchSd: null, rate: null };
      if (v.length) out.loudness = mean(v.map(x => x.rel));
      const p = v.map(x => x.pitch).filter(x => x).sort((a, b) => a - b);
      if (p.length >= 5) { out.pitch = p[p.length >> 1]; out.pitchSd = sd(p.map(x => 12 * Math.log2(x / 100))); }
      const voicedS = v.length * V_FRAME / this.rate;
      if (voicedS >= 0.5) {
        let peaks = 0;
        for (let i = 0; i < f.length; i++) {
          if (!f[i].voiced || f[i].rel < V_ON_DB + 3) continue;
          let top = true;
          for (let j = Math.max(0, i - 5); j <= Math.min(f.length - 1, i + 5) && top; j++) if (j !== i && f[j].db > f[i].db) top = false;
          if (top) peaks++;
        }
        out.rate = peaks / voicedS;
      }
      return out;
    }
    clip(seconds) {                                // the last `seconds` of audio, for the voice model
      const n = Math.min(this.len, Math.round(seconds * this.rate), this.buf.length), out = new Float32Array(n);
      for (let i = 0; i < n; i++) out[i] = this.buf[(this.len - n + i) % this.buf.length];
      return out;
    }
    setAvd(avd, t) { this.avd = this.avd && t - this.avdT < 5000 ? this.avd.map((v, i) => v + (avd[i] - v) * 0.5) : avd.slice(); this.avdT = t; }
    currentAvd(now) { return this.avd && now - this.avdT < 5000 ? this.avd : null; }
  }

  // Valence/arousal from a fit made by tools/expr_eval.py on your feeling reports (data/va_fit.json):
  // { features: [...], models: { "<name>" | "*": { mean, scale, v: { w, b }, a: { w, b } } } }.
  // Uses the person's own model, else the shared one ("*"). Missing features count as average.
  // Returns [valence, arousal] in -1..1, or null when there's no fit.
  function applyFit(fit, name, feats) {
    const m = fit?.models?.[name] || fit?.models?.["*"];
    if (!m || !fit.features) return null;
    const z = fit.features.map((k, i) => (feats[k] == null ? 0 : (feats[k] - m.mean[i]) / (m.scale[i] || 1)));
    const lin = side => Math.max(-1, Math.min(1, side.b + side.w.reduce((s, w, i) => s + w * z[i], 0)));
    return [lin(m.v), lin(m.a)];
  }

  // Short evidence line for one face: "talking · moving .40 · jaw moving (.052) · blinks 14/min · pulse 72 (6 dB)".
  // talking: the page's decision (jaw movement confirmed by the microphone when it's on).
  function evidence(fs, now, voice, talking) {
    const parts = talking ? ["talking"] : [];
    const m = fs.motion(now);
    if (m) parts.push(`moving ${m.speed.toFixed(2).replace(/^0/, "")}${Math.abs(m.approach) > 0.3 ? (m.approach > 0 ? " closer" : " away") : ""}`);
    const k = fs.talking(now);
    if (k) parts.push(`jaw ${k.talking ? "moving" : "still"} (${k.score.toFixed(3).replace(/^0/, "")})`);
    const b = fs.blinkRate(now);
    if (b != null) parts.push(`blinks ${Math.round(b)}/min`);
    const g = fs.gazeAway(now, 5000);
    if (g != null) parts.push(`gaze away ${Math.round(g * 100)}%`);
    const tm = fs.timing(now);
    if (tm.smileRise != null && tm.smileRise > 0.5) parts.push(`smile rise ${tm.smileRise.toFixed(1)}/s`);
    if (tm.smileAsym != null) parts.push(`smile asym ${tm.smileAsym.toFixed(2).replace(/^0/, "")}`);
    if (tm.held != null) parts.push(`held ${Math.round(tm.held)} s`);
    const p = fs.pulse(now);
    if (p) parts.push(`pulse ${p.bpm} (${Math.round(p.snr)} dB${p.snr <= 0 ? ", weak" : ""})`);
    if (voice) {
      const s = voice.stats, avd = voice.avd;
      if (s && s.speaking > 0.1) {
        const bits = [`voice ${Math.round(s.speaking * 100)}%`];
        if (s.loudness != null) bits.push(`+${Math.round(s.loudness)} dB`);
        if (s.pitch != null) bits.push(`${Math.round(s.pitch)} Hz ±${s.pitchSd.toFixed(1)} st`);
        if (s.rate != null) bits.push(`${s.rate.toFixed(1)}/s`);
        parts.push(bits.join(" "));
      }
      if (avd) parts.push(`voice A${avd[0].toFixed(2).replace(/^0/, "")} V${avd[2].toFixed(2).replace(/^0/, "")}`);
    }
    return parts.join(" · ");
  }

  const api = { FaceSignals, VoiceSignals, skinRegions, pulseFrom, applyFit, evidence, FEATURES,
                S: { TALK_JAW_SD, TALK_MS, TALK_MIN, MOTION_MS, BLINK_ON, BLINK_MIN_MS, GAZE_AWAY, SMILE_ON, SUMMARY_MS,
                     PULSE_MS, PULSE_MIN_MS, PULSE_HZ, PULSE_MAX_MOTION, V_RATE, V_FRAME, V_ON_DB } };
  if (typeof module === "object" && module.exports) module.exports = api;
  else root.Signals = api;
})(typeof window !== "undefined" ? window : globalThis);
