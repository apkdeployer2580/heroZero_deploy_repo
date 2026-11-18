//! 가디언 모드 — HZ-030, the replay round after 헬 (user 2026-10-28: *"헬이후에 가디언 모드를 추가할꺼야
//! 몬스터 레벨을 85~110이고 몬스터가 쌔지는 정도는 기존과 비례하게 강해지면돼 … 퀘스트 보상과 경험치도
//! 모드가 올라갔으니 당연히 더 줘야하고"*).
//!
//! The game already has replay rounds: `ed.h:B`, 0 노말 · 1 하드코어 · 2 헬, bumped by the ending
//! (`ed.a()V` @430, at most 2) and saved with the slot (`ed.a(B)[B` byte 16). HZ-030 lets the ending
//! go one further, to [`ROUND`], and answers for that round wherever the game reads it
//! (HEROZERO_GAME_OPTIONS_RE.md §18 lists every reader, §19 the patch):
//!
//! * **Monsters** (`cq.i()V`). The game's own rounds: `n = (r+1)·n0/5 + 40|60`, both defences ×n/n0, HP
//!   `(HP−70)·(n + 20|40)/n0`, attack `(atk−13)·n/n0 + 20|40`, evasion `g` `(g−10)·n/n0`.
//!   가디언 maps the kinds' base levels 1..45 onto [`LOW`]..[`HIGH`] and keeps the same rule with the
//!   next offset, [`STEP`] — "proportional to what was there". The game's random +0..2 (`q()`, not
//!   bosses) and the 난이도 option (HZ-017) still come after.
//! * **Quest rewards** (script actions 17 money · 20 EXP read three values and take `min(round, 2)`):
//!   헬's value × [`QUEST_PCT`]. Items (actions 12 · 16) stay 헬's until the 85-tier items exist.
//! * **The ending's line**: 「가디언 모드로 재시작합니다.」 from 헬 on.
//!
//! Kill EXP needs nothing: `cq.n()V` pays by the monster's level (18·n·clamp(20 + n − LV, 0, 30)/20),
//! so a LV 110 monster pays a LV 99 hero 2,970 against 헬's top 541.

use crate::text::euc_kr;

/// 가디언 모드's value of the game's replay round `ed.h`.
pub const ROUND: i8 = hz_patches::hooks::GUARDIAN_ROUND;
/// The level of the weakest kind (base 1) and of the strongest (base 45, 거인신) in 가디언 모드.
pub const LOW: i32 = 85;
pub const HIGH: i32 = 110;
/// The kinds' base levels in `/en/edf` (`eq.d:B`): 평신도·슬라임 1 … 거인신 45.
pub const BASE_LOW: i32 = 1;
pub const BASE_HIGH: i32 = 45;
/// HP and attack's offset: the game's 20 (하드코어) and 40 (헬), one step on.
pub const STEP: i32 = 60;
/// Attack × this percent on top (HEROZERO_CLASS_BALANCE.md §6-E, 2026-10-29): at LV 99 a monster's 233~266 met
/// armour of 229~287 and the hit formula's floor, so the end game did not hurt.
pub const ATTACK_PCT: i32 = 160;
/// Quest money and EXP in 가디언 모드: 헬's × this percent.
pub const QUEST_PCT: i64 = 200;
/// `t/com.tdf` [34] 「헬 모드로」 — what the ending letters before 「재시작합니다.」 from 하드코어 on (`ce.d(dd)`
/// @884: [33] 「하드코어 모드로」 at round 0, [34] after). The round is bumped only once the line is shown.
pub const NEXT_ROUND_TEXT: i32 = 34;
/// What it says from 헬 on.
pub const NEXT_ROUND: &str = "가디언 모드로";

/// A monster's numbers as `cq.p()V` copies them from its kind (`n:B`, `h:I`, `d:S`, `e:S`, `f:S`,
/// `g:S`) and as `cq.i()V` leaves them for a round.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Monster {
    pub level: i8,
    pub hp: i32,
    /// `cq.d:S` (from `eq.b`) — what the player's basic attacks and skills meet (`cq.b()I`, javap 2026-10-28).
    pub defence: i16,
    /// `cq.e:S` (from `eq.c`) — what the spirit's attacks meet (`cq.a(IB)Z`).
    pub spirit_defence: i16,
    pub attack: i16,
    /// `cq.g:S` (from `eq.f`) — evasion (the player's hit chance − g/5) and the monster's crit (g/8).
    pub evasion: i16,
}

/// The 가디언 level of a kind whose base level is `base`.
pub fn level(base: i32) -> i32 {
    LOW + (HIGH - LOW) * (base.clamp(BASE_LOW, BASE_HIGH) - BASE_LOW) / (BASE_HIGH - BASE_LOW)
}

fn short(v: i32) -> i16 {
    v.clamp(i16::MIN.into(), i16::MAX.into()) as i16
}

/// The kind's numbers `base` raised to 가디언 모드 — the game's round rule (int division, as the JVM's)
/// with [`level`] and [`STEP`]. `None` for a kind of level 0 or less, which the rule divides by.
pub fn scale(base: Monster) -> Option<Monster> {
    let n0 = i32::from(base.level);
    if n0 <= 0 {
        return None;
    }
    let n = level(n0);
    let ratio = |v: i32| (i64::from(v) * i64::from(n) / i64::from(n0)) as i32;
    let hp = (i64::from(base.hp - 70) * i64::from(n + STEP) / i64::from(n0)).clamp(1, i64::from(i32::MAX)) as i32;
    Some(Monster {
        level: n as i8,
        hp,
        defence: short(ratio(base.defence.into())),
        spirit_defence: short(ratio(base.spirit_defence.into())),
        attack: short((ratio(i32::from(base.attack) - 13) + STEP) * ATTACK_PCT / 100),
        evasion: short(ratio(i32::from(base.evasion) - 10)),
    })
}

/// A quest's money or EXP in 가디언 모드, from what the script gives in 헬.
pub fn quest(hell: i32) -> i32 {
    (i64::from(hell) * QUEST_PCT / 100).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// HZ-034: the LV 65 recipes (`/itm/18` 7..13 · 16 · 17 · 26 — 블러드캘러미티 … 봉인의 고글).
pub const TOP_RECIPES: [i8; 10] = [7, 8, 9, 10, 11, 12, 13, 16, 17, 26];
/// The categories whose reward count doubles: 조합재료 16, 마법석 17, 특수(물약) 20.
pub const DOUBLED: [i8; 3] = [16, 17, 20];

/// HZ-034 (user 2026-10-28, HEROZERO_NEW_ITEM_DRAFT.md §6 plan A): the item a 가디언 모드 quest gives in
/// place of 헬's (`cat`, `id`) — gear becomes its category's guardian piece (HZ-023's number), a recipe
/// one of the LV 65 recipes; the rest stays.
pub fn quest_item(cat: i8, id: i8) -> (i8, i8) {
    match cat {
        0..=15 => (cat, hz_patches::hooks::GEAR_ID),
        18 if !TOP_RECIPES.contains(&id) => (cat, TOP_RECIPES[id.unsigned_abs() as usize % TOP_RECIPES.len()]),
        _ => (cat, id),
    }
}

/// HZ-034: how many of category `cat` a 가디언 모드 quest gives for 헬's `count`.
pub fn quest_count(cat: i8, count: i32) -> i32 {
    if DOUBLED.contains(&cat) { count.saturating_mul(2) } else { count }
}

/// What `cq.i()V`'s two reads of the round should see: the round itself up to 헬, and 0 in 가디언 모드
/// — the game's own round block is skipped and [`scale`] has already run.
pub fn game_round(round: i8) -> i8 {
    if round >= ROUND { 0 } else { round }
}

/// The ending's 「… 모드로」 for text `index` at round `round`, when it is not the game's.
pub fn ending_text(index: i32, round: i8) -> Option<Vec<u8>> {
    (index == NEXT_ROUND_TEXT && round >= ROUND - 1).then(|| euc_kr(NEXT_ROUND)).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `/en/edf` (tools_re/hz_endgame.py 2026-10-28): 아이스트롤 (97) and 거인신 (119) as `cq.p()V`
    /// copies them — the defence the probe's `--mobs` prints (`cq.e`; `d` and `g` are left 0 here).
    const ICE_TROLL: Monster = Monster { level: 43, hp: 1856, defence: 0, spirit_defence: 43, attack: 156, evasion: 0 };
    const GIANT_GOD: Monster = Monster { level: 45, hp: 10280, defence: 0, spirit_defence: 270, attack: 295, evasion: 0 };

    #[test]
    fn the_levels_run_from_85_to_110() {
        assert_eq!((level(1), level(45)), (85, 110));
        assert_eq!(level(43), 108, "아이스트롤: 108..110 with the game's random +0..2");
        assert_eq!((level(0), level(60)), (85, 110), "outside the table: clamped");
        assert!((1..45).all(|b| level(b) <= level(b + 1)));
    }

    /// The game's rule with the next step: the 헬 numbers the probe measured (2026-10-28: 아이스트롤
    /// LV 85 HP 5,191 attack 322 defence 85) come from the same rule at round 2, so this is that rule.
    #[test]
    fn the_numbers_follow_the_games_round_rule() {
        let hell = |m: Monster| {
            let n0 = i32::from(m.level);
            let n = 3 * n0 / 5 + 60;
            ((m.hp - 70) * (n + 40) / n0, (i32::from(m.attack) - 13) * n / n0 + 40, i32::from(m.spirit_defence) * n / n0)
        };
        assert_eq!(hell(ICE_TROLL), (5191, 322, 85));
        let t = scale(ICE_TROLL).unwrap();
        assert_eq!((t.level, t.hp, t.attack, t.spirit_defence), (108, 1786 * 168 / 43, ((143 * 108 / 43 + 60) * 160 / 100) as i16, 108));
        let g = scale(GIANT_GOD).unwrap();
        assert_eq!((g.level, g.hp, g.attack, g.spirit_defence), (110, 10210 * 170 / 45, ((282 * 110 / 45 + 60) * 160 / 100) as i16, 660));
        assert!(g.hp > hell(GIANT_GOD).0 && i32::from(g.attack) > hell(GIANT_GOD).1, "stronger than 헬");
        assert_eq!(scale(Monster { level: 0, ..ICE_TROLL }), None);
    }

    #[test]
    fn quests_pay_double_hells() {
        assert_eq!(quest(50_500), 101_000);
        assert_eq!(quest(0), 0);
        assert_eq!(quest(i32::MAX), i32::MAX);
    }

    #[test]
    fn guardian_quests_give_the_next_tier() {
        assert_eq!(quest_item(4, 25), (4, 40), "맵 4's dagger -> 가디언 대거");
        assert_eq!((quest_item(18, 18), quest_item(18, 7)), ((18, 17), (18, 7)), "a recipe -> a LV 65 one; one already stays");
        assert_eq!((quest_item(19, 9), quest_item(20, 7)), ((19, 9), (20, 7)));
        assert_eq!((quest_count(20, 10), quest_count(16, 10), quest_count(19, 1), quest_count(1, 1)), (20, 20, 1, 1));
    }

    #[test]
    fn the_game_skips_its_own_rounds_only_in_guardian_mode() {
        assert_eq!([0, 1, 2, 3, 4].map(game_round), [0, 1, 2, 0, 0]);
    }

    #[test]
    fn the_ending_names_guardian_mode_from_hell_on() {
        let line = euc_kr(NEXT_ROUND).unwrap();
        assert_eq!(ending_text(34, 1), None, "하드코어's ending still says 헬");
        assert_eq!(ending_text(34, 2), Some(line.clone()));
        assert_eq!(ending_text(34, 3), Some(line));
        assert_eq!(ending_text(33, 3), None);
        assert_eq!(ending_text(32, 2), None, "재시작합니다. is the game's");
    }
}
