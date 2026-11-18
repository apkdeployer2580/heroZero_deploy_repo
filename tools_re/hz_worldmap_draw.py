#!/usr/bin/env python3
"""The world map's painted picture (Phase 7-4) — hero2's `tools_re/worldmap_draw.py` brought to
영웅서기 Zero's world, and on 2026-10-29 (user: *"월드맵 배경을 이제 업스케일링 할꺼야 … hero2 급 퀄리티"*)
re-made as a PAINTING rather than pixel art: hero2's finished background is a detailed painting made
from its generator's layout (a brief + the layout picture → an image model → Lanczos to 2560×1440),
and this generator now paints that kind of picture itself — rendered at 3840×2160 and resampled to
the asset's 2560×1440, so every edge is anti-aliased.

What it paints, all procedural and seeded (the same bytes for the same table):
  * a heightfield — low coasts, rolling hills, ridged mountain massifs in the snowy north-west and
    on the snowy island, red mesas round 휼렌 and 뼈의평원 — lit from the upper left (hill-shading,
    valley shadow, rock on the steep faces, snow above the snowline);
  * the sea by depth (navy → blue → turquoise shallows → wet sand), horizontal wave streaks,
    whitecaps, two foam lines along every shore, the inner sea the ships cross a calmer green-blue;
  * every place's own ground as the game's maps show it (probe, every place's map, 2026-10-23):
    snow, red rock, green sea cliffs, forest floor, golden forest, marsh with pools, dry dirt;
  * forests as thousands of individual trees (broadleaf crowns with a lit side, pines, golden trees,
    dead trees on 잊혀진땅), each with its shadow, drawn back to front; rocks, reeds, bones;
  * the roads as worn dirt tracks painted INTO the ground — curved, feathered verges, grain, a
    corridor through the trees — joining ONLY the pairs the game's exits join (`WM_ROADS`, read
    from the Rust table; the tunnel's run under the sea and the arena's inside the city are not
    painted). The land is forced under every road, so no road crosses water;
  * a landmark at every place: the snowy village, 휼렌's keep, the white-and-blue towns with their
    walls, piers and ships, 퓌렌's orange harbour, the cave, the bunkers, the ruins, the temple, the
    arena, the gatehouse, the hermit's hut, the tunnel mouth (with its run dotted under the sea).

The places, roads, ships and the inner sea are READ from `hz_android/src/worldmap_view.rs`
(`WM_PLACES`, `WM_ROADS`, `WM_SEA_LANES`, `WM_INNER_SEA`) — one table, so moving a place there
moves it here. The places were re-fitted to the exits' edges on 2026-10-29 (see that file's note):
when a map's exit to another place lies on its south edge, that place is below it here.

    python -I tools_re/hz_worldmap_draw.py --png out.png            # the 2560x1440 picture
    python -I tools_re/hz_worldmap_draw.py --rgb565 hz_worldmap.rgb565 [--png base.png] [--marks m.png]
    python -I tools_re/hz_worldmap_draw.py --prompt brief.txt       # the painter's brief
    python -I tools_re/hz_worldmap_draw.py --art painting.png --rgb565 hz_worldmap.rgb565

`--rgb565` writes what the app loads (`assets/hz_worldmap.rgb565`, gitignored — derived at build
time by build_android.ps1): 2560x1440, `[u32 w][u32 h]` + w*h RGB565 little-endian. Needs numpy
and Pillow; about a minute.

The painter's brief (`--prompt`, hero2's way of getting a painting from an image model) still
writes every place in percent, the roads to paint and only those, and what is water, from the same
table; `--art` puts such a finished 16:9 painting into the asset instead (Lanczos to 2560x1440);
build_android.ps1 uses `HEROZERO_WORLDMAP_ART`, or `art/hz_worldmap.png`, when one exists.
"""

import math
import os
import random
import re
import struct
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFilter

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
VIEW_RS = os.path.join(ROOT, "hz_android", "src", "worldmap_view.rs")

# Layout units: the old 1280x720 picture (every sprite is sized in these). The canvas is S times
# that; the asset is OUT_W x OUT_H, resampled from the canvas.
LW, LH = 1280, 720
S = 3
W, H = LW * S, LH * S
OUT_W, OUT_H = 2560, 1440


# ── the table, from the Rust source ──────────────────────────────────────────────────────────────

def read_table(path=VIEW_RS):
    src = open(path, encoding="utf-8").read()
    consts = {m[0]: int(m[1]) for m in re.findall(r"pub const WM_(\w+): usize = (\d+);", src)}
    places_src = src[src.index("pub const WM_PLACES"):src.index("pub const WM_INNER_SEA")]
    places = [(n, int(x), int(y)) for n, x, y in re.findall(r'WmPlace \{\s*name: "([^"]+)",\s*x: (\d+),\s*y: (\d+),', places_src)]
    roads_src = src[src.index("pub const WM_ROADS"):]
    roads_src = roads_src[:roads_src.index("];")]
    roads = [(places[consts[a]][0], places[consts[b]][0]) for a, b in re.findall(r"\(WM_(\w+), WM_(\w+)\)", roads_src)]
    lanes_src = src[src.index("pub const WM_SEA_LANES"):]
    lanes_src = lanes_src[:lanes_src.index("];")]
    lanes = [(places[consts[a]][0], places[consts[b]][0], int(x), int(y)) for a, b, x, y in re.findall(r"\(WM_(\w+), WM_(\w+), (\d+), (\d+)\)", lanes_src)]
    sea = tuple(int(v) for v in re.search(r"WM_INNER_SEA: \(u16, u16, u16, u16\) = \((\d+), (\d+), (\d+), (\d+)\)", src).groups())
    if len(places) != 24 or len(roads) < 20:
        raise SystemExit(f"{path}: read {len(places)} places / {len(roads)} roads — the table's shape changed")
    return {n: (x, y) for n, x, y in places}, roads, lanes, sea


PLACES, ROADS, LANES, INNER_SEA = read_table()


def lp(name):
    """A place in layout px."""
    x, y = PLACES[name]
    return x * LW / 1000, y * LH / 1000


def cp(name):
    """A place in canvas px."""
    x, y = lp(name)
    return x * S, y * S


def tc(x, y):
    """Thousandths → canvas px."""
    return x * W / 1000, y * H / 1000


# ── colours (painted, a little less saturated than the pixel art's) ──────────────────────────────
C = lambda *v: np.array(v, np.float32)
SEA_DEEP = C(12, 38, 96)
SEA_MID = C(24, 78, 156)
SEA_SHALLOW = C(58, 150, 190)
SEA_TURQ = C(112, 206, 212)
INNER_TINT = C(40, 120, 150)
FOAM = C(232, 244, 250)
SAND = C(216, 198, 150)
SAND_WET = C(176, 160, 118)
GRASS = C(104, 150, 66)
GRASS_2 = C(126, 168, 78)
GRASS_DARK = C(74, 118, 52)
MEADOW = C(146, 180, 92)
FOREST_FLOOR = C(54, 96, 44)
GOLD_FLOOR = C(130, 118, 50)
CLIFF = C(120, 140, 62)
CLIFF_ROCK = C(104, 100, 84)
RED = C(158, 88, 60)
RED_DARK = C(112, 58, 40)
RED_LIGHT = C(198, 128, 88)
MARSH = C(96, 128, 66)
MARSH_DARK = C(70, 96, 50)
MARSH_MUD = C(108, 96, 62)
POOL = C(58, 124, 140)
DIRT = C(132, 108, 72)
DIRT_DARK = C(96, 76, 50)
SNOW = C(238, 243, 250)
SNOW_SHADE = C(186, 202, 226)
ROCK = C(128, 120, 108)
ROCK_DARK = C(78, 72, 66)
ROCK_LIGHT = C(176, 168, 154)
ROAD = C(190, 160, 108)
ROAD_EDGE = C(132, 106, 66)
# sprite colours (tuples for PIL)
T = lambda a: tuple(int(v) for v in a)
PINE = (36, 86, 52)
PINE_DARK = (22, 58, 36)
PINE_LIGHT = (78, 136, 76)
LEAF = (56, 122, 52)
LEAF_DARK = (30, 78, 36)
LEAF_LIGHT = (112, 176, 84)
GOLD_LEAF = (184, 160, 58)
GOLD_DARK = (120, 96, 30)
GOLD_LIGHT = (232, 208, 100)
TRUNK = (84, 58, 36)
WALL = (172, 166, 152)
WALL_DARK = (96, 90, 84)
WALL_LIGHT = (214, 208, 196)
ROOF = (184, 70, 46)
ROOF_LIGHT = (226, 112, 76)
ROOF_DARK = (122, 42, 28)
BLUE_ROOF = (46, 92, 170)
BLUE_ROOF_LIGHT = (96, 146, 214)
BLUE_ROOF_DARK = (28, 56, 118)
WHITE_WALL = (238, 234, 222)
WHITE_WALL_SHADE = (196, 190, 178)
WOOD = (150, 110, 64)
WOOD_DARK = (92, 66, 38)
DARK = (14, 12, 18)
BONE = (232, 226, 206)
SAIL = (242, 238, 224)


# ── fields ───────────────────────────────────────────────────────────────────────────────────────

def noise(shape, scale, seed, stretch=1):
    """Smooth value noise 0..1 (a small random field, bicubic up). `stretch` > 1 makes it
    `stretch` times longer along x (wave streaks)."""
    rng = np.random.default_rng(seed)
    h, w = shape
    small = rng.random((max(2, int(h / scale)), max(2, int(w / (scale * stretch))))).astype(np.float32)
    # Float all the way: an 8-bit field would terrace the heightfield into contour lines.
    im = Image.fromarray(small, mode="F").resize((w, h), Image.BICUBIC)
    return np.clip(np.asarray(im), 0, 1).astype(np.float32)


def fbm(shape, scale, seed, octaves=4, gain=0.5):
    out, amp, tot = np.zeros(shape, np.float32), 1.0, 0.0
    for i in range(octaves):
        out += noise(shape, max(2.0, scale / 2 ** i), seed * 7 + i) * amp
        tot += amp
        amp *= gain
    return out / tot


def ridged(shape, scale, seed, octaves=4):
    """Ridged multifractal 0..1: sharp crests (mountains)."""
    out, amp, tot = np.zeros(shape, np.float32), 1.0, 0.0
    for i in range(octaves):
        n = noise(shape, max(2.0, scale / 2 ** i), seed * 11 + i)
        out += (1.0 - np.abs(2.0 * n - 1.0)) ** 2 * amp
        tot += amp
        amp *= 0.55
    return out / tot


def blur(a, r):
    """A smooth low-pass of a float array (radius in canvas px): shrink by `r` with an
    anti-aliased filter, grow back bicubic — float throughout (Pillow's Gaussian blur has no float
    mode, and 8 bits terrace a heightfield)."""
    r = max(1, int(round(r)))
    im = Image.fromarray(np.ascontiguousarray(a, dtype=np.float32), mode="F")
    small = im.resize((max(2, W // r), max(2, H // r)), Image.BICUBIC)
    return np.asarray(small.resize((W, H), Image.BICUBIC)).astype(np.float32)


def ellipse_field(cx, cy, rx, ry):
    """In thousandths: 1 at the centre, 0 on the rim, negative outside."""
    yy, xx = np.mgrid[0:H, 0:W].astype(np.float32)
    (cx, cy), (rx, ry) = tc(cx, cy), tc(rx, ry)
    return 1.0 - (((xx - cx) / rx) ** 2 + ((yy - cy) / ry) ** 2)


def zone(cx, cy, rx, ry, seed, wobble=0.35):
    return ellipse_field(cx, cy, rx, ry) + (noise((H, W), 16 * S, seed) - 0.5) * wobble > 0


def binary_erode(m, r):
    return np.asarray(Image.fromarray(m.astype(np.uint8) * 255).filter(ImageFilter.MinFilter(2 * r + 1))) > 127


def binary_dilate(m, r):
    return np.asarray(Image.fromarray(m.astype(np.uint8) * 255).filter(ImageFilter.MaxFilter(2 * r + 1))) > 127


def smoothstep(a, b, x):
    t = np.clip((x - a) / (b - a), 0, 1)
    return t * t * (3 - 2 * t)


def lerp_into(col, target, t):
    """col ← col·(1−t) + target·t, t per pixel (H×W), target a colour or an H×W×3 array."""
    t = t[..., None]
    col *= 1 - t
    col += target * t


# ── roads (curved, in layout px) ─────────────────────────────────────────────────────────────────

def catmull_rom(pts, per_seg=24):
    """A Catmull-Rom spline through `pts` (≥ 2), sampled `per_seg` times a segment."""
    if len(pts) < 3:
        return pts
    p = [pts[0]] + list(pts) + [pts[-1]]
    out = []
    for i in range(1, len(p) - 2):
        p0, p1, p2, p3 = p[i - 1], p[i], p[i + 1], p[i + 2]
        for k in range(per_seg):
            t = k / per_seg
            t2, t3 = t * t, t * t * t
            x = 0.5 * ((2 * p1[0]) + (-p0[0] + p2[0]) * t + (2 * p0[0] - 5 * p1[0] + 4 * p2[0] - p3[0]) * t2 + (-p0[0] + 3 * p1[0] - 3 * p2[0] + p3[0]) * t3)
            y = 0.5 * ((2 * p1[1]) + (-p0[1] + p2[1]) * t + (2 * p0[1] - 5 * p1[1] + 4 * p2[1] - p3[1]) * t2 + (-p0[1] + 3 * p1[1] - 3 * p2[1] + p3[1]) * t3)
            out.append((x, y))
    out.append(p[-2])
    return out


def road_path(a, b, seed):
    """The track from place `a` to place `b` (layout px): a spline through two or three waypoints
    pushed sideways a little — restrained organic bends, never geometric strokes."""
    rng = random.Random(seed)
    (x0, y0), (x1, y1) = a, b
    dx, dy = x1 - x0, y1 - y0
    ln = math.hypot(dx, dy) or 1.0
    nx, ny = -dy / ln, dx / ln
    n = 2 if ln < 120 else 3
    amp = min(22.0, ln * 0.11)
    pts = [(x0, y0)]
    sgn = rng.choice((-1, 1))
    for i in range(1, n + 1):
        t = i / (n + 1)
        off = sgn * amp * math.sin(t * math.pi) * rng.uniform(0.5, 1.0) + rng.uniform(-4, 4)
        pts.append((x0 + dx * t + nx * off, y0 + dy * t + ny * off))
        sgn = -sgn if rng.random() < 0.6 else sgn
    pts.append((x1, y1))
    return catmull_rom(pts, 20)


def painted_roads():
    """(a, b, path) for every road that is a dirt track on the ground."""
    out = []
    for i, (a, b) in enumerate(ROADS):
        if frozenset((a, b)) in UNDER or "투기장" in (a, b):
            continue
        out.append((a, b, road_path(lp(a), lp(b), 100 + i)))
    return out


def mask_of_paths(paths, width, extra=None):
    """An H×W bool mask of the paths stroked `width` layout px wide (plus `extra` discs)."""
    im = Image.new("L", (W, H), 0)
    d = ImageDraw.Draw(im)
    for path in paths:
        d.line([(x * S, y * S) for x, y in path], fill=255, width=int(width * S), joint="curve")
    for (x, y, r) in extra or ():
        d.ellipse((x * S - r * S, y * S - r * S, x * S + r * S, y * S + r * S), fill=255)
    return np.asarray(im) > 127


# ── the land ─────────────────────────────────────────────────────────────────────────────────────

def land_field(road_mask):
    """Land = the union of the landmasses' blobs − the inner sea and its arms − the outer bays +
    fractal noise, then forced positive under every road and round every place."""
    f = np.full((H, W), -1.0, np.float32)
    for b in (
        (190, 470, 215, 470),   # the west: 실바나르's snow down to 메렌 and the gate
        (320, 360, 140, 105),   # 휼렌's red rock and the bunker, east of it
        (450, 880, 330, 200),   # the south shore: 습지대 · 게힌시티 · 남부길 · 은거지
        (860, 650, 205, 330),   # the east: 퓌렌 · 고난의길 · 뼈의평원 · the ruins
        (610, 205, 160, 150),   # the snowy land across the water: 잊혀진땅 · 신전
        (515, 705, 60, 50),     # the tunnel's cape on the south shore
    ):
        f = np.maximum(f, ellipse_field(*b))
    cx, cy, rx, ry = INNER_SEA
    f = np.minimum(f, -ellipse_field(cx, cy, rx, ry) * 1.4)
    # The inner sea's arms: a gulf west to 메렌, an arm north-west under 휼렌's rock, a bay in
    # front of 게힌시티's harbour, a bay at 퓌렌's quay.
    for b in ((385, 605, 115, 40), (440, 445, 90, 50), (615, 712, 60, 42), (770, 480, 48, 36)):
        f = np.minimum(f, -ellipse_field(*b) * 1.3)
    for b in ((-60, 470, 90, 140), (420, 30, 140, 90), (1080, 120, 170, 160), (1000, 1050, 160, 120)):
        f = np.minimum(f, -ellipse_field(*b))
    f += (fbm((H, W), 40 * S, 1, 3) - 0.5) * 0.5 + (noise((H, W), 10 * S, 2) - 0.5) * 0.14 + (noise((H, W), 3 * S, 3) - 0.5) * 0.05
    # No road on water, no place in the sea.
    f = np.where(road_mask, np.maximum(f, 0.25), f)
    # No islets: an opening of 3 layout px drops the noise's crumbs and keeps every coast.
    crumbs = (f > 0) & ~binary_dilate(binary_erode(f > 0, 3 * S), 3 * S)
    return np.where(crumbs, -0.02, f)


def heightfield(land, zones, dist_in):
    """0..1, lit later: low near the coast, hills inland, mountains in the massifs, mesas in the
    red zones, a lip along the cliff coasts."""
    h = 0.12 + 0.18 * dist_in
    h += (fbm((H, W), 60 * S, 21, 4) - 0.5) * 0.22
    # Mountains: broad ridges, in ranges (the range noise leaves valleys between massifs), with
    # finer crests on top.
    massif = blur(zones["massif"].astype(np.float32), 14 * S)
    h += ridged((H, W), 22 * S, 27, 2) * massif * 0.16
    h = np.maximum(h, peak_field(zones["massif"], 28, 24, (18, 40), (0.6, 1.0)) * 0.98 + 0.1)
    mesa = blur(zones["red"].astype(np.float32), 10 * S)
    steps = blur(np.floor(fbm((H, W), 34 * S, 23, 3) * 5) / 5, 3 * S)
    h += (steps * 0.3 + ridged((H, W), 40 * S, 24, 2) * 0.1) * mesa
    hills = blur(zones["hills"].astype(np.float32), 14 * S)
    h += fbm((H, W), 22 * S, 25, 3) * hills * 0.3
    cliff = blur(zones["cliffs"].astype(np.float32), 8 * S)
    h += (1 - dist_in) * cliff * 0.28
    h = np.where(land, h, -0.05)
    return h


def peak_field(mask, seed, spacing, radius, height):
    """Mountain peaks: cones scattered over `mask` (layout px `spacing` apart, radii and heights
    in the given ranges), jagged at the edges, the union of them a range — the classic painted
    massif once the hill-shading lights one face and darkens the other."""
    rng = random.Random(seed)
    jag = noise((H, W), 5 * S, seed)
    field = np.zeros((H, W), np.float32)
    for x, y in scatter(mask, spacing, seed, 0.9):
        r = rng.uniform(*radius) * S
        hgt = rng.uniform(*height)
        cx, cy = x * S, y * S
        ang, ax, ay, lobes, ph = rng.uniform(0, math.pi), rng.uniform(0.7, 1.3), rng.uniform(0.7, 1.3), rng.choice((3, 4, 5)), rng.uniform(0, 6.3)
        rr = r * 1.4
        x0, x1 = max(0, int(cx - rr - 2)), min(W, int(cx + rr + 3))
        y0, y1 = max(0, int(cy - rr - 2)), min(H, int(cy + rr + 3))
        if x1 <= x0 or y1 <= y0:
            continue
        yy, xx = np.mgrid[y0:y1, x0:x1].astype(np.float32)
        dx, dy = xx - cx, yy - cy
        u = (dx * math.cos(ang) + dy * math.sin(ang)) / ax
        v = (-dx * math.sin(ang) + dy * math.cos(ang)) / ay
        th = np.arctan2(v, u)
        # ridges run out from the summit: the radius is longer along `lobes` spurs
        d = np.hypot(u, v) / (1 + 0.28 * np.cos(lobes * th + ph)) * (0.84 + 0.32 * jag[y0:y1, x0:x1])
        cone = hgt * np.clip(1 - d / r, 0, 1) ** 1.6
        field[y0:y1, x0:x1] = np.maximum(field[y0:y1, x0:x1], cone)
    return field


def hillshade(h, zscale):
    gy, gx = np.gradient(h * zscale)
    nx, ny, nz = -gx, -gy, np.ones_like(h)
    ln = np.sqrt(nx * nx + ny * ny + nz * nz)
    lx, ly, lz = -0.55, -0.6, 0.58
    ll = math.sqrt(lx * lx + ly * ly + lz * lz)
    shade = (nx * lx + ny * ly + nz * lz) / (ln * ll)
    slope = np.sqrt(gx * gx + gy * gy)
    return np.clip(shade, 0, 1), slope


# ── painting the ground ──────────────────────────────────────────────────────────────────────────

def paint_ground(land, zones, h, dist_in, dist_out, road_mask):
    """The float RGB canvas with the sea and every kind of ground, lit."""
    col = np.zeros((H, W, 3), np.float32)
    yy, xx = np.mgrid[0:H, 0:W].astype(np.float32)

    # ── sea ──
    t = dist_out                              # 1 at the shore, 0 far out
    t = np.clip(t + (noise((H, W), 8 * S, 30) - 0.5) * 0.12 * (t > 0.05), 0, 1)   # shoals wander
    sea = SEA_DEEP * (1 - smoothstep(0.0, 0.5, t))[..., None] + SEA_MID * smoothstep(0.0, 0.5, t)[..., None]
    lerp_into(sea, SEA_SHALLOW, smoothstep(0.5, 0.84, t))
    lerp_into(sea, SEA_TURQ, smoothstep(0.84, 0.96, t))
    lerp_into(sea, SAND_WET, smoothstep(0.965, 1.0, t) * 0.75)
    cx, cy, rx, ry = INNER_SEA
    inner = np.clip(ellipse_field(cx, cy, rx * 1.2, ry * 1.25), 0, 1)
    lerp_into(sea, INNER_TINT, inner * 0.3)
    waves = noise((H, W), 2.0 * S, 31, stretch=8)
    swell = fbm((H, W), 36 * S, 32, 3)
    sea += ((swell - 0.5) * 20)[..., None]
    sea += ((waves - 0.5) * 11 * (1 - inner * 0.6) * (1 - smoothstep(0.84, 1.0, t)))[..., None]
    caps = (waves > 0.9) & (t < 0.6) & (t > 0.08) & (noise((H, W), 9 * S, 33) > 0.56)
    sea[caps] = sea[caps] * 0.45 + FOAM * 0.55
    foam1 = smoothstep(0.975, 1.0, t) * (0.5 + 0.5 * noise((H, W), 3 * S, 34))
    foam2 = (np.abs(t - 0.93) < 0.015) * (noise((H, W), 5 * S, 35) > 0.45) * 0.6
    lerp_into(sea, FOAM, np.clip(foam1 + foam2, 0, 1) * 0.85)
    col[~land] = sea[~land]

    # ── ground ──
    g = fbm((H, W), 14 * S, 41, 4)
    grain = noise((H, W), 1.6 * S, 42)
    ground = GRASS * (1 - g)[..., None] + GRASS_2 * g[..., None]
    lerp_into(ground, GRASS_DARK, smoothstep(0.62, 0.9, fbm((H, W), 9 * S, 43, 3)) * 0.6)
    lerp_into(ground, MEADOW, smoothstep(0.6, 0.85, fbm((H, W), 40 * S, 44, 2)) * 0.5)
    lerp_into(ground, C(150, 168, 78), smoothstep(0.58, 0.8, fbm((H, W), 24 * S, 57, 2)) * 0.45)   # dry patches
    lerp_into(ground, C(66, 118, 70), smoothstep(0.6, 0.85, fbm((H, W), 18 * S, 58, 2)) * 0.4)     # damp, darker
    strokes = noise((H, W), 2.4 * S, 59, stretch=4)                                                # brushed grass
    ground *= (0.95 + 0.1 * strokes)[..., None]

    def soft(name, r=6):
        return blur(zones[name].astype(np.float32), r * S)

    cl = soft("cliffs", 5)
    lerp_into(ground, CLIFF * (0.85 + 0.3 * g)[..., None], cl)
    rd = soft("red", 4)
    redc = RED * (1 - g * 0.5)[..., None] + RED_LIGHT * (g * 0.5)[..., None]
    lerp_into(redc, RED_DARK, smoothstep(0.55, 0.9, fbm((H, W), 8 * S, 45, 3)) * 0.6)
    lerp_into(ground, redc, rd)
    lerp_into(ground, FOREST_FLOOR * (0.8 + 0.4 * g)[..., None], soft("forest", 5))
    lerp_into(ground, GOLD_FLOOR * (0.8 + 0.4 * g)[..., None], soft("golden", 5))
    mr = soft("marsh", 5)
    marsh = MARSH * (1 - g)[..., None] + MARSH_DARK * g[..., None]
    lerp_into(marsh, MARSH_MUD, smoothstep(0.55, 0.8, fbm((H, W), 11 * S, 50, 3)) * 0.6)
    pools = smoothstep(0.6, 0.7, fbm((H, W), 13 * S, 46, 3)) * zones["marsh"]
    lerp_into(marsh, POOL * (0.8 + 0.4 * noise((H, W), 3 * S, 47))[..., None], pools)
    lerp_into(ground, marsh, mr)
    dr = soft("dirt", 5)
    dirt = DIRT * (1 - g * 0.6)[..., None] + DIRT_DARK * (g * 0.6)[..., None]
    lerp_into(ground, dirt, dr)

    # beaches: sand where the land is low and close to the water (not on the cliff coasts)
    beach_t = np.clip(blur(land.astype(np.float32), 9 * S) * 2 - 1, 0, 1)
    beach = smoothstep(0.0, 0.4, beach_t + (noise((H, W), 4 * S, 55) - 0.5) * 0.25)
    sand = SAND * (0.9 + 0.2 * grain)[..., None]
    lerp_into(sand, SAND_WET, (1 - smoothstep(0.0, 0.12, beach_t)) * 0.75)
    lerp_into(ground, sand, (1 - beach) * (1 - cl) * (1 - rd * 0.6) * (1 - mr * 0.5))

    # rock on the steep faces (grey, or red in the red zones), snow above the snowline
    shade, slope = hillshade(h, 170.0)
    rock = ROCK * (0.85 + 0.3 * g)[..., None]
    lerp_into(rock, RED_DARK * (0.9 + 0.2 * g)[..., None], rd)
    lerp_into(rock, CLIFF_ROCK, cl)
    slope_n = slope / np.percentile(slope[land], 99.0)
    steep = smoothstep(0.2, 0.75, slope_n) * (1 - soft("marsh", 3))
    lerp_into(ground, rock, steep * 0.92)
    sn = soft("snow", 7)
    snow_line = smoothstep(0.55, 0.66, h + (noise((H, W), 6 * S, 48) - 0.5) * 0.08) * (1 - rd) * (1 - cl * 0.7)
    snow_t = np.clip(sn + snow_line, 0, 1)
    snowc = SNOW * (0.96 + 0.06 * grain)[..., None]
    lerp_into(snowc, SNOW_SHADE, smoothstep(0.55, 0.85, fbm((H, W), 10 * S, 49, 3)) * 0.55)
    # bare rock pokes through the snow only on the steepest SHADED faces; the sunlit faces stay white
    expose = smoothstep(0.55, 0.95, slope_n) * (1 - shade) * 0.85
    lerp_into(ground, snowc, snow_t * (1 - expose))

    # fine grain and lighting; the shadows on snow stay light and blue, not black
    ground *= (0.92 + 0.16 * grain)[..., None]
    light = 0.42 + 0.98 * shade
    light = light * (1 - snow_t) + np.maximum(light, 0.62) * snow_t
    ambient = 1 - 0.25 * np.clip(blur(h, 24 * S) - h, 0, 0.2) / 0.2   # valleys a little darker
    ground *= (light * ambient)[..., None]
    lerp_into(ground, C(150, 172, 222), np.clip(1.2 - light, 0, 1) * snow_t * 0.45)
    col[land] = ground[land]

    # the cliff coasts: a dark layered rock face along the water with a lit lip above it
    strata = noise((H, W), 1.4 * S, 56, stretch=6)
    face = (1 - smoothstep(0.0, 0.42, beach_t)) * cl * land
    lerp_into(col, CLIFF_ROCK * (0.35 + 0.35 * strata)[..., None], face * 0.92)
    lip = smoothstep(0.1, 0.42, beach_t) * (1 - smoothstep(0.42, 0.6, beach_t)) * cl * land
    lerp_into(col, CLIFF_ROCK * 1.3, lip * 0.55)
    return col, shade


# ── the painter's wrapper: layout units, canvas pixels ───────────────────────────────────────────

class Painter:
    """ImageDraw in layout units (every coordinate × S), so the sprites keep their old sizes."""

    def __init__(self, img):
        self.d = ImageDraw.Draw(img)

    def _p(self, pts):
        return [(x * S, y * S) for x, y in pts]

    def _b(self, box):
        x0, y0, x1, y1 = box
        return (x0 * S, y0 * S, x1 * S, y1 * S)

    def line(self, pts, fill, width=1):
        self.d.line(self._p(pts), fill=fill, width=max(1, int(round(width * S))), joint="curve")

    def polygon(self, pts, fill, outline=None):
        self.d.polygon(self._p(pts), fill=fill, outline=outline)

    def rectangle(self, box, fill, outline=None):
        self.d.rectangle(self._b(box), fill=fill, outline=outline, width=S if outline else 1)

    def ellipse(self, box, fill, outline=None):
        self.d.ellipse(self._b(box), fill=fill, outline=outline, width=S if outline else 1)

    def pieslice(self, box, a0, a1, fill, outline=None):
        self.d.pieslice(self._b(box), a0, a1, fill=fill, outline=outline, width=S if outline else 1)

    def point(self, xy, fill):
        x, y = xy
        self.d.rectangle((x * S, y * S, x * S + S - 1, y * S + S - 1), fill=fill)


class Shadows:
    """Soft drop shadows, gathered on one layer and multiplied in before the sprites."""

    def __init__(self):
        self.im = Image.new("L", (W, H), 0)
        self.d = ImageDraw.Draw(self.im)

    def ellipse(self, x, y, rx, ry, a=255):
        self.d.ellipse(((x - rx) * S, (y - ry) * S, (x + rx) * S, (y + ry) * S), fill=a)

    def apply(self, col, strength=0.42, r=1.2):
        sh = np.asarray(self.im.filter(ImageFilter.GaussianBlur(r * S))).astype(np.float32) / 255.0
        col *= (1 - strength * sh)[..., None]


# ── sprites (layout units) ───────────────────────────────────────────────────────────────────────

def shade_col(c, k):
    return tuple(max(0, min(255, int(v * k))) for v in c)


def tree(p, x, y, s=1.0, leaf=LEAF, dark=LEAF_DARK, light=LEAF_LIGHT, rng=random):
    """A broadleaf crown: a dark mass, a few mid lobes, a lit lobe to the upper left, a trunk;
    every tree its own shade of green."""
    k = rng.uniform(0.86, 1.12)
    leaf, dark, light = shade_col(leaf, k), shade_col(dark, k), shade_col(light, k)
    r = 4.6 * s
    p.line([(x, y), (x, y - r)], TRUNK, 1.3 * s)
    p.ellipse((x - r * 1.1, y - 2.2 * r, x + r * 1.1, y - 0.25 * r), dark)
    for _ in range(3):
        ox, oy, rr = rng.uniform(-0.45, 0.45) * r, rng.uniform(-0.3, 0.3) * r, r * rng.uniform(0.6, 0.8)
        p.ellipse((x + ox - rr, y - 1.3 * r + oy - rr, x + ox + rr, y - 1.3 * r + oy + rr), leaf)
    p.ellipse((x - r * 0.8, y - 2.0 * r, x + r * 0.05, y - 1.15 * r), light)
    p.ellipse((x - r * 0.3, y - 1.75 * r, x + r * 0.3, y - 1.25 * r), leaf)


def pine(p, x, y, s=1.0, snow=False):
    h, w = 12 * s, 7.5 * s
    p.line([(x, y), (x, y - 3 * s)], TRUNK, 1.3 * s)
    for yy, ww in ((y - 2.5 * s, w), (y - 2.5 * s - h / 3, w * 0.8), (y - 2.5 * s - 2 * h / 3, w * 0.58)):
        top = yy - h / 2
        p.polygon([(x, top), (x - ww / 2, yy), (x + ww / 2, yy)], PINE_DARK)
        p.polygon([(x, top), (x - ww / 2, yy), (x + ww * 0.08, yy)], PINE)
        p.polygon([(x, top), (x - ww / 2, yy), (x - ww * 0.3, yy)], SNOW_T if snow else PINE_LIGHT)
        if snow:
            p.line([(x - ww / 2, yy), (x, top)], SNOW_T, 1.0 * s)


SNOW_T = (238, 243, 250)


def dead_tree(p, x, y, col=(96, 84, 74)):
    p.line([(x, y), (x, y - 9)], col, 1.8)
    p.line([(x, y - 5), (x - 4, y - 10)], col, 1.1)
    p.line([(x, y - 4), (x + 4, y - 9)], col, 1.1)
    p.line([(x - 2, y - 7), (x - 4, y - 12)], col, 0.8)


def rock(p, x, y, s=1.0):
    p.ellipse((x - 3 * s, y - 2 * s, x + 3 * s, y + 1.5 * s), ROCK_DARK_T)
    p.ellipse((x - 2.6 * s, y - 2.2 * s, x + 1.6 * s, y), ROCK_T)
    p.ellipse((x - 2 * s, y - 2 * s, x - 0.4 * s, y - 1 * s), ROCK_LIGHT_T)


ROCK_T, ROCK_DARK_T, ROCK_LIGHT_T = T(ROCK), T(ROCK_DARK), T(ROCK_LIGHT)


def bones(p, x, y):
    p.line([(x - 2.5, y), (x + 2.5, y - 0.8)], BONE_T, 0.8)
    p.ellipse((x - 3.6, y - 0.8, x - 2, y + 0.8), BONE_T)
    p.ellipse((x + 2, y - 1.6, x + 3.6, y), BONE_T)


BONE_T = (222, 214, 190)


def reeds(p, x, y):
    for ox in (-2, 0, 2):
        p.line([(x + ox, y), (x + ox + ox / 2, y - 5)], (76, 104, 50), 0.9)
    p.line([(x, y - 5), (x, y - 7)], (150, 120, 70), 1.2)


def house(p, sh, x, y, w=14, h=9, roof=ROOF, light=ROOF_LIGHT, dark=ROOF_DARK, wall=(214, 196, 160), snow=False, timber=True):
    sh.ellipse(x + 3, y + 2, w * 0.8, 4, 200)
    p.rectangle((x - w / 2, y - h, x + w / 2, y), wall, WOOD_DARK)
    if timber:
        p.line([(x - w / 2, y - h / 2), (x + w / 2, y - h / 2)], WOOD_DARK, 0.6)
        p.line([(x - w / 4, y - h), (x - w / 4, y)], WOOD_DARK, 0.6)
    rh = h * 2 / 3
    p.polygon([(x - w / 2 - 2, y - h), (x, y - h - rh), (x + w / 2 + 2, y - h)], roof, dark)
    p.polygon([(x - w / 2 - 2, y - h), (x, y - h - rh), (x, y - h)], light)
    for k in range(1, 3):
        yy = y - h - rh + rh * k / 3
        xx = (rh - rh * k / 3) / rh * (w / 2 + 2)
        p.line([(x - xx, yy), (x + xx, yy)], dark, 0.5)
    if snow:
        p.line([(x - w / 2 - 1, y - h), (x, y - h - rh), (x + w / 2 + 1, y - h)], SNOW_T, 2)
    p.rectangle((x - 2, y - 4, x + 1, y), WOOD_DARK)
    p.rectangle((x + w / 4 - 1, y - h / 2 - 2, x + w / 4 + 1, y - h / 2), (112, 150, 190))


def white_house(p, sh, x, y, w=13, h=9):
    """메렌 · 게힌시티: white walls, a blue dome (the maps' blue-and-white towns)."""
    sh.ellipse(x + 3, y + 2, w * 0.75, 4, 200)
    p.rectangle((x - w / 2, y - h, x + w / 2, y), WHITE_WALL, (150, 150, 150))
    p.rectangle((x + w / 2 - 3, y - h, x + w / 2, y), WHITE_WALL_SHADE)
    p.pieslice((x - w / 2, y - h - h / 2 - 2, x + w / 2, y - h + h / 2), 180, 360, BLUE_ROOF, BLUE_ROOF_DARK)
    p.pieslice((x - w / 2 + 2, y - h - h / 2, x, y - h + h / 3), 180, 270, BLUE_ROOF_LIGHT)
    p.line([(x, y - h - h / 2 - 2), (x, y - h - h / 2 - 5)], BLUE_ROOF_DARK, 0.8)
    p.rectangle((x - 2, y - 4, x + 1, y), (50, 76, 130))
    p.rectangle((x - w / 2 + 2, y - h + 2, x - w / 2 + 4, y - h + 4), (90, 120, 170))


def town_wall(p, sh, pts, color=(176, 170, 156), dark=(96, 90, 84)):
    """A low crenellated wall through `pts` (closed if the last point is the first)."""
    sh.ellipse(sum(x for x, _ in pts) / len(pts) + 2, sum(y for _, y in pts) / len(pts) + 2, 1, 1, 0)
    p.line(pts, dark, 3.2)
    p.line(pts, color, 2.2)
    for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
        ln = math.hypot(x1 - x0, y1 - y0) or 1
        for k in range(int(ln / 4)):
            t = (k + 0.5) * 4 / ln
            x, y = x0 + (x1 - x0) * t, y0 + (y1 - y0) * t
            p.rectangle((x - 0.7, y - 1.9, x + 0.7, y - 0.8), dark)
    for x, y in pts[:-1]:
        p.rectangle((x - 2, y - 4, x + 2, y + 1.5), color, dark)


def docks(p, sh, x, y, dx, dy):
    """Two wooden piers from (x, y) out into the water along (dx, dy) — a unit step, (0, -1) =
    north — and a moored ship beyond them."""
    sx, sy = -dy, dx
    for side, ln in ((-7, 26), (7, 36)):
        x0, y0 = x + sx * side, y + sy * side
        x1, y1 = x0 + dx * ln, y0 + dy * ln
        p.line([(x0, y0), (x1, y1)], WOOD_DARK, 5)
        p.line([(x0, y0), (x1, y1)], WOOD, 3.4)
        for k in range(2, int(ln), 4):
            p.line([(x0 + dx * k + sx * 2.5, y0 + dy * k + sy * 2.5), (x0 + dx * k - sx * 2.5, y0 + dy * k - sy * 2.5)], WOOD_DARK, 0.5)
    ship(p, sh, x + dx * 46, y + dy * 46)


def ship(p, sh, bx, by, s=1.0):
    sh.ellipse(bx + 1, by + 3, 12 * s, 3 * s, 160)
    p.polygon([(bx - 11 * s, by), (bx + 11 * s, by), (bx + 8 * s, by + 5 * s), (bx - 8 * s, by + 5 * s)], WOOD_DARK)
    p.polygon([(bx - 10 * s, by + 1 * s), (bx + 10 * s, by + 1 * s), (bx + 7 * s, by + 4 * s), (bx - 7 * s, by + 4 * s)], WOOD)
    p.line([(bx, by), (bx, by - 15 * s)], WOOD_DARK, 1.2 * s)
    p.polygon([(bx + 1 * s, by - 14 * s), (bx + 10 * s, by - 6 * s), (bx + 1 * s, by - 4 * s)], SAIL)
    p.polygon([(bx - 1 * s, by - 12 * s), (bx - 8 * s, by - 5 * s), (bx - 1 * s, by - 5 * s)], (220, 214, 196))
    p.line([(bx, by - 15 * s), (bx + 4 * s, by - 14 * s)], (190, 40, 40), 1.2 * s)


def cave(p, sh, x, y, s=1.0):
    rw, rh = 34 * s, 26 * s
    sh.ellipse(x + 3, y + 4, rw * 0.55, rh * 0.25, 180)
    p.ellipse((x - rw / 2, y - rh, x + rw / 2, y + rh / 4), ROCK_T, ROCK_DARK_T)
    p.ellipse((x - rw / 2 + 3, y - rh + 2, x + rw / 2 - 8, y - rh / 3), ROCK_LIGHT_T)
    p.ellipse((x - rw / 2 + 6, y - rh / 2, x + rw / 2 - 2, y + rh / 5), ROCK_T)
    aw, ah = 12 * s, 12 * s
    p.pieslice((x - aw / 2 - 1.5, y - ah - 1.5, x + aw / 2 + 1.5, y + 2.5), 180, 360, ROCK_DARK_T)
    p.pieslice((x - aw / 2, y - ah, x + aw / 2, y + 1), 180, 360, DARK)
    p.rectangle((x - aw / 2, y - ah / 2, x + aw / 2, y), DARK)
    p.rectangle((x - aw / 2 - 1.5, y - ah / 2, x + aw / 2 + 1.5, y + 1), ROCK_DARK_T)
    p.rectangle((x - aw / 2, y - ah / 2, x + aw / 2, y), DARK)


def tunnel(p, sh, x, y):
    """해저터널: a stone arch on the shore, its tube going north under the inner sea."""
    sh.ellipse(x + 2, y + 6, 16, 4, 180)
    p.rectangle((x - 16, y - 2, x + 12, y + 7), ROCK_T, ROCK_DARK_T)
    p.pieslice((x - 14, y - 22, x + 10, y + 2), 180, 360, WALL, ROCK_DARK_T)
    p.rectangle((x - 14, y - 10, x + 10, y), WALL)
    for yy in range(int(y - 20), int(y + 2), 4):
        p.line([(x - 14, yy), (x + 10, yy)], WALL_DARK, 0.6)
    p.pieslice((x - 9, y - 17, x + 5, y - 3), 180, 360, DARK, ROCK_DARK_T)
    p.rectangle((x - 9, y - 10, x + 5, y), DARK)


def bunker(p, sh, x, y, broken=False):
    sh.ellipse(x + 3, y + 2, 14, 4, 190)
    p.rectangle((x - 13, y - 11, x + 11, y + 1), (170, 170, 164), (72, 72, 70))
    p.rectangle((x - 13, y - 14, x + 11, y - 11), (200, 200, 194), (72, 72, 70))
    p.rectangle((x + 7, y - 11, x + 11, y + 1), (136, 136, 130))
    p.rectangle((x - 4, y - 6, x + 2, y + 1), (44, 50, 64))
    p.rectangle((x - 10, y - 8, x - 7, y - 6), (90, 110, 130))
    if broken:
        p.polygon([(x + 2, y - 14), (x + 11, y - 14), (x + 11, y - 4)], T(RED_DARK))
        p.line([(x - 10, y - 9), (x - 3, y - 3)], (72, 72, 70), 0.8)
        p.line([(x + 4, y - 11), (x + 7, y - 2)], (72, 72, 70), 0.8)
        for ox, oy in ((14, 0), (17, -3), (-17, 1), (-15, 4)):
            p.rectangle((x + ox, y + oy, x + ox + 3, y + oy + 2), (150, 150, 144), (90, 90, 86))
    else:
        p.line([(x - 8, y - 14), (x - 8, y - 25)], (60, 60, 64), 0.8)
        p.ellipse((x - 9, y - 27, x - 7, y - 25), (255, 90, 90))


def temple(p, sh, x, y):
    sh.ellipse(x + 3, y + 7, 22, 5, 200)
    p.rectangle((x - 20, y + 3, x + 20, y + 8), (176, 168, 148), WALL_DARK)
    p.rectangle((x - 18, y + 1, x + 18, y + 3), (200, 192, 170), WALL_DARK)
    p.rectangle((x - 15, y - 9, x + 15, y + 1), (150, 142, 124))
    for i in range(5):
        cx = x - 14 + i * 7
        p.rectangle((cx, y - 9, cx + 2.6, y + 1), (238, 232, 214), WALL_DARK)
    p.polygon([(x - 19, y - 9), (x, y - 20), (x + 19, y - 9)], (168, 148, 118), WALL_DARK)
    p.polygon([(x - 13, y - 10), (x, y - 16.5), (x + 13, y - 10)], (222, 214, 192))
    p.line([(x - 18, y - 10), (x, y - 20), (x + 18, y - 10)], SNOW_T, 1.6)
    p.rectangle((x - 2, y - 6, x + 2, y + 1), DARK)


def fortress(p, sh, x, y):
    """휼렌: a squat stone keep on red rock (the map's dark fortress)."""
    sh.ellipse(x + 4, y + 4, 26, 6, 210)
    p.rectangle((x - 22, y - 10, x + 22, y + 4), (98, 88, 84), (50, 44, 42))
    p.rectangle((x + 16, y - 10, x + 22, y + 4), (72, 64, 60))
    for ox in (-22, 14):
        p.rectangle((x + ox, y - 20, x + ox + 8, y + 4), (112, 102, 98), (50, 44, 42))
        p.rectangle((x + ox + 6, y - 20, x + ox + 8, y + 4), (80, 72, 68))
        for k in range(3):
            p.rectangle((x + ox + k * 3, y - 23, x + ox + k * 3 + 1.4, y - 20), (112, 102, 98))
    p.rectangle((x - 8, y - 28, x + 8, y - 10), (122, 112, 106), (50, 44, 42))
    p.rectangle((x + 5, y - 28, x + 8, y - 10), (86, 78, 74))
    for k in range(5):
        p.rectangle((x - 8 + k * 4, y - 31, x - 6.6 + k * 4, y - 28), (122, 112, 106))
    p.rectangle((x - 1, y - 24, x + 1, y - 21), DARK)
    p.pieslice((x - 4, y - 6, x + 4, y + 4), 180, 360, DARK)
    p.rectangle((x - 4, y - 1, x + 4, y + 4), DARK)
    p.line([(x, y - 31), (x, y - 40)], WOOD_DARK, 0.8)
    p.polygon([(x + 0.8, y - 40), (x + 8, y - 37), (x + 0.8, y - 34)], (180, 36, 36))


def gate(p, sh, x, y):
    """남부관문: a brick wall across the way with two towers and an open gate."""
    sh.ellipse(x + 3, y + 4, 34, 5, 200)
    p.rectangle((x - 30, y - 10, x + 30, y + 3), (150, 96, 74), (92, 58, 44))
    for xx in range(int(x - 28), int(x + 28), 5):
        p.line([(xx, y - 10), (xx, y + 3)], (126, 80, 60), 0.6)
    p.line([(x - 30, y - 5), (x + 30, y - 5)], (126, 80, 60), 0.6)
    for ox in (-34, 24):
        p.rectangle((x + ox, y - 22, x + ox + 10, y + 3), (160, 104, 80), (92, 58, 44))
        p.rectangle((x + ox + 7, y - 22, x + ox + 10, y + 3), (120, 76, 58))
        p.polygon([(x + ox - 1, y - 22), (x + ox + 5, y - 30), (x + ox + 11, y - 22)], ROOF, ROOF_DARK)
        p.polygon([(x + ox - 1, y - 22), (x + ox + 5, y - 30), (x + ox + 5, y - 22)], ROOF_LIGHT)
    p.pieslice((x - 7, y - 12, x + 7, y + 4), 180, 360, DARK)
    p.rectangle((x - 7, y - 4, x + 7, y + 3), DARK)


def arena(p, sh, x, y):
    """투기장: a ring of stone arches round a sand floor."""
    sh.ellipse(x + 3, y + 3, 26, 15, 210)
    p.ellipse((x - 24, y - 15, x + 24, y + 13), WALL, WALL_DARK)
    p.ellipse((x - 24, y - 13, x + 24, y + 13), (150, 144, 132))
    p.ellipse((x - 24, y - 15, x + 24, y + 11), WALL, WALL_DARK)
    p.ellipse((x - 17, y - 10, x + 17, y + 8), (214, 190, 136), WALL_DARK)
    for i in range(0, 360, 24):
        a = math.radians(i)
        ax, ay = x + 21 * math.cos(a), y - 2 + 13 * math.sin(a)
        p.rectangle((ax - 1, ay - 2, ax + 1, ay + 0.5), WALL_DARK)
    p.line([(x - 24, y - 2), (x - 24, y - 9)], WALL_DARK, 0.8)


def ruins(p, sh, x, y):
    """지하유적지: broken pillars round a stair going down."""
    sh.ellipse(x + 2, y + 3, 20, 5, 180)
    p.rectangle((x - 9, y - 5, x + 9, y + 5), (124, 100, 86), WALL_DARK)
    p.rectangle((x - 6, y - 3, x + 6, y + 3), (84, 72, 64))
    p.rectangle((x - 3, y - 1, x + 3, y + 3), DARK)
    for ox, hh in ((-16, 14), (-12, 8), (14, 12), (18, 6)):
        p.rectangle((x + ox, y - hh, x + ox + 3, y + 2), WALL_LIGHT, WALL_DARK)
        p.rectangle((x + ox + 2, y - hh, x + ox + 3, y + 2), WALL)
    p.polygon([(x - 2, y - 9), (x + 6, y - 11), (x + 7, y - 7)], WALL_LIGHT, WALL_DARK)


def hut(p, sh, x, y):
    house(p, sh, x, y, 15, 10, roof=(70, 60, 56), light=(96, 84, 78), dark=(40, 34, 32), wall=(120, 104, 90))
    p.line([(x + 4, y - 16), (x + 4, y - 21)], (60, 50, 46), 1)
    p.ellipse((x + 2, y - 25, x + 7, y - 20), (190, 186, 182))


# ── scattering ───────────────────────────────────────────────────────────────────────────────────

def scatter(mask, spacing, seed, density=1.0):
    """Jittered-grid points inside `mask` (canvas px), about `spacing` layout px apart — returned
    in layout units, sorted top to bottom so later sprites overlap earlier ones."""
    rng = np.random.default_rng(seed)
    step = spacing * S
    ny, nx = int(H / step) + 1, int(W / step) + 1
    gy, gx = np.mgrid[0:ny, 0:nx].astype(np.float32)
    xs = (gx + rng.random(gx.shape)) * step
    ys = (gy + rng.random(gy.shape)) * step
    keep = rng.random(gx.shape) < density
    xi = np.clip(xs.astype(np.int32), 0, W - 1)
    yi = np.clip(ys.astype(np.int32), 0, H - 1)
    keep &= mask[yi, xi]
    pts = np.stack([xs[keep] / S, ys[keep] / S], -1)
    return pts[np.argsort(pts[:, 1])]


# ── the picture ──────────────────────────────────────────────────────────────────────────────────

# What each place looks like (the probe's walk through every place's maps, 2026-10-23) — the
# painter's brief; build() paints the same zones and landmarks.
LOOK = {
    "실바나르": "the hero's home: a small snowy village of brown-roofed houses (an orphanage among them)",
    "실바나르 동부": "a snowfield with snowy mountains and frosted pines",
    "실바나르 남부": "a snowy clearing where five dirt roads meet",
    "실바나르 서부": "green sea cliffs over the western coast",
    "동굴": "a dark cave mouth in snowy rock",
    "휼렌": "a stone fortress town on red rock",
    "지하연구소": "a small concrete bunker door in red rock (an underground laboratory)",
    "소버린구릉지": "rolling forested hills",
    "메렌": "white houses with blue roofs on the inner sea's shore",
    "남부관문": "a brick gatehouse in a wall across the road",
    "습지대": "a marsh of reeds and pools",
    "남부길": "dry dirt country with scattered rocks",
    "은거지": "one dark hut hidden in the forest (a hideout)",
    "해저터널": "a stone arch, the mouth of a tunnel that runs under the sea to the north",
    "잊혀진땅": "bleak snowy land with bare dead trees, across the water",
    "신전": "a snow-covered temple",
    "게힌시티": "a large white harbour city with blue roofs, docks and moored ships",
    "투기장": "an arena (colosseum) beside the city",
    "황금숲": "a golden-leaved forest",
    "고난의길": "green sea cliffs along a hard road",
    "퓌렌": "a harbour town with orange roofs, docks and ships",
    "뼈의평원": "a red rock plain scattered with bones",
    "폐허연구소": "broken bunker ruins on red rock (a ruined laboratory)",
    "지하유적지": "ancient ruins with stairs going underground, on red rock",
}
# Roads build() does not paint: the tunnel's run under the sea, and the arena's inside the city.
UNDER = {frozenset(("습지대", "해저터널")), frozenset(("잊혀진땅", "해저터널"))}


def brief():
    """The painter's brief for a detailed painting of this composition (see the module doc)."""
    if set(LOOK) != set(PLACES):
        raise SystemExit(f"LOOK and the table differ: {sorted(set(LOOK) ^ set(PLACES))}")
    tag = {name: chr(ord("A") + i) for i, name in enumerate(PLACES)}
    pct = lambda name: f"({PLACES[name][0] / 10:.1f}%, {PLACES[name][1] / 10:.1f}%)"
    painted = [(a, b) for a, b in ROADS if frozenset((a, b)) not in UNDER and "투기장" not in (a, b)]
    cx, cy, rx, ry = INNER_SEA
    lines = [
        "Asset: the world map of a fantasy RPG (영웅서기 Zero), one finished painting, landscape 16:9, at least",
        "2560x1440 (3840x2160 if possible). Use the attached layout picture as the exact composition: the same",
        "coastlines, the same inner sea, every place where it is. Repaint it with much finer detail — hand-painted",
        "terrain, individual trees, layered rock, readable landmarks, natural light. Do not merely upscale it.",
        "No text, no letters, no labels, no markers, no pins, no grid, no UI, no graphic lines: the game draws the",
        "names, the highlighted roads, the cursor and the ship routes itself, on top of this picture.",
        "",
        "Coordinates are percent of the width / height from the upper left; centre each landmark exactly there:",
    ]
    lines += [f"{tag[n]} {pct(n)} {LOOK[n]}." for n in PLACES]
    lines += [
        "",
        f"Dirt roads, painted INTO the ground (worn earth, grassy or snowy verges, no hard strokes), joining ONLY",
        f"these {len(painted)} pairs — no other branches, no shortcuts, no roads on water:",
        "; ".join(f"{tag[a]}-{tag[b]}" for a, b in painted) + ".",
        f"Not painted: {tag['습지대']}-{tag['해저터널']} and {tag['잊혀진땅']}-{tag['해저터널']} run UNDER the sea "
        f"(only the tunnel mouth at {tag['해저터널']} shows); {tag['게힌시티']}-{tag['투기장']} is inside the city.",
        "",
        f"An inner sea fills the middle: an ellipse about ({cx / 10:.1f}%, {cy / 10:.1f}%), radii {rx / 10:.1f}% x "
        f"{ry / 10:.1f}%, open water with no place in it. Ships cross it from {tag['게힌시티']} to "
        f"{tag['메렌']}, {tag['퓌렌']} and {tag['휼렌']} (the game draws those routes): paint docks and moored ships",
        f"at {tag['게힌시티']} and {tag['퓌렌']}, nothing on the open water. The land north of the inner sea "
        f"({tag['잊혀진땅']}, {tag['신전']}) is snowy, across the water; so is the north-west around "
        + ", ".join(tag[n] for n in ("실바나르", "실바나르 동부", "실바나르 남부", "동굴")) + ".",
    ]
    return "\n".join(lines) + "\n"


def log(msg):
    print(f"  {msg}", file=sys.stderr, flush=True)


def build():
    random.seed(7)
    P = PLACES
    roads = painted_roads()
    road_core = mask_of_paths([r for _, _, r in roads], 4.6)
    road_land = mask_of_paths([r for _, _, r in roads], 34, extra=[(lp(n)[0], lp(n)[1], 30) for n in P])
    log("land")
    f = land_field(road_land)
    land = f > 0
    land_f = land.astype(np.float32)
    dist_in = np.minimum(blur(land_f, 14 * S), blur(land_f, 5 * S)) * 2 - 1   # ~ how far inside the land (0 at the shore)
    dist_in = np.clip(dist_in, 0, 1)
    near, mid, far = (np.clip(blur(land_f, r * S) * 2, 0, 1) for r in (5, 20, 60))
    dist_out = np.clip(0.3 * near + 0.45 * mid + 0.25 * far, 0, 1)   # ~ 1 at the shore, 0 far at sea
    inland = binary_erode(land, 6 * S)

    def z(name, rx, ry, seed, wobble=0.4, dx=0, dy=0):
        x, y = P[name]
        return zone(x + dx, y + dy, rx, ry, seed, wobble) & inland

    log("zones")
    snow = (z("실바나르", 150, 150, 31) | z("실바나르 동부", 115, 115, 32) | z("실바나르 남부", 120, 110, 33) | z("동굴", 75, 115, 34)
            | (zone(610, 205, 180, 165, 35, 0.3) & inland))
    red = z("휼렌", 78, 90, 41) | z("뼈의평원", 115, 120, 42) | z("폐허연구소", 72, 70, 43) | z("지하유적지", 58, 80, 44) | z("지하연구소", 48, 52, 45)
    cliffs = z("실바나르 서부", 72, 110, 51) | z("고난의길", 72, 92, 52)
    forest = z("소버린구릉지", 98, 100, 61) | z("은거지", 85, 62, 62)
    golden = z("황금숲", 88, 82, 63)
    marsh = z("습지대", 92, 72, 71)
    dirt = z("남부길", 72, 62, 72) | z("해저터널", 46, 42, 73)
    snow &= ~(red | cliffs)
    massif = (z("실바나르 동부", 90, 80, 36, 0.5, dy=-20) | z("동굴", 55, 80, 37, 0.5, dx=20)
              | (zone(600, 170, 120, 90, 38, 0.5) & inland) | z("실바나르", 70, 50, 39, 0.5, dx=60, dy=-70))
    hills = z("소버린구릉지", 110, 110, 64, 0.5) | z("고난의길", 60, 80, 65, 0.5)
    zones = dict(snow=snow, red=red, cliffs=cliffs, forest=forest, golden=golden, marsh=marsh, dirt=dirt,
                 massif=massif & ~red, hills=hills)
    plain = inland & ~(snow | red | cliffs | forest | golden | marsh | dirt)

    log("heightfield")
    h = heightfield(land, zones, dist_in)
    log("ground")
    col, shade = paint_ground(land, zones, h, dist_in, dist_out, road_core)

    # ── the roads, painted into the ground ──
    log("roads")
    soft_road = blur(road_core.astype(np.float32), 1.2 * S)
    edge_noise = noise((H, W), 2.5 * S, 51)
    track = np.clip(soft_road * (0.75 + 0.5 * edge_noise) * 1.6 - 0.25, 0, 1)
    grain = noise((H, W), 1.3 * S, 52)
    tint = col.copy()
    roadc = ROAD * (0.85 + 0.3 * grain)[..., None]
    lerp_into(roadc, ROAD_EDGE, (1 - smoothstep(0.3, 0.9, track)) * 0.7)
    snow_soft = blur(snow.astype(np.float32), 5 * S)
    lerp_into(roadc, C(196, 200, 214), snow_soft * 0.8)               # trampled snow on the snowfields
    lerp_into(roadc, tint * 0.9, 0.2 * np.ones((H, W), np.float32))  # a little of the ground it crosses
    roadc *= (0.72 + 0.5 * shade)[..., None]
    lerp_into(col, roadc, track * 0.94)
    verge = np.clip(blur(road_core.astype(np.float32), 3 * S) - soft_road, 0, 1) * (noise((H, W), 2 * S, 53) > 0.45)
    lerp_into(col, col * 0.86, verge * 0.6)
    pebbles = (track > 0.5) & (noise((H, W), 1.0 * S, 54) > 0.93)
    col[pebbles] = col[pebbles] * 0.6 + ROCK_LIGHT * 0.4

    # the tunnel's run under the sea, dotted
    (x0, y0), (x1, y1) = cp("해저터널"), cp("잊혀진땅")
    img = Image.fromarray(np.clip(col, 0, 255).astype(np.uint8))
    d = ImageDraw.Draw(img)
    for t in np.linspace(0.1, 0.9, 17):
        x, y = x0 + (x1 - x0) * t, y0 + (y1 - y0) * t
        d.ellipse((x - 2.2 * S, y - 1.3 * S, x + 2.2 * S, y + 1.3 * S), fill=(18, 48, 104))

    # ── sprites ──
    log("sprites")
    keep = inland & ~mask_of_paths([r for _, _, r in roads], 13)
    yy, xx = np.mgrid[0:H, 0:W]
    for name in P:
        x, y = cp(name)
        rx, ry = 30 * S, 22 * S
        keep &= ~((((xx - x) / rx) ** 2 + ((yy - y) / ry) ** 2) < 1)
    pools_m = marsh & (smoothstep(0.6, 0.7, fbm((H, W), 13 * S, 46, 3)) > 0.5)
    shore_keep = binary_erode(land, 3 * S)

    # Gather every sprite with its y, then draw back to front in one pass.
    sprites = []  # (y, fn)
    rng = random.Random(44)

    def add(pts, fn):
        for x, y in pts:
            sprites.append((y, fn, x, y))

    groves = fbm((H, W), 26 * S, 103, 3) > 0.56          # the plains' trees stand in clumps
    add(scatter(keep & forest, 5.5, 91, 0.95), "forest")
    add(scatter(keep & golden, 5.5, 92, 0.95), "golden")
    add(scatter(keep & hills & plain & groves, 7, 93, 0.85), "forest")
    add(scatter(keep & hills & plain & ~groves, 14, 104, 0.3), "forest")
    add(scatter(keep & snow & ~massif & groves, 8, 94, 0.75), "snowpine")
    add(scatter(keep & snow & ~massif & ~groves, 17, 105, 0.2), "snowpine")
    add(scatter(keep & snow & massif & (h < 0.45) & groves, 9, 95, 0.6), "snowpine")
    add(scatter(keep & plain & ~hills & groves, 8, 96, 0.75), "plain")
    add(scatter(keep & plain & ~hills & ~groves, 18, 106, 0.2), "plain")
    add(scatter(keep & cliffs, 15, 97, 0.3), "plain")
    add(scatter(keep & marsh & ~pools_m, 7, 98, 0.8), "reeds")
    add(scatter(keep & red, 12, 99, 0.35), "rock")
    add(scatter(keep & dirt, 11, 100, 0.45), "rock")
    add(scatter(keep & zone(*P["뼈의평원"], 80, 80, 101, 0.3), 12, 101, 0.3), "bones")
    add(scatter(keep & zone(*P["잊혀진땅"], 80, 60, 102, 0.3) & snow, 11, 102, 0.6), "dead")
    sprites.sort(key=lambda s: s[0])

    sh = Shadows()
    for _, kind, x, y in sprites:
        if kind in ("forest", "golden", "plain", "snowpine"):
            s = rng.uniform(0.85, 1.25)
            sh.ellipse(x + 2.5 * s, y + 0.8, 4.8 * s, 2.2 * s, 230)
        elif kind == "rock":
            sh.ellipse(x + 1.5, y + 1, 3, 1.4, 180)
    sh.apply(col := np.asarray(img).astype(np.float32), 0.45, 1.0)
    img = Image.fromarray(np.clip(col, 0, 255).astype(np.uint8))
    p = Painter(img)
    rng = random.Random(44)
    for _, kind, x, y in sprites:
        if kind == "forest":
            s = rng.uniform(0.85, 1.25)
            if rng.random() < 0.45:
                pine(p, x, y, s)
            else:
                tree(p, x, y, s, rng=rng)
        elif kind == "golden":
            s = rng.uniform(0.85, 1.25)
            tree(p, x, y, s, GOLD_LEAF, GOLD_DARK, GOLD_LIGHT, rng=rng)
        elif kind == "plain":
            s = rng.uniform(0.85, 1.25)
            if rng.random() < 0.65:
                tree(p, x, y, s * 0.95, rng=rng)
            else:
                pine(p, x, y, s * 0.95)
        elif kind == "snowpine":
            s = rng.uniform(0.85, 1.25)
            pine(p, x, y, s, snow=True)
        elif kind == "reeds":
            reeds(p, x, y)
        elif kind == "rock":
            rock(p, x, y, rng.uniform(0.7, 1.3))
        elif kind == "bones":
            bones(p, x, y)
        elif kind == "dead":
            dead_tree(p, x, y)

    # ── landmarks ──
    log("landmarks")
    lm = Shadows()
    col = np.asarray(img).astype(np.float32)
    # the landmarks' shadows go on first (one soft layer), then their drawing
    ops = []
    vx, vy = lp("실바나르")
    for ox, oy, w in ((-16, -6, 14), (2, -12, 16), (18, -2, 14), (-2, 6, 15), (-20, 10, 12), (14, 12, 12)):
        ops.append(lambda p, sh, ox=ox, oy=oy, w=w: house(p, sh, vx + ox, vy + oy, w, 9, roof=(118, 92, 66), light=(150, 120, 90), dark=(76, 56, 40), wall=(176, 164, 150), snow=True))
    ops.append(lambda p, sh: fortress(p, sh, *lp("휼렌")))
    mx, my = lp("메렌")
    ops.append(lambda p, sh: town_wall(p, sh, [(mx - 24, my - 18), (mx + 26, my - 20), (mx + 28, my + 14), (mx - 22, my + 16), (mx - 24, my - 18)]))
    for ox, oy in ((-14, -8), (2, -12), (16, -4), (-6, 4), (10, 8), (-16, 10)):
        ops.append(lambda p, sh, ox=ox, oy=oy: white_house(p, sh, mx + ox, my + oy))
    gx, gy = lp("게힌시티")
    ops.append(lambda p, sh: town_wall(p, sh, [(gx - 36, gy - 12), (gx - 10, gy - 22), (gx + 24, gy - 20), (gx + 36, gy - 4), (gx + 32, gy + 18), (gx - 34, gy + 20), (gx - 36, gy - 12)]))
    ops.append(lambda p, sh: docks(p, sh, gx, gy - 20, 0, -1))
    for ox, oy, w in ((-24, -6, 13), (-8, -12, 14), (8, -10, 13), (24, -4, 13), (-16, 6, 13), (0, 4, 15), (16, 8, 13), (-28, 14, 12), (26, 12, 12)):
        ops.append(lambda p, sh, ox=ox, oy=oy, w=w: white_house(p, sh, gx + ox, gy + oy, w, 9))
    fx, fy = lp("퓌렌")
    ops.append(lambda p, sh: docks(p, sh, fx - 22, fy + 4, -1, 0))
    for ox, oy, w in ((-16, -4, 14), (0, -8, 16), (14, -2, 14), (-8, 8, 13), (10, 8, 13), (24, 6, 12)):
        ops.append(lambda p, sh, ox=ox, oy=oy, w=w: house(p, sh, fx + ox, fy + oy, w, 9, roof=(214, 118, 48), light=(238, 160, 84), dark=(146, 76, 28), wall=(236, 222, 194), timber=False))
    ops.append(lambda p, sh: cave(p, sh, *lp("동굴")))
    ops.append(lambda p, sh: bunker(p, sh, *lp("지하연구소")))
    ops.append(lambda p, sh: bunker(p, sh, *lp("폐허연구소"), broken=True))
    ops.append(lambda p, sh: ruins(p, sh, *lp("지하유적지")))
    ops.append(lambda p, sh: temple(p, sh, *lp("신전")))
    ops.append(lambda p, sh: arena(p, sh, *lp("투기장")))
    ops.append(lambda p, sh: gate(p, sh, *lp("남부관문")))
    ops.append(lambda p, sh: hut(p, sh, *lp("은거지")))
    ops.append(lambda p, sh: tunnel(p, sh, *lp("해저터널")))

    class NoDraw:
        def __getattr__(self, _):
            return lambda *a, **k: None

    for op in ops:
        op(NoDraw(), lm)
    lm.apply(col, 0.5, 1.4)
    img = Image.fromarray(np.clip(col, 0, 255).astype(np.uint8))
    p = Painter(img)

    class NoShadow:
        def ellipse(self, *a, **k):
            pass

    for op in ops:
        op(p, NoShadow())
    lx, ly = lp("잊혀진땅")
    for ox, oy in ((-22, -6), (18, -10), (4, 10), (-8, -14)):
        dead_tree(p, lx + ox, ly + oy)

    # ── finish: resample to the asset's size, a touch of contrast ──
    log("finish")
    # The popup stretches the picture 1.3–2× bilinear: a little unsharp mask keeps it crisp there.
    out = img.resize((OUT_W, OUT_H), Image.LANCZOS).filter(ImageFilter.UnsharpMask(radius=1.2, percent=55, threshold=2))
    a = np.asarray(out).astype(np.float32) / 255.0
    a = np.clip((a - 0.5) * 1.06 + 0.5, 0, 1)
    lum = a.mean(-1, keepdims=True)
    a = np.clip(lum + (a - lum) * 1.08, 0, 1)
    return Image.fromarray((a * 255 + 0.5).astype(np.uint8))


# ── output ───────────────────────────────────────────────────────────────────────────────────────

def write_rgb565(img, out):
    save_rgb565(np.asarray(img.convert("RGB")), out)


def write_art(path, out):
    """A finished painting as the asset: Lanczos to the asset's size."""
    art = Image.open(path).convert("RGB")
    w, h = art.size
    if abs(w / h - OUT_W / OUT_H) > 0.02:
        print(f"warning: {path} is {w}x{h}, not 16:9 — stretched to {OUT_W}x{OUT_H}", file=sys.stderr)
    save_rgb565(np.asarray(art.resize((OUT_W, OUT_H), Image.LANCZOS)), out)


def save_rgb565(up, out):
    r = (up[..., 0].astype(np.uint16) >> 3) << 11
    g = (up[..., 1].astype(np.uint16) >> 2) << 5
    b = up[..., 2].astype(np.uint16) >> 3
    h, w = up.shape[:2]
    with open(out, "wb") as f:
        f.write(struct.pack("<II", w, h))
        f.write((r | g | b).astype("<u2").tobytes())
    print(f"-> {out} {w}x{h} ({os.path.getsize(out):,} B)")


def main(argv):
    sys.stdout.reconfigure(encoding="utf-8")
    opt = lambda k: argv[argv.index(k) + 1] if k in argv else None
    if not any(k in argv for k in ("--png", "--rgb565", "--marks", "--prompt")):
        print(__doc__)
        return 2
    if opt("--prompt"):
        with open(opt("--prompt"), "w", encoding="utf-8") as f:
            f.write(brief())
        print(f"-> {opt('--prompt')}")
    if opt("--art"):
        if not opt("--rgb565"):
            raise SystemExit("--art needs --rgb565")
        write_art(opt("--art"), opt("--rgb565"))
        return 0
    if not any(k in argv for k in ("--png", "--rgb565", "--marks")):
        return 0
    img = build()
    if opt("--png"):
        img.save(opt("--png"))
        print(f"-> {opt('--png')} {OUT_W}x{OUT_H}")
    if opt("--marks"):
        m = img.copy()
        md = ImageDraw.Draw(m)
        for name in PLACES:
            x, y = PLACES[name][0] * OUT_W / 1000, PLACES[name][1] * OUT_H / 1000
            md.line([(x - 12, y), (x + 12, y)], fill=(255, 0, 255), width=2)
            md.line([(x, y - 12), (x, y + 12)], fill=(255, 0, 255), width=2)
        m.save(opt("--marks"))
        print(f"-> {opt('--marks')}")
    if opt("--rgb565"):
        write_rgb565(img, opt("--rgb565"))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
