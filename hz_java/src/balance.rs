//! The balance patches' pure parts beside the tables ([`crate::tables`]) and the stat formulas
//! ([`crate::stats`]) — HEROZERO_BALANCE_PLAN.md §4 (user 2026-10-29 「진행하자」, the recommended options):
//!
//! * **HZ-042** — the job skills' level cap 3 → [`ACTIVE_CAP`] (the skill window's `cb.j`, read in its
//!   raise and draw paths; the passives and the spirit window keep theirs).
//! * **HZ-043** — the basic combo pays SP and its hits are weighted ([`COMBO_SP`], [`COMBO_PCT`]).
//! * **HZ-044** — a skill point every even level (the game's) and every tenth, and one for a boss.
//! * **HZ-045** — a boss part's reflection (a fifth of the damage, unmitigated) becomes a tenth,
//!   mitigated like a hit and capped at [`REFLECT_CAP_PCT`] of max HP.
//! * **HZ-046** — a boss drops one piece of its level band, rare / epic / set by [`BOSS_GRADE_PCT`].
//! * **HZ-047** — 전력방어 cuts damage by a percent instead of the game's integer division.

/// HZ-042: the job skills' highest level (the game's `cb.j` is 3; the spirit window `cr` sets 5).
pub const ACTIVE_CAP: i32 = 5;

/// HZ-043: SP a basic hit pays, by combo step `ek.ac[ek.l]` 0..3, and the step's damage percent.
pub const COMBO_SP: [i32; 4] = [4, 5, 6, 8];
pub const COMBO_PCT: [i32; 4] = [100, 80, 80, 120];

/// HZ-043: the percent and SP of combo step `action` (a skill — 4 and up — is untouched).
pub fn combo(action: i8) -> Option<(i32, i32)> {
    usize::try_from(action).ok().filter(|&a| a < 4).map(|a| (COMBO_PCT[a], COMBO_SP[a]))
}

/// HZ-044: the skill points a level-up to `level` grants (the game: one at an even level).
pub fn skill_points_at(level: i32) -> i32 {
    let mut n = 0;
    if level % 2 == 0 {
        n += 1;
    }
    if level % 10 == 0 {
        n += 1;
    }
    // hero2's early curve (user 2026-10-29): one more at every level up to 20.
    if level <= EARLY_LEVELS {
        n += 1;
    }
    n
}

/// HZ-044 (2nd pass): every level up to this one grants an extra point.
pub const EARLY_LEVELS: i32 = 20;
/// HZ-055: max SP counts the level this many times (the game's 5 × level becomes 15 × — hero2's +10 a level).
pub const SP_LEVEL_TIMES: i32 = 3;

/// The skill multipliers the tables cannot hold, applied to the hit's damage in `hitDamage`: 기습 ×1.5 and
/// 근접사격 ×1.5 (2nd pass, HEROZERO_CLASS_BALANCE.md §6-B — an approximation of 「공격 ×2.0 + 민첩 ×0.5」) and the
/// 정령사's transformed hits (actions 6 · 7 while 동화 is up) ×2.5 (3rd pass, §8.5: 580 a hit every 7 s was the
/// job's whole melee).
pub fn skill_pct(job: i8, action: i8) -> i32 {
    match (job, action) {
        (1, 4) | (3, 4) => 150,
        (5, 6) | (5, 7) => 250,
        _ => 100,
    }
}

/// 3rd pass: the spirit's skills ×1.5 for every player (on top of the 85제 pieces' percent) — `spiritPower`.
pub const SPIRIT_POWER_BONUS: i32 = 50;

/// HZ-044: the boss kinds whose death is a boss kill — the heads, not their parts or summons
/// (`/en/edf` 99..131, `cq.a(SSBB)` the factory: 맨이터 99/122, 키메라 101/124, 켈베로스 105/128, 안톤
/// 109/111/113, 알리사 110/112/114, 티모로스 115, 아퀼라 117/118, 거인신 119).
pub const BOSS_HEADS: [i16; 16] = [99, 101, 105, 109, 110, 111, 112, 113, 114, 115, 117, 118, 119, 122, 124, 128];

pub fn is_boss_head(kind: i16) -> bool {
    BOSS_HEADS.contains(&kind)
}

/// HZ-045: what comes back of the damage a boss part took — the game hands the hook a fifth; a tenth.
pub fn reflect(fifth: i32) -> i32 {
    fifth / 2
}
/// HZ-045: a reflection is at most this percent of the player's max HP.
pub const REFLECT_CAP_PCT: i32 = 15;

/// HZ-045: the reflection the player takes for `reflected` raw, with `defence` and `max_hp` — mitigated
/// as a monster's hit is (`− defence`, at least 5, ÷ 3, at least 1), then capped.
pub fn reflected_hit(reflected: i32, defence: i32, max_hp: i32) -> i32 {
    let mitigated = ((reflected - defence).max(5) / 3).max(1);
    let cap = (max_hp * REFLECT_CAP_PCT / 100).max(1);
    mitigated.min(cap)
}

/// HZ-046: a boss drop's grade by a roll in 100: rare / epic / set.
pub const BOSS_GRADE_PCT: [i32; 3] = [50, 35, 15];
/// HZ-046: the level window round the boss's level.
pub const BOSS_BELOW: i32 = 10;
pub const BOSS_ABOVE: i32 = 5;
/// HZ-046: in 가디언 모드 a boss adds an 85제 piece this often (percent).
pub const BOSS_LEGEND_PCT: u32 = 10;

/// Every rare, epic and set piece of number 31 and under: (category, number, grade 1..3, level) —
/// `tools_re/hz_items.py` over `/itm/00..15` (2026-10-29).
pub const GRADED: [(i8, i8, i8, i8); 273] = [
    (0, 15, 1, 15), (0, 16, 1, 20), (0, 17, 1, 27), (0, 18, 1, 30), (0, 19, 1, 35), (0, 20, 3, 40), (0, 21, 2, 43), (0, 22, 1, 45), (0, 23, 2, 48),
    (0, 24, 1, 50), (0, 25, 2, 53), (0, 26, 1, 55), (0, 27, 2, 58), (0, 28, 1, 60), (0, 29, 2, 25), (0, 30, 2, 65), (1, 15, 1, 15), (1, 16, 1, 20),
    (1, 17, 1, 25), (1, 18, 1, 30), (1, 19, 1, 35), (1, 20, 3, 40), (1, 21, 2, 43), (1, 22, 1, 45), (1, 23, 2, 48), (1, 24, 1, 50), (1, 25, 2, 53),
    (1, 26, 1, 55), (1, 27, 2, 58), (1, 28, 1, 60), (1, 29, 2, 25), (1, 30, 3, 65), (1, 31, 1, 60), (2, 15, 1, 15), (2, 16, 1, 20), (2, 17, 1, 25),
    (2, 18, 1, 30), (2, 19, 1, 35), (2, 20, 3, 40), (2, 21, 2, 43), (2, 22, 1, 45), (2, 23, 2, 48), (2, 24, 1, 50), (2, 25, 2, 53), (2, 26, 1, 55),
    (2, 27, 2, 58), (2, 28, 1, 60), (2, 29, 2, 25), (2, 30, 2, 65), (2, 31, 1, 60), (3, 15, 1, 15), (3, 16, 1, 20), (3, 17, 1, 25), (3, 18, 1, 30),
    (3, 19, 1, 35), (3, 20, 3, 40), (3, 21, 2, 43), (3, 22, 1, 45), (3, 23, 2, 48), (3, 24, 1, 50), (3, 25, 2, 53), (3, 26, 1, 55), (3, 27, 2, 58),
    (3, 28, 1, 60), (3, 29, 2, 25), (3, 30, 2, 65), (4, 15, 1, 15), (4, 16, 1, 20), (4, 17, 1, 25), (4, 18, 1, 30), (4, 19, 1, 35), (4, 20, 1, 40),
    (4, 21, 2, 43), (4, 22, 1, 45), (4, 23, 2, 48), (4, 24, 1, 50), (4, 25, 2, 53), (4, 26, 1, 55), (4, 27, 3, 58), (4, 28, 3, 60), (4, 29, 2, 25),
    (4, 30, 3, 65), (4, 31, 1, 60), (5, 15, 1, 15), (5, 16, 1, 20), (5, 17, 1, 25), (5, 18, 1, 30), (5, 19, 1, 35), (5, 20, 1, 40), (5, 21, 2, 43),
    (5, 22, 1, 45), (5, 23, 2, 48), (5, 24, 1, 50), (5, 25, 2, 53), (5, 26, 1, 55), (5, 27, 2, 58), (5, 28, 3, 60), (5, 29, 2, 25), (5, 30, 3, 65),
    (5, 31, 1, 60), (6, 15, 3, 45), (6, 16, 1, 48), (6, 17, 1, 51), (6, 18, 1, 55), (6, 19, 1, 40), (6, 20, 1, 44), (6, 21, 2, 43), (6, 22, 1, 45),
    (6, 23, 2, 48), (6, 24, 1, 50), (6, 25, 2, 53), (6, 26, 1, 58), (6, 27, 2, 60), (6, 28, 3, 65), (6, 29, 1, 60), (7, 15, 1, 15), (7, 16, 1, 20),
    (7, 17, 1, 25), (7, 18, 1, 30), (7, 19, 1, 35), (7, 20, 1, 40), (7, 21, 2, 43), (7, 22, 1, 45), (7, 23, 2, 48), (7, 24, 1, 50), (7, 25, 2, 53),
    (7, 26, 1, 55), (7, 27, 2, 58), (7, 28, 1, 60), (7, 29, 2, 25), (7, 30, 3, 65), (7, 31, 1, 60), (8, 15, 1, 15), (8, 16, 1, 20), (8, 17, 1, 25),
    (8, 18, 1, 30), (8, 19, 1, 35), (8, 20, 3, 40), (8, 21, 2, 43), (8, 22, 1, 45), (8, 23, 2, 48), (8, 24, 1, 50), (8, 25, 2, 53), (8, 26, 1, 55),
    (8, 27, 2, 58), (8, 28, 3, 60), (8, 29, 1, 60), (9, 15, 1, 15), (9, 16, 1, 20), (9, 17, 1, 25), (9, 18, 1, 30), (9, 19, 1, 35), (9, 20, 1, 40),
    (9, 21, 2, 43), (9, 22, 1, 45), (9, 23, 2, 48), (9, 24, 1, 50), (9, 25, 2, 53), (9, 26, 1, 55), (9, 27, 2, 58), (9, 28, 3, 60), (9, 29, 1, 60),
    (10, 15, 1, 15), (10, 16, 1, 20), (10, 17, 1, 25), (10, 18, 1, 30), (10, 19, 1, 35), (10, 20, 1, 40), (10, 21, 2, 43), (10, 22, 1, 45),
    (10, 23, 2, 48), (10, 24, 1, 50), (10, 25, 2, 53), (10, 26, 1, 55), (10, 27, 3, 58), (10, 28, 1, 60), (10, 29, 3, 25), (10, 30, 3, 65),
    (10, 31, 1, 60), (11, 0, 1, 55), (11, 1, 1, 55), (11, 2, 1, 55), (11, 3, 3, 55), (11, 4, 3, 55), (11, 5, 3, 55), (11, 6, 2, 25), (11, 7, 2, 25),
    (11, 8, 2, 25), (11, 9, 2, 25), (11, 10, 2, 40), (11, 11, 2, 40), (11, 12, 2, 40), (11, 13, 2, 25), (11, 14, 3, 40), (11, 15, 1, 20),
    (11, 16, 1, 20), (11, 17, 1, 20), (11, 18, 1, 35), (11, 19, 1, 35), (11, 20, 1, 35), (11, 21, 1, 35), (11, 22, 1, 35), (11, 23, 2, 50),
    (11, 24, 2, 50), (11, 25, 2, 50), (11, 26, 2, 50), (11, 27, 2, 50), (11, 28, 2, 50), (12, 15, 1, 15), (12, 16, 1, 20), (12, 17, 1, 25),
    (12, 18, 1, 30), (12, 19, 1, 35), (12, 20, 3, 40), (12, 21, 2, 43), (12, 22, 1, 45), (12, 23, 2, 48), (12, 24, 1, 50), (12, 25, 2, 53),
    (12, 26, 1, 55), (12, 27, 2, 58), (12, 28, 3, 60), (12, 29, 3, 25), (12, 30, 2, 65), (12, 31, 1, 60), (13, 15, 1, 15), (13, 16, 1, 20),
    (13, 17, 1, 25), (13, 18, 1, 30), (13, 19, 1, 35), (13, 20, 3, 40), (13, 21, 2, 43), (13, 22, 1, 45), (13, 23, 2, 48), (13, 24, 1, 50),
    (13, 25, 2, 53), (13, 26, 1, 55), (13, 27, 2, 58), (13, 28, 3, 60), (13, 29, 2, 25), (13, 30, 2, 65), (13, 31, 1, 60), (14, 15, 1, 15),
    (14, 16, 1, 20), (14, 17, 1, 25), (14, 18, 1, 30), (14, 19, 1, 35), (14, 20, 2, 40), (14, 21, 2, 43), (14, 22, 1, 45), (14, 23, 2, 48),
    (14, 24, 1, 50), (14, 25, 2, 53), (14, 26, 1, 55), (14, 27, 2, 58), (14, 28, 3, 60), (14, 29, 3, 25), (14, 30, 1, 60), (15, 15, 1, 15),
    (15, 16, 1, 20), (15, 17, 1, 25), (15, 18, 1, 30), (15, 19, 1, 35), (15, 20, 2, 40), (15, 21, 2, 43), (15, 22, 1, 45), (15, 23, 2, 48),
    (15, 24, 1, 50), (15, 25, 2, 53), (15, 26, 1, 55), (15, 27, 2, 58), (15, 28, 3, 60), (15, 29, 1, 60),
];

/// HZ-046: the (category, number) a boss of `level` drops for one `roll` of the game's Random — a grade
/// by [`BOSS_GRADE_PCT`], then one of that grade within [`BOSS_BELOW`]..[`BOSS_ABOVE`] of the level (any
/// grade of the window when the grade has none there; the nearest levels when the window is empty).
pub fn boss_drop(level: i32, roll: i32) -> Option<(i8, i8)> {
    let r = roll.unsigned_abs();
    let g = r % 100;
    let grade: i8 = if (g as i32) < BOSS_GRADE_PCT[0] {
        1
    } else if (g as i32) < BOSS_GRADE_PCT[0] + BOSS_GRADE_PCT[1] {
        2
    } else {
        3
    };
    let window = |e: &&(i8, i8, i8, i8)| (level - BOSS_BELOW..=level + BOSS_ABOVE).contains(&i32::from(e.3));
    let mut band: Vec<(i8, i8)> = GRADED.iter().filter(|e| e.2 == grade).filter(window).map(|e| (e.0, e.1)).collect();
    if band.is_empty() {
        band = GRADED.iter().filter(window).map(|e| (e.0, e.1)).collect();
    }
    if band.is_empty() {
        let nearest = GRADED.iter().map(|e| (i32::from(e.3) - level).abs()).min()?;
        band = GRADED.iter().filter(|e| (i32::from(e.3) - level).abs() == nearest).map(|e| (e.0, e.1)).collect();
    }
    Some(band[(r / 100) as usize % band.len()])
}

/// HZ-047: 전력방어's damage `dmg` for its `y` (`db.c(9)`): `level` from `y`, then `100 − (40 + 10·L)` percent,
/// never under a tenth, never under 1 point.
pub fn guard(dmg: i32, level: i32) -> i32 {
    let pct = (100 - (40 + 10 * level)).clamp(10, 100);
    (dmg * pct / 100).max(1)
}

/// HZ-047: 전력방어's level from `y` — `1 + L` with HZ-039's table, the game's `9 − 2L` without.
pub fn guard_level(y: i32, host_table: bool) -> i32 {
    if host_table { (y - 1).max(0) } else { ((9 - y) / 2).max(0) }
}

/// HZ-047: 전력방어's frames for skill level `level` — `4 + L` seconds (the game: 80, 4 s).
pub fn guard_frames(level: i32) -> i32 {
    (4 + level.max(0)) * 20
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combo_and_points() {
        assert_eq!(combo(0), Some((100, 4)));
        assert_eq!(combo(3), Some((120, 8)));
        assert_eq!((combo(4), combo(-1)), (None, None));
        let total: i32 = (2..=99).map(skill_points_at).sum();
        assert_eq!(total, 49 + 9 + 19, "LV 99: the game's 49, the nine tenth levels, one more at each of LV 2..20");
        assert_eq!((skill_points_at(10), skill_points_at(11), skill_points_at(20), skill_points_at(21)), (3, 1, 3, 0));
        assert!(is_boss_head(119) && !is_boss_head(120) && !is_boss_head(44));
    }

    #[test]
    fn reflection_is_a_tenth_mitigated_and_capped() {
        assert_eq!(reflect(600), 300, "the game handed a fifth of 3,000");
        assert_eq!(reflected_hit(300, 90, 1800), 70);
        assert_eq!(reflected_hit(3000, 90, 1800), 270, "15 % of 1,800");
        assert_eq!(reflected_hit(1, 500, 1800), 1);
    }

    #[test]
    fn boss_drops_follow_grade_and_level() {
        let (cat, id) = boss_drop(45, 10).unwrap();
        let e = GRADED.iter().find(|e| (e.0, e.1) == (cat, id)).unwrap();
        assert_eq!(e.2, 1, "a roll under 50 is a rare");
        assert!((35..=50).contains(&i32::from(e.3)));
        let (cat, id) = boss_drop(45, 99).unwrap();
        assert_eq!(GRADED.iter().find(|e| (e.0, e.1) == (cat, id)).unwrap().2, 3, "99 is a set piece");
        assert!(boss_drop(10, 60).is_some(), "a LV 10 boss finds the LV 15 pieces");
        assert!(boss_drop(110, 7).is_some(), "a 가디언 boss finds the nearest");
        assert!(GRADED.iter().all(|e| (1..=3).contains(&e.2) && e.1 <= 31));
    }

    #[test]
    fn guard_percent_and_level() {
        assert_eq!((guard_level(4, true), guard_level(6, true), guard_level(7, false), guard_level(3, false)), (3, 5, 1, 3));
        assert_eq!((guard(1000, 1), guard(1000, 3), guard(1000, 5), guard(1000, 9)), (500, 300, 100, 100));
        assert_eq!(guard(3, 5), 1);
        assert_eq!((guard_frames(3), guard_frames(5)), (140, 180));
    }
}
