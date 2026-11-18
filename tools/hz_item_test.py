"""The new equipment, every piece, in the game (2026-10-30, user: "신규장비들이 문제없이 옵션 잘 작동하는지 전수 깊게
테스트해봐"): the 85제 (`hz_java::legend::ALL`, HZ-031/032) and the 가디언 장비 (`hz_java::gear`, HZ-023).

    python -I tools/hz_item_test.py records [--jobs 0,1,2,3,4,5]      # 1: records, option rolls, what wearing does
    python -I tools/hz_item_test.py effects [--only NAME,...] [-j 12]  # 2: every 85제 effect in a fight
    python -I tools/hz_item_test.py roundtrip                          # 3: the pieces through the game's save and load

1 `records` — the probe's `--item-check` (hz_probe/src/field_tick.rs) as each job at LV 99 (stats 100 each): every piece
  made by the game's factory `ba.a(cat, id)`, worn alone in its slot, taken off. Checked: the fields the game parsed against
  the host's tables (level, grade, set, sprite, palette, stats, `h`, name); the option template (rolled slots in 0..15, the
  rest as the template); the item's own option sums `bx.a(k)` = count × the game's table `bx.d`; the player's worn option sums
  `ek.g(k)`; stats 힘 민첩 건강 의지, max HP, max SP, attack and defence by the formulas (HZ-040's PATCH rules, the game's
  weapon term min + max·15/100 — the game's own LV 65 pieces are worn as controls under the same expectations); and nothing
  left behind once it is off.

2 `effects` — one fight per piece (map 44, 가디언 모드, LV 99, the piece's job, every active skill at 5, the job's 가디언
  장비 on the other slots): basic attacks, every skill twice, a spirit skill, a potion, a walk, HP set low and very low, kills
  and hits taken, with the bench trace on (`HZ_BENCH=1` → `hz: fx …` lines from every HZ-032 hook: what the game asked and
  what the hook handed back). Each effect has its own check on those lines (`CHECKS`); every run also checks that no hook
  changed a value the piece does not explain (`unexplained`) and that the game logged no error.

3 `roundtrip` — all 118 pieces given (`--give`), listed field by field (`--item-check`'s read on the bag, `--bag-dump`),
  saved through the game's own quick save, then a second run loads the saved records and lists them again: the same bytes.
"""
import argparse
import concurrent.futures
import json
import os
import re
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.path.join(ROOT, "target", "release", "hz_probe.exe")
JAR = os.path.join(ROOT, "android_app", "app", "src", "main", "assets", "herozero.jar")
OUT = os.path.join(ROOT, "out", "itemtest")

# bx.d (javap 2026-10-30): option k adds this much — 0..3 힘 민첩 건강 의지 · 4 SP · 5 crit · 6 attack · 7 defence ·
# 8 evasion · 9 HP · 10..13 the four stats · 14 accuracy · 15 every stat.
OPTION_VALUE = [5, 5, 5, 5, 50, 1, 8, 5, 1, 100, 8, 8, 8, 8, 1, 3]
R = 100
WEAPON = [R, R, R, R, -1, -2]
BODY = [R, R, R, -1, -3, -3]
LIMB = [R, R, -1, -3, -3, -3]
TEMPLATE = {**{c: WEAPON for c in range(8)}, 8: LIMB, 9: LIMB, 10: [R, R, -2, -2, -3, -3], 11: [R, R, R, R, R, -1],
            12: BODY, 13: BODY, 14: LIMB, 15: LIMB}
JOB_NAMES = ["워리어", "로그", "나이트", "건너", "총검사", "정령사"]
# What `records` allocates to each of 힘 민첩 건강 의지.
BASE_STAT = 100


# ------------------------------------------------------------------------------------------- tables
def rust(path):
    with open(os.path.join(ROOT, path), encoding="utf-8") as f:
        return f.read()


def guardian_table():
    """gear.rs PIECES: category -> dict(name, price, sprite, palette, stats, h)."""
    out = {}
    for m in re.finditer(r'Piece \{ category: (\d+), name: "([^"]+)", price: ([\d_]+), sprite: (-?\d+), palette: (-?\d+), '
                         r'stats: \[(-?\d+), (-?\d+), (-?\d+)\], h: (-?\d+)', rust("hz_java/src/gear.rs")):
        cat = int(m[1])
        if cat in out:
            continue  # the test's own copy of a game piece further down
        out[cat] = dict(name=m[2], price=int(m[3].replace("_", "")), sprite=int(m[4]), palette=int(m[5]),
                        stats=[int(m[6]), int(m[7]), int(m[8])], h=int(m[9]))
    assert len(out) == 16, out.keys()
    return out


def legend_table():
    """legend.rs ALL: list of dict(cat, id, name, effect, text)."""
    src = rust("hz_java/src/legend.rs")
    body = src[src.index("pub const ALL"):]
    body = body[:body.index("];")]
    out = []
    for m in re.finditer(r'\bl\((\d+), (\d+), "([^"]+)", (\w+), \["([^"]*)", "([^"]*)"\]\)', body):
        out.append(dict(cat=int(m[1]), id=int(m[2]), name=m[3], effect=m[4], text=(m[5] + " " + m[6]).strip()))
    assert len(out) == 102, len(out)
    return out


def scaled(v, pct):
    return (v * pct + 50) // 100


# The PATCH rules (hz_java/src/stats.rs) the HZ-040 hooks use.
STR_PCT = [115, 75, 115, 75, 125, 100]
DEX_DIV = [4, 2, 4, 6, 4, 4]
WIL_DIV = [0, 0, 0, 0, 0, 2]
STR_DEF_DIV = [10, 10, 10, 10, 10, 0]
WIL_DEF_DIV = [0, 0, 0, 0, 0, 10]


def div(v, d):
    return v // d if d else 0


def attack_base(j, s, d, w):
    pct = STR_PCT[j]
    strength = {125: s + s // 4, 75: s - s // 4, 100: s}.get(pct, s * pct // 100)
    return strength + div(d, DEX_DIV[j]) + div(w, WIL_DIV[j])


def stat_defence(j, s, w):
    return div(s, STR_DEF_DIV[j]) + div(w, WIL_DEF_DIV[j])


# ------------------------------------------------------------------------------------------- the probe
def prelude():
    """A new game to the field (hz_rotation.py's keys) and the level-up notices after `--exp-test` at 7730."""
    k = ["300:OK", "400:OK", "500:OK"]
    for i, f in enumerate(range(600, 7401, 60)):
        k.append(f"{f}:{'OK' if i % 2 == 0 else 'HASH'}")
    k += [f"{f}:HASH" for f in range(7740, 7990, 8)]
    return k


def probe(args, out, env_extra=None, frames=8300):
    os.makedirs(out, exist_ok=True)
    cmd = [EXE, "--jar", JAR, "--w", "520", "--h", "240", "--ms", "20", "--frames", str(frames), "--every", "100000", "--out", out] + args
    env = dict(os.environ, **(env_extra or {}))
    t0 = time.time()
    r = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace", env=env)
    log = r.stdout + r.stderr
    with open(os.path.join(out, "log.txt"), "w", encoding="utf-8") as f:
        f.write(" ".join(cmd) + "\n\n" + log)
    return log, r.returncode, time.time() - t0


# The game's own: `cc.a()` sets a sound's loop count on a started player (CLAUDE.md — the game catches it), which the
# JVM first tries to throw as IllegalStateException, a class it does not have.
# And `g.c()` @1450 `cw.b()` on a pickup notice left over from the map before a warp (CLAUDE.md — the game's catch
# swallows it, nothing changes on screen).
KNOWN = ("setLoopCount() on a started Player", "No such class: java/lang/IllegalStateException", "Method cw::b:()I is called on null")


def guest_errors(log):
    return [l for l in log.splitlines()
            if re.search(r"Uncaught|GUEST ERROR|panicked|FAILED|Exception", l) and "[patch]" not in l and not any(k in l for k in KNOWN)]


# ------------------------------------------------------------------------------------------- 1 records
LINE = re.compile(
    r"item-check (\w+) (\d+) (\d+) name (\w*) lv (-?\d+) grade (-?\d+) set (-?\d+) sprite (-?\d+) pal (-?\d+) stats \[(.*?)\] h (-?\d+) "
    r"opts \[(.*?)\] own \[(.*?)\] slot (\d+) worn_at (-?\d+) \| worn (.*?) \| back (.*?) \| same (\w+)")
NUMS = re.compile(r"c \[(.*?)\] b \[(.*?)\] atk (-?\d+) def (-?\d+) g \[(.*?)\]")


def nums(s):
    m = NUMS.search(s)
    ints = lambda t: [int(x) for x in t.split(",")]
    return dict(c=ints(m[1]), b=ints(m[2]), atk=int(m[3]), df=int(m[4]), g=ints(m[5]))


def check_records(log, job, guardian, legends):
    m = re.search(r"item-check job (\d+) base (.*)", log)
    base = nums(m[2])
    by_key = {(l["cat"], l["id"]): l for l in legends}
    rows, fails = [], []
    for m in LINE.finditer(log):
        kind, cat, iid = m[1], int(m[2]), int(m[3])
        lv, grade, set_, sprite, pal, h = (int(m[i]) for i in (5, 6, 7, 8, 9, 11))
        stats = [int(x) for x in m[10].split(",")]
        opts = [int(x) for x in m[12].split(",")]
        own = [int(x) for x in m[13].split(",")]
        slot, worn_at = int(m[14]), int(m[15])
        worn, back, same = nums(m[16]), nums(m[17]), m[18] == "true"
        name = bytes.fromhex(m[4]).decode("euc-kr", "replace")
        bad = []
        # The record as the game parsed it.
        if kind in ("legend", "guardian"):
            g = guardian[cat]
            if kind == "legend":
                want = by_key[(cat, iid)]
                exp_name, exp_lv, exp_grade = want["name"], 85, 4
                exp_stats = [s if s < 0 else scaled(s, 110) for s in g["stats"]]
                exp_h = g["h"] if g["h"] < 0 else scaled(g["h"], 110)
            else:
                exp_name, exp_lv, exp_grade, exp_stats, exp_h = g["name"], 80, 2, g["stats"], g["h"]
            for what, got, exp in (("name", name, exp_name), ("lv", lv, exp_lv), ("grade", grade, exp_grade), ("set", set_, -1),
                                   ("sprite", sprite, g["sprite"]), ("palette", pal, g["palette"]), ("stats", stats, exp_stats), ("h", h, exp_h)):
                if got != exp:
                    bad.append(f"{what} {got} != {exp}")
            for i, (o, t) in enumerate(zip(opts, TEMPLATE[cat])):
                if t == R and not 0 <= o <= 15:
                    bad.append(f"option {i} rolled {o}")
                if t != R and o != t:
                    bad.append(f"option {i} {o} != template {t}")
        # The item's own option sums.
        for k in range(16):
            exp = sum(1 for o in opts if o == k) * OPTION_VALUE[k]
            if own[k] != exp:
                bad.append(f"own[{k}] {own[k]} != {exp}")
        # Worn: the slot, the option sums, the stats, the tops, attack, defence.
        if worn_at != slot:
            bad.append(f"worn at {worn_at} not {slot}")
        dg = [worn["g"][k] - base["g"][k] for k in range(16)]
        if dg != own:
            bad.append(f"ek.g {dg} != own {own}")
        dc = [worn["c"][i] - base["c"][i] for i in range(4)]
        exp_dc = [own[i] + own[10 + i] + own[15] for i in range(4)]
        if job == 5:
            # The 정령사's 의지 ×1.15 (the game's own controls do the same), on the allocated 100.
            exp_dc[3] = (BASE_STAT + exp_dc[3]) * 115 // 100 - BASE_STAT * 115 // 100
        if dc != exp_dc:
            bad.append(f"stats +{dc} != +{exp_dc}")
        s0, s1 = base["c"], worn["c"]
        exp_hp = own[9] + 14 * dc[2] + ((s1[2] // 5 - s0[2] // 5) if job == 2 else 0)
        if worn["b"][0] - base["b"][0] != exp_hp:
            bad.append(f"max HP +{worn['b'][0] - base['b'][0]} != +{exp_hp}")
        exp_sp = own[4] + 10 * dc[3]
        if worn["b"][1] - base["b"][1] != exp_sp:
            bad.append(f"max SP +{worn['b'][1] - base['b'][1]} != +{exp_sp}")
        # The hand: its minimum, and for the 워리어 15 % of its maximum too (the game's own controls do the same).
        weapon = stats[0] + (stats[1] * 15 // 100 if job == 0 else 0) if slot == 3 else 0
        exp_atk = weapon + own[6] + attack_base(job, *s1[:2], s1[3]) - attack_base(job, *s0[:2], s0[3])
        if worn["atk"] - base["atk"] != exp_atk:
            bad.append(f"attack +{worn['atk'] - base['atk']} != +{exp_atk}")
        armour = stats[2] if stats[2] > 0 and (slot in (0, 1, 4, 6) or (slot == 5 and job == 2)) else 0
        exp_def = armour + own[7] + stat_defence(job, s1[0], s1[3]) - stat_defence(job, s0[0], s0[3])
        if worn["df"] - base["df"] != exp_def:
            bad.append(f"defence +{worn['df'] - base['df']} != +{exp_def}")
        if not same or back != base:
            bad.append("taking it off left the player changed")
        rows.append(dict(kind=kind, cat=cat, id=iid, name=name, opts=opts, bad=bad))
        if bad:
            fails.append((kind, cat, iid, name, bad))
    return rows, fails


def cmd_records(a):
    guardian, legends = guardian_table(), legend_table()
    jobs = [int(j) for j in a.jobs.split(",")]

    def one(job):
        actions = ["--warp", "7700:44:21:19", "--job", f"7720:{job}", "--exp-test", "7730:200000000",
                   "--base-stat", "8000:0:100,8004:1:100,8008:2:100,8012:3:100", "--item-check", "8200"]
        log, code, wall = probe(["--keys", ",".join(prelude())] + actions, os.path.join(OUT, f"records_j{job}"))
        return job, log, code, wall

    total_fail = 0
    with concurrent.futures.ThreadPoolExecutor(max_workers=len(jobs)) as ex:
        for job, log, code, wall in ex.map(one, jobs):
            rows, fails = check_records(log, job, guardian, legends)
            errs = guest_errors(log)
            kinds = {k: sum(1 for r in rows if r["kind"] == k) for k in ("legend", "guardian", "control")}
            print(f"{JOB_NAMES[job]}: {len(rows)} pieces ({kinds}) — {len(rows) - len(fails)} ok, {len(fails)} failed, "
                  f"guest errors {len(errs)}, exit {code}, {wall:.0f} s")
            for kind, cat, iid, name, bad in fails:
                print(f"   FAIL {kind} ({cat},{iid}) {name}: " + "; ".join(bad))
            for e in errs[:5]:
                print("   ERR", e[:200])
            total_fail += len(fails) + len(errs) + (code != 0) + (len(rows) != 124)
            if job == jobs[0]:
                rolled = [o for r in rows if r["kind"] != "control" for o, t in zip(r["opts"], TEMPLATE[r["cat"]]) if t == R]
                hist = [rolled.count(k) for k in range(16)]
                print(f"   option rolls over {len(rolled)} rolled slots: " + " ".join(f"{k}:{n}" for k, n in enumerate(hist)))
    print("records:", "ALL OK" if total_fail == 0 else f"{total_fail} problem(s)")
    return total_fail


# ------------------------------------------------------------------------------------------- 2 effects
W, RG, KN, GN, BY, SP = range(6)

# What each piece promises, written from its item text (legend.rs `text` — what the player reads) and the game's own skill
# names (`t/s<job>.tdf`), NOT from effects.rs: the oracle the hooks are held to. Keys:
#   job        the job the fight is played as (the one the text names; else one that wears the category)
#   twice      skills that are cast a second time (SP and cooldown once)
#   hit        (selector, +pct) on the damage of a hit: a skill name, "skill" (any job skill), "basic", "any"; "@cond" =
#              only while clone / tr (동화) / untr / counter / hp<40 / hp>=70 / spfull / ambush; +pct "gear/2" = 중력
#   flat       "wil/2": half of 의지 added to every hit
#   taken      (cond, −pct) on damage taken: always / clone / counter / volley / tr / untr / hp<30 / hp>=70 / skillguard /
#              killguard (the reductions add, at most 90)
#   sp · cd    (selector, −pct) on a skill's SP / cooldown: a skill name, "skill"/"job" (every job skill), "skill@hp<50"
#   spirit_cd · spirit_power · spirit_cost   −pct / +pct / −pct of the cost on spirit skills
#   crit       (cond, +points): always / basic / hp<40 / clone;  dashcrit: the first hit after 신속 is a critical
#   duration   fusion / clone / mark / counter: percent, or "forever"
#   reflect    +pct on 반격's reflection;  leech: pct of it back as HP
#   drain      pct of the HP every hit really took off a monster back as HP (HZ-067, on top of the piece's own effect)
#   chance     (hook, "x2" | "+30");  poison: percent of 독수's poison;  knock: +pct on a knocked-back monster
#   knockable  False: never pushed;  walk: "fast" | "brisk" (not while 동화) | "drunk" (fast 10 s after a potion)
#   (knock, as the texts of 화약 장갑 and 폭발의 반지 say: the 총검사's knockbacks only)
#   luck       the 85제 / recipe drop multiplier;  smash_wide · bash_wide · frenzy_skip · killguard: the hook acts
#   events     (why, stat index 0 HP · 1 SP · 3 MP, amount, when): amount ("flat", n) · ("top_pct", p) · ("dealt_pct", p) ·
#              ("taken_pct", p); when basic · skillhit · kill · taken · drank
#   shave      (why, frames off every cooldown);  fusion_add: (why, frames added to 동화)
#   focus      (skill, attack): a last phase (30 s) pressing that skill again and again (with or without basic attacks) at
#              the monsters' spawn — for what needs hits taken while the skill is up (연사's guard, 반격); "@lowhp": idle
#              with HP put back to 25 % every 3 s
SPEC = {
    "WhirlwindTwice": dict(job=W, twice=["회전베기"]),
    "SmashWide": dict(job=W, hit=[("강타", 40)], smash_wide=True),
    "CounterPower": dict(job=KN, reflect=80, focus=("반격", False)),
    "ShieldBashWide": dict(job=KN, bash_wide=True),
    "CloneLong": dict(job=RG, duration={"clone": 200}, hit=[("any@clone", 35)]),
    "SlashTwice": dict(job=RG, twice=["참격"]),
    "FusionLong": dict(job=SP, duration={"fusion": 200}, hit=[("any@untr", 15), ("any@tr", 40)]),
    "SnipeDouble": dict(job=GN, chance=("SnipeDouble", "x2"), focus=("저격", True)),
    "SprayTwice": dict(job=GN, twice=["난사"]),
    "RushTwice": dict(job=BY, twice=["돌진"]),
    "CounterForever": dict(job=KN, duration={"counter": "forever"}, focus=("반격", True)),
    "PoisonBoost": dict(job=RG, chance=("PoisonBoost", "+30"), poison=300, focus=("독수", True)),
    "AimBoost": dict(job=GN, hit=[("조준사격", 60)], sp=[("조준사격", 50)]),
    "SpiritMana": dict(job=SP, spirit_cost=30),
    "CounterLeech": dict(job=KN, leech=6, focus=("반격", False)),
    "BlastBoost": dict(job=BY, chance=("BlastBoost", "+30"), knock=30),
    "ChainSkill": dict(job=W, hit=[("skill", 20)]),
    "WhirlwindCooldown": dict(job=W, cd=[("회전베기", 50)]),
    "TrickLong": dict(job=GN, duration={"mark": 300}, focus=("트릭샷", True)),
    "BarrageBoost": dict(job=BY, hit=[("포격", 50)], cd=[("포격", 30)]),
    "FusionOnHit": dict(job=SP, fusion_add=("FusionOnHit", 5)),
    "FocusSp": dict(job=W, sp=[("skill", 20)]),
    "SpiritPower": dict(job=SP, spirit_power=80, spirit_cd=20),
    "TimeThief": dict(job=W, shave=("TimeThief", 20)),
    "CloneGuard": dict(job=RG, taken=[("clone", 30)], focus=("분신", False)),
    "VolleyGuard": dict(job=GN, taken=[("volley", 40)], focus=("연사", False)),
    "FusionGuard": dict(job=SP, taken=[("untr", 10), ("tr", 20)], fusion_add=("FusionGuard", 20)),
    "TimeCloak": dict(job=SP, cd=[("job", 15)], spirit_cd=15),
    "FrenzyKills": dict(job=W, events=[("FrenzyKills", 1, ("top_pct", 6), "kill")], frenzy_skip=True),
    "CounterGuard": dict(job=KN, taken=[("counter", 25)], focus=("반격", False)),
    "BayonetGuard": dict(job=BY, taken=[("always", 15)], knockable=False),
    "Gravity": dict(job=W, hit=[("any", "gear/2")]),
    "DashCrit": dict(job=RG, dashcrit=True, sp=[("신속", 50)]),
    "FlowSp": dict(job=RG, events=[("FlowSp", 1, ("flat", 2), "basic")]),
    "ChargeBoost": dict(job=W, hit=[("돌격", 60)], cd=[("돌격", 50)]),
    "Drunkard": dict(job=W, walk="drunk"),
    "StrikeTwice": dict(job=W, twice=["강타"]),
    "BasicLeech": dict(job=W, events=[("BasicLeech", 0, ("dealt_pct", 5), "basic")], drain=2),
    "LowHpRage": dict(job=W, hit=[("any@hp<40", 50)]),
    "ShockBoost": dict(job=W, hit=[("충격", 80)], cd=[("충격", 50)]),
    "ShoutBoost": dict(job=W, cd=[("외침", 60)], sp=[("외침", 50)]),
    "ChargeTwice": dict(job=W, twice=["돌격"]),
    "WhirlwindPower": dict(job=W, hit=[("회전베기", 60)]),
    "SkillSpGain": dict(job=W, events=[("SkillSpGain", 1, ("flat", 5), "skillhit")]),
    "CritPlus15": dict(job=W, crit=[("always", 15)]),
    "HighHpGuard": dict(job=W, taken=[("hp>=70", 25)]),
    "BashTwice": dict(job=KN, twice=["방패치기"]),
    "ShieldChargeBoost": dict(job=KN, hit=[("방패 돌격", 80)], cd=[("방패 돌격", 50)]),
    "PushBoost": dict(job=KN, hit=[("방패 밀기", 80)], sp=[("방패 밀기", 50)]),
    "CounterStrike": dict(job=KN, hit=[("any@counter", 40)], focus=("반격", True), drain=2),
    "HealCheap": dict(job=KN, sp=[("회복", 60)]),
    "ShieldChargeTwice": dict(job=KN, twice=["방패 돌격"]),
    "MountainMace": dict(job=KN, taken=[("always", 10)], reflect=40, focus=("반격", False)),
    "KillCooldown": dict(job=KN, shave=("KillCooldown", 40)),
    "CounterLong": dict(job=KN, duration={"counter": 200}, focus=("반격", True)),
    "PushTwice": dict(job=KN, twice=["방패 밀기"]),
    "ComboTwice": dict(job=RG, twice=["연격"]),
    "AmbushBoost": dict(job=RG, hit=[("기습", 80)], cd=[("기습", 50)], drain=2),
    "SpiritDagger": dict(job=SP, events=[("SpiritDagger", 3, ("flat", 5), "basic")], spirit_power=30, spirit_cd=20),
    "VolleyTwice": dict(job=GN, twice=["연사"]),
    "PointBlankBoost": dict(job=GN, hit=[("근접사격", 80)], cd=[("근접사격", 50)], drain=2),
    "SnipeCheap": dict(job=GN, sp=[("저격", 60)], cd=[("저격", 50)]),
    "AimTwice": dict(job=GN, twice=["조준사격"]),
    "BarrageTwice": dict(job=BY, twice=["포격"]),
    "SkillHeal": dict(job=BY, events=[("SkillHeal", 0, ("top_pct", 3), "skillhit")], drain=2),
    "RushBoost": dict(job=BY, hit=[("돌진", 60)], cd=[("돌진", 50)], focus=("돌진", True)),
    "BarrageCheap": dict(job=BY, sp=[("포격", 50)]),
    "FullSpRage": dict(job=BY, hit=[("any@spfull", 30)]),
    "IronWall": dict(job=KN, taken=[("always", 20)]),
    "CounterCheap": dict(job=KN, cd=[("반격", 50)], sp=[("반격", 50)], focus=("반격", True)),
    "DamageToSp": dict(job=KN, events=[("DamageToSp", 1, ("taken_pct", 20), "taken")]),
    "LowHpGuard": dict(job=KN, taken=[("hp<30", 50)], focus=("@lowhp", False)),
    "BashBoost": dict(job=KN, hit=[("방패치기", 60)], sp=[("방패치기", 50)]),
    "CritGlove": dict(job=RG, crit=[("always", 20)]),
    "GunnerGlove": dict(job=GN, cd=[("트릭샷", 50)], hit=[("연사", 40)]),
    "FusionCheap": dict(job=SP, sp=[("동화", 50)], cd=[("동화", 50)], drain=2),
    "BasicCrit": dict(job=W, crit=[("basic", 25)]),
    "SkillGuard": dict(job=W, taken=[("skillguard", 30)]),
    "LowHpCrit": dict(job=W, crit=[("hp<40", 40)]),
    "SageHelm": dict(job=SP, spirit_cost=20, events=[("SageHelm", 3, ("top_pct", 10), "kill")]),
    "Lucky": dict(job=W, luck=2),
    "LifeRing": dict(job=W, events=[("LifeRing", 0, ("top_pct", 15), "kill")], drain=2),
    "LowHpCheap": dict(job=W, sp=[("skill@hp<50", 50)]),
    "Endurance": dict(job=SP, taken=[("always", 10)], cd=[("job", 10)], spirit_cd=10),
    "GhostCloak": dict(job=RG, crit=[("clone", 30)], duration={"clone": 150}),
    "HighHpRage": dict(job=RG, hit=[("any@hp>=70", 25)]),
    "KillGuard": dict(job=W, taken=[("killguard", 30)], killguard=True),
    "BasicPower": dict(job=W, hit=[("basic", 30)]),
    "SwiftBoots": dict(job=RG, walk="fast"),
    "DashTwice": dict(job=RG, twice=["신속"]),
    "FusionSwift": dict(job=SP, walk="brisk"),
    "VolleyBoost": dict(job=GN, hit=[("연사", 30)], sp=[("연사", 50)]),
    "RushCheap": dict(job=BY, sp=[("돌진", 50)]),
    "PotionSp": dict(job=W, events=[("PotionSp", 1, ("top_pct", 100), "drank")]),
    "WhirlwindCheap": dict(job=W, sp=[("회전베기", 50)]),
    "FullGuardCheap": dict(job=KN, sp=[("전력방어", 50)], cd=[("전력방어", 50)]),
    "SpiritRing": dict(job=SP, flat="wil/2"),
    "OathCrown": dict(job=SP, spirit_power=30, spirit_cost=10),
    "AssassinRing": dict(job=RG, hit=[("any@ambush", 40)]),
    "ShadowHelm": dict(job=RG, crit=[("clone", 20)]),
    "BlastRing": dict(job=BY, knock=50),
    "GunnerBoots": dict(job=BY, cd=[("포격", 30)]),
}

# The game's spirit power bonus for everyone (HZ-039 rides it: balance::SPIRIT_POWER_BONUS) — not a piece's.
SPIRIT_BONUS = 50
# The rotation bench's builds (hz_rotation.py JOBS): stat split in tenths, weapons (slot 3, slot 5), heavy armour, the
# job's actives in pairs for the three phases.
BUILDS = {
    W: ((6, 0, 4, 0), (0, None), True),
    RG: ((2, 6, 2, 0), (4, 4), False),
    KN: ((5, 0, 5, 0), (2, 7), True),
    GN: ((2, 6, 2, 0), (5, 5), False),
    BY: ((5, 2, 3, 0), (6, None), True),
    SP: ((0, 2, 4, 4), (4, None), False),
}
ARMOUR = {True: {0: 9, 1: 10, 2: 11, 4: 13, 6: 15}, False: {0: 8, 1: 10, 2: 11, 4: 12, 6: 14}}
SPIRIT_SKILL = 4  # the spirit skill on quick slot 2 (key 7) for the 정령사, as the rotation bench
POTION = (20, 2)  # 포션 (HP 100): the special bag's potion, quick slot 4 = key 0
FIGHT = 8400
WORN_FROM = 8134  # the piece goes on at 8132 (fight_plan): the trace before is the bare build
FPS = 80  # probe frames a game second at --ms 20


def skill_names():
    """(job, action) -> the game's skill name, from t/s<job>.tdf (name at 2·i, action 4 + i)."""
    sys.path.insert(0, os.path.join(ROOT, "tools_re"))
    import tdf
    import zipfile
    jar = zipfile.ZipFile(JAR)
    out = {}
    for job in range(6):
        rows, _ = tdf.decode(jar.read(f"t/s{job}.tdf"))
        for i in range(6):
            if rows[2 * i].strip():
                out[(job, 4 + i)] = rows[2 * i].strip()
    return out


def slot_of(cat):
    return {**{c: 3 for c in range(7)}, 7: 5, 8: 0, 9: 0, 10: 1, 11: 2, 12: 4, 13: 4}.get(cat, 6)


def fight_plan(job, piece, hit_me=False, focus=None):
    """The probe arguments of one fight: the build, the guardian set with `piece` (cat, id) or nothing in its slot, the
    keys (the same for every run of a job — `random.Random(job)`), the phases. `hit_me`: no 민첩 (a piece about damage
    taken needs the monsters to land — a 로그 or 건너 at 60 % 민첩 is hardly ever hit)."""
    import random
    split, (w1, w2), heavy = BUILDS[job]
    if hit_me:
        split = (split[0] + split[1], 0, split[2], split[3])
    actives = [a for a in range(4, 10) if (job, a) in NAMES]
    pairs = [actives[i:i + 2] for i in range(0, len(actives), 2)]
    while len(pairs) < 3:
        pairs.append(pairs[0])
    total = 4 * 10 + 4 * 98
    stats = [10 + (total - 40) * s // 10 for s in split]
    gear = dict(ARMOUR[heavy])
    gear[3] = w1
    if w2 is not None:
        gear[5] = w2
    actions = ["--round", "7690:3", "--warp", "7700:44:21:19", "--job", f"7720:{job}", "--exp-test", "7730:200000000"]
    actions += ["--base-stat", ",".join(f"{8000 + 4 * i}:{i}:{v}" for i, v in enumerate(stats))]
    actions += ["--skill-level", ",".join(f"{8020 + 4 * i}:{s}:{5 if s < 10 else 3}" for i, s in enumerate(range(4, 12)))]
    if job == SP:
        actions += ["--spirit-level", ",".join(f"{8060 + 4 * i}:{s}:5" for i, s in enumerate(range(0, 10)))]
    equips = [f"{8100 + 4 * i}:{slot}:{c}:40" for i, (slot, c) in enumerate(sorted(gear.items()))]
    if piece:
        equips.append(f"8132:{slot_of(piece[0])}:{piece[0]}:{piece[1]}")
    actions += ["--equip-test", ",".join(equips)]
    actions += ["--give", ",".join(f"{8090 + i}:{POTION[0]}:{POTION[1]}" for i in range(3))]
    p0 = FIGHT
    phases = [(p0, p0 + 800, pairs[0]), (p0 + 800, p0 + 1600, pairs[1]), (p0 + 1600, p0 + 2400, pairs[2]),
              (p0 + 2400, p0 + 2800, pairs[0]), (p0 + 2800, p0 + 3200, pairs[1]), (p0 + 3200, p0 + 3600, None),
              (p0 + 3600, p0 + 4200, pairs[2]), (p0 + 4200, p0 + 4800, pairs[0])]
    focus_action = None
    low_hp = []
    if focus:
        # 30 s: 반격's cooldown is 9 s, and a landed hit while it is up is not every hit.
        if focus[0] == "@lowhp":
            focus_action = -1
            low_hp = [f"{f}:25" for f in range(p0 + 4800, p0 + 7200, 240)]
        else:
            focus_action = next(a for (j, a), n in NAMES.items() if j == job and n == focus[0])
        phases.append((p0 + 4800, p0 + 7200, [focus_action]))
    # Back to the monsters' spawn at each phase (the fight drifts away from it).
    actions += ["--warp", ",".join(f"{start - 10}:44:21:19" for start, _, _ in phases[1:])]
    quick = [f"8140:4:{POTION[1]}"]
    if job == SP:
        quick.append(f"8144:2:{SPIRIT_SKILL}")
    for start, _, pair in phases:
        if pair and pair != [-1]:
            quick += [f"{start - 6}:0:{pair[0]}", f"{start - 4}:1:{pair[-1]}"]
    actions += ["--quick-set", ",".join(quick)]
    actions += ["--refill", ",".join(str(f) for f in (8160, p0 + 790, p0 + 1590, p0 + 2390, p0 + 3190, p0 + 3590, p0 + 4790))]
    actions += ["--hp-pct", ",".join([f"{p0 + 2400}:35", f"{p0 + 2800}:28", f"{p0 + 3205}:60"] + low_hp)]
    rnd = random.Random(1000 + job)
    used = set()
    keys = []

    def put(f, key, hold=1):
        # Every key its own game frame (4 probe frames) and then some: the game holds one key, and a press
        # inside the same game frame releases the one before it unread.
        while any(g in used for g in range(f - 5, f + hold + 6)):
            f += 1
        used.update(range(f, f + hold + 1))
        keys.append((f, f"{key}:{hold}" if hold > 1 else key))

    walks = [(p0 + 200, "LEFT"), (p0 + 260, "RIGHT"), (p0 + 3240, "LEFT"), (p0 + 3300, "RIGHT"), (p0 + 3360, "UP"), (p0 + 3420, "DOWN")]
    for f, k in walks:
        put(f, k, 48)
    put(p0 + 3220, "0")
    quiet = []  # (from, to): no attacks, so the monsters land their hits (hit_me fights)
    for start, end, pair in phases:
        if focus_action is not None and pair == [focus_action]:
            if focus_action == -1:
                quiet.append((start, end))
            else:
                t = start + 40
                while t < end - 20:
                    put(t, "1")
                    if not focus[1]:
                        quiet.append((t, t + 80))
                    t += rnd.randrange(70, 91)
            pair = None
        if pair:
            t, which = start + 10, 0
            while t < end - 20:
                put(t, "13"[which])
                if hit_me:
                    quiet.append((t + 30, t + 150))
                which ^= 1
                t += rnd.randrange(80, 101) + (120 if hit_me else 0)
        if job == SP:
            t = start + 40
            while t < end - 20:
                put(t, "7")
                t += rnd.randrange(150, 181)
        if hit_me and start == p0 + 2800:
            quiet.append((start, start + 240))
        t = start + 30
        while t < end:
            put(t, f"{rnd.choice(['LEFT', 'DOWN', 'RIGHT', 'UP'])}", 4)
            t += rnd.randrange(40, 61)
        t = start
        while t < end:
            if not any(a <= t < b for a, b in quiet):
                put(t, "OK")
            t += rnd.choice((12, 16, 20))
    keys.sort()
    keystr = ",".join(prelude() + ["7500:DOWN:20", "7560:LEFT:15", "7620:RIGHT:15"] + [f"{f}:{k}" for f, k in keys])
    return ["--keys", keystr, "--opt", "loot=on,respawn=6"] + actions, phases[-1][1] + 100


# ---- the trace
FX = re.compile(r"hz: fx (.*)$")


def events(log):
    """The trace in order, each with the probe frame before it: (frame, kind, fields dict, raw)."""
    out = []
    f = 0
    for line in log.splitlines():
        m = re.search(r"\[probe\] f=(\d+)", line)
        if m:
            f = int(m[1])
            continue
        if "hz: 85제 skill" in line and "second cast" in line:
            out.append((f, "second", {"action": int(re.search(r"skill (\d+)", line)[1])}, line))
            continue
        if "hz: bench kill" in line:
            out.append((f, "kill", {}, line))
            continue
        m = FX.search(line)
        if not m:
            continue
        body = m[1]
        kind = body.split()[0]
        d = {}
        for k, v in re.findall(r"\b(\w+) (true|false|-?\d+)\b", body):
            d.setdefault(k, int(v) if v.lstrip("-").isdigit() else v == "true")
        m2 = re.match(r"\S+(?: (\w+))? (-?\d+) -> (-?\d+)", body)
        out.append((f, kind, dict(d, body=body), line))
    return out


COUNTER_FOREVER = 2**31 - 1  # effects::COUNTER_FOREVER — 응징의 방패's 반격 length
# A status's length counts drawn frames (`aq.a()V`); the probe draws one game frame in 4 of its frames (the 반격 of
# 성채의 철퇴, 200, reflects until ~800 frames after the cast).
PROBE_FRAMES_PER_DRAW = 4


def scale(v, pct):
    q = abs(v * pct) // 100
    return q if v * pct >= 0 else -q


# A skill whose damage lands in a later stage of its own: the 총검사's 돌진 queues action 6 on reaching a monster and that
# stage deals the blast (av.a(BI)Z @367 ek.f(6), @392 db.a(5) — javap 2026-10-30; the game names no skill 6 for the job).
STAGES = {(BY, 6): "돌진"}


def sel_match(sel, job, action, flags, names):
    """A selector of SPEC against a hit / SP / cooldown: (skill or class)@cond."""
    what, _, cond = sel.partition("@")
    if what == "any":
        ok = True
    elif what in ("skill", "job"):
        ok = 4 <= action <= 9
    elif what == "basic":
        ok = action < 4
    else:
        ok = names.get((job, action)) == what or STAGES.get((job, action)) == what
    if not ok:
        return False
    return cond_ok(cond, flags)


def cond_ok(cond, f):
    return {
        "": True, "always": True, "clone": f.get("clone"), "tr": f.get("tr"), "untr": not f.get("tr"), "counter": f.get("counter"),
        "hp<40": f.get("hp", 100) < 40, "hp<30": f.get("hp", 100) < 30, "hp>=70": f.get("hp", 100) >= 70, "hp<50": f.get("hp", 100) < 50,
        "spfull": f.get("spfull"), "ambush": f.get("ambush"), "volley": f.get("volley"), "skillguard": f.get("skillguard"),
        "killguard": f.get("killguard"),
    }[cond]


def check_effects(spec, log, job, names, baseline=False):
    """Every traced hook value against SPEC; returns (problems, exercised by key, notes)."""
    ev = events(log)
    probs, hits = [], {}
    ex = {}

    def mark(key, n=1):
        ex[key] = ex.get(key, 0) + n

    twice_actions = {a for (j, a), n in names.items() if j == job and n in spec.get("twice", [])}
    last_hit = None
    last_taken = None
    counter_cast = None  # (frame, the game's length) of the last 반격
    kills = basic_hits = skill_hits = taken_hits = drinks = 0
    stat_counts = {}
    dash_casts = forced = 0
    drunk_from = None
    second_casts = 0
    sources = {}
    for f, kind, d, raw in ev:
        if f < WORN_FROM:
            continue
        b = d.get("body", "")
        if kind == "kill":
            kills += 1
            continue
        if kind == "second":
            second_casts += 1
            if d["action"] in twice_actions:
                mark("twice")
            else:
                probs.append(f"f={f} second cast of action {d['action']} not in twice")
            continue
        if kind == "hit" and b.startswith("hit final"):
            if last_hit is not None:
                last_hit["dealt"] = int(b.split()[-1])
            continue
        if kind == "hit":
            base, dealt = (int(x) for x in re.match(r"hit (-?\d+) -> (-?\d+)", b).groups())
            action, pct, flat, table = d["action"], d["pct"], d["flat"], d["table"]
            exp = 100
            for sel, add in spec.get("hit", []):
                if sel_match(sel, job, action, d, names):
                    exp += d.get("gear", 0) // 2 if add == "gear/2" else add
            if exp != 100:
                mark("hit")
            exp_flat = 0
            if spec.get("flat") == "wil/2":
                exp_flat = None  # 의지 is not in the line: checked as flat > 0
                if flat > 0:
                    mark("flat")
                else:
                    probs.append(f"f={f} hit flat {flat} (의지/2 expected)")
            if pct != exp:
                probs.append(f"f={f} hit action {action} pct {pct} != {exp} ({b})")
            if exp_flat is not None and flat != exp_flat:
                probs.append(f"f={f} hit flat {flat} != 0")
            want = scale(scale(base, pct) + flat, table)
            if dealt != want:
                probs.append(f"f={f} hit {base} -> {dealt} != {want} (pct {pct} flat {flat} table {table})")
            last_hit = dict(d, dealt=dealt)
            if action < 4:
                basic_hits += 1
            else:
                skill_hits += 1
            continue
        if kind == "crit":
            m = re.match(r"crit (-?\d+) -> (-?\d+)", b)
            game, out = int(m[1]), int(m[2])
            if "forced" in b:
                forced += 1
                if not spec.get("dashcrit"):
                    probs.append(f"f={f} forced crit without 그림자 걸음")
                else:
                    mark("dashcrit")
                continue
            plus = sum(p for c, p in spec.get("crit", []) if (c == "basic" and d.get("action", 0) < 4) or (c != "basic" and cond_ok(c, d)))
            if plus:
                mark("crit")
            if out != game + plus:
                probs.append(f"f={f} crit {game} -> {out} != {game + plus} ({b})")
            continue
        if kind == "taken":
            m = re.match(r"taken (-?\d+) -> (-?\d+) sp (-?\d+)", b)
            game, out, to_sp = int(m[1]), int(m[2]), int(m[3])
            if game < 0:
                taken_hits += 1
                cut = sum(c for cond, c in spec.get("taken", []) if cond_ok(cond, d))
                exp = min(scale(game, max(10, 100 - cut)), -1) if cut else game
                if cut:
                    mark("taken")
                if out != exp:
                    probs.append(f"f={f} taken {game} -> {out} != {exp} ({b})")
                last_taken = out
            continue
        if kind == "sp":
            m = re.match(r"sp (\d+) (-?\d+) -> (-?\d+)", b)
            action, cost, out = int(m[1]), int(m[2]), int(m[3])
            if "replay" in b:
                if action not in twice_actions:
                    probs.append(f"f={f} SP 0 for a replay of action {action} not in twice")
                continue
            cut = sum(c for sel, c in spec.get("sp", []) if sel_match(sel, job, action, d, names))
            exp = scale(cost, max(0, 100 - cut))
            if cut and cost:
                mark("sp")
            if names.get((job, action)) == "신속":
                dash_casts += 1
            if out != exp:
                probs.append(f"f={f} SP action {action} {cost} -> {out} != {exp}")
            continue
        if kind == "cast":
            action = int(re.match(r"cast (\d+)", b)[1])
            tw = d.get("twice")
            if tw != (action in twice_actions):
                probs.append(f"f={f} cast action {action} twice {tw} (expected {action in twice_actions})")
            continue
        if kind == "cooldown":
            m = re.match(r"cooldown (job|spirit) (-?\d+) (-?\d+) -> (-?\d+)", b)
            which, skill, frames, out = m[1], int(m[2]), int(m[3]), int(m[4])
            if which == "job":
                cut = sum(c for sel, c in spec.get("cd", []) if sel_match(sel, job, skill, d, names))
            else:
                cut = spec.get("spirit_cd", 0)
            exp = scale(frames, max(10, 100 - cut))
            if cut:
                mark("cd" if which == "job" else "spirit_cd")
            if out != exp:
                probs.append(f"f={f} cooldown {which} {skill} {frames} -> {out} != {exp}")
            continue
        if kind == "duration":
            m = re.match(r"duration (\w+)(?: status (-?\d+))? (-?\d+) -> (-?\d+)", b)
            what, status, game, out = m[1], m[2], int(m[3]), int(m[4])
            want = spec.get("duration", {}).get(what)
            if what == "mark" and int(status) != 17:
                want = None
            exp = game if want is None else (COUNTER_FOREVER if want == "forever" else scale(game, want))
            if what == "clone":
                exp = max(-32768, min(32767, exp))
            if what == "counter":
                # The hook's number is not enough (2026-11-03: −1 passed this check while the game's `aq` ended the
                # status on its first frame — no reflect at all): a longer 반격 counts once a reflect lands after the
                # game's own length since the cast (the reflect branch below).
                counter_cast = (f, game)
            elif want is not None:
                mark("duration")
            if out != exp:
                probs.append(f"f={f} duration {what} {game} -> {out} != {exp}")
            continue
        if kind == "spirit":
            if b.startswith("spirit power"):
                m = re.match(r"spirit power (-?\d+) -> (-?\d+) pct (-?\d+)", b)
                game, out, pct = int(m[1]), int(m[2]), int(m[3])
                exp_pct = 100 + SPIRIT_BONUS + spec.get("spirit_power", 0)
                if spec.get("spirit_power"):
                    mark("spirit_power")
                if pct != exp_pct or out != scale(game, exp_pct):
                    probs.append(f"f={f} spirit power {game} -> {out} pct {pct} != {exp_pct}")
            else:
                m = re.match(r"spirit cost (-?\d+) cut (-?\d+) -> (-?\d+)", b)
                cost, game, out = int(m[1]), int(m[2]), int(m[3])
                pct = spec.get("spirit_cost", 0)
                exp = min(game + scale(cost, pct), max(cost, game)) if pct else game
                if pct:
                    mark("spirit_cost")
                if out != exp:
                    probs.append(f"f={f} spirit cost {cost} cut {game} -> {out} != {exp}")
            continue
        if kind == "reflect":
            m = re.match(r"reflect (-?\d+) -> (-?\d+) leech (-?\d+)", b)
            game, out, heal = int(m[1]), int(m[2]), int(m[3])
            exp = scale(game, 100 + spec.get("reflect", 0))
            exp_heal = max(1, scale(exp, spec["leech"])) if spec.get("leech") else 0
            if spec.get("reflect"):
                mark("reflect")
            if spec.get("leech"):
                mark("leech")
            if spec.get("duration", {}).get("counter") is not None and counter_cast is not None \
                    and f - counter_cast[0] > counter_cast[1] * PROBE_FRAMES_PER_DRAW + 40:
                mark("duration")  # still reflecting after the game's 반격 would have ended
            if out != exp or heal != exp_heal:
                probs.append(f"f={f} reflect {game} -> {out} leech {heal} != {exp} / {exp_heal}")
            continue
        if kind == "chance":
            m = re.match(r"chance (\w+) (-?\d+) -> (-?\d+)", b)
            which, game, out = m[1], int(m[2]), int(m[3])
            ch = spec.get("chance")
            exp = game
            if ch and ch[0] == which:
                exp = scale(game, 200) if ch[1] == "x2" else game + 30
                mark("chance")
            if out != exp:
                probs.append(f"f={f} chance {which} {game} -> {out} != {exp}")
            continue
        if kind == "poison":
            m = re.match(r"poison (-?\d+) -> (-?\d+)", b)
            game, out = int(m[1]), int(m[2])
            exp = scale(game, spec.get("poison", 100))
            if spec.get("poison"):
                mark("poison")
            if out != exp:
                probs.append(f"f={f} poison {game} -> {out} != {exp}")
            continue
        if kind == "knock":
            m = re.match(r"knock (-?\d+) extra (-?\d+)", b)
            dmg, extra = int(m[1]), int(m[2])
            add = spec.get("knock", 0) if job == BY else 0
            exp = max(1, scale(dmg, add)) if add else 0
            if add:
                mark("knock")
            if extra != exp:
                probs.append(f"f={f} knock {dmg} extra {extra} != {exp}")
            continue
        if kind == "knockable":
            out = b.rstrip().endswith("false")
            if spec.get("knockable") is False:
                mark("knockable")
                if not out:
                    probs.append(f"f={f} knockable: pushed with 철벽 총검갑 ({b})")
            continue
        if kind == "step":
            m = re.match(r"step (-?\d+) -> (-?\d+) walk (\w+)", b)
            walk = m[3]
            if walk == "off":
                # A step that is not the walking 8 px (the hook leaves those alone — 동화 walks 16).
                continue
            want = spec.get("walk")
            if want == "fast":
                ok = walk == "Fast"
            elif want == "brisk":
                ok = walk == ("game" if d.get("tr") else "Brisk")
            elif want == "drunk":
                ok = walk == ("Fast" if drunk_from is not None and f < drunk_from + 820 else "game")
            else:
                ok = walk == "game"
            if want and walk != "game":
                mark("walk")
            if not ok:
                probs.append(f"f={f} step walk {walk} ({b})")
            continue
        if kind == "drank":
            drinks += 1
            drunk_from = f
            continue
        if kind == "drop":
            m = re.match(r"drop luck (\d+) round (\d+)", b)
            luck, rnd = int(m[1]), int(m[2])
            exp = spec.get("luck", 1) if rnd >= 3 else 1
            if spec.get("luck"):
                mark("luck")
            if luck != exp:
                probs.append(f"f={f} drop luck {luck} != {exp}")
            continue
        if kind == "stat":
            m = re.match(r"stat (\w+) (\d+) (-?\d+) -> (-?\d+) delta (-?\d+) top (-?\d+)", b)
            why, idx, before, after, delta, top = m[1], int(m[2]), int(m[3]), int(m[4]), int(m[5]), int(m[6])
            if why == "combo":
                continue
            evs = {e[0]: e for e in spec.get("events", [])}
            if why not in evs:
                probs.append(f"f={f} stat event {why} not promised ({b})")
                continue
            _, e_idx, (how, n), _ = evs[why]
            if idx != e_idx:
                probs.append(f"f={f} stat {why} on index {idx} not {e_idx}")
            exp = {"flat": lambda: n, "top_pct": lambda: max(1, scale(top, n)),
                   "dealt_pct": lambda: max(1, scale(last_hit["dealt"], n)) if last_hit else None,
                   "taken_pct": lambda: max(1, scale(-last_taken, n)) if last_taken else None}[how]()
            if exp is not None and delta != exp:
                probs.append(f"f={f} stat {why} delta {delta} != {exp}")
            if after != max(0, min(top, before + delta)):
                probs.append(f"f={f} stat {why} {before} + {delta} -> {after} (top {top})")
            stat_counts[why] = stat_counts.get(why, 0) + 1
            mark("events")
            continue
        if kind == "shave":
            m = re.match(r"shave (\w+) (\d+) \[(.*?)\] -> \[(.*?)\]", b)
            why, frames = m[1], int(m[2])
            before = [int(x) for x in m[3].split(",") if x.strip()]
            after = [int(x) for x in m[4].split(",") if x.strip()]
            if not spec.get("shave") or spec["shave"][0] != why:
                probs.append(f"f={f} shave {why} not promised")
                continue
            if frames != spec["shave"][1] or after != [max(0, x - frames) for x in before]:
                probs.append(f"f={f} shave {why} {frames} {before} -> {after}")
            stat_counts[why] = stat_counts.get(why, 0) + 1
            mark("shave")
            continue
        if kind == "fusion":
            m = re.match(r"fusion (\w+) (-?\d+) -> (-?\d+)", b)
            why, left, after = m[1], int(m[2]), int(m[3])
            fa = spec.get("fusion_add")
            if not fa or fa[0] != why:
                probs.append(f"f={f} fusion {why} not promised")
                continue
            if after != min(left + fa[1], max(1200, left)):
                probs.append(f"f={f} fusion {why} {left} -> {after}")
            stat_counts[why] = stat_counts.get(why, 0) + 1
            mark("fusion_add")
            continue
        if kind == "killguard":
            if not spec.get("killguard"):
                probs.append(f"f={f} kill guard without 불굴의 갑주")
            mark("killguard")
            continue
        if kind == "frenzy":
            skipped = b.rstrip().endswith("true")
            if skipped != bool(spec.get("frenzy_skip")):
                probs.append(f"f={f} frenzy skipped {skipped}")
            if skipped:
                mark("frenzy_skip")
            continue
        if kind == "bash":
            if not spec.get("bash_wide"):
                probs.append(f"f={f} wide 방패치기 without 울림의 철퇴")
            mark("bash_wide")
            continue
        if kind == "drain":
            removed, heal, pct, source, hp = d["removed"], d["heal"], d["pct"], d["source"], d["hp"]
            if not spec.get("drain"):
                probs.append(f"f={f} drain without a draining piece")
            elif pct != spec["drain"] or heal != (max(1, scale(removed, pct)) if removed > 0 else 0):
                probs.append(f"f={f} drain {removed} at {pct} -> {heal}")
            elif removed > d["dealt"] or removed < 0:
                probs.append(f"f={f} drain removed {removed} of {d['dealt']}")
            sources[source] = sources.get(source, 0) + 1
            if removed < d["dealt"]:
                sources["overkill"] = sources.get("overkill", 0) + 1
            mark("drain")
            continue
        if kind == "smash":
            if not spec.get("smash_wide"):
                probs.append(f"f={f} wide 강타 without 분쇄자의 대검")
            mark("smash_wide")
            continue
    # Counts that the trace's events promise.
    want_counts = {}
    for why, _, _, when in spec.get("events", []):
        want_counts[why] = {"basic": basic_hits, "skillhit": skill_hits, "kill": kills, "taken": taken_hits, "drank": drinks}[when]
    if spec.get("shave"):
        why = spec["shave"][0]
        want_counts[why] = basic_hits // 10 if why == "TimeThief" else kills
    for why, n in want_counts.items():
        if stat_counts.get(why, 0) != n:
            probs.append(f"{why}: {stat_counts.get(why, 0)} events for {n} occasions")
    if spec.get("dashcrit") and forced > dash_casts:
        probs.append(f"{forced} forced crits for {dash_casts} 신속 casts")
    if spec.get("twice") and second_casts == 0:
        pass  # reported as not exercised below
    keys = {k for k in spec if k not in ("job", "focus")}
    alias = {"twice": "twice", "chance": "chance", "events": "events"}
    missing = sorted(k for k in keys if not ex.get(alias.get(k, k)) and not (k == "flat" and ex.get("flat")))
    counts = dict(kills=kills, basic=basic_hits, skill=skill_hits, taken=taken_hits, drinks=drinks, second=second_casts)
    if sources:
        counts["drain"] = sources
    return probs, ex, missing, counts


NAMES = {}


def cmd_effects(a):
    global NAMES
    NAMES = skill_names()
    legends = legend_table()
    by_effect = {l["effect"]: l for l in legends}
    unknown = set(by_effect) - set(SPEC)
    assert not unknown, f"no SPEC for {sorted(unknown)}"
    # Every skill a SPEC names is the game's.
    for eff, s in SPEC.items():
        for key in ("twice", "hit", "sp", "cd"):
            for item in s.get(key, []):
                name = (item if isinstance(item, str) else item[0]).partition("@")[0]
                if name not in ("any", "skill", "job", "basic") and (s["job"], next((a for (j, a), n in NAMES.items() if j == s["job"] and n == name), None)) not in NAMES:
                    raise SystemExit(f"{eff}: {name!r} is no skill of job {s['job']}")
    # `--only NAME` (effect or item name), `NAME:JOB` to wear it as another job.
    only = dict((x.partition(":")[0], x.partition(":")[2]) for x in a.only.split(",") if x)
    todo = [(None, j) for j in range(6)] if not only else []
    for e, piece in by_effect.items():
        pick = None if not only else only.get(e, only.get(piece["name"]))
        if only and pick is None:
            continue
        todo.append((piece, int(pick) if pick else SPEC[e]["job"]))

    def one(item):
        piece, job = item
        spec = SPEC[piece["effect"]] if piece else {}
        hit_me = any(k in spec for k in ("taken", "killguard", "reflect", "leech", "focus")) or any(e[3] == "taken" for e in spec.get("events", []))
        args, frames = fight_plan(job, (piece["cat"], piece["id"]) if piece else None, hit_me, spec.get("focus"))
        name = (piece["effect"] + ("" if job == SPEC[piece["effect"]]["job"] else f"_j{job}")) if piece else f"base_j{job}"
        log, code, wall = probe(args, os.path.join(OUT, "effects", name), {"HZ_BENCH": "1"}, frames=frames)
        return piece, job, log, code, wall

    results = []
    t0 = time.time()
    with concurrent.futures.ThreadPoolExecutor(max_workers=a.j) as ex:
        for piece, job, log, code, wall in ex.map(one, todo):
            spec = SPEC[piece["effect"]] if piece else {}
            probs, exd, missing, counts = check_effects(spec, log, job, NAMES)
            errs = guest_errors(log)
            died = len(re.findall(r"stats HP 0/", log))
            label = f"{piece['name']} ({piece['cat']},{piece['id']}) {piece['effect']}" if piece else f"기준 {JOB_NAMES[job]} (85제 없음)"
            state = "OK" if not probs and not missing and not errs and code == 0 else ("FAIL" if probs or errs or code else "NOT EXERCISED")
            results.append(dict(label=label, job=job, state=state, probs=probs, missing=missing, errs=errs, counts=counts, ex=exd,
                                text=piece["text"] if piece else "", died=died, wall=wall))
            print(f"[{len(results):3}/{len(todo)}] {state:13} {JOB_NAMES[job]} {label} — {exd} {counts}"
                  + (f" died {died}" if died else "") + f" {wall:.0f} s", flush=True)
            for p in probs[:6]:
                print("      ", p)
            if len(probs) > 6:
                print(f"       … {len(probs) - 6} more")
            if missing:
                print("       not exercised:", missing)
            for e in errs[:3]:
                print("       ERR", e[:200])
    with open(os.path.join(OUT, "effects.json" if not only else "effects_only.json"), "w", encoding="utf-8") as f:
        json.dump(results, f, ensure_ascii=False, indent=1)
    bad = [r for r in results if r["state"] != "OK"]
    print(f"effects: {len(results) - len(bad)} OK, {sum(r['state'] == 'FAIL' for r in bad)} FAIL, "
          f"{sum(r['state'] == 'NOT EXERCISED' for r in bad)} not exercised — {time.time() - t0:.0f} s")
    return len(bad)


# ------------------------------------------------------------------------------------------- 3 roundtrip
BAG_ITEM = re.compile(r"bag-item (\S+) (\d+) (-?\d+) (-?\d+) (.*)$")


def bag_items(log):
    return [(m[1], int(m[3]), int(m[4]), m[5].strip()) for m in (BAG_ITEM.search(l) for l in log.splitlines()) if m]


def cmd_roundtrip(a):
    legends = legend_table()
    pieces = sorted((l["cat"], l["id"]) for l in legends) + [(c, 40) for c in range(16)]
    rms = os.path.join(OUT, "roundtrip_rms")
    if os.path.isdir(rms):
        import shutil
        shutil.rmtree(rms)
    give = ",".join(f"7800:{c}:{i}" for c, i in pieces)
    log_a, code_a, _ = probe(["--keys", ",".join(prelude()), "--give", give, "--bags", "7900", "--save-ask", "8000", "--rms-out", rms],
                             os.path.join(OUT, "roundtrip_save"), frames=8200)
    saved = re.search(r"SAVE tab -> (\w+)", log_a)
    log_b, code_b, _ = probe(["--rms-in", rms, "--keys", "600:OK,900:OK,1200:OK,1500:OK,1800:OK", "--bags", "2300"],
                             os.path.join(OUT, "roundtrip_load"), frames=2400)
    before, after = bag_items(log_a), bag_items(log_b)
    new = [x for x in before if (x[1], x[2]) in set(pieces)]
    print(f"saved: {saved[1] if saved else 'no save line'}; given {len(pieces)}, in the bag/storage before the save {len(new)} "
          f"(of {len(before)} pieces), after the load {len(after)}")
    places = {}
    for p, *_ in before:
        places[p] = places.get(p, 0) + 1
    print("   where:", places)
    problems = 0
    if before != after:
        problems += 1
        for i, (x, y) in enumerate(zip(before, after)):
            if x != y:
                print(f"   DIFF {i}: {x} | {y}")
        if len(before) != len(after):
            print(f"   DIFF count {len(before)} -> {len(after)}")
    if len(new) != len(pieces):
        problems += 1
        print("   missing:", sorted(set(pieces) - {(c, i) for _, c, i, _ in new}))
    for log, code in ((log_a, code_a), (log_b, code_b)):
        errs = guest_errors(log)
        problems += len(errs) + (code != 0)
        for e in errs[:5]:
            print("   ERR", e[:200])
    if not saved or saved[1] != "Saved":
        problems += 1
    print("roundtrip:", "ALL OK — every field and option the same after the game's save and load" if problems == 0 else f"{problems} problem(s)")
    return problems


# ------------------------------------------------------------------------------------------- main
def main():
    sys.stdout.reconfigure(encoding="utf-8")
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("records")
    r.add_argument("--jobs", default="0,1,2,3,4,5")
    e = sub.add_parser("effects")
    e.add_argument("--only", default="")
    e.add_argument("-j", type=int, default=12)
    sub.add_parser("roundtrip")
    a = ap.parse_args()
    if a.cmd == "records":
        sys.exit(1 if cmd_records(a) else 0)
    if a.cmd == "effects":
        sys.exit(1 if cmd_effects(a) else 0)
    sys.exit(1 if cmd_roundtrip(a) else 0)


if __name__ == "__main__":
    main()
