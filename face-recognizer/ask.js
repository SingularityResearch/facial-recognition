// ask.js - a yes-or-no question asked in the page itself, in place of the browser's confirm().
//
// confirm() stops every script of the page until it is answered. In the app window the tab page and the pages in
// its tabs run in one process, so a question left open stopped all of them: the camera's checks and its stream to
// the other devices, and the tab page's "still running" signal, so that after 30 s the app's watchdog took the
// window for frozen and started the app again (app.py watch_window). This dialog stops nothing.
// Served by all three servers; a test (tests/pages.test.js) keeps confirm(), alert() and prompt() out of the pages.
//
// askUser(text, yes) answers a promise of true (the `yes` button) or false (Cancel, Esc). Text only, no markup.
function askUser(text, yes = "OK") {
  return new Promise(done => {
    const make = (tag, css, props = {}) => { const n = Object.assign(document.createElement(tag), props); n.style.cssText = css; return n; };
    const d = make("dialog", "max-width: min(460px, calc(100vw - 32px)); padding: 18px 20px 14px; border-radius: 12px; font: inherit;" +
      "border: 1px solid var(--line, GrayText); background: var(--panel, Canvas); color: var(--fg, CanvasText); box-shadow: 0 18px 50px rgb(0 0 0 / .35);");
    // each page's own colours where it names them (--panel, --fg, --line, --accent), else the system's for its scheme
    const button = "font: inherit; padding: 6px 16px; border-radius: 8px; cursor: pointer; background: transparent; color: inherit;";
    const no = make("button", button + "border: 1px solid var(--line, GrayText);", { type: "button", textContent: "Cancel" });
    const ok = make("button", button + "border: 1px solid var(--accent, currentColor); font-weight: 600;", { type: "button", textContent: yes });
    const row = make("div", "display: flex; justify-content: flex-end; gap: 8px; margin-top: 16px;");
    row.append(no, ok);
    d.append(make("p", "margin: 0; line-height: 1.45; white-space: pre-wrap;", { textContent: String(text) }), row);
    d.setAttribute("role", "alertdialog");
    no.addEventListener("click", () => d.close("no"));
    ok.addEventListener("click", () => d.close("yes"));
    // the page's own keyboard shortcuts stay out of it while the question is open (confirm() blocked them too)
    d.addEventListener("keydown", e => e.stopPropagation());
    d.addEventListener("close", () => { d.remove(); done(d.returnValue === "yes"); });
    document.body.append(d);
    d.showModal();
    ok.focus();
  });
}
