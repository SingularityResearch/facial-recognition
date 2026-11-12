// What a page remembers, kept by its own server too (devicesettings.py), so it outlasts the browser's storage and,
// with a shared database, is kept per device there (and each person's learned signals are shared by every device).
// localStorage stays the page's quick copy: restore() runs first thing (one short synchronous request to the page's
// own server), puts the server's values into localStorage, and sends up any the server doesn't have yet (the first
// time on a device that had them in the browser only); set() keeps both. Served by tracker.py and label_tool.py.
(function (root) {
  "use strict";
  const NOT = new Set(["ft-usual", "ft-baselines", "ft-forgot-done"]);
  const S = {};
  let app = null, prefix = "", pending = {}, timer = null;
  const ls = {
    get: k => { try { return localStorage.getItem(k); } catch { return null; } },
    set: (k, v) => { try { if (v === null) localStorage.removeItem(k); else localStorage.setItem(k, v); } catch {} },
    keys: () => { try { return Object.keys(localStorage); } catch { return []; } },
  };
  function post(path, body) {
    const text = JSON.stringify(body);
    return fetch(path, { method: "POST", headers: { "Content-Type": "application/json", "X-Face-Tool": "1" }, body: text,
                         keepalive: text.length < 60000 }).catch(() => {});
  }
  function flush() {
    timer = null;
    if (!app || !Object.keys(pending).length) return;
    const settings = pending; pending = {};
    post("/api/settings", { app, settings });
  }
  function queue(changes) {
    Object.assign(pending, changes);
    if (!timer) timer = setTimeout(flush, 300);
  }
  function sendSignals(key, text) {
    let people;
    try { people = JSON.parse(text || "{}"); } catch { return; }
    post("/api/signals", { key, people });
  }
  // Restore this page's settings ("track" with "ft-" keys, "label" with "fl-"); returns the server's answer
  // ({settings, signals?, forgotten?}) or null when it couldn't be asked (the page then runs on localStorage alone).
  S.restore = function (name, keyPrefix) {
    app = name; prefix = keyPrefix;
    let state = null;
    try {
      const x = new XMLHttpRequest();
      x.open("GET", "/api/settings?app=" + encodeURIComponent(name), false);
      x.setRequestHeader("X-Face-Tool", "1");
      x.send();
      if (x.status === 200) state = JSON.parse(x.responseText);
    } catch { state = null; }
    S.state = state;
    if (!state) return null;
    const server = state.settings || {}, up = {};
    for (const [k, v] of Object.entries(server)) if (ls.get(k) !== v) ls.set(k, v);
    for (const k of ls.keys())
      if (k.startsWith(prefix) && !NOT.has(k) && !(k in server)) up[k] = ls.get(k);
    if (Object.keys(up).length) queue(up);
    for (const [k, people] of Object.entries(state.signals || {})) {
      if (people && Object.keys(people).length) ls.set(k, JSON.stringify(people));     // shared: the server's win
      else { const mine = ls.get(k); if (mine && mine !== "{}") sendSignals(k, mine); }  // first time: send this browser's
    }
    return state;
  };
  S.set = (k, v) => { ls.set(k, v); queue({ [k]: v }); };
  S.get = k => ls.get(k);
  // For Expression.UsualLooks and Signals.Baselines: their getItem/setItem/removeItem, kept on the server as well.
  S.signals = {
    getItem: k => ls.get(k),
    setItem: (k, v) => { ls.set(k, v); sendSignals(k, v); },
    removeItem: k => { ls.set(k, null); sendSignals(k, "{}"); },
  };
  S.flush = flush;
  root.addEventListener?.("pagehide", flush);
  root.PageStore = S;
})(typeof window !== "undefined" ? window : globalThis);
