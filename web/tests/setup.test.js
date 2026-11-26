// The setup page (setup.html, setup.js): it asks its own server alone, puts what came from it into the page as text
// only, changes nothing but at a button the user presses, talks only to the app window around it, and says what an
// update changed on a device that had the app before.
const test = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");
const path = require("node:path");
const read = f => fs.readFileSync(path.join(__dirname, "..", f), "utf8");

test("the setup page asks its own server alone and puts what came from it in as text only", () => {
  const js = read("setup.js"), html = read("setup.html");
  assert.doesNotMatch(js, /innerHTML|outerHTML|insertAdjacentHTML|document\.write|eval\(|new Function/);
  assert.doesNotMatch(js, /https?:\/\/(?!\(127)/, "no other address (the parent's own check aside)");
  for (const asked of js.matchAll(/(?<![.\w])(?:get|post|maybe)\("([^"]+)"/g)) assert.match(asked[1], /^\/api\//, asked[1]);
  // every route it uses is one this server answers
  const routes = ["/api/summary", "/api/db", "/api/describe/status", "/api/hardware", "/api/analyze/servers", "/api/analyze/status",
                  "/api/lmstudio", "/api/websearch/key", "/api/music", "/api/setup", "/api/db/passphrase", "/api/db/certificates",
                  "/api/db/setup", "/api/describe/config", "/api/describe/getmodel", "/api/describe/load", "/api/analyze/config",
                  "/api/analyze/speech/get", "/api/lmstudio/keep"];
  for (const r of routes) assert.ok(js.includes(`"${r}"`), r);
  assert.doesNotMatch(html, /\son[a-z]+=/i, "no inline handlers");
  assert.doesNotMatch(html, /<script(?![^>]*\ssrc=)/i, "no inline script");
  assert.match(html, /<script src="\/setup\.js"><\/script>/);
  // secrets are typed into password fields and cleared after the button
  assert.match(js, /type: f\.secret \? "password" : "text"/);
});

test("it talks only to the app window around it, and the window takes only what it expects from it", () => {
  const js = read("setup.js"), app = read("app_ui.js");
  assert.match(js, /location\.ancestorOrigins\?\.\[0\]/);
  assert.match(js, /\/\^http:\\\/\\\/\(127\\\.0\\\.0\\\.1\|localhost\)\(:\\d\+\)\?\$\/\.test\(o\)/);
  assert.match(js, /window\.parent\.postMessage\(\{ fs: "setup", \.\.\.m \}, parentOrigin\)/);
  // the window: only from the setup frame, at its own address; a tab of three, a group's name, one of two folders
  assert.match(app, /frame\("setup"\)\.contentWindow === e\.source && originOf\("setup"\) === e\.origin && e\.data\?\.fs === "setup"/);
  assert.match(app, /\["track", "label", "set"\]\.includes\(m\.go\)/);
  assert.match(app, /m\.open === "private" \|\| m\.open === "data"/);
  assert.match(app, /\/\^\[a-z\]\+\$\/\.test\(m\.group\)/);
  // it is served by the labeling tool and carried in the program
  assert.match(read("../crates/face-server/src/label.rs"), /\("\/setup", "setup\.html", HTML\),\s*\("\/setup\.js", "setup\.js", SCRIPT\)/);
  const carried = read("../crates/face-server/src/pagefiles.rs");
  assert.ok(carried.includes('"setup.html"') && carried.includes('"setup.js"'));
});

test("it changes nothing by itself: every change is a button, and what is set up is only said", () => {
  const js = read("setup.js");
  // every POST sits in a click (an action's or a form's run, or a choice)
  const posts = [...js.matchAll(/post\("\/api\/[^"]+"/g)].length;
  const inButtons = [...js.matchAll(/(?:action\([^\n]*|form\([\s\S]*?\n[^\n]*)post\("\/api\//g)].length;
  assert.ok(posts > 0 && inButtons === posts, `${inButtons} of ${posts} changes sit behind a button`);
  // shown by itself only for a new copy with nothing set up, or an update whose changes weren't seen
  assert.match(js, /const fresh = !\(summary\.photos > 0\) && !db\.config && store\.get\("fl-setupClosed"\) !== "1";/);
  assert.match(js, /if \(fresh \|\| \(updated\(f\) && unseen\(f\)\.length\)\) tell\(\{ want: true \}\);/);
  // what an update changed is said only where the app was there before
  assert.match(js, /const updated = f => !!f\.version && !f\.version\.fresh;/);
  assert.match(js, /function changes\(\) \{\s*if \(!updated\(found\)\) return null;/);
});

test("the version note tells a new copy from an update before the app writes anything of its own", () => {
  const app = read("../crates/face-server/src/app.rs");
  for (const start of ["pub fn start(", "pub fn start_asking("]) {
    const body = app.slice(app.indexOf(start), app.indexOf(start) + 1500);
    assert.ok(body.indexOf("versionnote::note(home)") >= 0 && body.indexOf("versionnote::note(home)") < body.indexOf("take_place(home)"), start);
  }
});
