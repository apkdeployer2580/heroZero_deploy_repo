//! The host's own lines in Korean, in the game's own font (B1, 2026-10-26 — the user's choice of
//! hero3's way: *"글꼴: hero3 식으로"*). The host keeps naming a line by its ASCII text, as every
//! caller already does (`Hud::notice("SAVE - NOT NOW")`, `SettingsAction::label`); this table is
//! what that line says in Korean. The worker asks the game's font for every Korean line once its
//! screen is up (the title on — `hz_java::GuestState::ask_texts`, a few a frame), and the HUD draws
//! the picture it gets back (`render::draw_line`); until then, or for a line with none, the ASCII
//! text in the host's 5×7 font — so a game that stops before its title still says why.
//!
//! What the game's font draws (probe `--labels`, 2026-10-26): the 2,350 Hangul syllables of KS X
//! 1001 (`/ftd/kor` + `/ftd/table`), A–Z a–z 0–9 and the space (`/ftd/eng7`), and of the other
//! ASCII ``! " # % ( ) * + - . / : < = > ? @ [ ] ~ ' ` `` . Nine are its control codes: `$ ^ { | } \`
//! change the colour, `&` and `;` end the line (`;` is the game's forced line break), `_` is a wide
//! gap — so no line here uses them, nor anything outside KS X 1001's syllables (no `·`, `›`, `…`).

/// `(the host's ASCII line, what it says in Korean)`. The ASCII is the key every caller uses.
pub const LINES: &[(&str, &str)] = &[
    // ---- notices (the toast line)
    ("SAVE - NOT NOW", "지금은 저장할 수 없습니다"),
    // The SAVE tab's two other answers (2026-10-29: it saves at once) — the game's own words for
    // them (`t/gm.tdf` [16] 「저장되었습니다.」, [61] 「저장할 수 없는 지역입니다.」).
    ("SAVED", "저장되었습니다"),
    ("PICK A QUICK SLOT", "등록할 퀵슬롯을 고르세요"),
    ("SAVE - NOT HERE", "저장할 수 없는 지역입니다"),
    // HZ-065: a quest reward into a full bag waits for the next save, which puts it into the 계정창고
    ("BAG FULL - REWARD GOES TO STORAGE ON SAVE", "가방이 가득 차 보상은 저장할 때 계정창고로 들어갑니다"),
    ("REWARD PUT IN STORAGE", "보상을 계정창고에 넣었습니다"),
    ("REWARD NOT STORED - SAVE AGAIN", "보상을 계정창고에 저장하지 못했습니다 - 다시 저장하세요"),
    ("STORAGE FULL - REWARD WAITS - CLEAR IT AND SAVE", "계정창고가 가득 차 보상을 넣지 못했습니다 - 창고를 비우고 저장하세요"),
    ("REWARD IN STORAGE - NOT VERIFIED", "보상을 계정창고에 넣었지만 기록을 확인하지 못했습니다"),
    ("SAVE FAILED - STORAGE ERROR", "저장하지 못했습니다 - 저장소 오류"),
    ("SAVE READ ERROR - THE FILE IS KEPT", "세이브를 읽지 못했습니다 - 파일은 그대로 둡니다"),
    ("GAMEPAD - BADGES SHOW ITS BUTTONS", "게임패드 - 화면에 패드 버튼을 표시합니다"),
    ("LAYOUT RESTORED", "배치를 되돌렸습니다"),
    ("LAYOUT SAVED", "배치를 저장했습니다"),
    ("HEAT SAVER ON - NEXT START", "발열 절감: 켬 (재시작 후 적용)"),
    ("HEAT SAVER OFF - NEXT START", "발열 절감: 끔 (재시작 후 적용)"),
    ("THE GAME STOPPED - SAVES ARE SAFE - RESTART THE APP", "게임이 멈췄습니다 - 세이브는 안전합니다 - 앱을 다시 실행하세요"),
    ("SAVE RESTORED - THE ONE IT REPLACED IS IN THE VAULT", "세이브를 복원했습니다 - 바뀐 세이브는 금고에 있습니다"),
    ("RESTORE INCOMPLETE - GEAR > DIAG", "복원이 덜 끝났습니다 - 설정 > 진단 정보"),
    ("SAVE 1 IS GONE - THE VAULT HAS IT - GEAR > SAVES", "세이브 1이 없습니다 - 금고에 있습니다 - 설정 > 진단 정보 > 세이브 관리"),
    ("SAVE 2 IS GONE - THE VAULT HAS IT - GEAR > SAVES", "세이브 2가 없습니다 - 금고에 있습니다 - 설정 > 진단 정보 > 세이브 관리"),
    ("SAVE 3 IS GONE - THE VAULT HAS IT - GEAR > SAVES", "세이브 3이 없습니다 - 금고에 있습니다 - 설정 > 진단 정보 > 세이브 관리"),
    ("SAVE 4 IS GONE - THE VAULT HAS IT - GEAR > SAVES", "세이브 4가 없습니다 - 금고에 있습니다 - 설정 > 진단 정보 > 세이브 관리"),
    ("SAVE 5 IS GONE - THE VAULT HAS IT - GEAR > SAVES", "세이브 5가 없습니다 - 금고에 있습니다 - 설정 > 진단 정보 > 세이브 관리"),
    ("SAVE 6 IS GONE - THE VAULT HAS IT - GEAR > SAVES", "세이브 6이 없습니다 - 금고에 있습니다 - 설정 > 진단 정보 > 세이브 관리"),
    ("SAVE 7 IS GONE - THE VAULT HAS IT - GEAR > SAVES", "세이브 7이 없습니다 - 금고에 있습니다 - 설정 > 진단 정보 > 세이브 관리"),
    ("SAVE 8 IS GONE - THE VAULT HAS IT - GEAR > SAVES", "세이브 8이 없습니다 - 금고에 있습니다 - 설정 > 진단 정보 > 세이브 관리"),
    ("SAVE 9 IS GONE - THE VAULT HAS IT - GEAR > SAVES", "세이브 9가 없습니다 - 금고에 있습니다 - 설정 > 진단 정보 > 세이브 관리"),
    ("SAVE 10 IS GONE - THE VAULT HAS IT - GEAR > SAVES", "세이브 10이 없습니다 - 금고에 있습니다 - 설정 > 진단 정보 > 세이브 관리"),
    // HZ-061: a boss's story counting down (`STORY_WAIT`) — hero2's raid 「다음 보스로 이동 중...5」
    ("STORY IN 5", "스토리 진행까지...5"),
    ("STORY IN 4", "스토리 진행까지...4"),
    ("STORY IN 3", "스토리 진행까지...3"),
    ("STORY IN 2", "스토리 진행까지...2"),
    ("STORY IN 1", "스토리 진행까지...1"),
    // ---- under the 창고 question (`hud::ANSWER_HINT`): the pad screen's words for OK and CLR
    ("OK - YES   CLR - NO", "확인: 예    취소: 아니오"),
    // ---- under the status block in 가디언 모드 (HZ-030, `render::draw_status`)
    (crate::render::GUARDIAN_MODE, "가디언 모드"),
    // ---- the settings window (B2): its title, each row's name and the words of its state —
    // hero2's (`overlay.rs` `draw_settings`)
    ("SETTINGS", "설정"),
    ("CLOSE", "닫기"),
    ("LAYOUT", "화면 배치"),
    ("EDIT", "편집"),
    ("CUSTOM", "사용자 배치"),
    ("DIAG", "진단 정보"),
    ("SAVES", "세이브 관리"),
    ("HEAT SAVER", "발열 절감"),
    ("ON", "켬"),
    ("OFF", "끔"),
    ("SFX LEVEL", "효과음 크기"),
    ("DEFAULT", "기본"),
    ("-6 DB", "-6 dB"),
    ("-3 DB", "-3 dB"),
    ("+3 DB", "+3 dB"),
    ("+6 DB", "+6 dB"),
    ("STATS", "상태 표시"),
    ("PAD BADGES", "패드 표시"),
    ("AUTO", "자동"),
    ("ALWAYS", "항상 켬"),
    ("PAD REMAP", "패드 버튼 설정"),
    // ---- the layout editor (A9: hero2's row and hint) and the PAD REMAP panel
    ("PAD", "패드"),
    ("FLIP", "반전"),
    ("HIDE", "숨기기"),
    ("SHOW", "보이기"),
    ("RESET", "기본값"),
    ("CANCEL", "취소"),
    ("SAVE", "저장"),
    ("TAP - DRAG - PINCH", "누르기: 선택 - 끌기: 이동 - 두 손가락: 크기"),
    ("KEEP ONE QUICK SLOT", "퀵슬롯 하나는 남겨야 합니다"),
    ("SELECT A QUICK SLOT TO HIDE", "숨길 퀵슬롯을 먼저 고르세요"),
    ("QUICK SLOTS ALL SHOWN", "숨긴 퀵슬롯을 모두 보입니다"),
    ("NO QUICK SLOT IS HIDDEN", "숨긴 퀵슬롯이 없습니다"),
    // ---- the pad screen (B6, hero2's 「패드 버튼 설정」 — its words): the functions
    // (`gamepad::REMAP_ROWS`, `gamepad::key_label`), the buttons by position (`gamepad::button_name`),
    // and the window's lines, put together side by side (`touch::Line`)
    ("OK - ATTACK", "확인 / 공격"),
    ("QUICK PAGE", "퀵슬롯 쪽 넘김"),
    ("BACK - MENU", "뒤로 / 메뉴"),
    ("QUICK SLOT 1", "퀵슬롯 1"),
    ("QUICK SLOT 3", "퀵슬롯 3"),
    ("QUICK SLOT 7 - PAGE BACK", "퀵슬롯 7 / 이전 쪽"),
    ("QUICK SLOT 9 - PAGE ON", "퀵슬롯 9 / 다음 쪽"),
    ("QUICK SLOT 0", "퀵슬롯 0"),
    ("QUICK SLOT #", "퀵슬롯 #"),
    ("QUICK SLOT *", "퀵슬롯 *"),
    ("SAVE TAB", "저장 (SAVE 탭)"),
    ("MAP TAB", "월드맵 (MAP 탭)"),
    ("KEY 2", "숫자 2"),
    ("KEY 4", "숫자 4"),
    ("KEY 5", "숫자 5"),
    ("KEY 6", "숫자 6"),
    ("KEY 8", "숫자 8"),
    ("ANOTHER KEY", "다른 키"),
    ("NO BUTTON", "없음"),
    ("(HIDDEN)", "(숨김)"),
    ("=", "="),
    ("FACE TOP (XBOX Y)", "면버튼 위 (XBOX Y)"),
    ("FACE BOTTOM (XBOX A)", "면버튼 아래 (XBOX A)"),
    ("FACE LEFT (XBOX X)", "면버튼 왼쪽 (XBOX X)"),
    ("FACE RIGHT (XBOX B)", "면버튼 오른쪽 (XBOX B)"),
    ("LEFT BUMPER (L1)", "왼쪽 어깨 (L1/LB)"),
    ("LEFT TRIGGER (L2)", "왼쪽 트리거 (L2/LT)"),
    ("RIGHT BUMPER (R1)", "오른쪽 어깨 (R1/RB)"),
    ("RIGHT TRIGGER (R2)", "오른쪽 트리거 (R2/RT)"),
    ("MIDDLE LEFT (SELECT)", "가운데 왼쪽 (SELECT)"),
    ("MIDDLE RIGHT (START)", "가운데 오른쪽 (START)"),
    ("LEFT STICK PRESS (L3)", "왼스틱 누르기 (L3)"),
    ("RIGHT STICK PRESS (R3)", "오른스틱 누르기 (R3)"),
    ("ANOTHER BUTTON", "다른 버튼"),
    ("TAP A CONTROL - THEN PRESS A PAD BUTTON", "화면의 버튼을 누르고 패드 버튼을 누르세요"),
    ("A PAD BUTTON ALONE SHOWS WHAT IT DOES", "패드 버튼만 누르면 하는 일을 보여줍니다"),
    ("PRESS THE PAD BUTTON FOR IT", "바꿀 패드 버튼을 누르세요"),
    ("(1S)", "(1초)"),
    ("(2S)", "(2초)"),
    ("(3S)", "(3초)"),
    ("(4S)", "(4초)"),
    ("(5S)", "(5초)"),
    ("TIME IS UP", "시간이 지났습니다"),
    ("TAP THE CONTROL AGAIN", "다시 화면의 버튼을 누르세요"),
    ("DIRECTIONS CANNOT CHANGE", "방향키는 바꿀 수 없습니다"),
    ("THEY WALK AND MOVE THE CURSOR", "걷기와 메뉴 이동에 쓰입니다"),
    ("TO CHANGE IT TAP A CONTROL FIRST", "바꾸려면 화면의 버튼을 먼저 누르세요"),
    ("NOTHING CHANGED", "바뀐 것이 없습니다"),
    ("WOULD HAVE NO BUTTON", "버튼이 없어지므로 바꿀 수 없습니다"),
    ("TRY ANOTHER PAD BUTTON", "다른 패드 버튼으로 다시 해 보세요"),
    ("SAVE TO APPLY", "저장을 누르면 적용됩니다"),
    ("BACK TO THE BUILT-IN LAYOUT", "기본값으로 되돌렸습니다"),
    // ---- the PC (hz_desktop, 2026-10-29): the settings row, and the host windows that stand in
    // for MainActivity's dialogs (`crate::hostwin`, `crate::saves_ui`)
    ("FULLSCREEN", "전체 화면"),
    ("APP", "앱"),
    ("WINDOW", "창 크기 / 게임 그림"),
    ("VAULT FILES", "금고 파일"),
    ("PATCHES", "패치"),
    ("DEFAULT SET", "기본 묶음"),
    ("FROM FILE", "파일에서 (hz patches)"),
    ("EXPORT FOLDER", "내보내기 폴더"),
    ("OPEN FOLDER", "폴더 열기"),
    ("EXPORT TO FOLDER", "세이브 폴더로 내보내기"),
    ("SEND TO ANOTHER DEVICE", "다른 기기로 보내기 (zip)"),
    ("RECEIVE FROM ANOTHER DEVICE", "다른 기기에서 받기 (zip)"),
    ("RESTORE FROM BACKUP", "백업에서 복원"),
    ("NO RESTORE POINTS", "복원 지점이 없습니다"),
    ("THE CURRENT SAVE GOES TO THE VAULT", "지금 세이브는 금고에 보관됩니다"),
    ("SCHEDULE", "예약"),
    ("RESTORE SCHEDULED", "복원 예약됨"),
    ("APPLIES AT THE NEXT START", "다음 실행 때 적용됩니다"),
    ("QUIT NOW", "지금 닫기"),
    ("LATER", "나중에"),
    ("EXPORT FAILED - STORAGE ERROR", "내보내지 못했습니다 - 저장소 오류"),
    ("COPY THIS FILE TO THE OTHER DEVICE", "이 파일을 다른 기기로 옮기세요"),
    ("ON A PHONE: SAVES > RECEIVE", "폰에서는: 세이브 관리 > 다른 폰에서 받기"),
    ("PUT THE ZIP IN THIS FOLDER AND PICK IT", "zip 파일을 이 폴더에 넣고 고르세요"),
    ("NO ZIP FILE IN THE FOLDER", "폴더에 zip 파일이 없습니다"),
    ("MADE AT", "만든 때"),
    ("APP VERSION", "앱 버전"),
    ("SLOTS IN THE FILE", "가져올 슬롯"),
    ("WITH THE ACCOUNT STORAGE", "계정창고 포함"),
    ("SAVES ON THIS PC GO TO THE VAULT", "이 PC 의 세이브는 금고로"),
    ("RECEIVE", "받기"),
    ("IMPORT SCHEDULED", "가져오기 예약됨"),
    ("KEYS: ARROWS MOVE - X ATTACK - SPACE ENTER TALK - ESC CLR - F5 SAVE - M MAP - F11 FULL", "키: 방향키 이동 - X 공격 - 스페이스 엔터 대화 - ESC 취소 - F5 저장 - M 지도 - F11 전체화면"),
];

/// The boot check's line for slot `n` (index `n − 1`; HZ-020's ten): a slot the list would show
/// empty while the vault holds it — 세이브 관리 (진단 정보, B3) puts it back.
pub const SAVE_GONE: [&str; 10] = [
    "SAVE 1 IS GONE - THE VAULT HAS IT - GEAR > SAVES",
    "SAVE 2 IS GONE - THE VAULT HAS IT - GEAR > SAVES",
    "SAVE 3 IS GONE - THE VAULT HAS IT - GEAR > SAVES",
    "SAVE 4 IS GONE - THE VAULT HAS IT - GEAR > SAVES",
    "SAVE 5 IS GONE - THE VAULT HAS IT - GEAR > SAVES",
    "SAVE 6 IS GONE - THE VAULT HAS IT - GEAR > SAVES",
    "SAVE 7 IS GONE - THE VAULT HAS IT - GEAR > SAVES",
    "SAVE 8 IS GONE - THE VAULT HAS IT - GEAR > SAVES",
    "SAVE 9 IS GONE - THE VAULT HAS IT - GEAR > SAVES",
    "SAVE 10 IS GONE - THE VAULT HAS IT - GEAR > SAVES",
];

/// HZ-061: the countdown's line with `n` seconds left is `STORY_WAIT[n − 1]` (`hz_java::story`).
pub const STORY_WAIT: [&str; hz_java::story::WAIT_SECONDS as usize] = ["STORY IN 1", "STORY IN 2", "STORY IN 3", "STORY IN 4", "STORY IN 5"];

/// The countdown's line for `seconds` left (`hz_java::GuestState::story_seconds`), `None` for none.
pub fn story_wait(seconds: u8) -> Option<&'static str> {
    STORY_WAIT.get(usize::from(seconds).checked_sub(1)?).copied()
}

/// The table's number for `en` (what the lettered picture is filed under).
pub fn index(en: &str) -> Option<u16> {
    LINES.iter().position(|(e, _)| *e == en).map(|i| i as u16)
}

/// What `en` says in Korean.
pub fn korean(en: &str) -> Option<&'static str> {
    LINES.iter().find(|(e, _)| *e == en).map(|(_, k)| *k)
}

/// Every Korean line as the game's font takes it (EUC-KR), with its number — the worker's boot ask.
pub fn lettering() -> Vec<(u16, Vec<u8>)> {
    LINES.iter().enumerate().filter_map(|(i, (_, ko))| hz_java::euc_kr(ko).map(|b| (i as u16, b))).collect()
}

/// The ASCII the game's font treats as control codes ([module docs](self)).
pub const FONT_CONTROLS: &[char] = &['$', '^', '{', '|', '}', '\\', '&', ';', '_'];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every Korean line is one the game's font can draw: KS X 1001 syllables and plain ASCII, none
    /// of the font's control codes — and no comma, so the probe's `--labels` can letter the table.
    #[test]
    fn every_line_is_in_the_games_font() {
        for (en, ko) in LINES {
            assert!(hz_java::euc_kr(ko).is_some(), "{en}: {ko:?} has a character outside KS X 1001's syllables");
            assert!(!ko.contains(FONT_CONTROLS), "{en}: {ko:?} uses a control code of the game's font");
            assert!(!ko.contains(','), "{en}: {ko:?}");
            assert!(crate::font::drawable(en), "{en}: the fallback must be the host font's");
        }
        assert_eq!(lettering().len(), LINES.len());
    }

    #[test]
    fn each_line_is_listed_once() {
        for (i, (en, _)) in LINES.iter().enumerate() {
            assert_eq!(index(en), Some(i as u16), "{en} is listed twice");
        }
        assert!(LINES.len() < usize::from(u16::MAX));
    }

    /// The host's lines that come from its own types: every one has its Korean.
    #[test]
    fn every_label_has_its_korean() {
        use crate::{hud::SettingsAction as A, padhud::PadHud};
        // Every row's name and every word its state can say, through the HUD in each state.
        let mut h = crate::touch::Hud::new(crate::hud::Screen::for_window(2220, 1080), crate::placement::Table::default());
        let mut said = std::collections::HashSet::new();
        for (heat, stats, custom) in [(false, true, false), (true, false, true)] {
            h.set_heat_saver(heat);
            h.set_stats_on(stats);
            let mut ov = crate::gamepad::Overrides::default();
            if custom {
                ov.set(crate::gamepad::KEYCODE_BUTTON_R2, None);
            }
            h.set_overrides(ov);
            for pad in [PadHud::Auto, PadHud::On, PadHud::Off] {
                h.set_pad_hud(pad);
                for db in crate::audio_mix::SFX_STEPS_DB {
                    h.set_sfx_db(db);
                    for a in [A::LayoutEdit, A::Diag, A::HeatSaver, A::SfxLevel, A::Stats, A::PadHud, A::PadRemap, A::Close] {
                        for can in [false, true] {
                            let (name, state, _) = h.settings_words(a, can);
                            said.insert(name);
                            said.insert(state);
                        }
                    }
                }
            }
        }
        said.remove("");
        for en in &said {
            assert!(korean(en).is_some(), "settings word {en:?}");
        }
        assert!(said.len() >= 20, "{said:?}");
        for e in crate::hud::EDIT_ACTIONS {
            assert!(korean(e.label()).is_some(), "{e:?}");
        }
        assert!(korean(crate::render::EDITOR_HINT).is_some());
        for t in SAVE_GONE.into_iter().chain(STORY_WAIT).chain([crate::database::SAVE_FAILED, crate::database::SAVE_UNREADABLE]) {
            assert!(korean(t).is_some(), "{t}");
        }
    }

    /// Every notice the code raises with a literal has its Korean — read off the sources, so a new
    /// `notice("…")` without a line here fails the build's tests, not a player's screen.
    #[test]
    fn every_notice_in_the_sources_has_its_korean() {
        let sources = [include_str!("touch.rs"), include_str!("app.rs"), include_str!("emulator.rs")];
        let mut seen = 0;
        for src in sources {
            let body = src.split("#[cfg(test)]").next().unwrap_or(src);
            for marker in ["notice(\"", "notice_through_title(\"", "notice_stuck(\"", "Notice(\""] {
                for (at, _) in body.match_indices(marker) {
                    let rest = &body[at + marker.len()..];
                    let text = &rest[..rest.find('"').unwrap()];
                    assert!(korean(text).is_some(), "notice {text:?} has no Korean line");
                    seen += 1;
                }
            }
            // `notice(if … { "A" } else { "B" })` and the restore / export / boot-check results:
            // every all-capital literal of a line with "notice" or of a `=> "…"` arm.
            for line in body.lines().filter(|l| l.contains("notice(if") || l.trim_start().starts_with("1 =>") || l.trim_start().starts_with("2 =>") || l.trim_start().starts_with("_ =>")) {
                for lit in line.split('"').skip(1).step_by(2) {
                    if lit.len() > 6 && lit.contains(' ') && !lit.contains('{') && lit.chars().all(|c| !c.is_ascii_lowercase()) {
                        assert!(korean(lit).is_some(), "notice {lit:?} has no Korean line");
                        seen += 1;
                    }
                }
            }
        }
        assert!(seen > 10, "the scan found the notices ({seen})");
    }
}
