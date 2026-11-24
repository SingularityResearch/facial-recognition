// MediaPipe's parts of the tracker, run by the page (beside it in workers, or in it: loadAny): face-shape cues and
// the face mesh (Face Landmarker), body posture and a hand on the face (Pose Landmarker), and fingers (Hand
// Landmarker).
//
// The server has no MediaPipe. The page runs MediaPipe's own web build, a fixed version with fixed model files,
// on the frame it just sent, and puts what it found into the server's answer before the page reads it:
// `cues`, `mesh`, `pose`, `hand`, `body`, `hands`, each in a fixed shape and rounding. What is
// made of MediaPipe's output (which cue is which blendshapes, whose body and whose hand, a hand on a face) is
// fixed too: a test holds it to numbers.
//
// Loaded only where the server names it (`parts` in /api/people). Nothing here reaches the internet: the
// runtime and the models come from the app's own server.

export const POSE_VIS = 0.5;             // a body point counts as seen at this visibility
export const MAX_HANDS = 4;              // hands looked for in a frame (two people's)
export const CUES = {
  smile: ["mouthSmileLeft", "mouthSmileRight"], cheek: ["cheekSquintLeft", "cheekSquintRight"],
  frown: ["mouthFrownLeft", "mouthFrownRight"], press: ["mouthPressLeft", "mouthPressRight"],
  stretch: ["mouthStretchLeft", "mouthStretchRight"], jaw: ["jawOpen"],
  browDown: ["browDownLeft", "browDownRight"], browInnerUp: ["browInnerUp"],
  browOuterUp: ["browOuterUpLeft", "browOuterUpRight"], eyeWide: ["eyeWideLeft", "eyeWideRight"],
  eyeSquint: ["eyeSquintLeft", "eyeSquintRight"], sneer: ["noseSneerLeft", "noseSneerRight"],
  upperLip: ["mouthUpperUpLeft", "mouthUpperUpRight"], blink: ["eyeBlinkLeft", "eyeBlinkRight"],
};
// eye directions for the "gaze" cue: (left eye, right eye) blendshapes that move together
export const GAZE = [["eyeLookOutLeft", "eyeLookInRight"], ["eyeLookInLeft", "eyeLookOutRight"],
                     ["eyeLookUpLeft", "eyeLookUpRight"], ["eyeLookDownLeft", "eyeLookDownRight"]];
// body points sent with a face, in this order (MediaPipe numbering): nose, ears, shoulders, elbows, wrists,
// index fingers, hips
export const POSE_POINTS = [0, 7, 8, 11, 12, 13, 14, 15, 16, 19, 20, 23, 24];

// v rounded to that many decimals: the nearest number, a half going to the even neighbour.
export function roundEven(v, places) {
  const s = Math.abs(v).toFixed(places + 30), cut = s.length - 30;      // exact enough to see a half as a half
  let n = BigInt(s.slice(0, cut).replace(".", ""));
  const rest = s.slice(cut);
  if (rest > "5" + "0".repeat(29) || (rest === "5" + "0".repeat(29) && n % 2n === 1n)) n += 1n;
  const out = Number(n) / 10 ** places;
  return v < 0 ? -out : out;
}

// MediaPipe's 52 blendshapes ({name: score}) reduced to the cues, plus smileAsym (left/right smile difference)
// and gaze (how far both eyes look away from straight ahead, in the strongest direction), 2 decimals each.
export function faceCues(blendshapes) {
  const g = n => blendshapes[n] ?? 0.0;
  const out = {};
  for (const [cue, names] of Object.entries(CUES)) out[cue] = roundEven(names.reduce((s, n) => s + g(n), 0) / names.length, 2);
  out.smileAsym = roundEven(Math.abs(g("mouthSmileLeft") - g("mouthSmileRight")), 2);
  out.gaze = roundEven(Math.max(...GAZE.map(([a, b]) => (g(a) + g(b)) / 2)), 2);
  return out;
}

// Index of the pose (33 points of [x, y, visibility]) whose nose lies in the face box (x, y, w, h), nearest
// the box's centre, or null.
export function poseForFace(poses, box) {
  const [x, y, w, h] = box;
  let best = null, dist = null;
  poses.forEach((p, i) => {
    const [nx, ny, vis] = p[0];
    if (vis < POSE_VIS || !(x - 0.25 * w <= nx && nx <= x + 1.25 * w && y - 0.25 * h <= ny && ny <= y + 1.25 * h)) return;
    const d = Math.hypot(nx - (x + w / 2), ny - (y + h / 2));
    if (dist === null || d < dist) { best = i; dist = d; }
  });
  return best;
}

// True when a seen wrist or index fingertip is over the face (the box enlarged by 15% each side): a hand on the
// chin, cheek or mouth. null when no hand point is seen.
export function handOnFace(pose, box) {
  const [x, y, w, h] = box;
  const pts = [15, 16, 19, 20].map(i => pose[i]).filter(p => p[2] >= POSE_VIS);
  if (!pts.length) return null;
  return pts.some(([px, py]) => x - 0.15 * w <= px && px <= x + 1.15 * w && y - 0.15 * h <= py && py <= y + 1.15 * h);
}

// The POSE_POINTS of one pose, and all 33 of it, as [[x, y, visibility], ...] rounded for the page.
const point = ([x, y, v]) => [roundEven(x, 1), roundEven(y, 1), roundEven(v, 2)];
export const posePoints = pose => POSE_POINTS.map(i => point(pose[i]));
export const poseAll = pose => pose.map(point);

// Which face each hand belongs to: a list (one per face box) of that face's hands, each [[x, y] * 21] rounded.
// A hand goes to the face whose body has a seen wrist within one face width of the hand's wrist (the
// nearest), else to the nearest face whose centre is within 4 face widths; a hand near nobody is left out.
export function handsForFaces(hands, boxes, poses, bodies) {
  const out = boxes.map(() => []);
  for (const hand of hands) {
    const [wx, wy] = hand[0];
    let best = null, dist = null;
    boxes.forEach((box, k) => {
      const w = box[2], bi = bodies[k];
      if (bi === null || bi === undefined) return;
      for (const i of [15, 16]) {                 // the body's wrists
        const [px, py, vis] = poses[bi][i], d = Math.hypot(px - wx, py - wy);
        if (vis >= POSE_VIS && d <= w && (dist === null || d < dist)) { best = k; dist = d; }
      }
    });
    if (best === null) {
      boxes.forEach(([x, y, w, h], k) => {
        const d = Math.hypot(x + w / 2 - wx, y + h / 2 - wy);
        if (d <= 4 * w && (dist === null || d / w < dist)) { best = k; dist = d / w; }
      });
    }
    if (best !== null) out[best].push(hand.map(([px, py]) => [roundEven(px, 1), roundEven(py, 1)]));
  }
  return out;
}

// The part of a frame the face landmarker gets for the face in `box`: a square 1.6 times the box, so the whole
// head is in view, as far as the frame has it: [x0, y0, width, height], or null when nothing of it is in the frame.
export function regionOf(box, W, H) {
  const [x, y, w, h] = box, side = Math.max(w, h) * 1.6;
  const x0 = Math.trunc(Math.max(0, x + w / 2 - side / 2)), y0 = Math.trunc(Math.max(0, y + h / 2 - side / 2));
  const x1 = Math.min(W, Math.max(0, Math.trunc(x + w / 2 + side / 2))), y1 = Math.min(H, Math.max(0, Math.trunc(y + h / 2 + side / 2)));
  return x1 > x0 && y1 > y0 ? [x0, y0, x1 - x0, y1 - y0] : null;
}

// A picture's pixels (RGBA, `w` by `h`) shrunk to `dw` by `dh`, each new pixel the average of the old pixels it
// covers, a partly covered one counted by its share: what OpenCV's INTER_AREA does, which is how the
// face landmarker's 256 px view is to be made. A browser's own shrinking of a canvas is another filter, and
// the landmarker's cues came out further from the measured ones with it (0.006 apart on average, not 0.003).
// Written for speed: this runs for every readable face of every frame (22 ms on a phone with lists of pairs,
// before the shares were laid out flat).
export function shrinkArea(px, w, h, dw, dh) {
  // for one side: where each new pixel's old pixels start, how many they are, and their shares one after another
  const taps = (n, dn) => {
    const scale = n / dn, first = new Int32Array(dn), count = new Int32Array(dn), at = new Int32Array(dn), shares = [];
    for (let d = 0; d < dn; d++) {
      const from = d * scale, to = Math.min(n, (d + 1) * scale), i0 = Math.floor(from), i1 = Math.ceil(to - 1e-9);
      first[d] = i0; count[d] = i1 - i0; at[d] = shares.length;
      for (let i = i0; i < i1; i++) shares.push((Math.min(to, i + 1) - Math.max(from, i)) / scale);
    }
    return { first, count, at, shares: Float32Array.from(shares) };
  };
  const X = taps(w, dw), Y = taps(h, dh);
  const wide = new Float32Array(h * dw * 3);                     // every old row at the new width
  for (let y = 0; y < h; y++) {
    const row = y * w * 4;
    for (let d = 0, to = y * dw * 3; d < dw; d++, to += 3) {
      let r = 0, g = 0, b = 0;
      for (let k = 0, i = row + X.first[d] * 4, s = X.at[d]; k < X.count[d]; k++, i += 4, s++) {
        const share = X.shares[s];
        r += px[i] * share; g += px[i + 1] * share; b += px[i + 2] * share;
      }
      wide[to] = r; wide[to + 1] = g; wide[to + 2] = b;
    }
  }
  const out = new Uint8ClampedArray(dw * dh * 4), line = dw * 3;
  for (let d = 0; d < dh; d++) {
    for (let x = 0, to = d * dw * 4; x < dw; x++, to += 4) {
      let r = 0, g = 0, b = 0;
      for (let k = 0, i = Y.first[d] * line + x * 3, s = Y.at[d]; k < Y.count[d]; k++, i += line, s++) {
        const share = Y.shares[s];
        r += wide[i] * share; g += wide[i + 1] * share; b += wide[i + 2] * share;
      }
      out[to] = Math.round(r); out[to + 1] = Math.round(g); out[to + 2] = Math.round(b); out[to + 3] = 255;
    }
  }
  return out;
}

// The same pixels made larger (or larger one way and smaller the other), as OpenCV's resize does when it is
// asked for INTER_AREA and the picture is not shrunk both ways: each new pixel from its two old neighbours a
// side, at OpenCV's place for it (a new pixel takes its old one whole until the next one begins under it).
export function enlargeArea(px, w, h, dw, dh) {
  const taps = (n, dn) => {
    const scale = n / dn, inv = dn / n, i0 = new Int32Array(dn), i1 = new Int32Array(dn), f = new Float32Array(dn);
    for (let d = 0; d < dn; d++) {
      let i = Math.floor(d * scale), part = Math.fround(d + 1 - (i + 1) * inv);
      part = part <= 0 ? 0 : part - Math.floor(part);
      if (i >= n - 1) { i = n - 1; part = 0; }
      i0[d] = i; i1[d] = Math.min(i + 1, n - 1); f[d] = part;
    }
    return { i0, i1, f };
  };
  const X = taps(w, dw), Y = taps(h, dh), out = new Uint8ClampedArray(dw * dh * 4);
  for (let d = 0; d < dh; d++) {
    const top = Y.i0[d] * w * 4, low = Y.i1[d] * w * 4, fy = Y.f[d], gy = 1 - fy;
    for (let x = 0, to = d * dw * 4; x < dw; x++, to += 4) {
      const a = X.i0[x] * 4, b = X.i1[x] * 4, fx = X.f[x], gx = 1 - fx;
      for (let c = 0; c < 3; c++) {
        out[to + c] = Math.round((px[top + a + c] * gx + px[top + b + c] * fx) * gy + (px[low + a + c] * gx + px[low + b + c] * fx) * fy);
      }
      out[to + 3] = 255;
    }
  }
  return out;
}

// cv2.resize(picture, (dw, dh), interpolation=cv2.INTER_AREA), to within a level: the true average when the
// picture is shrunk both ways, OpenCV's two-neighbour way otherwise.
export const resizeArea = (px, w, h, dw, dh) => (w >= dw && h >= dh ? shrinkArea : enlargeArea)(px, w, h, dw, dh);

// ---------------------------------------------------------------------------------------------------------------
// What a check asks for, from the request's parameters (c: cues, m: mesh, p: posture, b: all 33 body points,
// f: fingers): [cues, mesh, pose, body, fingers].
export const asks = want => ["c", "m", "p", "b", "f"].map(k => want.get(k) === "1");
// One landmarker run gives a face's cues and its mesh: the cues from readable faces only, the mesh from any face.
const readable = f => f.xskip === null || f.xskip === undefined;
export const meshAsked = (info, f, cues, mesh) => !!info.cues && (mesh || (cues && readable(f)));

// The server's answer for one frame, filled with MediaPipe's parts, from what the
// three models gave: `meshes` a [blendshapes, points] pair for each face ([null, null] where none was asked for or
// found), `found` the bodies, `handPts` the hands. Wherever the models ran, this is what is made of their output.
export function combine(faces, [cues, mesh, , body, fingers], meshes, found, handPts) {
  const bodies = faces.map(f => poseForFace(found, f.box));
  const theirs = handsForFaces(handPts, faces.map(f => f.box), found, bodies);
  faces.forEach((f, k) => {
    const [bs, pts] = meshes[k] || [null, null], bi = bodies[k];
    f.cues = bs && cues && readable(f) ? faceCues(bs) : null;
    f.mesh = mesh && pts ? pts.flatMap(p => p.map(v => Math.round(v * 10) / 10)) : null;
    f.pose = bi !== null ? posePoints(found[bi]) : null;
    f.hand = bi !== null ? handOnFace(found[bi], f.box) : null;
    f.body = body && bi !== null ? poseAll(found[bi]) : null;
    f.hands = fingers ? theirs[k] : null;
  });
}

// What the parts cost, for the log: a line after the first half minute, then every five. `how` says where they
// ran; `more()` adds what is known of the stages.
function costNote(how, more = () => "") {
  const cost = { n: 0, ms: 0, most: 0, next: performance.now() + 30000 };
  return (ms, asked) => {
    cost.n++; cost.ms += ms; cost.most = Math.max(cost.most, ms);
    if (performance.now() < cost.next) return;
    const text = `MediaPipe ${how}: ${(cost.ms / cost.n).toFixed(0)} ms a frame on average, ${cost.most.toFixed(0)} at most, over ${cost.n} frames (asked for ${asked || "nothing"})${more(cost.n)}`;
    fetch("/api/page-note", { method: "POST", headers: { "Content-Type": "application/json", "X-Face-Tool": "1" },
                              body: JSON.stringify({ text }) }).then(r => r.body?.cancel(), () => {});
    Object.assign(cost, { n: 0, ms: 0, most: 0, next: performance.now() + 300000 });
  };
}

// ---------------------------------------------------------------------------------------------------------------
// MediaPipe itself. `info` is what the server says: {base, cues, pose, hands} (where the runtime and the models
// are, and which model files it has). Returns what the page uses: which parts there are, where each runs, and
// fill(res, canvas, want). It runs where it is loaded: in the page, or in a worker beside it (loadBeside), where
// there is no document and the canvases are the worker's own.
//
// On the processor ("CPU"): measured on real pictures,
// the processor gives the answers the app was measured with (cues 0.003 apart on average, body points 0.0006 face
// widths) and the graphics processor does not (body points 0.19 face widths apart, "a hand on the face" said
// differently in 3 of 9), though it is faster. "GPU" only where the server asks for it, to try it.
export async function load(info, delegate = info.delegate === "GPU" ? "GPU" : "CPU") {
  const base = new URL(info.base.replace(/\/?$/, "/"), location.href).href;
  const mp = await import(base + "vision_bundle.mjs");
  const fileset = await mp.FilesetResolver.forVisionTasks(base + "wasm");
  const beside = typeof document === "undefined";             // in a worker
  const small = beside ? new OffscreenCanvas(256, 256) : document.createElement("canvas");   // the face landmarker's 256 px view of a face
  small.width = small.height = 256;
  const sctx = small.getContext("2d", { willReadFrequently: true });
  sctx.imageSmoothingQuality = "high";

  // A landmarker made when it is first needed, on the graphics processor where that works, else on the
  // processor; null (and not tried again) when neither does. Whoever asks while it is being made waits for the
  // same one: a second asker that was told "none" meanwhile answered a frame as one with nobody in it.
  const made = {}, where = {};
  const task = (key, kind, model, options) => made[key] ||= make(key, kind, model, options);
  async function make(key, kind, model, options) {
    let lm = null;
    for (const d of delegate === "GPU" ? ["GPU", "CPU"] : ["CPU"]) {
      try {
        lm = await kind.createFromOptions(fileset, {
          baseOptions: { modelAssetPath: base + "models/" + model, delegate: d }, runningMode: "IMAGE",
          // (a worker names its own canvas for MediaPipe's graph: left to itself, the runtime decides by the
          // browser's name whether there is one, and a web view's name is not a browser's)
          ...(beside ? { canvas: new OffscreenCanvas(1, 1) } : {}), ...options });
        where[key] = `${d} (MediaPipe ${beside ? "beside" : "in"} this page)`;
        break;
      } catch (e) { console.warn(`MediaPipe ${key} on the ${d}: ${e.message || e}`); }
    }
    return lm;
  }
  const face = () => task("cues", mp.FaceLandmarker, "face_landmarker.task", { outputFaceBlendshapes: true, numFaces: 1 });
  const poser = () => task("pose", mp.PoseLandmarker, "pose_landmarker_lite.task", { numPoses: 4 });
  const hander = () => task("hands", mp.HandLandmarker, "hand_landmarker.task", { numHands: MAX_HANDS });

  // (blendshapes, points) for the face in `box`: the 52 scores by name, and the 478 landmarks as [x, y, depth]
  // in the frame's pixels; [null, null] when MediaPipe finds no face there.
  const stage = { n: 0, read: 0, size: 0, model: 0 };          // a face's view: read from the frame, sized, run
  async function faceMesh(canvas, box) {
    const lm = await face(), region = regionOf(box, canvas.width, canvas.height);
    if (!lm || !region) return [null, null];
    const [x0, y0, rw, rh] = region;
    // made 256 px a side by averaging (resizeArea), not as a browser scales a canvas
    const t0 = performance.now();
    const from = canvas.getContext("2d").getImageData(x0, y0, rw, rh).data;
    const t1 = performance.now();
    sctx.putImageData(new ImageData(resizeArea(from, rw, rh, 256, 256), 256, 256), 0, 0);
    const t2 = performance.now();
    const found = lm.detect(small);
    stage.read += t1 - t0; stage.size += t2 - t1; stage.model += performance.now() - t2; stage.n++;
    if (!found.faceBlendshapes?.length) return [null, null];
    const bs = {};
    for (const c of found.faceBlendshapes[0].categories) bs[c.categoryName] = c.score;
    return [bs, found.faceLandmarks[0].map(p => [x0 + p.x * rw, y0 + p.y * rh, p.z * rw])];
  }
  // each asked-for face's pair, in the faces' order; rest() is awaited before each run (a pause, in the page)
  async function meshes(canvas, faces, cues, mesh, rest = async () => {}) {
    const out = [];
    for (const f of faces) {
      const asked = meshAsked(info, f, cues, mesh);
      if (asked) await rest();
      out.push(asked ? await faceMesh(canvas, f.box) : [null, null]);
    }
    return out;
  }
  // (`canvas` is the frame: a canvas, or in a worker the picture it was handed)
  async function poses(canvas) {
    const lm = await poser();
    if (!lm) return [];
    const w = canvas.width, h = canvas.height;
    return lm.detect(canvas).landmarks.map(body => body.map(p => [p.x * w, p.y * h, p.visibility ?? 0.0]));
  }
  async function hands(canvas) {
    const lm = await hander();
    if (!lm) return [];
    const w = canvas.width, h = canvas.height;
    return lm.detect(canvas).landmarks.map(hand => hand.map(p => [p.x * w, p.y * h]));
  }

  // What the parts cost here, for the log: on a phone the page's own processor time is what a frame waits for.
  const per = v => (v / Math.max(1, stage.n)).toFixed(0);
  const note = costNote("in the page", () => {
    const text = `; a face: ${per(stage.read)} ms read from the frame, ${per(stage.size)} sized, ${per(stage.model)} in the model`;
    Object.assign(stage, { n: 0, read: 0, size: 0, model: 0 });
    return text;
  });

  // A turn of the page's own loop between two of the models' runs. Each run holds the page's one thread (20 to 60
  // ms on a computer); run one after the other without a pause they held it for the whole of a check, and whatever
  // the page does on a timer waited that long: with a face in view and every part on, this camera's stream to the
  // other devices fell from 20 frames a second to the rate of the checks, about 10 (measured 2026-11-23). The
  // pause is a message the page sends itself, not a timer: a hidden page's timers are slowed, a message is not.
  const breathe = () => new Promise(done => {
    const c = new MessageChannel();
    c.port1.onmessage = () => { c.port1.close(); done(); };
    c.port2.postMessage(0);
  });

  // The server's answer for one frame, filled with MediaPipe's parts. `canvas`
  // is the frame as it was sent; `want` the request's parameters.
  async function fill(res, canvas, want) {
    const faces = res.faces || [];
    if (!faces.length) return res;
    const t0 = performance.now();
    const on = asks(want), [cues, mesh, pose, body, fingers] = on;
    // (the frame is the one this check sent: nothing draws on it meanwhile, so a pause changes no answer)
    let ran = false;
    const rest = async () => { if (ran) await breathe(); ran = true; };
    const meshed = await meshes(canvas, faces, cues, mesh, rest);
    const posed = (pose || body) && info.pose, fingered = fingers && info.hands;
    if (posed) await rest();
    const found = posed ? await poses(canvas) : [];
    if (fingered) await rest();
    const handPts = fingered ? await hands(canvas) : [];
    combine(faces, on, meshed, found, handPts);
    note(performance.now() - t0, ["c", "m", "p", "b", "f"].filter((_, k) => on[k]).join(""));
    return res;
  }

  // the face landmarker is made now: that MediaPipe runs here at all is known before the page offers its parts
  const cuesWork = !!info.cues && !!(await face());
  return {
    cues: cuesWork, pose: !!info.pose, hands: !!info.hands,
    // the mesh's triangles: each triangle's three corners, from its edges' starts
    meshTriangles: cuesWork ? mp.FaceLandmarker.FACE_LANDMARKS_TESSELATION.map(c => c.start) : null,
    // where each part runs, once it has run: "GPU (MediaPipe in this page)"
    get devices() { return { cues: where.cues ?? null, pose: where.pose ?? null, hands: where.hands ?? null }; },
    fill, run: { faceMesh, meshes, poses, hands },
  };
}

// ---------------------------------------------------------------------------------------------------------------
// The same parts beside the page: in workers (parts_worker.js, which loads this module and runs `load` there), so
// that the page's one thread is not held while the models run. In the page they held it 100 to 150 ms a check
// with a face in view and every part on (also on the fastest computers: the runtime uses one thread), which is
// what a check then cost, and everything the page does on a timer waited behind it. A computer gets a worker for
// each model, and the three run at the same time: a check waits for the slowest of them, not for their sum. A
// phone gets one worker for all three (`info.beside` 1, the server's word): each worker is the whole runtime
// again in memory.
// Same runtime, same model files, same pictures (the frame as the page sent it, handed over as a picture of its
// own for each worker), so the same answers: measured against the page's own run on the same frames before this
// went in. What is made of the models' output is `combine`, here as there.
// Returns what `load` returns. Throws where there are no workers with a canvas, or the first one doesn't start;
// `fill` throws when a worker fails later (loadAny then goes back to the page's own run).
export async function loadBeside(info, { count = info.beside === 1 ? 1 : 3, script = new URL("parts_worker.js", import.meta.url).href,
                                         make = url => new Worker(url), within = 10000 } = {}) {
  if (typeof Worker === "undefined" || typeof OffscreenCanvas === "undefined" || typeof createImageBitmap === "undefined") {
    throw new Error("no workers with a canvas here");
  }
  const waiting = new Map(), workers = [], where = {}, took = { meshes: 0, poses: 0, hands: 0, n: 0 };
  let n = 0, dead = "";
  const fail = why => {
    dead = dead || why;
    for (const [, [, no]] of waiting) no(new Error(dead));
    waiting.clear();
    for (const w of workers) { try { w.terminate(); } catch {} }
  };
  const start = () => {
    const w = make(script);
    w.onmessage = e => {
      const wait = waiting.get(e.data.n);
      if (!wait) return;
      waiting.delete(e.data.n);
      if (e.data.error) wait[1](new Error(e.data.error)); else wait[0](e.data);
    };
    w.onerror = e => { try { e.preventDefault(); } catch {} fail(`a worker stopped (${e.message || "its script didn't load"})`); };
    workers.push(w);
    return w;
  };
  const ask = (w, msg, moved, ms) => new Promise((yes, no) => {
    if (dead) { no(new Error(dead)); return; }
    const id = ++n;
    waiting.set(id, [yes, no]);
    w.postMessage({ ...msg, n: id }, moved);
    const t = setTimeout(() => { if (waiting.delete(id)) { no(new Error(`no answer in ${ms / 1000} s`)); fail(`a worker gave no answer in ${ms / 1000} s`); } }, ms);
    if (t && t.unref) t.unref();
  });
  // Each model's worker: started, and the module loaded in it, when that model is first asked for (the face
  // landmarker's now, as in the page). With one worker for all, the three share it.
  const of = {};
  const worker = kind => of[count === 1 ? "all" : kind] ||= (async () => {
    const w = start();
    const loaded = (await ask(w, { load: true, module: import.meta.url, info }, [], 60000)).loaded;
    return { w, loaded };
  })();
  let first;
  try { first = await worker("meshes"); } catch (e) { fail(String(e.message || e)); throw e; }

  const note = costNote(`beside the page (${count === 1 ? "one worker" : "a worker for each model"})`, () => {
    const per = v => (v / Math.max(1, took.n)).toFixed(0);
    const text = `; of that the face ${per(took.meshes)} ms, the body ${per(took.poses)}, the hands ${per(took.hands)}`;
    Object.assign(took, { meshes: 0, poses: 0, hands: 0, n: 0 });
    return text;
  });
  const run = async (canvas, kind, more) => {
    const { w } = await worker(kind);
    const bmp = await createImageBitmap(canvas);            // the frame as it is now, a picture of its own for this worker
    const r = await ask(w, { run: kind, bmp, ...more }, [bmp], within);
    took[kind] += r.ms || 0;
    for (const [k, v] of Object.entries(r.devices || {})) if (typeof v === "string") where[k] = v.slice(0, 80);
    return r.out;
  };
  const whole = ([, , pose, body, fingers]) => [(pose || body) && !!info.pose, fingers && !!info.hands];
  // The body and the hands are looked for in the whole frame, not in a face's box: their runs can begin with the
  // frame, while the server still reads it for faces, and a check then waits for the slower of the two ways and
  // not for one after the other. The page begins them only while a face is in view (with nobody there a frame is
  // read for faces alone, many times faster than these two models run). What was begun for a frame that then
  // has no face is left to finish and dropped. Given to fill as its fourth argument.
  function begin(canvas, want) {
    if (dead) return null;
    const [posed, fingered] = whole(asks(want));
    const kept = p => { p.catch(() => {}); return p; };     // (a failure is fill's to hear, when it awaits this)
    return { found: posed ? kept(run(canvas, "poses")) : null, handPts: fingered ? kept(run(canvas, "hands")) : null };
  }
  async function fill(res, canvas, want, begun) {
    const faces = res.faces || [];
    if (!faces.length) return res;
    if (dead) throw new Error(dead);
    const t0 = performance.now();
    const on = asks(want), [cues, mesh] = on, [posed, fingered] = whole(on);
    // (only what a worker needs of a face: where it is, and whether its expression could be read)
    const boxes = faces.map(f => ({ box: f.box, xskip: f.xskip ?? null }));
    const meshed = boxes.some(f => meshAsked(info, f, cues, mesh));
    let meshes, found, handPts;
    try {
      [meshes, found, handPts] = await Promise.all([
        meshed ? run(canvas, "meshes", { faces: boxes, cues, mesh }) : faces.map(() => [null, null]),
        posed ? begun?.found || run(canvas, "poses") : [], fingered ? begun?.handPts || run(canvas, "hands") : []]);
    } catch (e) { fail(String(e.message || e)); throw e; }
    if (!Array.isArray(meshes) || !Array.isArray(found) || !Array.isArray(handPts)) { fail("a worker's answer was not one"); throw new Error(dead); }
    took.n++;
    combine(faces, on, meshes, found, handPts);
    note(performance.now() - t0, ["c", "m", "p", "b", "f"].filter((_, k) => on[k]).join(""));
    return res;
  }
  return {
    cues: !!first.loaded?.cues, pose: !!info.pose, hands: !!info.hands,
    meshTriangles: Array.isArray(first.loaded?.meshTriangles) ? first.loaded.meshTriangles : null,
    get devices() { return { cues: where.cues ?? null, pose: where.pose ?? null, hands: where.hands ?? null }; },
    fill, begin, beside: count, end: () => fail("ended"),
  };
}

// The parts beside the page where that works, in the page where it doesn't, and in the page from the moment a
// worker fails (said once in the log): a check is never left without its answer because a worker stopped.
export async function loadAny(info, options) {
  const say = text => fetch("/api/page-note", { method: "POST", headers: { "Content-Type": "application/json", "X-Face-Tool": "1" },
                                                body: JSON.stringify({ text }) }).then(r => r.body?.cancel(), () => {});
  let beside = null, own = null;
  if (info.beside !== 0 && info.beside !== false) {
    try { beside = await loadBeside(info, options); }
    catch (e) { say(`MediaPipe's parts run in the page itself: ${String(e.message || e).slice(0, 150)}`); }
  }
  const page = options?.page || load;                       // (the page's own run; a test names a stand-in)
  if (!beside) return page(info);
  say(`MediaPipe's parts run beside the page (${beside.beside === 1 ? "one worker" : "a worker for each model"})`);
  return {
    get cues() { return (own || beside).cues; }, pose: beside.pose, hands: beside.hands, meshTriangles: beside.meshTriangles,
    get devices() { return (own || beside).devices; },
    get beside() { return own ? 0 : beside.beside; },
    begin: (canvas, want) => own ? null : beside.begin(canvas, want),
    async fill(res, canvas, want, begun) {
      if (!own) {
        try { return await beside.fill(res, canvas, want, begun); }
        catch (e) {
          say(`MediaPipe's parts run in the page itself from now on: ${String(e.message || e).slice(0, 150)}`);
          own = await page(info);
        }
      }
      return own.fill(res, canvas, want);
    },
  };
}
