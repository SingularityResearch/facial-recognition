// Tests for signals.js.  Run from face-recognizer\:  node --test tests/*.test.js
const test = require("node:test");
const assert = require("node:assert/strict");
const S = require("../signals.js");

const RATE = S.S.V_RATE;
const tone = (hz, seconds, amp = 0.3) => Float32Array.from({ length: Math.round(seconds * RATE) }, (_, i) => amp * Math.sin(2 * Math.PI * hz * i / RATE));
const noise = (seconds, amp = 0.001) => { let x = 1; return Float32Array.from({ length: Math.round(seconds * RATE) }, () => { x = (x * 16807) % 2147483647; return amp * (x / 2147483647 - 0.5); }); };
const feedVoice = (v, samples, t0) => { for (let i = 0; i < samples.length; i += 1600) v.push(samples.subarray(i, i + 1600), t0 + i / RATE * 1000); };

test("motion: speed in face widths per second, approach when the face grows", () => {
  const f = new S.FaceSignals();
  for (let i = 0; i <= 10; i++) {                                // centre moves 10 px a check while the face grows
    const w = 200 * Math.exp(0.05 * i);
    f.addPosition(i * 100, [200 + i * 10 - w / 2, 100, w, 200]);
  }
  const m = f.motion(1000);
  assert.ok(m.speed > 0.35 && m.speed < 0.45, `speed ${m.speed}`);   // 100 px/s on a face growing 200 -> 330 px
  assert.ok(Math.abs(m.approach - 0.5) < 0.01, `approach ${m.approach}`);
  assert.equal(new S.FaceSignals().motion(0), null);
});

test("talking: a moving jaw is talking, a still one isn't, too few cues is no answer", () => {
  const f = new S.FaceSignals(), g = new S.FaceSignals();
  for (let i = 0; i < 12; i++) { f.addCues(i * 100, { jaw: i % 2 ? 0.3 : 0.05 }); g.addCues(i * 100, { jaw: 0.1 }); }
  assert.equal(f.talking(1100).talking, true);
  assert.equal(g.talking(1100).talking, false);
  const h = new S.FaceSignals();
  h.addCues(0, { jaw: 0.2 });
  assert.equal(h.talking(0), null);
});

test("blinks per minute after 20 s, gaze-away share", () => {
  const f = new S.FaceSignals();
  for (let i = 0; i <= 300; i++) f.addCues(i * 100, { blink: i % 30 === 0 ? 0.9 : 0.1, gaze: i % 4 === 0 ? 0.5 : 0.1 });
  assert.equal(Math.round(f.blinkRate(30000)), 20);                // a blink every 3 s
  assert.ok(Math.abs(f.gazeAway(30000) - 0.25) < 0.02);
  const g = new S.FaceSignals();
  g.addCues(0, { blink: 0.9 }); g.addCues(5000, { blink: 0.1 });
  assert.equal(g.blinkRate(5000), null);
});

test("timing: smile rise, asymmetry while smiling, how long the reading held", () => {
  const f = new S.FaceSignals();
  f.addLabel(0, "neutral"); f.addLabel(1000, "happy"); f.addLabel(1500, "happy");
  [0, 0.1, 0.5, 0.8].forEach((s, i) => f.addCues(1000 + i * 100, { smile: s, smileAsym: 0.1 }));
  const t = f.timing(1300);
  assert.ok(Math.abs(t.smileRise - 4) < 1e-9);                  // 0.1 -> 0.5 in 100 ms
  assert.ok(Math.abs(t.smileAsym - 0.1) < 1e-9);
  assert.equal(t.held, 0.3);
});

test("pulse: finds a 72 bpm colour wave in noisy skin colour", () => {
  let x = 7;
  const rnd = () => { x = (x * 16807) % 2147483647; return x / 2147483647 - 0.5; };
  const rows = [];
  for (let t = 0; t <= 10000; t += 1000 / 30) {
    const p = Math.sin(2 * Math.PI * 1.2 * t / 1000);          // 1.2 Hz = 72 bpm, strongest in green
    rows.push({ t, r: 150 + 0.2 * p + 0.3 * rnd(), g: 110 + 0.6 * p + 0.3 * rnd(), b: 90 + 0.1 * p + 0.3 * rnd() });
  }
  const p = S.pulseFrom(rows);
  assert.ok(Math.abs(p.bpm - 72) <= 2, `bpm ${p.bpm}`);
  assert.ok(p.snr > 0, `snr ${p.snr}`);
  assert.equal(S.pulseFrom(rows.slice(0, 20)), null);
  assert.equal(S.skinRegions([[100, 100], [140, 100], [120, 120], [105, 140], [135, 140]]).length, 3);
});

test("voice: silence isn't voice; a 150 Hz tone is voiced with that pitch", () => {
  const v = new S.VoiceSignals();
  feedVoice(v, noise(2), 0);
  assert.equal(v.active(2000), false);
  assert.ok(v.stats(2000).speaking < 0.05);
  feedVoice(v, tone(150, 1.5), 2000);
  assert.equal(v.active(3500), true);
  const s = v.stats(3500, 1000);
  assert.ok(s.speaking > 0.9, `speaking ${s.speaking}`);
  assert.ok(Math.abs(s.pitch - 150) < 5, `pitch ${s.pitch}`);
  assert.ok(s.pitchSd < 0.5);
  assert.ok(s.loudness > S.S.V_ON_DB);
});

test("voice: clip returns the latest audio, and avd readings smooth", () => {
  const v = new S.VoiceSignals();
  feedVoice(v, tone(200, 12), 0);                                // more than the 10 s kept
  const c = v.clip(2);
  assert.equal(c.length, 2 * RATE);
  assert.equal(v.clip(30).length, 10 * RATE);
  v.setAvd([0.2, 0.4, 0.6], 1000); v.setAvd([0.4, 0.4, 0.2], 2000);
  assert.deepEqual(v.currentAvd(2000).map(x => +x.toFixed(2)), [0.3, 0.4, 0.4]);
  assert.equal(v.currentAvd(9000), null);
});

// Body points in tracker.py's order: nose, earL, earR, shL, shR, elL, elR, wrL, wrR, ixL, ixR, hipL, hipR.
// A person facing the camera: their left shoulder appears on the image's right.
const body = ({ dy = 0, noseUp = 100, earUp = 80, shoulderY = 300, wrists = "down", tilt = 0 } = {}) => {
  const y = shoulderY + dy, v = 0.9, gone = [0, 0, 0];
  const wr = wrists === "crossed" ? [[350, y + 80, v], [450, y + 80, v]] : wrists === "down" ? [[520, y + 150, v], [280, y + 150, v]] : [gone, gone];
  return [[400, y - noseUp, v], [440, y - earUp, v], [360, y - earUp, v], [500, y + tilt, v], [300, y - tilt, v],
          [520, y + 80, v], [280, y + 80, v], wr[0], wr[1], gone, gone, [470, y + 250, v], [330, y + 250, v]];
};

test("posture: head height, neck, tilt, arms crossed", () => {
  const q = S.postureOf(body());
  assert.equal(q.sw, 200);
  assert.equal(q.head, 0.5);                                      // nose 100 px above a 200 px shoulder line
  assert.equal(q.neck, 0.4);
  assert.equal(q.tilt, 0);
  assert.equal(q.armsCrossed, false);
  assert.equal(S.postureOf(body({ wrists: "crossed" })).armsCrossed, true);
  assert.ok(Math.abs(S.postureOf(body({ tilt: 20 })).tilt - 11.3) < 0.1);
  assert.ok(S.postureOf(body({ noseUp: 40 })).head < q.head);     // head dropped
  const noShoulders = body(); noShoulders[3] = [0, 0, 0.1];
  assert.equal(S.postureOf(noShoulders), null);
});

test("posture over time: leaning in, hand on face, body movement", () => {
  const f = new S.FaceSignals();
  for (let i = 0; i < 20; i++) f.addPose(i * 100, body(), false);
  const wide = body().map(([x, y, v]) => [400 + (x - 400) * 1.3, y, v]);   // shoulders 30% wider: leaning in
  f.addPose(2000, wide, true);
  const q = f.posture(2000);
  assert.ok(q.lean > 0.08, `lean ${q.lean}`);
  assert.equal(q.hand, true);
  assert.ok(f.bodyMotion(1900) < 1e-9);
  const g = new S.FaceSignals();
  for (let i = 0; i <= 10; i++) g.addPose(i * 100, body({ dy: i * 20 }), false);   // 200 px/s = 1 shoulder width/s
  assert.ok(Math.abs(g.bodyMotion(1000) - 1) < 0.01);
  assert.match(S.bodyEvidence(f, 2000), /head \.\d+ · neck \.\d+ · tilt 0° · leaning in · .*hand on face/);
});

test("breathing: 15 breaths a minute from the shoulders' rise and fall", () => {
  const f = new S.FaceSignals();
  for (let t = 0; t <= 30000; t += 100) f.addPose(t, body({ dy: 3 * Math.sin(2 * Math.PI * 0.25 * t / 1000) + t / 3000 }), false);
  const b = f.breathing(30000);
  assert.ok(Math.abs(b.bpm - 15) <= 1, `bpm ${b.bpm}`);
  assert.ok(b.snr > 0);
  const moving = new S.FaceSignals();
  for (let t = 0; t <= 30000; t += 100) moving.addPose(t, body({ dy: (t / 100) % 2 ? 40 : 0 }), false);
  assert.equal(moving.breathing(30000), null);                    // too much movement to read breathing
});

const memStore = () => { const m = {}; return { getItem: k => m[k] ?? null, setItem: (k, v) => { m[k] = v; }, removeItem: k => { delete m[k]; }, m }; };
// n one-a-second samples of a person, head height around `head` (±0.05), speaking pitch around `pitch`
const teach = (b, name, n, { head = 0.5, pitch = 120, ctx = "alone", t0 = 0 } = {}) => {
  for (let i = 0; i < n; i++) b.learn(name, { head: head + (i % 2 ? 0.05 : -0.05), pitch: pitch + (i % 2 ? 10 : -10) }, ctx, t0 + i * 1000);
};

test("baselines: learning until trusted, then departures in spreads from the person's normal", () => {
  const b = new S.Baselines(memStore());
  teach(b, "Ann", 60);
  assert.deepEqual(b.z("Ann", { head: 0.2 }, "alone"), {});          // not trusted yet
  assert.equal(b.progress("Ann"), 0.5);
  assert.match(S.unusualEvidence(b, "Ann", { head: 0.2 }, "alone"), /learning Ann's normal 50%/);
  teach(b, "Ann", 120, { t0: 60000 });
  const z = b.z("Ann", { head: 0.3, pitch: 121 }, "alone");
  assert.ok(z.head < -3, `head z ${z.head}`);                          // 0.2 below a normal of 0.5 ± 0.05
  assert.ok(Math.abs(z.pitch) < 0.5);
  assert.match(S.unusualEvidence(b, "Ann", { head: 0.3, pitch: 121 }, "alone"), /^unusual for Ann \(alone\): head -\d\.\d SD$/);
  assert.equal(S.unusualEvidence(b, "Ann", { head: 0.5, pitch: 120 }, "alone"), "usual for Ann (alone)");
  // the Live readout's three fixed cells
  const nf = S.normalFields(b, "Ann", { head: 0.3, pitch: 121 }, "alone");
  assert.deepEqual(nf.map(([k]) => k), ["now", "most unusual", "next"]);
  assert.equal(nf[0][1], "unusual (alone)");
  assert.match(nf[1][1], /^head -\d\.\d SD$/);
  assert.equal(nf[2][1], null);
  assert.deepEqual(S.normalFields(b, "Ann", { head: 0.5, pitch: 120 }, "alone").map(([, v]) => v), ["usual (alone)", null, null]);
  assert.deepEqual(S.normalFields(b, "Unknown", {}, "alone").map(([, v]) => v), [null, null, null]);
  assert.deepEqual(S.normalFields(b, "Ann", null, "alone").map(([k]) => k), ["now", "most unusual", "next"]);
});

test("readout fields: fixed labels in a fixed order, a null value when there's no reading", () => {
  const f = new S.FaceSignals();
  const r = S.fields(f, 1000, null, false);
  assert.deepEqual(r.face.map(([k]) => k),
    ["talking", "moving", "jaw", "blinks", "gaze away", "pulse", "smile rise", "smile asym", "held"]);
  assert.equal(r.face[0][1], "no");
  assert.deepEqual(r.voice.map(([k]) => k), ["speaking", "loudness", "pitch", "pitch range", "rate", "arousal", "valence"]);
  assert.ok(r.voice.every(([, v]) => v === null));                 // no voice for this face: the same cells, all dashes
  assert.ok(r.face.slice(1).every(([, v]) => v === null));
  assert.equal(S.fields(f, 1000, null, true).face[0][2], true);    // talking is highlighted
  const body = S.bodyFields(f, 1000);
  assert.deepEqual(body.map(([k]) => k), ["head", "neck", "tilt", "lean", "movement", "breathing", "hand on face", "arms crossed"]);
  assert.ok(body.every(([, v]) => v === null));
  const v = S.fields(f, 1000, { stats: { speaking: 0.5, loudness: 12, pitch: 120, pitchSd: 1.24, rate: 3.1 }, avd: [0.4, 0.5, 0.6] }, false).voice;
  assert.deepEqual(v.map(([k, val]) => [k, val]), [["speaking", "50%"], ["loudness", "+12 dB"], ["pitch", "120 Hz"],
    ["pitch range", "±1.2 st"], ["rate", "3.1/s"], ["arousal", ".40"], ["valence", ".60"]]);
});

test("baselines: one sample a second, never for Unknown, people differ", () => {
  const b = new S.Baselines(memStore());
  assert.equal(b.learn("Ann", { head: 0.5 }, "alone", 0), true);
  assert.equal(b.learn("Ann", { head: 0.5 }, "alone", 500), false);   // too soon
  assert.equal(b.learn("Unknown", { head: 0.5 }, "alone", 0), false);
  teach(b, "Ann", 150, { head: 0.5, t0: 2000 });
  teach(b, "Bo", 150, { head: 0.3, t0: 2000 });
  const head = { head: 0.3 };
  assert.ok(b.z("Ann", head, "alone").head < -3);                    // low for Ann...
  assert.ok(Math.abs(b.z("Bo", head, "alone").head) < 0.5);           // ...normal for Mom
});

test("baselines: company has its own normal, falling back to the overall one until trusted", () => {
  const b = new S.Baselines(memStore());
  teach(b, "Ann", 150, { pitch: 120, ctx: "alone" });
  assert.equal(b.normal("Ann", "pitch", "company").scope, "all");    // no company normal yet: overall
  teach(b, "Ann", 150, { pitch: 160, ctx: "company", t0: 200000 });
  const n = b.normal("Ann", "pitch", "company");
  assert.equal(n.scope, "company");
  assert.ok(Math.abs(n.m - 160) < 1);
  assert.ok(Math.abs(b.z("Ann", { pitch: 160 }, "company").pitch) < 0.5);   // usual in company...
  assert.ok(b.z("Ann", { pitch: 160 }, "alone").pitch > 2);                // ...unusual alone
});

test("baselines: saved in the store, forgotten per person or all", () => {
  const store = memStore(), b = new S.Baselines(store);
  teach(b, "Ann", 130); teach(b, "Bo", 130);
  b.save();
  assert.deepEqual(new S.Baselines(store).names(), ["Ann", "Bo"]);
  b.forget("Bo");
  assert.deepEqual(new S.Baselines(store).names(), ["Ann"]);
  b.forget();
  assert.equal(store.m["ft-baselines"], undefined);
  assert.deepEqual(b.names(), []);
});

test("snapshot has every baseline signal", () => {
  const f = new S.FaceSignals();
  for (let i = 0; i < 12; i++) { f.addPosition(i * 100, [100, 100, 200, 200]); f.addCues(i * 100, { smile: 0.4, jaw: 0.1 }); f.addPose(i * 100, body(), false); }
  const s = f.snapshot(1100, { stats: { pitch: 150, pitchSd: 2, loudness: 15, rate: 4 }, avd: [0.6, 0.5, 0.4] });
  assert.deepEqual(Object.keys(s).sort(), Object.keys(S.B_KEYS).sort());
  assert.equal(s.head, 0.5); assert.ok(Math.abs(s.smile - 0.4) < 1e-9); assert.equal(s.pitch, 150); assert.equal(s.voiceValence, 0.4);
});

test("summary has every feature, and applyFit uses the person's model or the shared one", () => {
  const f = new S.FaceSignals();
  for (let i = 0; i < 50; i++) {
    f.addPosition(i * 100, [100, 100, 200, 200]);
    f.addCues(i * 100, { smile: 0.5, jaw: 0.1, blink: 0.1, gaze: 0.1, smileAsym: 0.05 });
    f.addExpr(i * 100, [0, 0, 0, 0.8, 0.2, 0, 0]);
  }
  const s = f.summary(4900, { stats: { speaking: 0.5, loudness: 20, pitchSd: 2, rate: 4 }, avd: [0.6, 0.5, 0.7] });
  assert.deepEqual(Object.keys(s).sort(), [...S.FEATURES].sort());
  assert.equal(s.p_happy, 0.8);
  assert.equal(s.motion, 0);
  assert.equal(s.voiceValence, 0.7);
  const fit = { features: ["p_happy", "motion"], models: {
    "*": { mean: [0.5, 0], scale: [0.1, 1], v: { w: [0.2, 0], b: 0 }, a: { w: [0, 0], b: 0.1 } },
    Ann: { mean: [0.5, 0], scale: [0.1, 1], v: { w: [-0.1, 0], b: 0 }, a: { w: [0, 0], b: 0 } } } };
  assert.deepEqual(S.applyFit(fit, "Someone", s).map(v => +v.toFixed(2)), [0.6, 0.1]);
  assert.deepEqual(S.applyFit(fit, "Ann", s).map(v => +v.toFixed(2)), [-0.3, 0]);
  assert.deepEqual(S.applyFit(fit, "Ann", { p_happy: 5 }), [-1, 0]);   // clamped to -1..1
  assert.equal(S.applyFit(null, "Ann", s), null);
});

test("baselines: progress is 0, not -Infinity, for a person with nothing learned yet", () => {
  const store = memStore();
  store.setItem("ft-baselines", JSON.stringify({ Ann: { all: {} }, Bob: {} }));
  const b = new S.Baselines(store);
  assert.equal(b.progress("Ann"), 0);
  assert.equal(b.progress("Bob"), 0);
  assert.equal(b.progress("Nobody"), 0);
  assert.equal(S.unusualEvidence(b, "Ann", {}, "alone"), "learning Ann's normal 0%");
});
