//! 「일괄판매」 — HZ-021, hero2's bulk sale (user 2026-10-25: 「hero2 양식 그대로」, hero2's
//! HERO2_GUEST_RE.md §8.85). In the shop's sell window a button opens the game's list popup of the
//! grades; the grade chosen is sold at once — 에픽 and 세트 after the game's 예/아니오 with the
//! cursor on 아니오 — equipment only, never what is worn, exactly that grade, at the game's own
//! price (the shop's `price / 10`), paid through the game's purse before the items leave.
//!
//! The items are taken out by OBJECT (`Vector.removeElement`), not by the game's `bt.b(cat, id, n)`,
//! which walks the bag from its end and takes the last unworn one of a kind — another copy, maybe of
//! another grade, would go. The shop's own one-item sale did exactly that (HZ-062, [`chosen_first`]).

use std::sync::atomic::{AtomicI8, Ordering};

use hz_patches::hooks;
use jvm::{ClassInstance, JavaValue, Jvm, Result as JvmResult};

/// The grades' names in `t/it.tdf` (`rpg/e.g`): 일반 레어 에픽 세트, `bx.e` 0..3.
pub const GRADE_NAMES: i32 = 28;
pub const GRADES: i8 = 4;
/// From 에픽 up the game asks first (hero2's `ASK_FROM`).
pub const ASK_FROM: i8 = 2;

/// 일괄판매 — the button.
pub const LABEL: &[u8] = &[0xc0, 0xcf, 0xb0, 0xfd, 0xc6, 0xc7, 0xb8, 0xc5];
/// 판매할 아이템이 없습니다 — that grade has nothing to sell.
pub const NOTHING: &[u8] = &[
    0xc6, 0xc7, 0xb8, 0xc5, 0xc7, 0xd2, 0x20, 0xbe, 0xc6, 0xc0, 0xcc, 0xc5, 0xdb, 0xc0, 0xcc, 0x20, 0xbe, 0xf8, 0xbd, 0xc0, 0xb4,
    0xcf, 0xb4, 0xd9,
];
/// 높은 가치의 아이템;등급이 선택되었습니다.;정말 판매하시겠습니까? — hero2's three lines, `;` the game
/// font's line break.
pub const ASK: &[u8] = &[
    0xb3, 0xf4, 0xc0, 0xba, 0x20, 0xb0, 0xa1, 0xc4, 0xa1, 0xc0, 0xc7, 0x20, 0xbe, 0xc6, 0xc0, 0xcc, 0xc5, 0xdb, b';', 0xb5, 0xee,
    0xb1, 0xde, 0xc0, 0xcc, 0x20, 0xbc, 0xb1, 0xc5, 0xc3, 0xb5, 0xc7, 0xbe, 0xfa, 0xbd, 0xc0, 0xb4, 0xcf, 0xb4, 0xd9, 0x2e, b';',
    0xc1, 0xa4, 0xb8, 0xbb, 0x20, 0xc6, 0xc7, 0xb8, 0xc5, 0xc7, 0xcf, 0xbd, 0xc3, 0xb0, 0xda, 0xbd, 0xc0, 0xb4, 0xcf, 0xb1, 0xee,
    0x3f,
];
/// The shop's own 판매하였습니다 (`t/ni.tdf` [22], the shop's `co.b`).
const SOLD: i32 = 22;
/// How long the shop shows a message (`bz.f`, as its own sale sets it).
const MESSAGE_TICKS: i8 = 15;

/// The grade between the list and the question (−1 = none).
static PENDING: AtomicI8 = AtomicI8::new(-1);

/// An equipment item as the sale sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece {
    pub grade: i8,
    pub worn: bool,
    /// `ba.a:I`, the shop's buying price; it buys back at a tenth.
    pub price: i32,
    pub count: i32,
}

/// Which of the bag's places go, and what they fetch: equipment of exactly `grade`, not worn.
pub fn plan(bag: &[Option<Piece>], grade: i8) -> (Vec<usize>, i64) {
    let mut sold = Vec::new();
    let mut total = 0i64;
    for (i, p) in bag.iter().enumerate() {
        if let Some(p) = p
            && p.grade == grade
            && !p.worn
        {
            sold.push(i);
            total += i64::from(p.price.max(0) / 10) * i64::from(p.count.max(1));
        }
    }
    (sold, total)
}

/// HZ-062: a sale of `n` that begins with the item the cursor chose, which holds `held` — what it gives
/// and what is left for the game's own take (a stack sold past it). Never more than it holds, never
/// below nothing.
pub fn chosen_first(held: i32, n: i32) -> (i32, i32) {
    let n = n.max(0);
    let take = held.clamp(0, n);
    (take, n - take)
}

fn object(v: JavaValue) -> Option<Box<dyn ClassInstance>> {
    v.into()
}

async fn bytes_array(jvm: &Jvm, bytes: &[u8]) -> JvmResult<Box<dyn ClassInstance>> {
    let mut array = jvm.instantiate_array("B", bytes.len()).await?;
    jvm.store_array(&mut array, 0, bytes.iter().map(|&b| b as i8).collect::<Vec<_>>()).await?;
    Ok(array)
}

/// Whether the shop `co` is selling with nothing open over it — the button is there.
#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
pub async fn button_shown(jvm: &Jvm, co: &Box<dyn ClassInstance>) -> JvmResult<bool> {
    let selling = jvm.get_field_from::<i8>(co, "co", "h", "B").await? >= 3;
    let up = jvm.get_field_from::<i8>(co, "co", "g", "B").await? != 0;
    let child = object(jvm.get_field_from(co, "o", "b", "Lo;").await?);
    Ok(selling && up && child.is_none())
}

/// The button's rectangle on the canvas for a window frame at `(x, y)`.
pub fn button_at(frame_x: i32, frame_y: i32) -> (i32, i32, i32, i32) {
    let (dx, dy, w, h) = hooks::BULK_BUTTON;
    (frame_x + dx, frame_y + dy, w, h)
}

/// The button tapped: the game's list popup `ee` of the four grades as the shop's child, tagged.
pub async fn open(jvm: &Jvm, co: &mut Box<dyn ClassInstance>) -> JvmResult<()> {
    let Some(names) = object(jvm.get_static_field("rpg/e", "g", "Lcx;").await?) else {
        return Ok(());
    };
    let mut rows = jvm.instantiate_array("[B", GRADES as usize).await?;
    let mut list = Vec::new();
    for g in 0..i32::from(GRADES) {
        let name: Box<dyn ClassInstance> = jvm.invoke_virtual(&names, "a", "(I)[B", (GRADE_NAMES + g,)).await?;
        list.push(JavaValue::Object(Some(name)));
    }
    jvm.store_array(&mut rows, 0, list).await?;
    let args = [JavaValue::Object(Some(co.clone())), JavaValue::Object(Some(rows)), JavaValue::Int(0)];
    let mut popup = jvm.new_class("ee", "(Lo;[[BI)V", args).await?;
    jvm.put_field_from(&mut popup, "ey", "f", "B", hooks::BULK_GRADE).await?;
    jvm.put_field_from(co, "o", "b", "Lo;", JavaValue::Object(Some(popup))).await?;
    jvm.put_field_from(co, "o", "a", "Z", true).await?;
    PENDING.store(-1, Ordering::Relaxed);
    tracing::info!("hz: bulk sell: the grades");
    Ok(())
}

/// A grade chosen in the list (its window already gone).
#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
pub async fn chose(jvm: &Jvm, co: &mut Box<dyn ClassInstance>, grade: i8) -> JvmResult<()> {
    if !(0..GRADES).contains(&grade) {
        return Ok(());
    }
    let (_, sold, _) = read_bag(jvm, grade).await?;
    if sold.is_empty() {
        return message(jvm, co, NOTHING).await;
    }
    if grade < ASK_FROM {
        return sell(jvm, co, grade).await;
    }
    PENDING.store(grade, Ordering::Relaxed);
    let text = bytes_array(jvm, ASK).await?;
    // f = 4: the game's question starts on 아니오 (cf's ctor); then the sale's own tag.
    let args = [JavaValue::Object(Some(co.clone())), JavaValue::Object(Some(text)), JavaValue::Byte(4), JavaValue::Boolean(true)];
    let mut ask = jvm.new_class("cf", "(Lo;[BBZ)V", args).await?;
    jvm.put_field_from(&mut ask, "ey", "f", "B", hooks::BULK_ASK).await?;
    jvm.put_field_from(co, "o", "b", "Lo;", JavaValue::Object(Some(ask))).await?;
    jvm.put_field_from(co, "o", "a", "Z", true).await?;
    tracing::info!("hz: bulk sell: grade {grade} - asking first");
    Ok(())
}

/// The question answered (its window gone): 예 (row 0) sells the grade asked about.
pub async fn confirmed(jvm: &Jvm, co: &mut Box<dyn ClassInstance>, row: i8) -> JvmResult<()> {
    let grade = PENDING.swap(-1, Ordering::Relaxed);
    if row == 0 && grade >= 0 {
        sell(jvm, co, grade).await?;
    } else {
        tracing::info!("hz: bulk sell: 아니오 - nothing sold");
    }
    Ok(())
}

/// The equipment bag's Vector, its items as [`Piece`]s, and the places of `grade` that go.
async fn read_bag(jvm: &Jvm, grade: i8) -> JvmResult<(Option<Box<dyn ClassInstance>>, Vec<Box<dyn ClassInstance>>, i64)> {
    let Some(bag) = object(jvm.invoke_static("ed", "a", "(B)Lbt;", [JavaValue::Byte(0)]).await?) else {
        return Ok((None, Vec::new(), 0));
    };
    let Some(items) = object(jvm.get_field(&bag, "a", "Ljava/util/Vector;").await?) else {
        return Ok((None, Vec::new(), 0));
    };
    let n: i32 = jvm.invoke_virtual(&items, "size", "()I", ()).await?;
    let mut objects = Vec::new();
    let mut pieces = Vec::new();
    for i in 0..n {
        let item = object(jvm.invoke_virtual(&items, "elementAt", "(I)Ljava/lang/Object;", (i,)).await?);
        let piece = match &item {
            Some(it) if jvm.is_instance(&**it, "bx") => Some(Piece {
                grade: jvm.get_field_from(it, "bx", "e", "B").await?,
                worn: jvm.get_field_from::<i8>(it, "bx", "i", "B").await? >= 0,
                price: jvm.get_field_from(it, "ba", "a", "I").await?,
                count: i32::from(jvm.get_field_from::<i8>(it, "ba", "c", "B").await?),
            }),
            _ => None,
        };
        pieces.push(piece);
        objects.push(item);
    }
    let (places, total) = plan(&pieces, grade);
    let sold = places.into_iter().filter_map(|i| objects[i].clone()).collect();
    Ok((Some(items), sold, total))
}

/// Pay first (the game's `ed.e(I)V`), then take exactly those items out, then the shop's own
/// refresh and 판매하였습니다.
#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
async fn sell(jvm: &Jvm, co: &mut Box<dyn ClassInstance>, grade: i8) -> JvmResult<()> {
    let (items, sold, total) = read_bag(jvm, grade).await?;
    let Some(items) = items.filter(|_| !sold.is_empty()) else {
        return message(jvm, co, NOTHING).await;
    };
    let _: () = jvm.invoke_static("ed", "e", "(I)V", [JavaValue::Int(total.min(i64::from(i32::MAX)) as i32)]).await?;
    let mut gone = 0;
    for item in &sold {
        let removed: bool = jvm.invoke_virtual(&items, "removeElement", "(Ljava/lang/Object;)Z", (item.clone(),)).await?;
        gone += usize::from(removed);
    }
    let _: () = jvm.invoke_virtual(co, "d", "()V", ()).await?;
    let table = object(jvm.get_field_from(co, "co", "b", "Lcx;").await?);
    let sold_text: Option<Box<dyn ClassInstance>> = match table {
        Some(t) => Some(jvm.invoke_virtual(&t, "a", "(I)[B", (SOLD,)).await?),
        None => None,
    };
    if let Some(text) = sold_text {
        jvm.put_field_from(co, "bz", "a", "[B", JavaValue::Object(Some(text))).await?;
        jvm.put_field_from(co, "bz", "f", "B", MESSAGE_TICKS).await?;
    }
    tracing::info!("hz: bulk sell: grade {grade} - {gone} of {} sold for {total}", sold.len());
    Ok(())
}

#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
async fn message(jvm: &Jvm, co: &mut Box<dyn ClassInstance>, text: &[u8]) -> JvmResult<()> {
    let text = bytes_array(jvm, text).await?;
    jvm.put_field_from(co, "bz", "a", "[B", JavaValue::Object(Some(text))).await?;
    jvm.put_field_from(co, "bz", "f", "B", MESSAGE_TICKS).await?;
    jvm.put_field_from(co, "o", "a", "Z", true).await?;
    tracing::info!("hz: bulk sell: nothing of that grade");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(grade: i8, worn: bool, price: i32) -> Option<Piece> {
        Some(Piece { grade, worn, price, count: 1 })
    }

    /// HZ-062: one piece of equipment is the chosen one alone; a stack gives what it holds and the game
    /// takes the rest.
    #[test]
    fn a_sale_begins_with_the_chosen_item() {
        assert_eq!(chosen_first(1, 1), (1, 0), "a piece of equipment");
        assert_eq!(chosen_first(40, 7), (7, 0), "part of a stack");
        assert_eq!(chosen_first(5, 9), (5, 4), "past it: the game's take for the rest");
        assert_eq!((chosen_first(0, 2), chosen_first(3, 0), chosen_first(3, -1)), ((0, 2), (0, 0), (0, 0)));
    }

    #[test]
    fn exactly_the_grade_not_worn_at_a_tenth() {
        let bag = [p(0, false, 900), p(1, false, 5000), p(0, true, 900), None, p(0, false, 1234), p(2, false, 90_000)];
        assert_eq!(plan(&bag, 0), (vec![0, 4], 90 + 123), "the worn one stays");
        assert_eq!(plan(&bag, 1), (vec![1], 500));
        assert_eq!(plan(&bag, 3), (vec![], 0));
        assert_eq!(plan(&[Some(Piece { grade: 1, worn: false, price: 100, count: 3 })], 1).1, 30, "a stack");
    }

    #[test]
    fn the_texts_are_the_game_fonts() {
        assert_eq!(crate::euc_kr("일괄판매").unwrap(), LABEL);
        assert_eq!(crate::euc_kr("판매할 아이템이 없습니다").unwrap(), NOTHING);
        assert_eq!(crate::euc_kr("높은 가치의 아이템;등급이 선택되었습니다.;정말 판매하시겠습니까?").unwrap(), ASK);
    }
}
