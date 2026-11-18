//! What the guest is showing — read on the game's own thread by the HZ-008 blit hook once a
//! frame, published as a [`GuestView`] for the touch HUD (`hz_android`), which shows the number
//! keys only where the game listens to them.
//!
//! The source is the game's mode byte `ed.d:B`, the switch `ce.a(II)Z` routes every key on
//! (javap 2026-10-22): 2 = the title window `i`, 3 = the field (the only mode that reads the
//! number keys: skills `1 3 7 9`, items `0 #`, minimap `*`), 4 = a scenario `eb`, 5 = the menu
//! `bh`, 8 = a yes/no box, 11 = game over, 12 = `ct`. Two overlays sit above the mode: the
//! message-box queue `ax.a` (keys go to it while non-empty) and the 「# 확인」 notice `ed.f`
//! (`ce.a(II)Z` @14: `#` dismisses it whatever the mode).
//!
//! The same hook reads what the host HUD draws in the field: the player's numbers ([`Stats`]),
//! the quick slots ([`QuickBar`]), the minimap ([`MapGrid`], [`MapMarks`]) and the portrait
//! (`crate::portrait`).

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicU64, Ordering},
    },
};

/// The game's mode (`ed.d:B`), named where the HUD cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Scene {
    /// Nothing reported yet: the patch is off, the jar is unknown, or the first frame is not
    /// drawn. The HUD shows everything, as it did before HZ-008 existed.
    #[default]
    Unknown,
    /// The canvas's screen is not the game screen `ce` yet (the logo `ch`).
    Loading,
    /// 2 — the title window.
    Title,
    /// 3 — the field: walking, fighting, number keys live.
    Field,
    /// 4 — a scenario: dialogue, cut-scenes.
    Script,
    /// 5 — the menu window.
    Menu,
    /// 8 — the game's own minimap window (menu 「8. 미니맵」 / `*`; `ce.a(II)Z` sends it RSK to
    /// close and `*`/LSK to toggle, S25 2026-10-23). Its red title band runs along the top of the
    /// screen.
    MapWindow,
    /// Any other value of `ed.d` (1 before the game starts, 7 the death's 「게임 오버」 before the
    /// title — probe and S9 2026-10-24, where 11 had been read off the key dispatch — 12 `ct`).
    Other(u8),
}

impl Scene {
    pub fn from_mode(mode: u8) -> Scene {
        match mode {
            2 => Scene::Title,
            3 => Scene::Field,
            4 => Scene::Script,
            5 => Scene::Menu,
            8 => Scene::MapWindow,
            m => Scene::Other(m),
        }
    }

    /// The player's own HUD — the status block, the EXP bar and the quick plates — is up: the
    /// field and the game's windows over play, as hero2's is in its menu mode (user 2026-10-26
    /// 「hero2·hero3 처럼」): the menu `bh` and all it opens, the game's minimap window, a message
    /// box over the field. Not the title or a game over (no character of the game's own), not a
    /// scenario — its choices and the shops it opens included, as hero2's cut-scene shows none.
    pub fn in_play(self) -> bool {
        matches!(self, Scene::Field | Scene::Menu | Scene::MapWindow)
    }
}

/// One frame's worth of what the HUD needs. Plain data: the UI thread keeps a copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct GuestView {
    pub scene: Scene,
    /// A message box (`ax`) is up: keys go to it, not to the field.
    pub popup: bool,
    /// The 「# 확인」 notice (`ed.f`) is up.
    pub notice: bool,
    /// One of the game's windows is open over a scenario (`eb`'s window: its choices `cf`, a shop
    /// an NPC opened — `crate::menu_touch::chain`): the scene is a window's, not the dialogue's.
    pub window: bool,
    /// The window with the keys answers only the soft keys (HZ-024 took their pictures away): the
    /// two-way message `cd` with `e:Z` set — the 창고 question `de` makes (`cd.a(II)Z` @11..52:
    /// LSK yes, RSK no, FIRE nothing). The host's OK sends LSK while it is up.
    pub lsk_ok: bool,
    /// The window with the keys has pages a pad's page buttons turn (`crate::menu_touch::Pager`: the
    /// shop's buying tabs and 계정창고's sides — the game's `1` / `3` — a bag's pages, the title's slot
    /// pages). The app sends [`GuestState::ask_page`] for them then, not their quick-slot keys.
    pub pager: bool,
}

impl GuestView {
    /// [`Scene::in_play`].
    pub fn in_play(&self) -> bool {
        self.scene.in_play()
    }

    /// The ring of number keys around OK: wherever the player's HUD is up ([`Self::in_play`]),
    /// and until the game says where it is. A plate sends its key to whatever has the keys, as
    /// hero2's do over its menus: in the field the game's skills and potions; in the menu `bh` a
    /// digit is its row (「1」 = 캐릭터, 「0」 = 시스템); a message box (`ab.b(II)V`: left/right only)
    /// and the minimap window (`ce.a(II)Z` @709: RSK, LSK, FIRE) do nothing with them.
    pub fn ring_keys(&self) -> bool {
        self.scene == Scene::Unknown || self.in_play()
    }

    /// `#` has a job: items in the field, 확인 on the notice.
    pub fn hash_key(&self) -> bool {
        self.ring_keys() || self.notice
    }
}

/// The player's numbers — Phase 3 (2026-10-22). What the game keeps in `ek.a:[I` (current) and
/// `ek.b:[I` (maximum), read through its own accessors `ek.e(B)I` / `ek.f(B)I` on the player
/// `ed.a:Lek;`, plus the level `ek.n:B`. The indices are the game's (javap 2026-10-22: the bottom
/// bar `ce` draws 0 red at 118 px, 2 as the thin line under it, 3 blue and 1 green at 40 px; the
/// character window `rpg/d` labels "HP" and "MP"; skills check `ek.c()` = `a[3]` for their cost):
/// **0 HP, 1 SP, 2 EXP (`b[2]` = the next level), 3 MP.** The game's bar shows these as bars with
/// no numbers but the level; the host HUD shows the numbers.
///
/// Phase 6-3 (2026-10-23): plus the UNSPENT POINTS — stat `ek.a:S` (the 캐릭터 window `dy.a(II)Z`
/// spends one with `ek.a -= 1; ek.h(row)`) and skill `ek.b:S` (the skill reset `p.i()` refunds
/// into it). The game's bar blinked an icon over itself for each that is above 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Stats {
    pub hp: i32,
    pub hp_max: i32,
    pub sp: i32,
    pub sp_max: i32,
    pub exp: i32,
    pub exp_next: i32,
    pub mp: i32,
    pub mp_max: i32,
    pub level: i32,
    pub stat_points: i32,
    pub skill_points: i32,
    /// HZ-022's guardian level, 0 = none (the EXP above is then into it).
    pub guardian: u16,
    /// The game's replay round `ed.h`: 0 노말 · 1 하드코어 · 2 헬 · 3 가디언 모드 (HZ-030).
    pub round: i8,
}

impl Stats {
    /// EXP towards the next level, in tenths of a percent, `0..=1000`.
    pub fn exp_permille(&self) -> i32 {
        if self.exp_next <= 0 {
            return 0;
        }
        (i64::from(self.exp.clamp(0, self.exp_next)) * 1000 / i64::from(self.exp_next)) as i32
    }

    /// The HUD's two lines, ASCII (the host font): the three pools, then the level and EXP.
    pub fn lines(&self) -> (String, String) {
        let p = self.exp_permille();
        let points = if self.stat_points > 0 || self.skill_points > 0 {
            format!("  PT {}/{}", self.stat_points, self.skill_points)
        } else {
            String::new()
        };
        (
            format!("HP {}/{}  SP {}/{}  MP {}/{}", self.hp, self.hp_max, self.sp, self.sp_max, self.mp, self.mp_max),
            format!(
                "LV {}{}  EXP {}.{}%{points}{}",
                self.level,
                if self.guardian > 0 { format!(" GL {}", self.guardian) } else { String::new() },
                p / 10,
                p % 10,
                if self.round > 0 { format!("  R{}", self.round) } else { String::new() }
            ),
        )
    }
}

/// Where a quick-slot icon comes from — the game's own pictures, as its bar draws them (Phase 6-2,
/// javap `ce.f()` / `ce.a(III)` / `ba.a(dd,IIZZ)`, 2026-10-23). The icon cache is keyed on this.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IconKey {
    /// A character skill (keys `1` `3`): 12×12 cell `cell` (= skill id − 4) of the class's strip
    /// `rpg/e.f` (`/img/sk`, picked by the class byte `ed.b:B`).
    CharSkill { class: i8, cell: i8 },
    /// A spirit skill (keys `7` `9`): `rpg/e.g[id]` (`/img/si`).
    SpiritSkill(i8),
    /// An item: `rpg/e.k[category + (category == 20 ? id : 0)]` — potions are category 20.
    Item(i16),
    /// The unspent-points icon the bar blinked: `rpg/e.c[0]` stat points, `[1]` skill points.
    Points(i8),
    /// The status block's portrait (Phase 6-6, `crate::portrait`): composed, not read, so never
    /// cached here; the number is a checksum of its pixels.
    Portrait(u32),
    /// A line of text in the game's own font (Phase 7-3: the world map's place names), drawn by
    /// the hook from EUC-KR bytes the host asked for: index `n` of that list. White, the alpha the
    /// ink's coverage.
    Label(u8),
    /// A map's own title in the game's font (Phase 7-5): `g.b()[B` of map `n`, the name the game's
    /// minimap window letters in its red band (「고아원-2층A」). White, alpha = coverage.
    MapTitle(u8),
    /// 「부활의 구」's picture (HZ-016, made by the host — the game has none): the buff band's
    /// mark for the untouchable seconds after a revive.
    Orb,
    /// A line of the host's own UI in the game's font (B1, 2026-10-26: `hz_android::host_text`),
    /// the host's number for it. White, alpha = coverage.
    Text(u16),
    /// A pickup notice the game made (HZ-025: its 120×16 box, 「귀환서」 or 「N금」), the `n`th of
    /// the run — each is its own picture, opaque.
    Pickup(u32),
}

/// An icon's pixels as the game's `Image.getRGB` gives them: `w × h` ARGB, alpha 0 = transparent.
/// Two icons are equal when they come from the same place.
#[derive(Clone, Debug)]
pub struct Icon {
    pub key: IconKey,
    pub w: u32,
    pub h: u32,
    pub px: Arc<[u32]>,
}

impl PartialEq for Icon {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for Icon {}

/// The plates the HUD shows (HZ-058, hero2's seven — `crate::quickpage`): a page of them. With the
/// patch off, the game's six slots in key order `1 3 7 9 0 #` on the first six and the seventh empty.
pub const QUICK_SLOTS: usize = crate::quickpage::PLATES;
/// The potion cooldown's full length: `ce.a(ZII)Z` draws `ed.a[4] * 12 / 10` of the box.
pub const POTION_COOLDOWN: i32 = 10;

/// One quick slot as the game's bar shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QuickSlot {
    /// `None` = empty (or the picture not loaded).
    pub icon: Option<Icon>,
    /// A potion slot's stock in the bag; 0 for a skill.
    pub count: i32,
    /// Frames of cooldown left, out of `cooldown_max` (`ed.a:[I` / `ed.b:[I`).
    pub cooldown: i32,
    pub cooldown_max: i32,
    /// A potion's plate: its stock is drawn.
    pub item: bool,
}

impl QuickSlot {
    /// How much of the cooldown is still to run, `0.0` (ready) ..= `1.0`.
    pub fn cooldown_left(&self) -> f64 {
        if self.cooldown <= 0 || self.cooldown_max <= 0 {
            return 0.0;
        }
        (f64::from(self.cooldown) / f64::from(self.cooldown_max)).min(1.0)
    }
}

/// What the game's bar showed besides the gauges: the six quick slots (Phase 6-2) and the two
/// unspent-point icons (6-3, shown by the status block while [`Stats`] has points) — and the timed
/// states the game shows nowhere, for the buff band (hero2's).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QuickBar {
    /// The plates of the page shown.
    pub slots: [QuickSlot; QUICK_SLOTS],
    /// HZ-058: the page shown (0-based, of `quickpage::PAGES`), and whether the player is choosing the
    /// plate a skill or potion goes on (its window's 「퀵슬롯등록」).
    pub page: u8,
    pub picking: bool,
    /// `[stat, skill]`: the game's pictures (`IconKey::Points`).
    pub points: [Option<Icon>; 2],
    /// The timed states running now, in [`BUFFS`] order.
    pub buffs: Vec<Buff>,
}

/// A timed state of the player (javap `ek.c()V`, 2026-10-25): what it is, its picture, the whole
/// seconds left (rounded up).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Buff {
    pub kind: BuffKind,
    pub icon: Option<Icon>,
    pub secs: u32,
}

/// The game's three timed states. Ticks are the game's frames, `rpg/c.b:I` ms each (1000 /
/// `ed.g`, the 게임속도 option's frame rate).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuffKind {
    /// `ek.b:I` ticks: battle style 5's skill 5 (`eo`, `ek.n(I)V` at its 10th frame) changes the
    /// hero's form until it runs out (`ek.K()V`). Its skill icon.
    Form,
    /// `ek.j:I` ticks: a spirit skill (1, 2, 5, 9 — `p`, `ek.o(I)V`), counted while the hero is not
    /// in state 10; `ek.L()V` at 0. That skill's icon (`p.a:B`).
    Spirit,
    /// `ek.a:J` ms: untouchable after the game's revive `ek.d()V` (5,000 — HZ-016's orb).
    Guard,
}

/// The band's order.
pub const BUFFS: [BuffKind; 3] = [BuffKind::Form, BuffKind::Spirit, BuffKind::Guard];

/// `ticks` of `tick_ms` each, as whole seconds rounded up (a state with a tick left shows 1).
pub fn buff_secs(ticks: i64, tick_ms: i64) -> u32 {
    if ticks <= 0 {
        return 0;
    }
    let ms = ticks.saturating_mul(tick_ms.max(1));
    ((ms + 999) / 1000).min(u32::MAX as i64) as u32
}

impl QuickBar {
    /// One line for the logs: `1:sk/4 3:- 7:si/2 9:- 0:it/21x5 #:-`, with `~n` for a cooldown.
    pub fn summary(&self) -> String {
        const KEYS: [&str; QUICK_SLOTS] = ["P1", "P2", "P3", "P4", "P5", "P6", "P7"];
        let head = if self.page > 0 || self.picking { format!("page {}{} ", self.page + 1, if self.picking { " picking" } else { "" }) } else { String::new() };
        head + &KEYS
            .iter()
            .zip(&self.slots)
            .map(|(k, s)| {
                let what = match s.icon.as_ref().map(|i| i.key) {
                    None => "-".to_string(),
                    Some(IconKey::CharSkill { cell, .. }) => format!("sk/{cell}"),
                    Some(IconKey::SpiritSkill(id)) => format!("si/{id}"),
                    Some(IconKey::Item(i)) => format!("it/{i}x{}", s.count),
                    Some(IconKey::Points(i)) => format!("pt/{i}"),
                    Some(IconKey::Portrait(_)) => "head".to_string(),
                    Some(IconKey::Label(i)) => format!("label/{i}"),
                    Some(IconKey::MapTitle(n)) => format!("title/{n}"),
                    Some(IconKey::Orb) => "orb".to_string(),
                    Some(IconKey::Text(n)) => format!("text/{n}"),
                    Some(IconKey::Pickup(n)) => format!("pickup/{n}"),
                };
                let cd = if s.cooldown > 0 { format!("~{}", s.cooldown) } else { String::new() };
                format!("{k}:{what}{cd}")
            })
            .collect::<Vec<_>>()
            .join(" ")
            + &self.buffs.iter().map(|b| format!(" | {:?} {}s", b.kind, b.secs)).collect::<String>()
    }
}

/// The minimap's tile classes (Phase 6-5): the values of `minimap_view.rs` (`MM_*` — hero2's file,
/// byte for byte, in `hz_android`, which checks at compile time that the two agree).
pub const MM_WALL: u8 = 0;
pub const MM_FLOOR: u8 = 1;
pub const MM_EXIT: u8 = 2;
pub const MM_CHEST: u8 = 3;

/// One map as the minimap draws it (Phase 6-5, javap `g` 2026-10-23). The game keeps an attribute
/// byte per tile in `g.a:[B` (`g.c:B` × `g.d:B` tiles of 16 px, row-major): `& 192` blocks
/// (`g.b(II)Z`), `& 63` is a trigger (`g.a(II)B`). Its own minimap (`g.i()`, the `*` window)
/// paints a trigger tile blue `0x397DFF` whatever else the byte says, so a trigger is an exit
/// here (probe, maps 0 4 44 46: the map edges and the doors), a blocked tile a wall, the rest
/// floor. A chest's own tile carries a trigger too — its opening event (map 46: 7 triggers, one
/// under the chest) — so a chest is [`MM_CHEST`] while the game draws it closed and its tile
/// without the trigger once opened, never an exit.
///
/// But most triggers lead nowhere: 107 in the 139 maps never warp to another map — shop counters,
/// inn beds, library shelves, one-tile event spots (2026-10-23 eighteenth session, `hz_maps`
/// parser over every trigger's scripts). The game's window paints them blue all the same, and the
/// world map's inset, which IS that window, keeps [`cells`](Self::cells); the field's panel is
/// hero2's, where a blue bar is a way to another map, and draws [`doors`](Self::doors).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapGrid {
    /// `g.b:B`, the map number.
    pub id: u8,
    pub w: u8,
    pub h: u8,
    /// `w × h` classes, row-major — the game window's.
    pub cells: Arc<[u8]>,
    /// The triggers whose scripts warp to another map ([`crate::mapfile::MapFile::exits`]), once
    /// the map's file has been read ([`MapGrid::with_leaving`]).
    pub leaving: Option<Arc<[u8]>>,
    /// `cells` with only those triggers as exits — any other is the floor or wall it stands on.
    pub door_cells: Option<Arc<[u8]>>,
    /// The title the game's minimap window gives the map, lettered by the hook in the game's font
    /// ([`IconKey::MapTitle`]); `None` until then, and for a map read from its file.
    pub title: Option<Icon>,
    /// Whether the map's file places monsters ([`crate::mapfile::MapFile::fights`]) — where the
    /// game's target panel can be drawn; `None` until the file has been read.
    pub fights: Option<bool>,
}

impl MapGrid {
    /// The class of one attribute byte.
    pub fn class_of(attr: i8) -> u8 {
        let a = attr as u8;
        if a & 63 != 0 {
            MM_EXIT
        } else if a & 192 != 0 {
            MM_WALL
        } else {
            MM_FLOOR
        }
    }

    /// The grid from the attribute layer and the chests: `(x, y, closed)`. `None` for an empty map
    /// or a layer that is not `w × h`; a chest off the map is ignored.
    pub fn new(id: u8, w: u8, h: u8, attrs: &[i8], chests: &[(u8, u8, bool)]) -> Option<MapGrid> {
        let n = usize::from(w) * usize::from(h);
        if n == 0 || attrs.len() != n {
            return None;
        }
        let mut cells: Vec<u8> = attrs.iter().map(|&a| Self::class_of(a)).collect();
        for &(x, y, closed) in chests {
            if x < w && y < h {
                let i = usize::from(y) * usize::from(w) + usize::from(x);
                cells[i] = if closed { MM_CHEST } else { Self::class_of(attrs[i] & !63) };
            }
        }
        Some(MapGrid { id, w, h, cells: cells.into(), leaving: None, door_cells: None, title: None, fights: None })
    }

    /// Keep as exits only the triggers in `leaving` — the attribute layer `attrs` is the one the
    /// grid was made from. A chest stays a chest.
    pub fn with_leaving(mut self, attrs: &[i8], leaving: Arc<[u8]>) -> MapGrid {
        if attrs.len() == self.cells.len() {
            let doors: Vec<u8> = self
                .cells
                .iter()
                .zip(attrs)
                .map(|(&c, &a)| if c == MM_EXIT && !leaving.contains(&(a as u8 & 63)) { Self::class_of(a & !63) } else { c })
                .collect();
            self.door_cells = Some(doors.into());
            self.leaving = Some(leaving);
        }
        self
    }

    /// The classes the field's panel draws: a blue bar is a way to another map. The game window's
    /// [`cells`](Self::cells) until the map's file has said which triggers leave it.
    pub fn doors(&self) -> &[u8] {
        self.door_cells.as_deref().unwrap_or(&self.cells)
    }

    /// A map straight from its file (Phase 7-5: the world map shows any place's map the way the
    /// game's own minimap window does): `g.a([B)V`'s order — kind, `ac` (2 B), two names (s8 length
    /// + bytes), the tile kinds (s8 count + ids), width, height, the tile layer, then the attribute
    /// layer this reads (`tools_re/hz_maps.py` parses all 139 to their ends). No chests: the game's
    /// window draws none. `None` if the bytes end early.
    pub fn from_map_file(id: u8, b: &[u8]) -> Option<MapGrid> {
        let mut p = 3usize;
        for _ in 0..3 {
            let n = *b.get(p)? as i8;
            p += 1 + n.max(0) as usize;
        }
        let (w, h) = (*b.get(p)?, *b.get(p + 1)?);
        let n = usize::from(w) * usize::from(h);
        p += 2 + n;
        let attrs: Vec<i8> = b.get(p..p + n)?.iter().map(|&a| a as i8).collect();
        MapGrid::new(id, w, h, &attrs, &[])
    }

    /// The jar path the game loads map `n` from (`rpg/e.g()`): `/m/m/NN.m`, 100 and up
    /// `/m/m2/<'a' + (n − 100) / 10><n % 10>.m`.
    pub fn path(n: u8) -> String {
        if n < 100 { format!("/m/m/{n:02}.m") } else { format!("/m/m2/{}{}.m", char::from(b'a' + (n - 100) / 10), n % 10) }
    }

    /// `(floor, exits, chests)` tile counts, for the logs.
    pub fn census(&self) -> (usize, usize, usize) {
        let n = |c: u8| self.cells.iter().filter(|&&x| x == c).count();
        (n(MM_FLOOR), n(MM_EXIT), n(MM_CHEST))
    }
}

/// What moves on the map, in TILES — fractional, the centre of the entity's tile, as
/// `minimap_view` takes them. An entity's `er.h:S` / `er.i:S` are pixels whose `>> 4` is its tile
/// (`er.E()`), so a centre is `px / 16 + 0.5`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MapMarks {
    /// The player, `ed.a:Lek;`.
    pub player: Option<(f32, f32)>,
    /// Every live monster on the map's draw list.
    pub mobs: Vec<(f32, f32)>,
}

/// A pixel position as a minimap mark (see [`MapMarks`]).
pub fn mark_of(px: i16, py: i16) -> (f32, f32) {
    (f32::from(px) / 16.0 + 0.5, f32::from(py) / 16.0 + 0.5)
}

/// Not reported yet.
const NO_MODE: u8 = 0xff;
/// Reported: the game screen does not exist yet.
const LOADING: u8 = 0xfe;

/// Written by the hook, read by the worker after each tick. Both run on the worker thread (the
/// JVM's threads are tasks on its executor), so `Relaxed` is enough; the atomics are for `Sync`.
#[derive(Debug)]
pub struct GuestState {
    mode: AtomicU8,
    popup: AtomicBool,
    notice: AtomicBool,
    /// [`GuestView::window`], [`GuestView::lsk_ok`] and [`GuestView::pager`], from the open windows.
    window: AtomicBool,
    lsk_ok: AtomicBool,
    pager: AtomicBool,
    frames: AtomicU64,
    /// [`Stats`] as eleven cells, and whether any were reported yet.
    stats: [AtomicI32; 13],
    has_stats: AtomicBool,
    /// The quick slots as last read (field only), and every icon read so far — `None` for a key
    /// whose picture could not be read, so it is not tried again every frame.
    quick: Mutex<Option<QuickBar>>,
    icons: Mutex<HashMap<IconKey, Option<Icon>>>,
    /// The minimap (Phase 6-5, field only): the map as last read, and its marks.
    map: Mutex<Option<MapGrid>>,
    marks: Mutex<MapMarks>,
    /// The status block's portrait (Phase 6-6, field only), as last composed.
    portrait: Mutex<Option<Icon>>,
    /// HZ-014: how many guest px the game's target-monster panel moves left, set by the host so it
    /// clears the minimap. 0 = where the game puts it.
    target_shift: AtomicI32,
    /// HZ-054: how many guest px lower the panel goes — under the host's minimap in portrait. 0 =
    /// where the game puts it.
    target_drop: AtomicI32,
    /// HZ-059: the diagonal the host's arrows make (5..8, [`crate::input::dir`]; 0 = none).
    walk_diag: std::sync::atomic::AtomicI8,
    /// Phase 7-1: the SAVE tab asked for the game's quick save, and how the next frame answered
    /// ([`SaveAnswer`] as a byte, [`NO_ANSWER`] = none waiting).
    save_asked: AtomicBool,
    save_answer: AtomicU8,
    /// Phase 7-3: lines of EUC-KR text the host wants in the game's font (the world map's place
    /// names), and the pictures once the hook drew them.
    label_asks: Mutex<Option<Vec<Vec<u8>>>>,
    labels: Mutex<Option<Vec<Option<Icon>>>>,
    /// B1: the host's own lines (EUC-KR, its number for each) waiting for the game's font, and the
    /// pictures drawn since the worker last took them.
    text_asks: Mutex<std::collections::VecDeque<(u16, Vec<u8>)>>,
    texts: Mutex<Vec<(u16, Option<Icon>)>>,
    /// Phase 7-5: maps the host wants read from the jar (the world map's places), and the grids.
    map_asks: Mutex<Option<Vec<u8>>>,
    maps: Mutex<Option<Vec<Option<MapGrid>>>>,
    /// Phase 7-6: the maps whose guide tiles the host wants, the guidance as last computed and
    /// what it was computed for `(quest states, player's map)`, and the files read for it.
    guide_maps: Mutex<Vec<u8>>,
    guide: Mutex<Option<QuestGuide>>,
    guide_for: Mutex<Option<(Vec<u8>, u8)>>,
    mmi: Mutex<HashMap<u8, Option<Arc<[u8]>>>>,
    files: Mutex<HashMap<u8, Option<Arc<crate::mapfile::MapFile>>>>,
    /// A tap on the game's picture (guest px) for the game's own windows ([`crate::menu_touch`]),
    /// taken by the next drawn frame; and the open windows as last logged.
    tap: Mutex<Option<(i32, i32)>>,
    /// A pad's page press (−1 back, +1 on) for the window with the keys, taken by the next drawn frame.
    page: std::sync::atomic::AtomicI8,
    windows: Mutex<String>,
    /// The slot 이어하기 should start on (0-based, −1 = the game's own choice): the one played
    /// last, which the host reads from the save files at boot.
    preferred_slot: std::sync::atomic::AtomicI8,
    /// HZ-022: the guardian level last worked out (0 = none: below 99, or the patch off).
    guardian: std::sync::atomic::AtomicU16,
    /// HZ-020: the slot page the game's three slots are now (0 = slots 1..3). Turned only by the
    /// slot window at the title ([`crate::slots`]), so a loaded slot's page never changes under it.
    slot_page: AtomicU8,
    /// HZ-025: the game's pickup notices drawn for the first time since the worker last took them,
    /// and how many there have been (each picture's [`IconKey::Pickup`] number).
    pickups: Mutex<Vec<Icon>>,
    pickup_count: std::sync::atomic::AtomicU32,
    /// The key the canvas holds as of the last drawn frame (`f.c:I`, the MIDP code; 0 = none) —
    /// the host's key → drawn latency gauge reads when it changes.
    key: AtomicI32,
    /// The JVM, lent by the blit hook so the worker can collect garbage between ticks
    /// ([`GuestState::collect_garbage`]).
    jvm: Mutex<Option<LentJvm>>,
    /// The `hz_patches` ids chosen for this boot ([`GuestState::set_patches`]): a hook that several
    /// patches share answers for the ones chosen (HZ-031's pieces through HZ-016's / HZ-023's record hook).
    patches: Mutex<Vec<&'static str>>,
    /// HZ-058: what the plates were asked (a plate pressed, the page turned), taken by the next drawn
    /// frame in order; and the field's ticks (HZ-006's hook), the cooldowns' clock.
    plate_asks: Mutex<std::collections::VecDeque<PlateAsk>>,
    field_ticks: AtomicU64,
    /// HZ-061: a boss's story waiting for its countdown, and the seconds the host shows (0 = none).
    story: Mutex<Option<crate::story::Wait>>,
    story_seconds: AtomicU8,
    /// HZ-065: quest rewards waiting for the next save ([`crate::reward`]), what the host is to say
    /// about them, and — while a save's put-away arms it — the storage payload the game hands its
    /// writer (`dz.i()V` → `rpg/b.b([B)Z`) for the read-back.
    rewards: Mutex<crate::reward::Pending>,
    reward_notices: Mutex<Vec<crate::reward::Notice>>,
    storage_capture: AtomicBool,
    storage_payload: Mutex<Option<Vec<u8>>>,
}

/// A request to the plates (HZ-058, [`GuestState::ask_plate`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlateAsk {
    /// Plate `n` (0-based) of the page shown was pressed: used, or — while a skill window's
    /// 「퀵슬롯등록」 waits — chosen.
    Press(u8),
    /// The other page.
    Page,
}

/// The JVM handle the hook lends (`Jvm` itself has no `Debug`).
struct LentJvm(jvm::Jvm);

impl std::fmt::Debug for LentJvm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Jvm")
    }
}

/// The game's own quest guidance as the world map shows it (Phase 7-6): the quest under way, the
/// map its guide tiles lead to, and the guide tile in each map asked about.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QuestGuide {
    /// The quest under way (1..=35, `crate::mapfile::guided_quest`), `None` when none is.
    pub quest: Option<u8>,
    /// Where following the guide tiles ends (`crate::mapfile::guide_target`): the first map with no
    /// guide tile for the quest — the NPC or spot itself. From the player's map, or from the first
    /// map that guides the quest when the player's does not. `None` without a quest or a trail.
    pub target: Option<u8>,
    /// `(map, x, y)`: the guide tile (`/mmi/<map>`, the game's yellow 「!」) in the player's map and
    /// in every map of [`GuestState::ask_guide_maps`].
    pub tiles: Vec<(u8, u8, u8)>,
}

/// How the hook answered the SAVE tab (Phase 7-1; since 2026-10-29 the tab saves at once — the
/// user: *"저장하시겠습니까? 안뜨고 바로 저장되게"*).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveAnswer {
    /// The game ran its own save `ed.k()`, what its 「예」 to 「저장하시겠습니까?」 runs. Not proof the
    /// slot was written: the game swallows a failed write — the host's store counts those
    /// (`hz_android::database`, `save_failures`).
    Saved,
    /// The map does not allow it (`g.e:Z` false — the game's 「저장할 수 없는 지역입니다.」).
    NotHere,
    /// Not now: not the bare field, or the player is not standing or walking (`ce.a()Z`, the
    /// game's own test for the menu keys — attacking, hit, a skill playing).
    Busy,
}

const NO_ANSWER: u8 = 0;

impl Default for GuestState {
    fn default() -> Self {
        Self::new()
    }
}

impl GuestState {
    /// The patches this boot applies (`hz_patches::Patches::chosen`), set once before the guest runs.
    pub fn set_patches(&self, ids: impl IntoIterator<Item = &'static str>) {
        *self.patches.lock().unwrap() = ids.into_iter().collect();
    }

    /// Whether patch `id` was chosen for this boot.
    pub fn patch_on(&self, id: &str) -> bool {
        self.patches.lock().unwrap().contains(&id)
    }

    pub fn new() -> GuestState {
        GuestState {
            mode: AtomicU8::new(NO_MODE),
            popup: AtomicBool::new(false),
            notice: AtomicBool::new(false),
            window: AtomicBool::new(false),
            lsk_ok: AtomicBool::new(false),
            pager: AtomicBool::new(false),
            frames: AtomicU64::new(0),
            stats: Default::default(),
            has_stats: AtomicBool::new(false),
            quick: Mutex::new(None),
            icons: Mutex::new(HashMap::new()),
            map: Mutex::new(None),
            marks: Mutex::new(MapMarks::default()),
            portrait: Mutex::new(None),
            target_shift: AtomicI32::new(0),
            target_drop: AtomicI32::new(0),
            walk_diag: std::sync::atomic::AtomicI8::new(0),
            save_asked: AtomicBool::new(false),
            save_answer: AtomicU8::new(NO_ANSWER),
            label_asks: Mutex::new(None),
            labels: Mutex::new(None),
            text_asks: Mutex::new(std::collections::VecDeque::new()),
            texts: Mutex::new(Vec::new()),
            map_asks: Mutex::new(None),
            maps: Mutex::new(None),
            guide_maps: Mutex::new(Vec::new()),
            guide: Mutex::new(None),
            guide_for: Mutex::new(None),
            mmi: Mutex::new(HashMap::new()),
            files: Mutex::new(HashMap::new()),
            tap: Mutex::new(None),
            page: std::sync::atomic::AtomicI8::new(0),
            windows: Mutex::new(String::new()),
            preferred_slot: std::sync::atomic::AtomicI8::new(-1),
            slot_page: AtomicU8::new(0),
            pickups: Mutex::new(Vec::new()),
            plate_asks: Mutex::new(std::collections::VecDeque::new()),
            field_ticks: AtomicU64::new(0),
            story: Mutex::new(None),
            story_seconds: AtomicU8::new(0),
            rewards: Mutex::new(crate::reward::Pending::default()),
            reward_notices: Mutex::new(Vec::new()),
            storage_capture: AtomicBool::new(false),
            storage_payload: Mutex::new(None),
            pickup_count: std::sync::atomic::AtomicU32::new(0),
            key: AtomicI32::new(0),
            jvm: Mutex::new(None),
            patches: Mutex::new(Vec::new()),
            guardian: std::sync::atomic::AtomicU16::new(0),
        }
    }

    /// The slot played last (0-based), for the 이어하기 list to start on.
    /// The slot 이어하기 should start on (0-based, of all ten — HZ-020): before the game boots, so
    /// its first read of the slots is that slot's page.
    pub fn set_preferred_slot(&self, slot: Option<u8>) {
        self.preferred_slot.store(slot.map_or(-1, |s| s as i8), Ordering::Relaxed);
        if let Some((page, _)) = slot.and_then(crate::slots::page_of) {
            self.set_slot_page(page);
        }
    }

    pub fn preferred_slot(&self) -> Option<i8> {
        Some(self.preferred_slot.load(Ordering::Relaxed)).filter(|s| *s >= 0)
    }

    pub fn guardian(&self) -> u16 {
        self.guardian.load(Ordering::Relaxed)
    }

    pub fn report_guardian(&self, g: u16) {
        self.guardian.store(g, Ordering::Relaxed);
    }

    pub fn slot_page(&self) -> u8 {
        self.slot_page.load(Ordering::Relaxed)
    }

    pub fn set_slot_page(&self, page: u8) {
        self.slot_page.store(page % hz_patches::hooks::SLOT_PAGES, Ordering::Relaxed);
    }

    /// The player tapped the game's picture at guest `(x, y)`: the next drawn frame does what the
    /// keys would ([`crate::menu_touch::tap`]). A second tap before that frame replaces the first.
    pub fn ask_tap(&self, x: i32, y: i32) {
        *self.tap.lock().unwrap() = Some((x, y));
    }

    pub fn take_tap(&self) -> Option<(i32, i32)> {
        self.tap.lock().unwrap().take()
    }

    /// A pad's page button: `delta` −1 back, +1 on (`crate::menu_touch::page`, done by the next frame).
    pub fn ask_page(&self, delta: i8) {
        self.page.store(delta.signum(), Ordering::Relaxed);
    }

    pub fn take_page(&self) -> Option<i8> {
        Some(self.page.swap(0, Ordering::Relaxed)).filter(|d| *d != 0)
    }

    /// The open windows as last reported.
    pub fn windows_line(&self) -> String {
        self.windows.lock().unwrap().clone()
    }

    /// The open windows this frame, as a log line. Returns whether they changed.
    pub fn report_windows(&self, line: String) -> bool {
        let mut cur = self.windows.lock().unwrap();
        let changed = *cur != line;
        *cur = line;
        changed
    }

    /// The maps whose guide tiles the world map shows (its places' maps).
    pub fn ask_guide_maps(&self, maps: Vec<u8>) {
        *self.guide_maps.lock().unwrap() = maps;
    }

    pub fn guide_maps(&self) -> Vec<u8> {
        self.guide_maps.lock().unwrap().clone()
    }

    /// Whether the guidance must be computed again for these quest states and this map (and, if
    /// so, remembers them).
    pub fn guide_stale(&self, quests: &[u8], map: u8) -> bool {
        let mut last = self.guide_for.lock().unwrap();
        let stale = last.as_ref().is_none_or(|(q, m)| q != quests || *m != map);
        if stale {
            *last = Some((quests.to_vec(), map));
        }
        stale
    }

    /// The guidance this time. Returns whether it changed.
    pub fn report_guide(&self, g: QuestGuide) -> bool {
        let mut cur = self.guide.lock().unwrap();
        let changed = cur.as_ref() != Some(&g);
        *cur = Some(g);
        changed
    }

    /// `None` until the field has computed it once.
    pub fn guide(&self) -> Option<QuestGuide> {
        self.guide.lock().unwrap().clone()
    }

    /// A cached `/mmi/<map>` (`Some(None)` = known unreadable).
    pub fn cached_mmi(&self, map: u8) -> Option<Option<Arc<[u8]>>> {
        self.mmi.lock().unwrap().get(&map).cloned()
    }

    pub fn cache_mmi(&self, map: u8, b: Option<Arc<[u8]>>) {
        self.mmi.lock().unwrap().insert(map, b);
    }

    pub fn cached_file(&self, map: u8) -> Option<Option<Arc<crate::mapfile::MapFile>>> {
        self.files.lock().unwrap().get(&map).cloned()
    }

    pub fn cache_file(&self, map: u8, f: Option<Arc<crate::mapfile::MapFile>>) {
        self.files.lock().unwrap().insert(map, f);
    }

    /// The host wants these maps' grids (read once, next field frame, through the game's loader).
    pub fn ask_maps(&self, ids: Vec<u8>) {
        *self.map_asks.lock().unwrap() = Some(ids);
    }

    pub fn take_map_asks(&self) -> Option<Vec<u8>> {
        self.map_asks.lock().unwrap().take()
    }

    pub fn report_maps(&self, maps: Vec<Option<MapGrid>>) {
        *self.maps.lock().unwrap() = Some(maps);
    }

    /// The worker's side: the grids, in asked order, once.
    pub fn take_maps(&self) -> Option<Vec<Option<MapGrid>>> {
        self.maps.lock().unwrap().take()
    }

    /// The host wants these lines (EUC-KR) drawn in the game's font; the hook draws them on the
    /// next field frame, once (the font is the game's, loaded by then).
    pub fn ask_labels(&self, lines: Vec<Vec<u8>>) {
        *self.label_asks.lock().unwrap() = Some(lines);
    }

    pub fn take_label_asks(&self) -> Option<Vec<Vec<u8>>> {
        self.label_asks.lock().unwrap().take()
    }

    /// One picture per asked line, in order (`None` = could not be drawn).
    pub fn report_labels(&self, labels: Vec<Option<Icon>>) {
        *self.labels.lock().unwrap() = Some(labels);
    }

    /// The worker's side: the pictures, once (taken).
    pub fn take_labels(&self) -> Option<Vec<Option<Icon>>> {
        self.labels.lock().unwrap().take()
    }

    /// The host wants these lines of its own UI (EUC-KR, each with its number) in the game's font.
    /// Queued behind any asked before; the hook letters a few each drawn frame once the game's
    /// screen is up (the title on), so a long list costs no single frame much.
    pub fn ask_texts(&self, lines: impl IntoIterator<Item = (u16, Vec<u8>)>) {
        self.text_asks.lock().unwrap().extend(lines);
    }

    /// [`Self::ask_texts`] ahead of everything still queued, in their order: a host window's lines
    /// are on screen now (the PC's import report opened at the title waited ~4 s behind the boot's
    /// 142 lines, drawn as ASCII scraps meanwhile — 2026-10-30).
    pub fn ask_texts_first(&self, lines: impl IntoIterator<Item = (u16, Vec<u8>)>) {
        let mut q = self.text_asks.lock().unwrap();
        let lines: Vec<_> = lines.into_iter().collect();
        for line in lines.into_iter().rev() {
            q.push_front(line);
        }
    }

    /// The hook's side: up to `max` lines, oldest first.
    pub fn take_text_asks(&self, max: usize) -> Vec<(u16, Vec<u8>)> {
        let mut q = self.text_asks.lock().unwrap();
        let n = max.min(q.len());
        q.drain(..n).collect()
    }

    /// Lines lettered (`None` = could not be): kept until the worker takes them.
    pub fn report_texts(&self, done: Vec<(u16, Option<Icon>)>) {
        self.texts.lock().unwrap().extend(done);
    }

    /// The worker's side: everything lettered since the last call.
    pub fn take_texts(&self) -> Vec<(u16, Option<Icon>)> {
        std::mem::take(&mut *self.texts.lock().unwrap())
    }

    /// HZ-025, the hook's side: a pickup notice the game just drew for the first time, `w × h`
    /// ARGB. Numbered here, so two notices are never one icon.
    pub fn report_pickup(&self, w: u32, h: u32, px: Vec<u32>) {
        let n = self.pickup_count.fetch_add(1, Ordering::Relaxed);
        self.pickups.lock().unwrap().push(Icon { key: IconKey::Pickup(n), w, h, px: px.into() });
    }

    /// The worker's side: the pickup notices since the last call, oldest first.
    pub fn take_pickups(&self) -> Vec<Icon> {
        std::mem::take(&mut *self.pickups.lock().unwrap())
    }

    /// HZ-058: a plate pressed, the page turned — the next drawn frame acts on it.
    pub fn ask_plate(&self, ask: PlateAsk) {
        self.plate_asks.lock().unwrap().push_back(ask);
    }

    /// The hook's side: every ask waiting, oldest first.
    pub fn take_plate_asks(&self) -> Vec<PlateAsk> {
        self.plate_asks.lock().unwrap().drain(..).collect()
    }

    /// HZ-006's hook: one field tick.
    pub fn count_field_tick(&self) {
        self.field_ticks.fetch_add(1, Ordering::Relaxed);
    }

    pub fn field_ticks(&self) -> u64 {
        self.field_ticks.load(Ordering::Relaxed)
    }

    /// HZ-061: a boss died and its story waits ([`crate::story`]); the countdown shows from now.
    pub fn story_wait(&self, wait: crate::story::Wait) {
        self.story_seconds.store(wait.seconds(), Ordering::Relaxed);
        *self.story.lock().unwrap() = Some(wait);
    }

    /// Whether a story waits (the field tick reads the map only then).
    pub fn story_waiting(&self) -> bool {
        self.story.lock().unwrap().is_some()
    }

    /// HZ-061, a field tick on `map`: what the wait does ([`crate::story::Wait::tick`]); a start or a
    /// drop ends it. `None` = nothing waits.
    pub fn story_tick(&self, map: i8, may: bool) -> Option<crate::story::Step> {
        use crate::story::Step;
        let mut story = self.story.lock().unwrap();
        let step = story.as_mut()?.tick(map, may);
        match step {
            Step::Wait(s) => self.story_seconds.store(s, Ordering::Relaxed),
            Step::Go(_) | Step::Drop => {
                *story = None;
                self.story_seconds.store(0, Ordering::Relaxed);
            }
        }
        Some(step)
    }

    /// HZ-061: the game left the field for the title or its game over — a wait is gone with it.
    pub fn story_drop(&self) -> bool {
        self.story_seconds.store(0, Ordering::Relaxed);
        self.story.lock().unwrap().take().is_some()
    }

    /// The worker's side: the countdown's seconds to show (0 = none).
    pub fn story_seconds(&self) -> u8 {
        self.story_seconds.load(Ordering::Relaxed)
    }

    /// HZ-065: a quest reward the full bag could not take waits for the next save.
    pub fn reward_wait(&self, cat: i8, id: i8, count: i32) {
        self.rewards.lock().unwrap().add(cat, id, count);
        self.reward_notice(crate::reward::Notice::Waiting);
    }

    /// HZ-065, a save: every waiting reward, to put into the storage ([`GuestState::rewards_keep`] for
    /// what it cannot).
    pub fn rewards_take(&self) -> Vec<crate::reward::Reward> {
        self.rewards.lock().unwrap().take()
    }

    /// What a save could not put away, back as the save left them (`reward::after_save` marks the
    /// ones its progress record went past; a save whose progress was refused returns them as they were).
    pub fn rewards_keep(&self, rest: Vec<crate::reward::Reward>) {
        self.rewards.lock().unwrap().keep(rest);
    }

    /// HZ-065: the game was left without a save — the rewards no save went past are dropped.
    pub fn rewards_drop_unsaved(&self) -> usize {
        self.rewards.lock().unwrap().drop_unsaved()
    }

    pub fn reward_notice(&self, n: crate::reward::Notice) {
        self.reward_notices.lock().unwrap().push(n);
    }

    /// The worker's side: what to tell the player (cleared by reading).
    pub fn take_reward_notices(&self) -> Vec<crate::reward::Notice> {
        std::mem::take(&mut *self.reward_notices.lock().unwrap())
    }

    /// HZ-065, at boot: the rewards an earlier run's save went past ([`crate::reward::FILE`]).
    pub fn rewards_restore(&self, text: &str) -> usize {
        self.rewards.lock().unwrap().restore(text)
    }

    /// HZ-065: the file's text for the rewards a save went past (`None`: no file).
    pub fn rewards_saved_text(&self) -> Option<String> {
        self.rewards.lock().unwrap().saved_text()
    }

    /// HZ-065: a save's put-away arms the capture of the storage write's payload around the game's
    /// `dz.i()V`; the storage's other writes (the player moving things) are not copied.
    pub fn storage_capture_arm(&self, on: bool) {
        self.storage_capture.store(on, Ordering::Relaxed);
        if !on {
            self.storage_payload.lock().unwrap().take();
        }
    }

    pub fn storage_capture_armed(&self) -> bool {
        self.storage_capture.load(Ordering::Relaxed)
    }

    /// HZ-065: the payload the game's storage write was handed (cleared by reading).
    pub fn storage_payload_set(&self, bytes: Vec<u8>) {
        *self.storage_payload.lock().unwrap() = Some(bytes);
    }

    pub fn storage_payload_take(&self) -> Option<Vec<u8>> {
        self.storage_payload.lock().unwrap().take()
    }

    /// The SAVE tab was tapped: the next drawn frame saves the game, or says why not.
    pub fn ask_save(&self) {
        self.save_asked.store(true, Ordering::Relaxed);
    }

    /// The hook's side: whether an ask is waiting (cleared by reading).
    pub fn take_save_ask(&self) -> bool {
        self.save_asked.swap(false, Ordering::Relaxed)
    }

    pub fn answer_save(&self, answer: SaveAnswer) {
        let b = match answer {
            SaveAnswer::Saved => 1,
            SaveAnswer::Busy => 2,
            SaveAnswer::NotHere => 3,
        };
        self.save_answer.store(b, Ordering::Relaxed);
    }

    /// The worker's side: the answer to the last ask, once.
    pub fn take_save_answer(&self) -> Option<SaveAnswer> {
        match self.save_answer.swap(NO_ANSWER, Ordering::Relaxed) {
            1 => Some(SaveAnswer::Saved),
            2 => Some(SaveAnswer::Busy),
            3 => Some(SaveAnswer::NotHere),
            _ => None,
        }
    }

    /// The portrait this poll (`None` = the doll could not be composed). Returns whether it
    /// changed.
    pub fn report_portrait(&self, portrait: Option<Icon>) -> bool {
        let mut p = self.portrait.lock().unwrap();
        let changed = *p != portrait;
        *p = portrait;
        changed
    }

    /// `None` until the field has composed one.
    pub fn portrait(&self) -> Option<Icon> {
        self.portrait.lock().unwrap().clone()
    }

    /// HZ-014: the host's minimap covers `px` guest px of the target panel (0 = none).
    pub fn set_target_shift(&self, px: i32) {
        self.target_shift.store(px.max(0), Ordering::Relaxed);
    }

    pub fn target_shift(&self) -> i32 {
        self.target_shift.load(Ordering::Relaxed)
    }

    /// HZ-054: the panel goes `px` guest px lower (0 = where the game puts it).
    pub fn set_target_drop(&self, px: i32) {
        self.target_drop.store(px.max(0), Ordering::Relaxed);
    }

    pub fn target_drop(&self) -> i32 {
        self.target_drop.load(Ordering::Relaxed)
    }

    /// HZ-059: the arrows held make diagonal `d` (5..8; anything else = none). Set before the key that
    /// starts or turns the walk goes to the game — the walk hook reads it when the game takes the key.
    pub fn set_walk_diag(&self, d: i8) {
        self.walk_diag.store(if (5..=8).contains(&d) { d } else { 0 }, Ordering::Relaxed);
    }

    pub fn walk_diag(&self) -> i8 {
        self.walk_diag.load(Ordering::Relaxed)
    }

    /// The map this frame. Returns whether it changed (another map, a door's attribute, a chest
    /// opened).
    pub fn report_map(&self, grid: MapGrid) -> bool {
        let mut m = self.map.lock().unwrap();
        let changed = m.as_ref() != Some(&grid);
        if changed {
            *m = Some(grid);
        }
        changed
    }

    /// `None` until the field has been drawn once.
    pub fn map(&self) -> Option<MapGrid> {
        self.map.lock().unwrap().clone()
    }

    /// The marks this frame. Returns whether they changed.
    pub fn report_marks(&self, marks: MapMarks) -> bool {
        let mut m = self.marks.lock().unwrap();
        let changed = *m != marks;
        *m = marks;
        changed
    }

    pub fn marks(&self) -> MapMarks {
        self.marks.lock().unwrap().clone()
    }

    /// The quick slots this frame. Returns whether they changed.
    pub fn report_quick(&self, bar: QuickBar) -> bool {
        let mut q = self.quick.lock().unwrap();
        let changed = q.as_ref() != Some(&bar);
        *q = Some(bar);
        changed
    }

    /// `None` until the field has been drawn once.
    pub fn quick(&self) -> Option<QuickBar> {
        self.quick.lock().unwrap().clone()
    }

    /// A cached icon: `Some(None)` = known to be unreadable, `None` = not tried yet.
    pub fn cached_icon(&self, key: IconKey) -> Option<Option<Icon>> {
        self.icons.lock().unwrap().get(&key).cloned()
    }

    pub fn cache_icon(&self, key: IconKey, icon: Option<Icon>) {
        self.icons.lock().unwrap().insert(key, icon);
    }

    /// Every icon read so far (the probe dumps them).
    pub fn icons(&self) -> Vec<Icon> {
        self.icons.lock().unwrap().values().flatten().cloned().collect()
    }

    /// The player's numbers this frame. Returns whether they changed.
    pub fn report_stats(&self, s: Stats) -> bool {
        let before = self.stats();
        let cells = [
            s.hp,
            s.hp_max,
            s.sp,
            s.sp_max,
            s.exp,
            s.exp_next,
            s.mp,
            s.mp_max,
            s.level,
            s.stat_points,
            s.skill_points,
            i32::from(s.guardian),
            i32::from(s.round),
        ];
        for (cell, v) in self.stats.iter().zip(cells) {
            cell.store(v, Ordering::Relaxed);
        }
        self.has_stats.store(true, Ordering::Relaxed);
        before != Some(s)
    }

    /// `None` until the player exists (title, loading).
    pub fn stats(&self) -> Option<Stats> {
        if !self.has_stats.load(Ordering::Relaxed) {
            return None;
        }
        let c = |i: usize| self.stats[i].load(Ordering::Relaxed);
        Some(Stats {
            hp: c(0),
            hp_max: c(1),
            sp: c(2),
            sp_max: c(3),
            exp: c(4),
            exp_next: c(5),
            mp: c(6),
            mp_max: c(7),
            level: c(8),
            stat_points: c(9),
            skill_points: c(10),
            guardian: c(11).clamp(0, i32::from(u16::MAX)) as u16,
            round: c(12).clamp(0, i32::from(i8::MAX)) as i8,
        })
    }

    /// The blit hook lends the JVM (once).
    pub fn lend_jvm(&self, jvm: &jvm::Jvm) {
        let mut slot = self.jvm.lock().unwrap();
        if slot.is_none() {
            *slot = Some(LentJvm(jvm.clone()));
        }
    }

    /// A stop-the-world collection, for the worker to run between ticks — after the painted frame
    /// is out, during the game thread's sleep (the paint path's own collection is switched off by
    /// the host, `wie_util::stats::set_gc_in_paint`). The collector's roots are the JVM's own
    /// thread frames, the same whether a task or the worker calls it. `None` until the hook ran.
    pub fn collect_garbage(&self) -> Option<Result<usize, jvm::JavaError>> {
        let slot = self.jvm.lock().unwrap();
        slot.as_ref().map(|jvm| jvm.0.collect_garbage())
    }

    /// The key the canvas held when the last frame was drawn (`f.c:I`; 0 = none).
    pub fn report_key(&self, code: i32) {
        self.key.store(code, Ordering::Relaxed);
    }

    pub fn key(&self) -> i32 {
        self.key.load(Ordering::Relaxed)
    }

    /// One drawn frame: `mode` is `ed.d`, or `None` while the game screen does not exist.
    /// Returns whether the view changed.
    pub fn report(&self, mode: Option<u8>, popup: bool, notice: bool) -> bool {
        let before = self.view();
        self.mode.store(mode.unwrap_or(LOADING), Ordering::Relaxed);
        self.popup.store(popup, Ordering::Relaxed);
        self.notice.store(notice, Ordering::Relaxed);
        self.frames.fetch_add(1, Ordering::Relaxed);
        self.view() != before
    }

    /// The open windows' part of the view, read after [`GuestState::report`]: a window over a
    /// scenario, a top window that answers only the soft keys, and one with pages. True if the view
    /// changed.
    pub fn report_windows_view(&self, window: bool, lsk_ok: bool, pager: bool) -> bool {
        let before = self.view();
        self.window.store(window, Ordering::Relaxed);
        self.lsk_ok.store(lsk_ok, Ordering::Relaxed);
        self.pager.store(pager, Ordering::Relaxed);
        self.view() != before
    }

    pub fn view(&self) -> GuestView {
        let scene = match self.mode.load(Ordering::Relaxed) {
            NO_MODE => Scene::Unknown,
            LOADING => Scene::Loading,
            m => Scene::from_mode(m),
        };
        GuestView {
            scene,
            popup: self.popup.load(Ordering::Relaxed),
            notice: self.notice.load(Ordering::Relaxed),
            window: self.window.load(Ordering::Relaxed),
            lsk_ok: self.lsk_ok.load(Ordering::Relaxed),
            pager: self.pager.load(Ordering::Relaxed),
        }
    }

    /// Frames the hook has seen (the probe's gauge).
    pub fn frames(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buff_seconds_round_up_and_none_is_zero() {
        assert_eq!(buff_secs(0, 83), 0);
        assert_eq!(buff_secs(-3, 83), 0);
        assert_eq!(buff_secs(1, 83), 1, "a tick left still shows");
        assert_eq!(buff_secs(12, 83), 1, "996 ms");
        assert_eq!(buff_secs(13, 83), 2);
        assert_eq!(buff_secs(5_000, 1), 5, "the revive's 5,000 ms");
        assert_eq!(buff_secs(4_001, 1), 5);
    }

    /// B1: the host's lines queue, the hook takes a few a frame in order, the worker takes what was
    /// lettered since it last looked.
    #[test]
    fn host_texts_queue_in_order_and_are_taken_once() {
        let st = GuestState::new();
        st.ask_texts((0..20u16).map(|i| (i, vec![b'A'])));
        assert_eq!(st.take_text_asks(8).iter().map(|t| t.0).collect::<Vec<_>>(), (0..8).collect::<Vec<_>>());
        st.ask_texts([(99, vec![b'B'])]);
        st.ask_texts_first([(200, vec![b'C']), (201, vec![b'D'])]);
        assert_eq!(st.take_text_asks(3).iter().map(|t| t.0).collect::<Vec<_>>(), vec![200, 201, 8], "a window's lines first, in order");
        assert_eq!(st.take_text_asks(100).len(), 12, "the rest, then the later ask");
        assert!(st.take_text_asks(8).is_empty());
        st.report_texts(vec![(3, None)]);
        st.report_texts(vec![(4, None)]);
        assert_eq!(st.take_texts().len(), 2);
        assert!(st.take_texts().is_empty(), "taken once");
    }

    #[test]
    fn modes_are_the_games_switch() {
        assert_eq!(Scene::from_mode(2), Scene::Title);
        assert_eq!(Scene::from_mode(3), Scene::Field);
        assert_eq!(Scene::from_mode(4), Scene::Script);
        assert_eq!(Scene::from_mode(5), Scene::Menu);
        assert_eq!(Scene::from_mode(1), Scene::Other(1), "before the game starts");
        assert_eq!(Scene::from_mode(8), Scene::MapWindow, "the minimap window, not a yes/no (S25 2026-10-23)");
    }

    #[test]
    fn the_ring_is_for_play_and_for_the_unknown() {
        let v = |scene, popup, notice| GuestView { scene, popup, notice, ..Default::default() };
        assert!(v(Scene::Unknown, false, false).ring_keys(), "patch off: as before");
        assert!(!v(Scene::Unknown, false, false).in_play(), "but no player's HUD");
        // hero2's menu mode (A7): the field, the menu and what it opens, the minimap window, a
        // message box over the field.
        for (s, popup) in [(Scene::Field, false), (Scene::Field, true), (Scene::Menu, false), (Scene::Menu, true), (Scene::MapWindow, false)] {
            assert!(v(s, popup, false).in_play() && v(s, popup, false).ring_keys() && v(s, popup, false).hash_key(), "{s:?} {popup}");
        }
        for s in [Scene::Loading, Scene::Title, Scene::Script, Scene::Other(7)] {
            assert!(!v(s, false, false).in_play(), "{s:?}");
            assert!(!v(s, false, false).ring_keys(), "{s:?}");
            assert!(!v(s, false, false).hash_key(), "{s:?}");
            assert!(v(s, false, true).hash_key(), "{s:?}: # dismisses the notice");
        }
        let choices = GuestView { scene: Scene::Script, window: true, ..Default::default() };
        assert!(!choices.in_play(), "a scenario's window: hero2's cut-scene choices show no HUD");
    }

    #[test]
    fn stats_round_trip_and_the_lines_read_like_the_game() {
        let st = GuestState::new();
        assert_eq!(st.stats(), None, "no player yet");
        let s = Stats { hp: 120, hp_max: 150, sp: 30, sp_max: 40, exp: 423, exp_next: 1000, mp: 5, mp_max: 20, level: 12, ..Default::default() };
        assert!(st.report_stats(s));
        assert!(!st.report_stats(s), "the same numbers again");
        assert_eq!(st.stats(), Some(s));
        assert_eq!(s.lines().0, "HP 120/150  SP 30/40  MP 5/20");
        assert_eq!(s.lines().1, "LV 12  EXP 42.3%");
        assert_eq!(Stats { stat_points: 3, ..s }.lines().1, "LV 12  EXP 42.3%  PT 3/0", "unspent points when there are any");
        assert!(st.report_stats(Stats { skill_points: 1, ..s }), "a point is a change");
        assert_eq!(Stats { exp: 7, exp_next: 0, ..s }.exp_permille(), 0, "no next level: nothing to show");
        assert_eq!(Stats { exp: 5000, exp_next: 1000, ..s }.exp_permille(), 1000, "clamped");
        assert_eq!(Stats { exp: -3, exp_next: 1000, ..s }.exp_permille(), 0);
    }

    #[test]
    fn quick_slots_compare_by_icon_source_and_numbers() {
        let st = GuestState::new();
        assert_eq!(st.quick(), None, "nothing until the field");
        let icon = |key| Icon { key, w: 1, h: 1, px: Arc::from(vec![0xff00_0000u32]) };
        let mut bar = QuickBar::default();
        bar.slots[0] = QuickSlot { icon: Some(icon(IconKey::CharSkill { class: 1, cell: 0 })), count: 0, cooldown: 0, cooldown_max: 40, item: false };
        bar.slots[4] = QuickSlot { icon: Some(icon(IconKey::Item(21))), count: 5, cooldown: 3, cooldown_max: POTION_COOLDOWN, item: true };
        assert!(st.report_quick(bar.clone()));
        let mut same = bar.clone();
        same.slots[0].icon = Some(Icon { px: Arc::from(vec![1u32; 4]), w: 2, h: 2, ..icon(IconKey::CharSkill { class: 1, cell: 0 }) });
        assert!(!st.report_quick(same), "the same source is the same icon");
        let mut ticked = bar.clone();
        ticked.slots[4].cooldown = 2;
        assert!(st.report_quick(ticked), "a cooldown tick is a change");
        assert_eq!(bar.summary(), "P1:sk/0 P2:- P3:- P4:- P5:it/21x5~3 P6:- P7:-");
        let mut picking = bar.clone();
        (picking.page, picking.picking) = (1, true);
        assert!(picking.summary().starts_with("page 2 picking P1:sk/0"), "{}", picking.summary());
        let mut guarded = bar.clone();
        guarded.buffs = vec![Buff { kind: BuffKind::Guard, icon: None, secs: 5 }];
        assert_eq!(guarded.summary(), "P1:sk/0 P2:- P3:- P4:- P5:it/21x5~3 P6:- P7:- | Guard 5s");
        assert!(st.report_quick(guarded), "a buff is a change");
        assert!((bar.slots[4].cooldown_left() - 0.3).abs() < 1e-9);
        assert_eq!(bar.slots[0].cooldown_left(), 0.0);
        assert_eq!(QuickSlot { cooldown: 99, cooldown_max: 10, ..Default::default() }.cooldown_left(), 1.0);
        assert_eq!(st.cached_icon(IconKey::SpiritSkill(2)), None);
        st.cache_icon(IconKey::SpiritSkill(2), None);
        assert_eq!(st.cached_icon(IconKey::SpiritSkill(2)), Some(None), "unreadable is remembered");
    }

    /// The field's panel draws only the triggers that leave the map as exits: a shop counter (on a
    /// blocked tile) is a wall there, an event spot floor; the game window's classes stay.
    #[test]
    fn only_a_trigger_that_leaves_is_a_door() {
        let attrs = [3, -128 | 5, 6, 0, -128 | 4, 64];
        let g = MapGrid::new(7, 3, 2, &attrs, &[(1, 1, true)]).unwrap();
        assert_eq!(g.doors(), &g.cells[..], "until the file has been read: the game's");
        let d = g.clone().with_leaving(&attrs, Arc::from(vec![3u8]));
        assert_eq!(d.cells, g.cells, "the game window's classes stay");
        assert_eq!(d.doors(), &[MM_EXIT, MM_WALL, MM_FLOOR, MM_FLOOR, MM_CHEST, MM_WALL]);
        assert_eq!(d.leaving.as_deref(), Some(&[3u8][..]));
        assert_eq!(g.clone().with_leaving(&attrs[..3], Arc::from(vec![3u8])), g, "another layer: left alone");
    }

    #[test]
    fn the_map_reads_like_the_games_own_minimap() {
        // `g.i()`: a trigger is blue whatever its block bits; 64 and 128 each block (`& 192`).
        assert_eq!(MapGrid::class_of(0), MM_FLOOR);
        assert_eq!(MapGrid::class_of(64), MM_WALL);
        assert_eq!(MapGrid::class_of(-128), MM_WALL, "bit 7 is the sign bit of a Java byte");
        assert_eq!(MapGrid::class_of(-64), MM_WALL);
        assert_eq!(MapGrid::class_of(5), MM_EXIT);
        assert_eq!(MapGrid::class_of(-128 | 5), MM_EXIT, "a blocked trigger is still a trigger");
        // A chest stands on a blocked tile that carries its opening event (`-128 | 4`).
        let attrs = [0, 64, 3, 0, -128 | 4, -128];
        let g = MapGrid::new(7, 3, 2, &attrs, &[(1, 1, true), (9, 0, true)]).unwrap();
        assert_eq!(&*g.cells, &[MM_FLOOR, MM_WALL, MM_EXIT, MM_FLOOR, MM_CHEST, MM_WALL], "a chest off the map is ignored");
        assert_eq!(g.census(), (2, 1, 1));
        assert_eq!(MapGrid::new(7, 3, 3, &attrs, &[]), None, "not w × h");
        assert_eq!(MapGrid::new(7, 0, 0, &[], &[]), None, "no map");
        let opened = MapGrid::new(7, 3, 2, &attrs, &[(1, 1, false)]).unwrap();
        assert_eq!(opened.cells[4], MM_WALL, "an opened chest's event is not an exit");
        assert_eq!(MapGrid::new(7, 3, 2, &attrs, &[]).unwrap().cells[4], MM_EXIT, "the bare trigger, as the game paints it");

        let st = GuestState::new();
        assert_eq!(st.map(), None);
        assert!(st.report_map(g.clone()));
        assert!(!st.report_map(g.clone()), "the same map again");
        assert!(st.report_map(opened), "a chest opened");

        // A mark is the centre of the entity's tile: `er.E()` makes the tile `px >> 4`.
        assert_eq!(mark_of(0, 0), (0.5, 0.5));
        assert_eq!(mark_of(16 * 12, 16 * 3 + 8), (12.5, 4.0), "half-way to the next tile");
        let m = MapMarks { player: Some(mark_of(32, 48)), mobs: vec![mark_of(64, 64)] };
        assert!(st.report_marks(m.clone()));
        assert!(!st.report_marks(m.clone()));
        assert_eq!(st.marks(), m);
    }

    #[test]
    fn reports_say_when_the_view_changed() {
        let st = GuestState::new();
        assert_eq!(st.view(), GuestView::default());
        assert!(st.report(None, false, false), "unknown -> loading");
        assert_eq!(st.view().scene, Scene::Loading);
        assert!(st.report(Some(3), false, false));
        assert!(!st.report(Some(3), false, false), "same frame again");
        assert!(st.report(Some(3), true, false), "a popup is a change");
        assert_eq!(st.frames(), 4);
        assert_eq!(
            st.view(),
            GuestView {
                scene: Scene::Field,
                popup: true,
                notice: false,
                ..Default::default()
            }
        );
    }

    /// A map file's grid: the layers after the two names and the tile kinds, the paths the game
    /// loads from.
    #[test]
    fn a_map_file_reads_like_the_games_loader() {
        // kind, ac, name "ab", no second name, 2 tile kinds, 3 × 2, tiles, attributes.
        let mut file = vec![9, 0, 0x1a, 2, b'a', b'b', 0, 2, 0, 1, 3, 2];
        file.extend([0u8; 6]);
        file.extend([0x80u8, 0, 2, 0, 0, 0x40]);
        file.extend([7, 7, 7]); // what follows (objects, scripts) is not read
        let g = MapGrid::from_map_file(44, &file).unwrap();
        assert_eq!((g.id, g.w, g.h), (44, 3, 2));
        assert_eq!(&g.cells[..], &[MM_WALL, MM_FLOOR, MM_EXIT, MM_FLOOR, MM_FLOOR, MM_WALL]);
        assert!(MapGrid::from_map_file(44, &file[..20]).is_none(), "short");
        assert_eq!((MapGrid::path(0), MapGrid::path(44), MapGrid::path(100), MapGrid::path(136)), ("/m/m/00.m".into(), "/m/m/44.m".into(), "/m/m2/a0.m".into(), "/m/m2/d6.m".into()));
    }

    /// The SAVE tab's ask is taken by one frame, and its answer by one worker poll.
    #[test]
    fn a_save_ask_is_answered_once() {
        let st = GuestState::new();
        assert!(!st.take_save_ask() && st.take_save_answer().is_none());
        st.ask_save();
        st.ask_save();
        assert!(st.take_save_ask(), "two taps before a frame are one ask");
        assert!(!st.take_save_ask());
        st.answer_save(SaveAnswer::Busy);
        assert_eq!(st.take_save_answer(), Some(SaveAnswer::Busy));
        assert_eq!(st.take_save_answer(), None);
        st.answer_save(SaveAnswer::Saved);
        assert_eq!(st.take_save_answer(), Some(SaveAnswer::Saved));
        st.answer_save(SaveAnswer::NotHere);
        assert_eq!(st.take_save_answer(), Some(SaveAnswer::NotHere));
    }
}
