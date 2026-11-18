"""Class balance model: every job's derived stats and skill numbers at the rotation bench's four tiers
(tools/hz_rotation.py), from the measured formulas (HEROZERO_BALANCE_PLAN.md §2 with the 1st pass
HZ-039..047 — `hz_java::stats::PATCH`, `hz_java::tables`) and the bench's gear. No game is run: this is
the arithmetic the bench's numbers are checked against (HEROZERO_CLASS_BALANCE.md).

    python -I tools/hz_balance_model.py
"""

JOBS = ["워리어", "로그", "나이트", "건너", "총검사", "정령사"]
# The bench's builds (힘 · 민첩 · 건강 · 의지 in tenths of the points).
SPLIT = [(6, 0, 4, 0), (2, 6, 2, 0), (5, 0, 5, 0), (2, 6, 2, 0), (5, 2, 3, 0), (0, 2, 4, 4)]
# hz_java::stats::PATCH
STR_PCT = [125, 75, 100, 75, 125, 100]
DEX_DIV = [4, 3, 4, 6, 4, 4]
CRIT_DEX_DIV, CRIT_GUNNER_DIV, CRIT_CAPS = 10, 20, (35, 50, 60)
EVADE_DEX_DIV, EVADE_ROGUE_DIV, EVADE_CAPS = 8, 16, (40, 50, 60)
HP_PER_CON = 14
HIT_FLOOR = 30

# Weapon (min, max, crit h) and armour defence by tier — tools_re/hz_items.py (ids 22 / 30·28 / 40 guardian
# ×1.1 for the 85제). Category: 0 도끼 1 양손검 2 도검 3 둔기 4 단검 5 총 6 총검 7 방패(+def) · 8 경장갑 9 중장갑
# 10 투구 12 경갑옷 13 중갑옷 14 경신발 15 중신발.
WEAPON = {
    1: {0: (77, 309, 16), 1: (167, 251, 13), 2: (108, 144, 6), 3: (62, 145, 8), 4: (59, 72, 22), 5: (47, 71, 14), 6: (81, 151, 13), 7: (61, 92, 13)},
    2: {0: (108, 435, 16), 1: (242, 364, 13), 2: (170, 204, 6), 3: (77, 231, 8), 4: (83, 102, 22), 5: (73, 109, 14), 6: (132, 198, 13), 7: (92, 138, 13)},
    3: {0: (135, 544, 16), 1: (303, 455, 13), 2: (213, 255, 6), 3: (96, 289, 8), 4: (104, 128, 22), 5: (91, 136, 14), 6: (165, 248, 13), 7: (115, 173, 13)},
}
WEAPON[4] = {c: (round(a * 1.1), round(b * 1.1), round(h * 1.1)) for c, (a, b, h) in WEAPON[3].items()}
SHIELD_DEF = {1: 23, 2: 30, 3: 38, 4: 42}
ARMOUR = {
    1: {8: 14, 9: 18, 10: 22, 12: 31, 13: 41, 14: 16, 15: 19},
    2: {8: 22, 9: 28, 10: 31, 12: 56, 13: 70, 14: 28, 15: 34},
    3: {8: 28, 9: 35, 10: 39, 12: 70, 13: 88, 14: 35, 15: 43},
}
ARMOUR[4] = {c: round(d * 1.1) for c, d in ARMOUR[3].items()}
TIER_LEVEL = {1: 45, 2: 65, 3: 99, 4: 99}
# Monsters the bench meets (map 44, --mobs): round 1 · 2 · 3.
# The slime (kind 39, three of four) and the 평신도 (kind 1): LV, HP, ATK, DEF — 2026-10-29 bench logs.
MONSTER = {1: (41, 740, 103, 40), 2: (61, 1220, 163, 60), 3: (86, 1760, 233, 85)}
MONSTER_BIG = {1: (43, 1585, 121, 41), 2: (63, 2598, 188, 61), 3: (88, 3738, 266, 86)}

# The job's weapon slots and armour kind, as the bench wears them.
WEAPONS = {0: (0, None), 1: (4, 4), 2: (2, 7), 3: (5, 5), 4: (6, None), 5: (4, None)}
HEAVY = {0: True, 1: False, 2: True, 3: False, 4: True, 5: False}

# Skills at L5 (HZ-039 tables, the game's own where untouched): name, kind, (A, B, C), SP, cooldown frames.
# kind: dmg = attack × (A+B·L)/C + roll · stun/curse/buff = frames · heal = maxHP × … · other.
SKILLS = {
    0: [("강타", "dmg", (11, 3, 10), 40, 20), ("충격", "stun", (30, 10, 1), 40, 40), ("돌격", "dmg", (9, 3, 10), 60, 10),
        ("외침", "curse", (40, 20, 1), 60, 70), ("광기", "sp", (0, 1, 3), 10, 10), ("회전베기", "dmg-area", (0, 5, 10), 90, 60)],
    1: [("기습", "crit", (0, 3, 1), 30, 10), ("독수", "poison%", (40, 10, 1), 40, 20), ("연격", "dmg×2", (15, 5, 20), 70, 20),
        ("신속", "dash", (2, 1, 1), 80, 5), ("참격", "dmg", (10, 5, 10), 90, 30), ("분신", "buff", (0, 100, 1), 150, 200)],
    2: [("방패치기", "dmg(방패)", (10, 6, 10), 40, 10), ("회복", "heal", (3, 3, 40), 60, 100), ("방패돌격", "dmg", (10, 4, 10), 60, 20),
        ("반격", "reflect", (5, 5, 10), 90, 180), ("방패밀기", "dmg", (13, 4, 10), 80, 20), ("전력방어", "guard", (1, 1, 1), 120, 300)],
    3: [("근접사격", "dmg(민첩)", (1, 3, 10), 40, 5), ("조준사격", "dmg", (10, 5, 10), 40, 5), ("연사", "dmg", (11, 3, 10), 40, 5),
        ("난사", "dmg-area", (10, 5, 10), 100, 5), ("트릭샷", "mark", (50, 50, 1), 30, 200), ("저격", "kill%", (25, 5, 1), 100, 5)],
    4: [("포격", "dmg-area", (10, 5, 10), 90, 5), ("돌진", "dmg", (10, 4, 10), 90, 5)],
    5: [("맹약", "mp", (0, 10, 1), 45, 5), ("동화", "buff", (0, 100, 1), 180, 200)],
}


def derived(job, tier):
    level = TIER_LEVEL[tier]
    points = 4 * (level - 1)
    s = [10 + points * p // 10 for p in SPLIT[job]]
    str_, dex, con, wil = s
    strength = {125: str_ + str_ // 4, 75: str_ - str_ // 4, 100: str_}[STR_PCT[job]]
    w1, w2 = WEAPONS[job]
    wmin, wmax, wh = WEAPON[tier][w1]
    attack = strength + dex // DEX_DIV[job] + wmin
    crit = min(dex // CRIT_DEX_DIV + (dex // CRIT_GUNNER_DIV if job == 3 else 0), CRIT_CAPS[0])
    crit = min(crit + wh, CRIT_CAPS[1])
    evade = min(dex // EVADE_DEX_DIV + (dex // EVADE_ROGUE_DIV if job == 1 else 0), EVADE_CAPS[0])
    heavy = HEAVY[job]
    arm = ARMOUR[tier]
    defence = sum(arm[c] for c in ((9, 10, 13, 15) if heavy else (8, 10, 12, 14))) + str_ // 10 + (SHIELD_DEF[tier] if w2 == 7 else 0)
    hp = 10 * level + HP_PER_CON * con + (con // 5 if job == 2 else 0)
    sp = 5 * level + 10 * wil
    roll = attack + (wmax - wmin) // 2  # the average basic hit before defence and the critical
    return dict(level=level, stats=s, attack=attack, weapon=(wmin, wmax), crit=crit, evade=evade, defence=defence, hp=hp, sp=sp, roll=roll)


def table():
    out = []
    for tier in (1, 2, 3, 4):
        mlv, mhp, matk, mdef = MONSTER[min(tier, 3)]
        out.append(f"\n### Tier {tier} — LV {TIER_LEVEL[tier]} vs map 44 round {min(tier, 3)} (monster LV {mlv} HP {mhp} ATK {matk} DEF {mdef})\n")
        out.append("| 직업 | 힘·민첩·건강·의지 | 공격 (무기) | 평타 기대 ×(1+크리) | 크리 | 회피 → 몬스터 명중 | 방어 → 한 대 | HP (버틸 대수) | SP |")
        out.append("|---|---|---|---|---|---|---|---|---|")
        for job in range(6):
            d = derived(job, tier)
            hit = max(5, d["roll"] - mdef) * (1 + d["crit"] / 100)
            mhit = max(HIT_FLOOR, 90 + mlv - d["level"] - d["evade"])
            taken = max(1, (max(5, matk * 11 // 10 - d["defence"])) // 3)
            out.append(f"| {JOBS[job]} | {'·'.join(map(str, d['stats']))} | {d['attack']} ({d['weapon'][0]}-{d['weapon'][1]}) | {hit:.0f} | {d['crit']} % | {d['evade']} % → {mhit} % | {d['defence']} → {taken} | {d['hp']} ({d['hp'] // taken}) | {d['sp']} |")
    out.append("\n### 스킬 L5 (HZ-039 뒤) — 피해 배율 = 공격 × (A+B·5)/C + 굴림, SP, 쿨(초)\n")
    out.append("| 직업 | 스킬 | 종류 | L5 값 | SP | 쿨 |")
    out.append("|---|---|---|---|---|---|")
    for job in range(6):
        for name, kind, abc, sp, cd in SKILLS[job]:
            a, b, c = abc
            v = (a + b * 5) / c if c else 0  # the game multiplies the attack first, then divides (db.a(BIZ)I)
            shown = f"×{v:.1f}" if kind.startswith("dmg") else (f"{v:.0%}" if kind == "heal" else f"{v:g}")
            out.append(f"| {JOBS[job]} | {name} | {kind} | {shown} | {sp} | {cd / 20:.2f} |")
    return "\n".join(out)


if __name__ == "__main__":
    print(table())
