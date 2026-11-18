//! HZ-016 「부활의 구」 — the item this game never had, and the question its death asks when the bag
//! holds one (user 2026-10-24: hero2's 부활의 구). The pure parts; the JVM side is `hooks`.
//!
//! The orb is the special bag's tenth item, `/itm/20` number 9 (`hz_patches::hooks::REVIVE_ORB`):
//! the file holds 0..8, so its record is handed to the game's item factory by the host, in the
//! file's own layout, and the shop's loop over the tab reaches it.

use hz_patches::hooks::REVIVE_ORB;

use crate::text::euc_kr;

/// The name in the bag and the shop.
pub const NAME: &str = "부활의 구";
/// What the shop asks — the user's 1,500 (2026-10-30; hero2's 5,000 until then). Every other item
/// keeps the game's own price (gear 80 .. 20,000, potions 80 .. 700) but HZ-060's 엘릭서 (1,000).
pub const PRICE: i32 = 1_500;
/// The bag's 상세정보 line, in the special items' style (「HP 100 회복」, 「지정된 장소로 귀환」) and the
/// spirit skill 윤회's wording (「사망 시 일정 확률로 그 자리에서 부활한다.」).
pub const TEXT: &str = "사망 시 그 자리에서 부활";

/// The record `rpg/e.a(String,I)[B` returns for an `/itm/20` item: name (length byte + EUC-KR),
/// price (big-endian int, `rpg/g.a([BI)I`), text (length byte + EUC-KR), then the check byte
/// `rpg/e.a([B)Z` wants — the byte sum of everything before it.
pub fn record() -> Vec<u8> {
    let name = euc_kr(NAME).expect("in the game's font");
    let text = euc_kr(TEXT).expect("in the game's font");
    let mut r = vec![name.len() as u8];
    r.extend(&name);
    r.extend(PRICE.to_be_bytes());
    r.push(text.len() as u8);
    r.extend(&text);
    r.push(r.iter().fold(0u8, |s, &b| s.wrapping_add(b)));
    r
}

/// Whether `path`/`index` is the orb's record (`ba.a(BB)Lba;` asks `/itm/20` for number 9).
pub fn is_orb_record(path: &str, index: i32) -> bool {
    path == format!("/itm/{:02}", REVIVE_ORB.0) && index == i32::from(REVIVE_ORB.1)
}

/// `ba.a()Z` with HZ-016: the bag offers 「0번 등록 / #에 등록」 for the special bag from number 2
/// (the game's own rule — what the `0`/`#` keys drink), but not for the orb, which only the death
/// uses (hero2: its popup is 상세보기·버리기 alone).
pub fn quick_item(cat: i8, id: i8) -> bool {
    cat == 20 && id >= 2 && (cat, id) != REVIVE_ORB
}

/// The question, EUC-KR, `;` between lines — the game's own forced break (`ah` draws and counts
/// lines on it; `t/it` [53] 「가방에 공간이 부족;합니다.」): hero2's lines — 쓰러졌습니다. /
/// 부활의 구를 사용하여 / 부활하시겠습니까? / 보유 N개.
pub fn question(held: i32) -> Vec<u8> {
    let lines = ["쓰러졌습니다.".to_string(), "부활의 구를 사용하여".into(), "부활하시겠습니까?".into(), format!("보유 {}개", held.max(0))];
    lines.iter().map(|l| euc_kr(l).expect("in the game's font")).collect::<Vec<_>>().join(&b';')
}

/// What a `cf` answer row means for the question: row 0 is 예.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Revive,
    GameOver,
}

pub fn answer(row: i32) -> Answer {
    if row == 0 { Answer::Revive } else { Answer::GameOver }
}

/// The corner of the tile a position (`er.h` / `er.i`, px) counts in — `er.E()V`'s `h >> 4`, back to px
/// as `ek.p(I)V` places the player. The revive stands the player there: a fall between two tiles left
/// `er.k` / `er.l` set, and a step's check then tested no tile (walks through walls — see the hook).
pub fn tile_corner(x: i16, y: i16) -> (i16, i16) {
    ((x >> 4) << 4, (y >> 4) << 4)
}

/// The orb's picture: a glass ball with a flame in it, 12×12 like the pack's special items (the
/// pack has no spare picture — 29 is the selection frame), outlined like them.
pub const ICON_W: usize = 12;
pub const ICON_H: usize = 12;
const ICON: [&str; ICON_H] = [
    "....OOOO....",
    "..OOggggOO..",
    ".OgWWggggbO.",
    ".OgWgggYgbO.",
    "OgggggYFgbbO",
    "OggggYFFYgbO",
    "OgggYFRFYgbO",
    "OgggYFRRFYbO",
    ".OggYFRFYbO.",
    ".ObgYFFFYbO.",
    "..OObbbbOO..",
    "....OOOO....",
];

/// The picture as ARGB (`Image.createRGBImage` with alpha), row by row.
pub fn icon() -> Vec<i32> {
    ICON.iter()
        .flat_map(|row| row.bytes())
        .map(|c| match c {
            b'O' => 0xFF1E_1A28u32,
            b'W' => 0xFFFF_FFFF,
            b'g' => 0xFFB0_D6EC,
            b'b' => 0xFF5C_84B0,
            b'Y' => 0xFFFF_CE48,
            b'F' => 0xFFFF_8020,
            b'R' => 0xFFFF_F6BE,
            _ => 0,
        } as i32)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-01 (잊혀진땅-설원): a fall mid-step stands up on the tile `er.E()` counts it in; one on a
    /// tile stays.
    #[test]
    fn a_fall_between_tiles_stands_on_one() {
        assert_eq!(tile_corner(328, 240), (320, 240), "the probe's fall: x tile 20.5 -> 20");
        assert_eq!(tile_corner(352, 272), (352, 272), "on a tile: unchanged");
        assert_eq!(tile_corner(336, 248), (336, 240), "between rows too");
        assert_eq!(tile_corner(15, 31), (0, 16));
    }

    /// The record parses the way `ba.a([BI)I` reads one and passes `rpg/e.a([B)Z`'s check, in the
    /// same layout as the file's own potion (`/itm/20` number 2: 포션, 80, HP 100 회복, check).
    #[test]
    fn the_record_is_one_the_game_reads() {
        let r = record();
        let name_len = r[0] as usize;
        assert_eq!(encoding_rs::EUC_KR.decode(&r[1..1 + name_len]).0, NAME);
        let at = 1 + name_len;
        assert_eq!(i32::from_be_bytes(r[at..at + 4].try_into().unwrap()), PRICE);
        let text_len = r[at + 4] as usize;
        assert_eq!(encoding_rs::EUC_KR.decode(&r[at + 5..at + 5 + text_len]).0, TEXT);
        assert_eq!(r.len(), at + 5 + text_len + 1, "one check byte after the text, nothing else");
        let sum = r[..r.len() - 1].iter().fold(0i8, |s, &b| s.wrapping_add(b as i8));
        assert_eq!(sum, *r.last().unwrap() as i8, "rpg/e.a([B)Z");
    }

    #[test]
    fn only_the_orb_record_is_answered() {
        assert!(is_orb_record("/itm/20", 9));
        assert!(!is_orb_record("/itm/20", 8) && !is_orb_record("/itm/19", 9) && !is_orb_record("/itm/2", 9));
    }

    /// The game's rule for the 0/# popup, minus the orb.
    #[test]
    fn the_orb_is_not_offered_to_the_potion_keys() {
        assert!(!quick_item(20, 0) && !quick_item(20, 1), "제련석, 귀환서");
        assert!((2..=8).all(|id| quick_item(20, id)), "the potions");
        assert!(!quick_item(REVIVE_ORB.0, REVIVE_ORB.1));
        assert!(!quick_item(19, 5) && !quick_item(0, 3));
    }

    /// Four lines in the game's font, the count in ASCII digits.
    #[test]
    fn the_question_draws_in_the_games_font() {
        let q = question(3);
        let text = encoding_rs::EUC_KR.decode(&q).0;
        assert_eq!(text, "쓰러졌습니다.;부활의 구를 사용하여;부활하시겠습니까?;보유 3개");
        assert_eq!(q.iter().filter(|&&b| b == b';').count(), 3, "no ';' inside a Hangul pair (trail bytes are 0xA1..)");
        assert!(encoding_rs::EUC_KR.decode(&question(-1)).0.ends_with("보유 0개"));
        assert_eq!((answer(0), answer(1)), (Answer::Revive, Answer::GameOver));
    }

    #[test]
    fn the_picture_is_square_and_outlined() {
        assert!(ICON.iter().all(|r| r.len() == ICON_W));
        let px = icon();
        assert_eq!(px.len(), ICON_W * ICON_H);
        assert_eq!(px[0], 0, "transparent corner");
        assert_eq!(px[4] as u32, 0xFF1E_1A28, "outline on the top edge");
        assert!(px.iter().all(|&p| p == 0 || (p as u32) >> 24 == 0xFF), "opaque or clear, nothing between");
    }
}
