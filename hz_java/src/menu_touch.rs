//! Taps on the game's own windows (user 2026-10-25, the list drawn against hero2 and hero3: hero3's
//! touch menus). The game was made for a keypad, so a tap becomes what the keys would have done:
//! a row that is not the cursor's gets the cursor, the cursor's row gets OK — hero3's "first tap
//! picks, second runs" — and the main menu's rows are its own number keys.
//!
//! The windows are the game's `o` (javap 2026-10-25): 160 × 174 at `(o.a:I, o.b:I)` =
//! `((W − 160) >> 1, (H − 174) >> 1)`, the cursor `o.a:B`, the item count `o.b:B`, the open child
//! `o.b:Lo` — the last of that chain has the keys (`o.b(II)Z` passes them down). Fields are read
//! as `o` declares them ([`Jvm::get_field_from`]): 33 subclass fields shadow a superclass's.
//!
//! What a tap does is done by the HZ-008 blit hook between two frames, as the SAVE tab's ask is: a
//! key goes through the game's own entry `f.a:Lbj`.`a(0|1, code)` (`f.a()` calls it the same way
//! for a held key), a cursor is written and the window marked for drawing (`o.a:Z`).

use jvm::{ClassInstance, JavaValue, Jvm, Result as JvmResult};

/// The canvas codes the game compares (the Nokia profile, `wie.keyCodes=nokia`).
pub const OK: i32 = -5;
pub const HASH: i32 = 35;

/// A window's frame: `o`'s 160 × 174 (`o.<init>`; every window paints its frame there).
pub const WIN_W: i32 = 160;
pub const WIN_H: i32 = 174;

/// The most rows a list window draws (`m.a(dd,IIZ)I` fills to 10 slots).
const LIST_SLOTS: i32 = 10;

/// Where a window draws its rows across the frame, `(first row's top below the frame's, pitch)`
/// (javap 2026-10-25). The game's list `m.a(dd,IIZ)I` — rows 14 high, 15 apart, 16 down — in the
/// main menu `bh` (150 wide, numbered), 환경설정 `bn` (158), 상점및창고 `de` and 시스템 `ec`
/// (150); the slot list `bb` draws the same bars itself (`bb.a(dd)` @35); 계정창고 `bg` its two
/// 15 high, 17 apart, 25 down (`bg.a(dd)` @42).
pub fn row_layout(class: &str) -> Option<(i32, i32)> {
    match class {
        "bh" | "bn" | "de" | "ec" | "bb" => Some((16, 15)),
        "bg" => Some((25, 17)),
        _ => None,
    }
}

/// One open window as the game keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Win {
    pub class: String,
    pub cursor: i8,
    pub count: i8,
    pub x: i32,
    pub y: i32,
}

impl Win {
    /// For the log: `bh 3/10 @180,33`.
    pub fn line(&self) -> String {
        format!("{} {}/{} @{},{}", self.class, self.cursor, self.count, self.x, self.y)
    }

    /// The row under the guest point `(x, y)`, if the window draws rows ([`row_layout`]) and the
    /// point is on one of them (between two rows counts for the lower one).
    pub fn list_row(&self, x: i32, y: i32) -> Option<i8> {
        let (top, pitch) = row_layout(&self.class)?;
        if !(self.x..self.x + WIN_W).contains(&x) {
            return None;
        }
        let dy = y - (self.y + top);
        if dy < 0 {
            return None;
        }
        let row = dy / pitch;
        (row < i32::from(self.count).min(LIST_SLOTS)).then_some(row as i8)
    }
}

/// The game's question window `cf` (an `ey`: its own frame `ey.e/f/g/h:I`, javap 2026-10-25):
/// `count` choices under `lines` lines of question, 16 px a line — a column (`cf.h:Z`, the
/// scenario's choices: row i's bar at `top + 8 + 16·(i + lines)`, 16 high, the frame's width) or
/// 예/아니오 side by side (buttons 52 × 15 at `centre − 56 + 60·i`, `top + 8 + 16·lines`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choices {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub count: i32,
    pub lines: i32,
    pub column: bool,
    /// `rpg/a.c`, the canvas's centre x.
    pub centre: i32,
}

impl Choices {
    /// The choice under the guest point, with a few px of slack round a side-by-side button.
    pub fn at(&self, x: i32, y: i32) -> Option<i8> {
        let first = self.top + 8 + 16 * self.lines;
        if self.column {
            if !(self.left..self.left + self.width).contains(&x) || y < first {
                return None;
            }
            let row = (y - first) / 16;
            return (row < self.count).then_some(row as i8);
        }
        const SLACK: i32 = 3;
        (0..self.count).find(|&i| {
            let bx = self.centre - 56 + 60 * i;
            (bx - SLACK..bx + 52 + SLACK).contains(&x) && (first - SLACK..first + 15 + SLACK).contains(&y)
        }).map(|i| i as i8)
    }
}

/// The canvas codes for LEFT / RIGHT / DOWN (Nokia).
pub const LEFT: i32 = -3;
pub const RIGHT: i32 = -4;
pub const DOWN: i32 = -2;

/// The cell of a game grid (`q.a(dd,bt,BZBBI)V`: 14 px cells 15 apart from `(x0, y0)`, index =
/// row × cols + col) under the guest point.
pub fn grid_cell(x0: i32, y0: i32, cols: i32, rows: i32, x: i32, y: i32) -> Option<i8> {
    let (dx, dy) = (x - x0, y - y0);
    if dx < 0 || dy < 0 {
        return None;
    }
    let (c, r) = (dx / 15, dy / 15);
    (c < cols && r < rows).then_some((r * cols + c) as i8)
}

/// The bag tab under the guest point, of the column a bag window draws (`o.a(dd,BZIZ)V`: `count`
/// 11 px squares 13 apart at x `((W − 105) >> 1) − 12`, from 16 below the frame's top), 2 px of
/// slack round each.
pub fn bag_tab(canvas_w: i32, frame_y: i32, count: i32, x: i32, y: i32) -> Option<i8> {
    let tx = ((canvas_w - 105) >> 1) - 12;
    if !(tx - 2..tx + 13).contains(&x) {
        return None;
    }
    (0..count).find(|&i| {
        let ty = frame_y + 16 + 13 * i;
        (ty - 1..ty + 12).contains(&y)
    }).map(|i| i as i8)
}

/// The shop's buying tabs (`bz.a(dd,BBZ)V`): `count` of them, 11 × 10 at `frame_x + 20`, from
/// 19 below the frame's top, 2 px of slack either side.
pub fn shop_tab(frame_x: i32, frame_y: i32, count: i32, x: i32, y: i32) -> Option<i8> {
    if !(frame_x + 18..frame_x + 33).contains(&x) {
        return None;
    }
    let dy = y - (frame_y + 19);
    if dy < 0 {
        return None;
    }
    let tab = dy / 10;
    (tab < count).then_some(tab as i8)
}

/// The equipment slot under the guest point (`ea.a(dd,BLba;ZZ)V` @47..146): slots 0..5 three to
/// a row 15 apart from `(frame_x + 19, frame_y + 24)`, the middle column 7 px higher, slot 6
/// under the middle one at +30 − 7; each a 14 px square.
pub fn equip_slot(frame_x: i32, frame_y: i32, x: i32, y: i32) -> Option<i8> {
    let (x0, y0) = (frame_x + 19, frame_y + 24);
    let at = |i: i32| -> (i32, i32) {
        if i == 6 {
            (x0 + 15, y0 + 30 - 7)
        } else {
            let (col, row) = (i % 3, i / 3);
            (x0 + 15 * col, y0 + 15 * row + if col == 1 { -7 } else { 0 })
        }
    };
    (0..7).find(|&i| {
        let (sx, sy) = at(i);
        (sx..sx + 15).contains(&x) && (sy..sy + 15).contains(&y)
    }).map(|i| i as i8)
}

/// 환경설정 `bn`: a value changes with LEFT / RIGHT, OK does nothing to it (`bn.a(II)Z` @13..190;
/// RSK closes). A tap on the cursor's row picks by where it lands: its value half — left of the
/// value's middle LEFT, right of it RIGHT (the ◀ ▶ either side, measured on the S9's window) —
/// or its name, OK (the 「추가 설정」 row opens on it, HZ-007).
pub fn option_key(frame_x: i32, x: i32) -> i32 {
    match x - frame_x {
        ..80 => OK,
        80..118 => LEFT,
        _ => RIGHT,
    }
}

/// 캐릭터 `dy`'s four stats, 2 × 2 (`dy.a(dd)` @639..692): from `(frame_x + 45, frame_y + 59)`,
/// 53 apart across and 18 down, each a 26 px name and its value — 53 × 16.
pub fn stat_cell(frame_x: i32, frame_y: i32, x: i32, y: i32) -> Option<i8> {
    let (dx, dy) = (x - (frame_x + 45), y - (frame_y + 59));
    if !(0..106).contains(&dx) || !(0..34).contains(&dy) || dy % 18 >= 16 {
        return None;
    }
    Some((dy / 18 * 2 + dx / 53) as i8)
}

/// 퀘스트 `ao`'s page arrows either side of its `n/m` box (x 212..306 × y 48..56 at frame (180, 33),
/// measured on the probe's dump 2026-10-25): LEFT / RIGHT turn the quest (`o.d(II)Z`).
pub fn quest_arrow(frame_x: i32, frame_y: i32, x: i32, y: i32) -> Option<i32> {
    if !(frame_y + 10..frame_y + 28).contains(&y) {
        return None;
    }
    match x - frame_x {
        0..52 => Some(LEFT),
        108..160 => Some(RIGHT),
        _ => None,
    }
}

/// The rows of a popup menu `ee` (an `ey`: `count` rows 14 high from 2 below its top, `width`
/// = `ee.j:I` — what 사용 / 장착 / 버리기 is drawn in, `ee.a(dd)`) under the guest point.
pub fn popup_row(left: i32, top: i32, width: i32, count: i32, x: i32, y: i32) -> Option<i8> {
    if !(left..left + width).contains(&x) || y < top + 2 {
        return None;
    }
    let row = (y - top - 2) / 14;
    (row < count).then_some(row as i8)
}

/// The title's carousel button drawn at `(centre, top)`: 84 px of picture with its arrows either
/// side (measured on the S9's title, 2026-10-25) — the middle is OK, either side turns it.
pub fn carousel_key(centre: i32, top: i32, x: i32, y: i32) -> Option<i32> {
    if !(top - 6..top + 26).contains(&y) {
        return None;
    }
    match x - centre {
        -42..=42 => Some(OK),
        -80..=-43 => Some(LEFT),
        43..=80 => Some(RIGHT),
        _ => None,
    }
}

/// What is under a tap in 스킬 `ad` / 정령 `cr` (both a `cb`, javap 2026-10-25).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkillSpot {
    /// The tab header — `LEFT` / `RIGHT`, by the half.
    Tabs(i32),
    /// The page bar — `LEFT` / `RIGHT`, by the half.
    Pages(i32),
    /// A cell of the grid.
    Cell(i8),
}

/// The `cb` window's spots (`cb.b(dd)`, its focus painter): the tab header 104 wide at
/// `(centre − 52, frame_y + 13)`, 16 high, its arrows at the ends; the grid's 2 × 4 cells, 76 × 14
/// with a 5 px notch under each, at `(frame_x + 2 + 80·col, frame_y + 39 + 29·row)` (index row × 2
/// + col, below `count`); and, when the page has one (`pages`), the page bar 60 × 14 at
/// `(frame_x + 1, frame_y + 159)`. A cell takes the 29 px band from 5 above its box.
pub fn skill_spot(frame_x: i32, frame_y: i32, centre: i32, pages: bool, count: i8, x: i32, y: i32) -> Option<SkillSpot> {
    let half = |left: i32, w: i32| if x < left + w / 2 { LEFT } else { RIGHT };
    if (frame_y + 13..frame_y + 29).contains(&y) && (centre - 52..centre + 52).contains(&x) {
        return Some(SkillSpot::Tabs(half(centre - 52, 104)));
    }
    if pages && (frame_y + 159..frame_y + 173).contains(&y) && (frame_x + 1..frame_x + 61).contains(&x) {
        return Some(SkillSpot::Pages(half(frame_x + 1, 60)));
    }
    let (dx, dy) = (x - frame_x, y - (frame_y + 34));
    if !(0..160).contains(&dx) || !(0..29 * 4).contains(&dy) {
        return None;
    }
    let cell = dy / 29 * 2 + dx / 80;
    (cell < i32::from(count)).then_some(SkillSpot::Cell(cell as i8))
}

/// What is under a tap in 장비's pick list `bv` (an `ea`, javap 2026-10-25, `bv.b(dd)`): the 4 × 3
/// grid of 15 px cells from `(frame_x + 75, frame_y + 17)`, or — when there is more than one page —
/// the page arrows either side of it (`ea.e:Z` / `ea.f:Z` their focus), as `LEFT` / `RIGHT`. The
/// left arrow's strip stops short of 장비's own slots (to `frame_x + 64`).
pub fn pick_spot(frame_x: i32, frame_y: i32, pages: bool, x: i32, y: i32) -> Option<SkillSpot> {
    let (x0, y0) = (frame_x + 75, frame_y + 17);
    if pages && (y0..y0 + 45).contains(&y) {
        if (frame_x + 65..x0).contains(&x) {
            return Some(SkillSpot::Pages(LEFT));
        }
        if (x0 + 60..x0 + 72).contains(&x) {
            return Some(SkillSpot::Pages(RIGHT));
        }
    }
    grid_cell(x0, y0, 4, 3, x, y).map(SkillSpot::Cell)
}

/// What is under a tap in 계정창고's money window `w` (javap 2026-10-25, `w.a(dd)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoneySpot {
    /// 넣기 (0) / 꺼내기 (1): 154 × 18 at `(frame_x + 3, frame_y + 25 + 24·row)`.
    Row(i8),
    /// A digit of the amount, by its power of ten (`w.e:I`, 0 = units): nine of them 7 px apart
    /// from `frame_x + 14`, the highest first, on `frame_y + 68 .. + 86`.
    Digit(i32),
    /// The red 넣기 / 꺼내기 button, 27 × 14 at `(frame_x + 96, frame_y + 71)`: OK.
    Confirm,
    /// The ◀ ▶ either side of the digits (`frame_x + 8` and `+ 131`): `LEFT` / `RIGHT`.
    Arrow(i32),
}

pub fn money_spot(frame_x: i32, frame_y: i32, x: i32, y: i32) -> Option<MoneySpot> {
    let dx = x - frame_x;
    if (3..157).contains(&dx) {
        for row in 0..2 {
            let top = frame_y + 25 + 24 * row;
            if (top..top + 18).contains(&y) {
                return Some(MoneySpot::Row(row as i8));
            }
        }
    }
    if (frame_y + 71..frame_y + 85).contains(&y) && (96..123).contains(&dx) {
        return Some(MoneySpot::Confirm);
    }
    if !(frame_y + 68..frame_y + 86).contains(&y) {
        return None;
    }
    match dx {
        3..14 => Some(MoneySpot::Arrow(LEFT)),
        14..77 => Some(MoneySpot::Digit(8 - (dx - 14) / 7)),
        126..140 => Some(MoneySpot::Arrow(RIGHT)),
        _ => None,
    }
}

/// 전투스타일 선택 `bo` (the new game's style, javap 2026-10-25 `bo.a(II)Z`: LEFT / RIGHT turn it,
/// OK starts the game in it): the band from the three pictures to the 「◀ 워리어 ▶」 bar (`frame_y +
/// 55 .. + 113`, measured on the probe's dump) — the middle picture and name OK, either side
/// turns it, as the title's carousel.
pub fn style_key(centre: i32, frame_y: i32, x: i32, y: i32) -> Option<i32> {
    if !(frame_y + 55..frame_y + 113).contains(&y) {
        return None;
    }
    match x - centre {
        -20..=20 => Some(OK),
        -80..=-21 => Some(LEFT),
        21..=80 => Some(RIGHT),
        _ => None,
    }
}

/// What is under a tap in 전직 `bi` (the inn's job change, an `r`; javap 2026-10-26 `bi.a(dd)`,
/// `r.a(dd,B)I`, `m.a(dd,ZZZZZ)I`). Its page starts 27 below the frame (`m`'s title and job line):
/// the 이전 / 이후 pictures 80 × 47 at `frame_x − 1` and `+ 81`, 2 below that; the band of five job
/// boxes 160 × 18, 51 below it (box k centred on `frame_x + 19 + 31·k`, the middle one the
/// cursor's — [`job_row`]); and `r`'s name bar 「◀ 로그 ▶」 114 × 14 at `(frame_x + 23, frame_y +
/// 100)` over its description.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobSpot {
    /// The band's box `k` (0..5, left to right).
    Box(usize),
    /// The name bar — `LEFT` / `RIGHT`, by the half.
    Turn(i32),
    /// The 이후 picture, the job the cursor is on: OK.
    After,
}

pub fn job_spot(frame_x: i32, frame_y: i32, x: i32, y: i32) -> Option<JobSpot> {
    let top = frame_y + 27;
    let dx = x - frame_x;
    if (top + 51..top + 69).contains(&y) && (3..158).contains(&dx) {
        return Some(JobSpot::Box(((dx - 3) / 31) as usize));
    }
    if (top + 2..top + 49).contains(&y) && (81..161).contains(&dx) {
        return Some(JobSpot::After);
    }
    if (frame_y + 100..frame_y + 114).contains(&y) && (23..137).contains(&dx) {
        return Some(JobSpot::Turn(if dx < 80 { LEFT } else { RIGHT }));
    }
    None
}

/// The jobs in `bi`'s five boxes, as `bi.a(dd)` @220..368 lays them: two steps back from the cursor
/// then forward, a step never landing on the job the player has (`ed.b:B`) — the same steps its
/// LEFT / RIGHT take (`bi.a(II)Z` @240..330), so a box's job is where those keys would go.
pub fn job_row(cursor: i8, current: i8, count: i8) -> [i8; 5] {
    if count < 2 {
        return [cursor; 5]; // no other job to step to (the game's loop would never end)
    }
    let step = |j: i8, by: i8| (j + by + count) % count;
    let mut j = cursor;
    let mut back = 2;
    while back > 0 {
        j = step(j, -1);
        if j != current {
            back -= 1;
        }
    }
    let mut row = [0; 5];
    for slot in &mut row {
        *slot = j;
        j = step(j, 1);
        if j == current {
            j = step(j, 1);
        }
    }
    row
}

/// 도움말 `s` and 게임정보 `j` (`o.c(II)Z` pages their text with UP / DOWN; `s`'s topic header,
/// `frame_y + 13 .. + 27`, turns with LEFT / RIGHT): the header's halves, then the text's — its
/// upper half UP, its lower half DOWN.
pub fn text_key(frame_x: i32, frame_y: i32, header: bool, x: i32, y: i32) -> Option<i32> {
    if !(frame_x..frame_x + WIN_W).contains(&x) {
        return None;
    }
    let mid = frame_x + WIN_W / 2;
    if header && (frame_y + 13..frame_y + 27).contains(&y) {
        return Some(if x < mid { LEFT } else { RIGHT });
    }
    let (top, bottom) = (frame_y + 30, frame_y + WIN_H - 4);
    if !(top..bottom).contains(&y) {
        return None;
    }
    Some(if y < (top + bottom) / 2 { UP } else { DOWN })
}

/// The canvas codes for UP and RSK (Nokia).
pub const UP: i32 = -1;
pub const RSK: i32 = -7;

/// The main menu's number key for row `row` of `rows` (`bh.a(II)Z` @101: '0' is the last row — 9,
/// or 8 without 미니맵 (HZ-026) — and '1'.. the rows from 0).
pub fn menu_digit(row: i8, rows: i8) -> i32 {
    if row == rows - 1 { 48 } else { 49 + i32::from(row) }
}

/// What a tap did, for the log and the tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tapped {
    /// Nothing there to tap (the field, a window this does not know, outside its rows).
    Nothing,
    /// The game got this key, pressed and let go.
    Key(i32),
    /// The window's cursor moved to this row.
    Cursor(i8),
    /// A bag window's tabs got the focus, on this tab.
    Tabs(i8),
    /// The host opened a window of the game's over the one tapped (the bulk sale's grades).
    Opened,
    /// HZ-036: a bag window's pager turned the bag shown to this page (0-based).
    Page(u8),
}

fn object(v: JavaValue) -> Option<Box<dyn ClassInstance>> {
    v.into()
}

/// The window that has the keys in game mode `mode` (`ed.d:B`), as `ce.a(II)Z` routes them:
/// the main menu `bh` (5), the title `i` (2), and in a scenario (4) `eb.a(II)Z`'s order — its
/// shop or smith `eb.a:Lo`, its LSK/RSK question `eb.b:Ley`, its choices or message `eb.a:Ley`.
/// Read from the static fields, never through `bh.a()` / `i.a()`, which would make one.
async fn root(jvm: &Jvm, mode: u8) -> JvmResult<Option<Box<dyn ClassInstance>>> {
    Ok(match mode {
        5 => object(jvm.get_static_field("bh", "a", "Lbh;").await?),
        2 => object(jvm.get_static_field("i", "a", "Li;").await?),
        4 => {
            for (name, descriptor) in [("a", "Lo;"), ("b", "Ley;"), ("a", "Ley;")] {
                if let Some(w) = object(jvm.get_static_field("eb", name, descriptor).await?) {
                    return Ok(Some(w));
                }
            }
            None
        }
        _ => None,
    })
}

/// The open windows from the root down to the one with the keys.
pub async fn chain(jvm: &Jvm, mode: u8) -> JvmResult<Vec<Win>> {
    let mut out = Vec::new();
    let mut next = root(jvm, mode).await?;
    while let Some(w) = next {
        out.push(read(jvm, &w).await?);
        if out.len() > 8 {
            break; // a cycle would be the game's bug; never spin on it
        }
        next = object(jvm.get_field_from(&w, "o", "b", "Lo;").await?);
    }
    Ok(out)
}

#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
async fn read(jvm: &Jvm, w: &Box<dyn ClassInstance>) -> JvmResult<Win> {
    Ok(Win {
        class: w.class_definition().name(),
        cursor: jvm.get_field_from(w, "o", "a", "B").await?,
        count: jvm.get_field_from(w, "o", "b", "B").await?,
        x: jvm.get_field_from(w, "o", "a", "I").await?,
        y: jvm.get_field_from(w, "o", "b", "I").await?,
    })
}

/// The window with the keys itself (the end of [`chain`]).
pub async fn top(jvm: &Jvm, mode: u8) -> JvmResult<Option<Box<dyn ClassInstance>>> {
    let mut cur = root(jvm, mode).await?;
    for _ in 0..8 {
        let Some(w) = cur.as_ref() else { return Ok(None) };
        match object(jvm.get_field_from(w, "o", "b", "Lo;").await?) {
            Some(child) => cur = Some(child),
            None => break,
        }
    }
    Ok(cur)
}

/// The title's 이어하기 list (`bb` of kind 1, `bb.f:B`) just opened: put its cursor on `slot`
/// (0-based) if that slot holds a save (`ed.a:[[B`) — what a player's DOWN presses would do. The
/// game starts it on the slot chosen earlier this run (`ed.j:B`) or the first one with a save
/// (`bb.<init>` @76..136); across runs it knows none, and `ed.j` is the slot the game writes, so
/// only the list's cursor is touched. Returns whether it moved.
pub async fn preselect_slot(jvm: &Jvm, slot: i8) -> JvmResult<bool> {
    let Some(mut w) = top(jvm, 2).await? else { return Ok(false) };
    if w.class_definition().name() != "bb" || jvm.get_field_from::<i8>(&w, "bb", "f", "B").await? != 1 {
        return Ok(false);
    }
    let Some(saves) = object(jvm.get_static_field("ed", "a", "[[B").await?) else { return Ok(false) };
    if slot < 0 || slot as usize >= jvm.array_length(&saves).await? {
        return Ok(false);
    }
    let at: Vec<JavaValue> = jvm.load_array(&saves, slot as usize, 1).await?;
    if at.into_iter().next().and_then(object).is_none() || jvm.get_field_from::<i8>(&w, "o", "a", "B").await? == slot {
        return Ok(false);
    }
    jvm.put_field_from(&mut w, "o", "a", "B", slot).await?;
    jvm.put_field_from(&mut w, "o", "a", "Z", true).await?;
    Ok(true)
}

/// A key as the game's canvas would deliver it: pressed, then let go (`f.a()`'s calls).
pub async fn press(jvm: &Jvm, code: i32) -> JvmResult<()> {
    let Some(screen) = object(jvm.get_static_field("f", "a", "Lbj;").await?) else {
        return Ok(());
    };
    for kind in [0, 1] {
        let _: bool = jvm.invoke_virtual(&screen, "a", "(II)Z", [JavaValue::Int(kind), JavaValue::Int(code)]).await?;
    }
    Ok(())
}

/// What a pad's page buttons turn in the window that has the keys (user 2026-10-30: 「패드일때
/// 다음페이지는 1,3번 버튼은 LB RB 버튼으로」). The game's own `1` / `3` where it has them — the shop's
/// buying tabs 무기 · 방어구 · 기타 (`co.a(II)Z` @151..226) and 계정창고's two sides (`eg.a(II)Z`
/// @29..71), the only keys of the game's windows that no arrow reaches (the number-key audit, javap
/// 2026-10-26 and again 2026-10-30) — then HZ-036's pages of the bag a window shows, and HZ-020's slot
/// pages at the title (LEFT / RIGHT, which `Hooks.slotKey` turns).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pager {
    /// `1` back, `3` on.
    Keys,
    /// The player's bag (0..3) the window shows.
    Bag(usize),
    /// LEFT back, RIGHT on.
    Arrows,
}

/// [`Pager`] for what is up now (`bags` = HZ-036 on, `slots` = HZ-020 on); `None` = the page buttons
/// have nothing to turn — a message box or the 「# 확인」 notice has the keys, or the window has no pages.
pub async fn pager(jvm: &Jvm, mode: Option<u8>, popup: bool, notice: bool, bags: bool, slots: bool) -> JvmResult<Option<Pager>> {
    let Some(mode) = mode else { return Ok(None) };
    if popup || notice {
        return Ok(None);
    }
    let Some(w) = top(jvm, mode).await? else { return Ok(None) };
    let class = w.class_definition().name();
    if class == "eg" || (class == "co" && jvm.get_field_from::<i8>(&w, "co", "h", "B").await? < 3) {
        return Ok(Some(Pager::Keys));
    }
    if class == "bb" {
        return Ok((slots && mode == 2).then_some(Pager::Arrows));
    }
    if !bags {
        return Ok(None);
    }
    Ok(crate::bags::window_bag(jvm, &w).await?.map(Pager::Bag))
}

/// The page buttons' press, `delta` −1 back · +1 on: what [`pager`] says, done as the tap on the same
/// thing does it (the title band's [1] / [3], the bag's ◀ ▶, the slot window's arrows).
pub async fn page(jvm: &Jvm, mode: Option<u8>, popup: bool, notice: bool, delta: i8, bags: bool, slots: bool) -> JvmResult<Tapped> {
    let back = delta < 0;
    match pager(jvm, mode, popup, notice, bags, slots).await? {
        None => Ok(Tapped::Nothing),
        Some(Pager::Keys) => {
            let key = if back { 49 } else { 51 };
            press(jvm, key).await?;
            Ok(Tapped::Key(key))
        }
        Some(Pager::Arrows) => {
            let key = if back { LEFT } else { RIGHT };
            press(jvm, key).await?;
            Ok(Tapped::Key(key))
        }
        Some(Pager::Bag(bag)) => {
            let page = crate::bags::turn(bag, delta.signum());
            if let Some(mode) = mode
                && let Some(mut w) = top(jvm, mode).await?
            {
                jvm.put_field_from(&mut w, "o", "a", "Z", true).await?;
            }
            Ok(Tapped::Page(page))
        }
    }
}

/// The boot's 「소리를 켭니까?」 (`ch` in its state 4, before the game screen `ce` exists): 예 / 아니오
/// are 50 × 20 boxes at `(centre − 75, middle + 58)` and `(centre + 25, …)`, the choice `ch.b:Z`
/// (true = 예, `ch.d(dd)` @317) — `true` for a point on 예, `false` on 아니오.
pub fn sound_answer(centre: i32, middle: i32, x: i32, y: i32) -> Option<bool> {
    const SLACK: i32 = 4;
    let top = middle + 58;
    if !(top - SLACK..top + 20 + SLACK).contains(&y) {
        return None;
    }
    if (centre - 75 - SLACK..centre - 25 + SLACK).contains(&x) {
        Some(true)
    } else if (centre + 25 - SLACK..centre + 75 + SLACK).contains(&x) {
        Some(false)
    } else {
        None
    }
}

/// A tap before the game screen exists, the boot `ch` (`ch.a(II)Z`): its 「PRESS ANY KEY」 (state
/// 2) takes any key; its sound question (state 4) turns its choice over with LEFT or RIGHT
/// (@202..233) and takes it with OK.
async fn boot_tap(jvm: &Jvm, x: i32, y: i32) -> JvmResult<Tapped> {
    let Some(boot) = object(jvm.get_static_field("f", "a", "Lbj;").await?) else {
        return Ok(Tapped::Nothing);
    };
    if !jvm.is_instance(&*boot, "ch") {
        return Ok(Tapped::Nothing);
    }
    match jvm.get_field_from::<i8>(&boot, "ch", "b", "B").await? {
        2 => {
            press(jvm, OK).await?;
            return Ok(Tapped::Key(OK));
        }
        4 => {}
        _ => return Ok(Tapped::Nothing),
    }
    let (cx, cy): (i32, i32) = (jvm.get_static_field("rpg/a", "c", "I").await?, jvm.get_static_field("rpg/a", "d", "I").await?);
    let Some(yes) = sound_answer(cx, cy, x, y) else {
        return Ok(Tapped::Nothing);
    };
    let chosen: bool = jvm.get_static_field("ch", "b", "Z").await?;
    let key = if yes == chosen { OK } else if yes { LEFT } else { RIGHT };
    press(jvm, key).await?;
    Ok(Tapped::Key(key))
}

/// A tap at the guest point `(x, y)` in game mode `mode` (`None` before the game screen
/// exists), with a message box up (`popup`) or the 「# 확인」 notice (`notice`) — the scene the
/// hook just read.
pub async fn tap(jvm: &Jvm, mode: Option<u8>, popup: bool, notice: bool, x: i32, y: i32) -> JvmResult<Tapped> {
    let Some(mode) = mode else {
        return boot_tap(jvm, x, y).await;
    };
    // `ce.a(II)Z`'s order: the notice takes `#`, a message box takes the next key.
    if notice {
        press(jvm, HASH).await?;
        return Ok(Tapped::Key(HASH));
    }
    if popup {
        press(jvm, OK).await?;
        return Ok(Tapped::Key(OK));
    }
    let Some(mut w) = top(jvm, mode).await? else {
        // A scenario with no window up is its dialogue: OK turns the page (`eb.a(II)Z` @952).
        if mode == 4 {
            press(jvm, OK).await?;
            return Ok(Tapped::Key(OK));
        }
        return Ok(Tapped::Nothing);
    };
    let win = read(jvm, &w).await?;
    // HZ-036: a bag window's ◀ n/4 ▶ under its tabs turns the player's bag it shows.
    if matches!(win.class.as_str(), "ds" | "eg" | "co" | "dv" | "et")
        && let Some(bag) = crate::bags::window_bag(jvm, &w).await?
    {
        let canvas_w: i32 = jvm.get_static_field("rpg/a", "a", "I").await?;
        if let Some(delta) = crate::bags::pager_hit(canvas_w, win.y, x, y) {
            let page = crate::bags::turn(bag, delta);
            jvm.put_field_from(&mut w, "o", "a", "Z", true).await?;
            return Ok(Tapped::Page(page));
        }
    }
    let row = match win.class.as_str() {
        // A message (`b.a([B)`): OK closes it (`cd.a(II)Z` @62) — unless it is the two-way kind
        // that answers LSK / RSK (`cd.e:Z`) or one that takes no key (`cd.g:Z`).
        "cd" => {
            let two_way: bool = jvm.get_field_from(&w, "cd", "e", "Z").await?;
            let deaf: bool = jvm.get_field_from(&w, "cd", "g", "Z").await?;
            if two_way || deaf {
                return Ok(Tapped::Nothing);
            }
            press(jvm, OK).await?;
            return Ok(Tapped::Key(OK));
        }
        "cf" => {
            let c = Choices {
                left: jvm.get_field_from(&w, "ey", "e", "I").await?,
                top: jvm.get_field_from(&w, "ey", "f", "I").await?,
                width: jvm.get_field_from(&w, "ey", "g", "I").await?,
                count: jvm.get_field_from(&w, "ey", "i", "I").await?,
                lines: jvm.get_field_from(&w, "cf", "l", "I").await?,
                column: jvm.get_field_from(&w, "cf", "h", "Z").await?,
                centre: jvm.get_static_field("rpg/a", "c", "I").await?,
            };
            c.at(x, y)
        }
        // The title's carousel (`i.b(dd)`): one button at `(H/2) · 3/2 − 3`, turned with LEFT /
        // RIGHT (`o.d(II)Z`), OK opens it.
        "i" => {
            let (cx, cy): (i32, i32) = (jvm.get_static_field("rpg/a", "c", "I").await?, jvm.get_static_field("rpg/a", "d", "I").await?);
            let Some(key) = carousel_key(cx, cy + (cy >> 1) - 3, x, y) else {
                return Ok(Tapped::Nothing);
            };
            press(jvm, key).await?;
            return Ok(Tapped::Key(key));
        }
        // The bag (인벤토리): its three bag tabs and its 7 × 4 grid (`ds.a(dd)`); `ds.e:Z` is the
        // tabs' focus, `ds.g:B` their cursor, `ds.f:B` the bag shown (`ds.a(II)Z` @29..145).
        "ds" => {
            let shown: i8 = jvm.get_field_from(&w, "ds", "f", "B").await?;
            let canvas_w: i32 = jvm.get_static_field("rpg/a", "a", "I").await?;
            let tab = bag_tab(canvas_w, win.y, 3, x, y);
            return tabbed(jvm, &mut w, &win, "ds", ("e", "g"), shown, 3, tab, x, y).await;
        }
        // The shop (`co`, `bz`'s 7 × 4 grid): buying, `co.h:B` < 3, its category tabs (`bz.a(dd,BBZ)V`:
        // 11 × 10 at `x + 20`, from 19 down); selling, the bag tabs. `co.e:Z` focus, `co.i:B` the
        // tab (and the list shown — DOWN on the tabs turns and refills it, `co.a(II)Z` @228..321),
        // `co.j:B` how many.
        "co" => {
            // HZ-021's 「일괄판매」, while the shop sells with nothing open over it.
            let (bx, by, bw, bh) = crate::bulk::button_at(win.x, win.y);
            if (bx..bx + bw).contains(&x) && (by..by + bh).contains(&y) && crate::bulk::button_shown(jvm, &w).await? {
                crate::bulk::open(jvm, &mut w).await?;
                return Ok(Tapped::Opened);
            }
            let buying = jvm.get_field_from::<i8>(&w, "co", "h", "B").await? < 3;
            // Buying, its title band's [1] and [3] (where 계정창고's are: x 181..190 and 330..338 at
            // frame (180, 33), probe 2026-10-26) turn between 무기, 방어구 and 기타 (`co.a(II)Z`
            // @151..226: '1' back, '3' on) — keys only the pull-out keypad carried until it went.
            if buying && (win.y..win.y + 13).contains(&y) {
                let key = match x - win.x {
                    0..20 => 49,
                    140..160 => 51,
                    _ => return Ok(Tapped::Nothing),
                };
                press(jvm, key).await?;
                return Ok(Tapped::Key(key));
            }
            let shown: i8 = jvm.get_field_from(&w, "co", "i", "B").await?;
            let count = i32::from(jvm.get_field_from::<i8>(&w, "co", "j", "B").await?);
            let tab = if buying {
                shop_tab(win.x, win.y, count, x, y)
            } else {
                let canvas_w: i32 = jvm.get_static_field("rpg/a", "a", "I").await?;
                bag_tab(canvas_w, win.y, count, x, y)
            };
            return tabbed(jvm, &mut w, &win, "co", ("e", "i"), shown, count, tab, x, y).await;
        }
        // 장비: the seven slots; the first look shows a note that any key closes (`em.h:Z`).
        "em" => {
            let note: bool = jvm.get_field_from(&w, "em", "h", "Z").await?;
            if note {
                press(jvm, OK).await?;
                return Ok(Tapped::Key(OK));
            }
            equip_slot(win.x, win.y, x, y)
        }
        // 계정창고's items (`eg`): the bag tabs and 7 × 4 grid again — `eg.h:Z` the tabs' focus,
        // `eg.g:B` the tab (DOWN on the tabs turns and refills, `eg.a(II)Z` @73..178), `eg.h:B`
        // how many.
        "eg" => {
            // Its title band's [1] and [3] (x 181..190 and 330..338 at frame (180, 33), measured):
            // the keys that turn between 캐릭터창고 and 계정창고 (`eg.a(II)Z` @29..71).
            if (win.y..win.y + 13).contains(&y) {
                let key = match x - win.x {
                    0..20 => 49,
                    140..160 => 51,
                    _ => return Ok(Tapped::Nothing),
                };
                press(jvm, key).await?;
                return Ok(Tapped::Key(key));
            }
            let shown: i8 = jvm.get_field_from(&w, "eg", "g", "B").await?;
            let count = i32::from(jvm.get_field_from::<i8>(&w, "eg", "h", "B").await?);
            let canvas_w: i32 = jvm.get_static_field("rpg/a", "a", "I").await?;
            let tab = bag_tab(canvas_w, win.y, count, x, y);
            return tabbed(jvm, &mut w, &win, "eg", ("h", "g"), shown, count, tab, x, y).await;
        }
        // 대장간 `et` and 연금술사 `dv` while picking from the bag (`e:Z`): `bz`'s 7 × 4 grid. Their
        // other page (the refine or the recipe and its keys) takes no taps.
        "et" | "dv" => {
            let picking: bool = jvm.get_field_from(&w, &win.class, "e", "Z").await?;
            if !picking {
                return Ok(Tapped::Nothing);
            }
            grid_cell(win.x + 40, win.y + 17, 7, 4, x, y)
        }
        // 캐릭터: the stats take a point only while there are points to spend (`dy.e:Z`); without,
        // OK would close it, and a tap is nothing.
        "dy" => {
            let points: bool = jvm.get_field_from(&w, "dy", "e", "Z").await?;
            if !points {
                return Ok(Tapped::Nothing);
            }
            stat_cell(win.x, win.y, x, y)
        }
        // 퀘스트: its arrows; OK would close it.
        "ao" => {
            let Some(key) = quest_arrow(win.x, win.y, x, y) else {
                return Ok(Tapped::Nothing);
            };
            press(jvm, key).await?;
            return Ok(Tapped::Key(key));
        }
        // 스킬 `ad` and 정령 `cr` (a `cb`): the focus is the tab header (`cb.e:Z`, where the window
        // opens), the page bar (`cb.f:Z` — `ad`'s 패시브 tab, `cr` always) or the grid. LEFT / RIGHT
        // turn whichever of the first two has it (`cb.a(IIB)Z`, `cb.b(IIB)Z`), so a tap there takes
        // the focus first; a cell is the cursor's (the grid's own moves only change `o.a:B`, `cb.a(
        // IIIZ)Z`), and a second tap on it is OK.
        "ad" | "cr" => {
            let tab: i32 = jvm.get_field_from(&w, "cb", "e", "I").await?;
            let centre: i32 = jvm.get_static_field("rpg/a", "c", "I").await?;
            let pages = win.class == "cr" || tab == 1;
            let Some(spot) = skill_spot(win.x, win.y, centre, pages, win.count, x, y) else {
                return Ok(Tapped::Nothing);
            };
            let (on_tabs, on_pages): (bool, bool) =
                (jvm.get_field_from(&w, "cb", "e", "Z").await?, jvm.get_field_from(&w, "cb", "f", "Z").await?);
            return match spot {
                SkillSpot::Tabs(key) | SkillSpot::Pages(key) => {
                    let tabs = matches!(spot, SkillSpot::Tabs(_));
                    jvm.put_field_from(&mut w, "cb", "e", "Z", tabs).await?;
                    jvm.put_field_from(&mut w, "cb", "f", "Z", !tabs).await?;
                    press(jvm, key).await?;
                    Ok(Tapped::Key(key))
                }
                SkillSpot::Cell(cell) if !on_tabs && !on_pages && cell == win.cursor => {
                    press(jvm, OK).await?;
                    Ok(Tapped::Key(OK))
                }
                SkillSpot::Cell(cell) => {
                    jvm.put_field_from(&mut w, "cb", "e", "Z", false).await?;
                    jvm.put_field_from(&mut w, "cb", "f", "Z", false).await?;
                    jvm.put_field_from(&mut w, "o", "a", "B", cell).await?;
                    jvm.put_field_from(&mut w, "o", "a", "Z", true).await?;
                    Ok(Tapped::Cursor(cell))
                }
            };
        }
        // 장비's pick list `bv`: a cell is the cursor's and tries the item on, as the arrows do
        // (`bv.h:Z`, which `bv.a(dd)` acts on); a second tap is OK (its 장착 menu). With pages
        // (`ea.f:I` > 1) LEFT / RIGHT on an arrow's focus turn them (`bv.a(II)Z` @160..340).
        "bv" => {
            let pages: i32 = jvm.get_field_from(&w, "ea", "f", "I").await?;
            let Some(spot) = pick_spot(win.x, win.y, pages > 1, x, y) else {
                return Ok(Tapped::Nothing);
            };
            let (left, right): (bool, bool) =
                (jvm.get_field_from(&w, "ea", "e", "Z").await?, jvm.get_field_from(&w, "ea", "f", "Z").await?);
            return match spot {
                SkillSpot::Pages(key) => {
                    jvm.put_field_from(&mut w, "ea", "e", "Z", key == LEFT).await?;
                    jvm.put_field_from(&mut w, "ea", "f", "Z", key == RIGHT).await?;
                    press(jvm, key).await?;
                    Ok(Tapped::Key(key))
                }
                SkillSpot::Cell(cell) if !left && !right && cell == win.cursor => {
                    press(jvm, OK).await?;
                    Ok(Tapped::Key(OK))
                }
                SkillSpot::Cell(cell) => {
                    jvm.put_field_from(&mut w, "ea", "e", "Z", false).await?;
                    jvm.put_field_from(&mut w, "ea", "f", "Z", false).await?;
                    jvm.put_field_from(&mut w, "o", "a", "B", cell).await?;
                    jvm.put_field_from(&mut w, "bv", "h", "Z", true).await?;
                    jvm.put_field_from(&mut w, "o", "a", "Z", true).await?;
                    Ok(Tapped::Cursor(cell))
                }
                SkillSpot::Tabs(_) => Ok(Tapped::Nothing),
            };
        }
        // The new game's 전투스타일 선택, and the title's / 시스템's 도움말 and 게임정보.
        "bo" | "s" | "j" => {
            let key = if win.class == "bo" {
                let centre: i32 = jvm.get_static_field("rpg/a", "c", "I").await?;
                style_key(centre, win.y, x, y)
            } else {
                text_key(win.x, win.y, win.class == "s", x, y)
            };
            let Some(key) = key else {
                return Ok(Tapped::Nothing);
            };
            press(jvm, key).await?;
            return Ok(Tapped::Key(key));
        }
        // 전직 `bi`, with nothing over it: its state `bi.f:B` is 1 on the job page (0 = the opening
        // question, 2..5 the change drawing itself, 6 its closing question); from 7 any key closes
        // it (`bi.a(II)Z` @28..58, after its 소지금 message). On the page a box is the cursor's (what
        // LEFT / RIGHT would reach — the keys only move `o.a:B`) and the cursor's box, or the 이후
        // picture, is OK (the game's cost question).
        "bi" => {
            let state: i8 = jvm.get_field_from(&w, "bi", "f", "B").await?;
            if state >= 7 {
                press(jvm, OK).await?;
                return Ok(Tapped::Key(OK));
            }
            if state != 1 {
                return Ok(Tapped::Nothing);
            }
            let key = match job_spot(win.x, win.y, x, y) {
                None => return Ok(Tapped::Nothing),
                Some(JobSpot::Turn(key)) => key,
                Some(JobSpot::After) | Some(JobSpot::Box(2)) => OK,
                Some(JobSpot::Box(k)) => {
                    let current: i8 = jvm.get_static_field("ed", "b", "B").await?;
                    let job = job_row(win.cursor, current, win.count)[k];
                    jvm.put_field_from(&mut w, "o", "a", "B", job).await?;
                    jvm.put_field_from(&mut w, "o", "a", "Z", true).await?;
                    return Ok(Tapped::Cursor(job));
                }
            };
            press(jvm, key).await?;
            return Ok(Tapped::Key(key));
        }
        // 계정창고's money `w`: first 넣기 / 꺼내기 (`w.g:Z`, rows), then the amount — the cursor
        // `o.a:B` is the chosen digit's value, `w.e:I` which digit, `w.c:[B` the digits (`w.a(II)Z`:
        // LEFT / RIGHT move along them, UP / DOWN `o.a(4)` / `o.a(3)` step the value, OK moves
        // the money, RSK goes back to the rows). A digit is picked, and a second tap steps it up;
        // a row tapped while the amount is open goes back to the rows (RSK), on that row.
        "w" => {
            let choosing: bool = jvm.get_field_from(&w, "w", "g", "Z").await?;
            let Some(spot) = money_spot(win.x, win.y, x, y) else {
                return Ok(Tapped::Nothing);
            };
            let key = match spot {
                MoneySpot::Row(row) if choosing => {
                    if row == win.cursor {
                        OK
                    } else {
                        jvm.put_field_from(&mut w, "o", "a", "B", row).await?;
                        jvm.put_field_from(&mut w, "o", "a", "Z", true).await?;
                        return Ok(Tapped::Cursor(row));
                    }
                }
                MoneySpot::Row(row) => {
                    press(jvm, RSK).await?;
                    jvm.put_field_from(&mut w, "o", "a", "B", row).await?;
                    jvm.put_field_from(&mut w, "o", "a", "Z", true).await?;
                    return Ok(Tapped::Key(RSK));
                }
                MoneySpot::Confirm => OK,
                MoneySpot::Digit(_) | MoneySpot::Arrow(_) if choosing => return Ok(Tapped::Nothing),
                MoneySpot::Arrow(key) => key,
                MoneySpot::Digit(power) => {
                    if power == jvm.get_field_from::<i32>(&w, "w", "e", "I").await? {
                        UP
                    } else {
                        let digits = object(jvm.get_field_from(&w, "w", "c", "[B").await?);
                        let Some(digits) = digits else {
                            return Ok(Tapped::Nothing);
                        };
                        let value: Vec<i8> = jvm.load_array(&digits, power as usize, 1).await?;
                        jvm.put_field_from(&mut w, "w", "e", "I", power).await?;
                        jvm.put_field_from(&mut w, "o", "a", "B", value[0]).await?;
                        jvm.put_field_from(&mut w, "o", "a", "Z", true).await?;
                        return Ok(Tapped::Cursor(value[0]));
                    }
                }
            };
            press(jvm, key).await?;
            return Ok(Tapped::Key(key));
        }
        // The slot window's title band (HZ-020): its ◀ ▶ turn the slot page.
        "bb" if (win.y..win.y + 14).contains(&y) && (win.x..win.x + WIN_W).contains(&x) => {
            let key = if x < win.x + WIN_W / 2 { LEFT } else { RIGHT };
            press(jvm, key).await?;
            return Ok(Tapped::Key(key));
        }
        // An item's menu (사용 / 장착 / 버리기, `ee`).
        "ee" => popup_row(
            jvm.get_field_from(&w, "ey", "e", "I").await?,
            jvm.get_field_from(&w, "ey", "f", "I").await?,
            jvm.get_field_from(&w, "ee", "j", "I").await?,
            jvm.get_field_from(&w, "ey", "i", "I").await?,
            x,
            y,
        ),
        _ => win.list_row(x, y),
    };
    let Some(row) = row else {
        return Ok(Tapped::Nothing);
    };
    if win.class == "bh" {
        let key = menu_digit(row, win.count);
        press(jvm, key).await?;
        return Ok(Tapped::Key(key));
    }
    if row == win.cursor {
        let key = if win.class == "bn" { option_key(win.x, x) } else { OK };
        press(jvm, key).await?;
        return Ok(Tapped::Key(key));
    }
    jvm.put_field_from(&mut w, "o", "a", "B", row).await?;
    jvm.put_field_from(&mut w, "o", "a", "Z", true).await?;
    Ok(Tapped::Cursor(row))
}

/// A window of tabs beside the game's 7 × 4 grid (from `(x + 40, y + 17)`): the bag, the shop.
/// `fields` = its `(tabs' focus Z, tab cursor B)`; on the tabs DOWN goes to the next tab, so a
/// tab is reached by putting the cursor just before it. A tapped tab that is the one shown takes
/// the focus; a cell is the cursor's, and a second tap on it is OK.
#[allow(clippy::too_many_arguments, clippy::borrowed_box)]
async fn tabbed(
    jvm: &Jvm,
    w: &mut Box<dyn ClassInstance>,
    win: &Win,
    class: &str,
    (focus, cursor): (&str, &str),
    shown: i8,
    count: i32,
    tab: Option<i8>,
    x: i32,
    y: i32,
) -> JvmResult<Tapped> {
    let on_tabs: bool = jvm.get_field_from(w, class, focus, "Z").await?;
    if let Some(tab) = tab {
        jvm.put_field_from(w, class, focus, "Z", true).await?;
        if tab == shown {
            jvm.put_field_from(w, "o", "a", "Z", true).await?;
            return Ok(Tapped::Tabs(tab));
        }
        let before = (i32::from(tab) + count - 1) % count.max(1);
        jvm.put_field_from(w, class, cursor, "B", before as i8).await?;
        press(jvm, DOWN).await?;
        return Ok(Tapped::Key(DOWN));
    }
    let Some(cell) = grid_cell(win.x + 40, win.y + 17, 7, 4, x, y) else {
        return Ok(Tapped::Nothing);
    };
    if !on_tabs && cell == win.cursor {
        press(jvm, OK).await?;
        return Ok(Tapped::Key(OK));
    }
    jvm.put_field_from(w, class, focus, "Z", false).await?;
    jvm.put_field_from(w, "o", "a", "B", cell).await?;
    jvm.put_field_from(w, "o", "a", "Z", true).await?;
    Ok(Tapped::Cursor(cell))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(class: &str, cursor: i8, count: i8) -> Win {
        // 520 × 240: the frame at ((520 − 160) >> 1, (240 − 174) >> 1) = (180, 33).
        Win { class: class.into(), cursor, count, x: 180, y: 33 }
    }

    #[test]
    fn a_list_row_is_fifteen_pixels_from_sixteen_below_the_frame() {
        let w = win("bh", 0, 10);
        assert_eq!(w.list_row(260, 33 + 16), Some(0), "the first row's top line");
        assert_eq!(w.list_row(260, 33 + 16 + 14), Some(0), "its gap");
        assert_eq!(w.list_row(260, 33 + 16 + 15), Some(1));
        assert_eq!(w.list_row(260, 33 + 16 + 15 * 9 + 13), Some(9), "the tenth row");
        assert_eq!(w.list_row(260, 33 + 16 + 15 * 10), None, "below the last slot");
        assert_eq!(w.list_row(260, 33 + 15), None, "the title band");
        assert_eq!(w.list_row(179, 60), None, "left of the frame");
        assert_eq!(w.list_row(340, 60), None, "right of it");
        assert_eq!(win("bn", 0, 4).list_row(260, 33 + 16 + 15 * 4), None, "a 4-row page ends there");
        let storage = win("bg", 0, 2);
        assert_eq!((storage.list_row(260, 33 + 25), storage.list_row(260, 33 + 25 + 17)), (Some(0), Some(1)), "17 apart");
        assert_eq!(storage.list_row(260, 33 + 24), None);
        assert_eq!(win("ds", 0, 28).list_row(260, 60), None, "a grid is not a list");
    }

    #[test]
    fn the_title_button_is_ok_in_the_middle_and_turns_at_its_sides() {
        // 520 × 240: the button at y 177.
        assert_eq!(carousel_key(260, 177, 260, 185), Some(OK));
        assert_eq!(carousel_key(260, 177, 205, 185), Some(LEFT));
        assert_eq!(carousel_key(260, 177, 315, 185), Some(RIGHT));
        assert_eq!(carousel_key(260, 177, 260, 120), None, "the logo");
        assert_eq!(carousel_key(260, 177, 100, 185), None, "far off");
    }

    #[test]
    fn a_question_is_answered_where_its_choices_are_drawn() {
        // 「저장하시겠습니까?」 on 520 × 240: one line, 예/아니오 side by side; the frame is
        // ((2 + 1) << 4) + 13 − 16 = 45 high at 120 − 22 = 98 (`cf.<init>`).
        let yes_no = Choices { left: 180, top: 98, width: 160, count: 2, lines: 1, column: false, centre: 260 };
        assert_eq!(yes_no.at(230, 129), Some(0), "예: 204..256 x 122..137");
        assert_eq!(yes_no.at(290, 129), Some(1), "아니오: 264..316");
        assert_eq!(yes_no.at(260, 129), None, "between the buttons");
        assert_eq!(yes_no.at(230, 108), None, "on the question");
        // A column of three under two lines: rows from 98 + 8 + 32.
        let column = Choices { left: 200, top: 98, width: 120, count: 3, lines: 2, column: true, centre: 260 };
        assert_eq!(column.at(250, 138), Some(0));
        assert_eq!(column.at(250, 138 + 16 * 2 + 15), Some(2));
        assert_eq!(column.at(250, 138 + 16 * 3), None, "below the last");
        assert_eq!(column.at(199, 140), None, "beside the frame");
    }

    #[test]
    fn the_bag_has_tabs_left_of_a_seven_by_four_grid() {
        // 520 × 240: the frame at (180, 33), the grid from (220, 50), the tabs at x 195.
        assert_eq!(grid_cell(220, 50, 7, 4, 220, 50), Some(0));
        assert_eq!(grid_cell(220, 50, 7, 4, 220 + 15 * 6 + 13, 50), Some(6), "the first row's last");
        assert_eq!(grid_cell(220, 50, 7, 4, 222, 50 + 15 * 3 + 2), Some(21), "the last row's first");
        assert_eq!(grid_cell(220, 50, 7, 4, 220 + 15 * 7, 60), None, "right of the grid");
        assert_eq!(grid_cell(220, 50, 7, 4, 219, 60), None);
        assert_eq!(bag_tab(520, 33, 3, 200, 33 + 16 + 5), Some(0));
        assert_eq!(bag_tab(520, 33, 3, 200, 33 + 16 + 13 * 2 + 5), Some(2));
        assert_eq!(bag_tab(520, 33, 3, 200, 33 + 16 + 13 * 3 + 5), None, "below the third");
        assert_eq!(bag_tab(520, 33, 3, 215, 55), None, "the grid's side");
        assert_eq!(shop_tab(180, 33, 3, 205, 33 + 19), Some(0));
        assert_eq!(shop_tab(180, 33, 3, 205, 33 + 19 + 25), Some(2));
        assert_eq!(shop_tab(180, 33, 3, 205, 33 + 19 + 30), None);
        assert_eq!(shop_tab(180, 33, 3, 220, 60), None, "the grid");
        assert_eq!(popup_row(230, 90, 60, 3, 250, 92), Some(0));
        assert_eq!(popup_row(230, 90, 60, 3, 250, 92 + 14 * 2 + 13), Some(2));
        assert_eq!(popup_row(230, 90, 60, 3, 250, 92 + 14 * 3), None);
    }

    #[test]
    fn the_equipment_slots_and_the_option_values() {
        // Frame (180, 33): slots from (199, 57); the middle column 7 higher.
        assert_eq!(equip_slot(180, 33, 200, 58), Some(0));
        assert_eq!(equip_slot(180, 33, 215, 51), Some(1), "the middle column, 7 up");
        assert_eq!(equip_slot(180, 33, 230, 58), Some(2));
        assert_eq!(equip_slot(180, 33, 200, 73), Some(3));
        assert_eq!(equip_slot(180, 33, 216, 74), Some(4), "the middle column's second");
        assert_eq!(equip_slot(180, 33, 216, 82), Some(6), "under it");
        assert_eq!(equip_slot(180, 33, 260, 58), None, "the info box");
        assert_eq!((option_key(180, 200), option_key(180, 270), option_key(180, 330)), (OK, LEFT, RIGHT));
    }

    #[test]
    fn the_sound_question_has_its_answers_either_side_of_the_middle() {
        // 520 × 240: centre 260, middle 120 — the boxes at y 178..198.
        assert_eq!(sound_answer(260, 120, 210, 188), Some(true), "예: 185..235");
        assert_eq!(sound_answer(260, 120, 310, 188), Some(false), "아니오: 285..335");
        assert_eq!(sound_answer(260, 120, 260, 188), None, "between them");
        assert_eq!(sound_answer(260, 120, 210, 150), None, "the question");
    }

    #[test]
    fn the_stats_and_the_quest_arrows() {
        // Frame (180, 33): stats from (225, 92).
        assert_eq!(stat_cell(180, 33, 226, 93), Some(0), "힘");
        assert_eq!(stat_cell(180, 33, 280, 93), Some(1), "민첩");
        assert_eq!(stat_cell(180, 33, 226, 111), Some(2), "건강");
        assert_eq!(stat_cell(180, 33, 330, 125), Some(3), "의지");
        assert_eq!(stat_cell(180, 33, 226, 109), None, "between the rows");
        assert_eq!(stat_cell(180, 33, 340, 93), None, "past the values");
        assert_eq!(quest_arrow(180, 33, 210, 52), Some(LEFT));
        assert_eq!(quest_arrow(180, 33, 310, 52), Some(RIGHT));
        assert_eq!(quest_arrow(180, 33, 260, 52), None, "the page box");
        assert_eq!(quest_arrow(180, 33, 210, 100), None, "the quest's text");
    }

    /// Measured on the probe's dumps (520 × 240, frame (180, 33), 2026-10-25): the header box 212..308
    /// × 46..62, the cells 182..258 / 262..338 from y 72, 29 apart, the page bar 181..241 × 192..206.
    #[test]
    fn the_skill_window_has_tabs_cells_and_a_page_bar() {
        let spot = |pages: bool, count: i8, x: i32, y: i32| skill_spot(180, 33, 260, pages, count, x, y);
        assert_eq!(spot(false, 8, 212, 54), Some(SkillSpot::Tabs(LEFT)), "◀ of 액티브 스킬");
        assert_eq!(spot(false, 8, 305, 54), Some(SkillSpot::Tabs(RIGHT)));
        assert_eq!(spot(false, 8, 190, 78), Some(SkillSpot::Cell(0)));
        assert_eq!(spot(false, 8, 330, 78), Some(SkillSpot::Cell(1)));
        assert_eq!(spot(false, 8, 190, 107), Some(SkillSpot::Cell(2)), "강타");
        assert_eq!(spot(false, 8, 300, 166), Some(SkillSpot::Cell(7)), "the last row");
        assert_eq!(spot(false, 8, 190, 196), None, "no page bar on this tab");
        assert_eq!(spot(true, 8, 190, 198), Some(SkillSpot::Pages(LEFT)), "◀ 1/5 페이지");
        assert_eq!(spot(true, 8, 235, 198), Some(SkillSpot::Pages(RIGHT)));
        assert_eq!(spot(false, 2, 330, 78), Some(SkillSpot::Cell(1)), "직업 패시브: two cells");
        assert_eq!(spot(false, 2, 190, 107), None, "its other rows are decoration");
        assert_eq!(spot(false, 8, 260, 40), None, "the title band");
    }

    /// The probe's dump (frame (180, 33)): the cells from (255, 50), 15 apart.
    #[test]
    fn the_pick_list_is_a_four_by_three_grid_with_page_arrows() {
        assert_eq!(pick_spot(180, 33, false, 256, 51), Some(SkillSpot::Cell(0)));
        assert_eq!(pick_spot(180, 33, false, 301, 81), Some(SkillSpot::Cell(11)), "the last cell");
        assert_eq!(pick_spot(180, 33, false, 316, 51), None, "right of the grid");
        assert_eq!(pick_spot(180, 33, false, 250, 60), None, "no arrows on one page");
        assert_eq!(pick_spot(180, 33, true, 250, 60), Some(SkillSpot::Pages(LEFT)));
        assert_eq!(pick_spot(180, 33, true, 320, 60), Some(SkillSpot::Pages(RIGHT)));
        assert_eq!(pick_spot(180, 33, true, 240, 60), None, "장비's own slots");
    }

    /// The probe's dumps (frame (180, 33)): rows 58..76 / 82..100, digits 194..257 on 103..117, the
    /// button 276..303 × 104..118, the arrows at 187 and 310.
    #[test]
    fn the_money_window_has_rows_digits_a_button_and_arrows() {
        assert_eq!(money_spot(180, 33, 260, 60), Some(MoneySpot::Row(0)), "계정창고에 넣기");
        assert_eq!(money_spot(180, 33, 260, 90), Some(MoneySpot::Row(1)));
        assert_eq!(money_spot(180, 33, 195, 110), Some(MoneySpot::Digit(8)), "the highest digit, leftmost");
        assert_eq!(money_spot(180, 33, 255, 110), Some(MoneySpot::Digit(0)), "the units");
        assert_eq!(money_spot(180, 33, 290, 110), Some(MoneySpot::Confirm));
        assert_eq!(money_spot(180, 33, 187, 110), Some(MoneySpot::Arrow(LEFT)));
        assert_eq!(money_spot(180, 33, 312, 110), Some(MoneySpot::Arrow(RIGHT)));
        assert_eq!(money_spot(180, 33, 265, 110), None, "between the digits and the button");
        assert_eq!(money_spot(180, 33, 260, 150), None, "the balances");
    }

    /// The probe's dumps (520 × 240, frame (180, 33)): 전투스타일 선택's pictures 215..305 × 90..127
    /// and its bar 202..318 × 131..145; 도움말's header 181..339 × 46..60, its text 63..195.
    #[test]
    fn the_style_picker_and_the_text_windows() {
        assert_eq!(style_key(260, 33, 262, 110), Some(OK), "the middle picture");
        assert_eq!(style_key(260, 33, 205, 138), Some(LEFT), "◀ of the bar");
        assert_eq!(style_key(260, 33, 292, 100), Some(RIGHT), "the right picture");
        assert_eq!(style_key(260, 33, 260, 170), None, "the description");
        assert_eq!(text_key(180, 33, true, 185, 52), Some(LEFT), "◀ 기본 조작");
        assert_eq!(text_key(180, 33, true, 335, 52), Some(RIGHT));
        assert_eq!(text_key(180, 33, true, 260, 80), Some(UP));
        assert_eq!(text_key(180, 33, true, 260, 180), Some(DOWN));
        assert_eq!(text_key(180, 33, false, 185, 52), None, "게임정보 has no topics");
        assert_eq!(text_key(180, 33, true, 100, 100), None, "outside the window");
    }

    /// The probe's dump of 전직 (520 × 240, frame (180, 33), 2026-10-26): the 이후 picture 261..340 ×
    /// 62..108, the boxes 184..212 · 215..242 · 245..274 · 277..304 · 308..335 × 113..126, the name
    /// bar 202..317 × 133..146, the description from 147.
    #[test]
    fn the_job_page_has_boxes_a_name_bar_and_the_after_picture() {
        let boxes: Vec<_> = [197, 229, 260, 290, 322].iter().map(|&x| job_spot(180, 33, x, 120)).collect();
        assert_eq!(boxes, (0..5).map(|k| Some(JobSpot::Box(k))).collect::<Vec<_>>());
        assert_eq!(job_spot(180, 33, 300, 85), Some(JobSpot::After));
        assert_eq!(job_spot(180, 33, 220, 85), None, "이전 is the job the player has");
        assert_eq!(job_spot(180, 33, 215, 140), Some(JobSpot::Turn(LEFT)), "◀ 로그");
        assert_eq!(job_spot(180, 33, 300, 140), Some(JobSpot::Turn(RIGHT)), "로그 ▶");
        assert_eq!(job_spot(180, 33, 260, 170), None, "the description");
        assert_eq!(job_spot(180, 33, 260, 52), None, "the title's job line");
    }

    /// 전직's five boxes never show the player's own job and keep the cursor's in the middle.
    #[test]
    fn the_job_boxes_step_round_the_players_job() {
        assert_eq!(job_row(1, 0, 6), [4, 5, 1, 2, 3], "워리어 → 로그 first");
        assert_eq!(job_row(4, 3, 6), [1, 2, 4, 5, 0]);
        assert_eq!(job_row(0, 5, 6), [3, 4, 0, 1, 2], "the wrap past the last job");
        for current in 0..6 {
            for cursor in (0..6).filter(|&c| c != current) {
                let row = job_row(cursor, current, 6);
                assert_eq!(row[2], cursor);
                assert!(!row.contains(&current), "{row:?} shows job {current}");
            }
        }
        assert_eq!(job_row(0, 0, 1), [0; 5], "no other job: no endless step");
    }

    #[test]
    fn the_menu_rows_are_its_number_keys() {
        assert_eq!((0..10).map(|r| menu_digit(r, 10)).collect::<Vec<_>>(), vec![49, 50, 51, 52, 53, 54, 55, 56, 57, 48]);
        // HZ-026: 미니맵 gone — 1..8, then 0 for 시스템; '9' is no row.
        assert_eq!((0..9).map(|r| menu_digit(r, 9)).collect::<Vec<_>>(), vec![49, 50, 51, 52, 53, 54, 55, 56, 48]);
    }
}
