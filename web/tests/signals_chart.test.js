// Tests for signals_chart.js: the readings the Signals chart keeps, the events made of them, and how a device's
// readings travel to another device's chart.  node --test web/tests/*.test.js
const test = require("node:test");
const assert = require("node:assert/strict");
const C = require("../signals_chart.js");
const Signals = require("../signals.js");

const face = (name, nums = {}, usual = {}) => ({ name, nums: { pulse: 70, smile: 0.2, match: 0.8, ...nums }, usual });
const device = { checks: 20, check: 12, strict: 0.35, labels: ["angry", "happy", "neutral"] };

test("every signal a person's normal is learned from has its chart, and every charted number comes from the page", () => {
  // the green band needs a usual range: only signals.js learns them (B_KEYS)
  assert.deepEqual([...C.BASE].sort(), Object.keys(Signals.B_KEYS).sort());
  // what the page makes of one face (signals.js numbers) holds every number the person's sections draw but
  // recognition and expression, which the tracker page adds itself
  const made = Object.keys(Signals.numbers(new Signals.FaceSignals(), 0, null));
  const added = ["match", "quality", "size", "valence", "arousal", "share"];
  for (const k of [...C.SCALARS, ...C.FLAGS]) assert.ok(made.includes(k) || added.includes(k), `nothing makes ${k}`);
});

test("readings are checked value by value: numbers, yes or no, or nothing", () => {
  const s = C.clean({
    device: { checks: "20", check: NaN, strict: 0.35, labels: ["a", 2] },
    faces: [{ name: "<b>Ann</b>", nums: { pulse: 71.5, smile: "0.9", hand: 1, armsCrossed: true, probs: [0.2, "x"], evil: 1, __proto__: { jaw: 1 } },
              usual: { pulse: [70, 3], smile: [0.2, 0], motion: "x", match: [1, 1] } },
            { nums: null }, "not a face"],
    sound: { seq: 3, t: 5, interfaces: [{ name: "Mic", kind: "input", what: "speech", level: -30, tempo: Infinity }, { kind: "radio" }, 7] },
  });
  assert.deepEqual(s.device, { checks: null, check: null, strict: 0.35, labels: null, off: null,
                               charging: null, facts: null, load: null, memory: null, temp: null, heat: null, battery: null });
  const [ann, nameless, junk] = s.faces;
  assert.equal(ann.name, "<b>Ann</b>");                      // a name is text: it is only ever shown as text
  assert.equal(ann.nums.pulse, 71.5);
  assert.equal(ann.nums.smile, null);                        // a number written as text is no reading
  assert.equal(ann.nums.hand, null);
  assert.equal(ann.nums.armsCrossed, true);
  assert.equal(ann.nums.probs, null);
  assert.ok(!("evil" in ann.nums));
  assert.deepEqual(ann.usual, { pulse: [70, 3] });           // a spread of nothing, a signal without a usual range: left out
  assert.equal(nameless.name, "Unknown");
  assert.equal(junk.name, "Unknown");
  assert.deepEqual(s.sound.interfaces.map(i => [i.name, i.kind, i.v.what, i.v.level, i.v.tempo]), [["Mic", "input", C.WHAT.indexOf("speech"), -30, null]]);
  assert.equal(C.clean(null).faces.length, 0);
  assert.equal(C.clean({ sound: { interfaces: [] } }).sound, null);     // a listening without its number
});

test("a reading is kept for each person in view, with how far each signal is from their usual", () => {
  const m = new C.Model();
  assert.equal(m.sample(1000, { device, faces: [face("Ann", { pulse: 82, smile: 0.2 }, { pulse: [70, 4], smile: [0.2, 0.05] })] }), true);
  assert.equal(m.sample(1100, { device, faces: [face("Ann", { pulse: 71 }, { pulse: [70, 4] })] }), false);
  const rows = m.people.get("Ann").rows;
  assert.equal(rows.length, 2);
  assert.equal(rows[0].v.z.pulse, 3);                         // (82 - 70) / 4
  assert.equal(rows[0].v.z.smile, 0);
  assert.equal(rows[0].v.z.pitch, null);                      // no usual range for it yet
  assert.equal(rows[0].v.odd, 1);
  assert.equal(rows[1].v.odd, 0);
  assert.deepEqual(m.people.get("Ann").usual, { pulse: [70, 4] });   // their usual as it is now
  assert.deepEqual(m.device.rows.map(r => [r.v.checks, r.v.faces, r.v.known, r.v.unknown]), [[20, 1, 1, false], [20, 1, 1, false]]);
  assert.deepEqual(m.labels, ["angry", "happy", "neutral"]);
});

test("events: in view, left after ten seconds unseen, unknown faces, unusual signals that last", () => {
  const m = new C.Model();
  const odd = { pulse: 90, smile: 0.9, jaw: 0.09 }, usual = { pulse: [70, 3], smile: [0.2, 0.05], jaw: [0.03, 0.01] };
  let t = 0;
  for (; t <= 4000; t += 100) m.sample(t, { device, faces: [face("Ann", odd, usual), face("Unknown"), face("Unknown")] });
  assert.deepEqual(m.events.map(e => e.text), ["Ann in view", "unknown face"]);   // three signals beyond 2 SD, but not for long enough yet
  assert.equal(m.device.rows[0].v.faces, 3);
  assert.equal(m.people.get("Unknown").rows.length, 41);       // two unknown faces are one lane
  for (; t <= 6000; t += 100) m.sample(t, { device, faces: [face("Ann", odd, usual)] });
  assert.deepEqual(m.events.map(e => e.kind), ["arrived", "unknown", "unusual"]);
  assert.equal(m.events[2].text, "Ann: unusual signals");
  for (; t <= 9000; t += 100) m.sample(t, { device, faces: [face("Ann", odd, usual)] });
  assert.equal(m.events.filter(e => e.kind === "unusual").length, 1, "said once while it lasts");
  // nobody for more than ten seconds: Ann left, at the time she was last seen
  for (; t <= 21000; t += 1000) m.sample(t, { device: { ...device, checks: null }, faces: [] });
  const left = m.events.find(e => e.kind === "left");
  assert.deepEqual([left.text, left.t], ["Ann left", 9000]);
  assert.ok(!m.events.some(e => e.text.includes("Unknown")));
  assert.deepEqual(m.events.map(e => e.t), [...m.events.map(e => e.t)].sort((a, b) => a - b), "oldest first");
  m.sample(t, { device, faces: [face("Ann")] });
  assert.equal(m.events[m.events.length - 1].text, "Ann in view");
});

test("sound: one reading for each listening, each interface its own; music beginning is an event", () => {
  const m = new C.Model();
  const heard = (seq, what) => ({ seq, t: seq * 4000, interfaces: [{ name: "Speakers", kind: "output", what, level: -20, tempo: what === "music" ? 120 : null },
                                                                   { name: "Microphone", kind: "input", what: "quiet" }] });
  assert.equal(m.sample(4100, { device, faces: [], sound: heard(1, "quiet") }), true);
  m.sample(4200, { device, faces: [], sound: heard(1, "quiet") });          // the same listening again: not a second reading
  assert.equal(m.sample(8100, { device, faces: [], sound: heard(2, "music") }), false);
  m.sample(12100, { device, faces: [], sound: heard(3, "music") });
  const out = m.sound.get("output:Speakers");
  assert.deepEqual(out.rows.map(r => [r.t, r.v.what, r.v.tempo]), [[4000, 0, null], [8000, 1, 120], [12000, 1, 120]]);
  assert.equal(m.sound.get("input:Microphone").rows.length, 3);
  assert.deepEqual(m.events.map(e => [e.t, e.text]), [[8000, "music heard"]]);
});

test("what is kept is the last 15 minutes, and of a fast device's readings the oldest are thinned", () => {
  const m = new C.Model(60000);
  for (let t = 0; t <= 120000; t += 1000) m.sample(t, { device, faces: t < 30000 ? [face("Ann")] : [] });
  assert.equal(m.device.rows[0].t, 60000);
  assert.ok(!m.people.has("Ann"), "nobody is kept longer than their readings");
  assert.ok(m.events.every(e => e.t >= 60000));
  const rows = Array.from({ length: 1001 }, (_, i) => ({ t: i }));
  C.thin(rows, 1000);
  assert.equal(rows.length, 751);                             // every second one of the older half went
  assert.deepEqual(rows.slice(0, 3).map(r => r.t), [1, 3, 5]);
  assert.deepEqual(rows.slice(-2).map(r => r.t), [999, 1000]);
  assert.deepEqual(rows.map(r => r.t), [...rows.map(r => r.t)].sort((a, b) => a - b));
});

test("a device's readings reach another device's chart whole: packed, sent, unpacked, each with its own time", () => {
  const usual = { pulse: [70, 3] };
  const samples = [
    { t: 1000, device, faces: [face("Ann", { pulse: 71, hand: true, probs: [0.1, 0.7, 0.2] }, usual)] },
    { t: 1040, device: { ...device, check: 15 }, faces: [face("Ann", { pulse: 72 }, usual), face("Unknown", { match: null })] },
    { t: 1080, device, faces: [] },
  ];
  samples[2].device = { ...device, off: ["body", "voice"] };            // what the device has switched off goes along
  const sent = JSON.parse(JSON.stringify(C.pack(samples, 1100, { seq: 9, age: 1.5, interfaces: [] })));
  assert.deepEqual(sent.off, ["body", "voice"]);
  assert.equal(sent.rows.length, 3);
  assert.deepEqual(sent.people.map(p => p.name), ["Ann", "Unknown"]);        // a name and a usual range once, not with every reading
  const back = C.unpack(sent);
  assert.deepEqual(back.map(r => r.t), [1000, 1040, 1080]);
  const a = new C.Model(), b = new C.Model();
  for (const s of samples) a.sample(s.t, { device: s.device, faces: s.faces.map(f => ({ ...f, nums: { ...f.nums } })) });
  for (const r of back) b.sample(r.t, r.raw);
  assert.deepEqual(b.people.get("Ann").rows, a.people.get("Ann").rows);
  assert.deepEqual(b.device.rows, a.device.rows);
  assert.deepEqual(b.off, ["body", "voice"]);
  assert.deepEqual(C.clean({ device: { off: ["body", "<b>", 7, "pulse"] } }).device.off, ["body", "pulse"]);   // only the words the chart knows
  assert.deepEqual(b.people.get("Unknown").rows, a.people.get("Unknown").rows);
  // what doesn't have the shape is left out, never trusted
  assert.deepEqual(C.unpack(null), []);
  assert.deepEqual(C.unpack({ rows: [["x"], [5, 1, 1, [[7, [1]], "y"]]], keys: [{}], people: [] }).map(r => [r.t, r.raw.faces.length]), [[5, 0]]);
});

test("reading a chart at a time: the nearest reading, or a sound interface's latest before it", () => {
  const rows = [100, 200, 300, 5000].map(t => ({ t, v: { x: t } }));
  assert.equal(C.firstFrom(rows, 250), 2);
  assert.equal(C.rowNear(rows, 240).t, 200);
  assert.equal(C.rowNear(rows, 260).t, 300);
  assert.equal(C.rowNear(rows, 2500), null);                  // nothing within a second and a half: out of view then
  assert.equal(C.rowNear([], 1), null);
  assert.equal(C.rowBefore(rows, 4999).t, 300);
  assert.equal(C.rowBefore(rows, 5000).t, 5000);
  assert.equal(C.rowBefore(rows, 99), null);
  assert.equal(C.rowBefore(rows, 5000 + C.C.LISTEN_GAP_MS).t, 5000);      // a reading stands until it is too old
  assert.equal(C.rowBefore(rows, 5000 + C.C.LISTEN_GAP_MS + 1), null);
});

test("many readings on one pixel are drawn as that pixel's lowest and highest; no reading breaks the line", () => {
  const rows = [[0, 1], [1, 5], [2, 3], [10, 2], [20, null], [30, 4]].map(([t, x]) => ({ t, v: { x } }));
  const pts = C.points(rows, 0, "x", t => t / 10, r => r.v.x >= 5);
  assert.deepEqual(pts.map(p => [p.x, p.first, p.last, p.lo, p.hi, p.hot, p.gap]),
                   [[0, 1, 3, 1, 5, true, false], [1, 2, 2, 2, 2, false, false], [3, 4, 4, 4, 4, false, true]]);
});

test("a chart's range holds every reading and the usual range, and a value is shown in its own unit", () => {
  const s = { lo: 0, hi: 1 };
  assert.deepEqual(C.range(s, [0.2, 0.9]), [0, 1]);
  const [lo, hi] = C.range(s, [0.2, 1.5], [0.5, 0.4]);        // a reading above, and the usual range reaching below
  assert.ok(lo < -0.3 && hi > 1.5);
  assert.equal(C.fmt({ unit: "%", mul: 100 }, 0.456), "46%");
  assert.equal(C.fmt({ unit: "/min" }, 71.6), "72 /min");
  assert.equal(C.fmt({ d: 2 }, 0.456), "0.46");
  assert.equal(C.fmt({ kind: "flag" }, true), "yes");
  assert.equal(C.fmt({ unit: "Hz" }, null), "–");
});

test("the device itself: its own readings are charted, checked, and reach another device with what it says it is", () => {
  // the Device section draws exactly the device's own readings, and the page's sample names each of them
  const host = C.DEVICE.find(sec => sec.key === "host");
  assert.deepEqual(host.signals, [...C.HOST, "charging"]);
  const page = require("node:fs").readFileSync(require("node:path").join(__dirname, "..", "tracker_ui.js"), "utf8");
  for (const key of [...C.HOST, "charging"]) assert.match(page, new RegExp(key + ": hostNow\\." + key), `the tracker page gives the chart no ${key}`);
  // checked value by value: a share is a share, a heat state one of the four, a yes or no a yes or no
  const good = { ...device, load: 37.5, memory: 62, temp: 48.3, heat: 1, battery: 80, charging: true };
  const d = C.clean({ device: good }).device;
  assert.deepEqual([d.load, d.memory, d.temp, d.heat, d.battery, d.charging], [37.5, 62, 48.3, 1, 80, true]);
  const bad = C.clean({ device: { load: 140, memory: -3, temp: "hot", heat: 7, battery: NaN, charging: "yes" } }).device;
  assert.deepEqual([bad.load, bad.memory, bad.temp, bad.heat, bad.battery, bad.charging], [null, null, null, null, null, null]);
  assert.equal(C.clean({ device: { heat: 1.5 } }).device.heat, null);
  // what it says of itself: only the lines there are names for, as text of a line's length
  const facts = C.cleanFacts({ device: "a phone", system: "  iOS\n27.2 ", app: "<b>Face Recognizer</b> 2.4.0", camera: "x".repeat(900), evil: "y", processor: 7 });
  assert.deepEqual(Object.keys(facts), ["device", "system", "app", "camera"]);
  assert.equal(facts.system, "iOS 27.2");
  assert.equal(facts.camera.length, 240);
  assert.equal(C.cleanFacts({}), null);
  assert.equal(C.cleanFacts("a phone"), null);
  assert.ok(C.FACTS.every(key => typeof key === "string"));
  // a reading is kept with every sample, and the words as they were last said
  const m = new C.Model();
  m.sample(1000, { device: { ...good, facts: { device: "a computer", system: "Linux" } }, faces: [] });
  m.sample(2000, { device: { ...good, load: 41, temp: null }, faces: [] });
  assert.deepEqual(m.device.rows.map(r => [r.v.load, r.v.temp, r.v.heat, r.v.charging]), [[37.5, 48.3, 1, true], [41, null, 1, true]]);
  assert.deepEqual(m.facts, { device: "a computer", system: "Linux" });
  // and they travel: the newest reading once with a readout, the words with it
  const samples = [{ t: 1000, device: good, faces: [] }, { t: 1040, device: { ...good, load: 41, facts: { device: "a phone", camera: "1280×720" } }, faces: [] }];
  const sent = JSON.parse(JSON.stringify(C.pack(samples, 1100, null)));
  assert.deepEqual(sent.host, [41, 62, 48.3, 1, 80, true]);
  assert.deepEqual(sent.facts, { device: "a phone", camera: "1280×720" });
  const other = new C.Model();
  for (const r of C.unpack(sent)) other.sample(r.t, r.raw);
  assert.deepEqual(other.device.rows.map(r => [r.v.load, r.v.memory, r.v.battery, r.v.charging]), [[41, 62, 80, true], [41, 62, 80, true]]);
  assert.deepEqual(other.facts, { device: "a phone", camera: "1280×720" });
  // a device that says nothing of itself sends nothing of it, and one that sends something odd is not believed
  assert.equal(C.pack([{ t: 1, device, faces: [] }], 2, null).host, undefined);
  const odd = C.unpack({ rows: [[5, 1, 1, []]], keys: [], people: [], host: ["<b>", 999, null, 9, -1, "yes"], facts: "none" });
  const o = new C.Model();
  o.sample(odd[0].t, odd[0].raw);
  assert.deepEqual([o.device.rows[0].v.load, o.device.rows[0].v.memory, o.device.rows[0].v.heat, o.device.rows[0].v.battery, o.device.rows[0].v.charging], [null, null, null, null, null]);
  assert.equal(o.facts, null);
  // a heat state is shown as its word, a temperature in degrees
  assert.equal(C.fmt({ words: C.HEAT }, 2), "serious");
  assert.equal(C.fmt({ words: C.HEAT }, null), "–");
  assert.equal(C.fmt({ unit: "°C", d: 1 }, 48.31), "48.3 °C");
});

test("the history's rows: every reading of a stretch in one row for the device and one for each named person", () => {
  const H = new C.History(10000), T = 1_795_161_900_000;      // (a whole stretch begins here)
  const dev = { ...device, load: 40, memory: 50, heat: 1, charging: true };
  const snd = seq => ({ seq, t: T, interfaces: [{ name: "Speakers", kind: "output", what: "music", level: -20, tempo: 120 }] });
  H.add(T + 100, { device: dev, faces: [{ ...face("Ann", { pulse: 60, smile: 0.2, hand: false, probs: [0.1, 0.7, 0.2] }, { pulse: [60, 5] }), look: "happy", state: "plain" },
                                        face("Unknown")], sound: snd(1) });
  H.add(T + 4000, { device: { ...dev, load: 60 }, faces: [{ ...face("Ann", { pulse: 80, smile: 0.4, hand: true, probs: [0.3, 0.5, 0.2] }, { pulse: [60, 5] }), look: "happy" }], sound: snd(1) });
  H.add(T + 9000, { device: dev, faces: [{ ...face("Ann", { pulse: 70, smile: null, probs: [0.2, 0.6, 0.2] }, { pulse: [60, 5] }), look: "neutral" },
                                        face("Bea", { pulse: 55 })], sound: snd(2) });
  assert.deepEqual(H.take(T + 9500), [], "a stretch that is still running has no rows yet");
  H.add(T + 10200, { device: dev, faces: [] });                 // the next stretch's first reading closes it
  const rows = H.take(T + 10300);
  assert.deepEqual(rows.map(r => [r.t, r.s, r.name, r.n]), [[T, 10, "", 3], [T, 10, "Ann", 3], [T, 10, "Bea", 1]]);
  const [d, ann, bea] = rows.map(r => r.data);
  // a number: its average, lowest, highest and how many readings; a yes-or-no the same, with yes as 1
  assert.deepEqual(d.load, [46.6667, 40, 60, 3]);
  assert.deepEqual(d.charging, [1, 1, 1, 3]);
  assert.deepEqual([d.faces, d.unknown], [[1.6667, 1, 2, 3], [0.3333, 0, 1, 3]]);
  assert.deepEqual(d.heat, { fair: 3 });
  // a listening counts once however many checks it lasts
  assert.deepEqual(d.sound, [{ name: "Speakers", kind: "output", level: [-20, -20, -20, 2], tempo: [120, 120, 120, 2], what: { music: 2 } }]);
  assert.deepEqual(ann.pulse, [70, 60, 80, 3]);
  assert.deepEqual(ann.smile, [0.3, 0.2, 0.4, 2], "a reading that wasn't there is not counted");
  assert.deepEqual(ann.hand, [0.5, 0, 1, 2]);
  assert.deepEqual(ann.looks, { angry: 0.2, happy: 0.6, neutral: 0.2 });
  assert.deepEqual(ann.look, { happy: 2, neutral: 1 });
  assert.deepEqual(ann.state, { plain: 1 });
  // how far from her own usual the pulse was at its furthest (80 against 60 with a spread of 5), and that it was unusual
  assert.deepEqual([ann.z, ann.odd], [{ pulse: 4 }, 1]);
  assert.deepEqual([bea.pulse, bea.z, bea.odd], [[55, 55, 55, 1], undefined, undefined]);
  // no row names another person, and a face nobody is named for has no row of its own
  assert.ok(!JSON.stringify(ann).includes("Bea") && !JSON.stringify(d).includes("Ann"));
  assert.ok(rows.every(r => r.name !== "Unknown"));
  // every signal the chart draws for a person can be in a row, and nothing else is
  const known = new Set([...C.SCALARS, ...C.FLAGS, "looks", "look", "state", "z", "odd"]);
  assert.deepEqual(Object.keys(ann).filter(k => !known.has(k)), []);
  // the stretch that only has the device's reading is closed by the clock, and each row is given once
  assert.deepEqual(H.take(T + 21000).map(r => [r.t, r.name, r.n]), [[T + 10000, "", 1]]);
  assert.deepEqual(H.take(T + 99000), []);
  // what is no reading at all changes nothing, and a row is plain data of numbers and short words
  H.add(T + 30000, null); H.add(T + 30100, { faces: "none", device: 7, sound: "loud" });
  const odd = H.take(T + 41000);
  assert.deepEqual(odd.map(r => [r.name, r.n, Object.keys(r.data)]), [["", 2, ["faces", "unknown"]]]);
  assert.equal(C.HISTORY_MS, 10000);
});

test("no class of the chart has two methods of one name: the later one would silently be the only one", () => {
  // (it happened: a second tell() took the place of the one every channel's value is written with, and each
  // reading then threw in the page, which no test without a page sees)
  const source = require("node:fs").readFileSync(require("node:path").join(__dirname, "..", "signals_chart.js"), "utf8");
  const classes = source.split(/\r?\n(?=  class \w+ )/).slice(1);
  assert.ok(classes.length >= 2, "the chart's classes were not found");
  for (const body of classes) {
    const name = /class (\w+)/.exec(body)[1], seen = new Set();
    for (const line of body.split(/\r?\n/)) {
      const m = /^    (?:async |static |get |set )?(\w+)\(.*\) \{/.exec(line);
      if (!m || ["if", "for", "while", "switch", "catch"].includes(m[1])) continue;
      assert.ok(!seen.has(m[1]), `${name} has two methods named ${m[1]}`);
      seen.add(m[1]);
    }
    assert.ok(seen.size > 0, `no methods read in ${name}`);
  }
});

test("the catalog a node publishes names every section and signal of the chart, as plain data", () => {
  const c = C.catalog();
  assert.deepEqual(JSON.parse(JSON.stringify(c)), c, "nothing in it but data");
  assert.deepEqual(c.device.map(sec => sec.key), C.DEVICE.map(sec => sec.key));
  assert.deepEqual(c.person.map(sec => sec.key), ["recognition", "expression", "face", "voice", "body", "normal"]);
  // every signal a reading is kept of is in it, by the name its readings travel under
  const person = c.person.flatMap(sec => sec.signals.map(s => s.key));
  for (const key of [...C.SCALARS, ...C.FLAGS, "probs"]) assert.ok(person.includes(key), `the catalog has no ${key}`);
  const host = c.device.find(sec => sec.key === "host").signals;
  assert.deepEqual(host.map(s => s.key), [...C.HOST, "charging"]);
  // each with what a reader needs to show it: a label, a kind, and for a number its range
  const all = [...c.device, ...c.person].flatMap(sec => sec.signals).concat(c.sound.output, c.sound.input);
  for (const s of all) {
    assert.ok(typeof s.label === "string" && s.label, `${s.key} has no label`);
    assert.ok(["number", "word", "flag", "cat", "stack", "lanes", "heat"].includes(s.kind), `${s.key}: ${s.kind}`);
    if (s.kind === "number") assert.ok(typeof s.lo === "number" && typeof s.hi === "number" && s.lo < s.hi, `${s.key} has no range`);
  }
  assert.deepEqual(host.find(s => s.key === "heat").words, C.HEAT);
  assert.deepEqual(c.person[1].signals.find(s => s.key === "probs").words, C.LABELS);
  assert.deepEqual(c.sound.input.find(s => s.key === "what").words, C.WHAT);
  // a signal that only helps the drawing isn't published, and a usual range is said where one is learned
  assert.ok(!person.includes("odd"));
  assert.deepEqual(c.person.flatMap(sec => sec.signals).filter(s => s.usual).map(s => s.key), C.BASE);
  // why a section can be without readings: each reason names a section there is
  assert.deepEqual(Object.keys(c.off), C.OFF);
  for (const [key, why] of Object.entries(c.off)) {
    assert.ok(c.person.some(sec => sec.key === why.section), `${key} names no section`);
    assert.ok(typeof why.why === "string" && why.why);
  }
  // and the tracker page publishes it with what is on and its settings, to its own server only
  const page = require("node:fs").readFileSync(require("node:path").join(__dirname, "..", "tracker_ui.js"), "utf8");
  assert.match(page, /now\.catalog = SignalsChart\.catalog\(\)/);
  assert.match(page, /fetch\("\/api\/bus", \{ method: "POST"/);
  assert.ok(!/fetch\(["`]https?:/.test(page), "the page asks no other server");
});

test("readings are published as they are taken, and the Cameras page follows each camera's device", () => {
  const read = name => require("node:fs").readFileSync(require("node:path").join(__dirname, "..", name), "utf8");
  const page = read("tracker_ui.js"), cams = read("cameras_ui.js");
  // the tracker page: what is new since the last time, a few times a second, to its own server
  assert.match(page, /busFresh\.push\(\{ t: at, device, faces \}\)/);
  assert.match(page, /SignalsChart\.pack\(busFresh\.splice\(0\), Date\.now\(\), sound\)/);
  const every = +/BUS_READINGS_EVERY = (\d+)/.exec(page)[1];
  assert.ok(every >= 100 && every <= 500, "a few times a second");
  // the Cameras page: one waiting question a camera, to its own server, taken by the chart that keeps each once
  assert.match(cams, /\/api\/bus\/many\?nodes=\$\{nodes\}&paths=readings&wait=20/);
  assert.match(cams, /t\.panel\.take\(e\.value\)/);
  assert.equal(cams.split("fetch(`/api/bus").length, 2, "one question for every camera, not one for each");
  assert.match(page, /if \(!\$\("optStream"\)\.checked\) \{ busFresh\.length = 0; return; \}/);
  assert.ok(!/fetch\(["`]https?:/.test(cams), "the page asks no other server");
  // batches that follow each other reach a chart whole, each reading once, though a readout repeats some of them
  const samples = Array.from({ length: 12 }, (_, i) => ({ t: 1000 + i * 50, device: { ...device, checks: 20 + i }, faces: [face("Ann", { pulse: 60 + i })] }));
  const model = new C.Model(), seen = new Set();
  const take = sig => { for (const r of C.unpack(JSON.parse(JSON.stringify(sig)))) if (!seen.has(r.t)) { seen.add(r.t); model.sample(r.t, r.raw); } };
  take(C.pack(samples.slice(0, 5), 1300, null));
  take(C.pack(samples.slice(5, 10), 1550, null));
  take(C.pack(samples.slice(2, 12), 1600, null));               // (a readout's last seconds, overlapping both)
  assert.deepEqual(model.people.get("Ann").rows.map(r => r.v.pulse), samples.map((_, i) => 60 + i));
});

test("a batch that arrives late is placed at its own time, and a clock that was set is noticed", () => {
  const s = {};
  // the first batch says how far the clocks are apart; one that arrived sooner says it better
  assert.equal(C.settle(s, 5200, 1000), false);
  assert.equal(s.offset, 5200);
  assert.equal(C.settle(s, 5010, 2000), false);
  assert.equal(s.offset, 5010);
  // batches that waited on the way (a connection made again brings a minute of them at once): the clocks are
  // no further apart for that, nothing is forgotten, and each reading keeps its own time
  for (const [gap, now] of [[65010, 3000], [64760, 3001], [20000, 3002], [5400, 3003]]) assert.equal(C.settle(s, gap, now), false);
  assert.equal(s.offset, 5010);
  // a stale source beside a fresh one (a readout that stopped, the bus going on) doesn't move it either
  for (let i = 0; i < 9; i++) { assert.equal(C.settle(s, 5010 + (i + 4) * 1000, 4000 + i * 1000), false); assert.equal(C.settle(s, 5020, 4500 + i * 1000), false); }
  assert.equal(s.offset, 5010);
  // its clock was set back an hour: every batch is that late for half a minute, then the gap is taken anew and
  // the caller is told to forget what it took (those readings' times mean something else now)
  let told = 0;
  for (let now = 20000; now <= 52000; now += 1000) told += C.settle(s, 3605010, now) ? 1 : 0;
  assert.equal(told, 1);
  assert.equal(s.offset, 3605010);
  // set forward: seen at once (a smaller gap), with nothing to forget, since its readings' times only grew
  assert.equal(C.settle(s, 5010, 60000), false);
  assert.equal(s.offset, 5010);
});
