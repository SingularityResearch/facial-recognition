// setup.js - the setup page (setup.html): a page of the labeling tool's server, shown over the app window from the
// Setup button at the top of the Settings tab, and by itself at a new copy's first start. The owner's words (2026-11-25): "an in-app setup wizard
// asks the user the relevant questions about their environment and that guides the user thru the setup process and
// automates anything possible (with their consent)", "If the user has already setup everything ... detect that and
// only show whats missing", and "this is really about how new users setup their devices ... only change any of the
// settings ... if the wizard detects differences or needs new install or settings decisions".
// So it looks first and marks what is set up as done, changing none of it; it asks only what decides the rest; and it
// does nothing but what a button the user presses says. It asks this server alone, and what came from the server is
// put into the page as text only.
(() => {
  "use strict";
  const $ = s => document.querySelector(s);
  const el = (tag, props = {}, ...kids) => {
    const n = Object.assign(document.createElement(tag), props);
    n.append(...kids.filter(k => k != null && k !== false));
    return n;
  };
  async function get(p) {
    const r = await fetch(p, { cache: "no-store", headers: { "X-Face-Tool": "1" } });
    const j = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(j.error || r.statusText);
    return j;
  }
  async function post(p, body) {
    const r = await fetch(p, { method: "POST", cache: "no-store", headers: { "X-Face-Tool": "1", "Content-Type": "application/json" },
                               body: JSON.stringify(body || {}) });
    const j = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(j.error || r.statusText);
    return j;
  }
  const maybe = p => get(p).catch(() => null);
  // the wizard's own answers, remembered with the labeling page's options (one device or several, which way)
  const store = {
    get: k => { try { return window.PageStore?.get(k) ?? null; } catch { return null; } },
    set: (k, v) => { try { window.PageStore?.set(k, v); } catch {} },
  };
  // The app window around this page: only its tab page, on this computer, as the browser reports the frame's parent.
  const parentOrigin = (() => {
    const o = location.ancestorOrigins?.[0];
    return o && /^http:\/\/(127\.0\.0\.1|localhost)(:\d+)?$/.test(o) ? o : null;
  })();
  const tell = m => { if (parentOrigin) window.parent.postMessage({ fs: "setup", ...m }, parentOrigin); };
  const auto = new URLSearchParams(location.search).has("auto");

  // ---------------------------------------------------------------- what is set up
  let found = {};
  async function look() {
    const [summary, db, describe, hardware, servers, analyze, lms, keys, music, version] = await Promise.all([
      maybe("/api/summary"), maybe("/api/db"), maybe("/api/describe/status"), maybe("/api/hardware"),
      maybe("/api/analyze/servers"), maybe("/api/analyze/status"), maybe("/api/lmstudio"), maybe("/api/websearch/key"),
      maybe("/api/music"), maybe("/api/setup")]);
    found = { summary, db, describe, hardware, servers: servers?.servers || [], analyze, lms, keys, music, version };
  }

  // What an update changed, said once on a device that had the app before ("Keep their existing setup where
  // possible - if not notify them about what will change and how"). Each says what it is on THIS device.
  const NOTICES = [
    { id: "settings-groups", text: () => "Settings are grouped by what each one is for now, with a row of buttons at the top that goes to each group. No setting changed." },
    // (a device without LM Studio whose switch is off: nothing changed for it)
    { id: "keep-open", when: f => !!f.lms && (f.lms.installed || f.lms.keep_open), text: f => f.lms.keep_open
      ? `Keeping this computer's model servers open to your other devices is a switch now (Settings, AI models, Model servers on this computer). On this device it is on, as before${f.lms.installed ? ": LM Studio stays served on your network" : ""}. New with it:`
      + " Ollama, llama.cpp's server and vLLM are kept open too, so one that listens on this computer only is started again, just as it was started, listening on your network. Switch it off there to stop all of it."
      : "Keeping this computer's model servers open to your other devices is a switch now (Settings, AI models, Model servers on this computer). On this device it is off, because it has no shared database and no other device uses its servers: the app no longer keeps LM Studio serving on your network or announces it there. What LM Studio's own setting says now is left as it is. Switch it on there if your other devices should use this computer's servers." },
    { id: "analyze", text: () => "New: the Analyze tab, for questions in your own words about anyone or any signal, answered from your records by a language model. It needs a model server (Describing photos and Analyze, below)." },
    { id: "guide", text: () => "The setup guide is inside the app now: the Guide button above, and a button beside each step opens it at that step." },
    // (a device with a shared database keeps it: nothing changed for it)
    { id: "db-here", when: f => !f.db?.config && !!f.db?.host?.can, text: () => "New: a shared database for several devices can be on this computer, set up by the app at one button (Your devices, below): it installs PostgreSQL with pgvector, without administrator rights, and keeps it running." },
  ];
  const seen = () => new Set(String(store.get("fl-setupSeen") || "").split(",").filter(Boolean));
  const updated = f => !!f.version && !f.version.fresh;
  const unseen = f => NOTICES.filter(n => (!n.when || n.when(f)) && !seen().has(n.id));
  const markSeen = f => store.set("fl-setupSeen", [...new Set([...seen(), ...unseen(f).map(n => n.id)])].join(","));

  // Shown by itself only when it has something to say: a new copy with nothing set up yet (no photos, no shared
  // database) that was never closed, or an update whose changes weren't seen yet. Otherwise it stays out of the way.
  async function firstStart() {
    const [summary, db, version, lms] = await Promise.all([maybe("/api/summary"), maybe("/api/db"), maybe("/api/setup"), maybe("/api/lmstudio")]);
    if (!summary || !db || !version) return;
    const f = { summary, db, version, lms };
    const fresh = !(summary.photos > 0) && !db.config && store.get("fl-setupClosed") !== "1";
    if (fresh || (updated(f) && unseen(f).length)) tell({ want: true });
  }

  // ---------------------------------------------------------------- the page
  const CHIP = { done: "Set up", todo: "To do", optional: "Optional", info: "" };
  function card(title, state, ...body) {
    return el("section", { className: "card" },
      el("h2", {}, document.createTextNode(title), CHIP[state] ? el("span", { className: "chip " + state, textContent: CHIP[state] }) : null),
      ...body);
  }
  const p = (text, cls) => el("p", { className: cls || "", textContent: text });
  // A button that does one thing the user asked for: disabled while it runs, its answer said beside it.
  function action(text, run, said) {
    const b = el("button", { className: "b", type: "button", textContent: text });
    b.addEventListener("click", async () => {
      b.disabled = true; if (said) { said.className = "said"; said.textContent = "…"; }
      try {
        const r = await run();
        if (said) said.textContent = typeof r === "string" ? r : r?.message || r?.text || "Done.";
        if (r?.ok === false && said) said.className = "said bad";
      } catch (e) { if (said) { said.className = "said bad"; said.textContent = String(e.message || e); } }
      b.disabled = false;
      await refresh();
    });
    return b;
  }
  // A form of a few fields and one button; secrets are typed into password fields and kept nowhere in the page.
  function form(fields, text, run) {
    const said = el("p", { className: "said" });
    const inputs = fields.map(f => el("input", { type: f.secret ? "password" : "text", value: f.value || "", placeholder: f.placeholder || "",
                                                 autocomplete: "off", ariaLabel: f.label }));
    const b = action(text, () => run(Object.fromEntries(fields.map((f, i) => [f.name, inputs[i].value.trim()]))), said);
    b.addEventListener("click", () => setTimeout(() => inputs.forEach((i, j) => { if (fields[j].secret) i.value = ""; }), 0));
    return el("div", {}, el("div", { className: "form" },
      ...fields.map((f, i) => el("label", {}, document.createTextNode(f.label), inputs[i])), b), said);
  }
  const go = (tab, group) => el("button", { className: "b quiet", type: "button", textContent: { label: "Open Labeling", track: "Open Face tracking", set: "Open Settings" }[tab],
                                             onclick: () => tell({ go: tab, group }) });
  // The setup guide over this page, at one of its parts (the program carries it: /guide, the parts are its headings'
  // links); a test holds that every part named here is one the guide has.
  function openGuide(part) {
    $("#guideFrame").src = "/guide" + (part ? "#" + part : "");
    $("#guidePanel").hidden = false;
    $("#guideBack").focus();
  }
  const guideAt = (part, text) => el("button", { className: "b quiet", type: "button", textContent: text || "How, step by step",
                                                  onclick: () => openGuide(part) });
  const choice = (key, options) => el("div", { className: "choices" }, ...options.map(([value, text]) =>
    el("button", { className: "b", type: "button", textContent: text, onclick: () => { store.set(key, value); draw(); } })));

  // 0. what this version changed, on a device that had the app before
  function changes() {
    if (!updated(found)) return null;
    const list = unseen(found);
    if (!list.length) return null;
    const from = found.version.previous && found.version.previous !== "older" ? ` (from ${found.version.previous})` : "";
    return card(`What changed in this version${from}`, "info",
      p("Your setup is kept as it was. What this update changed on this device:"),
      el("ul", {}, ...list.map(n => el("li", { textContent: n.text(found) }))),
      el("div", { className: "choices" }, el("button", { className: "b", type: "button", textContent: "Got it",
        onclick: () => { markSeen(found); draw(); } })));
  }

  // 1. photos
  function photos() {
    const s = found.summary;
    if (!s) return card("Your photos", "info", p("The labeling tool doesn't answer yet: look again in a moment.", "muted"));
    const people = Array.isArray(s.people) ? s.people.length : 0;
    if (s.photos > 0) return card("Your photos", "done", p(`${s.photos.toLocaleString()} photos, ${people.toLocaleString()} people named.`));
    return card("Your photos", "todo",
      p("Add a folder of your photos and name the faces it finds: the more photos of a person, the better they are recognized."),
      el("div", { className: "choices" }, go("label")));
  }

  // 2. one device or several: the shared database
  function devices() {
    const db = found.db;
    if (!db) return card("Your devices", "info", p("Couldn't ask about the shared database: look again in a moment.", "muted"));
    if (db.config) {
      const c = db.config, n = (db.devices || []).length + 1;
      const rows = [p(`Shared database on ${c.host}:${c.port} (${c.dbname}). ${n} device${n === 1 ? "" : "s"}; this device is ${db.device?.name || "this one"}.`)];
      // (just connected, the sync still says it has none until its first round with the new one is done)
      if (db.sync?.text) rows.push(p(db.sync.state === "off" ? "The sync with it is starting: Look again in a moment." : db.sync.text, "muted"));
      const k = db.keys || {};
      if (!k.current) rows.push(el("div", {}, p("Recommended: a passphrase for keys, so your Web search keys and the model server's key are kept in the database encrypted and every device you let in gets them.", "note"),
        form([{ name: "passphrase", label: "Passphrase (10 characters or more)", secret: true }, { name: "again", label: "Again", secret: true }],
             "Set the passphrase", a => post("/api/db/passphrase", a).then(() => "Set."))));
      else if (!k.here) rows.push(p("This device doesn't have the passphrase for keys yet: Settings, Passphrase for keys, Get it from another device.", "note"));
      const h = db.host || {};
      if (h.missing) {
        const said = el("p", { className: "said" });
        rows.push(el("div", {}, p(`The database this computer held isn't in ${h.folder} any more, so this device can't use it. Forget it to set a new one up or join your other devices' one.`, "bad"),
          el("div", { className: "choices" }, action("Forget it", () => post("/api/db/host", { way: "forget" }).then(r => r.message), said)), said));
      }
      if (h.held && !h.missing) rows.push(p(`This computer holds it: PostgreSQL ${h.postgres} with pgvector ${h.pgvector}, ${h.running ? "running" : "not running"} on port ${h.port}; it keeps running when the app is closed and starts at sign-in. Its data is in ${h.folder}.`, "muted"));
      return card("Your devices", "done", ...rows);
    }
    if (!db.available) return card("Your devices", "info", p(db.why || "No shared database on this device.", "muted"));
    // a database this computer held, still in the app's folder, that this private folder doesn't know (installed again)
    if (db.host?.found) return card("Your devices", "todo", ...takeOn(db.host));
    const many = store.get("fl-setupDevices");
    if (many === "one") return card("Your devices", "done", p("Just this device: everything stays on it."),
      el("div", { className: "choices" }, el("button", { className: "b quiet", type: "button", textContent: "I'll use several devices after all",
        onclick: () => { store.set("fl-setupDevices", "many"); draw(); } })));
    if (many !== "many") return card("Your devices", "todo",
      p("Will you use Face Recognizer on more than one device (several computers, phones, cameras that share names and watch each other)?"),
      choice("fl-setupDevices", [["one", "Just this device"], ["many", "Several devices"]]));
    const way = store.get("fl-setupDbWay");
    const back = el("button", { className: "b quiet", type: "button", textContent: "Back", onclick: () => { store.set("fl-setupDbWay", ""); draw(); } });
    if (way === "join") return card("Your devices", "todo",
      p("Your other devices share a database already: this device joins it with a code, nothing typed."),
      el("ol", {}, el("li", { textContent: "Both devices on the same local network (not a guest Wi-Fi)." }),
        el("li", { textContent: "Here: Face tracking, Controls, Join a shared database. It shows a code." }),
        el("li", { textContent: "On a device of the database: Face tracking, Controls, Join a new device: scan the code, and say yes. (Or the other way round.)" })),
      el("div", { className: "choices" }, go("track"), guideAt("4-each-further-device"), back));
    if (way === "here" && db.host?.can) return card("Your devices", "todo", ...hostHere(db.host), el("div", { className: "choices" }, back));
    if (way === "new" || way === "have") return card("Your devices", "todo", ...newServer(way === "have"), el("div", { className: "choices" }, back));
    return card("Your devices", "todo",
      p("Several devices share one database (PostgreSQL with pgvector) on a computer of yours that is on most of the time: a desktop, a NAS, a small server."),
      choice("fl-setupDbWay", [["join", "My other devices have one: join it"],
        ...(db.host?.can ? [["here", "Set up a new one on this computer"]] : []),
        ["new", db.host?.can ? "Set up a new one on another computer or a NAS" : "Set up a new one"],
        ["have", "Use a PostgreSQL server I already have"]]));
  }
  // A database this computer held, taken on again (dbhost.rs take_on): its login gets a new password, nothing else.
  function takeOn(h) {
    const w = h.work || {};
    const out = [p(`The shared database this computer held is still here, in ${h.folder}, from before the app was installed again. Take it on: the app gives its own login there a new password, starts it and connects this device. Nothing in the database changes; your other devices use it as before.`)];
    if (w.state === "working") { out.push(p(`${w.text}`, "said")); poll(); return out; }
    if (w.state === "failed") out.push(p(w.text, "said bad"));
    const said = el("p", { className: "said" });
    out.push(el("div", {}, el("div", { className: "choices" },
      action("Take it on", () => post("/api/db/host", { way: "take" }).then(r => { poll(); return r.message; }), said)), said));
    return out;
  }
  // The shared database on this computer: the app installs PostgreSQL with pgvector, makes, starts and connects it
  // (dbhost.rs), at one button; while it works the page looks again every two seconds.
  let polling = null;
  function poll() { if (!polling) polling = setTimeout(async () => { polling = null; await refresh(); }, 2000); }
  function hostHere(h) {
    const w = h.work || {};
    const out = [
      p(`The app installs PostgreSQL ${h.postgres} with pgvector ${h.pgvector} on this computer (${h.mb} MB from conda-forge, each file checked), makes the database with certificates of your own, starts it, and connects this device. It keeps running when the app is closed and starts again when you sign in, so choose a computer that is on, and signed in to, most of the time. Nothing is installed into the system and no administrator rights are needed; the data stays in ${h.folder}.`),
      p("Windows asks whether PostgreSQL may be reached on your network: choose Show more, Private networks only, then Allow. Your other devices then join it with a code.", "note"),
    ];
    if (w.state === "working") {
      out.push(p(`${w.text}${w.percent != null ? ` ${w.percent}%` : ""}`, "said"));
      poll();
      return out;
    }
    if (w.state === "failed") out.push(p(w.text, "said bad"));
    const said = el("p", { className: "said" });
    out.push(el("div", {}, el("div", { className: "choices" },
      action("Install and set it up", () => post("/api/db/host", {}).then(r => { poll(); return r.message; }), said),
      guideAt("3-a-shared-database-for-several-devices-once", "What this does, step by step")), said));
    return out;
  }
  function newServer(have) {
    const certs = found.db?.certs || {};
    const out = [];
    if (!have) out.push(el("div", {}, p("1. On the computer that will hold it, install PostgreSQL 13 or later with pgvector 0.5 or later, and give its postgres login a password. The guide says how on Linux, Docker or a NAS, macOS and Windows."),
      el("div", { className: "choices" }, guideAt("31-install-postgresql-and-pgvector-on-the-server-computer"))));
    // the certificates: made here, with the user's yes (nothing leaves this device)
    const step = have ? "1." : "2.";
    if (certs.made || certs.ca) {
      out.push(el("div", {}, p(`${step} Certificates: ${certs.made ? `made for ${(certs.names || []).join(", ")}` : "a certificate authority of yours is kept"}, in ${certs.folder}.`),
        certs.steps ? el("pre", { textContent: certs.steps }) : null,
        el("div", { className: "choices" }, el("button", { className: "b quiet", type: "button", textContent: "Open the folder", onclick: () => tell({ open: "private" }) }))));
    } else {
      out.push(el("div", {},
        p(`${step} The server needs a certificate from a certificate authority of yours, which every device checks it with. The app makes both: the authority (kept on this device, its key protected) and the server's certificate and key, with the lines its two configuration files need. Nothing is sent anywhere.`),
        form([{ name: "server", label: "The server's address, as your devices reach it", placeholder: "for example 192.168.1.20" }],
             "Make the certificates", a => post("/api/db/certificates", a))));
      if (have) out.push(p("Made the server's certificate yourself? Settings, Devices and shared database, A certificate authority of your own: paste it there instead.", "note"));
    }
    const last = have ? "2." : "3.";
    out.push(el("div", {},
      p(`${last} Put server.crt and server.key on the server as setup-the-server.txt says, restart PostgreSQL, then Connect. The first time with the server's administrator login (user postgres): the app checks the server shows its certificate, then makes its own login, the database, pgvector and its tables, registers your certificate authority there for every device, and keeps only its own login, never the administrator's.`),
      form([{ name: "host", label: "Server", value: (certs.names || [])[0] || "", placeholder: "192.168.1.20" },
            { name: "port", label: "Port", value: "5432" },
            { name: "user", label: "User", value: "postgres" },
            { name: "password", label: "Password", secret: true }],
           "Connect", a => post("/api/db/setup", { host: a.host, port: +a.port || 5432, dbname: "facerec", user: a.user || "postgres", password: a.password })),
      el("div", { className: "choices" }, guideAt("33-set-up-the-server", "The server's steps, and what Connect's answers mean"))));
    return out;
  }

  // 3. describing photos and Analyze
  const KINDS = { "LM Studio": "lmstudio", "Ollama": "ollama", "llama.cpp": "llamacpp", "vLLM": "vllm" };
  function models() {
    const d = found.describe, a = found.analyze;
    if (!d) return card("Describing photos and Analyze", "info", p("Couldn't ask about the model server: look again in a moment.", "muted"));
    const rows = [];
    const here = (found.hardware?.ai || []).filter(e => e.running && KINDS[e.app] && /^https?:\/\/(127\.0\.0\.1|localhost)[:/]/.test(e.url || ""));
    const describing = d.ok && d.model;
    if (describing) rows.push(p(`Describing photos with ${d.model}${d.model === d.builtin?.name ? " (the built-in model)" : `, on ${d.config?.url || "this computer"}`}.`));
    // Analyze needs a model server (the built-in model describes photos only)
    const answering = a?.ok && a?.model;
    if (answering) rows.push(p(`Analyze answers with ${a.model} on ${a.where || "this computer"}.`));
    const done = describing && answering;
    if (!describing || !answering) {
      rows.push(p(describing ? "Analyze needs a model server that answers (the built-in model describes photos only)." :
        "No model answers yet. Choose where the model runs:"));
      for (const e of here) rows.push(el("div", { className: "row" },
        el("span", { textContent: `${e.app} on this computer: ${e.models} model${e.models === 1 ? "" : "s"}${e.vision?.length ? `, reads pictures: ${e.vision.slice(0, 3).join(", ")}` : ""}` }),
        action(`Use ${e.app} here`, () => post("/api/describe/config", { platform: KINDS[e.app], url: "" }).then(() => `The model server is ${e.app} on this computer.`))));
      for (const s of found.servers) rows.push(el("div", { className: "row" },
        el("span", { textContent: `${s.kind} on ${s.name}: ${s.models.length} model${s.models.length === 1 ? "" : "s"}${(s.loaded || []).length ? `, loaded ${s.loaded[0]}` : ""}` }),
        action(`Use it for both`, () => post("/api/describe/config", { platform: s.platform, url: s.url }).then(() => `The model server is ${s.kind} on ${s.name}: each photo described and each question goes there.`))));
      if (!here.length && !found.servers.length) rows.push(p("None found on this computer or your devices. Install one (LM Studio from lmstudio.ai, or Ollama from ollama.com), load a vision model, start its server, and look again. Or describe photos with the built-in model (below).", "note"));
    }
    // the built-in model, for photos, where it isn't there yet
    const b = d.builtin || {};
    if (!describing && !b.ready && !b.running) rows.push(el("div", { className: "row" },
      el("span", { textContent: `The built-in model describes photos on this computer's processor, without a server (${((b.size_mb || 831) / 1000).toFixed(1)} GB from Hugging Face, checked by SHA-256).` }),
      action("Download it", () => post("/api/describe/getmodel", {}).then(() => "Downloading: Settings, AI models, Describe photo shows how far."))));
    // a model this computer can run, loaded with the settings measured for it
    // (only where something is missing: a device that describes and answers keeps the model it has)
    const s = d.suggested || {};
    if (!done && s.here && s.there && s.model && !["llamacpp", "vllm"].includes(d.config?.platform)) rows.push(el("div", { className: "row" },
      el("span", { textContent: `Suggested for this computer: ${s.name} (${s.model}).` }),
      action(`Load ${s.model}`, () => post("/api/describe/load", {}).then(() => "Loading: it takes a minute."))));
    // Analyze on another computer keeps this one's processor for its camera
    if (!done && found.servers.length && a && !a.server) rows.push(el("div", { className: "row" },
      el("span", { className: "note", textContent: "Analyze can run on another of your computers, so this one keeps its processor and graphics for its camera." }),
      ...found.servers.slice(0, 3).map(sv => action(`Analyze on ${sv.name}`, () => post("/api/analyze/config", { server: sv.url }).then(() => `Analyze runs on ${sv.name}.`)))));
    // speaking questions
    const sp = a?.speech || {};
    if (sp.can && !sp.ready && !sp.getting) rows.push(el("div", { className: "row" },
      el("span", { className: "note", textContent: `To speak questions to Analyze: the speech model, made into words on this computer (${Math.round((sp.bytes || 0) / 1e6)} MB).` }),
      action("Download it", () => post("/api/analyze/speech/get", {}).then(() => "Downloading."))));
    if (!done) rows.push(el("div", { className: "choices" }, guideAt("5-describing-photos-and-analyze", "Model servers, step by step")));
    return card("Describing photos and Analyze", done ? "done" : "todo", ...rows);
  }

  // 4. this computer's model servers for the other devices
  function keepOpen() {
    const lms = found.lms, many = !!found.db?.config || store.get("fl-setupDevices") === "many";
    const here = (found.hardware?.ai || []).filter(e => e.running && KINDS[e.app]);
    if (!lms || !many || !here.length) return null;
    if (lms.keep_open) return card("This computer's model servers for your devices", "done", p("Kept open to your other devices while the app runs."));
    return card("This computer's model servers for your devices", "optional",
      p(`Your other devices can use ${here.map(e => e.app).join(" and ")} on this computer if it listens on your network. Switched on, the app keeps LM Studio serving on your network, and starts Ollama, llama.cpp or vLLM again listening there where they listen on this computer only (just as they were started otherwise). Anyone else on your local network could reach them too.`),
      el("div", { className: "choices" }, action("Keep them open", () => post("/api/lmstudio/keep", { on: true }).then(() => "Switched on."))));
  }

  // 5. optional services
  function optional() {
    const k = found.keys?.services || {}, m = found.music || {};
    const row = (text, state, tab, group) => el("div", { className: "row" },
      el("span", {}, document.createTextNode(text), el("span", { className: "chip " + state, textContent: " " + CHIP[state], style: "margin-left:8px" })),
      go(tab, group));
    return card("Optional", "info",
      row("Web search on a face: your own Google Cloud Vision and Amazon Rekognition keys.", k.vision || k.rekognition ? "done" : "optional", "set", "labeling"),
      row("Music as a signal: your own Spotify key.", m.spotify?.connected ? "done" : "optional", "set", "music"),
      row("This device's location (off until you switch it on).", "optional", "set", "location"),
      el("div", { className: "choices" }, guideAt("6-optional-services", "Each, step by step")));
  }

  // 6. the network, for several devices
  function network() {
    if (!found.db?.config && store.get("fl-setupDevices") !== "many") return null;
    return card("Your network", "info",
      p("Your devices reach each other on your local network: photos and cameras on port 8767, a join code on 8770, the database on 5432 (on its server), model servers on 1234, 11434, 8080 or 8000."),
      p("On Windows the network must be set as Private (Settings, Network & internet), and when Windows asks whether Face Recognizer (or a model server) may be reached on your network, choose Show more, Private networks only, then Allow.", "note"),
      el("div", { className: "choices" }, guideAt("7-your-network-and-firewall", "Your network, step by step")));
  }

  function draw() {
    $("#cards").replaceChildren(...[changes(), photos(), devices(), models(), keepOpen(), optional(), network()].filter(Boolean));
  }
  async function refresh() { await look(); draw(); }

  $("#again").addEventListener("click", refresh);
  $("#guide").addEventListener("click", () => openGuide(""));
  $("#guideBack").addEventListener("click", () => { $("#guidePanel").hidden = true; $("#guide").focus(); });
  // closed: a new copy isn't asked again by itself, and what the update changed counts as said
  $("#close").addEventListener("click", () => { store.set("fl-setupClosed", "1"); if (updated(found)) markSeen(found); tell({ close: true }); });
  // (the labeling page's remembered options first, as the other pages take them)
  try { window.PageStore?.restore("label", "fl-"); } catch {}
  if (auto) firstStart(); else refresh();
})();
