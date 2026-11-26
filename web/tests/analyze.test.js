// The Analyze page (analyze.html, analyze.js): an answer's Markdown read into elements, never markup; the charts'
// arithmetic; and what the page may and may not do with what came from the records.
const test = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");
const path = require("node:path");
const A = require("../analyze.js");
const read = f => fs.readFileSync(path.join(__dirname, "..", f), "utf8");

test("an answer's Markdown is read into blocks of text, and nothing in it becomes markup", () => {
  const b = A.blocks("**Ann** was seen at `10:42`.\n\n- one\n- *two*\n\n1. first\n2. second\n### Heading\n<img src=x onerror=alert(1)>");
  assert.deepStrictEqual(b.map(x => x.type), ["p", "ul", "ol", "h", "p"]);
  assert.deepStrictEqual(b[0].spans, [["b", "Ann"], ["t", " was seen at "], ["c", "10:42"], ["t", "."]]);
  assert.deepStrictEqual(b[1].items, [[["t", "one"]], [["i", "two"]]]);
  assert.deepStrictEqual(b[4].spans, [["t", "<img src=x onerror=alert(1)>"]], "a tag is text");
  // a table's rows become lines of text, its rule line goes
  assert.deepStrictEqual(A.blocks("| a | b |\n|---|---|\n| 1 | 2 |").map(x => x.spans.map(s => s[1]).join("")), ["a · b 1 · 2"]);
});

test("a chart's axes are round numbers, its lines break where nobody was in view", () => {
  assert.deepStrictEqual(A.ticks(0, 100), [0, 50, 100]);
  assert.deepStrictEqual(A.ticks(58, 109), [60, 80, 100]);
  assert.deepStrictEqual(A.ticks(5, 5).length > 0, true, "a flat line still has an axis");
  const y = A.yRange([{ points: [[0, 10, 8, 12], [1, 20, 18, 22]] }]);
  assert.ok(y[0] < 8 && y[1] > 22);
  assert.deepStrictEqual(A.runs([[0], [10], [20], [100], [110]], 30).map(r => r.length), [3, 2]);
  const t = A.timeTicks(Date.UTC(2026, 10, 25, 0), Date.UTC(2026, 10, 25, 6));
  assert.ok(t.ticks.length >= 2 && t.ticks.length <= 7, String(t.ticks.length));
});

test("a count reads whole, and a bar is as wide as the step between two times, not the plot over the bars' number", () => {
  assert.deepStrictEqual([23, 0, 62.5, 58, 0.25, -0.0001, 1234.4].map(A.fmtNum), ["23", "0", "62.5", "58", "0.25", "0", "1234"]);
  const month = 30 * 86400e3;
  // fifteen months over seven years: a bar stands for a month, so the bars are thin and the axis is the whole time
  const times = [0, month, 2 * month, 40 * month, 84 * month];
  assert.strictEqual(A.barSlot(times, 60), month);
  assert.strictEqual(A.barSlot([5], 3600), 3600e3, "one time alone stands for the chart's bucket");
  assert.strictEqual(A.barSlot([5, 5], 0), 86400e3, "equal times and no bucket: a day");
});

test("a gallery's pictures: most relevant first is the answer's own order, by date puts the undated last", () => {
  const p = [{ photo: "a", taken: 20 }, { photo: "b", taken: null }, { photo: "c", taken: 30 }, { photo: "d" }, { photo: "e", taken: 10 }];
  const ids = l => l.map(x => x.photo).join("");
  assert.strictEqual(ids(A.ordered(p, "relevance")), "abcde");
  assert.strictEqual(ids(A.ordered(p, "newest")), "caebd");
  assert.strictEqual(ids(A.ordered(p, "oldest")), "eacbd");
  assert.strictEqual(ids(A.ordered(p, "anything else")), "abcde", "an unknown order is the answer's own");
  assert.strictEqual(ids(p), "abcde", "the list given is left as it was");
  const page = read("analyze.js");
  assert.match(page, /SORT_KEY = "fl-anaSort"/, "the choice is remembered with the page's other options");
  assert.match(page, /id: "anaSort"/, "and shown in the Settings tab");
});

test("the page puts what came from the records into it as text only, and asks its own server only", () => {
  const js = read("analyze.js");
  assert.doesNotMatch(js, /innerHTML|outerHTML|insertAdjacentHTML|document\.write/);
  assert.doesNotMatch(js, /https?:\/\/(?!www\.w3\.org\/2000\/svg)/, "no other address");
  for (const route of ["/api/analyze/ask", "/api/analyze/job", "/api/analyze/stop", "/api/analyze/status", "/api/analyze/photo",
                       "/api/analyze/picture", "/api/analyze/listen", "/api/analyze/speech/get", "/api/analyze/config", "/api/analyze/load"]) {
    assert.ok(js.includes(route), route);
  }
  // the conversation is the page's memory only
  assert.doesNotMatch(js, /localStorage|sessionStorage|indexedDB/);
  const html = read("analyze.html");
  assert.doesNotMatch(html, /\son[a-z]+=/i, "no inline handlers");
  assert.doesNotMatch(html, /<script(?![^>]*\ssrc=)/i, "no inline script");
  // no example questions under the heading (the owner's word)
  assert.doesNotMatch(js, /chips|className: "chip"/);
});

test("the window has an Analyze tab, the labeling tool's page, Ctrl+5", () => {
  const ui = read("app_ui.js"), html = read("app_ui.html");
  assert.match(html, /id="tab-ana"[^>]*data-tab="ana">Analyze</);
  assert.match(html, /<iframe id="f-ana" title="Analyze"/);
  assert.match(ui, /const anaUrl = s => new URL\("analyze", s\.url\)\.href;/);
  assert.match(ui, /"5": "ana"/);
});
