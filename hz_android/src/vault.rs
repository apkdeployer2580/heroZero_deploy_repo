//! The save vault — archive-before-overwrite for the game's RecordStores.
//!
//! Every write that replaces a save record first copies the bytes that are about to stop
//! existing (the PRE-image) to `<data>/save_vault/<store>/<record id>/<ms>-<len>.bin` — or, when
//! those bytes cannot be read, moves the record itself there (`<ms>-unreadable.bin`). The vault
//! takes no view on whether the write is legitimate, so it stays correct whatever the rest of the
//! stack gets wrong; in a destructive event the newest write IS the wipe, and only a pre-image
//! never has to know which generation was the good one.
//!
//! Policy is pure functions ([`is_vaulted`], [`should_vault`], [`parse_gen`], [`retain`]) with
//! selftests; [`before_overwrite`] gathers the facts and does the I/O. The files are plain copies,
//! so a restore is a file copy before the game boots and needs nothing from it (`crate::saves`);
//! the generation it put back is PINNED ([`PIN_FILE`]) so the restore costs one seat, not three.
//!
//! What 영웅서기 Zero writes (javap, 2026-10-22): only `addRecord` / `setRecord` — never
//! `deleteRecord` or `deleteRecordStore` — record 1 of each store. One save slot is THREE stores
//! written together (`HOL.slot<n>` 45 B summary the slot list reads, `HOL.gamedata<n>` 379 B,
//! `HOL.item<n>` 117 B, sizes from the probe's first save), so a restore must put back all three
//! from the same moment.

use std::{
    fs,
    path::{Path, PathBuf},
};

/// Whitelist, by prefix of the store name: everything the game keeps under its own `HOL.` names —
/// the three per-slot stores, `HOL.storage` (창고) and `HOL.gamestore`, whose meaning is not
/// pinned yet (23 B, rewritten at boot) and which costs nothing to keep. NOT the `TELEM_*` stores
/// (QQ telemetry) or the one-letter `d`.
pub fn is_vaulted(store: &str) -> bool {
    store.starts_with("HOL.")
}

/// Recency tier: the newest N generations, unconditionally.
pub const KEEP_NEWEST: usize = 8;
/// Calendar tier: the newest generation of each of the last D days (UTC). Reports arrive a day or
/// more after the loss; "the newest 8" alone cannot survive that.
pub const KEEP_DAYS: u64 = 7;
/// Ceiling, whatever the tiers say: the listing runs inside a guest storage call.
pub const KEEP_MAX: usize = 20;
/// Minimum spacing between two archived OVERWRITES of one record. A DELETE is exempt — it is the
/// last thing that ever happens to that record.
///
/// ⚠ The three tiers and this interval are first guesses (2026-10-22): this game saves from the
/// menu, and whether it also autosaves (on a map change, a shop) is not measured yet. Re-derive
/// them from the device logs' write cadence before trusting them.
pub const MIN_INTERVAL_MS: u64 = 30_000;

const DAY_MS: u64 = 24 * 60 * 60 * 1000;

/// The sidecar naming the generation a restore put back (its file name, one line). [`parse_gen`]
/// rejects the name, so no listing, export or rotation ever sees it as a save; retention keeps the
/// generation it names outside the tiers but inside the ceiling, and a pre-image identical to it
/// is not archived again (save-integrity: "the restore that ate the backups").
pub const PIN_FILE: &str = "pinned";

/// Whether to archive `old` before it is replaced by `new` (`None` = a delete).
pub fn should_vault(old: &[u8], new: Option<&[u8]>, last_vault_ms: u64, now_ms: u64) -> bool {
    if old.is_empty() {
        return false;
    }
    match new {
        // ★ A delete is never skipped — not by the rate limit, not by "identical".
        None => true,
        Some(new) => {
            if new == old {
                return false;
            }
            last_vault_ms == 0 || now_ms.saturating_sub(last_vault_ms) >= MIN_INTERVAL_MS
        }
    }
}

/// `<14-digit unix ms>-<length>.bin`: a lexicographic listing is a chronological one, and the
/// length column names the moment a slot started holding a different character.
pub fn gen_name(ms: u64, len: usize) -> String {
    format!("{ms:014}-{len}.bin")
}

/// `<14-digit unix ms>-unreadable.bin`: a record whose bytes could not be read, moved here whole
/// before a write replaced it ([`before_overwrite`]). Not a [`parse_gen`] name, so retention never
/// prunes it — it may be the only copy of a character, and nothing here can tell.
pub fn quarantine_name(ms: u64) -> String {
    format!("{ms:014}-unreadable.bin")
}

/// The inverse of [`gen_name`]; anything else in the directory is not a generation and is left
/// alone.
pub fn parse_gen(name: &str) -> Option<(u64, usize)> {
    let stem = name.strip_suffix(".bin")?;
    let (ms, len) = stem.split_once('-')?;
    if ms.len() != 14 || !ms.bytes().all(|b| b.is_ascii_digit()) || len.is_empty() || !len.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((ms.parse().ok()?, len.parse().ok()?))
}

/// Which generations (by timestamp) to keep, given all of them and the pinned one. Pure: the
/// retention rule — the pin is kept whatever the tiers say and counts toward the ceiling.
pub fn retain(mut gens: Vec<u64>, now_ms: u64, pinned: Option<u64>) -> Vec<u64> {
    let pinned = pinned.filter(|p| gens.contains(p));
    gens.retain(|g| Some(*g) != pinned);
    gens.sort_unstable_by(|a, b| b.cmp(a));
    gens.dedup();
    let mut keep: Vec<u64> = gens.iter().copied().take(KEEP_NEWEST).collect();
    let today = now_ms / DAY_MS;
    let mut days_seen = std::collections::BTreeSet::new();
    for &g in &gens {
        let day = g / DAY_MS;
        if today.saturating_sub(day) < KEEP_DAYS && days_seen.insert(day) && !keep.contains(&g) {
            keep.push(g);
        }
    }
    keep.sort_unstable_by(|a, b| b.cmp(a));
    keep.truncate(KEEP_MAX - usize::from(pinned.is_some()));
    keep.extend(pinned);
    keep
}

/// The generation the pin names in `dir`, as `(timestamp, file name)`.
pub fn read_pin(dir: &Path) -> Option<(u64, String)> {
    let name = fs::read_to_string(dir.join(PIN_FILE)).ok()?.trim().to_string();
    let (ms, _) = parse_gen(&name)?;
    Some((ms, name))
}

/// Pin generation `name` in `dir` (a restore put it back).
pub fn write_pin(dir: &Path, name: &str) -> std::io::Result<()> {
    crate::database::write_atomic(&dir.join(PIN_FILE), name.as_bytes())
}

/// `<data>/save_vault/<store>/<id>/`. Derived from the record's own path components, never from a
/// remembered root.
pub fn dir_for(data_dir: &Path, store: &str, id: u32) -> PathBuf {
    data_dir.join("save_vault").join(store).join(id.to_string())
}

fn newest_gen(dir: &Path) -> Option<u64> {
    fs::read_dir(dir).ok()?.filter_map(|e| e.ok()).filter_map(|e| parse_gen(e.file_name().to_str()?).map(|(ms, _)| ms)).max()
}

/// Archive the record at `record_path` (store `store`, record `id`) before it is replaced by
/// `new` (`None` = deleted). Errors are logged and swallowed: a vault that cannot write must never
/// be the reason a save fails.
///
/// Returns `true` when the record is no longer at `record_path`: its bytes could not be read, so
/// the file itself went into the vault ([`quarantine_name`]) instead of a copy.
pub fn before_overwrite(data_dir: &Path, store: &str, id: u32, record_path: &Path, new: Option<&[u8]>, now_ms: u64) -> bool {
    if !is_vaulted(store) {
        return false;
    }
    let old = match fs::read(record_path) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
        Err(e) => return quarantine(data_dir, store, id, record_path, &e, now_ms),
    };
    let dir = dir_for(data_dir, store, id);
    let last = newest_gen(&dir).unwrap_or(0);
    if !should_vault(&old, new, last, now_ms) {
        return false;
    }
    // The first save after a restore writes over the restored bytes, which the vault already holds
    // (the pin) — another copy would only spend a seat. A delete is still taken.
    if new.is_some()
        && let Some((_, name)) = read_pin(&dir)
        && fs::read(dir.join(&name)).is_ok_and(|pinned| pinned == old)
    {
        return false;
    }
    if let Err(e) = fs::create_dir_all(&dir) {
        log::error!("vault: cannot create {dir:?}: {e}");
        return false;
    }
    // A clock that went backwards must not overwrite or sort below an existing generation.
    let ms = now_ms.max(last + 1);
    let path = dir.join(gen_name(ms, old.len()));
    if let Err(e) = crate::database::write_atomic(&path, &old) {
        log::error!("vault: cannot write {path:?}: {e}");
        return false;
    }
    log::warn!(
        "vault: kept {store}#{id} {} B before {} ({path:?})",
        old.len(),
        match new {
            Some(n) => format!("an overwrite with {} B", n.len()),
            None => "a DELETE".to_string(),
        }
    );
    prune(&dir, now_ms);
    if new.is_some() {
        keep_slot_together(data_dir, store, id, record_path, now_ms);
    }
    false
}

/// The other two of a save slot's three stores when `store` is its summary (`HOL.slot<n>`) or
/// payload (`HOL.gamedata<n>`) — the two only a save writes; a 계정창고 move writes the bag
/// (`HOL.item<n>`) alone and is no save.
fn slot_partners(store: &str) -> Option<[String; 2]> {
    let (n, summary) = match store.strip_prefix("HOL.slot") {
        Some(n) => (n, true),
        None => (store.strip_prefix("HOL.gamedata")?, false),
    };
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let other = if summary { format!("HOL.gamedata{n}") } else { format!("HOL.slot{n}") };
    Some([other, format!("HOL.item{n}")])
}

/// A save is a restore point only with a generation of each of its slot's three stores at one
/// moment (`crate::saves::points`), and [`before_overwrite`] takes a pre-image only of bytes that
/// change — a save that left the bag as it was would be no point. So when a save's summary or
/// payload goes in, the other two go in with it, as they are on disk now (their pre-images: the
/// game writes summary → payload → bag in one frame). Once a save, not once a write: a partner that
/// already has a generation inside the point window is left alone, and the partners' own writes in
/// this save then fall inside [`MIN_INTERVAL_MS`] and add nothing. Nor when the partner holds the
/// pinned bytes — a restore's generation, already a point.
fn keep_slot_together(data_dir: &Path, store: &str, id: u32, record_path: &Path, now_ms: u64) {
    let Some(partners) = slot_partners(store) else { return };
    // `<db>/<store>/<id>`, as `crate::database` keeps it — derived from the record's own path.
    let Some(db) = record_path.parent().filter(|p| p.file_name().is_some_and(|n| n == store)).and_then(Path::parent) else { return };
    for partner in partners {
        let dir = dir_for(data_dir, &partner, id);
        if newest_gen(&dir).is_some_and(|ms| ms.abs_diff(now_ms) <= crate::saves::POINT_WINDOW_MS) {
            continue;
        }
        let path = db.join(&partner).join(id.to_string());
        if let Some((_, name)) = read_pin(&dir)
            && let (Ok(pinned), Ok(now)) = (fs::read(dir.join(&name)), fs::read(&path))
            && pinned == now
        {
            continue;
        }
        copy_now(data_dir, &partner, id, &path, now_ms, "with its slot's save");
    }
}

/// Archive the record at `record_path` NOW, whatever the interval says — a restore or an import is
/// about to replace it (save-integrity: "vault the current bytes first, unconditionally") — and a
/// new generation even when the newest already holds these bytes: both keep a slot's three stores
/// at one moment, and a restore point pairs generations by time (`crate::saves::points`) — an older
/// copy of the same bytes pairs with nothing. Not when there is nothing there. Returns the
/// generation written, if one was.
pub fn archive_fresh(data_dir: &Path, store: &str, id: u32, record_path: &Path, now_ms: u64) -> Option<PathBuf> {
    copy_now(data_dir, store, id, record_path, now_ms, "before a RESTORE or IMPORT")
}

fn copy_now(data_dir: &Path, store: &str, id: u32, record_path: &Path, now_ms: u64, why: &str) -> Option<PathBuf> {
    let old = fs::read(record_path).ok().filter(|b| !b.is_empty())?;
    let dir = dir_for(data_dir, store, id);
    let newest = newest_gen(&dir);
    fs::create_dir_all(&dir).ok()?;
    let ms = now_ms.max(newest.map_or(0, |ms| ms + 1));
    let path = dir.join(gen_name(ms, old.len()));
    match crate::database::write_atomic(&path, &old) {
        Ok(()) => {
            log::warn!("vault: kept {store}#{id} {} B {why} ({path:?})", old.len());
            prune(&dir, now_ms);
            Some(path)
        }
        Err(e) => {
            log::error!("vault: cannot write {path:?} {why}: {e}");
            None
        }
    }
}

/// A record that exists but cannot be read is not an empty one. The game cannot tell the two
/// apart — `ac.a()` catches the exception and the slot shows as 빈 슬롯 (javap, 2026-10-23) — so
/// the player starts over in it, and the next save would replace the only copy without a vault
/// generation (there were no bytes to copy). A rename needs no read access, so the file itself
/// moves aside. Every write is rate-exempt here: this is never a redundant copy.
fn quarantine(data_dir: &Path, store: &str, id: u32, record_path: &Path, read_err: &std::io::Error, now_ms: u64) -> bool {
    let dir = dir_for(data_dir, store, id);
    if let Err(e) = fs::create_dir_all(&dir) {
        log::error!("vault: {store}#{id} cannot be read ({read_err}) and {dir:?} cannot be created ({e}) — this write replaces it unarchived");
        return false;
    }
    // A rename onto an existing file would replace it: never reuse a name.
    let mut ms = now_ms;
    while dir.join(quarantine_name(ms)).exists() {
        ms += 1;
    }
    let path = dir.join(quarantine_name(ms));
    match fs::rename(record_path, &path) {
        Ok(()) => {
            log::warn!("vault: {store}#{id} cannot be read ({read_err}) — moved whole to {path:?} before the write");
            true
        }
        Err(e) => {
            log::error!("vault: {store}#{id} cannot be read ({read_err}) nor moved aside ({e}) — this write replaces it unarchived");
            false
        }
    }
}

fn prune(dir: &Path, now_ms: u64) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let gens: Vec<(u64, PathBuf)> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name();
            let (ms, _) = parse_gen(name.to_str()?)?;
            Some((ms, e.path()))
        })
        .collect();
    let keep = retain(gens.iter().map(|(ms, _)| *ms).collect(), now_ms, read_pin(dir).map(|(ms, _)| ms));
    for (ms, path) in gens {
        if !keep.contains(&ms)
            && let Err(e) = fs::remove_file(&path)
        {
            log::warn!("vault: cannot prune {path:?}: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_games_own_stores_are_vaulted() {
        for s in ["HOL.gamedata1", "HOL.slot3", "HOL.item2", "HOL.storage", "HOL.gamestore"] {
            assert!(is_vaulted(s), "{s}");
        }
        for s in ["TELEM_INFO", "TELEM_LOTTERY", "d", "hol.gamedata1", "xHOL.slot1"] {
            assert!(!is_vaulted(s), "{s}");
        }
    }

    #[test]
    fn empty_and_identical_pre_images_are_skipped_on_overwrite() {
        assert!(!should_vault(b"", Some(b"new"), 0, 1_000_000));
        assert!(!should_vault(b"same", Some(b"same"), 0, 1_000_000));
        assert!(should_vault(b"old", Some(b"new"), 0, 1_000_000));
    }

    #[test]
    fn overwrites_are_rate_limited() {
        let t = 10_000_000;
        assert!(!should_vault(b"old", Some(b"new"), t, t + MIN_INTERVAL_MS - 1));
        assert!(should_vault(b"old", Some(b"new"), t, t + MIN_INTERVAL_MS));
    }

    #[test]
    fn a_delete_is_exempt_from_the_rate_limit() {
        let t = 10_000_000;
        assert!(should_vault(b"old", None, t, t + 1));
    }

    #[test]
    fn the_identical_rule_does_not_leak_into_the_delete_arm() {
        // There is no "new" to compare with on a delete — and a pre-image that equals the last
        // archived one is still the last copy of the record once the delete lands.
        assert!(should_vault(b"same", None, 0, 1));
        assert!(!should_vault(b"", None, 0, 1), "nothing to lose");
    }

    #[test]
    fn generation_names_round_trip_and_sort_chronologically() {
        assert_eq!(gen_name(1_756_100_480_123, 379), "01756100480123-379.bin");
        assert_eq!(parse_gen("01756100480123-379.bin"), Some((1_756_100_480_123, 379)));
        let mut names = vec![gen_name(20_000, 1), gen_name(3_000, 1), gen_name(100_000, 1)];
        names.sort();
        assert_eq!(names.iter().map(|n| parse_gen(n).unwrap().0).collect::<Vec<_>>(), [3_000, 20_000, 100_000]);
    }

    #[test]
    fn junk_in_the_directory_is_not_a_generation() {
        for junk in ["1756100480123-379.bin", "01756100480123-379.tmp", "01756100480123_379.bin", "01756100480123-.bin", "0175610048012x-379.bin", "pin", "01756100480123-37a.bin"] {
            assert_eq!(parse_gen(junk), None, "{junk}");
        }
        // …which is what keeps a quarantined record out of retention's reach.
        assert_eq!(parse_gen(&quarantine_name(1_756_100_480_123)), None);
    }

    #[test]
    fn retention_keeps_the_newest_and_one_per_recent_day_under_the_ceiling() {
        // Midday: "a minute ago" must still be today for the test to mean what it says.
        let now = 30 * DAY_MS + DAY_MS / 2;
        // 12 writes today, one on each of the 10 days before.
        let mut gens: Vec<u64> = (0..12).map(|i| now - i * 60_000).collect();
        gens.extend((1..=10).map(|d| now - d * DAY_MS));
        let keep = retain(gens, now, None);
        assert!(keep.len() <= KEEP_MAX);
        for i in 0..KEEP_NEWEST as u64 {
            assert!(keep.contains(&(now - i * 60_000)), "recency {i}");
        }
        for d in 1..KEEP_DAYS {
            assert!(keep.contains(&(now - d * DAY_MS)), "day {d}");
        }
        assert!(!keep.contains(&(now - 10 * DAY_MS)), "older than the calendar tier");
    }

    /// Rotation is the one part of the vault that can itself destroy a save (save-vault §14): over
    /// seeded random corpora, with and without a pin, the newest is never pruned, the ceiling holds,
    /// a second pass prunes nothing, and a pinned generation is always kept.
    #[test]
    fn retention_properties_over_random_corpora() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let now = 400 * DAY_MS + 12_345;
        for size in [0usize, 1, 5, KEEP_NEWEST, KEEP_MAX, KEEP_MAX + 7, 60] {
            for _ in 0..40 {
                let gens: Vec<u64> = (0..size).map(|_| now - next() % (20 * DAY_MS)).collect();
                for pinned in [None, gens.get(next() as usize % size.max(1)).copied()] {
                    let keep = retain(gens.clone(), now, pinned);
                    assert!(keep.len() <= KEEP_MAX, "size {size}: {}", keep.len());
                    if let Some(&newest) = gens.iter().max() {
                        assert!(keep.contains(&newest), "the newest is never pruned");
                    }
                    if let Some(p) = pinned {
                        assert!(keep.contains(&p), "the pin is kept");
                    }
                    let mut again = retain(keep.clone(), now, pinned);
                    let mut once = keep.clone();
                    again.sort_unstable();
                    once.sort_unstable();
                    assert_eq!(again, once, "a second pass prunes nothing");
                }
            }
        }
    }

    #[test]
    fn before_overwrite_archives_the_pre_image_once_per_interval() {
        let data = std::env::temp_dir().join(format!("hz_vault_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&data);
        let rec = data.join("rec");
        fs::create_dir_all(&data).unwrap();
        fs::write(&rec, b"generation-1").unwrap();
        let t = 1_756_100_480_123;
        before_overwrite(&data, "HOL.gamedata1", 1, &rec, Some(b"generation-2"), t);
        before_overwrite(&data, "HOL.gamedata1", 1, &rec, Some(b"generation-3"), t + 5);
        let dir = dir_for(&data, "HOL.gamedata1", 1);
        let gens: Vec<_> = fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().into_string().unwrap()).collect();
        assert_eq!(gens, vec![gen_name(t, 12)], "the second overwrite fell inside the interval");
        assert_eq!(fs::read(dir.join(gen_name(t, 12))).unwrap(), b"generation-1");
        // …but a delete right after is still kept.
        before_overwrite(&data, "HOL.gamedata1", 1, &rec, None, t + 6);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        // Not vaulted: telemetry.
        before_overwrite(&data, "TELEM_INFO", 1, &rec, Some(b"x"), t);
        assert!(!dir_for(&data, "TELEM_INFO", 1).exists());
        let _ = fs::remove_dir_all(&data);
    }

    #[test]
    fn an_unreadable_record_is_moved_aside_whole_and_never_pruned() {
        let data = std::env::temp_dir().join(format!("hz_vault_quarantine_{}", std::process::id()));
        let _ = fs::remove_dir_all(&data);
        // Unreadable but present: a DIRECTORY where the record file is (`fs::read` fails with
        // something other than NotFound on every host), with content to recognise it by.
        let rec = data.join("rec");
        fs::create_dir_all(&rec).unwrap();
        fs::write(rec.join("payload"), b"the character").unwrap();
        let t = 1_756_100_480_123;
        assert!(before_overwrite(&data, "HOL.gamedata1", 1, &rec, Some(b"new game"), t));
        assert!(!rec.exists(), "the write that follows lands on a free path");
        let dir = dir_for(&data, "HOL.gamedata1", 1);
        let kept = dir.join(quarantine_name(t));
        assert_eq!(fs::read(kept.join("payload")).unwrap(), b"the character");
        // A second one in the same millisecond does not replace the first.
        fs::create_dir_all(&rec).unwrap();
        assert!(before_overwrite(&data, "HOL.gamedata1", 1, &rec, None, t));
        assert!(dir.join(quarantine_name(t + 1)).exists());
        // Retention runs over a full directory of generations and leaves both alone.
        let (file, late) = (data.join("rec_file"), t + 40 * DAY_MS);
        fs::write(&file, b"g").unwrap();
        for i in 0..(KEEP_MAX as u64 + 5) {
            assert!(!before_overwrite(&data, "HOL.gamedata1", 1, &file, Some(b"h"), late + i * MIN_INTERVAL_MS));
        }
        assert_eq!(fs::read_dir(&dir).unwrap().filter_map(|e| parse_gen(e.ok()?.file_name().to_str()?)).count(), KEEP_NEWEST, "pruned (one day's writes)");
        assert!(kept.join("payload").exists() && dir.join(quarantine_name(t + 1)).exists());
        // Not vaulted: nothing moves.
        fs::create_dir_all(&rec).unwrap();
        assert!(!before_overwrite(&data, "TELEM_INFO", 1, &rec, Some(b"x"), t));
        assert!(rec.exists());
        let _ = fs::remove_dir_all(&data);
    }
}
