#!/usr/bin/env python3
"""Generate the Linh app icon family from one shared geometry.

The mark is a transit-map route: one incoming line forks into two lines and the
chosen line is lit in the accent colour, ending in a destination dot. Every asset (master, macOS
plate, tray symbolic, Windows tiles, size previews) is derived from the same
centre-line definition so the family stays consistent.

Requires PyGObject with Rsvg 2.0 and pycairo.
Usage: python3 scripts/generate-icon-assets.py [variant]
Variants: indigo-coral (default, soft depth) and blue-green (flat, no shadow).
The shipped app icon is blue-green; indigo-coral remains an alternative design.
"""

import math
import sys
from pathlib import Path

import cairo
import gi

gi.require_version("Rsvg", "2.0")
from gi.repository import Rsvg  # noqa: E402

# Each variant: output dir, master colours (a tuple is a top-to-bottom gradient),
# optional depth lip, and the macOS plate treatment.
VARIANTS = {
    "indigo-coral": {
        "out": "design/icon",
        "dim": ("#7477F8", "#4E51DE"),
        "accent": ("#FF8A6B", "#F2533A"),
        "lip": ("#3436A8", "#B8361F", 22),
        "plate": ("#7477F8", "#4E51DE"),
        "mac_dim": ("#FFFFFF", "#E4E5FF"),
        "mac_accent": ("#FF8A6B", "#F2533A"),
        "mac_lip": ("#3A3DB4", "#B8361F", 16),
        "plate_shadow": True,
    },
    "blue-green": {
        "out": "design/icon/blue-green",
        "dim": "#2563EB",
        "accent": "#10A35C",
        "lip": None,
        "plate": "#2563EB",
        "mac_dim": "#FFFFFF",
        "mac_accent": "#4ADE80",
        "mac_lip": None,
        "plate_shadow": False,
    },
}
VARIANT_NAME = sys.argv[1] if len(sys.argv) > 1 else "indigo-coral"
V = VARIANTS[VARIANT_NAME]
OUT = Path(V["out"])

# Transit-map construction: the incoming line runs left to right and splits at a
# junction into two lines with 45 degree bends. The chosen line is lit in the
# accent colour and ends in a destination dot.
STROKE = 120
DOT_R = 100
JUNCTION = (360, 512)
ROUTES = {
    "stem": [(130, 512), (360, 512)],
    "chosen": [(360, 512), (640, 232), (860, 232)],
    "down": [(360, 512), (640, 792), (880, 792)],
}
DOT = (860, 232)


def d_of(points):
    return "M" + " L".join(f"{x:g} {y:g}" for x, y in points)


def make_transform(box):
    """Uniform scale + centre so the mark (strokes and dot included) fits a box x box square."""
    pts = [p for line in ROUTES.values() for p in line]
    r = STROKE / 2
    minx = min(min(p[0] for p in pts) - r, DOT[0] - DOT_R)
    maxx = max(max(p[0] for p in pts) + r, DOT[0] + DOT_R)
    miny = min(min(p[1] for p in pts) - r, DOT[1] - DOT_R)
    maxy = max(max(p[1] for p in pts) + r, DOT[1] + DOT_R)
    k = box / max(maxx - minx, maxy - miny)
    e = 512 - k * (minx + maxx) / 2
    f = 512 - k * (miny + maxy) / 2
    return (k, 0, 0, k, e, f), k


def apply(m, p):
    a, b, c, d, e, f = m
    return (a * p[0] + c * p[1] + e, b * p[0] + d * p[1] + f)


def invert(m, p):
    a, b, c, d, e, f = m
    det = a * d - b * c
    x, y = p[0] - e, p[1] - f
    return ((d * x - c * y) / det, (-b * x + a * y) / det)


def mark(m, face_dim, face_accent, lip_dim, lip_accent, lip, gid):
    """Layered route: lip (depth) underneath, then faces. Chosen line sits under the stem."""
    mat = "matrix(%s)" % " ".join(f"{v:.4f}" for v in m)
    g1 = invert(m, (512, 96))
    g2 = invert(m, (512, 928))
    grad = lambda name, top, bottom: (
        f'<linearGradient id="{gid}-{name}" gradientUnits="userSpaceOnUse" '
        f'x1="{g1[0]:.1f}" y1="{g1[1]:.1f}" x2="{g2[0]:.1f}" y2="{g2[1]:.1f}">'
        f'<stop offset="0" stop-color="{top}"/><stop offset="1" stop-color="{bottom}"/></linearGradient>'
    )
    defs = ""
    if isinstance(face_dim, tuple):
        defs += grad("dim", *face_dim)
        face_dim = f"url(#{gid}-dim)"
    if isinstance(face_accent, tuple):
        defs += grad("acc", *face_accent)
        face_accent = f"url(#{gid}-acc)"

    def layer(dim, acc):
        st = f'fill="none" stroke-width="{STROKE}" stroke-linecap="round" stroke-linejoin="round"'
        return (
            f'<path d="{d_of(ROUTES["chosen"])}" stroke="{acc}" {st}/>'
            f'<circle cx="{DOT[0]}" cy="{DOT[1]}" r="{DOT_R}" fill="{acc}"/>'
            f'<path d="{d_of(ROUTES["down"])}" stroke="{dim}" {st}/>'
            f'<path d="{d_of(ROUTES["stem"])}" stroke="{dim}" {st}/>'
        )

    out = f"<defs>{defs}</defs>" if defs else ""
    if lip:
        out += f'<g transform="translate(0 {lip})"><g transform="{mat}">{layer(lip_dim, lip_accent)}</g></g>'
    out += f'<g transform="{mat}">{layer(face_dim, face_accent)}</g>'
    return out


def svg(body, size=1024):
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" '
        f'viewBox="0 0 {size} {size}">{body}</svg>\n'
    )


def squircle(x0, y0, size, n=5.0, steps=360):
    r = size / 2
    cx, cy = x0 + r, y0 + r
    pts = []
    for i in range(steps):
        t = 2 * math.pi * i / steps
        ct, st = math.cos(t), math.sin(t)
        pts.append((cx + r * math.copysign(abs(ct) ** (2 / n), ct), cy + r * math.copysign(abs(st) ** (2 / n), st)))
    return "M" + " L".join(f"{x:.2f} {y:.2f}" for x, y in pts) + " Z"


def master_svg():
    m, _ = make_transform(860)
    lip_dim, lip_acc, lip = V["lip"] or (None, None, 0)
    return svg(mark(m, V["dim"], V["accent"], lip_dim, lip_acc, lip, "m"))


def macos_svg():
    # Apple Big Sur grid: 824pt plate centred on a 1024 canvas (100pt margins).
    m, _ = make_transform(560)
    plate = squircle(100, 100, 824)
    defs, fill, filt = "", V["plate"], ""
    if isinstance(fill, tuple):
        defs += (
            '<linearGradient id="plate" x1="0" y1="0" x2="0" y2="1">'
            f'<stop offset="0" stop-color="{fill[0]}"/><stop offset="1" stop-color="{fill[1]}"/></linearGradient>'
        )
        fill = "url(#plate)"
    if V["plate_shadow"]:
        defs += (
            '<filter id="shadow" x="-20%" y="-20%" width="140%" height="150%">'
            '<feGaussianBlur in="SourceAlpha" stdDeviation="14"/><feOffset dy="12"/>'
            '<feComponentTransfer><feFuncA type="linear" slope="0.30"/></feComponentTransfer>'
            '<feMerge><feMergeNode/><feMergeNode in="SourceGraphic"/></feMerge></filter>'
        )
        filt = ' filter="url(#shadow)"'
    lip_dim, lip_acc, lip = V["mac_lip"] or (None, None, 0)
    body = (
        (f"<defs>{defs}</defs>" if defs else "")
        + f'<path d="{plate}" fill="{fill}"{filt}/>'
        + mark(m, V["mac_dim"], V["mac_accent"], lip_dim, lip_acc, lip, "mac")
    )
    return svg(body)


def tray_svg(colour):
    """16px symbolic: each segment is a filled rounded rect (no strokes, so GTK and
    macOS template recolouring work), one colour, chosen line ends in a dot."""
    m, _ = make_transform(14 * 64)  # 14px of the 16px grid, 1px margin
    q = lambda p: tuple(round(v / 64 * 2) / 2 for v in apply(m, p))
    w = 2.0
    parts = []
    for line in ROUTES.values():
        for a, b in zip(line, line[1:]):
            p, r = q(a), q(b)
            length = math.hypot(r[0] - p[0], r[1] - p[1])
            ang = math.degrees(math.atan2(r[1] - p[1], r[0] - p[0]))
            parts.append(
                f'<rect x="{p[0] - w / 2:g}" y="{p[1] - w / 2:g}" width="{length + w:.3f}" height="{w:g}" '
                f'rx="{w / 2:g}" transform="rotate({ang:.3f} {p[0]:g} {p[1]:g})"/>'
            )
    d = q(DOT)
    parts.append(f'<circle cx="{d[0]:g}" cy="{d[1]:g}" r="2"/>')
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16">'
        f'<g fill="{colour}">{"".join(parts)}</g></svg>\n'
    )


def render(svg_path, png_path, w, h=None, box=None):
    """Render svg into a transparent w x h PNG; box=(x, y, size) places the icon inside."""
    h = h or w
    surface = cairo.ImageSurface(cairo.FORMAT_ARGB32, w, h)
    ctx = cairo.Context(surface)
    handle = Rsvg.Handle.new_from_file(str(svg_path))
    x, y, s = box or (0, 0, w)
    viewport = Rsvg.Rectangle()
    viewport.x, viewport.y, viewport.width, viewport.height = x, y, s, s
    handle.render_document(ctx, viewport)
    surface.write_to_png(str(png_path))


def main():
    png = OUT / "png"
    win = OUT / "windows"
    for d in (OUT, png, win):
        d.mkdir(parents=True, exist_ok=True)

    files = {
        "linh-icon-1024.svg": master_svg(),
        "linh-icon-macos-1024.svg": macos_svg(),
        "linh-tray-symbolic.svg": tray_svg("#000000"),
        "linh-tray-symbolic-white.svg": tray_svg("#FFFFFF"),
    }
    for name, text in files.items():
        (OUT / name).write_text(text)

    master = OUT / "linh-icon-1024.svg"
    render(master, OUT / "linh-icon-1024.png", 1024)
    render(OUT / "linh-icon-macos-1024.svg", OUT / "linh-icon-macos-1024.png", 1024)
    for size in (16, 32, 48, 128, 512):
        render(master, png / f"linh-icon-{size}.png", size)
        render(OUT / "linh-icon-macos-1024.svg", png / f"linh-icon-macos-{size}.png", size)
    for tone in ("", "-white"):
        for size in (16, 32):
            render(OUT / f"linh-tray-symbolic{tone}.svg", png / f"linh-tray-symbolic{tone}-{size}.png", size)

    # Windows tiles: transparent, no plate. Plated logos keep tile padding;
    # the unplated taskbar variant uses the full 44px target.
    tiles = {
        "linh-square-44x44-logo.png": (44, 44, (4, 4, 36)),
        "linh-square-44x44-logo-unplated.png": (44, 44, (0, 0, 44)),
        "linh-store-logo-50x50.png": (50, 50, (0, 0, 50)),
        "linh-square-150x150-logo.png": (150, 150, (27, 27, 96)),
        "linh-wide-310x150-logo.png": (310, 150, (107, 27, 96)),
        "linh-square-310x310-logo.png": (310, 310, (55, 55, 200)),
    }
    for name, (w, h, box) in tiles.items():
        render(master, win / name, w, h, box)

    print("\n".join(sorted(str(p) for p in OUT.rglob("*") if p.is_file())))


if __name__ == "__main__":
    main()
