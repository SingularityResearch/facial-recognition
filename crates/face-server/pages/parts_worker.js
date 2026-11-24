// parts_worker.js - MediaPipe's parts of the tracker, run beside the tracker page (parts.js loadBeside): the page's
// one thread is not held while the models run. This worker loads the page's own module (parts.js) and runs it
// here, on the pictures the page hands it: the frame as the page sent it to be read. A picture is used for the one
// run it came for and let go; nothing is kept here and nothing is sent anywhere. The runtime and the models come
// from the app's own server, as in the page.
//
// A worker of the old kind on purpose, not a module: MediaPipe's runtime loads its own script with importScripts,
// which a module worker doesn't have. The module is loaded with import().
//
// Asked: {n, load: true, module, info} -> {n, loaded: {cues, pose, hands, meshTriangles}};
// {n, run: "meshes", bmp, faces, cues, mesh} | {n, run: "poses", bmp} | {n, run: "hands", bmp}
// -> {n, out, ms, devices}; or {n, error}.
let parts = null;                             // the module's parts, once asked for (a promise)
let frame = null, fctx = null;                // the frame on a canvas whose pixels can be read (a face's region)
function canvasOf(bmp) {
  if (!frame || frame.width !== bmp.width || frame.height !== bmp.height) {
    frame = new OffscreenCanvas(bmp.width, bmp.height);
    fctx = frame.getContext("2d", { willReadFrequently: true });
  }
  fctx.drawImage(bmp, 0, 0);
  return frame;
}
async function answer(m) {
  if (m.load) {
    parts = import(m.module).then(module => module.load(m.info));
    const p = await parts;
    return { loaded: { cues: p.cues, pose: p.pose, hands: p.hands, meshTriangles: p.meshTriangles } };
  }
  if (!parts) throw new Error("asked to run before it was loaded");
  const p = await parts, t0 = performance.now();
  let out;
  if (m.run === "meshes") out = await p.run.meshes(canvasOf(m.bmp), m.faces, m.cues, m.mesh);
  else if (m.run === "poses") out = await p.run.poses(m.bmp);
  else if (m.run === "hands") out = await p.run.hands(m.bmp);
  else throw new Error("not a run");
  return { out, ms: performance.now() - t0, devices: p.devices };
}
if (typeof self !== "undefined" && typeof module === "undefined") {
  self.onmessage = async e => {
    const m = e.data;
    try { self.postMessage({ n: m.n, ...await answer(m) }); }
    catch (err) { self.postMessage({ n: m.n, error: String(err && err.message || err).slice(0, 200) }); }
    finally { try { m.bmp?.close(); } catch {} }
  };
}
if (typeof module !== "undefined") module.exports = { answer };
