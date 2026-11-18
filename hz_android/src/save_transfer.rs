//! 세이브 옮기기 (B4, 2026-10-26 — hero2's `save_transfer.rs`, its user: *"A핸드폰에서 B핸드폰으로
//! 세이브 데이터를 추출해서 보내고싶어"*): the one file that carries a phone's 영웅서기 Zero saves to
//! another. Nothing else can — the records are app-private, a release build is not debuggable and
//! `allowBackup=false` keeps them out of every backup — and before this a release build could only
//! copy them OUT (`saves::export`), never put them back IN.
//!
//! WHAT TRAVELS: the game's own records, byte for byte, as `crate::database` keeps them
//! (`<data>/herozero.jar/db/<store>/1`) — the receiving phone reads them with the code that wrote
//! them: each slot's three stores (`HOL.slot<n>` the summary the slot list reads, `HOL.gamedata<n>`,
//! `HOL.item<n>` — a slot travels whole or not at all, save-integrity's "two record families" rule),
//! the 계정창고 the slots share (`HOL.storage`) and `HOL.gamestore`. Not the vault (each phone keeps
//! its own history), not the game's options record `d` nor any host file: those belong to the phone.
//!
//! FORMAT: a plain zip (stored — a PC opens it) of the manifest `herozero_save.txt` and `db/<store>`:
//!   format herozero-save 1
//!   game <jar md5>     the build whose records these are (`hz_patches::Build`) — another one's refused
//!   created <unix ms>  when it was made
//!   app <versionCode>  the app that made it — a NEWER app's file is refused
//!   legend <n>         how many 85제 pieces (`hz_java::legend::COUNT`) and recipes
//!   recipes <n>        (`legend::RECIPE_COUNT`) the app that made it knows — a file from an app that
//!                      knows MORE is refused (2026-10-30: the PC's build 48 knew 102 pieces, the
//!                      phones' vC 48 knew 36; the game's loader drops an item whose record it cannot
//!                      find, and the next save writes the bag without it — measured with the probe,
//!                      `HOL.item1` 157 → 117 B). A file without these lines is an older app's.
//!   specials <n>       the special items (`/itm/20` 0..n−1, `hz_java::elixir::SPECIALS` — 11 with
//!                      HZ-060's 엘릭서) it knows, the same rule; no line = an older app's
//!   saved <n> <ms>     when slot n was last saved (the dialogs name a slot by it: the summary
//!                      record is the game's own cipher)
//!   quick <n> <job> …  HZ-058: the quick plates of slot n's character (`hz_java::quickpage` — the
//!                      app's, not the game's: seven × two pages over the game's six slots); an older
//!                      app skips the line
//! ⛔ Only the stores [`names`] lists are ever read or written, and no path is ever taken from the
//! zip: a member outside the list makes the whole file invalid.

use std::io::{Cursor, Read, Write};

/// The manifest member.
pub const MANIFEST: &str = "herozero_save.txt";
/// Its first line's value. Bump the number for a change an older build must refuse.
pub const FORMAT: &str = "herozero-save 1";
/// The member directory of the stores.
pub const DB: &str = "db";
/// The stores every slot shares: the 계정창고, and `HOL.gamestore` — which a file without one leaves
/// as the phone has it (the game rewrites it at boot; meaning not pinned, `crate::vault`).
pub const SHARED: [&str; 2] = ["HOL.storage", "HOL.gamestore"];
/// The shared store a file without one does NOT remove.
pub const KEPT_IF_ABSENT: &str = "HOL.gamestore";

/// Every store a transfer file may hold — the whitelist, and the exact set of records an import owns:
/// the slots' three stores (`crate::saves::stores`), slot by slot, then [`SHARED`].
pub fn names() -> Vec<String> {
    (1..=crate::saves::SLOTS).flat_map(crate::saves::stores).chain(SHARED.iter().map(|s| s.to_string())).collect()
}

/// Largest member: far above any record (a save is 45 + 379 + 117 B) — a sanity limit.
pub const MEMBER_MAX: usize = 64 * 1024;
/// Largest transfer file. A real one is a few KB.
pub const FILE_MAX: usize = 1024 * 1024;
const MEMBERS_MAX: usize = 64;

/// One transfer file, read or about to be written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bundle {
    /// The md5 of the game's jar (`hz_patches::Build::jar_md5`).
    pub game: String,
    pub created_ms: u64,
    /// The versionCode of the app that made it (0 = unknown).
    pub app: u32,
    /// The 85제 pieces and recipes the app that made it knows (0 = it does not say: an older app).
    pub legend: u32,
    pub recipes: u32,
    /// The special items it knows (0 = it does not say).
    pub specials: u32,
    /// When each slot it carries was saved, `(slot, unix ms)` — as the manifest says.
    pub saved: Vec<(u8, u64)>,
    /// HZ-058: the plates of the slots it carries, as manifest lines (`hz_java::quickpage::Store::lines_for`).
    pub quick: Vec<String>,
    /// `(store, record bytes)`.
    pub files: Vec<(String, Vec<u8>)>,
}

impl Bundle {
    pub fn get(&self, store: &str) -> Option<&[u8]> {
        self.files.iter().find(|(s, _)| s == store).map(|(_, b)| b.as_slice())
    }

    /// The slots it carries — whole, once [`decode`] has accepted it.
    pub fn slots(&self) -> Vec<u8> {
        (1..=crate::saves::SLOTS).filter(|&n| crate::saves::stores(n).iter().all(|s| self.get(s).is_some_and(|b| !b.is_empty()))).collect()
    }

    /// When slot `n` was saved, as the manifest says (0 = it does not say).
    pub fn saved_ms(&self, n: u8) -> u64 {
        self.saved.iter().find(|(s, _)| *s == n).map_or(0, |(_, ms)| *ms)
    }

    pub fn has_storage(&self) -> bool {
        self.get(SHARED[0]).is_some()
    }
}

/// A game id the manifest may carry: an md5, 32 lower-case hex digits.
pub fn game_ok(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Write a transfer file. `Err` is a sentence for the player (the cause is logged).
pub fn encode(b: &Bundle) -> Result<Vec<u8>, String> {
    let fail = |what: &str, e: &dyn std::fmt::Display| {
        log::warn!("save transfer: encode {what}: {e}");
        "세이브 파일을 만들지 못했습니다.".to_string()
    };
    let allowed = names();
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut manifest = format!(
        "format {FORMAT}\ngame {}\ncreated {}\napp {}\nlegend {}\nrecipes {}\nspecials {}\n",
        b.game, b.created_ms, b.app, b.legend, b.recipes, b.specials
    );
    for (n, ms) in &b.saved {
        manifest += &format!("saved {n} {ms}\n");
    }
    for line in &b.quick {
        manifest += line.trim();
        manifest += "\n";
    }
    w.start_file(MANIFEST, opts).map_err(|e| fail(MANIFEST, &e))?;
    w.write_all(manifest.as_bytes()).map_err(|e| fail(MANIFEST, &e))?;
    for (store, bytes) in &b.files {
        if !allowed.contains(store) {
            return Err(fail("member", &format!("{store} is not a transfer store")));
        }
        let member = format!("{DB}/{store}");
        w.start_file(member.as_str(), opts).map_err(|e| fail(&member, &e))?;
        w.write_all(bytes).map_err(|e| fail(&member, &e))?;
    }
    Ok(w.finish().map_err(|e| fail("finish", &e))?.into_inner())
}

/// Read and check a transfer file. `Err` is a sentence for the player (the cause is logged).
///
/// Accepted only when: a zip of at most [`FILE_MAX`] bytes and [`MEMBERS_MAX`] members, every member
/// the manifest or `db/<store>` for a store [`names`] lists (each once, at most [`MEMBER_MAX`], its
/// CRC intact), the manifest says [`FORMAT`] with a [`game_ok`] game, and every slot it carries is
/// WHOLE — its three stores, none empty — and at least one. Half a slot next to another moment's
/// other half is worse than no slot (the slot list reads the summary alone).
pub fn decode(bytes: &[u8]) -> Result<Bundle, String> {
    const NOT_OURS: &str = "영웅서기 Zero 세이브 파일이 아닙니다. 내보내기로 만든 .zip 파일을 압축을 풀지 말고 그대로 골라 주세요.";
    const DAMAGED: &str = "세이브 파일이 손상됐습니다. 보낸 기기에서 다시 내보내 주세요.";
    let refuse = |msg: &str, why: String| {
        log::warn!("save transfer: refused: {why}");
        msg.to_string()
    };
    if bytes.len() > FILE_MAX {
        return Err(refuse(NOT_OURS, format!("{} bytes", bytes.len())));
    }
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| refuse(NOT_OURS, format!("not a zip: {e}")))?;
    if zip.len() > MEMBERS_MAX {
        return Err(refuse(NOT_OURS, format!("{} members", zip.len())));
    }
    let allowed = names();
    let mut manifest: Option<String> = None;
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| refuse(DAMAGED, format!("member {i}: {e}")))?;
        if f.is_dir() {
            continue;
        }
        let member = f.name().to_string();
        if f.size() > MEMBER_MAX as u64 {
            return Err(refuse(NOT_OURS, format!("{member}: {} bytes", f.size())));
        }
        let mut data = Vec::new();
        // Reading to the end is what checks the CRC.
        (&mut f).take(MEMBER_MAX as u64 + 1).read_to_end(&mut data).map_err(|e| refuse(DAMAGED, format!("{member}: {e}")))?;
        if data.len() > MEMBER_MAX {
            return Err(refuse(NOT_OURS, format!("{member}: over {MEMBER_MAX} bytes")));
        }
        if member == MANIFEST {
            if manifest.is_some() {
                return Err(refuse(NOT_OURS, "two manifests".to_string()));
            }
            manifest = Some(String::from_utf8_lossy(&data).into_owned());
            continue;
        }
        let Some(store) = member.strip_prefix(DB).and_then(|r| r.strip_prefix('/')).filter(|s| allowed.iter().any(|a| a == s)) else {
            return Err(refuse(NOT_OURS, format!("{member:?} is not a transfer store")));
        };
        if files.iter().any(|(s, _)| s == store) {
            return Err(refuse(NOT_OURS, format!("{member} twice")));
        }
        files.push((store.to_string(), data));
    }
    let manifest = manifest.ok_or_else(|| refuse(NOT_OURS, "no manifest".to_string()))?;
    let lines: Vec<&str> = manifest.lines().map(str::trim).collect();
    let field = |key: &str| lines.iter().find_map(|l| l.strip_prefix(key).and_then(|v| v.strip_prefix(' ')).map(str::trim));
    if field("format") != Some(FORMAT) {
        return Err(refuse(
            "다른 버전의 세이브 파일입니다. 두 기기의 앱을 같은 최신 버전으로 맞춘 뒤 다시 내보내 주세요.",
            format!("format {:?}", field("format")),
        ));
    }
    let game = field("game").filter(|g| game_ok(g)).ok_or_else(|| refuse(NOT_OURS, format!("game {:?}", field("game"))))?.to_string();
    let saved = lines
        .iter()
        .filter_map(|l| {
            let mut f = l.strip_prefix("saved ")?.split_whitespace();
            Some((f.next()?.parse().ok()?, f.next()?.parse().ok()?))
        })
        .collect();
    let number = |key: &str| field(key).and_then(|v| v.parse().ok()).unwrap_or(0);
    let b = Bundle {
        game,
        created_ms: field("created").and_then(|v| v.parse().ok()).unwrap_or(0),
        app: number("app"),
        legend: number("legend"),
        recipes: number("recipes"),
        specials: number("specials"),
        saved,
        quick: lines.iter().filter(|l| l.starts_with(hz_java::quickpage::LINE_QUICK)).map(|l| l.to_string()).collect(),
        files,
    };
    for n in 1..=crate::saves::SLOTS {
        let three = crate::saves::stores(n).map(|s| b.get(&s).map(<[u8]>::len));
        let whole = three.iter().all(|l| l.is_some_and(|l| l > 0));
        if three.iter().any(Option::is_some) && !whole {
            return Err(refuse(DAMAGED, format!("slot {n} is not whole: {three:?}")));
        }
    }
    if b.slots().is_empty() {
        return Err(refuse("이 파일에는 세이브가 없습니다. 보낸 폰에서 먼저 게임을 저장한 뒤 다시 내보내 주세요.", "no slot".to_string()));
    }
    Ok(b)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const GAME: &str = "850ad70748b9e1e48b30f6fa60164229";

    pub(crate) fn sample() -> Bundle {
        Bundle {
            game: GAME.into(),
            created_ms: 1_793_000_000_000,
            app: 35,
            legend: 0,
            recipes: 0,
            specials: 0,
            quick: vec!["quick 1 0 s4 - - - i2 - - - - - - - - -".into()],
            saved: vec![(1, 1_792_900_000_000), (5, 1_792_950_000_000)],
            files: vec![
                ("HOL.slot1".into(), vec![1u8; 45]),
                ("HOL.gamedata1".into(), vec![2u8; 379]),
                ("HOL.item1".into(), vec![3u8; 117]),
                ("HOL.slot5".into(), vec![4u8; 45]),
                ("HOL.gamedata5".into(), vec![5u8; 381]),
                ("HOL.item5".into(), vec![6u8; 120]),
                ("HOL.storage".into(), vec![7u8; 90]),
            ],
        }
    }

    fn zip_of(members: &[(&str, &[u8])]) -> Vec<u8> {
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, data) in members {
            w.start_file(*name, opts).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    const MANIFEST_OK: &[u8] = b"format herozero-save 1\ngame 850ad70748b9e1e48b30f6fa60164229\ncreated 5\napp 35\n";

    fn slot(n: u8) -> Vec<(String, Vec<u8>)> {
        crate::saves::stores(n).into_iter().map(|s| (format!("{DB}/{s}"), vec![n; 40])).collect()
    }

    fn with(manifest: &[u8], extra: &[(&str, &[u8])]) -> Vec<u8> {
        let one = slot(1);
        let mut m: Vec<(&str, &[u8])> = vec![(MANIFEST, manifest)];
        m.extend(one.iter().map(|(n, b)| (n.as_str(), b.as_slice())));
        m.extend_from_slice(extra);
        zip_of(&m)
    }

    #[test]
    fn a_bundle_reads_back_exactly() {
        let b = sample();
        assert_eq!(decode(&encode(&b).unwrap()).unwrap(), b);
        assert_eq!(b.slots(), vec![1, 5]);
        assert_eq!((b.saved_ms(5), b.saved_ms(2)), (1_792_950_000_000, 0));
        assert!(b.has_storage());
        assert_eq!(names().len(), 3 * usize::from(crate::saves::SLOTS) + 2);
    }

    #[test]
    fn only_listed_stores_are_accepted() {
        assert_eq!(decode(&with(MANIFEST_OK, &[])).unwrap().slots(), vec![1]);
        for stranger in ["../evil", "db/../../evil", "db/HOL.slot11", "db/d", "db/TELEM_INFO", "db/HOL.slot1/1", "HOL.storage", "save_vault/HOL.slot1"] {
            assert!(decode(&with(MANIFEST_OK, &[(stranger, b"x")])).is_err(), "{stranger} must be refused");
        }
        assert!(decode(&zip_of(&[("db/HOL.storage", b"x")])).is_err(), "no manifest");
        assert!(decode(b"PK not really").is_err() && decode(b"").is_err());
    }

    #[test]
    fn a_slot_travels_only_whole() {
        let lone = zip_of(&[(MANIFEST, MANIFEST_OK), ("db/HOL.slot2", b"summary")]);
        assert!(decode(&lone).is_err(), "a lone summary would list a character that is not there");
        assert!(decode(&with(MANIFEST_OK, &[("db/HOL.slot3", b"s"), ("db/HOL.gamedata3", b"g"), ("db/HOL.item3", b"")])).is_err(), "an empty store");
        assert!(decode(&zip_of(&[(MANIFEST, MANIFEST_OK), ("db/HOL.storage", b"bag")])).is_err(), "no slot at all");
    }

    #[test]
    fn what_the_maker_knows_travels_and_an_older_file_says_nothing() {
        let mut b = sample();
        (b.legend, b.recipes, b.specials) = (102, 100, 11);
        let back = decode(&encode(&b).unwrap()).unwrap();
        assert_eq!((back.legend, back.recipes, back.specials), (102, 100, 11));
        let old = decode(&with(MANIFEST_OK, &[])).unwrap();
        assert_eq!((old.legend, old.recipes, old.specials), (0, 0, 0), "vC 48 and before wrote no such lines");
    }

    #[test]
    fn the_manifest_is_checked() {
        assert!(decode(&with(b"format herozero-save 2\ngame 850ad70748b9e1e48b30f6fa60164229\n", &[])).is_err(), "a newer format");
        assert!(decode(&with(b"format herozero-save 1\ngame ../x\n", &[])).is_err(), "not a game id");
        assert!(decode(&with(b"format herozero-save 1\n", &[])).is_err(), "no game");
        let crlf = decode(&with(b"format herozero-save 1\r\ngame 850ad70748b9e1e48b30f6fa60164229\r\ncreated 7\r\napp 35\r\nsaved 1 9\r\n", &[])).unwrap();
        assert_eq!((crlf.created_ms, crlf.app, crlf.saved_ms(1)), (7, 35, 9), "a manifest a Windows editor touched");
    }
}
