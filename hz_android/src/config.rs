//! Host-side preferences that are not game options (those are `hz_java::options`) and not the
//! layout (`placement`): one small text file each in the app-private data dir, missing or
//! unreadable = the shipped default.

use std::path::{Path, PathBuf};

use crate::padhud::PadHud;

/// STATS (Phase 3, 2026-10-22): the player's numbers (HP/SP/MP, level, EXP %) in the top-left
/// corner of the field. Default ON — the game's own bar has the bars but no numbers.
pub const STATS_FILE: &str = "hz_stats.txt";

pub fn read_stats(data_dir: &Path) -> bool {
    std::fs::read_to_string(data_dir.join(STATS_FILE)).map_or(true, |s| s.trim() != "off")
}

pub fn write_stats(data_dir: &Path, on: bool) -> std::io::Result<()> {
    std::fs::write(data_dir.join(STATS_FILE), if on { "on\n" } else { "off\n" })
}

/// SFX LEVEL (2026-10-25): the effects' level in dB over the mix's own, one of
/// `audio_mix::SFX_STEPS_DB`. Default 0; anything else in the file reads as 0.
pub const SFX_LEVEL_FILE: &str = "hz_sfx_level.txt";

pub fn read_sfx_db(data_dir: &Path) -> i32 {
    std::fs::read_to_string(data_dir.join(SFX_LEVEL_FILE))
        .ok()
        .and_then(|s| s.trim().parse::<i32>().ok())
        .filter(|db| crate::audio_mix::SFX_STEPS_DB.contains(db))
        .unwrap_or(0)
}

pub fn write_sfx_db(data_dir: &Path, db: i32) -> std::io::Result<()> {
    std::fs::write(data_dir.join(SFX_LEVEL_FILE), format!("{db}\n"))
}

/// The minimap panel (Phase 6-5): up unless the player tapped it away. Default ON.
pub const MINIMAP_FILE: &str = "hz_minimap.txt";

pub fn read_minimap(data_dir: &Path) -> bool {
    std::fs::read_to_string(data_dir.join(MINIMAP_FILE)).map_or(true, |s| s.trim() != "off")
}

pub fn write_minimap(data_dir: &Path, on: bool) -> std::io::Result<()> {
    std::fs::write(data_dir.join(MINIMAP_FILE), if on { "on\n" } else { "off\n" })
}

/// A DEVELOPER's patch selection (`hz_patches::Selection::parse` syntax, e.g. `default,HZ-013`),
/// read once at boot. Nothing writes it and players never have it: it is put there with
/// `adb shell run-as` on a debuggable build, to try a patch that is still off by default (Phase 6:
/// HZ-013, the game's bar off). Missing, empty or unreadable = the shipped default set.
pub const PATCHES_FILE: &str = "hz_patches.txt";

/// A DEVELOPER's switch for the host's debug log lines (`log::debug!` — key → drawn samples per
/// press, …): the file's presence, read once at start. Players never have it.
pub const DEBUG_FILE: &str = "hz_debug.txt";

pub fn read_patches(data_dir: &Path) -> hz_patches::Selection {
    let Ok(text) = std::fs::read_to_string(data_dir.join(PATCHES_FILE)) else {
        return hz_patches::Selection::Default;
    };
    let line = text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#')).unwrap_or("");
    hz_patches::Selection::parse(line).unwrap_or_else(|e| {
        log::warn!("{PATCHES_FILE}: {e} — the default patches");
        hz_patches::Selection::Default
    })
}

/// HEAT SAVER (hero2's 57차): draw the host surface at half the window's resolution on each axis
/// and let the compositor upscale it — a quarter of the blit and HUD work per frame. Default
/// OFF: the upscale softens the picture a little, and nothing may regress on a phone that is
/// fine as it is. Read once at start (the surface and the touch mapping are sized from it).
pub const HEAT_SAVER_FILE: &str = "hz_heat_saver.txt";

fn heat_saver_path(data_dir: &Path) -> PathBuf {
    data_dir.join(HEAT_SAVER_FILE)
}

pub fn read_heat_saver(data_dir: &Path) -> bool {
    std::fs::read_to_string(heat_saver_path(data_dir)).is_ok_and(|s| s.trim() == "on")
}

pub fn write_heat_saver(data_dir: &Path, on: bool) -> std::io::Result<()> {
    std::fs::write(heat_saver_path(data_dir), if on { "on\n" } else { "off\n" })
}

/// The render divisor for a preference: 2 = half resolution on each axis.
pub fn render_div(heat_saver: bool) -> u32 {
    if heat_saver { 2 } else { 1 }
}

/// PAD BADGES (`crate::padhud::PadHud`): whether the gamepad position badges are drawn on the
/// controls — `auto` (default: after a pad press, until the next touch), `on`, `off`.
pub const PAD_HUD_FILE: &str = "hz_pad_hud.txt";

pub fn read_pad_hud(data_dir: &Path) -> PadHud {
    std::fs::read_to_string(data_dir.join(PAD_HUD_FILE)).map_or(PadHud::Auto, |s| PadHud::parse(&s))
}

pub fn write_pad_hud(data_dir: &Path, mode: PadHud) -> std::io::Result<()> {
    std::fs::write(data_dir.join(PAD_HUD_FILE), format!("{}\n", mode.as_token()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heat_saver_round_trips_and_defaults_off() {
        let dir = std::env::temp_dir().join(format!("hz_cfg_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!read_heat_saver(&dir), "missing = off");
        write_heat_saver(&dir, true).unwrap();
        assert!(read_heat_saver(&dir));
        write_heat_saver(&dir, false).unwrap();
        assert!(!read_heat_saver(&dir));
        std::fs::write(heat_saver_path(&dir), "garbage").unwrap();
        assert!(!read_heat_saver(&dir), "unreadable = off");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(render_div(true), 2);
        assert_eq!(render_div(false), 1);
    }

    #[test]
    fn the_minimap_defaults_on_and_round_trips() {
        let dir = std::env::temp_dir().join(format!("hz_cfg_mm_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read_minimap(&dir), "missing = on");
        write_minimap(&dir, false).unwrap();
        assert!(!read_minimap(&dir));
        write_minimap(&dir, true).unwrap();
        assert!(read_minimap(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_patch_lever_defaults_to_the_shipped_set() {
        use hz_patches::Selection;
        let dir = std::env::temp_dir().join(format!("hz_cfg_patch_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(read_patches(&dir), Selection::Default, "missing = default");
        std::fs::write(dir.join(PATCHES_FILE), "# dev\ndefault,HZ-013\n").unwrap();
        let s = read_patches(&dir);
        assert!(s.wants_id("HZ-013") && s.wants_id("HZ-008"));
        std::fs::write(dir.join(PATCHES_FILE), "HZ-01").unwrap();
        assert_eq!(read_patches(&dir), Selection::Default, "a typo = default, never 'no patch'");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pad_hud_round_trips_and_defaults_auto() {
        let dir = std::env::temp_dir().join(format!("hz_cfg_pad_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(read_pad_hud(&dir), PadHud::Auto, "missing = auto");
        write_pad_hud(&dir, PadHud::Off).unwrap();
        assert_eq!(read_pad_hud(&dir), PadHud::Off);
        write_pad_hud(&dir, PadHud::On).unwrap();
        assert_eq!(read_pad_hud(&dir), PadHud::On);
        std::fs::write(dir.join(PAD_HUD_FILE), "garbage").unwrap();
        assert_eq!(read_pad_hud(&dir), PadHud::Auto, "unreadable = auto");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
