//! The player's way back to a save (2026-10-25, the hero2/hero3 list's "세이브 복원·내보내기"):
//! restore a slot from the vault (`crate::vault`), see that a slot went missing while the vault
//! still holds it, and copy every save and its vault out where a release build can reach them.
//!
//! The rules are save-integrity's (`/featurephone-port:save-integrity`, save-vault §9–§13):
//!
//! * A slot is THREE stores written together — `HOL.slot<n>` (the 45 B summary the slot list
//!   reads), `HOL.gamedata<n>`, `HOL.item<n>` — so a restore point is one generation of each from
//!   the same save ([`points`]); a point missing one of them is never offered, a lone summary
//!   least of all (it would list a character that is not there).
//! * A restore is applied at the NEXT launch, before the game opens a store ([`apply_pending`]):
//!   the stores may be open now, and the game rewrites what it holds. The request names the exact
//!   generations, checked on disk when it is written; at boot the request file is deleted FIRST,
//!   then applied — a restore that dies half-way must not run again on every launch.
//! * The bytes it replaces are vaulted first, whatever the interval, the slot's three at one moment
//!   (`vault::archive_fresh`) so the restore is itself a point to go back to — and a record that
//!   could not be kept is not overwritten; the generations it puts back are pinned
//!   (`vault::PIN_FILE`) so rotation cannot take them and the next save does not archive them twice.
//! * Export is the player's own act, copies files whole (they are the game's encrypted records),
//!   and names every file so a listing alone says when, which slot and how big.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::vault;

/// The save slots: the game's three and HZ-020's pages of three more (`HOL.slot1..10`).
pub const SLOTS: u8 = hz_patches::hooks::SAVE_SLOTS;
/// The only record the game writes in a store (javap, 2026-10-22: record 1 of each).
const RECORD: u32 = 1;
/// The request a restore leaves for the next launch, one line per store: `<store> <generation>`.
pub const REQUEST_FILE: &str = "hz_restore.txt";
/// How far apart one save's three writes may land and still be one point (`slot` → `gamedata` →
/// `item` within one frame of the game's save, so a few ms; the vault's 30 s rate limit is per
/// store, and the three fall in or out of it together).
pub const POINT_WINDOW_MS: u64 = 3_000;
/// Where an export goes under the app's external files directory.
pub const EXPORT_DIR: &str = "save_export";

/// A slot's three stores, summary first.
pub fn stores(slot: u8) -> [String; 3] {
    [format!("HOL.slot{slot}"), format!("HOL.gamedata{slot}"), format!("HOL.item{slot}")]
}

/// One moment a slot can be put back to: a generation `(ms, len)` of each of its three stores.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    /// The payload's (`HOL.gamedata<n>`) generation time: what the list shows.
    pub ms: u64,
    pub gens: [(u64, usize); 3],
}

/// The restore points of one slot from each store's generations (any order). Pure: for each
/// payload generation, the summary and bag generation nearest to it within [`POINT_WINDOW_MS`] —
/// none, no point. Newest first.
pub fn points(slot: &[(u64, usize)], gamedata: &[(u64, usize)], item: &[(u64, usize)]) -> Vec<Point> {
    let near = |list: &[(u64, usize)], ms: u64| list.iter().copied().filter(|(t, _)| t.abs_diff(ms) <= POINT_WINDOW_MS).min_by_key(|(t, _)| t.abs_diff(ms));
    let mut out: Vec<Point> = gamedata
        .iter()
        .filter_map(|&(ms, len)| Some(Point { ms, gens: [near(slot, ms)?, (ms, len), near(item, ms)?] }))
        .collect();
    out.sort_by(|a, b| b.ms.cmp(&a.ms));
    out
}

/// `<data>/<app id>/db/<store>/1`, as `crate::database` keeps it.
pub fn record_path(data_dir: &Path, store: &str) -> PathBuf {
    data_dir.join(crate::JAR_NAME).join("db").join(store).join(RECORD.to_string())
}

/// The generations in a store's vault directory.
fn generations(data_dir: &Path, store: &str) -> Vec<(u64, usize)> {
    let Ok(rd) = fs::read_dir(vault::dir_for(data_dir, store, RECORD)) else { return Vec::new() };
    rd.filter_map(|e| e.ok()).filter_map(|e| vault::parse_gen(e.file_name().to_str()?)).collect()
}

/// What a slot holds now and what it can go back to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Slot {
    pub slot: u8,
    /// Each store's record length now (`None` = no record).
    pub now: [Option<usize>; 3],
    pub points: Vec<Point>,
}

impl Slot {
    /// The slot list would show it as 빈 슬롯 (its summary or payload is gone) while the vault
    /// still has a point to go back to — the boot check's sentence (save-vault §13).
    pub fn missing_but_recoverable(&self) -> bool {
        (self.now[0].is_none() || self.now[1].is_none()) && !self.points.is_empty()
    }
}

/// Every slot, read from the files (never from anything this process remembers).
pub fn slots(data_dir: &Path) -> Vec<Slot> {
    (1..=SLOTS)
        .map(|slot| {
            let names = stores(slot);
            let now = names.clone().map(|s| fs::metadata(record_path(data_dir, &s)).ok().map(|m| m.len() as usize));
            let [a, b, c] = names.map(|s| generations(data_dir, &s));
            Slot { slot, now, points: points(&a, &b, &c) }
        })
        .collect()
}

/// The slot played last (its payload record written most recently), 1-based — where 이어하기
/// should start (hero3 161차: the list opens at the last-loaded slot).
pub fn last_played(data_dir: &Path) -> Option<u8> {
    (1..=SLOTS)
        .filter_map(|n| Some((fs::metadata(record_path(data_dir, &format!("HOL.gamedata{n}"))).ok()?.modified().ok()?, n)))
        .max()
        .map(|(_, n)| n)
}

/// Ask for slot `slot` to go back to `point` at the next launch. Checks that every generation is
/// still in the vault (a stale list must not schedule a rotated-out one). Cancels an import that was
/// waiting: one thing is put back at a launch, and it is the last one asked for.
pub fn request_restore(data_dir: &Path, slot: u8, point: &Point) -> std::io::Result<()> {
    let mut text = String::new();
    for (store, (ms, len)) in stores(slot).iter().zip(point.gens) {
        let name = vault::gen_name(ms, len);
        let path = vault::dir_for(data_dir, store, RECORD).join(&name);
        if !path.is_file() {
            return Err(std::io::Error::new(std::io::ErrorKind::NotFound, format!("{store}: {name} is no longer in the vault")));
        }
        text += &format!("{store} {name}\n");
    }
    crate::database::write_atomic(&data_dir.join(REQUEST_FILE), text.as_bytes())?;
    if fs::remove_file(import_path(data_dir)).is_ok() {
        log::warn!("saves: the import waiting is cancelled — a restore was asked for after it");
    }
    log::warn!("saves: restore of slot {slot} to {} requested for the next launch", point.ms);
    Ok(())
}

/// What the boot's restore did, for the log and the player.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Applied {
    /// No request waiting.
    None,
    /// Every store of the request was put back.
    Done(Vec<String>),
    /// The request could not be carried out whole; what was put back, and why the rest was not.
    Partial(Vec<String>, String),
}

/// At boot, before the game opens any store: carry out a waiting restore. The request file goes
/// first, so a restore that fails half-way is never retried on its own. The import's rules after
/// that: the whole request is read before anything is written, the slot as it is now goes into the
/// vault first (its three at one moment — [`keep_changing`] — so the restore can be undone), a
/// record that could not be kept is not overwritten, and a write that fails puts back the ones
/// written before it.
pub fn apply_pending(data_dir: &Path, now_ms: u64) -> Applied {
    let req = data_dir.join(REQUEST_FILE);
    let Ok(text) = fs::read_to_string(&req) else { return Applied::None };
    if let Err(e) = fs::remove_file(&req) {
        // Never apply what could not be consumed: it would run again on every launch.
        log::error!("saves: restore request {req:?} cannot be removed ({e}) — not applied");
        return Applied::Partial(Vec::new(), format!("the request could not be removed: {e}"));
    }
    // 1. Every line and the generation it names.
    let mut plan: Vec<(String, String, Vec<u8>)> = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let Some((store, name)) = line.trim().split_once(' ') else {
            return Applied::Partial(Vec::new(), format!("bad request line {line:?}"));
        };
        if !vault::is_vaulted(store) || vault::parse_gen(name).is_none() {
            return Applied::Partial(Vec::new(), format!("bad request line {line:?}"));
        }
        match fs::read(vault::dir_for(data_dir, store, RECORD).join(name)) {
            Ok(bytes) => plan.push((store.to_string(), name.to_string(), bytes)),
            Err(e) => return Applied::Partial(Vec::new(), format!("{store} {name}: {e}")),
        }
    }
    // 2. Pin what goes back before the vault grows: the generation keeping the slot adds must not
    //    rotate it out.
    for (store, name, _) in &plan {
        if let Err(e) = vault::write_pin(&vault::dir_for(data_dir, store, RECORD), name) {
            log::warn!("saves: {store} {name} not pinned: {e}");
        }
    }
    // 3. What the records hold now. One that cannot be read goes into the vault whole (the vault's
    //    quarantine): a slot the game shows empty may be why the player is restoring.
    let mut before: Vec<Option<Vec<u8>>> = Vec::with_capacity(plan.len());
    for (store, _, bytes) in &plan {
        let record = record_path(data_dir, store);
        match fs::read(&record) {
            Ok(old) => before.push(Some(old)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => before.push(None),
            Err(e) if vault::before_overwrite(data_dir, store, RECORD, &record, Some(bytes), now_ms) => {
                log::warn!("saves: {store} could not be read ({e}) — moved into the vault whole");
                before.push(None);
            }
            Err(e) => return Applied::Partial(Vec::new(), format!("{store} cannot be read ({e}) nor kept")),
        }
    }
    let changes: Vec<(&str, Option<&[u8]>)> = plan.iter().map(|(s, _, b)| (s.as_str(), Some(b.as_slice()))).collect();
    if let Err(why) = keep_changing(data_dir, &changes, &before, now_ms) {
        return Applied::Partial(Vec::new(), why);
    }
    // 4. The generations; on the first failure every record written so far goes back, newest first.
    for (i, (store, name, bytes)) in plan.iter().enumerate() {
        if let Err(e) = put(&record_path(data_dir, store), Some(bytes)) {
            let stuck: Vec<String> = plan[..i]
                .iter()
                .zip(&before[..i])
                .rev()
                .filter_map(|((s, _, _), old)| put(&record_path(data_dir, s), old.as_deref()).err().map(|_| s.clone()))
                .collect();
            let why = format!("writing {store}: {e}; {}", if stuck.is_empty() { "the others put back".to_string() } else { format!("not put back: {}", stuck.join(", ")) });
            return Applied::Partial(stuck, why);
        }
        log::warn!("saves: {store} restored from {name} ({} B)", bytes.len());
    }
    // 5. HZ-058: the slot's quick plates were the replaced save's (the vault keeps none) — kept, they
    //    would show that save's skills over this one's, and a plate whose skill the restored character
    //    has not learned does nothing. The rows go; its next play starts them from the restored save's
    //    own six slots (`Plates::adopt`), as an import without plates does.
    let restored: Vec<u8> = (1..=SLOTS).filter(|&n| stores(n).iter().any(|s| plan.iter().any(|(p, _, _)| p == s))).collect();
    let mut plates = read_plates(data_dir);
    if plates.rows.keys().any(|o| restored.contains(&o.slot)) {
        plates.replace_slots(&restored, &[]);
        match crate::database::write_atomic(&data_dir.join(hz_java::quickpage::FILE), plates.format().as_bytes()) {
            Ok(()) => log::warn!("saves: the quick plates of slot {restored:?} start again from the restored save"),
            Err(e) => log::error!("saves: the quick plates of slot {restored:?} not reset ({e}) — they are the replaced save's"),
        }
    }
    Applied::Done(plan.into_iter().map(|(s, _, _)| s).collect())
}

/// Copy every save record and the whole vault to `<dest>/save_export/<ms>/` — the only way a
/// release build's saves leave the phone (save-vault §11). Returns the folder and how many files.
pub fn export(data_dir: &Path, dest: &Path, now_ms: u64) -> std::io::Result<(PathBuf, usize)> {
    let out = dest.join(EXPORT_DIR).join(format!("{now_ms:014}"));
    let mut n = 0;
    let db = data_dir.join(crate::JAR_NAME).join("db");
    if let Ok(rd) = fs::read_dir(&db) {
        for e in rd.filter_map(|e| e.ok()) {
            let store = e.file_name().to_string_lossy().into_owned();
            if vault::is_vaulted(&store) {
                n += copy_tree(&e.path(), &out.join("db").join(&store))?;
            }
        }
    }
    let v = data_dir.join("save_vault");
    if v.is_dir() {
        n += copy_tree(&v, &out.join("save_vault"))?;
    }
    log::warn!("saves: exported {n} files to {out:?}");
    Ok((out, n))
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<usize> {
    fs::create_dir_all(to)?;
    let mut n = 0;
    for e in fs::read_dir(from)?.filter_map(|e| e.ok()) {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if src.is_dir() {
            n += copy_tree(&src, &dst)?;
        } else if !e.file_name().to_string_lossy().ends_with(".tmp") {
            fs::copy(&src, &dst)?;
            n += 1;
        }
    }
    Ok(n)
}

// ------------------------------------------- the platform dialogs' side (B3, hero2's) ----
//
// `MainActivity`'s 진단 정보 › 세이브 관리 asks these over JNI (`lib.rs` `saves_jni`), handing over
// its own `getFilesDir()` — the directory `android_main` was given — so each is a plain function of
// the files, read when the dialog asks, never of anything this process remembers.

/// 진단 정보's save lines: how many slots are in use, then each of them — its three records' sizes,
/// its restore points, and whether the slot list would show it empty while the vault has it.
pub fn slot_report(slots: &[Slot]) -> Vec<String> {
    let used: Vec<&Slot> = slots.iter().filter(|s| s.now.iter().any(Option::is_some) || !s.points.is_empty()).collect();
    let mut out = vec![format!("세이브 슬롯 {} / {} 사용", used.len(), slots.len())];
    for s in used {
        let size = |n: Option<usize>| n.map_or("-".to_string(), |n| n.to_string());
        out.push(format!(
            "  슬롯 {}: {} + {} + {} B · 복원 지점 {}개{}",
            s.slot,
            size(s.now[0]),
            size(s.now[1]),
            size(s.now[2]),
            s.points.len(),
            if s.missing_but_recoverable() { " · 지금 없음 (세이브 관리에서 되살릴 수 있음)" } else { "" }
        ));
    }
    out
}

/// The restore points as the 「백업에서 복원」 list reads them (hero2's `offer_line`): a line a point,
/// `slot|ms|bytes|gone` — slot by slot, newest first in each; `bytes` its three records together,
/// `gone` 1 while the slot list would show that slot empty. Empty when nothing can be put back.
pub fn offer_lines(data_dir: &Path) -> String {
    let mut out = String::new();
    for s in slots(data_dir) {
        for p in &s.points {
            let bytes: usize = p.gens.iter().map(|g| g.1).sum();
            out += &format!("{}|{}|{}|{}\n", s.slot, p.ms, bytes, u8::from(s.missing_but_recoverable()));
        }
    }
    out
}

/// Ask for the point an [`offer_lines`] line names (its `slot|ms`; what follows is the player's to
/// read) — found again in the files, so a list that went stale while it was open schedules nothing
/// the vault no longer has. The slot it will put back.
pub fn request_offer(data_dir: &Path, key: &str) -> std::io::Result<u8> {
    let bad = || std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("bad restore key {key:?}"));
    let mut f = key.trim().split('|');
    let slot: u8 = f.next().and_then(|v| v.parse().ok()).ok_or_else(bad)?;
    let ms: u64 = f.next().and_then(|v| v.parse().ok()).ok_or_else(bad)?;
    let point = slots(data_dir)
        .into_iter()
        .find(|s| s.slot == slot)
        .and_then(|s| s.points.into_iter().find(|p| p.ms == ms))
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, format!("slot {slot} has no point {ms} now")))?;
    request_restore(data_dir, slot, &point)?;
    Ok(slot)
}

/// 「세이브 폴더로 내보내기」: [`export`] into the app's external files directory, and what the
/// dialog says about it.
pub fn export_sentence(data_dir: &Path, ext_dir: Option<&Path>, now_ms: u64) -> String {
    match ext_dir.map(|d| export(data_dir, d, now_ms)) {
        Some(Ok((dir, n))) => format!(
            "세이브 파일 {n}개를 복사했습니다.\n\n{}\n\n이 폰을 PC 에 USB 로 연결하면 이 폴더에서 꺼낼 수 있습니다. 지금 세이브와 금고(백업)가 모두 들어 있습니다.",
            dir.display()
        ),
        Some(Err(e)) => {
            log::error!("saves: export failed: {e}");
            format!("내보내지 못했습니다 — 저장소 오류.\n\n{e}")
        }
        None => "내보내지 못했습니다 — 이 기기에는 앱이 쓸 외부 저장소가 없습니다.".to_string(),
    }
}

// ------------------------------------------------- 세이브 옮기기 (B4, hero2's `vault.rs`) ----
//
// `crate::save_transfer` is the file; this is where its records come from and go to. Export reads the
// live records. An import is STAGED by the dialog and APPLIED at the next launch, before the game opens
// a store — the restore's reason (above): the running game holds its character in its heap and would
// write it back over the import. Before an import writes anything, every record it will replace goes
// into the vault (`vault::archive_fresh`, the three of a slot at one moment — a restore point
// 「백업에서 복원」 offers), and every record it owns is read first, so a write that fails half-way puts
// all of them back. ⛔ The vault is not best effort here: a record that could not be kept is not
// overwritten.

/// The game this app runs, as a transfer file names it (`hz_patches::Build::jar_md5`) — set by
/// whoever identifies the jar first: the worker at boot, or `android_main` when an import waits.
static GAME: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

pub fn set_game(md5: &'static str) {
    let _ = GAME.set(md5);
}

pub fn game() -> Option<&'static str> {
    GAME.get().copied()
}

/// The staged import, under the data dir: `<data>/save_import/pending.zip`.
const IMPORT_DIR: &str = "save_import";
const IMPORT_FILE: &str = "pending.zip";

fn import_path(data_dir: &Path) -> PathBuf {
    data_dir.join(IMPORT_DIR).join(IMPORT_FILE)
}

/// Is an import waiting for this launch?
pub fn import_pending(data_dir: &Path) -> bool {
    import_path(data_dir).is_file()
}

/// When slot `n` was last saved: its payload record's time (unix ms).
fn saved_ms(data_dir: &Path, n: u8) -> Option<u64> {
    let t = fs::metadata(record_path(data_dir, &format!("HOL.gamedata{n}"))).ok()?.modified().ok()?;
    t.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_millis() as u64)
}

/// The slots this phone holds now (any of their three records), with when each was saved.
fn live_slots(data_dir: &Path) -> Vec<(u8, u64)> {
    (1..=SLOTS)
        .filter(|&n| stores(n).iter().any(|s| record_path(data_dir, s).is_file()))
        .map(|n| (n, saved_ms(data_dir, n).unwrap_or(0)))
        .collect()
}

/// The transfer file of this phone's records, and the slots it carries. `Err` is a sentence for the
/// player. Every record `save_transfer::names` lists that exists goes, byte for byte — except a slot
/// that is not whole: it stays behind (it could only show a character that is not there on the
/// other phone), and the slots that went are returned so the dialog can say so.
pub fn export_bundle(data_dir: &Path, game: Option<&str>, app: u32, created_ms: u64) -> Result<(Vec<u8>, Vec<u8>), String> {
    let Some(game) = game.filter(|g| crate::save_transfer::game_ok(g)) else {
        log::warn!("save transfer: the game is not identified ({game:?})");
        return Err("앱의 게임 정보를 읽지 못했습니다.".to_string());
    };
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for store in crate::save_transfer::names() {
        match fs::read(record_path(data_dir, &store)) {
            Ok(bytes) => files.push((store, bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                log::warn!("save transfer: cannot read {store}: {e}");
                return Err("세이브 파일을 읽지 못했습니다.".to_string());
            }
        }
    }
    let whole = |n: u8| stores(n).iter().all(|s| files.iter().any(|(f, b)| f == s && !b.is_empty()));
    let slots: Vec<u8> = (1..=SLOTS).filter(|&n| whole(n)).collect();
    files.retain(|(f, _)| (1..=SLOTS).all(|n| !stores(n).contains(f) || slots.contains(&n)));
    if slots.is_empty() {
        return Err("내보낼 세이브가 없습니다. 게임에서 먼저 저장해 주세요.".to_string());
    }
    let saved = slots.iter().map(|&n| (n, saved_ms(data_dir, n).unwrap_or(0))).collect();
    let (legend, recipes, specials) = KNOWN;
    let quick = read_plates(data_dir).lines_for(&slots);
    let bundle = crate::save_transfer::Bundle { game: game.to_string(), created_ms, app, legend, recipes, specials, saved, quick, files };
    let bytes = crate::save_transfer::encode(&bundle)?;
    // Read back with the importer before anyone sends it anywhere: a file this build writes but
    // cannot read is worse than none.
    crate::save_transfer::decode(&bytes)?;
    Ok((bytes, slots))
}

/// [`export_bundle`] into `out`, whole or not at all. The slots that went.
pub fn export_to(data_dir: &Path, game: Option<&str>, app: u32, out: &Path, now_ms: u64) -> Result<Vec<u8>, String> {
    let (bytes, slots) = export_bundle(data_dir, game, app, now_ms)?;
    let written = out.parent().map_or(Ok(()), fs::create_dir_all).and_then(|()| crate::database::write_atomic(out, &bytes));
    if let Err(e) = written {
        log::warn!("save transfer: cannot write {out:?}: {e}");
        return Err("세이브 파일을 쓰지 못했습니다. 저장 공간을 확인해 주세요.".to_string());
    }
    log::warn!("save transfer: exported slots {slots:?} ({} B) to {out:?}", bytes.len());
    Ok(slots)
}

/// A transfer file the player picked, read with the ceiling `save_transfer::decode` holds.
fn read_transfer_file(path: &Path) -> Result<Vec<u8>, String> {
    if fs::metadata(path).is_ok_and(|m| m.len() > crate::save_transfer::FILE_MAX as u64) {
        return Err("영웅서기 Zero 세이브 파일이 아닙니다(파일이 너무 큽니다).".to_string());
    }
    fs::read(path).map_err(|e| {
        log::warn!("save transfer: cannot read {path:?}: {e}");
        "파일을 읽지 못했습니다.".to_string()
    })
}

/// The 85제 pieces and recipes (`hz_java::legend`) and the special items (`hz_java::elixir::SPECIALS`)
/// this build serves — what a transfer file says its maker knew, and what one it brings may not exceed.
const KNOWN: (u32, u32, u32) = (hz_java::legend::COUNT as u32, hz_java::legend::RECIPE_COUNT as u32, hz_java::elixir::SPECIALS);

/// Read and check a transfer file for THIS app: the game it belongs to, and the app that made it —
/// `app` is this build's versionCode (0 = unknown, which skips that check). A NEWER app's file is
/// refused: its records may carry what this build's patches do not know. So is a file from an app
/// that knows more 85제 pieces or recipes than this one, whatever its versionCode says (2026-10-30:
/// the PC's 48 and the phones' 48 were different code) — the game drops an item it has no record for
/// and the next save writes the bag without it.
fn check_import(game: Option<&str>, app: u32, bytes: &[u8]) -> Result<crate::save_transfer::Bundle, String> {
    let b = crate::save_transfer::decode(bytes)?;
    if app != 0 && b.app > app {
        return Err(format!("더 새 버전의 앱에서 만든 파일입니다(파일 {} · 이 기기 {app}). 이 기기의 앱을 먼저 최신 버전으로 업데이트해 주세요.", b.app));
    }
    // Its versionCode may say no newer (the 48 and 48 above): what the player reads is why.
    if b.legend > KNOWN.0 || b.recipes > KNOWN.1 || b.specials > KNOWN.2 {
        log::warn!("save transfer: the file's app knows {} pieces, {} recipes and {} specials, this one {KNOWN:?}", b.legend, b.recipes, b.specials);
        return Err("이 기기의 앱이 모르는 아이템(85제·조합서·엘릭서)이 들어 있을 수 있는, 더 새 버전의 앱에서 만든 파일입니다. 그대로 받으면 그 아이템이 사라집니다. 이 기기의 앱을 먼저 최신 버전으로 업데이트해 주세요.".to_string());
    }
    match game {
        None => Err("앱의 게임 정보를 읽지 못했습니다.".to_string()),
        Some(g) if g != b.game => {
            log::warn!("save transfer: the file is game {} and this app runs {g}", b.game);
            Err("다른 게임(다른 버전의 게임 파일)의 세이브 파일입니다.".to_string())
        }
        Some(_) => Ok(b),
    }
}

/// The import dialog's account of the file at `path` (`MainActivity.importPicked`): `ok`, then
/// `created|<ms>`, `app|<versionCode>`, one `file|<slot>|<saved ms>` per slot it carries, `extra|storage`
/// when it carries the 계정창고, and one `here|<slot>|<saved ms>` per slot this phone holds now — what
/// the import will move into the vault.
pub fn import_summary(data_dir: &Path, game: Option<&str>, app: u32, path: &Path) -> Result<String, String> {
    let b = check_import(game, app, &read_transfer_file(path)?)?;
    let mut out = vec!["ok".to_string(), format!("created|{}", b.created_ms), format!("app|{}", b.app)];
    out.extend(b.slots().into_iter().map(|n| format!("file|{n}|{}", b.saved_ms(n))));
    if b.has_storage() {
        out.push("extra|storage".to_string());
    }
    out.extend(live_slots(data_dir).into_iter().map(|(n, ms)| format!("here|{n}|{ms}")));
    Ok(out.join("\n"))
}

/// Schedule the import of the file at `path` for the next launch — checked again first (the file the
/// dialog showed may have been replaced since). Cancels a restore that was waiting.
pub fn stage_import(data_dir: &Path, game: Option<&str>, app: u32, path: &Path) -> Result<(), String> {
    let bytes = read_transfer_file(path)?;
    let b = check_import(game, app, &bytes)?;
    let dest = import_path(data_dir);
    if let Err(e) = dest.parent().map_or(Ok(()), fs::create_dir_all).and_then(|()| crate::database::write_atomic(&dest, &bytes)) {
        log::warn!("save transfer: cannot stage {dest:?}: {e}");
        return Err("가져오기를 준비하지 못했습니다. 저장 공간을 확인해 주세요.".to_string());
    }
    if fs::remove_file(data_dir.join(REQUEST_FILE)).is_ok() {
        log::warn!("saves: the restore waiting is cancelled — an import was scheduled after it");
    }
    log::warn!("save transfer: import of slots {:?} (app {}) scheduled for the next launch", b.slots(), b.app);
    Ok(())
}

/// What the boot did with a staged import: a line for the log, and the player's account for the
/// first frame's dialog (its first line is the title — `MainActivity.showSaveMessage`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportReport {
    pub log: String,
    pub notice: String,
}

/// The notice of an import that changed nothing on this device.
fn import_refused(why: &str) -> String {
    format!("세이브 가져오기 실패\n가져오기를 적용하지 못했습니다. 이 기기의 세이브는 그대로입니다.\n\n이유: {why}")
}

/// At boot, before the game opens any store: carry out a staged import. The staged file goes FIRST
/// (the restore's rule) — an import that fails must not run again on every launch, each run putting
/// this phone's saves into the vault again until rotation pushed out the generations that matter.
pub fn apply_pending_import(data_dir: &Path, game: Option<&str>, now_ms: u64) -> Option<ImportReport> {
    let path = import_path(data_dir);
    let bytes = fs::read(&path).ok()?;
    let report = |log: String, notice: String| Some(ImportReport { log, notice });
    if let Err(e) = fs::remove_file(&path) {
        return report(format!("import NOT applied: the staged file could not be consumed ({e})"), import_refused("가져올 파일을 정리하지 못했습니다."));
    }
    let b = match check_import(game, 0, &bytes) {
        Ok(b) => b,
        Err(why) => return report(format!("import NOT applied: {why}"), import_refused(&why)),
    };
    let slots = b.slots();
    let list = |v: &[u8]| v.iter().map(u8::to_string).collect::<Vec<_>>().join(", ");
    match import_into(data_dir, &b, now_ms) {
        Ok(kept) => {
            let mut notice = format!("세이브 가져오기 완료\n다른 기기에서 가져온 세이브를 적용했습니다.\n\n가져온 슬롯: {}", list(&slots));
            if b.has_storage() {
                notice += " · 계정창고";
            }
            if !kept.is_empty() {
                notice += &format!(
                    "\n\n이 기기에 있던 세이브(슬롯 {})는 백업에 보관했습니다.\n되돌리려면: 설정 → 진단 정보 → 세이브 관리 → 백업에서 복원",
                    list(&kept)
                );
            }
            report(format!("IMPORTED slots {slots:?} (app {}); this phone's slots {kept:?} kept in the vault @ {now_ms}", b.app), notice)
        }
        Err(ImportFailed::Untouched(why)) => report(format!("import NOT applied: {why}"), import_refused("이 기기의 저장 공간에 쓰지 못했습니다.")),
        Err(ImportFailed::RolledBack(why)) => report(format!("import NOT applied (rolled back): {why}"), import_refused("이 기기의 저장 공간에 쓰지 못했습니다.")),
        Err(ImportFailed::Partial(why)) => report(
            format!("import STOPPED HALF-WAY: {why}"),
            "세이브 가져오기 실패\n가져오기가 중간에 멈췄습니다. 받은 파일로 가져오기를 한 번 더 해 주세요.\n\n이 기기에 있던 세이브는 백업에 있습니다: 설정 → 진단 정보 → 세이브 관리 → 백업에서 복원".to_string(),
        ),
    }
}

/// Why [`import_into`] stopped; the string is for the log.
#[derive(Debug)]
enum ImportFailed {
    /// Before anything was written.
    Untouched(String),
    /// A write failed and every record went back to what it held.
    RolledBack(String),
    /// A write failed and putting a record back failed too.
    Partial(String),
}

/// HZ-058: the quick plates on this device (`hz_java::quickpage::FILE`; none = empty).
fn read_plates(data_dir: &Path) -> hz_java::quickpage::Store {
    fs::read_to_string(data_dir.join(hz_java::quickpage::FILE)).map(|t| hz_java::quickpage::Store::parse(&t)).unwrap_or_default()
}

/// Put `bytes` at `path` (`None`: no record — the store stays, empty, and the game adds into it).
fn put(path: &Path, bytes: Option<&[u8]>) -> std::io::Result<()> {
    match bytes {
        Some(b) => path.parent().map_or(Ok(()), fs::create_dir_all).and_then(|()| crate::database::write_atomic(path, b)),
        None => match fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
    }
}

/// Keep `old` (the record of `store` now) in the vault, and know it is there: a generation of this
/// moment holding exactly these bytes — an older copy would pair with no restore point.
fn keep(data_dir: &Path, store: &str, old: &[u8], now_ms: u64) -> Result<(), String> {
    vault::archive_fresh(data_dir, store, RECORD, &record_path(data_dir, store), now_ms);
    let dir = vault::dir_for(data_dir, store, RECORD);
    let held = generations(data_dir, store)
        .into_iter()
        .filter(|&(ms, len)| ms >= now_ms && len == old.len())
        .any(|(ms, len)| fs::read(dir.join(vault::gen_name(ms, len))).is_ok_and(|b| b == old));
    if held { Ok(()) } else { Err(format!("{store} could not be kept in the vault")) }
}

/// Before `plan`'s records take their new bytes (`None`: no record), [`keep`] what they hold now
/// (`before`, the same order) — every record of a slot that changes at all, the three at one moment:
/// a restore point is one generation of each ([`points`]), a bag kept alone is none. A store outside
/// the slots goes in when it changes. The slots kept.
fn keep_changing(data_dir: &Path, plan: &[(&str, Option<&[u8]>)], before: &[Option<Vec<u8>>], now_ms: u64) -> Result<Vec<u8>, String> {
    let changes: Vec<bool> = plan.iter().zip(before).map(|((_, new), old)| old.as_deref() != *new).collect();
    let slot_of = |store: &str| (1..=SLOTS).find(|&n| stores(n).iter().any(|s| s == store));
    let mut kept: Vec<u8> = Vec::new();
    for (i, ((store, _), old)) in plan.iter().zip(before).enumerate() {
        let Some(old) = old.as_deref().filter(|o| !o.is_empty()) else { continue };
        let slot = slot_of(store);
        let moves = match slot {
            Some(n) => plan.iter().zip(&changes).any(|((s, _), &c)| c && slot_of(s) == Some(n)),
            None => changes[i],
        };
        if !moves {
            continue;
        }
        keep(data_dir, store, old, now_ms)?;
        if let Some(n) = slot
            && !kept.contains(&n)
        {
            kept.push(n);
        }
    }
    kept.sort_unstable();
    Ok(kept)
}

/// Make this phone's records the bundle's: every record `save_transfer::names` lists holds the
/// bundle's bytes afterwards, or none — except `HOL.gamestore`, which a bundle without one leaves as
/// it is. The slots whose records went into the vault first.
fn import_into(data_dir: &Path, b: &crate::save_transfer::Bundle, now_ms: u64) -> Result<Vec<u8>, ImportFailed> {
    let plan: Vec<(String, Option<&[u8]>)> = crate::save_transfer::names()
        .into_iter()
        .map(|s| {
            let new = b.get(&s);
            (s, new)
        })
        .filter(|(s, new)| !(s == crate::save_transfer::KEPT_IF_ABSENT && new.is_none()))
        .collect();
    // 1. What every one of those records holds now — before anything is written, so a failure can
    //    put all of it back. One that exists but cannot be read stops the import here.
    let mut before: Vec<Option<Vec<u8>>> = Vec::with_capacity(plan.len());
    for (store, _) in &plan {
        match fs::read(record_path(data_dir, store)) {
            Ok(bytes) => before.push(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => before.push(None),
            Err(e) => return Err(ImportFailed::Untouched(format!("cannot read {store}: {e}"))),
        }
    }
    // 2. Every record that will change into the vault, with the rest of its slot.
    let changes: Vec<(&str, Option<&[u8]>)> = plan.iter().map(|(s, new)| (s.as_str(), *new)).collect();
    let kept = keep_changing(data_dir, &changes, &before, now_ms).map_err(ImportFailed::Untouched)?;
    // 3. The bundle's records, slots first; on the first failure every record written so far goes
    //    back, newest first.
    for (i, (store, new)) in plan.iter().enumerate() {
        if before[i].as_deref() == *new {
            continue;
        }
        if let Err(e) = put(&record_path(data_dir, store), *new) {
            let undo: Vec<String> = plan[..i]
                .iter()
                .zip(&before[..i])
                .rev()
                .filter_map(|((s, _), old)| put(&record_path(data_dir, s), old.as_deref()).err().map(|e| format!("{s}: {e}")))
                .collect();
            let why = format!("writing {store}: {e}");
            return Err(if undo.is_empty() { ImportFailed::RolledBack(why) } else { ImportFailed::Partial(format!("{why}; not put back: {}", undo.join(", "))) });
        }
    }
    // 4. HZ-058: the plates of the slots it brings are the file's (none in it, none here — an older app's
    //    file leaves this device's plates for those slots out, and the plates start from the game's
    //    slots at the next play).
    let mut plates = read_plates(data_dir);
    let rows = hz_java::quickpage::Store::read_lines(b.quick.iter().map(String::as_str));
    let slots = b.slots();
    plates.replace_slots(&slots, &rows);
    if let Err(e) = crate::database::write_atomic(&data_dir.join(hz_java::quickpage::FILE), plates.format().as_bytes()) {
        log::warn!("save transfer: the quick plates not written ({e}) — they start from the game's slots");
    }
    // 5. Each slot's records keep when it was saved on the other device (the manifest's, never later
    //    than now): 이어하기 starts on the slot played last there (`last_played` reads the payload's
    //    time), not on the one this loop happened to write last.
    for n in b.slots() {
        let ms = b.saved_ms(n).min(now_ms);
        if ms == 0 {
            continue;
        }
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms);
        for s in stores(n) {
            if let Err(e) = fs::File::options().write(true).open(record_path(data_dir, &s)).and_then(|f| f.set_modified(t)) {
                log::warn!("save transfer: {s}: its save time not kept ({e})");
            }
        }
    }
    Ok(kept)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_is_one_save_of_all_three_stores() {
        let t = 1_761_400_000_000;
        // Two saves of slot 1, the bag also written alone (a 계정창고 move) a minute before the last.
        let slot = [(t, 45), (t + 600_000 + 2, 45)];
        let gamedata = [(t + 1, 379), (t + 600_000 + 3, 381)];
        let item = [(t + 2, 117), (t + 540_000, 117), (t + 600_000 + 4, 120)];
        let p = points(&slot, &gamedata, &item);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0], Point { ms: t + 600_003, gens: [(t + 600_002, 45), (t + 600_003, 381), (t + 600_004, 120)] }, "newest first");
        assert_eq!(p[1].gens[2], (t + 2, 117), "the nearest bag, not the lone one");
        // A payload with no summary near it (the summary fell inside the vault's interval) is no point.
        assert!(points(&[(t, 45)], &[(t + 10_000, 379)], &[(t + 10_001, 117)]).is_empty());
        assert!(points(&[], &[(t, 379)], &[(t, 117)]).is_empty(), "never a payload without its summary");
    }

    #[test]
    fn the_last_played_slot_is_the_newest_payload() {
        let d = data("last");
        assert_eq!(last_played(&d), None);
        let t = 1_761_400_000_000;
        save(&d, 3, [b"S", b"G", b"I"], t);
        std::thread::sleep(std::time::Duration::from_millis(20));
        save(&d, 2, [b"S", b"G", b"I"], t + 1);
        assert_eq!(last_played(&d), Some(2));
        let _ = fs::remove_dir_all(&d);
    }

    /// B3: the dialog's list names each point by `slot|ms`, asks for it again by that key from the
    /// files (a stale or garbled key schedules nothing), and says which slot is gone.
    #[test]
    fn the_dialog_lists_points_and_asks_for_one_by_its_key() {
        let d = data("offer");
        assert_eq!(offer_lines(&d), "", "nothing to put back");
        let t = 1_761_400_000_000;
        save(&d, 1, [b"S0", b"G0", b"I0"], t);
        save(&d, 1, [b"S1", b"G1", b"I1"], t + 60_000);
        save(&d, 4, [b"S00", b"G00", b"I00"], t + 100_000);
        save(&d, 4, [b"S11", b"G11", b"I11"], t + 160_000);
        fs::remove_file(record_path(&d, "HOL.slot4")).unwrap();
        let report = slot_report(&slots(&d));
        assert_eq!(report[0], format!("세이브 슬롯 2 / {SLOTS} 사용"));
        assert!(report[1].starts_with("  슬롯 1: 2 + 2 + 2 B · 복원 지점 1개") && !report[1].contains("지금 없음"), "{report:?}");
        assert!(report[2].starts_with("  슬롯 4: - + 3 + 3 B") && report[2].contains("지금 없음"), "{report:?}");
        let lines: Vec<String> = offer_lines(&d).lines().map(String::from).collect();
        // A point is stamped when its bytes were written over (the vault's pre-image) — the save's
        // first write, the summary's, which keeps the payload and the bag with it.
        assert_eq!(lines, vec![format!("1|{}|6|0", t + 60_000), format!("4|{}|9|1", t + 160_000)]);
        assert!(request_offer(&d, "4|1").is_err() && request_offer(&d, "x").is_err() && request_offer(&d, "").is_err());
        assert!(!d.join(REQUEST_FILE).exists(), "nothing scheduled by a bad key");
        assert_eq!(request_offer(&d, &lines[1]).unwrap(), 4, "the whole line is a key too");
        assert_eq!(apply_pending(&d, t + 200_000), Applied::Done(stores(4).to_vec()));
        assert_eq!(fs::read(record_path(&d, "HOL.slot4")).unwrap(), b"S00", "the slot is back");
        // The sentence: an export, and none without storage.
        assert!(export_sentence(&d, Some(&d.join("ext")), t).starts_with("세이브 파일 "));
        assert!(export_sentence(&d, None, t).contains("외부 저장소"));
        let _ = fs::remove_dir_all(&d);
    }

    fn data(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hz_saves_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    /// Write `bytes` as slot `n`'s three records, going through the vault the way a save does.
    fn save(d: &Path, n: u8, bytes: [&[u8]; 3], t: u64) {
        for (i, (store, b)) in stores(n).iter().zip(bytes).enumerate() {
            let rec = record_path(d, store);
            fs::create_dir_all(rec.parent().unwrap()).unwrap();
            vault::before_overwrite(d, store, RECORD, &rec, Some(b), t + i as u64);
            crate::database::write_atomic(&rec, b).unwrap();
        }
    }

    #[test]
    fn a_restore_puts_all_three_back_at_the_next_launch_and_costs_one_seat() {
        let d = data("restore");
        let t = 1_761_400_000_000;
        save(&d, 1, [b"S-one", b"G-one", b"I-one"], t);
        save(&d, 1, [b"S-two", b"G-two", b"I-two"], t + 60_000);
        save(&d, 1, [b"S-three", b"G-three", b"I-three"], t + 120_000);
        let s = &slots(&d)[0];
        assert_eq!(s.now, [Some(7), Some(7), Some(7)]);
        assert_eq!(s.points.len(), 2, "the pre-images of the 2nd and 3rd saves: 'one' and 'two'");
        let one = *s.points.last().unwrap();
        assert_eq!(fs::read(vault::dir_for(&d, "HOL.gamedata1", RECORD).join(vault::gen_name(one.ms, 5))).unwrap(), b"G-one");
        let seats = |d: &Path| -> usize { stores(1).iter().map(|st| generations(d, st).len()).sum() };
        let before = seats(&d);
        request_restore(&d, 1, &one).unwrap();
        // Nothing happens until the next launch.
        assert_eq!(fs::read(record_path(&d, "HOL.gamedata1")).unwrap(), b"G-three");
        assert_eq!(apply_pending(&d, t + 180_000), Applied::Done(stores(1).to_vec()));
        assert!(!d.join(REQUEST_FILE).exists(), "consumed");
        assert_eq!(apply_pending(&d, t + 180_001), Applied::None, "and never again");
        for (store, want) in stores(1).iter().zip([&b"S-one"[..], b"G-one", b"I-one"]) {
            assert_eq!(fs::read(record_path(&d, store)).unwrap(), want, "{store}");
        }
        assert_eq!(seats(&d), before + 3, "the bytes it replaced, one per store");
        // The game's first save after it writes over the restored bytes: no duplicate.
        save(&d, 1, [b"S-four", b"G-four", b"I-four"], t + 240_000);
        assert_eq!(seats(&d), before + 3, "the pre-image equals the pinned generation");
        let _ = fs::remove_dir_all(&d);
    }

    /// Code review 2026-10-29 (the import's twin): a restore skipped a store whose vault's newest
    /// generation already held its bytes — here the bag, put back as it was by a lone bag write inside
    /// the vault's interval — so the slot as it was before the restore was no point (a generation of
    /// each of the three at one moment), and the restore could not be undone.
    /// HZ-058 (code review 2026-10-31): the plates were the replaced save's — the restored slot's rows
    /// go (its next play adopts the restored save's own slots), every other slot's stay.
    #[test]
    fn a_restore_lets_the_slots_quick_plates_start_again_from_the_restored_save() {
        use hz_java::quickpage::{Entry, Owner, Plates, Store, FILE};
        let d = data("restore_quick");
        let t = 1_761_400_000_000;
        save(&d, 1, [b"S-one", b"G-one", b"I-one"], t);
        save(&d, 1, [b"S-two", b"G-two", b"I-two"], t + 60_000);
        let mut plates = Store::default();
        let mut later = Plates::default();
        later.set(1, 6, Entry::Skill(12));
        plates.rows.insert(Owner { slot: 1, class: 0 }, later);
        plates.rows.insert(Owner { slot: 1, class: 3 }, Plates::default());
        plates.rows.insert(Owner { slot: 2, class: 0 }, later);
        fs::write(d.join(FILE), plates.format()).unwrap();
        let one = *slots(&d)[0].points.last().unwrap();
        request_restore(&d, 1, &one).unwrap();
        assert_eq!(apply_pending(&d, t + 120_000), Applied::Done(stores(1).to_vec()));
        let now = Store::parse(&fs::read_to_string(d.join(FILE)).unwrap());
        assert_eq!(now.rows.keys().copied().collect::<Vec<_>>(), vec![Owner { slot: 2, class: 0 }], "slot 1's rows, every job's, gone");
        assert_eq!(now.rows[&Owner { slot: 2, class: 0 }].get(1, 6), Some(Entry::Skill(12)));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_restore_leaves_the_slot_it_replaced_as_a_restore_point() {
        let d = data("undo");
        let t = 1_761_400_000_000;
        save(&d, 1, [b"S0", b"G0", b"I0"], t);
        save(&d, 1, [b"S1", b"G1", b"I1"], t + 60_000);
        // A 계정창고 move puts the bag back as it was 10 s after the save: inside the interval, not kept.
        let bag = record_path(&d, "HOL.item1");
        vault::before_overwrite(&d, "HOL.item1", RECORD, &bag, Some(b"I0"), t + 70_000);
        crate::database::write_atomic(&bag, b"I0").unwrap();
        let p = slots(&d)[0].points[0];
        request_restore(&d, 1, &p).unwrap();
        assert_eq!(apply_pending(&d, t + 200_000), Applied::Done(stores(1).to_vec()));
        assert_eq!(fs::read(record_path(&d, "HOL.gamedata1")).unwrap(), b"G0");
        let undo = slots(&d)[0].points.iter().copied().find(|p| p.ms >= t + 200_000).expect("the slot before the restore is a point");
        for (store, ((ms, len), want)) in stores(1).iter().zip(undo.gens.into_iter().zip([&b"S1"[..], b"G1", b"I0"])) {
            assert_eq!(fs::read(vault::dir_for(&d, store, RECORD).join(vault::gen_name(ms, len))).unwrap(), want, "{store}");
        }
        let _ = fs::remove_dir_all(&d);
    }

    /// Code review 2026-10-29: the vault takes a pre-image only of bytes that change, so a save that
    /// left the bag as it was kept no bag generation and that save was no point (a generation of each
    /// of the three at one moment). The slot's three go in together — once a save, not once a write.
    #[test]
    fn a_save_that_leaves_the_bag_as_it_was_is_still_a_point() {
        let d = data("samebag");
        let t = 1_761_400_000_000;
        save(&d, 1, [b"S0", b"G0", b"I0"], t);
        save(&d, 1, [b"S1", b"G1", b"I0"], t + 60_000);
        save(&d, 1, [b"S2", b"G2", b"I1"], t + 120_000);
        let p = slots(&d)[0].points.clone();
        assert_eq!(p.len(), 2, "the pre-images of the 2nd and 3rd saves: {p:?}");
        for (point, want) in p.iter().zip([[&b"S1"[..], b"G1", b"I0"], [b"S0", b"G0", b"I0"]]) {
            for (store, ((ms, len), want)) in stores(1).iter().zip(point.gens.into_iter().zip(want)) {
                assert_eq!(fs::read(vault::dir_for(&d, store, RECORD).join(vault::gen_name(ms, len))).unwrap(), want, "{store}");
            }
        }
        assert!(stores(1).iter().all(|s| generations(&d, s).len() == 2), "one generation of each a save, no more");
        // A 계정창고 move writes the bag alone: the bag's own pre-image, not the slot's.
        let bag = record_path(&d, "HOL.item1");
        vault::before_overwrite(&d, "HOL.item1", RECORD, &bag, Some(b"I2"), t + 180_000);
        crate::database::write_atomic(&bag, b"I2").unwrap();
        assert_eq!(stores(1).map(|s| generations(&d, &s).len()), [2, 2, 3]);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn the_pinned_generation_survives_a_long_session() {
        let d = data("pin");
        let t = 1_761_400_000_000;
        save(&d, 2, [b"S0", b"G0", b"I0"], t);
        save(&d, 2, [b"S1", b"G1", b"I1"], t + 60_000);
        let p = slots(&d)[1].points[0];
        request_restore(&d, 2, &p).unwrap();
        apply_pending(&d, t + 120_000);
        for i in 0..500u64 {
            let b = format!("{i:05}");
            save(&d, 2, [b.as_bytes(); 3], t + 200_000 + i * vault::MIN_INTERVAL_MS);
        }
        for store in stores(2) {
            let dir = vault::dir_for(&d, &store, RECORD);
            let (_, name) = vault::read_pin(&dir).expect("still pinned");
            assert!(dir.join(&name).is_file(), "{store}: the restored generation is still there");
            assert!(generations(&d, &store).len() <= vault::KEEP_MAX, "{store}: under the ceiling");
        }
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_stale_or_broken_request_does_nothing_twice() {
        let d = data("stale");
        let t = 1_761_400_000_000;
        save(&d, 1, [b"S0", b"G0", b"I0"], t);
        save(&d, 1, [b"S1", b"G1", b"I1"], t + 60_000);
        let mut p = slots(&d)[0].points[0];
        p.gens[1].0 += 1; // a generation that is not on disk
        assert!(request_restore(&d, 1, &p).is_err());
        assert!(!d.join(REQUEST_FILE).exists(), "nothing scheduled");
        fs::write(d.join(REQUEST_FILE), "HOL.gamedata1 nonsense\n").unwrap();
        assert!(matches!(apply_pending(&d, t), Applied::Partial(ref done, _) if done.is_empty()));
        assert!(!d.join(REQUEST_FILE).exists(), "consumed even so");
        assert_eq!(fs::read(record_path(&d, "HOL.gamedata1")).unwrap(), b"G1", "untouched");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_slot_gone_while_the_vault_has_it_is_seen_from_the_files() {
        let d = data("missing");
        let t = 1_761_400_000_000;
        save(&d, 3, [b"S0", b"G0", b"I0"], t);
        save(&d, 3, [b"S1", b"G1", b"I1"], t + 60_000);
        assert!(!slots(&d)[2].missing_but_recoverable());
        fs::remove_file(record_path(&d, "HOL.slot3")).unwrap();
        let s = &slots(&d)[2];
        assert!(s.now[0].is_none() && s.missing_but_recoverable());
        assert!(!slots(&d)[0].missing_but_recoverable(), "an empty slot with no vault is just empty");
        let _ = fs::remove_dir_all(&d);
    }

    const GAME: &str = crate::save_transfer::tests::GAME;

    fn put_record(d: &Path, store: &str, bytes: &[u8]) {
        let rec = record_path(d, store);
        fs::create_dir_all(rec.parent().unwrap()).unwrap();
        fs::write(rec, bytes).unwrap();
    }

    fn records(d: &Path) -> Vec<(String, Option<Vec<u8>>)> {
        crate::save_transfer::names().into_iter().map(|s| (s.clone(), fs::read(record_path(d, &s)).ok())).collect()
    }

    /// B4 (hero2's 세이브 옮기기): phone A's file makes phone B's records A's — its slots whole, the
    /// 계정창고 too, the slot B had and A has not gone — every record B had kept in the vault first as
    /// a restore point, applied once at the next launch and never again.
    #[test]
    fn a_transfer_file_makes_the_other_phone_this_ones_and_keeps_what_it_had() {
        let (a, b) = (data("xfer_a"), data("xfer_b"));
        let t = 1_793_000_000_000;
        save(&a, 1, [b"A-S1", b"A-G1", b"A-I1"], t);
        save(&a, 7, [b"A-S7", b"A-G7", b"A-I7"], t);
        put_record(&a, "HOL.storage", b"A-bag");
        put_record(&a, "HOL.slot2", b"a lone summary");
        put_record(&a, "d", b"A-options");
        let file = a.join("out").join("hz.zip");
        assert!(export_to(&a, None, 35, &file, t).is_err(), "no game, no file");
        assert_eq!(export_to(&a, Some(GAME), 35, &file, t).unwrap(), vec![1, 7], "slot 2 is not whole: it stays");
        // Phone B: slots 1 and 3, its own bag and gamestore.
        save(&b, 1, [b"B-S1", b"B-G1", b"B-I1"], t);
        save(&b, 3, [b"B-S3", b"B-G3", b"B-I3"], t);
        put_record(&b, "HOL.storage", b"B-bag");
        put_record(&b, "HOL.gamestore", b"B-flags");
        put_record(&b, "d", b"B-options");
        let summary = import_summary(&b, Some(GAME), 35, &file).unwrap();
        let lines: Vec<&str> = summary.lines().collect();
        assert_eq!(lines[0], "ok");
        assert!(lines.iter().any(|l| l.starts_with("file|1|")) && lines.iter().any(|l| l.starts_with("file|7|")) && lines.contains(&"extra|storage"));
        assert_eq!(lines.iter().filter(|l| l.starts_with("here|")).count(), 2, "{summary}");
        // Refused: another game, a newer app.
        assert!(import_summary(&b, Some("0123456789abcdef0123456789abcdef"), 35, &file).is_err());
        assert!(import_summary(&b, Some(GAME), 34, &file).unwrap_err().contains("새 버전"));
        // Staged: nothing changes now, and a waiting restore is cancelled.
        fs::write(b.join(REQUEST_FILE), "x").unwrap();
        stage_import(&b, Some(GAME), 35, &file).unwrap();
        assert!(import_pending(&b) && !b.join(REQUEST_FILE).exists());
        assert_eq!(fs::read(record_path(&b, "HOL.gamedata1")).unwrap(), b"B-G1");
        // The next launch.
        let r = apply_pending_import(&b, Some(GAME), t + 60_000).unwrap();
        assert!(r.notice.starts_with("세이브 가져오기 완료\n") && r.notice.contains("슬롯 1, 3"), "{r:?}");
        assert!(!import_pending(&b) && apply_pending_import(&b, Some(GAME), t + 60_001).is_none(), "once");
        for (store, want) in records(&b) {
            let from_a = fs::read(record_path(&a, &store)).ok().filter(|_| !store.starts_with("HOL.slot2"));
            match store.as_str() {
                "HOL.gamestore" => assert_eq!(want.as_deref(), Some(&b"B-flags"[..]), "a file without one leaves it"),
                _ => assert_eq!(want, from_a, "{store}"),
            }
        }
        assert_eq!(fs::read(record_path(&b, "d")).unwrap(), b"B-options", "the options are the phone's");
        // What B had is a restore point: slot 3 is gone from the list, the vault has it.
        let s3 = &slots(&b)[2];
        assert!(s3.missing_but_recoverable() && s3.points.len() == 1, "{s3:?}");
        assert!(slots(&b)[0].points.iter().any(|p| p.ms >= t + 60_000), "B's slot 1 as it was before the import");
        let _ = fs::remove_dir_all(&a);
        let _ = fs::remove_dir_all(&b);
    }

    /// Code review 2026-10-29: a file that changes only the bag kept only the bag — no restore point
    /// (one generation of each of the three), so 「백업에서 복원」 listed nothing. The slot's other two
    /// go in at the same moment, even when the vault's newest already holds their bytes.
    #[test]
    fn an_import_that_changes_only_the_bag_still_leaves_a_restore_point() {
        let (a, b) = (data("xbag_a"), data("xbag_b"));
        let t = 1_793_000_000_000;
        save(&a, 1, [b"S1", b"G1", b"A-bag"], t);
        let file = a.join("hz.zip");
        export_to(&a, Some(GAME), 35, &file, t).unwrap();
        save(&b, 1, [b"S0", b"G0", b"I0"], t);
        save(&b, 1, [b"S1", b"G1", b"B-bag"], t + 60_000);
        // The summary's newest generation already holds its bytes now (as after a restore).
        let dir = vault::dir_for(&b, "HOL.slot1", RECORD);
        fs::write(dir.join(vault::gen_name(t + 90_000, 2)), b"S1").unwrap();
        stage_import(&b, Some(GAME), 35, &file).unwrap();
        let r = apply_pending_import(&b, Some(GAME), t + 120_000).unwrap();
        assert!(r.notice.contains("슬롯 1)는 백업에"), "{r:?}");
        assert_eq!(fs::read(record_path(&b, "HOL.item1")).unwrap(), b"A-bag");
        let p = slots(&b)[0].points.iter().copied().find(|p| p.ms >= t + 120_000).expect("a point of the import's moment");
        for (store, ((ms, len), want)) in stores(1).iter().zip(p.gens.into_iter().zip([&b"S1"[..], b"G1", b"B-bag"])) {
            assert_eq!(fs::read(vault::dir_for(&b, store, RECORD).join(vault::gen_name(ms, len))).unwrap(), want, "{store}");
        }
        let _ = fs::remove_dir_all(&a);
        let _ = fs::remove_dir_all(&b);
    }

    /// 2026-10-30 (PC ↔ phone): the slots arrive with the times they were saved on the other device,
    /// so 이어하기 starts where the player left off there — not on the slot written last here.
    #[test]
    fn an_import_keeps_when_each_slot_was_saved() {
        let (a, b) = (data("xtime_a"), data("xtime_b"));
        let t = 1_793_000_000_000;
        save(&a, 2, [b"S2", b"G2", b"I2"], t);
        save(&a, 5, [b"S5", b"G5", b"I5"], t);
        let at = |ms: u64| std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms);
        for (n, ms) in [(2u8, t + 60_000), (5, t)] {
            fs::File::options().write(true).open(record_path(&a, &format!("HOL.gamedata{n}"))).unwrap().set_modified(at(ms)).unwrap();
        }
        assert_eq!(last_played(&a), Some(2));
        let file = a.join("hz.zip");
        export_to(&a, Some(GAME), 35, &file, t + 120_000).unwrap();
        stage_import(&b, Some(GAME), 35, &file).unwrap();
        apply_pending_import(&b, Some(GAME), t + 180_000).unwrap();
        assert_eq!(last_played(&b), Some(2), "slot 5 was written after slot 2 here, slot 2 was played last there");
        assert_eq!(saved_ms(&b, 2), Some(t + 60_000));
        let _ = fs::remove_dir_all(&a);
        let _ = fs::remove_dir_all(&b);
    }

    /// HZ-058: the quick plates of the slots a file carries go with it and replace the receiver's for
    /// those slots; the receiver's other slots keep theirs.
    #[test]
    fn the_quick_plates_travel_with_their_slots() {
        use hz_java::quickpage::{Entry, Owner, Plates, Store, FILE};
        let (a, b) = (data("xquick_a"), data("xquick_b"));
        let t = 1_793_000_000_000;
        save(&a, 1, [b"S1", b"G1", b"I1"], t);
        let mut theirs = Store::default();
        let mut p = Plates::default();
        p.set(1, 6, Entry::Skill(9));
        theirs.rows.insert(Owner { slot: 1, class: 2 }, p);
        theirs.rows.insert(Owner { slot: 4, class: 0 }, Plates::default());
        fs::write(a.join(FILE), theirs.format()).unwrap();
        let file = a.join("hz.zip");
        export_to(&a, Some(GAME), 49, &file, t).unwrap();
        let bundle = crate::save_transfer::decode(&fs::read(&file).unwrap()).unwrap();
        assert_eq!(bundle.quick.len(), 1, "slot 1's line only — slot 4 is not in the file: {:?}", bundle.quick);
        save(&b, 1, [b"s1", b"g1", b"i1"], t);
        let mut mine = Store::default();
        mine.rows.insert(Owner { slot: 1, class: 0 }, Plates::adopt(&[4, -1, -1, -1, -1, -1]));
        mine.rows.insert(Owner { slot: 2, class: 1 }, Plates::default());
        fs::write(b.join(FILE), mine.format()).unwrap();
        stage_import(&b, Some(GAME), 49, &file).unwrap();
        apply_pending_import(&b, Some(GAME), t + 60_000).unwrap();
        let now = Store::parse(&fs::read_to_string(b.join(FILE)).unwrap());
        assert_eq!(now.rows.keys().copied().collect::<Vec<_>>(), vec![Owner { slot: 1, class: 2 }, Owner { slot: 2, class: 1 }]);
        assert_eq!(now.rows[&Owner { slot: 1, class: 2 }].get(1, 6), Some(Entry::Skill(9)));
        let _ = fs::remove_dir_all(&a);
        let _ = fs::remove_dir_all(&b);
    }

    /// 2026-10-30: the PC's build 48 knew 102 85제 pieces and the phones' vC 48 knew 36 — the same
    /// versionCode, so the newer-app check let the file through, and the game dropped every piece it
    /// had no record for (probe: `HOL.item1` 157 → 117 B at the next save). A file whose maker knew
    /// more is refused; one that says nothing (an older app's) is not.
    #[test]
    fn a_file_from_an_app_that_knows_more_items_is_refused() {
        let (a, b) = (data("xknown_a"), data("xknown_b"));
        let t = 1_793_000_000_000;
        save(&a, 1, [b"S1", b"G1", b"I1"], t);
        let file = a.join("hz.zip");
        export_to(&a, Some(GAME), 48, &file, t).unwrap();
        let mut bundle = crate::save_transfer::decode(&fs::read(&file).unwrap()).unwrap();
        assert_eq!((bundle.legend, bundle.recipes, bundle.specials), KNOWN, "the maker's knowledge is in the file");
        assert!(import_summary(&b, Some(GAME), 48, &file).is_ok());
        for known in [(KNOWN.0 + 1, KNOWN.1, KNOWN.2), (KNOWN.0, KNOWN.1 + 1, KNOWN.2), (KNOWN.0, KNOWN.1, KNOWN.2 + 1)] {
            (bundle.legend, bundle.recipes, bundle.specials) = known;
            fs::write(&file, crate::save_transfer::encode(&bundle).unwrap()).unwrap();
            assert!(import_summary(&b, Some(GAME), 48, &file).unwrap_err().contains("모르는 아이템"), "{known:?}");
            assert!(stage_import(&b, Some(GAME), 48, &file).is_err() && !import_pending(&b));
        }
        (bundle.legend, bundle.recipes, bundle.specials) = (0, 0, 0);
        fs::write(&file, crate::save_transfer::encode(&bundle).unwrap()).unwrap();
        assert!(import_summary(&b, Some(GAME), 48, &file).is_ok(), "an older app's file");
        let _ = fs::remove_dir_all(&a);
        let _ = fs::remove_dir_all(&b);
    }

    /// A write that fails half-way puts every record back; a file that is not ours, or another
    /// game's, changes nothing — and none of them runs again.
    #[test]
    fn a_failed_or_refused_import_changes_nothing() {
        let (a, b) = (data("xfail_a"), data("xfail_b"));
        let t = 1_793_000_000_000;
        save(&a, 1, [b"A-S1", b"A-G1", b"A-I1"], t);
        put_record(&a, "HOL.storage", b"A-bag");
        let file = a.join("hz.zip");
        export_to(&a, Some(GAME), 35, &file, t).unwrap();
        save(&b, 1, [b"B-S1", b"B-G1", b"B-I1"], t);
        put_record(&b, "HOL.storage", b"B-bag");
        let was = records(&b);
        // The bag's temp file is a directory: its write fails after slot 1's three went in.
        fs::create_dir_all(record_path(&b, "HOL.storage").with_extension("tmp")).unwrap();
        stage_import(&b, Some(GAME), 35, &file).unwrap();
        let r = apply_pending_import(&b, Some(GAME), t + 60_000).unwrap();
        assert!(r.log.contains("rolled back") && r.notice.contains("그대로"), "{r:?}");
        assert_eq!(records(&b), was, "every record back as it was");
        assert!(!import_pending(&b));
        // Another game's file: refused at the launch too (the staged check ran on this build).
        fs::remove_dir_all(record_path(&b, "HOL.storage").with_extension("tmp")).unwrap();
        stage_import(&b, Some(GAME), 35, &file).unwrap();
        let r = apply_pending_import(&b, Some("0123456789abcdef0123456789abcdef"), t + 120_000).unwrap();
        assert!(r.log.contains("NOT applied") && r.notice.contains("다른 게임"), "{r:?}");
        assert_eq!(records(&b), was);
        // Not a transfer file at all.
        let junk = b.join("junk.zip");
        fs::write(&junk, b"PK nope").unwrap();
        assert!(stage_import(&b, Some(GAME), 35, &junk).is_err() && !import_pending(&b));
        let _ = fs::remove_dir_all(&a);
        let _ = fs::remove_dir_all(&b);
    }

    #[test]
    fn export_copies_the_saves_and_the_vault_whole() {
        let d = data("export");
        let t = 1_761_400_000_000;
        save(&d, 1, [b"S0", b"G0", b"I0"], t);
        save(&d, 1, [b"S1", b"G1", b"I1"], t + 60_000);
        let opts = record_path(&d, "d");
        fs::create_dir_all(opts.parent().unwrap()).unwrap();
        fs::write(&opts, b"options").unwrap();
        let ext = d.join("external");
        let (out, n) = export(&d, &ext, t + 120_000).unwrap();
        assert_eq!(n, 3 + 3, "three records and one generation of each");
        assert_eq!(fs::read(out.join("db").join("HOL.gamedata1").join("1")).unwrap(), b"G1");
        assert!(!out.join("db").join("d").exists(), "not the options record");
        let g = out.join("save_vault").join("HOL.item1").join("1");
        assert_eq!(fs::read_dir(&g).unwrap().count(), 1);
        let _ = fs::remove_dir_all(&d);
    }
}
