"""Rotation bench: every job at four tiers against map 44 (HEROZERO_BALANCE_PLAN.md §6-2, hero2's
class_rotation_probe.py). Each run is a new game (워리어 LV 1) turned into the job, levelled, built,
geared, skilled and warped to map 44 at the tier's replay round, then 20 game seconds of the key script:
basic attacks every game frame with the job's two main skills on the quick slots (1 · 3, and 7 for the
정령사's spirit skill) every 1.5 s. The probe runs on a virtual clock, so a run is the same bytes every
time (tools/hz_watchline.py).

    python -I tools/hz_rotation.py [--exe target/release/hz_probe.exe] [--out dir] [--tiers 1,2,3,4] [--jobs 0,1,...]
                                   [--patches default] [--seconds 20]

What it reads back (HZ_BENCH=1 — `hz: bench hit`, `hz: bench taken`, `hz: bench kill`, the
probe's `stats` lines and `--mobs` snapshots):
  hits      landed hits and the damage the hit hook handed back (before the game's critical ×2)
  skills    of those, the job skills' share
  dmg/s     monster HP taken a second from the --mobs snapshots (what really left the monsters: with
            the critical, after misses — the truest DPS; respawns and monsters out of the snapshot add noise)
  kills     monsters killed
  taken/s   HP lost a second, deaths (HP 0)
  sp        SP at the end against its top (the skill economy)

Tiers: 1 = LV 45 · LV 45 레어 · round 1 (LV 40-43) · 2 = LV 65 · LV 65 조합/세트 · round 2 (LV 60-63) ·
3 = LV 99 · 가디언 장비 · round 3 (가디언 모드 LV 85~) · 4 = LV 99 · 85제 7 pieces · round 3.
"""
import argparse
import os
import random
import re
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FRAMES_A_SECOND = 20 * 4  # a game frame is four probe frames at --ms 20

# job: (name, stat split 힘·민첩·건강·의지 in tenths, weapon categories (slot 3, slot 5), heavy armour, two
# skill ids for quick slots 0·1, spirit skill id for slot 2 or None)
JOBS = {
    0: ("워리어", (6, 0, 4, 0), (0, None), True, (4, 9), None),
    1: ("로그", (2, 6, 2, 0), (4, 4), False, (8, 6), None),
    2: ("나이트", (5, 0, 5, 0), (2, 7), True, (4, 8), None),
    3: ("건너", (2, 6, 2, 0), (5, 5), False, (5, 7), None),
    4: ("총검사", (5, 2, 3, 0), (6, None), True, (4, 5), None),
    5: ("정령사", (0, 2, 4, 4), (4, None), False, (5, 4), 4),
}
# slot -> category: 0 장갑 · 1 머리(투구) · 2 장신구 · 4 갑옷 · 6 신발 (3 · 5 the weapons)
ARMOUR = {True: {0: 9, 1: 10, 2: 11, 4: 13, 6: 15}, False: {0: 8, 1: 10, 2: 11, 4: 12, 6: 14}}

# tier -> (level, round, piece number by category, name)
def tier_gear(tier, job):
    _, _, (w1, w2), heavy, _, _ = JOBS[job]
    cats = dict(ARMOUR[heavy])
    cats[3] = w1
    if w2 is not None:
        cats[5] = w2
    if tier == 1:
        num = lambda c: 14 if c == 11 else 22  # LV 45 레어 (장신구: LV 40 세트 14)
        return 45, 1, {s: (c, num(c)) for s, c in cats.items()}, "LV45 레어"
    if tier == 2:
        def num(c):
            if c == 11:
                return 5
            if c in (8, 9, 14, 15):
                return 28  # LV 60 세트 limbs
            if c == 6:
                return 28  # 크림슨 라이언 LV 65 세트
            return 30  # LV 65 조합/세트
        return 65, 2, {s: (c, num(c)) for s, c in cats.items()}, "LV65 조합·세트"
    if tier == 3:
        return 99, 3, {s: (c, 40) for s, c in cats.items()}, "가디언 장비"
    # Tier 4: 85제, the job's own set (HEROZERO_NEW_ITEM_DRAFT.md §2 · §9).
    legend = {
        0: {3: (0, 41), 0: (9, 43), 1: (10, 41), 2: (11, 42), 4: (13, 41), 6: (15, 41)},  # 선풍의 도끼·연환의 장갑·폭풍의 투구·시간도둑·광전사의 흉갑·돌격대장의 장화
        1: {3: (4, 42), 5: (4, 41), 0: (8, 41), 1: (10, 45), 2: (11, 42), 4: (12, 41), 6: (14, 41)},  # 바람가르개·그림자 쌍검·맹독의 손·집중의 관·시간도둑·밤의 장막·그림자 걸음
        2: {3: (2, 41), 5: (7, 41), 0: (9, 41), 1: (10, 45), 2: (11, 42), 4: (13, 42), 6: (15, 46)},  # 심판의 검·응징의 방패·되갚는 손·집중의 관·시간도둑·가시 판금·기사의 장화
        3: {3: (5, 42), 5: (5, 41), 0: (8, 42), 1: (10, 42), 2: (11, 42), 4: (12, 42), 6: (14, 46)},  # 탄막의 총·사신의 총·사냥꾼의 장갑·매의 눈·시간도둑·바람의 코트·사수의 신발
        4: {3: (6, 41), 0: (9, 42), 1: (10, 43), 2: (11, 42), 4: (13, 43), 6: (15, 43)},  # 연쇄 폭뢰·화약 장갑·포격수의 투구·시간도둑·철벽 총검갑·돌진의 장화
        5: {3: (4, 43), 0: (8, 43), 1: (10, 44), 2: (11, 41), 4: (12, 43), 6: (14, 45)},  # 맹약의 단검·공허의 장갑·정령왕의 관·원소의 반지·영혼의 외투·술사의 신발
    }
    return 99, 3, legend[job], "85제"


def keys(seconds, skills, spirit, seed=None):
    """The fight's keys after the common prelude. Without `seed`, the fixed rhythm of the first benches (OK every
    20 probe frames = 5 game frames, the two skills every 120 in turn, a turn every 40). With `seed`, the same
    kinds of key at jittered times (`random.Random(seed)`, so still the same bytes every time): fixed periods lock
    onto a job's combo cycle — `ek.d(B)` refuses every key while the recovery lock `ek.m` runs, the 정령사's 2-step
    combo (60 frames) halves the 120-frame skill period so its 동화 key always fell in the lock, and OK every 5 game
    frames chains 5-frame steps but never the 정령사's 4-frame transformed ones (HEROZERO_CLASS_BALANCE.md §8.8).
    Jittered: OK every 3..5 game frames (a tapping player — holding OK attacks once), the skills every ~1.5 s each,
    a turn every ~0.55 s; the 정령사 casts 동화 once at the start (it lasts the 30 s; tries until it lands, its 8 s
    cooldown refusing the rest), its HP-into-MP skill (id 4, slot 3) every ~4.5 s and the spirit skill every ~2 s."""
    k = ["300:OK", "400:OK", "500:OK"]
    for i, f in enumerate(range(600, 7401, 60)):
        k.append(f"{f}:{'OK' if i % 2 == 0 else 'HASH'}")
    k += ["7500:DOWN:20", "7560:LEFT:15", "7620:RIGHT:15"]
    # Map 44 (the warp at 7700 into the monsters' cluster at (21, 19)): the level-up notices of --exp-test (one 「# 확인」 holds the field tick), then the
    # build at 8000..8150, the walk to the monsters, then the fight.
    k += [f"{f}:HASH" for f in range(7740, 7990, 8)]
    start = 8400
    end = start + seconds * FRAMES_A_SECOND
    if seed is not None:
        return ",".join(k + jittered(start, end, spirit, seed)), start, end
    f = start
    i = 0
    while f < end:
        k.append(f"{f}:OK")
        if i % 6 == 2:
            k.append(f"{f + 5}:1")
        elif i % 6 == 5:
            k.append(f"{f + 5}:3")
        if spirit is not None and i % 8 == 7:
            k.append(f"{f + 10}:7")
        if i % 2 == 1:
            # A one-game-frame tap turns the player toward the next side (the monsters walk round it).
            k.append(f"{f + 12}:{['LEFT', 'DOWN', 'RIGHT', 'UP'][(i // 2) % 4]}:4")
        f += 20
        i += 1
    return ",".join(k), start, end


def jittered(start, end, spirit, seed):
    """`keys()`'s seeded fight. A turn holds its 4 frames alone (the game holds one key: a press releases it)."""
    rnd = random.Random(seed)
    used = set()
    out = []

    def put(f, key, hold=1):
        while any(g in used for g in range(f, f + hold)):
            f += 1
        if f + hold <= end:
            used.update(range(f, f + hold))
            out.append((f, key))

    t = start + rnd.randrange(20, 60)
    while t < end:
        put(t, f"{rnd.choice(['LEFT', 'DOWN', 'RIGHT', 'UP'])}:4", 4)
        t += rnd.randrange(30, 61)
    if spirit is None:
        t, slot = start + rnd.randrange(20, 60), 0
        while t < end:
            put(t, "13"[slot])
            slot ^= 1
            t += rnd.randrange(40, 81)
    else:
        t = start + rnd.randrange(0, 20)
        for _ in range(5):
            put(t, "1")
            t += rnd.randrange(30, 51)
        t = start + rnd.randrange(200, 360)
        while t < end:
            put(t, "3")
            t += rnd.randrange(320, 401)
        t = start + rnd.randrange(60, 160)
        while t < end:
            put(t, "7")
            t += rnd.randrange(140, 181)
    t = start
    while t < end:
        put(t, "OK")
        t += rnd.choice((12, 16, 20))
    return [f"{f}:{key}" for f, key in sorted(out)]


def run(a, job, tier, seed=None):
    name, split, _, _, skills, spirit = JOBS[job]
    level, rnd, gear, gear_name = tier_gear(tier, job)
    keystr, start, end = keys(a.seconds, skills, spirit, seed)
    total = 4 * 10 + 4 * (level - 1)
    stats = [10 + (total - 40) * s // 10 for s in split]
    actions = []
    actions += ["--round", f"7690:{rnd}", "--warp", "7700:44:21:19", "--job", f"7720:{job}"]
    # n³ − n² + 80n is the EXP to the next level; the sum to `level` (GAME_OPTIONS_RE §18). LV 99 = the cap.
    need = 200_000_000 if level == 99 else sum(n * n * n - n * n + 80 * n for n in range(1, level))
    actions += ["--exp-test", f"7730:{need}"]
    actions += ["--base-stat", ",".join(f"{8000 + 4 * i}:{i}:{v}" for i, v in enumerate(stats))]
    # The six actives at the cap (5, HZ-042) and the job's two passives (10 숙련 = the 4-hit combo, 11) at theirs (3).
    actions += ["--skill-level", ",".join(f"{8020 + 4 * i}:{s}:{5 if s < 10 else 3}" for i, s in enumerate(range(4, 12)))]
    if spirit is not None:
        actions += ["--spirit-level", ",".join(f"{8060 + 4 * i}:{s}:5" for i, s in enumerate(range(0, 10)))]
    actions += ["--equip-test", ",".join(f"{8100 + 4 * i}:{slot}:{c}:{n}" for i, (slot, (c, n)) in enumerate(sorted(gear.items())))]
    quick = [f"8140:0:{skills[0]}", f"8144:1:{skills[1]}"]
    if spirit is not None:
        quick.append(f"8148:2:{spirit}")
    actions += ["--quick-set", ",".join(quick)]
    actions += ["--refill", "8160"]
    actions += ["--mobs", ",".join(str(f) for f in range(start, end + 1, 200))]
    out = os.path.join(a.out, f"j{job}_t{tier}" + ("" if seed is None else f"_s{seed}"))
    os.makedirs(out, exist_ok=True)
    cmd = [a.exe, "--jar", a.jar, "--w", "520", "--h", "240", "--ms", "20", "--frames", str(end + 100), "--every", "100000",
           "--keys", keystr, "--patches", a.patches, "--opt", "loot=on,respawn=6", "--out", out] + actions
    env = dict(os.environ, HZ_BENCH="1")
    t0 = time.time()
    r = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace", env=env)
    log = r.stdout + r.stderr
    with open(os.path.join(out, "log.txt"), "w", encoding="utf-8") as f:
        f.write(" ".join(cmd) + "\n\n" + log)
    return analyse(log, start, end, name, level, gear_name, time.time() - t0, r.returncode)


def analyse(log, start, end, name, level, gear_name, wall, code):
    # Frame numbers on the hook lines are not printed; the probe's `[probe] f=N` lines bracket them.
    f = 0
    hits = skills = dmg = taken = kills = 0
    deaths = 0
    hp_lo = 10 ** 9
    sp_end = sp_top = None
    snaps = []
    for line in log.splitlines():
        m = re.search(r"\[probe\] f=(\d+)", line)
        if m:
            f = int(m[1])
        if not (start <= f <= end):
            continue
        m = re.search(r"bench hit (-?\d+) -> (-?\d+) action (\d+)", line)
        if m:
            hits += 1
            dmg += int(m[2])
            if int(m[3]) >= 4:
                skills += 1
            continue
        m = re.search(r"bench taken (-?\d+)", line)
        if m:
            taken -= int(m[1])
            continue
        if "bench kill" in line:
            kills += 1
            continue
        m = re.search(r"stats HP (\d+)/(\d+)\s+SP (\d+)/(\d+)", line)
        if m:
            hp = int(m[1])
            hp_lo = min(hp_lo, hp)
            if hp == 0:
                deaths += 1
            sp_end, sp_top = int(m[3]), int(m[4])
        m = re.search(r"\[probe\] f=(\d+) (\d+) monsters:", line)
        if m:
            snaps.append((int(m[1]), 0, []))
        m = re.search(r"^\[probe\]\s+(.*LV\s*(\d+).*HP\s*(\d+)/(\d+).*)$", line)
        if m and snaps:
            snaps[-1][2].append(int(m[3]))
    # Monster HP that left between snapshots (only drops count; a respawn or a new monster adds HP).
    lost = 0
    for (f0, _, a), (f1, _, b) in zip(snaps, snaps[1:]):
        n = min(len(a), len(b))
        lost += sum(max(0, x - y) for x, y in zip(a[:n], b[:n]))
    secs = (end - start) / FRAMES_A_SECOND
    errors = len([l for l in log.splitlines() if re.search(r"Uncaught|GUEST ERROR|panicked|FAILED", l)])
    return {
        "job": name, "lv": level, "gear": gear_name, "hits": hits, "skills": skills, "hit_dmg": dmg,
        "dps_hook": dmg / secs, "dps_mobs": lost / secs, "kills": kills, "taken_s": taken / secs, "hp_lo": hp_lo,
        "deaths": deaths, "sp": f"{sp_end}/{sp_top}", "wall": wall, "code": code, "errors": errors,
    }


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--exe", default=os.path.join(ROOT, "target", "release", "hz_probe.exe"))
    ap.add_argument("--jar", default=os.path.join(ROOT, "android_app", "app", "src", "main", "assets", "herozero.jar"))
    ap.add_argument("--out", default=os.path.join(os.environ.get("TEMP", ROOT), "hz-rotation"))
    ap.add_argument("--tiers", default="1,2,3,4")
    ap.add_argument("--jobs", default="0,1,2,3,4,5")
    ap.add_argument("--patches", default="default")
    ap.add_argument("--seconds", type=int, default=30)
    ap.add_argument("--seeds", default="", help="jittered keys, one run per seed (e.g. 1,2,3 — see keys())")
    a = ap.parse_args()
    a.exe, a.jar, a.out = (os.path.abspath(p) for p in (a.exe, a.jar, a.out))
    rows = []
    print("| job | LV | gear | hits (skills) | hook dmg/s | mobs dmg/s | kills | taken/s | HP low | deaths | SP | err |")
    print("|---|---|---|---|---|---|---|---|---|---|---|---|")
    for tier in (int(t) for t in a.tiers.split(",")):
        for job in (int(j) for j in a.jobs.split(",")):
          for seed in [int(x) for x in a.seeds.split(",") if x] or [None]:
            r = run(a, job, tier, seed)
            rows.append(r)
            print(f"| {r['job']}{'' if seed is None else ' s' + str(seed)} | {r['lv']} | {r['gear']} | {r['hits']} ({r['skills']}) | {r['dps_hook']:.0f} | {r['dps_mobs']:.0f} | {r['kills']} | "
                  f"{r['taken_s']:.1f} | {r['hp_lo']} | {r['deaths']} | {r['sp']} | {r['errors']}{'' if r['code'] == 0 else ' exit ' + str(r['code'])} |", flush=True)
    return rows


if __name__ == "__main__":
    main()
