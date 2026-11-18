//! HZ-033 — the game's own items drop far more readily once the 85제 tier is the goal (user 2026-10-28:
//! *"85제가 신규로 생성되면서 그 이전 아이템들의 드랍률을 대폭 상승 … hero2랑 동일한 방식"*; plan A of
//! HEROZERO_NEW_ITEM_DRAFT.md §5). The pure parts; the hooks draw the game's own Random (`ar.a`).
//!
//! The game's drop (`cq.o()V`, HEROZERO_GAME_OPTIONS_RE.md §18): one roll R decides; three equipment slots
//! of ~7.4 % each, the item number fixed by the monster's level (common `min(LV/4, 14)`, rare
//! `min(LV/5 + 13, 28)`) — so one level band gave one item per category, and the LV 65 recipe gear and
//! the LV 60 rares past number 28 never dropped. Plan A:
//! 1. the equipment slots twice as wide ([`SLOT_PCT`]),
//! 2. the number anywhere in the few under the game's ([`COMMON_SPREAD`], [`RARE_SPREAD`] — the top is
//!    the game's, so growth keeps its pace),
//! 3. in 헬 and 가디언 모드 a rare at the band's top is one in [`EXTRA_ODDS`] a category's LV 60..65 piece
//!    past 28 ([`EXTRA`]),
//! 4. in 노말 and 하드코어 one kill in 50 ([`EPIC_PER_MILLE`]) an 에픽 or 세트 of the monster's level band
//!    ([`EPIC`]).
//! 5. (2026-11-06) the game's fixed bands past the slots keep their odds: the same R that the wider slots
//!    now take ran on, in the game, to 14500..22750 potion · 22750..30250 material · 30250..43428 제련석 ·
//!    43428..47928 recipe · 47928..52428 magic stone (`cq.o()V` @281..535) — with the slots ×2 a 「드랍률」 ×1
//!    kill never gave a potion, ×2 nothing but equipment. HZ-033 turns the `return` after a slot's drop into
//!    a `nop`, so the loop goes on with the slots left answering [`SLOT_TAKEN`] and the game's own bands
//!    follow for that R: equipment boosted, every band at the game's odds, sometimes both in one kill.

/// Equipment slot widths, percent of the game's (`/en/edf` thresholds are cumulative, so scaling all of
/// them scales each slot).
pub const SLOT_PCT: i32 = 200;
/// How far under the game's number a common / rare drop may land.
pub const COMMON_SPREAD: i32 = 3;
pub const RARE_SPREAD: i32 = 4;
/// The rare band's floor (`LV/5 + 13` at LV 0).
pub const RARE_FLOOR: i32 = 13;
/// A rare at or past this number (and at most the game's 28 — not HZ-023's guardian piece) may become a
/// top piece, in 헬 and 가디언 모드 (round 2 and up).
pub const EXTRA_FROM: i8 = 26;
pub const RARE_TOP: i8 = 28;
pub const EXTRA_ROUND: i8 = 2;
pub const EXTRA_ODDS: u32 = 4;
/// 에픽 · 세트 a kill in 1,000, below 헬.
pub const EPIC_PER_MILLE: u32 = 20;
/// The level window of [`EPIC`] around the monster's level.
pub const EPIC_BELOW: i32 = 10;
pub const EPIC_ABOVE: i32 = 3;

/// Every 에픽 / 세트 a drop can name (number 28 and under): (category, number, level) — `tools_re/hz_items.py`
/// over `/itm/00..15` (2026-10-28).
pub const EPIC: [(i8, i8, i8); 97] = [
    (0, 20, 40), (0, 21, 43), (0, 23, 48), (0, 25, 53), (0, 27, 58), (1, 20, 40), (1, 21, 43), (1, 23, 48), (1, 25, 53), (1, 27, 58),
    (2, 20, 40), (2, 21, 43), (2, 23, 48), (2, 25, 53), (2, 27, 58), (3, 20, 40), (3, 21, 43), (3, 23, 48), (3, 25, 53), (3, 27, 58),
    (4, 21, 43), (4, 23, 48), (4, 25, 53), (4, 27, 58), (4, 28, 60), (5, 21, 43), (5, 23, 48), (5, 25, 53), (5, 27, 58), (5, 28, 60),
    (6, 15, 45), (6, 21, 43), (6, 23, 48), (6, 25, 53), (6, 27, 60), (6, 28, 65), (7, 21, 43), (7, 23, 48), (7, 25, 53), (7, 27, 58),
    (8, 20, 40), (8, 21, 43), (8, 23, 48), (8, 25, 53), (8, 27, 58), (8, 28, 60), (9, 21, 43), (9, 23, 48), (9, 25, 53), (9, 27, 58),
    (9, 28, 60), (10, 21, 43), (10, 23, 48), (10, 25, 53), (10, 27, 58), (11, 3, 55), (11, 4, 55), (11, 5, 55), (11, 6, 25), (11, 7, 25),
    (11, 8, 25), (11, 9, 25), (11, 10, 40), (11, 11, 40), (11, 12, 40), (11, 13, 25), (11, 14, 40), (11, 23, 50), (11, 24, 50), (11, 25, 50),
    (11, 26, 50), (11, 27, 50), (11, 28, 50), (12, 20, 40), (12, 21, 43), (12, 23, 48), (12, 25, 53), (12, 27, 58), (12, 28, 60), (13, 20, 40),
    (13, 21, 43), (13, 23, 48), (13, 25, 53), (13, 27, 58), (13, 28, 60), (14, 20, 40), (14, 21, 43), (14, 23, 48), (14, 25, 53), (14, 27, 58),
    (14, 28, 60), (15, 20, 40), (15, 21, 43), (15, 23, 48), (15, 25, 53), (15, 27, 58), (15, 28, 60),
];

/// The pieces past number 28 of LV 60 and up — the LV 65 recipe gear and the LV 60 rares the game never
/// drops: (category, number, level).
pub const EXTRA: [(i8, i8, i8); 23] = [
    (0, 30, 65), (1, 30, 65), (1, 31, 60), (2, 30, 65), (2, 31, 60), (3, 30, 65), (4, 30, 65), (4, 31, 60), (5, 30, 65), (5, 31, 60),
    (6, 29, 60), (7, 30, 65), (7, 31, 60), (8, 29, 60), (9, 29, 60), (10, 30, 65), (10, 31, 60), (12, 30, 65), (12, 31, 60), (13, 30, 65),
    (13, 31, 60), (14, 30, 60), (15, 29, 60),
];

/// The highest number [`EXTRA`] names (the ground list must take it — `g.a(BBBB)V`'s limit).
pub const EXTRA_TOP: i8 = 31;

/// Where `rpg/g.a([BI)I` reads slot 0's threshold (`slot * 6 + 2`): every roll reads it first.
pub const FIRST_SLOT_AT: i32 = 2;
/// A slot's threshold once a slot dropped in the roll — no `R` (0..65534) is under it.
pub const SLOT_TAKEN: i32 = 0;

/// What `dropThreshold` answers for the slot at `offset`, its threshold `scaled` (HZ-005 / HZ-033), when a
/// slot already dropped in this roll (`taken`), and whether the roll starts here (the flag is cleared).
pub fn slot_threshold(offset: i32, scaled: i32, taken: bool) -> (i32, bool) {
    if offset == FIRST_SLOT_AT {
        (scaled, true)
    } else if taken {
        (SLOT_TAKEN, false)
    } else {
        (scaled, false)
    }
}

/// A drop's number `top` (the game's) moved down by `roll` (the game's `abs(nextInt())`) by up to `spread`,
/// never under `floor`.
pub fn spread(top: i32, roll: i32, spread: i32, floor: i32) -> i32 {
    (top - (roll.unsigned_abs() % (spread as u32 + 1)) as i32).max(floor.min(top))
}

/// A rare `id` of `category` in round `round`: one of the category's [`EXTRA`] for one `roll` in
/// [`EXTRA_ODDS`].
pub fn extra(category: i8, id: i8, round: i8, roll: i32) -> Option<i8> {
    if !(EXTRA_FROM..=RARE_TOP).contains(&id) || round < EXTRA_ROUND {
        return None;
    }
    let roll = roll.unsigned_abs();
    if roll % EXTRA_ODDS != 0 {
        return None;
    }
    let mine: Vec<i8> = EXTRA.iter().filter(|e| e.0 == category).map(|e| e.1).collect();
    (!mine.is_empty()).then(|| mine[(roll / EXTRA_ODDS) as usize % mine.len()])
}

/// An 에픽 / 세트 for a kill of a level-`level` monster below 헬, for one `roll`: whether
/// ([`EPIC_PER_MILLE`]) and which of the band's.
pub fn epic(level: i32, round: i8, roll: i32) -> Option<(i8, i8)> {
    if round >= EXTRA_ROUND {
        return None;
    }
    let roll = roll.unsigned_abs();
    if roll % 1000 >= EPIC_PER_MILLE {
        return None;
    }
    let band: Vec<(i8, i8)> = EPIC
        .iter()
        .filter(|e| (level - EPIC_BELOW..=level + EPIC_ABOVE).contains(&i32::from(e.2)))
        .map(|e| (e.0, e.1))
        .collect();
    (!band.is_empty()).then(|| band[(roll / 1000) as usize % band.len()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spread_stays_under_the_games_number() {
        let got: Vec<i32> = (0..8).map(|r| spread(14, r, COMMON_SPREAD, 0)).collect();
        assert_eq!(got, [14, 13, 12, 11, 14, 13, 12, 11]);
        assert_eq!((spread(1, 3, COMMON_SPREAD, 0), spread(13, 4, RARE_SPREAD, RARE_FLOOR), spread(28, 4, RARE_SPREAD, RARE_FLOOR)), (0, 13, 24));
        assert_eq!(spread(5, i32::MIN, 3, 0), 5, "the JVM's abs(MIN) is MIN — still in range");
    }

    #[test]
    fn top_pieces_only_at_the_top_in_hell_and_up() {
        assert_eq!(extra(1, 28, 2, 0), Some(30));
        assert_eq!(extra(1, 28, 2, 4), Some(31));
        assert_eq!((extra(1, 28, 1, 0), extra(1, 25, 3, 0), extra(1, 28, 2, 1), extra(11, 28, 3, 0)), (None, None, None, None));
        assert_eq!(extra(1, 40, 3, 0), None, "HZ-023's guardian piece stays");
        assert!(EXTRA.iter().all(|e| e.1 > 28 && e.1 <= EXTRA_TOP && e.2 >= 60));
    }

    #[test]
    fn a_dropped_slot_closes_the_rest_of_its_roll() {
        assert_eq!(slot_threshold(FIRST_SLOT_AT, 9666, true), (9666, true), "a new roll: slot 0 as ever, the flag cleared");
        assert_eq!(slot_threshold(8, 19332, false), (19332, false));
        assert_eq!(slot_threshold(8, 19332, true), (SLOT_TAKEN, false));
        assert_eq!(slot_threshold(20, 65535, true), (SLOT_TAKEN, false), "even a slot at the top");
    }

    #[test]
    fn epics_follow_the_monsters_level() {
        assert_eq!(epic(40, 0, 1_000_000 + 999), None, "the roll misses");
        let (cat, id) = epic(42, 0, 5).unwrap();
        let lvl = EPIC.iter().find(|e| (e.0, e.1) == (cat, id)).unwrap().2;
        assert!((32..=45).contains(&i32::from(lvl)));
        assert_eq!(epic(10, 0, 5), None, "no epic that low");
        assert_eq!(epic(42, 2, 5), None, "헬 has its own top");
        assert!(EPIC.iter().all(|e| e.1 <= 28));
    }
}
