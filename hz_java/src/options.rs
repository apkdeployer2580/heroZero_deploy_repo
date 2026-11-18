//! The player's game options — the rows of the 「추가 설정」 page (HZ-007), read by the hooks.
//!
//! One small text file per option in the app-private data dir (the shape `hz_android` already
//! used for the orientation): a value says what it does over `adb shell run-as`, a corrupt file
//! damages one option, and every reader treats missing / unreadable / unknown as the SHIPPED
//! default — a fresh install and a corrupt file behave the same. The defaults are the original
//! game: landscape (the user's 2026-10-22 decision), auto-loot off, EXP and drops 1배, the
//! monsters as the game makes them (난이도 쉬움, 리젠 1배) — and the touch stick as it has been,
//! 자유 (its centre where the thumb lands).
//!
//! None of this goes into the game's own options record (RMS `d`): that one is checksummed and
//! a save under the save-integrity rules. These are the host's.

use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU8, Ordering},
};

/// A row of the 「추가 설정」 page, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Orientation,
    AutoLoot,
    ExpRate,
    DropRate,
    /// 난이도 (HZ-017, hero2's 2026-09-29 row): monster HP and attack x1 / x1.5 / x2 — 쉬움 /
    /// 보통 / 어려움, hero2's EASY / NORMAL / HARD, where EASY is the original game.
    Difficulty,
    /// 몬스터 리젠 (HZ-018, hero2's row): a slain monster comes back 1..6 times sooner.
    Respawn,
    /// 조작 방식 (2026-10-25, hero2/hero3's control type, [`ControlMode`]). The host's, read by
    /// `hz_android`.
    Control,
}

impl Row {
    pub const ALL: [Row; 7] =
        [Row::Orientation, Row::AutoLoot, Row::ExpRate, Row::DropRate, Row::Difficulty, Row::Respawn, Row::Control];

    pub fn at(index: usize) -> Option<Row> {
        Row::ALL.get(index).copied()
    }
}

/// How the field is walked on the touch HUD — hero3's three (자유 / 십자키 / 고정), which are hero2's
/// free stick, touch type B and fixed_A.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ControlMode {
    /// The stick, its centre where the thumb lands. The default, as it has been.
    #[default]
    Free,
    /// Four arrows on the stick's seat, the ones a window has (A8, 2026-10-26).
    Dpad,
    /// The stick, its centre its seat: a thumb landing off centre already walks.
    Fixed,
}

impl ControlMode {
    /// The row's order: LEFT / RIGHT step through it with wrap-around.
    pub const ALL: [ControlMode; 3] = [ControlMode::Free, ControlMode::Dpad, ControlMode::Fixed];

    /// File token — `free` / `dpad` / `fixed`; anything else reads back as [`ControlMode::Free`].
    pub fn as_token(self) -> &'static str {
        match self {
            ControlMode::Free => "free",
            ControlMode::Dpad => "dpad",
            ControlMode::Fixed => "fixed",
        }
    }

    pub fn parse(s: &str) -> Option<ControlMode> {
        ControlMode::ALL.into_iter().find(|m| m.as_token() == s.trim())
    }
}

/// EXP multipliers, LEFT/RIGHT cycles through them. [0] is the original game.
pub const EXP_RATES: [u32; 4] = [1, 2, 3, 5];
/// Item-slot drop multipliers. [0] is the original game.
pub const DROP_RATES: [u32; 3] = [1, 2, 3];
/// Monster HP / attack in percent, [0] the original game (hero2's `DIFFICULTY_PCT`).
pub const DIFFICULTY_PCT: [u32; 3] = [100, 150, 200];
/// How many times sooner a slain monster returns. [0] is the original game.
pub const RESPAWN_RATES: [u32; 6] = [1, 2, 3, 4, 5, 6];

pub const ORIENTATION_FILE: &str = "hz_orientation.txt";
pub const AUTO_LOOT_FILE: &str = "hz_auto_loot.txt";
pub const EXP_RATE_FILE: &str = "hz_exp_rate.txt";
pub const DROP_RATE_FILE: &str = "hz_drop_rate.txt";
/// The percent: `100` / `150` / `200`.
pub const DIFFICULTY_FILE: &str = "hz_difficulty.txt";
pub const RESPAWN_FILE: &str = "hz_respawn_rate.txt";
/// `free` / `dpad` / `fixed` ([`ControlMode::as_token`]).
pub const CONTROL_FILE: &str = "hz_control.txt";

pub struct GameOptions {
    /// The preference — what the NEXT launch uses.
    landscape: AtomicBool,
    /// What THIS run's canvas was made for. The game sizes its back buffer once and its
    /// `sizeChanged` is empty, so a change only lands at the next launch.
    landscape_now: bool,
    auto_loot: AtomicBool,
    /// Indices into [`EXP_RATES`] / [`DROP_RATES`] / [`DIFFICULTY_PCT`] / [`RESPAWN_RATES`].
    exp: AtomicU8,
    drop: AtomicU8,
    difficulty: AtomicU8,
    respawn: AtomicU8,
    /// 조작 방식: an index into [`ControlMode::ALL`].
    control: AtomicU8,
    /// Where changes are written. `None` (the probe) keeps them in memory.
    dir: Option<PathBuf>,
}

impl GameOptions {
    /// The shipped defaults, kept in memory only.
    pub fn new() -> GameOptions {
        GameOptions {
            landscape: AtomicBool::new(true),
            landscape_now: true,
            auto_loot: AtomicBool::new(false),
            exp: AtomicU8::new(0),
            drop: AtomicU8::new(0),
            difficulty: AtomicU8::new(0),
            respawn: AtomicU8::new(0),
            control: AtomicU8::new(0),
            dir: None,
        }
    }

    /// Read every option from `dir`; later changes are written back there.
    pub fn load(dir: &Path) -> GameOptions {
        let read = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap_or_default();
        let landscape = parse_landscape(&read(ORIENTATION_FILE));
        GameOptions {
            landscape: AtomicBool::new(landscape),
            landscape_now: landscape,
            auto_loot: AtomicBool::new(read(AUTO_LOOT_FILE).trim() == "on"),
            exp: AtomicU8::new(parse_rate(&read(EXP_RATE_FILE), &EXP_RATES)),
            drop: AtomicU8::new(parse_rate(&read(DROP_RATE_FILE), &DROP_RATES)),
            difficulty: AtomicU8::new(parse_rate(&read(DIFFICULTY_FILE), &DIFFICULTY_PCT)),
            respawn: AtomicU8::new(parse_rate(&read(RESPAWN_FILE), &RESPAWN_RATES)),
            control: AtomicU8::new(control_index(ControlMode::parse(&read(CONTROL_FILE)).unwrap_or_default())),
            dir: Some(dir.to_path_buf()),
        }
    }

    pub fn landscape(&self) -> bool {
        self.landscape.load(Ordering::Relaxed)
    }

    pub fn landscape_now(&self) -> bool {
        self.landscape_now
    }

    /// The orientation was changed this run and will apply at the next launch.
    pub fn orientation_pending(&self) -> bool {
        self.landscape() != self.landscape_now
    }

    /// Where the host's files are (`None` in the probe: nothing written).
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    pub fn auto_loot(&self) -> bool {
        self.auto_loot.load(Ordering::Relaxed)
    }

    pub fn exp_rate(&self) -> u32 {
        EXP_RATES[self.exp.load(Ordering::Relaxed) as usize % EXP_RATES.len()]
    }

    pub fn drop_rate(&self) -> u32 {
        DROP_RATES[self.drop.load(Ordering::Relaxed) as usize % DROP_RATES.len()]
    }

    /// Monster HP and attack in percent (100 = the original game).
    pub fn difficulty_pct(&self) -> u32 {
        DIFFICULTY_PCT[self.difficulty_level()]
    }

    /// Index of [`Self::difficulty_pct`] — 0 쉬움, 1 보통, 2 어려움.
    pub fn difficulty_level(&self) -> usize {
        self.difficulty.load(Ordering::Relaxed) as usize % DIFFICULTY_PCT.len()
    }

    pub fn respawn_rate(&self) -> u32 {
        RESPAWN_RATES[self.respawn.load(Ordering::Relaxed) as usize % RESPAWN_RATES.len()]
    }

    /// 조작 방식.
    pub fn control(&self) -> ControlMode {
        ControlMode::ALL[self.control.load(Ordering::Relaxed) as usize % ControlMode::ALL.len()]
    }

    /// LEFT (`delta` < 0) or RIGHT / OK (> 0) on a row: toggles, or steps through the rates
    /// with wrap-around like the game's own rows (`bn.a(BB)B`). Persists at once.
    pub fn step(&self, row: Row, delta: i32) {
        let cycle = |a: &AtomicU8, n: usize| {
            let i = (a.load(Ordering::Relaxed) as i32 + delta).rem_euclid(n as i32);
            a.store(i as u8, Ordering::Relaxed);
        };
        match row {
            Row::Orientation => {
                self.landscape.fetch_xor(true, Ordering::Relaxed);
            }
            Row::AutoLoot => {
                self.auto_loot.fetch_xor(true, Ordering::Relaxed);
            }
            Row::ExpRate => cycle(&self.exp, EXP_RATES.len()),
            Row::DropRate => cycle(&self.drop, DROP_RATES.len()),
            Row::Difficulty => cycle(&self.difficulty, DIFFICULTY_PCT.len()),
            Row::Respawn => cycle(&self.respawn, RESPAWN_RATES.len()),
            Row::Control => cycle(&self.control, ControlMode::ALL.len()),
        }
        tracing::info!("option {row:?} -> {}", self.describe(row));
        self.save(row);
    }

    /// The row's value as the files store it (also what the log prints).
    pub fn describe(&self, row: Row) -> String {
        match row {
            Row::Orientation => if self.landscape() { "landscape" } else { "portrait" }.to_string(),
            Row::AutoLoot => if self.auto_loot() { "on" } else { "off" }.to_string(),
            Row::ExpRate => self.exp_rate().to_string(),
            Row::DropRate => self.drop_rate().to_string(),
            Row::Difficulty => self.difficulty_pct().to_string(),
            Row::Respawn => self.respawn_rate().to_string(),
            Row::Control => self.control().as_token().to_string(),
        }
    }

    fn save(&self, row: Row) {
        let Some(dir) = &self.dir else { return };
        let name = match row {
            Row::Orientation => ORIENTATION_FILE,
            Row::AutoLoot => AUTO_LOOT_FILE,
            Row::ExpRate => EXP_RATE_FILE,
            Row::DropRate => DROP_RATE_FILE,
            Row::Difficulty => DIFFICULTY_FILE,
            Row::Respawn => RESPAWN_FILE,
            Row::Control => CONTROL_FILE,
        };
        let path = dir.join(name);
        if let Err(e) = std::fs::write(&path, self.describe(row)) {
            tracing::warn!("options: cannot write {path:?}: {e}");
        }
    }

    /// `exp=2,drop=3,diff=150,respawn=3,loot=on,orient=portrait` — the probe's `--opt`. In memory only; an unknown
    /// key or value is an error, so a typo cannot silently mean "default".
    pub fn apply_spec(&self, spec: &str) -> Result<(), String> {
        for item in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let (k, v) = item.split_once('=').ok_or_else(|| format!("{item}: expected key=value"))?;
            let index = |rates: &[u32]| v.parse::<u32>().ok().and_then(|n| rates.iter().position(|&r| r == n)).map(|i| i as u8);
            match k.trim() {
                "exp" => self.exp.store(
                    index(&EXP_RATES).ok_or_else(|| format!("exp={v}: one of {EXP_RATES:?}"))?,
                    Ordering::Relaxed,
                ),
                "drop" => self.drop.store(
                    index(&DROP_RATES).ok_or_else(|| format!("drop={v}: one of {DROP_RATES:?}"))?,
                    Ordering::Relaxed,
                ),
                "diff" => self.difficulty.store(
                    index(&DIFFICULTY_PCT).ok_or_else(|| format!("diff={v}: one of {DIFFICULTY_PCT:?}"))?,
                    Ordering::Relaxed,
                ),
                "respawn" => self.respawn.store(
                    index(&RESPAWN_RATES).ok_or_else(|| format!("respawn={v}: one of {RESPAWN_RATES:?}"))?,
                    Ordering::Relaxed,
                ),
                "loot" => match v {
                    "on" | "off" => self.auto_loot.store(v == "on", Ordering::Relaxed),
                    _ => return Err(format!("loot={v}: on or off")),
                },
                "orient" => match v {
                    "landscape" | "portrait" => self.landscape.store(v == "landscape", Ordering::Relaxed),
                    _ => return Err(format!("orient={v}: landscape or portrait")),
                },
                "stick" => match ControlMode::parse(v) {
                    Some(m) => self.control.store(control_index(m), Ordering::Relaxed),
                    None => return Err(format!("stick={v}: free, dpad or fixed")),
                },
                other => return Err(format!("unknown option {other} (exp, drop, diff, respawn, loot, orient, stick)")),
            }
        }
        Ok(())
    }
}

impl Default for GameOptions {
    fn default() -> Self {
        GameOptions::new()
    }
}

/// `portrait` or anything else (= landscape, the default).
pub fn parse_landscape(s: &str) -> bool {
    s.trim() != "portrait"
}

fn control_index(m: ControlMode) -> u8 {
    ControlMode::ALL.iter().position(|&x| x == m).unwrap_or(0) as u8
}

fn parse_rate(s: &str, rates: &[u32]) -> u8 {
    s.trim().parse::<u32>().ok().and_then(|n| rates.iter().position(|&r| r == n)).unwrap_or(0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hz_options_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn defaults_are_the_original_game() {
        let o = GameOptions::new();
        assert!(o.landscape() && !o.orientation_pending());
        assert!(!o.auto_loot());
        assert_eq!((o.exp_rate(), o.drop_rate()), (1, 1));
        assert_eq!((o.difficulty_pct(), o.difficulty_level(), o.respawn_rate()), (100, 0, 1));
    }

    #[test]
    fn difficulty_and_respawn_step_persist_and_parse() {
        let dir = temp_dir("monsters");
        let o = GameOptions::load(&dir);
        o.step(Row::Difficulty, 1);
        o.step(Row::Respawn, -1);
        assert_eq!((o.difficulty_pct(), o.respawn_rate()), (150, 6), "LEFT from 1배 wraps to 6배");
        assert_eq!(std::fs::read_to_string(dir.join(DIFFICULTY_FILE)).unwrap(), "150");
        assert_eq!(std::fs::read_to_string(dir.join(RESPAWN_FILE)).unwrap(), "6");
        let again = GameOptions::load(&dir);
        assert_eq!((again.difficulty_pct(), again.difficulty_level(), again.respawn_rate()), (150, 1, 6));
        std::fs::write(dir.join(DIFFICULTY_FILE), "300").unwrap();
        std::fs::write(dir.join(RESPAWN_FILE), "0").unwrap();
        let bad = GameOptions::load(&dir);
        assert_eq!((bad.difficulty_pct(), bad.respawn_rate()), (100, 1), "unknown = the original game");
        let p = GameOptions::new();
        p.apply_spec("diff=200,respawn=3").unwrap();
        assert_eq!((p.difficulty_pct(), p.respawn_rate()), (200, 3));
        assert!(p.apply_spec("diff=120").is_err() && p.apply_spec("respawn=7").is_err());
        assert_eq!((Row::at(4), Row::at(5)), (Some(Row::Difficulty), Some(Row::Respawn)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_control_row_steps_free_dpad_fixed_and_persists() {
        let dir = temp_dir("control");
        let o = GameOptions::load(&dir);
        assert_eq!(o.control(), ControlMode::Free, "자유 by default");
        o.step(Row::Control, 1);
        assert_eq!(o.control(), ControlMode::Dpad, "then 십자키, hero3's order");
        assert_eq!(std::fs::read_to_string(dir.join(CONTROL_FILE)).unwrap(), "dpad");
        o.step(Row::Control, 1);
        assert_eq!(o.control(), ControlMode::Fixed);
        assert_eq!(std::fs::read_to_string(dir.join(CONTROL_FILE)).unwrap(), "fixed", "the file a 2026-10-25 build wrote");
        assert_eq!(GameOptions::load(&dir).control(), ControlMode::Fixed, "read back");
        o.step(Row::Control, 1);
        assert_eq!(GameOptions::load(&dir).control(), ControlMode::Free, "wraps");
        o.step(Row::Control, -1);
        assert_eq!(o.control(), ControlMode::Fixed, "LEFT from 자유 wraps to 고정");
        std::fs::write(dir.join(CONTROL_FILE), "sideways").unwrap();
        assert_eq!(GameOptions::load(&dir).control(), ControlMode::Free, "unknown = 자유");
        let p = GameOptions::new();
        p.apply_spec("stick=dpad").unwrap();
        assert_eq!(p.control(), ControlMode::Dpad);
        p.apply_spec("stick=fixed").unwrap();
        assert!(p.control() == ControlMode::Fixed && p.apply_spec("stick=cross").is_err());
        assert_eq!(Row::at(6), Some(Row::Control));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_or_garbage_files_are_the_defaults() {
        let dir = temp_dir("garbage");
        let o = GameOptions::load(&dir);
        assert!(o.landscape() && !o.auto_loot() && o.exp_rate() == 1 && o.drop_rate() == 1);
        for (f, v) in [
            (ORIENTATION_FILE, "sideways"),
            (AUTO_LOOT_FILE, "yes"),
            (EXP_RATE_FILE, "4"),
            (DROP_RATE_FILE, "-1"),
        ] {
            std::fs::write(dir.join(f), v).unwrap();
        }
        let o = GameOptions::load(&dir);
        assert!(o.landscape() && !o.auto_loot() && o.exp_rate() == 1 && o.drop_rate() == 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn steps_wrap_and_persist() {
        let dir = temp_dir("steps");
        let o = GameOptions::load(&dir);
        o.step(Row::ExpRate, -1);
        assert_eq!(o.exp_rate(), 5, "LEFT from 1배 wraps to the top, like the game's own rows");
        o.step(Row::ExpRate, 1);
        o.step(Row::ExpRate, 1);
        assert_eq!(o.exp_rate(), 2);
        o.step(Row::DropRate, 1);
        o.step(Row::AutoLoot, 1);
        o.step(Row::Orientation, -1);
        let again = GameOptions::load(&dir);
        assert_eq!(
            (again.exp_rate(), again.drop_rate(), again.auto_loot(), again.landscape()),
            (2, 2, true, false)
        );
        assert_eq!(
            std::fs::read_to_string(dir.join(ORIENTATION_FILE)).unwrap(),
            "portrait",
            "the file hz_android has always read"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn orientation_applies_next_launch() {
        let dir = temp_dir("orient");
        let o = GameOptions::load(&dir);
        o.step(Row::Orientation, 1);
        assert!(!o.landscape() && o.landscape_now() && o.orientation_pending());
        o.step(Row::Orientation, 1);
        assert!(!o.orientation_pending(), "changed back: nothing pending");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn probe_spec() {
        let o = GameOptions::new();
        o.apply_spec("exp=3, drop=2,loot=on,orient=portrait").unwrap();
        assert_eq!((o.exp_rate(), o.drop_rate(), o.auto_loot(), o.landscape()), (3, 2, true, false));
        assert!(o.apply_spec("exp=4").is_err());
        assert!(o.apply_spec("speed=2").is_err());
        assert!(o.apply_spec("loot").is_err());
    }
}
