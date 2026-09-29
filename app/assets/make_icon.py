"""Generates the Calyptra icon: a spiral of self-similar crescents (hoods),
each a scaled and rotated copy of the last, like an IFS orbit.

    python app/assets/make_icon.py   (needs rsvg-convert on PATH)

Writes icon.svg, icon.png (256 px, the window icon) and icon.ico.
"""
import math
import pathlib
import struct
import subprocess

HERE = pathlib.Path(__file__).parent
SIZE = 256
COUNT = 14
SCALE = 0.82             # each crescent relative to the previous
TURN = 36.0              # degrees between crescents
RADIUS = 0.36            # crescent size relative to its distance from the center
SWIRL = 55.0             # degrees the hood opening leans along the spiral


def lerp(a, b, t):
    return a + (b - a) * t


def palette(t):
    """Flame-ish ramp: gold -> orange -> magenta -> violet."""
    stops = [(0.0, (255, 214, 102)), (0.35, (255, 138, 61)),
             (0.7, (236, 64, 122)), (1.0, (140, 82, 255))]
    for (t0, c0), (t1, c1) in zip(stops, stops[1:]):
        if t <= t1:
            u = (t - t0) / (t1 - t0)
            return "#%02x%02x%02x" % tuple(round(lerp(a, b, u)) for a, b in zip(c0, c1))
    return "#%02x%02x%02x" % stops[-1][1]


def crescent(cx, cy, r, angle):
    """A hood: outer circle minus an inner circle pushed toward `angle`."""
    ri = r * 0.80
    off = r * 0.36
    ix, iy = cx + off * math.cos(angle), cy + off * math.sin(angle)
    # Intersection points of the two circles.
    d = off
    a = (r * r - ri * ri + d * d) / (2 * d)
    h = math.sqrt(max(r * r - a * a, 0.0))
    ux, uy = math.cos(angle), math.sin(angle)
    px, py = cx + a * ux, cy + a * uy
    p1 = (px - h * uy, py + h * ux)
    p2 = (px + h * uy, py - h * ux)
    return (f"M{p1[0]:.2f},{p1[1]:.2f} A{r:.2f},{r:.2f} 0 1 1 {p2[0]:.2f},{p2[1]:.2f} "
            f"A{ri:.2f},{ri:.2f} 0 1 0 {p1[0]:.2f},{p1[1]:.2f} Z")


def layout(count, turn, scale):
    """Crescent circles (x, y, r) around a spiral centered at the origin."""
    circles = []
    for i in range(count):
        dist = scale ** i
        theta = math.radians(150 + turn * i)
        circles.append((dist * math.cos(theta), dist * math.sin(theta), RADIUS * dist))
    return circles


def svg(simple=False):
    count, turn, scale = (7, 48.0, 0.74) if simple else (COUNT, TURN, SCALE)
    circles = layout(count, turn, scale)
    # Fit the whole spiral into the tile with a margin, centered.
    x0 = min(x - r for x, y, r in circles); x1 = max(x + r for x, y, r in circles)
    y0 = min(y - r for x, y, r in circles); y1 = max(y + r for x, y, r in circles)
    fit = (256 - 2 * (38 if not simple else 30)) / max(x1 - x0, y1 - y0)
    ox, oy = 128 - fit * (x0 + x1) / 2, 128 - fit * (y0 + y1) / 2
    global CENTER
    CENTER = (ox, oy)
    parts = []
    for i, (x, y, r) in enumerate(circles):
        cx, cy = ox + fit * x, oy + fit * y
        # The hood opens toward the spiral's center, turned along the spiral.
        opening = math.atan2(oy - cy, ox - cx) - math.radians(SWIRL)
        t = i / (count - 1)
        parts.append(f'<path d="{crescent(cx, cy, fit * r, opening)}" fill="{palette(t)}"/>')
    shapes = "\n    ".join(parts)
    glow = "" if simple else 'filter="url(#glow)"'
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="{SIZE}" height="{SIZE}" viewBox="0 0 256 256">
  <defs>
    <radialGradient id="bg" cx="0.55" cy="0.48" r="0.75">
      <stop offset="0" stop-color="#2d1f5e"/>
      <stop offset="0.6" stop-color="#170f36"/>
      <stop offset="1" stop-color="#0b0820"/>
    </radialGradient>
    <radialGradient id="core" cx="{CENTER[0] / 256:.3f}" cy="{CENTER[1] / 256:.3f}" r="0.35">
      <stop offset="0" stop-color="#ff8ad8" stop-opacity="0.45"/>
      <stop offset="1" stop-color="#ff8ad8" stop-opacity="0"/>
    </radialGradient>
    <filter id="glow" x="-20%" y="-20%" width="140%" height="140%">
      <feGaussianBlur stdDeviation="5" result="b"/>
      <feMerge><feMergeNode in="b"/><feMergeNode in="SourceGraphic"/></feMerge>
    </filter>
  </defs>
  <rect x="8" y="8" width="240" height="240" rx="52" fill="url(#bg)"/>
  <rect x="8" y="8" width="240" height="240" rx="52" fill="url(#core)"/>
  <g {glow}>
    {shapes}
  </g>
</svg>
'''


def rasterize(svg_path, size, out):
    subprocess.run(["rsvg-convert", "-w", str(size), "-h", str(size), "-o", str(out), str(svg_path)],
                   check=True)
    return out.read_bytes()


def main():
    full = HERE / "icon.svg"
    full.write_text(svg())
    small = HERE / "icon-small.svg"   # fewer, bolder shapes for 16-32 px
    small.write_text(svg(simple=True))
    rasterize(full, 256, HERE / "icon.png")

    tmp = HERE / "_tmp.png"
    images = []
    for size in (16, 20, 24, 32, 40, 48, 64, 128, 256):
        images.append((size, rasterize(small if size <= 32 else full, size, tmp)))
    tmp.unlink()

    # ICO with PNG-compressed entries (Vista+).
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    entries, blobs = b"", b""
    for size, data in images:
        dim = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
        blobs += data
    (HERE / "icon.ico").write_bytes(header + entries + blobs)


if __name__ == "__main__":
    main()
