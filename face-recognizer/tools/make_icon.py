"""
make_icon.py - draws the app's icon and writes every file made from it into icons/ (kept in git, so builds need
neither this script nor anything it uses):

    icons/icon.ico        Windows: FaceRecognizer.exe, the installer, the pages' favicon (16 to 256 px)
    icons/wizard.bmp      Windows: the picture in the corner of the installer's pages
    icons/icon.icns       macOS: FaceRecognizer.app (16 to 1024 px, with the margin macOS icons keep)
    icons/icon.png        Linux: the app window and the launcher (512 px); icon-256.png, icon-64.png beside it

The icon: a face as a faceted mask inside a camera's corner marks, the left half solid, the right half only its
mesh: a face being read. It is drawn here from a few dozen points (no picture of anyone, no artwork from elsewhere),
in the app's own greens. Small sizes are drawn again with fewer, heavier marks, not shrunk.

    python tools/make_icon.py              -> writes icons/
    python tools/make_icon.py --sheet x.png   also a sheet of every size on dark and light, to look at
"""
import argparse
import math
from pathlib import Path

import numpy as np
from PIL import Image, ImageChops, ImageDraw, ImageFilter

OUT = Path(__file__).resolve().parent.parent / "icons"
ICO_SIZES = (16, 20, 24, 32, 40, 48, 64, 128, 256)
ICNS_SIZES = (16, 32, 64, 128, 256, 512, 1024)

# Half a face, seen from the front: (name, u, v, lift). u runs from the middle of the face to its right edge, v from
# the top of the head (-1) to the chin (1); lift moves the point towards the viewer (nose) or away (eye sockets), on
# top of the head's round shape. The left half is the mirror image.
MIDDLE = [("top", 0, -1.00, 0), ("forehead", 0, -0.62, 0), ("brows", 0, -0.31, .07), ("bridge", 0, -0.04, .14),
          ("tip", 0, 0.21, .40), ("under", 0, 0.33, .16), ("lip", 0, 0.47, .12), ("mouth", 0, 0.545, .04),
          ("lip2", 0, 0.63, .10), ("chin", 0, 0.82, .10), ("jaw", 0, 1.00, 0)]
SIDE = [("skull", .36, -0.95, 0), ("crown", .62, -0.72, 0), ("temple", .74, -0.38, 0), ("ear", .745, -0.02, 0),
        ("jaw1", .68, 0.30, 0), ("jaw2", .54, 0.62, 0), ("jaw3", .30, 0.88, 0),
        ("head", .30, -0.66, 0), ("brow2", .52, -0.36, .05), ("brow1", .22, -0.37, .08),
        ("eye1", .15, -0.17, -.10), ("eye2", .33, -0.255, -.06), ("eye3", .53, -0.165, -.10), ("eye4", .34, -0.085, -.08),
        ("cheek", .52, 0.08, .06), ("cheek2", .29, 0.11, .03), ("nose1", .09, 0.02, .08), ("nose2", .165, 0.27, .14),
        ("corner", .27, 0.545, 0), ("peak", .11, 0.455, .10), ("low", .13, 0.635, .07),
        ("fold", .40, 0.38, .02), ("chin2", .22, 0.76, .06)]
EYE = {"eye1", "eye2", "eye3", "eye4"}
LIGHT = np.array([0.40, -0.44, 0.80])                # from the upper right, in front: the solid half turns away from it
RAMP = [(0.00, (8, 62, 50)), (0.30, (14, 116, 91)), (0.58, (40, 190, 138)), (0.80, (110, 231, 183)), (1.00, (214, 252, 233))]
MINT, PALE, DARK = (110, 231, 183), (209, 250, 229), (6, 14, 12)


def mesh():
    """The face's points [(name, u, v, z)] and its triangles (index triples), by Delaunay over the points."""
    import cv2
    pts = [(n, u, v, lift) for n, u, v, lift in MIDDLE]
    for n, u, v, lift in SIDE:
        pts += [(n + "R", u, v, lift), (n + "L", -u, v, lift)]
    full = []
    for n, u, v, lift in pts:
        round_ = math.sqrt(max(0.0, 1 - (u / 0.86) ** 2 - (v / 1.12) ** 2)) * 0.62
        full.append((n, u, v, round_ + lift))
    sub = cv2.Subdiv2D((-2, -2, 4, 4))
    where = {}
    for i, (_, u, v, _) in enumerate(full):
        sub.insert((float(u), float(v)))
        where[(round(u, 4), round(v, 4))] = i
    tris = set()
    for t in sub.getTriangleList():
        key = [where.get((round(float(t[k]), 4), round(float(t[k + 1]), 4))) for k in (0, 2, 4)]
        if None not in key:
            tris.add(tuple(sorted(key)))
    return full, sorted(tris)


def ramp(t):
    t = min(1.0, max(0.0, t))
    for (a, ca), (b, cb) in zip(RAMP, RAMP[1:]):
        if t <= b:
            k = (t - a) / (b - a)
            return tuple(round(ca[i] + (cb[i] - ca[i]) * k) for i in range(3))
    return RAMP[-1][1]


def draw(size, margin=0.03, corner=0.225):
    """The icon at size x size pixels (RGBA). margin: empty room around the rounded square, as a share of the size;
    corner: how round its corners are (0: a plain square)."""
    k = max(2, min(8, 2048 // size))                 # drawn k times larger, then reduced: smooth edges
    n = size * k
    small, tiny = size <= 48, size <= 24

    # the rounded square: dark green to near black, a soft light behind the face
    y, x = np.mgrid[0:n, 0:n] / n
    t = np.clip((x * 0.55 + y) / 1.55, 0, 1)[..., None]
    back = np.array([20, 58, 48]) * (1 - t) + np.array([5, 11, 10]) * t
    glow = np.exp(-(((x - 0.5) / 0.36) ** 2 + ((y - 0.47) / 0.40) ** 2))[..., None]
    back = back + (np.array([52, 211, 153]) - back) * glow * 0.20
    img = Image.fromarray(np.dstack([back, np.full((n, n), 255)]).astype(np.uint8), "RGBA")
    m = margin * n
    shape = Image.new("L", (n, n), 0)
    ImageDraw.Draw(shape).rounded_rectangle([m, m, n - 1 - m, n - 1 - m], radius=(n - 2 * m) * corner, fill=255)

    full, tris = mesh()
    side = n - 2 * m
    cx, cy, half = n / 2, m + side * 0.505, side * (0.355 if not tiny else 0.37)
    P = [(cx + u * half * 0.98, cy + v * half) for _, u, v, _ in full]
    names = [p[0] for p in full]
    pts3 = np.array([(u, v, z * 1.9) for _, u, v, z in full])       # deeper than a real face: clearer facets

    def layer():
        return Image.new("RGBA", (n, n), (0, 0, 0, 0))

    def put(lay):
        nonlocal img
        img = Image.alpha_composite(img, lay)

    # the solid half (left) and a faint fill under the mesh half
    solid = layer()
    d = ImageDraw.Draw(solid)
    edges_solid, edges_wire = set(), set()
    for a, b, c in tris:
        us = [full[i][1] for i in (a, b, c)]
        eye = all(names[i][:-1] in EYE for i in (a, b, c))
        right = min(us) >= 0 and max(us) > 0
        nrm = np.cross(pts3[b] - pts3[a], pts3[c] - pts3[a])
        nrm = nrm / (np.linalg.norm(nrm) or 1)
        if nrm[2] < 0:
            nrm = -nrm
        lit = 0.36 + 0.80 * max(0.0, float(nrm @ (LIGHT / np.linalg.norm(LIGHT)))) ** 1.5
        tri = [P[a], P[b], P[c]]
        for e in ((a, b), (b, c), (a, c)):
            (edges_wire if right else edges_solid).add((min(e), max(e)))
        if right:
            d.polygon(tri, fill=(*ramp(lit * 0.55), 34 if not eye else 0))
        else:
            d.polygon(tri, fill=(*(DARK if eye else ramp(lit)), 255))
    put(solid)

    # facet edges on the solid half (thin, dark), then the mesh half: lines and points of light
    w = max(1.0, n * (0.0050 if not small else 0.0))
    if w and not small:
        lines = layer()
        d = ImageDraw.Draw(lines)
        for a, b in edges_solid - edges_wire:
            d.line([P[a], P[b]], fill=(6, 40, 32, 150), width=round(w))
        put(lines)
    wire = layer()
    d = ImageDraw.Draw(wire)
    lw = n * (0.0105 if not small else 0.020 if not tiny else 0.030)
    for a, b in edges_wire:
        mid = full[a][1] == 0 and full[b][1] == 0
        if tiny and not mid and not (names[a][:-1] in EYE and names[b][:-1] in EYE) \
                and not ({names[a][:-1], names[b][:-1]} <= {"skull", "crown", "temple", "ear", "jaw1", "jaw2", "jaw3", "top", "jaw"}):
            continue                                 # the smallest sizes: the outline, the eye, the middle line
        d.line([P[a], P[b]], fill=(*(PALE if mid else MINT), 255 if mid else 215), width=max(1, round(lw * (1.25 if mid else 1))))
    if not small:
        r = n * 0.0125
        for i, (_, u, _, _) in enumerate(full):
            if u >= 0:
                d.ellipse([P[i][0] - r, P[i][1] - r, P[i][0] + r, P[i][1] + r], fill=(*PALE, 255))
    if not tiny:                                     # the eye of the mesh half: a point of light
        ex = sum(P[names.index(e + "R")][0] for e in EYE) / 4
        ey = sum(P[names.index(e + "R")][1] for e in EYE) / 4
        r = n * (0.021 if not small else 0.030)
        d.ellipse([ex - r, ey - r, ex + r, ey + r], fill=(255, 255, 255, 255))
        ex2 = sum(P[names.index(e + "L")][0] for e in EYE) / 4
        r2 = n * 0.012
        if not small:
            d.ellipse([ex2 - r2 - n * .006, ey - r2 - n * .004, ex2 + r2 - n * .006, ey + r2 - n * .004], fill=(*MINT, 255))
    if not small:
        halo = wire.filter(ImageFilter.GaussianBlur(n * 0.012))
        put(ImageChops.multiply(halo, Image.new("RGBA", (n, n), (255, 255, 255, 150))))
    put(wire)

    # the camera's corner marks
    marks = layer()
    d = ImageDraw.Draw(marks)
    inset, arm = side * (0.135 if not tiny else 0.12), side * (0.150 if not tiny else 0.17)
    bw = round(n * (0.034 if not small else 0.052 if not tiny else 0.075))
    for sx in (-1, 1):
        for sy in (-1, 1):
            px = (m + inset) if sx < 0 else (n - m - inset)
            py = (m + inset) if sy < 0 else (n - m - inset)
            path = [(px - sx * arm, py), (px, py), (px, py - sy * arm)]
            d.line(path, fill=(*MINT, 255), width=bw, joint="curve")
            for qx, qy in path:
                d.ellipse([qx - bw / 2, qy - bw / 2, qx + bw / 2, qy + bw / 2], fill=(*MINT, 255))
    if not small:
        put(ImageChops.multiply(marks.filter(ImageFilter.GaussianBlur(n * 0.02)), Image.new("RGBA", (n, n), (255, 255, 255, 120))))
    put(marks)

    # a thin light rim, then the rounded square's shape
    if not tiny:
        rim = layer()
        ImageDraw.Draw(rim).rounded_rectangle([m, m, n - 1 - m, n - 1 - m], radius=side * corner,
                                              outline=(110, 231, 183, 70), width=max(1, round(n * 0.006)))
        put(rim)
    img.putalpha(ImageChops.multiply(img.getchannel("A"), shape))
    return img.resize((size, size), Image.LANCZOS)


def write(out=OUT):
    out.mkdir(parents=True, exist_ok=True)
    ico = [draw(s) for s in ICO_SIZES]
    ico[-1].save(out / "icon.ico", format="ICO", sizes=[(s, s) for s in ICO_SIZES], append_images=ico[:-1])
    icns = [draw(s, margin=0.085) for s in ICNS_SIZES]          # macOS icons keep room around their shape
    icns[-1].save(out / "icon.icns", format="ICNS", append_images=icns[:-1])
    for size, name in ((512, "icon.png"), (256, "icon-256.png"), (64, "icon-64.png")):
        draw(size).save(out / name, optimize=True)
    # the Windows installer's corner picture, at its largest size: a plain square (a .bmp's see-through corners
    # showed black there)
    draw(140, margin=0.0, corner=0.0).convert("RGB").resize((138, 140), Image.LANCZOS).save(out / "wizard.bmp")
    return sorted(p.name for p in out.iterdir())


def sheet(path):
    """Every size, on a dark and a light desktop, to judge the small ones."""
    sizes = (256, 128, 64, 48, 32, 24, 20, 16)
    W = sum(sizes) + 24 * (len(sizes) + 1)
    img = Image.new("RGB", (W, 2 * 304), (24, 26, 30))
    img.paste((236, 238, 240), (0, 304, W, 608))
    for row in (0, 1):
        x = 24
        for s in sizes:
            ic = draw(s)
            img.paste(ic, (x, row * 304 + 24 + (256 - s) // 2), ic)
            x += s + 24
    img.save(path)


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description="Draw the app icon and write icons/")
    ap.add_argument("--sheet", help="also write a sheet of every size to this file")
    ap.add_argument("--only-sheet", action="store_true", help="don't write icons/")
    a = ap.parse_args()
    if not a.only_sheet:
        print("icons/:", ", ".join(write()))
    if a.sheet:
        sheet(a.sheet)
        print("sheet:", a.sheet)
