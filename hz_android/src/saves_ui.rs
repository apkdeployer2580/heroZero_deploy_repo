//! 진단 정보 · 세이브 관리 · 세이브 옮기기 as host windows (`crate::hostwin`) — the PC's (2026-10-29)
//! stand-in for `MainActivity`'s dialogs (B3, B4), over the same `crate::saves` the JNI side uses.
//! A function of the files and a [`Ctx`]: every window is built from what is on disk when it opens,
//! and what a row does ([`perform`]) returns the next window, a folder to open, or the app's end —
//! the app (`crate::app`) carries that out. Host-tested with a temporary data directory.
//!
//! The PC's folders: the saves and options under the data directory (`%APPDATA%\heroZero`), the
//! player's own under `Ctx::ext_dir` (`Documents\heroZero`): 세이브 폴더로 내보내기 copies into its
//! `save_export/`, 다른 기기로 보내기 writes a transfer zip into it, 다른 기기에서 받기 lists the
//! zips the player put in its [`IMPORT_DIR`] — there is no share sheet and no file picker here; a
//! folder the file manager opens is the PC's way.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    hostwin::{HostAct, HostWin, Row, Text},
    saves,
};

/// Where the player puts a transfer zip to receive it, under `Ctx::ext_dir`.
pub const IMPORT_DIR: &str = "save_import";

/// 진단 정보's facts the app knows (the files' side is read here).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagInfo {
    /// What the app calls itself: `PC 0.1.0 (48)`.
    pub version: String,
    pub window: (u32, u32),
    pub guest: (u32, u32),
    pub sfx_db: i32,
    /// A developer's `hz_patches.txt` is in force.
    pub patches_file: bool,
}

/// What the windows read and write.
pub struct Ctx<'a> {
    pub data_dir: &'a Path,
    /// The player's folder: exports, the transfer zips, the import folder.
    pub ext_dir: &'a Path,
    /// This build's versionCode (0 = unknown: `saves::stage_import` then skips the newer-app check).
    pub app: u32,
    /// The game this app runs (`saves::game`), for the transfer file.
    pub game: Option<&'a str>,
    pub now_ms: u64,
    /// A moment as the player reads it — the desktop's local clock; [`utc_stamp`] by default.
    pub fmt_time: fn(u64) -> String,
    pub diag: DiagInfo,
}

/// What a row's action came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// Show this window (in place of the one up).
    Window(HostWin),
    /// Close the window.
    Close,
    /// End the app now.
    Quit,
    /// Open this folder in the file manager (the window stays).
    Folder(PathBuf),
}

/// Files under `dir`, recursively.
pub fn count_files(dir: &Path) -> usize {
    fs::read_dir(dir).map_or(0, |rd| rd.filter_map(|e| e.ok()).map(|e| if e.path().is_dir() { count_files(&e.path()) } else { 1 }).sum())
}

/// `ms` since the epoch as `YYYY-MM-DD HH:MM` in UTC (Howard Hinnant's civil-from-days); the
/// desktop passes its own local-time formatter.
pub fn utc_stamp(ms: u64) -> String {
    let secs = ms / 1000;
    let days = (secs / 86_400) as i64;
    let (hh, mm) = ((secs % 86_400) / 3600, (secs % 3600) / 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}")
}

/// A stamp as a file name takes it: `2026-10-29_2040`.
fn file_stamp(t: &str) -> String {
    t.chars().filter_map(|c| if c.is_ascii_alphanumeric() || c == '-' { Some(c) } else if c == ' ' { Some('_') } else { None }).collect()
}

/// A file name as a row shows it: ASCII in the 5×7 font, a name with Korean in it in the game's —
/// the phone names its file 「영웅서기제로_세이브_….zip」, which the 5×7 font drew as `-   -2026….ZIP`
/// (2026-10-30). The game's font takes `_` as a gap code: it becomes a space ([`Text::dynamic`]).
fn name_text(name: String) -> Text {
    if name.is_ascii() { Text::ascii(name) } else { Text::dynamic(name) }
}

/// A folder or file path the same way, `/` between its parts — the player's 문서 is `OneDrive\문서`
/// on a Korean Windows that syncs it, and a user name may be Korean.
fn path_text(p: &Path) -> Text {
    let s = p.display().to_string();
    if s.is_ascii() { Text::ascii(s) } else { Text::dynamic(s.replace('\\', "/")) }
}

fn list(v: &[u8]) -> String {
    v.iter().map(u8::to_string).collect::<Vec<_>>().join(" ")
}

/// 진단 정보: what this install holds, then 세이브 관리 and the player's folder.
pub fn diag_window(ctx: &Ctx) -> HostWin {
    let d = &ctx.diag;
    let mut rows = vec![
        Row::pair(Text::Line("APP"), Text::ascii(&d.version)),
        Row::pair(Text::Line("WINDOW"), Text::ascii(format!("{}x{} / {}x{}", d.window.0, d.window.1, d.guest.0, d.guest.1))),
    ];
    for line in saves::slot_report(&saves::slots(ctx.data_dir)) {
        rows.push(Row::text(Text::dynamic(line.trim())));
    }
    rows.push(Row::pair(Text::Line("VAULT FILES"), Text::ascii(count_files(&ctx.data_dir.join("save_vault")).to_string())));
    rows.push(Row::pair(Text::Line("SFX LEVEL"), Text::Line(crate::audio_mix::sfx_db_word(d.sfx_db))));
    rows.push(Row::pair(Text::Line("PATCHES"), Text::Line(if d.patches_file { "FROM FILE" } else { "DEFAULT SET" })));
    rows.push(Row::text(Text::Line("EXPORT FOLDER")));
    rows.push(Row::text(path_text(ctx.ext_dir)));
    rows.push(Row::text(Text::Line("KEYS: ARROWS MOVE - X ATTACK - SPACE ENTER TALK - ESC CLR - F5 SAVE - M MAP - F11 FULL")));
    rows.push(Row::button(Text::Line("SAVES"), HostAct::OpenSaves));
    rows.push(Row::button(Text::Line("OPEN FOLDER"), HostAct::OpenFolder(ctx.ext_dir.to_path_buf())));
    HostWin::new(Text::Line("DIAG"), rows)
}

/// 세이브 관리: export, send, receive, and the vault's restore points (`saves::offer_lines`).
pub fn saves_window(ctx: &Ctx) -> HostWin {
    let mut rows = vec![
        Row::button(Text::Line("EXPORT TO FOLDER"), HostAct::Export),
        Row::button(Text::Line("SEND TO ANOTHER DEVICE"), HostAct::TransferExport),
        Row::button(Text::Line("RECEIVE FROM ANOTHER DEVICE"), HostAct::TransferList),
        Row::text(Text::Line("RESTORE FROM BACKUP")),
    ];
    let mut any = false;
    for line in saves::offer_lines(ctx.data_dir).lines() {
        let f: Vec<&str> = line.split('|').collect();
        if f.len() < 4 {
            continue;
        }
        let (slot, ms, bytes, gone) = (f[0], f[1].parse::<u64>().unwrap_or(0), f[2], f[3] == "1");
        any = true;
        let text = format!("슬롯 {slot} - {} - {bytes} B{}", (ctx.fmt_time)(ms), if gone { " (지금 비어 있음)" } else { "" });
        rows.push(Row::button(Text::dynamic(text), HostAct::RestorePoint(format!("{slot}|{ms}"))));
    }
    if !any {
        rows.push(Row::text(Text::Line("NO RESTORE POINTS")));
    }
    rows.push(Row::button(Text::Line("DIAG"), HostAct::OpenDiag));
    HostWin::new(Text::Line("SAVES"), rows)
}

/// The boot's account of a 세이브 옮기기 import (`saves::ImportReport::notice`): its first line is
/// the title, the rest the window's text.
pub fn message_window(text: &str) -> HostWin {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let title = lines.next().map_or(Text::Line("SAVES"), Text::dynamic);
    let mut rows: Vec<Row> = lines.map(|l| Row::text(Text::dynamic(l))).collect();
    rows.push(Row::button(Text::Line("CLOSE"), HostAct::Close));
    HostWin::new(title, rows)
}

fn export_window(ctx: &Ctx) -> HostWin {
    match saves::export(ctx.data_dir, ctx.ext_dir, ctx.now_ms) {
        Ok((dir, n)) => HostWin::new(
            Text::Line("EXPORT TO FOLDER"),
            vec![
                Row::text(Text::dynamic(format!("세이브 파일 {n}개를 복사했습니다"))),
                Row::text(path_text(&dir)),
                Row::button(Text::Line("OPEN FOLDER"), HostAct::OpenFolder(dir)),
                Row::button(Text::Line("SAVES"), HostAct::OpenSaves),
            ],
        ),
        Err(e) => {
            log::error!("saves: export failed: {e}");
            HostWin::new(
                Text::Line("EXPORT TO FOLDER"),
                vec![Row::text(Text::Line("EXPORT FAILED - STORAGE ERROR")), Row::text(Text::ascii(e.to_string())), Row::button(Text::Line("SAVES"), HostAct::OpenSaves)],
            )
        }
    }
}

fn transfer_export_window(ctx: &Ctx) -> HostWin {
    let out = ctx.ext_dir.join(format!("herozero_save_{}.zip", file_stamp(&(ctx.fmt_time)(ctx.now_ms))));
    match saves::export_to(ctx.data_dir, ctx.game, ctx.app, &out, ctx.now_ms) {
        Ok(slots) => HostWin::new(
            Text::Line("SEND TO ANOTHER DEVICE"),
            vec![
                Row::text(Text::dynamic(format!("슬롯 {} 을 담았습니다", list(&slots)))),
                Row::text(path_text(&out)),
                Row::text(Text::Line("COPY THIS FILE TO THE OTHER DEVICE")),
                Row::text(Text::Line("ON A PHONE: SAVES > RECEIVE")),
                Row::button(Text::Line("OPEN FOLDER"), HostAct::OpenFolder(ctx.ext_dir.to_path_buf())),
                Row::button(Text::Line("SAVES"), HostAct::OpenSaves),
            ],
        ),
        Err(msg) => HostWin::new(Text::Line("SEND TO ANOTHER DEVICE"), vec![Row::text(Text::dynamic(msg)), Row::button(Text::Line("SAVES"), HostAct::OpenSaves)]),
    }
}

/// The zips in the import folder, newest name first.
pub fn import_zips(ext_dir: &Path) -> Vec<PathBuf> {
    let dir = ext_dir.join(IMPORT_DIR);
    let _ = fs::create_dir_all(&dir);
    let mut zips: Vec<PathBuf> = fs::read_dir(&dir)
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("zip"))).collect())
        .unwrap_or_default();
    zips.sort();
    zips.reverse();
    zips
}

fn transfer_list_window(ctx: &Ctx) -> HostWin {
    let dir = ctx.ext_dir.join(IMPORT_DIR);
    let mut rows = vec![
        Row::text(Text::Line("PUT THE ZIP IN THIS FOLDER AND PICK IT")),
        Row::text(path_text(&dir)),
        Row::button(Text::Line("OPEN FOLDER"), HostAct::OpenFolder(dir)),
    ];
    let zips = import_zips(ctx.ext_dir);
    if zips.is_empty() {
        rows.push(Row::text(Text::Line("NO ZIP FILE IN THE FOLDER")));
    }
    for p in zips {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        rows.push(Row::button(name_text(name), HostAct::TransferPick(p)));
    }
    rows.push(Row::button(Text::Line("SAVES"), HostAct::OpenSaves));
    HostWin::new(Text::Line("RECEIVE FROM ANOTHER DEVICE"), rows)
}

fn transfer_pick_window(ctx: &Ctx, path: &Path) -> HostWin {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let title = Text::Line("RECEIVE FROM ANOTHER DEVICE");
    match saves::import_summary(ctx.data_dir, ctx.game, ctx.app, path) {
        Ok(summary) => {
            let mut rows = vec![Row::text(name_text(name))];
            let (mut file_slots, mut here_slots) = (Vec::new(), Vec::new());
            let mut storage = false;
            for line in summary.lines() {
                let f: Vec<&str> = line.split('|').collect();
                match f[0] {
                    "created" => rows.push(Row::pair(Text::Line("MADE AT"), Text::ascii((ctx.fmt_time)(f.get(1).and_then(|v| v.parse().ok()).unwrap_or(0))))),
                    "app" => rows.push(Row::pair(Text::Line("APP VERSION"), Text::ascii(f.get(1).copied().unwrap_or("?")))),
                    "file" => file_slots.push(f.get(1).and_then(|v| v.parse::<u8>().ok()).unwrap_or(0)),
                    "extra" => storage = true,
                    "here" => here_slots.push(f.get(1).and_then(|v| v.parse::<u8>().ok()).unwrap_or(0)),
                    _ => {}
                }
            }
            rows.push(Row::pair(Text::Line("SLOTS IN THE FILE"), Text::ascii(list(&file_slots))));
            if storage {
                rows.push(Row::text(Text::Line("WITH THE ACCOUNT STORAGE")));
            }
            if !here_slots.is_empty() {
                rows.push(Row::pair(Text::Line("SAVES ON THIS PC GO TO THE VAULT"), Text::ascii(list(&here_slots))));
            }
            rows.push(Row::text(Text::Line("APPLIES AT THE NEXT START")));
            rows.push(Row::button(Text::Line("RECEIVE"), HostAct::TransferConfirm(path.to_path_buf())));
            rows.push(Row::button(Text::Line("CANCEL"), HostAct::TransferList));
            HostWin::new(title, rows)
        }
        Err(msg) => HostWin::new(title, vec![Row::text(name_text(name)), Row::text(Text::dynamic(msg)), Row::button(Text::Line("CANCEL"), HostAct::TransferList)]),
    }
}

fn transfer_confirm_window(ctx: &Ctx, path: &Path) -> HostWin {
    match saves::stage_import(ctx.data_dir, ctx.game, ctx.app, path) {
        Ok(()) => HostWin::new(
            Text::Line("IMPORT SCHEDULED"),
            vec![Row::text(Text::Line("APPLIES AT THE NEXT START")), Row::button(Text::Line("QUIT NOW"), HostAct::Quit), Row::button(Text::Line("LATER"), HostAct::Close)],
        ),
        Err(msg) => HostWin::new(Text::Line("RECEIVE FROM ANOTHER DEVICE"), vec![Row::text(Text::dynamic(msg)), Row::button(Text::Line("CANCEL"), HostAct::TransferList)]),
    }
}

fn restore_point_window(ctx: &Ctx, key: &str) -> HostWin {
    let mut f = key.split('|');
    let slot = f.next().unwrap_or("?");
    let ms: u64 = f.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    HostWin::new(
        Text::Line("RESTORE FROM BACKUP"),
        vec![
            Row::text(Text::dynamic(format!("슬롯 {slot} 을 이 시점으로 되돌립니다: {}", (ctx.fmt_time)(ms)))),
            Row::text(Text::Line("APPLIES AT THE NEXT START")),
            Row::text(Text::Line("THE CURRENT SAVE GOES TO THE VAULT")),
            Row::button(Text::Line("SCHEDULE"), HostAct::RestoreConfirm(key.to_string())),
            Row::button(Text::Line("CANCEL"), HostAct::OpenSaves),
        ],
    )
}

fn restore_confirm_window(ctx: &Ctx, key: &str) -> HostWin {
    match saves::request_offer(ctx.data_dir, key) {
        Ok(slot) => HostWin::new(
            Text::Line("RESTORE SCHEDULED"),
            vec![
                Row::text(Text::dynamic(format!("슬롯 {slot}"))),
                Row::text(Text::Line("APPLIES AT THE NEXT START")),
                Row::button(Text::Line("QUIT NOW"), HostAct::Quit),
                Row::button(Text::Line("LATER"), HostAct::Close),
            ],
        ),
        Err(e) => {
            log::warn!("saves: restore request failed: {e}");
            HostWin::new(Text::Line("RESTORE FROM BACKUP"), vec![Row::text(Text::ascii(e.to_string())), Row::button(Text::Line("SAVES"), HostAct::OpenSaves)])
        }
    }
}

/// Carry out `act`.
pub fn perform(ctx: &Ctx, act: HostAct) -> Reply {
    match act {
        HostAct::Close => Reply::Close,
        HostAct::Quit => Reply::Quit,
        HostAct::OpenFolder(p) => {
            let _ = fs::create_dir_all(&p);
            Reply::Folder(p)
        }
        HostAct::OpenDiag => Reply::Window(diag_window(ctx)),
        HostAct::OpenSaves => Reply::Window(saves_window(ctx)),
        HostAct::Export => Reply::Window(export_window(ctx)),
        HostAct::TransferExport => Reply::Window(transfer_export_window(ctx)),
        HostAct::TransferList => Reply::Window(transfer_list_window(ctx)),
        HostAct::TransferPick(p) => Reply::Window(transfer_pick_window(ctx, &p)),
        HostAct::TransferConfirm(p) => Reply::Window(transfer_confirm_window(ctx, &p)),
        HostAct::RestorePoint(key) => Reply::Window(restore_point_window(ctx, &key)),
        HostAct::RestoreConfirm(key) => Reply::Window(restore_confirm_window(ctx, &key)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hz_saves_ui_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn ctx<'a>(data: &'a Path, ext: &'a Path) -> Ctx<'a> {
        Ctx {
            data_dir: data,
            ext_dir: ext,
            app: 48,
            game: None,
            now_ms: 1_793_000_000_000,
            fmt_time: utc_stamp,
            diag: DiagInfo { version: "PC 0.1.0 (48)".into(), window: (1280, 720), guest: (426, 240), sfx_db: 0, patches_file: false },
        }
    }

    #[test]
    fn the_utc_stamp_is_civil_time() {
        assert_eq!(utc_stamp(0), "1970-01-01 00:00");
        assert_eq!(utc_stamp(1_761_700_800_000), "2025-10-29 01:20");
        assert_eq!(file_stamp("2026-10-29 20:40"), "2026-10-29_2040");
    }

    #[test]
    fn the_diagnosis_window_lists_the_install_and_leads_to_the_saves() {
        let (data, ext) = (tmp("diag_data"), tmp("diag_ext"));
        let w = diag_window(&ctx(&data, &ext));
        assert_eq!(w.title, Text::Line("DIAG"));
        assert_eq!(w.rows[0].value, Some(Text::ascii("PC 0.1.0 (48)")));
        assert_eq!(w.rows[1].value, Some(Text::ascii("1280x720 / 426x240")));
        assert!(w.rows.iter().any(|r| r.act == Some(HostAct::OpenSaves)));
        assert!(w.rows.iter().any(|r| r.act == Some(HostAct::OpenFolder(ext.clone()))));
        assert_eq!(w.cursor, w.rows.iter().position(|r| r.act.is_some()), "the keyboard starts on the first plate");
        // The slot report's lines are the game's font's: no `·`, no `,`.
        for r in &w.rows {
            if let Text::Dyn { text, .. } = &r.text {
                assert!(!text.contains('·') && !text.contains(','), "{text}");
            }
        }
    }

    #[test]
    fn the_saves_window_has_the_three_ways_and_no_restore_points_on_a_fresh_install() {
        let (data, ext) = (tmp("saves_data"), tmp("saves_ext"));
        let w = saves_window(&ctx(&data, &ext));
        let acts: Vec<_> = w.rows.iter().filter_map(|r| r.act.clone()).collect();
        assert_eq!(acts, vec![HostAct::Export, HostAct::TransferExport, HostAct::TransferList, HostAct::OpenDiag]);
        assert!(w.rows.iter().any(|r| r.text == Text::Line("NO RESTORE POINTS")));
    }

    #[test]
    fn receiving_lists_the_zips_in_the_import_folder_and_refuses_what_is_not_a_save() {
        let (data, ext) = (tmp("recv_data"), tmp("recv_ext"));
        let c = ctx(&data, &ext);
        let Reply::Window(w) = perform(&c, HostAct::TransferList) else { panic!() };
        assert!(ext.join(IMPORT_DIR).is_dir(), "the folder is made so the player can find it");
        assert!(w.rows.iter().any(|r| r.text == Text::Line("NO ZIP FILE IN THE FOLDER")));
        let bogus = ext.join(IMPORT_DIR).join("a.zip");
        fs::write(&bogus, b"not a zip").unwrap();
        let Reply::Window(w) = perform(&c, HostAct::TransferList) else { panic!() };
        assert!(w.rows.iter().any(|r| r.act == Some(HostAct::TransferPick(bogus.clone()))));
        let Reply::Window(w) = perform(&c, HostAct::TransferPick(bogus.clone())) else { panic!() };
        assert!(w.rows.iter().any(|r| r.act == Some(HostAct::TransferList)), "a way back");
        assert!(!w.rows.iter().any(|r| matches!(r.act, Some(HostAct::TransferConfirm(_)))), "nothing to confirm");
        // Nothing was staged.
        assert!(!saves::import_pending(&data));
        // The phone's own name for its file is Korean: the game's font letters it.
        let phone = ext.join(IMPORT_DIR).join("영웅서기제로_세이브_20261030_0100.zip");
        fs::write(&phone, b"not a zip either").unwrap();
        let Reply::Window(w) = perform(&c, HostAct::TransferList) else { panic!() };
        let row = w.rows.iter().find(|r| r.act == Some(HostAct::TransferPick(phone.clone()))).unwrap();
        assert_eq!(row.text, Text::dynamic("영웅서기제로_세이브_20261030_0100.zip"));
        assert_eq!(row.text, Text::Dyn { id: 0, text: "영웅서기제로 세이브 20261030 0100.zip".into() });
        assert_eq!(path_text(Path::new("C:\\Users\\a\\OneDrive\\문서\\heroZero")), Text::dynamic("C:/Users/a/OneDrive/문서/heroZero"));
        assert_eq!(path_text(Path::new("C:\\Users\\a\\Documents")), Text::ascii("C:\\Users\\a\\Documents"));
    }

    #[test]
    fn exporting_on_a_fresh_install_says_so_and_sending_refuses_without_a_save() {
        let (data, ext) = (tmp("exp_data"), tmp("exp_ext"));
        let c = ctx(&data, &ext);
        let Reply::Window(w) = perform(&c, HostAct::Export) else { panic!() };
        assert_eq!(w.title, Text::Line("EXPORT TO FOLDER"));
        assert!(matches!(&w.rows[0].text, Text::Dyn { text, .. } if text.contains("0개")), "{:?}", w.rows[0].text);
        let Reply::Window(w) = perform(&c, HostAct::TransferExport) else { panic!() };
        assert!(matches!(&w.rows[0].text, Text::Dyn { .. }), "the refusal, in Korean: {:?}", w.rows[0].text);
        assert!(w.rows.iter().any(|r| r.act == Some(HostAct::OpenSaves)));
    }

    #[test]
    fn the_simple_replies() {
        let (data, ext) = (tmp("rep_data"), tmp("rep_ext"));
        let c = ctx(&data, &ext);
        assert_eq!(perform(&c, HostAct::Close), Reply::Close);
        assert_eq!(perform(&c, HostAct::Quit), Reply::Quit);
        assert_eq!(perform(&c, HostAct::OpenFolder(ext.join("x"))), Reply::Folder(ext.join("x")));
        assert!(ext.join("x").is_dir());
        let m = message_window("세이브 가져오기 완료\n다른 폰에서 가져온 세이브를 적용했습니다.\n\n가져온 슬롯: 1, 2");
        assert_eq!(m.title, Text::dynamic("세이브 가져오기 완료"));
        assert_eq!(m.rows.len(), 3);
        assert_eq!(m.rows[2].act, Some(HostAct::Close));
    }
}
