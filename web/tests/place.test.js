// Tests for place.js: a device's location as one line of text, made only of what was checked.
//     node --test web/tests/
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const Place = require("../place.js");

const NOW = Date.UTC(2026, 10, 24, 15, 0, 0);
const good = { lat: 39.8017, lon: -89.6437, acc: 60.4, at: NOW - 120e3, source: "system", town: "Near Springfield, Illinois, United States", live: true };

test("a place is said in one line: the town, the numbers, how sure, how long ago", () => {
  assert.equal(Place.words(good, NOW), "Near Springfield, Illinois, United States · 39.80170, -89.64370 · within 60 m · 2 min ago");
  assert.equal(Place.words({ ...good, at: NOW - 20e3 }, NOW).split(" · ").at(-1), "just now");
  assert.equal(Place.words({ ...good, at: NOW - 3 * 3600e3 }, NOW).split(" · ").at(-1), "3 h ago");
  assert.match(Place.words({ ...good, at: NOW - 3 * 86400e3 }, NOW).split(" · ").at(-1), /^on \S/);
  // a coarse reading says so in kilometres; a town that isn't known is left out
  assert.equal(Place.words({ ...good, acc: 6410, town: "" }, NOW), "39.80170, -89.64370 · within 6.4 km · 2 min ago");
  // a place typed on the device has no uncertainty and no age
  assert.equal(Place.words({ ...good, acc: null, source: "hand" }, NOW), "Near Springfield, Illinois, United States · 39.80170, -89.64370 · typed on the device");
  // the newest row of a device's history: where it was last known to be, and since when
  const kept = Place.words({ ...good, live: false }, NOW).split(" · ");
  assert.match(kept.at(-1), /^last known, since \S/);
  assert.match(Place.words({ ...good, acc: null, source: "hand", live: false }, NOW), /typed on the device · last known, since /);
});

test("what is no place has no words, whoever said it", () => {
  for (const bad of [null, undefined, "here", 7, [], {}, { ...good, lat: 91 }, { ...good, lon: -181 }, { ...good, lat: "39.8" },
                     { ...good, lat: NaN }, { ...good, acc: -1 }, { ...good, acc: "near" }, { ...good, acc: 2e6 }, { ...good, acc: null },
                     { ...good, at: 0 }, { ...good, at: "now" }, { ...good, source: "guess" }, { ...good, source: undefined }]) {
    assert.equal(Place.clean(bad), null, JSON.stringify(bad));
    assert.equal(Place.words(bad, NOW), "");
  }
});

test("a town's name from anywhere is one line of text, and nothing else of the object is kept", () => {
  const p = Place.clean({ ...good, town: "  <b>Springfield</b>\n\u0007 Illinois " + "x".repeat(500), extra: "<script>", live: "yes" });
  assert.deepEqual(Object.keys(p), ["lat", "lon", "acc", "at", "source", "town", "live"]);
  assert.equal(p.town.length, 240);
  assert.ok(p.town.startsWith("<b>Springfield</b> Illinois x"), p.town);       // (text: the pages put it in as text)
  assert.equal(Place.clean({ ...good, town: 7 }).town, "");
  assert.equal(Place.clean({ ...good, live: false }).live, false);
  assert.equal(p.live, true);
});

test("the pages put a place into the page as text only, and the file brings nothing from anywhere", () => {
  const source = fs.readFileSync(path.join(__dirname, "..", "place.js"), "utf8");
  assert.doesNotMatch(source, /innerHTML|insertAdjacentHTML|outerHTML|document\.write|https?:\/\/|fetch\(/);
  for (const page of ["tracker_ui.js", "cameras_ui.js"]) {
    const js = fs.readFileSync(path.join(__dirname, "..", page), "utf8");
    assert.match(js, /Place\.words\(/, page);
    for (const m of js.matchAll(/^.*Place\.words\(.*$/gm)) assert.doesNotMatch(m[0], /innerHTML/, m[0]);
  }
});
