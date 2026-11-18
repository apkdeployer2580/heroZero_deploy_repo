//! HZ-020: ten save slots as pages of the game's three (user 2026-10-25). See the patch's
//! `evidence` for the game side; this is the arithmetic the hooks share.
//!
//! Slot `n` (1-based) is the game's slot `(n − 1) % 3` on page `(n − 1) / 3`, so its stores are
//! `HOL.slot<n>` / `HOL.gamedata<n>` / `HOL.item<n>` — page 0 is the game's own three, and the
//! game encrypts a slot with its index's key, so slots 4, 7 and 10 share slot 1's.

use hz_patches::hooks::{SAVE_SLOTS, SLOT_PAGES, SLOTS_PER_PAGE, page_slots};

/// The 1-based slot number of the game's slot `index` on `page`.
pub fn slot_number(page: u8, index: u8) -> u32 {
    u32::from(page) * u32::from(SLOTS_PER_PAGE) + u32::from(index) + 1
}

/// The store `HOL.<kind><n>` of the game's slot `index` on `page` (`kind` = `slot` / `gamedata` /
/// `item`).
pub fn store_name(kind: &str, page: u8, index: u8) -> String {
    format!("HOL.{kind}{}", slot_number(page, index))
}

/// The page and the game's index of a 0-based slot (`None` past the last slot).
pub fn page_of(slot: u8) -> Option<(u8, u8)> {
    (slot < SAVE_SLOTS).then_some((slot / SLOTS_PER_PAGE, slot % SLOTS_PER_PAGE))
}

/// The page `delta` steps from `page`, round the end.
pub fn turn(page: u8, delta: i32) -> u8 {
    (i32::from(page) + delta).rem_euclid(i32::from(SLOT_PAGES)) as u8
}

/// The slot window's row whose box the game draws at `y` (`bb.a(dd)V`: `o.b + 16 + 15·row`; the
/// label one pixel higher), for its frame at `frame_y`.
pub fn row_at(frame_y: i32, y: i32) -> i32 {
    (y - frame_y - 16).div_euclid(15)
}

/// Whether `row` of `page` is a slot (the last page has fewer).
pub fn has_row(page: u8, row: i32) -> bool {
    (0..i32::from(page_slots(page))).contains(&row)
}

/// 「저장 N」 with N the slot's number: the game's label (`rpg/e.b[27]` + its ASCII `row + 1`) with
/// the trailing digits replaced.
pub fn relabel(label: &[u8], number: u32) -> Vec<u8> {
    let end = label.iter().rposition(|b| !b.is_ascii_digit()).map_or(0, |i| i + 1);
    [&label[..end], number.to_string().as_bytes()].concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_slots_are_four_pages_and_page_zero_is_the_game() {
        assert_eq!(SLOT_PAGES, 4);
        assert_eq!((0..SLOT_PAGES).map(page_slots).collect::<Vec<_>>(), vec![3, 3, 3, 1]);
        assert_eq!(store_name("slot", 0, 0), "HOL.slot1", "the game's own");
        assert_eq!(store_name("gamedata", 0, 2), "HOL.gamedata3");
        assert_eq!(store_name("item", 1, 0), "HOL.item4");
        assert_eq!(store_name("slot", 3, 0), "HOL.slot10");
        assert_eq!((page_of(0), page_of(5), page_of(9), page_of(10)), (Some((0, 0)), Some((1, 2)), Some((3, 0)), None));
        assert_eq!((turn(0, -1), turn(3, 1), turn(1, 1)), (3, 0, 2));
    }

    #[test]
    fn rows_and_labels() {
        // Frame at y 33: boxes at 49, 64, 79; labels a pixel higher.
        assert_eq!((row_at(33, 49), row_at(33, 64), row_at(33, 79), row_at(33, 48)), (0, 1, 2, -1), "a label is drawn at its box − 1: pass y + 1");
        assert!(has_row(3, 0) && !has_row(3, 1) && has_row(2, 2));
        let word = [0xc0, 0xfa, 0xc0, 0xe5]; // 저장
        assert_eq!(relabel(&[&word[..], b"2"].concat(), 5), [&word[..], b"5"].concat());
        assert_eq!(relabel(&[&word[..], b"3"].concat(), 10), [&word[..], b"10"].concat());
        assert_eq!(relabel(&word, 4), [&word[..], b"4"].concat(), "no number: one is added");
    }
}
