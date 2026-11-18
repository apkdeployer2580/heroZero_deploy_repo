//! HZ-023 가디언 장비 — one piece past the game's best in every equipment category (user 2026-10-25:
//! *"새 아이템 추가할꺼야. hero2처럼"*; hero2's first new tier was its 「가디언」 gear, the end-game best
//! ×1.25, dropped after the level cap — HERO2_GUARDIAN_PLAN.md §2.6). The pure parts; the JVM side is
//! `hooks`.
//!
//! The game's gear stops at LV 65 (the recipe-only `[30]` of most files) or LV 60, its own second
//! replay round has monsters of LV 60..87 (`cq.i()V`), and the guardian level (HZ-022) carries the
//! player past 99. Each piece is number [`GEAR_ID`] of its file, served to the item factory in the
//! files' own layout (`tools_re/hz_items.py`).
//!
//! A piece in a save keeps its category, number, its three stats, `h` and its six options
//! (`bx.a(DataOutputStream)`); name, price, level, grade, sprite and palette are read from here at
//! every load. So stats freeze when a piece is made, the rest can be changed later — and the number
//! never.
//!
//! ⚠ Names and numbers are Claude's draft (2026-10-25), in hero2's way: the category's best ×1.25
//! (stats rounded half up), price ×1.5, LV 80, one option more than that best, its sprite and
//! palette. Revisable before the user's play reaches them.

use hz_patches::hooks::{GEAR_CATEGORIES, GEAR_ID, RARE_TOP};

use crate::text::euc_kr;

/// One piece's record fields, in the order `bx.a([BI)I` reads them.
#[derive(Clone, Copy, Debug)]
pub struct Piece {
    pub category: i8,
    pub name: &'static str,
    pub price: i32,
    pub sprite: i8,
    pub palette: i16,
    /// `b c d`: attack low / high (weapons, shields) and defence (armour, shields); −1 = none.
    pub stats: [i16; 3],
    /// Weapons and shields: the critical rate the item window shows after the attack (「15-23/9%」).
    pub h: i8,
    /// 0..15 = a 마법석 effect (100 = rolled on load), −1 an empty socket, −2 a closed one, −3 none.
    pub options: [i8; 6],
}

/// Every piece's level: past the game's 65, under round 2's monsters (LV 75 and up drop them).
pub const LEVEL: i8 = 80;
/// 에픽 (`t/it` grades 일반 · 레어 · 에픽 · 세트).
pub const GRADE: i8 = 2;
/// No set (`f`; the game's sets are 2..5).
pub const SET: i8 = -1;
/// The text the files give every piece (the game never shows it).
const TEXT: &str = "A";

const R: i8 = 100;
const WEAPON: [i8; 6] = [R, R, R, R, -1, -2];
const BODY: [i8; 6] = [R, R, R, -1, -3, -3];
const LIMB: [i8; 6] = [R, R, -1, -3, -3, -3];

/// The tier, by category (`/itm/00`..`/itm/15`). Each line names the piece it is ×1.25 of.
pub const PIECES: [Piece; GEAR_CATEGORIES as usize] = [
    // 블러드캘러미티 LV 65: 108–435 / 15 %, 19,500
    Piece { category: 0, name: "가디언 액스", price: 29_250, sprite: 15, palette: -1, stats: [135, 544, -1], h: 16, options: WEAPON },
    // 붉은사자대검 LV 65 (세트): 242–364 / 12 %, 26,000
    Piece { category: 1, name: "가디언 블레이드", price: 39_000, sprite: 15, palette: -1, stats: [303, 455, -1], h: 13, options: WEAPON },
    // 블리츠블럿 LV 65: 170–204 / 5 %, 15,600
    Piece { category: 2, name: "가디언 세이버", price: 23_400, sprite: 14, palette: -1, stats: [213, 255, -1], h: 6, options: WEAPON },
    // 묘르닐 LV 65: 77–231 / 7 %, 15,600
    Piece { category: 3, name: "가디언 메이스", price: 23_400, sprite: 14, palette: -1, stats: [96, 289, -1], h: 8, options: WEAPON },
    // 아웃버스트 LV 65 (세트): 83–102 / 21 %, 15,600
    Piece { category: 4, name: "가디언 대거", price: 23_400, sprite: 15, palette: -1, stats: [104, 128, -1], h: 22, options: WEAPON },
    // 헤일스톰 LV 65 (세트): 73–109 / 13 %, 20,800
    Piece { category: 5, name: "가디언 캐논", price: 31_200, sprite: 14, palette: -1, stats: [91, 136, -1], h: 14, options: WEAPON },
    // 크림슨 라이언 LV 65 (세트): 132–198 / 12 %, 31,200
    Piece { category: 6, name: "가디언 베요넷", price: 46_800, sprite: 8, palette: -1, stats: [165, 248, -1], h: 13, options: WEAPON },
    // 붉은사자 방패 LV 65 (세트): 92–138, 방어 30 / 12 %, 13,000
    Piece { category: 7, name: "가디언 실드", price: 19_500, sprite: 14, palette: -1, stats: [115, 173, 38], h: 13, options: WEAPON },
    // 빙해의 장갑 LV 60 (세트): 방어 22, 12,000
    Piece { category: 8, name: "가디언 글러브", price: 18_000, sprite: 10, palette: 145, stats: [-1, -1, 28], h: -1, options: LIMB },
    // 신전기사건틀릿 LV 60 (세트): 방어 28, 19,200
    Piece { category: 9, name: "가디언 건틀릿", price: 28_800, sprite: 9, palette: -1, stats: [-1, -1, 35], h: -1, options: LIMB },
    // 봉인의 고글 LV 65 (세트): 방어 31, 26,000 — the game's helmets close a socket (−2)
    Piece { category: 10, name: "가디언 헬름", price: 39_000, sprite: 17, palette: -1, stats: [-1, -1, 39], h: -1, options: [R, R, -2, -2, -3, -3] },
    // 봉인의 줄 LV 55 (세트): options only, 22,000
    Piece { category: 11, name: "가디언의 인장", price: 33_000, sprite: -1, palette: -1, stats: [-1, -1, -1], h: -1, options: [R, R, R, R, R, -1] },
    // 흑암의 흉갑 LV 65: 방어 56, 15,600
    Piece { category: 12, name: "가디언 레더", price: 23_400, sprite: 2, palette: 191, stats: [-1, -1, 70], h: -1, options: BODY },
    // 단층의 갑옷 LV 65: 방어 70, 19,500
    Piece { category: 13, name: "가디언 플레이트", price: 29_250, sprite: 1, palette: 207, stats: [-1, -1, 88], h: -1, options: BODY },
    // 빙해의 신발 LV 60 (세트): 방어 28, 14,400
    Piece { category: 14, name: "가디언 부츠", price: 21_600, sprite: 6, palette: 246, stats: [-1, -1, 35], h: -1, options: LIMB },
    // 신전기사의 부츠 LV 60 (세트): 방어 34, 19,200
    Piece { category: 15, name: "가디언 그리브", price: 28_800, sprite: 6, palette: 268, stats: [-1, -1, 43], h: -1, options: LIMB },
];

/// The piece of `category`, if it has one.
pub fn piece(category: i8) -> Option<&'static Piece> {
    PIECES.iter().find(|p| p.category == category)
}

/// The category whose file `path` is (`ba.a(BB)Lba;` reads `/itm/` + two digits), if it is equipment.
pub fn category_of(path: &str) -> Option<i8> {
    let digits = path.strip_prefix("/itm/")?;
    (digits.len() == 2 && digits.bytes().all(|b| b.is_ascii_digit())).then_some(())?;
    let cat: i8 = digits.parse().ok()?;
    (0..GEAR_CATEGORIES).contains(&cat).then_some(cat)
}

/// The piece `path` / `index` asks the factory for, if it is one.
pub fn asked(path: &str, index: i32) -> Option<&'static Piece> {
    (index == i32::from(GEAR_ID)).then_some(())?;
    piece(category_of(path)?)
}

/// The record `rpg/e.a(String,I)[B` returns for a piece: `ba.a([BI)I`'s name (length byte + EUC-KR),
/// price (big-endian int) and text, then `bx.a([BI)I`'s level, grade, set, sprite, palette and the
/// three stats (big-endian shorts), `h`, the six options, and the check byte `rpg/e.a([B)Z` wants —
/// the byte sum of everything before it.
pub fn record(p: &Piece) -> Vec<u8> {
    record_at(p, LEVEL, GRADE)
}

/// [`record`] for a piece of another tier (HZ-031's LV 85 「전설」).
pub fn record_at(p: &Piece, level: i8, grade: i8) -> Vec<u8> {
    let name = euc_kr(p.name).expect("in the game's font");
    let text = euc_kr(TEXT).expect("ASCII");
    let mut r = vec![name.len() as u8];
    r.extend(&name);
    r.extend(p.price.to_be_bytes());
    r.push(text.len() as u8);
    r.extend(&text);
    r.extend([level as u8, grade as u8, SET as u8, p.sprite as u8]);
    r.extend(p.palette.to_be_bytes());
    for s in p.stats {
        r.extend(s.to_be_bytes());
    }
    r.push(p.h as u8);
    r.extend(p.options.map(|o| o as u8));
    r.push(r.iter().fold(0u8, |s, &b| s.wrapping_add(b)));
    r
}

/// One in this many of the rare band's top drops is a piece.
pub const ODDS: i32 = 4;

/// Whether a drop at the rare band's top may be a piece — the hook draws the game's Random only then.
pub fn at_top(band: i32, top: i32) -> bool {
    top == RARE_TOP && band >= top
}

/// The rare band's number for `band` (`lvl / 5 + 13`) under `top` (28): `min` as the game has it,
/// except at the top, where `roll` (the game's `abs(nextInt())`) makes one in [`ODDS`] a piece.
pub fn rare_drop(band: i32, top: i32, roll: i32) -> i32 {
    if at_top(band, top) && roll.rem_euclid(ODDS) == 0 { i32::from(GEAR_ID) } else { band.min(top) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse a record the way `ba.a([BI)I` + `bx.a([BI)I` do.
    fn parse(r: &[u8]) -> (String, i32, i8, i8, i8, i8, i16, [i16; 3], i8, [i8; 6]) {
        let n = r[0] as usize;
        let name = encoding_rs::EUC_KR.decode(&r[1..1 + n]).0.into_owned();
        let mut p = 1 + n;
        let price = i32::from_be_bytes(r[p..p + 4].try_into().unwrap());
        p += 4;
        p += 1 + r[p] as usize;
        let [level, grade, set, sprite] = [r[p], r[p + 1], r[p + 2], r[p + 3]].map(|b| b as i8);
        p += 4;
        let short = |p: usize| i16::from_be_bytes([r[p], r[p + 1]]);
        let palette = short(p);
        let stats = [short(p + 2), short(p + 4), short(p + 6)];
        p += 8;
        let h = r[p] as i8;
        let options: [i8; 6] = r[p + 1..p + 7].iter().map(|&b| b as i8).collect::<Vec<_>>().try_into().unwrap();
        assert_eq!(p + 7, r.len() - 1, "the check byte is last");
        (name, price, level, grade, set, sprite, palette, stats, h, options)
    }

    /// The game's own LV 65 블러드캘러미티 (`/itm/00` [30], 41 bytes — hz_items.py), rebuilt from its
    /// fields: the layout is the file's to the byte.
    #[test]
    fn a_record_has_the_files_layout() {
        let file = [
            0x0e, 0xba, 0xed, 0xb7, 0xaf, 0xb5, 0xe5, 0xc4, 0xb6, 0xb7, 0xaf, 0xb9, 0xcc, 0xc6, 0xbc, 0x00, 0x00, 0x4c, 0x2c, 0x01,
            0x41, 0x41, 0x02, 0xff, 0x0f, 0xff, 0xff, 0x00, 0x6c, 0x01, 0xb3, 0xff, 0xff, 0x0f, 0x64, 0x64, 0x64, 0xff, 0xff, 0xfe,
            0xfa,
        ];
        let same = Piece { category: 0, name: "블러드캘러미티", price: 19_500, sprite: 15, palette: -1, stats: [108, 435, -1], h: 15, options: [R, R, R, -1, -1, -2] };
        let mut r = record(&same);
        // `record` writes LV 80; the file's piece is 65 (0x41) — set it, then fix the check byte.
        let at = 1 + 14 + 4 + 2;
        r[at] = 65;
        let n = r.len();
        r[n - 1] = r[..n - 1].iter().fold(0u8, |s, &b| s.wrapping_add(b));
        assert_eq!(r, file);
    }

    #[test]
    fn every_category_has_one_piece_that_parses_back() {
        for (i, p) in PIECES.iter().enumerate() {
            assert_eq!(p.category, i as i8, "in category order");
            let r = record(p);
            assert!(r.len() < 256, "a record's u16 length");
            assert_eq!(r[r.len() - 1], r[..r.len() - 1].iter().fold(0u8, |s, &b| s.wrapping_add(b)), "check byte");
            let (name, price, level, grade, set, sprite, palette, stats, h, options) = parse(&r);
            assert_eq!((name.as_str(), price, level, grade, set), (p.name, p.price, LEVEL, GRADE, SET));
            assert_eq!((sprite, palette, stats, h, options), (p.sprite, p.palette, p.stats, p.h, p.options));
            assert!(p.name.chars().count() <= 8, "{}: the item window's name band", p.name);
            // A weapon or shield has an attack range; armour a defence; the accessory neither.
            match p.category {
                0..=7 => assert!(0 < stats[0] && stats[0] < stats[1], "{}", p.name),
                11 => assert_eq!(stats, [-1, -1, -1]),
                _ => assert!(stats[..2] == [-1, -1] && stats[2] > 0, "{}", p.name),
            }
            assert!(options.iter().all(|&o| o == R || (-3..=-1).contains(&o)));
        }
    }

    #[test]
    fn only_number_40_of_an_equipment_file_is_a_piece() {
        assert_eq!(asked("/itm/00", 40).map(|p| p.name), Some("가디언 액스"));
        assert_eq!(asked("/itm/15", 40).map(|p| p.name), Some("가디언 그리브"));
        assert!(asked("/itm/00", 30).is_none(), "the file's own");
        assert!(asked("/itm/16", 40).is_none() && asked("/itm/20", 40).is_none(), "not equipment");
        assert!(asked("/itm/1", 40).is_none() && asked("itm/01", 40).is_none() && asked("/itm/0a", 40).is_none());
    }

    #[test]
    fn the_rare_band_is_the_games_min_below_its_top() {
        for lvl in 14..=127 {
            let band = lvl / 5 + 13;
            for roll in 0..8 {
                let got = rare_drop(band, RARE_TOP, roll);
                if lvl < 75 {
                    assert_eq!(got, band.min(RARE_TOP), "LV {lvl}");
                    assert!(!at_top(band, RARE_TOP), "no draw below LV 75");
                } else {
                    assert_eq!(got, if roll % ODDS == 0 { i32::from(GEAR_ID) } else { RARE_TOP }, "LV {lvl}");
                }
            }
        }
        assert_eq!(rare_drop(28, 14, 0), 14, "a different cap is never the rare band");
    }
}
