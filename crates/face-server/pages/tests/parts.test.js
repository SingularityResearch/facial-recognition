// The page's MediaPipe module against fixed numbers: everything it makes of MediaPipe's output
// (cues from blendshapes, whose body, a hand on a face, whose hands, the region a face is read in, the
// rounding) must give what parts_vectors.json has for the same made-up numbers.
//
//     node --test crates/face-server/pages/tests/
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import { faceCues, handOnFace, handsForFaces, poseAll, poseForFace, posePoints, roundEven, regionOf, resizeArea, shrinkArea } from "../parts.js";

const V = JSON.parse(readFileSync(new URL("./parts_vectors.json", import.meta.url), "utf8"));

test("numbers are rounded to the nearest, a half to the even neighbour", () => {
  for (const [v, one, two] of V.round) {
    assert.equal(roundEven(v, 1), one, `round(${v}, 1)`);
    assert.equal(roundEven(v, 2), two, `round(${v}, 2)`);
  }
  assert.equal(roundEven(0.125, 2), 0.12);                 // a half goes to the even neighbour
  assert.equal(roundEven(0.375, 2), 0.38);
  assert.equal(roundEven(-0.125, 2), -0.12);
});

test("blendshapes become the cues", () => {
  assert.equal(V.cues.length, 40);
  for (const { blendshapes, cues } of V.cues) assert.deepEqual(faceCues(blendshapes), cues);
});

test("bodies and hands go to the faces they belong to", () => {
  let withBody = 0, withHand = 0, hands = 0;
  for (const s of V.people) {
    const bodies = s.boxes.map(b => poseForFace(s.poses, b));
    assert.deepEqual(bodies, s.bodies);
    s.boxes.forEach((box, k) => {
      const bi = bodies[k];
      assert.equal(bi === null ? null : handOnFace(s.poses[bi], box), s.handOnFace[k]);
      assert.deepEqual(bi === null ? null : posePoints(s.poses[bi]), s.posePoints[k]);
      assert.deepEqual(bi === null ? null : poseAll(s.poses[bi]), s.poseAll[k]);
      if (bi !== null) withBody++;
      if (s.handOnFace[k]) withHand++;
    });
    assert.deepEqual(handsForFaces(s.hands, s.boxes, s.poses, bodies), s.handsForFaces);
    hands += s.handsForFaces.flat().length;
  }
  // the made-up scenes have every case in them, not only the empty ones
  assert.ok(withBody > 20 && withHand > 5 && hands > 20, `${withBody} bodies, ${withHand} hands on faces, ${hands} hands given`);
});

test("a face is read in its fixed region", () => {
  let none = 0;
  for (const r of V.regions) {
    assert.deepEqual(regionOf(r.box, r.W, r.H), r.region, JSON.stringify(r));
    if (!r.region) none++;
  }
  assert.ok(none > 0 && none < V.regions.length);
});

test("the mesh has MediaPipe's 852 triangles", () => {
  assert.equal(V.meshTriangles.length, 852 * 3);
});

test("a picture is shrunk by averaging what each new pixel covers", () => {
  // 4 by 2 to 2 by 1: each new pixel is the mean of a 2 by 2 block
  const px = Uint8ClampedArray.from([10, 0, 0, 255, 20, 0, 0, 255, 100, 50, 0, 255, 200, 50, 0, 255,
                                     30, 0, 0, 255, 40, 0, 0, 255, 100, 150, 0, 255, 200, 150, 8, 255]);
  assert.deepEqual([...shrinkArea(px, 4, 2, 2, 1)], [25, 0, 0, 255, 150, 100, 2, 255]);
  // 3 to 2: the middle pixel is shared half and half (each new pixel: one whole old pixel and half of the middle one)
  const row = Uint8ClampedArray.from([0, 0, 0, 255, 90, 90, 90, 255, 30, 30, 30, 255]);
  assert.deepEqual([...shrinkArea(row, 3, 1, 2, 1)], [30, 30, 30, 255, 50, 50, 50, 255]);
  // a flat picture stays flat at any size
  const flat = new Uint8ClampedArray(37 * 29 * 4).fill(77);
  assert.ok(shrinkArea(flat, 37, 29, 16, 16).every((v, i) => v === (i % 4 === 3 ? 255 : 77)));
});

test("the landmarker's view of a face is OpenCV's, to within a level", () => {
  assert.equal(V.resize.length, 8);
  for (const r of V.resize) {
    // three values a pixel there, four here
    const px = new Uint8ClampedArray(r.w * r.h * 4);
    for (let i = 0; i < r.w * r.h; i++) px.set([r.px[i * 3], r.px[i * 3 + 1], r.px[i * 3 + 2], 255], i * 4);
    const got = resizeArea(px, r.w, r.h, r.dw, r.dh);
    let most = 0;
    for (let i = 0; i < r.dw * r.dh; i++) for (let c = 0; c < 3; c++) most = Math.max(most, Math.abs(got[i * 4 + c] - r.out[i * 3 + c]));
    assert.ok(most <= 1, `${r.w}x${r.h} to ${r.dw}x${r.dh}: ${most} levels from OpenCV's`);
  }
});

test("in the page, its thread gets a turn between two of the models' runs", () => {
  // Run one after the other without a pause, the models held the page's one thread for the whole of a check
  // (100 to 150 ms with every part on), and this camera's stream to the other devices waited that long. (Where
  // the parts run beside the page there is nothing to pause for.)
  const src = readFileSync(new URL("../parts.js", import.meta.url), "utf8").replace(/\r\n/g, "\n");
  const own = src.slice(src.indexOf("export async function load(info"), src.indexOf("export async function loadBeside("));
  const fill = own.slice(own.indexOf("  async function fill(res, canvas, want) {"), own.indexOf("  // the face landmarker is made now"));
  assert.ok(fill.length > 400, "fill isn't where this test reads it");
  // a pause before each model's run but the first of a check
  assert.match(fill, /const rest = async \(\) => \{ if \(ran\) await breathe\(\); ran = true; \};/);
  assert.match(fill, /const meshed = await meshes\(canvas, faces, cues, mesh, rest\);/);
  assert.match(own, /if \(asked\) await rest\(\);\s+out\.push\(asked \? await faceMesh\(canvas, f\.box\) : \[null, null\]\);/);
  assert.match(fill, /if \(posed\) await rest\(\);\s+const found = posed \? await poses\(canvas\) : \[\];/);
  assert.match(fill, /if \(fingered\) await rest\(\);\s+const handPts = fingered \? await hands\(canvas\) : \[\];/);
  // the pause is a message the page sends itself: a hidden page's timers are slowed, and the tracker's page is
  // hidden whenever another tab of the app is shown
  const breathe = own.slice(own.indexOf("  const breathe = "), own.indexOf("  async function fill("));
  assert.match(breathe, /new MessageChannel\(\)/);
  assert.doesNotMatch(breathe, /setTimeout|requestAnimationFrame/);
  // whoever asks for a model while it is being made waits for the same one (a second asker was told "none"
  // and answered a frame as one with nobody in it: found with two runs sent to a new worker at once)
  assert.match(own, /const task = \(key, kind, model, options\) => made\[key\] \|\|= make\(key, kind, model, options\);/);
});

// What the parts' workers are to the page: a stand-in that answers as parts_worker.js does, with made-up output.
const OUT = {
  meshes: m => m.faces.map(f => [{ jawOpen: 0.5, mouthSmileLeft: 0.25, mouthSmileRight: 0.25 }, [[f.box[0] + 1.26, f.box[1] + 2, 3]]]),
  poses: () => [],
  hands: () => [],
};
function standIn({ answerIn = 5, fails = null, loads = true } = {}) {
  const made = [], log = [];
  class W {
    constructor(url) { this.url = url; this.got = []; made.push(this); }
    postMessage(m, moved) {
      this.got.push(m);
      log.push(`asked ${m.load ? "load" : m.run}`);
      setTimeout(() => {
        if (this.ended) return;
        log.push(`answered ${m.load ? "load" : m.run}`);
        if (m.load) this.onmessage({ data: loads ? { n: m.n, loaded: { cues: true, pose: true, hands: true, meshTriangles: [0, 1, 2] } } : { n: m.n, error: "no graphics here" } });
        else if (fails === m.run) this.onmessage({ data: { n: m.n, error: "the model stopped" } });
        else this.onmessage({ data: { n: m.n, out: OUT[m.run](m), ms: 7, devices: { [m.run === "meshes" ? "cues" : m.run === "poses" ? "pose" : "hands"]: "CPU (MediaPipe beside this page)" } } });
      }, answerIn);
    }
    terminate() { this.ended = true; }
  }
  return { W, made, log };
}
const all = () => new URLSearchParams({ c: "1", m: "1", p: "1", b: "1", f: "1" });
const answer = () => ({ faces: [{ box: [100, 80, 60, 60], xskip: null }] });
const INFO = { base: "/mp", cues: true, pose: true, hands: true, beside: 3 };
function browser() {
  globalThis.Worker = class {}; globalThis.OffscreenCanvas = class {};
  globalThis.createImageBitmap = async canvas => ({ of: canvas, close() {} });
  return () => { delete globalThis.Worker; delete globalThis.OffscreenCanvas; delete globalThis.createImageBitmap; };
}

test("beside the page a computer has a worker for each model, and the three run at the same time", async () => {
  const { asks, combine, loadBeside, meshAsked } = await import("../parts.js");
  const done = browser();
  try {
    const s = standIn();
    const p = await loadBeside(INFO, { make: url => new s.W(url) });
    // the face landmarker's worker is made and loaded at once (that MediaPipe runs at all is known then): the
    // worker's own script, told to load this module with what the server said
    assert.deepEqual([s.made.length, p.beside, p.cues, p.pose, p.hands, p.meshTriangles], [1, 3, true, true, true, [0, 1, 2]]);
    assert.match(s.made[0].url, /\/pages\/parts_worker\.js$/);
    assert.match(s.made[0].got[0].module, /\/pages\/parts\.js$/);
    assert.deepEqual([s.made[0].got[0].load, s.made[0].got[0].info], [true, INFO]);
    // a frame with nobody in it asks nothing
    const canvas = { width: 640, height: 360 };
    assert.deepEqual(await p.fill({ faces: [] }, canvas, all()), { faces: [] });
    assert.equal(s.made.length, 1);
    // a face, everything asked for: a worker for each model, each handed a picture of its own, and all three
    // asked before any has answered
    s.log.length = 0;
    const res = await p.fill(answer(), canvas, all());
    assert.equal(s.made.length, 3);
    const runs = s.made.map(w => w.got.filter(m => m.run).map(m => m.run).join());
    assert.deepEqual(runs.sort(), ["hands", "meshes", "poses"]);
    const asked = s.log.filter(l => /^asked (meshes|poses|hands)$/.test(l)).map(l => s.log.indexOf(l));
    assert.ok(Math.max(...asked) < s.log.indexOf("answered meshes"), s.log.join("; "));
    const mesh = s.made.find(w => w.got.some(m => m.run === "meshes")).got.find(m => m.run === "meshes");
    assert.deepEqual([mesh.faces, mesh.cues, mesh.mesh, mesh.bmp.of === canvas], [[{ box: [100, 80, 60, 60], xskip: null }], true, true, true]);
    // what is made of the models' output is what the page's own run makes of it
    const want = answer();
    combine(want.faces, asks(all()), OUT.meshes({ faces: want.faces }), [], []);
    assert.deepEqual(res, want);
    assert.deepEqual([res.faces[0].mesh, res.faces[0].pose, res.faces[0].hands], [[101.3, 82, 3], null, []]);
    assert.deepEqual(p.devices, { cues: "CPU (MediaPipe beside this page)", pose: "CPU (MediaPipe beside this page)", hands: "CPU (MediaPipe beside this page)" });
    // only what is asked for is run: cues alone ask the face's worker and no other
    const before = s.made.map(w => w.got.length);
    await p.fill(answer(), canvas, new URLSearchParams({ c: "1" }));
    assert.deepEqual(s.made.map((w, k) => w.got.length - before[k]).sort(), [0, 0, 1]);
    // a face whose expression couldn't be read is not looked at for its cues alone, but is for its mesh
    assert.deepEqual([meshAsked(INFO, { xskip: "turned away" }, true, false), meshAsked(INFO, { xskip: "turned away" }, true, true), meshAsked({ cues: false }, {}, true, true)], [false, true, false]);
    p.end();
    assert.ok(s.made.every(w => w.ended));
  } finally { done(); }
});

test("the body's and the hands' runs begin with the frame, and a check doesn't ask for them twice", async () => {
  const { loadBeside } = await import("../parts.js");
  const done = browser();
  try {
    const s = standIn();
    const p = await loadBeside(INFO, { make: url => new s.W(url) });
    const canvas = { width: 640, height: 360 }, count = kind => s.made.reduce((n, w) => n + w.got.filter(m => m.run === kind).length, 0);
    // begun while the server still reads the frame for faces: the two that need no face's box, and not the mesh
    // (each of their workers is started and loaded when it is first asked for)
    const begun = p.begin(canvas, all());
    await new Promise(r => setTimeout(r, 40));
    assert.deepEqual([count("poses"), count("hands"), count("meshes")], [1, 1, 0]);
    const res = await p.fill(answer(), canvas, all(), begun);
    assert.deepEqual([count("poses"), count("hands"), count("meshes")], [1, 1, 1], "what was begun is what is used");
    assert.deepEqual(res.faces[0].mesh, [101.3, 82, 3]);
    // begun for a frame that then has no face: left to finish, dropped, and nothing is said of it
    const dropped = p.begin(canvas, all());
    assert.deepEqual(await p.fill({ faces: [] }, canvas, all(), dropped), { faces: [] });
    await dropped.found; await dropped.handPts;
    // only what was asked for is begun
    const some = p.begin(canvas, new URLSearchParams({ c: "1", f: "1" }));
    assert.deepEqual([some.found, typeof some.handPts.then], [null, "function"]);
    await some.handPts;
    p.end();
    assert.equal(p.begin(canvas, all()), null);
  } finally { done(); }
});

test("a phone has one worker for all three models", async () => {
  const { loadBeside } = await import("../parts.js");
  const done = browser();
  try {
    const s = standIn();
    const p = await loadBeside({ ...INFO, beside: 1 }, { make: url => new s.W(url) });
    await p.fill(answer(), { width: 640, height: 360 }, all());
    assert.deepEqual([s.made.length, p.beside, s.made[0].got.filter(m => m.run).map(m => m.run).sort()], [1, 1, ["hands", "meshes", "poses"]]);
    assert.equal(s.made[0].got.filter(m => m.load).length, 1);
    p.end();
  } finally { done(); }
});

test("where the workers can't, or one stops, the parts run in the page itself and a check still gets its answer", async () => {
  const { loadAny, loadBeside } = await import("../parts.js");
  const own = info => ({ cues: true, pose: true, hands: true, meshTriangles: [9], devices: { cues: "CPU (MediaPipe in this page)", pose: null, hands: null },
                         fill: async res => { for (const f of res.faces) f.from = "the page"; return res; } });
  const canvas = { width: 640, height: 360 };
  // no workers with a canvas in this browser
  await assert.rejects(loadBeside(INFO), /no workers with a canvas here/);
  let p = await loadAny(INFO, { page: own });
  assert.deepEqual([(await p.fill(answer(), canvas, all())).faces[0].from, p.meshTriangles], ["the page", [9]]);
  const done = browser();
  try {
    // the server says "in the page" (FACE_PARTS=page): no worker is made
    let s = standIn();
    p = await loadAny({ ...INFO, beside: 0 }, { make: url => new s.W(url), page: own });
    assert.deepEqual([s.made.length, (await p.fill(answer(), canvas, all())).faces[0].from], [0, "the page"]);
    // the first worker doesn't load (no graphics for MediaPipe in a worker there): the page's own run
    s = standIn({ loads: false });
    p = await loadAny(INFO, { make: url => new s.W(url), page: own });
    assert.deepEqual([(await p.fill(answer(), canvas, all())).faces[0].from, s.made.every(w => w.ended)], ["the page", true]);
    // a worker fails in the middle: that very check is answered by the page, and every one after it
    s = standIn({ fails: "hands" });
    p = await loadAny(INFO, { make: url => new s.W(url), page: own });
    assert.equal(p.beside, 3);
    const res = await p.fill(answer(), canvas, all());
    assert.deepEqual([res.faces[0].from, p.beside, s.made.every(w => w.ended), p.devices.cues], ["the page", 0, true, "CPU (MediaPipe in this page)"]);
    const asked = s.made.map(w => w.got.length);
    assert.equal((await p.fill(answer(), canvas, all())).faces[0].from, "the page");
    assert.deepEqual(s.made.map(w => w.got.length), asked, "a stopped worker is not asked again");
    assert.equal(p.begin(canvas, all()), null);
    // a worker that never answers is one that stopped
    s = standIn({ answerIn: 5 });
    const slow = await loadBeside(INFO, { make: url => new s.W(url), within: 30 });
    s.made[0].postMessage = () => {};
    await assert.rejects(slow.fill(answer(), canvas, new URLSearchParams({ c: "1" })), /no answer in 0\.03 s/);
    await assert.rejects(slow.fill(answer(), canvas, new URLSearchParams({ c: "1" })), /gave no answer/);
  } finally { done(); }
});

test("the worker runs the page's own module on the picture it is handed, and nothing else", async () => {
  const { createRequire } = await import("node:module");
  const W = createRequire(import.meta.url)("../parts_worker.js");
  const drawn = [];
  globalThis.OffscreenCanvas = class { constructor(w, h) { this.width = w; this.height = h; } getContext() { return { drawImage: b => drawn.push(b) }; } };
  const fake = "data:text/javascript," + encodeURIComponent(`export async function load(info) { return { cues: true, pose: !!info.pose, hands: false, meshTriangles: [1, 2, 3], devices: { cues: "CPU (here)", pose: null, hands: null },
    run: { meshes: async (canvas, faces, cues, mesh) => faces.map(f => [{ wide: canvas.width, cues, mesh }, [[f.box[0], 0, 0]]]), poses: async picture => [[[picture.width, picture.height, 1]]], hands: async () => [] } }; }`);
  try {
    await assert.rejects(W.answer({ run: "poses", bmp: {} }), /before it was loaded/);
    assert.deepEqual(await W.answer({ load: true, module: fake, info: { pose: true } }), { loaded: { cues: true, pose: true, hands: false, meshTriangles: [1, 2, 3] } });
    const bmp = { width: 640, height: 360 };
    // a face's region is read from a canvas of the worker's own, on which the picture is drawn
    const m = await W.answer({ run: "meshes", bmp, faces: [{ box: [5, 6, 7, 8], xskip: null }], cues: true, mesh: false });
    assert.deepEqual([m.out, drawn.length, drawn[0] === bmp, m.devices.cues], [[[{ wide: 640, cues: true, mesh: false }, [[5, 0, 0]]]], 1, true, "CPU (here)"]);
    // the body and the hands are looked for in the picture as it is
    assert.deepEqual([(await W.answer({ run: "poses", bmp })).out, (await W.answer({ run: "hands", bmp })).out, drawn.length], [[[[640, 360, 1]]], [], 1]);
    await assert.rejects(W.answer({ run: "anything else", bmp }), /not a run/);
    // nothing is kept and nothing is sent: the script names no address and has no request in it
    const src = readFileSync(new URL("../parts_worker.js", import.meta.url), "utf8");
    assert.doesNotMatch(src, /fetch\(|XMLHttpRequest|WebSocket|https?:\/\/|\/api\//);
    assert.match(src, /finally \{ try \{ m\.bmp\?\.close\(\); \} catch \{\} \}/);
  } finally { delete globalThis.OffscreenCanvas; }
});
