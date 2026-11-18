//! HZ-068: one name for the fourth stat (user 2026-11-04, 「의지로 맞추자」). The options, the 마석, the help and the
//! spirit passives all say 의지, but the character window's four names are one picture, `rpg/e.w`, whose third cell
//! reads 지력. This redraws that cell: the game's 지 moves to the right half and a 의 in the sheet's own hand takes the
//! left. The sheet comes from the jar at run time; only 의, below, is ours.

/// A name's width in the sheet (`dy.a(Ldd;IIB)V` clips to 26 × 14 and slides the sheet by `cell × 26`).
pub const CELL_W: usize = 26;
/// The cell of stat 3: `dy.a(Ldd;IIB)V` @0..5 draws stat `n` from cell `(n + 3) % 4`.
const WILL_CELL: usize = 2;
/// 지's width at the cell's left, and where it goes to leave a gap after 의.
const SYLLABLE_W: usize = 12;
const RIGHT_AT: usize = 13;

/// The sheet's two inks.
const OUTLINE: u32 = 0xFF3F_1F3F;
const FILL: u32 = 0xFFDE_BE7B;

/// 의, drawn the way the sheet draws 힘's ㅇ and 지's ㅣ: `#` outline, `o` fill, `.` clear.
const WILL: [&str; 13] = [
    "..#####..###",
    ".##ooo##.#o#",
    ".#o###o#.#o#",
    ".#o###o#.#o#",
    ".#o###o#.#o#",
    ".##ooo##.#o#",
    "..#####..#o#",
    ".........#o#",
    "##########o#",
    "#oooooooo#o#",
    "##########o#",
    ".........#o#",
    ".........###",
];

/// The sheet with 의지 for 지력, `None` if it is not the sheet this expects (too small, or no ㅣ where 지 keeps it —
/// the game then draws its own). ARGB in, ARGB out (`Image.getRGB` / `createRGBImage`).
pub fn will_sheet(px: &[u32], w: usize, h: usize) -> Option<Vec<i32>> {
    let left = WILL_CELL * CELL_W;
    if w < left + CELL_W || h < WILL.len() || px.len() < w * h {
        return None;
    }
    // 지's ㅣ: outline across its top, fill down its middle.
    if px[left + 9..left + SYLLABLE_W] != [OUTLINE; 3] || px[w + left + 10] != FILL {
        return None;
    }
    let mut out: Vec<i32> = px.iter().map(|&p| p as i32).collect();
    for (row, will) in WILL.iter().enumerate() {
        let at = row * w + left;
        let jee = out[at..at + SYLLABLE_W].to_vec();
        out[at..at + CELL_W].fill(0);
        for (cell, ink) in out[at..at + SYLLABLE_W].iter_mut().zip(will.bytes()) {
            *cell = match ink {
                b'#' => OUTLINE as i32,
                b'o' => FILL as i32,
                _ => 0,
            };
        }
        out[at + RIGHT_AT..at + RIGHT_AT + SYLLABLE_W].copy_from_slice(&jee);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 4 * CELL_W;
    const H: usize = 14;

    /// A sheet of the game's shape: every pixel its own value, and 지's ㅣ in the third cell.
    fn sheet() -> Vec<u32> {
        let mut px: Vec<u32> = (0..W * H).map(|i| 0x1000_0000 + i as u32).collect();
        let left = WILL_CELL * CELL_W;
        px[left + 9..left + 12].copy_from_slice(&[OUTLINE; 3]);
        px[W + left + 10] = FILL;
        px
    }

    #[test]
    fn only_the_third_cell_changes() {
        let px = sheet();
        let out = will_sheet(&px, W, H).unwrap();
        let cell = WILL_CELL * CELL_W..(WILL_CELL + 1) * CELL_W;
        for (i, (&was, &now)) in px.iter().zip(&out).enumerate() {
            if !cell.contains(&(i % W)) || i / W >= WILL.len() {
                assert_eq!(was as i32, now, "pixel {i}");
            }
        }
    }

    #[test]
    fn the_cell_is_the_new_syllable_then_the_old_one() {
        let px = sheet();
        let out = will_sheet(&px, W, H).unwrap();
        let left = WILL_CELL * CELL_W;
        for row in 0..WILL.len() {
            let at = row * W + left;
            for (c, ink) in WILL[row].bytes().enumerate() {
                let want = match ink {
                    b'#' => OUTLINE as i32,
                    b'o' => FILL as i32,
                    _ => 0,
                };
                assert_eq!(out[at + c], want, "의 row {row} col {c}");
            }
            assert_eq!(out[at + SYLLABLE_W], 0, "the gap, row {row}");
            for c in 0..SYLLABLE_W {
                assert_eq!(out[at + RIGHT_AT + c], px[at + c] as i32, "지 row {row} col {c}");
            }
            assert_eq!(out[at + RIGHT_AT + SYLLABLE_W], 0, "지력's tail is gone, row {row}");
        }
    }

    #[test]
    fn a_sheet_it_does_not_know_stays_the_games() {
        assert_eq!(will_sheet(&sheet(), 3 * CELL_W - 1, H), None, "too narrow");
        assert_eq!(will_sheet(&sheet(), W, WILL.len() - 1), None, "too short");
        let mut other = sheet();
        other[WILL_CELL * CELL_W + 9] = 0;
        assert_eq!(will_sheet(&other, W, H), None, "no ㅣ");
    }

    #[test]
    fn every_row_of_the_syllable_fills_its_width() {
        assert!(WILL.iter().all(|r| r.len() == SYLLABLE_W));
    }
}
