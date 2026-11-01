"""
websearch.py - the labeling tool's "Web search" (only when you click it): suggests who a face is.

Two US services, English results only (owner's rule), each optional and each capped under $10 a month:
  Amazon Rekognition  celebrity recognition on the face: a well-known person's name with Amazon's match
                      confidence (%). Never names private people.
  Google Cloud Vision web detection on the whole photo: whether this exact picture is on web pages, and
                      the names those pages give. Only English text and US-style domains are kept.
suggest() combines them into suggestions with a confidence level, or none.

The face goes to Amazon and the photo to Google; both may keep them. Nothing else in this project sends
anything off the computer. Keys live outside the project (it is public), in %APPDATA%\\FaceLabels\\:
  google_vision_key.txt     (or the GOOGLE_VISION_API_KEY environment variable)
  aws_rekognition.json      {"access_key_id", "secret_access_key", "region"} of an AWS user that may only
                            call rekognition:RecognizeCelebrities
"""
import base64
import http.client
import json
import os
import re
import time
import urllib.parse
from pathlib import Path

UA = "FaceLabels/1.0 (local photo labeling tool)"
TIMEOUT = 30
HOME = Path(os.environ.get("APPDATA", Path.home())) / "FaceLabels"
KEY_FILE = HOME / "google_vision_key.txt"
AWS_FILE = HOME / "aws_rekognition.json"
# searches per calendar month, so each bill stays under the owner's $10/month (2026-11-01 prices):
CAPS = {"vision": 3700,          # 1,000 free + 2,700 at $3.50 per 1,000 = $9.45
        "rekognition": 9000}     # $1.00 per 1,000 images = $9.00
US_TLDS = {"com", "org", "net", "edu", "gov", "mil", "us"}
ENGLISH = re.compile(r"^[A-Za-z0-9\s.,:;!?'\"&()/+#%@$*_\-–—’“”|]+$")
PERSON = re.compile(r"^[A-Z][a-z'.]+(?:[ -](?:[A-Z][a-z'.]+|[A-Z]\.)){1,3}$")   # "Javier Bardem", "Mary J. Blige"


# --------------------------------------------------------------------------- keys and limits
def key():
    k = os.environ.get("GOOGLE_VISION_API_KEY", "").strip()
    if not k and KEY_FILE.exists():
        k = KEY_FILE.read_text("utf-8").strip()
    return k or None


def save_key(k):
    k = (k or "").strip()
    if k and not re.fullmatch(r"[A-Za-z0-9_\-]{20,80}", k):
        raise ValueError("That doesn't look like a Google API key")
    if k:
        KEY_FILE.parent.mkdir(parents=True, exist_ok=True)
        KEY_FILE.write_text(k, "utf-8")
    elif KEY_FILE.exists():
        KEY_FILE.unlink()


def aws():
    try:
        c = json.loads(AWS_FILE.read_text("utf-8-sig"))
        return c if c.get("access_key_id") and c.get("secret_access_key") else None
    except (OSError, ValueError):
        return None


def services():
    return {"vision": bool(key()), "rekognition": bool(aws())}


def usage(service="vision"):
    """{'month': 'YYYY-MM', 'used': n, 'cap': CAPS[service]} for this calendar month."""
    month = time.strftime("%Y-%m")
    try:
        u = json.loads((HOME / f"{service}_usage.json").read_text("utf-8"))
    except (OSError, ValueError):
        u = {}
    return {"month": month, "used": int(u.get("used", 0)) if u.get("month") == month else 0, "cap": CAPS[service]}


def _count(service):
    """Count a search before sending it (a failed call may still be billed); refuse past the cap."""
    u = usage(service)
    if u["used"] >= u["cap"]:
        raise ValueError(f"{service} limit reached: {u['cap']} searches this month (keeps the bill under $10)")
    HOME.mkdir(parents=True, exist_ok=True)
    (HOME / f"{service}_usage.json").write_text(json.dumps({"month": u["month"], "used": u["used"] + 1}), "utf-8")


def english(s):
    return bool(s) and bool(ENGLISH.match(s))


def us_site(url):
    host = urllib.parse.urlparse(url if "//" in url else "https://" + url).hostname or ""
    return host.rsplit(".", 1)[-1].lower() in US_TLDS


# --------------------------------------------------------------------------- Amazon Rekognition
def rekognition(jpeg):
    """Celebrities in the picture: [{name, confidence %, links}], best first."""
    c = aws()
    if not c:
        raise ValueError("No Amazon Rekognition key saved")
    import boto3
    _count("rekognition")
    rk = boto3.client("rekognition", aws_access_key_id=c["access_key_id"], aws_secret_access_key=c["secret_access_key"],
                      region_name=c.get("region", "us-east-1"))
    faces = rk.recognize_celebrities(Image={"Bytes": jpeg}).get("CelebrityFaces", [])
    out = [{"name": f["Name"], "confidence": round(float(f.get("MatchConfidence", 0)), 1),
            "links": ["https://" + u for u in f.get("Urls", []) if us_site(u)]}
           for f in faces if english(f.get("Name", ""))]
    return sorted(out, key=lambda f: -f["confidence"])


# --------------------------------------------------------------------------- Google Cloud Vision
def vision(jpeg, api_key=None):
    """Google Cloud Vision Web Detection, described by describe()."""
    api_key = api_key or key()
    if not api_key:
        raise ValueError("No Cloud Vision key saved")
    _count("vision")
    req = {"requests": [{"image": {"content": base64.b64encode(jpeg).decode()},
                         "features": [{"type": "WEB_DETECTION", "maxResults": 30}]}]}
    c = http.client.HTTPSConnection("vision.googleapis.com", timeout=TIMEOUT)
    try:
        c.request("POST", "/v1/images:annotate?key=" + urllib.parse.quote(api_key), json.dumps(req),
                  {"Content-Type": "application/json", "User-Agent": UA})
        r = c.getresponse()
        data = json.loads(r.read() or b"{}")
    finally:
        c.close()
    if r.status != 200:
        raise ValueError("Cloud Vision: " + str(data.get("error", {}).get("message", f"answer {r.status}")))
    return describe((data.get("responses") or [{}])[0].get("webDetection", {}))


def describe(w):
    """Cloud Vision web detection, English and US sites only.
      match   'exact'   this same picture is on web pages: the names those pages give are a real clue
              'partial' pages share part of the picture: usually just a similar scene (on a meme,
                        2026-11-01: 9-12 such pages, none with the photo), so not used for suggestions
              'similar' only different pictures that look alike;  'none' nothing
      names   web entities in Google's order. Their scores are unbounded relevance weights, not
              confidence (a song titled "Disc-Jockey" gave its artist 10.72 for someone else's picture).
      pages   [{title, url, domain, match: exact|partial}], exact copies first"""
    pages_all = [p for p in w.get("pagesWithMatchingImages", []) if str(p.get("url", "")).startswith("http")]
    kind = lambda p: "exact" if p.get("fullMatchingImages") else "partial"
    pages = [{"title": re.sub(r"<[^>]+>", "", p.get("pageTitle", "")).strip()[:160], "url": p["url"],
              "domain": urllib.parse.urlparse(p["url"]).hostname or "", "match": kind(p)}
             for p in pages_all if us_site(p["url"])]
    pages = sorted((p for p in pages if not p["title"] or english(p["title"])), key=lambda p: p["match"] != "exact")
    full = len(w.get("fullMatchingImages", []))
    exact_pages = sum(1 for p in pages_all if kind(p) == "exact")
    match = ("exact" if full or exact_pages else "partial" if w.get("partialMatchingImages") or pages_all
             else "similar" if w.get("visuallySimilarImages") else "none")
    seen, names = set(), []
    for e in sorted(w.get("webEntities", []), key=lambda e: -e.get("score", 0)):
        n = e.get("description", "").strip()
        if english(n) and n.lower() not in seen:
            seen.add(n.lower())
            names.append(n)
    guess = [g["label"] for g in w.get("bestGuessLabels", []) if english(g.get("label", ""))]
    return {"match": match, "guess": guess[:1], "names": names[:8], "pages": pages[:8], "exactPages": exact_pages}


# --------------------------------------------------------------------------- combined
LEVELS = ["very high", "high", "medium", "low"]


def level(pct):
    """Words for Amazon's match confidence. Display bands, not measured on this project's faces."""
    return "high" if pct >= 95 else "medium" if pct >= 85 else "low"


def suggest(face_jpeg, photo_jpeg):
    """Ask every service that has a key, and combine: {'suggestions': [...], 'checked': [...], 'details', 'errors', 'usage'}.
    A suggestion is {name, level, confidence (Amazon %, or None), why: [reasons], links}:
      Amazon names a celebrity            -> its confidence: high >= 95%, medium >= 85%, else low
      Google finds this exact picture     -> person-like names from those pages: medium
      both name the same person           -> very high"""
    out = {"suggestions": [], "checked": [], "details": {}, "errors": [], "usage": {}}
    have = services()
    if not any(have.values()):
        raise ValueError("No web search service is set up (see Settings in this dialog)")
    by = {}
    if have["rekognition"]:
        out["checked"].append("Amazon celebrity recognition (the face)")
        try:
            celebs = rekognition(face_jpeg)
            out["details"]["amazon"] = celebs
            for c in celebs[:2]:
                by[c["name"].lower()] = {"name": c["name"], "level": level(c["confidence"]), "confidence": c["confidence"],
                                         "why": [f"Amazon recognizes {c['name']} ({c['confidence']:.1f}% match)"], "links": c["links"]}
        except Exception as e:           # one service failing mustn't hide the other's answer
            out["errors"].append(f"Amazon: {e}")
        out["usage"]["rekognition"] = usage("rekognition")
    if have["vision"]:
        out["checked"].append("Google pages with this picture (the whole photo)")
        try:
            v = vision(photo_jpeg)
            out["details"]["google"] = v
            if v["match"] == "exact":
                reason = f"Google finds this exact picture on {v['exactPages'] or 'some'} web page{'s' if v['exactPages'] != 1 else ''} naming {{}}"
                for n in [n for n in v["names"] if PERSON.match(n)][:2]:
                    s = by.get(n.lower())
                    if s:
                        s["level"] = "very high"
                        s["why"].append(reason.format(n))
                    else:
                        by[n.lower()] = {"name": n, "level": "medium", "confidence": None, "why": [reason.format(n)],
                                         "links": [p["url"] for p in v["pages"] if p["match"] == "exact"][:2]}
        except Exception as e:
            out["errors"].append(f"Google: {e}")
        out["usage"]["vision"] = usage("vision")
    out["suggestions"] = sorted(by.values(), key=lambda s: (LEVELS.index(s["level"]), -(s["confidence"] or 0)))
    return out
