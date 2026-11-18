//! The host's rows of the 환경설정 page (HZ-007) and the game strings it renames, as the game draws them: EUC-KR bytes through its own bitmap font
//! (`ah.a` -> `ftd/kor` + `ftd/table`). `ftd/table` is 4,700 B = 2,350 x 2, one entry per KS X
//! 1001 Hangul syllable, so any syllable in that set draws — and only those. The bytes below were
//! produced with Python's strict `euc_kr` codec (KS X 1001 only, 2026-10-22); 켜기 / 끄기 are
//! byte-identical to the game's own `t/com.tdf` [37] / [38].

use crate::options::{ControlMode, Row};

/// 다음 실행부터 적용 — under the rows while an orientation change is pending.
pub const NEXT_LAUNCH: &[u8] = &[
    0xb4, 0xd9, 0xc0, 0xbd, 0x20, 0xbd, 0xc7, 0xc7, 0xe0, 0xba, 0xce, 0xc5, 0xcd, 0x20, 0xc0, 0xfb, 0xbf, 0xeb,
];

const ORIENTATION: &[u8] = &[0xc8, 0xad, 0xb8, 0xe9, 0x20, 0xb9, 0xe6, 0xc7, 0xe2]; // 화면 방향
const AUTO_LOOT: &[u8] = &[0xbf, 0xc0, 0xc5, 0xe4, 0xb7, 0xe7, 0xc6, 0xc3]; // 오토루팅
const EXP_RATE: &[u8] = &[0xb0, 0xe6, 0xc7, 0xe8, 0xc4, 0xa1, 0x20, 0xb9, 0xe8, 0xc0, 0xb2]; // 경험치 배율
const DROP_RATE: &[u8] = &[0xb5, 0xe5, 0xb6, 0xf8, 0xb7, 0xfc]; // 드랍률
const CONTROL: &[u8] = &[0xc1, 0xb6, 0xc0, 0xdb, 0x20, 0xb9, 0xe6, 0xbd, 0xc4]; // 조작 방식
const DIFFICULTY: &[u8] = &[0xb3, 0xad, 0xc0, 0xcc, 0xb5, 0xb5]; // 난이도
const RESPAWN: &[u8] = &[0xb8, 0xf3, 0xbd, 0xba, 0xc5, 0xcd, 0x20, 0xb8, 0xae, 0xc1, 0xa8]; // 몬스터 리젠

const LANDSCAPE: &[u8] = &[0xb0, 0xa1, 0xb7, 0xce]; // 가로
const PORTRAIT: &[u8] = &[0xbc, 0xbc, 0xb7, 0xce]; // 세로
const ON: &[u8] = &[0xc4, 0xd1, 0xb1, 0xe2]; // 켜기
const OFF: &[u8] = &[0xb2, 0xf4, 0xb1, 0xe2]; // 끄기
const TIMES: &[u8] = &[0xb9, 0xe8]; // 배
const FREE: &[u8] = &[0xc0, 0xda, 0xc0, 0xaf]; // 자유
const DPAD: &[u8] = &[0xbd, 0xca, 0xc0, 0xda, 0xc5, 0xb0]; // 십자키
const FIXED: &[u8] = &[0xb0, 0xed, 0xc1, 0xa4]; // 고정
/// 쉬움 / 보통 / 어려움 — hero2's EASY / NORMAL / HARD (the original game is the first).
const LEVELS: [&[u8]; 3] = [&[0xbd, 0xac, 0xbf, 0xf2], &[0xba, 0xb8, 0xc5, 0xeb], &[0xbe, 0xee, 0xb7, 0xc1, 0xbf, 0xf2]];

pub fn label(row: Row) -> &'static [u8] {
    match row {
        Row::Orientation => ORIENTATION,
        Row::AutoLoot => AUTO_LOOT,
        Row::ExpRate => EXP_RATE,
        Row::DropRate => DROP_RATE,
        Row::Difficulty => DIFFICULTY,
        Row::Respawn => RESPAWN,
        Row::Control => CONTROL,
    }
}

/// 쉬움 / 보통 / 어려움 — the 난이도 row, by [`crate::options::GameOptions::difficulty_level`].
pub fn difficulty(level: usize) -> &'static [u8] {
    LEVELS[level.min(LEVELS.len() - 1)]
}

/// 자유 / 십자키 / 고정 — the 조작 방식 row.
pub fn control(mode: ControlMode) -> &'static [u8] {
    match mode {
        ControlMode::Free => FREE,
        ControlMode::Dpad => DPAD,
        ControlMode::Fixed => FIXED,
    }
}

/// HZ-011: the game's own strings this port renames, byte for byte (original → replacement). The
/// slot-shared store is the QQ build's 「네트워크창고」 in name only — it lives in RMS `HOL.storage`
/// — so it is called what it is, the 계정창고. Matched on the whole entry, so a longer entry that
/// merely contains one of these is left alone.
const RENAMES: &[(&[u8], &[u8])] = &[
    // 네트워크창고 → 계정창고 (nui[0])
    (
        &[0xb3, 0xd7, 0xc6, 0xae, 0xbf, 0xf6, 0xc5, 0xa9, 0xc3, 0xa2, 0xb0, 0xed],
        &[0xb0, 0xe8, 0xc1, 0xa4, 0xc3, 0xa2, 0xb0, 0xed],
    ),
    // 네트워크로 올리기 → 계정창고에 넣기 (nui[20])
    (
        &[0xb3, 0xd7, 0xc6, 0xae, 0xbf, 0xf6, 0xc5, 0xa9, 0xb7, 0xce, 0x20, 0xbf, 0xc3, 0xb8, 0xae, 0xb1, 0xe2],
        &[0xb0, 0xe8, 0xc1, 0xa4, 0xc3, 0xa2, 0xb0, 0xed, 0xbf, 0xa1, 0x20, 0xb3, 0xd6, 0xb1, 0xe2],
    ),
    // 싱글로 내리기 → 계정창고에서 꺼내기 (nui[21])
    (
        &[0xbd, 0xcc, 0xb1, 0xdb, 0xb7, 0xce, 0x20, 0xb3, 0xbb, 0xb8, 0xae, 0xb1, 0xe2],
        &[0xb0, 0xe8, 0xc1, 0xa4, 0xc3, 0xa2, 0xb0, 0xed, 0xbf, 0xa1, 0xbc, 0xad, 0x20, 0xb2, 0xa8, 0xb3, 0xbb, 0xb1, 0xe2],
    ),
    // 네트워크보유액 → 계정창고 보유액 (nui[23])
    (
        &[0xb3, 0xd7, 0xc6, 0xae, 0xbf, 0xf6, 0xc5, 0xa9, 0xba, 0xb8, 0xc0, 0xaf, 0xbe, 0xd7],
        &[0xb0, 0xe8, 0xc1, 0xa4, 0xc3, 0xa2, 0xb0, 0xed, 0x20, 0xba, 0xb8, 0xc0, 0xaf, 0xbe, 0xd7],
    ),
    // 싱글보유액 → 캐릭터 보유액 (nui[24])
    (
        &[0xbd, 0xcc, 0xb1, 0xdb, 0xba, 0xb8, 0xc0, 0xaf, 0xbe, 0xd7],
        &[0xc4, 0xb3, 0xb8, 0xaf, 0xc5, 0xcd, 0x20, 0xba, 0xb8, 0xc0, 0xaf, 0xbe, 0xd7],
    ),
    // 공용창고 → 계정창고 (m[3], the title carousel)
    (
        &[0xb0, 0xf8, 0xbf, 0xeb, 0xc3, 0xa2, 0xb0, 0xed],
        &[0xb0, 0xe8, 0xc1, 0xa4, 0xc3, 0xa2, 0xb0, 0xed],
    ),
    // 올리기 → 넣기 (nui[48], the money window's confirm button)
    (&[0xbf, 0xc3, 0xb8, 0xae, 0xb1, 0xe2], &[0xb3, 0xd6, 0xb1, 0xe2]),
    // 내리기 → 꺼내기 (nui[49])
    (&[0xb3, 0xbb, 0xb8, 0xae, 0xb1, 0xe2], &[0xb2, 0xa8, 0xb3, 0xbb, 0xb1, 0xe2]),
    // 네트워크 → 계정창고 (com[9]: the chooser `bg`'s title; its other user `ay` is unreachable)
    (&[0xb3, 0xd7, 0xc6, 0xae, 0xbf, 0xf6, 0xc5, 0xa9], &[0xb0, 0xe8, 0xc1, 0xa4, 0xc3, 0xa2, 0xb0, 0xed]),
    // 싱글창고 → 캐릭터창고 (nui[47], the item window's own side)
    (
        &[0xbd, 0xcc, 0xb1, 0xdb, 0xc3, 0xa2, 0xb0, 0xed],
        &[0xc4, 0xb3, 0xb8, 0xaf, 0xc5, 0xcd, 0xc3, 0xa2, 0xb0, 0xed],
    ),
];

/// 계정창고 — the menu's row 8 (HZ-012).
pub const ACCOUNT_STORAGE: &[u8] = &[0xb0, 0xe8, 0xc1, 0xa4, 0xc3, 0xa2, 0xb0, 0xed];

/// HZ-011: the one-time help before the storage — `rpg/d.J` (268 B of EUC-KR, set byte by byte in
/// `rpg/d.<clinit>`, so it is in no table) — and its 계정창고 wording; 개인창고 is the item window's
/// own side, which nui[47] already calls 캐릭터창고.
const STORAGE_HELP: (&str, &str) = (
    "공용창고에는 게임에서 얻은 돈, 장비, 아이템을 보관하고 옮길 수 있습니다. 한 슬롯을 골라 그 슬롯에서 얻은 \
     돈과 장비, 아이템을 공용창고에 보관할 수 있습니다. 다른 슬롯에서는 공용창고에서 이 돈과 장비를 꺼낼 수 \
     있습니다. 보관 시 1,3 키로 개인창고와 공용창고를 전환합니다.",
    "계정창고에는 게임에서 얻은 돈, 장비, 아이템을 보관하고 옮길 수 있습니다. 한 슬롯을 골라 그 슬롯에서 얻은 \
     돈과 장비, 아이템을 계정창고에 보관할 수 있습니다. 다른 슬롯에서는 계정창고에서 이 돈과 장비를 꺼낼 수 \
     있습니다. 보관 시 1,3 키로 캐릭터창고와 계정창고를 전환합니다.",
);

/// The replacement for a text-table entry, if this port renames it.
pub fn rename(original: &[u8]) -> Option<&'static [u8]> {
    RENAMES.iter().find(|(from, _)| *from == original).map(|(_, to)| *to)
}

/// HZ-058: the popups' registration rows (`t/gm.tdf` — the skill window's 「1번 · 3번 등록」, the spirit
/// window's 「7번 · 9번」, the inventory's 「0번 등록 · #에 등록」) as the plates' two: the first row puts the
/// skill or potion on a plate, the second takes it off (`crate::quickpage`).
const QUICK_ROWS: [(&str, &str); 6] = [
    ("\r\n1번 등록", "\r\n퀵슬롯등록"),
    ("\r\n3번 등록", "\r\n퀵슬롯해제"),
    ("\r\n7번 등록", "\r\n퀵슬롯등록"),
    ("\r\n9번 등록", "\r\n퀵슬롯해제"),
    ("0번 등록", "퀵슬롯등록"),
    ("#에 등록", "퀵슬롯해제"),
];

/// HZ-058: the plates' word for a popup's registration row, if `original` is one.
pub fn quick_row(original: &[u8]) -> Option<Vec<u8>> {
    QUICK_ROWS.iter().find(|(from, _)| euc_kr(from).is_some_and(|f| f == original)).and_then(|(_, to)| euc_kr(to))
}

/// HZ-068: 「고양」's line (`t/s5.tdf` [15]) in the stat's one name.
const WILL_LINE: (&str, &str) = ("정신력이 증가한다.", "의지가 증가한다.");
/// Its length in EUC-KR: most strings the game draws go by on this alone.
const WILL_LINE_BYTES: usize = 18;

/// HZ-068: 「고양」's line, if `original` is the game's.
pub fn will_line(original: &[u8]) -> Option<Vec<u8>> {
    (original.len() == WILL_LINE_BYTES && euc_kr(WILL_LINE.0)? == original).then(|| euc_kr(WILL_LINE.1)).flatten()
}

/// HZ-011: the storage help in 계정창고 wording, when `original` is exactly the game's paragraph.
pub fn storage_help(original: &[u8]) -> Option<Vec<u8>> {
    (euc_kr(STORAGE_HELP.0)? == original).then(|| euc_kr(STORAGE_HELP.1)).flatten()
}

pub fn landscape(on: bool) -> &'static [u8] {
    if on { LANDSCAPE } else { PORTRAIT }
}

pub fn on_off(on: bool) -> &'static [u8] {
    if on { ON } else { OFF }
}

/// `2배`: ASCII digits (the game draws them from `ftd/eng*`, as its own 시선이동 value) + 배.
pub fn times(n: u32) -> Vec<u8> {
    [n.to_string().as_bytes(), TIMES].concat()
}

/// `text` as the game's EUC-KR (Phase 7-3: the world map's place names, lettered by the game's
/// own font), or `None` when a character is outside what that font can draw — ASCII and the 2,350
/// KS X 1001 Hangul syllables (encoding_rs's EUC-KR is the wider CP949).
pub fn euc_kr(text: &str) -> Option<Vec<u8>> {
    let (bytes, _, unmappable) = encoding_rs::EUC_KR.encode(text);
    (!unmappable && ks_x_1001(&bytes)).then(|| bytes.into_owned())
}

/// Every non-ASCII pair is a KS X 1001 Hangul syllable (lead 0xB0..=0xC8, trail 0xA1..=0xFE).
fn ks_x_1001(b: &[u8]) -> bool {
    let mut i = 0;
    while i < b.len() {
        if b[i] < 0x80 {
            i += 1;
        } else if i + 1 < b.len() && (0xb0..=0xc8).contains(&b[i]) && (0xa1..=0xfe).contains(&b[i + 1]) {
            i += 2;
        } else {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stat_goes_by_one_name_in_goyang() {
        let line = euc_kr(WILL_LINE.0).unwrap();
        assert_eq!(line.len(), WILL_LINE_BYTES);
        assert_eq!(will_line(&line), euc_kr("의지가 증가한다."));
        assert_eq!(will_line(&euc_kr("건강이 증가한다.").unwrap()), None, "the other passives' lines stay");
        assert_eq!(will_line(&euc_kr("정신력이 증가했다.").unwrap()), None, "the same length is not the same line");
    }

    /// The only Korean the game's font can draw.
    fn drawable(b: &[u8]) -> bool {
        ks_x_1001(b)
    }

    /// Place names become the game's bytes — 실바나르 is byte-identical to map 0's own header —
    /// and a syllable the font lacks (CP949-only 똠) or a non-Korean character is refused.
    #[test]
    fn euc_kr_is_the_games_and_refuses_what_its_font_lacks() {
        assert_eq!(euc_kr("실바나르").unwrap(), [0xbd, 0xc7, 0xb9, 0xd9, 0xb3, 0xaa, 0xb8, 0xa3]);
        assert_eq!(euc_kr("실바나르 남부").unwrap()[8], b' ');
        assert!(euc_kr("똠").is_none() && euc_kr("漢").is_none());
        assert!(drawable(&euc_kr("퓌렌 게힌시티 뼈의평원").unwrap()));
    }

    #[test]
    fn every_string_is_in_the_games_font() {
        let all: Vec<Vec<u8>> = [NEXT_LAUNCH, LANDSCAPE, PORTRAIT, ON, OFF, TIMES, FREE, FIXED]
            .iter()
            .chain(LEVELS.iter())
            .map(|b| b.to_vec())
            .chain(Row::ALL.iter().map(|&r| label(r).to_vec()))
            .chain(RENAMES.iter().map(|(_, to)| to.to_vec()))
            .chain([ACCOUNT_STORAGE.to_vec()])
            .chain([times(5)])
            .collect();
        for b in all {
            assert!(drawable(&b), "{b:02x?}");
        }
    }

    /// The paragraph is the game's 268 bytes (rpg/d.J, decoded from `<clinit>` 2026-10-24), its
    /// rename is drawable, and anything else — a byte off, a shorter text — passes through.
    #[test]
    fn storage_help_is_renamed_only_when_it_is_the_games() {
        let game = euc_kr(STORAGE_HELP.0).unwrap();
        assert_eq!(game.len(), 268);
        assert_eq!(&game[..8], &[0xb0, 0xf8, 0xbf, 0xeb, 0xc3, 0xa2, 0xb0, 0xed], "공용창고");
        let renamed = storage_help(&game).unwrap();
        assert!(drawable(&renamed));
        let text = encoding_rs::EUC_KR.decode(&renamed).0;
        assert_eq!(text.matches("계정창고").count(), 4);
        assert!(text.contains("1,3 키로 캐릭터창고와 계정창고를 전환합니다.") && !text.contains("공용") && !text.contains("개인"));
        let mut off = game.clone();
        off[0] ^= 1;
        assert_eq!(storage_help(&off), None);
        assert_eq!(storage_help(&game[..game.len() - 1]), None);
    }

    #[test]
    fn renames_match_whole_entries_only() {
        let network_store = &RENAMES[0].0;
        assert_eq!(rename(network_store), Some(RENAMES[0].1));
        let mut longer = network_store.to_vec();
        longer.extend_from_slice(b"!");
        assert_eq!(rename(&longer), None);
        assert_eq!(rename(b""), None);
        assert_eq!(rename(ORIENTATION), None);
    }

    /// HZ-058: the six registration rows as `t/gm.tdf` has them (length-prefixed, 2026-10-30) become the
    /// plates' two words, in the game's font; anything else is left alone.
    #[test]
    fn the_registration_rows_become_the_plates() {
        let one: &[u8] = &[0x0d, 0x0a, 0x31, 0xb9, 0xf8, 0x20, 0xb5, 0xee, 0xb7, 0xcf];
        assert_eq!(quick_row(one), euc_kr("\r\n퀵슬롯등록"));
        assert_eq!(quick_row(&[0x30, 0xb9, 0xf8, 0x20, 0xb5, 0xee, 0xb7, 0xcf]), euc_kr("퀵슬롯등록"));
        assert_eq!(quick_row(&[0x23, 0xbf, 0xa1, 0x20, 0xb5, 0xee, 0xb7, 0xcf]), euc_kr("퀵슬롯해제"));
        for (from, to) in QUICK_ROWS {
            assert!(euc_kr(from).is_some() && euc_kr(to).is_some(), "{from:?} {to:?}");
        }
        assert_eq!(quick_row(&one[2..]), None, "the whole entry only");
        assert_eq!(quick_row(b""), None);
    }
}
