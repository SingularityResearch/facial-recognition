// first_run.js - the question before the app's first start: what it needs and doesn't find on this computer,
// asked about before anything else is shown or started (crates/face-server firstrun.rs). The app's own server
// answers /first-run with what is missing and how far a download is, and takes the answer there; when the
// question is done this page gives way to the app's tab page. Text only goes into the page (textContent).
const $ = id => document.getElementById(id);
const HEAD = { "X-Face-Tool": "1" };
let shown = "";

function show(s) {
  if (s.stage === "done") { location.replace("/"); return; }
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
async function answer(get) {
  $("get").disabled = $("skip").disabled = true;
  try {
    const r = await fetch("/first-run", { method: "POST", headers: { ...HEAD, "Content-Type": "application/json" }, body: JSON.stringify({ get }) });
    show(await r.json());
  } catch { look(); }
}
$("get").addEventListener("click", () => answer(true));
$("skip").addEventListener("click", () => answer(false));
look();
setInterval(look, 1000);
