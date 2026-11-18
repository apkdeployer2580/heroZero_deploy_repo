// hero2's MINIMAP VIEW — the hero5-style panel (2026-09-29).
//
// ── WHAT THE PLAYER ASKED FOR ────────────────────────────────────────────────────
// *"hero5처럼 미니맵을 현재 내 위치로 살짝 확대되서 … 확대된 미니맵의 크기는 어떤 크기의 맵을
// 가든 고정이어야해 … 내가 이동해서 현위치가 바뀜에 따라 미니맵도 같이 바뀌었으면 좋겠어"* +
// (asked back, answered 「다른 맵으로 가는 출구만」) exits as blue bars.
//
// Until this day the panel FITTED the whole map: a 20×20 room and a 60×20 field were
// both squeezed into the same square, so the zoom changed with every door. Now:
//
//  * The zoom is FIXED — `MM_VIEW_TILES` tiles across on every map, whatever its shape.
//  * The view FOLLOWS the player, and stops at the map's edge (the dot then walks to the
//    panel's edge instead of the map sliding off it). That is hero5's own rule:
//    `Map::Map_Draw_Minimap` @0xaf4d0 (libHeroesLore5.so) sets its window's offset to
//    `(player_tile − view/2) × scale` clamped to `[0, image − view]`, and shows a whole
//    axis when the map is smaller than the window.
//  * Only the WALKABLE area is drawn, hero5's way: a dark translucent floor with a light
//    outline where it meets a wall. Walls are not drawn at all — the world shows through,
//    untouched (the faint backdrop the panel had for the snowfields went with the panel,
//    below; the floor's own darkening is what keeps the outline readable on snow).
//  * A way to ANOTHER map is a blue BAR across its doorway — the half of the exit tile
//    that faces the floor it opens onto, so a one-tile door is a short bar and a road end
//    three tiles wide a long one (`hero2java::h2_trigger_groups` finds them).
//  * A treasure chest that can still be opened is a GREEN BOX inset in its tile (same day,
//    *"보물상자 같이 열수 있는항목은 초록색으로"*); it goes away once the game's event flag
//    says it has been opened.
//  * ★ NO PANEL, and the MONSTERS on it (same day, the status-HUD round — player: the
//    minimap *"레퍼런스처럼 + 몬스터 점"*, the reference being hero5's own HUD). hero5 draws
//    its minimap straight onto the world — no backdrop, no frame — with the monsters as red
//    squares and the player as a teal one (`hl5-08-gameplay-hud.png`: `#ef1821`, `#317173`,
//    three minimap pixels each). So the panel is now just the view, the player's dot is a
//    teal square (red means "monster" now), and every live monster on the stage is a red
//    square (`hero2java::h2_monster_marks`, `UserEvent::MinimapMobs`).
//
// ── WHY A FILE OF ITS OWN ─────────────────────────────────────────────────────────
// Dependency-free, so the same code draws on the phone (`overlay.rs` `include!`s it) and
// in `wie_probe --h2-minimap` (which composites it over a dumped frame — the only way to
// SEE the panel without a device), and so `minimap_view_selftest.rs` can test it with
// plain `rustc`. The tile classes are repeated from `hero2java.rs` (`H2_MM_*`) because
// neither file may name the other; `overlay.rs` asserts at compile time that they agree.

/// Tiles the view spans across, on EVERY map (the panel is square, so down as well).
///
/// "살짝 확대": the S9 town (25×25) used to show all 25 tiles, a 30×30 hunting ground 30
/// and a 60×20 field 60 — now each shows 20 (19 to 21 on a panel whose pixel size does
/// not divide evenly, see [`mm_tile_px`]).
pub const MM_VIEW_TILES: f64 = 20.0;

/// Nothing to draw: a wall, a blocked NPC counter, the void past the map's edge.
pub const MM_WALL: u8 = 0;
/// Walkable floor.
pub const MM_FLOOR: u8 = 1;
/// A trigger that takes the player to another map — the blue bar.
pub const MM_EXIT: u8 = 2;
/// A treasure chest that can still be opened — the green box (2026-09-29, player:
/// *"보물상자 같이 열수 있는항목은 초록색으로"*). It is an object on a blocked tile, so the
/// floor next to it keeps its outline, and the box sits inside the tile.
pub const MM_CHEST: u8 = 3;

/// The floor: black at `MM_FILL_A` — hero5 darkens its walkable area over the world
/// (its minimap blits a greyed image at blend level 9). It is also what keeps the light
/// outline readable on hero2's snowfields now that the panel has no backdrop.
pub const MM_FILL: u32 = 0x0000_0000;
pub const MM_FILL_A: u32 = 160;
/// The OLD panel's backdrop and gold frame colours. Since the status-HUD round
/// (2026-09-29) the map panel draws neither — hero5 has none — and these only dress the
/// "no map yet" panel and hero3's picture panel in `overlay.rs` (at its own alphas).
pub const MM_BACKDROP: u32 = 0x000c_1018;
pub const MM_FRAME: u32 = 0x00c8_ae72;
/// The outline where floor meets wall (hero5: light grey, one minimap pixel).
///
/// ★ 2026-10-01 (player: *"미니맵의 지형 윤곽선을 회색으로 변경해주고 투명도를 더 올려줘"*):
/// GREY, and see-through — was `#e6e6e6` at 240 (6 % of the world through it), now
/// `#a0a0a0` at 160 (37 %). Still well apart from the floor it rims: on dark dungeon rock
/// it lands near grey 110 against the floor's ~15, on snow near 185 against ~85.
pub const MM_LINE: u32 = 0x00a0_a0a0;
pub const MM_LINE_A: u32 = 160;
/// The exit bar.
pub const MM_EXIT_RGB: u32 = 0x0028_78ff;
pub const MM_EXIT_A: u32 = 240;
/// The chest box — hero5's own minimap green (`Map::Map_Draw_Minimap` fills its event
/// markers with `0x00e248`), with a dark rim so it reads on snow as well as on stone.
pub const MM_CHEST_RGB: u32 = 0x0000_e248;
pub const MM_CHEST_RIM: u32 = 0x0004_3a14;
pub const MM_CHEST_A: u32 = 245;
/// The player — hero5's teal, brightened so it holds up on the dark floor (hero5's own
/// `#317173` is drawn over unlit dungeon rock). Red is the monsters' now.
pub const MM_DOT: u32 = 0x003c_c8c8;
/// A live monster — hero5's red.
pub const MM_MOB: u32 = 0x00ef_1821;
/// The one-pixel rim around both kinds of square, so they read on snow as well as on stone.
pub const MM_MARK_RIM: u32 = 0x0008_1010;

/// Screen pixels per tile for a panel whose drawable side is `side` px: the whole
/// number nearest to `side / MM_VIEW_TILES`, never below 1.
///
/// ★ WHOLE pixels on purpose. At a fractional size (252 / 20 = 12.6) the tiles come out
/// 12 or 13 px wide depending on where they fall, and since the view scrolls with the
/// player that pattern MOVES — every step would make the outlines shimmer between two
/// and three pixels. Whole-pixel tiles scrolled by whole pixels look the same wherever
/// they are; the price is that the view spans 19.4 tiles instead of 20.
pub fn mm_tile_px(side: u32) -> u32 {
    ((f64::from(side) / MM_VIEW_TILES).round() as u32).max(1)
}

/// Where the view starts along one axis, in TILES: `centre` in the middle, clamped so
/// the view stays on the map when the map is longer than the view; the map centred in
/// the view when it is shorter (a small room keeps the zoom and sits in the middle).
pub fn mm_view_origin(map_len: u32, centre: f64, view: f64) -> f64 {
    let m = f64::from(map_len);
    if m <= view {
        (m - view) / 2.0
    } else {
        (centre - view / 2.0).clamp(0.0, m - view)
    }
}

/// `buf[i] = color OVER buf[i]` at alpha `a` — the same arithmetic as `overlay::blend_px`,
/// repeated so this file needs nothing from its includer.
fn mm_blend(buf: &mut [u32], i: usize, color: u32, a: u32) {
    let Some(dst) = buf.get(i).copied() else { return };
    let inv = 255 - a;
    let mix = |sh: u32| {
        let s = (color >> sh) & 0xff;
        let d = (dst >> sh) & 0xff;
        ((s * a + d * inv) / 255) & 0xff
    };
    let da = (dst >> 24) & 0xff;
    let na = ((a * 255 + da * inv) / 255) & 0xff;
    buf[i] = (na << 24) | (mix(16) << 16) | (mix(8) << 8) | mix(0);
}

/// Is this a map `mm_draw_tiles` can draw?
pub fn mm_is_map(map_w: u32, map_h: u32, cells: &[u8]) -> bool {
    map_w > 0 && map_h > 0 && cells.len() == (map_w as usize) * (map_h as usize)
}

/// The whole PANEL `p = (x, y, w, h)` exactly as the app shows it; `wie_probe
/// --h2-minimap` calls this same function. Since 2026-09-29 the panel IS the view — no
/// backdrop, no frame, hero5's way (see the file header). Draws nothing and returns `false`
/// when the input is not a map (the caller shows its "no map yet" panel instead).
#[allow(clippy::too_many_arguments)]
pub fn mm_draw_panel(
    buf: &mut [u32],
    win_w: u32,
    win_h: u32,
    p: (u32, u32, u32, u32),
    map_w: u32,
    map_h: u32,
    cells: &[u8],
    marker: Option<(f32, f32)>,
    mobs: &[(f32, f32)],
) -> bool {
    let (_, _, w, h) = p;
    if w == 0 || h == 0 || !mm_is_map(map_w, map_h, cells) {
        return false;
    }
    mm_draw_tiles(buf, win_w, win_h, p, map_w, map_h, cells, marker, mobs)
}

/// A square mark `side` px wide centred on `(cx, cy)` (screen px, fractional): a one-pixel
/// `MM_MARK_RIM` border around `color`, clipped to the view `(x0, y0)..(x1, y1)`.
#[allow(clippy::too_many_arguments)]
fn mm_mark(buf: &mut [u32], win_w: u32, cx: f64, cy: f64, side: u32, color: u32, clip: (u32, u32, u32, u32)) {
    let (x0, y0, x1, y1) = clip;
    let left = (cx - f64::from(side) / 2.0).round() as i64;
    let top = (cy - f64::from(side) / 2.0).round() as i64;
    let s = i64::from(side);
    for yy in top.max(i64::from(y0))..(top + s).min(i64::from(y1)) {
        for xx in left.max(i64::from(x0))..(left + s).min(i64::from(x1)) {
            let rim = xx == left || yy == top || xx == left + s - 1 || yy == top + s - 1;
            mm_blend(buf, (yy as usize) * (win_w as usize) + xx as usize, if rim { MM_MARK_RIM } else { color }, 255);
        }
    }
}

/// Draw the view into `buf` (`win_w × win_h`, one `u32` per pixel), inside the drawable
/// rect `area` = `(x, y, w, h)` and never outside it.
///
/// `cells` = `map_w × map_h` tile classes, row-major (`MM_*`). `marker` = the player in
/// TILES, fractional, at the centre of the character's tile (`h2_player_marker`); the
/// view follows it, and without one it holds the middle of the map and draws no dot.
/// `mobs` = the live monsters, in the same units (`h2_monster_marks`), drawn under the
/// player's square. Returns `false` (and draws nothing) when the input is not a map.
#[allow(clippy::too_many_arguments)]
pub fn mm_draw_tiles(
    buf: &mut [u32],
    win_w: u32,
    win_h: u32,
    area: (u32, u32, u32, u32),
    map_w: u32,
    map_h: u32,
    cells: &[u8],
    marker: Option<(f32, f32)>,
    mobs: &[(f32, f32)],
) -> bool {
    let (ax, ay, aw, ah) = area;
    if !mm_is_map(map_w, map_h, cells) || aw == 0 || ah == 0 {
        return false;
    }
    let sc = mm_tile_px(aw.min(ah));
    let scf = f64::from(sc);
    let centre = match marker {
        Some((mx, my)) if mx.is_finite() && my.is_finite() => (f64::from(mx), f64::from(my)),
        _ => (f64::from(map_w) / 2.0, f64::from(map_h) / 2.0),
    };
    // The view's top-left in WORLD pixels (tile × sc), snapped to a whole pixel so the
    // scroll moves every tile by the same amount (see `mm_tile_px`).
    let ox = (mm_view_origin(map_w, centre.0, f64::from(aw) / scf) * scf).round() as i64;
    let oy = (mm_view_origin(map_h, centre.1, f64::from(ah) / scf) * scf).round() as i64;
    let sc_i = i64::from(sc);
    let lw = i64::from((sc + 2) / 4).max(1);
    // The exit bar's thickness: about half a tile, so a one-tile door still reads as a bar.
    let bt = i64::from((sc + 1) / 2).max(3).min(sc_i);
    // The chest box's inset from its tile's edges: ~1/8 of a tile (1 px at 13 — the probe
    // render at 2 px read as a speck next to the player's dot), none on a tile too small
    // to spare it.
    let ci = if sc >= 7 { i64::from(sc / 8).max(1) } else { 0 };

    let cell = |tx: i64, ty: i64| -> u8 {
        if tx < 0 || ty < 0 || tx >= i64::from(map_w) || ty >= i64::from(map_h) {
            return MM_WALL;
        }
        cells[(ty as usize) * (map_w as usize) + tx as usize]
    };
    // Visible pixel span, clipped to the window as well as to the area.
    let x_end = (ax + aw).min(win_w);
    let y_end = (ay + ah).min(win_h);
    if ax >= x_end || ay >= y_end {
        return true;
    }
    // Per-column tile and in-tile offset, once per frame instead of once per pixel.
    let cols: Vec<(i64, i64)> = (ax..x_end)
        .map(|x| {
            let w = ox + i64::from(x - ax);
            (w.div_euclid(sc_i), w.rem_euclid(sc_i))
        })
        .collect();
    for y in ay..y_end {
        let wy = oy + i64::from(y - ay);
        let (ty, fy) = (wy.div_euclid(sc_i), wy.rem_euclid(sc_i));
        let (top, bottom) = (fy < lw, fy >= sc_i - lw);
        let row = (y as usize) * (win_w as usize);
        for (i, &(tx, fx)) in cols.iter().enumerate() {
            let idx = row + ax as usize + i;
            match cell(tx, ty) {
                MM_FLOOR => {
                    let (left, right) = (fx < lw, fx >= sc_i - lw);
                    // An outline pixel sits on the floor side of a floor/wall border — the
                    // four sides, plus the corner square where two floor sides meet a wall
                    // diagonally (without it every inside corner has a notch). A chest is an
                    // object like a wall; an exit is an opening and gets no line.
                    let solid = |dx: i64, dy: i64| matches!(cell(tx + dx, ty + dy), MM_WALL | MM_CHEST);
                    let edge = (left && solid(-1, 0))
                        || (right && solid(1, 0))
                        || (top && solid(0, -1))
                        || (bottom && solid(0, 1))
                        || (left && top && solid(-1, -1))
                        || (right && top && solid(1, -1))
                        || (left && bottom && solid(-1, 1))
                        || (right && bottom && solid(1, 1));
                    if edge {
                        mm_blend(buf, idx, MM_LINE, MM_LINE_A);
                    } else {
                        mm_blend(buf, idx, MM_FILL, MM_FILL_A);
                    }
                }
                MM_EXIT => {
                    // A BAR across the doorway: the half of the exit tile that faces the
                    // one floor tile it opens onto — a door in a wall is a short bar, a road
                    // end three tiles wide a long one (hero5 draws its gates as bars too).
                    // A tile you step ON (floor on several sides) or a lone corner stays whole.
                    let floor = |dx: i64, dy: i64| cell(tx + dx, ty + dy) == MM_FLOOR;
                    let bar = match (floor(0, -1), floor(0, 1), floor(-1, 0), floor(1, 0)) {
                        (true, false, false, false) => fy < bt,
                        (false, true, false, false) => fy >= sc_i - bt,
                        (false, false, true, false) => fx < bt,
                        (false, false, false, true) => fx >= sc_i - bt,
                        _ => true,
                    };
                    if bar {
                        mm_blend(buf, idx, MM_EXIT_RGB, MM_EXIT_A);
                    }
                }
                MM_CHEST => {
                    // A box inset in its tile — a marker on the object, not the whole
                    // square, so it never reads as a doorway (the exits' job).
                    let inside = |f: i64| (ci..sc_i - ci).contains(&f);
                    if inside(fx) && inside(fy) {
                        let rim = fx == ci || fx == sc_i - ci - 1 || fy == ci || fy == sc_i - ci - 1;
                        mm_blend(buf, idx, if rim { MM_CHEST_RIM } else { MM_CHEST_RGB }, MM_CHEST_A);
                    }
                }
                _ => {}
            }
        }
    }

    // The marks, at panel resolution from their fractional positions — so they glide between
    // tiles instead of hopping (the reason the marker is its own event). Squares, hero5's
    // way: the monsters a little smaller than the player, and the player drawn last.
    let clip = (ax, ay, x_end, y_end);
    let at = |(mx, my): (f32, f32)| (f64::from(ax) + f64::from(mx) * scf - ox as f64, f64::from(ay) + f64::from(my) * scf - oy as f64);
    let mob_side = ((scf * 0.6).round() as u32).max(3);
    for &m in mobs.iter().filter(|(x, y)| x.is_finite() && y.is_finite()) {
        let (cx, cy) = at(m);
        mm_mark(buf, win_w, cx, cy, mob_side, MM_MOB, clip);
    }
    if let Some(m) = marker.filter(|(x, y)| x.is_finite() && y.is_finite()) {
        let (cx, cy) = at(m);
        mm_mark(buf, win_w, cx, cy, ((scf * 0.75).round() as u32).max(4), MM_DOT, clip);
    }
    true
}

#[cfg(test)]
mod minimap_view_tests {
    use super::*;

    /// A WHITE window (so "untouched", "darkened" and "outlined" are three different
    /// values) with the view drawn into a `side`-px area at the origin.
    fn draw(side: u32, map_w: u32, map_h: u32, cells: &[u8], marker: Option<(f32, f32)>) -> Vec<u32> {
        draw_with(side, map_w, map_h, cells, marker, &[])
    }

    fn draw_with(side: u32, map_w: u32, map_h: u32, cells: &[u8], marker: Option<(f32, f32)>, mobs: &[(f32, f32)]) -> Vec<u32> {
        let mut buf = vec![0xffff_ffffu32; (side * side) as usize];
        assert!(mm_draw_tiles(&mut buf, side, side, (0, 0, side, side), map_w, map_h, cells, marker, mobs));
        buf
    }

    /// The screen pixel of in-tile offset `(fx, fy)` of tile `(tx, ty)`, with the view
    /// placed the way `mm_draw_tiles` places it for this panel and centre.
    fn pixel(buf: &[u32], side: u32, map: (u32, u32), centre: (f64, f64), t: (i64, i64), f: (i64, i64)) -> u32 {
        let sc = mm_tile_px(side);
        let view = f64::from(side) / f64::from(sc);
        let ox = (mm_view_origin(map.0, centre.0, view) * f64::from(sc)).round() as i64;
        let oy = (mm_view_origin(map.1, centre.1, view) * f64::from(sc)).round() as i64;
        let (x, y) = (t.0 * i64::from(sc) + f.0 - ox, t.1 * i64::from(sc) + f.1 - oy);
        buf[(y as usize) * side as usize + x as usize]
    }

    fn red(p: u32) -> u32 {
        (p >> 16) & 0xff
    }

    /// The outline as it lands on the WHITE window these tests draw into — computed with
    /// the drawing's own blend, so the tests check WHERE the outline is drawn, not a
    /// brightness that moves whenever its colour is retuned.
    fn outline_on_white() -> u32 {
        let mut px = [0xffff_ffffu32];
        mm_blend(&mut px, 0, MM_LINE, MM_LINE_A);
        px[0]
    }

    #[test]
    fn the_outline_is_a_see_through_grey() {
        // ★ 2026-10-01, player: "미니맵의 지형 윤곽선을 회색으로 변경해주고 투명도를 더 올려줘".
        let (r, g, b) = ((MM_LINE >> 16) & 0xff, (MM_LINE >> 8) & 0xff, MM_LINE & 0xff);
        assert!(r == g && g == b, "a neutral grey: {MM_LINE:#08x}");
        assert!((0x80..=0xb0).contains(&r), "grey, not the old near-white: {r:#04x}");
        assert!(MM_LINE_A <= 170, "see-through (the old line was 240): {MM_LINE_A}");
        // …and still an outline: on dark rock and on snow alike it stays clearly apart
        // from the darkened floor it borders.
        for ground in [0xff28_2028u32, 0xffe6_e6e6] {
            let (mut line, mut floor) = ([ground], [ground]);
            mm_blend(&mut line, 0, MM_LINE, MM_LINE_A);
            mm_blend(&mut floor, 0, MM_FILL, MM_FILL_A);
            assert!(red(line[0]) >= red(floor[0]) + 0x50, "on {ground:#010x}: line {:#010x} vs floor {:#010x}", line[0], floor[0]);
        }
    }

    #[test]
    fn the_zoom_is_the_same_on_every_map_shape() {
        // ★ The player's rule: long, tall or big, the panel shows the same number of
        // tiles. The tile size depends on the PANEL only — the map is not an input.
        for side in [120u32, 252, 280] {
            let px = mm_tile_px(side);
            assert!((19.0..=21.5).contains(&(f64::from(side) / f64::from(px))), "side {side}: {px} px/tile");
        }
        assert_eq!(mm_tile_px(252), 13);
        assert_eq!(mm_tile_px(3), 1, "never zero, even on a degenerate panel");
    }

    #[test]
    fn the_view_follows_the_player_and_stops_at_the_edge() {
        // A 60-tile axis and a 20-tile view: centred on the player in the middle…
        assert_eq!(mm_view_origin(60, 30.5, 20.0), 20.5);
        // …and pinned to the map at both ends (hero5 clamps the same way).
        assert_eq!(mm_view_origin(60, 3.5, 20.0), 0.0);
        assert_eq!(mm_view_origin(60, 58.5, 20.0), 40.0);
        // A 10-tile room keeps the zoom and sits in the middle of the view.
        assert_eq!(mm_view_origin(10, 3.5, 20.0), -5.0);
        assert_eq!(mm_view_origin(10, 9.5, 20.0), -5.0, "a small axis does not scroll at all");
        // Exactly as long as the view: shown whole, no scrolling.
        assert_eq!(mm_view_origin(20, 17.5, 20.0), 0.0);
    }

    #[test]
    fn walls_are_not_drawn_and_floor_is_darkened() {
        // 3×3 with one floor tile in the middle: a wall pixel keeps the world (white), the
        // floor's middle is the dark fill, the floor's rim is the light outline.
        let side = 280u32;
        let mut cells = [MM_WALL; 9];
        cells[4] = MM_FLOOR;
        let buf = draw(side, 3, 3, &cells, None);
        let at = |t, f| pixel(&buf, side, (3, 3), (1.5, 1.5), t, f);
        assert_eq!(at((0, 0), (7, 7)), 0xffff_ffff, "a wall pixel is untouched");
        let mid = at((1, 1), (7, 7));
        assert!((0x40..0x80).contains(&red(mid)), "the floor's middle is darkened: {mid:#010x}");
        let rim = at((1, 1), (0, 7));
        assert_eq!(rim, outline_on_white(), "the floor's rim next to a wall is outlined: {rim:#010x}");
    }

    #[test]
    fn an_exit_is_blue_and_the_floor_meets_it_without_an_outline() {
        // 1 row: floor · floor · exit, 14 px tiles on a 280 px area.
        let side = 280u32;
        let cells = [MM_FLOOR, MM_FLOOR, MM_EXIT];
        let buf = draw(side, 3, 1, &cells, None);
        assert_eq!(mm_tile_px(side), 14);
        let at = |t, f| pixel(&buf, side, (3, 1), (1.5, 0.5), t, f);
        // The exit opens onto the floor to its LEFT: a vertical bar on that half.
        let exit = at((2, 0), (3, 7));
        assert!(exit & 0xff > 0xe0 && red(exit) < 0x40, "the exit bar is blue: {exit:#010x}");
        assert_eq!(at((2, 0), (12, 7)), 0xffff_ffff, "the far half of the exit tile is not drawn");
        // The floor's top edge touches the void above → outline.
        let edge = at((0, 0), (7, 0));
        assert_eq!(edge, outline_on_white(), "outline on the floor side of the border: {edge:#010x}");
        // Floor next to the exit is NOT outlined on that side — the bar sits flush.
        let flush = at((1, 0), (13, 7));
        assert!((0x40..0x80).contains(&red(flush)), "plain fill between floor and exit: {flush:#010x}");
    }

    #[test]
    fn a_door_is_a_bar_on_the_side_it_opens_onto_and_a_step_on_exit_is_whole() {
        // 3×3: a door above the middle column's floor, and a walk-on exit at the
        // bottom-right with floor on two sides.
        //   row 0: wall · EXIT  · wall
        //   row 1: wall · floor · floor
        //   row 2: wall · floor · EXIT
        let side = 280u32;
        let cells = [MM_WALL, MM_EXIT, MM_WALL, MM_WALL, MM_FLOOR, MM_FLOOR, MM_WALL, MM_FLOOR, MM_EXIT];
        let buf = draw(side, 3, 3, &cells, None);
        let at = |t, f| pixel(&buf, side, (3, 3), (1.5, 1.5), t, f);
        let blue = |p: u32| p & 0xff > 0xe0 && red(p) < 0x40;
        // The door opens DOWN onto (1,1): the bar is the tile's bottom half.
        assert!(blue(at((1, 0), (7, 13))), "bottom edge of the door tile");
        assert_eq!(at((1, 0), (7, 0)), 0xffff_ffff, "top edge of the door tile: not drawn");
        // (2,2) has floor on its left (1,2) and above (2,1): two sides → the whole tile.
        assert!(blue(at((2, 2), (13, 13))) && blue(at((2, 2), (0, 0))), "a step-on exit is whole");
    }

    #[test]
    fn a_chest_is_a_green_box_inside_its_tile_and_the_floor_keeps_its_outline() {
        // 1 row: floor · CHEST.
        let side = 280u32;
        let cells = [MM_FLOOR, MM_CHEST];
        let buf = draw(side, 2, 1, &cells, None);
        let at = |t, f| pixel(&buf, side, (2, 1), (1.0, 0.5), t, f);
        let green = |p: u32| (p >> 8) & 0xff > 0xc0 && red(p) < 0x40 && p & 0xff < 0x80;
        assert!(green(at((1, 0), (7, 7))), "the box's middle is green: {:#010x}", at((1, 0), (7, 7)));
        assert_eq!(at((1, 0), (0, 7)), 0xffff_ffff, "the inset margin is not drawn");
        let rim = at((1, 0), (1, 7));
        assert!((rim >> 8) & 0xff < 0x80, "the box has a dark rim: {rim:#010x}");
        // The floor meets the chest like a wall: outlined on that side.
        assert_eq!(at((0, 0), (13, 7)), outline_on_white(), "outline between floor and chest");
    }

    #[test]
    fn the_dot_sits_on_the_players_tile() {
        let side = 280u32;
        let cells = vec![MM_FLOOR; 40 * 40];
        // Player at the centre of tile (30, 12) of a 40×40 map: the view is clamped on
        // neither axis there, and the dot lands in the middle of that tile.
        let buf = draw(side, 40, 40, &cells, Some((30.5, 12.5)));
        let p = pixel(&buf, side, (40, 40), (30.5, 12.5), (30, 12), (7, 7));
        assert_eq!(p & 0x00ff_ffff, MM_DOT, "the dot's centre pixel: {p:#010x}");
        // A SQUARE (hero5's), with a dark rim: the tile's corner-ish pixel is inside it too.
        let corner = pixel(&buf, side, (40, 40), (30.5, 12.5), (30, 12), (3, 3));
        assert_eq!(corner & 0x00ff_ffff, MM_DOT, "a square, not a disc: {corner:#010x}");
        // …and a tile away it is not.
        let q = pixel(&buf, side, (40, 40), (30.5, 12.5), (32, 12), (7, 7));
        assert_ne!(q & 0x00ff_ffff, MM_DOT);
    }

    #[test]
    fn a_monster_is_a_red_square_on_its_tile_and_the_player_is_on_top() {
        let side = 280u32;
        let cells = vec![MM_FLOOR; 40 * 40];
        // A monster two tiles right of the player, and one ON the player's tile.
        let buf = draw_with(side, 40, 40, &cells, Some((30.5, 12.5)), &[(32.5, 12.5), (30.5, 12.5)]);
        let at = |t, f| pixel(&buf, side, (40, 40), (30.5, 12.5), t, f);
        assert_eq!(at((32, 12), (7, 7)) & 0x00ff_ffff, MM_MOB, "the monster's tile");
        assert_eq!(at((30, 12), (7, 7)) & 0x00ff_ffff, MM_DOT, "the player is drawn over a monster");
        // The monster's square is smaller than a tile: its tile's edge is floor.
        assert_ne!(at((32, 12), (0, 7)) & 0x00ff_ffff, MM_MOB);
        // A monster outside the view is not drawn anywhere (and does not panic).
        let far = draw_with(side, 40, 40, &cells, Some((30.5, 12.5)), &[(1.5, 38.5), (f32::NAN, 3.0)]);
        assert!(!far.iter().any(|p| p & 0x00ff_ffff == MM_MOB));
    }

    #[test]
    fn a_non_map_draws_nothing() {
        let mut buf = vec![0u32; 100];
        assert!(!mm_draw_tiles(&mut buf, 10, 10, (0, 0, 10, 10), 3, 3, &[MM_FLOOR; 8], None, &[]), "wrong cell count");
        assert!(!mm_draw_tiles(&mut buf, 10, 10, (0, 0, 10, 10), 0, 3, &[], None, &[]));
        // The panel too — the caller draws its "no map" panel.
        assert!(!mm_draw_panel(&mut buf, 10, 10, (0, 0, 10, 10), 3, 3, &[MM_FLOOR; 8], None, &[]));
        assert!(buf.iter().all(|p| *p == 0));
    }

    #[test]
    fn the_panel_is_just_the_view() {
        // ★ 2026-09-29: no backdrop and no frame (hero5 has neither). 280 px panel at
        // (10, 10) of a white 300×300 window, one wall tile as the map: nothing at all is
        // drawn — the world shows through wherever the map has no floor.
        let (w, side) = (300u32, 280u32);
        let mut buf = vec![0xffff_ffffu32; (w * w) as usize];
        assert!(mm_draw_panel(&mut buf, w, w, (10, 10, side, side), 1, 1, &[MM_WALL], None, &[]));
        assert!(buf.iter().all(|p| *p == 0xffff_ffff), "a wall-only map leaves the world untouched");
        // With floor, the view spans the whole panel: its edge pixel is drawn.
        let cells = vec![MM_FLOOR; 60 * 60];
        assert!(mm_draw_panel(&mut buf, w, w, (10, 10, side, side), 60, 60, &cells, Some((30.5, 30.5)), &[]));
        assert_ne!(buf[(10 * w + 10) as usize], 0xffff_ffff, "the panel's corner is map, not frame");
        assert_eq!(buf[(5 * w + 5) as usize], 0xffff_ffff, "outside the panel: untouched");
    }

    #[test]
    fn the_view_never_writes_outside_its_area_or_the_window() {
        // Area hanging off the bottom-right corner of a small window.
        let (w, h) = (50u32, 40u32);
        let mut buf = vec![0xffff_ffffu32; (w * h) as usize];
        let cells = vec![MM_FLOOR; 100 * 100];
        assert!(mm_draw_tiles(&mut buf, w, h, (30, 20, 60, 60), 100, 100, &cells, Some((50.0, 50.0)), &[(49.0, 49.0), (52.0, 51.0), (47.0, 50.0)]));
        for y in 0..h {
            for x in 0..w {
                let inside = x >= 30 && y >= 20;
                if !inside {
                    assert_eq!(buf[(y * w + x) as usize], 0xffff_ffff, "({x},{y}) is outside the area");
                }
            }
        }
    }
}
