//! HZ-039 — the skill tables the game loads, served by the host (HEROZERO_BALANCE_PLAN.md §4-B · §4-C,
//! user 2026-10-29 「진행하자」 on the plan's recommendations). The jar's files stay byte-for-byte
//! (CLAUDE.md rule 3): the hook calls the game's loader `rpg/e.a(String)[B` and lays these edits over
//! what it returned, then rewrites the file's check byte (every one of the four ends in the sum of the
//! bytes before it, mod 256 — `tools_re` 2026-10-29).
//!
//! * `/hero/hsd1` — the job skills: six jobs × eight skills × 7 bytes `A B C SP(2) cool(2)`; a skill's
//!   value is `(A + B·L) / C` of what the job class feeds it (attack for a damage skill, max HP for
//!   회복, frames for a duration), `db.b(B)V`. The level cap is 3 (5 with HZ-042), so the values are
//!   chosen for L3 and L5.
//! * `/hero/hsd2` — the twenty common passives, `A B C` each: `x·(A + B·L) / C`, `ek.b(BI)I`.
//! * `/s/d/a` — the ten spirit actives, 8 bytes each (`A B C` first): the three area skills a fifth
//!   stronger, 죽음의 검's chance made a chance (HZ-047 turns its comparison round).
//! * `/s/d/p` — the ten spirit passives, `A B C`: 신경가속 `(1+L)/2` → `(1+L)/3`.

/// A job skill's new numbers: job, skill index 0..5 (the game's id − 4), `A B C`, SP, cooldown frames.
pub const JOB_SKILLS: [(usize, usize, [i8; 3], i16, i16); 22] = [
    // 워리어: 강타 ×1.8/2.4 (2nd pass; was 2.0/2.6) · 충격 stun 3/4 s · 돌격 ×1.8/2.4 · 외침 curse 5/7 s · 회전베기 ×1.4/2.2 (was 1.5/2.5), SP 100, 4 s
    (0, 0, [9, 3, 10], 45, 20),
    (0, 1, [30, 10, 1], 40, 40),
    (0, 2, [9, 3, 10], 60, 10),
    (0, 3, [40, 20, 1], 60, 70),
    (0, 5, [2, 4, 10], 100, 80),
    // 로그: 독수 70/90 % · 연격 ×1.7/2.3 (two hits; 2nd pass, SP 60) · 참격 ×2.8/4.0 (2nd pass, SP 80) · 분신 15/25 s
    (1, 1, [40, 10, 1], 40, 20),
    (1, 2, [16, 6, 20], 60, 20),
    (1, 4, [10, 6, 10], 80, 30),
    (1, 5, [0, 100, 1], 150, 200),
    // 나이트: 방패치기 ×3.6/5.0 (of the weapon's roll since HZ-048; 4th pass, was 3.3/4.5), SP 30 (4th pass, was 40) ·
    // 회복 30/45 % every 5 s · 방패 돌격 ×2.2/3.0 · 반격 ×2.0/3.0 · 방패 밀기 ×3.4/4.2 (4th pass, was 3.0/3.8), SP 60
    // (4th pass, was 80) · 전력방어 y = 1 + L (HZ-047's hook reads the level back from it)
    (2, 0, [15, 7, 10], 30, 10),
    (2, 1, [3, 3, 40], 60, 100),
    (2, 2, [10, 4, 10], 60, 20),
    (2, 3, [5, 5, 10], 90, 180),
    (2, 4, [22, 4, 10], 60, 20),
    (2, 5, [1, 1, 1], 120, 300),
    // 건너: 조준사격 ×2.5/3.5 · 연사 ×2.0/2.6 · 난사 ×2.5/3.5 · 저격 40/50 %
    (3, 1, [10, 5, 10], 40, 5),
    (3, 2, [11, 3, 10], 40, 5),
    (3, 3, [10, 5, 10], 100, 5),
    (3, 5, [25, 5, 1], 100, 5),
    // 총검사: 포격 ×2.2/3.2 (2nd pass; was 2.5/3.5) · 돌진 ×2.2/3.0
    (4, 0, [7, 5, 10], 90, 5),
    (4, 1, [10, 4, 10], 90, 5),
    // 정령사: 동화 18/30 s · 120 SP · 8 s (2nd pass; was 15/25 s · 180 SP · 10 s)
    (5, 1, [0, 120, 1], 120, 160),
];

/// A common passive's new `A B C`, by id (`t/ps` order).
pub const PASSIVES: [(usize, [i8; 3]); 18] = [
    (0, [100, -8, 100]), // 지구력 SP −8 %/L
    (1, [100, -8, 100]), // 기술숙련 cooldown −8 %/L
    (2, [0, 2, 1]),      // 집중 hit +2/L
    (3, [0, 4, 100]),    // 강한체력 힘 +4 %/L
    (5, [0, 2, 1]),      // 속임수 crit +2/L
    (6, [0, 15, 100]),   // 냉혹함 crit damage +15 %/L
    (7, [100, -8, 100]), // 전술숙련 defence −8 %/L
    (8, [0, 4, 100]),    // 강인함 건강 +4 %/L
    (9, [1, 5, 100]),    // 갑옷숙련 defence +5 %/L
    (10, [10, 10, 1]),   // 인내력 10 + 10L
    (11, [10, 10, 1]),   // 대지의 수호 10 + 10L
    (12, [0, 5, 1]),     // 출혈 5L %
    (13, [0, 4, 100]),   // 민첩성 +4 %/L
    (14, [0, 3, 1]),     // 기민함 −3/L
    (15, [0, 1, 25]),    // 살쾡이 민첩·L/25
    (16, [0, 4, 1]),     // 기회공격 SP 4L
    (17, [0, 8, 1]),     // 전투능력 HP 8L
    (19, [0, 20, 1]),    // 묵시 MP +20L
];

/// A spirit active's new `A B C` and 의지 divisor (byte 3; 0 = the game's), by id (`t/sa` order). The 2nd
/// pass (HEROZERO_CLASS_BALANCE.md §6-B) makes the three area skills ×1.5 of the 1st pass and halves the
/// divisor — an absolute table could not follow the other jobs' 「공격 × 3」 at LV 99. A byte is signed, so
/// 127 is a value's ceiling (파멸·심판 take what fits: ×1.3 of the 1st pass).
pub const SPIRIT_ACTIVES: [(usize, [i8; 3], i8); 4] = [
    (4, [127, 127, 1], 4), // 파멸 127 + 127 L = 762 at L5, 의지/4 (was 120 + 90 L = 570, /8)
    (5, [10, 5, 1], 0),    // 죽음의 검 10 + 5L % (with HZ-047 that is the kill chance)
    (6, [120, 114, 1], 2), // 심판 120 + 114 L = 690, 의지/2 (was 60 + 90 L = 510, /3)
    (8, [45, 45, 1], 9),   // 구원 45 + 45 L, 의지/9 (was 30 + 30 L, /17)
];

/// A spirit passive's new `A B C`, by id (`t/sp` order).
pub const SPIRIT_PASSIVES: [(usize, [i8; 3]); 1] = [
    (5, [1, 1, 3]), // 신경가속 (1+L)/3: +1 from L2, +2 at L5
];

pub const HSD1: &str = "/hero/hsd1";
pub const HSD2: &str = "/hero/hsd2";
pub const SPIRIT_A: &str = "/s/d/a";
pub const SPIRIT_P: &str = "/s/d/p";
const JOB_STRIDE: usize = 56;
const SKILL_STRIDE: usize = 7;
const SPIRIT_A_STRIDE: usize = 8;

/// The table `name` with the host's edits laid over the game's `bytes`, and the check byte redone.
/// `None` for a file this patch does not touch (the hook then hands the game's bytes back as they
/// are) or for bytes too short to be the file.
pub fn overlay(name: &str, bytes: &[u8]) -> Option<Vec<u8>> {
    let mut out = bytes.to_vec();
    let put = |out: &mut Vec<u8>, at: usize, abc: [i8; 3]| -> Option<()> {
        out.get_mut(at..at + 3)?.copy_from_slice(&abc.map(|v| v as u8));
        Some(())
    };
    match name {
        HSD1 => {
            for (job, skill, abc, sp, cool) in JOB_SKILLS {
                let at = job * JOB_STRIDE + skill * SKILL_STRIDE;
                put(&mut out, at, abc)?;
                out.get_mut(at + 3..at + 5)?.copy_from_slice(&sp.to_be_bytes());
                out.get_mut(at + 5..at + 7)?.copy_from_slice(&cool.to_be_bytes());
            }
        }
        HSD2 => {
            for (id, abc) in PASSIVES {
                put(&mut out, id * 3, abc)?;
            }
        }
        SPIRIT_A => {
            for (id, abc, wil) in SPIRIT_ACTIVES {
                put(&mut out, id * SPIRIT_A_STRIDE, abc)?;
                if wil != 0 {
                    *out.get_mut(id * SPIRIT_A_STRIDE + 3)? = wil as u8;
                }
            }
        }
        SPIRIT_P => {
            for (id, abc) in SPIRIT_PASSIVES {
                put(&mut out, id * 3, abc)?;
            }
        }
        _ => return None,
    }
    let (last, rest) = out.split_last_mut()?;
    *last = rest.iter().fold(0u8, |s, &b| s.wrapping_add(b));
    Some(out)
}

/// `(A + B·L) / C` as `db.a(II)I` computes it (the JVM's int division).
pub fn value(abc: [i8; 3], level: i32) -> i32 {
    let c = i32::from(abc[2]);
    if c == 0 {
        return 0;
    }
    (i32::from(abc[0]) + i32::from(abc[1]) * level) / c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checked(len: usize) -> Vec<u8> {
        let mut v: Vec<u8> = (0..len as u32).map(|i| (i * 7 % 251) as u8).collect();
        let sum = v[..len - 1].iter().fold(0u8, |s, &b| s.wrapping_add(b));
        v[len - 1] = sum;
        v
    }

    #[test]
    fn the_check_byte_is_redone_and_untouched_entries_stay() {
        let game = checked(512);
        let out = overlay(HSD1, &game).unwrap();
        assert_eq!(out.len(), 512);
        assert_eq!(out[511], out[..511].iter().fold(0u8, |s, &b| s.wrapping_add(b)));
        // 광기 (job 0 skill 4) and the passives (6, 7) are not in the list.
        assert_eq!((&out[28..35], &out[42..56]), (&game[28..35], &game[42..56]));
        // 강타's new numbers.
        assert_eq!(&out[..7], &[9, 3, 10, 0, 45, 0, 20]);
        assert_eq!(overlay("/hero/hs", &game), None, "another file is the game's");
        assert_eq!(overlay(HSD1, &[1, 2, 3]), None, "too short to be the file");
    }

    #[test]
    fn the_values_the_plan_names() {
        let at = |job: usize, skill: usize| JOB_SKILLS.iter().find(|s| s.0 == job && s.1 == skill).unwrap().2;
        assert_eq!((value(at(0, 5), 3), value(at(0, 5), 5)), (1, 2), "회전베기 ×1.4 / ×2.2 — int: 14/10, 22/10");
        assert_eq!((value(at(2, 1), 3) , value(at(2, 1), 5)), (0, 0), "회복 is scaled from max HP, not from 1");
        assert_eq!(value([3, 3, 40], 3) * 0 + (3 + 3 * 3) * 1000 / 40, 300, "회복 L3 = 30 % of max HP");
        assert_eq!((value(at(2, 5), 1), value(at(2, 5), 3), value(at(2, 5), 5)), (2, 4, 6), "전력방어 y = 1 + L");
        assert_eq!((value(at(3, 5), 3), value(at(3, 5), 5)), (40, 50), "저격");
        assert_eq!((value(at(1, 5), 3), value(at(1, 5), 5)), (300, 500), "분신 15 / 25 s");
        assert_eq!((value(at(5, 1), 3), value(at(5, 1), 5)), (360, 600), "동화 18 / 30 s");
        let p = |id: usize| PASSIVES.iter().find(|p| p.0 == id).unwrap().1;
        assert_eq!(100 * (100 + i32::from(p(0)[1]) * 3) / 100, 76, "지구력 L3: SP 76 %");
        assert_eq!(1000 * (0 + 15 * 3) / 100, 450, "냉혹함 L3: +45 % on a crit's extra");
        assert_eq!(value([1, 1, 3], 5), 2, "신경가속 L5 = +2");
        assert_eq!(value([1, 1, 3], 2), 1);
    }

    #[test]
    fn spirit_tables() {
        let a = overlay(SPIRIT_A, &checked(81)).unwrap();
        assert_eq!(&a[40..43], &[10, 5, 1], "죽음의 검");
        assert_eq!(&a[32..36], &[127, 127, 1, 4], "파멸, 의지/4");
        assert_eq!(&a[48..52], &[120, 114, 1, 2], "심판");
        assert_eq!(&a[64..68], &[45, 45, 1, 9], "구원");
        let p = overlay(SPIRIT_P, &checked(31)).unwrap();
        assert_eq!(&p[15..18], &[1, 1, 3]);
        let h2 = overlay(HSD2, &checked(61)).unwrap();
        assert_eq!(&h2[..3], &[100, 0xf8, 100]);
        assert_eq!(&h2[12..15], &checked(61)[12..15], "회피 stays");
    }
}
