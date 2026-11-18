//! HZ-040 — the player's derived stats, recomputed by the host where `ek.i()V` · `G()` · `H()` store them
//! (HEROZERO_BALANCE_PLAN.md §2.1 · §4-A; user 2026-10-29). The pure parts: a [`Rules`] holds every
//! constant the formulas use, [`GAME`] is the game's own (javap 2026-10-29 — the tests pin it) and
//! [`PATCH`] the plan's. The hooks gather the inputs from the player object and call these.
//!
//! What the game does (`ek.i()V`, stats `c[0]` 힘 · `c[1]` 민첩 · `c[2]` 건강 · `c[3]` 의지):
//! * attack `h` @422..582 = 힘 + 민첩/4 + option 6 (+ job's adjustment) + 살쾡이 + 쇄도;
//! * defence `a` @726..817 = the four armour slots + option 7 (+ 나이트 shield, 워리어 힘/12), then
//!   갑옷숙련 and 오러;
//! * `G()`: crit = 민첩/8 (+건너 민첩/16) + option 5, ≤ 50, + 속임수, + the weapon's `h`, ≤ 70, + 폭풍;
//! * `H()`: evasion = 민첩/5 (+로그 민첩/10, −나이트 민첩/10) + option 8, ≤ 75, + 회피, ≤ 80, + 영체화, ≤ 85;
//! * max HP @1051..1117 = 10·LV + 12·건강 + option 9 + 내성 (+나이트 건강/5).
//!
//! The monster's hit on the player is `90 + its LV − mine − evasion`, floored at 15 (`cq.a(Lar;)Z` @28 —
//! HZ-041 raises the floor to [`HIT_FLOOR`]).

/// `ed.b:B`, the job.
pub const WARRIOR: i8 = 0;
pub const ROGUE: i8 = 1;
pub const KNIGHT: i8 = 2;
pub const GUNNER: i8 = 3;
pub const BAYONET: i8 = 4;
pub const SPIRIT: i8 = 5;

/// Every constant the derived-stat formulas use. Divisors are the JVM's int division; `0` means "no
/// term".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rules {
    /// 힘's share of attack, a percent, by job 0..5.
    pub str_pct: [i32; 6],
    /// 민첩's divisor in attack, by job (0 = none).
    pub dex_div: [i32; 6],
    /// 의지's divisor in attack, by job (0 = none) — the 2nd pass gives the 정령사 의지/2.
    pub wil_div: [i32; 6],
    /// 힘's divisor in defence, by job (0 = none).
    pub str_def_div: [i32; 6],
    /// 의지's divisor in defence, by job (0 = none) — the 정령사's 의지/10 (2nd pass).
    pub wil_def_div: [i32; 6],
    /// Crit: 민첩's divisor, 건너's extra divisor, the cap after options, after the weapon, after 폭풍 (0 = no cap).
    pub crit_dex_div: i32,
    pub crit_gunner_div: i32,
    pub crit_caps: [i32; 3],
    /// Evasion: 민첩's divisor, 로그's extra divisor, 나이트's penalty divisor (0 = none), the caps.
    pub evade_dex_div: i32,
    pub evade_rogue_div: i32,
    pub evade_knight_div: i32,
    /// 의지's divisor in evasion after the first cap, by job (0 = none) — the 정령사's 의지/4 (3rd pass, its
    /// survival: the caster took the most damage; the 「a hit breaks its attack」 it was first put in for was
    /// wrong — HEROZERO_CLASS_BALANCE.md §8.8).
    pub evade_wil_div: [i32; 6],
    pub evade_caps: [i32; 3],
    /// HP per 건강.
    pub hp_per_con: i32,
}

/// The game's own numbers.
pub const GAME: Rules = Rules {
    str_pct: [100, 67, 50, 50, 100, 100],
    dex_div: [4, 0, 4, 12, 4, 4],
    wil_div: [0, 0, 0, 0, 0, 0],
    str_def_div: [12, 0, 0, 0, 0, 0],
    wil_def_div: [0, 0, 0, 0, 0, 0],
    crit_dex_div: 8,
    crit_gunner_div: 16,
    crit_caps: [50, 70, 0],
    evade_dex_div: 5,
    evade_rogue_div: 10,
    evade_knight_div: 10,
    evade_wil_div: [0, 0, 0, 0, 0, 0],
    evade_caps: [75, 80, 85],
    hp_per_con: 12,
};

/// The plan's (§4-A), with the 2nd pass (HEROZERO_CLASS_BALANCE.md §6-A, 2026-10-29): 워리어 힘 ×1.15 (was
/// 1.25), 로그 민첩/2 (was /3), 정령사 + 의지/2 and 의지/10 in defence instead of 힘/10; the 3rd pass (§8.5)
/// 나이트 힘 ×1.15 (user 2026-10-29, after ×1.1 left it at 10 kills).
pub const PATCH: Rules = Rules {
    str_pct: [115, 75, 115, 75, 125, 100],
    dex_div: [4, 2, 4, 6, 4, 4],
    wil_div: [0, 0, 0, 0, 0, 2],
    str_def_div: [10, 10, 10, 10, 10, 0],
    wil_def_div: [0, 0, 0, 0, 0, 10],
    crit_dex_div: 10,
    crit_gunner_div: 20,
    crit_caps: [35, 50, 60],
    evade_dex_div: 8,
    evade_rogue_div: 16,
    evade_knight_div: 0,
    evade_wil_div: [0, 0, 0, 0, 0, 4],
    evade_caps: [40, 50, 60],
    hp_per_con: 14,
};

/// HZ-041: the least a monster's hit chance can be (the game's 15).
pub const HIT_FLOOR: i8 = hz_patches::hooks::HIT_FLOOR as i8;

fn div(v: i32, d: i32) -> i32 {
    if d == 0 { 0 } else { v / d }
}

fn job(j: i8) -> usize {
    j.clamp(0, 5) as usize
}

impl Rules {
    /// Attack before the weapon: 힘 and 민첩's shares (the game adds option 6, 살쾡이 and 쇄도 after,
    /// and the weapon's minimum later in `ek.i()`).
    pub fn attack_base(&self, j: i8, str: i32, dex: i32, wil: i32) -> i32 {
        let j = job(j);
        // The game's 로그 is 힘 + 민첩/4 + 민첩/6 − 힘/3; its two int divisions are kept apart.
        if *self == GAME {
            let base = str + dex / 4;
            return match j {
                1 => base + dex / 6 - str / 3,
                2 => base - str / 2,
                3 => base - (dex / 6 + str / 2),
                _ => base,
            };
        }
        let strength = match self.str_pct[j] {
            125 => str + str / 4,
            75 => str - str / 4,
            100 => str,
            pct => str * pct / 100,
        };
        strength + div(dex, self.dex_div[j]) + div(wil, self.wil_div[j])
    }

    /// The stats' part of defence (the game: 워리어 only, 힘/12; the patch 힘/10, the 정령사 의지/10).
    pub fn stat_defence(&self, j: i8, str: i32, wil: i32) -> i32 {
        div(str, self.str_def_div[job(j)]) + div(wil, self.wil_def_div[job(j)])
    }

    /// Crit percent: `dex` 민첩, `options` the worn option-5 sum, `trick` 속임수's value, `weapon` the
    /// hand's `bx.h` (0 bare), `storm` 폭풍's value while it is up (0 otherwise).
    pub fn crit(&self, j: i8, dex: i32, options: i32, trick: i32, weapon: i32, storm: i32) -> i32 {
        let mut s = dex / self.crit_dex_div + options;
        if j == GUNNER {
            s += dex / self.crit_gunner_div;
        }
        s = cap(s, self.crit_caps[0]);
        s += trick;
        s += weapon;
        s = cap(s, self.crit_caps[1]);
        s += storm;
        cap(s, self.crit_caps[2])
    }

    /// Evasion percent: `options` the worn option-8 sum, `dodge` 회피's value, `ghost` 영체화's while up.
    pub fn evasion(&self, j: i8, dex: i32, wil: i32, options: i32, dodge: i32, ghost: i32) -> i32 {
        let mut s = dex / self.evade_dex_div + options;
        if j == ROGUE {
            s += div(dex, self.evade_rogue_div);
        } else if j == KNIGHT {
            s -= div(dex, self.evade_knight_div);
        }
        s = cap(s, self.evade_caps[0]);
        s += dodge + div(wil, self.evade_wil_div[job(j)]);
        s = cap(s, self.evade_caps[1]);
        s += ghost;
        cap(s, self.evade_caps[2])
    }

    /// What the host adds to the game's max HP (`12·건강` is the game's): `(per − 12)·건강`.
    pub fn hp_extra(&self, con: i32) -> i32 {
        (self.hp_per_con - GAME.hp_per_con) * con
    }
}

fn cap(v: i32, c: i32) -> i32 {
    if c > 0 && v > c { c } else { v }
}

/// The monster's hit chance on the player, as `cq.a(Lar;)Z` rolls it: `90 + its LV − mine − evasion`,
/// at least `floor`.
pub fn monster_hit(monster_lv: i32, my_lv: i32, evasion: i32, floor: i32) -> i32 {
    (90 + monster_lv - my_lv - evasion).max(floor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_games_rules_are_the_bytecode() {
        // ek.i()V @422..541, LV 60 with 246 points in one stat (HEROZERO_BALANCE_PLAN.md §2.6).
        assert_eq!(GAME.attack_base(WARRIOR, 256, 10, 99), 256 + 2);
        assert_eq!(GAME.attack_base(WARRIOR, 10, 256, 0), 10 + 64);
        assert_eq!(GAME.attack_base(ROGUE, 10, 256, 0), 10 + 64 + 42 - 3);
        assert_eq!(GAME.attack_base(KNIGHT, 256, 10, 0), 256 + 2 - 128);
        assert_eq!(GAME.attack_base(GUNNER, 256, 10, 0), 256 + 2 - (1 + 128));
        assert_eq!(GAME.attack_base(SPIRIT, 10, 10, 200), 10 + 2, "의지 is nothing to the game's attack");
        assert_eq!((GAME.stat_defence(WARRIOR, 256, 99), GAME.stat_defence(KNIGHT, 256, 99)), (21, 0));
        // G(): 민첩 256 → 32 (+16 for 건너), the caps.
        assert_eq!(GAME.crit(WARRIOR, 256, 0, 0, 0, 0), 32);
        assert_eq!(GAME.crit(GUNNER, 256, 0, 0, 0, 0), 48);
        assert_eq!(GAME.crit(GUNNER, 400, 30, 3, 16, 0), 50 + 3 + 16);
        assert_eq!(GAME.crit(WARRIOR, 400, 30, 10, 16, 20), 70 + 20, "no cap after 폭풍");
        // H(): 민첩 246 → 49, 로그 +24, 나이트 −24, the caps.
        assert_eq!(GAME.evasion(WARRIOR, 246, 0, 0, 0, 0), 49);
        assert_eq!(GAME.evasion(ROGUE, 246, 0, 0, 0, 0), 73);
        assert_eq!(GAME.evasion(KNIGHT, 246, 0, 0, 0, 0), 25);
        assert_eq!(GAME.evasion(ROGUE, 400, 0, 0, 6, 25), 85);
        assert_eq!(GAME.evasion(SPIRIT, 88, 166, 0, 0, 0), 17, "의지 is nothing to the game's evasion");
        assert_eq!(GAME.hp_extra(100), 0);
        assert_eq!(monster_hit(60, 60, 73, 15), 17);
        assert_eq!(monster_hit(60, 60, 80, 15), 15);
    }

    #[test]
    fn the_patch_bends_dex_and_lifts_str() {
        assert_eq!(PATCH.attack_base(WARRIOR, 256, 10, 0), 256 * 115 / 100 + 2, "힘 ×1.15 (2nd pass; was ×1.25)");
        assert_eq!(PATCH.attack_base(KNIGHT, 256, 10, 0), 256 * 115 / 100 + 2, "힘 ×1.15 (3rd pass)");
        assert_eq!(PATCH.attack_base(ROGUE, 10, 256, 0), 10 - 2 + 128, "민첩/2 (2nd pass; was /3)");
        assert_eq!(PATCH.attack_base(GUNNER, 100, 60, 0), 75 + 10);
        assert_eq!(PATCH.attack_base(BAYONET, 100, 0, 0), 125);
        assert_eq!(PATCH.attack_base(SPIRIT, 100, 40, 166), 110 + 83, "의지/2 (2nd pass)");
        assert_eq!(PATCH.attack_base(WARRIOR, 100, 40, 166), 115 + 10, "의지 is the 정령사's alone");
        assert_eq!(PATCH.stat_defence(KNIGHT, 256, 99), 25);
        assert_eq!((PATCH.stat_defence(SPIRIT, 256, 99), PATCH.stat_defence(WARRIOR, 256, 99)), (9, 25));
        assert_eq!(PATCH.crit(WARRIOR, 246, 0, 0, 0, 0), 24);
        assert_eq!(PATCH.crit(GUNNER, 400, 20, 6, 16, 30), 60, "capped at 60 after 폭풍");
        assert_eq!(PATCH.evasion(ROGUE, 246, 0, 0, 0, 0), 40, "30 + 15 meets the cap");
        assert_eq!(PATCH.evasion(ROGUE, 120, 0, 0, 0, 0), 15 + 7);
        assert_eq!(PATCH.evasion(KNIGHT, 246, 0, 0, 0, 0), 30, "no 나이트 penalty");
        assert_eq!(PATCH.evasion(ROGUE, 400, 0, 10, 6, 25), 60);
        assert_eq!(PATCH.evasion(SPIRIT, 88, 166, 0, 0, 0), 50, "11 + 41 meets the second cap (3rd pass)");
        assert_eq!(PATCH.evasion(WARRIOR, 88, 166, 0, 0, 0), 11, "의지 is the 정령사's alone");
        assert_eq!(PATCH.hp_extra(100), 200);
        assert_eq!(monster_hit(60, 60, 45, HIT_FLOOR.into()), 45);
        assert_eq!(monster_hit(60, 60, 60, HIT_FLOOR.into()), 30);
    }
}
