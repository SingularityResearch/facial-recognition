// The address the Microsoft Store downloads the Windows installer from.
//
// The Store lists the project's own installer by an address (docs/store/microsoft-store.md) and takes only one
// that answers with the file itself. GitHub answers a release's download address with a redirect to its storage,
// so the Store refuses it. This Cloudflare Worker stands in front: asked for
//     /facerecognizer/v<version>/FaceRecognizer-Setup-x64.exe
// it asks GitHub for that release's file, follows the redirect itself, and hands the bytes on as they come.
// Nothing is kept here and nothing is changed: the Store gets exactly the released, signed file, and a version's
// address always gives that version's file.
//
// It answers for nothing else: only this project's releases, only the files named below, only GET and HEAD.
// It reads nothing of the visitor's and keeps no log of its own.

const RELEASES = "https://github.com/SingularityResearch/facial-recognition/releases/download";
const FILES = new Set(["FaceRecognizer-Setup-x64.exe"]);
const PATH = /^\/facerecognizer\/(v\d{1,3}\.\d{1,3}\.\d{1,3})\/([A-Za-z0-9._-]{1,80})$/;

export default {
  async fetch(request) {
    if (request.method !== "GET" && request.method !== "HEAD") {
      return new Response("Method not allowed\n", { status: 405, headers: { Allow: "GET, HEAD" } });
    }
    const asked = PATH.exec(new URL(request.url).pathname);
    if (!asked || !FILES.has(asked[2])) return new Response("Not found\n", { status: 404 });
    const [, version, file] = asked;

    // a part of the file may be asked for (a download that goes on where it stopped): passed on as it is
    const ask = new Headers();
    const range = request.headers.get("Range");
    if (range) ask.set("Range", range);
    const from = await fetch(`${RELEASES}/${version}/${file}`, { headers: ask, redirect: "follow" });
    if (from.status !== 200 && from.status !== 206) {
      await from.body?.cancel();
      return new Response("Not found\n", { status: from.status === 404 ? 404 : 502 });
    }

    const headers = new Headers({
      "Content-Type": "application/octet-stream",
      "Content-Disposition": `attachment; filename="${file}"`,
      "Accept-Ranges": "bytes",
      // a version's file never changes
      "Cache-Control": "public, max-age=86400, immutable",
      "X-Content-Type-Options": "nosniff",
    });
    for (const name of ["Content-Length", "Content-Range", "ETag", "Last-Modified"]) {
      const value = from.headers.get(name);
      if (value) headers.set(name, value);
    }
    if (request.method === "HEAD") {
      await from.body?.cancel();
      return new Response(null, { status: from.status, headers });
    }
    return new Response(from.body, { status: from.status, headers });
  },
};
