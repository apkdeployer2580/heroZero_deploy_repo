//! HZ-058 퀵슬롯 7칸 × 2쪽 (hero2 식 — user 2026-10-30: *"퀵슬롯이 hero2처럼 7칸있어야해"*,
//! `HEROZERO_QUICKSLOT_PLAN.md`): the plates the host keeps over the game's six quick slots.
//!
//! The game has six slots, `ed.b:[B` — [0][1] the character's skills (keys 1 · 3), [2][3] the
//! spirit's (7 · 9), [4][5] the potions (0 · #) — and they are what its save holds. The plates are
//! the host's: seven a page, two pages, each an [`Entry`] (a value the game's slots take). A plate
//! is used by SEATING its entry in a slot of its kind ([`seat`]: where it already is, else an empty
//! one, else the one used longest ago) and using that slot the game's way — so the save never holds
//! anything the game could not have made itself.
//!
//! Cooldowns stay honest across swaps (the user's choice, 2026-10-30 — hero2 seats as ready): the
//! game keeps one per slot, so [`Cooldowns`] remembers each entry's and a seated entry gets its own
//! back ([`Cooldowns::observe`] reads the game's slots every frame; an entry out of the slots counts
//! down by the field's ticks).
//!
//! Who the plates belong to is an [`Owner`] — the save slot (1..10) and the job — kept in the file
//! [`FILE`] ([`Store`]) and carried by a 세이브 옮기기 file's manifest ([`Store::lines_for`]).
//! Pure: the hook (`crate::hooks`) reads and writes the game.

use std::collections::{BTreeMap, HashMap};

/// Plates a page.
pub const PLATES: usize = 7;
/// Pages.
pub const PAGES: usize = 2;
/// The game's quick slots (`ed.b:[B`).
pub const GAME_SLOTS: usize = 6;
/// The host file, in the app's data directory.
pub const FILE: &str = "hz_quickpages.txt";

/// What a plate holds: a value of the game's slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Entry {
    /// A character skill (its action number, 4.. — slots 0 · 1).
    Skill(i8),
    /// A spirit skill (`/s/d/a` order — slots 2 · 3, used as value + 100).
    Spirit(i8),
    /// A potion (`/itm/20` number — slot 4, the `0` key).
    Potion(i8),
}

impl Entry {
    /// The game's slots this kind sits in. A potion uses the `0` slot only — the `#` slot stays the
    /// player's own.
    pub fn slots(self) -> &'static [usize] {
        match self {
            Entry::Skill(_) => &[0, 1],
            Entry::Spirit(_) => &[2, 3],
            Entry::Potion(_) => &[4],
        }
    }

    /// The byte the game's slot holds for it.
    pub fn value(self) -> i8 {
        match self {
            Entry::Skill(v) | Entry::Spirit(v) | Entry::Potion(v) => v,
        }
    }

    /// What game slot `g` holding `v` is, if anything (`v < 0` = empty).
    pub fn of_slot(g: usize, v: i8) -> Option<Entry> {
        if v < 0 {
            return None;
        }
        match g {
            0 | 1 => Some(Entry::Skill(v)),
            2 | 3 => Some(Entry::Spirit(v)),
            4 | 5 => Some(Entry::Potion(v)),
            _ => None,
        }
    }

    /// Its word in the file: `s12`, `p3`, `i1`.
    pub fn token(self) -> String {
        match self {
            Entry::Skill(v) => format!("s{v}"),
            Entry::Spirit(v) => format!("p{v}"),
            Entry::Potion(v) => format!("i{v}"),
        }
    }

    /// A plate's word: `-` (empty) or an entry's. `None` = not a plate's word.
    pub fn parse(t: &str) -> Option<Option<Entry>> {
        if t == "-" {
            return Some(None);
        }
        let (kind, n) = t.split_at_checked(1)?;
        let v: i8 = n.parse().ok().filter(|v| *v >= 0)?;
        Some(Some(match kind {
            "s" => Entry::Skill(v),
            "p" => Entry::Spirit(v),
            "i" => Entry::Potion(v),
            _ => return None,
        }))
    }
}

/// One owner's plates: [`PAGES`] of [`PLATES`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Plates {
    pub pages: [[Option<Entry>; PLATES]; PAGES],
}

impl Plates {
    /// What a player who never set plates starts with: the game's six slots as they are, on page 1's
    /// first six plates (1 3 7 9 0 #) — the screen they had before.
    pub fn adopt(game: &[i8; GAME_SLOTS]) -> Plates {
        let mut p = Plates::default();
        for (g, v) in game.iter().enumerate() {
            if let Some(e) = Entry::of_slot(g, *v)
                && !p.contains(e)
            {
                p.pages[0][g] = Some(e);
            }
        }
        p
    }

    pub fn get(&self, page: usize, plate: usize) -> Option<Entry> {
        self.pages.get(page).and_then(|p| p.get(plate)).copied().flatten()
    }

    pub fn contains(&self, e: Entry) -> bool {
        self.pages.iter().flatten().any(|x| *x == Some(e))
    }

    /// Put `e` on a plate — and off any other (one plate an entry, as the game keeps one slot a skill).
    pub fn set(&mut self, page: usize, plate: usize, e: Entry) {
        if page >= PAGES || plate >= PLATES {
            return;
        }
        self.remove(e);
        self.pages[page][plate] = Some(e);
    }

    /// Take `e` off every plate; whether it was on one.
    pub fn remove(&mut self, e: Entry) -> bool {
        let mut was = false;
        for x in self.pages.iter_mut().flatten() {
            if *x == Some(e) {
                *x = None;
                was = true;
            }
        }
        was
    }

    /// The file's words: [`PAGES`] × [`PLATES`], page by page.
    pub fn tokens(&self) -> String {
        self.pages.iter().flatten().map(|e| e.map_or_else(|| "-".to_string(), Entry::token)).collect::<Vec<_>>().join(" ")
    }

    /// From the file's words (exactly [`PAGES`] × [`PLATES`] of them).
    pub fn from_tokens<'a>(words: impl Iterator<Item = &'a str>) -> Option<Plates> {
        let words: Vec<&str> = words.collect();
        if words.len() != PAGES * PLATES {
            return None;
        }
        let mut p = Plates::default();
        for (i, w) in words.iter().enumerate() {
            p.pages[i / PLATES][i % PLATES] = Entry::parse(w)?;
        }
        Some(p)
    }
}

/// Whose plates: the save slot (1..10, HZ-020's numbering) and the job (`ed.b:B`) — a new character in
/// the slot, or a job change, starts from the game's slots again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Owner {
    pub slot: u8,
    pub class: i8,
}

/// Every owner's plates: the file [`FILE`], one line each — `own <slot> <class> <14 words>`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Store {
    pub rows: BTreeMap<Owner, Plates>,
}

/// A line's first word in the file, and in a transfer file's manifest.
pub const LINE_OWN: &str = "own";
pub const LINE_QUICK: &str = "quick";

impl Store {
    /// The file's text; a line that does not read is skipped.
    pub fn parse(text: &str) -> Store {
        let mut s = Store::default();
        for line in text.lines() {
            if let Some((owner, plates)) = Store::line(line, LINE_OWN) {
                s.rows.insert(owner, plates);
            }
        }
        s
    }

    /// `<word> <slot> <class> <14 words>`.
    fn line(line: &str, word: &str) -> Option<(Owner, Plates)> {
        let mut w = line.split_whitespace();
        (w.next()? == word).then_some(())?;
        let slot: u8 = w.next()?.parse().ok()?;
        let class: i8 = w.next()?.parse().ok()?;
        Some((Owner { slot, class }, Plates::from_tokens(w)?))
    }

    pub fn format(&self) -> String {
        self.rows.iter().map(|(o, p)| format!("{LINE_OWN} {} {} {}\n", o.slot, o.class, p.tokens())).collect()
    }

    /// The manifest lines of a transfer file for these save slots: `quick <slot> <class> <14 words>`.
    pub fn lines_for(&self, slots: &[u8]) -> Vec<String> {
        self.rows.iter().filter(|(o, _)| slots.contains(&o.slot)).map(|(o, p)| format!("{LINE_QUICK} {} {} {}", o.slot, o.class, p.tokens())).collect()
    }

    /// A transfer file's manifest lines, read.
    pub fn read_lines<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<(Owner, Plates)> {
        lines.filter_map(|l| Store::line(l.trim(), LINE_QUICK)).collect()
    }

    /// An import (「파일 상태 그대로」): the save slots it brings lose every row they had here and take
    /// the file's.
    pub fn replace_slots(&mut self, slots: &[u8], rows: &[(Owner, Plates)]) {
        self.rows.retain(|o, _| !slots.contains(&o.slot));
        for (o, p) in rows {
            if slots.contains(&o.slot) {
                self.rows.insert(*o, *p);
            }
        }
    }
}

/// The game slot to use `e` from: where it already sits, else an empty one of its kind, else the one
/// of its kind used longest ago (`last_used`: a frame count each).
pub fn seat(e: Entry, game: &[i8; GAME_SLOTS], last_used: &[u64; GAME_SLOTS]) -> usize {
    let slots = e.slots();
    if let Some(&g) = slots.iter().find(|&&g| game[g] == e.value()) {
        return g;
    }
    if let Some(&g) = slots.iter().find(|&&g| game[g] < 0) {
        return g;
    }
    *slots.iter().min_by_key(|&&g| last_used[g]).unwrap_or(&slots[0])
}

/// Each skill's cooldown, `(left, full)` in the game's frames — the game's own while it sits in a
/// slot, counted down by the field's ticks while it does not.
#[derive(Clone, Debug, Default)]
pub struct Cooldowns {
    left: HashMap<Entry, (i32, i32)>,
    tick: Option<u64>,
}

impl Cooldowns {
    /// One look at the game's skill slots (`game` = `ed.b:[B`, `left` / `full` = `ed.a:[I` /
    /// `ed.b:[I` [0..4]) at field tick `tick`.
    pub fn observe(&mut self, game: &[i8; GAME_SLOTS], left: &[i32; 4], full: &[i32; 4], tick: u64) {
        let elapsed = self.tick.map_or(0, |t| tick.saturating_sub(t)).min(i32::MAX as u64) as i32;
        self.tick = Some(tick);
        let seated: Vec<Entry> = (0..4).filter_map(|g| Entry::of_slot(g, game[g])).collect();
        for (e, (l, _)) in self.left.iter_mut() {
            if !seated.contains(e) {
                *l = (*l - elapsed).max(0);
            }
        }
        for g in 0..4 {
            if let Some(e) = Entry::of_slot(g, game[g]) {
                self.left.insert(e, (left[g].max(0), full[g].max(0)));
            }
        }
        self.left.retain(|_, (l, _)| *l > 0);
    }

    /// `(left, full)` for `e` — `(0, 0)` when ready.
    pub fn of(&self, e: Entry) -> (i32, i32) {
        self.left.get(&e).copied().unwrap_or((0, 0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY: [i8; GAME_SLOTS] = [-1; GAME_SLOTS];

    #[test]
    fn a_new_owner_starts_from_the_games_slots() {
        let p = Plates::adopt(&[4, 7, 2, -1, 1, 3]);
        assert_eq!(p.pages[0][..6], [Some(Entry::Skill(4)), Some(Entry::Skill(7)), Some(Entry::Spirit(2)), None, Some(Entry::Potion(1)), Some(Entry::Potion(3))]);
        assert_eq!(p.pages[0][6], None);
        assert!(p.pages[1].iter().all(Option::is_none));
        assert_eq!(Plates::adopt(&EMPTY), Plates::default());
    }

    #[test]
    fn an_entry_is_on_one_plate() {
        let mut p = Plates::default();
        p.set(0, 2, Entry::Skill(5));
        p.set(1, 6, Entry::Skill(5));
        assert_eq!((p.get(0, 2), p.get(1, 6)), (None, Some(Entry::Skill(5))));
        p.set(0, 0, Entry::Spirit(5));
        assert!(p.contains(Entry::Spirit(5)) && p.contains(Entry::Skill(5)), "a spirit skill is not a character skill");
        assert!(p.remove(Entry::Skill(5)) && !p.remove(Entry::Skill(5)));
        p.set(2, 0, Entry::Skill(4));
        p.set(0, 7, Entry::Skill(4));
        assert!(!p.contains(Entry::Skill(4)), "off the pages: nothing");
    }

    #[test]
    fn the_file_reads_back() {
        let mut s = Store::default();
        let mut p = Plates::adopt(&[4, -1, 2, -1, 1, -1]);
        p.set(1, 3, Entry::Skill(9));
        s.rows.insert(Owner { slot: 7, class: 3 }, p);
        s.rows.insert(Owner { slot: 1, class: 0 }, Plates::default());
        let text = s.format();
        assert!(text.starts_with("own 1 0 - - -"), "{text}");
        assert_eq!(Store::parse(&text), s);
        assert_eq!(Store::parse("own 1 0 s4\nnonsense\nown x 0\n"), Store::default(), "short or broken lines are skipped");
        assert_eq!(Entry::parse("p12"), Some(Some(Entry::Spirit(12))));
        assert_eq!(Entry::parse("s-1"), None);
        assert_eq!(Entry::parse("x1"), None);
    }

    #[test]
    fn a_transfer_carries_its_slots_rows_and_replaces_them() {
        let mut here = Store::default();
        let mine = Plates::adopt(&[4, -1, -1, -1, -1, -1]);
        here.rows.insert(Owner { slot: 1, class: 0 }, mine);
        here.rows.insert(Owner { slot: 2, class: 1 }, mine);
        let mut there = Store::default();
        let theirs = Plates::adopt(&[5, 6, -1, -1, 2, -1]);
        there.rows.insert(Owner { slot: 1, class: 2 }, theirs);
        there.rows.insert(Owner { slot: 3, class: 0 }, theirs);
        let lines = there.lines_for(&[1]);
        assert_eq!(lines.len(), 1);
        let rows = Store::read_lines(lines.iter().map(String::as_str).chain(["format herozero-save 1", "quick 9 9"]));
        here.replace_slots(&[1], &rows);
        assert_eq!(here.rows.keys().copied().collect::<Vec<_>>(), vec![Owner { slot: 1, class: 2 }, Owner { slot: 2, class: 1 }], "slot 1 is the file's, slot 2 stays");
        assert_eq!(here.rows[&Owner { slot: 1, class: 2 }], theirs);
    }

    #[test]
    fn a_seat_is_where_it_is_else_empty_else_least_recently_used() {
        let mut game = [4, -1, 2, 3, -1, -1];
        let used = [5, 0, 9, 1, 0, 0];
        assert_eq!(seat(Entry::Skill(4), &game, &used), 0, "already there");
        assert_eq!(seat(Entry::Skill(6), &game, &used), 1, "the empty one");
        game[1] = 7;
        assert_eq!(seat(Entry::Skill(6), &game, &used), 1, "the one used longest ago (frame 0 < 5)");
        assert_eq!(seat(Entry::Spirit(5), &game, &used), 3, "spirits among 2 · 3");
        assert_eq!(seat(Entry::Potion(1), &game, &used), 4, "a potion in the 0 slot");
    }

    #[test]
    fn a_cooldown_follows_its_skill_out_of_the_slot_and_back() {
        let mut cd = Cooldowns::default();
        // Skill 4 cast in slot 0: 100 of 120 left.
        cd.observe(&[4, -1, -1, -1, -1, -1], &[100, 0, 0, 0], &[120, 0, 0, 0], 10);
        assert_eq!(cd.of(Entry::Skill(4)), (100, 120));
        // Skill 6 takes slot 0 at tick 30: 4 is out, counting down by the ticks.
        cd.observe(&[6, -1, -1, -1, -1, -1], &[0, 0, 0, 0], &[80, 0, 0, 0], 30);
        assert_eq!(cd.of(Entry::Skill(4)), (80, 120));
        assert_eq!(cd.of(Entry::Skill(6)), (0, 0), "ready");
        cd.observe(&[6, -1, -1, -1, -1, -1], &[0, 0, 0, 0], &[80, 0, 0, 0], 100);
        assert_eq!(cd.of(Entry::Skill(4)), (10, 120));
        cd.observe(&[6, -1, -1, -1, -1, -1], &[0, 0, 0, 0], &[80, 0, 0, 0], 200);
        assert_eq!(cd.of(Entry::Skill(4)), (0, 0), "run out, forgotten");
        // A spirit skill and a character skill of the same number are two cooldowns.
        cd.observe(&[3, -1, 3, -1, -1, -1], &[50, 0, 7, 0], &[60, 0, 9, 0], 210);
        assert_eq!((cd.of(Entry::Skill(3)), cd.of(Entry::Spirit(3))), ((50, 60), (7, 9)));
    }
}
