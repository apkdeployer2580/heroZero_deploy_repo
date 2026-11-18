//! A host window in the game's own look (the user's choice ④, 2026-10-29: *"게임 글꼴 호스트창"*) —
//! the PC's stand-in for `MainActivity`'s dialogs: 진단 정보 (B3), 세이브 관리 (restore from the
//! vault, export) and 세이브 옮기기 (B4). A title, rows that are text or plates that act, 닫기 at the
//! title's right as the settings window has it (B2), the keyboard's cursor on the plates.
//!
//! Pure: the rows, the cursor and the scroll live here and are tested on the host; the geometry is
//! `crate::hud::host_geom`, the drawing `crate::render`, and what a row DOES is `crate::saves_ui`
//! (`HostAct`) — carried out by the app, which then opens the next window.
//!
//! Text comes three ways ([`Text`]): a host line lettered at boot (`crate::host_text`, by its ASCII
//! key), a line made at run time — a date, a slot's size — lettered on demand in the game's font
//! (`Hud::open_host` numbers it, the worker letters it a few a frame), or ASCII the game's font must
//! not get (a path: `\` and `_` are its colour and gap codes) in the host's 5×7 font.

use std::path::PathBuf;

use wie_backend::KeyCode;

/// A line of a host window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Text {
    /// A host line by its ASCII key (`crate::host_text::LINES`): Korean in the game's font once
    /// lettered at boot, the key in the 5×7 font before.
    Line(&'static str),
    /// A line made at run time, lettered on demand under `id` (0 until `Hud::open_host` numbers
    /// it); `text` is drawn in the 5×7 font meanwhile — its ASCII, upper-cased, the Korean
    /// skipped — and for good when the game's font cannot letter it.
    Dyn { id: u16, text: String },
    /// ASCII only, always the 5×7 font: paths and file names.
    Ascii(String),
}

impl Text {
    /// A run-time line, with what the game's font cannot draw replaced ([`fontable`]).
    pub fn dynamic(text: impl AsRef<str>) -> Text {
        Text::Dyn { id: 0, text: fontable(text.as_ref()) }
    }

    pub fn ascii(text: impl Into<String>) -> Text {
        Text::Ascii(text.into())
    }

    /// What the 5×7 font draws for it (`crate::font` has capitals, digits and a little punctuation:
    /// a path's `\` becomes `/`, `_` a dash).
    pub fn fallback(&self) -> String {
        match self {
            Text::Line(k) => (*k).to_string(),
            Text::Dyn { text, .. } | Text::Ascii(text) => text.to_ascii_uppercase().replace('\\', "/").replace('_', "-"),
        }
    }
}

/// `s` as the game's font can letter it (`crate::host_text` module docs): its control codes `$ ^ { |
/// } \ & ; _` become spaces, `,` and `·` — not in its ASCII — a space and a dash, `→` a `>`. Hangul
/// outside KS X 1001 cannot be helped: the worker answers no picture and the 5×7 fallback draws.
pub fn fontable(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '$' | '^' | '{' | '|' | '}' | '\\' | '&' | ';' | '_' | ',' => ' ',
            '·' | '…' => '-',
            '→' | '›' => '>',
            c => c,
        })
        .collect()
}

/// What a row does when chosen (`crate::saves_ui::perform` carries it out).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostAct {
    /// 닫기: the window goes away.
    Close,
    /// 세이브 관리: the saves window.
    OpenSaves,
    /// 진단 정보 again (the saves window's way back).
    OpenDiag,
    /// 세이브 폴더로 내보내기 (`crate::saves::export`).
    Export,
    /// 다른 기기로 보내기: a transfer zip into the export folder (`crate::saves::export_to`).
    TransferExport,
    /// 다른 기기에서 받기: the zips waiting in the import folder.
    TransferList,
    /// A zip from that list: what it holds, before anything is scheduled.
    TransferPick(PathBuf),
    /// Schedule that zip for the next start (`crate::saves::stage_import`).
    TransferConfirm(PathBuf),
    /// A restore point (`slot|ms`, `crate::saves::offer_lines`): confirm first.
    RestorePoint(String),
    /// Schedule that restore for the next start (`crate::saves::request_offer`).
    RestoreConfirm(String),
    /// Open this folder in the system's file manager.
    OpenFolder(PathBuf),
    /// Close the app now (a scheduled restore or import applies at the next start).
    Quit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub text: Text,
    /// A value at the row's right (a setting's state look), if any.
    pub value: Option<Text>,
    /// What the row does; `None` = plain text on the parchment, no plate, no cursor.
    pub act: Option<HostAct>,
}

impl Row {
    pub fn text(t: Text) -> Row {
        Row { text: t, value: None, act: None }
    }

    pub fn pair(t: Text, value: Text) -> Row {
        Row { text: t, value: Some(value), act: None }
    }

    pub fn button(t: Text, act: HostAct) -> Row {
        Row { text: t, value: None, act: Some(act) }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostWin {
    pub title: Text,
    pub rows: Vec<Row>,
    /// The keyboard's row: a plate, or none while the window has no plate.
    pub cursor: Option<usize>,
    /// How far the rows are scrolled (window px, `crate::hud::host_geom`'s pitch a row).
    pub scroll: i32,
}

impl HostWin {
    /// A window with the cursor on its first plate.
    pub fn new(title: Text, rows: Vec<Row>) -> HostWin {
        let cursor = rows.iter().position(|r| r.act.is_some());
        HostWin { title, rows, cursor, scroll: 0 }
    }

    /// The cursor to the next plate down (`down`) or up; whether it moved.
    pub fn step(&mut self, down: bool) -> bool {
        let Some(at) = self.cursor else { return false };
        let next = if down {
            self.rows.iter().enumerate().skip(at + 1).find(|(_, r)| r.act.is_some()).map(|(i, _)| i)
        } else {
            self.rows.iter().enumerate().take(at).rev().find(|(_, r)| r.act.is_some()).map(|(i, _)| i)
        };
        match next {
            Some(i) => {
                self.cursor = Some(i);
                true
            }
            None => false,
        }
    }

    /// What the cursor's row does.
    pub fn act(&self) -> Option<HostAct> {
        self.cursor.and_then(|i| self.rows.get(i)).and_then(|r| r.act.clone())
    }

    /// The run-time lines, numbered from `first` (each `Text::Dyn` gets the next id, the title
    /// included): what the worker is asked to letter, `(id, EUC-KR)` — a line the game's font
    /// cannot take (`hz_java::euc_kr` refuses it) is left to its fallback. The next free id.
    pub fn number(&mut self, first: u16) -> (Vec<(u16, Vec<u8>)>, u16) {
        let mut next = first;
        let mut asks = Vec::new();
        let mut take = |t: &mut Text| {
            if let Text::Dyn { id, text } = t {
                *id = next;
                next = next.wrapping_add(1);
                if let Some(b) = hz_java::euc_kr(text) {
                    asks.push((*id, b));
                }
            }
        };
        take(&mut self.title);
        for r in &mut self.rows {
            take(&mut r.text);
            if let Some(v) = r.value.as_mut() {
                take(v);
            }
        }
        (asks, next)
    }

    /// Every id this window's lines were lettered under.
    pub fn dyn_ids(&self) -> Vec<u16> {
        let id = |t: &Text| match t {
            Text::Dyn { id, .. } => Some(*id),
            _ => None,
        };
        std::iter::once(&self.title).chain(self.rows.iter().map(|r| &r.text)).chain(self.rows.iter().filter_map(|r| r.value.as_ref())).filter_map(id).collect()
    }
}

/// The keys a host window answers (the keyboard, the pad): the cursor up and down, OK acts, the
/// back keys close. Everything else is swallowed while it is up (`None` = not a key of the window's,
/// but the window still has the keyboard).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostKey {
    Up,
    Down,
    Ok,
    Back,
}

pub fn host_key(code: KeyCode) -> Option<HostKey> {
    Some(match code {
        KeyCode::UP | KeyCode::NUM2 => HostKey::Up,
        KeyCode::DOWN | KeyCode::NUM8 => HostKey::Down,
        KeyCode::OK | KeyCode::NUM5 => HostKey::Ok,
        KeyCode::RIGHT_SOFT_KEY | KeyCode::CLEAR => HostKey::Back,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win() -> HostWin {
        HostWin::new(
            Text::Line("DIAG"),
            vec![
                Row::text(Text::dynamic("영웅서기 Zero PC - 버전 48")),
                Row::pair(Text::Line("WINDOW"), Text::ascii("1280x720")),
                Row::button(Text::Line("SAVES"), HostAct::OpenSaves),
                Row::text(Text::ascii("C:\\Users\\x")),
                Row::button(Text::Line("OPEN FOLDER"), HostAct::OpenFolder(PathBuf::from("C:/x"))),
            ],
        )
    }

    #[test]
    fn the_cursor_starts_on_the_first_plate_and_steps_between_plates_only() {
        let mut w = win();
        assert_eq!(w.cursor, Some(2));
        assert!(w.step(true));
        assert_eq!(w.cursor, Some(4), "over the text row");
        assert!(!w.step(true), "no plate below");
        assert!(w.step(false));
        assert_eq!(w.cursor, Some(2));
        assert!(!w.step(false));
        assert_eq!(w.act(), Some(HostAct::OpenSaves));
        let none = HostWin::new(Text::Line("X"), vec![Row::text(Text::ascii("a"))]);
        assert_eq!(none.cursor, None);
        assert_eq!(none.act(), None);
    }

    #[test]
    fn numbering_gives_each_dynamic_line_an_id_and_the_games_bytes() {
        let mut w = win();
        let (asks, next) = w.number(4096);
        assert_eq!(next, 4097, "one dynamic line");
        assert_eq!(asks.len(), 1);
        assert_eq!(asks[0].0, 4096);
        assert_eq!(asks[0].1, hz_java::euc_kr("영웅서기 Zero PC - 버전 48").unwrap());
        assert_eq!(w.dyn_ids(), vec![4096]);
        assert_eq!(w.rows[0].text, Text::Dyn { id: 4096, text: "영웅서기 Zero PC - 버전 48".into() });
    }

    #[test]
    fn what_the_games_font_cannot_draw_is_replaced_before_it_is_asked() {
        assert_eq!(fontable("a_b,c·d→e\\f;g"), "a b c-d>e f g");
        let t = Text::dynamic("슬롯 1: 541 B, 복원 지점 3개");
        assert_eq!(t, Text::Dyn { id: 0, text: "슬롯 1: 541 B  복원 지점 3개".into() });
        // The fallback is what the 5×7 font has: capitals.
        assert_eq!(Text::ascii("C:\\Users\\my_x").fallback(), "C:/USERS/MY-X");
        assert_eq!(Text::Line("SAVES").fallback(), "SAVES");
    }

    #[test]
    fn the_window_keys() {
        assert_eq!(host_key(KeyCode::UP), Some(HostKey::Up));
        assert_eq!(host_key(KeyCode::NUM8), Some(HostKey::Down));
        assert_eq!(host_key(KeyCode::OK), Some(HostKey::Ok));
        assert_eq!(host_key(KeyCode::RIGHT_SOFT_KEY), Some(HostKey::Back));
        assert_eq!(host_key(KeyCode::LEFT), None);
    }
}
