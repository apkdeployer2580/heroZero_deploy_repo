// ★ heroZero fork (2026-10-21): `control` (the `Control` interface + the `media.control` package
// with `VolumeControl`) and `controllable` are new; `player` gained the MIDP 2.0 lifecycle.
pub mod control;
mod controllable;
mod manager;
mod media_exception;
pub mod player;

pub use self::{control::Control, controllable::Controllable, manager::Manager, media_exception::MediaException, player::Player};
