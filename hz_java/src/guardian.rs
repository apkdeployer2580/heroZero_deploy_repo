//! 가디언 레벨 — HZ-022, hero2's answer to the level cap (user 2026-10-25: 「레벨 상한 … hero2처럼」;
//! hero2's own words: *"레벨 99를 찍으면 따로 레벨을 관리하는쪽에서 1부터 시작하는 경험치바 위에
//! 가디언레벨(가칭)을 … 가디언 레벨은 250이 최대치고 경험치바 최대치는 아주 조금씩만 늘어났으면 …
//! 가디언 레벨은 단순히 레벨표시와 스탯 추가를 위한 수단이고 레벨 보정같은 시스템엔 영향을 주면
//! 안돼."*, HERO2_GUARDIAN_LEVEL_PLAN.md). The level byte stays 99, so every level formula, unlock
//! and requirement of the game is untouched; past 99 each guardian level gives what a level-up
//! does in this game, 4 stat points (`ek.j(I)V` @99: `ek.a:S += 4`), no skill points.
//!
//! The game's EXP rule is hero2's (`ek.i()V` @970..1014: next = L³ − L² + 80L, 968,418 at 99), and so
//! is the guardian curve (hero2 D3): GL `g` needs `968,418 + 19,368·(g − 1)` (+2 % a level).
//!
//! **Storage = the game's own EXP.** At 99 the game throws EXP away as it fills (`a[2] −= b[2]`); with
//! HZ-022 it keeps counting there, so `ek.a:[I[2]` (an int the save already writes, `ek.a()[B`) holds
//! the EXP earned since reaching 99 and the GL is worked out from it — nothing else to save or to
//! keep in step with the save. GL 1..250 reaches ~838 million, inside an int.

/// The guardian levels' top.
pub const MAX: u16 = 250;
/// LV 99's EXP to the next level (`99³ − 99² + 80·99`) — GL 1's.
pub const BASE: i64 = 968_418;
/// What each GL adds to the next (2 % of [`BASE`], floored — hero2's 19,368).
pub const STEP: i64 = 19_368;
/// The game's top level.
pub const TOP_LEVEL: i8 = 99;
/// Stat points a guardian level gives — a level-up's in this game.
pub const STAT_POINTS: i16 = 4;

/// EXP from GL `g` to `g + 1`.
pub fn need(g: u16) -> i64 {
    BASE + STEP * i64::from(g.max(1) - 1)
}

/// EXP since 99 at which GL `g + 1` begins: `T(g) = Σ need(1..=g)` (`T(0) = 0`).
pub fn threshold(g: u16) -> i64 {
    let g = i64::from(g);
    g * BASE + STEP * g * (g - 1) / 2
}

/// The guardian level of `exp` EXP since 99, and the EXP into it: GL `g` while `T(g − 1) ≤ exp <
/// T(g)`; at [`MAX`] the bar stays full.
pub fn level_of(exp: i64) -> (u16, i64) {
    let exp = exp.max(0);
    let mut g: u16 = 1;
    while g < MAX && exp >= threshold(g) {
        g += 1;
    }
    (g, (exp - threshold(g - 1)).min(need(g)))
}

/// What the game's level-up loop compares EXP with at 99 for GL `g`: the start of the next one, or
/// never once it is the last.
pub fn next_at(g: u16) -> i32 {
    if g >= MAX { i32::MAX } else { threshold(g).min(i64::from(i32::MAX)) as i32 }
}

/// 가디언 레벨 N 달성 — the notice, as hero2's (EUC-KR; ASCII digits draw from the game's font).
pub fn notice(g: u16) -> Vec<u8> {
    [&[0xb0, 0xa1, 0xb5, 0xf0, 0xbe, 0xf0, 0x20, 0xb7, 0xb9, 0xba, 0xa7, 0x20][..], g.to_string().as_bytes(), &[0x20, 0xb4, 0xde, 0xbc, 0xba]].concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hero2s_curve_and_top() {
        assert_eq!((need(1), need(2), need(250)), (968_418, 987_786, 968_418 + 19_368 * 249));
        assert_eq!(threshold(0), 0);
        assert_eq!(threshold(1), 968_418);
        assert_eq!(threshold(2), 968_418 + 987_786);
        assert!(threshold(MAX) < i64::from(i32::MAX), "inside an int: {}", threshold(MAX));
        assert_eq!(level_of(0), (1, 0), "reaching 99 is GL 1");
        assert_eq!(level_of(968_417), (1, 968_417));
        assert_eq!(level_of(968_418), (2, 0));
        assert_eq!(level_of(threshold(249)), (MAX, 0));
        assert_eq!(level_of(i64::from(i32::MAX)), (MAX, need(MAX)), "the bar stays full");
        assert_eq!(level_of(-5), (1, 0));
        assert_eq!((next_at(1), next_at(MAX)), (968_418, i32::MAX));
    }

    #[test]
    fn the_notice_is_the_game_fonts() {
        assert_eq!(notice(12), crate::euc_kr("가디언 레벨 12 달성").unwrap());
    }
}
