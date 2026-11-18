//! 영웅서기 Zero's host side of the JVM, shared by `hz_probe` and `hz_android`.
//!
//! * [`options`] — the player's game options: the 「추가 설정」 page the HZ-007 patch adds to the
//!   game's own 환경설정 (화면 방향 · 오토루팅 · 경험치 배율 · 드랍률). Host files, never the
//!   game's RMS.
//! * [`hooks`] — `net/wie/hz/Hooks`, the class the `hz_patches` hook patches call. Give it to the
//!   emulator with `J2meOptions::extra_protos`.
//!
//! * [`input`] — how the game's canvas consumes keys (one held key) and the projection of the
//!   host's honest key edges onto that, shared by the probe's key script and the APK's worker.
//! * [`state`] — what the guest is showing (field / menu / dialogue, a message box up), read
//!   once a frame by the HZ-008 blit hook for the touch HUD.
//! * [`portrait`] — the status block's head: the hero's paper doll composed from the game's own
//!   pictures (Phase 6-6).
//! * [`revive`] — HZ-016's 「부활의 구」: its item record, picture and death question.
//! * [`gear`] — HZ-023's 가디언 장비: one LV 80 piece per equipment category and its drop.
//! * [`guardian_mode`] — HZ-030's 가디언 모드: the replay round after 헬, its monsters and rewards.
//! * [`legend`] — HZ-031's 85제 「전설」 pieces: their records, item-window lines and drop.
//! * [`effects`] — HZ-032: what the worn 85제 pieces do (skill, damage, cooldown and duration tables).
//! * [`drops`] — HZ-033: the game's own items drop far more readily (slots, numbers, top pieces, 에픽).
//! * [`boss`] — HZ-057: the bosses' reflect stance — when it comes, what comes back, the mirror coat.
//! * [`story`] — HZ-061: a boss's story waits five seconds after its death, for what it dropped.
//! * [`menu_touch`] — taps on the game's own windows, turned into what its keys would do.
//!
//! Game state is read by JVM field name, never by address (HEROZERO_REMAKE_PLAN.md §2.2).

pub mod bags;
pub mod balance;
pub mod boss;
pub mod bulk;
pub mod drops;
pub mod effects;
pub mod elixir;
pub mod gear;
pub mod guardian;
pub mod guardian_mode;
pub mod hooks;
pub mod input;
pub mod legend;
pub mod mapfile;
pub mod menu_touch;
pub mod options;
pub mod portrait;
pub mod quickpage;
pub mod revive;
pub mod reward;
pub mod slots;
pub mod state;
pub mod statname;
pub mod stats;
pub mod story;
pub mod tables;
mod text;

pub use options::{ControlMode, GameOptions, Row};

pub use state::{Buff, BuffKind, GuestState, GuestView, Icon, IconKey, MapGrid, MapMarks, PlateAsk, QuestGuide, QuickBar, QuickSlot, SaveAnswer, Scene, Stats};
pub use text::euc_kr;

/// How often the collector runs on the paint path, in painted frames (`wie_util::stats::
/// set_gc_every`; the APK's worker and the probe both set it, so the probe measures what ships).
/// Upstream collects on every painted frame, a full mark of every live object and class static
/// 20 times a second; the game never asks how much memory is free (`Runtime.freeMemory` is not
/// called — javap 2026-10-27) and its own `System.gc()` calls still collect at once, so a quarter
/// of that only holds a few frames' garbage a little longer.
pub const GC_EVERY: u64 = 4;
