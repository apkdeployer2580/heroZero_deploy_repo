//! HZ-036: the player's bags of 112, a page of 28 at a time (user 2026-10-28: 28 → 112).
//!
//! The game's bag windows (`ds` 인벤토리, `eg` 계정창고's 캐릭터창고 side, `co` selling, `dv` / `et`
//! picking) draw a fixed 7 × 4 from item 0 and keep the cursor 0..27, going round at the ends
//! (`o.b(B)` / `o.a(B)`, javap 2026-10-28). The hooks add the page where a window reads a cell
//! ([`cell`]), turn the page when the cursor goes round ([`went_round`]) and draw ◀ n/4 ▶ under the
//! tabs ([`pager_at`]); a tap there turns it too (`menu_touch`). A bag is the player's when it is one
//! of `ed.a / b / c:Lbt;` (장비 · 조합 · 특수) — the shop's stock and the 계정창고 pages are not.

use std::sync::atomic::{AtomicU8, Ordering};

use hz_patches::hooks::{BAG_PAGE, BAG_PAGES};
use jvm::{ClassInstance, JavaValue, Jvm, Result as JvmResult};

/// The page each bag shows (장비 · 조합 · 특수). One emulator to a process (the app, the probe).
static PAGES: [AtomicU8; 3] = [const { AtomicU8::new(0) }; 3];

/// The page bag `bag` (0..3) shows.
pub fn page(bag: usize) -> u8 {
    PAGES[bag].load(Ordering::Relaxed)
}

/// Turn bag `bag` by `delta` pages, round the ends; returns the page now shown.
pub fn turn(bag: usize, delta: i8) -> u8 {
    let now = turned(page(bag), delta);
    PAGES[bag].store(now, Ordering::Relaxed);
    now
}

/// `page` turned by `delta`, round the ends.
pub fn turned(page: u8, delta: i8) -> u8 {
    (i32::from(page) + i32::from(delta)).rem_euclid(i32::from(BAG_PAGES)) as u8
}

/// Every bag back on its first page — no bag window is open.
pub fn reset() {
    for p in &PAGES {
        p.store(0, Ordering::Relaxed);
    }
}

/// The bag's place of cell `i` (0..27) on `page`.
pub fn cell(page: u8, i: i32) -> i32 {
    i32::from(page) * i32::from(BAG_PAGE) + i
}

/// Whether a grid move the game made with these keys went forward: `o.e(II)Z` (`vertical`) tests UP
/// (`'2'`, action 1) first and takes anything else as DOWN; `o.d(II)Z` LEFT (`'4'`, action 2) first,
/// else RIGHT.
pub fn forward(vertical: bool, action: i32, raw: i32) -> bool {
    if vertical { !(raw == 50 || action == 1) } else { !(raw == 52 || action == 2) }
}

/// The page a grid move turns: the game's cursor goes round at the ends (DOWN on the last row to the
/// first, RIGHT on 27 to 0, and back), and round is where the next page — or the last — begins.
/// +1, −1 or 0.
pub fn went_round(forward: bool, before: i8, after: i8) -> i8 {
    match (forward, after.cmp(&before)) {
        (true, std::cmp::Ordering::Less) => 1,
        (false, std::cmp::Ordering::Greater) => -1,
        _ => 0,
    }
}

/// ◀ n/4 ▶ under a bag window's tabs, `(x, y, w, h)` guest px: the tab column (`o.a(dd,BZIZ)V`: 11 px
/// squares at x `((W − 105) >> 1) − 12`, 13 apart from 16 below the frame — at most three for a bag)
/// leaves the column empty from 57 below the frame to the grid's foot at 77 (`q` and `bz` both draw
/// the grid from 17 down, 4 rows of 15).
pub fn pager_at(canvas_w: i32, frame_y: i32) -> (i32, i32, i32, i32) {
    let centre = ((canvas_w - 105) >> 1) - 12 + 5;
    (centre - PAGER_W / 2, frame_y + 57, PAGER_W, 20)
}

const PAGER_W: i32 = 34;

/// A tap on the pager: −1 its left half, +1 its right — a few px of slack round it.
pub fn pager_hit(canvas_w: i32, frame_y: i32, x: i32, y: i32) -> Option<i8> {
    const SLACK: i32 = 3;
    let (px, py, pw, ph) = pager_at(canvas_w, frame_y);
    if !(py - SLACK..py + ph + SLACK).contains(&y) || !(px - SLACK..px + pw + SLACK).contains(&x) {
        return None;
    }
    Some(if x < px + pw / 2 { -1 } else { 1 })
}

/// The page's label, `2/4`.
pub fn label(page: u8) -> String {
    format!("{}/{}", page + 1, BAG_PAGES)
}

/// The pager's ink: the game's dark brown on parchment (`q`'s 「소지금」, ldc 3550755).
pub const INK: i32 = 3_550_755;

/// HZ-066: what 계정창고's 「넣기」 knows of one storage page — room for the item (the game's
/// `bt.a(BBI)Z`: a free cell, or stacks of its kind with room for all of it) and whether the page
/// already holds that kind (`bt.a(BB)I` > 0; asked of stacking kinds only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageRoom {
    pub room: bool,
    pub holds_kind: bool,
}

/// HZ-066: the 계정창고 page a deposit goes into. The game numbered it like the character side's tab
/// (`dz.a[eg.g]`: 장비 page 1, 조합 page 2, 특수 page 3, page 4 never) while everything else treats the
/// storage as one run of pages — its tabs are alike, its load fills page after page, the gift and
/// HZ-065's rewards take the first with room. So: a page that holds the kind and has room (the
/// stack grows there rather than a second one starting), else the first page with room; `None` = no
/// room anywhere (the game's own refusal, 「창고의 공간이 부족합니다」).
pub fn deposit_page(pages: &[PageRoom]) -> Option<usize> {
    pages.iter().position(|p| p.room && p.holds_kind).or_else(|| pages.iter().position(|p| p.room))
}

/// Whether items of category `cat` stack (`bt.a(B)Z`: 16 and up — 조합재료 · 마법석 · 조합서 · 퀘스트 ·
/// 특수); equipment (0..15) is one item to a cell.
pub fn stacks(cat: i8) -> bool {
    cat >= 16
}

fn object(v: JavaValue) -> Option<Box<dyn ClassInstance>> {
    v.into()
}

/// Which of the player's bags `bag` is (0 장비 · 1 조합 · 2 특수), if it is one.
#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
pub async fn player_bag(jvm: &Jvm, bag: &Box<dyn ClassInstance>) -> JvmResult<Option<usize>> {
    for (i, name) in ["a", "b", "c"].into_iter().enumerate() {
        if let Some(b) = object(jvm.get_static_field("ed", name, "Lbt;").await?)
            && b.as_ref() == bag.as_ref()
        {
            return Ok(Some(i));
        }
    }
    Ok(None)
}

/// The bag a window's grid shows, as its painter hands it over: `ds` / `eg` their `a:Lbt;`, `dv` /
/// `et` theirs while they pick from it (`e:Z`), the shop `co.a:[Lbt;`'s second while it sells (`co.h`
/// 3 and up). Any other window has none.
#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
pub async fn shown_bag(jvm: &Jvm, w: &Box<dyn ClassInstance>) -> JvmResult<Option<Box<dyn ClassInstance>>> {
    let class = w.class_definition().name();
    Ok(match class.as_str() {
        "ds" | "eg" => object(jvm.get_field_from(w, &class, "a", "Lbt;").await?),
        "dv" | "et" => {
            if jvm.get_field_from::<bool>(w, &class, "e", "Z").await? {
                object(jvm.get_field_from(w, &class, "a", "Lbt;").await?)
            } else {
                None
            }
        }
        "co" => {
            if jvm.get_field_from::<i8>(w, "co", "h", "B").await? < 3 {
                None
            } else {
                match object(jvm.get_static_field("co", "a", "[Lbt;").await?) {
                    Some(bags) => jvm.load_array::<JavaValue>(&bags, 1, 1).await?.into_iter().next().and_then(object),
                    None => None,
                }
            }
        }
        _ => None,
    })
}

/// The player's bag a window shows, if it shows one (0 장비 · 1 조합 · 2 특수).
#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
pub async fn window_bag(jvm: &Jvm, w: &Box<dyn ClassInstance>) -> JvmResult<Option<usize>> {
    match shown_bag(jvm, w).await? {
        Some(bag) => player_bag(jvm, &bag).await,
        None => Ok(None),
    }
}

/// Draw bag `bag`'s pager for the window at frame top `frame_y`: the game's ◀ ▶ (`o.a(Ldd;IIIIZ)V`)
/// and the page in its font (`ah.a(Ldd;II[BII)V`, centred).
pub async fn draw_pager(jvm: &Jvm, dd: JavaValue, frame_y: i32, bag: usize) -> JvmResult<()> {
    let canvas_w: i32 = jvm.get_static_field("rpg/a", "a", "I").await?;
    let (x, y, w, h) = pager_at(canvas_w, frame_y);
    let arrows = [dd.clone(), JavaValue::Int(x), JavaValue::Int(y + h / 2), JavaValue::Int(w), JavaValue::Int(INK), JavaValue::Boolean(false)];
    let _: () = jvm.invoke_static("o", "a", "(Ldd;IIIIZ)V", arrows).await?;
    if let Some(g) = object(dd.clone()) {
        let _: () = jvm.invoke_virtual(&g, "a", "(I)V", (INK,)).await?;
    }
    let text = label(page(bag)).into_bytes();
    let mut bytes = jvm.instantiate_array("B", text.len()).await?;
    jvm.store_array(&mut bytes, 0, text.into_iter().map(|b| b as i8).collect::<Vec<_>>()).await?;
    let args = [dd, JavaValue::Int(x + w / 2), JavaValue::Int(y + LABEL_DY), JavaValue::Object(Some(bytes)), JavaValue::Int(0), JavaValue::Int(1)];
    let _: () = jvm.invoke_static("ah", "a", "(Ldd;II[BII)V", args).await?;
    Ok(())
}

/// The label's top below the pager's.
const LABEL_DY: i32 = 4;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_is_a_run_of_twenty_eight() {
        assert_eq!((cell(0, 0), cell(0, 27), cell(1, 0), cell(3, 27)), (0, 27, 28, 111));
        assert_eq!(cell(3, 27) + 1, i32::from(hz_patches::hooks::BAG_SIZE), "the last page ends the bag");
    }

    #[test]
    fn the_pages_go_round() {
        assert_eq!((turned(0, 1), turned(3, 1), turned(0, -1), turned(2, -1)), (1, 0, 3, 1));
        assert_eq!((label(0), label(3)), ("1/4".to_string(), "4/4".to_string()));
    }

    /// `o.b(B)` / `o.a(B)` on a 7 × 4 of 28: what each key does to the cursor, and the page that follows.
    #[test]
    fn a_cursor_gone_round_turns_the_page() {
        // DOWN: 22 → 1 (the last row's second to the first row's) — the next page; 3 → 10 stays.
        assert_eq!(went_round(forward(true, 6, 0), 22, 1), 1);
        assert_eq!(went_round(forward(true, 6, 0), 3, 10), 0);
        // UP: 1 → 22 the last page; 10 → 3 stays. The keypad's '2' / '8' the same.
        assert_eq!(went_round(forward(true, 1, 0), 1, 22), -1);
        assert_eq!(went_round(forward(true, 0, 50), 10, 3), 0);
        assert_eq!(went_round(forward(true, 0, 56), 27, 6), 1);
        // RIGHT: 27 → 0 next; LEFT: 0 → 27 last; one step either way stays.
        assert_eq!(went_round(forward(false, 5, 0), 27, 0), 1);
        assert_eq!(went_round(forward(false, 2, 0), 0, 27), -1);
        assert_eq!(went_round(forward(false, 0, 54), 4, 5), 0);
        assert_eq!(went_round(forward(false, 0, 52), 5, 4), 0);
        // A window of one cell does not move at all.
        assert_eq!(went_round(true, 0, 0), 0);
    }

    /// HZ-066: a deposit goes to the first page with room — equipment past a full page 1 to page 2
    /// (the game refused it there), a stack to the page that has its kind.
    #[test]
    fn a_deposit_takes_the_first_page_with_room() {
        let free = PageRoom { room: true, holds_kind: false };
        let full = PageRoom { room: false, holds_kind: false };
        let kind = PageRoom { room: true, holds_kind: true };
        let full_kind = PageRoom { room: false, holds_kind: true };
        assert_eq!(deposit_page(&[free, free, free, free]), Some(0), "an empty storage: page 1");
        assert_eq!(deposit_page(&[full, free, free, free]), Some(1), "page 1 full: page 2 (the reported refusal)");
        assert_eq!(deposit_page(&[full, full, full, free]), Some(3), "page 4 is reachable");
        assert_eq!(deposit_page(&[full; 4]), None, "nowhere: the game's refusal");
        assert_eq!(deposit_page(&[free, free, kind, free]), Some(2), "a stack joins its kind");
        assert_eq!(deposit_page(&[free, full_kind, free, free]), Some(0), "its kind's page out of room: the first with room");
        assert_eq!(deposit_page(&[]), None);
        assert!(stacks(16) && stacks(20) && !stacks(0) && !stacks(15), "bt.a(B)Z");
    }

    /// The pager sits under the tab column, clear of the three tabs and of the grid.
    #[test]
    fn the_pager_is_under_the_tabs() {
        for w in [240, 320, 427, 492, 496, 520] {
            let frame_x = (w - 160) >> 1;
            let frame_y = (240 - 174) >> 1;
            let (x, y, pw, h) = pager_at(w, frame_y);
            let tabs_x = ((w - 105) >> 1) - 12;
            assert!(x > frame_x && x + pw <= frame_x + 40, "{w}: inside the column left of the grid ({x}..{})", x + pw);
            assert!(x <= tabs_x && x + pw >= tabs_x + 11, "{w}: centred on the tabs");
            assert!(y >= frame_y + 16 + 13 * 3, "{w}: under the third tab");
            assert!(y + h <= frame_y + 17 + 60 + 1, "{w}: no lower than the grid's foot");
            assert_eq!(pager_hit(w, frame_y, x + 2, y + 10), Some(-1));
            assert_eq!(pager_hit(w, frame_y, x + pw - 2, y + 10), Some(1));
            assert_eq!(pager_hit(w, frame_y, x + 2, y - 20), None, "{w}: a tab above is not the pager");
        }
    }
}
