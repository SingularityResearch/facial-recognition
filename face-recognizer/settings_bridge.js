// settings_bridge.js - lets the Settings tab of the app window (app.py) show and change this page's settings.
//
// Each page (tracker_ui.html, label_ui.html) lists its settings with SettingsBridge.init(). The Settings tab
// asks the page for them over postMessage, shows them in sections and sends changes back; the page applies
// each change through its own control, so the tab and the page's own controls always agree, and changes made
// on the page show up in the tab. Only the window around this page is answered, and only when the browser
// reports it on this computer (location.ancestorOrigins, which a page can't fake). Which pages may frame this
// one at all is set by the servers (localguard.FRAME_PARENTS: the app's tab page). Opened on its own
// (recognize.bat, label.bat) the page has no such window and this does nothing.
// It also passes the app window's own shortcuts (Ctrl+1 to Ctrl+4) on to the tab page: see the keydown listener.
//
// A setting is {section, id, label, type, hint?, def?, get(), set(v)}; type is "toggle", "range" (min, max,
// step), "select" (options [[value, text]]), "text", "info" (read-only text), "button" or "form" (fields
// [{name, placeholder, secret}], button): the last two run actions[id](args) on the page. A button with `confirm`
// (text; `confirmYes` names its button) is asked about in the Settings tab first and runs only on that yes.
const SettingsBridge = (() => {
  let app = "", items = [], actions = {}, origin = null, timer = 0;
  const local = o => /^http:\/\/(127\.0\.0\.1|localhost)(:\d+)?$/.test(o);
  const framer = () => location.ancestorOrigins?.[0];      // the real origin of the window around this page

  // A setting from one of the page's own controls; label, kind, range and default come from the control.
  function control(id, section, more = {}) {
    const el = document.getElementById(id);
    const lab = document.querySelector(`label[for="${id}"]`);
    const label = more.label || (lab ? [...lab.childNodes].filter(n => n.nodeType === 3).map(n => n.textContent).join("").trim()
                                     : el.getAttribute("aria-label") || id);
    const type = el.type === "checkbox" ? "toggle" : el.type === "range" ? "range" : el.tagName === "SELECT" ? "select" : "text";
    const it = { section, id, label, type, ...more };
    if (type === "range") Object.assign(it, { min: +el.min, max: +el.max, step: +el.step });
    if (type === "select") it.options = () => [...el.options].map(o => [o.value, o.textContent]);
    it.def = () => more.def ?? (type === "toggle" ? el.defaultChecked
      : type === "select" ? ([...el.options].find(o => o.defaultSelected) || el.options[0] || {}).value ?? "" : el.defaultValue);
    it.get = () => type === "toggle" ? el.checked : type === "range" ? +el.value : el.value;
    it.disabled = () => el.disabled;
    it.set = v => {
      if (type === "toggle") el.checked = !!v; else el.value = v;
      el.dispatchEvent(new Event("input", { bubbles: true }));
      el.dispatchEvent(new Event("change", { bubbles: true }));
    };
    return it;
  }

  async function describe() {
    const out = [];
    for (const it of items) {
      const d = {};
      for (const [k, v] of Object.entries(it))
        if (!["get", "set", "_err"].includes(k)) d[k] = typeof v === "function" ? v() : v;
      try { if (it.get) d.value = await it.get(); } catch (e) { d.value = null; d.error = e.message; }
      if (it._err) d.error = it._err;                 // the last change didn't work: why
      out.push(d);
    }
    return out;
  }
  async function send() {
    if (origin) parent.postMessage({ fs: "state", app, items: await describe() }, origin);
  }
  // something changed on this page (its own controls, or a status): tell the tab, at most every 200 ms
  function changed() { clearTimeout(timer); timer = setTimeout(send, 200); }

  addEventListener("message", async e => {
    if (e.source !== parent || parent === window || !local(e.origin)) return;
    if (framer() !== undefined && framer() !== e.origin) return;
    const m = e.data;
    if (!m || typeof m.fs !== "string") return;
    // "Does this page run?" (the tab page, every 2 s): each tab's page has a process of its own, so one that hangs
    // no longer stops the tab page, whose /alive is what the app's watchdog goes by. Answered at once, nothing else.
    if (m.fs === "ping") { parent.postMessage({ fs: "pong", app }, e.origin); return; }
    origin = e.origin;
    const it = items.find(x => x.id === m.id);
    if (m.fs === "set" && it?.set && !(it.disabled?.())) {
      try { await it.set(m.value); it._err = null; } catch (err) { it._err = err.message || String(err); }
    }
    if (m.fs === "act" && actions[m.id]) {
      let reply;
      try { reply = { ok: true, value: await actions[m.id](m.args || {}) }; }
      catch (err) { reply = { ok: false, error: err.message || String(err) }; }
      parent.postMessage({ fs: "result", app, rid: m.rid, ...reply }, origin);
    }
    send();
  });

  // Tell the app window's tab page something this page did (e.g. a person forgotten, so the tracker tab can forget
  // what its page learned about them). Only to the window around this page, on this computer; else nothing.
  function notify(msg) {
    const o = origin || framer();
    if (parent !== window && o && local(o)) parent.postMessage({ ...msg, app }, o);
  }

  // Ctrl+1 to Ctrl+4 (the app window's tabs and its Alerts) pressed while this page has the keyboard: the tab page
  // never sees a key pressed inside one of its tabs, so the page passes it on. Every page in a tab loads this file
  // for that, the Cameras page too (which has no settings). Opened on its own, the browser keeps its shortcuts.
  addEventListener("keydown", e => {
    if (!e.ctrlKey || e.altKey || e.metaKey || !/^[1-4]$/.test(e.key) || parent === window) return;
    const o = origin || framer();
    if (!o || !local(o)) return;
    e.preventDefault();
    parent.postMessage({ fs: "key", key: e.key, app }, o);
  });

  function init(name, list, acts = {}) {
    app = name; items = list; actions = acts;
    const ids = new Set(list.map(x => x.id));
    for (const ev of ["change", "input"]) document.addEventListener(ev, e => { if (ids.has(e.target.id)) changed(); }, true);
  }
  return { init, control, changed, notify };
})();
