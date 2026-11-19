// jpeg_worker.js - makes the tracker page's JPEGs off its main thread (tracker_ui.js jpegOf): the frame sent to be
// read and the camera's picture for the other devices. The page hands over a picture (an ImageBitmap, which it no
// longer has afterwards) and gets the JPEG back; nothing is kept here, and nothing is sent anywhere.
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
if (typeof self !== "undefined" && typeof module === "undefined") {
  self.onmessage = async e => {
    const { n, bmp, q } = e.data;
    try { self.postMessage({ n, blob: await jpeg(bmp, q) }); }
    catch (err) { self.postMessage({ n, error: String(err && err.message || err).slice(0, 150) }); }
  };
}
if (typeof module !== "undefined") module.exports = { jpeg, canvasFor, canvases };
