// place.js - where a device is, as the tracker page and the Cameras page show it: one line of text.
//
// A place comes from this page's own server ({lat, lon, acc, at, source, town, live}): this device's own, or
// another device's as that device published it or as the shared database's history has it. Whoever said it,
// it is checked here (clean) before anything is made of it, and it is only ever put into a page as text.
//   lat, lon   degrees north and east
//   acc        metres around that point the device may be; null for a place typed on the device
//   at         when (milliseconds since 1970): when the device said it, or (live false) when it came to be there
//   source     "system" (the device's own location service) or "hand" (typed on the device)
//   town       the nearest town in words, from the list this device's own app carries
//   live       the device itself says so now; false: the newest row of its history
// Served by the tracker (the Face tracking tab and the Cameras tab).
(function (root) {
  "use strict";
  const num = v => typeof v === "number" && Number.isFinite(v);
  const line = (v, most) => typeof v === "string" ? v.replace(/[\u0000-\u001f\u007f]/g, " ").replace(/\s+/g, " ").trim().slice(0, most) : "";

  // A place, checked: the same object with nothing in it but what a place has, or null.
  function clean(p) {
    if (!p || typeof p !== "object" || Array.isArray(p)) return null;
    if (!num(p.lat) || !num(p.lon) || Math.abs(p.lat) > 90 || Math.abs(p.lon) > 180) return null;
    if (p.source !== "system" && p.source !== "hand") return null;
    const acc = p.acc == null ? null : p.acc;
    if (acc !== null && (!num(acc) || acc < 0 || acc > 1e6)) return null;
    if (acc === null && p.source !== "hand") return null;
    if (!num(p.at) || p.at <= 0) return null;
    return { lat: p.lat, lon: p.lon, acc, at: p.at, source: p.source, town: line(p.town, 240), live: p.live !== false };
  }

  // How sure: "within 60 m", "within 6.4 km"; a typed place says that it was typed.
  function sure(p) {
    if (p.source === "hand") return "typed on the device";
    return p.acc < 1000 ? `within ${Math.max(1, Math.round(p.acc))} m` : `within ${(p.acc / 1000).toFixed(1)} km`;
  }

  // When, in the viewer's own time: how long ago the device said so, or since when its history has it there.
  function when(p, now) {
    const ago = now - p.at, day = new Date(p.at);
    const clock = day.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    const date = ago < 20 * 3600e3 ? clock : `${day.toLocaleDateString([], { month: "short", day: "numeric" })}, ${clock}`;
    if (!p.live) return `last known, since ${date}`;
    if (p.source === "hand") return "";
    if (ago < 90e3) return "just now";
    if (ago < 90 * 60e3) return `${Math.round(ago / 60e3)} min ago`;
    if (ago < 20 * 3600e3) return `${Math.round(ago / 3600e3)} h ago`;
    return `on ${date}`;
  }

  // One line: "Near Springfield, Illinois, United States · 39.80170, -89.64370 · within 60 m · 2 min ago".
  // Empty for what is no place.
  function words(place, now = Date.now()) {
    const p = clean(place);
    if (!p) return "";
    return [p.town, `${p.lat.toFixed(5)}, ${p.lon.toFixed(5)}`, sure(p), when(p, now)].filter(Boolean).join(" · ");
  }

  const api = { clean, words };
  if (typeof module === "object" && module.exports) module.exports = api;
  else root.Place = api;
})(typeof window !== "undefined" ? window : globalThis);
