"""영웅서기 Zero 드랍표 — 레어 이상 장비(레어 · 에픽 · 세트 · 가디언 · 85제 전설)와 그것을 떨어뜨리는 몬스터, 모드별.
hero2 의 tools_re/droptable 과 같은 모양의 자기 완결형 HTML 한 장(검색 · 종류/등급 필터 · 열 정렬).

usage:  python -I tools_re/hz_drops.py <herozero.jar> [--out out/herozero-drop-table]
        cd out/herozero-drop-table && vercel deploy --prod --yes

Read-only. ⛔ The page carries the game's names — it goes to out/ (gitignored), never into the repo.

Who drops what (each "can drop", at the 「드랍률」 option's ×1):
  the game's drop  cq.o()V (javap 2026-10-31, HEROZERO_GAME_OPTIONS_RE.md §3·§18): one roll R; slots j = 0..3 of
      `/en/edf` are cumulative thresholds, the first valid slot with R < threshold drops — 0·1 `min(LV/4, 14)`,
      2 `min(LV/5 + 13, 28)` past LV 13 only, 3 fixed; the category anywhere in the slot's range. Past the slots
      R in 43428..47927 is the game's recipe `min(3·kind/10, 27)`. Kind 0 drops nothing; a boss (`ak`) drops
      nothing of its own. LV = the round's level (`cq.i()V`) + 0..2 for a non-boss (`cq.q()V`).
  HZ-033  thresholds ×2, the number a few under the game's (drops::spread), 헬+ top pieces (drops::extra),
          노말·하드코어 any monster an 에픽/세트 of its level band (drops::epic); since 2026-11-06 a slot's drop
          goes on to the game's bands for the same R (potion · material · 제련석 · recipe · magic stone at the
          game's odds), so the recipe band is there whatever the slots take.
  HZ-023  at the rare band's top (LV ≥ 75) the category's guardian piece.
  HZ-046  a boss — every `ak` that dies, parts too (cq.y() → g()) — one of balance::GRADED round its level.
  HZ-031 / HZ-035  가디언 모드 (2026-10-31): a monster's 85제 = in each category of its slots 0..2, the piece whose
          share of 가디언 levels LOW..HIGH holds its level (legend::pool); its recipe = one of those pieces'.
The host's tables are parsed from hz_java's sources; a table that no longer parses stops the script.
"""
import json
import pathlib
import re
import struct
import sys
import zipfile

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent
sys.path.insert(0, str(HERE))
import hz_items  # noqa: E402
import hz_maps  # noqa: E402

CATEGORIES = hz_items.CATEGORIES[:16]
MODES = ["노말", "하드코어", "헬", "가디언"]
# cq's factory (`cq.a(SSSB)V` tableswitch 99..131, javap 2026-10-31) makes an `ak` (x · dk · di · at · ck · be · bs ·
# aw · dq · bm · e) for these kinds; 100/123 포자, 116 파이어엘레멘탈 and 121 돌기둥 are summons of plain classes.
BOSS_KINDS = {99, *range(101, 116), 117, 118, 119, 120, 122, *range(124, 132)}
TUTORIAL_MAP = 135  # 실바나르남부 전투 — the opening's fight, never revisited
RECIPE_BAND = (43428, 47928)


def records(data):
    out, off = [], 0
    while off + 2 <= len(data):
        n = struct.unpack(">H", data[off:off + 2])[0]
        out.append(data[off + 2:off + 2 + n])
        off += 2 + n
    return out


def clean(name):
    """A name without the game font's colour codes (`{ } ^ | $`) and line ends (`& ;`)."""
    return re.sub(r"[{}^|$&;]", "", name).strip()


def monsters(z):
    out = []
    for kind, r in enumerate(records(z.read("en/edf"))):
        nl = r[0]
        p = 1 + nl + 21  # edf_drops.py's skipped fields, then the level
        drop = r[p + 11:p + 11 + 24]
        slots = [[*struct.unpack("bb", drop[j * 6:j * 6 + 2]), struct.unpack(">i", drop[j * 6 + 2:j * 6 + 6])[0]] for j in range(4)]
        out.append({"kind": kind, "name": clean(r[1:1 + nl].decode("euc-kr", "replace")), "lv": r[p], "slots": slots,
                    "boss": kind in BOSS_KINDS})
    return out


def stat_line(stats, h):
    b, c, d = stats
    parts = [f"공격 {b}–{c}"] if b >= 0 else []
    if d >= 0:
        parts.append(f"방어 {d}")
    if h >= 0 and b >= 0:
        parts.append(f"크리 {h}%")
    return " · ".join(parts)


def option_line(options):
    rolled = sum(1 for o in options if o == 100 or 0 <= o <= 15)
    sockets = sum(1 for o in options if o == -1)
    return " · ".join(p for p in (f"옵션 {rolled}" if rolled else "", f"소켓 {sockets}" if sockets else "") if p)


# ---------------------------------------------------------------------------- hz_java's tables ----

def rust(path):
    return (ROOT / path).read_text(encoding="utf-8")


def const(src, name):
    m = re.search(rf"pub const {name}: [^=]+= (-?[\d_]+);", src)
    if not m:
        raise SystemExit(f"hz_drops: constant {name} not found")
    return int(m.group(1).replace("_", ""))


def ints(src, name):
    m = re.search(rf"pub const {name}: [^=]+= \[([^\]]*)\];", src)
    if not m:
        raise SystemExit(f"hz_drops: table {name} not found")
    return [int(v) for v in m.group(1).split(",") if v.strip()]


def block(src, name):
    """`pub const NAME …` up to its closing `];` — the sources' tests build records of the same shape."""
    start = src.index(f"pub const {name}:")
    return src[start:src.index("\n];", start)]


def table(src, name, n):
    body = block(src, name)
    want = int(re.match(rf"pub const {name}: \[\([^)]*\); (\d+)\]", body).group(1))
    got = [tuple(int(v) for v in m) for m in re.findall(r"\(" + r",\s*".join([r"(-?\d+)"] * n) + r"\)", body)]
    if len(got) != want:
        raise SystemExit(f"hz_drops: parsed {len(got)} of {name}, the source has {want}")
    return got


def host_tables():
    drops, gear, legend = rust("hz_java/src/drops.rs"), rust("hz_java/src/gear.rs"), rust("hz_java/src/legend.rs")
    balance, guardian, hooks = rust("hz_java/src/balance.rs"), rust("hz_java/src/guardian_mode.rs"), rust("hz_patches/src/lib.rs")
    k = {name: const(src, name) for src, names in (
        (drops, ["SLOT_PCT", "COMMON_SPREAD", "RARE_SPREAD", "RARE_FLOOR", "EXTRA_FROM", "RARE_TOP", "EXTRA_ROUND",
                 "EPIC_BELOW", "EPIC_ABOVE"]),
        (balance, ["BOSS_BELOW", "BOSS_ABOVE"]),
        (guardian, ["LOW", "HIGH", "BASE_LOW", "BASE_HIGH"]),
        (hooks, ["GEAR_ID", "LEGEND_GRADE", "GUARDIAN_ROUND"]),
    ) for name in names}
    k.update(GEAR_LEVEL=const(gear, "LEVEL"), GEAR_GRADE=const(gear, "GRADE"), LEGEND_LEVEL=const(legend, "LEVEL"),
             STAT_PCT=const(legend, "STAT_PCT"), RECIPE_COUNT=const(legend, "RECIPE_COUNT"),
             BOSS_GRADE_PCT=ints(balance, "BOSS_GRADE_PCT"),
             EPIC=table(drops, "EPIC", 3), EXTRA=table(drops, "EXTRA", 3), GRADED=table(balance, "GRADED", 4))
    named = dict(re.findall(r"const (\w+): \[i8; 6\] = \[([^\]]*)\];", gear))
    pieces = {}
    pat = (r'Piece \{ category: (\d+), name: "([^"]+)", price: [\d_]+, sprite: -?\d+, palette: -?\d+, '
           r"stats: \[(-?\d+), (-?\d+), (-?\d+)\], h: (-?\d+), options: (\w+|\[[^\]]*\]) \}")
    for cat, name, b, c, d, h, opts in re.findall(pat, block(gear, "PIECES")):
        opts = [100 if o.strip() == "R" else int(o) for o in named.get(opts, opts.strip("[]")).split(",")]
        pieces[int(cat)] = {"name": name, "stats": [int(b), int(c), int(d)], "h": int(h), "options": opts}
    legends = [{"cat": int(c), "id": int(i), "name": n, "text": [t for t in (t1, t2) if t]}
               for c, i, n, t1, t2 in re.findall(r'l\((\d+), (\d+), "([^"]+)", \w+, \["([^"]*)", "([^"]*)"\]\)', block(legend, "ALL"))]
    if len(pieces) != 16 or len(legends) != const(legend, "COUNT"):
        raise SystemExit(f"hz_drops: parsed {len(pieces)} guardian pieces / {len(legends)} 85제 — the sources moved")
    return k, pieces, legends


# ------------------------------------------------------------------------------------- drops ----

def round_level(n0, r, k):
    """`cq.i()V` (rounds 1·2, r = min(round, 2)) and guardian_mode::level (round 3)."""
    if r == 0:
        return n0
    if r >= k["GUARDIAN_ROUND"]:
        b = min(max(n0, k["BASE_LOW"]), k["BASE_HIGH"])
        return k["LOW"] + (k["HIGH"] - k["LOW"]) * (b - k["BASE_LOW"]) // (k["BASE_HIGH"] - k["BASE_LOW"])
    return (r + 1) * n0 // 5 + (40 if r == 1 else 60)


def spread(top, s, floor):
    """drops::spread over every roll."""
    return {max(top - u, min(floor, top)) for u in range(s + 1)}


def legend_pool(m, level, k, legends):
    """legend::pool — the 85제 a 가디언 monster of `level` drops: the categories of its equipment slots 0..2
    (legend::slot_categories; kind 0 none), in each the piece whose even share of LOW..HIGH holds the level."""
    if m["kind"] == 0:
        return []
    cats = set()
    for lo, hi, _ in m["slots"][:3]:
        if lo >= 0:
            cats.update(range(lo, min(hi, 15) + 1))
    span = k["HIGH"] - k["LOW"] + 1
    at = min(max(level, k["LOW"]), k["HIGH"]) - k["LOW"]
    out = []
    for cat in sorted(cats):
        mine = [n for n, l in enumerate(legends) if l["cat"] == cat]
        if mine:
            out.append(mine[min(at * len(mine) // span, len(mine) - 1)])
    return out


def kill(m, level, r, k, extras):
    """What a kill of `m` at `level` in round `r` can drop: (category, number) pairs and the game's recipe ids."""
    got, recipes = set(), set()
    if not m["boss"] and m["kind"] != 0:
        thresholds = [min(65535, s[2] * k["SLOT_PCT"] // 100) for s in m["slots"]]  # HZ-033 ×2, HZ-005 ×1
        prev, falling = 0, False
        for j, (lo, hi, _) in enumerate(m["slots"]):
            reached = thresholds[j] > prev or falling
            prev = max(prev, thresholds[j])
            valid = (lo >= 0 and hi >= 0) if j == 3 else (lo >= 0 and not (j == 2 and level <= 13))
            if not reached:
                continue
            if not valid:
                falling = True
                continue
            falling = False
            if j == 3:
                got.add((lo, hi))
                continue
            for cat in range(lo, hi + 1):
                if j < 2:
                    ids = spread(min(level // 4, 14), k["COMMON_SPREAD"], 0)
                else:
                    band = level // 5 + 13
                    ids = spread(min(band, k["RARE_TOP"]), k["RARE_SPREAD"], k["RARE_FLOOR"])
                    if band >= k["RARE_TOP"]:  # gear::at_top
                        got.add((cat, k["GEAR_ID"]))
                    if r >= k["EXTRA_ROUND"] and ids & set(range(k["EXTRA_FROM"], k["RARE_TOP"] + 1)):
                        got.update((cat, e) for e in extras.get(cat, []))
                got.update((cat, i) for i in ids)
        recipes.add(min(3 * m["kind"] // 10, 27))  # HZ-033's band fix (2026-11-06): past the slots too
    if r < k["EXTRA_ROUND"]:  # HZ-033's 에픽/세트, every monster (bosses too)
        got.update((c, i) for c, i, lv in k["EPIC"] if level - k["EPIC_BELOW"] <= lv <= level + k["EPIC_ABOVE"])
    if m["boss"]:  # HZ-046
        window = [g for g in k["GRADED"] if level - k["BOSS_BELOW"] <= g[3] <= level + k["BOSS_ABOVE"]]
        if not window:
            near = min(abs(g[3] - level) for g in k["GRADED"])
            window = [g for g in k["GRADED"] if abs(g[3] - level) == near]
        for grade, pct in enumerate(k["BOSS_GRADE_PCT"], 1):
            if pct > 0:
                band = [g for g in window if g[2] == grade] or window
                got.update((g[0], g[1]) for g in band)
    return got, recipes


# --------------------------------------------------------------------------------------- main ----

def main(argv):
    if not argv or argv[0].startswith("-"):
        print(__doc__)
        return 2
    jar = argv[0]
    out = pathlib.Path(argv[argv.index("--out") + 1]) if "--out" in argv else ROOT / "out" / "herozero-drop-table"
    z = zipfile.ZipFile(jar)
    k, pieces, legends = host_tables()

    items = {}
    for cat in range(16):
        for i, r in enumerate(records(z.read(f"itm/{cat:02d}"))):
            it = hz_items.parse(cat, r)
            if it["grade"] >= 1:
                items[(cat, i)] = {"n": clean(it["name"]), "g": it["grade"], "t": cat, "lv": it["level"],
                                   "st": stat_line(it["stats"], it["slots"]), "op": option_line(it["options"])}
    for cat, p in pieces.items():
        items[(cat, k["GEAR_ID"])] = {"n": p["name"], "g": k["GEAR_GRADE"], "tier": "가디언", "t": cat, "lv": k["GEAR_LEVEL"],
                                     "st": stat_line(p["stats"], p["h"]), "op": option_line(p["options"])}
    for n, l in enumerate(legends):
        p = pieces[l["cat"]]
        up = lambda v: v if v < 0 else (v * k["STAT_PCT"] + 50) // 100  # legend::scaled  # noqa: E731
        items[(l["cat"], l["id"])] = {"n": l["name"], "g": k["LEGEND_GRADE"], "tier": "85제", "t": l["cat"], "lv": k["LEGEND_LEVEL"],
                                      "st": stat_line([up(s) for s in p["stats"]], up(p["h"])), "op": option_line(p["options"]),
                                      "fx": l["text"], "legend": n}
    for c, i, *_ in k["EPIC"] + k["EXTRA"] + k["GRADED"]:
        if (c, i) not in items:
            raise SystemExit(f"hz_drops: ({c}, {i}) of a host table is not a rare-and-up item of the jar")

    # The game's recipes: `/itm/18` 0..27 end in three ingredients and the result (es.a([BI)I).
    makes = {}
    for rid, r in enumerate(records(z.read("itm/18"))):
        rest = bytes.fromhex(hz_items.parse(18, r)["rest"])
        makes[rid] = struct.unpack("bb", rest[6:8])

    maps = hz_maps.load(jar)
    mons = monsters(z)
    placed = {}
    for num, m in maps.items():
        if num != TUTORIAL_MAP:
            for kind in m.monster_kinds:
                placed.setdefault(kind, set()).add(num)
    extras = {}
    for c, i, _ in k["EXTRA"]:
        extras.setdefault(c, []).append(i)

    drops = {key: {} for key in items}  # item -> monster name -> rounds
    recipe_from = {}
    legend_recipe_from = {}  # 85제 -> the monsters that drop its recipe
    for m in mons:
        if m["kind"] not in placed:
            continue
        for r in range(len(MODES)):
            base = round_level(m["lv"], r, k)
            for level in ([base] if m["boss"] else [base, base + 1, base + 2]):
                got, recipes = kill(m, level, r, k, extras)
                if r >= k["GUARDIAN_ROUND"]:  # HZ-031 / HZ-035
                    for n in legend_pool(m, level, k, legends):
                        l = legends[n]
                        got.add((l["cat"], l["id"]))
                        if n < k["RECIPE_COUNT"]:
                            legend_recipe_from.setdefault((l["cat"], l["id"]), set()).add(m["name"])
                for key in got & items.keys():
                    drops[key].setdefault(m["name"], set()).add(r)
                for rid in recipes:
                    recipe_from.setdefault(rid, set()).add(m["name"])
    crafted = {}
    for rid, key in makes.items():
        if key in items:
            crafted.setdefault(key, set()).update(recipe_from.get(rid, ()))

    rows = []
    for key, it in items.items():
        groups = {}
        for name, rounds in drops[key].items():
            groups.setdefault(tuple(sorted(rounds)), []).append(name)
        d = [[" · ".join(MODES[r] for r in rs), sorted(names)] for rs, names in sorted(groups.items())]
        if "legend" in it:
            it.pop("legend")
            if key in legend_recipe_from:
                d.append(["조합서 · 가디언", sorted(legend_recipe_from[key])])
        if key in crafted:
            d.append(["조합서", sorted(crafted[key]) or ["드랍 없음"]])
        rows.append({**it, "tn": CATEGORIES[it["t"]], "d": d})
    rows.sort(key=lambda it: (it["t"], it["lv"], it["n"]))
    for it in rows:
        if not it["d"]:
            print(f"hz_drops: ⚠ no source for {it['n']} ({it['tn']} LV {it['lv']})")

    vc = re.search(r"versionCode=(\d+)", (ROOT / "android_app" / "version.properties").read_text(encoding="ascii")).group(1)
    blob = json.dumps({"vc": int(vc), "items": rows}, ensure_ascii=False, separators=(",", ":")).replace("</", "<\\/")
    tpl = (HERE / "hz_drops.html").read_text(encoding="utf-8")
    if "{{DATA}}" not in tpl:
        raise SystemExit("hz_drops.html: placeholder {{DATA}} missing")
    out.mkdir(parents=True, exist_ok=True)
    (out / "index.html").write_text(tpl.replace("{{DATA}}", blob), encoding="utf-8")
    (out / ".gitignore").write_text(".vercel\n", encoding="ascii")
    tiers = {t: sum(1 for it in rows if it.get("tier") == t) for t in ("가디언", "85제")}
    print(f"hz_drops: {len(rows)} items (가디언 {tiers['가디언']}, 85제 {tiers['85제']}), vC {vc} -> {out / 'index.html'}")
    return 0


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    sys.exit(main(sys.argv[1:]))
