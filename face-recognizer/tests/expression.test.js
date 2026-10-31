// Tests for expression.js.  Run from face-recognizer\:  node --test tests
const test = require("node:test");
const assert = require("node:assert/strict");
const X = require("../expression.js");

const LABELS = ["angry", "disgust", "fearful", "happy", "neutral", "sad", "surprised"];
const probs = (label, share) => LABELS.map(l => (l === label ? share : (1 - share) / (LABELS.length - 1)));
const memStore = () => { const m = {}; return { getItem: k => m[k] ?? null, setItem: (k, v) => { m[k] = v; }, removeItem: k => { delete m[k]; }, m }; };
const feed = (track, n, p, quality = 0.5, cues) => { for (let i = 0; i < n; i++) track.add(p, quality, null, cues); };

test("no reading without votes or without the model", () => {
  const t = new X.Track();
  assert.equal(X.read(t, LABELS, "Unknown", null), null);
  feed(t, 3, probs("happy", 0.8));
  assert.equal(X.read(t, null, "Unknown", null), null);
});

test("needs X_MIN_VOTES frames before a clear winner", () => {
  const t = new X.Track();
  feed(t, X.X.X_MIN_VOTES - 1, probs("happy", 0.8));
  let e = X.read(t, LABELS, "Unknown", null);
  assert.equal(e.state, "unsure");
  assert.deepEqual(e.flags, ["few-votes"]);
  feed(t, 1, probs("happy", 0.8));
  e = X.read(t, LABELS, "Unknown", null);
  assert.equal(e.state, "plain");
  assert.equal(e.label, "happy");
  assert.deepEqual(e.flags, []);
});

test("share and lead gates flag a close call", () => {
  const t = new X.Track();
  const close = LABELS.map(l => ({ happy: 0.4, neutral: 0.35 }[l] ?? 0.05));
  feed(t, 5, close);
  const e = X.read(t, LABELS, "Unknown", null);
  assert.equal(e.state, "unsure");
  assert.deepEqual(e.flags, ["low-share", "low-lead"]);
  assert.equal(X.text(e), "happy?");
});

test("sharp frames outweigh blurry ones", () => {
  const t = new X.Track();
  feed(t, 4, probs("sad", 0.9), 0.05);
  feed(t, 4, probs("happy", 0.9), 0.6);
  assert.equal(X.read(t, LABELS, "Unknown", null).label, "happy");
});

test("window keeps the last X_WINDOW scored frames", () => {
  const t = new X.Track();
  feed(t, 20, probs("sad", 0.9));
  feed(t, X.X.X_WINDOW, probs("happy", 0.9));
  const e = X.read(t, LABELS, "Unknown", null);
  assert.equal(e.label, "happy");
  assert.equal(e.votes, X.X.X_WINDOW);
});

test("unscored frames keep the reading, then drop it after X_STALE in a row", () => {
  const t = new X.Track();
  feed(t, 5, probs("happy", 0.9));
  for (let i = 0; i < X.X.X_STALE; i++) t.add(null, 0.1, "blurry");
  let e = X.read(t, LABELS, "Unknown", null);
  assert.equal(e.label, "happy");
  assert.ok(e.flags.includes("blurry"));
  t.add(null, 0.1, "turned");
  assert.equal(X.read(t, LABELS, "Unknown", null), null);
  assert.equal(t.skip, "turned");
});

test("usual look: learning until settled, then a matching reading is 'usual'", () => {
  const looks = new X.UsualLooks(memStore()), t = new X.Track();
  feed(t, 3, probs("sad", 0.7));
  let e = X.read(t, LABELS, "Chris", looks);
  assert.ok(e.flags.includes("learning"));
  assert.ok(e.learning > 0 && e.learning < 1);
  for (let i = 0; i < X.X.X_SETTLED; i++) { t.add(probs("sad", 0.7), 0.5); X.read(t, LABELS, "Chris", looks); }
  e = X.read(t, LABELS, "Chris", looks);
  assert.equal(e.learning, null);
  assert.equal(e.state, "usual");
  assert.equal(X.text(e), "sad · usual");
});

test("usual look: a departure is a 'change'", () => {
  const looks = new X.UsualLooks(memStore()), t = new X.Track();
  for (let i = 0; i < X.X.X_SETTLED + 5; i++) { t.add(probs("neutral", 0.7), 0.5); X.read(t, LABELS, "Chris", looks); }
  const t2 = new X.Track();
  feed(t2, 5, probs("happy", 0.8));
  const e = X.read(t2, LABELS, "Chris", looks);
  assert.equal(e.state, "change");
  assert.equal(e.label, "happy");
});

test("usual look learns each check once, not again on unscored frames", () => {
  const looks = new X.UsualLooks(memStore()), t = new X.Track();
  t.add(probs("sad", 0.7), 0.5); X.read(t, LABELS, "Chris", looks);
  for (let i = 0; i < 5; i++) { t.add(null, 0.5, "turned"); X.read(t, LABELS, "Chris", looks); }
  assert.equal(looks.looks.Chris.n, 1);
});

test("usual look ignores blurry frames and Unknown faces, and can be forgotten", () => {
  const store = memStore(), looks = new X.UsualLooks(store), t = new X.Track();
  t.add(probs("sad", 0.7), 0.2); X.read(t, LABELS, "Chris", looks);
  assert.equal(looks.looks.Chris, undefined);
  t.add(probs("sad", 0.7), 0.5); X.read(t, LABELS, "Unknown", looks);
  assert.equal(looks.looks.Unknown, undefined);
  t.add(probs("sad", 0.7), 0.5); X.read(t, LABELS, "Chris", looks); looks.save();
  assert.ok(store.m["ft-usual"]);
  assert.equal(new X.UsualLooks(store).looks.Chris.n, 1);   // survives a reload
  looks.forget();
  assert.equal(store.m["ft-usual"], undefined);
  assert.deepEqual(looks.looks, {});
});

test("valence/arousal and cues follow the votes", () => {
  const t = new X.Track();
  feed(t, 4, probs("happy", 1), 0.5, { smile: 0.8, cheek: 0.6, browDown: 0.1 });
  const e = X.read(t, LABELS, "Unknown", null);
  assert.deepEqual(e.va.map(v => +v.toFixed(2)), X.VA.happy);
  assert.deepEqual(e.cues, { smile: 0.8, cheek: 0.6 });          // only the cues that go with "happy"
  assert.match(X.evidence(e), /^4 votes · 100% · lead 100 · V\+0\.8 A\+0\.5 · smile \.80 cheek \.60$/);
});
