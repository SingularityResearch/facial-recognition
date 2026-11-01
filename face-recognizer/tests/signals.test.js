// Tests for signals.js.  Run from face-recognizer\:  node --test tests
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
    Chris: { mean: [0.5, 0], scale: [0.1, 1], v: { w: [-0.1, 0], b: 0 }, a: { w: [0, 0], b: 0 } } } };
  assert.deepEqual(S.applyFit(fit, "Someone", s).map(v => +v.toFixed(2)), [0.6, 0.1]);
  assert.deepEqual(S.applyFit(fit, "Chris", s).map(v => +v.toFixed(2)), [-0.3, 0]);
  assert.deepEqual(S.applyFit(fit, "Chris", { p_happy: 5 }), [-1, 0]);   // clamped to -1..1
  assert.equal(S.applyFit(null, "Chris", s), null);
});
