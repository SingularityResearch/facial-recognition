// first_run.js - the question before the app's first start: the licence where no installer asked for it (a Mac,
// Linux, the Raspberry Pi), then what the app needs and doesn't find on this computer, asked about before anything
// else is shown or started (crates/face-server firstrun.rs, licence.rs). The app's own server answers /first-run
// with the stage, the licence while it is asked about, what is missing and how far a download is, and takes the
// answers there; when the question is done this page gives way to the app's tab page. Text only goes into the page
// (textContent).
const $ = id => document.getElementById(id);
const HEAD = { "X-Face-Tool": "1" };
let shown = "", licenceShown = false;

function show(s) {
  if (s.stage === "done") { location.replace("/"); return; }
  // the licence, first
  const licence = s.stage === "licence" || s.stage === "declined";
  $("licence").hidden = !licence;
  $("files").hidden = licence;
  $("main").classList.toggle("wide", licence);
  if (licence) {
    if (s.licence && !licenceShown) { licenceShown = true; $("licenceText").textContent = s.licence; $("licenceText").scrollTop = 0; }
    $("declined").hidden = s.stage !== "declined";
    $("licenceRow").hidden = s.stage === "declined";
    $("error").hidden = !s.error;
    $("error").textContent = s.error || "";
    return;
  }
  const busy = s.stage === "getting", megabytes = Math.max(1, Math.round(s.megabytes));
  $("what").textContent = `Face Recognizer needs files that aren't on this computer yet (${megabytes} MB to download):`;
  const lines = JSON.stringify(s.lines);
  if (lines !== shown) {
    shown = lines;
    $("lines").replaceChildren(...s.lines.map(text => Object.assign(document.createElement("li"), { textContent: text })));
  }
  $("where").textContent = "They come from their publishers and from this project's release page, each is checked " +
    `(its size and SHA-256) before it is kept, and they are put into ${s.folder}. Nothing is sent but the requests for them.`;
  $("without").hidden = !s.needed || busy;
  $("without").textContent = "Without them no face can be found or recognized: the app starts, and its two programs say what they miss.";
  $("bar").hidden = $("step").hidden = !busy;
  if (busy) {
    $("bar").value = s.total ? Math.min(1, s.done / s.total) : 0;
    $("step").textContent = (s.step ? `Getting ${s.step} … ` : "Starting … ") +
      (s.total ? `${Math.round(s.done / 1e6)} of ${Math.round(s.total / 1e6)} MB` : "");
  }
  $("error").hidden = s.stage !== "failed";
  $("error").textContent = s.stage === "failed" ? `That didn't finish: ${s.error}\nWhat was downloaded whole is kept.` : "";
  $("get").textContent = s.stage === "failed" ? "Try again" : busy ? "Downloading …" : "Download";
  $("get").disabled = $("skip").disabled = busy;
}

async function look() {
  try { show(await (await fetch("/first-run", { headers: HEAD })).json()); } catch { /* the app is starting or ending: asked again */ }
}
async function send(answer) {
  $("get").disabled = $("skip").disabled = $("accept").disabled = $("decline").disabled = true;
  try {
    const r = await fetch("/first-run", { method: "POST", headers: { ...HEAD, "Content-Type": "application/json" }, body: JSON.stringify(answer) });
    show(await r.json());
  } catch { look(); }
  $("accept").disabled = $("decline").disabled = false;
}
$("get").addEventListener("click", () => send({ get: true }));
$("skip").addEventListener("click", () => send({ get: false }));
$("accept").addEventListener("click", () => send({ licence: true }));
$("decline").addEventListener("click", () => send({ licence: false }));
look();
setInterval(look, 1000);
