// jpeg_worker.js - makes the tracker page's JPEGs off its main thread (tracker_ui.js jpegOf): the frame sent to be
// read and the camera's picture for the other devices. The page hands over a picture (an ImageBitmap, which it no
// longer has afterwards) and gets the JPEG back; nothing is kept here. The camera's picture for the other devices
// is also sent from here, to the page's own server at the one address the page itself sent it to, and nowhere
// else (tracker_ui.js streamByWorker): sent from the page, a frame waited four times for the page's thread.
// Why a worker: canvas.toBlob on the page waits for the page to have idle time (see jpegOf).
const canvases = new Map();                  // one canvas per picture size (two in use: the read frame, the stream's)
function canvasFor(w, h) {
  const key = `${w}x${h}`;
  let c = canvases.get(key);
  if (!c) {
    if (canvases.size >= 8) canvases.clear();                 // the camera's size changed a few times: start over
    const canvas = new OffscreenCanvas(w, h), show = canvas.getContext("bitmaprenderer");
    c = { canvas, show, draw: show ? null : canvas.getContext("2d") };
    canvases.set(key, c);
  }
  return c;
}
async function jpeg(bmp, quality) {
  const c = canvasFor(bmp.width, bmp.height);
  if (c.show) c.show.transferFromImageBitmap(bmp);            // the picture itself, not a copy of it
  else { c.draw.drawImage(bmp, 0, 0); bmp.close(); }
  return c.canvas.convertToBlob({ type: "image/jpeg", quality });
}
// One frame of the camera for the other devices: its JPEG made, and sent to the page's own server as the page
// sends it. The answer is what the page acts on: {ok}, {off} (the camera is not shared now) or {error}.
const STREAM_TO = "/api/stream/frame";
async function send(bmp, q, failed, ask = fetch) {
  const blob = await jpeg(bmp, q);
  const headers = { "Content-Type": "image/jpeg", "X-Face-Tool": "1" };
  if (failed) headers["X-Stream-Failed"] = String(failed).replace(/[^\x20-\x7e]/g, "?").slice(0, 200);  // logged
  // never wait long: a request that doesn't come back must not stop the stream
  const signal = typeof AbortSignal !== "undefined" && AbortSignal.timeout ? AbortSignal.timeout(5000) : undefined;
  const r = await ask(STREAM_TO, { method: "POST", body: blob, headers, signal });
  const j = await r.json().catch(() => ({}));
  if (j.stream === false) return { off: true };
  if (!r.ok) return { error: `HTTP ${r.status} ${j.error || ""}`.slice(0, 150) };
  return { ok: true };
}
// (one after the other, so the frames arrive in the order they were taken)
let sending = Promise.resolve();
function stream(n, bmp, q, failed, tell, ask) {
  sending = sending.then(() => send(bmp, q, failed, ask)).then(
    r => tell({ stream: n, ...r }),
    err => tell({ stream: n, error: `${err && err.name || "Error"}: ${err && err.message || err}`.slice(0, 150) }));
  return sending;
}
if (typeof self !== "undefined" && typeof module === "undefined") {
  self.onmessage = async e => {
    const { n, bmp, q } = e.data;
    if (e.data.stream) { stream(e.data.stream, bmp, q, e.data.failed, m => self.postMessage(m)); return; }
    try { self.postMessage({ n, blob: await jpeg(bmp, q) }); }
    catch (err) { self.postMessage({ n, error: String(err && err.message || err).slice(0, 150) }); }
  };
}
if (typeof module !== "undefined") module.exports = { jpeg, canvasFor, canvases, send, stream, STREAM_TO };
