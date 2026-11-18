//! HZ-060 「엘릭서」 — HP and SP back to full, sold in the shop's 기타 tab (user 2026-10-30: 1,000원,
//! the potion's picture in white). The pure parts; the JVM side is `hooks`.
//!
//! The special bag's eleventh item, `/itm/20` number 10 (`hz_patches::hooks::ELIXIR`): the file holds
//! 0..8 and HZ-016's orb is 9, so its record is handed to the game's item factory by the host as the
//! orb's is, and the shop's loop over the tab reaches it. The game drinks a potion only in `ek.a(Z)Z`
//! (the `0` / `#` slots and HZ-058's plates), by number: 2 포션 HP 100 · 3 포도주 SP 80 · 6 하이 포션
//! HP 600 · 7 상급 회복 포션 HP 1000 SP 80 · 8 브랜디 SP 300 (`/itm/20`, javap 2026-10-30).

use hz_patches::hooks::ELIXIR;

use crate::text::euc_kr;

/// The name in the bag and the shop.
pub const NAME: &str = "엘릭서";
/// What the shop asks (user 2026-10-30). The game's own potions ask 80 .. 700.
pub const PRICE: i32 = 1_000;
/// The bag's 상세정보 line, in the potions' style (「HP 1000 SP 80 회복」).
pub const TEXT: &str = "HP SP 100% 회복";

/// The special items the app knows, `/itm/20` 0..=10 — what a save file says it was made with
/// (`hz_android::save_transfer`, the `specials` line): an app that knows fewer would drop the rest.
pub const SPECIALS: u32 = ELIXIR.1 as u32 + 1;

/// The record `rpg/e.a(String,I)[B` returns for an `/itm/20` item: name (length byte + EUC-KR),
/// price (big-endian int), text (length byte + EUC-KR), then the check byte `rpg/e.a([B)Z` wants —
/// the byte sum of everything before it (`revive::record`'s layout).
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

/// The spirit skills `/s/d/a` holds (81 bytes: a count, then eight bytes each).
pub const SPIRIT_SKILLS: i8 = 10;

/// The number a quick slot's cooldown is read under (2026-11-02, user report 「정령사로 직업변경 →
/// 무한로딩, 이어하기가 안돼」): `ed.g(I)V` takes every slot past the job's two from the SPIRIT's table —
/// the `0` / `#` potion slots too, by the potion's number (`p.d(B)I` = the short at `id * 8 + 5`). The
/// game's potions 2..8 and the orb's 9 fall inside it; the elixir's 10 fell past its end, an
/// ArrayIndexOutOfBoundsException wherever the six are set again — the job change (it stayed on
/// 「직업 변경 중...」) and every load. The elixir reads as the potion it drinks as; any other number as
/// it was.
pub fn cooldown_number(id: i8) -> i8 {
    if id == ELIXIR.1 { hz_patches::hooks::ELIXIR_DRINK as i8 } else { id }
}

/// Whether `path`/`index` is the elixir's record (`ba.a(BB)Lba;` asks `/itm/20` for number 10).
pub fn is_elixir_record(path: &str, index: i32) -> bool {
    path == format!("/itm/{:02}", ELIXIR.0) && index == i32::from(ELIXIR.1)
}

/// The game's picture it is drawn from: 포션 (`rpg/e.k` 20 + 2), the round flask.
pub const BASE_PICTURE: i32 = ELIXIR.0 as i32 + 2;

/// 포션's flask in white (user 2026-10-30, 「포션 아이콘에서 하얀색계열」): each pixel by its brightness
/// into five bands — black stays, the dark outline goes slate so the flask keeps its edge, the rest
/// pearl to white. Clear pixels stay clear. ARGB in, ARGB out (`Image.getRGB` / `createRGBImage`).
pub fn whiten(px: &[u32]) -> Vec<i32> {
    const BANDS: [(u32, u32); 4] = [(80, 0x38_3E_54), (130, 0x9E_A8_C2), (180, 0xD4_DA_EA), (230, 0xF0_F3_FA)];
    px.iter()
        .map(|&p| {
            let a = p >> 24;
            let (r, g, b) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
            let lum = (299 * r + 587 * g + 114 * b) / 1000;
            let rgb = if a == 0 || lum == 0 {
                p & 0xFF_FFFF
            } else {
                BANDS.iter().find(|(top, _)| lum <= *top).map_or(0xFF_FFFF, |(_, c)| *c)
            };
            ((a << 24) | rgb) as i32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The record parses the way `ba.a([BI)I` reads one and passes `rpg/e.a([B)Z`'s check — the orb's
    /// layout, the file's potions' layout.
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
    fn only_the_elixir_record_is_answered() {
        assert!(is_elixir_record("/itm/20", 10));
        assert!(!is_elixir_record("/itm/20", 9) && !is_elixir_record("/itm/16", 10) && !is_elixir_record("/itm/2", 10));
        assert_eq!((BASE_PICTURE, SPECIALS), (22, 11));
    }

    /// The potion keys take it: the game's 0/# rule (number 2 and up) without the orb.
    #[test]
    fn the_potion_keys_take_it() {
        assert!(crate::revive::quick_item(ELIXIR.0, ELIXIR.1));
    }

    /// A slot's cooldown is a two-byte read at `id * 8 + 5` of the spirit table: the elixir's number is
    /// past it, so it reads as 포션 (2); every number the table holds — the game's potions, the orb, the
    /// spirit skills of the 7 / 9 slots — as it was.
    #[test]
    fn a_slot_cooldown_reads_the_elixir_as_a_potion() {
        const TABLE: i32 = 1 + SPIRIT_SKILLS as i32 * 8;
        let past_the_end = |id: i8| i32::from(id) * 8 + 5 + 2 > TABLE;
        assert!(past_the_end(ELIXIR.1) && !past_the_end(9), "the elixir alone falls off the table");
        assert_eq!(cooldown_number(ELIXIR.1), 2);
        assert!(!past_the_end(cooldown_number(ELIXIR.1)));
        for id in 0..SPIRIT_SKILLS {
            assert_eq!(cooldown_number(id), id);
        }
    }

    /// Clear stays clear, black stays black, the outline darkens to slate, a highlight goes white.
    #[test]
    fn whitening_keeps_the_flask_and_its_edge() {
        let px = [0x0000_0000, 0xFF00_0000, 0xFF60_3018, 0xFFA0_5020, 0xFFFF_FFF0, 0x80C0_6020];
        let out: Vec<u32> = whiten(&px).into_iter().map(|p| p as u32).collect();
        assert_eq!(out[0], 0, "clear");
        assert_eq!(out[1], 0xFF00_0000, "black");
        assert_eq!(out[2], 0xFF38_3E54, "the dark outline");
        assert_eq!(out[3], 0xFF9E_A8C2, "the orange body's shade");
        assert_eq!(out[4], 0xFFFF_FFFF, "the highlight");
        assert_eq!(out[5] >> 24, 0x80, "alpha kept");
        assert!(out.iter().skip(2).all(|&p| { let (r, g, b) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF); b >= g && g >= r }), "grey to pearl, never warm");
    }
}
