//! The player's own control layout: where each placeable control sits and how big it is — one
//! table per orientation (portrait's since 2026-10-29, [`layout_file`]: a fraction of a tall window
//! is not the same seat in a wide one, and the built-ins differ). hero2's model (`wie_android/src/placement.rs`), which its players shaped over two months:
//!
//! * **Stored as a fraction of the window and a percent of the built-in size** — never pixels
//!   (rotation, another phone) and never guest pixels (frozen at boot). Move changes `cx`/`cy`
//!   only, resize changes `pct` only; `pct` is carried, never re-derived from a pixel rect (a
//!   round trip floors twice and a dragged control withers).
//! * **Built-ins are a pure function of the window** ([`builtin`], from `hud`); the player's
//!   choices are a layer on top, so making one control bigger can never shrink another.
//! * **Controls may overlap each other** (hero2 player, 2026-08-07: "겹칠 수 있게") — that is a
//!   cosmetic choice and the hit chain resolves a shared touch by list order. What a control may
//!   NEVER cover is an OBSTACLE (`hud::obstacles`): each of those wins the touch first, so a
//!   control under one silently does something else.
//! * **The editor's predicate IS the resolver** ([`would_accept_place`] runs [`resolve_with`] on
//!   a table holding the candidate), so what the editor accepts is exactly what is honoured.

use crate::hud::{self, Rect, Screen};

pub const PCT_MIN: u32 = 50;
pub const PCT_MAX: u32 = 250;
pub const PCT_DEFAULT: u32 = 100;

/// A placeable control, identified by its SEAT, not by the key it sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CtrlId {
    /// The walk stick's home seat (the floating stick's hint, and the centre of its zone).
    Stick,
    /// The big action disc.
    Ok,
    /// CLR (Phase 6-4): the menu in the field, "back" elsewhere (`hud::cancel_key`).
    Cancel,
    /// Quick plate seat `i` on the ring around OK, `hud::RING_PLATES[i]` (ring seat `i + 1`).
    Ring(u8),
    /// HZ-058: the plates' page button (`hud::page_seat`).
    Page,
    /// The minimap panel (Phase 6-5; hero2's `CtrlId::Minimap`): a square the player moves and
    /// sizes like a control, under rules of its own ([`hud::minimap_seat_ok`]). Last, so a
    /// control drawn over it is the one a touch picks.
    Minimap,
}

/// Every placeable control, in priority order (a shared touch goes to the earlier one).
pub const IDS: [CtrlId; 5 + hud::RING_PLATES.len()] = [
    CtrlId::Stick,
    CtrlId::Ok,
    CtrlId::Cancel,
    CtrlId::Ring(0),
    CtrlId::Ring(1),
    CtrlId::Ring(2),
    CtrlId::Ring(3),
    CtrlId::Ring(4),
    CtrlId::Ring(5),
    CtrlId::Ring(6),
    CtrlId::Page,
    CtrlId::Minimap,
];

impl CtrlId {
    /// Stable on-disk key. Renaming one drops that line from every player's layout.
    pub fn key(self) -> &'static str {
        match self {
            CtrlId::Stick => "stick",
            CtrlId::Ok => "ok",
            CtrlId::Cancel => "cancel",
            CtrlId::Ring(i) => hud::RING_PLATES[i as usize].2,
            CtrlId::Page => "page",
            CtrlId::Minimap => "map",
        }
    }

    pub fn parse(s: &str) -> Option<CtrlId> {
        match s {
            "stick" => Some(CtrlId::Stick),
            "ok" => Some(CtrlId::Ok),
            "cancel" => Some(CtrlId::Cancel),
            "map" => Some(CtrlId::Minimap),
            "page" => Some(CtrlId::Page),
            _ => hud::RING_PLATES.iter().position(|k| k.2 == s).map(|i| CtrlId::Ring(i as u8)),
        }
    }

    /// Editor caption (ASCII — the host font has nothing else).
    pub fn label(self) -> &'static str {
        match self {
            CtrlId::Stick => "STICK",
            CtrlId::Ok => "OK",
            CtrlId::Cancel => "CLR",
            CtrlId::Ring(i) => hud::RING_PLATES[i as usize].1,
            CtrlId::Page => "PAGE",
            CtrlId::Minimap => "MAP",
        }
    }

    /// Drawn as a disc (everything but the stick's square seat and the minimap).
    pub fn is_round(self) -> bool {
        !matches!(self, CtrlId::Stick | CtrlId::Minimap)
    }

    /// FLIP mirrors it: the thumbs' controls. The minimap is a picture of the map, not under a
    /// thumb — it stays where the player put it (hero2 keeps its top tabs out of FLIP the same way).
    fn mirrors(self) -> bool {
        self != CtrlId::Minimap
    }
}

fn slot(id: CtrlId) -> usize {
    IDS.iter().position(|c| *c == id).expect("every CtrlId is in IDS")
}

/// One control's player-chosen geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    /// Centre, as fractions of the window.
    pub cx: f32,
    pub cy: f32,
    /// Size, as a percent of the built-in size.
    pub pct: u32,
}

impl Place {
    pub fn from_rect(r: Rect, s: &Screen) -> Place {
        let (cx, cy) = r.centre();
        Place {
            cx: cx as f32 / s.w.max(1) as f32,
            cy: cy as f32 / s.h.max(1) as f32,
            pct: PCT_DEFAULT,
        }
    }

    /// The rect this placement asks for, its position clamped into the window (so a drag against
    /// the edge slides along it rather than stopping dead).
    pub fn to_rect(self, builtin: Rect, s: &Screen) -> Rect {
        let w = ((i64::from(builtin.w.max(1)) * i64::from(self.pct) / 100) as i32).min(s.w);
        let h = ((i64::from(builtin.h.max(1)) * i64::from(self.pct) / 100) as i32).min(s.h);
        let cx = (self.cx.clamp(0.0, 1.0) * s.w as f32) as i32;
        let cy = (self.cy.clamp(0.0, 1.0) * s.h as f32) as i32;
        Rect::new((cx - w / 2).clamp(0, s.w - w), (cy - h / 2).clamp(0, s.h - h), w, h)
    }
}

/// THE placement table. `None` = the built-in seat.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    places: [Option<Place>; IDS.len()],
    /// The quick plates the player switched off (the editor's HIDE, hero2's — hero3 82차), bit `i`
    /// = `CtrlId::Ring(i)`. Never all of them ([`Table::hide`]).
    hidden: u8,
}

/// Every plate's bit.
const ALL_PLATES: u8 = (1 << hud::RING_PLATES.len()) - 1;

impl Table {
    pub fn get(&self, id: CtrlId) -> Option<Place> {
        self.places[slot(id)]
    }

    pub fn set(&mut self, id: CtrlId, place: Option<Place>) {
        self.places[slot(id)] = place.map(|p| Place {
            pct: p.pct.clamp(PCT_MIN, PCT_MAX),
            ..p
        });
    }

    /// Every control back to its built-in seat (the editor's RESET). The plates the player switched
    /// off stay off — SHOW is what brings them back (hero2's).
    pub fn clear(&mut self) {
        self.places = Default::default();
    }

    /// Nothing changed from the built-ins: no placement, no plate switched off.
    pub fn is_empty(&self) -> bool {
        self.places.iter().all(Option::is_none) && self.hidden == 0
    }

    /// Is `id` a quick plate the player switched off?
    pub fn is_hidden(&self, id: CtrlId) -> bool {
        matches!(id, CtrlId::Ring(i) if self.hidden & (1 << i) != 0)
    }

    /// Any plate switched off?
    pub fn any_hidden(&self) -> bool {
        self.hidden != 0
    }

    /// Would [`Table::hide`] take `id`? A plate that is up, and not the last one up (hero2's
    /// `hide_plate`: a ring with no plate left has nothing to show for the game's slots).
    pub fn can_hide(&self, id: CtrlId) -> bool {
        match id {
            CtrlId::Ring(i) => {
                let bit = 1u8 << i;
                self.hidden & bit == 0 && (self.hidden | bit) != ALL_PLATES
            }
            _ => false,
        }
    }

    /// Switch the plate `id` off: not drawn, not touched — its game key is still the pad's and the
    /// keypad's. `false` (and nothing changes) when [`Table::can_hide`] says no.
    pub fn hide(&mut self, id: CtrlId) -> bool {
        if !self.can_hide(id) {
            return false;
        }
        if let CtrlId::Ring(i) = id {
            self.hidden |= 1 << i;
        }
        true
    }

    /// Every plate back on (the editor's SHOW); `false` when none was off.
    pub fn show_all(&mut self) -> bool {
        std::mem::take(&mut self.hidden) != 0
    }

    /// Mirror the whole layout left<->right (a left-handed player, in one tap). Two phases — hero2
    /// learned both the hard way:
    ///
    /// 1. mirror EVERY control, unvalidated (validating in flight lets controls not yet mirrored
    ///    block the ones moving into their seats);
    /// 2. for each control the resolver now refuses, search the nearest legal seat — straight up
    ///    first, then sideways — and never take one that costs a higher-priority control its own.
    ///
    /// Controls with no placement get one from their built-in, so a tap flips the WHOLE layout.
    pub fn mirror(&mut self, s: &Screen) {
        for id in IDS.into_iter().filter(|id| id.mirrors()) {
            let cur = self.get(id).unwrap_or_else(|| Place::from_rect(builtin(id, s), s));
            self.set(id, Some(Place { cx: 1.0 - cur.cx, ..cur }));
        }
        for (i, id) in IDS.iter().copied().enumerate().filter(|(_, id)| id.mirrors()) {
            let before = self.honoured(s);
            if before[i] {
                continue;
            }
            let Some(want) = self.get(id) else { continue };
            for (dx, dy) in mirror_search() {
                let cand = Place {
                    cx: (want.cx + dx).clamp(0.0, 1.0),
                    cy: (want.cy + dy).clamp(0.0, 1.0),
                    ..want
                };
                let mut trial = self.clone();
                trial.set(id, Some(cand));
                let after = trial.honoured(s);
                if after[i] && before[..i].iter().zip(&after[..i]).all(|(was, now)| !*was || *now) {
                    *self = trial;
                    break;
                }
            }
        }
    }

    /// Per control (in [`IDS`] order): is its stored request the rect it actually gets? A control
    /// with no request counts as honoured.
    pub fn honoured(&self, s: &Screen) -> Vec<bool> {
        let resolved = resolve_with(s, self);
        IDS.iter()
            .enumerate()
            .map(|(i, id)| self.get(*id).is_none_or(|p| resolved[i].1 == p.to_rect(builtin(*id, s), s)))
            .collect()
    }

    /// `field <key> <cx> <cy> <pct>` per stored control (hero2's format; `field` keeps the file
    /// self-describing), then `hide <key>` per plate switched off.
    pub fn to_text(&self) -> String {
        let mut out = String::from("# 영웅서기 Zero touch layout — field <control> <centre x> <centre y> <size %> / hide <plate>\n");
        for id in IDS {
            if let Some(p) = self.get(id) {
                out.push_str(&format!("field {} {:.4} {:.4} {}\n", id.key(), p.cx, p.cy, p.pct));
            }
        }
        for id in IDS.into_iter().filter(|id| self.is_hidden(*id)) {
            out.push_str(&format!("hide {}\n", id.key()));
        }
        out
    }

    /// Parse a layout file defensively, line by line: a bad line is dropped and counted, every
    /// other line still loads (one stray character must not cost a player their whole layout). A
    /// `hide` that would leave no plate up is dropped the same way.
    pub fn from_text(text: &str) -> (Table, usize) {
        let mut t = Table::default();
        let mut dropped = 0;
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let ok = match line.strip_prefix("hide ") {
                Some(key) => CtrlId::parse(key.trim()).is_some_and(|id| t.is_hidden(id) || t.hide(id)),
                None => parse_line(line).map(|(id, p)| t.set(id, Some(p))).is_some(),
            };
            if !ok {
                dropped += 1;
            }
        }
        (t, dropped)
    }
}

/// The layout files in the app-private data dir: landscape's (the one there always was) and
/// portrait's.
pub const LAYOUT_FILE: &str = "hz_layout.txt";
pub const LAYOUT_FILE_PORTRAIT: &str = "hz_layout_portrait.txt";

/// The file of the layout for a window `landscape` or not.
pub fn layout_file(landscape: bool) -> &'static str {
    if landscape { LAYOUT_FILE } else { LAYOUT_FILE_PORTRAIT }
}

/// The stored layout (empty = the built-ins) and how many lines were dropped as unreadable.
pub fn load(dir: &std::path::Path, landscape: bool) -> (Table, usize) {
    std::fs::read_to_string(dir.join(layout_file(landscape))).map_or((Table::default(), 0), |t| Table::from_text(&t))
}

/// Write the layout atomically (temp + rename): a kill mid-write leaves the old file, not half a
/// new one.
pub fn save(dir: &std::path::Path, t: &Table, landscape: bool) -> std::io::Result<()> {
    let file = layout_file(landscape);
    let tmp = dir.join(format!("{file}.tmp"));
    std::fs::write(&tmp, t.to_text())?;
    std::fs::rename(&tmp, dir.join(file))
}

fn parse_line(line: &str) -> Option<(CtrlId, Place)> {
    let mut it = line.split_whitespace();
    if it.next()? != "field" {
        return None;
    }
    let id = CtrlId::parse(it.next()?)?;
    let cx: f32 = it.next()?.parse().ok()?;
    let cy: f32 = it.next()?.parse().ok()?;
    let pct: u32 = it.next()?.parse().ok()?;
    if it.next().is_some() || !(0.0..=1.0).contains(&cx) || !(0.0..=1.0).contains(&cy) {
        return None;
    }
    Some((
        id,
        Place {
            cx,
            cy,
            pct: pct.clamp(PCT_MIN, PCT_MAX),
        },
    ))
}

/// [`Table::mirror`]'s search offsets: straight up first (the bottom band is where a mirrored
/// cluster collides), then progressively sideways.
fn mirror_search() -> impl Iterator<Item = (f32, f32)> {
    (0..11).flat_map(|step| {
        let dy = -0.05 * step as f32;
        [0.0f32, 0.04, -0.04, 0.08, -0.08].into_iter().map(move |dx| (dx, dy))
    })
}

/// A control's built-in rect — a pure function of the screen.
pub fn builtin(id: CtrlId, s: &Screen) -> Rect {
    match id {
        CtrlId::Stick => hud::stick_seat(s),
        CtrlId::Ok => hud::ok_disc(s),
        CtrlId::Cancel => hud::cancel_seat(s),
        CtrlId::Ring(i) => hud::plate_seat(s, i as usize),
        CtrlId::Page => hud::page_seat(s),
        CtrlId::Minimap => hud::minimap_panel(s),
    }
}

/// Every control with the player's placement applied wherever it is legal, in [`IDS`] order.
///
/// One pass is exact: a seat's legality depends only on the window and the obstacles, never on
/// another control (controls may overlap), so no acceptance can change another's. hero2 needed a
/// fixpoint because its controls once excluded each other — if that ever comes back, so must the
/// fixpoint.
pub fn resolve_with(s: &Screen, table: &Table) -> Vec<(CtrlId, Rect)> {
    let obstacles = hud::obstacles(s);
    IDS.iter()
        .map(|&id| {
            let base = builtin(id, s);
            let r = table
                .get(id)
                .map(|p| p.to_rect(base, s))
                .filter(|r| accepts(s, *r, &obstacles) && (id != CtrlId::Minimap || hud::minimap_seat_ok(s, *r)))
                .unwrap_or(base);
            (id, r)
        })
        .collect()
}

/// Inside the window, and clear of every obstacle by a pad (strict, unlike control vs control).
fn accepts(s: &Screen, r: Rect, obstacles: &[Rect]) -> bool {
    !r.is_empty() && r.inside(&s.window()) && !obstacles.iter().any(|o| r.grown(s.pad()).intersects(o))
}

/// "If I stored this, would it be honoured — and would every OTHER control stay where it is?"
/// Answered by running the real resolver (hero2: two look-alike predicates diverged, and the
/// editor accepted placements the layout then dropped under the player's finger).
pub fn would_accept_place(s: &Screen, table: &Table, id: CtrlId, place: Place) -> bool {
    let want = place.to_rect(builtin(id, s), s);
    let mut probe = table.clone();
    probe.set(id, Some(place));
    let after = resolve_with(s, &probe);
    if !after.iter().any(|(c, r)| *c == id && *r == want) {
        return false;
    }
    let before = resolve_with(s, table);
    IDS.iter().enumerate().all(|(j, c)| {
        *c == id
            || table.get(*c).is_none_or(|p| {
                let asked = p.to_rect(builtin(*c, s), s);
                before[j].1 != asked || after[j].1 == asked
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hud::tests::WINDOWS;

    fn s9() -> Screen {
        Screen::for_window(2220, 1080)
    }

    #[test]
    fn keys_round_trip_and_labels_draw() {
        for id in IDS {
            assert_eq!(CtrlId::parse(id.key()), Some(id));
            assert!(crate::font::drawable(id.label()));
        }
        assert_eq!(CtrlId::parse("dpad"), None);
    }

    #[test]
    fn an_empty_table_is_the_built_ins() {
        for (w, h) in WINDOWS {
            let s = Screen::for_window(w, h);
            for (id, r) in resolve_with(&s, &Table::default()) {
                assert_eq!(r, builtin(id, &s));
            }
        }
    }

    #[test]
    fn a_placement_is_honoured_or_falls_back_to_the_built_in() {
        let s = s9();
        let mut t = Table::default();
        let free = Place {
            cx: 0.30,
            cy: 0.40,
            pct: 130,
        };
        t.set(CtrlId::Ok, Some(free));
        let r = resolve_with(&s, &t);
        assert_eq!(r[1].1, free.to_rect(builtin(CtrlId::Ok, &s), &s));
        // Down to the window's bottom edge (no game bar since HZ-013): honoured.
        let low = Place { cx: 0.5, cy: 0.97, pct: 100 };
        t.set(CtrlId::Ok, Some(low));
        assert_eq!(resolve_with(&s, &t)[1].1, low.to_rect(builtin(CtrlId::Ok, &s), &s));
        // On the gear: refused, back to the built-in, and the stored value untouched.
        let g0 = hud::gear(&s);
        let on_g = Place { cx: g0.centre().0 as f32 / s.w as f32, cy: g0.centre().1 as f32 / s.h as f32, pct: 100 };
        t.set(CtrlId::Ok, Some(on_g));
        assert_eq!(resolve_with(&s, &t)[1].1, builtin(CtrlId::Ok, &s));
        assert_eq!(t.get(CtrlId::Ok), Some(on_g));
        let g = hud::gear(&s);
        let on_gear = Place {
            cx: g.centre().0 as f32 / s.w as f32,
            cy: g.centre().1 as f32 / s.h as f32,
            pct: 50,
        };
        assert!(!would_accept_place(&s, &Table::default(), CtrlId::Ring(0), on_gear));
    }

    #[test]
    fn controls_may_overlap_each_other() {
        let s = s9();
        let ok = builtin(CtrlId::Ok, &s);
        let on_ok = Place {
            cx: ok.centre().0 as f32 / s.w as f32,
            cy: ok.centre().1 as f32 / s.h as f32,
            pct: 100,
        };
        assert!(would_accept_place(&s, &Table::default(), CtrlId::Ring(3), on_ok));
    }

    #[test]
    fn dragging_never_changes_the_size() {
        let s = s9();
        let mut t = Table::default();
        let mut p = Place { cx: 0.5, cy: 0.6, pct: 137 };
        for step in 0..50 {
            p.cx = 0.2 + step as f32 * 0.002; // clear of the keypad panel at the top centre
            assert!(would_accept_place(&s, &t, CtrlId::Ok, p));
            t.set(CtrlId::Ok, Some(p));
        }
        assert_eq!(t.get(CtrlId::Ok).unwrap().pct, 137);
    }

    #[test]
    fn the_editor_predicate_agrees_with_the_resolver() {
        // Fuzz: whatever the predicate says, storing it and resolving must agree.
        let s = s9();
        let (mut yes, mut no) = (0, 0);
        let mut seed = 12345u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            (seed >> 8) as f32 / (1 << 24) as f32
        };
        for _ in 0..300 {
            let id = IDS[(rnd() * IDS.len() as f32) as usize % IDS.len()];
            let p = Place {
                cx: rnd(),
                cy: rnd(),
                pct: PCT_MIN + (rnd() * (PCT_MAX - PCT_MIN) as f32) as u32,
            };
            let ok = would_accept_place(&s, &Table::default(), id, p);
            let mut t = Table::default();
            t.set(id, Some(p));
            let got = resolve_with(&s, &t).into_iter().find(|(c, _)| *c == id).unwrap().1;
            assert_eq!(ok, got == p.to_rect(builtin(id, &s), &s), "{id:?} {p:?}");
            if ok { yes += 1 } else { no += 1 }
        }
        assert!(yes > 30 && no > 30, "both answers occur ({yes}/{no})");
    }

    #[test]
    fn flip_sends_the_stick_right_and_the_cluster_left() {
        for (w, h) in WINDOWS {
            let s = Screen::for_window(w, h);
            let mut t = Table::default();
            t.mirror(&s);
            let r = resolve_with(&s, &t);
            let mid = s.w / 2;
            for (id, rect) in &r {
                let (cx, _) = rect.centre();
                match id {
                    CtrlId::Stick => assert!(cx > mid, "{w}x{h}: stick stayed left"),
                    CtrlId::Minimap => assert_eq!(*rect, builtin(*id, &s), "{w}x{h}: the minimap does not flip"),
                    _ => assert!(cx < mid, "{w}x{h}: {id:?} stayed right"),
                }
            }
            assert!(t.honoured(&s).iter().all(|h| *h), "{w}x{h}: every flipped seat is honoured");
            // Flipping back restores the built-in sides.
            t.mirror(&s);
            let back = resolve_with(&s, &t);
            assert!(back[0].1.centre().0 < mid);
        }
    }

    #[test]
    fn a_layout_survives_another_screen() {
        // Authored on the S9, the same file is legal (or cleanly falls back) everywhere.
        let s = s9();
        let mut t = Table::default();
        t.set(
            CtrlId::Ok,
            Some(Place {
                cx: 0.85,
                cy: 0.55,
                pct: 160,
            }),
        );
        t.set(CtrlId::Stick, Some(Place { cx: 0.15, cy: 0.5, pct: 80 }));
        for (w, h) in WINDOWS {
            let other = Screen::for_window(w, h);
            let obstacles = hud::obstacles(&other);
            for (_, r) in resolve_with(&other, &t) {
                assert!(r.inside(&other.window()));
                assert!(!obstacles.iter().any(|o| r.grown(other.pad()).intersects(o)), "{w}x{h}: {r:?}");
            }
        }
        let _ = s;
    }

    #[test]
    fn the_file_round_trips_and_a_bad_line_costs_only_itself() {
        let mut t = Table::default();
        t.set(CtrlId::Ok, Some(Place { cx: 0.25, cy: 0.5, pct: 120 }));
        t.set(CtrlId::Ring(5), Some(Place { cx: 0.1, cy: 0.2, pct: 999 }));
        let text = t.to_text();
        let (back, dropped) = Table::from_text(&text);
        assert_eq!(dropped, 0);
        assert_eq!(back.get(CtrlId::Ok), t.get(CtrlId::Ok));
        assert_eq!(back.get(CtrlId::Ring(5)).unwrap().pct, PCT_MAX, "clamped");
        let broken = format!("{text}field ok 0.x 0.5 100\nfield nosuch 0.5 0.5 100\nfield k1 1.5 0.5 100\nmenu ok 0.5 0.5 100\n");
        let (back, dropped) = Table::from_text(&broken);
        assert_eq!(dropped, 4);
        assert_eq!(back.get(CtrlId::Ok), t.get(CtrlId::Ok), "the good lines still load");
    }

    #[test]
    fn plates_switch_off_and_on_never_all_and_persist() {
        let mut t = Table::default();
        assert!(!t.can_hide(CtrlId::Ok) && !t.hide(CtrlId::Stick), "only a quick plate");
        assert!(t.hide(CtrlId::Ring(1)) && t.is_hidden(CtrlId::Ring(1)) && !t.is_empty());
        assert!(!t.hide(CtrlId::Ring(1)), "already off");
        for i in [0, 2, 3, 4, 5] {
            assert!(t.hide(CtrlId::Ring(i)));
        }
        assert!(!t.can_hide(CtrlId::Ring(6)) && !t.hide(CtrlId::Ring(6)), "the last plate stays up");
        // RESET keeps them off; the file carries them.
        t.set(CtrlId::Ok, Some(Place { cx: 0.3, cy: 0.5, pct: 120 }));
        let text = t.to_text();
        assert!(text.contains("hide k7\n") && text.contains("hide khash") && !text.contains("hide p7"), "{text}");
        let (back, dropped) = Table::from_text(&text);
        assert_eq!((back.clone(), dropped), (t.clone(), 0));
        t.clear();
        assert!(t.is_hidden(CtrlId::Ring(0)) && t.get(CtrlId::Ok).is_none());
        // A hand-made file that hides all seven: the last line is dropped, one plate stays up.
        let all: String = hud::RING_PLATES.iter().map(|k| format!("hide {}\n", k.2)).collect();
        let (seven, dropped) = Table::from_text(&all);
        assert_eq!(dropped, 1);
        assert!(!seven.is_hidden(CtrlId::Ring(6)));
        assert_eq!(Table::from_text("hide ok\nhide nosuch\n").1, 2);
        // SHOW: all back.
        assert!(t.show_all() && !t.any_hidden() && t.is_empty());
        assert!(!t.show_all(), "nothing to show");
    }

    #[test]
    fn save_and_load() {
        let dir = std::env::temp_dir().join(format!("hz_layout_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(load(&dir, true), (Table::default(), 0), "no file = the built-ins");
        let mut t = Table::default();
        t.set(CtrlId::Stick, Some(Place { cx: 0.8, cy: 0.6, pct: 90 }));
        save(&dir, &t, true).unwrap();
        assert_eq!(load(&dir, true).0, t);
        assert!(!dir.join(format!("{LAYOUT_FILE}.tmp")).exists());
        // Portrait's is a file of its own: neither orientation's layout moves the other's.
        assert_eq!(load(&dir, false).0, Table::default());
        let mut p = Table::default();
        p.set(CtrlId::Ok, Some(Place { cx: 0.7, cy: 0.9, pct: 120 }));
        save(&dir, &p, false).unwrap();
        assert_eq!((load(&dir, true).0, load(&dir, false).0), (t, p));
        assert!(dir.join(LAYOUT_FILE_PORTRAIT).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
