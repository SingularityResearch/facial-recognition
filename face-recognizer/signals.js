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
//   body       from MediaPipe's body points (tracker.py): head height above the shoulders (lower =
//              head down or slumped), neck (ear height above the shoulders; smaller = shoulders
//              raised, as in a shrug or tension), shoulder tilt, leaning in or back (shoulder width
//              against the last 30 s), body movement (arousal), hand on the face (it also hides the
//              face from the expression model), arms crossed, and breathing rate from the slow rise
//              and fall of the shoulders. For intense emotions the body is often more telling than
//              the face (Aviezer et al. 2012); movement tracks arousal well, valence poorly.
//
// Each named person's normal: what counts as "head low" or "fast speech" differs a lot between people
// (a soft-spoken person isn't sad, a habitual slouch isn't dejection), so Baselines learns every
// signal's usual level and spread for each person, separately alone and in company, kept in this
// browser only. Readings are then shown as departures from that person's normal ("head -2.1 SD").
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
  const PULSE_REUSE_MS = 1000;                   // a pulse estimate is reused for this long (speed only)
  const V_RATE = 16000, V_FRAME = 320, V_KEEP_S = 10;   // voice: 20 ms frames at 16 kHz, 10 s kept
  const V_ON_DB = 9;                              // a frame this much louder than the room's noise counts as voice
  const V_PITCH = [75, 400], V_PITCH_R = 0.5;     // pitch range (Hz) and the correlation needed to call it voiced
  const POSE_VIS = 0.5;                           // a body point counts as seen at this visibility (as face_id.POSE_VIS)
  const LEAN_ON = 0.08;                           // shoulder width this much (log) above/below the last 30 s = leaning in/back
  const CROSS_FRAC = 0.15;                        // a wrist this far (shoulder widths) past the body's midline counts as crossed
  const BREATH_MS = 30000, BREATH_MIN_MS = 15000, BREATH_HZ = 5, BREATH_BPM = [6, 30], BREATH_MAX_MOTION = 0.1;
  // Per-person baselines: one sample a second; trusted after 2 minutes of a person; after that a slow
  // running average (about 10 minutes' memory). A reading B_Z spreads from the person's normal is shown.
  const B_EVERY_MS = 1000, B_SETTLED = 120, B_RATE = 1 / 600, B_Z = 2, B_STORE = "ft-baselines";
  // Signals with a per-person baseline, and the smallest spread that counts (so a signal that never
  // varies doesn't make tiny changes look huge)
  const B_KEYS = { head: 0.02, neck: 0.02, tilt: 1, bodyMotion: 0.02, motion: 0.02, smile: 0.03, jaw: 0.02,
                   blinkRate: 2, gazeAway: 0.05, pitch: 5, pitchSd: 0.3, loudness: 1, speechRate: 0.3,
                   voiceArousal: 0.03, voiceValence: 0.03, pulse: 3, breathing: 1 };
  // Body points as sent by tracker.py (face_id.POSE_POINTS order)
  const P = { nose: 0, earL: 1, earR: 2, shL: 3, shR: 4, elL: 5, elR: 6, wrL: 7, wrR: 8, ixL: 9, ixR: 10, hipL: 11, hipR: 12 };

  // Features of a feeling report, in a fixed order (tools/expr_eval.py fits valence/arousal on them).
  const LABELS = ["angry", "disgust", "fearful", "happy", "neutral", "sad", "surprised"];
  const FACE_CUES = ["smile", "cheek", "frown", "browDown", "browInnerUp", "eyeWide", "jaw"];
  const FEATURES = [...LABELS.map(l => "p_" + l), ...FACE_CUES,
    "motion", "approach", "talking", "blinkRate", "gazeAway", "smileAsym",
    "voiceArousal", "voiceDominance", "voiceValence", "speaking", "loudness", "pitchSd", "speechRate", "pulse",
    "headHeight", "neck", "shoulderTilt", "bodyMotion", "handOnFace", "armsCrossed", "breathing"];

  const mean = a => a.length ? a.reduce((s, v) => s + v, 0) / a.length : null;
  const sd = a => { const m = mean(a); return a.length > 1 ? Math.sqrt(a.reduce((s, v) => s + (v - m) ** 2, 0) / (a.length - 1)) : 0; };
  const trim = (rows, now, ms) => { while (rows.length && now - rows[0].t > ms) rows.shift(); };
  const since = (rows, now, ms) => rows.filter(r => r.t <= now && now - r.t <= ms);

  // ------------------------------------------------------------------ one tracked face
  // Posture of one body (points in tracker.py's order): { sw shoulder width px, head, neck, tilt
  // degrees, armsCrossed, mid [x, y] shoulder midpoint }, or null without both shoulders.
  function postureOf(p) {
    const seen = k => p[P[k]] && p[P[k]][2] >= POSE_VIS;
    if (!seen("shL") || !seen("shR")) return null;
    const L = p[P.shL], R = p[P.shR], sw = Math.hypot(R[0] - L[0], R[1] - L[1]);
    if (!(sw > 10)) return null;
    const mid = [(L[0] + R[0]) / 2, (L[1] + R[1]) / 2];
    let tilt = Math.atan2(R[1] - L[1], R[0] - L[0]) * 180 / Math.PI;   // shoulder line against horizontal
    if (tilt > 90) tilt -= 180;
    if (tilt < -90) tilt += 180;
    const ears = ["earL", "earR"].filter(seen).map(k => p[P[k]][1]);
    // A wrist is crossed when it's well past the midline, on the other shoulder's side, below the shoulders.
    const crossed = (w, other) => seen(w) && p[P[w]][1] > mid[1] &&
      Math.sign(p[P[w]][0] - mid[0]) === Math.sign(p[P[other]][0] - mid[0]) && Math.abs(p[P[w]][0] - mid[0]) > CROSS_FRAC * sw;
    return { sw, mid, tilt: Math.abs(tilt),
             head: seen("nose") ? (mid[1] - p[P.nose][1]) / sw : null,
             neck: ears.length ? (mid[1] - mean(ears)) / sw : null,
             armsCrossed: crossed("wrL", "shR") && crossed("wrR", "shL") };
  }

  // Strongest periodic rate in an evenly sampled series (hz samples a second) between lo and hi per
  // minute, by a Hann-windowed DFT in 0.5-per-minute steps. Returns { rate per minute, snr dB: power
  // within ±band of the peak and its harmonic over the rest of the range }.
  function spectralPeak(h, hz, lo, hi, band) {
    const n = h.length, m = mean([...h]);
    const power = perMin => {
      const f = perMin / 60 / hz * 2 * Math.PI;
      let c = 0, s = 0;
      for (let i = 0; i < n; i++) { const v = (h[i] - m) * (0.5 - 0.5 * Math.cos(2 * Math.PI * i / (n - 1))); c += v * Math.cos(f * i); s += v * Math.sin(f * i); }
      return c * c + s * s;
    };
    const spec = [];
    for (let r = lo; r <= hi; r += 0.5) spec.push([r, power(r)]);
    let peak = 0;
    for (let i = 1; i < spec.length; i++) if (spec[i][1] > spec[peak][1]) peak = i;
    const rate = spec[peak][0];
    let sig = 0, rest = 0;
    for (const [r, p] of spec) { if (Math.abs(r - rate) <= band || Math.abs(r - 2 * rate) <= band) sig += p; else rest += p; }
    return { rate, snr: 10 * Math.log10((sig + 1e-12) / (rest + 1e-12)) };
  }

  // Resample rows [{ t, v }] evenly at hz; null if too short.
  function resample(rows, hz, minMs) {
    if (rows.length < 4 || rows[rows.length - 1].t - rows[0].t < minMs) return null;
    const t0 = rows[0].t, n = Math.floor((rows[rows.length - 1].t - t0) / 1000 * hz), out = new Float64Array(n);
    for (let k = 0, j = 0; k < n; k++) {
      const t = t0 + k * 1000 / hz;
      while (j < rows.length - 2 && rows[j + 1].t < t) j++;
      const a = rows[j], b = rows[j + 1], u = b.t > a.t ? Math.min(1, Math.max(0, (t - a.t) / (b.t - a.t))) : 0;
      out[k] = a.v + (b.v - a.v) * u;
    }
    return out;
  }

  class FaceSignals {
    constructor() { this.pos = []; this.cues = []; this.rgb = []; this.exprs = []; this.labels = []; this.body = []; }

    // Body points of this face (tracker.py's pose, or null) and whether a hand is on the face.
    addPose(t, pts, hand) {
      const q = pts ? postureOf(pts) : null;
      if (!q && hand == null) return;
      this.body.push({ t, q, hand, pts });
      trim(this.body, t, Math.max(SUMMARY_MS, BREATH_MS));
    }

    // { head, neck, tilt, lean (log shoulder width against the last 30 s; > 0 leaning in),
    //   armsCrossed, hand } from the latest body points, or null
    posture(now) {
      const rows = since(this.body, now, SUMMARY_MS), last = rows[rows.length - 1];
      if (!last) return null;
      const q = last.q, widths = rows.filter(r => r.q).map(r => r.q.sw);
      return { head: q?.head ?? null, neck: q?.neck ?? null, tilt: q?.tilt ?? null,
               lean: q && widths.length >= 5 ? Math.log(q.sw / mean(widths)) : null,
               armsCrossed: q ? q.armsCrossed : null, hand: last.hand ?? null };
    }

    // Body movement: shoulders and seen wrists, shoulder widths per second over the last second.
    bodyMotion(now) {
      const rows = since(this.body, now, MOTION_MS).filter(r => r.q && r.pts);
      if (rows.length < 2) return null;
      const dt = (rows[rows.length - 1].t - rows[0].t) / 1000;
      if (!(dt > 0)) return null;
      let d = 0;
      for (let i = 1; i < rows.length; i++) {
        const a = rows[i - 1].pts, b = rows[i].pts, sw = (rows[i].q.sw + rows[i - 1].q.sw) / 2, moved = [];
        for (const k of [P.shL, P.shR, P.wrL, P.wrR]) if (a[k][2] >= POSE_VIS && b[k][2] >= POSE_VIS) moved.push(Math.hypot(b[k][0] - a[k][0], b[k][1] - a[k][1]) / sw);
        if (moved.length) d += mean(moved);
      }
      return d / dt;
    }

    // Breaths per minute from the shoulders' slow rise and fall over the last 30 s (still body only).
    breathing(now) {
      const m = this.bodyMotion(now);
      if (m != null && m > BREATH_MAX_MOTION) return null;
      const rows = since(this.body, now, BREATH_MS).filter(r => r.q).map(r => ({ t: r.t, v: r.q.mid[1] / r.q.sw }));
      const h = resample(rows, BREATH_HZ, BREATH_MIN_MS);
      if (!h) return null;
      const n = h.length, xm = (n - 1) / 2, ym = mean([...h]);           // remove a straight-line drift first
      let sxy = 0, sxx = 0;
      for (let i = 0; i < n; i++) { sxy += (i - xm) * (h[i] - ym); sxx += (i - xm) ** 2; }
      const slope = sxy / (sxx || 1);
      for (let i = 0; i < n; i++) h[i] -= ym + slope * (i - xm);
      const r = spectralPeak(h, BREATH_HZ, BREATH_BPM[0], BREATH_BPM[1], 2);
      return { bpm: r.rate, snr: r.snr };
    }

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
      // The estimate takes ~2 ms and is asked for several times a frame (evidence, baselines,
      // reports), but it covers the last 10 s, so working it out once a second is plenty
      const c = this._pulse;
      if (c && now >= c.t && now - c.t < PULSE_REUSE_MS && c.n <= this.rgb.length) return c.p;
      const p = pulseFrom(this.rgb);
      this._pulse = { t: now, n: this.rgb.length, p };
      return p;
    }

    // The signals right now, for the per-person baselines (B_KEYS; null where not available).
    snapshot(now, voice) {
      const q = this.posture(now), m = this.motion(now), cues = since(this.cues, now, 1000);
      const s = voice?.stats, avd = voice?.avd, p = this.pulse(now), br = this.breathing(now);
      return {
        head: q?.head ?? null, neck: q?.neck ?? null, tilt: q?.tilt ?? null,
        bodyMotion: this.bodyMotion(now), motion: m ? m.speed : null,
        smile: cues.length ? mean(cues.map(c => c.smile ?? 0)) : null,
        jaw: cues.length ? mean(cues.map(c => c.jaw ?? 0)) : null,
        blinkRate: this.blinkRate(now), gazeAway: this.gazeAway(now, 5000),
        pitch: s?.pitch ?? null, pitchSd: s?.pitchSd ?? null, loudness: s?.loudness ?? null, speechRate: s?.rate ?? null,
        voiceArousal: avd ? avd[0] : null, voiceValence: avd ? avd[2] : null,
        pulse: p && p.snr > 0 ? p.bpm : null, breathing: br && br.snr > 0 ? br.bpm : null,
      };
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
      const body = since(this.body, now, SUMMARY_MS), qs = body.filter(r => r.q).map(r => r.q);
      f.headHeight = mean(qs.map(q => q.head).filter(v => v != null));
      f.neck = mean(qs.map(q => q.neck).filter(v => v != null));
      f.shoulderTilt = mean(qs.map(q => q.tilt));
      const moves = [];
      for (let t = now; t > now - SUMMARY_MS && body.length > 1; t -= MOTION_MS) {
        const m = this.bodyMotion(t);
        if (m != null) moves.push(m);
        if (body[0].t > t) break;
      }
      f.bodyMotion = mean(moves);
      const hands = body.filter(r => r.hand != null);
      f.handOnFace = hands.length ? hands.filter(r => r.hand).length / hands.length : null;
      f.armsCrossed = qs.length ? qs.filter(q => q.armsCrossed).length / qs.length : null;
      const br = this.breathing(now);
      f.breathing = br && br.snr > 0 ? br.bpm : null;
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

  // Body evidence line: "head .62 · neck .35 · tilt 4° · leaning in · body .12 · breathing 14/min (5 dB) · hand on face".
  function bodyEvidence(fs, now) {
    const q = fs.posture(now);
    if (!q) return "";
    const n2 = v => v.toFixed(2).replace(/^(-?)0/, "$1");
    const parts = [];
    if (q.head != null) parts.push(`head ${n2(q.head)}`);
    if (q.neck != null) parts.push(`neck ${n2(q.neck)}`);
    if (q.tilt != null) parts.push(`tilt ${Math.round(q.tilt)}°`);
    if (q.lean != null && Math.abs(q.lean) >= LEAN_ON) parts.push(q.lean > 0 ? "leaning in" : "leaning back");
    const m = fs.bodyMotion(now);
    if (m != null) parts.push(`body ${n2(m)}`);
    const b = fs.breathing(now);
    if (b) parts.push(`breathing ${b.bpm}/min (${Math.round(b.snr)} dB${b.snr <= 0 ? ", weak" : ""})`);
    if (q.hand) parts.push("hand on face");
    if (q.armsCrossed) parts.push("arms crossed");
    return parts.join(" · ");
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

  // ------------------------------------------------------------------ each person's normal
  // { "<name>": { all: {key: {n, m, v}}, alone: {...}, company: {...} } } in localStorage (B_STORE).
  // ctx: "alone" (the only face in view) or "company".
  class Baselines {
    constructor(store) {
      this.store = store; this.saved = 0; this.last = {};
      try { this.people = JSON.parse(store?.getItem(B_STORE) || "{}"); } catch { this.people = {}; }
    }
    // Learn one sample (at most one a second per person). Returns true when it learned.
    learn(name, snap, ctx, now) {
      if (!name || name === "Unknown" || now - (this.last[name] ?? -Infinity) < B_EVERY_MS) return false;
      this.last[name] = now;
      const person = this.people[name] || (this.people[name] = {});
      for (const scope of ["all", ctx]) {
        const b = person[scope] || (person[scope] = {});
        for (const k of Object.keys(B_KEYS)) {
          const x = snap[k];
          if (x == null || !isFinite(x)) continue;
          const s = b[k] || (b[k] = { n: 0, m: x, v: 0 });
          const rate = Math.max(B_RATE, 1 / (s.n + 1)), d = x - s.m;   // plain average at first, then slow
          s.m += rate * d; s.v = (1 - rate) * (s.v + rate * d * d); s.n++;
        }
      }
      if (now - this.saved > 5000) this.save(now);
      return true;
    }
    save(now = 0) { this.saved = now; try { this.store?.setItem(B_STORE, JSON.stringify(this.people)); } catch {} }
    // The person's normal for one signal: the context's own once trusted, else their overall one.
    normal(name, k, ctx) {
      const p = this.people[name];
      for (const scope of [ctx, "all"]) { const s = p?.[scope]?.[k]; if (s && s.n >= B_SETTLED) return { m: s.m, sd: Math.max(Math.sqrt(s.v), B_KEYS[k]), scope }; }
      return null;
    }
    // How far each current signal is from the person's normal, in spreads: { key: z }.
    z(name, snap, ctx) {
      const out = {};
      for (const k of Object.keys(B_KEYS)) {
        const x = snap[k], b = x == null ? null : this.normal(name, k, ctx);
        if (b) out[k] = (x - b.m) / b.sd;
      }
      return out;
    }
    progress(name) {                              // 0..1: how far the person's normal has been learned
      const all = this.people[name]?.all;
      if (!all) return 0;
      const ns = Object.values(all).map(s => s.n);
      return Math.min(1, Math.max(...ns) / B_SETTLED);
    }
    names() { return Object.keys(this.people).sort(); }
    forget(name) {
      if (name) { delete this.people[name]; delete this.last[name]; } else { this.people = {}; this.last = {}; }
      try { if (name) this.save(); else this.store?.removeItem(B_STORE); } catch {}
    }
  }

  // Evidence line of departures from a person's normal: "unusual for Chris (alone): head -2.3 SD, pitch +2.1 SD",
  // "learning Chris's normal 40%", or "" when nothing stands out.
  function unusualEvidence(baselines, name, snap, ctx) {
    if (!baselines || !name || name === "Unknown") return "";
    const z = baselines.z(name, snap, ctx), keys = Object.keys(z);
    if (!keys.length) {
      const p = baselines.progress(name);
      return p < 1 ? `learning ${name}'s normal ${Math.round(p * 100)}%` : "";
    }
    const odd = keys.filter(k => Math.abs(z[k]) >= B_Z).sort((a, b) => Math.abs(z[b]) - Math.abs(z[a]));
    const scope = keys.some(k => baselines.normal(name, k, ctx)?.scope === ctx) ? ` (${ctx})` : "";
    return odd.length ? `unusual for ${name}${scope}: ` + odd.map(k => `${k} ${z[k] > 0 ? "+" : ""}${z[k].toFixed(1)} SD`).join(", ")
                      : `usual for ${name}${scope}`;
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

  const api = { FaceSignals, VoiceSignals, Baselines, unusualEvidence, skinRegions, pulseFrom, applyFit, evidence, bodyEvidence,
                postureOf, spectralPeak, FEATURES, B_KEYS,
                S: { TALK_JAW_SD, TALK_MS, TALK_MIN, MOTION_MS, BLINK_ON, BLINK_MIN_MS, GAZE_AWAY, SMILE_ON, SUMMARY_MS,
                     PULSE_MS, PULSE_MIN_MS, PULSE_HZ, PULSE_MAX_MOTION, V_RATE, V_FRAME, V_ON_DB,
                     B_EVERY_MS, B_SETTLED, B_RATE, B_Z } };
  if (typeof module === "object" && module.exports) module.exports = api;
  else root.Signals = api;
})(typeof window !== "undefined" ? window : globalThis);
