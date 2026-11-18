//! The MIDP RecordStores on disk: `<data>/<app id>/db/<store>/<record id>`.
//!
//! This is where the player's character lives (`HOL.slot<n>` / `HOL.gamedata<n>` / `HOL.item<n>`),
//! so every rule here is a save-safety rule:
//!
//! * a record is replaced ATOMICALLY ([`write_atomic`]) — the old bytes stay readable until the
//!   new ones are complete;
//! * a failed write says so (`add` → `None`, `set` → `false` → `RecordStoreException` in
//!   `wie_midp`), and leaves the previous record intact;
//! * the bytes about to be replaced are archived first ([`crate::vault`]) — and a record that
//!   cannot be read is moved there whole, since the game will have shown its slot as empty;
//! * the PLAYER is told ([`Notify`]): the game catches every RMS exception and carries on as if
//!   the save worked (`ac.e()` and `rpg/b` around the writes, `ac.a()` around the read, javap
//!   2026-10-23), so an exception alone changes nothing on screen. A read error and "no record"
//!   are the same exception to it (`InvalidRecordIDException` is a `RecordStoreException`), so the
//!   trait's `None` costs nothing the game could have used.
//!
//! Generic parts follow hero3NativeAPP's `database.rs` (its 60차 / 72차 fixes), none of its
//! slot-pin machinery: this game has no network shell that re-aims a slot cursor under a loaded
//! character, and it never deletes a record.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use wie_backend::RecordId;

/// The HUD notice line (ASCII, `crate::font`), from the worker thread.
pub type Notify = Arc<dyn Fn(&'static str) + Send + Sync>;

/// A save write (`addRecord` / `setRecord` of a `HOL.` store) did not land.
pub const SAVE_FAILED: &str = "SAVE FAILED - STORAGE ERROR";
/// A save record exists and cannot be read: the game is about to show its slot as empty.
pub const SAVE_UNREADABLE: &str = "SAVE READ ERROR - THE FILE IS KEPT";

/// Write `data` to `path` so that a reader never sees a half-written record: temp file in the
/// same directory → `sync_all` → `rename` → best-effort directory sync.
///
/// `fs::write` is `open(O_TRUNC)` + `write`: between the two the save is ZERO bytes long, and a
/// kill, a crash or a storage error inside that window leaves a record the game reads back as an
/// empty slot. Refusing to save is recoverable; half-saving is not — and a failed write must leave
/// the previous record byte-intact, which the rename guarantees.
pub fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    // A fixed name: one writer per record, and a temp left by a killed write is simply replaced.
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    // The record is already whole; a directory that refuses a sync is not a failed save.
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// A store or app-id name as one safe path segment list: no traversal, no NUL, never empty.
fn sanitize(name: &str) -> PathBuf {
    let mut out = PathBuf::new();
    for segment in name.split(['/', '\\']) {
        match segment {
            "" | "." => {}
            ".." => out.push("_"),
            s => out.push(s.replace('\0', "_")),
        }
    }
    if out.as_os_str().is_empty() {
        out.push("_");
    }
    out
}

pub struct AndroidDatabaseRepository {
    data_dir: PathBuf,
    notify: Notify,
    failures: Arc<AtomicU32>,
}

impl AndroidDatabaseRepository {
    pub fn new(data_dir: PathBuf, notify: Notify) -> Self {
        Self { data_dir, notify, failures: Arc::default() }
    }

    /// How many save writes have failed since boot — the game's own save returns as if it worked
    /// (it swallows the exception), so this is what tells the SAVE tab's quick save it did not.
    pub fn save_failures(&self) -> Arc<AtomicU32> {
        self.failures.clone()
    }

    fn path_for(&self, name: &str, app_id: &str) -> PathBuf {
        self.data_dir.join(sanitize(app_id)).join("db").join(sanitize(name))
    }
}

#[async_trait::async_trait]
impl wie_backend::DatabaseRepository for AndroidDatabaseRepository {
    async fn open(&self, name: &str, app_id: &str) -> Box<dyn wie_backend::Database> {
        let path = self.path_for(name, app_id);
        if let Err(e) = fs::create_dir_all(&path) {
            log::error!("RMS open {name:?}: cannot create {path:?}: {e}");
        }
        Box::new(Database {
            data_dir: self.data_dir.clone(),
            store: name.to_string(),
            path,
            notify: self.notify.clone(),
            failures: self.failures.clone(),
        })
    }

    async fn exists(&self, name: &str, app_id: &str) -> bool {
        self.path_for(name, app_id).is_dir()
    }

    async fn delete(&self, name: &str, app_id: &str) -> bool {
        // `deleteRecordStore`. 영웅서기 Zero never calls it (javap, 2026-10-22); if anything ever
        // does, every record goes through the vault before the directory goes.
        let path = self.path_for(name, app_id);
        let Ok(rd) = fs::read_dir(&path) else { return false };
        let now = now_ms();
        for e in rd.filter_map(|e| e.ok()) {
            if let Some(id) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) {
                crate::vault::before_overwrite(&self.data_dir, name, id, &e.path(), None, now);
            }
        }
        match fs::remove_dir_all(&path) {
            Ok(()) => {
                log::warn!("RMS deleteRecordStore {name:?} — vaulted first");
                true
            }
            Err(e) => {
                log::error!("RMS deleteRecordStore {name:?} FAILED: {e}");
                false
            }
        }
    }
}

pub struct Database {
    data_dir: PathBuf,
    store: String,
    path: PathBuf,
    notify: Notify,
    failures: Arc<AtomicU32>,
}

impl Database {
    /// Tell the player — about the game's own saves only (the `HOL.` stores the vault keeps), not
    /// its options record or the telemetry stores.
    fn tell(&self, text: &'static str) {
        if crate::vault::is_vaulted(&self.store) {
            (self.notify)(text);
        }
    }

    /// A save write did not land: counted ([`AndroidDatabaseRepository::save_failures`]) and told.
    fn save_failed(&self) {
        if crate::vault::is_vaulted(&self.store) {
            self.failures.fetch_add(1, Ordering::Relaxed);
        }
        self.tell(SAVE_FAILED);
    }

    fn record(&self, id: RecordId) -> PathBuf {
        self.path.join(id.to_string())
    }

    fn ids(&self) -> Vec<RecordId> {
        let mut ids: Vec<RecordId> = match fs::read_dir(&self.path) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_file())
                .filter_map(|e| e.file_name().to_str().and_then(|s| s.parse().ok()))
                .collect(),
            Err(_) => Vec::new(),
        };
        ids.sort_unstable();
        ids
    }

    /// MIDP numbers records from 1 and does not reuse ids. This is "highest + 1", so only a
    /// deleted HIGHEST id can come back — a case this game never creates (it never deletes).
    fn next(&self) -> RecordId {
        self.ids().last().map_or(1, |last| last + 1)
    }
}

#[async_trait::async_trait]
impl wie_backend::Database for Database {
    async fn next_id(&self) -> RecordId {
        self.next()
    }

    async fn add(&mut self, data: &[u8]) -> Option<RecordId> {
        let id = self.next();
        // The game adds only into an empty store (`getNumRecords() == 0`), so a failed add is a
        // first save that does not exist; its next save tries the add again.
        match write_atomic(&self.record(id), data) {
            Ok(()) => {
                log::info!("RMS {} add #{id} ({} B)", self.store, data.len());
                Some(id)
            }
            Err(e) => {
                log::error!("RMS {} add #{id} ({} B) FAILED: {e}", self.store, data.len());
                self.save_failed();
                None
            }
        }
    }

    async fn get(&self, id: RecordId) -> Option<Vec<u8>> {
        let path = self.record(id);
        match fs::read(&path) {
            Ok(v) => Some(v),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            // One retry covers the transient cases. A persistent failure: the guest is about to
            // read this slot as EMPTY with the file right there, so the player is told, and a save
            // into that slot moves the file into the vault before replacing it.
            Err(first) => match fs::read(&path) {
                Ok(v) => {
                    log::error!("RMS {} get #{id} failed once ({first}) and succeeded on retry", self.store);
                    Some(v)
                }
                Err(e) => {
                    log::error!("RMS {} get #{id} FAILED: {e} — the game will read this as empty although the file exists", self.store);
                    self.tell(SAVE_UNREADABLE);
                    None
                }
            },
        }
    }

    async fn set(&mut self, id: RecordId, data: &[u8]) -> bool {
        let path = self.record(id);
        if !path.is_file() {
            // MIDP: `setRecord` on an id that does not exist is an InvalidRecordIDException.
            log::warn!("RMS {} set #{id}: no such record", self.store);
            return false;
        }
        let moved = crate::vault::before_overwrite(&self.data_dir, &self.store, id, &path, Some(data), now_ms());
        match write_atomic(&path, data) {
            Ok(()) => {
                log::info!("RMS {} set #{id} ({} B)", self.store, data.len());
                true
            }
            Err(e) => {
                let previous = if moved { "the unreadable previous record is in the vault" } else { "the previous record is intact" };
                log::error!("RMS {} set #{id} ({} B) FAILED: {e} — {previous}", self.store, data.len());
                self.save_failed();
                false
            }
        }
    }

    async fn delete(&mut self, id: RecordId) -> bool {
        let path = self.record(id);
        // An unreadable record is moved into the vault whole, which is the delete done.
        let moved = crate::vault::before_overwrite(&self.data_dir, &self.store, id, &path, None, now_ms());
        let ok = moved || fs::remove_file(&path).is_ok();
        log::warn!("RMS {} delete #{id} -> {ok}", self.store);
        ok
    }

    async fn get_record_ids(&self) -> Vec<RecordId> {
        self.ids()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wie_backend::DatabaseRepository as _;

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        // The futures here never actually wait, so one poll with a no-op waker completes them.
        let mut f = std::pin::pin!(f);
        let waker = std::task::Waker::noop();
        match f.as_mut().poll(&mut std::task::Context::from_waker(waker)) {
            std::task::Poll::Ready(v) => v,
            std::task::Poll::Pending => panic!("database future did not complete synchronously"),
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hz_db_test_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// A repository whose notices land in the returned list.
    fn repo(data: &Path) -> (AndroidDatabaseRepository, Arc<std::sync::Mutex<Vec<&'static str>>>) {
        let told = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = told.clone();
        (AndroidDatabaseRepository::new(data.to_path_buf(), Arc::new(move |t| sink.lock().unwrap().push(t))), told)
    }

    #[test]
    fn names_cannot_escape_the_data_dir() {
        assert_eq!(sanitize("HOL.gamedata1"), PathBuf::from("HOL.gamedata1"));
        assert_eq!(sanitize("../../etc"), PathBuf::from("_").join("_").join("etc"));
        assert_eq!(sanitize(""), PathBuf::from("_"));
        assert_eq!(sanitize("/"), PathBuf::from("_"));
    }

    #[test]
    fn add_set_get_round_trip_and_the_overwrite_is_vaulted() {
        let data = temp_dir("rt");
        let (repo, told) = repo(&data);
        let mut db = block_on(repo.open("HOL.gamedata1", "herozero.jar"));
        assert_eq!(block_on(db.get_record_ids()), Vec::<RecordId>::new());
        assert_eq!(block_on(db.add(b"first save")), Some(1));
        assert_eq!(block_on(db.get(1)).as_deref(), Some(&b"first save"[..]));
        assert!(block_on(db.set(1, b"second save")));
        assert_eq!(block_on(db.get(1)).as_deref(), Some(&b"second save"[..]));
        assert!(!block_on(db.set(7, b"x")), "no such record");
        assert_eq!(block_on(db.get_record_ids()), vec![1]);
        let vault = crate::vault::dir_for(&data, "HOL.gamedata1", 1);
        let kept: Vec<_> = fs::read_dir(&vault).unwrap().map(|e| fs::read(e.unwrap().path()).unwrap()).collect();
        assert_eq!(kept, vec![b"first save".to_vec()]);
        assert!(block_on(repo.exists("HOL.gamedata1", "herozero.jar")));
        assert!(!block_on(repo.exists("HOL.gamedata2", "herozero.jar")));
        assert!(told.lock().unwrap().is_empty(), "nothing went wrong");
        let _ = fs::remove_dir_all(&data);
    }

    #[test]
    fn a_failed_first_save_is_not_an_id_and_the_player_is_told() {
        let data = temp_dir("addfail");
        let (repo, told) = repo(&data);
        let mut db = block_on(repo.open("HOL.slot1", "herozero.jar"));
        // Make the temp path unusable: a DIRECTORY where record 1's temp file must go.
        fs::create_dir_all(repo.path_for("HOL.slot1", "herozero.jar").join("1.tmp")).unwrap();
        assert_eq!(block_on(db.add(b"first save")), None);
        assert_eq!(block_on(db.get_record_ids()), Vec::<RecordId>::new(), "no record appeared");
        assert_eq!(*told.lock().unwrap(), vec![SAVE_FAILED]);
        assert_eq!(repo.save_failures().load(Ordering::Relaxed), 1, "counted for the SAVE tab");
        // Not a save: the options record fails without a notice.
        let mut d = block_on(repo.open("d", "herozero.jar"));
        fs::create_dir_all(repo.path_for("d", "herozero.jar").join("1.tmp")).unwrap();
        assert_eq!(block_on(d.add(b"opts")), None);
        assert_eq!(told.lock().unwrap().len(), 1);
        assert_eq!(repo.save_failures().load(Ordering::Relaxed), 1);
        let _ = fs::remove_dir_all(&data);
    }

    #[test]
    fn an_unreadable_record_is_not_reported_silently() {
        let data = temp_dir("unreadable");
        let (repo, told) = repo(&data);
        let db = block_on(repo.open("HOL.slot2", "herozero.jar"));
        // Present but unreadable: a directory where record 1 is.
        fs::create_dir_all(repo.path_for("HOL.slot2", "herozero.jar").join("1")).unwrap();
        assert_eq!(block_on(db.get(1)), None, "the trait has only None to give");
        assert_eq!(*told.lock().unwrap(), vec![SAVE_UNREADABLE]);
        // Absent is not an error.
        assert_eq!(block_on(db.get(2)), None);
        assert_eq!(told.lock().unwrap().len(), 1);
        assert!(crate::font::drawable(SAVE_FAILED) && crate::font::drawable(SAVE_UNREADABLE));
        let _ = fs::remove_dir_all(&data);
    }

    #[test]
    fn a_failed_write_leaves_the_previous_record_byte_identical() {
        let data = temp_dir("fail");
        let rec = data.join("1");
        fs::write(&rec, b"the character").unwrap();
        // Make the temp path unusable: a DIRECTORY where the temp file must go.
        fs::create_dir_all(rec.with_extension("tmp")).unwrap();
        assert!(write_atomic(&rec, b"new bytes").is_err());
        assert_eq!(fs::read(&rec).unwrap(), b"the character");
        let _ = fs::remove_dir_all(&data);
    }

    #[test]
    fn record_ids_are_never_reused() {
        let data = temp_dir("ids");
        let (repo, _) = repo(&data);
        let mut db = block_on(repo.open("HOL.item1", "herozero.jar"));
        assert_eq!(block_on(db.add(b"a")), Some(1));
        assert_eq!(block_on(db.add(b"b")), Some(2));
        assert!(block_on(db.delete(1)));
        assert_eq!(block_on(db.add(b"c")), Some(3));
        let _ = fs::remove_dir_all(&data);
    }
}
