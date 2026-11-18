"""영웅서기 Zero end-game survey (HEROZERO_GAME_OPTIONS_RE.md §18): monsters per map per round (노말 0 / 하드코어 1 / 헬 2), drops, quest rewards.

usage:  python -I tools_re/hz_endgame.py <herozero.jar>     (read-only)

Formulas (javap 2026-10-28, probe-checked at rounds 0..2 on maps 109 and 119):
  cq.i()V  r = min(ed.h, 2); r > 0: n = (r+1)*n0/5 + (40|60); d,e(DEF) *= n/n0; HP = (HP-70)*(n+(20|40))/n0;
           ATK = (ATK-13)*n/n0 + (20|40); g = (g-10)*n/n0.   then q(): non-boss +k (k 0..2): n+k, HP+20k, ATK+3k
  cq.n()V  kind 81..97: 100 EXP; else 18*n*clamp(20+n-player,0,30)/20
  cq.o()V  slot 0/1 id = min(n/4,14); slot 2 (n>13) id = min(n/5+13,28); recipe 18: min(3*kind/10,27)
"""
import struct, sys, zipfile
sys.path.insert(0, r"D:\tmp\heroZeroNativeAPP\tools_re")
import hz_maps

sys.stdout.reconfigure(encoding="utf-8")
JAR = sys.argv[1]
z = zipfile.ZipFile(JAR)


def jdiv(a, b):
    q = abs(a) // abs(b)
    return q if (a >= 0) == (b > 0) else -q


def recs(data):
    out, off = [], 0
    while off + 2 <= len(data):
        n = struct.unpack(">H", data[off:off + 2])[0]
        out.append(data[off + 2:off + 2 + n]); off += 2 + n
    return out


MON = []
for i, r in enumerate(recs(z.read("en/edf"))):
    nl = r[0]
    name = r[1:1 + nl].decode("euc-kr", "replace")
    p = 1 + nl + 15
    resp, b, de = struct.unpack(">hhh", r[p:p + 6]); p += 6
    lvl = r[p]; p += 1
    hp, atk, f = struct.unpack(">hhh", r[p:p + 6])
    MON.append(dict(kind=i, name=name, lvl=lvl, hp=hp, atk=atk, de=de, b=b, f=f, resp=resp))


def boss(kind):
    return kind >= 99


def scaled(m, r):
    n0, hp, atk, de = m["lvl"], m["hp"], m["atk"], m["de"]
    if r == 0:
        return n0, hp, atk, de
    n = jdiv((r + 1) * n0, 5) + (40 if r == 1 else 60)
    x = 20 if r == 1 else 40
    return n, jdiv((hp - 70) * (n + x), n0), jdiv((atk - 13) * n, n0) + x, jdiv(de * n, n0)


def ranges(m, r):
    n, hp, atk, de = scaled(m, r)
    k = 0 if boss(m["kind"]) else 2
    return (n, n + k), (hp, hp + 20 * k), (atk, atk + 3 * k), de


def kill_exp(kind, n, player):
    if 81 <= kind <= 97:
        return 100
    d = max(0, min(30, 20 + n - player))
    return 18 * n * d // 20


maps = hz_maps.load(JAR)
ROUND = ["노말", "하드코어", "헬"]

print("## 1. 몬스터 맵 — 회차별 레벨(무작위 +0..2 포함)·HP·공격·방어")
rows = []
for num, m in sorted(maps.items()):
    if not m.monster_kinds:
        continue
    kinds = [k for k in m.monster_kinds if k < len(MON)]
    cells = []
    for r in range(3):
        lo = min(ranges(MON[k], r)[0][0] for k in kinds)
        hi = max(ranges(MON[k], r)[0][1] for k in kinds)
        cells.append(f"{lo}-{hi}")
    names = ", ".join(f"{MON[k]['name']}({k})" for k in kinds)
    rows.append((num, m.name.strip(), m.name2.strip(), cells, names))
    print(f"{num:3d} {m.name.strip()} {m.name2.strip():<10} | {' | '.join(cells)} | {names}")

print("\n## 2. 회차별 꼭대기 몬스터")
for r in range(3):
    top = sorted(((ranges(MON[k], r)[0][1], k) for mm in maps.values() for k in mm.monster_kinds if k < len(MON)), reverse=True)[:8]
    print(ROUND[r], [(lv, MON[k]["name"], k) for lv, k in dict.fromkeys(top)])

print("\n## 3. 보스 (회차별 LV · HP · 공격 · 방어)")
for num, m in sorted(maps.items()):
    if 121 <= num <= 134:
        for k in m.monster_kinds:
            mo = MON[k]
            cells = []
            for r in range(3):
                n, hp, atk, de = scaled(mo, r)
                cells.append(f"LV{n} HP{hp} 공{atk} 방{de}")
            print(f"{num} {m.name.strip()} {mo['name']}({k}) | " + " | ".join(cells))

print("\n## 4. 처치 EXP — 회차 꼭대기 몬스터 vs 플레이어 LV")
for r in range(3):
    best = max((ranges(MON[k], r)[0][1], k) for mm in maps.values() for k in mm.monster_kinds if k < 81 or 97 < k < 99)
    n, k = best
    print(ROUND[r], f"최고 일반 몬스터 LV{n} {MON[k]['name']}:",
          ", ".join(f"P{p}={kill_exp(k, n, p)}" for p in (40, 50, 60, 64, 65, 70, 78, 79, 80, 90, 98, 99)),
          f"| EXP 0 이 되는 플레이어 LV = {n + 20}")

print("\n## 5. 드랍 번호 — 회차별 몬스터 LV 에서")
for r in range(3):
    lv = sorted({ranges(MON[k], r)[0][1] for mm in maps.values() for k in mm.monster_kinds if k < 99})
    print(ROUND[r], f"LV {lv[0]}..{lv[-1]}: 일반 id ≤ {min(lv[-1] // 4, 14)}, 레어 대역 id {min(lv[0] // 5 + 13, 28)}..{min(lv[-1] // 5 + 13, 28)}")

print("\n## 6. 조합서 → 그것을 떨어뜨리는 몬스터 종류 (id = 3*kind/10)")
items18 = [r for r in recs(z.read("itm/18"))]
for rid in range(28):
    ks = [k for k in range(len(MON)) if min(3 * k // 10, 27) == rid]
    where = sorted({num for num, mm in maps.items() for k in mm.monster_kinds if k in ks})
    nm = items18[rid][1:1 + items18[rid][0]].decode("euc-kr", "replace").strip("}^|& ;")
    print(f"  조합서 {rid:2d} {nm:<10} 종류 {ks[0]}..{ks[-1]} 맵 {where}")

print("\n## 7. 대본 — 회차별 보상(동작 16 아이템·17 돈·20 EXP·12 반납)·회차 조건(8)")
tot = {16: [[], [], []], 17: [0, 0, 0], 20: [0, 0, 0]}
for num, m in sorted(maps.items()):
    for li, code in enumerate(m.actions):
        for op, a in hz_maps.walk_actions(code, [], ""):
            if op in (12, 16):
                trip = [tuple(struct.unpack("bbb", a[3 * r:3 * r + 3])) for r in range(3)]
                print(f"  map {num:3d} list {li:2d} {'반납' if op == 12 else '보상'} " + " / ".join(f"{ROUND[r]} {t}" for r, t in enumerate(trip)))
                if op == 16:
                    for r in range(3):
                        tot[16][r].append(trip[r])
            elif op == 17:
                v = struct.unpack("<hhh", a)
                print(f"  map {num:3d} list {li:2d} 돈 {v}")
                for r in range(3):
                    tot[17][r] += v[r]
            elif op == 20:
                v = struct.unpack("<HHH", a)
                print(f"  map {num:3d} list {li:2d} EXP {v}")
                for r in range(3):
                    tot[20][r] += v[r]
print("합계 돈", tot[17], "EXP", tot[20])
