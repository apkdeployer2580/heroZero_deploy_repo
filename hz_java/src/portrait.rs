//! The status block's portrait (Phase 6-6, 2026-10-23): the head inside the ring. hero2's ring
//! shows the head its part loader loaded; this game draws its hero from layered parts too, so the
//! hook composes the frame the game would draw for the hero standing and facing the viewer, from
//! the game's own pictures, and keeps the square round the head. A helmet, a new class or the
//! buff head (`/hero/0/t`) shows because the pictures are the live ones.
//!
//! The doll (javap `t` `cn` `rpg/e` `ek`, 2026-10-23) is one `cn extends t`, the static
//! `rpg/e.a:Lcn;`, built by `rpg/e.a()` from the class's animation file `/hero/h<class>` and its
//! eight parts (`rpg/e.a(cn, part)`, equipment slot from `rpg/e.a:[B` = {1 4 0 6 -1 -1 3 5}):
//!
//!   * 0 the head — `/hero/0/NN`, NN = the helmet's sprite `bx.g` (0 bare); `cn.a(true)` swaps in
//!     `/hero/0/t` while a buff runs (`ek.n(I)`);
//!   * 1..3 the body — `/hero/1..3/[l]NN`, slots 4 0 6;
//!   * 4 the class's effects `/hero/4/<class>`, 5 `/hero/eh`;
//!   * 6 and 7 weapons — `/hero/w/<category><NN>`, slots 3 5.
//!
//! Colours are the item's palette (`/hero/h.p`), baked into the pictures at load. The portrait
//! takes parts 0..=3: the effects are not the hero, and a weapon held out to the side
//! (warrior: 30 px left of the feet) is not a face.
//!
//! `t.a([BZ)V` reads the animation file into `t.a:B` the layer count L, `t.a:[[Lcw;` the
//! pictures — slot `p` part `p`, slot `L + p` the same part mirrored —, `t.a:[I` each action's
//! offset into `t.a:[B`, and `t.b:[B` its frame count. An action at its offset is its frame
//! count, then per frame FOUR directions (1-based: 1 up, 2 down, 3 and 4 the sides — the step
//! tables `rpg/d.V/W`), each a count and that many 4-byte entries `part | 0x80 if mirrored ·
//! picture · dx · dy`, drawn in order with the picture's top-left at the feet + (dx, dy)
//! (`t.a(BBIII)I`). Action 0 is the hero standing at ease (`ek.a:B` in a town; 1 is the stance in
//! the wild, 8 under a buff).

use std::sync::Arc;

use crate::state::{Icon, IconKey};

/// The action composed: standing at ease.
pub const STAND: usize = 0;
/// The direction composed (1-based): down, facing the viewer.
pub const FACING_VIEWER: u8 = 2;
/// The head's part.
pub const HEAD: u8 = 0;
/// The last part that is the hero's own figure (head and body).
pub const LAST_BODY_PART: u8 = 3;
/// The portrait's side, in game px: the ring's inside (`hud_view` draws it 1:1 in hud px).
pub const PORTRAIT: u32 = 16;

/// One picture a frame draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The slot in `t.a:[[Lcw;`: the part, or layers + the part when mirrored.
    pub slot: u8,
    pub part: u8,
    pub picture: u8,
    pub dx: i8,
    pub dy: i8,
}

/// What `t.a(BBIII)I` draws for frame 0 of an action in direction `dir` (1-based), from the
/// action's bytes (`t.a:[B` from the action's offset on) and the doll's layer count. `None` when
/// the bytes run out first.
pub fn frame_entries(bytes: &[u8], layers: u8, dir: u8) -> Option<Vec<Entry>> {
    // The frame count, then the directions before ours.
    let mut p = 1usize;
    for _ in 1..dir {
        p += 1 + usize::from(*bytes.get(p)?) * 4;
    }
    let n = usize::from(*bytes.get(p)?);
    let body = bytes.get(p + 1..p + 1 + n * 4)?;
    Some(
        body.chunks_exact(4)
            .map(|e| Entry {
                slot: (e[0] & 0x7f).wrapping_add(if e[0] & 0x80 != 0 { layers } else { 0 }),
                part: e[0] & 0x7f,
                picture: e[1],
                dx: e[2] as i8,
                dy: e[3] as i8,
            })
            .collect(),
    )
}

/// A picture placed: top-left `(x, y)` from the feet, `w × h` ARGB as `Image.getRGB` gives it.
#[derive(Clone, Debug)]
pub struct Piece {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub px: Vec<u32>,
    /// The head part's picture: the window centres on the first one.
    pub head: bool,
}

/// The portrait from the pieces in draw order: a [`PORTRAIT`]-square window centred on the head,
/// each piece over the earlier ones where it is not transparent. `None` without a head or with
/// nothing in the window. Its key is a checksum of the pixels, so the same look is the same
/// portrait.
pub fn compose(pieces: &[Piece]) -> Option<Icon> {
    let head = pieces.iter().find(|p| p.head)?;
    let side = PORTRAIT as i32;
    let x0 = head.x + (head.w as i32 - side) / 2;
    let y0 = head.y + (head.h as i32 - side) / 2;
    let mut px = vec![0u32; (PORTRAIT * PORTRAIT) as usize];
    for p in pieces {
        if p.px.len() < (p.w * p.h) as usize {
            continue;
        }
        for sy in 0..p.h as i32 {
            for sx in 0..p.w as i32 {
                let (wx, wy) = (p.x + sx - x0, p.y + sy - y0);
                if !(0..side).contains(&wx) || !(0..side).contains(&wy) {
                    continue;
                }
                let c = p.px[(sy * p.w as i32 + sx) as usize];
                if c >> 24 != 0 {
                    px[(wy * side + wx) as usize] = c;
                }
            }
        }
    }
    if px.iter().all(|c| c >> 24 == 0) {
        return None;
    }
    // FNV-1a over the pixels.
    let sum = px.iter().fold(0x811c_9dc5u32, |h, &c| c.to_le_bytes().iter().fold(h, |h, &b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193)));
    Some(Icon { key: IconKey::Portrait(sum), w: PORTRAIT, h: PORTRAIT, px: Arc::from(px) })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An action of one frame: four directions, each `(part byte, picture, dx, dy)` entries.
    fn action(dirs: [&[(u8, u8, i8, i8)]; 4]) -> Vec<u8> {
        let mut b = vec![1u8];
        for d in dirs {
            b.push(d.len() as u8);
            for &(part, pic, dx, dy) in d {
                b.extend([part, pic, dx as u8, dy as u8]);
            }
        }
        b
    }

    #[test]
    fn the_entries_are_the_directions_own() {
        // The warrior's (`/hero/h0`, 7 layers) action 0 facing down, as the probe parsed it.
        let down: &[(u8, u8, i8, i8)] = &[(0x82, 1, -8, -13), (3, 4, -7, -7), (1, 1, -3, -12), (0, 2, -6, -25), (2, 1, -7, -12), (6, 0, -34, -8)];
        let bytes = action([&[(0, 0, -7, -25)], down, &[], &[(7, 1, 0, 0)]]);
        let e = frame_entries(&bytes, 7, FACING_VIEWER).unwrap();
        assert_eq!(e.len(), 6);
        assert_eq!(e[0], Entry { slot: 9, part: 2, picture: 1, dx: -8, dy: -13 }, "bit 7 = the mirrored slot, layers on");
        assert_eq!(e[3], Entry { slot: 0, part: HEAD, picture: 2, dx: -6, dy: -25 });
        assert_eq!(frame_entries(&bytes, 7, 1).unwrap().len(), 1, "up");
        assert_eq!(frame_entries(&bytes, 7, 3).unwrap(), vec![], "an empty direction");
        assert_eq!(frame_entries(&bytes, 7, 4).unwrap()[0].slot, 7);
        assert_eq!(frame_entries(&bytes[..bytes.len() - 1], 7, 4), None, "cut short");
        assert_eq!(frame_entries(&[], 7, 1), None);
    }

    fn piece(x: i32, y: i32, w: u32, h: u32, c: u32, head: bool) -> Piece {
        Piece { x, y, w, h, px: vec![c; (w * h) as usize], head }
    }

    #[test]
    fn the_window_centres_on_the_head_and_later_pieces_cover() {
        // Rows: the head -25..=-13, the body -14..=-2 — they share -14 and -13.
        let body = piece(-7, -14, 14, 13, 0xff00_00ff, false);
        let head = piece(-6, -25, 12, 13, 0xffff_0000, true);
        let p = compose(&[body.clone(), head.clone()]).unwrap();
        assert_eq!((p.w, p.h), (PORTRAIT, PORTRAIT));
        // Centred on the head: x -6 + (12 - 16) / 2 = -8, y -25 + (13 - 16) / 2 = -26 (the odd
        // row goes below) — the window spans x -8..8, y -26..-10.
        let at = |p: &Icon, x: i32, y: i32| p.px[((y + 26) * 16 + (x + 8)) as usize];
        assert_eq!(at(&p, 0, -19), 0xffff_0000, "the head");
        assert_eq!(at(&p, -7, -11), 0xff00_00ff, "the body under the head");
        assert_eq!(at(&p, -8, -26), 0, "nothing drawn there");
        assert_eq!(at(&p, 0, -13), 0xffff_0000, "the head drawn after the body covers it");
        let under = compose(&[head.clone(), body]).unwrap();
        assert_eq!(at(&under, 0, -13), 0xff00_00ff, "draw order is the frame's");
        assert_ne!(p, under, "two looks, two keys");
        assert_eq!(p, compose(&[piece(-7, -14, 14, 13, 0xff00_00ff, false), head]).unwrap(), "the same look, the same key");
    }

    #[test]
    fn transparent_pixels_do_not_cover_and_no_head_is_no_portrait() {
        let mut head = piece(0, 0, 4, 4, 0xff11_2233, true);
        head.px[0] = 0x00ff_ffff;
        let behind = piece(0, 0, 4, 4, 0xff44_5566, false);
        let p = compose(&[behind, head.clone()]).unwrap();
        // A 4×4 head at (0, 0): the window starts at (-6, -6).
        assert_eq!(p.px[6 * 16 + 6], 0xff44_5566, "alpha 0 shows what is behind");
        assert_eq!(p.px[6 * 16 + 7], 0xff11_2233);
        assert_eq!(compose(&[piece(0, 0, 4, 4, 0xff00_0000, false)]), None, "no head");
        assert_eq!(compose(&[piece(0, 0, 4, 4, 0, true)]), None, "nothing to see");
        assert_eq!(compose(&[Piece { px: vec![], ..head }]), None, "a short picture is skipped");
    }
}
