"""
websearch_check.py - try the labeling tool's Web search on picture files, without the page.

Uses the keys saved on this computer (%APPDATA%\\FaceLabels\\); they are never kept in the project, which is
public. Each picture is one Amazon and one Google search and counts toward their monthly limits.

  python tools\\websearch_check.py picture.jpg [more.jpg ...]
  python tools\\websearch_check.py picture.jpg --face 440,0,880,410     (face area x0,y0,x1,y1 for Amazon;
                                                                        default: the whole picture)
"""
import argparse
import io
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import websearch as W  # noqa: E402


def jpeg(im):
    buf = io.BytesIO()
    im.save(buf, "JPEG", quality=92)
    return buf.getvalue()


def main():
    ap = argparse.ArgumentParser(description="Try the Web search on picture files.")
    ap.add_argument("pictures", nargs="+")
    ap.add_argument("--face", help="x0,y0,x1,y1 of the face, sent to Amazon")
    a = ap.parse_args()
    from PIL import Image
    for p in a.pictures:
        im = Image.open(p).convert("RGB")
        face = im.crop(tuple(int(v) for v in a.face.split(","))) if a.face else im
        r = W.suggest(jpeg(face), jpeg(im))
        print(f"\n== {p}")
        for s in r["suggestions"]:
            print(f"  {s['name']}: {s['level']} confidence. " + ". ".join(s["why"]))
        if not r["suggestions"]:
            print("  No suggestions found. Checked: " + "; ".join(r["checked"]))
        g = r["details"].get("google")
        if g:
            print(f"  (Google: {g['match']} match, describes it as {g['guess'][:1]}, names {g['names'][:5]})")
        for e in r["errors"]:
            print("  error:", e)
    print("\nThis month:", {k: f"{u['used']} of {u['cap']}" for k, u in r["usage"].items()})


if __name__ == "__main__":
    main()
