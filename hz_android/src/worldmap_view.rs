//! The WORLD MAP the MAP tab opens (Phase 7, user 2026-10-23: *"map은 hero2처럼 월드맵 아예 신규로
//! 리뉴얼해서 만들꺼야"*) — hero2's `worldmap_view.rs`, which is hero5's world map (StateMap::
//! DrawWorldMap @0x126cec in libHeroesLore5.so): the painted continent behind a camera window, a
//! black title band, a dark vignette along the picture's edges, a small stud on every place with
//! its name in a half-transparent box under it, the roads as straight lines under the studs, and a
//! pin over the place the cursor is on. Touching a place, or an arrow, moves the cursor there along
//! the roads and the camera GLIDES after it (half the remaining distance a frame). A popup over the
//! dimmed field; the game holds still while it is open. Nothing here is written to a save.
//!
//! What is heroZero's: the places, the roads and which map is which place — from the game's own
//! maps (`tools_re/hz_maps.py --exits`: the step triggers' warps, action 23, collapsed to places;
//! HEROZERO_REMAKE_PLAN.md §5.6), the layout drafted from which edge each exit leaves by; the
//! harbour's ships (게힌시티 항구 → 메렌 · 퓌렌 · 휼렌, a choice with no walking way back) as dashed
//! sea lanes the arrows do not follow (user 2026-10-23); pixels as softbuffer's `0x00RRGGBB`.

/// One named place on the picture. `x` and `y` are thousandths of the picture (0 = left/top, 1000
/// = right/bottom): the camera centres here, the stud is drawn here, the name sits under it.
/// `maps` are the game's maps that are this place (the town's houses with the town, a dungeon's
/// floors with the dungeon); the first one's header carries the name the game itself uses.
#[derive(Clone, Copy, Debug)]
pub struct WmPlace {
    pub name: &'static str,
    pub x: u16,
    pub y: u16,
    pub maps: &'static [u8],
}

pub const WM_SILVA: usize = 0;
pub const WM_SILVA_E: usize = 1;
pub const WM_SILVA_S: usize = 2;
pub const WM_SILVA_W: usize = 3;
pub const WM_CAVE: usize = 4;
pub const WM_HYULEN: usize = 5;
pub const WM_LAB: usize = 6;
pub const WM_SOVEREIGN: usize = 7;
pub const WM_MEREN: usize = 8;
pub const WM_GATE: usize = 9;
pub const WM_MARSH: usize = 10;
pub const WM_SOUTH_ROAD: usize = 11;
pub const WM_HERMIT: usize = 12;
pub const WM_TUNNEL: usize = 13;
pub const WM_FORGOTTEN: usize = 14;
pub const WM_TEMPLE: usize = 15;
pub const WM_GEHIN: usize = 16;
pub const WM_ARENA: usize = 17;
pub const WM_GOLDEN: usize = 18;
pub const WM_HARDSHIP: usize = 19;
pub const WM_PUREN: usize = 20;
pub const WM_BONES: usize = 21;
pub const WM_RUINED_LAB: usize = 22;
pub const WM_RUINS: usize = 23;

/// The places, laid out from the exits' edges (`--exits` gives the tile each warp leaves from:
/// 실바나르's south edge → 실바나르 남부, whose west / east / south-west edges → 서부 / 휼렌 /
/// 소버린구릉지, …) with no two roads crossing, around what the maps look like (probe, every
/// place's map): the snowy north-west (실바나르 and its fields), 휼렌 and 뼈의평원 red rock, the
/// forests, the white towns and harbours on an INNER SEA the 게힌시티 ships cross, 해저터널 running
/// under it from its south shore to 잊혀진땅 — a snowy land across the water in the north.
/// Boss rooms 121~134 are the temple's (117/118/119 step into 132/133/134).
/// Re-fitted to the exits' edges on 2026-10-29 (every between-place warp's tile against its map's
/// size, both ways): 동굴 is reached from 실바나르 서부's WEST edge and its own floor leaves SOUTH,
/// so it sits north-west of 서부; 지하연구소 from 휼렌's EAST side (tile (26,9) of 30 × 30);
/// 고난의길 from 퓌렌's WEST edge and 퓌렌 from 고난의길's NORTH edge, so south-west of 퓌렌;
/// 은거지 from 남부길's SOUTH edges (73 (14,29) · 76 (9,19)), straight below it; 습지대 from the
/// gate's south event map, east-south-east; 실바나르 남부 from 실바나르's bottom-left (4,33).
/// A dungeon floor's own exit is always on its north edge (the door the player came in by), so
/// only the outdoor side's edge places a dungeon.
/// `tools_re/hz_worldmap_draw.py` paints the picture from this table (it reads this file).
pub const WM_PLACES: &[WmPlace] = &[
    WmPlace { name: "실바나르", x: 150, y: 190, maps: &[0, 7, 8, 9, 10, 11, 12, 13, 14, 15, 136] },
    WmPlace { name: "실바나르 동부", x: 310, y: 200, maps: &[46] },
    WmPlace { name: "실바나르 남부", x: 155, y: 355, maps: &[44, 45, 135] },
    WmPlace { name: "실바나르 서부", x: 90, y: 435, maps: &[49] },
    WmPlace { name: "동굴", x: 40, y: 320, maps: &[67, 68, 69, 70, 71, 72] },
    WmPlace { name: "휼렌", x: 320, y: 385, maps: &[1] },
    WmPlace { name: "지하연구소", x: 405, y: 330, maps: &[82, 83, 84, 85, 86, 87] },
    WmPlace { name: "소버린구릉지", x: 130, y: 560, maps: &[56, 57] },
    WmPlace { name: "메렌", x: 280, y: 640, maps: &[3, 16, 17, 18, 19, 20, 21] },
    WmPlace { name: "남부관문", x: 290, y: 745, maps: &[58, 94, 95, 96, 97, 98, 99, 100, 140] },
    WmPlace { name: "습지대", x: 440, y: 800, maps: &[50, 51, 52] },
    WmPlace { name: "남부길", x: 445, y: 880, maps: &[73, 74, 75, 76] },
    WmPlace { name: "은거지", x: 430, y: 968, maps: &[2, 43] },
    WmPlace { name: "해저터널", x: 515, y: 680, maps: &[77, 78, 79] },
    WmPlace { name: "잊혀진땅", x: 560, y: 320, maps: &[47, 48, 66] },
    WmPlace {
        name: "신전",
        x: 575,
        y: 160,
        maps: &[115, 116, 117, 118, 119, 120, 137, 121, 122, 123, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 134],
    },
    WmPlace {
        name: "게힌시티",
        x: 610,
        y: 790,
        maps: &[4, 5, 53, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33],
    },
    WmPlace { name: "투기장", x: 665, y: 905, maps: &[101, 102, 103, 104, 105, 106, 107, 108, 109, 110] },
    WmPlace { name: "황금숲", x: 745, y: 790, maps: &[59, 60, 61] },
    WmPlace { name: "고난의길", x: 725, y: 615, maps: &[54, 55, 139] },
    WmPlace { name: "퓌렌", x: 815, y: 475, maps: &[6, 34, 35, 36, 37, 38, 39, 40, 41, 42] },
    WmPlace { name: "뼈의평원", x: 880, y: 700, maps: &[62, 63, 64, 65] },
    WmPlace { name: "폐허연구소", x: 885, y: 545, maps: &[88, 89, 90, 91, 92, 93] },
    WmPlace { name: "지하유적지", x: 965, y: 650, maps: &[111, 112, 113, 114] },
];

/// The inner sea the ships cross (thousandths: centre and radii) — the picture's water and where
/// the lanes bend.
pub const WM_INNER_SEA: (u16, u16, u16, u16) = (570, 515, 175, 125);

/// The roads: every pair of places the character can WALK between — the maps' step-trigger warps
/// collapsed to places, each pair found both ways (`tools_re/hz_maps.py --exits`; the one-way
/// passage from the orphanage's cellar into 지하연구소 is not a road). The lines and the arrows
/// come from this table alone.
pub const WM_ROADS: &[(usize, usize)] = &[
    (WM_SILVA, WM_SILVA_S),
    (WM_SILVA_S, WM_SILVA_W),
    (WM_SILVA_S, WM_SOVEREIGN),
    (WM_SILVA_S, WM_HYULEN),
    (WM_SILVA_S, WM_SILVA_E),
    (WM_HYULEN, WM_LAB),
    (WM_HERMIT, WM_SOUTH_ROAD),
    (WM_MEREN, WM_SOVEREIGN),
    (WM_MEREN, WM_GATE),
    (WM_GEHIN, WM_MARSH),
    (WM_GEHIN, WM_GOLDEN),
    (WM_PUREN, WM_HARDSHIP),
    (WM_SILVA_W, WM_CAVE),
    (WM_SILVA_W, WM_SOVEREIGN),
    (WM_MARSH, WM_GATE),
    (WM_MARSH, WM_TUNNEL),
    (WM_MARSH, WM_SOUTH_ROAD),
    (WM_HARDSHIP, WM_BONES),
    (WM_HARDSHIP, WM_GOLDEN),
    (WM_GOLDEN, WM_BONES),
    (WM_BONES, WM_RUINED_LAB),
    (WM_BONES, WM_RUINS),
    (WM_FORGOTTEN, WM_TUNNEL),
    (WM_FORGOTTEN, WM_TEMPLE),
    (WM_GEHIN, WM_ARENA),
];

/// The harbour's ships (map 5 「게힌시티 항구」, trigger 6: a choice of 메렌, 퓌렌, 휼렌). Drawn as
/// dashed sea lanes bending through `(x, y)` (thousandths, over the inner sea); not roads — no
/// arrow follows them.
pub const WM_SEA_LANES: &[(usize, usize, u16, u16)] = &[
    (WM_GEHIN, WM_MEREN, 450, 600),
    (WM_GEHIN, WM_PUREN, 690, 560),
    (WM_GEHIN, WM_HYULEN, 500, 470),
];

/// The title band's words.
pub const WM_TITLE: &str = "월드맵";

/// The lines the game's font letters for the map (Phase 7-3), as its EUC-KR: every place's name
/// in [`WM_PLACES`] order, then [`WM_TITLE`]. A label's index is its place's.
pub fn wm_label_lines() -> Vec<Vec<u8>> {
    WM_PLACES.iter().map(|p| p.name).chain([WM_TITLE]).map(|s| hz_java::euc_kr(s).unwrap_or_default()).collect()
}

/// The places a road leads to from `from`.
pub fn wm_neighbours(from: usize) -> impl Iterator<Item = usize> {
    WM_ROADS.iter().filter_map(move |&(a, b)| if a == from { Some(b) } else if b == from { Some(a) } else { None })
}

/// Which place a live map number is (`None`: the epilogue room, a map the table does not know).
pub fn wm_place_index(map: u8) -> Option<usize> {
    WM_PLACES.iter().position(|p| p.maps.contains(&map))
}

// ----------------------------------------------------------------------- the picture ----

/// The painted continent (RGB565, `w × h`), loaded once from the assets (Phase 7-4). Without one
/// the map draws on a flat sea over [`WM_FLAT_SIZE`].
pub struct WmPicture {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u16>,
}

static PICTURE: std::sync::OnceLock<WmPicture> = std::sync::OnceLock::new();

/// The picture's virtual size when there is none: the camera still frames two fifths of it.
pub const WM_FLAT_SIZE: (u32, u32) = (2560, 1440);
/// The flat sea drawn when there is no picture.
pub const WM_FLAT_SEA: u32 = 0x0024_4a6e;

/// `w`, `h` (LE u32 each), then `w × h` LE RGB565 — the asset's format. `None` if it is not that.
pub fn wm_parse_picture(raw: &[u8]) -> Option<WmPicture> {
    let w = u32::from_le_bytes(raw.get(0..4)?.try_into().ok()?);
    let h = u32::from_le_bytes(raw.get(4..8)?.try_into().ok()?);
    let px: Vec<u16> = raw[8..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    (w > 0 && h > 0 && px.len() == (w as usize) * (h as usize)).then_some(WmPicture { w, h, px })
}

/// Install the picture (once; later calls are ignored).
pub fn wm_set_picture(p: WmPicture) {
    let _ = PICTURE.set(p);
}

pub fn wm_picture() -> Option<&'static WmPicture> {
    PICTURE.get()
}

/// The size the camera works in: the picture's, or the flat stand-in's.
pub fn wm_picture_size() -> (u32, u32) {
    wm_picture().map_or(WM_FLAT_SIZE, |p| (p.w, p.h))
}

// ------------------------------------------------------------------------- the popup ----

/// A rectangle in window pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct WmBox {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl WmBox {
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x as i32 && py >= self.y as i32 && px < (self.x + self.w) as i32 && py < (self.y + self.h) as i32
    }
}

/// Where everything sits: the chrome frame, the black title band inside it, the picture window
/// under the band and the round back button at the band's right end. `unit` is one hero5 logical
/// pixel in window pixels (the picture window is 240 of them tall), so every stud, pin and box
/// keeps hero5's proportions at any resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct WmPanel {
    pub frame: WmBox,
    pub band: WmBox,
    pub view: WmBox,
    pub back: WmBox,
    pub unit: u32,
}

/// Frame thickness of the chrome around the band and the picture (window px).
pub const WM_EDGE: u32 = 4;

/// hero2's `wm_panel`: 4 % in from the sides, the top below the hanging tabs (`top`, so the MAP tab
/// that opened it stays visible and closes it again), 4 % above the bottom.
pub fn wm_panel(win_w: u32, win_h: u32, top: u32) -> WmPanel {
    if win_w == 0 || win_h == 0 {
        return WmPanel::default();
    }
    let (fx, fw) = if win_w >= win_h { (win_w * 4 / 100, win_w - win_w * 8 / 100) } else { (win_w * 2 / 100, win_w - win_w * 4 / 100) };
    let fy = top.max(win_h * 2 / 100).min(win_h / 3);
    let fh = win_h.saturating_sub(fy + win_h * 4 / 100).max(1);
    let frame = WmBox { x: fx, y: fy, w: fw, h: fh };
    let inner_w = fw.saturating_sub(2 * WM_EDGE).max(1);
    let inner_h = fh.saturating_sub(2 * WM_EDGE).max(1);
    // One hero5 pixel: hero5's screen is 400 × 240, so the unit is the smaller of the picture
    // window's height / 240 and width / 400; the band is 1/11 of the inside, at most 24 units.
    let unit0 = ((inner_h + 120) / 240).min((inner_w + 200) / 400).max(1);
    let band_h = (inner_h / 11).min(24 * unit0).max(8);
    let view_h = inner_h.saturating_sub(band_h).max(1);
    let unit = ((view_h + 120) / 240).min((inner_w + 200) / 400).max(1);
    let band = WmBox { x: fx + WM_EDGE, y: fy + WM_EDGE, w: inner_w, h: band_h };
    let view = WmBox { x: fx + WM_EDGE, y: band.y + band_h, w: inner_w, h: view_h };
    let bd = band_h.saturating_sub(2).max(4);
    let back = WmBox { x: (band.x + band.w).saturating_sub(bd + band_h / 2), y: band.y + 1, w: bd, h: bd };
    WmPanel { frame, band, view, back, unit }
}

// ------------------------------------------------------------------------ the camera ----

/// The window of the picture currently on screen, in picture pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct WmCam {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// How much of the picture's width the camera shows: two fifths, hero2's — the place, its
/// neighbours and a few beyond, the names readable.
pub const WM_CAM_FRACTION: (u32, u32) = (2, 5);

/// The camera centred on place `index`, clamped inside the picture.
pub fn wm_open_at(index: usize, img_w: u32, img_h: u32, view_w: u32, view_h: u32) -> WmCam {
    let place = &WM_PLACES[index];
    let (vw, vh) = wm_view_size(img_w, img_h, view_w, view_h);
    let ax = u32::from(place.x) * img_w / 1000;
    let ay = u32::from(place.y) * img_h / 1000;
    WmCam {
        x: ax.saturating_sub(vw / 2).min(img_w.saturating_sub(vw)),
        y: ay.saturating_sub(vh / 2).min(img_h.saturating_sub(vh)),
        w: vw,
        h: vh,
    }
}

fn wm_view_size(img_w: u32, img_h: u32, view_w: u32, view_h: u32) -> (u32, u32) {
    if img_w == 0 || img_h == 0 || view_w == 0 || view_h == 0 {
        return (img_w, img_h);
    }
    let mut vw = (img_w * WM_CAM_FRACTION.0 / WM_CAM_FRACTION.1).clamp(1, img_w);
    let vh = ((vw as u64 * view_h as u64) / view_w as u64).clamp(1, img_h as u64) as u32;
    if vh == img_h {
        vw = ((vh as u64 * view_w as u64) / view_h as u64).clamp(1, img_w as u64) as u32;
    }
    (vw, vh)
}

/// One frame of hero5's camera motion: half the remaining distance (at least a pixel). `true` when
/// it arrived.
pub fn wm_glide(cam: &mut WmCam, target: WmCam) -> bool {
    let step = |from: u32, to: u32| -> u32 {
        if from == to {
            return to;
        }
        let s = (from.abs_diff(to) / 2).max(1);
        if to > from { from + s } else { from - s }
    };
    cam.x = step(cam.x, target.x);
    cam.y = step(cam.y, target.y);
    cam.w = target.w;
    cam.h = target.h;
    cam.x == target.x && cam.y == target.y
}

/// An arrow, or a diagonal: the keypad's 1·3·7·9, or two arrows pressed together (a pad's D-pad
/// corner, the stick) — the only way past four roads by keys (실바나르 남부 has five).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WmDir {
    Left,
    Right,
    Up,
    Down,
    UpLeft,
    UpRight,
    DownLeft,
    DownRight,
}

impl WmDir {
    /// The direction on the picture (y down), one step per axis.
    fn vector(self) -> (i64, i64) {
        match self {
            WmDir::Left => (-1, 0),
            WmDir::Right => (1, 0),
            WmDir::Up => (0, -1),
            WmDir::Down => (0, 1),
            WmDir::UpLeft => (-1, -1),
            WmDir::UpRight => (1, -1),
            WmDir::DownLeft => (-1, 1),
            WmDir::DownRight => (1, 1),
        }
    }

    /// Two arrows at right angles as one diagonal; `None` for anything else.
    pub fn with(self, other: WmDir) -> Option<WmDir> {
        match (self.vector(), other.vector()) {
            ((0, v), (h, 0)) | ((h, 0), (0, v)) if h != 0 && v != 0 => Some(match (h, v) {
                (-1, -1) => WmDir::UpLeft,
                (1, -1) => WmDir::UpRight,
                (-1, _) => WmDir::DownLeft,
                _ => WmDir::DownRight,
            }),
            _ => None,
        }
    }
}

/// The place an arrow reaches from `from`, along the roads only: of the neighbours that lie that
/// way at all (a positive component in the arrow's direction), the one at the smallest angle to
/// it, distance breaking ties; `None` when no road leaves that way. A diagonal is measured the
/// same way, along its own line.
pub fn wm_step(from: usize, dir: WmDir) -> Option<usize> {
    let a = WM_PLACES[from];
    let (vx, vy) = dir.vector();
    let mut best: Option<(usize, i64)> = None;
    for i in wm_neighbours(from) {
        let b = WM_PLACES[i];
        let dx = i64::from(b.x) - i64::from(a.x);
        let dy = i64::from(b.y) - i64::from(a.y);
        // Along the direction and off it (both × its length, which the ratio cancels).
        let (primary, side) = (vx * dx + vy * dy, (vx * dy - vy * dx).abs());
        if primary <= 0 {
            continue;
        }
        let rank = (side * 100_000 / primary) * 1_000_000 + primary * primary + side * side;
        if best.is_none_or(|(_, r)| rank < r) {
            best = Some((i, rank));
        }
    }
    best.map(|(i, _)| i)
}

/// Picture point (thousandths) → window pixel inside `view`, clipped or not.
pub fn wm_screen_raw(cam: WmCam, px: u16, py: u16, img_w: u32, img_h: u32, view: WmBox) -> (i32, i32) {
    if cam.w == 0 || cam.h == 0 || img_w == 0 || img_h == 0 {
        return (0, 0);
    }
    let sx = i64::from(px) * i64::from(img_w) / 1000;
    let sy = i64::from(py) * i64::from(img_h) / 1000;
    let x = (sx - i64::from(cam.x)) * i64::from(view.w) / i64::from(cam.w);
    let y = (sy - i64::from(cam.y)) * i64::from(view.h) / i64::from(cam.h);
    (x as i32 + view.x as i32, y as i32 + view.y as i32)
}

/// Same, `None` when the point is outside the window.
pub fn wm_screen(cam: WmCam, px: u16, py: u16, img_w: u32, img_h: u32, view: WmBox) -> Option<(i32, i32)> {
    let (x, y) = wm_screen_raw(cam, px, py, img_w, img_h, view);
    view.contains(x, y).then_some((x, y))
}

/// The place a touch at `(x, y)` lands on: the nearest stud within 12 hero5 px, or its name box
/// where it is drawn ([`wm_name_box`]; `label_w(i)` = place i's name width in window px, 0 while
/// it has no picture; `inset` = the game-style minimap's box, if up).
#[allow(clippy::too_many_arguments)]
pub fn wm_hit_place(cam: WmCam, img_w: u32, img_h: u32, view: WmBox, unit: u32, label_w: impl Fn(usize) -> u32, inset: Option<WmBox>, x: i32, y: i32) -> Option<usize> {
    if !view.contains(x, y) {
        return None;
    }
    let reach = i64::from(12 * unit);
    let mut best: Option<(usize, i64)> = None;
    for (i, p) in WM_PLACES.iter().enumerate() {
        let Some((cx, cy)) = wm_screen(cam, p.x, p.y, img_w, img_h, view) else { continue };
        let d = i64::from(x - cx).pow(2) + i64::from(y - cy).pow(2);
        let hit_box = label_w(i) > 0 && wm_name_box(cx, cy, label_w(i), unit, view, inset).is_some_and(|b| b.contains(x, y));
        if (d <= reach * reach || hit_box) && best.is_none_or(|(_, bd)| d < bd) {
            best = Some((i, d));
        }
    }
    best.map(|(i, _)| i)
}

/// hero5's half-transparent name box under a stud: the text plus 4 px each side, 17 px high,
/// starting 10 px under the stud (hero5 px, times `unit`), centred on the stud.
pub fn wm_label_box(cx: i32, cy: i32, text_w: u32, unit: u32) -> WmBox {
    let w = text_w + 8 * unit;
    let h = 17 * unit;
    WmBox { x: (cx - (w / 2) as i32).max(0) as u32, y: (cy + (10 * unit) as i32).max(0) as u32, w, h }
}

/// A name box pushed back inside `view` when it spills over an edge; `None` when it is wider or
/// taller than the window.
pub fn wm_label_inside(b: WmBox, view: WmBox) -> Option<WmBox> {
    if b.w > view.w || b.h > view.h {
        return None;
    }
    let x = b.x.max(view.x).min(view.x + view.w - b.w);
    let y = b.y.max(view.y).min(view.y + view.h - b.h);
    Some(WmBox { x, y, w: b.w, h: b.h })
}

/// Where a place's name is drawn: hero5's box under the stud, pushed back inside the window, and
/// kept off the game-style minimap inset (the S25 showed 「실바나르 동부」 half under it,
/// 2026-10-23) — ending at the inset's left edge when the stud is left of it, over the stud when
/// the stud is above it; `None` when the stud itself is under the inset (or the box cannot fit).
pub fn wm_name_box(cx: i32, cy: i32, text_w: u32, unit: u32, view: WmBox, inset: Option<WmBox>) -> Option<WmBox> {
    let b = wm_label_inside(wm_label_box(cx, cy, text_w, unit), view)?;
    let Some(inset) = inset else { return Some(b) };
    let clear = b.x + b.w <= inset.x || b.y + b.h <= inset.y || b.x >= inset.x + inset.w || b.y >= inset.y + inset.h;
    if clear {
        return Some(b);
    }
    let gap = 2 * unit;
    if cx < inset.x as i32 {
        let x = inset.x.checked_sub(b.w + gap)?;
        return Some(WmBox { x: x.max(view.x), ..b });
    }
    if cy < inset.y as i32 {
        let y = (cy - (10 * unit + b.h) as i32).max(view.y as i32) as u32;
        return Some(WmBox { y, ..b });
    }
    None
}

/// The game-style minimap inset (Phase 7-5, the fusion with the game's own menu-8 minimap window):
/// in the picture window's bottom-right corner, a `map_w × map_h`-tile map at the largest whole
/// tile size that fits a third of the window's width and half its height, under a title band 17
/// hero5 px tall. Returns `(panel, band, cells origin, tile px)`; `None` for an empty map.
pub fn wm_inset(view: WmBox, map_w: u8, map_h: u8, unit: u32) -> Option<(WmBox, WmBox, (u32, u32), u32)> {
    if map_w == 0 || map_h == 0 {
        return None;
    }
    let (mw, mh) = (u32::from(map_w), u32::from(map_h));
    let pad = 3 * unit;
    let band_h = 17 * unit;
    let s = ((view.w / 3).saturating_sub(2 * pad) / mw).min((view.h / 2).saturating_sub(2 * pad + band_h) / mh).max(1);
    let (w, h) = (mw * s + 2 * pad, mh * s + 2 * pad + band_h);
    let margin = 8 * unit;
    let panel = WmBox { x: (view.x + view.w).saturating_sub(w + margin), y: (view.y + view.h).saturating_sub(h + margin), w, h };
    let band = WmBox { x: panel.x, y: panel.y, w, h: band_h };
    Some((panel, band, (panel.x + pad, panel.y + band_h + pad), s))
}

/// How high the pin hops after a selection (hero5's short bounce, a frame every 33 ms; `frame`
/// counts from the selection). hero5 px.
pub fn wm_pin_lift(frame: u32) -> i32 {
    const LIFT: [i32; 12] = [10, 8, 5, 2, 0, 3, 5, 3, 0, 1, 2, 0];
    LIFT.get(frame as usize).copied().unwrap_or(0)
}

// ------------------------------------------------------------------------- drawing ----

fn rgb565(p: u16) -> u32 {
    let r = u32::from((p >> 11) & 31) * 255 / 31;
    let g = u32::from((p >> 5) & 63) * 255 / 63;
    let b = u32::from(p & 31) * 255 / 31;
    (r << 16) | (g << 8) | b
}

/// Bilinear blit of `cam` into `dest`. `px` is RGB565, `iw * ih` long; a short buffer draws
/// nothing. Bilinear: the window stretches the picture 1.3–2×, and nearest sampling stairs it.
#[allow(clippy::too_many_arguments)]
pub fn wm_blit(buf: &mut [u32], scr_w: u32, scr_h: u32, px: &[u16], iw: u32, ih: u32, cam: WmCam, dest: WmBox) {
    let WmBox { x: dx, y: dy, w: dw, h: dh } = dest;
    if iw == 0 || ih == 0 || cam.w == 0 || cam.h == 0 || dw == 0 || dh == 0 || px.len() < (iw as usize) * (ih as usize) {
        return;
    }
    if buf.len() < (scr_w as usize) * (scr_h as usize) || dx + dw > scr_w || dy + dh > scr_h {
        return;
    }
    // 16.16 fixed-point source coordinates, sampled at pixel centres.
    let xs: Vec<(u32, u32, u32)> = (0..dw)
        .map(|x| {
            let fx = ((x as u64 * 2 + 1) * cam.w as u64 * 32768 / dw as u64).saturating_sub(32768) as u32 + (cam.x << 16);
            let x0 = (fx >> 16).min(iw - 1);
            (x0, (x0 + 1).min(iw - 1), fx & 0xffff)
        })
        .collect();
    let at = |x: u32, y: u32| px[(y * iw + x) as usize];
    for y in 0..dh {
        let fy = ((y as u64 * 2 + 1) * cam.h as u64 * 32768 / dh as u64).saturating_sub(32768) as u32 + (cam.y << 16);
        let y0 = (fy >> 16).min(ih - 1);
        let y1 = (y0 + 1).min(ih - 1);
        let wy = u64::from(fy & 0xffff);
        let row = ((dy + y) * scr_w + dx) as usize;
        for (x, &(x0, x1, wx)) in xs.iter().enumerate() {
            let wx = u64::from(wx);
            let (p00, p10, p01, p11) = (rgb565(at(x0, y0)), rgb565(at(x1, y0)), rgb565(at(x0, y1)), rgb565(at(x1, y1)));
            let mut out = 0u32;
            for sh in [16u32, 8, 0] {
                let c = |p: u32| u64::from((p >> sh) & 0xff);
                let top = c(p00) * (65536 - wx) + c(p10) * wx;
                let bot = c(p01) * (65536 - wx) + c(p11) * wx;
                let v = (top * (65536 - wy) + bot * wy) >> 32;
                out |= (v as u32 & 0xff) << sh;
            }
            buf[row + x] = out;
        }
    }
}

fn blend(buf: &mut [u32], i: usize, color: u32, a: u32) {
    let d = buf[i];
    let a = a.min(255);
    let inv = 255 - a;
    let mix = |sh: u32| ((((color >> sh) & 0xff) * a + ((d >> sh) & 0xff) * inv) / 255) & 0xff;
    buf[i] = (mix(16) << 16) | (mix(8) << 8) | mix(0);
}

/// Fill `b` with `color` at alpha `a`, clipped to `clip`.
#[allow(clippy::too_many_arguments)]
pub fn wm_fill_a(buf: &mut [u32], scr_w: u32, scr_h: u32, clip: WmBox, b: WmBox, color: u32, a: u32) {
    if a == 0 || buf.len() < (scr_w as usize) * (scr_h as usize) {
        return;
    }
    let x0 = b.x.max(clip.x);
    let y0 = b.y.max(clip.y);
    let x1 = (b.x + b.w).min(clip.x + clip.w).min(scr_w);
    let y1 = (b.y + b.h).min(clip.y + clip.h).min(scr_h);
    for y in y0..y1 {
        let row = (y * scr_w) as usize;
        for x in x0..x1 {
            if a >= 255 {
                buf[row + x as usize] = color & 0x00ff_ffff;
            } else {
                blend(buf, row + x as usize, color, a);
            }
        }
    }
}

/// hero5's edge shading: one-pixel frames from the picture's edge inward, the outermost at alpha
/// 0x88 fading to nothing over `width` px.
pub fn wm_vignette(buf: &mut [u32], scr_w: u32, scr_h: u32, view: WmBox, width: u32) {
    if width == 0 || view.w < 2 * width || view.h < 2 * width || buf.len() < (scr_w as usize) * (scr_h as usize) {
        return;
    }
    for d in 0..width {
        let a = 0x88 * (width - d) / width;
        if a == 0 {
            continue;
        }
        let (x0, x1) = (view.x + d, view.x + view.w - 1 - d);
        let (y0, y1) = (view.y + d, view.y + view.h - 1 - d);
        for x in x0..=x1 {
            blend(buf, (y0 * scr_w + x) as usize, 0, a);
            blend(buf, (y1 * scr_w + x) as usize, 0, a);
        }
        for y in (y0 + 1)..y1 {
            blend(buf, (y * scr_w + x0) as usize, 0, a);
            blend(buf, (y * scr_w + x1) as usize, 0, a);
        }
    }
}

/// Anti-aliased disc, clipped to `clip`. `r` in window px.
#[allow(clippy::too_many_arguments)]
pub fn wm_disc(buf: &mut [u32], scr_w: u32, scr_h: u32, clip: WmBox, cx: i32, cy: i32, r: f32, color: u32, a: u32) {
    if buf.len() < (scr_w as usize) * (scr_h as usize) || a == 0 {
        return;
    }
    let ri = r.ceil() as i32 + 1;
    for y in (cy - ri).max(clip.y as i32)..(cy + ri + 1).min((clip.y + clip.h).min(scr_h) as i32) {
        for x in (cx - ri).max(clip.x as i32)..(cx + ri + 1).min((clip.x + clip.w).min(scr_w) as i32) {
            let d = (((x - cx) * (x - cx) + (y - cy) * (y - cy)) as f32).sqrt();
            let cov = (r + 0.5 - d).clamp(0.0, 1.0);
            if cov > 0.0 {
                blend(buf, (y as u32 * scr_w + x as u32) as usize, color, (a as f32 * cov) as u32);
            }
        }
    }
}

/// Anti-aliased ring of thickness `t`, clipped.
#[allow(clippy::too_many_arguments)]
pub fn wm_ring(buf: &mut [u32], scr_w: u32, scr_h: u32, clip: WmBox, cx: i32, cy: i32, r: f32, t: f32, color: u32, a: u32) {
    if buf.len() < (scr_w as usize) * (scr_h as usize) || a == 0 {
        return;
    }
    let ri = r.ceil() as i32 + 1;
    for y in (cy - ri).max(clip.y as i32)..(cy + ri + 1).min((clip.y + clip.h).min(scr_h) as i32) {
        for x in (cx - ri).max(clip.x as i32)..(cx + ri + 1).min((clip.x + clip.w).min(scr_w) as i32) {
            let d = (((x - cx) * (x - cx) + (y - cy) * (y - cy)) as f32).sqrt();
            let cov = (r + 0.5 - d).clamp(0.0, 1.0) * (d - (r - t) + 0.5).clamp(0.0, 1.0);
            if cov > 0.0 {
                blend(buf, (y as u32 * scr_w + x as u32) as usize, color, (a as f32 * cov) as u32);
            }
        }
    }
}

/// Anti-aliased line of thickness `t`, clipped to `clip`. Walks the major axis and touches only
/// the pixels around the line, so a road across the whole window costs its visible length.
#[allow(clippy::too_many_arguments)]
pub fn wm_line(buf: &mut [u32], scr_w: u32, scr_h: u32, clip: WmBox, x0: i32, y0: i32, x1: i32, y1: i32, t: f32, color: u32, a: u32) {
    if buf.len() < (scr_w as usize) * (scr_h as usize) || a == 0 || clip.w == 0 || clip.h == 0 {
        return;
    }
    let (cx0, cy0) = (clip.x as i32, clip.y as i32);
    let (cx1, cy1) = ((clip.x + clip.w).min(scr_w) as i32, (clip.y + clip.h).min(scr_h) as i32);
    let (dx, dy) = ((x1 - x0) as f32, (y1 - y0) as f32);
    let len2 = dx * dx + dy * dy;
    if len2 < 0.25 {
        return;
    }
    let len = len2.sqrt();
    let half = t / 2.0;
    let dist = |px: f32, py: f32| -> f32 {
        let u = (((px - x0 as f32) * dx + (py - y0 as f32) * dy) / len2).clamp(0.0, 1.0);
        let (qx, qy) = (x0 as f32 + u * dx, y0 as f32 + u * dy);
        ((px - qx).powi(2) + (py - qy).powi(2)).sqrt()
    };
    let horizontal = dx.abs() >= dy.abs();
    let reach = (half * len / if horizontal { dx.abs() } else { dy.abs() } + 1.5).ceil() as i32;
    let (ma0, ma1) = if horizontal { (x0.min(x1), x0.max(x1)) } else { (y0.min(y1), y0.max(y1)) };
    let (lo, hi) = if horizontal { (cx0, cx1) } else { (cy0, cy1) };
    for m in (ma0 - reach).max(lo)..(ma1 + reach + 1).min(hi) {
        let u = if horizontal { (m as f32 + 0.5 - x0 as f32) / dx } else { (m as f32 + 0.5 - y0 as f32) / dy };
        let u = u.clamp(0.0, 1.0);
        let centre = if horizontal { y0 as f32 + u * dy } else { x0 as f32 + u * dx };
        let (nlo, nhi) = if horizontal { (cy0, cy1) } else { (cx0, cx1) };
        for n in (centre.floor() as i32 - reach).max(nlo)..(centre.ceil() as i32 + reach + 1).min(nhi) {
            let (px, py) = if horizontal { (m, n) } else { (n, m) };
            let cov = (half + 0.5 - dist(px as f32 + 0.5, py as f32 + 0.5)).clamp(0.0, 1.0);
            if cov > 0.0 {
                blend(buf, (py as u32 * scr_w + px as u32) as usize, color, (a as f32 * cov) as u32);
            }
        }
    }
}

/// A road is a dirt track over land; the undersea tunnel's and the arena's (indoors, under the
/// town's inn) are hero5's plain grey line.
pub fn wm_road_is_dirt(a: usize, b: usize) -> bool {
    ![a, b].iter().any(|&i| i == WM_TUNNEL || i == WM_ARENA)
}

#[allow(clippy::too_many_arguments)]
pub fn wm_draw_dirt_road(buf: &mut [u32], scr_w: u32, scr_h: u32, clip: WmBox, x0: i32, y0: i32, x1: i32, y1: i32, unit: u32) {
    let u = unit as f32;
    wm_line(buf, scr_w, scr_h, clip, x0, y0, x1, y1, 8.0 * u, 0x0075_6038, 90);
    wm_line(buf, scr_w, scr_h, clip, x0, y0, x1, y1, 6.5 * u, 0x00b7_8a49, 220);
    wm_line(buf, scr_w, scr_h, clip, x0, y0, x1, y1, 4.8 * u, 0x00df_b872, 245);
}

#[allow(clippy::too_many_arguments)]
pub fn wm_draw_road(buf: &mut [u32], scr_w: u32, scr_h: u32, clip: WmBox, x0: i32, y0: i32, x1: i32, y1: i32, unit: u32) {
    let u = unit as f32;
    wm_line(buf, scr_w, scr_h, clip, x0, y0, x1, y1, 3.0 * u, 0x0000_0000, 110);
    wm_line(buf, scr_w, scr_h, clip, x0, y0, x1, y1, 1.6 * u, 0x00e8_e8f0, 210);
}

/// A ship's lane: a dashed pale curve from `a` through the pull of `c` to `b` (a quadratic
/// Bézier), dashes 6 and gaps 4 hero5 px along it.
#[allow(clippy::too_many_arguments)]
pub fn wm_draw_sea_lane(buf: &mut [u32], scr_w: u32, scr_h: u32, clip: WmBox, a: (i32, i32), c: (i32, i32), b: (i32, i32), unit: u32) {
    let at = |t: f32| {
        let u = 1.0 - t;
        (u * u * a.0 as f32 + 2.0 * u * t * c.0 as f32 + t * t * b.0 as f32, u * u * a.1 as f32 + 2.0 * u * t * c.1 as f32 + t * t * b.1 as f32)
    };
    let (dash, gap) = (6.0 * unit as f32, 4.0 * unit as f32);
    const STEPS: usize = 96;
    let (mut run, mut on) = (0.0f32, true);
    let mut p = at(0.0);
    for i in 1..=STEPS {
        let q = at(i as f32 / STEPS as f32);
        let seg = ((q.0 - p.0).powi(2) + (q.1 - p.1).powi(2)).sqrt();
        if on {
            wm_line(buf, scr_w, scr_h, clip, p.0 as i32, p.1 as i32, q.0 as i32, q.1 as i32, 1.8 * unit as f32, 0x00e8_f4ff, 200);
        }
        run += seg;
        if run >= if on { dash } else { gap } {
            run = 0.0;
            on = !on;
        }
        p = q;
    }
}

/// What a stud is: an ordinary place, the place the character is in, or the one the cursor is on
/// (`glow` 0..=255 is hero5's breathing pulse).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WmStud {
    Plain,
    Here,
    Cursor { glow: u32, here: bool },
}

/// hero5's stud: a dark grey disc with a light rim; green for the character's place, glowing
/// under the cursor.
#[allow(clippy::too_many_arguments)]
pub fn wm_draw_stud(buf: &mut [u32], scr_w: u32, scr_h: u32, clip: WmBox, cx: i32, cy: i32, unit: u32, kind: WmStud) {
    let u = unit as f32;
    match kind {
        WmStud::Plain => {
            wm_disc(buf, scr_w, scr_h, clip, cx, cy, 3.6 * u, 0x0020_2028, 255);
            wm_disc(buf, scr_w, scr_h, clip, cx, cy, 2.6 * u, 0x0060_6068, 255);
            wm_ring(buf, scr_w, scr_h, clip, cx, cy, 3.6 * u, u, 0x00d0_d0d8, 200);
        }
        WmStud::Here => {
            wm_disc(buf, scr_w, scr_h, clip, cx, cy, 3.6 * u, 0x0010_4018, 255);
            wm_disc(buf, scr_w, scr_h, clip, cx, cy, 2.6 * u, 0x0040_d850, 255);
            wm_ring(buf, scr_w, scr_h, clip, cx, cy, 3.6 * u, u, 0x00d0_ffd8, 200);
        }
        WmStud::Cursor { glow, here } => {
            let (core, halo) = if here { (0x0040_e050u32, 0x0090_ff90u32) } else { (0x0040_a0ff, 0x00a0_e0ff) };
            wm_disc(buf, scr_w, scr_h, clip, cx, cy, (6.0 + 2.0 * glow as f32 / 255.0) * u, halo, 60 + glow * 70 / 255);
            wm_disc(buf, scr_w, scr_h, clip, cx, cy, 3.6 * u, 0x0010_2030, 255);
            wm_disc(buf, scr_w, scr_h, clip, cx, cy, 2.6 * u, core, 255);
            wm_disc(buf, scr_w, scr_h, clip, cx - (u * 0.8) as i32, cy - (u * 0.8) as i32, 0.9 * u, 0x00ff_ffff, 150 + glow * 100 / 255);
            wm_ring(buf, scr_w, scr_h, clip, cx, cy, 3.6 * u, u, 0x00ff_ffff, 230);
        }
    }
}

/// hero5's cursor pin: an orange teardrop, tip just above the stud, `lift` hero5 px higher while
/// it bounces.
#[allow(clippy::too_many_arguments)]
pub fn wm_draw_pin(buf: &mut [u32], scr_w: u32, scr_h: u32, clip: WmBox, cx: i32, cy: i32, unit: u32, lift: i32) {
    if buf.len() < (scr_w as usize) * (scr_h as usize) {
        return;
    }
    let u = unit as f32;
    let tip_y = cy as f32 - 5.0 * u - (lift * unit as i32) as f32;
    let head_r = 4.5 * u;
    let head_y = tip_y - 9.0 * u;
    let (x0, x1) = ((cx as f32 - head_r - 1.0).floor() as i32, (cx as f32 + head_r + 1.0).ceil() as i32);
    let (y0, y1) = ((head_y - head_r - 1.0).floor() as i32, (tip_y + 1.0).ceil() as i32);
    for y in y0.max(clip.y as i32)..(y1 + 1).min((clip.y + clip.h).min(scr_h) as i32) {
        for x in x0.max(clip.x as i32)..(x1 + 1).min((clip.x + clip.w).min(scr_w) as i32) {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            // Signed distance to the teardrop: the head disc, and the cone down to the tip.
            let dh = ((fx - cx as f32).powi(2) + (fy - head_y).powi(2)).sqrt() - head_r;
            let dc = if fy >= head_y && fy <= tip_y { (fx - cx as f32).abs() - head_r * (tip_y - fy) / (tip_y - head_y) } else { f32::MAX };
            let d = dh.min(dc);
            let cov = (0.5 - d).clamp(0.0, 1.0);
            if cov <= 0.0 {
                continue;
            }
            let edge = (-d - 1.2 * u).clamp(0.0, 1.0);
            let hi = (1.0 - ((fx - (cx as f32 - 1.5 * u)).powi(2) + (fy - (head_y - 1.5 * u)).powi(2)).sqrt() / (2.5 * u)).clamp(0.0, 1.0);
            let r = (240.0 * edge + 96.0 * (1.0 - edge) + 15.0 * hi).min(255.0) as u32;
            let g = (88.0 * edge + 32.0 * (1.0 - edge) + 96.0 * hi).min(255.0) as u32;
            let b = (24.0 * edge + 16.0 * (1.0 - edge) + 48.0 * hi).min(255.0) as u32;
            blend(buf, (y as u32 * scr_w + x as u32) as usize, (r << 16) | (g << 8) | b, (255.0 * cov) as u32);
        }
    }
}

/// The game's quest mark (Phase 7-6): a yellow disc with a dark rim and a dark 「!」 — what its own
/// minimap window draws on the guide tile — `r` px round, centred on `(cx, cy)`.
#[allow(clippy::too_many_arguments)]
pub fn wm_draw_quest(buf: &mut [u32], scr_w: u32, scr_h: u32, clip: WmBox, cx: i32, cy: i32, r: f32) {
    wm_disc(buf, scr_w, scr_h, clip, cx, cy, r + 1.0, 0x0030_2000, 255);
    wm_disc(buf, scr_w, scr_h, clip, cx, cy, r, 0x00ff_d820, 255);
    let (w, h) = ((r * 0.34).max(1.0), r * 0.95);
    let x0 = (cx as f32 - w / 2.0).round() as u32;
    let y0 = (cy as f32 - h * 0.62).round().max(0.0) as u32;
    wm_fill_a(buf, scr_w, scr_h, clip, WmBox { x: x0, y: y0, w: w.round() as u32, h: (h * 0.75).round() as u32 }, 0x0030_2000, 255);
    let dot = (cy as f32 + h * 0.3).round().max(0.0) as u32;
    wm_fill_a(buf, scr_w, scr_h, clip, WmBox { x: x0, y: dot, w: w.round() as u32, h: w.round() as u32 }, 0x0030_2000, 255);
}

/// hero5's round back button: a translucent dark disc, a light rim and a counter-clockwise arrow.
pub fn wm_draw_back(buf: &mut [u32], scr_w: u32, scr_h: u32, clip: WmBox, b: WmBox, pressed: bool) {
    let (cx, cy) = ((b.x + b.w / 2) as i32, (b.y + b.h / 2) as i32);
    let r = b.w.min(b.h) as f32 / 2.0;
    wm_disc(buf, scr_w, scr_h, clip, cx, cy, r, if pressed { 0x0060_6070 } else { 0x0020_2028 }, 190);
    wm_ring(buf, scr_w, scr_h, clip, cx, cy, r, (r / 9.0).max(1.0), 0x00d0_d0d8, 220);
    // The arrow: a three-quarter ring, and a wedge head at its open end.
    let ar = r * 0.5;
    let t = (r / 7.0).max(1.0);
    let ri = (ar + t + 1.0).ceil() as i32;
    for y in (cy - ri).max(clip.y as i32)..(cy + ri + 1).min((clip.y + clip.h).min(scr_h) as i32) {
        for x in (cx - ri).max(clip.x as i32)..(cx + ri + 1).min((clip.x + clip.w).min(scr_w) as i32) {
            let (fx, fy) = ((x - cx) as f32, (y - cy) as f32);
            let d = (fx * fx + fy * fy).sqrt();
            let ang = fy.atan2(fx);
            let open = ang > -1.5 && ang < -0.2;
            let cov = (ar + t / 2.0 + 0.5 - d).clamp(0.0, 1.0) * (d - (ar - t / 2.0) + 0.5).clamp(0.0, 1.0);
            if cov > 0.0 && !open {
                blend(buf, (y as u32 * scr_w + x as u32) as usize, 0x00ff_ffff, (230.0 * cov) as u32);
            }
        }
    }
    let (hx, hy) = (cx as f32 + ar * (-0.2f32).cos(), cy as f32 + ar * (-0.2f32).sin());
    let hs = t * 2.2;
    for y in ((hy - hs) as i32).max(clip.y as i32)..((hy + hs) as i32 + 1).min((clip.y + clip.h).min(scr_h) as i32) {
        for x in ((hx - hs) as i32).max(clip.x as i32)..((hx + hs) as i32 + 1).min((clip.x + clip.w).min(scr_w) as i32) {
            let (fx, fy) = (x as f32 + 0.5 - hx, y as f32 + 0.5 - hy);
            if fy.abs() <= hs && fx <= hs * 0.3 && fx >= -hs + fy.abs() {
                blend(buf, (y as u32 * scr_w + x as u32) as usize, 0x00ff_ffff, 230);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The collapsed graph `tools_re/hz_maps.py --exits` gives (2026-10-23): each road is a pair
    /// of step-trigger warps, one each way — no road without both, no pair left out.
    #[test]
    fn the_roads_are_the_games_two_way_exits() {
        let mut want: Vec<(&str, &str)> = vec![
            ("실바나르", "실바나르 남부"),
            ("실바나르 남부", "실바나르 서부"),
            ("실바나르 남부", "소버린구릉지"),
            ("실바나르 남부", "휼렌"),
            ("실바나르 남부", "실바나르 동부"),
            ("휼렌", "지하연구소"),
            ("은거지", "남부길"),
            ("메렌", "소버린구릉지"),
            ("메렌", "남부관문"),
            ("게힌시티", "습지대"),
            ("게힌시티", "황금숲"),
            ("퓌렌", "고난의길"),
            ("실바나르 서부", "동굴"),
            ("실바나르 서부", "소버린구릉지"),
            ("습지대", "남부관문"),
            ("습지대", "해저터널"),
            ("습지대", "남부길"),
            ("고난의길", "뼈의평원"),
            ("고난의길", "황금숲"),
            ("황금숲", "뼈의평원"),
            ("뼈의평원", "폐허연구소"),
            ("뼈의평원", "지하유적지"),
            ("잊혀진땅", "해저터널"),
            ("잊혀진땅", "신전"),
            ("게힌시티", "투기장"),
        ];
        let mut have: Vec<(&str, &str)> = WM_ROADS.iter().map(|&(a, b)| (WM_PLACES[a].name, WM_PLACES[b].name)).collect();
        fn norm(v: &mut [(&'static str, &'static str)]) {
            for p in v.iter_mut() {
                if p.0 > p.1 {
                    *p = (p.1, p.0);
                }
            }
            v.sort();
        }
        norm(&mut want);
        norm(&mut have);
        assert_eq!(have, want);
        for (a, b) in WM_ROADS.iter().copied().chain(WM_SEA_LANES.iter().map(|&(a, b, _, _)| (a, b))) {
            assert_ne!(a, b);
            assert!(a < WM_PLACES.len() && b < WM_PLACES.len());
        }
    }

    /// Every name is something the game's font can letter.
    #[test]
    fn every_name_is_in_the_games_font() {
        let lines = wm_label_lines();
        assert_eq!(lines.len(), WM_PLACES.len() + 1);
        for (i, l) in lines.iter().enumerate() {
            assert!(!l.is_empty(), "line {i}");
        }
    }

    /// Every map with a step trigger (the ones a player walks) is some place, and no map is two.
    #[test]
    fn every_walked_map_is_one_place() {
        let walked: &[u8] = &[
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 32, 33, 34,
            35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 46, 47, 48, 49, 50, 52, 53, 54, 55, 56, 57, 58, 59, 62, 63, 64, 66, 67, 68, 69, 70,
            72, 73, 74, 75, 76, 77, 78, 79, 82, 83, 84, 85, 86, 88, 89, 90, 91, 92, 94, 95, 96, 97, 98, 101, 102, 103, 104, 105, 106,
            107, 108, 109, 110, 111, 112, 113, 115, 116, 117, 118, 119, 136, 139, 31, 45, 51, 60, 61, 65,
        ];
        for &m in walked {
            assert!(wm_place_index(m).is_some(), "map {m}");
        }
        let mut seen = std::collections::HashSet::new();
        for p in WM_PLACES {
            for m in p.maps {
                assert!(seen.insert(*m), "map {m} in two places");
            }
        }
        assert_eq!(wm_place_index(138), None, "the epilogue room is nowhere");
    }

    /// Each road is some arrow from each end — except out of a place with more roads than there
    /// are arrows: 실바나르 남부 has five (실바나르 up, 서부 left, 소버린구릉지 down, 휼렌 right, 동부
    /// up-right), so 동부 is reached from it by the up-right diagonal. With the diagonals every road
    /// is some key from each end. No two roads cross on the picture.
    #[test]
    fn every_road_is_some_arrow_and_none_cross() {
        let degree = |i: usize| wm_neighbours(i).count();
        assert_eq!((0..WM_PLACES.len()).filter(|&i| degree(i) > 4).collect::<Vec<_>>(), vec![WM_SILVA_S]);
        const ARROWS: [WmDir; 4] = [WmDir::Left, WmDir::Right, WmDir::Up, WmDir::Down];
        const ALL: [WmDir; 8] =
            [WmDir::Left, WmDir::Right, WmDir::Up, WmDir::Down, WmDir::UpLeft, WmDir::UpRight, WmDir::DownLeft, WmDir::DownRight];
        for &(a, b) in WM_ROADS {
            for (from, to) in [(a, b), (b, a)] {
                let by_arrow = ARROWS.iter().any(|&d| wm_step(from, d) == Some(to));
                assert!(by_arrow || degree(from) > 4, "{} -> {} by no arrow", WM_PLACES[from].name, WM_PLACES[to].name);
                assert!(ALL.iter().any(|&d| wm_step(from, d) == Some(to)), "{} -> {} by no key", WM_PLACES[from].name, WM_PLACES[to].name);
            }
        }
        assert_eq!(wm_step(WM_SILVA_S, WmDir::UpRight), Some(WM_SILVA_E), "동부 from 남부 by the diagonal");
        assert_eq!(wm_step(WM_SILVA_S, WmDir::Up), Some(WM_SILVA), "while up stays 실바나르's");
        assert_eq!(wm_step(WM_SILVA_S, WmDir::Right), Some(WM_HYULEN), "and right 휼렌's");
        assert_eq!(wm_step(WM_SILVA_E, WmDir::Left), Some(WM_SILVA_S), "and back from 동부 by arrow");
        assert_eq!(WmDir::Up.with(WmDir::Right), Some(WmDir::UpRight));
        assert_eq!(WmDir::Left.with(WmDir::Down), Some(WmDir::DownLeft));
        assert_eq!(WmDir::Up.with(WmDir::Down), None, "opposite arrows are no diagonal");
        assert_eq!(WmDir::Up.with(WmDir::Up), None);
        assert_eq!(WmDir::UpLeft.with(WmDir::Right), None);
        let p = |i: usize| (i64::from(WM_PLACES[i].x), i64::from(WM_PLACES[i].y));
        let o = |a: (i64, i64), b: (i64, i64), c: (i64, i64)| (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
        for (i, &(a, b)) in WM_ROADS.iter().enumerate() {
            for &(c, d) in &WM_ROADS[i + 1..] {
                if [a, b].contains(&c) || [a, b].contains(&d) {
                    continue;
                }
                let crosses = o(p(a), p(b), p(c)) * o(p(a), p(b), p(d)) < 0 && o(p(c), p(d), p(a)) * o(p(c), p(d), p(b)) < 0;
                assert!(!crosses, "{}–{} crosses {}–{}", WM_PLACES[a].name, WM_PLACES[b].name, WM_PLACES[c].name, WM_PLACES[d].name);
            }
        }
        // A road that leaves nowhere that way: 은거지's only road runs up-right.
        assert_eq!(wm_step(WM_HERMIT, WmDir::Down), None);
        assert_eq!(wm_step(WM_HERMIT, WmDir::Up), Some(WM_SOUTH_ROAD));
    }

    /// The inner sea holds no place, and every ship's lane bends over its water.
    #[test]
    fn the_inner_sea_is_water_and_the_ships_cross_it() {
        let (cx, cy, rx, ry) = WM_INNER_SEA;
        let inside = |x: u16, y: u16| {
            let (dx, dy) = ((f32::from(x) - f32::from(cx)) / f32::from(rx), (f32::from(y) - f32::from(cy)) / f32::from(ry));
            dx * dx + dy * dy < 1.0
        };
        for p in WM_PLACES {
            assert!(!inside(p.x, p.y), "{} in the sea", p.name);
        }
        for &(a, b, qx, qy) in WM_SEA_LANES {
            assert!(inside(qx, qy), "{} – {} bends on land", WM_PLACES[a].name, WM_PLACES[b].name);
        }
    }

    #[test]
    fn the_panel_starts_below_the_tabs_and_keeps_hero5_proportions() {
        for (w, h, top) in [(2340u32, 1080u32, 70u32), (2220, 1080, 60), (1080, 2340, 70)] {
            let p = wm_panel(w, h, top);
            assert_eq!(p.frame.y, top, "{w}x{h}");
            assert!(p.view.y == p.band.y + p.band.h && p.view.y + p.view.h <= p.frame.y + p.frame.h);
            assert!(p.back.x + p.back.w <= p.band.x + p.band.w && p.back.h < p.band.h);
            assert!(p.unit >= 1 && p.view.h / p.unit >= 200, "{w}x{h}: ~240 hero5 px tall");
        }
    }

    #[test]
    fn the_camera_frames_the_place_and_glides_there() {
        let (iw, ih) = (2560, 1440);
        let p = wm_panel(2340, 1080, 70);
        let cam = wm_open_at(WM_SILVA_S, iw, ih, p.view.w, p.view.h);
        assert_eq!(cam.w, iw * 2 / 5);
        let (x, y) = wm_screen(cam, WM_PLACES[WM_SILVA_S].x, WM_PLACES[WM_SILVA_S].y, iw, ih, p.view).expect("on screen");
        assert!(p.view.contains(x, y));
        let mut c = cam;
        let target = wm_open_at(WM_RUINS, iw, ih, p.view.w, p.view.h);
        let mut frames = 0;
        while !wm_glide(&mut c, target) {
            frames += 1;
        }
        assert!(frames < 20, "halving arrives quickly: {frames}");
        assert!(target.x + target.w <= iw && target.y + target.h <= ih, "clamped inside the picture");
    }

    #[test]
    fn a_touch_finds_the_nearest_stud_or_its_name() {
        let (iw, ih) = (2560, 1440);
        let p = wm_panel(2340, 1080, 70);
        let cam = wm_open_at(WM_MARSH, iw, ih, p.view.w, p.view.h);
        let (x, y) = wm_screen(cam, WM_PLACES[WM_MARSH].x, WM_PLACES[WM_MARSH].y, iw, ih, p.view).unwrap();
        assert_eq!(wm_hit_place(cam, iw, ih, p.view, p.unit, |_| 0, None, x + 2, y - 2), Some(WM_MARSH));
        let below = y + (14 * p.unit) as i32;
        assert_eq!(wm_hit_place(cam, iw, ih, p.view, p.unit, |_| 60 * p.unit, None, x, below), Some(WM_MARSH), "on the name box");
        assert_eq!(wm_hit_place(cam, iw, ih, p.view, p.unit, |_| 0, None, x, below), None, "no name drawn, no name box");
        // A name moved off the inset answers where it is drawn.
        let inset = WmBox { x: x as u32 + 10 * p.unit, y: y as u32, w: 200 * p.unit, h: 200 * p.unit };
        let b = wm_name_box(x, y, 60 * p.unit, p.unit, p.view, Some(inset)).unwrap();
        let (bx, by) = ((b.x + 2) as i32, (b.y + b.h / 2) as i32);
        assert_eq!(wm_hit_place(cam, iw, ih, p.view, p.unit, |_| 60 * p.unit, Some(inset), bx, by), Some(WM_MARSH));
    }

    #[test]
    fn names_keep_off_the_minimap_inset() {
        let u = 4;
        let view = WmBox { x: 100, y: 100, w: 2000, h: 900 };
        let inset = WmBox { x: 1600, y: 500, w: 450, h: 450 };
        let (w, h) = (60 * u, 17 * u);
        let clear_of = |b: WmBox| b.x + b.w <= inset.x || b.y + b.h <= inset.y;
        // Far away: as hero5 draws it.
        assert_eq!(wm_name_box(800, 300, w, u, view, Some(inset)), Some(wm_label_box(800, 300, w, u)));
        assert_eq!(wm_name_box(1590, 600, w, u, view, None), Some(wm_label_box(1590, 600, w, u)), "no inset up");
        // The stud left of the inset, its name running under it: it ends at the inset's edge.
        let b = wm_name_box(1590, 600, w, u, view, Some(inset)).unwrap();
        assert!(clear_of(b) && b.x + b.w == inset.x - 2 * u && b.y == wm_label_box(1590, 600, w, u).y);
        // The stud just above the inset: the name goes over the stud.
        let b = wm_name_box(1800, 480, w, u, view, Some(inset)).unwrap();
        assert!(clear_of(b) && b.y + b.h == (480 - 10 * u as i32) as u32 && b.h == h);
        // The stud under the inset: no name (the inset covers the stud too).
        assert_eq!(wm_name_box(1800, 700, w, u, view, Some(inset)), None);
    }

    #[test]
    fn drawing_stays_inside_its_clip() {
        let (w, h) = (200u32, 120u32);
        let mut buf = vec![0x0011_2233u32; (w * h) as usize];
        let clip = WmBox { x: 20, y: 20, w: 100, h: 60 };
        wm_draw_dirt_road(&mut buf, w, h, clip, -50, 10, 300, 110, 2);
        wm_draw_sea_lane(&mut buf, w, h, clip, (0, 0), (150, 10), (199, 119), 2);
        wm_draw_stud(&mut buf, w, h, clip, 25, 25, 3, WmStud::Cursor { glow: 200, here: true });
        wm_draw_pin(&mut buf, w, h, clip, 60, 30, 3, 10);
        for y in 0..h {
            for x in 0..w {
                if !clip.contains(x as i32, y as i32) {
                    assert_eq!(buf[(y * w + x) as usize], 0x0011_2233, "({x},{y}) outside the clip");
                }
            }
        }
        assert!(buf.iter().all(|p| p >> 24 == 0), "softbuffer pixels carry no alpha byte");
    }
}
