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
    // the answer's body is let go at once (only ok is looked at): unread, it kept a descriptor open in the page's
    // process until the window froze (see drop in tracker_ui.js)
    return fetch(path, { method: "POST", headers: { "Content-Type": "application/json", "X-Face-Tool": "1" }, body: text,
                         keepalive: text.length < 60000 })
      .then(r => { try { r.body?.cancel().catch(() => {}); } catch {} return r; }, () => {});
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
  // Each person's learned signals ("ft-usual", "ft-baselines": {name: data}). Only the people this page changed since
  // it last said are sent, and the ones it removed by name: sending the whole map dropped everyone another device
  // had learned since this page loaded. sent[key][name] is that person's data as the server has it, as far as this
  // page knows ("" for a removal that didn't arrive, so it is tried again).
  const sent = {};
  let posting = 0, posts = 0, version = null;
  const parse = text => { try { const v = JSON.parse(text || "{}"); return v && typeof v === "object" ? v : {}; } catch { return null; } };
  function sendSignals(key, text) {
    const people = parse(text);
    if (!people) return;
    const was = sent[key] || (sent[key] = {}), changed = {}, gone = [];
    for (const [name, data] of Object.entries(people)) {
      const t = JSON.stringify(data);
      if (was[name] !== t) { changed[name] = data; was[name] = t; }
    }
    for (const name of Object.keys(was)) if (!(name in people)) { gone.push(name); delete was[name]; }
    if (!Object.keys(changed).length && !gone.length) return;
    posts++; posting++;
    post("/api/signals", { key, changed, gone }).then(r => {
      posting--;
      if (r && r.ok) return;
      for (const name of Object.keys(changed)) if (was[name] === JSON.stringify(changed[name])) delete was[name];   // again next time
      for (const name of gone) if (!(name in was)) was[name] = "";
    });
  }
  // What other devices learned or forgot since the last check (the sync writes it into the server's copy): put into
  // localStorage, and each changed person handed to take(key, name, data or null) for the page's own objects.
  // Returns the server's answer ({v, same} or {v, signals, forgotten}), or null when it couldn't be asked or a
  // change of this page's was on its way (next time then).
  async function refreshSignals(take) {
    if (posting) return null;
    const at = posts;
    let r;
    try {
      r = await (await fetch("/api/signals?v=" + encodeURIComponent(version || ""), { headers: { "X-Face-Tool": "1" } })).json();
    } catch { return null; }
    if (!r || !r.v || posting || posts !== at) return null;
    if (!r.same) for (const [key, people] of Object.entries(r.signals || {})) {
      const was = sent[key] || (sent[key] = {}), mine = parse(ls.get(key)) || {};
      let any = false;
      for (const [name, data] of Object.entries(people || {})) {
        const t = JSON.stringify(data);
        if (was[name] === t) continue;
        was[name] = t; mine[name] = data; any = true;
        if (take) take(key, name, data);
      }
      for (const name of Object.keys(was)) if (!(name in (people || {}))) {
        delete was[name]; delete mine[name]; any = true;
        if (take) take(key, name, null);
      }
      if (any) ls.set(key, JSON.stringify(mine));
    }
    version = r.v;
    return r;
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
      if (people && Object.keys(people).length) {                                        // shared: the server's win
        ls.set(k, JSON.stringify(people));
        sent[k] = Object.fromEntries(Object.entries(people).map(([name, data]) => [name, JSON.stringify(data)]));
      } else { const mine = ls.get(k); if (mine && mine !== "{}") sendSignals(k, mine); }  // first time: send this browser's
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
    refresh: refreshSignals,
  };
  S.flush = flush;
  root.addEventListener?.("pagehide", flush);
  root.PageStore = S;
})(typeof window !== "undefined" ? window : globalThis);
