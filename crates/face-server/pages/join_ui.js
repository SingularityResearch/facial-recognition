"use strict";
// The join page. Either device shows the join code and the other reads it with its camera:
// - a device of a shared database shows one for a new device, or reads the code a new device shows and asks its
//   owner before it hands anything over;
// - a device without a shared database reads the code a device of one shows, or shows its own for such a device.
// Every request's answer is read, and nothing here asks with confirm(). What another device says of itself (its
// name, its address) is only ever put into the page as text.

const $ = id => document.getElementById(id);
const PROMPT = "Point the camera at the join code.";
const ENDED = {
  given: ["A device read the code and was handed the login. It joins by itself now.", "good"],
  expired: ["Nobody read the code in time.", "bad"],
  refused: ["The code was ended: something tried it with a wrong key several times.", "bad"],
  failed: ["Joining failed.", "bad"],
  joined: ["", ""],
  stopped: ["", ""],
};
// what the page says on a device of a shared database, and on one without
const TEXT = {
  member: {
    scanWay: "Scan the new device's code",
    scan: "On the new device open Face Tracker, choose Join a shared database, then Show a code, and point this " +
      "camera at it. You are asked before anything is handed over.",
    show: "On the new device open Face Tracker, choose Join a shared database, and point its camera at the code " +
      "(a phone's own camera app reads it too). Whoever reads it gets this database's login and the passphrase " +
      "for keys: show it only to a device of your own.",
  },
  fresh: {
    scanWay: "Scan a code",
    scan: "On a device of the database open Shared database, choose Show a code, and point this camera at it (a " +
      "phone's own camera app reads it too, and opens this page).",
    show: "On a device of the database open Shared database, choose Scan the new device's code, and point its " +
      "camera at this screen. Its owner is asked there before anything is handed over.",
  },
};
const NO_ANSWER = { error: "The app doesn't answer." };

let stream = null, facing = "environment", scanning = false, errorUntil = 0, shownCode = false, canvas = null;
let state = null;   // the server's last answer to /api/join
let showError = ""; // why this device showed no code when it was asked to
let named = null;   // what this device is called in the shared database ({name, own, numbered}), once known
let way = null;     // "scan" or "show", once the owner chose; before that the device's usual one
// a join code the phone's own camera read: the shell opened this page with it. Taken out of the address at
// once, so that loading the page again doesn't ask again.
let opened = new URLSearchParams(location.search).get("code");
if (opened !== null) history.replaceState(null, "", location.pathname);

function say(el, text, cls = "") { el.textContent = text; el.className = "say " + cls; }

// GET without a body, POST with one. The answer is always read.
async function api(path, body) {
  const r = await fetch(path, body === undefined ? {} : { method: "POST", headers: { "X-Face-Tool": "1" }, body });
  return r.json();
}

function stopCamera() {
  if (stream) for (const t of stream.getTracks()) t.stop();
  stream = null;
  $("cam").srcObject = null;
}

async function startCamera() {
  stopCamera();
  try {
    stream = await navigator.mediaDevices.getUserMedia({
      audio: false, video: { facingMode: { ideal: facing }, width: { ideal: 1280 }, height: { ideal: 720 } } });
  } catch (e) {
    say($("scanSay"), "The camera didn't start: " + (e.message || e.name), "bad");
    return false;
  }
  $("cam").srcObject = stream;
  try { await $("cam").play(); } catch { /* it plays once it has a picture */ }
  return true;
}

// The camera's picture now, as a JPEG (null before the first picture).
function shot() {
  const v = $("cam");
  if (!v.videoWidth) return Promise.resolve(null);
  canvas = canvas || document.createElement("canvas");
  canvas.width = v.videoWidth; canvas.height = v.videoHeight;
  canvas.getContext("2d").drawImage(v, 0, 0);
  return new Promise(done => canvas.toBlob(done, "image/jpeg", 0.9));
}

const wait = ms => new Promise(done => setTimeout(done, ms));

function stopScan() { scanning = false; stopCamera(); }

async function scan() {
  if (scanning) return;
  scanning = true;
  if (await startCamera()) say($("scanSay"), PROMPT);
  while (scanning) {
    const jpeg = stream ? await shot() : null;
    if (jpeg) {
      let r;
      try { r = await api("/api/join/frame", jpeg); } catch { r = NO_ANSWER; }
      if (!scanning) break;
      // this device joined, or read a new device's code: the page says so from the server's own state
      if (r.joined || r.asked) { await look(); continue; }
      if (r.error) { say($("scanSay"), r.error, "bad"); errorUntil = Date.now() + 5000; }
      else if (r.found) say($("scanSay"), "Read the code: asking the other device…");
      else if (Date.now() > errorUntil) say($("scanSay"), PROMPT);
    }
    await wait(250);
  }
}

// The code this device shows, and how its last one ended.
function showState(s) {
  const on = !!s.showing;
  $("code").hidden = $("hideBtn").hidden = !on;
  $("showBtn").textContent = on ? "A new code" : "Show a join code";
  if (on) {
    if (!shownCode) { $("code").src = "/api/join/code.svg?" + Date.now(); shownCode = true; }
    say($("showSay"), `Good for ${s.showing.left} more seconds, for one device.`);
  } else {
    shownCode = false;
    $("code").removeAttribute("src");
    const [text, cls] = ENDED[s.last] || ["", ""];
    if (showError) say($("showSay"), showError, "bad");
    else say($("showSay"), s.last === "failed" && s.error ? "Joining failed: " + s.error : text, cls);
  }
}

// The page as the server's state has it.
function render() {
  if (!state) return;
  const s = state, member = !!s.joined;
  // a code the camera app opened is asked about first on a new device; a device of a database hands it to its
  // server at once (which only keeps it) and asks below
  $("opened").hidden = !(opened && !member);
  $("asked").hidden = !s.asked;
  const waits = !$("opened").hidden || !!s.asked;
  $("ways").hidden = waits;
  if (s.asked) {
    $("askedName").textContent = s.asked.name || "A device that gave no name";
    $("askedAddr").textContent = s.asked.address ? `at ${s.asked.address} on this network` : "";
  }
  // a code that is being shown keeps the page on it
  const on = waits ? null : s.showing ? "show" : way || (member ? "show" : "scan");
  const text = TEXT[member ? "member" : "fresh"];
  $("namePart").hidden = !member || waits || !named;
  $("intro").textContent = member
    ? `This device is in the shared database as ${named?.name || s.name || "this device"}. To let a new device in, show it a code or scan the one it shows.`
    : "This device isn't in a shared database yet. A device that is in one lets it in: scan the code it shows, or show one for it to scan.";
  $("scanWay").textContent = text.scanWay;
  $("scanHelp").textContent = text.scan;
  $("showHelp").textContent = text.show;
  $("scanWay").setAttribute("aria-pressed", String(on === "scan"));
  $("showWay").setAttribute("aria-pressed", String(on === "show"));
  $("scanPart").hidden = on !== "scan";
  $("showPart").hidden = on !== "show";
  showState(s);
  if (on === "scan") scan(); else stopScan();
}

// What this device is called in the shared database. A device that calls itself what another one goes by already
// was given a number to tell it apart: whoever is at it is then asked for a name of their own.
async function loadName() {
  let r;
  try { r = await api("/api/join/name"); } catch { return false; }
  if (!r || !r.name) return false;
  const first = !named;
  named = r;
  $("namePart").classList.toggle("ask", !!r.numbered);
  $("nameHelp").textContent = r.numbered
    ? `Another of your devices is called ${r.own} already, so this one is ${r.name} for now. Give it a name of your own (every device shows it):`
    : `Every device of your shared database shows this one as ${r.name}. You can give it another name here:`;
  if (first || document.activeElement !== $("nameField")) $("nameField").value = r.name;
  if (state) render();
  return true;
}

// (a device that only just joined is known to the database a few seconds later)
async function nameWhenKnown() {
  for (let i = 0; i < 20 && !(await loadName()); i++) await wait(2000);
}

$("nameBtn").addEventListener("click", async () => {
  $("nameBtn").disabled = true;
  let r;
  try { r = await api("/api/join/name", $("nameField").value); } catch { r = NO_ANSWER; }
  $("nameBtn").disabled = false;
  if (r.error) return say($("nameSay"), r.error, "bad");
  say($("nameSay"), `This device is called ${r.name} now.`, "good");
  $("nameField").blur();
  await loadName();
});

async function look() {
  let s;
  try { s = await api("/api/join"); } catch { return; }
  const was = state;
  state = s;
  if (was && !was.joined && s.joined) {
    way = null; opened = null;
    say($("mainSay"), "This device joined the shared database.", "good");
    nameWhenKnown();
  }
  render();
}

// A code the camera app opened, on a device of a shared database: its server keeps a new device's code, and the
// owner is asked like after a scan; any other code is refused there with the reason.
async function useOpened() {
  if (!opened || !state || !state.joined) return;
  const text = opened;
  opened = null;
  let r;
  try { r = await api("/api/join/code", text); } catch { r = NO_ANSWER; }
  if (r.error) say($("mainSay"), r.error, "bad");
  await look();
}

$("joinBtn").addEventListener("click", async () => {
  if (!opened) return;
  $("joinBtn").disabled = true;
  say($("openSay"), "Asking the other device…");
  let r;
  try { r = await api("/api/join/code", opened); } catch { r = NO_ANSWER; }
  $("joinBtn").disabled = false;
  if (r.joined) { opened = null; return look(); }
  // the code can't be used: say why, and offer this device's camera for a new one
  $("joinBtn").hidden = true;
  $("notNowBtn").textContent = "Scan a new code";
  say($("openSay"), r.error || "The other device is busy: try again.", "bad");
});

$("notNowBtn").addEventListener("click", () => { opened = null; render(); });

$("giveBtn").addEventListener("click", async () => {
  $("giveBtn").disabled = true;
  say($("askedSay"), "Handing over…");
  let r;
  try { r = await api("/api/join/give", "{}"); } catch { r = NO_ANSWER; }
  $("giveBtn").disabled = false;
  say($("askedSay"), r.error || "", "bad");
  if (r.given) {
    way = null;   // done: back to what a device of the database starts on, the camera off
    say($("mainSay"), `${r.name || "The new device"} was handed the login${r.passphrase ? " and the passphrase for keys" : ""}. It joins by itself now.`, "good");
  } else say($("mainSay"), r.error || "Nothing was handed over.", "bad");
  await look();
});

async function hide() {
  try { await api("/api/join/hide", "{}"); } catch { /* a code ends by itself */ }
}

$("notGiveBtn").addEventListener("click", async () => { say($("askedSay"), ""); await hide(); await look(); });

$("flipBtn").addEventListener("click", async () => {
  facing = facing === "environment" ? "user" : "environment";
  if (await startCamera()) say($("scanSay"), PROMPT);
});

$("scanWay").addEventListener("click", async () => {
  way = "scan";
  say($("mainSay"), "");
  if (state && state.showing) await hide();   // (the code is good for nothing once the owner turns to the camera)
  await look();
});

$("showWay").addEventListener("click", () => { way = "show"; say($("mainSay"), ""); render(); });

$("showBtn").addEventListener("click", async () => {
  shownCode = false;
  let r;
  try { r = await api("/api/join/show", "{}"); } catch { r = NO_ANSWER; }
  showError = r.error || "";
  await look();
});

$("hideBtn").addEventListener("click", async () => { await hide(); await look(); });

// leaving the page ends a code that is being shown, and forgets one that was read and not answered
addEventListener("pagehide", () => {
  if (shownCode || (state && state.asked)) {
    fetch("/api/join/hide", { method: "POST", headers: { "X-Face-Tool": "1" }, body: "{}", keepalive: true })
      .then(r => r.body?.cancel()).catch(() => {});
  }
  stopCamera();
});

(async () => {
  await look();
  if (state && state.joined) loadName();
  await useOpened();
  for (;;) { await wait(1000); await look(); }
})();
