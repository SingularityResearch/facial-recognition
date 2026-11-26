// analyze.js - the Analyze page (analyze.html, served by the labeling tool): the owner's questions about everything the
// devices read, typed or spoken, answered by the language model of Describe photo with the server's tools (analyze.rs).
//
// The page sends a question with the conversation before it (kept in this page's memory only: nothing of it is stored,
// in the browser or anywhere), the Signals chart's catalog of every signal (signals_chart.js) and the owner's time zone,
// then follows the answer as the server works it out: each step, each chart, then the words. Everything that came from
// the records (an answer's words, a step, a chart's title or a series' name) is put into the page as text, never as
// markup: an answer's Markdown is read here into elements (paragraphs, lists, bold, code), never parsed as HTML.
// The charts are drawn here from the numbers (SVG, the page's own colours), with the values on hover and as a table.
// The pure parts (reading Markdown, the charts' scales, ticks and gaps) run in Node too, for the tests.
(function (root) {
  "use strict";

  // ---------------------------------------------------------------- Markdown, as far as an answer needs it
  // Blocks: paragraphs, "- " / "* " / "1. " lists, "#" headings; inside them **bold**, *italic*, `code`. Anything else
  // is text as it is. [{type: "p"|"ul"|"ol"|"h", items?: [spans], spans?: spans}], a span [kind, text].
  function spans(line) {
    const out = [], re = /(\*\*[^*]+\*\*|`[^`]+`|\*[^*\s][^*]*\*|_[^_\s][^_]*_)/g;
    let at = 0, m;
    while ((m = re.exec(line))) {
      if (m.index > at) out.push(["t", line.slice(at, m.index)]);
      const s = m[0];
      if (s.startsWith("**")) out.push(["b", s.slice(2, -2)]);
      else if (s.startsWith("`")) out.push(["c", s.slice(1, -1)]);
      else out.push(["i", s.slice(1, -1)]);
      at = m.index + s.length;
    }
    if (at < line.length) out.push(["t", line.slice(at)]);
    return out;
  }
  function blocks(text) {
    const out = [];
    let para = [];
    const flush = () => { if (para.length) { out.push({ type: "p", spans: spans(para.join(" ")) }); para = []; } };
    for (const raw of String(text || "").replace(/\r/g, "").split("\n")) {
      const line = raw.trim();
      const bullet = /^[-*•]\s+(.*)$/.exec(line), num = /^\d+[.)]\s+(.*)$/.exec(line), head = /^#{1,6}\s+(.*)$/.exec(line);
      if (!line) { flush(); continue; }
      if (head) { flush(); out.push({ type: "h", spans: spans(head[1]) }); continue; }
      if (bullet || num) {
        flush();
        const type = bullet ? "ul" : "ol", last = out[out.length - 1];
        const item = spans((bullet || num)[1]);
        if (last && last.type === type) last.items.push(item); else out.push({ type, items: [item] });
        continue;
      }
      if (/^\|.*\|$/.test(line) && /^\|[\s:|-]+\|$/.test(line)) continue;   // a table's rule line: dropped
      para.push(line.replace(/^\|\s*|\s*\|$/g, "").replace(/\s*\|\s*/g, " · "));
    }
    flush();
    return out;
  }

  // ---------------------------------------------------------------- the charts' arithmetic
  // Round numbers for an axis from lo to hi: about `n` of them, steps of 1, 2 or 5 times a power of ten.
  function ticks(lo, hi, n = 4) {
    if (!isFinite(lo) || !isFinite(hi)) return [];
    if (hi === lo) { hi = lo + 1; lo = lo - 1; }
    const raw = (hi - lo) / n, p = Math.pow(10, Math.floor(Math.log10(raw)));
    const step = [1, 2, 5, 10].map(m => m * p).find(s => s >= raw) || 10 * p;
    const out = [];
    for (let v = Math.ceil(lo / step) * step; v <= hi + step * 1e-9; v += step) out.push(+v.toFixed(10));
    return out;
  }
  // The y range of a chart's series: their lowest and highest (band included), with a little room.
  function yRange(series) {
    let lo = Infinity, hi = -Infinity;
    for (const s of series) for (const p of s.points) for (const v of p.slice(1)) if (typeof v === "number" && isFinite(v)) { lo = Math.min(lo, v); hi = Math.max(hi, v); }
    if (lo === Infinity) return [0, 1];
    const pad = (hi - lo) * 0.08 || Math.abs(hi) * 0.1 || 1;
    return [lo >= 0 && lo - pad < 0 ? 0 : lo - pad, hi + pad];
  }
  // A series' points cut where two are further apart than `gap` (nobody in view between): runs of points.
  function runs(points, gap) {
    const out = [];
    let cur = [];
    for (const p of points) {
      if (cur.length && p[0] - cur[cur.length - 1][0] > gap) { out.push(cur); cur = []; }
      cur.push(p);
    }
    if (cur.length) out.push(cur);
    return out;
  }
  // Where the axis's time labels go: about five, at whole hours, days or months as the span asks.
  function timeTicks(from, to) {
    const span = to - from, H = 3600e3, D = 24 * H;
    const steps = [5 * 60e3, 15 * 60e3, H, 3 * H, 6 * H, 12 * H, D, 2 * D, 7 * D, 14 * D, 30 * D, 91 * D, 365 * D];
    const step = steps.find(s => span / s <= 6) || 365 * D;
    const out = [];
    const off = new Date(from).getTimezoneOffset() * 60e3;
    for (let t = Math.ceil((from - off) / step) * step + off; t <= to; t += step) out.push(t);
    return { ticks: out, step };
  }
  function fmtTime(t, step) {
    const d = new Date(t);
    if (step < 24 * 3600e3) return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    if (step < 28 * 24 * 3600e3) return d.toLocaleDateString([], { month: "short", day: "numeric" });
    return d.toLocaleDateString([], { year: "numeric", month: "short" });
  }
  // A number as few digits say it: a count stays whole ("23", not "23.0"), and no zero ends a fraction.
  function fmtNum(v) {
    if (typeof v !== "number" || !isFinite(v)) return "–";
    const a = Math.abs(v);
    let s = a >= 100 ? v.toFixed(0) : a >= 10 ? v.toFixed(1) : a >= 1 ? v.toFixed(2) : v.toFixed(3);
    if (s.includes(".")) s = s.replace(/\.?0+$/, "");
    return s === "-0" ? "0" : s;
  }
  // A gallery's pictures in the order chosen: "relevance" is the order the model's question gave (best match first),
  // "newest" and "oldest" by each picture's date, the undated after the dated in both.
  function ordered(photos, how) {
    const list = photos.slice();
    if (how !== "newest" && how !== "oldest") return list;
    const sign = how === "newest" ? -1 : 1;
    return list.map((p, i) => [p, i]).sort(([a, i], [b, j]) => {
      const ta = typeof a.taken === "number" ? a.taken : null, tb = typeof b.taken === "number" ? b.taken : null;
      if (ta === null || tb === null) return ta === tb ? i - j : ta === null ? 1 : -1;
      return ta === tb ? i - j : sign * (ta - tb);
    }).map(([p]) => p);
  }
  // The time one bar stands for: the shortest step between two of the times, else the chart's bucket (seconds).
  function barSlot(times, bucket) {
    let g = Infinity;
    for (let k = 1; k < times.length; k++) if (times[k] > times[k - 1]) g = Math.min(g, times[k] - times[k - 1]);
    return isFinite(g) ? g : Math.max(1, (bucket || 86400) * 1000);
  }

  const api = { spans, blocks, ticks, yRange, runs, timeTicks, fmtNum, barSlot, ordered };
  if (typeof module === "object" && module.exports) { module.exports = api; return; }
  root.AnalyzePage = api;
  if (typeof document === "undefined") return;

  // ---------------------------------------------------------------- the page
  const $ = s => document.querySelector(s);
  const el = (tag, props = {}, ...kids) => { const n = Object.assign(document.createElement(tag), props); n.append(...kids); return n; };
  const NS = "http://www.w3.org/2000/svg";
  const svg = (tag, attrs = {}) => { const n = document.createElementNS(NS, tag); for (const [k, v] of Object.entries(attrs)) n.setAttribute(k, v); return n; };
  const drop = r => { try { r?.body?.cancel().catch(() => {}); } catch {} return r; };
  async function get(path) {
    const r = await fetch(path, { cache: "no-store", headers: { "X-Face-Tool": "1" } });
    if (!r.ok) { const t = await r.text().catch(() => ""); throw new Error(t.slice(0, 300) || r.statusText); }
    return r.json();
  }
  async function post(path, body) {
    const r = await fetch(path, { method: "POST", cache: "no-store", headers: { "X-Face-Tool": "1", "Content-Type": "application/json" }, body: JSON.stringify(body || {}) });
    const j = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(j.error || r.statusText);
    return j;
  }

  let conversation = [];        // [{role, content}]: this page's memory, nothing else
  let current = null;           // the answer being worked out: {id}
  let status = {};

  function render(text) {
    const box = document.createDocumentFragment();
    const inline = (parent, list) => {
      for (const [k, t] of list) parent.append(k === "t" ? document.createTextNode(t) : el(k === "b" ? "strong" : k === "c" ? "code" : "em", { textContent: t }));
      return parent;
    };
    for (const b of blocks(text)) {
      if (b.type === "p") box.append(inline(el("p"), b.spans));
      else if (b.type === "h") box.append(inline(el("h3"), b.spans));
      else box.append(el(b.type, {}, ...b.items.map(i => inline(el("li"), i))));
    }
    return box;
  }

  // ------------------------------------------------ a chart
  const COLORS = ["--s1", "--s2", "--s3", "--s4", "--s5"];
  const color = i => `var(${COLORS[i % COLORS.length]})`;
  function chart(c) {
    const fig = el("figure", { className: "chart" });
    const cap = el("figcaption", {}, document.createTextNode(c.title || "Chart"));
    if (c.unit) cap.append(el("small", { textContent: c.unit }));
    fig.append(cap);
    const series = (c.series || []).filter(s => Array.isArray(s.points) && s.points.length).slice(0, 8);
    if (series.length > 1) fig.append(el("div", { className: "legend" }, ...series.map((s, i) => el("span", {}, el("i", { style: `background:${color(i)}` }), document.createTextNode(s.name)))));
    const W = 860, H = 220, L = 48, R = 12, T = 10, B = 26;
    const plot = svg("svg", { class: "plot", viewBox: `0 0 ${W} ${H}`, preserveAspectRatio: "none", role: "img", "aria-label": c.title || "chart" });
    const all = series.flatMap(s => s.points.map(p => p[0]));
    const times = [...new Set(all)].sort((a, b) => a - b);
    if (!times.length) { fig.append(el("p", { textContent: "Nothing to draw." })); return fig; }
    // bars: each stands for the time between two of them, so the axis has half of that before the first and after the last
    const slot = c.kind === "bars" ? barSlot(times, c.bucket) : 0;
    const from = c.kind === "bars" ? times[0] - slot / 2 : (c.from ?? times[0]), to = c.kind === "bars" ? times[times.length - 1] + slot / 2 : (c.to ?? times[times.length - 1]);
    const span = Math.max(1, to - from);
    const [lo0, hi0] = c.kind === "bars" ? [0, Math.max(1, ...series.flatMap(s => s.points.map(p => p[1])))] : yRange(series);
    const yt = ticks(lo0, hi0), lo = Math.min(lo0, yt[0] ?? lo0), hi = Math.max(hi0, yt[yt.length - 1] ?? hi0);
    const x = t => L + (W - L - R) * (t - from) / span;
    const y = v => T + (H - T - B) * (1 - (v - lo) / ((hi - lo) || 1));
    for (const v of yt) {
      plot.append(svg("line", { class: "grid", x1: L, x2: W - R, y1: y(v), y2: y(v) }));
      const lab = svg("text", { x: L - 6, y: y(v) + 4, "text-anchor": "end" }); lab.textContent = fmtNum(v); plot.append(lab);
    }
    const tt = timeTicks(from, to);
    for (const t of tt.ticks) {
      const lab = svg("text", { x: x(t), y: H - 6, "text-anchor": "middle" }); lab.textContent = fmtTime(t, tt.step); plot.append(lab);
    }
    const gap = Math.max((c.bucket || 60) * 1000 * 2.5, 1);
    series.forEach((s, i) => {
      if (c.kind === "bars") {
        // four fifths of a slot for the bars of one time, side by side when there are several lines
        const whole = Math.max(2, (W - L - R) * slot / span * 0.8), w = Math.max(1, whole / series.length);
        for (const p of s.points) {
          const h = Math.max(1, y(0) - y(p[1]));
          plot.append(svg("rect", { x: x(p[0]) - whole / 2 + i * w, y: y(p[1]), width: w, height: h, rx: Math.min(2, w / 2), fill: color(i) }));
        }
        return;
      }
      for (const run of runs(s.points, gap)) {
        // the lowest to highest of each stretch, faint; the average, a line
        if (run.some(p => p[2] != null && p[3] != null && p[2] !== p[3])) {
          const top = run.map(p => `${x(p[0])},${y(p[3] ?? p[1])}`), bottom = run.slice().reverse().map(p => `${x(p[0])},${y(p[2] ?? p[1])}`);
          plot.append(svg("polygon", { points: [...top, ...bottom].join(" "), fill: color(i), "fill-opacity": 0.14, stroke: "none" }));
        }
        if (run.length === 1) plot.append(svg("circle", { cx: x(run[0][0]), cy: y(run[0][1]), r: 3, fill: color(i) }));
        else plot.append(svg("polyline", { points: run.map(p => `${x(p[0])},${y(p[1])}`).join(" "), fill: "none", stroke: color(i), "stroke-width": 2, "stroke-linejoin": "round", "vector-effect": "non-scaling-stroke" }));
      }
    });
    // hover: the nearest time's values of every series
    const guide = svg("line", { x1: 0, x2: 0, y1: T, y2: H - B, stroke: "var(--muted)", "stroke-width": 1, visibility: "hidden", "vector-effect": "non-scaling-stroke" });
    plot.append(guide);
    const tip = el("div", { className: "tip", hidden: true });
    const hit = svg("rect", { x: L, y: T, width: W - L - R, height: H - T - B, fill: "transparent" });
    plot.append(hit);
    hit.addEventListener("mousemove", e => {
      const box = plot.getBoundingClientRect();
      const t = from + ((e.clientX - box.left) / box.width * W - L) / (W - L - R) * span;
      let best = times[0];
      for (const u of times) if (Math.abs(u - t) < Math.abs(best - t)) best = u;
      guide.setAttribute("x1", x(best)); guide.setAttribute("x2", x(best)); guide.setAttribute("visibility", "visible");
      const head = el("b", { textContent: new Date(best).toLocaleString([], { dateStyle: "medium", timeStyle: c.kind === "bars" ? undefined : "short" }) });
      const lines = series.map((s, i) => {
        const p = s.points.find(q => q[0] === best);
        if (!p) return null;
        const more = p[2] != null && p[3] != null && p[2] !== p[3] ? ` (${fmtNum(p[2])} to ${fmtNum(p[3])})` : "";
        return el("div", {}, el("i", { style: `display:inline-block;width:8px;height:8px;border-radius:2px;margin-right:5px;background:${color(i)}` }),
                  document.createTextNode(`${series.length > 1 ? s.name + ": " : ""}${fmtNum(p[1])}${c.unit && c.unit !== "photos" ? " " + c.unit : ""}${more}`));
      }).filter(Boolean);
      tip.replaceChildren(head, ...lines);
      tip.hidden = false;
      const fx = e.clientX - fig.getBoundingClientRect().left;
      tip.style.left = Math.min(Math.max(8, fx + 12), fig.clientWidth - tip.offsetWidth - 8) + "px";
      tip.style.top = "36px";
    });
    hit.addEventListener("mouseleave", () => { tip.hidden = true; guide.setAttribute("visibility", "hidden"); });
    fig.append(plot, tip);
    // the numbers, as a table
    const rows = series.flatMap(s => s.points.map(p => [p[0], s.name, p[1], p[2], p[3]])).sort((a, b) => a[0] - b[0]);
    const table = el("table", {}, el("thead", {}, el("tr", {}, ...["When", series.length > 1 ? "Of" : null, "Value", c.kind === "bars" ? null : "Lowest", c.kind === "bars" ? null : "Highest"].filter(Boolean).map(h => el("th", { textContent: h })))));
    const tb = el("tbody");
    for (const r of rows.slice(-400)) {
      tb.append(el("tr", {}, ...[new Date(r[0]).toLocaleString([], { dateStyle: "short", timeStyle: "short" }), series.length > 1 ? r[1] : null,
                                 fmtNum(r[2]), c.kind === "bars" ? null : fmtNum(r[3]), c.kind === "bars" ? null : fmtNum(r[4])].filter(v => v !== null).map(v => el("td", { textContent: v }))));
    }
    table.append(tb);
    fig.append(el("details", { className: "numbers" }, el("summary", { textContent: `Numbers (${rows.length})` }), el("div", { className: "tablewrap" }, table)));
    return fig;
  }

  // ------------------------------------------------ pictures found: a gallery, and one picture with its details
  // A page of pictures at a time (the owner's word: "show X number of images at a time - then go back and forward in
  // the list", for speed): only the page shown is in the page, so only its pictures are fetched. How many a page is
  // remembered (with the labeling page's own settings).
  const SIZES = [12, 24, 48, 96], SIZE_KEY = "fl-anaPageSize";
  const pageSize = () => { const n = +(root.PageStore?.get(SIZE_KEY) || 24); return SIZES.includes(n) ? n : 24; };
  // The pictures' order (the owner: "sort by relevance or sort by time ... relevance as default - but keep whatever
  // sort the user selects going forward"): remembered, and every gallery on the page follows a change.
  const SORTS = [["relevance", "Most relevant first"], ["newest", "Newest first"], ["oldest", "Oldest first"]], SORT_KEY = "fl-anaSort";
  const sortOf = () => { const v = root.PageStore?.get(SORT_KEY); return SORTS.some(s => s[0] === v) ? v : "relevance"; };
  function setSort(v) {
    if (!SORTS.some(s => s[0] === v)) return;
    try { root.PageStore?.set(SORT_KEY, v); } catch {}
    document.dispatchEvent(new CustomEvent("anasort"));
    SettingsBridge.changed();
  }
  const photoUrl = (p, size) => `/api/analyze/photo?id=${encodeURIComponent(p.photo)}${p.frame != null ? "&frame=" + encodeURIComponent(p.frame) : ""}${size ? "&size=" + size : ""}`;
  const dateOf = t => t == null ? "no date" : new Date(t).toLocaleDateString([], { year: "numeric", month: "short", day: "numeric" });
  function gallery(g) {
    const given = Array.isArray(g.photos) ? g.photos : [];
    let photos = ordered(given, sortOf());
    const box = el("section", { className: "gallery" });
    const sorts = el("select", { ariaLabel: "Order of the pictures" }, ...SORTS.map(([v, t]) => el("option", { value: v, textContent: t })));
    sorts.value = sortOf();
    sorts.hidden = given.length < 2;
    sorts.addEventListener("change", () => setSort(sorts.value));
    const follow = () => {
      if (!box.isConnected && box.dataset.shown) { document.removeEventListener("anasort", follow); return; }
      if (sorts.value !== sortOf()) sorts.value = sortOf();
      photos = ordered(given, sortOf()); page = 0; draw();
    };
    document.addEventListener("anasort", follow);
    requestAnimationFrame(() => { box.dataset.shown = "1"; });
    box.append(el("h4", {}, document.createTextNode(g.title || "Pictures"),
      el("small", { textContent: `${given.length}${g.more ? "+" : ""} ${given.length === 1 ? "picture" : "pictures"}` }), sorts));
    const grid = el("div", { className: "grid" });
    let size = pageSize(), page = 0;
    const btn = (text, label) => el("button", { className: "btn small", type: "button", textContent: text, ariaLabel: label, title: label });
    const first = btn("«", "First page"), prev = btn("‹", "Previous page"), next = btn("›", "Next page"), last = btn("»", "Last page");
    const where = el("span", { className: "hint" });
    const sizes = el("select", { ariaLabel: "Pictures a page" }, ...SIZES.map(n => el("option", { value: String(n), textContent: `${n} a page` })));
    sizes.value = String(size);
    const pages = () => Math.max(1, Math.ceil(photos.length / size));
    const draw = () => {
      page = Math.min(Math.max(0, page), pages() - 1);
      const from = page * size, shown = photos.slice(from, from + size);
      grid.replaceChildren(...shown.map((p, i) => {
        const b = el("button", { className: "pic", type: "button", title: `${dateOf(p.taken)}${(p.people || []).length ? " · " + p.people.join(", ") : ""}` },
          el("img", { src: photoUrl(p, "thumb"), alt: `A picture of ${(p.people || []).join(", ") || "the library"}, ${dateOf(p.taken)}`, decoding: "async" }),
          el("span", { textContent: dateOf(p.taken) }));
        if (p.video) b.append(el("i", { className: "vid", textContent: "video" }));
        // (the picture view steps through the whole list, and takes the grid's page along)
        b.addEventListener("click", () => openPicture(photos, from + i, at => { if (Math.floor(at / size) !== page) { page = Math.floor(at / size); draw(); } }));
        return b;
      }));
      where.textContent = photos.length ? `${from + 1}–${from + shown.length} of ${photos.length}${g.more ? "+" : ""}` : "none";
      first.disabled = prev.disabled = page === 0;
      next.disabled = last.disabled = page >= pages() - 1;
      pager.hidden = photos.length <= SIZES[0];
    };
    const pager = el("div", { className: "pager" }, first, prev, where, next, last, sizes);
    first.addEventListener("click", () => { page = 0; draw(); });
    prev.addEventListener("click", () => { page--; draw(); });
    next.addEventListener("click", () => { page++; draw(); });
    last.addEventListener("click", () => { page = pages() - 1; draw(); });
    sizes.addEventListener("change", () => {
      const top = page * size;                       // (the first picture shown stays on the page)
      size = +sizes.value; page = Math.floor(top / size);
      try { root.PageStore?.set(SIZE_KEY, String(size)); } catch {}
      draw();
    });
    box.append(grid, pager);
    draw();
    return box;
  }
  let viewing = { list: [], at: 0, moved: null };
  async function openPicture(list, at, moved = viewing.moved) {
    viewing = { list, at, moved };
    moved?.(at);
    const p = list[at], d = $("#view");
    $("#viewImg").src = photoUrl(p, "view");
    $("#viewImg").alt = `A picture of ${(p.people || []).join(", ") || "the library"}`;
    $("#viewPrev").disabled = at <= 0;
    $("#viewNext").disabled = at >= list.length - 1;
    const info = $("#viewInfo");
    info.replaceChildren(el("h3", { textContent: dateOf(p.taken) }), el("div", { className: "hint", textContent: "Reading what is known of it…" }));
    if (!d.open) d.showModal();
    let x = {};
    try { x = await get(`/api/analyze/picture?id=${encodeURIComponent(p.photo)}`); } catch (e) { x = { error: String(e.message || e) }; }
    if (viewing.list !== list || viewing.at !== at) return;          // another picture was opened meanwhile
    const parts = [el("h3", { textContent: x.taken != null ? new Date(x.taken).toLocaleString([], { dateStyle: "long", timeStyle: "short" }) : dateOf(p.taken) })];
    if (x.error) parts.push(el("div", { className: "err", textContent: x.error }));
    const named = [...new Set((x.faces || []).map(f => f.name).filter(Boolean))];
    const unnamed = (x.faces || []).filter(f => !f.name).length;
    parts.push(el("div", { className: "tags" }, ...(named.length ? named : p.people || []).map(n => el("span", { className: "tag", textContent: n })),
      ...(unnamed ? [el("span", { className: "hint", textContent: `and ${unnamed} ${unnamed === 1 ? "face" : "faces"} not named` })] : [])));
    if (x.video || p.video) parts.push(el("div", { className: "hint", textContent: "A video: shown is the frame where they were found." }));
    if (x.text) parts.push(el("div", {}, el("div", { className: "hint", textContent: "Its description" }), el("div", { textContent: x.text })));
    for (const r of x.rows || []) {
      const d2 = r.data || {}, kv = el("div", { className: "kv" });
      const looks = d2.looks && typeof d2.looks === "object" ? Object.entries(d2.looks).slice(0, 3).map(([k, v]) => `${k} ${Math.round(v * 100)}%`).join(", ") : "";
      const pairs = [["looked", looks], ["picture quality", d2.quality], ["face size", d2.px != null ? d2.px + " px" : null],
                     ["faces in the picture", d2.faces], ["of them named", d2.named], ["its time from", d2.time === "file" ? "the file (no camera date)" : d2.time ? "the camera" : null]];
      for (const [k, v] of pairs) if (v !== undefined && v !== null && v !== "") kv.append(el("span", { textContent: k }), el("span", { textContent: String(v) }));
      parts.push(el("div", {}, el("div", { style: "font-weight:600", textContent: r.name || "" }), kv));
    }
    if (!(x.rows || []).length && !x.error) parts.push(el("div", { className: "hint", textContent: "No readings were worked out of this picture yet (Settings, History and logs, The library's history: Work out the history from the library)." }));
    info.replaceChildren(...parts);
  }
  $("#viewClose").addEventListener("click", () => $("#view").close());
  $("#viewPrev").addEventListener("click", () => { if (viewing.at > 0) openPicture(viewing.list, viewing.at - 1); });
  $("#viewNext").addEventListener("click", () => { if (viewing.at < viewing.list.length - 1) openPicture(viewing.list, viewing.at + 1); });
  $("#view").addEventListener("keydown", e => {
    if (e.key === "ArrowLeft") $("#viewPrev").click();
    if (e.key === "ArrowRight") $("#viewNext").click();
  });
  $("#view").addEventListener("click", e => { if (e.target === $("#view")) $("#view").close(); });

  // ------------------------------------------------ the conversation
  const talk = $("#talk");
  function scroll() { const m = $("main"); m.scrollTop = m.scrollHeight; }
  // (no example questions: the owner's word, 2026-11-25)
  function empty() {
    talk.replaceChildren(el("div", { className: "empty" }, el("h2", { textContent: "Ask about anyone or any signal" }),
      el("div", { textContent: "Now or over time: faces, voices, bodies, sound, the devices, places, music and photos." })));
  }
  function userMessage(text) {
    if (talk.querySelector(".empty")) talk.replaceChildren();
    talk.append(el("div", { className: "msg user" }, el("div", { className: "bubble", textContent: text })));
  }
  function answerBox() {
    const box = el("div", { className: "msg bot" });
    const working = el("div", { className: "working" }, el("span", { className: "dot" }), el("span", { textContent: "Thinking…" }));
    const steps = el("details", { className: "steps" }, el("summary", { textContent: "How it was found" }), el("ol"));
    steps.hidden = true;
    const charts = el("div", { style: "display:grid;gap:10px" });
    const pictures = el("div", { style: "display:grid;gap:10px" });
    const bubble = el("div", { className: "bubble" }); bubble.hidden = true;
    const meta = el("div", { className: "meta" });
    box.append(working, bubble, pictures, charts, steps, meta);
    talk.append(box);
    return { box, working, steps, charts, pictures, bubble, meta, shown: new Set(), galleries: new Set() };
  }
  function show(view, j) {
    const list = view.steps.querySelector("ol");
    for (let i = list.children.length; i < (j.steps || []).length; i++) list.append(el("li", { textContent: j.steps[i].text }));
    view.steps.hidden = !list.children.length;
    view.steps.querySelector("summary").textContent = `How it was found (${list.children.length} ${list.children.length === 1 ? "step" : "steps"})`;
    for (const g of j.galleries || []) if (!view.galleries.has(g.id)) { view.galleries.add(g.id); view.pictures.append(gallery(g)); }
    for (const c of j.charts || []) if (!view.shown.has(c.id)) { view.shown.add(c.id); view.charts.append(chart(c)); }
    const last = (j.steps || [])[j.steps.length - 1];
    view.working.lastChild.textContent = last ? last.text + "…" : (j.server ? `Asking the model on ${j.server}…` : "Thinking…");
    if (j.answer) { view.bubble.replaceChildren(render(j.answer)); view.bubble.hidden = false; }
    if (j.error) { view.bubble.replaceChildren(el("div", { className: "err", textContent: j.error })); view.bubble.hidden = false; }
    if (j.done) {
      view.working.remove();
      view.meta.textContent = [j.model, j.server && `on ${j.server}`, j.secs && `${j.secs} s`].filter(Boolean).join(" · ");
    }
    scroll();
  }
  function busy(on) {
    $("#send").textContent = on ? "Stop" : "Ask";
    $("#send").classList.toggle("primary", !on);
    $("#q").disabled = false;
  }
  async function ask() {
    if (current) { post("/api/analyze/stop", { id: current.id }).catch(() => {}); return; }
    const text = $("#q").value.trim();
    if (!text) return;
    $("#q").value = ""; fit();
    userMessage(text);
    const view = answerBox();
    scroll();
    let tz = "", offset = 0;
    try { tz = Intl.DateTimeFormat().resolvedOptions().timeZone || ""; offset = -new Date().getTimezoneOffset(); } catch {}
    let catalog = {};
    try { catalog = root.SignalsChart ? root.SignalsChart.catalog() : {}; } catch {}
    try {
      const { id } = await post("/api/analyze/ask", { question: text, history: conversation, catalog, tz, offset });
      current = { id }; busy(true);
      let after = 0, j = {};
      for (;;) {
        j = await get(`/api/analyze/job?id=${encodeURIComponent(id)}&after=${after}&wait=20`);
        after = j.seq || after;
        show(view, j);
        if (j.done) break;
      }
      conversation.push({ role: "user", content: text });
      if (j.answer) conversation.push({ role: "assistant", content: j.answer });
    } catch (e) {
      show(view, { error: String(e.message || e), done: true });
    } finally {
      current = null; busy(false); $("#q").focus();
    }
  }

  // ------------------------------------------------ speaking a question
  let listening = false;
  async function mic() {
    const b = $("#mic");
    if (!status.speech?.ready) {
      const mb = Math.round((status.speech?.bytes || 0) / 1e6);
      if (!status.speech?.can) { await askUser(status.speech?.why || "Speech isn't available on this computer.", "OK"); return; }
      if (await askUser(`Speaking a question needs the speech model, downloaded once (${mb} MB, from Hugging Face, checked by SHA-256). It runs on this computer: what you say is never sent anywhere.`, "Download")) {
        await post("/api/analyze/speech/get").catch(e => askUser(String(e.message || e)));
        refresh();
      }
      return;
    }
    if (!listening) {
      try { await post("/api/analyze/listen", { do: "start" }); } catch (e) { await askUser(String(e.message || e)); return; }
      listening = true; b.classList.add("on"); b.setAttribute("aria-label", "Stop and use what was said");
      $("#q").placeholder = "Listening… click the microphone again when you're done";
      return;
    }
    listening = false; b.classList.remove("on"); b.setAttribute("aria-label", "Speak a question");
    $("#q").placeholder = "Making it into words…";
    try {
      const r = await post("/api/analyze/listen", { do: "stop" });
      if (r.text) { $("#q").value = ($("#q").value ? $("#q").value + " " : "") + r.text; fit(); $("#q").focus(); }
    } catch (e) { await askUser(String(e.message || e)); }
    $("#q").placeholder = "Ask about anyone or any signal: now, or over time";
  }

  // ------------------------------------------------ settings and status
  async function refresh() {
    try { status = await get("/api/analyze/status"); } catch (e) { $("#where").textContent = "Couldn't ask this computer's server: " + (e.message || e); return; }
    const short = status.loaded_with && status.loaded_with < (status.context || 16384)
      ? ` · loaded with a context of ${status.loaded_with}: Analyze needs ${status.context}, press Load it` : "";
    const w = status.ok
      ? `${status.model || "a model"} on ${status.where === "this computer" ? "this computer" : status.where}${short}${status.history ? "" : " · no shared database: only what the devices read now"}`
      : (status.why || "No language model answers: set up the model server (Settings, AI models), or pick another computer's under Where Analyze runs.");
    $("#where").textContent = w;
    // the model servers of your other computers, as found (each with the kind it is and its loaded model)
    const servers = $("#anaServer"), chosenServer = status.server || "";
    const listed = [["", "The model server (the one Describe photo uses)"], ...(status.candidates || []).map(c => [c.url, `${c.name}: ${c.kind}${(c.loaded || [])[0] ? " (" + c.loaded[0] + " loaded)" : ""}`])];
    if (chosenServer && !listed.some(o => o[0] === chosenServer)) listed.push([chosenServer, `${status.where} (not answering now)`]);
    servers.replaceChildren(...listed.map(([v, t]) => el("option", { value: v, textContent: t })));
    servers.value = chosenServer;
    const sel = $("#anaModel"), keep = status.picked || "";
    const opts = [["", `Automatic (${status.model && !status.picked ? status.model : status.server ? "the server's loaded model" : "the describer's"})`], ...(status.models || []).map(m => [m, m])];
    if (keep && !opts.some(o => o[0] === keep)) opts.push([keep, keep]);
    sel.replaceChildren(...opts.map(([v, t]) => el("option", { value: v, textContent: t })));
    sel.value = keep;
    $("#anaThink").checked = !!status.think;
    const s = status.suggestion;
    $("#suggest").textContent = s ? `Suggested for this computer: ${s.name}${s.there ? "" : " (not on the server yet: get it first)"}, with a context of ${s.context}. ${s.command || ""}` : "";
    const l = status.loading;
    $("#loadSaid").textContent = l ? (l.running ? "Loading…" : l.said) : "";
    const sp = status.speech || {};
    $("#speechSaid").textContent = sp.ready ? `Ready (${sp.name || "on this computer"}).` : sp.can === false ? (sp.why || "Not on this computer.") : `Not downloaded yet (${Math.round((sp.bytes || 0) / 1e6)} MB).`;
    $("#speechGet").hidden = !!sp.ready || sp.can === false || !!sp.getting;
    if (sp.getting) $("#speechSaid").textContent = `Downloading: ${Math.round((sp.done || 0) / 1e6)} of ${Math.round((sp.bytes || 0) / 1e6)} MB`;
    if (talk.querySelector(".empty") || !talk.children.length) empty();
    SettingsBridge.changed();
    if (l?.running || sp.getting) setTimeout(refresh, 1500);
  }
  function fit() { const q = $("#q"); q.style.height = "auto"; q.style.height = Math.min(q.scrollHeight + 2, innerHeight * 0.3) + "px"; }

  $("#form").addEventListener("submit", e => { e.preventDefault(); ask(); });
  $("#q").addEventListener("keydown", e => { if (e.key === "Enter" && !e.shiftKey && !e.isComposing) { e.preventDefault(); ask(); } });
  $("#q").addEventListener("input", fit);
  $("#mic").addEventListener("click", mic);
  $("#newChat").addEventListener("click", () => { if (current) return; conversation = []; empty(); $("#q").focus(); });
  $("#setupBtn").addEventListener("click", () => { const s = $("#setup"); s.hidden = !s.hidden; $("#setupBtn").setAttribute("aria-expanded", String(!s.hidden)); });
  $("#anaServer").addEventListener("change", e => post("/api/analyze/config", { server: e.target.value }).then(refresh).catch(err => { $("#where").textContent = String(err.message || err); }));
  $("#anaModel").addEventListener("change", e => post("/api/analyze/config", { model: e.target.value }).then(refresh).catch(() => {}));
  $("#anaThink").addEventListener("change", e => post("/api/analyze/config", { think: e.target.checked }).then(refresh).catch(() => {}));
  $("#anaLoad").addEventListener("click", () => post("/api/analyze/load").then(refresh).catch(e => { $("#loadSaid").textContent = String(e.message || e); }));
  $("#speechGet").addEventListener("click", () => post("/api/analyze/speech/get").then(refresh).catch(e => { $("#speechSaid").textContent = String(e.message || e); }));

  const A = "Analyze";
  SettingsBridge.init("analyze", [
    SettingsBridge.control("anaServer", A, { label: "Where Analyze runs", hint: "The model server above, the one Describe photo uses, or one of your other computers' model servers (that keeps this computer's processor and graphics for its camera). Each question, and what it reads of your records, goes to that computer." }),
    SettingsBridge.control("anaModel", A, { label: "Model for answering", hint: "The language model that answers questions. Automatic: the one Describe photo uses, on the same server." }),
    SettingsBridge.control("anaThink", A, { label: "Let the model think first", hint: "Slower, sometimes better for questions that compare many things." }),
    { section: A, id: "anaSort", label: "Pictures in an answer", type: "select", options: () => SORTS, def: () => "relevance",
      get: () => sortOf(), set: v => setSort(v), hint: "Most relevant first: the order the answer ranked them. Or by each picture's date." },
    { section: A, id: "anaLoadIt", label: "Load the model for Analyze", type: "button", button: () => "Load it",
      hint: () => `Loads the model with the longer context Analyze needs (${status.context || 16384} tokens): LM Studio on this computer, Ollama anywhere.` },
    { section: A, id: "anaWhere", label: "Answers come from", type: "info", get: () => $("#where").textContent },
  ], { anaLoadIt: () => post("/api/analyze/load").then(refresh) });

  // (this device's remembered options from the server first, as the labeling page takes them)
  if (root.PageStore) PageStore.restore("label", "fl-");
  empty();
  refresh();
  setInterval(() => { if (!$("#setup").hidden) refresh(); }, 10000);
})(typeof window !== "undefined" ? window : globalThis);
