//! `net/wie/hz/Hooks` — the host class the `hz_patches` hook patches call (`hz_patches::hooks`
//! is the contract; [`proto`] builds the class from that list).
//!
//! Every hook returns exactly what the replaced instruction would have when its option is at
//! the default, so "patch on, option default" plays like the original game. They run on the
//! game's own thread, inside the guest call that hit them, so guest objects cannot change
//! under them. Guest state is read by field NAME and descriptor (`o.b:B`, `g.b:Ljava/util/
//! Vector;`) — the names are the jar's, pinned by the class md5 the patch was gated on.

use std::sync::Arc;

use java_class_proto::{JavaFieldProto, JavaMethodProto, MethodBody};
use java_constants::{FieldAccessFlags, MethodAccessFlags};
use jvm::{ClassInstance, JavaError, JavaValue, Jvm, Result as JvmResult, runtime::JavaLangString};
use wie_jvm_support::{WieJavaClassProto, WieJvmContext};

use hz_patches::hooks::{self, Hook};

use crate::{
    balance, boss, drops, effects, elixir, gear, guardian_mode, input, legend, mapfile,
    options::{GameOptions, Row},
    portrait, quickpage, revive, reward, statname, stats, story,
    state::{Buff, BuffKind, GuestState, Icon, IconKey, MapGrid, MapMarks, POTION_COOLDOWN, PlateAsk, QuestGuide, QuickBar, QuickSlot, SaveAnswer, Scene, Stats, buff_secs, mark_of},
    tables, text,
};

/// Auto-loot reach, in 16-px tiles (Chebyshev distance from the player's tile).
pub const AUTO_LOOT_RADIUS: i32 = 3;

/// A row of the one 환경설정 page (HZ-007, user 2026-10-29: *"추가설정에 있는거 따로 추가설정으로
/// 빼지않고 환경설정 한페이지에서 스크롤로"*): one of the game's own, by its number in the original
/// window (`bn.a(II)Z`'s switch on `o.a:B`, `bn.c(B)V`), or one of the host's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptRow {
    Game(u8),
    Host(Row),
}

/// The page, top to bottom: the game's rows — 배경음 효과음 게임속도 시선이동 진동, then 퀘스트 도움말;
/// its row 5 *키 단축키 (`rpg/c.d:B`, what the field's `*` opens: the minimap window or the quick
/// save) is gone, the user's 「*키 단축키 제거」 — the MAP and SAVE tabs do both — then the host's.
pub const OPT_ROWS: [OptRow; 13] = [
    OptRow::Game(0),
    OptRow::Game(1),
    OptRow::Game(2),
    OptRow::Game(3),
    OptRow::Game(4),
    OptRow::Game(6),
    OptRow::Host(Row::Orientation),
    OptRow::Host(Row::AutoLoot),
    OptRow::Host(Row::ExpRate),
    OptRow::Host(Row::DropRate),
    OptRow::Host(Row::Difficulty),
    OptRow::Host(Row::Respawn),
    OptRow::Host(Row::Control),
];

/// The rows the title's window shows at once: its seven rows and the line under them end where the
/// old 「추가 설정」 page's did, clear of the title art's bottom border on a 240-high canvas (rows from
/// `rpg/a.d − 32`, 15 apart).
pub const TITLE_ROWS: usize = 7;

/// The rows shown at once: in game the list frame's rows ([`hooks::OPTIONS_ROWS`] — its last slot
/// is the line under them), from the title [`TITLE_ROWS`].
pub fn visible_rows(in_game: bool) -> usize {
    if in_game { hooks::OPTIONS_ROWS as usize } else { TITLE_ROWS }
}

/// The scroll (the first row on screen) that shows `cursor`, moving `scroll` as little as it takes.
pub fn scroll_to(cursor: usize, scroll: usize, visible: usize) -> usize {
    let s = if cursor < scroll {
        cursor
    } else if cursor >= scroll + visible {
        cursor + 1 - visible
    } else {
        scroll
    };
    s.min(OPT_ROWS.len().saturating_sub(visible))
}

/// The game's text-table entries the page uses (`t/com.tdf`): row `r`'s label is `[16 + r]`, 켜기 /
/// 끄기 `[37]` / `[38]`, the speeds 느림 … 매우 빠름 `[23..=26]` (the title `[7]` is the game's own).
const FIRST_LABEL: i32 = 16;
const COM_ON: i32 = 37;
const COM_OFF: i32 = 38;
const COM_SPEED: i32 = 23;

const WHITE: i32 = 0xFF_FFFF;
/// The game's colour for a value that is not under the cursor (`bn.a(dd)`, ldc #1).
const DIM: i32 = 12_630_160;

/// The class, with one native static method per hook. `state` is where the HZ-008 blit hook
/// reports the scene, once a drawn frame.
pub fn proto(options: Arc<GameOptions>, state: Arc<GuestState>) -> WieJavaClassProto {
    let method = |hook: Hook, body: Box<dyn MethodBody<JavaError, WieJvmContext>>| JavaMethodProto {
        name: hook.name.into(),
        descriptor: hook.descriptor.into(),
        body,
        access_flags: MethodAccessFlags::STATIC,
    };
    // HZ-032: what the effect hooks share.
    let fx = Arc::new(Fx { state: state.clone(), live: Default::default() });
    // HZ-007: what its three hooks share.
    let page = Arc::new(OptPage { options: options.clone(), scroll: Default::default(), in_game_code: Default::default() });
    // HZ-057: what the reflect stance's hooks share, the field tick's and the blit's among them.
    let stance = Arc::new(Boss::new(state.clone()));
    // HZ-058: the plates, shared by the blit hook and the popups' rows.
    let quick = Arc::new(Quick::new(state.clone(), options.dir()));
    let duration = |kind| Box::new(Duration { kind });
    // HZ-033: a slot dropped in the drop roll under way (dropItem sets it, dropThreshold reads it).
    let slot_taken = Arc::new(std::sync::atomic::AtomicBool::new(false));
    WieJavaClassProto {
        name: hooks::CLASS,
        parent_class: Some("java/lang/Object"),
        interfaces: vec![],
        methods: vec![
            method(hooks::EXP, Box::new(Exp(options.clone()))),
            method(hooks::DROP_THRESHOLD, Box::new(DropThreshold { options: options.clone(), state: state.clone(), taken: slot_taken.clone() })),
            method(hooks::AUTO_LOOT, Box::new(AutoLoot(options.clone(), stance.clone()))),
            method(hooks::OPT_LABEL, Box::new(OptLabel(page.clone()))),
            method(hooks::OPT_VALUES, Box::new(OptValues(page.clone()))),
            method(hooks::OPT_KEY, Box::new(OptKey(page.clone()))),
            method(hooks::BLIT, Box::new(Blit(state.clone(), stance.clone(), quick.clone()))),
            method(hooks::CASH_FREE, Box::new(CashFree(options.clone()))),
            method(hooks::TEXT, Box::new(Text(state.clone()))),
            method(hooks::MENU_LABEL, Box::new(MenuLabel)),
            method(hooks::OPEN_STORAGE, Box::new(OpenStorage)),
            method(hooks::STORAGE_CLOSED, Box::new(StorageClosed)),
            method(hooks::TARGET_X, Box::new(TargetX(state.clone()))),
            method(hooks::TARGET_PICTURE, Box::new(TargetPicture(state.clone()))),
            method(hooks::TARGET_BAR, Box::new(TargetBar(state.clone()))),
            method(hooks::STORAGE_HELP, Box::new(StorageHelp)),
            method(hooks::ITEM_RECORD, Box::new(ItemRecord { orb: true, gear: false, legend: Legends::IfChosen(state.clone()) })),
            method(hooks::ITEM_ICON, Box::new(ItemIcon)),
            method(hooks::QUICK_ITEM, Box::new(QuickItem)),
            method(hooks::DEATH, Box::new(Death)),
            method(hooks::ANSWER, Box::new(Answer)),
            method(hooks::MONSTER_INIT, Box::new(MonsterInit(options.clone()))),
            method(hooks::RESPAWN_DELAY, Box::new(RespawnDelay(options.clone()))),
            method(hooks::NOTICE_BOTTOM, Box::new(NoticeBottom)),
            method(hooks::GAMEDATA_STORES, Box::new(SlotStores(state.clone(), "a", "gamedata"))),
            method(hooks::ITEM_STORES, Box::new(SlotStores(state.clone(), "b", "item"))),
            method(hooks::SLOT_STORES, Box::new(SlotStores(state.clone(), "c", "slot"))),
            method(hooks::SLOT_KEY, Box::new(SlotKey(state.clone()))),
            method(hooks::SLOT_TITLE, Box::new(SlotTitle)),
            method(hooks::SLOT_BOX, Box::new(SlotBox(state.clone()))),
            method(hooks::SLOT_LABEL, Box::new(SlotLabel(state.clone()))),
            method(hooks::SAVE_COUNT, Box::new(SaveCount(state.clone()))),
            method(hooks::SHOP_GOLD, Box::new(ShopGold)),
            method(hooks::LEVEL_UP, Box::new(LevelUp(state.clone()))),
            method(hooks::EXP_TABLE, Box::new(ExpTable(state.clone()))),
            method(hooks::STAT_NOW, Box::new(StatView(true))),
            method(hooks::STAT_MAX, Box::new(StatView(false))),
            method(hooks::GEAR_RECORD, Box::new(ItemRecord { orb: false, gear: true, legend: Legends::IfChosen(state.clone()) })),
            method(hooks::ORB_GEAR_RECORD, Box::new(ItemRecord { orb: true, gear: true, legend: Legends::IfChosen(state.clone()) })),
            method(hooks::RARE_DROP, Box::new(RareDrop(state.clone()))),
            method(hooks::PICKUP_NOTICE, Box::new(PickupNotice { state: state.clone(), last: Default::default() })),
            method(hooks::FILL_INTS, Box::new(FillInts)),
            method(hooks::MASK_ALPHA, Box::new(MaskAlpha)),
            method(hooks::GUARDIAN_MONSTER, Box::new(GuardianMonster)),
            method(hooks::MONSTER_ROUND, Box::new(MonsterRound)),
            method(hooks::QUEST_MONEY, Box::new(QuestMoney)),
            method(hooks::QUEST_EXP, Box::new(QuestExp)),
            method(hooks::ROUND_TEXT, Box::new(RoundText)),
            method(hooks::LEGEND_RECORD, Box::new(ItemRecord { orb: false, gear: false, legend: Legends::Always(state.clone()) })),
            method(hooks::GRADE_COLOR, Box::new(GradeColor)),
            method(hooks::GRADE_NAME, Box::new(GradeName)),
            method(hooks::ITEM_INFO, Box::new(ItemInfo)),
            method(hooks::MONSTER_DROP, Box::new(MonsterDrop { options: options.clone(), state: state.clone() })),
            method(hooks::HIT_DAMAGE, Box::new(HitDamage(fx.clone()))),
            method(hooks::CRIT_CHANCE, Box::new(CritChance(fx.clone()))),
            method(hooks::TAKE_DAMAGE, Box::new(TakeDamage(fx.clone()))),
            method(hooks::MONSTER_EXP, Box::new(MonsterExp(fx.clone()))),
            method(hooks::SKILL_SP, Box::new(SkillSp(fx.clone()))),
            method(hooks::CAST_START, Box::new(CastStart(fx.clone()))),
            method(hooks::JOB_COOLDOWN, Box::new(Cooldown { spirit: false, state: state.clone() })),
            method(hooks::SPIRIT_COOLDOWN, Box::new(Cooldown { spirit: true, state: state.clone() })),
            method(hooks::FUSION_START, duration(DurationKind::Fusion)),
            method(hooks::CLONE_LIFE, duration(DurationKind::Clone)),
            method(hooks::MARK_STATUS, duration(DurationKind::Mark)),
            method(hooks::COUNTER_STATUS, duration(DurationKind::Counter)),
            method(hooks::SPIRIT_COST, Box::new(Spirit { power: false, state: state.clone() })),
            method(hooks::SPIRIT_POWER, Box::new(Spirit { power: true, state: state.clone() })),
            method(hooks::COMMON_DROP, Box::new(CommonDrop)),
            method(hooks::DROP_ITEM, Box::new(DropItem { taken: slot_taken.clone() })),
            method(hooks::QUEST_ITEM, Box::new(QuestItem)),
            method(hooks::QUEST_GIVE, Box::new(QuestGive(state.clone()))),
            method(hooks::COUNTER_REFLECT, Box::new(CounterReflect)),
            method(hooks::MONSTER_HURT, Box::new(MonsterHurt)),
            method(hooks::STAT_NAMES, Box::new(StatNames)),
            method(hooks::SHIELD_BASH, Box::new(ShieldBash)),
            method(hooks::SMASH, Box::new(Smash)),
            method(hooks::FRENZY_COST, Box::new(FrenzyCost)),
            method(hooks::SNIPE_CHANCE, Box::new(Chance(legend::Effect::SnipeDouble))),
            method(hooks::POISON_CHANCE, Box::new(Chance(legend::Effect::PoisonBoost))),
            method(hooks::BLAST_CHANCE, Box::new(Chance(legend::Effect::BlastBoost))),
            method(hooks::POISON_DAMAGE, Box::new(PoisonDamage)),
            method(hooks::KNOCK, Box::new(Knock)),
            method(hooks::KNOCKABLE, Box::new(Knockable)),
            method(hooks::DRANK, Box::new(Drank(fx.clone()))),
            method(hooks::MOVE_STEP, Box::new(MoveStep(fx))),
            method(hooks::BOX_RECIPE, Box::new(BoxRecipe)),
            method(hooks::BAG_AT, Box::new(BagAt)),
            method(hooks::BAG_SIDEWAYS, Box::new(BagMove { vertical: false })),
            method(hooks::BAG_UPDOWN, Box::new(BagMove { vertical: true })),
            method(hooks::BAG_GRID, Box::new(BagGrid { bz: false })),
            method(hooks::BAG_GRID_BZ, Box::new(BagGrid { bz: true })),
            method(hooks::BAG_CELLS, Box::new(BagCells)),
            method(hooks::BAG_TAKE, Box::new(BagTake { some: false })),
            method(hooks::BAG_TAKE_SOME, Box::new(BagTake { some: true })),
            method(hooks::TABLE, Box::new(Table)),
            method(hooks::ATTACK, Box::new(Attack)),
            method(hooks::DEFENCE, Box::new(Defence)),
            method(hooks::HP_BONUS, Box::new(HpBonus)),
            method(hooks::SP_LEVEL, Box::new(SpLevel)),
            method(hooks::SPIRIT_CAST_END, Box::new(SpiritCastEnd)),
            method(hooks::QUICK_REGISTER, Box::new(QuickRegister(quick.clone()))),
            method(hooks::QUICK_POTION, Box::new(QuickPotion(quick.clone()))),
            method(hooks::CRIT, Box::new(Crit)),
            method(hooks::EVASION, Box::new(Evasion)),
            method(hooks::LEVEL_CAP, Box::new(LevelCap)),
            method(hooks::SKILL_POINTS, Box::new(SkillPoints)),
            method(hooks::BOSS_REFLECT, Box::new(BossReflect(state.clone()))),
            method(hooks::MONSTER_DRAW, Box::new(MonsterDraw(stance.clone()))),
            method(hooks::MONSTER_THINK, Box::new(MonsterThink(stance.clone()))),
            method(hooks::BOSS_HIT, Box::new(BossHit(stance.clone()))),
            method(hooks::GUARD, Box::new(Guard(state.clone()))),
            method(hooks::GUARD_STATUS, Box::new(GuardStatus)),
            method(hooks::WALK, Box::new(Walk(state.clone()))),
            method(hooks::CAMERA, Box::new(Camera)),
            method(hooks::FACE, Box::new(Face)),
            method(hooks::PASSABLE, Box::new(Passable)),
            method(hooks::SLIDE, Box::new(Slide)),
            method(hooks::DRAW_FACING, Box::new(DrawFacing)),
            method(hooks::POTION, Box::new(Potion(state.clone()))),
            method(hooks::KILL_COUNT, Box::new(KillCount(state.clone()))),
            method(hooks::SELL_TAKE, Box::new(SellTake)),
            method(hooks::SCRIPT_FLAG, Box::new(ScriptFlag)),
            method(hooks::SCRIPT_FLAG_ON, Box::new(ScriptFlagOn)),
            method(hooks::AIM_HOLD, Box::new(AimHold)),
            method(hooks::SLOT_SAVE, Box::new(SlotSave::new(state.clone(), options.dir()))),
            method(hooks::STORAGE_WRITE, Box::new(StorageWrite(state.clone()))),
            method(hooks::STORAGE_ROOM, Box::new(StorageRoom)),
            method(hooks::STORAGE_PUT, Box::new(StoragePut)),
        ],
        // HZ-016: the orb's picture, made once. A class static is a root the guest's collector
        // sees; a handle kept on the Rust side would not be one.
        // HZ-057: the pixels the stance reads around a boss and draws back, one array for good.
        fields: vec![
            JavaFieldProto::new(ORB_PICTURE, "Ljava/lang/Object;", FieldAccessFlags::STATIC),
            JavaFieldProto::new(ELIXIR_PICTURE, "Ljava/lang/Object;", FieldAccessFlags::STATIC),
            JavaFieldProto::new(STAT_PICTURE, "Ljava/lang/Object;", FieldAccessFlags::STATIC),
            JavaFieldProto::new(BOSS_PIXELS, "[I", FieldAccessFlags::STATIC),
        ],
        access_flags: Default::default(),
    }
}

// ---------------------------------------------------------------- pure parts ----

/// HZ-014: the target-monster panel's x on a canvas `width` wide — the game's `width − 80`, moved
/// left by `shift` but never off the canvas.
pub fn target_x(width: i32, shift: i32) -> i32 {
    let x = width - hooks::TARGET_PANEL_W;
    x - shift.clamp(0, x.max(0))
}

/// HZ-014, [`target_x`] kept inside the map picture `left..right` (canvas x). On a canvas wider
/// than 240 the game clips its field HUD to the map (`ce.e(dd)`: `setClip(g.d, g.e, min(g.a,
/// ce.g − g.d), …)`, javap 2026-10-24), and a map narrower than the canvas is drawn centred — so at
/// `width − 80` the panel fell in the black margin, cut or gone (S25, map 135: 320 px in 520).
/// A canvas of 240 or less clips to the whole canvas: no change there.
pub fn target_x_on_map(width: i32, shift: i32, left: i32, right: i32) -> i32 {
    let x = target_x(width, shift);
    if width <= 240 || right >= width {
        return x;
    }
    x.min(right - hooks::TARGET_PANEL_W).max(left.max(0))
}

/// A map tile's side in px (`g.a:S` = `g.c:B` tiles × 16).
pub const MAP_TILE: i32 = 16;

/// The narrowest map the target panel is ever drawn over: 20 tiles — the boss rooms 121..134, the
/// tutorial fight 135 and 89 (`hz_maps` over the 139 maps 2026-10-25: the 72 with monster kinds are
/// 20, 24, 25, 26, 30, 35, 36, 40, 44 and 50 tiles wide, and none is under 20 tiles tall, so on a
/// 240-high canvas the clip never reaches the panel's top).
pub const FIGHT_MAP_MIN_W: i32 = 20 * MAP_TILE;

/// The map picture's canvas span `left..right` for a map `map_w` px wide on a canvas `width` wide,
/// as `g.c()` places it (javap 2026-10-25: the scroll clamped to `width − g.a ..= 0`, then a map
/// narrower than the canvas centred at `(width − g.a) >> 1`) and `ce.e(dd)` clips to it. The hook
/// reads `g.d` itself; this is for the host, which knows the map's size (`MapGrid::w`) but not its
/// scroll. A map at least as wide as the canvas spans all of it.
pub fn map_span(width: i32, map_w: i32) -> (i32, i32) {
    if map_w < width {
        let left = (width - map_w) >> 1;
        (left, left + map_w)
    } else {
        (0, width)
    }
}

/// HZ-004: what `ek.j(I)V` adds — `|amount|` as the game does, times the rate, saturating.
pub fn exp(amount: i32, rate: u32) -> i32 {
    (amount.unsigned_abs() as u64 * rate as u64).min(i32::MAX as u64) as i32
}

/// HZ-005: an item slot's threshold for `R < threshold`, where `R` is in 0..65535.
pub fn drop_threshold(threshold: i32, rate: u32) -> i32 {
    (threshold.max(0) as i64 * rate as i64).min(65_535) as i32
}

/// HZ-019: where the pickup notices stand — the viewport's bottom `ce.h`, over the host's EXP bar
/// when the viewport is the whole canvas (HZ-013: the game's bar is gone and the host draws its
/// EXP bar there).
pub fn notice_bottom(bottom: i32, canvas_h: i32) -> i32 {
    if bottom >= canvas_h { bottom - hooks::NOTICE_LIFT } else { bottom }
}

/// HZ-017: a monster's max HP at `pct` percent (the field is an int).
pub fn scale_hp(hp: i32, pct: u32) -> i32 {
    (hp as i64 * pct as i64 / 100).clamp(1, i32::MAX as i64) as i32
}

/// HZ-017: a monster's attack at `pct` percent (the field is a short).
pub fn scale_attack(attack: i16, pct: u32) -> i16 {
    (attack as i64 * pct as i64 / 100).clamp(i16::MIN as i64, i16::MAX as i64) as i16
}

/// HZ-018: the death's respawn delay in field ticks at `rate`. A kind that never comes back (0)
/// or anything below stays as it is, and a positive delay stays at least one tick.
pub fn respawn_delay(delay: i16, rate: u32) -> i16 {
    if delay <= 0 || rate <= 1 { delay } else { (delay as u32 / rate).max(1) as i16 }
}

/// The keys of `bn.a(II)Z`: arg 1 is the game action, or −6/−7 for the soft keys; arg 2 is the
/// raw key code (javap of `o.f/g/h/i(II)Z` and `o.c(II)Z`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    Fire,
    Rsk,
    /// LSK and the rest of `o.f`/`o.g`: consumed, nothing happens (as in the game's rows).
    Other,
    /// Up / down — the game's own cursor movement (`o.c(II)Z`).
    Move,
}

pub fn classify(action: i32, raw: i32) -> Key {
    match (action, raw) {
        (-7, _) => Key::Rsk,
        (2, _) | (_, 52) => Key::Left,
        (5, _) | (_, 54) => Key::Right,
        (8, _) | (_, 53) => Key::Fire,
        (1, _) | (6, _) | (_, 50) | (_, 56) => Key::Move,
        _ => Key::Other,
    }
}

/// The value text of a host row.
fn value(options: &GameOptions, row: Row) -> Vec<u8> {
    match row {
        Row::Orientation => text::landscape(options.landscape()).to_vec(),
        Row::AutoLoot => text::on_off(options.auto_loot()).to_vec(),
        Row::ExpRate => text::times(options.exp_rate()),
        Row::DropRate => text::times(options.drop_rate()),
        Row::Difficulty => text::difficulty(options.difficulty_level()).to_vec(),
        Row::Respawn => text::times(options.respawn_rate()),
        Row::Control => text::control(options.control()).to_vec(),
    }
}

// --------------------------------------------------------------- JVM helpers ----

fn object(v: JavaValue) -> Option<Box<dyn ClassInstance>> {
    v.into()
}

fn int(v: JavaValue) -> i32 {
    v.into()
}

/// A `B` argument.
fn byte(v: JavaValue) -> i8 {
    v.into()
}

async fn byte_array(jvm: &Jvm, bytes: &[u8]) -> JvmResult<Box<dyn ClassInstance>> {
    let mut array = jvm.instantiate_array("B", bytes.len()).await?;
    jvm.store_array(&mut array, 0, bytes.iter().map(|&b| b as i8).collect::<Vec<_>>()).await?;
    Ok(array)
}

async fn null_pointer(jvm: &Jvm, what: &str) -> JavaError {
    jvm.exception("java/lang/NullPointerException", what).await
}

// --------------------------------------------------------------------- hooks ----

/// HZ-004 `exp(I)I`.
struct Exp(Arc<GameOptions>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for Exp {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let amount = int(args[0].clone());
        let rate = self.0.exp_rate();
        let mut out = exp(amount, rate);
        // HZ-022 keeps the EXP since 99 in a[2]: a gain never takes it past an int.
        if let Some(player) = object(jvm.get_static_field("ed", "a", "Lek;").await?)
            && let Some(a) = object(jvm.get_field_from(&player, "ek", "a", "[I").await?)
        {
            let now = jvm.load_array::<i32>(&a, 2, 1).await?[0];
            out = out.min(i32::MAX - now.max(0));
        }
        tracing::info!("hz: EXP +{} x{rate} = +{out}", amount.unsigned_abs());
        Ok(JavaValue::Int(out))
    }
}

/// HZ-005 `dropThreshold([BI)I`: the original read (`rpg/g.a([BI)I`), scaled.
struct DropThreshold {
    options: Arc<GameOptions>,
    state: Arc<GuestState>,
    /// HZ-033: a slot dropped in this roll (set by [`DropItem`]).
    taken: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for DropThreshold {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        use std::sync::atomic::Ordering::Relaxed;
        let offset = int(args[1].clone());
        let threshold: i32 = jvm.invoke_static("rpg/g", "a", "([BI)I", args.into_vec()).await?;
        // HZ-005's 「추가 설정」 rate (its hook), HZ-033's ×2 (which may have put the hook here alone).
        let rate = if self.state.patch_on("HZ-005") { self.options.drop_rate() } else { 1 };
        let boosted = if self.state.patch_on("HZ-033") { effects::scale(threshold, drops::SLOT_PCT) } else { threshold };
        let scaled = if rate == 1 { boosted.min(65_535) } else { drop_threshold(boosted, rate) };
        // HZ-033: past a slot's drop the loop goes on to the game's bands — the slots left are closed.
        let (out, fresh) = drops::slot_threshold(offset, scaled, self.taken.load(Relaxed));
        if fresh {
            self.taken.store(false, Relaxed);
        }
        tracing::debug!("hz: drop threshold {threshold} x{rate} = {out}");
        Ok(JavaValue::Int(out))
    }
}

/// HZ-034 `questItem(byte cat, byte id)Object`, in place of `ba.a(BB)Lba;` where a quest reward makes
/// its item (script action 16).
struct QuestItem;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for QuestItem {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let (cat, id): (i8, i8) = (args[0].clone().into(), args[1].clone().into());
        let (cat2, id2) = if replay_round(jvm).await? >= guardian_mode::ROUND { guardian_mode::quest_item(cat, id) } else { (cat, id) };
        let made: JavaValue = jvm.invoke_static("ba", "a", "(BB)Lba;", [JavaValue::Byte(cat2), JavaValue::Byte(id2)]).await?;
        if (cat2, id2) != (cat, id) {
            // A guardian piece only exists with HZ-023: otherwise the game's own reward.
            if object(made.clone()).is_none() {
                return jvm.invoke_static("ba", "a", "(BB)Lba;", [JavaValue::Byte(cat), JavaValue::Byte(id)]).await;
            }
            tracing::info!("hz: 가디언 quest item ({cat}, {id}) -> ({cat2}, {id2})");
        }
        Ok(made)
    }
}

/// HZ-034 / HZ-065 `questGive(Object bt, Object ba, int count)I`, in place of `bt.a(Lba;I)I` where a
/// quest reward goes into the bag (script action 16) — put there by either patch.
struct QuestGive(Arc<GuestState>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for QuestGive {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let args = args.into_vec();
        let (Some(bag), Some(item)) = (object(args[0].clone()), object(args[1].clone())) else {
            return Err(null_pointer(jvm, "Hooks.questGive").await);
        };
        let mut count = int(args[2].clone());
        let cat: i8 = jvm.get_field_from(&item, "ba", "a", "B").await?;
        // HZ-034: 가디언 모드's materials, stones and potions twice.
        if self.0.patch_on("HZ-034") && replay_round(jvm).await? >= guardian_mode::ROUND {
            let more = guardian_mode::quest_count(cat, count);
            if more != count {
                tracing::info!("hz: 가디언 quest count ({cat}) {count} -> {more}");
                count = more;
            }
        }
        let added: i32 = jvm.invoke_virtual(&bag, "a", "(Lba;I)I", [JavaValue::Object(Some(item.clone())), JavaValue::Int(count)]).await?;
        if added >= count || !self.0.patch_on("HZ-065") {
            return Ok(JavaValue::Int(added));
        }
        // HZ-065: the full bag's rest waits for the next save, which puts it into the 계정창고 with the
        // progress that earned it (`crate::reward`); the whole count answered, so the list goes on. No
        // room in the storage for it now: the game's failure as it was (the quest stays to be done).
        let id: i8 = jvm.get_field_from(&item, "ba", "b", "B").await?;
        let rest = count - added;
        if !storage_room(jvm, cat, id, rest).await? {
            tracing::info!("hz: quest reward ({cat},{id}) x{rest}: no room in the bag nor the 계정창고 - the game's failure");
            return Ok(JavaValue::Int(added));
        }
        self.0.reward_wait(cat, id, rest);
        tracing::info!("hz: quest reward ({cat},{id}) x{rest}: the bag is full - to the 계정창고 with the next save");
        Ok(JavaValue::Int(count))
    }
}

/// The 계정창고's pages (`dz.a:[Lbt;`, pages of 28 — 112 with HZ-037), read from `HOL.storage` first
/// (`dz.h()V`, as the storage chooser does) when they are not in memory. The probe's `--give` and
/// `--bags` read them through this too.
pub async fn storage_pages(jvm: &Jvm) -> JvmResult<Vec<Box<dyn ClassInstance>>> {
    let mut pages: Option<Box<dyn ClassInstance>> = jvm.get_static_field("dz", "a", "[Lbt;").await?;
    if pages.is_none() {
        let _: () = jvm.invoke_static("dz", "h", "()V", ()).await?;
        pages = jvm.get_static_field("dz", "a", "[Lbt;").await?;
    }
    let Some(pages) = pages else { return Ok(Vec::new()) };
    let len = jvm.array_length(&pages).await?;
    let list: Vec<Option<Box<dyn ClassInstance>>> = jvm.load_array(&pages, 0, len).await?;
    Ok(list.into_iter().flatten().collect())
}

/// HZ-065: the first 계정창고 page with room for `n` of (`cat`, `id`) — the bag's own room test
/// `bt.a(BBI)Z` (a free cell, or a stack with room) — by its index in `pages`.
async fn storage_page_with_room(jvm: &Jvm, pages: &[Box<dyn ClassInstance>], cat: i8, id: i8, n: i32) -> JvmResult<Option<usize>> {
    for (i, page) in pages.iter().enumerate() {
        let room: bool = jvm.invoke_virtual(page, "a", "(BBI)Z", [JavaValue::Byte(cat), JavaValue::Byte(id), JavaValue::Int(n)]).await?;
        if room {
            return Ok(Some(i));
        }
    }
    Ok(None)
}

async fn storage_room(jvm: &Jvm, cat: i8, id: i8, n: i32) -> JvmResult<bool> {
    let pages = storage_pages(jvm).await?;
    Ok(storage_page_with_room(jvm, &pages, cat, id, n).await?.is_some())
}

/// HZ-066: the 계정창고 page 「넣기」 puts `n` of (`cat`, `id`) into ([`crate::bags::deposit_page`]) in
/// place of `given`, the one the game numbered like the character side's tab — with the pages' places
/// of both (for the log). `given` itself when no page has room (the game's refusal follows) or when it
/// is not one of the storage's pages `dz.a:[Lbt;` (read as it is — the window loaded it).
async fn deposit_page(
    jvm: &Jvm,
    given: Box<dyn ClassInstance>,
    cat: i8,
    id: i8,
    n: i32,
) -> JvmResult<(Box<dyn ClassInstance>, Option<usize>, Option<usize>)> {
    let array: Option<Box<dyn ClassInstance>> = jvm.get_static_field("dz", "a", "[Lbt;").await?;
    let pages: Vec<Box<dyn ClassInstance>> = match array {
        Some(array) => {
            let len = jvm.array_length(&array).await?;
            jvm.load_array::<Option<Box<dyn ClassInstance>>>(&array, 0, len).await?.into_iter().flatten().collect()
        }
        None => Vec::new(),
    };
    let Some(was) = pages.iter().position(|p| p.as_ref() == given.as_ref()) else {
        return Ok((given, None, None));
    };
    let mut rooms = Vec::with_capacity(pages.len());
    for page in &pages {
        let room: bool = jvm.invoke_virtual(page, "a", "(BBI)Z", [JavaValue::Byte(cat), JavaValue::Byte(id), JavaValue::Int(n)]).await?;
        let holds_kind = crate::bags::stacks(cat) && jvm.invoke_virtual::<_, i32>(page, "a", "(BB)I", [JavaValue::Byte(cat), JavaValue::Byte(id)]).await? > 0;
        rooms.push(crate::bags::PageRoom { room, holds_kind });
    }
    match crate::bags::deposit_page(&rooms) {
        Some(at) => Ok((pages.into_iter().nth(at).expect("a page deposit_page found"), Some(at), Some(was))),
        None => Ok((given, Some(was), Some(was))),
    }
}

/// HZ-066 `storageRoom(Object page, byte cat, byte id, int n)Z`, in place of `bt.a(BBI)Z` where
/// 계정창고's 「넣기」 tests its page (`eg.a(BB)V` @95): the room in the page [`deposit_page`] picks.
struct StorageRoom;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for StorageRoom {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(given) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.storageRoom").await);
        };
        let (cat, id, n) = (byte(args[1].clone()), byte(args[2].clone()), int(args[3].clone()));
        let (page, _, _) = deposit_page(jvm, given, cat, id, n).await?;
        jvm.invoke_virtual(&page, "a", "(BBI)Z", [JavaValue::Byte(cat), JavaValue::Byte(id), JavaValue::Int(n)]).await
    }
}

/// HZ-066 `storagePut(Object page, Object item, int n)I`, in place of `bt.a(Lba;I)I` where 계정창고's
/// 「넣기」 adds (`eg.a(BB)V` @145): into the page [`deposit_page`] picks — the one the room test just
/// passed (nothing between the two writes the storage).
struct StoragePut;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for StoragePut {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let (Some(given), Some(item)) = (object(args[0].clone()), object(args[1].clone())) else {
            return Err(null_pointer(jvm, "Hooks.storagePut").await);
        };
        let n = int(args[2].clone());
        let cat: i8 = jvm.get_field_from(&item, "ba", "a", "B").await?;
        let id: i8 = jvm.get_field_from(&item, "ba", "b", "B").await?;
        let (page, at, was) = deposit_page(jvm, given, cat, id, n).await?;
        let took: i32 = jvm.invoke_virtual(&page, "a", "(Lba;I)I", [JavaValue::Object(Some(item)), JavaValue::Int(n)]).await?;
        let page_no = |p: Option<usize>| p.map_or_else(|| "?".to_string(), |p| (p + 1).to_string());
        tracing::info!("hz: storage deposit ({cat},{id}) x{n} -> page {} (the game's page {}), took {took}", page_no(at), page_no(was));
        Ok(JavaValue::Int(took))
    }
}

/// HZ-065 `slotSave(byte[] gamedata, byte slot)Z`, in place of the slot's progress record's write
/// `rpg/b.a([BB)Z` (`ed.k()V` @143 — the quests' states are in that record): the game's write, then,
/// with rewards waiting, the record read back (`rpg/b.a(B)[B`) — on the disk, the rewards go into
/// the 계정창고 and its record is written and read back too ([`put_rewards_away`]); refused, they wait
/// as they were (the progress is not saved either, the host's record layer says so). The rewards a
/// save went past are kept in [`reward::FILE`] under `dir` (none: the probe, memory only).
struct SlotSave {
    state: Arc<GuestState>,
    dir: Option<std::path::PathBuf>,
}

impl SlotSave {
    fn new(state: Arc<GuestState>, dir: Option<&std::path::Path>) -> SlotSave {
        if let Some(text) = dir.and_then(|d| std::fs::read_to_string(d.join(reward::FILE)).ok()) {
            let n = state.rewards_restore(&text);
            tracing::info!("hz: quest rewards owed from an earlier run: {n} ({})", reward::FILE);
        }
        SlotSave { state, dir: dir.map(std::path::Path::to_path_buf) }
    }

    /// The file, whole, through a temporary — or gone when nothing is owed.
    fn save_file(&self) {
        let Some(dir) = &self.dir else { return };
        let path = dir.join(reward::FILE);
        let done = match self.state.rewards_saved_text() {
            Some(text) => {
                let tmp = path.with_extension("tmp");
                std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, &path))
            }
            None => match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            },
        };
        if let Err(e) = done {
            tracing::warn!("hz: quest rewards file not written: {e}");
        }
    }
}

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for SlotSave {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let wrote: bool = jvm.invoke_static("rpg/b", "a", "([BB)Z", args.to_vec()).await?;
        let waiting = self.state.rewards_take();
        if waiting.is_empty() {
            return Ok(JavaValue::Boolean(wrote));
        }
        let slot = byte(args[1].clone());
        let progress = match object(args[0].clone()) {
            Some(data) => match bytes_of(jvm, &data).await {
                Ok(sent) => read_back(jvm, &sent, || jvm.invoke_static("rpg/b", "a", "(B)[B", (slot,))).await,
                Err(e) => Err(e),
            },
            None => Ok(false),
        };
        let (keep, say) = match progress {
            Ok(true) => put_rewards_away(jvm, &self.state, waiting).await,
            Ok(false) => {
                tracing::warn!("hz: quest rewards: the slot's progress record was not written (slot {slot}) - {} wait as they were", waiting.len());
                (waiting, Vec::new())
            }
            Err(e) => {
                tracing::warn!("hz: quest rewards: the slot's progress record could not be read back: {e:?} - {} wait as they were", waiting.len());
                (waiting, Vec::new())
            }
        };
        self.state.rewards_keep(keep);
        self.save_file();
        for n in say {
            self.state.reward_notice(n);
        }
        Ok(JavaValue::Boolean(wrote))
    }
}

/// A byte array's bytes.
#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
async fn bytes_of(jvm: &Jvm, array: &Box<dyn ClassInstance>) -> JvmResult<Vec<u8>> {
    let n = jvm.array_length(array).await?;
    Ok(jvm.load_array::<i8>(array, 0, n).await?.into_iter().map(|b| b as u8).collect())
}

/// HZ-065: does a record read back as `sent` — the game's reader (`rpg/b.a(String,[B)[B`: two bytes
/// of length, the XOR'd data, five of checksum; `null` when they do not agree or the record is not
/// there) handed the very payload its writer was given?
async fn read_back<F, R>(jvm: &Jvm, sent: &[u8], reader: F) -> JvmResult<bool>
where
    F: FnOnce() -> R,
    R: std::future::Future<Output = JvmResult<Option<Box<dyn ClassInstance>>>>,
{
    match reader().await? {
        Some(back) => Ok(bytes_of(jvm, &back).await? == sent),
        None => Ok(false),
    }
}

/// A 계정창고 page's cells: each item's identity and count (`ba.c:B`), `None` for an empty cell.
#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
async fn page_cells(jvm: &Jvm, page: &Box<dyn ClassInstance>) -> JvmResult<Vec<Option<(Box<dyn ClassInstance>, i8)>>> {
    let size: i32 = jvm.invoke_virtual(page, "a", "()I", ()).await?;
    let mut cells = Vec::with_capacity(size.max(0) as usize);
    for i in 0..size {
        match object(jvm.invoke_virtual(page, "a", "(I)Lba;", (i,)).await?) {
            Some(it) => {
                let count: i8 = jvm.get_field_from(&it, "ba", "c", "B").await?;
                cells.push(Some((it, count)));
            }
            None => cells.push(None),
        }
    }
    Ok(cells)
}

/// What a put into a page (`bt.a(Lba;I)I`) added, cell by cell as the page is now: the stacks it
/// merged into (so many more than before) and the cell it made (all of it) — the game merges into
/// the stacks of the kind first and puts only the rest into a cell of its own, inserted in order
/// (`bt.a(Vector,Object)`), so identity, not place, tells the cells apart. Taking exactly these back
/// (`bt.a(II)V`, highest cell first) leaves the page as it was.
type Cells = [Option<(Box<dyn ClassInstance>, i8)>];
fn cells_added(before: &Cells, after: &Cells) -> Vec<(i32, i32)> {
    after
        .iter()
        .enumerate()
        .filter_map(|(i, cell)| {
            let (it, now) = cell.as_ref()?;
            let was = before.iter().flatten().find(|(b, _)| b.as_ref() == it.as_ref()).map_or(0, |(_, c)| i32::from(*c));
            let more = i32::from(*now) - was;
            (more > 0).then_some((i as i32, more))
        })
        .collect()
}

/// HZ-065: a put taken back out of its page — each cell `n` fewer (`bt.a(II)V`: at 0 the cell leaves
/// the page, so the highest cell first and the ones before it keep their places).
#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
async fn take_back(jvm: &Jvm, page: &Box<dyn ClassInstance>, added: &[(i32, i32)]) -> JvmResult<()> {
    for &(cell, n) in added.iter().rev() {
        jvm.invoke_virtual::<_, ()>(page, "a", "(II)V", (cell, n)).await?;
    }
    Ok(())
}

/// A reward put into its page: how many the game took (`bt.a(Lba;I)I`), and the exact change.
async fn place_reward(jvm: &Jvm, pages: &[Box<dyn ClassInstance>], r: reward::Reward) -> JvmResult<(i32, Option<(usize, Vec<(i32, i32)>)>)> {
    let Some(at) = storage_page_with_room(jvm, pages, r.cat, r.id, r.count).await? else { return Ok((0, None)) };
    let Some(item) = object(jvm.invoke_static("ba", "a", "(BB)Lba;", [JavaValue::Byte(r.cat), JavaValue::Byte(r.id)]).await?) else {
        tracing::warn!("hz: quest reward ({},{}): no item record - it waits", r.cat, r.id);
        return Ok((0, None));
    };
    let page = &pages[at];
    let before = page_cells(jvm, page).await?;
    let added: i32 = jvm.invoke_virtual(page, "a", "(Lba;I)I", [JavaValue::Object(Some(item)), JavaValue::Int(r.count)]).await?;
    let change = cells_added(&before, &page_cells(jvm, page).await?);
    Ok((added, (added > 0).then_some((at, change))))
}

/// HZ-065, the storage's record: the game's own `dz.i()V` with the capture armed, its payload (what
/// `dz.i()V` @138 handed `rpg/b.b([B)Z`, [`StorageWrite`]) read back with the game's reader
/// `rpg/b.b()[B` — the game's writer answers `true` even when the record store refused the write, so
/// only the record itself can say. A failure before the write was reached is a write not made; one
/// after it (the write made, the read-back failed) is [`reward::Written::Unknown`].
async fn write_storage(jvm: &Jvm, state: &GuestState) -> reward::Written {
    state.storage_capture_arm(true);
    let wrote = jvm.invoke_static::<_, ()>("dz", "i", "()V", ()).await;
    let sent = state.storage_payload_take();
    state.storage_capture_arm(false);
    match (wrote, sent) {
        (Err(e), None) => {
            tracing::warn!("hz: quest rewards: the storage's write was not reached: {e:?}");
            reward::Written::No
        }
        (Ok(()), None) => {
            tracing::warn!("hz: quest rewards: dz.i() wrote nothing (HZ-065's dz site missing?)");
            reward::Written::No
        }
        (Err(e), Some(_)) => {
            tracing::warn!("hz: quest rewards: the storage's write was made, then: {e:?} - taken as written");
            reward::Written::Unknown
        }
        (Ok(()), Some(sent)) => match read_back(jvm, &sent, || jvm.invoke_static("rpg/b", "b", "()[B", ())).await {
            Ok(true) => reward::Written::Yes,
            Ok(false) => reward::Written::No,
            Err(e) => {
                tracing::warn!("hz: quest rewards: the storage record could not be read back: {e:?} - taken as written");
                reward::Written::Unknown
            }
        },
    }
}

/// HZ-065, a save whose progress record is on the disk: each waiting reward into the first 계정창고
/// page with room (what the page does not take — the game's count — waits), the storage record
/// written and read back ([`write_storage`]). Not written: exactly what this save added is taken back
/// out of the storage in memory ([`cells_added`] — the rest of it stays as it was; after an earlier
/// refused write it may hold what only it holds), so a later write never puts away a reward that is
/// still waiting. A failure while putting them in takes back what was put and keeps them all. Returns
/// what keeps waiting and what to say ([`reward::after_save`]).
async fn put_rewards_away(jvm: &Jvm, state: &GuestState, waiting: Vec<reward::Reward>) -> (Vec<reward::Reward>, Vec<reward::Notice>) {
    let everything_waits = |waiting: Vec<reward::Reward>| reward::after_save(waiting, Vec::new(), reward::Written::No);
    let pages = match storage_pages(jvm).await {
        Ok(pages) => pages,
        Err(e) => {
            tracing::warn!("hz: quest rewards: the 계정창고's pages could not be read: {e:?} - they wait");
            return everything_waits(waiting);
        }
    };
    let (mut placed, mut left) = (Vec::new(), Vec::new());
    // What went into which page, to take it back: the last put first.
    let mut changes: Vec<(usize, Vec<(i32, i32)>)> = Vec::new();
    let mut failed = None;
    for r in &waiting {
        match place_reward(jvm, &pages, *r).await {
            Ok((added, change)) => {
                changes.extend(change);
                if added > 0 {
                    placed.push(reward::Reward { count: added, ..*r });
                }
                if added < r.count {
                    left.push(reward::Reward { count: r.count - added, ..*r });
                }
            }
            Err(e) => {
                failed = Some(e);
                break;
            }
        }
    }
    let written = if let Some(e) = &failed {
        tracing::warn!("hz: quest rewards: putting them into the 계정창고 failed: {e:?} - they all wait");
        reward::Written::No
    } else if placed.is_empty() {
        reward::Written::Yes
    } else {
        write_storage(jvm, state).await
    };
    if written == reward::Written::No {
        for (at, change) in changes.iter().rev() {
            if let Err(e) = take_back(jvm, &pages[*at], change).await {
                tracing::warn!("hz: quest rewards: taking a put back out of page {} failed: {e:?}", at + 1);
            }
        }
        if failed.is_some() {
            return everything_waits(waiting);
        }
    }
    tracing::info!(
        "hz: quest rewards at the save: {} into the 계정창고 ({}), {} without room",
        placed.len(),
        match written {
            reward::Written::Yes => "written, read back",
            reward::Written::No => "NOT written - they wait",
            reward::Written::Unknown => "written, NOT read back",
        },
        left.len()
    );
    reward::after_save(placed, left, written)
}

/// HZ-065 `storageWrite(byte[] data)Z`, in place of `rpg/b.b([B)Z` where the 계정창고 is written
/// (`dz.i()V` @138): while a save's put-away has armed it, the payload kept for the read-back; then
/// the game's write. The storage's other writes (the player moving things at the counter) copy nothing.
struct StorageWrite(Arc<GuestState>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for StorageWrite {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        if self.0.storage_capture_armed()
            && let Some(data) = object(args[0].clone())
        {
            self.0.storage_payload_set(bytes_of(jvm, &data).await?);
        }
        jvm.invoke_static("rpg/b", "b", "([B)Z", args.into_vec()).await
    }
}

/// HZ-035 `boxRecipe(int n)I`, in place of `n / 4` (the random box's recipe number).
struct BoxRecipe;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for BoxRecipe {
    async fn call(&self, _: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        Ok(JavaValue::Int(legend::box_recipe(int(args[0].clone()))))
    }
}

/// HZ-036 `bagAt(Object bt, int cell)Object`, in place of `bt.a(I)Lba;` where a bag window reads a
/// cell: on one of the player's bags, the cell of the page shown.
struct BagAt;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for BagAt {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let (bag, cell) = (object(args[0].clone()), int(args[1].clone()));
        let Some(bag) = bag else {
            return Err(null_pointer(jvm, "Hooks.bagAt").await);
        };
        let at = bag_place(jvm, &bag, cell).await?;
        jvm.invoke_virtual(&bag, "a", "(I)Lba;", (at,)).await
    }
}

/// HZ-036: the place in `bag` of a window's cell 0..27 — on one of the player's bags, on the page shown;
/// any other bag (a 계정창고 page, the shop's stock) as it was. [`BagAt`] reads there and [`BagTake`]
/// takes from there, so the item a window chose is the item that leaves.
#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
async fn bag_place(jvm: &Jvm, bag: &Box<dyn ClassInstance>, cell: i32) -> JvmResult<i32> {
    Ok(match crate::bags::player_bag(jvm, bag).await? {
        Some(b) => crate::bags::cell(crate::bags::page(b), cell),
        None => cell,
    })
}

/// HZ-036 `bagTake(Object bt, int cell)V` / `bagTakeSome(Object bt, int cell, int n)V`, in place of
/// `bt.a(I)V` / `bt.a(II)V` where a bag window takes the cursor's item away (인벤토리 「버리기」 and a
/// 귀환서 used, 계정창고 「넣기」 and 「버리기」): the same place [`BagAt`] read it from. Without it a page
/// past the first took the first page's item at that cell — the chosen one stayed (and went into the
/// 계정창고 as well) while another, a worn piece too, was gone (user report 2026-11-01).
struct BagTake {
    some: bool,
}

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for BagTake {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let (bag, cell) = (object(args[0].clone()), int(args[1].clone()));
        let Some(bag) = bag else {
            return Err(null_pointer(jvm, "Hooks.bagTake").await);
        };
        let at = bag_place(jvm, &bag, cell).await?;
        if at != cell {
            tracing::info!("hz: bag take cell {cell} -> place {at}");
        }
        if self.some {
            let n = int(args[2].clone());
            jvm.invoke_virtual::<_, ()>(&bag, "a", "(II)V", (at, n)).await?;
        } else {
            jvm.invoke_virtual::<_, ()>(&bag, "a", "(I)V", (at,)).await?;
        }
        Ok(JavaValue::Void)
    }
}

/// HZ-062 `sellTake(Object bag, Object item, byte id, int n)Z`, in place of `bt.b(BBI)Z` where the
/// shop sells (`co.a(BB)V` @353 one, @467 `n` of a stack; `co` is the menu's shop and the NPCs' alike):
/// the item the cursor chose, found by identity, gives what it holds of the `n` by place; any rest — a
/// stack sold past it — and the game's tidy after a take go the game's way. The game walked the bag
/// from its end for the last unworn item of that category and number: of two of one name the later one
/// went, not the one chosen (user report 2026-11-02). Not found (never, the cursor read it from this
/// bag), the game's call as it was.
struct SellTake;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for SellTake {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let (Some(bag), Some(item)) = (object(args[0].clone()), object(args[1].clone())) else {
            return Err(null_pointer(jvm, "Hooks.sellTake").await);
        };
        let id: i8 = args[2].clone().into();
        let n = int(args[3].clone());
        let cat: i8 = jvm.get_field_from(&item, "ba", "a", "B").await?;
        let game = |n: i32| [JavaValue::Byte(cat), JavaValue::Byte(id), JavaValue::Int(n)];
        let worn: bool = jvm.invoke_virtual(&item, "c", "()Z", ()).await?;
        let place = if worn { None } else { place_in(jvm, &bag, &item).await? };
        let Some(at) = place else {
            tracing::info!("hz: sell ({cat},{id}) x{n}: the chosen item is not in the bag — the game's take");
            return jvm.invoke_virtual(&bag, "b", "(BBI)Z", game(n)).await;
        };
        let held: i8 = jvm.get_field_from(&item, "ba", "c", "B").await?;
        let (take, rest) = crate::bulk::chosen_first(i32::from(held), n);
        jvm.invoke_virtual::<_, ()>(&bag, "a", "(II)V", (at, take)).await?;
        tracing::info!("hz: sell ({cat},{id}) x{take} from the chosen place {at}, {rest} more the game's way");
        // `bt.b(BBI)Z` with the rest: any of it from the other stacks, then its own tidy (private b()V)
        // — with nothing left it takes nothing and only tidies.
        let rest_taken: bool = jvm.invoke_virtual(&bag, "b", "(BBI)Z", game(rest)).await?;
        Ok(JavaValue::Boolean(rest == 0 || rest_taken))
    }
}

/// HZ-063 `scriptFlag(int n)Z`, in place of `ed.a(I)Z` where a map script's condition tests a story
/// flag: the flag — Karmen's [`hooks::KARMEN_AWAY`] only when the story can have set it.
struct ScriptFlag;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for ScriptFlag {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let n = int(args[0].clone());
        let on: bool = jvm.invoke_static("ed", "a", "(I)Z", (n,)).await?;
        if n != hooks::KARMEN_AWAY || !on {
            return Ok(JavaValue::Boolean(on));
        }
        let real = karmen_away_real(jvm).await?;
        if !real {
            tracing::info!("hz: flag 48 is on but not the story's (「수상한 남자」 turned down) - read as off");
        }
        Ok(JavaValue::Boolean(real))
    }
}

/// HZ-063 `scriptFlagOn(int n)V`, in place of `ed.c(I)V` where a map script turns a story flag on: the
/// game's call — but Karmen's [`hooks::KARMEN_AWAY`] only from the story's own map
/// ([`story::karmen_away_site`]: 은거지's masked figure; 「수상한 남자」's refusal runs on map 4).
struct ScriptFlagOn;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for ScriptFlagOn {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let n = int(args[0].clone());
        if n == hooks::KARMEN_AWAY {
            let map = live_map_number(jvm).await?;
            if !story::karmen_away_site(map) {
                tracing::info!("hz: flag 48 (Karmen spent) left off - not the story's site (map {map:?}: 「수상한 남자」 turned down)");
                return Ok(JavaValue::Void);
            }
        }
        let _: () = jvm.invoke_static("ed", "c", "(I)V", (n,)).await?;
        Ok(JavaValue::Void)
    }
}

/// HZ-063, the read: whether the story can have turned flag 48 on by now ([`story::karmen_away_real`])
/// — for a save made before the write was gated by its site.
async fn karmen_away_real(jvm: &Jvm) -> JvmResult<bool> {
    let round = replay_round(jvm).await?;
    let route: i8 = jvm.invoke_static("ed", "a", "(I)B", (story::KARMEN_ROUTE_QUEST,)).await?;
    let battle: i8 = jvm.invoke_static("ed", "a", "(I)B", (story::FINAL_BATTLE_QUEST,)).await?;
    Ok(story::karmen_away_real(round, route, battle))
}

/// HZ-064 `aimHold(Object target, int frames)V`, in place of `l.b(I)V` where 조준사격 and 저격 aim: the
/// game's hold — unless the target never walks (`l.a:I` −1, the mark that keeps its AI choosing).
struct AimHold;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for AimHold {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(target) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.aimHold").await);
        };
        let frames = int(args[1].clone());
        let hold: i32 = jvm.get_field_from(&target, "l", "a", "I").await?;
        if hold == -1 {
            tracing::info!("hz: aim hold {frames} skipped - the target never walks (its AI needs l.a -1)");
            return Ok(JavaValue::Void);
        }
        let _: () = jvm.invoke_virtual(&target, "b", "(I)V", (frames,)).await?;
        Ok(JavaValue::Void)
    }
}

/// Where `item` itself is in the bag `bt` (identity — the `Vector` holds the very objects the windows
/// read), if it is.
#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
async fn place_in(jvm: &Jvm, bag: &Box<dyn ClassInstance>, item: &Box<dyn ClassInstance>) -> JvmResult<Option<i32>> {
    let size: i32 = jvm.invoke_virtual(bag, "a", "()I", ()).await?;
    for i in 0..size {
        if let Some(it) = object(jvm.invoke_virtual(bag, "a", "(I)Lba;", (i,)).await?)
            && it.as_ref() == item.as_ref()
        {
            return Ok(Some(i));
        }
    }
    Ok(None)
}

/// HZ-036 `bagSideways / bagUpDown(Object window, int action, int raw)Z`, in place of `o.d(II)Z` /
/// `o.e(II)Z` in a bag window's keys: the move, then the page it went round to.
struct BagMove {
    vertical: bool,
}

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for BagMove {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let (this, action, raw) = (object(args[0].clone()), int(args[1].clone()), int(args[2].clone()));
        let Some(this) = this else {
            return Err(null_pointer(jvm, "Hooks.bagMove").await);
        };
        let before: i8 = jvm.get_field_from(&this, "o", "a", "B").await?;
        let moved: bool = jvm.invoke_virtual(&this, if self.vertical { "e" } else { "d" }, "(II)Z", (action, raw)).await?;
        if !moved {
            return Ok(JavaValue::Boolean(false));
        }
        let after: i8 = jvm.get_field_from(&this, "o", "a", "B").await?;
        let delta = crate::bags::went_round(crate::bags::forward(self.vertical, action, raw), before, after);
        if delta != 0
            && let Some(bag) = crate::bags::window_bag(jvm, &this).await?
        {
            let page = crate::bags::turn(bag, delta);
            tracing::info!("hz: bag {bag} page {}", crate::bags::label(page));
        }
        Ok(JavaValue::Boolean(true))
    }
}

/// HZ-036 `bagGrid(Object window, Object dd, Object bt, …)V` / `bagGridBz(Object window, Object dd,
/// Object bt)V`, in place of the bag windows' grid draws (`q.a(Ldd;Lbt;BZBBI)V`, `bz.a(Ldd;Lbt;)V`):
/// the draw, then — the player's bag — its page under the tabs.
struct BagGrid {
    bz: bool,
}

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for BagGrid {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let mut args = args.into_vec().into_iter();
        let this = object(args.next().unwrap());
        let Some(this) = this else {
            return Err(null_pointer(jvm, "Hooks.bagGrid").await);
        };
        let rest: Vec<JavaValue> = args.collect();
        let (dd, bag) = (rest[0].clone(), object(rest[1].clone()));
        let descriptor = if self.bz { "(Ldd;Lbt;)V" } else { "(Ldd;Lbt;BZBBI)V" };
        let _: () = jvm.invoke_virtual(&this, "a", descriptor, rest).await?;
        if let Some(bag) = bag
            && let Some(b) = crate::bags::player_bag(jvm, &bag).await?
        {
            let frame_y: i32 = jvm.get_field_from(&this, "o", "b", "I").await?;
            crate::bags::draw_pager(jvm, dd, frame_y, b).await?;
        }
        Ok(JavaValue::Void)
    }
}

/// HZ-036 `bagCells(Object bt)I`, in place of `getfield bt.a:B` where 계정창고's window sizes its grid:
/// a page at most.
struct BagCells;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for BagCells {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(bag) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.bagCells").await);
        };
        let size: i8 = jvm.get_field_from(&bag, "bt", "a", "B").await?;
        Ok(JavaValue::Int(i32::from(size.min(hooks::BAG_PAGE))))
    }
}

/// HZ-033: one draw of the game's Random (`abs` is the caller's business), or none without one.
async fn draw(jvm: &Jvm) -> JvmResult<Option<i32>> {
    let Some(random) = object(jvm.get_static_field("ar", "a", "Ljava/util/Random;").await?) else {
        return Ok(None);
    };
    Ok(Some(jvm.invoke_virtual(&random, "nextInt", "()I", ()).await?))
}

/// HZ-033 `commonDrop(int, int)I`, in place of `Math.min(lvl / 4, 14)` in a monster's drop.
struct CommonDrop;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for CommonDrop {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let id = int(args[0].clone()).min(int(args[1].clone()));
        let Some(roll) = draw(jvm).await? else {
            return Ok(JavaValue::Int(id));
        };
        Ok(JavaValue::Int(drops::spread(id, roll, drops::COMMON_SPREAD, 0)))
    }
}

/// HZ-033 `dropItem(Object g, byte x, byte y, byte cat, byte id)V`, in place of `g.a(BBBB)V` where an
/// item slot's drop goes onto the ground. The roll's slots are then taken: HZ-033 made the `return` after
/// this a `nop`, so the loop goes on (the slots left read [`drops::SLOT_TAKEN`]) to the game's bands.
struct DropItem {
    taken: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for DropItem {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let mut args = args.into_vec();
        let Some(map) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.dropItem").await);
        };
        self.taken.store(true, std::sync::atomic::Ordering::Relaxed);
        let (cat, id): (i8, i8) = (args[3].clone().into(), args[4].clone().into());
        if (drops::EXTRA_FROM..=drops::RARE_TOP).contains(&id)
            && replay_round(jvm).await? >= drops::EXTRA_ROUND
            && let Some(roll) = draw(jvm).await?
            && let Some(top) = drops::extra(cat, id, replay_round(jvm).await?, roll)
        {
            tracing::info!("hz: drop: a top piece ({cat}, {top}) for ({cat}, {id})");
            args[4] = JavaValue::Byte(top);
        }
        let _: () = jvm.invoke_virtual(&map, "a", "(BBBB)V", args[1..].to_vec()).await?;
        Ok(JavaValue::Void)
    }
}

/// HZ-006 `autoLoot(Object g, int x, int y)Z`.
/// HZ-009 (withdrawn with HZ-010 — the cheat shop is gone, so this is never reached in a default
/// build): what `ep.a(I)Z` returns in place of its QA-property test. Always granted, which is
/// HZ-001's `true`; there is no option row any more.
struct CashFree(#[allow(dead_code)] Arc<GameOptions>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for CashFree {
    async fn call(&self, _jvm: &Jvm, _: &mut WieJvmContext, _args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        tracing::info!("hz: cash purchase -> granted (HZ-009, 치트상점은 HZ-010 으로 숨김)");
        Ok(JavaValue::Boolean(true))
    }
}

/// HZ-012: the label of row `row` of a menu of `rows` rows, after the cheat shop: the last row is
/// gm[9] (시스템), the one before it 계정창고 (`None`), the rows above gm[r+1] — ten rows, or nine
/// with 미니맵 (gm[8]) gone too (HZ-026). Anything past the rows is itself.
pub fn menu_label_index(row: i32, rows: i32) -> Option<i32> {
    match row {
        _ if row == rows - 1 => Some(9),
        _ if row == rows - 2 => None,
        0.. if row < rows - 2 => Some(row + 1),
        _ => Some(row),
    }
}

/// HZ-012 `menuLabel(Object window, int index)[B` — in place of `bh.a.a(index)` where the list
/// window `m` draws its rows and where `o` draws a child's title. Shifted when the window is the
/// menu or opened under it at any depth (`o.a:Lo;` chain reaches `bh`): a child's title is always
/// the menu's selected row `bh.f`, also for the equipment window's bag grid `bv` (bh → em → bv)
/// and the equipment window opened from the inventory (bh → ds → em). The rows of the other lists
/// that reach here (de 71 + r, ec 49 + r) are past the menu's 0..9 and read the table as before;
/// bn passes base −1, which `m.a(dd,IIZ)I` @285 sends past every lookup.
struct MenuLabel;

/// Parent links followed before giving up — the menu's deepest window is three below it.
const MENU_DEPTH: usize = 8;

/// The menu `bh` the window is or was opened under, if any.
async fn under_menu(jvm: &Jvm, window: Box<dyn ClassInstance>) -> Result<Option<Box<dyn ClassInstance>>, JavaError> {
    let mut at = Some(window);
    for _ in 0..MENU_DEPTH {
        let Some(w) = at else { return Ok(None) };
        if jvm.is_instance(&*w, "bh") {
            return Ok(Some(w));
        }
        at = jvm.get_field(&w, "a", "Lo;").await?;
    }
    Ok(None)
}

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for MenuLabel {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let mut args = args.into_vec().into_iter();
        let (window, index) = (object(args.next().unwrap()), int(args.next().unwrap()));
        let Some(window) = window else {
            return Err(null_pointer(jvm, "Hooks.menuLabel: window").await);
        };
        let table = object(jvm.get_static_field("bh", "a", "Lcx;").await?);
        let Some(table) = table else {
            return Err(null_pointer(jvm, "bh.a").await);
        };
        let index = match under_menu(jvm, window).await? {
            Some(menu) => {
                // The row count o.b:B: 10, or 9 without 미니맵 (HZ-026).
                let rows: i8 = jvm.get_field_from(&menu, "o", "b", "B").await?;
                match menu_label_index(index, i32::from(rows)) {
                    Some(i) => i,
                    None => return Ok(JavaValue::Object(Some(byte_array(jvm, text::ACCOUNT_STORAGE).await?))),
                }
            }
            None => index,
        };
        jvm.invoke_virtual(&table, "a", "(I)[B", (index,)).await
    }
}

/// HZ-012 `openStorage(Object menu)V` — the menu's 계정창고 row: the game's 돈/아이템 chooser `bg`, as
/// the title's 계정창고 opens it after a slot is picked, made the menu's child.
struct OpenStorage;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for OpenStorage {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let mut args = args.into_vec().into_iter();
        let Some(mut menu) = object(args.next().unwrap()) else {
            return Err(null_pointer(jvm, "Hooks.openStorage: menu").await);
        };
        let chooser = jvm.new_class("bg", "(Lo;)V", (menu.clone(),)).await?;
        jvm.put_field(&mut menu, "b", "Lo;", chooser).await?;
        tracing::info!("hz: 계정창고 opened from the menu");
        Ok(JavaValue::Void)
    }
}

/// HZ-012 `storageClosed(Object chooser)V` — in place of `bg`'s RSK `parent.c()`: close as the
/// game did, then, when the parent is the in-game menu, the game's own quick save (`bh.c(2)`:
/// 시스템 → 「저장하시겠습니까?」), so the slot catches up with what HOL.storage already holds.
struct StorageClosed;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for StorageClosed {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let mut args = args.into_vec().into_iter();
        let Some(chooser) = object(args.next().unwrap()) else {
            return Err(null_pointer(jvm, "Hooks.storageClosed: chooser").await);
        };
        let parent: Option<Box<dyn ClassInstance>> = jvm.get_field(&chooser, "a", "Lo;").await?;
        let Some(parent) = parent else {
            return Err(null_pointer(jvm, "Hooks.storageClosed: parent").await);
        };
        let _: () = jvm.invoke_virtual(&parent, "c", "()V", ()).await?;
        if jvm.is_instance(&*parent, "bh") {
            tracing::info!("hz: 계정창고 closed in game -> quick save");
            let _: () = jvm.invoke_virtual(&parent, "c", "(B)V", (JavaValue::Byte(2),)).await?;
        }
        Ok(JavaValue::Void)
    }
}

/// HZ-011 `text(Object cx, int index)[B` — the body of `cx.a(I)[B`: the table's entry, or the
/// host's rename of it (`text::rename`). Every string the game draws passes here, so the common
/// path is one field read, one element load and a byte compare; a renamed entry allocates its
/// replacement (a few per screen at most).
struct Text(Arc<GuestState>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for Text {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let mut args = args.into_vec().into_iter();
        let (this, index) = (object(args.next().unwrap()), int(args.next().unwrap()));
        let Some(this) = this else {
            return Err(null_pointer(jvm, "Hooks.text: this").await);
        };
        let entries: Option<Box<dyn ClassInstance>> = jvm.get_field(&this, "a", "[Ljava/lang/Object;").await?;
        let Some(entries) = entries else {
            return Err(null_pointer(jvm, "Hooks.text: cx.a").await);
        };
        // The original's `aaload`: out of range throws, as it did.
        let mut loaded: Vec<Option<Box<dyn ClassInstance>>> = jvm.load_array(&entries, index.max(0) as usize, 1).await?;
        let Some(entry) = loaded.pop().flatten() else {
            return Ok(JavaValue::Object(None));
        };
        let len = jvm.array_length(&entry).await?;
        let mut bytes = vec![0u8; len];
        jvm.array_raw_buffer(&entry).await?.read(0, &mut bytes)?;
        // HZ-058: the popups' registration rows are the plates' two.
        if self.0.patch_on("HZ-058")
            && let Some(row) = text::quick_row(&bytes)
        {
            return Ok(JavaValue::Object(Some(byte_array(jvm, &row).await?)));
        }
        // HZ-068: 「고양」's line says 의지.
        if self.0.patch_on("HZ-068")
            && let Some(line) = text::will_line(&bytes)
        {
            return Ok(JavaValue::Object(Some(byte_array(jvm, &line).await?)));
        }
        match text::rename(&bytes) {
            Some(renamed) => Ok(JavaValue::Object(Some(byte_array(jvm, renamed).await?))),
            None => Ok(JavaValue::Object(Some(entry))),
        }
    }
}

/// HZ-011 `storageHelp()[B` — in place of `getstatic rpg/d.J:[B`: the one-time help before the
/// storage, in 계정창고 wording when it is exactly the game's paragraph (`text::storage_help`).
struct StorageHelp;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for StorageHelp {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, _args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let help: Option<Box<dyn ClassInstance>> = object(jvm.get_static_field("rpg/d", "J", "[B").await?);
        let Some(help) = help else {
            return Ok(JavaValue::Object(None));
        };
        let len = jvm.array_length(&help).await?;
        let mut bytes = vec![0u8; len];
        jvm.array_raw_buffer(&help).await?.read(0, &mut bytes)?;
        match text::storage_help(&bytes) {
            Some(renamed) => {
                tracing::info!("hz: 계정창고 help ({len} B) in 계정창고 wording");
                Ok(JavaValue::Object(Some(byte_array(jvm, &renamed).await?)))
            }
            None => Ok(JavaValue::Object(Some(help))),
        }
    }
}

/// The hooks class's static that keeps the orb's `cw` (HZ-016).
const ORB_PICTURE: &str = "orbPicture";
/// … and the elixir's (HZ-060).
const ELIXIR_PICTURE: &str = "elixirPicture";
/// HZ-068: the stat names' sheet with 의지 in it, made once.
const STAT_PICTURE: &str = "statPicture";

/// `rpg/e.a(String,I)[B` in the item factory `ba.a(BB)Lba;`: the file's record, or one the files do
/// not have — HZ-016 `itemRecord` the orb's (`revive::record`, `/itm/20` number 9) and with HZ-060
/// the elixir's (`elixir::record`, number 10), HZ-023 `gearRecord` a guardian piece's
/// (`gear::record`, number 40 of an equipment file), and `orbGearRecord` (both patches) either.
struct ItemRecord {
    orb: bool,
    gear: bool,
    legend: Legends,
}

/// Whether an [`ItemRecord`] hook serves HZ-031's 85제 pieces: its own hook always, HZ-016's and
/// HZ-023's when HZ-031 was chosen too (it then leaves their hook in place). HZ-035's recipes: with
/// HZ-031 and HZ-035 both chosen.
enum Legends {
    Always(Arc<GuestState>),
    IfChosen(Arc<GuestState>),
}

impl Legends {
    fn served(&self) -> bool {
        match self {
            Legends::Always(_) => true,
            Legends::IfChosen(state) => state.patch_on("HZ-031"),
        }
    }

    fn recipes(&self) -> bool {
        let (Legends::Always(state) | Legends::IfChosen(state)) = self;
        state.patch_on("HZ-031") && state.patch_on("HZ-035")
    }

    /// HZ-060's elixir, served where the orb is.
    fn elixir(&self) -> bool {
        let (Legends::Always(state) | Legends::IfChosen(state)) = self;
        state.patch_on("HZ-060")
    }
}

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for ItemRecord {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let mut args = args.into_vec().into_iter();
        let (path, index) = (object(args.next().unwrap()), int(args.next().unwrap()));
        // Only numbers 9, 10, 40 and 41.. can be served: the path is not read for every item the game loads.
        let orb = self.orb && index == i32::from(hooks::REVIVE_ORB.1);
        let elixir = self.orb && index == i32::from(hooks::ELIXIR.1) && self.legend.elixir();
        let gear = self.gear && index == i32::from(hooks::GEAR_ID);
        let legend = (i32::from(hooks::LEGEND_FIRST_ID)..i32::from(hooks::GROUND_TOP_ID)).contains(&index) && self.legend.served();
        // HZ-035's recipes (/itm/18 28..63): they make 85제 pieces, so only with HZ-031 too.
        let recipe = (i32::from(legend::RECIPE_FIRST)..i32::from(legend::RECIPE_FIRST) + legend::ALL.len() as i32).contains(&index)
            && self.legend.recipes();
        if (orb || elixir || gear || legend || recipe)
            && let Some(p) = &path
        {
            let path_s = JavaLangString::to_rust_string(jvm, p).await?;
            if recipe && let Some(r) = legend::recipe_asked(&path_s, index) {
                return Ok(JavaValue::Object(Some(byte_array(jvm, &legend::recipe_record(&r)).await?)));
            }
            if legend && let Some(piece) = legend::asked(&path_s, index) {
                return Ok(JavaValue::Object(Some(byte_array(jvm, &legend::record(piece)).await?)));
            }
            if orb && revive::is_orb_record(&path_s, index) {
                return Ok(JavaValue::Object(Some(byte_array(jvm, &revive::record()).await?)));
            }
            if elixir && elixir::is_elixir_record(&path_s, index) {
                return Ok(JavaValue::Object(Some(byte_array(jvm, &elixir::record()).await?)));
            }
            if gear && let Some(piece) = gear::asked(&path_s, index) {
                return Ok(JavaValue::Object(Some(byte_array(jvm, &gear::record(piece)).await?)));
            }
        }
        jvm.invoke_static("rpg/e", "a", "(Ljava/lang/String;I)[B", [JavaValue::Object(path), JavaValue::Int(index)]).await
    }
}

/// HZ-023 `rareDrop(int band, int top)I` — `Math.min(lvl / 5 + 13, 28)` in a monster's drop
/// `cq.o()V`: `min`, except at the band's top (LV 75 and up), where one in `gear::ODDS` draws of the
/// game's own Random `ar.a` (drawn only then) is a guardian piece's number.
struct RareDrop(Arc<GuestState>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for RareDrop {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let (band, top) = (int(args[0].clone()), int(args[1].clone()));
        // HZ-033 (which may have put this hook here alone): a few under the game's number.
        let spread = if self.0.patch_on("HZ-033")
            && let Some(roll) = draw(jvm).await?
        {
            drops::spread(band.min(top), roll, drops::RARE_SPREAD, drops::RARE_FLOOR)
        } else {
            band.min(top)
        };
        if !self.0.patch_on("HZ-023") || !gear::at_top(band, top) {
            return Ok(JavaValue::Int(spread));
        }
        let Some(random) = object(jvm.get_static_field("ar", "a", "Ljava/util/Random;").await?) else {
            return Ok(JavaValue::Int(band.min(top)));
        };
        let roll: i32 = jvm.invoke_virtual(&random, "nextInt", "()I", ()).await?;
        let got = gear::rare_drop(band, top, roll);
        let got = if got == i32::from(hooks::GEAR_ID) { got } else { spread };
        if got == i32::from(hooks::GEAR_ID) {
            tracing::info!("hz: gear: a guardian piece drops (rare band {band})");
        }
        Ok(JavaValue::Int(got))
    }
}

/// HZ-016 `itemIcon(int index)Object` — `rpg/e.k[index]` in `ba.a(dd,IIZZ)V`: the pack's picture,
/// or the orb's for its index (29 — in the pack, the selection frame), or HZ-060's elixir's (30 —
/// past the pack; only an elixir asks for it).
struct ItemIcon;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for ItemIcon {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let index = int(args[0].clone());
        if index == hooks::REVIVE_ORB_ICON {
            return Ok(JavaValue::Object(Some(orb_picture(jvm).await?)));
        }
        if index == hooks::ELIXIR_ICON {
            return Ok(JavaValue::Object(elixir_picture(jvm).await?));
        }
        let Some(list) = object(jvm.get_static_field("rpg/e", "k", "[Lcw;").await?) else {
            return Err(null_pointer(jvm, "rpg/e.k").await);
        };
        // The original's `aaload`: out of range fails, as it did.
        let mut got: Vec<JavaValue> = jvm.load_array(&list, index.max(0) as usize, 1).await?;
        Ok(got.pop().unwrap_or(JavaValue::Object(None)))
    }
}

/// The orb's `cw`: kept in [`ORB_PICTURE`], made the first time through the game's own
/// `cw.a([IIIZ)Lcw;` (`Image.createRGBImage` with alpha).
async fn orb_picture(jvm: &Jvm) -> JvmResult<Box<dyn ClassInstance>> {
    if let Some(cw) = object(jvm.get_static_field(hooks::CLASS, ORB_PICTURE, "Ljava/lang/Object;").await?) {
        return Ok(cw);
    }
    let px = revive::icon();
    let mut rgb = jvm.instantiate_array("I", px.len()).await?;
    jvm.store_array(&mut rgb, 0, px).await?;
    let args = [
        JavaValue::Object(Some(rgb)),
        JavaValue::Int(revive::ICON_W as i32),
        JavaValue::Int(revive::ICON_H as i32),
        JavaValue::Boolean(true),
    ];
    let cw: Box<dyn ClassInstance> = jvm.invoke_static("cw", "a", "([IIIZ)Lcw;", args).await?;
    jvm.put_static_field(hooks::CLASS, ORB_PICTURE, "Ljava/lang/Object;", JavaValue::Object(Some(cw.clone()))).await?;
    tracing::info!("hz: 부활의 구 picture made, {}x{}", revive::ICON_W, revive::ICON_H);
    Ok(cw)
}

/// The elixir's `cw` (HZ-060): 포션's picture (`rpg/e.k[22]`) read through `Image.getRGB`, whitened
/// (`elixir::whiten`) and made the orb's way; kept in [`ELIXIR_PICTURE`]. `None` while the pack is not
/// loaded yet — the bag then draws nothing in the cell, as for any picture the game lacks.
async fn elixir_picture(jvm: &Jvm) -> JvmResult<Option<Box<dyn ClassInstance>>> {
    if let Some(cw) = object(jvm.get_static_field(hooks::CLASS, ELIXIR_PICTURE, "Ljava/lang/Object;").await?) {
        return Ok(Some(cw));
    }
    let Some(list) = object(jvm.get_static_field("rpg/e", "k", "[Lcw;").await?) else {
        return Ok(None);
    };
    let base = elixir::BASE_PICTURE as usize;
    if jvm.array_length(&list).await? <= base {
        return Ok(None);
    }
    let Some(potion) = jvm.load_array::<JavaValue>(&list, base, 1).await?.pop().and_then(object) else {
        return Ok(None);
    };
    let Some((w, h, px)) = picture(jvm, &potion, None).await? else {
        return Ok(None);
    };
    let px = elixir::whiten(&px);
    let mut rgb = jvm.instantiate_array("I", px.len()).await?;
    jvm.store_array(&mut rgb, 0, px).await?;
    let args = [JavaValue::Object(Some(rgb)), JavaValue::Int(w as i32), JavaValue::Int(h as i32), JavaValue::Boolean(true)];
    let cw: Box<dyn ClassInstance> = jvm.invoke_static("cw", "a", "([IIIZ)Lcw;", args).await?;
    jvm.put_static_field(hooks::CLASS, ELIXIR_PICTURE, "Ljava/lang/Object;", JavaValue::Object(Some(cw.clone()))).await?;
    tracing::info!("hz: 엘릭서 picture made from 포션's, {w}x{h}");
    Ok(Some(cw))
}

/// HZ-068 `statNames()Object`, in place of `getstatic rpg/e.w:Lcw;` where 캐릭터 `dy` draws a stat's name: the sheet with
/// 의지 for 지력 (`statname::will_sheet`), made the elixir's way and kept in [`STAT_PICTURE`]; the game's own while it
/// cannot be made.
struct StatNames;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for StatNames {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, _args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        if let Some(cw) = object(jvm.get_static_field(hooks::CLASS, STAT_PICTURE, "Ljava/lang/Object;").await?) {
            return Ok(JavaValue::Object(Some(cw)));
        }
        let Some(sheet) = object(jvm.get_static_field("rpg/e", "w", "Lcw;").await?) else {
            return Ok(JavaValue::Object(None));
        };
        let renamed = match picture(jvm, &sheet, None).await? {
            Some((w, h, px)) => statname::will_sheet(&px, w as usize, h as usize).map(|px| (w, h, px)),
            None => None,
        };
        let Some((w, h, px)) = renamed else {
            tracing::warn!("hz: the stat names' sheet is not the one HZ-068 knows, left as it is");
            return Ok(JavaValue::Object(Some(sheet)));
        };
        let mut rgb = jvm.instantiate_array("I", px.len()).await?;
        jvm.store_array(&mut rgb, 0, px).await?;
        let args = [JavaValue::Object(Some(rgb)), JavaValue::Int(w as i32), JavaValue::Int(h as i32), JavaValue::Boolean(true)];
        let cw: Box<dyn ClassInstance> = jvm.invoke_static("cw", "a", "([IIIZ)Lcw;", args).await?;
        jvm.put_static_field(hooks::CLASS, STAT_PICTURE, "Ljava/lang/Object;", JavaValue::Object(Some(cw.clone()))).await?;
        tracing::info!("hz: stat names' sheet made, {w}x{h}");
        Ok(JavaValue::Object(Some(cw)))
    }
}

/// HZ-060 `potion(Object item)I` — `getfield ba.b:B` where the drink `ek.a(Z)Z` picks the effect: the
/// elixir fills HP (the game's `ek.c(I)V`, its own rules) and SP (`ek.c(BI)V`) to the player's maxima
/// and answers [`hooks::ELIXIR_DRINK`], so the game finishes the drink its way; any other item its
/// number.
struct Potion(Arc<GuestState>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for Potion {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(item) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.potion").await);
        };
        let id: i8 = jvm.get_field_from(&item, "ba", "b", "B").await?;
        let cat: i8 = jvm.get_field_from(&item, "ba", "a", "B").await?;
        if (cat, id) != hooks::ELIXIR || !self.0.patch_on("HZ-060") {
            return Ok(JavaValue::Int(id.into()));
        }
        let Some(me) = player(jvm).await? else {
            return Ok(JavaValue::Int(id.into()));
        };
        let Some(top) = object(jvm.get_field_from(&me, "ek", "b", "[I").await?) else {
            return Ok(JavaValue::Int(id.into()));
        };
        let max: Vec<i32> = jvm.load_array(&top, 0, 2).await?;
        let _: () = jvm.invoke_virtual(&me, "c", "(I)V", [JavaValue::Int(max[0].max(0))]).await?;
        let _: () = jvm.invoke_virtual(&me, "c", "(BI)V", [JavaValue::Byte(1), JavaValue::Int(max[1].max(0))]).await?;
        if let Some(now) = object(jvm.get_field_from(&me, "ek", "a", "[I").await?) {
            let now: Vec<i32> = jvm.load_array(&now, 0, 2).await?;
            tracing::info!("hz: 엘릭서 — HP SP {now:?} of {max:?}");
        }
        Ok(JavaValue::Int(hooks::ELIXIR_DRINK))
    }
}

/// HZ-061 `killCount(int kind, byte group)V`, in place of `ed.c(IB)V` where a monster turns dead
/// (`cq.a(B)V` @44): a boss room's last counted kill ([`story::waits`]) spends the counter as the game
/// does (`ed.ae[kind] = −1`) and its story group waits ([`GuestState::story_wait`]); every other kill —
/// and every kill without HZ-006's field tick, which counts the wait — is the game's own call.
struct KillCount(Arc<GuestState>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for KillCount {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let (kind, group): (i32, i8) = (int(args[0].clone()), args[1].clone().into());
        let game = || async { jvm.invoke_static::<_, ()>("ed", "c", "(IB)V", [JavaValue::Int(kind), JavaValue::Byte(group)]).await };
        if !(self.0.patch_on("HZ-061") && self.0.patch_on("HZ-006")) {
            game().await?;
            return Ok(JavaValue::Void);
        }
        let counters = object(jvm.get_static_field("ed", "ae", "[B").await?);
        let Some(mut counters) = counters.filter(|_| (0..=i32::from(i16::MAX)).contains(&kind)) else {
            game().await?;
            return Ok(JavaValue::Void);
        };
        let at = kind as usize;
        let counter = if at < jvm.array_length(&counters).await? { jvm.load_array::<i8>(&counters, at, 1).await?.pop() } else { None };
        if !counter.is_some_and(|c| story::waits(kind as i16, c)) {
            game().await?;
            return Ok(JavaValue::Void);
        }
        jvm.store_array(&mut counters, at, vec![-1i8]).await?;
        let map = live_map_number(jvm).await?.unwrap_or(-1);
        self.0.story_wait(story::Wait::new(group, map));
        tracing::info!("hz: story: boss {kind} down on map {} — group {group} waits {} s", map as u8, story::WAIT_SECONDS);
        Ok(JavaValue::Void)
    }
}

/// The map the player is on (`ed.a:Lg;` → `g.b:B`), if one is loaded.
async fn live_map_number(jvm: &Jvm) -> JvmResult<Option<i8>> {
    let Some(map) = object(jvm.get_static_field("ed", "a", "Lg;").await?) else { return Ok(None) };
    Ok(Some(jvm.get_field(&map, "b", "B").await?))
}

/// HZ-061, from HZ-006's field tick: the waiting boss story counts the tick, and starts through the
/// game's own `eb.a(B)Z` once its time is up — the field up (`ed.d:B` 3) and the player not falling
/// (`ar.b` 6 · 7). Called in `ed.d()V` before the map's and monsters' updates, where the game's own
/// call came from within them (`g.d()V` → a death): the same frame's work either way.
async fn story_tick(jvm: &Jvm, state: &GuestState) -> JvmResult<()> {
    let Some(map) = live_map_number(jvm).await? else { return Ok(()) };
    let scene: i8 = jvm.get_static_field("ed", "d", "B").await?;
    let standing = match player(jvm).await? {
        Some(me) => !matches!(jvm.get_field_from::<i8>(&me, "ar", "b", "B").await?, 6 | 7),
        None => false,
    };
    match state.story_tick(map, scene == 3 && standing) {
        Some(story::Step::Go(group)) => {
            let started: bool = jvm.invoke_static("eb", "a", "(B)Z", [JavaValue::Byte(group)]).await?;
            tracing::info!("hz: story: group {group} of map {} starts -> {started}", map as u8);
        }
        Some(story::Step::Wait(_)) => {
            let gone = clear_room(jvm).await?;
            if gone > 0 {
                tracing::info!("hz: story: {gone} monster(s) left standing in the boss's room go");
            }
        }
        Some(story::Step::Drop) => tracing::info!("hz: story: left for map {} — the wait is dropped", map as u8),
        None => {}
    }
    Ok(())
}

/// HZ-061: while a boss's story waits, what still stands in its room goes — the game's story would
/// have taken the player away from it at once. 거인신's room keeps two 돌기둥 (kind 121) after its head
/// falls, which went on hitting through the wait (probe 2026-11-01: HP 1103 → 748 in the five seconds);
/// a summon, or one back from the respawn queue, would too. Each goes by its own `m()` — off the tile
/// grid and the draw list (`cq.m()V`: `g.d(Ler;)V` → `ar.v()V`; a boss part's override its own way):
/// no drop, no EXP, no respawn, no kill counted. One dying (state 7) is the map loop's to take.
async fn clear_room(jvm: &Jvm) -> JvmResult<usize> {
    let Some(map) = object(jvm.get_static_field("ed", "a", "Lg;").await?) else { return Ok(0) };
    let mut standing = Vec::new();
    for head in ["a", "c"] {
        let mut next = object(jvm.get_field(&map, head, "Ler;").await?);
        for _ in 0..MAP_LIST_MAX {
            let Some(e) = next else { break };
            next = object(jvm.get_field_from(&e, "er", "b", "Ler;").await?);
            if jvm.is_instance(&*e, "cq") && jvm.get_field_from::<i8>(&e, "ar", "b", "B").await? != 7 {
                standing.push(e);
            }
        }
    }
    for m in &standing {
        let _: () = jvm.invoke_virtual(m, "m", "()V", ()).await?;
    }
    Ok(standing.len())
}

/// HZ-016 `quickItem(Object item)Z` — the body of `ba.a()Z`: whether the bag offers the item to
/// the `0`/`#` keys (`revive::quick_item`).
struct QuickItem;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for QuickItem {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(item) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.quickItem: item").await);
        };
        let cat: i8 = jvm.get_field(&item, "a", "B").await?;
        let id: i8 = jvm.get_field(&item, "b", "B").await?;
        Ok(JavaValue::Boolean(revive::quick_item(cat, id)))
    }
}

/// HZ-016 `death()Z` — where the death opened the cheat shop's 「즉시 부활」 (`ek.C()V` @512, the
/// player just down for good: state 7, no scenario running). `true` = the game over, as HZ-010
/// left it; `false` = the question is open (and `ek.C` returns). A failure here is the game over,
/// never a stuck death.
struct Death;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for Death {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, _args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let asked = match ask_revive(jvm).await {
            Ok(asked) => asked,
            Err(e) => {
                tracing::warn!("hz: revive: the question could not be opened ({e:?}) -> game over");
                false
            }
        };
        Ok(JavaValue::Boolean(!asked))
    }
}

/// The orbs in the bag: the special bag `ed.a(20)`, the game's own count `bt.a(BB)I`.
async fn orbs_held(jvm: &Jvm) -> JvmResult<i32> {
    let (cat, id) = hooks::REVIVE_ORB;
    let Some(bag) = object(jvm.invoke_static("ed", "a", "(B)Lbt;", [JavaValue::Byte(cat)]).await?) else {
        return Ok(0);
    };
    jvm.invoke_virtual(&bag, "a", "(BB)I", [JavaValue::Byte(cat), JavaValue::Byte(id)]).await
}

/// With an orb in the bag, the question the purchase used to stand in: the menu as the death
/// opened it (`ed.d = 5`, `bh.a().a()`), and as its child the game's own 예/아니오 window `cf` —
/// `f` = `REVIVE_ASK`, so RSK is no answer and the cursor starts on 예.
async fn ask_revive(jvm: &Jvm) -> JvmResult<bool> {
    let held = orbs_held(jvm).await?;
    if held <= 0 {
        tracing::info!("hz: revive: fell with no 부활의 구 -> game over");
        return Ok(false);
    }
    jvm.put_static_field("ed", "d", "B", JavaValue::Byte(5)).await?;
    let mut menu: Box<dyn ClassInstance> = jvm.invoke_static("bh", "a", "()Lbh;", ()).await?;
    let _: () = jvm.invoke_virtual(&menu, "a", "()V", ()).await?;
    let text = byte_array(jvm, &revive::question(held)).await?;
    let args = [
        JavaValue::Object(Some(menu.clone())),
        JavaValue::Object(Some(text)),
        JavaValue::Byte(hooks::REVIVE_ASK),
        JavaValue::Boolean(true),
    ];
    let ask = jvm.new_class("cf", "(Lo;[BBZ)V", args).await?;
    jvm.put_field(&mut menu, "b", "Lo;", ask).await?;
    let _: () = jvm.invoke_virtual(&menu, "e", "()V", ()).await?;
    tracing::info!("hz: revive: fell with {held} 부활의 구 -> asking");
    Ok(true)
}

/// HZ-016 `answer(Object parent, byte f, byte row)V` — `cf`'s `parent.a(f, row)`. The revive
/// question (its parent the menu, which no other `cf` has) is answered here as the cheat shop
/// answered its own: the window closed (`o.a(BB)V` = `c()`), the field back (`ed.d = 3`, the menu
/// gone `g()`), then 예 = an orb out of the bag and the game's revive `ek.d()`, 아니오 = the game's
/// refusal `ek.b()` (the game over on the next tick). Any other `cf` answers as before.
struct Answer;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for Answer {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let mut args = args.into_vec().into_iter();
        let parent = object(args.next().unwrap());
        let (f, row) = (i8::from(args.next().unwrap()), i8::from(args.next().unwrap()));
        let Some(parent) = parent else {
            return Err(null_pointer(jvm, "Hooks.answer: parent").await);
        };
        if f == hooks::REVIVE_ASK && jvm.is_instance(&*parent, "bh") {
            let _: () = jvm.invoke_virtual(&parent, "c", "()V", ()).await?;
            answer_revive(jvm, &parent, revive::answer(i32::from(row))).await?;
            return Ok(JavaValue::Void);
        }
        // HZ-021: the bulk sale's grade list and its question (tags only the host's windows carry).
        if (f == hooks::BULK_GRADE || f == hooks::BULK_ASK) && jvm.is_instance(&*parent, "co") {
            let mut shop = parent;
            let _: () = jvm.invoke_virtual(&shop, "c", "()V", ()).await?;
            if f == hooks::BULK_GRADE {
                crate::bulk::chose(jvm, &mut shop, row).await?;
            } else {
                crate::bulk::confirmed(jvm, &mut shop, row).await?;
            }
            return Ok(JavaValue::Void);
        }
        let _: () = jvm.invoke_virtual(&parent, "a", "(BB)V", [JavaValue::Byte(f), JavaValue::Byte(row)]).await?;
        Ok(JavaValue::Void)
    }
}

#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
async fn answer_revive(jvm: &Jvm, menu: &Box<dyn ClassInstance>, answer: revive::Answer) -> JvmResult<()> {
    let Some(mut player) = object(jvm.get_static_field("ed", "a", "Lek;").await?) else {
        return Err(null_pointer(jvm, "ed.a (the player)").await);
    };
    // An orb leaves the bag only for a 예 that will be honoured: the game's `bt.b(BBI)Z`.
    let stand = answer == revive::Answer::Revive && {
        let (cat, id) = hooks::REVIVE_ORB;
        match object(jvm.invoke_static("ed", "a", "(B)Lbt;", [JavaValue::Byte(cat)]).await?) {
            Some(bag) => jvm.invoke_virtual(&bag, "b", "(BBI)Z", [JavaValue::Byte(cat), JavaValue::Byte(id), JavaValue::Int(1)]).await?,
            None => false,
        }
    };
    jvm.put_static_field("ed", "d", "B", JavaValue::Byte(3)).await?;
    let _: () = jvm.invoke_virtual(menu, "g", "()V", ()).await?;
    if stand {
        stand_on_tile(jvm, &mut player).await?;
        let _: () = jvm.invoke_virtual(&player, "d", "()V", ()).await?;
        tracing::info!("hz: revive: 예 — standing where it fell, {} 부활의 구 left", orbs_held(jvm).await?);
    } else {
        let _: () = jvm.invoke_virtual(&player, "b", "()V", ()).await?;
        tracing::info!("hz: revive: {:?} -> game over", answer);
    }
    Ok(())
}

/// HZ-016: the player about to stand up where it fell, put on that tile first. A death can come
/// between two tiles — a hit mid-step (`ek.c(I)V` sets state 6 at once) — and the game never stood
/// anyone up there: it ended in the game over (the cheat shop's 「즉시 부활」, `ek.d()V`'s only other
/// caller, is gone with HZ-010). Between tiles `er.E()V` keeps `er.k` / `er.l` set, which the step's
/// check `ar.e()Z` takes as "finish the step" and so tests no tile: a walk across that axis went on
/// through walls to the map's edge and stood there (user report 2026-11-01, 잊혀진땅-설원; probe: fell
/// at x 328 = tile 20.5, a tap of UP walked 240 → 0 through the snow). The game's own placing,
/// `ek.p(I)V` @43..74: `v()` (off the map's grid), the tile's corner, `E()`, `u()` (back on it) — the
/// tile `E()` counts it in, one the step was leaving or entering, so an open one.
#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
async fn stand_on_tile(jvm: &Jvm, player: &mut Box<dyn ClassInstance>) -> JvmResult<()> {
    let x: i16 = jvm.get_field_from(player, "er", "h", "S").await?;
    let y: i16 = jvm.get_field_from(player, "er", "i", "S").await?;
    let (tx, ty) = revive::tile_corner(x, y);
    if (tx, ty) == (x, y) {
        return Ok(());
    }
    let _: () = jvm.invoke_virtual(player, "v", "()V", ()).await?;
    jvm.put_field_from(player, "er", "h", "S", tx).await?;
    jvm.put_field_from(player, "er", "i", "S", ty).await?;
    let _: () = jvm.invoke_virtual(player, "E", "()V", ()).await?;
    let _: () = jvm.invoke_virtual(player, "u", "()V", ()).await?;
    tracing::info!("hz: revive: fell between tiles at ({x},{y}) -> stands on ({tx},{ty})");
    Ok(())
}

/// Also HZ-057's clock: the reflect stance counts the field's ticks here; and HZ-059 straightens a
/// diagonal facing left outside a walk.
struct AutoLoot(Arc<GameOptions>, Arc<Boss>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for AutoLoot {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        self.1.tick();
        if self.1.state.patch_on("HZ-059") {
            straighten_facing(jvm).await?;
        }
        if self.1.state.story_waiting() {
            story_tick(jvm, &self.1.state).await?;
        }
        let mut args = args.into_vec().into_iter();
        let (g, x, y) = (object(args.next().unwrap()), int(args.next().unwrap()), int(args.next().unwrap()));
        let Some(g) = g else {
            return Err(null_pointer(jvm, "Hooks.autoLoot: g").await);
        };

        // The original: what lies on the player's own tile.
        let picked: bool = jvm
            .invoke_virtual(&g, "a", "(BB)Z", [JavaValue::Byte(x as i8), JavaValue::Byte(y as i8)])
            .await?;
        if picked || !self.0.auto_loot() {
            return Ok(JavaValue::Boolean(picked));
        }
        let Some((tx, ty)) = nearest_lootable(jvm, &g, x, y).await? else {
            return Ok(JavaValue::Boolean(false));
        };
        let picked: bool = jvm.invoke_virtual(&g, "a", "(BB)Z", [JavaValue::Byte(tx), JavaValue::Byte(ty)]).await?;
        tracing::info!("hz: auto-loot ({tx},{ty}) from ({x},{y}) -> {picked}");
        Ok(JavaValue::Boolean(picked))
    }
}

/// The tile of the nearest ground entry within [`AUTO_LOOT_RADIUS`] that `g.a(BB)Z` would take
/// without refusing: money, or an item its bag has room for (`bt.a(BBI)Z`, read-only). The
/// player's own tile is skipped — the original call just tried it.
#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
async fn nearest_lootable(jvm: &Jvm, g: &Box<dyn ClassInstance>, x: i32, y: i32) -> JvmResult<Option<(i8, i8)>> {
    let Some(ground) = object(jvm.get_field(g, "b", "Ljava/util/Vector;").await?) else {
        return Ok(None);
    };
    let n: i32 = jvm.invoke_virtual(&ground, "size", "()I", ()).await?;
    // Tile coordinates are bytes; compare them unsigned so a map past column 127 still measures.
    let (px, py) = (x & 0xff, y & 0xff);
    let mut best: Option<(i32, i8, i8)> = None;
    for i in 0..n {
        let Some(entry) = object(jvm.invoke_virtual(&ground, "elementAt", "(I)Ljava/lang/Object;", (i,)).await?) else {
            continue;
        };
        let e: Vec<i8> = jvm.load_array(&entry, 0, 6).await?;
        let d = ((e[0] as i32 & 0xff) - px).abs().max(((e[1] as i32 & 0xff) - py).abs());
        if d == 0 || d > AUTO_LOOT_RADIUS || best.is_some_and(|(bd, _, _)| bd <= d) {
            continue;
        }
        if e[2] != 1 {
            // An item: ed.a(B)Lbt; is the bag for its category, bt.a(BBI)Z "has room for 1".
            let bag = object(jvm.invoke_static("ed", "a", "(B)Lbt;", [JavaValue::Byte(e[3])]).await?);
            let Some(bag) = bag else { continue };
            let fits: bool = jvm
                .invoke_virtual(&bag, "a", "(BBI)Z", [JavaValue::Byte(e[3]), JavaValue::Byte(e[4]), JavaValue::Int(1)])
                .await?;
            if !fits {
                continue;
            }
        }
        best = Some((d, e[0], e[1]));
    }
    Ok(best.map(|(_, tx, ty)| (tx, ty)))
}

/// HZ-007's page state, shared by its three hooks: the scroll (the first row on screen — `o.a:B`
/// stays the cursor's slot on screen, which the game's frame, its cursor bar `b(dd)` and the touch
/// layer's row taps all read), and whether [`OptKey`] is running the game's own row code.
struct OptPage {
    options: Arc<GameOptions>,
    scroll: std::sync::atomic::AtomicUsize,
    in_game_code: std::sync::atomic::AtomicBool,
}

impl OptPage {
    fn scroll(&self, visible: usize) -> usize {
        self.scroll.load(std::sync::atomic::Ordering::Relaxed).min(OPT_ROWS.len().saturating_sub(visible))
    }

    fn set_scroll(&self, scroll: usize) {
        self.scroll.store(scroll, std::sync::atomic::Ordering::Relaxed);
    }
}

/// The window's facts: the rows it shows at once — in game (`bn.e:Z`, its own field, which shadows
/// `o.e:Z`) or not — with its row count `o.b:B` made that (the constructor's is the in-game one), and
/// the cursor's slot on screen.
async fn opt_window(jvm: &Jvm, this: &mut Box<dyn ClassInstance>) -> JvmResult<(usize, usize)> {
    let in_game: bool = jvm.get_field_from(this, "bn", "e", "Z").await?;
    let visible = visible_rows(in_game);
    let rows: i8 = jvm.get_field(this, "b", "B").await?;
    if rows as usize != visible {
        jvm.put_field(this, "b", "B", visible as i8).await?;
    }
    let slot: i8 = jvm.get_field(this, "a", "B").await?;
    Ok((visible, (slot.max(0) as usize).min(visible - 1)))
}

/// A string of the game's common text table (`rpg/e.b` = `t/com.tdf`, `cx.a(I)[B`).
async fn com_text(jvm: &Jvm, index: i32) -> JvmResult<Box<dyn ClassInstance>> {
    let table = object(jvm.get_static_field("rpg/e", "b", "Lcx;").await?);
    let Some(table) = table else {
        return Err(null_pointer(jvm, "rpg/e.b").await);
    };
    match object(jvm.invoke_virtual(&table, "a", "(I)[B", (index,)).await?) {
        Some(bytes) => Ok(bytes),
        None => Err(null_pointer(jvm, "cx.a").await),
    }
}

/// A game row's value as the game draws it (`bn.a(Ldd;)V` @149..481): 켜기 / 끄기 of its switch, the
/// speed's name, 시선이동's number (끄기 at 0).
async fn game_value(jvm: &Jvm, row: u8) -> JvmResult<Box<dyn ClassInstance>> {
    let switch = |on: bool| if on { COM_ON } else { COM_OFF };
    let index = match row {
        0 => switch(jvm.get_static_field("rpg/c", "b", "Z").await?),
        1 => switch(jvm.get_static_field("rpg/c", "c", "Z").await?),
        2 => COM_SPEED + i32::from(jvm.get_static_field::<i8>("rpg/c", "b", "B").await?.clamp(0, 3)),
        3 => match jvm.get_static_field::<i8>("rpg/c", "c", "B").await? {
            0 => COM_OFF,
            n => return byte_array(jvm, n.to_string().as_bytes()).await,
        },
        4 => switch(jvm.get_static_field("rpg/c", "a", "Z").await?),
        _ => switch(jvm.get_static_field("rpg/c", "d", "Z").await?),
    };
    com_text(jvm, index).await
}

/// HZ-007 `optLabel(Object bn, int index)[B` — `index` 7 is the title (the game's own), 16 + i slot
/// i's label: the row scrolled there, and under the rows 「다음 실행부터 적용」 while an orientation
/// change waits.
struct OptLabel(Arc<OptPage>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for OptLabel {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let mut args = args.into_vec().into_iter();
        let (this, index) = (object(args.next().unwrap()), int(args.next().unwrap()));
        let Some(mut this) = this else {
            return Err(null_pointer(jvm, "Hooks.optLabel: this").await);
        };
        if index < FIRST_LABEL {
            return Ok(JavaValue::Object(Some(com_text(jvm, index).await?)));
        }
        let (visible, _) = opt_window(jvm, &mut this).await?;
        let slot = (index - FIRST_LABEL) as usize;
        let array = match OPT_ROWS.get(self.0.scroll(visible) + slot).filter(|_| slot < visible) {
            Some(OptRow::Game(r)) => com_text(jvm, FIRST_LABEL + i32::from(*r)).await?,
            Some(OptRow::Host(row)) => byte_array(jvm, text::label(*row)).await?,
            None if slot == visible && self.0.options.orientation_pending() => byte_array(jvm, text::NEXT_LAUNCH).await?,
            None => byte_array(jvm, &[]).await?,
        };
        Ok(JavaValue::Object(Some(array)))
    }
}

/// The scroll bar's colours: its track the game's row plate, its thumb the parchment of its letters.
const SCROLL_TRACK: i32 = 0x36_2e23;
const SCROLL_THUMB: i32 = 0xc0_b890;

/// HZ-007 `optValues(Object bn, Object dd, int y0)I`: every value on screen — the cursor's white, the
/// rest the game's dim — and the bar that says where in the list they are; `−1` skips the game's.
struct OptValues(Arc<OptPage>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for OptValues {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let mut args = args.into_vec().into_iter();
        let (this, dd, y0) = (object(args.next().unwrap()), object(args.next().unwrap()), int(args.next().unwrap()));
        let (Some(mut this), Some(dd)) = (this, dd) else {
            return Err(null_pointer(jvm, "Hooks.optValues").await);
        };
        // What the replaced bytes computed: rpg/a.c + 24 + 5 + 8.
        let centre: i32 = jvm.get_static_field("rpg/a", "c", "I").await?;
        let x = centre + 37;
        let (visible, slot) = opt_window(jvm, &mut this).await?;
        let scroll = self.0.scroll(visible);
        for (i, row) in OPT_ROWS.iter().enumerate().skip(scroll).take(visible) {
            let at = i - scroll;
            let colour = if at == slot { WHITE } else { DIM };
            let _: () = jvm.invoke_virtual(&dd, "a", "(I)V", (colour,)).await?;
            let bytes = match *row {
                OptRow::Game(r) => game_value(jvm, r).await?,
                OptRow::Host(h) => byte_array(jvm, &value(&self.0.options, h)).await?,
            };
            // As the game draws a value: centred (last arg 1) on x, a pixel below the label.
            let _: () = jvm
                .invoke_static(
                    "ah",
                    "a",
                    "(Ldd;II[BII)V",
                    [
                        JavaValue::Object(Some(dd.clone())),
                        JavaValue::Int(x),
                        JavaValue::Int(y0 + 15 * at as i32 + 1),
                        JavaValue::Object(Some(bytes)),
                        JavaValue::Int(0),
                        JavaValue::Int(1),
                    ],
                )
                .await?;
        }
        // The bar, inside the list frame's right edge (158 wide round rpg/a.c): the track the rows'
        // height (`dd.a(IIII)V` fills), the thumb the share on screen.
        if OPT_ROWS.len() > visible {
            let (bx, track_h, total) = (centre + 73, 15 * visible as i32 - 1, OPT_ROWS.len() as i32);
            let (top, len) = (track_h * scroll as i32 / total, (track_h * visible as i32 / total).max(4));
            for (colour, y, h) in [(SCROLL_TRACK, y0, track_h), (SCROLL_THUMB, y0 + top, len)] {
                let _: () = jvm.invoke_virtual(&dd, "a", "(I)V", (colour,)).await?;
                let _: () = jvm.invoke_virtual(&dd, "a", "(IIII)V", (bx, y, 3, h)).await?;
            }
        }
        // -1: the patched `iflt` skips the game's values.
        Ok(JavaValue::Int(-1))
    }
}

/// HZ-007 `optKey(Object bn, int action, int raw)Z`, in place of `o.b(II)Z`: the child window first,
/// as the game; then UP / DOWN walk the cursor through the whole list, scrolling it; LEFT / RIGHT /
/// OK change the cursor's row — a host row here, a game row by the game's own code (OK as RIGHT:
/// the game's OK did nothing to its rows, and a tap on a row's name is OK); RSK is the game's (it
/// writes the options and closes the window). `true` = consumed.
struct OptKey(Arc<OptPage>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for OptKey {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        use std::sync::atomic::Ordering::Relaxed;
        let mut args = args.into_vec().into_iter();
        let (this, action, raw) = (object(args.next().unwrap()), int(args.next().unwrap()), int(args.next().unwrap()));
        let Some(mut this) = this else {
            return Err(null_pointer(jvm, "Hooks.optKey: this").await);
        };

        // The replaced call first: a child window gets the key before this one. Inside the game's
        // own row code (below) that is all: its key handling runs on.
        let child: bool = jvm.invoke_virtual(&this, "b", "(II)Z", (action, raw)).await?;
        if child || self.0.in_game_code.load(Relaxed) {
            return Ok(JavaValue::Boolean(child));
        }
        let (visible, slot) = opt_window(jvm, &mut this).await?;
        let scroll = self.0.scroll(visible);
        let cursor = (scroll + slot).min(OPT_ROWS.len() - 1);
        let consumed = match classify(action, raw) {
            Key::Rsk => {
                // The game writes the options and closes: the next window opens at the top.
                self.0.set_scroll(0);
                false
            }
            Key::Move => {
                let up = action == 1 || raw == 50;
                let n = OPT_ROWS.len();
                let to = if up { (cursor + n - 1) % n } else { (cursor + 1) % n };
                let scroll = scroll_to(to, scroll, visible);
                self.0.set_scroll(scroll);
                jvm.put_field(&mut this, "a", "B", (to - scroll) as i8).await?;
                true
            }
            key @ (Key::Left | Key::Right | Key::Fire) => {
                match OPT_ROWS[cursor] {
                    OptRow::Host(row) => self.0.options.step(row, if key == Key::Left { -1 } else { 1 }),
                    OptRow::Game(r) => {
                        // The game's row code reads the row from o.a:B: the game's number for the
                        // one call, then the slot back. OK goes in as RIGHT ('6').
                        let (a, k) = if key == Key::Fire { (5, 54) } else { (action, raw) };
                        jvm.put_field(&mut this, "a", "B", r as i8).await?;
                        self.0.in_game_code.store(true, Relaxed);
                        let done: JvmResult<bool> = jvm.invoke_virtual(&this, "a", "(II)Z", (a, k)).await;
                        self.0.in_game_code.store(false, Relaxed);
                        jvm.put_field(&mut this, "a", "B", slot as i8).await?;
                        done?;
                    }
                }
                true
            }
            // LSK and the rest: nothing, as on the game's rows.
            Key::Other => true,
        };
        Ok(JavaValue::Boolean(consumed))
    }
}

/// HZ-017 `monsterInit(Object cq)V`, in place of `this.q()V` at the end of `cq.i()V`.
struct MonsterInit(Arc<GameOptions>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for MonsterInit {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(mut this) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.monsterInit").await);
        };
        // The replaced call: virtual, so the boss base `ak`'s empty q() stays empty.
        let _: () = jvm.invoke_virtual(&this, "q", "()V", ()).await?;
        let pct = self.0.difficulty_pct();
        if pct == 100 {
            return Ok(JavaValue::Void);
        }
        // cq's own fields: 33 subclass fields shadow a superclass's (CLAUDE.md, 2026-10-24).
        let hp: i32 = jvm.get_field_from(&this, "cq", "h", "I").await?;
        let attack: i16 = jvm.get_field_from(&this, "cq", "f", "S").await?;
        let (hp2, attack2) = (scale_hp(hp, pct), scale_attack(attack, pct));
        jvm.put_field_from(&mut this, "cq", "h", "I", hp2).await?;
        jvm.put_field_from(&mut this, "cq", "i", "I", hp2).await?;
        jvm.put_field_from(&mut this, "cq", "f", "S", attack2).await?;
        let kind: i16 = jvm.get_field_from(&this, "cq", "c", "S").await?;
        tracing::debug!("hz: monster {} kind {kind} HP {hp} -> {hp2}, attack {attack} -> {attack2} ({pct} %)", this.class_definition().name());
        Ok(JavaValue::Void)
    }
}

/// HZ-018 `respawnDelay(Object eq)I`, in place of `getfield eq.a:S` in `cq.g()V`.
struct RespawnDelay(Arc<GameOptions>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for RespawnDelay {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(kind) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.respawnDelay").await);
        };
        let delay: i16 = jvm.get_field(&kind, "a", "S").await?;
        let rate = self.0.respawn_rate();
        let out = respawn_delay(delay, rate);
        tracing::debug!("hz: respawn in {delay} ticks x{rate} = {out}");
        Ok(JavaValue::Int(out as i32))
    }
}

/// The game's replay round `ed.h`: 0 노말 · 1 하드코어 · 2 헬 · 3 가디언 모드 (HZ-030).
async fn replay_round(jvm: &Jvm) -> JvmResult<i8> {
    jvm.get_static_field("ed", "h", "B").await
}

/// HZ-030 `guardianMonster(Object cq)V`, in place of `this.p()V` in `cq.i()V`.
struct GuardianMonster;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for GuardianMonster {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(mut this) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.guardianMonster").await);
        };
        // The replaced call: the kind's record into the monster (virtual, as the game made it).
        let _: () = jvm.invoke_virtual(&this, "p", "()V", ()).await?;
        if replay_round(jvm).await? < guardian_mode::ROUND {
            return Ok(JavaValue::Void);
        }
        // cq's own fields: 33 subclass fields shadow a superclass's (CLAUDE.md, 2026-10-24).
        let base = guardian_mode::Monster {
            level: jvm.get_field_from(&this, "cq", "n", "B").await?,
            hp: jvm.get_field_from(&this, "cq", "h", "I").await?,
            defence: jvm.get_field_from(&this, "cq", "d", "S").await?,
            spirit_defence: jvm.get_field_from(&this, "cq", "e", "S").await?,
            attack: jvm.get_field_from(&this, "cq", "f", "S").await?,
            evasion: jvm.get_field_from(&this, "cq", "g", "S").await?,
        };
        let Some(m) = guardian_mode::scale(base) else {
            return Ok(JavaValue::Void);
        };
        jvm.put_field_from(&mut this, "cq", "n", "B", m.level).await?;
        jvm.put_field_from(&mut this, "cq", "h", "I", m.hp).await?;
        jvm.put_field_from(&mut this, "cq", "i", "I", m.hp).await?;
        jvm.put_field_from(&mut this, "cq", "d", "S", m.defence).await?;
        jvm.put_field_from(&mut this, "cq", "e", "S", m.spirit_defence).await?;
        jvm.put_field_from(&mut this, "cq", "f", "S", m.attack).await?;
        jvm.put_field_from(&mut this, "cq", "g", "S", m.evasion).await?;
        let kind: i16 = jvm.get_field_from(&this, "cq", "c", "S").await?;
        tracing::debug!(
            "hz: 가디언 monster kind {kind} LV {} -> {}, HP {} -> {}, attack {} -> {}, defence {} -> {}",
            base.level,
            m.level,
            base.hp,
            m.hp,
            base.attack,
            m.attack,
            base.defence,
            m.defence
        );
        Ok(JavaValue::Void)
    }
}

/// HZ-030 `monsterRound()I`, in place of both `getstatic ed.h:B` in `cq.i()V`.
struct MonsterRound;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for MonsterRound {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, _: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        Ok(JavaValue::Int(guardian_mode::game_round(replay_round(jvm).await?).into()))
    }
}

/// HZ-030 `questMoney(I)V`, in place of `ed.e(I)V` in script action 17 (the quest's money, by round).
struct QuestMoney;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for QuestMoney {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let amount = int(args[0].clone());
        let paid = if replay_round(jvm).await? >= guardian_mode::ROUND { guardian_mode::quest(amount) } else { amount };
        if paid != amount {
            tracing::info!("hz: 가디언 quest money {amount} -> {paid}");
        }
        let _: () = jvm.invoke_static("ed", "e", "(I)V", (paid,)).await?;
        Ok(JavaValue::Void)
    }
}

/// HZ-030 `questExp(Object ek, I)V`, in place of `ek.j(I)V` in script action 20 (the quest's EXP, by
/// round). HZ-004's rate still applies inside `ek.j`.
struct QuestExp;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for QuestExp {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let mut args = args.into_vec().into_iter();
        let (player, amount) = (object(args.next().unwrap()), int(args.next().unwrap()));
        let Some(player) = player else {
            return Err(null_pointer(jvm, "Hooks.questExp").await);
        };
        let paid = if replay_round(jvm).await? >= guardian_mode::ROUND { guardian_mode::quest(amount) } else { amount };
        if paid != amount {
            tracing::info!("hz: 가디언 quest EXP {amount} -> {paid}");
        }
        let _: () = jvm.invoke_virtual(&player, "j", "(I)V", (paid,)).await?;
        Ok(JavaValue::Void)
    }
}

/// HZ-030 `roundText(Object cx, I)[B`, in place of `cx.a(I)[B` where the ending letters 「… 모드로」.
struct RoundText;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for RoundText {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let mut args = args.into_vec().into_iter();
        let (table, index) = (object(args.next().unwrap()), int(args.next().unwrap()));
        let Some(table) = table else {
            return Err(null_pointer(jvm, "Hooks.roundText").await);
        };
        if let Some(line) = guardian_mode::ending_text(index, replay_round(jvm).await?) {
            tracing::info!("hz: ending -> 가디언 모드");
            return Ok(JavaValue::Object(Some(byte_array(jvm, &line).await?)));
        }
        jvm.invoke_virtual(&table, "a", "(I)[B", (index,)).await
    }
}

/// HZ-031 `gradeColor(int[] table, Object bx)I`, in place of `getfield bx.e:B; iaload` where an item's
/// name takes its grade's colour.
struct GradeColor;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for GradeColor {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let mut args = args.into_vec().into_iter();
        let (table, item) = (object(args.next().unwrap()), object(args.next().unwrap()));
        let (Some(table), Some(item)) = (table, item) else {
            return Err(null_pointer(jvm, "Hooks.gradeColor").await);
        };
        let grade: i8 = jvm.get_field_from(&item, "bx", "e", "B").await?;
        if grade == hooks::LEGEND_GRADE {
            return Ok(JavaValue::Int(hooks::LEGEND_COLOUR));
        }
        // The original's `iaload`: out of range throws, as it did.
        let mut colour: Vec<i32> = jvm.load_array(&table, grade.max(0) as usize, 1).await?;
        Ok(JavaValue::Int(colour.pop().unwrap_or_default()))
    }
}

/// `t/it` [28..31] = 일반 · 레어 · 에픽 · 세트.
const GRADE_NAMES: i32 = 28;

/// HZ-031 `gradeName(Object cx, int index)[B`, in place of `cx.a(I)[B` where the item window letters
/// the grade (`t/it` [28 + grade]).
struct GradeName;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for GradeName {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let mut args = args.into_vec().into_iter();
        let (table, index) = (object(args.next().unwrap()), int(args.next().unwrap()));
        let Some(table) = table else {
            return Err(null_pointer(jvm, "Hooks.gradeName").await);
        };
        if index == GRADE_NAMES + i32::from(hooks::LEGEND_GRADE) {
            let name = crate::text::euc_kr(legend::GRADE_NAME).expect("in the game's font");
            return Ok(JavaValue::Object(Some(byte_array(jvm, &name).await?)));
        }
        jvm.invoke_virtual(&table, "a", "(I)[B", (index,)).await
    }
}

/// HZ-031: the item window's effect lines, under its option rows — x from the frame's left, y from
/// the `y` the window gives its contents (the frame's top + 14), a line apart (probe 520x240).
const INFO_TEXT_X: i32 = 8;
const INFO_TEXT_Y: i32 = 118;
const INFO_LINE: i32 = 15;
/// The lines' ink: dark brown on the window's parchment (the grade's gold is for the dark name rows).
const INFO_INK: i32 = 0x80_30_00;

/// HZ-031 `itemInfo(Object cz, Object dd, int x, int y, Object ba)V`, in place of `q.b(Ldd;IILba;)V`
/// in the item window `cz.a(dd)V`.
struct ItemInfo;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for ItemInfo {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let args = args.into_vec();
        let Some(window) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.itemInfo").await);
        };
        let _: () = jvm.invoke_virtual(&window, "b", "(Ldd;IILba;)V", args[1..].to_vec()).await?;
        let (x, y) = (int(args[2].clone()), int(args[3].clone()));
        let (Some(dd), Some(item)) = (object(args[1].clone()), object(args[4].clone())) else {
            return Ok(JavaValue::Void);
        };
        let cat: i8 = jvm.get_field_from(&item, "ba", "a", "B").await?;
        let id: i8 = jvm.get_field_from(&item, "ba", "b", "B").await?;
        let Some(piece) = legend::find(cat, id) else {
            return Ok(JavaValue::Void);
        };
        let _: () = jvm.invoke_virtual(&dd, "a", "(I)V", (INFO_INK,)).await?;
        for (i, line) in legend::lines(piece).into_iter().enumerate() {
            let text = byte_array(jvm, &line).await?;
            let at = [
                args[1].clone(),
                JavaValue::Int(x + INFO_TEXT_X),
                JavaValue::Int(y + INFO_TEXT_Y + INFO_LINE * i as i32),
                JavaValue::Object(Some(text)),
                JavaValue::Int(0),
            ];
            let _: () = jvm.invoke_static("ah", "a", "(Ldd;II[BI)V", at).await?;
        }
        Ok(JavaValue::Void)
    }
}

/// HZ-032's trace for the item test (`tools/hz_item_test.py`): what an effect hook got from the game and
/// what it handed back, at info level while the bench is on (the probe's `HZ_BENCH`; the app never sets it).
macro_rules! fx {
    ($($t:tt)*) => {
        if effects::bench() {
            tracing::info!("hz: fx {}", format_args!($($t)*));
        }
    };
}

/// HZ-031 `monsterDrop(Object cq)V`, in place of `this.o()V` in a monster's death `cq.g()V`.
struct MonsterDrop {
    options: Arc<GameOptions>,
    state: Arc<GuestState>,
}

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for MonsterDrop {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(monster) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.monsterDrop").await);
        };
        // The replaced call: the game's own drop.
        let _: () = jvm.invoke_virtual(&monster, "o", "()V", ()).await?;
        let round = replay_round(jvm).await?;
        // 가디언 모드: the 85제 this monster drops — by its drop slots and level, as the other modes' drops (legend::pool).
        let legends = if round >= guardian_mode::ROUND && self.state.patch_on("HZ-031") { legend_pool(jvm, &monster).await? } else { Vec::new() };
        // HZ-046: a boss drops one piece of its level band (the game's ak.o() drops nothing); a head also
        // pays HZ-044's skill point.
        if self.state.patch_on("HZ-046") && jvm.is_instance(&*monster, "ak") {
            let kind: i16 = jvm.get_field_from(&monster, "cq", "c", "S").await?;
            let level: i8 = jvm.get_field_from(&monster, "cq", "n", "B").await?;
            let x: i8 = jvm.get_field_from(&monster, "er", "s", "B").await?;
            let y: i8 = jvm.get_field_from(&monster, "er", "t", "B").await?;
            if let Some(roll) = draw(jvm).await?
                && let Some((cat, id)) = balance::boss_drop(level.into(), roll)
                && let Some(map) = object(jvm.get_static_field("ed", "a", "Lg;").await?)
            {
                let _: () = jvm.invoke_virtual(&map, "a", "(BBBB)V", [JavaValue::Byte(x), JavaValue::Byte(y), JavaValue::Byte(cat), JavaValue::Byte(id)]).await?;
                tracing::info!("hz: boss kind {kind} LV {level} drops ({cat}, {id}) at {x},{y}");
            }
            if round >= guardian_mode::ROUND
                && self.state.patch_on("HZ-031")
                && let Some(roll) = draw(jvm).await?
                && roll.unsigned_abs() % 100 < balance::BOSS_LEGEND_PCT
                && let Some(piece) = legend::boss_pick(roll, &legends)
                && let Some(map) = object(jvm.get_static_field("ed", "a", "Lg;").await?)
            {
                let _: () = jvm.invoke_virtual(&map, "a", "(BBBB)V", [JavaValue::Byte(x), JavaValue::Byte(y), JavaValue::Byte(piece.category), JavaValue::Byte(piece.id)]).await?;
                tracing::info!("hz: 가디언 boss kind {kind} LV {level} drops 85제 {} ({}, {})", piece.name, piece.category, piece.id);
            }
            if self.state.patch_on("HZ-044")
                && balance::is_boss_head(kind)
                && let Some(mut player) = player(jvm).await?
            {
                let points: i16 = jvm.get_field_from(&player, "ek", "b", "S").await?;
                jvm.put_field_from(&mut player, "ek", "b", "S", points.saturating_add(1)).await?;
                tracing::info!("hz: boss head {kind}: skill point {points} -> {}", points.saturating_add(1));
            }
        }
        if self.state.patch_on("HZ-033") && round < drops::EXTRA_ROUND {
            let level: i8 = jvm.get_field_from(&monster, "cq", "n", "B").await?;
            if let Some(roll) = draw(jvm).await?
                && let Some((cat, id)) = drops::epic(level.into(), round, roll)
                && let Some(map) = object(jvm.get_static_field("ed", "a", "Lg;").await?)
            {
                let x: i8 = jvm.get_field_from(&monster, "er", "s", "B").await?;
                let y: i8 = jvm.get_field_from(&monster, "er", "t", "B").await?;
                let _: () = jvm.invoke_virtual(&map, "a", "(BBBB)V", [JavaValue::Byte(x), JavaValue::Byte(y), JavaValue::Byte(cat), JavaValue::Byte(id)]).await?;
                tracing::info!("hz: drop: an 에픽/세트 ({cat}, {id}) from a LV {level}");
            }
        }
        // 행운의 반지 (the second draft): the 85제 and recipe chances × 2 while worn.
        let luck = if self.state.patch_on("HZ-032") && round >= guardian_mode::ROUND {
            effects::luck(&worn_now(jvm).await?)
        } else {
            1
        };
        fx!("drop luck {luck} round {round}");
        if self.state.patch_on("HZ-035") && self.state.patch_on("HZ-031") && round >= guardian_mode::ROUND {
            if let Some(roll) = draw(jvm).await?
                && let Some(id) = legend::recipe_roll(roll, luck, &legends)
                && let Some(map) = object(jvm.get_static_field("ed", "a", "Lg;").await?)
            {
                let x: i8 = jvm.get_field_from(&monster, "er", "s", "B").await?;
                let y: i8 = jvm.get_field_from(&monster, "er", "t", "B").await?;
                let _: () = jvm.invoke_virtual(&map, "a", "(BBBB)V", [JavaValue::Byte(x), JavaValue::Byte(y), JavaValue::Byte(18), JavaValue::Byte(id)]).await?;
                let makes = legend::recipe(id).map_or("?", |r| r.makes.name);
                tracing::info!("hz: 85제 recipe {id} ({makes}) drops from {}", describe_monster(jvm, &monster).await?);
            }
        }
        if !self.state.patch_on("HZ-031") || round < guardian_mode::ROUND {
            return Ok(JavaValue::Void);
        }
        let (Some(random), Some(map)) = (
            object(jvm.get_static_field("ar", "a", "Ljava/util/Random;").await?),
            object(jvm.get_static_field("ed", "a", "Lg;").await?),
        ) else {
            return Ok(JavaValue::Void);
        };
        let roll: i32 = jvm.invoke_virtual(&random, "nextInt", "()I", ()).await?;
        let Some(piece) = legend::roll(roll, self.options.difficulty_pct() >= 200, luck, &legends) else {
            return Ok(JavaValue::Void);
        };
        let x: i8 = jvm.get_field_from(&monster, "er", "s", "B").await?;
        let y: i8 = jvm.get_field_from(&monster, "er", "t", "B").await?;
        let at = [JavaValue::Byte(x), JavaValue::Byte(y), JavaValue::Byte(piece.category), JavaValue::Byte(piece.id)];
        let _: () = jvm.invoke_virtual(&map, "a", "(BBBB)V", at).await?;
        tracing::info!("hz: 85제 drop {} ({}, {}) at {x},{y} from {}", piece.name, piece.category, piece.id, describe_monster(jvm, &monster).await?);
        Ok(JavaValue::Void)
    }
}

/// HZ-031: what a 가디언 모드 monster can drop of the 85제 ([`legend::pool`]) — the categories of its drop block
/// (`cq.a:Leq;` → `eq.b:[B`, what `cq.o()V` reads) at its level (`cq.n:B`, `cq.a()B`). Kind 0 drops nothing (`cq.o()V` @4).
#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
async fn legend_pool(jvm: &Jvm, monster: &Box<dyn ClassInstance>) -> JvmResult<Vec<&'static legend::Legend>> {
    let kind: i16 = jvm.get_field_from(monster, "cq", "c", "S").await?;
    if kind == 0 {
        return Ok(Vec::new());
    }
    let Some(kind_data) = object(jvm.get_field_from(monster, "cq", "a", "Leq;").await?) else {
        return Ok(Vec::new());
    };
    let Some(block) = object(jvm.get_field_from(&kind_data, "eq", "b", "[B").await?) else {
        return Ok(Vec::new());
    };
    let n = jvm.array_length(&block).await?.min(24);
    let bytes: Vec<i8> = jvm.load_array(&block, 0, n).await?;
    let level: i8 = jvm.get_field_from(monster, "cq", "n", "B").await?;
    Ok(legend::pool(legend::slot_categories(&bytes), level.into()))
}

/// `kind K LV n` of a monster, for the drop logs.
#[allow(clippy::borrowed_box)]
async fn describe_monster(jvm: &Jvm, monster: &Box<dyn ClassInstance>) -> JvmResult<String> {
    let kind: i16 = jvm.get_field_from(monster, "cq", "c", "S").await?;
    let level: i8 = jvm.get_field_from(monster, "cq", "n", "B").await?;
    Ok(format!("kind {kind} LV {level}"))
}

/// HZ-032: what the effect hooks share — the boot's state (drawn frames) and the host's memory between
/// hooks (`effects::Live`, nothing of it saved).
struct Fx {
    state: Arc<GuestState>,
    live: std::sync::Mutex<effects::Live>,
}

impl Fx {
    fn live(&self) -> std::sync::MutexGuard<'_, effects::Live> {
        self.live.lock().unwrap()
    }
}

/// HZ-032: the player (`ed.a:Lek;`).
async fn player(jvm: &Jvm) -> JvmResult<Option<Box<dyn ClassInstance>>> {
    Ok(object(jvm.get_static_field("ed", "a", "Lek;").await?))
}

/// HZ-032: the effects of the 85제 pieces the player wears (`ek.a:[Lbx;`, seven slots).
async fn worn(jvm: &Jvm, player: &Box<dyn ClassInstance>) -> JvmResult<effects::Worn> {
    let Some(slots) = object(jvm.get_field_from(player, "ek", "a", "[Lbx;").await?) else {
        return Ok(effects::Worn::default());
    };
    let n = jvm.array_length(&slots).await?;
    let items: Vec<Option<Box<dyn ClassInstance>>> = jvm.load_array(&slots, 0, n).await?;
    let mut out = Vec::new();
    for item in items.into_iter().flatten() {
        let id: i8 = jvm.get_field_from(&item, "ba", "b", "B").await?;
        if id < hooks::LEGEND_FIRST_ID {
            continue;
        }
        let cat: i8 = jvm.get_field_from(&item, "ba", "a", "B").await?;
        if let Some(piece) = legend::find(cat, id) {
            out.push(piece.effect);
        }
    }
    Ok(effects::Worn(out))
}

/// HZ-032: the job (`ed.b:B`).
async fn job(jvm: &Jvm) -> JvmResult<i8> {
    jvm.get_static_field("ed", "b", "B").await
}

/// HZ-032: what the player does and has up, for the percent tables.
async fn moment(jvm: &Jvm, player: &Box<dyn ClassInstance>, fx: &Fx, w: &effects::Worn) -> JvmResult<effects::Moment> {
    let queue: Option<Box<dyn ClassInstance>> = object(jvm.get_field_from(player, "ek", "ac", "[B").await?);
    let at: i8 = jvm.get_field_from(player, "ek", "l", "B").await?;
    let action = match queue {
        Some(q) if at >= 0 && (at as usize) < jvm.array_length(&q).await? => jvm.load_array::<i8>(&q, at as usize, 1).await?.pop().unwrap_or(0),
        _ => 0,
    };
    let frames = fx.state.frames();
    let gear_count = if w.has(legend::Effect::Gravity) { gear_count(jvm, fx, frames).await? } else { 0 };
    let (volley_until, skill_guard_until, kill_guard_until, ambush_until) = {
        let live = fx.live();
        (live.volley_until, live.skill_guard_until, live.kill_guard_until, live.ambush_until)
    };
    let wil = if w.has(legend::Effect::SpiritRing) { stat(jvm, player, 3).await? } else { 0 };
    // HP and SP now and at their tops (`ek.a:[I` / `ek.b:[I`, 0 HP · 1 SP).
    let (hp_pct, sp_full) = match (object(jvm.get_field_from(player, "ek", "a", "[I").await?), object(jvm.get_field_from(player, "ek", "b", "[I").await?)) {
        (Some(now), Some(top)) => {
            let now: Vec<i32> = jvm.load_array(&now, 0, 2).await?;
            let top: Vec<i32> = jvm.load_array(&top, 0, 2).await?;
            let pct = |i: usize| if top.get(i).copied().unwrap_or(0) > 0 { (i64::from(now[i]) * 100 / i64::from(top[i])) as i32 } else { 100 };
            (pct(0), top.get(1).copied().unwrap_or(0) > 0 && now.get(1).copied().unwrap_or(0) >= top[1])
        }
        _ => (100, false),
    };
    Ok(effects::Moment {
        job: job(jvm).await?,
        action,
        transformed: jvm.get_field_from::<i32>(player, "ek", "b", "I").await? > 0,
        clone: object(jvm.get_static_field("ed", "a", "Lh;").await?).is_some(),
        counter: jvm.get_field_from(player, "ek", "g", "Z").await?,
        volley_guard: frames < volley_until,
        skill_guard: frames < skill_guard_until,
        kill_guard: frames < kill_guard_until,
        gear_count,
        wil,
        ambush: frames < ambush_until,
        hp_pct,
        sp_full,
    })
}

/// HZ-032: `ek.a:[I[index] += top × pct / 100` (0 HP · 1 SP · 3 MP), kept within the top; `why` names it in
/// the trace.
async fn add_stat_pct(jvm: &Jvm, player: &Box<dyn ClassInstance>, index: usize, pct: i32, why: &str) -> JvmResult<()> {
    let Some(top) = object(jvm.get_field_from(player, "ek", "b", "[I").await?) else {
        return Ok(());
    };
    if jvm.array_length(&top).await? <= index {
        return Ok(());
    }
    let max: i32 = jvm.load_array::<i32>(&top, index, 1).await?.pop().unwrap_or_default();
    add_stat(jvm, player, index, effects::scale(max, pct).max(1), why).await
}

/// HZ-032: `frames` off every cooling skill (`ed.a:[I`, the four cooldown cells).
async fn shave_cooldowns(jvm: &Jvm, frames: i32, why: &str) -> JvmResult<()> {
    if let Some(mut cooling) = object(jvm.get_static_field("ed", "a", "[I").await?) {
        let n = jvm.array_length(&cooling).await?.min(4);
        let now: Vec<i32> = jvm.load_array(&cooling, 0, n).await?;
        let after: Vec<i32> = now.iter().map(|c| (c - frames).max(0)).collect();
        fx!("shave {why} {frames} {now:?} -> {after:?}");
        jvm.store_array(&mut cooling, 0, after).await?;
    }
    Ok(())
}

/// F1: the equipment bag's size (worn pieces are in it), counted at most once a second.
async fn gear_count(jvm: &Jvm, fx: &Fx, frames: u64) -> JvmResult<i32> {
    let (count, at) = fx.live().gear_count;
    if at != 0 && frames < at + effects::SECOND as u64 {
        return Ok(count);
    }
    let mut count = 0;
    if let Some(bag) = object(jvm.invoke_static("ed", "a", "(B)Lbt;", [JavaValue::Byte(0)]).await?)
        && let Some(items) = object(jvm.get_field(&bag, "a", "Ljava/util/Vector;").await?)
    {
        count = jvm.invoke_virtual(&items, "size", "()I", ()).await?;
    }
    fx.live().gear_count = (count, frames.max(1));
    Ok(count)
}

/// HZ-032: `ek.a:[I[index] += delta`, kept within 0 ..= `ek.b:[I[index]` (1 = SP); `why` names it in the trace.
async fn add_stat(jvm: &Jvm, player: &Box<dyn ClassInstance>, index: usize, delta: i32, why: &str) -> JvmResult<()> {
    let (Some(mut now), Some(top)) = (object(jvm.get_field_from(player, "ek", "a", "[I").await?), object(jvm.get_field_from(player, "ek", "b", "[I").await?)) else {
        return Ok(());
    };
    let v: i32 = jvm.load_array::<i32>(&now, index, 1).await?.pop().unwrap_or_default();
    let max: i32 = jvm.load_array::<i32>(&top, index, 1).await?.pop().unwrap_or_default();
    let after = v.saturating_add(delta).clamp(0, max.max(0));
    fx!("stat {why} {index} {v} -> {after} delta {delta} top {max}");
    jvm.store_array(&mut now, index, [after]).await
}

/// HZ-032: 동화's frames (`ek.b:I`) + `delta` while transformed, never past `effects::FUSION_TOP`.
async fn add_fusion(jvm: &Jvm, player: &mut Box<dyn ClassInstance>, delta: i32, why: &str) -> JvmResult<()> {
    let left: i32 = jvm.get_field_from(player, "ek", "b", "I").await?;
    if left > 0 {
        let after = (left + delta).min(effects::FUSION_TOP.max(left));
        fx!("fusion {why} {left} -> {after}");
        jvm.put_field_from(player, "ek", "b", "I", after).await?;
    }
    Ok(())
}

/// HZ-032 `hitDamage(int attack, int defence)I`, in place of `cq.a(II)I` in the hit `cq.a(Lek;…)Z`.
struct HitDamage(Arc<Fx>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for HitDamage {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let base: i32 = jvm.invoke_static("cq", "a", "(II)I", args.into_vec()).await?;
        let Some(mut player) = player(jvm).await? else {
            return Ok(JavaValue::Int(base));
        };
        // The pieces count only while HZ-032 is chosen: HZ-043 puts this hook here without it.
        let w = if self.0.state.patch_on("HZ-032") { worn(jvm, &player).await? } else { effects::Worn::default() };
        let combo = self.0.state.patch_on("HZ-043");
        let tables = self.0.state.patch_on("HZ-039");
        if w.is_empty() && !combo && !tables {
            return Ok(JavaValue::Int(base));
        }
        let m = moment(jvm, &player, &self.0, &w).await?;
        let (pct, flat) = if w.is_empty() { (100, 0) } else { (effects::hit_pct(&w, &m), effects::hit_flat(&w, &m)) };
        let mut dealt = if w.is_empty() { base } else { effects::scale(base, pct) + flat };
        // The 2nd pass's 기습 · 근접사격 ×1.5 (balance::skill_pct) ride the skill tables' patch.
        let table_pct = if tables { balance::skill_pct(m.job, m.action) } else { 100 };
        if table_pct != 100 {
            dealt = effects::scale(dealt, table_pct);
        }
        fx!(
            "hit {base} -> {dealt} pct {pct} flat {flat} table {table_pct} action {} job {} hp {} spfull {} tr {} clone {} counter {} ambush {} gear {}",
            m.action, m.job, m.hp_pct, m.sp_full, m.transformed, m.clone, m.counter, m.ambush, m.gear_count
        );
        // The rotation bench (`tools/hz_rotation.py`, HZ_BENCH) counts every hit here.
        if effects::bench() {
            tracing::info!("hz: bench hit {base} -> {dealt} action {} job {}", m.action, m.job);
        }
        if dealt != base {
            tracing::info!("hz: 85제 hit {base} -> {dealt} (action {}, job {})", m.action, m.job);
        }
        // HZ-043: the basic combo's steps are weighted and pay SP.
        if combo && let Some((pct, sp)) = balance::combo(m.action) {
            dealt = effects::scale(dealt, pct);
            add_stat(jvm, &player, 1, sp, "combo").await?;
        }
        fx!("hit final {dealt}");
        if !effects::is_skill(m.action) {
            if w.has(legend::Effect::TimeThief) {
                let second = {
                    let mut live = self.0.live();
                    live.basic_hits += 1;
                    let due = live.basic_hits >= effects::THIEF_HITS;
                    if due {
                        live.basic_hits = 0;
                    }
                    due
                };
                if second {
                    shave_cooldowns(jvm, effects::SECOND, "TimeThief").await?;
                }
            }
            if w.has(legend::Effect::FlowSp) {
                add_stat(jvm, &player, 1, effects::FLOW_SP, "FlowSp").await?;
            }
            if w.has(legend::Effect::BasicLeech) {
                add_stat(jvm, &player, 0, effects::scale(dealt, effects::BASIC_LEECH_PCT).max(1), "BasicLeech").await?;
            }
            if w.has(legend::Effect::SpiritDagger) {
                add_stat(jvm, &player, 3, effects::BASIC_MANA, "SpiritDagger").await?;
            }
        } else {
            if w.has(legend::Effect::SkillSpGain) {
                add_stat(jvm, &player, 1, effects::SKILL_HIT_SP, "SkillSpGain").await?;
            }
            if w.has(legend::Effect::SkillHeal) {
                add_stat_pct(jvm, &player, 0, effects::SKILL_HEAL_PCT, "SkillHeal").await?;
            }
        }
        if w.has(legend::Effect::FusionOnHit) && m.transformed {
            add_fusion(jvm, &mut player, effects::FUSION_HIT, "FusionOnHit").await?;
        }
        Ok(JavaValue::Int(dealt))
    }
}

/// HZ-032 `critChance(Object ek, byte, boolean)I`, in place of `ek.a(BZ)I` in the hit.
struct CritChance(Arc<Fx>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for CritChance {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let args = args.into_vec();
        let Some(ek) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.critChance").await);
        };
        let chance: i32 = jvm.invoke_virtual(&ek, "a", "(BZ)I", args[1..].to_vec()).await?;
        let forced = std::mem::take(&mut self.0.live().dash_crit);
        if forced {
            fx!("crit {chance} -> 100 forced");
            return Ok(JavaValue::Int(100));
        }
        // The second draft's points on the chance (급소의 장갑 …).
        let w = worn(jvm, &ek).await?;
        if w.is_empty() {
            fx!("crit {chance} -> {chance}");
            return Ok(JavaValue::Int(chance));
        }
        let m = moment(jvm, &ek, &self.0, &w).await?;
        let out = chance.saturating_add(effects::crit_plus(&w, &m));
        fx!("crit {chance} -> {out} action {} hp {} clone {}", m.action, m.hp_pct, m.clone);
        Ok(JavaValue::Int(out))
    }
}

/// HZ-032 `takeDamage(Object ek, int change)V`, in place of `ek.c(I)V` where a monster's hit lands.
struct TakeDamage(Arc<Fx>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for TakeDamage {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(ek) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.takeDamage").await);
        };
        let game = int(args[1].clone());
        let mut change = game;
        let mut to_sp = 0;
        let mut flags = String::new();
        if change < 0 {
            let w = worn(jvm, &ek).await?;
            if !w.is_empty() {
                let m = moment(jvm, &ek, &self.0, &w).await?;
                change = effects::scale(change, effects::taken_pct(&w, &m)).min(-1);
                if w.has(legend::Effect::DamageToSp) {
                    to_sp = effects::scale(-change, effects::DAMAGE_TO_SP_PCT).max(1);
                }
                if effects::bench() {
                    flags = format!(
                        " hp {} tr {} clone {} counter {} volley {} skillguard {} killguard {}",
                        m.hp_pct, m.transformed, m.clone, m.counter, m.volley_guard, m.skill_guard, m.kill_guard
                    );
                }
            }
        }
        if effects::bench() {
            tracing::info!("hz: bench taken {change}");
        }
        fx!("taken {game} -> {change} sp {to_sp}{flags}");
        let _: () = jvm.invoke_virtual(&ek, "c", "(I)V", (change,)).await?;
        if to_sp > 0 {
            add_stat(jvm, &ek, 1, to_sp, "DamageToSp").await?;
        }
        Ok(JavaValue::Void)
    }
}

/// HZ-032 `monsterExp(Object cq)V`, in place of `this.n()V` in a monster's death `cq.g()V`.
struct MonsterExp(Arc<Fx>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for MonsterExp {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(monster) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.monsterExp").await);
        };
        let _: () = jvm.invoke_virtual(&monster, "n", "()V", ()).await?;
        if effects::bench() {
            let level: i8 = jvm.get_field_from(&monster, "cq", "n", "B").await?;
            tracing::info!("hz: bench kill LV {level}");
        }
        let Some(mut player) = player(jvm).await? else {
            return Ok(JavaValue::Void);
        };
        let w = worn(jvm, &player).await?;
        if w.is_empty() {
            return Ok(JavaValue::Void);
        }
        if w.has(legend::Effect::FrenzyKills) {
            add_stat_pct(jvm, &player, 1, effects::FRENZY_SP_PCT, "FrenzyKills").await?;
        }
        if w.has(legend::Effect::FusionGuard) {
            add_fusion(jvm, &mut player, effects::FUSION_KILL, "FusionGuard").await?;
        }
        // The second draft's kill effects.
        if w.has(legend::Effect::LifeRing) {
            add_stat_pct(jvm, &player, 0, effects::KILL_HEAL_PCT, "LifeRing").await?;
        }
        if w.has(legend::Effect::SageHelm) {
            add_stat_pct(jvm, &player, 3, effects::KILL_MANA_PCT, "SageHelm").await?;
        }
        if w.has(legend::Effect::KillCooldown) {
            shave_cooldowns(jvm, effects::KILL_COOLDOWN, "KillCooldown").await?;
        }
        if w.has(legend::Effect::KillGuard) {
            self.0.live().kill_guard_until = self.0.state.frames() + effects::KILL_GUARD_FRAMES;
            fx!("killguard on");
        }
        Ok(JavaValue::Void)
    }
}

/// HZ-032 `skillSp(Object db, byte action)I`, in place of `db.b(B)I` in the action driver `ek.C()V`.
struct SkillSp(Arc<Fx>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for SkillSp {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(db) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.skillSp").await);
        };
        let action: i8 = args[1].clone().into();
        let cost: i32 = jvm.invoke_virtual(&db, "b", "(B)I", [JavaValue::Byte(action)]).await?;
        let now = self.0.state.frames();
        if self.0.live().replay_due(action, now) {
            fx!("sp {action} {cost} -> 0 replay");
            return Ok(JavaValue::Int(0));
        }
        let Some(player) = player(jvm).await? else {
            return Ok(JavaValue::Int(cost));
        };
        let w = worn(jvm, &player).await?;
        if w.is_empty() {
            fx!("sp {action} {cost} -> {cost}");
            return Ok(JavaValue::Int(cost));
        }
        // The moment's action is the queue's; the SP asked is for `action`.
        let m = effects::Moment { action, ..moment(jvm, &player, &self.0, &w).await? };
        let out = effects::scale(cost, effects::sp_pct(&w, &m));
        fx!("sp {action} {cost} -> {out} hp {}", m.hp_pct);
        Ok(JavaValue::Int(out))
    }
}

/// HZ-032 `castStart(Object db, byte action)Z`, in place of `db.b(B)Z` at an action's start in `ek.C()V`.
struct CastStart(Arc<Fx>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for CastStart {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(db) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.castStart").await);
        };
        let action: i8 = args[1].clone().into();
        let turn: bool = jvm.invoke_virtual(&db, "b", "(B)Z", [JavaValue::Byte(action)]).await?;
        if !effects::is_skill(action) {
            return Ok(JavaValue::Boolean(turn));
        }
        let now = self.0.state.frames();
        if self.0.live().replay_due(action, now) {
            // This start is the second cast.
            self.0.live().replay = None;
            tracing::info!("hz: 85제 skill {action} second cast");
            return Ok(JavaValue::Boolean(turn));
        }
        let Some(player) = player(jvm).await? else {
            return Ok(JavaValue::Boolean(turn));
        };
        let w = worn(jvm, &player).await?;
        if w.is_empty() {
            return Ok(JavaValue::Boolean(turn));
        }
        let job = job(jvm).await?;
        fx!("cast {action} job {job} twice {}", effects::twice(&w, job, action));
        if effects::twice(&w, job, action) {
            let _: () = jvm.invoke_virtual(&player, "f", "(B)V", [JavaValue::Byte(action)]).await?;
            self.0.live().replay = Some((action, now));
            tracing::info!("hz: 85제 skill {action} again");
        }
        if w.has(legend::Effect::DashCrit) && job == effects::ROGUE && action == effects::DASH {
            self.0.live().dash_crit = true;
        }
        if w.has(legend::Effect::VolleyGuard) && job == effects::GUNNER && action == effects::VOLLEY {
            self.0.live().volley_until = now + effects::VOLLEY_GUARD_FRAMES;
        }
        if w.has(legend::Effect::SkillGuard) {
            self.0.live().skill_guard_until = now + effects::SKILL_GUARD_FRAMES;
        }
        if w.has(legend::Effect::AssassinRing) && job == effects::ROGUE && action == effects::AMBUSH {
            self.0.live().ambush_until = now + effects::AMBUSH_FRAMES;
        }
        Ok(JavaValue::Boolean(turn))
    }
}

/// HZ-032 `jobCooldown(Object ek, byte skill)I` / `spiritCooldown(Object p, byte skill)I`, in place of
/// `ek.d(B)I` / `p.d(B)I` where `ed.g(I)V` sets a cooldown — the worn 85제's cuts on it.
///
/// HZ-060 also puts `spiritCooldown` there (HZ-032 off): every slot past the job's two is read from the
/// spirit's table, the `0` / `#` potion slots by the potion's number, and the elixir's is past the
/// table's end — read as the potion it drinks as ([`elixir::cooldown_number`]; user report 2026-11-02,
/// the job change stuck on 「직업 변경 중...」 and 이어하기 dead). The cuts only with HZ-032 on.
struct Cooldown {
    spirit: bool,
    state: Arc<GuestState>,
}

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for Cooldown {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(owner) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.cooldown").await);
        };
        let mut skill: i8 = args[1].clone().into();
        if self.spirit {
            let read = elixir::cooldown_number(skill);
            if read != skill {
                tracing::info!("hz: quick slot cooldown — 엘릭서 ({skill}) read as potion {read}");
                skill = read;
            }
        }
        let frames: i32 = jvm.invoke_virtual(&owner, "d", "(B)I", [JavaValue::Byte(skill)]).await?;
        if !self.state.patch_on("HZ-032") {
            return Ok(JavaValue::Int(frames));
        }
        let Some(player) = player(jvm).await? else {
            return Ok(JavaValue::Int(frames));
        };
        let w = worn(jvm, &player).await?;
        let kind = if self.spirit { "spirit" } else { "job" };
        if w.is_empty() {
            fx!("cooldown {kind} {skill} {frames} -> {frames}");
            return Ok(JavaValue::Int(frames));
        }
        let pct = if self.spirit { effects::spirit_cooldown_pct(&w) } else { effects::job_cooldown_pct(&w, job(jvm).await?, skill) };
        let out = effects::scale(frames, pct);
        fx!("cooldown {kind} {skill} {frames} -> {out}");
        Ok(JavaValue::Int(out))
    }
}

/// HZ-032 durations: `fusionStart(Object ek, int)V` (동화, `ek.n(I)V`), `cloneLife(Object ek, short)V`
/// (분신, `h.a(Lek;S)V`), `markStatus` / `counterStatus(Object, byte, int)V` (`ar.b(BI)V` — 트릭샷's
/// mark, 반격).
struct Duration {
    kind: DurationKind,
}

#[derive(Clone, Copy)]
enum DurationKind {
    Fusion,
    Clone,
    Mark,
    Counter,
}

/// 트릭샷's mark on a monster (`cq.c(bc)V`).
const TRICK_MARK: i8 = 17;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for Duration {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let args = args.into_vec();
        let Some(target) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.duration").await);
        };
        let w = match player(jvm).await? {
            Some(p) => worn(jvm, &p).await?,
            None => effects::Worn::default(),
        };
        match self.kind {
            DurationKind::Fusion => {
                let game = int(args[1].clone());
                let frames = if w.has(legend::Effect::FusionLong) { effects::scale(game, 200) } else { game };
                fx!("duration fusion {game} -> {frames}");
                let _: () = jvm.invoke_virtual(&target, "n", "(I)V", (frames,)).await?;
            }
            DurationKind::Clone => {
                let game: i16 = args[1].clone().into();
                let life = effects::clone_life(&w, game);
                fx!("duration clone {game} -> {life}");
                let _: () = jvm.invoke_static("h", "a", "(Lek;S)V", [JavaValue::Object(Some(target)), JavaValue::Short(life)]).await?;
            }
            DurationKind::Mark | DurationKind::Counter => {
                let status: i8 = args[1].clone().into();
                let game = int(args[2].clone());
                let mut cycles = game;
                match self.kind {
                    DurationKind::Mark if w.has(legend::Effect::TrickLong) && status == TRICK_MARK => cycles = effects::scale(cycles, 300),
                    DurationKind::Counter => cycles = effects::counter_cycles(&w, cycles),
                    _ => {}
                }
                fx!("duration {} status {status} {game} -> {cycles}", if matches!(self.kind, DurationKind::Mark) { "mark" } else { "counter" });
                let _: () = jvm.invoke_virtual(&target, "b", "(BI)V", [JavaValue::Byte(status), JavaValue::Int(cycles)]).await?;
            }
        }
        Ok(JavaValue::Void)
    }
}

/// HZ-032 `spiritCost(Object p, byte, int cost)I` (공허의 손's MP cut, `p.a(BI)I`) and
/// `spiritPower(Object ek, int)I` (a spirit skill's power, `ek.c(I)I`).
struct Spirit {
    power: bool,
    state: Arc<GuestState>,
}

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for Spirit {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let args = args.into_vec();
        let Some(owner) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.spirit").await);
        };
        let w = match player(jvm).await? {
            Some(p) => worn(jvm, &p).await?,
            None => effects::Worn::default(),
        };
        if self.power {
            let power: i32 = jvm.invoke_virtual(&owner, "c", "(I)I", args[1..].to_vec()).await?;
            // The pieces' percent, plus the 3rd balance pass's +50 % for everyone (rides the skill tables' patch).
            let pct = effects::spirit_power_pct(&w) + if self.state.patch_on("HZ-039") { balance::SPIRIT_POWER_BONUS } else { 0 };
            let out = if pct != 100 { effects::scale(power, pct) } else { power };
            fx!("spirit power {power} -> {out} pct {pct}");
            return Ok(JavaValue::Int(out));
        }
        let cost = int(args[2].clone());
        let game: i32 = jvm.invoke_virtual(&owner, "a", "(BI)I", args[1..].to_vec()).await?;
        let pct = effects::spirit_mana_cut_pct(&w);
        let cut = if pct > 0 { (game + effects::scale(cost, pct)).min(cost.max(game)) } else { game };
        fx!("spirit cost {cost} cut {game} -> {cut}");
        Ok(JavaValue::Int(cut))
    }
}

/// HZ-032: the worn effects, or none without a player.
async fn worn_now(jvm: &Jvm) -> JvmResult<effects::Worn> {
    match player(jvm).await? {
        Some(p) => worn(jvm, &p).await,
        None => Ok(effects::Worn::default()),
    }
}

/// HZ-032 `counterReflect(Object attacker, Object player, int, byte, byte)V`, in place of
/// `ar.a(Lar;IBB)V` where 반격 reflects a hit.
struct CounterReflect;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for CounterReflect {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let mut args = args.into_vec();
        let (Some(attacker), Some(me)) = (object(args[0].clone()), object(args[1].clone())) else {
            return Err(null_pointer(jvm, "Hooks.counterReflect").await);
        };
        let w = worn(jvm, &me).await?;
        let game = int(args[2].clone());
        let mut damage = game;
        let pct = effects::counter_pct(&w);
        if pct != 100 {
            damage = effects::scale(damage, pct);
            args[2] = JavaValue::Int(damage);
        }
        let _: () = jvm.invoke_virtual(&attacker, "a", "(Lar;IBB)V", args[1..].to_vec()).await?;
        let heal = if w.has(legend::Effect::CounterLeech) { effects::scale(damage, effects::COUNTER_LEECH_PCT).max(1) } else { 0 };
        fx!("reflect {game} -> {damage} leech {heal}");
        if heal > 0 {
            let _: () = jvm.invoke_virtual(&me, "c", "(I)V", (heal,)).await?;
        }
        Ok(JavaValue::Void)
    }
}

/// HZ-067 `monsterHurt(Object cq, Object attacker, int)V`, in place of `cq.a(Lar;I)V`, the one call that takes a
/// monster's HP: `cq.a(Lar;IBB)V` @84 (a landed hit) and `cq.b(Lbc;)V` @103 (독수's tick — attacker null). The game's
/// method always runs; with a draining 85제 piece on, the player gets [`effects::DRAIN_PCT`] of what came off.
struct MonsterHurt;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for MonsterHurt {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let args = args.into_vec();
        let Some(monster) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.monsterHurt").await);
        };
        // Whose hit: the player's (a spirit's and 반격's come with the player as the attacker), the clone's, the
        // poison's. A monster's (on the clone) drains nothing.
        let source = match object(args[1].clone()) {
            None => Some(2),
            Some(a) if jvm.is_instance(&*a, "ek") => Some(0),
            Some(a) if jvm.is_instance(&*a, "h") => Some(1),
            Some(_) => None,
        };
        let me = if source.is_some() { player(jvm).await? } else { None };
        let pct = match &me {
            Some(p) => effects::drain_pct(&worn(jvm, p).await?),
            None => 0,
        };
        let game = args[1..].to_vec();
        let Some(me) = me.filter(|_| pct > 0) else {
            let _: () = jvm.invoke_virtual(&monster, "a", "(Lar;I)V", game).await?;
            return Ok(JavaValue::Void);
        };
        // `c()I`, not the field: a 거인신 part (`bs`) answers for its body `be.a`, whose HP the hit takes.
        let before: i32 = jvm.invoke_virtual(&monster, "c", "()I", ()).await?;
        let _: () = jvm.invoke_virtual(&monster, "a", "(Lar;I)V", game).await?;
        let after: i32 = jvm.invoke_virtual(&monster, "c", "()I", ()).await?;
        let (removed, heal) = (before - after, effects::drain_heal(before - after, pct));
        // Not at 0: a boss's reflection (just now) or a poison tick can land after the player's fall.
        let Some(now) = object(jvm.get_field_from(&me, "ek", "a", "[I").await?) else {
            return Ok(JavaValue::Void);
        };
        let hp: i32 = jvm.load_array::<i32>(&now, 0, 1).await?.pop().unwrap_or_default();
        fx!("drain dealt {} removed {removed} heal {heal} pct {pct} source {} hp {hp} part {}", int(args[2].clone()), source.unwrap_or(-1), jvm.is_instance(&*monster, "bs"));
        if heal > 0 && hp > 0 {
            let _: () = jvm.invoke_virtual(&me, "c", "(I)V", (heal,)).await?;
        }
        Ok(JavaValue::Void)
    }
}

/// HZ-032 `shieldBash(Object d, Object cq, byte, byte)Z`, in place of `d.a(Lcq;BB)Z` (private) in
/// 방패치기.
struct ShieldBash;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for ShieldBash {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let args = args.into_vec();
        let Some(knight) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.shieldBash").await);
        };
        let hit = |target: JavaValue| [target, args[2].clone(), args[3].clone()];
        let Some(me) = player(jvm).await? else {
            return jvm.invoke_special(&knight, "d", "a", "(Lcq;BB)Z", hit(args[1].clone()).to_vec()).await;
        };
        if !worn(jvm, &me).await?.has(legend::Effect::ShieldBashWide) {
            return jvm.invoke_special(&knight, "d", "a", "(Lcq;BB)Z", hit(args[1].clone()).to_vec()).await;
        }
        // Every monster in the 3×3: the player's eight directions (a big monster spans tiles — once).
        let mut seen: Vec<(i16, i16)> = Vec::new();
        let mut any = false;
        for dir in 1..=8i8 {
            let Some(m) = object(jvm.invoke_virtual(&me, "a", "(B)Lcq;", [JavaValue::Byte(dir)]).await?) else {
                continue;
            };
            let at: (i16, i16) = (jvm.get_field_from(&m, "er", "h", "S").await?, jvm.get_field_from(&m, "er", "i", "S").await?);
            if seen.contains(&at) {
                continue;
            }
            seen.push(at);
            let target = JavaValue::Object(Some(m));
            let landed: bool = jvm.invoke_special(&knight, "d", "a", "(Lcq;BB)Z", hit(target.clone()).to_vec()).await?;
            if landed && let Some(m) = object(target) {
                let _: () = jvm.invoke_virtual(&m, "c", "(I)V", (effects::BASH_STUN,)).await?;
                any = true;
            }
        }
        fx!("bash wide targets {} landed {any}", seen.len());
        Ok(JavaValue::Boolean(any))
    }
}

/// HZ-032 `smash(Object ek, byte, int, byte, boolean, boolean, boolean)Z`, in place of `ek.a(BIBZZZ)Z`
/// (the front hit) in 강타.
struct Smash;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for Smash {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let args = args.into_vec();
        let Some(me) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.smash").await);
        };
        let front: bool = jvm.invoke_virtual(&me, "a", "(BIBZZZ)Z", args[1..].to_vec()).await?;
        if !worn(jvm, &me).await?.has(legend::Effect::SmashWide) {
            return Ok(JavaValue::Boolean(front));
        }
        // The two tiles behind the front one (the player's directions facing + 8, + 12), hit as the game's
        // front hit does: (monster, action, damage, facing, fx, knock, false, false).
        let facing: i8 = jvm.get_field_from(&me, "ar", "c", "B").await?;
        let first = object(jvm.invoke_virtual(&me, "a", "()Lcq;", ()).await?);
        let mut seen = Vec::new();
        if let Some(f) = &first {
            seen.push((jvm.get_field_from::<i16>(f, "er", "h", "S").await?, jvm.get_field_from::<i16>(f, "er", "i", "S").await?));
        }
        let mut any = front;
        for dir in [facing + 8, facing + 12] {
            let Some(m) = object(jvm.invoke_virtual(&me, "a", "(B)Lcq;", [JavaValue::Byte(dir)]).await?) else {
                continue;
            };
            let at: (i16, i16) = (jvm.get_field_from(&m, "er", "h", "S").await?, jvm.get_field_from(&m, "er", "i", "S").await?);
            if seen.contains(&at) {
                continue;
            }
            seen.push(at);
            let hit = [
                JavaValue::Object(Some(m)),
                args[1].clone(),
                args[2].clone(),
                JavaValue::Byte(facing),
                args[3].clone(),
                args[4].clone(),
                JavaValue::Boolean(false),
                JavaValue::Boolean(false),
            ];
            let landed: bool = jvm.invoke_virtual(&me, "a", "(Lcq;BIBBZZZ)Z", hit).await?;
            fx!("smash wide behind landed {landed}");
            any |= landed;
        }
        fx!("smash wide front {front} first {}", first.is_some());
        Ok(JavaValue::Boolean(any))
    }
}

/// HZ-032 `frenzyCost(Object ek, int)V`, in place of `ek.c(I)V` where 광기 takes its HP.
struct FrenzyCost;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for FrenzyCost {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(me) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.frenzyCost").await);
        };
        let skip = worn(jvm, &me).await?.has(legend::Effect::FrenzyKills);
        fx!("frenzy hp {} skipped {skip}", int(args[1].clone()));
        if !skip {
            let _: () = jvm.invoke_virtual(&me, "c", "(I)V", (int(args[1].clone()),)).await?;
        }
        Ok(JavaValue::Void)
    }
}

/// HZ-032 `db.c(B)I` read as a chance: 저격 (G1 ×2), 독수 (R4 +30), 폭발탄 (B3 +30).
struct Chance(legend::Effect);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for Chance {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(job) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.chance").await);
        };
        let pct: i32 = jvm.invoke_virtual(&job, "c", "(B)I", [args[1].clone()]).await?;
        if !worn_now(jvm).await?.has(self.0) {
            fx!("chance {:?} {pct} -> {pct}", self.0);
            return Ok(JavaValue::Int(pct));
        }
        let out = match self.0 {
            legend::Effect::SnipeDouble => effects::scale(pct, 200),
            _ => pct + effects::CHANCE_PLUS,
        };
        fx!("chance {:?} {pct} -> {out}", self.0);
        Ok(JavaValue::Int(out))
    }
}

/// HZ-032 `poisonDamage(Object ek, boolean)I`, in place of `ek.c(Z)I` (the roll 독수's poison keeps).
struct PoisonDamage;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for PoisonDamage {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(me) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.poisonDamage").await);
        };
        let roll: i32 = jvm.invoke_virtual(&me, "c", "(Z)I", [args[1].clone()]).await?;
        let double = worn(jvm, &me).await?.has(legend::Effect::PoisonBoost);
        let out = if double { effects::scale(roll, effects::POISON_PCT) } else { roll };
        fx!("poison {roll} -> {out}");
        Ok(JavaValue::Int(out))
    }
}

/// HZ-032 `knock(Object cq, int damage, boolean knock, byte dir, byte fx)V`, in place of the hit's
/// `if (knock) this.b(dir)`.
struct Knock;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for Knock {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(monster) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.knock").await);
        };
        let knocked: bool = args[2].clone().into();
        if !knocked {
            return Ok(JavaValue::Void);
        }
        if let Some(me) = player(jvm).await?
            && job(jvm).await? == effects::BAYONET
            && let pct = effects::blast_pct(&worn(jvm, &me).await?)
            && pct > 0
        {
            let more = effects::scale(int(args[1].clone()), pct).max(1);
            fx!("knock {} extra {more}", int(args[1].clone()));
            let at = [JavaValue::Object(Some(me)), JavaValue::Int(more), args[3].clone(), args[4].clone()];
            let _: () = jvm.invoke_virtual(&monster, "a", "(Lar;IBB)V", at).await?;
        } else {
            fx!("knock {} extra 0", int(args[1].clone()));
        }
        let _: () = jvm.invoke_virtual(&monster, "b", "(B)V", [args[3].clone()]).await?;
        Ok(JavaValue::Void)
    }
}

/// HZ-032 `knockable(Object ar)Z`, in place of `ek.g()Z` / `ar.g()Z` where a hit would push the player.
struct Knockable;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for Knockable {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(target) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.knockable").await);
        };
        let player = jvm.is_instance(&*target, "ek");
        if player && worn(jvm, &target).await?.has(legend::Effect::BayonetGuard) {
            fx!("knockable player game ? -> false");
            return Ok(JavaValue::Boolean(false));
        }
        let game: bool = jvm.invoke_virtual(&target, "g", "()Z", ()).await?;
        if player {
            fx!("knockable player game {game} -> {game}");
        }
        Ok(JavaValue::Boolean(game))
    }
}

/// HZ-032 `drank(Object bt, Object ba)I`, in place of `bt.a(Lba;)I` once a potion is drunk.
struct Drank(Arc<Fx>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for Drank {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(bag) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.drank").await);
        };
        let at: i32 = jvm.invoke_virtual(&bag, "a", "(Lba;)I", [args[1].clone()]).await?;
        let w = worn_now(jvm).await?;
        fx!("drank");
        if w.has(legend::Effect::Drunkard) {
            self.0.live().drunk_until = self.0.state.frames() + effects::DRUNK_FRAMES;
            tracing::info!("hz: 85제 술꾼의 장화 — fast for 10 s");
        }
        if w.has(legend::Effect::PotionSp) && let Some(me) = player(jvm).await? {
            add_stat_pct(jvm, &me, 1, 100, "PotionSp").await?;
            tracing::info!("hz: 85제 철벽의 장화 — SP filled");
        }
        Ok(JavaValue::Int(at))
    }
}

/// HZ-032 `moveStep(Object ar)I`, in place of `getfield ar.f:B` in a walking frame `ar.d()Z`.
struct MoveStep(Arc<Fx>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for MoveStep {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(walker) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.moveStep").await);
        };
        let step: i8 = jvm.get_field_from(&walker, "ar", "f", "B").await?;
        if step != effects::WALK_STEP || !jvm.is_instance(&*walker, "ek") {
            if effects::bench() && jvm.is_instance(&*walker, "ek") {
                let transformed = jvm.get_field_from::<i32>(&walker, "ek", "b", "I").await? > 0;
                fx!("step {step} -> {step} walk off tr {transformed}");
            }
            return Ok(JavaValue::Int(step.into()));
        }
        let w = worn(jvm, &walker).await?;
        if w.is_empty() {
            return Ok(JavaValue::Int(step.into()));
        }
        let drunk = self.0.live().drunk_until > self.0.state.frames();
        let transformed = w.has(legend::Effect::FusionSwift) && jvm.get_field_from::<i32>(&walker, "ek", "b", "I").await? > 0;
        let walk = effects::fast_walk(&w, transformed, drunk);
        if walk == effects::Walk::Game {
            fx!("step {step} -> {step} walk game tr {transformed}");
            return Ok(JavaValue::Int(step.into()));
        }
        // On a tile (`er.k` / `er.l` clear — what `ar.e()Z` tests before it checks the next tile).
        let mid_x: bool = jvm.get_field_from(&walker, "er", "k", "Z").await?;
        let mid_y: bool = jvm.get_field_from(&walker, "er", "l", "Z").await?;
        let aligned = !mid_x && !mid_y;
        if !aligned {
            fx!("step {step} -> {step} walk {walk:?} mid tr {transformed}");
            return Ok(JavaValue::Int(step.into()));
        }
        let mut live = self.0.live();
        let out = if walk == effects::Walk::Fast { effects::drunk_step(&mut live) } else { effects::brisk_step(&mut live) };
        fx!("step {step} -> {out} walk {walk:?} tr {transformed}");
        Ok(JavaValue::Int(out))
    }
}

/// HZ-019 `noticeBottom()I`, in place of `getstatic ce.h:I` where `g.c()V` anchors the pickup
/// notices.
struct NoticeBottom;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for NoticeBottom {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, _: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let bottom: i32 = jvm.get_static_field("ce", "h", "I").await?;
        let canvas: i32 = jvm.get_static_field("rpg/a", "b", "I").await?;
        Ok(JavaValue::Int(notice_bottom(bottom, canvas)))
    }
}

/// HZ-025: is a pickup slot's count `count` a notice not reported yet, after `last` (the count the
/// slot was last drawn at, `None` = never)? The painter counts a live notice down by one before
/// each draw, so the same notice comes back exactly one lower; anything else is a new picture — the
/// game's 15 (drawn first at [`hooks::PICKUP_FIRST_COUNT`]), HZ-022's 45 for 「가디언 레벨 N 달성」,
/// or a full list's slot 0 taken over. A notice is reported once however its count started.
pub fn pickup_is_new(last: Option<i32>, count: i32) -> bool {
    last.is_none_or(|l| count != l - 1)
}

/// HZ-025 `pickupNotice(Object dd, Object cw, int x, int y, int slot)V`, in place of the pickup
/// notice's draw in `g.c()V` (`dd.a(cw, x, y, 33)`): draws nothing — the host stacks the picture
/// with its own notices (hero2's way) — and hands a notice's picture over the first time it is drawn
/// ([`pickup_is_new`], the slot's count in `g.a:[I`).
struct PickupNotice {
    state: Arc<GuestState>,
    last: std::sync::Mutex<[Option<i32>; 3]>,
}

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for PickupNotice {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let slot = int(args[4].clone());
        let (Some(cw), Some(counts)) = (object(args[1].clone()), object(jvm.get_static_field("g", "a", "[I").await?)) else {
            return Ok(JavaValue::Void);
        };
        if !(0..3).contains(&slot) || slot as usize >= jvm.array_length(&counts).await? {
            return Ok(JavaValue::Void);
        }
        let count: Vec<i32> = jvm.load_array(&counts, slot as usize, 1).await?;
        let count = count[0];
        let new = {
            let mut last = self.last.lock().unwrap();
            let new = pickup_is_new(last[slot as usize], count);
            last[slot as usize] = Some(count);
            new
        };
        if new {
            match picture(jvm, &cw, None).await? {
                Some((w, h, px)) => {
                    tracing::info!("hz: pickup notice {w}x{h} (slot {slot}, count {count})");
                    self.state.report_pickup(w, h, px);
                }
                None => tracing::warn!("hz: pickup notice in slot {slot} has no picture"),
            }
        }
        Ok(JavaValue::Void)
    }
}

/// HZ-028: how many elements [`FillInts`] stores at a time — a whole-screen fill is 124,800, and
/// `store_array` makes a value and four bytes for each element it is handed.
pub const FILL_PIECE: usize = 4096;

/// HZ-028 `fillInts(int[] a, int colour)I`, in place of the per-pixel store of the game's translucent
/// fill `dd.a(IIII)V`: every element of `a` set to `colour` — what the loop stores, top index down —
/// then −1 for the loop's index, so its next test leaves it. Natively, [`FILL_PIECE`] at a time.
struct FillInts;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for FillInts {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let mut args = args.into_vec().into_iter();
        let (array, colour) = (object(args.next().unwrap()), int(args.next().unwrap()));
        let Some(mut array) = array else {
            return Err(null_pointer(jvm, "Hooks.fillInts").await);
        };
        let len = jvm.array_length(&array).await?;
        let mut at = 0;
        while at < len {
            let n = FILL_PIECE.min(len - at);
            jvm.store_array(&mut array, at, std::iter::repeat_n(colour, n)).await?;
            at += n;
        }
        Ok(JavaValue::Int(-1))
    }
}

/// HZ-028: one pixel of the game's translucent picture `dd.a(Lcw;III)V` — `mask` ANDed in unless the
/// pixel is fully transparent (`p >>> 24 != 0`), as its loop does.
pub fn mask_alpha(pixel: i32, mask: i32) -> i32 {
    if (pixel as u32) >> 24 != 0 { pixel & mask } else { pixel }
}

/// HZ-028 `maskAlpha(int[] a, int mask)I`, in place of the per-pixel body of the game's translucent
/// picture `dd.a(Lcw;III)V`: every pixel of `a` through [`mask_alpha`], then 0 for the loop's index —
/// its own `iinc 1, -1` leaves it at −1, as the loop does. Natively, [`FILL_PIECE`] at a time.
struct MaskAlpha;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for MaskAlpha {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let mut args = args.into_vec().into_iter();
        let (array, mask) = (object(args.next().unwrap()), int(args.next().unwrap()));
        let Some(mut array) = array else {
            return Err(null_pointer(jvm, "Hooks.maskAlpha").await);
        };
        let len = jvm.array_length(&array).await?;
        let mut at = 0;
        while at < len {
            let n = FILL_PIECE.min(len - at);
            let pixels: Vec<i32> = jvm.load_array(&array, at, n).await?;
            jvm.store_array(&mut array, at, pixels.into_iter().map(|p| mask_alpha(p, mask))).await?;
            at += n;
        }
        Ok(JavaValue::Int(0))
    }
}

/// HZ-020 `gamedataStores / itemStores / slotStores()[Ljava/lang/String;`, in place of `getstatic
/// rpg/b.a / b / c`: the store names of the slot page shown — on page 0 the game's own array.
struct SlotStores(Arc<GuestState>, &'static str, &'static str);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for SlotStores {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, _: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let page = self.0.slot_page();
        if page == 0 {
            return jvm.get_static_field("rpg/b", self.1, "[Ljava/lang/String;").await;
        }
        let n = usize::from(hooks::SLOTS_PER_PAGE);
        let mut names = jvm.instantiate_array("Ljava/lang/String;", n).await?;
        let mut values = Vec::with_capacity(n);
        for index in 0..hooks::SLOTS_PER_PAGE {
            values.push(JavaValue::Object(Some(JavaLangString::from_rust_string(jvm, &crate::slots::store_name(self.2, page, index)).await?)));
        }
        jvm.store_array(&mut names, 0, values).await?;
        Ok(JavaValue::Object(Some(names)))
    }
}

/// HZ-020 `slotKey(Object bb, int action, int raw)Z`, in place of `o.b(II)Z` at the top of
/// `bb.a(II)Z`.
struct SlotKey(Arc<GuestState>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for SlotKey {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let mut args = args.into_vec().into_iter();
        let (this, action, raw) = (object(args.next().unwrap()), int(args.next().unwrap()), int(args.next().unwrap()));
        let Some(mut this) = this else {
            return Err(null_pointer(jvm, "Hooks.slotKey").await);
        };
        // The replaced call first: a child window (the overwrite question, the storage) has the keys.
        let child: bool = jvm.invoke_virtual(&this, "b", "(II)Z", (action, raw)).await?;
        if child {
            return Ok(JavaValue::Boolean(true));
        }
        let delta = match classify(action, raw) {
            Key::Left => -1,
            Key::Right => 1,
            _ => return Ok(JavaValue::Boolean(false)),
        };
        // Only at the title: a slot loaded in play keeps the page it came from.
        let mode: i8 = jvm.get_static_field("ed", "d", "B").await?;
        if mode != 2 {
            return Ok(JavaValue::Boolean(false));
        }
        let page = crate::slots::turn(self.0.slot_page(), delta);
        self.0.set_slot_page(page);
        // The game's own reload of the three summaries (ed.a:[[B …), now from this page.
        let _: () = jvm.invoke_static("ed", "m", "()V", ()).await?;
        fit_slot_window(jvm, &mut this, page, true).await?;
        tracing::info!("hz: save slots page {} (slots {}..{})", page + 1, crate::slots::slot_number(page, 0), crate::slots::slot_number(page, hooks::page_slots(page) - 1));
        Ok(JavaValue::Boolean(true))
    }
}

/// The slot window `bb` for `page`: its count (the last page has fewer), the cursor in it — on the
/// page's first save when it lists saves to continue (`bb.f` 1, as its ctor picks one), else row 0
/// when `reset`.
#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
pub async fn fit_slot_window(jvm: &Jvm, bb: &mut Box<dyn ClassInstance>, page: u8, reset: bool) -> JvmResult<()> {
    let count = hooks::page_slots(page) as i8;
    jvm.put_field_from(bb, "o", "b", "B", count).await?;
    let cursor: i8 = jvm.get_field_from(bb, "o", "a", "B").await?;
    let mut want = if reset || cursor >= count { 0 } else { cursor };
    if reset && jvm.get_field_from::<i8>(bb, "bb", "f", "B").await? == 1
        && let Some(saves) = object(jvm.get_static_field("ed", "a", "[[B").await?)
    {
        let rows: Vec<JavaValue> = jvm.load_array(&saves, 0, count as usize).await?;
        if let Some(first) = rows.into_iter().position(|r| object(r).is_some()) {
            want = first as i8;
        }
    }
    jvm.put_field_from(bb, "o", "a", "B", want).await?;
    jvm.put_field_from(bb, "o", "a", "Z", true).await?;
    Ok(())
}

/// HZ-020 `slotTitle(dd, x, y, title, a, b)V`, in place of the slot window's title draw: the title,
/// then the game's ◀ ▶ (`o.a(Ldd;IIIIZ)V`, the skill window's) either side — the pages turn there.
struct SlotTitle;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for SlotTitle {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let (dd, y) = (args[0].clone(), int(args[2].clone()));
        let _: () = jvm.invoke_static("ah", "a", "(Ldd;II[BII)V", args.into_vec()).await?;
        let centre: i32 = jvm.get_static_field("rpg/a", "c", "I").await?;
        let arrows = [dd, JavaValue::Int(centre - SLOT_ARROWS_W / 2), JavaValue::Int(y + 6), JavaValue::Int(SLOT_ARROWS_W), JavaValue::Int(TITLE_INK), JavaValue::Boolean(false)];
        let _: () = jvm.invoke_static("o", "a", "(Ldd;IIIIZ)V", arrows).await?;
        Ok(JavaValue::Void)
    }
}

/// The slot window's arrows: as wide as the skill window's header (`cb.b(dd)`).
const SLOT_ARROWS_W: i32 = 104;
/// The window titles' colour (`bb.a(dd)V` @6, `ldc` 14597755).
const TITLE_INK: i32 = 14_597_755;

/// The frame `y` of a full window on the canvas (`o.<init>`: `(H − 174) >> 1`).
async fn window_top(jvm: &Jvm) -> JvmResult<i32> {
    let h: i32 = jvm.get_static_field("rpg/a", "b", "I").await?;
    Ok((h - crate::menu_touch::WIN_H) >> 1)
}

/// HZ-020 `slotBox(dd, x, y, w, h, fill, edge)V`, in place of a slot row's box: as the game draws
/// it, but not for a row the page has not.
struct SlotBox(Arc<GuestState>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for SlotBox {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let row = crate::slots::row_at(window_top(jvm).await?, int(args[2].clone()));
        if crate::slots::has_row(self.0.slot_page(), row) {
            let _: () = jvm.invoke_static("o", "a", "(Ldd;IIIIII)V", args.into_vec()).await?;
        }
        Ok(JavaValue::Void)
    }
}

/// HZ-020 `slotLabel(dd, x, y, label, a, b)V`, in place of a slot row's 「저장 N」: N the slot's
/// number on this page; nothing for a row the page has not.
struct SlotLabel(Arc<GuestState>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for SlotLabel {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let mut args = args.into_vec();
        let page = self.0.slot_page();
        let row = crate::slots::row_at(window_top(jvm).await?, int(args[2].clone()) + 1);
        if !crate::slots::has_row(page, row) {
            return Ok(JavaValue::Void);
        }
        if page != 0
            && let Some(label) = object(args[3].clone())
        {
            let n = jvm.array_length(&label).await?;
            let bytes: Vec<i8> = jvm.load_array(&label, 0, n).await?;
            let bytes: Vec<u8> = bytes.into_iter().map(|b| b as u8).collect();
            let number = crate::slots::slot_number(page, row as u8);
            args[3] = JavaValue::Object(Some(byte_array(jvm, &crate::slots::relabel(&bytes, number)).await?));
        }
        let _: () = jvm.invoke_static("ah", "a", "(Ldd;II[BII)V", args).await?;
        Ok(JavaValue::Void)
    }
}

/// HZ-020 `saveCount()B`, in place of the title's `getstatic ed.i:B`: how many of the ten slots
/// hold a save, each read by the game's own `rpg/b.c(B)[B` on its page.
struct SaveCount(Arc<GuestState>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for SaveCount {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, _: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let keep = self.0.slot_page();
        let counted = count_saves(jvm, &self.0).await;
        // The page the title shows stays what it was, whatever a read did.
        self.0.set_slot_page(keep);
        Ok(JavaValue::Byte(counted? as i8))
    }
}

async fn count_saves(jvm: &Jvm, state: &GuestState) -> JvmResult<u8> {
    let mut n = 0;
    for page in 0..hooks::SLOT_PAGES {
        state.set_slot_page(page);
        for index in 0..hooks::page_slots(page) {
            let summary = object(jvm.invoke_static("rpg/b", "c", "(B)[B", [JavaValue::Byte(index as i8)]).await?);
            n += u8::from(summary.is_some());
        }
    }
    Ok(n)
}

/// HZ-021 `shopGold(Object co, Object dd, int x, int y, int gold)V`, in place of the shop painter's
/// gold (`o.b(Ldd;III)V`): the gold, then — selling, nothing open over it — 「일괄판매」 in the title
/// band, a box as the slot window's rows are drawn and the game's font.
struct ShopGold;

/// The button: the slot list's unpicked box (`bb.a(dd)V` @71..91) and white ink.
const BUTTON_FILL: i32 = 3_550_755;
const BUTTON_EDGE: i32 = 7_027_456;
const BUTTON_INK: i32 = 0xFF_FFFF;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for ShopGold {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let mut args = args.into_vec().into_iter();
        let (co, dd) = (object(args.next().unwrap()), args.next().unwrap());
        let rest: Vec<JavaValue> = args.collect();
        let Some(co) = co else {
            return Err(null_pointer(jvm, "Hooks.shopGold").await);
        };
        let _: () = jvm.invoke_virtual(&co, "b", "(Ldd;III)V", [vec![dd.clone()], rest].concat()).await?;
        if crate::bulk::button_shown(jvm, &co).await? {
            let (fx, fy): (i32, i32) = (jvm.get_field_from(&co, "o", "a", "I").await?, jvm.get_field_from(&co, "o", "b", "I").await?);
            let (x, y, w, h) = crate::bulk::button_at(fx, fy);
            let boxed = [dd.clone(), JavaValue::Int(x), JavaValue::Int(y), JavaValue::Int(w), JavaValue::Int(h), JavaValue::Int(BUTTON_FILL), JavaValue::Int(BUTTON_EDGE)];
            let _: () = jvm.invoke_static("o", "a", "(Ldd;IIIIII)V", boxed).await?;
            if let Some(g) = object(dd.clone()) {
                let _: () = jvm.invoke_virtual(&g, "a", "(I)V", (BUTTON_INK,)).await?;
            }
            let label = byte_array(jvm, crate::bulk::LABEL).await?;
            let text = [dd, JavaValue::Int(x + w / 2), JavaValue::Int(y), JavaValue::Object(Some(label)), JavaValue::Int(0), JavaValue::Int(1)];
            let _: () = jvm.invoke_static("ah", "a", "(Ldd;II[BII)V", text).await?;
        }
        Ok(JavaValue::Void)
    }
}

/// The player's EXP since 99 (`ek.a:[I[2]`, HZ-022 keeps it there) and its guardian level.
#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
async fn guardian_of(jvm: &Jvm, player: &Box<dyn ClassInstance>) -> JvmResult<(u16, i64)> {
    let a = object(jvm.get_field_from(player, "ek", "a", "[I").await?);
    let exp = match a {
        Some(a) => i64::from(jvm.load_array::<i32>(&a, 2, 1).await?[0]),
        None => 0,
    };
    Ok(crate::guardian::level_of(exp))
}

/// The HUD's guardian level: what the hooks last worked out, and only while the player is at 99.
#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
async fn state_guardian(_jvm: &Jvm, _player: &Box<dyn ClassInstance>, level: i8) -> JvmResult<u16> {
    Ok(if level >= crate::guardian::TOP_LEVEL { GUARDIAN_SEEN.load(std::sync::atomic::Ordering::Relaxed) } else { 0 })
}

/// The guardian level the HZ-022 hooks last worked out (they run only when the patch is on).
static GUARDIAN_SEEN: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0);
/// The EXP loop is taking the player from 98 to 99 (set by `levelUp`, taken by `expTable`).
static REACHING_TOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// HZ-022 `levelUp(Object ek)I`, in place of `getfield ek.n:B` in the EXP loop.
struct LevelUp(Arc<GuestState>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for LevelUp {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(mut player) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.levelUp").await);
        };
        let level: i8 = jvm.get_field_from(&player, "ek", "n", "B").await?;
        if level < crate::guardian::TOP_LEVEL {
            // About to become 99: the stat recompute it runs (`ek.i()`) must start at GL 1, so the
            // guardian levels the same EXP reaches come here and give their points.
            REACHING_TOP.store(level == crate::guardian::TOP_LEVEL - 1, std::sync::atomic::Ordering::Relaxed);
            return Ok(JavaValue::Int(i32::from(level)));
        }
        let (Some(mut a), Some(mut b)) = (object(jvm.get_field_from(&player, "ek", "a", "[I").await?), object(jvm.get_field_from(&player, "ek", "b", "[I").await?)) else {
            return Ok(JavaValue::Int(i32::from(level)));
        };
        // The loop has just taken b[2] off; at 99 it stays: EXP since 99.
        let (took, now) = (jvm.load_array::<i32>(&b, 2, 1).await?[0], jvm.load_array::<i32>(&a, 2, 1).await?[0]);
        let exp = (i64::from(now) + i64::from(took)).min(i64::from(i32::MAX));
        jvm.store_array(&mut a, 2, vec![exp as i32]).await?;
        let before = crate::guardian::level_of(i64::from(took) - 1).0;
        let (g, _) = crate::guardian::level_of(exp);
        jvm.store_array(&mut b, 2, vec![crate::guardian::next_at(g)]).await?;
        GUARDIAN_SEEN.store(g, std::sync::atomic::Ordering::Relaxed);
        self.0.report_guardian(g);
        if g > before {
            let points: i16 = jvm.get_field_from(&player, "ek", "a", "S").await?;
            let gained = crate::guardian::STAT_POINTS.saturating_mul((g - before) as i16);
            jvm.put_field_from(&mut player, "ek", "a", "S", points.saturating_add(gained)).await?;
            if let Err(e) = guardian_notice(jvm, &player, g).await {
                tracing::warn!("hz: guardian notice failed: {e:?}");
            }
            tracing::info!("hz: guardian level {before} -> {g} (+{gained} stat points), EXP since 99 {exp}");
        }
        Ok(JavaValue::Int(i32::from(level)))
    }
}

/// 「가디언 레벨 N 달성」: the game's level-up effect `cg(8)` on the player and a line in the game's
/// own pickup list, made as `g.a(BBBZ)V` makes its pictures (120 × 16: black, the 13682856 frame,
/// white centred text) and put in its first free slot (`g.b:[Lcw;`, `g.a:[I` the ticks).
#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
async fn guardian_notice(jvm: &Jvm, player: &Box<dyn ClassInstance>, g: u16) -> JvmResult<()> {
    let effect = jvm.new_class("cg", "(B)V", [JavaValue::Byte(8)]).await?;
    let _: () = jvm.invoke_virtual(player, "a", "(Lbc;)V", [JavaValue::Object(Some(effect))]).await?;
    let Some(map) = object(jvm.get_static_field("ed", "a", "Lg;").await?) else { return Ok(()) };
    let picture: Box<dyn ClassInstance> = jvm.invoke_static("cw", "a", "(II)Lcw;", [JavaValue::Int(120), JavaValue::Int(16)]).await?;
    let pen: Box<dyn ClassInstance> = jvm.invoke_virtual(&picture, "a", "()Ldd;", ()).await?;
    let _: () = jvm.invoke_virtual(&pen, "a", "(I)V", (0,)).await?;
    let _: () = jvm.invoke_virtual(&pen, "a", "(IIII)V", (0, 0, 120, 16)).await?;
    let _: () = jvm.invoke_virtual(&pen, "a", "(I)V", (13_682_856,)).await?;
    let _: () = jvm.invoke_virtual(&pen, "d", "(IIII)V", (1, 1, 117, 13)).await?;
    let _: () = jvm.invoke_virtual(&pen, "a", "(I)V", (0xFF_FFFF,)).await?;
    let text = byte_array(jvm, &crate::guardian::notice(g)).await?;
    let draw = [JavaValue::Object(Some(pen)), JavaValue::Int(60), JavaValue::Int(1), JavaValue::Object(Some(text)), JavaValue::Int(0), JavaValue::Int(1)];
    let _: () = jvm.invoke_static("ah", "a", "(Ldd;II[BII)V", draw).await?;
    let (Some(mut slots), Some(mut ticks)) = (object(jvm.get_field(&map, "b", "[Lcw;").await?), object(jvm.get_static_field("g", "a", "[I").await?)) else {
        return Ok(());
    };
    let n = jvm.array_length(&slots).await?;
    let held: Vec<Option<Box<dyn ClassInstance>>> = jvm.load_array(&slots, 0, n).await?;
    let at = held.iter().position(Option::is_none).unwrap_or(0);
    jvm.store_array(&mut slots, at, vec![Some(picture)]).await?;
    jvm.store_array(&mut ticks, at, vec![GUARDIAN_NOTICE_TICKS]).await?;
    Ok(())
}

/// How long the notice stays: three of the game's pickup lines (15 each).
const GUARDIAN_NOTICE_TICKS: i32 = 45;

/// HZ-022 `expTable(Object ek)[I`, in place of `getfield ek.b:[I` where `ek.i()V` stores `b[2]`.
struct ExpTable(Arc<GuestState>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for ExpTable {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(player) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.expTable").await);
        };
        let b = jvm.get_field_from(&player, "ek", "b", "[I").await?;
        let level: i8 = jvm.get_field_from(&player, "ek", "n", "B").await?;
        if level < crate::guardian::TOP_LEVEL {
            return Ok(b);
        }
        let Some(mut table) = object(b) else { return Ok(JavaValue::Object(None)) };
        // Reaching 99 in the EXP loop: from GL 1, so the loop goes on to the guardian levels' points.
        // Anywhere else (a load, gear, points spent) the GL the saved EXP says — its points were given.
        let reaching = REACHING_TOP.swap(false, std::sync::atomic::Ordering::Relaxed);
        let g = if reaching { 1 } else { guardian_of(jvm, &player).await?.0 };
        jvm.store_array(&mut table, 2, vec![crate::guardian::next_at(g)]).await?;
        GUARDIAN_SEEN.store(g, std::sync::atomic::Ordering::Relaxed);
        self.0.report_guardian(g);
        // The game's own value lands here instead.
        let scratch = jvm.instantiate_array("I", 4).await?;
        Ok(JavaValue::Object(Some(scratch)))
    }
}

/// HZ-022 `statNow` / `statMax(Object ek, int i)I`, the bodies of `ek.e(B)I` / `ek.f(B)I`: `a[i]` /
/// `b[i]`, except EXP (2) at 99 — the EXP into the guardian level / what it needs.
struct StatView(bool);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for StatView {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let (player, index) = (object(args[0].clone()), int(args[1].clone()));
        let Some(player) = player else {
            return Err(null_pointer(jvm, "Hooks.statNow/statMax").await);
        };
        if index == 2 && jvm.get_field_from::<i8>(&player, "ek", "n", "B").await? >= crate::guardian::TOP_LEVEL {
            let (g, into) = guardian_of(jvm, &player).await?;
            let v = if self.0 { into } else { crate::guardian::need(g) };
            return Ok(JavaValue::Int(v.min(i64::from(i32::MAX)) as i32));
        }
        let table = object(jvm.get_field_from(&player, "ek", if self.0 { "a" } else { "b" }, "[I").await?);
        let Some(table) = table else {
            return Err(null_pointer(jvm, "ek.a/b").await);
        };
        Ok(JavaValue::Int(jvm.load_array::<i32>(&table, index.max(0) as usize, 1).await?[0]))
    }
}

/// HZ-014 `targetX(int width)I`, in place of `bipush 80; isub` in `ce.f()V`.
struct TargetX(Arc<GuestState>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for TargetX {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let width = int(args[0].clone());
        let shift = self.0.target_shift();
        // A failed read leaves the panel where HZ-014 alone puts it: the game draws on regardless.
        let x = match read_map_span(jvm).await {
            Ok(Some((left, right))) => target_x_on_map(width, shift, left, right),
            Ok(None) => target_x(width, shift),
            Err(e) => {
                tracing::warn!("hz: targetX: the map's span unread ({e:?})");
                target_x(width, shift)
            }
        };
        Ok(JavaValue::Int(x))
    }
}

/// HZ-054 `targetPicture(Object dd, Object picture, int x, int y, int anchor)V`, in place of
/// `dd.a(Lcw;III)V` @40 of `ce.f()V`: the target panel's picture, [`GuestState::target_drop`] lower.
struct TargetPicture(Arc<GuestState>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for TargetPicture {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let mut args = args.into_vec().into_iter();
        let (dd, picture) = (object(args.next().unwrap()), args.next().unwrap());
        let (x, y, anchor) = (int(args.next().unwrap()), int(args.next().unwrap()), int(args.next().unwrap()));
        let Some(dd) = dd else {
            return Err(null_pointer(jvm, "Hooks.targetPicture: dd").await);
        };
        let y = y + self.0.target_drop();
        let _: () = jvm.invoke_virtual(&dd, "a", "(Lcw;III)V", [picture, JavaValue::Int(x), JavaValue::Int(y), JavaValue::Int(anchor)]).await?;
        Ok(JavaValue::Void)
    }
}

/// HZ-054 `targetBar(Object dd, int x, int y, int w, int h)V`, in place of `dd.a(IIII)V` @83 of
/// `ce.f()V`: the panel's HP bar, as lower.
struct TargetBar(Arc<GuestState>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for TargetBar {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let mut args = args.into_vec().into_iter();
        let dd = object(args.next().unwrap());
        let (x, y, w, h) = (int(args.next().unwrap()), int(args.next().unwrap()), int(args.next().unwrap()), int(args.next().unwrap()));
        let Some(dd) = dd else {
            return Err(null_pointer(jvm, "Hooks.targetBar: dd").await);
        };
        let _: () = jvm.invoke_virtual(&dd, "a", "(IIII)V", (x, y + self.0.target_drop(), w, h)).await?;
        Ok(JavaValue::Void)
    }
}

/// The map picture's canvas span, as `ce.e(dd)` clips to it: from `g.d` (the map's x — centred
/// when it is narrower than the canvas, `g.c()`) for `min(g.a, ce.g − g.d)` (`g.a:S` = width in
/// px, `ce.g` = the canvas width). `None` while there is no map. [`map_span`] is the host's copy.
async fn read_map_span(jvm: &Jvm) -> JvmResult<Option<(i32, i32)>> {
    let Some(map) = object(jvm.get_static_field("ed", "a", "Lg;").await?) else {
        return Ok(None);
    };
    let x: i32 = jvm.get_field(&map, "d", "I").await?;
    let w: i16 = jvm.get_field(&map, "a", "S").await?;
    let canvas: i32 = jvm.get_static_field("ce", "g", "I").await?;
    Ok(Some((x, x + i32::from(w).min(canvas - x))))
}

/// What the HZ-008 blit hook's reads cost the game's thread — everything after its `drawImage`:
/// the frames and the nanoseconds since [`take_blit_gauge`] last ran (the worker's 30-second
/// `emu gauge` line, the probe's `--timing`).
static BLIT_FRAMES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static BLIT_NANOS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// `(frames, nanoseconds)` the blit hook's reads took since the last call, and starts again.
pub fn take_blit_gauge() -> (u64, u64) {
    use std::sync::atomic::Ordering::Relaxed;
    (BLIT_FRAMES.swap(0, Relaxed), BLIT_NANOS.swap(0, Relaxed))
}

/// HZ-008 `blit(Object g, Object img, int x, int y, int anchor)V`, in place of the
/// `Graphics.drawImage` that puts the game's back buffer on the canvas: that draw, exactly, then
/// the scene for the HUD. A failed read is logged and dropped — the frame is already drawn and
/// nothing of the game's depends on the report.
struct Blit(Arc<GuestState>, Arc<Boss>, Arc<Quick>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for Blit {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let mut args = args.into_vec().into_iter();
        let (g, img) = (object(args.next().unwrap()), object(args.next().unwrap()));
        let (x, y, anchor) = (int(args.next().unwrap()), int(args.next().unwrap()), int(args.next().unwrap()));
        let Some(g) = g else {
            return Err(null_pointer(jvm, "Hooks.blit: g").await);
        };
        let _: () = jvm
            .invoke_virtual(
                &g,
                "drawImage",
                "(Ljavax/microedition/lcdui/Image;III)V",
                [JavaValue::Object(img), JavaValue::Int(x), JavaValue::Int(y), JavaValue::Int(anchor)],
            )
            .await?;
        // HZ-057: the stance's badge over the frame, never into the game's own picture.
        if let Err(e) = self.1.badge(jvm, &g, x, y).await {
            tracing::warn!("hz: boss badge failed: {e:?}");
        }
        let reads = std::time::Instant::now();
        match scene(jvm).await {
            Ok((mode, popup, notice)) => {
                if self.0.report(mode, popup, notice) {
                    tracing::info!("hz: scene {:?}", self.0.view());
                }
                // HZ-061: a game over (7) or the title (2) ends a boss story's wait.
                if matches!(mode, Some(2 | 7)) && self.0.story_drop() {
                    tracing::info!("hz: story: the game left the field (mode {mode:?}) — the wait is dropped");
                }
                // HZ-065: ... and a quest reward no save went past goes with the progress that earned it.
                if matches!(mode, Some(2 | 7)) {
                    let dropped = self.0.rewards_drop_unsaved();
                    if dropped > 0 {
                        tracing::info!("hz: quest rewards: the game left the field unsaved (mode {mode:?}) - {dropped} dropped with their progress");
                    }
                }
                // The key the canvas holds (`f.c`), for the host's key → drawn gauge.
                if let Ok(code) = jvm.get_static_field::<i32>("f", "c", "I").await {
                    self.0.report_key(code);
                }
                self.0.lend_jvm(jvm);
                // The SAVE tab (Phase 7-1): the game's own save, between two frames.
                if self.0.take_save_ask() {
                    let bare_field = mode == Some(3) && !popup && !notice;
                    let answer = match if bare_field { quick_save(jvm).await } else { Ok(SaveAnswer::Busy) } {
                        Ok(answer) => answer,
                        Err(e) => {
                            tracing::warn!("hz: quick save failed: {e:?}");
                            SaveAnswer::Busy
                        }
                    };
                    tracing::info!("hz: quick save -> {answer:?} (mode {mode:?}, popup {popup}, notice {notice})");
                    self.0.answer_save(answer);
                }
                // The game's own windows (touch): what a tap is (taken every frame, so none waits for
                // whatever comes up next), then what is open.
                if let Some((x, y)) = self.0.take_tap() {
                    match crate::menu_touch::tap(jvm, mode, popup, notice, x, y).await {
                        Ok(t) => tracing::info!("hz: tap {x},{y} (mode {mode:?}, popup {popup}, notice {notice}) -> {t:?}"),
                        Err(e) => tracing::warn!("hz: tap {x},{y} failed: {e:?}"),
                    }
                }
                // A pad's page button (LB / RB in a window with pages), done the way its tap is.
                let (bags, slots) = (self.0.patch_on("HZ-036"), self.0.patch_on("HZ-020"));
                if let Some(d) = self.0.take_page() {
                    match crate::menu_touch::page(jvm, mode, popup, notice, d, bags, slots).await {
                        Ok(t) => tracing::info!("hz: pad page {d:+} (mode {mode:?}) -> {t:?}"),
                        Err(e) => tracing::warn!("hz: pad page {d:+} failed: {e:?}"),
                    }
                }
                // The host's own lines in the game's font (B1), a few a frame from the title on.
                if mode.is_some() {
                    let asks = self.0.take_text_asks(TEXTS_PER_FRAME);
                    if !asks.is_empty() {
                        let mut done = Vec::with_capacity(asks.len());
                        for (id, text) in asks {
                            match label(jvm, 0, &text).await {
                                Ok(l) => done.push((id, l.map(|l| Icon { key: IconKey::Text(id), ..l }))),
                                Err(e) => {
                                    tracing::warn!("hz: host text {id} failed: {e:?}");
                                    done.push((id, None));
                                }
                            }
                        }
                        self.0.report_texts(done);
                    }
                }
                if mode.is_none() && self.0.report_windows_view(false, false, false) {
                    tracing::info!("hz: scene {:?}", self.0.view());
                }
                // Whether a window of the game's is open (HZ-058: a plate is used only without one).
                let mut windows_open = false;
                if let Some(mode) = mode {
                    match crate::menu_touch::chain(jvm, mode).await {
                        Ok(c) => {
                            windows_open = !c.is_empty();
                            // HZ-036: no window open, every bag opens again on its first page.
                            if c.is_empty() {
                                crate::bags::reset();
                            }
                            // The windows' part of the scene: one over a scenario, and the 창고 question
                            // that answers only the soft keys (HZ-024 took their pictures).
                            let lsk_ok = match c.last() {
                                Some(w) if w.class == "cd" => match crate::menu_touch::top(jvm, mode).await {
                                    Ok(Some(top)) => jvm.get_field_from::<bool>(&top, "cd", "e", "Z").await.unwrap_or(false),
                                    _ => false,
                                },
                                _ => false,
                            };
                            let pager = match crate::menu_touch::pager(jvm, Some(mode), popup, notice, bags, slots).await {
                                Ok(p) => p.is_some(),
                                Err(e) => {
                                    tracing::warn!("hz: pager failed: {e:?}");
                                    false
                                }
                            };
                            if self.0.report_windows_view(mode == 4 && !c.is_empty(), lsk_ok, pager) {
                                tracing::info!("hz: scene {:?}", self.0.view());
                            }
                            let line = c.iter().map(crate::menu_touch::Win::line).collect::<Vec<_>>().join(" > ");
                            let bb_new = c.last().is_some_and(|w| w.class == "bb") && !self.0.windows_line().contains(" > bb ");
                            // HZ-020: a new slot window gets its page's count (its ctor said 3).
                            if bb_new && let Ok(Some(mut bb)) = crate::menu_touch::top(jvm, mode).await {
                                let page = self.0.slot_page();
                                if let Err(e) = fit_slot_window(jvm, &mut bb, page, false).await {
                                    tracing::warn!("hz: slot window page fit failed: {e:?}");
                                }
                            }
                            if self.0.report_windows(line.clone()) && !line.is_empty() {
                                tracing::info!("hz: windows {line}");
                            }
                            // 이어하기 starts on the slot played last.
                            if bb_new
                                && let Some((page, slot)) = self.0.preferred_slot().and_then(|s| crate::slots::page_of(s as u8))
                                && page == self.0.slot_page()
                            {
                                match crate::menu_touch::preselect_slot(jvm, slot as i8).await {
                                    Ok(true) => tracing::info!("hz: 이어하기 starts on slot {} (played last)", crate::slots::slot_number(page, slot)),
                                    Ok(false) => {}
                                    Err(e) => tracing::warn!("hz: slot preselect failed: {e:?}"),
                                }
                            }
                        }
                        Err(e) => tracing::warn!("hz: windows read failed: {e:?}"),
                    }
                }
                // The player's numbers (Phase 3), once the game screen exists.
                if mode.is_some() {
                    match stats(jvm).await {
                        Ok(Some(s)) => {
                            let before = self.0.stats();
                            if self.0.report_stats(s) {
                                // A line per level or maximum (rare); every HP tick at debug.
                                let big = before.is_none_or(|b| (b.level, b.hp_max, b.sp_max, b.mp_max, b.exp_next) != (s.level, s.hp_max, s.sp_max, s.mp_max, s.exp_next));
                                if big {
                                    tracing::info!("hz: stats {} | {}", s.lines().0, s.lines().1);
                                } else {
                                    tracing::debug!("hz: stats {} | {}", s.lines().0, s.lines().1);
                                }
                            }
                        }
                        Ok(None) => {}
                        Err(e) => tracing::warn!("hz: stats read failed: {e:?}"),
                    }
                }
                // The quick slots (Phase 6-2) — what the game's bar showed — and the portrait (6-6)
                // wherever the player's HUD is up (A7, hero2's menu mode: the menu and what it
                // opens, the minimap window), so the plates and the head follow what a skill or an
                // equipment window changes there.
                if mode.is_some_and(|m| Scene::from_mode(m) == Scene::Title) {
                    self.2.at_title();
                }
                if mode.is_some_and(|m| Scene::from_mode(m).in_play()) {
                    match self.2.frame(jvm, mode, popup, notice, windows_open).await {
                        Ok(Some(bar)) => {
                            let before = self.0.quick().map(|b| b.summary());
                            let line = bar.summary();
                            if self.0.report_quick(bar) {
                                // Contents at info (rare), cooldown ticks at debug.
                                let strip = |s: &str| s.split(' ').map(|w| w.split('~').next().unwrap_or("")).collect::<Vec<_>>().join(" ");
                                if before.as_deref().map(strip) != Some(strip(&line)) {
                                    tracing::info!("hz: quick {line}");
                                } else {
                                    tracing::debug!("hz: quick {line}");
                                }
                            }
                        }
                        Ok(None) => {}
                        Err(e) => tracing::warn!("hz: quick slots read failed: {e:?}"),
                    }
                    // The portrait (Phase 6-6): the doll composed again now and then.
                    if self.0.frames().is_multiple_of(PORTRAIT_EVERY) {
                        match portrait(jvm).await {
                            Ok(p) => {
                                let line = p.as_ref().map(|i| format!("{:?}", i.key));
                                if self.0.report_portrait(p) {
                                    tracing::info!("hz: portrait {}", line.as_deref().unwrap_or("none"));
                                }
                            }
                            Err(e) => tracing::warn!("hz: portrait read failed: {e:?}"),
                        }
                    }
                }
                if mode == Some(3) {
                    // The world map's names (Phase 7-3), once, in the game's own font.
                    if let Some(lines) = self.0.take_label_asks() {
                        let mut out = Vec::with_capacity(lines.len());
                        for (i, text) in lines.iter().enumerate() {
                            match label(jvm, i as u8, text).await {
                                Ok(l) => out.push(l),
                                Err(e) => {
                                    tracing::warn!("hz: label {i} failed: {e:?}");
                                    out.push(None);
                                }
                            }
                        }
                        tracing::info!("hz: {} of {} labels drawn in the game's font", out.iter().flatten().count(), out.len());
                        self.0.report_labels(out);
                    }
                    // The world map's places' own maps (Phase 7-5), once, through the game's loader.
                    if let Some(ids) = self.0.take_map_asks() {
                        let mut out = Vec::with_capacity(ids.len());
                        for &id in &ids {
                            match map_file(jvm, id).await {
                                Ok(g) => out.push(g),
                                Err(e) => {
                                    tracing::warn!("hz: map {id} could not be read: {e:?}");
                                    out.push(None);
                                }
                            }
                        }
                        tracing::info!("hz: {} of {} place maps read", out.iter().flatten().count(), out.len());
                        self.0.report_maps(out);
                    }
                    // The quest guidance (Phase 7-6): when a quest or the map changed.
                    if self.0.frames().is_multiple_of(GUIDE_EVERY)
                        && let Some(here) = self.0.map().map(|m| m.id)
                    {
                        match quest_guide(jvm, &self.0, here).await {
                            Ok(Some(g)) => {
                                let line = format!("quest {:?} -> map {:?}, tiles {:?}", g.quest, g.target, g.tiles.iter().filter(|t| t.0 == here).collect::<Vec<_>>());
                                if self.0.report_guide(g) {
                                    tracing::info!("hz: guide {line}");
                                }
                            }
                            Ok(None) => {}
                            Err(e) => tracing::warn!("hz: quest guide failed: {e:?}"),
                        }
                    }
                    // The minimap (Phase 6-5): the map and what moves on it.
                    if let Err(e) = minimap(jvm, &self.0).await {
                        tracing::warn!("hz: minimap read failed: {e:?}");
                    }
                }
            }
            Err(e) => tracing::warn!("hz: scene read failed: {e:?}"),
        }
        BLIT_FRAMES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        BLIT_NANOS.fetch_add(reads.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
        Ok(JavaValue::Void)
    }
}

/// The SAVE tab (Phase 7-1): the game's quick save without its question (user 2026-10-29:
/// *"저장하시겠습니까? 안뜨고 바로 저장되게"*). The field's `*` with the save setting runs `ce.a()Z`, then
/// `ed.a(6,2)` = the menu at 시스템 with 「저장하시겠습니까?」 (`ce.a(II)Z` @614); its 시스템 window `ec`
/// asks only where the map allows a save (`ec.a()Z` @36: `ed.a:Lg;`'s `g.e:Z`, else 「저장할 수 없는
/// 지역입니다.」), and its 「예」 shows 「저장중입니다.」 for a frame, then writes with `ed.k()` — slot,
/// gamedata, item, the slot the game has loaded (`ed.j`) — and closes the menu (`ec.a(BB)V`,
/// `ec.a(dd)` @33, javap 2026-10-29). Here the same two tests, then the same write, from the same
/// paint the game writes from. `ce.a()Z` is the game's own gate for the menu keys: the player
/// standing, or walking (then stopped).
async fn quick_save(jvm: &Jvm) -> JvmResult<SaveAnswer> {
    let free: bool = jvm.invoke_static("ce", "a", "()Z", []).await?;
    if !free {
        return Ok(SaveAnswer::Busy);
    }
    let Some(map) = object(jvm.get_static_field("ed", "a", "Lg;").await?) else {
        return Ok(SaveAnswer::Busy);
    };
    let allowed: bool = jvm.get_field(&map, "e", "Z").await?;
    if !allowed {
        return Ok(SaveAnswer::NotHere);
    }
    let _: () = jvm.invoke_static("ed", "k", "()V", []).await?;
    Ok(SaveAnswer::Saved)
}

/// The player's numbers, through the game's own accessors (`hz_java::state::Stats` says which
/// index is what). `None` while there is no player (`ed.a` null: title, loading).
async fn stats(jvm: &Jvm) -> JvmResult<Option<Stats>> {
    let Some(player) = object(jvm.get_static_field("ed", "a", "Lek;").await?) else {
        return Ok(None);
    };
    let mut cur = [0i32; 4];
    let mut max = [0i32; 4];
    for i in 0..4u8 {
        cur[i as usize] = jvm.invoke_virtual(&player, "e", "(B)I", [JavaValue::Byte(i as i8)]).await?;
        max[i as usize] = jvm.invoke_virtual(&player, "f", "(B)I", [JavaValue::Byte(i as i8)]).await?;
    }
    let level: i8 = jvm.get_field(&player, "n", "B").await?;
    // The unspent points (Phase 6-3): `ek.a:S` stat, `ek.b:S` skill.
    let stat_points: i16 = jvm.get_field(&player, "a", "S").await?;
    let skill_points: i16 = jvm.get_field(&player, "b", "S").await?;
    Ok(Some(Stats {
        hp: cur[0],
        hp_max: max[0],
        sp: cur[1],
        sp_max: max[1],
        exp: cur[2],
        exp_next: max[2],
        mp: cur[3],
        mp_max: max[3],
        level: i32::from(level),
        stat_points: i32::from(stat_points),
        skill_points: i32::from(skill_points),
        guardian: state_guardian(jvm, &player, level).await?,
        round: replay_round(jvm).await?,
    }))
}

// ---------------------------------------------------------------------- HZ-058 the plates ----

/// HZ-058: the plates over the game's quick slots (`crate::quickpage`, hero2's seven × two). The blit
/// hook acts on the player's asks and reports the page shown ([`Quick::frame`]); the field tick is the
/// cooldowns' clock (`GuestState::field_ticks`); the skill and potion popups' rows put entries on the
/// plates or take them off ([`QuickRegister`], [`QuickPotion`]).
struct Quick {
    state: Arc<GuestState>,
    /// Where [`quickpage::FILE`] is kept (`None`, the probe: in memory).
    dir: Option<std::path::PathBuf>,
    inner: std::sync::Mutex<QuickInner>,
}

#[derive(Default)]
struct QuickInner {
    store: quickpage::Store,
    owner: Option<quickpage::Owner>,
    cooldowns: quickpage::Cooldowns,
    /// The frame each game slot was last used from a plate (the seat's LRU).
    last_used: [u64; quickpage::GAME_SLOTS],
    page: u8,
    /// A popup's 「퀵슬롯등록」: what goes on the next plate pressed.
    picking: Option<quickpage::Entry>,
}

impl QuickInner {
    /// The plates are `owner`'s now. Another character (or job) — whose skill numbers mean other
    /// skills — starts with none of the last one's cooldowns or seats, on page 1, nothing picking.
    /// Whether it changed.
    fn own(&mut self, owner: quickpage::Owner) -> bool {
        if self.owner == Some(owner) {
            return false;
        }
        self.leave();
        self.owner = Some(owner);
        self.page = 0;
        true
    }

    /// Nobody's: the cooldowns and seats forgotten (the title — [`Quick::at_title`]).
    fn leave(&mut self) {
        self.owner = None;
        self.cooldowns = quickpage::Cooldowns::default();
        self.last_used = [0; quickpage::GAME_SLOTS];
        self.picking = None;
    }
}

/// The game's six slots (`ed.b:[B`) and their cooldowns, left and full (`ed.a:[I` / `ed.b:[I`).
struct GameSlots {
    ids: [i8; quickpage::GAME_SLOTS],
    left: [i32; quickpage::GAME_SLOTS],
    full: [i32; quickpage::GAME_SLOTS],
}

async fn game_slots(jvm: &Jvm) -> JvmResult<Option<GameSlots>> {
    const N: usize = quickpage::GAME_SLOTS;
    let (Some(ids), Some(left), Some(full)) = (
        object(jvm.get_static_field("ed", "b", "[B").await?),
        object(jvm.get_static_field("ed", "a", "[I").await?),
        object(jvm.get_static_field("ed", "b", "[I").await?),
    ) else {
        return Ok(None);
    };
    if jvm.array_length(&ids).await? < N || jvm.array_length(&left).await? < N || jvm.array_length(&full).await? < N {
        return Ok(None);
    }
    let ids: Vec<i8> = jvm.load_array(&ids, 0, N).await?;
    let left: Vec<i32> = jvm.load_array(&left, 0, N).await?;
    let full: Vec<i32> = jvm.load_array(&full, 0, N).await?;
    Ok(Some(GameSlots { ids: ids.try_into().unwrap_or([-1; N]), left: left.try_into().unwrap_or([0; N]), full: full.try_into().unwrap_or([0; N]) }))
}

/// A potion's stock: the game's own count in its bag (`ed.a(B)Lbt` → `bt.a(BB)I`, `/itm/20`).
async fn potion_count(jvm: &Jvm, id: i8) -> JvmResult<i32> {
    Ok(match object(jvm.invoke_static("ed", "a", "(B)Lbt;", [JavaValue::Byte(20)]).await?) {
        Some(bag) => jvm.invoke_virtual(&bag, "a", "(BB)I", [JavaValue::Byte(20), JavaValue::Byte(id)]).await?,
        None => 0,
    })
}

/// A game slot's own use — what its key does in the field (`ce.a(II)Z`): '1' '3' '7' '9' are
/// `ce.a(0..3)`, '0' and '#' the player's `ek.a(Z)Z` (the `0` slot, the `#` slot) into `ce.d:Z`.
async fn use_game_slot(jvm: &Jvm, g: usize) -> JvmResult<()> {
    if g < 4 {
        let _: () = jvm.invoke_static("ce", "a", "(I)V", [JavaValue::Int(g as i32)]).await?;
        return Ok(());
    }
    let Some(player) = object(jvm.get_static_field("ed", "a", "Lek;").await?) else {
        return Ok(());
    };
    let used: bool = jvm.invoke_virtual(&player, "a", "(Z)Z", [JavaValue::Boolean(g == 4)]).await?;
    jvm.put_static_field("ce", "d", "Z", JavaValue::Boolean(used)).await?;
    Ok(())
}

impl Quick {
    fn new(state: Arc<GuestState>, dir: Option<&std::path::Path>) -> Quick {
        let store = dir.and_then(|d| std::fs::read_to_string(d.join(quickpage::FILE)).ok()).map(|t| quickpage::Store::parse(&t)).unwrap_or_default();
        Quick { state, dir: dir.map(std::path::Path::to_path_buf), inner: std::sync::Mutex::new(QuickInner { store, ..Default::default() }) }
    }

    fn on(&self) -> bool {
        self.state.patch_on("HZ-058")
    }

    fn inner(&self) -> std::sync::MutexGuard<'_, QuickInner> {
        self.inner.lock().unwrap()
    }

    /// The file, whole, through a temporary — a write cut short leaves the one before.
    fn save(&self, store: &quickpage::Store) {
        let Some(dir) = &self.dir else { return };
        let path = dir.join(quickpage::FILE);
        let tmp = path.with_extension("tmp");
        if let Err(e) = std::fs::write(&tmp, store.format()).and_then(|()| std::fs::rename(&tmp, &path)) {
            tracing::warn!("hz: quick plates not saved: {e}");
        }
    }

    /// Whose plates now: the save slot loaded (HZ-020's page and the game's own `ed.j:B`, what `ed.k()`
    /// saves into) and the job `ed.b:B`.
    async fn owner(&self, jvm: &Jvm) -> JvmResult<quickpage::Owner> {
        let index: i8 = jvm.get_static_field("ed", "j", "B").await?;
        let class: i8 = jvm.get_static_field("ed", "b", "B").await?;
        let slot = crate::slots::slot_number(self.state.slot_page(), index.max(0) as u8);
        Ok(quickpage::Owner { slot: slot.min(u32::from(u8::MAX)) as u8, class })
    }

    /// Back at the title: what is loaded next — another slot, or this one again with a new character
    /// of the same job — starts from its own slots' cooldowns, not the last game's. (Not on every way
    /// out of play: a cutscene keeps the cooldowns it interrupted.)
    fn at_title(&self) {
        self.inner().leave();
    }

    /// A drawn frame in play: the cooldowns looked at, the asks acted on (a plate is used only in the
    /// bare field; with a popup's 「퀵슬롯등록」 waiting it takes the entry), and the page shown.
    async fn frame(&self, jvm: &Jvm, mode: Option<u8>, popup: bool, notice: bool, windows: bool) -> JvmResult<Option<QuickBar>> {
        let asks = self.state.take_plate_asks();
        let field = mode == Some(3) && !popup && !notice && !windows;
        if !self.on() {
            // The game's six as they were: a plate is its slot's key.
            if field {
                for a in &asks {
                    if let PlateAsk::Press(n) = *a
                        && usize::from(n) < quickpage::GAME_SLOTS
                    {
                        use_game_slot(jvm, usize::from(n)).await?;
                    }
                }
            }
            return quick_bar(jvm, &self.state).await;
        }
        let Some(game) = game_slots(jvm).await? else {
            return Ok(None);
        };
        let owner = self.owner(jvm).await?;
        let (tick, frame) = (self.state.field_ticks(), self.state.frames());
        let mut uses = Vec::new();
        {
            let mut guard = self.inner();
            let q = &mut *guard;
            let four = |a: &[i32; quickpage::GAME_SLOTS]| [a[0], a[1], a[2], a[3]];
            // Before the new character's slots are looked at.
            if q.own(owner) {
                if !q.store.rows.contains_key(&owner) {
                    q.store.rows.insert(owner, quickpage::Plates::adopt(&game.ids));
                    self.save(&q.store);
                }
                tracing::info!("hz: quick plates of slot {} job {}: {}", owner.slot, owner.class, q.store.rows[&owner].tokens());
            }
            q.cooldowns.observe(&game.ids, &four(&game.left), &four(&game.full), tick);
            if q.picking.is_some() && !windows {
                tracing::info!("hz: quick {:?}: no plate picked", q.picking.take());
            }
            for a in asks {
                match a {
                    PlateAsk::Page => q.page = (q.page + 1) % quickpage::PAGES as u8,
                    PlateAsk::Press(n) => {
                        let (page, n) = (usize::from(q.page), usize::from(n));
                        if let Some(e) = q.picking.take() {
                            if let Some(p) = q.store.rows.get_mut(&owner) {
                                p.set(page, n, e);
                            }
                            self.save(&q.store);
                            tracing::info!("hz: quick {e:?} -> page {} plate {}", page + 1, n + 1);
                        } else if field && let Some(e) = q.store.rows.get(&owner).and_then(|p| p.get(page, n)) {
                            uses.push(e);
                        }
                    }
                }
            }
        }
        for e in uses {
            self.use_entry(jvm, e, frame).await?;
        }
        self.bar(jvm, owner).await
    }

    /// Seat `e` in a game slot of its kind ([`quickpage::seat`]) and use it there the game's way. A
    /// skill not learned (level 0 — a plate of an older character in the slot) is not seated. A seat
    /// that changes takes the skill's own cooldown: the game's `ed.g(I)V` puts the slot's full one
    /// (`ek.d(B)I` / the spirit's `p.d(B)I`) and the left is what the skill had (`Cooldowns`).
    async fn use_entry(&self, jvm: &Jvm, e: quickpage::Entry, frame: u64) -> JvmResult<()> {
        use quickpage::Entry;
        let Some(game) = game_slots(jvm).await? else { return Ok(()) };
        let Some(player) = object(jvm.get_static_field("ed", "a", "Lek;").await?) else { return Ok(()) };
        let spirit = match e {
            Entry::Spirit(_) => object(jvm.invoke_virtual(&player, "a", "()Lp;", ()).await?),
            _ => None,
        };
        let level: i8 = match (e, &spirit) {
            (Entry::Skill(v), _) => jvm.invoke_virtual(&player, "a", "(B)B", [JavaValue::Byte(v)]).await?,
            (Entry::Spirit(v), Some(p)) => jvm.invoke_virtual(p, "a", "(B)B", [JavaValue::Byte(v)]).await?,
            (Entry::Spirit(_), None) => 0,
            (Entry::Potion(_), _) => 1,
        };
        let learned = level > 0;
        if !learned {
            tracing::info!("hz: quick {e:?} not learned — not used");
            return Ok(());
        }
        let g = quickpage::seat(e, &game.ids, &self.inner().last_used);
        if game.ids[g] != e.value() {
            let Some(mut ids) = object(jvm.get_static_field("ed", "b", "[B").await?) else { return Ok(()) };
            jvm.store_array(&mut ids, g, vec![e.value()]).await?;
            if g < 4 {
                let _: () = jvm.invoke_static("ed", "g", "(I)V", [JavaValue::Int(g as i32)]).await?;
                let (Some(mut left), Some(full)) = (object(jvm.get_static_field("ed", "a", "[I").await?), object(jvm.get_static_field("ed", "b", "[I").await?)) else {
                    return Ok(());
                };
                let full: i32 = jvm.load_array::<i32>(&full, g, 1).await?.pop().unwrap_or(0);
                let had = self.inner().cooldowns.of(e).0;
                jvm.store_array(&mut left, g, vec![had.min(full).max(0)]).await?;
                // A spirit slot: what the game's registration does after it (`ed.d(IB)V` @62..69).
                if g >= 2
                    && let Some(p) = &spirit
                {
                    let _: () = jvm.invoke_virtual(p, "b", "(I)V", [JavaValue::Int(g as i32)]).await?;
                }
                tracing::info!("hz: quick {e:?} seated in game slot {g}, cooldown {} of {full}", had.min(full).max(0));
            } else {
                jvm.put_static_field("ce", "d", "Z", JavaValue::Boolean(true)).await?;
                tracing::info!("hz: quick {e:?} seated in game slot {g}");
            }
        }
        self.inner().last_used[g] = frame;
        use_game_slot(jvm, g).await
    }

    /// The page shown, as the HUD draws it: each plate's picture, a potion's stock, the cooldown —
    /// the game's own while the entry is seated, the host's while it is not.
    async fn bar(&self, jvm: &Jvm, owner: quickpage::Owner) -> JvmResult<Option<QuickBar>> {
        use quickpage::Entry;
        let Some(game) = game_slots(jvm).await? else { return Ok(None) };
        let (plates, page, picking, cooldowns) = {
            let q = self.inner();
            (q.store.rows.get(&owner).copied().unwrap_or_default(), q.page, q.picking.is_some(), q.cooldowns.clone())
        };
        let mut bar = QuickBar { page, picking, ..QuickBar::default() };
        for (i, e) in plates.pages[usize::from(page) % quickpage::PAGES].iter().enumerate() {
            let Some(e) = *e else { continue };
            let seated = e.slots().iter().copied().find(|&g| game.ids[g] == e.value());
            bar.slots[i] = match e {
                Entry::Skill(v) | Entry::Spirit(v) => {
                    let key = if matches!(e, Entry::Skill(_)) { IconKey::CharSkill { class: owner.class, cell: v - 4 } } else { IconKey::SpiritSkill(v) };
                    let (cooldown, cooldown_max) = match seated {
                        Some(g) => (game.left[g], game.full[g]),
                        None => cooldowns.of(e),
                    };
                    QuickSlot { icon: icon(jvm, &self.state, key).await?, count: 0, cooldown, cooldown_max, item: false }
                }
                Entry::Potion(v) => QuickSlot {
                    icon: icon(jvm, &self.state, IconKey::Item(20 + i16::from(v))).await?,
                    count: potion_count(jvm, v).await?,
                    cooldown: game.left[4],
                    cooldown_max: POTION_COOLDOWN,
                    item: true,
                },
            };
        }
        for (i, p) in bar.points.iter_mut().enumerate() {
            *p = icon(jvm, &self.state, IconKey::Points(i as i8)).await?;
        }
        bar.buffs = buffs(jvm, &self.state, owner.class).await?;
        Ok(Some(bar))
    }

    /// A popup's row: 「퀵슬롯등록」 (`add`) waits for the plate the player presses next; 「퀵슬롯해제」
    /// takes `e` off every plate and out of the game's slots.
    async fn register(&self, jvm: &Jvm, e: quickpage::Entry, add: bool) -> JvmResult<()> {
        if add {
            self.inner().picking = Some(e);
            tracing::info!("hz: quick {e:?}: pick a plate");
            return Ok(());
        }
        let removed = {
            let mut guard = self.inner();
            let q = &mut *guard;
            q.picking = None;
            let removed = q.owner.and_then(|o| q.store.rows.get_mut(&o)).is_some_and(|p| p.remove(e));
            if removed {
                self.save(&q.store);
            }
            removed
        };
        if let Some(mut ids) = object(jvm.get_static_field("ed", "b", "[B").await?) {
            let now: Vec<i8> = jvm.load_array(&ids, 0, quickpage::GAME_SLOTS).await?;
            for &g in e.slots() {
                if now[g] == e.value() {
                    jvm.store_array(&mut ids, g, vec![-1i8]).await?;
                }
            }
        }
        tracing::info!("hz: quick {e:?} off the plates ({removed})");
        Ok(())
    }
}

/// HZ-058 `quickRegister(int slot, byte skill)V` — in place of `ed.d(IB)V` where a skill window's popup
/// row is chosen (`cb.a(BBBBZ)V` @143 / @180): off, the game's registration; on, the first row (slot 0,
/// or 2 for the spirit) is 「퀵슬롯등록」 and the second 「퀵슬롯해제」.
struct QuickRegister(Arc<Quick>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for QuickRegister {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let slot = int(args[0].clone());
        let skill: i8 = args[1].clone().into();
        if !self.0.on() {
            let _: () = jvm.invoke_static("ed", "d", "(IB)V", [JavaValue::Int(slot), JavaValue::Byte(skill)]).await?;
            return Ok(JavaValue::Void);
        }
        let e = if slot < 2 { quickpage::Entry::Skill(skill) } else { quickpage::Entry::Spirit(skill) };
        self.0.register(jvm, e, slot % 2 == 0).await?;
        Ok(JavaValue::Void)
    }
}

/// HZ-058 `quickPotion(boolean first, byte potion)V` — in place of `ed.a(ZB)V` where an item's popup row
/// is chosen (`ds.a(BB)V` @72): off, the potion into the `0` (first) or `#` slot; on, 「퀵슬롯등록」 /
/// 「퀵슬롯해제」.
struct QuickPotion(Arc<Quick>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for QuickPotion {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let first: bool = args[0].clone().into();
        let potion: i8 = args[1].clone().into();
        if !self.0.on() {
            let _: () = jvm.invoke_static("ed", "a", "(ZB)V", [JavaValue::Boolean(first), JavaValue::Byte(potion)]).await?;
            return Ok(JavaValue::Void);
        }
        self.0.register(jvm, quickpage::Entry::Potion(potion), first).await?;
        Ok(JavaValue::Void)
    }
}

/// The six quick slots as the game's bar draws them (javap `ce.f()` @821.., `ce.a(ZII)Z`,
/// 2026-10-23). `ed.b:[B` holds the contents: [0..3] skill ids (< 0 = empty; `ed.b(I)Z` = slots 0
/// and 1 are the character's, 2 and 3 the spirit's), [4] / [5] the `0` / `#` potion ids, looked
/// up the game's way (`ed.a(Z)Lba` — the potion bag `ed.c`, null if none). Cooldowns `ed.a:[I`
/// (left) / `ed.b:[I` (full); both potions share `ed.a[4]`, out of [`POTION_COOLDOWN`]. The stock
/// is the game's own count, `ed.a(B)Lbt` → `bt.a(BB)I`. All read-only — what the painter calls.
async fn quick_bar(jvm: &Jvm, state: &GuestState) -> JvmResult<Option<QuickBar>> {
    let (Some(ids), Some(left), Some(full)) = (
        object(jvm.get_static_field("ed", "b", "[B").await?),
        object(jvm.get_static_field("ed", "a", "[I").await?),
        object(jvm.get_static_field("ed", "b", "[I").await?),
    ) else {
        return Ok(None);
    };
    if jvm.array_length(&ids).await? < quickpage::GAME_SLOTS || jvm.array_length(&left).await? < 5 || jvm.array_length(&full).await? < 4 {
        return Ok(None);
    }
    let ids: Vec<i8> = jvm.load_array(&ids, 0, quickpage::GAME_SLOTS).await?;
    let left: Vec<i32> = jvm.load_array(&left, 0, 5).await?;
    let full: Vec<i32> = jvm.load_array(&full, 0, 4).await?;
    let class: i8 = jvm.get_static_field("ed", "b", "B").await?;
    let mut bar = QuickBar::default();
    for (i, &v) in ids.iter().take(4).enumerate() {
        if v < 0 {
            continue;
        }
        let key = if i < 2 { IconKey::CharSkill { class, cell: v - 4 } } else { IconKey::SpiritSkill(v) };
        bar.slots[i] = QuickSlot { icon: icon(jvm, state, key).await?, count: 0, cooldown: left[i], cooldown_max: full[i], item: false };
    }
    for (i, first) in [(4usize, true), (5, false)] {
        let Some(item) = object(jvm.invoke_static("ed", "a", "(Z)Lba;", [JavaValue::Boolean(first)]).await?) else {
            continue;
        };
        let cat: i8 = jvm.get_field(&item, "a", "B").await?;
        let id: i8 = jvm.get_field(&item, "b", "B").await?;
        let count = match object(jvm.invoke_static("ed", "a", "(B)Lbt;", [JavaValue::Byte(cat)]).await?) {
            Some(bag) => jvm.invoke_virtual(&bag, "a", "(BB)I", [JavaValue::Byte(cat), JavaValue::Byte(id)]).await?,
            None => 0,
        };
        let index = i16::from(cat) + if cat == 20 { i16::from(id) } else { 0 };
        bar.slots[i] = QuickSlot { icon: icon(jvm, state, IconKey::Item(index)).await?, count, cooldown: left[4], cooldown_max: POTION_COOLDOWN, item: true };
    }
    // The unspent-point icons (6-3), read once each like the slots'.
    for (i, p) in bar.points.iter_mut().enumerate() {
        *p = icon(jvm, state, IconKey::Points(i as i8)).await?;
    }
    bar.buffs = buffs(jvm, state, class).await?;
    Ok(Some(bar))
}

/// The timed states (`BuffKind`) running now — reads of the fields the player's tick `ek.c()V`
/// counts down, nothing written.
async fn buffs(jvm: &Jvm, state: &GuestState, class: i8) -> JvmResult<Vec<Buff>> {
    let Some(player) = object(jvm.get_static_field("ed", "a", "Lek;").await?) else {
        return Ok(Vec::new());
    };
    let tick_ms: i32 = jvm.get_static_field("rpg/c", "b", "I").await?;
    let tick_ms = i64::from(tick_ms);
    let form: i32 = jvm.get_field(&player, "b", "I").await?;
    let spirit: i32 = jvm.get_field(&player, "j", "I").await?;
    let guard_ms: i64 = jvm.get_static_field("ek", "a", "J").await?;
    let mut out = Vec::new();
    if form > 0 {
        let icon = icon(jvm, state, IconKey::CharSkill { class, cell: 5 - 4 }).await?;
        out.push(Buff { kind: BuffKind::Form, icon, secs: buff_secs(i64::from(form), tick_ms) });
    }
    if spirit > 0 {
        let skill = match object(jvm.invoke_virtual(&player, "a", "()Lp;", ()).await?) {
            Some(p) => Some(jvm.get_field::<i8>(&p, "a", "B").await?),
            None => None,
        };
        let icon = match skill {
            Some(id) if id >= 0 => icon(jvm, state, IconKey::SpiritSkill(id)).await?,
            _ => None,
        };
        out.push(Buff { kind: BuffKind::Spirit, icon, secs: buff_secs(i64::from(spirit), tick_ms) });
    }
    if guard_ms > 0 {
        let icon = icon(jvm, state, IconKey::Orb).await?;
        out.push(Buff { kind: BuffKind::Guard, icon, secs: buff_secs(guard_ms, 1) });
    }
    Ok(out)
}

/// How often the minimap walks the map's draw list (monsters, chests) and re-reads its tiles, in
/// drawn frames — hero2's `MM_MOBS_EVERY`. The player is read every frame.
const MAP_LIST_EVERY: u64 = 3;
/// The longest draw list walked: a guard against a list the game is relinking, not a size it nears.
const MAP_LIST_MAX: usize = 512;

/// The minimap (Phase 6-5, javap `g` `er` `cq` `eu` 2026-10-23). The map `ed.a:Lg;` keeps every
/// entity it draws on one y-sorted list — head `g.a:Ler;`, next `er.b:Ler;` (`g.f(Ler;)`): the
/// player `ek`, monsters `cq` (dying while `ar.b:B` is 6 or 7 — `l.b()Z` — then taken off it for
/// the respawn queue `cq.c`), NPCs `bu`, objects `eu`. An `eu` is a chest when the map's table gave
/// it a flag (`eu.a:I` ≠ −1, from `g.a(II)I`); see [`chest_opened`]. The tiles are
/// [`MapGrid`]'s. Everything here is a read the game's own painters make. Probe `--warp` on maps
/// 0 4 27 44 46 (2026-10-23): the counts match the map files and the game's own chest table.
/// A second list, head `g.c:Ler;` (same `er.b` links), holds the big bosses' parts — kinds 101..108
/// and 124..131, which `ak.a(BB)Z` puts there with `g.b(Ler;)V` (maps 124 126 132 133, javap
/// 2026-10-27); its monsters are dots too.
async fn minimap(jvm: &Jvm, state: &GuestState) -> JvmResult<()> {
    let Some(map) = object(jvm.get_static_field("ed", "a", "Lg;").await?) else {
        return Ok(());
    };
    let mut marks = MapMarks::default();
    if let Some(player) = object(jvm.get_static_field("ed", "a", "Lek;").await?) {
        marks.player = Some(position(jvm, &player).await?);
    }
    let known = state.map();
    if known.is_some() && state.frames() % MAP_LIST_EVERY != 0 {
        marks.mobs = state.marks().mobs;
        state.report_marks(marks);
        return Ok(());
    }

    let id: i8 = jvm.get_field(&map, "b", "B").await?;
    let w: i8 = jvm.get_static_field("g", "c", "B").await?;
    let h: i8 = jvm.get_static_field("g", "d", "B").await?;
    let another = known.as_ref().is_none_or(|k| (k.id, k.w, k.h) != (id as u8, w as u8, h as u8));

    let mut chests = Vec::new();
    // What the list holds, by class — logged once per map (the census is how monsters were told
    // from the rest: probe, maps 44/46 list only `cq` and `ek`).
    let mut seen: std::collections::BTreeMap<String, usize> = Default::default();
    for head in ["a", "c"] {
        let mut next = object(jvm.get_field(&map, head, "Ler;").await?);
        for _ in 0..MAP_LIST_MAX {
            let Some(e) = next else { break };
            if another {
                *seen.entry(e.class_definition().name()).or_default() += 1;
            }
            if jvm.is_instance(&*e, "cq") {
                let st: i8 = jvm.get_field(&e, "b", "B").await?;
                if st != 6 && st != 7 {
                    marks.mobs.push(position(jvm, &e).await?);
                }
            } else if jvm.is_instance(&*e, "eu") {
                let flag: i32 = jvm.get_field(&e, "a", "I").await?;
                if flag != -1 {
                    // `eu` stands on the bottom middle of its tile (`eu.<init>`: `(x − 8) >> 4, (y − 16) >> 4`).
                    let x: i16 = jvm.get_field(&e, "h", "S").await?;
                    let y: i16 = jvm.get_field(&e, "i", "S").await?;
                    let (tx, ty) = ((i32::from(x) - 8) >> 4, (i32::from(y) - 16) >> 4);
                    if (0..=255).contains(&tx) && (0..=255).contains(&ty) {
                        chests.push((tx as u8, ty as u8, !chest_opened(jvm, flag).await?));
                    }
                }
            }
            next = object(jvm.get_field(&e, "b", "Ler;").await?);
        }
    }

    let n = usize::from(w.max(0) as u8) * usize::from(h.max(0) as u8);
    let grid = match object(jvm.get_field(&map, "a", "[B").await?) {
        Some(layer) if n > 0 && jvm.array_length(&layer).await? >= n => {
            let attrs: Vec<i8> = jvm.load_array(&layer, 0, n).await?;
            MapGrid::new(id as u8, w as u8, h as u8, &attrs, &chests).map(|g| (g, attrs))
        }
        _ => None,
    };
    // The game window's title for the map, lettered once per map, and which of its triggers lead
    // to another map, read once per map from its file.
    let grid = match grid {
        Some((mut g, attrs)) => {
            let same = known.as_ref().filter(|_| !another);
            g.title = match same {
                Some(k) => k.title.clone(),
                None => map_title(jvm, &map, g.id).await?,
            };
            let (leaving, fights) = match same.and_then(|k| Some((k.leaving.clone()?, k.fights?))) {
                Some(known) => known,
                None => file_facts(jvm, g.id).await?,
            };
            g.fights = Some(fights);
            Some(g.with_leaving(&attrs, leaving))
        }
        None => None,
    };
    if let Some(grid) = grid {
        let (floor, exits, closed) = grid.census();
        let doors = grid.doors().iter().filter(|&&c| c == crate::state::MM_EXIT).count();
        let (id, w, h, fights) = (grid.id, grid.w, grid.h, grid.fights.unwrap_or(true));
        if state.report_map(grid) {
            if another {
                let list = seen.iter().map(|(c, k)| format!("{c} {k}")).collect::<Vec<_>>().join(", ");
                tracing::info!("hz: map {id} {w}x{h}: floor {floor} exit {exits} (doors {doors}) chest {closed} fights {fights} | draw list: {list}");
            } else {
                tracing::info!("hz: map {id}: tiles or chests changed (exit {exits} chest {closed})");
            }
        }
    }
    if state.report_marks(marks.clone()) {
        tracing::debug!("hz: marks player {:?} mobs {}", marks.player, marks.mobs.len());
    }
    Ok(())
}

/// Whether the game draws chest `flag` as opened — `eu.b(II)`'s own test: 9 and 19..=21 are
/// opened once the bag holds item category 19 number `flag` (`bt.a(BB)I` > 0), any other flag
/// once the game's event flag `ed.a(I)Z` is set.
async fn chest_opened(jvm: &Jvm, flag: i32) -> JvmResult<bool> {
    if flag == 9 || (19..=21).contains(&flag) {
        let Some(bag) = object(jvm.invoke_static("ed", "a", "(B)Lbt;", [JavaValue::Byte(19)]).await?) else {
            return Ok(false);
        };
        let n: i32 = jvm.invoke_virtual(&bag, "a", "(BB)I", [JavaValue::Byte(19), JavaValue::Byte(flag as i8)]).await?;
        return Ok(n > 0);
    }
    Ok(flag > 0 && jvm.invoke_static::<_, bool>("ed", "a", "(I)Z", [JavaValue::Int(flag)]).await?)
}

/// An entity's minimap mark from its pixel position `er.h:S` / `er.i:S`.
#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
async fn position(jvm: &Jvm, e: &Box<dyn ClassInstance>) -> JvmResult<(f32, f32)> {
    let x: i16 = jvm.get_field(e, "h", "S").await?;
    let y: i16 = jvm.get_field(e, "i", "S").await?;
    Ok(mark_of(x, y))
}

/// The icon for `key`: from the cache, or read once from the game's own picture (and an
/// unreadable one remembered as such, so it is not tried every frame).
async fn icon(jvm: &Jvm, state: &GuestState, key: IconKey) -> JvmResult<Option<Icon>> {
    if let Some(known) = state.cached_icon(key) {
        return Ok(known);
    }
    let read = read_icon(jvm, key).await?;
    match &read {
        Some(i) => tracing::info!("hz: icon {key:?} read, {}x{}", i.w, i.h),
        None => tracing::warn!("hz: icon {key:?}: no picture"),
    }
    state.cache_icon(key, read.clone());
    Ok(read)
}

/// `cw` → its `Image` (`cw.a`) → `Image.getRGB` over the part the game draws: a 12×12 cell of the
/// class strip for a character skill (`ce.a(III)` clips to it), the whole picture otherwise (the
/// unspent-point icons are `rpg/e.c:[Lcw;`, drawn whole by `ce.f()` @207/@236).
async fn read_icon(jvm: &Jvm, key: IconKey) -> JvmResult<Option<Icon>> {
    let entry = |name: &'static str, index: i32| async move {
        let Some(list) = object(jvm.get_static_field("rpg/e", name, "[Lcw;").await?) else {
            return Ok::<_, JavaError>(None);
        };
        if index < 0 || index as usize >= jvm.array_length(&list).await? {
            return Ok(None);
        }
        let got: Vec<JavaValue> = jvm.load_array(&list, index as usize, 1).await?;
        Ok(got.into_iter().next().and_then(object))
    };
    let (cw, cell) = match key {
        IconKey::CharSkill { cell, .. } if cell >= 0 => (object(jvm.get_static_field("rpg/e", "f", "Lcw;").await?), Some(i32::from(cell))),
        IconKey::CharSkill { .. } => return Ok(None),
        IconKey::SpiritSkill(id) => (entry("g", i32::from(id)).await?, None),
        // HZ-060's elixir is past the pack: its own picture.
        IconKey::Item(i) if i32::from(i) == hooks::ELIXIR_ICON => (elixir_picture(jvm).await?, None),
        IconKey::Item(i) => (entry("k", i32::from(i)).await?, None),
        IconKey::Points(i) => (entry("c", i32::from(i)).await?, None),
        IconKey::Orb => (Some(orb_picture(jvm).await?), None),
        // Composed by `portrait` / drawn by `label`, never read as one picture.
        IconKey::Portrait(_) | IconKey::Label(_) | IconKey::MapTitle(_) | IconKey::Text(_) | IconKey::Pickup(_) => return Ok(None),
    };
    let Some(cw) = cw else { return Ok(None) };
    let rect = cell.map(|c| (c * 12, 0, 12, 12));
    Ok(picture(jvm, &cw, rect).await?.map(|(w, h, px)| Icon { key, w, h, px: px.into() }))
}

/// A `cw`'s pixels: its `Image` (`cw.a`) through `Image.getRGB` (the vendor fork), the whole
/// picture or the `(x, y, w, h)` part of it — `None` for no image or a part outside it.
#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
async fn picture(jvm: &Jvm, cw: &Box<dyn ClassInstance>, rect: Option<(i32, i32, i32, i32)>) -> JvmResult<Option<(u32, u32, Vec<u32>)>> {
    let Some(image) = object(jvm.get_field(cw, "a", "Ljavax/microedition/lcdui/Image;").await?) else {
        return Ok(None);
    };
    image_rgb(jvm, &image, rect).await
}

/// From map `id`'s file, read by the game's loader: the triggers whose scripts can take the player
/// to another map, and whether it places monsters. Every trigger — the game window's way — and a
/// fight map if the file does not parse.
async fn file_facts(jvm: &Jvm, id: u8) -> JvmResult<(Arc<[u8]>, bool)> {
    let file = resource(jvm, &MapGrid::path(id)).await?.and_then(|raw| crate::mapfile::MapFile::parse(&raw));
    Ok(match file {
        Some(m) => (m.leaving(id).into(), m.fights),
        None => {
            tracing::warn!("hz: map {id}: its file does not parse — every trigger stays a door");
            ((1..64u8).collect::<Vec<_>>().into(), true)
        }
    })
}

/// Map `id`'s grid from its file, read by the game's own resource loader `rpg/e.a(String)[B` (the
/// one `rpg/e.g()` loads a map with).
async fn map_file(jvm: &Jvm, id: u8) -> JvmResult<Option<MapGrid>> {
    Ok(resource(jvm, &MapGrid::path(id)).await?.and_then(|raw| MapGrid::from_map_file(id, &raw)))
}

/// A jar resource through the game's loader `rpg/e.a(String)[B` (`getResourceAsStream`).
async fn resource(jvm: &Jvm, path: &str) -> JvmResult<Option<Vec<u8>>> {
    let bytes = byte_array(jvm, path.as_bytes()).await?;
    let path = jvm.new_class("java/lang/String", "([B)V", (bytes,)).await?;
    let Some(bytes) = object(jvm.invoke_static("rpg/e", "a", "(Ljava/lang/String;)[B", [JavaValue::Object(Some(path))]).await?) else {
        return Ok(None);
    };
    let n = jvm.array_length(&bytes).await?;
    let raw: Vec<i8> = jvm.load_array(&bytes, 0, n).await?;
    Ok(Some(raw.into_iter().map(|b| b as u8).collect()))
}

/// How often the field checks the quest guidance (Phase 7-6), in drawn frames: a check is the
/// quest bits and the map number; the work only when either changed.
const GUIDE_EVERY: u64 = 20;

/// The game's own quest guidance (Phase 7-6), from where the game keeps it: the quest under way by
/// `g.a()V`'s rule over `ed.ad:[B` (2 bits a quest, `ed.a(I)B`), its guide tile in each map
/// (`/mmi/<map>`), and the trail those tiles make — from the player's map, through the step
/// trigger under each guide tile (`crate::mapfile`), to the first map whose tile is not one.
/// `None` = nothing changed since the last check.
async fn quest_guide(jvm: &Jvm, state: &GuestState, here: u8) -> JvmResult<Option<QuestGuide>> {
    let Some(bits) = object(jvm.get_static_field("ed", "ad", "[B").await?) else {
        return Ok(None);
    };
    let n = jvm.array_length(&bits).await?;
    let raw: Vec<i8> = jvm.load_array(&bits, 0, n).await?;
    let raw: Vec<u8> = raw.into_iter().map(|b| b as u8).collect();
    if !state.guide_stale(&raw, here) {
        return Ok(None);
    }
    let quest = mapfile::guided_quest(|q| raw.get(usize::from(q) / 4).map_or(0, |b| (b >> ((3 - q % 4) * 2)) & 3));
    let mut g = QuestGuide { quest, ..Default::default() };
    let Some(q) = quest else { return Ok(Some(g)) };
    // A map the trail reaches that guides nowhere is where the quest is (quest 1: the orphanage
    // room's door leads next door, whose table is empty — 「옆 방에 있는 … 만나러」). Every map that
    // guides a quest leads to the same one, so when the player's own map does not (a dungeon off
    // the trail, or the destination itself) the trail starts from the first map that does.
    let mut start = None;
    for m in std::iter::once(here).chain(0..=u8::MAX) {
        if mmi_of(jvm, state, m).await?.is_some_and(|t| mapfile::guide_tile(&t, q).is_some()) {
            start = Some(m);
            break;
        }
    }
    if let Some(start) = start {
        // Each miss loads one more map into the cache, so this ends within the maps there are.
        for _ in 0..=2 * 256 {
            match mapfile::guide_target(start, q, |m| state.cached_mmi(m).ok_or(m), |m| state.cached_file(m).ok_or(m)) {
                Ok(target) => {
                    g.target = target;
                    break;
                }
                Err(m) => {
                    mmi_of(jvm, state, m).await?;
                    mapfile_of(jvm, state, m).await?;
                }
            }
        }
    }
    let mut maps = state.guide_maps();
    if !maps.contains(&here) {
        maps.push(here);
    }
    for m in maps {
        if let Some(mmi) = mmi_of(jvm, state, m).await?
            && let Some((x, y)) = mapfile::guide_tile(&mmi, q)
        {
            g.tiles.push((m, x, y));
        }
    }
    Ok(Some(g))
}

/// `/mmi/<map>` once, then from the cache.
async fn mmi_of(jvm: &Jvm, state: &GuestState, map: u8) -> JvmResult<Option<Arc<[u8]>>> {
    if let Some(c) = state.cached_mmi(map) {
        return Ok(c);
    }
    let got: Option<Arc<[u8]>> = resource(jvm, &format!("/mmi/{map}")).await?.map(Into::into);
    state.cache_mmi(map, got.clone());
    Ok(got)
}

/// Map `map`'s file, parsed for its triggers' warps, once.
async fn mapfile_of(jvm: &Jvm, state: &GuestState, map: u8) -> JvmResult<Option<Arc<mapfile::MapFile>>> {
    if let Some(c) = state.cached_file(map) {
        return Ok(c);
    }
    let got = resource(jvm, &MapGrid::path(map)).await?.and_then(|raw| mapfile::MapFile::parse(&raw)).map(Arc::new);
    state.cache_file(map, got.clone());
    Ok(got)
}

/// The map's title as the game's minimap window letters it in its red band (`g.a(dd)` @69):
/// `g.b()[B` — the first name, then `-` and the second when there is one (「고아원-2층A」).
async fn map_title(jvm: &Jvm, map: &Box<dyn ClassInstance>, id: u8) -> JvmResult<Option<Icon>> {
    let Some(bytes) = object(jvm.invoke_virtual(map, "b", "()[B", ()).await?) else {
        return Ok(None);
    };
    let n = jvm.array_length(&bytes).await?;
    let text: Vec<i8> = jvm.load_array(&bytes, 0, n).await?;
    let text: Vec<u8> = text.into_iter().map(|b| b as u8).collect();
    let title = label(jvm, 0, &text).await?.map(|l| Icon { key: IconKey::MapTitle(id), ..l });
    tracing::info!("hz: map {id} title: {} bytes lettered {:?}", text.len(), title.as_ref().map(|t| (t.w, t.h)));
    Ok(title)
}

/// Height of a label's picture: the game's Korean glyphs are 12 px (`/ftd/kor`), with a row of
/// margin above and below.
const LABEL_H: i32 = 14;

/// How many of the host's lines (B1) one drawn frame letters: a label is an image, a draw through
/// the game's font and a `getRGB` — ~15 ms a line in the x86 probe (2026-10-26) — so a boot's table
/// is spread over the title's frames.
const TEXTS_PER_FRAME: usize = 2;

/// One line of EUC-KR text in the game's own font (Phase 7-3): `ah.a(dd,II[BI)V` — the call its
/// windows letter with — into a fresh image through `new dd(image.getGraphics())`, black on white,
/// read back with `getRGB` and turned into white ink whose alpha is the coverage. The width is the
/// game's own measure, `ah.a([B)I`.
async fn label(jvm: &Jvm, index: u8, text: &[u8]) -> JvmResult<Option<Icon>> {
    let bytes = byte_array(jvm, text).await?;
    let w: i32 = jvm.invoke_static("ah", "a", "([B)I", [JavaValue::Object(Some(bytes.clone()))]).await?;
    if w <= 0 {
        return Ok(None);
    }
    let (iw, ih) = (w + 2, LABEL_H);
    let image: Box<dyn ClassInstance> = jvm
        .invoke_static("javax/microedition/lcdui/Image", "createImage", "(II)Ljavax/microedition/lcdui/Image;", [JavaValue::Int(iw), JavaValue::Int(ih)])
        .await?;
    let g: Box<dyn ClassInstance> = jvm.invoke_virtual(&image, "getGraphics", "()Ljavax/microedition/lcdui/Graphics;", ()).await?;
    let _: () = jvm.invoke_virtual(&g, "setColor", "(I)V", [JavaValue::Int(0xFF_FFFF)]).await?;
    let _: () = jvm.invoke_virtual(&g, "fillRect", "(IIII)V", [JavaValue::Int(0), JavaValue::Int(0), JavaValue::Int(iw), JavaValue::Int(ih)]).await?;
    // The colour argument goes nowhere (`ah.a(dd,IIII[BIII)V` passes everything else on): the
    // glyphs take the Graphics' own colour, so it must be the ink's.
    let _: () = jvm.invoke_virtual(&g, "setColor", "(I)V", [JavaValue::Int(0)]).await?;
    let dd = jvm.new_class("dd", "(Ljavax/microedition/lcdui/Graphics;)V", (g,)).await?;
    let args = [JavaValue::Object(Some(dd)), JavaValue::Int(1), JavaValue::Int(1), JavaValue::Object(Some(bytes)), JavaValue::Int(0)];
    let _: () = jvm.invoke_static("ah", "a", "(Ldd;II[BI)V", args).await?;
    let Some((w, h, px)) = image_rgb(jvm, &image, None).await? else { return Ok(None) };
    // Black ink on white: the darker, the more ink.
    let px: Vec<u32> = px
        .into_iter()
        .map(|p| {
            let lum = (((p >> 16) & 0xff) * 3 + ((p >> 8) & 0xff) * 6 + (p & 0xff)) / 10;
            ((255 - lum) << 24) | 0x00ff_ffff
        })
        .collect();
    if px.iter().all(|p| p >> 24 == 0) {
        return Ok(None);
    }
    Ok(Some(Icon { key: IconKey::Label(index), w, h, px: px.into() }))
}

async fn image_rgb(jvm: &Jvm, image: &Box<dyn ClassInstance>, rect: Option<(i32, i32, i32, i32)>) -> JvmResult<Option<(u32, u32, Vec<u32>)>> {
    let image = image.clone();
    let iw: i32 = jvm.invoke_virtual(&image, "getWidth", "()I", ()).await?;
    let ih: i32 = jvm.invoke_virtual(&image, "getHeight", "()I", ()).await?;
    let (x, y, w, h) = rect.unwrap_or((0, 0, iw, ih));
    if w <= 0 || h <= 0 || x + w > iw || y + h > ih {
        return Ok(None);
    }
    let n = (w * h) as usize;
    let rgb = jvm.instantiate_array("I", n).await?;
    let args = [
        JavaValue::Object(Some(rgb.clone())),
        JavaValue::Int(0),
        JavaValue::Int(w),
        JavaValue::Int(x),
        JavaValue::Int(y),
        JavaValue::Int(w),
        JavaValue::Int(h),
    ];
    let _: () = jvm.invoke_virtual(&image, "getRGB", "([IIIIIII)V", args).await?;
    let px: Vec<i32> = jvm.load_array(&rgb, 0, n).await?;
    Ok(Some((w as u32, h as u32, px.into_iter().map(|p| p as u32).collect())))
}

/// How often the portrait is composed again in the field, in drawn frames: equipment changes in
/// the menu, so about a second after the field comes back is soon enough (hero2 polls every 5).
const PORTRAIT_EVERY: u64 = 15;
/// Bytes read from the standing action: its frame count and the first directions of frame 0 —
/// a count and 4-byte entries each (the warrior's: 7 and 6 entries).
const STAND_BYTES: usize = 512;

/// The portrait (Phase 6-6, `crate::portrait` has the doll's layout): the standing frame facing
/// the viewer, head and body parts, from the live doll `rpg/e.a:Lcn;` and its pictures. `None`
/// while there is no doll or no head in that frame. Read-only, as the game's own painter.
async fn portrait(jvm: &Jvm) -> JvmResult<Option<Icon>> {
    let Some(doll) = object(jvm.get_static_field("rpg/e", "a", "Lcn;").await?) else {
        return Ok(None);
    };
    let layers: i8 = jvm.get_field(&doll, "a", "B").await?;
    let (Some(offsets), Some(data), Some(slots)) = (
        object(jvm.get_field(&doll, "a", "[I").await?),
        object(jvm.get_field(&doll, "a", "[B").await?),
        object(jvm.get_field(&doll, "a", "[[Lcw;").await?),
    ) else {
        return Ok(None);
    };
    if layers <= 0 || jvm.array_length(&offsets).await? < portrait::STAND + 1 {
        return Ok(None);
    }
    let at: Vec<i32> = jvm.load_array(&offsets, portrait::STAND, 1).await?;
    let (at, len) = (at[0].max(0) as usize, jvm.array_length(&data).await?);
    if at >= len {
        return Ok(None);
    }
    let mut bytes = vec![0u8; (len - at).min(STAND_BYTES)];
    jvm.array_raw_buffer(&data).await?.read(at, &mut bytes)?;
    let Some(entries) = portrait::frame_entries(&bytes, layers as u8, portrait::FACING_VIEWER) else {
        return Ok(None);
    };
    let n_slots = jvm.array_length(&slots).await?;
    let mut pieces = Vec::new();
    for e in entries.iter().filter(|e| e.part <= portrait::LAST_BODY_PART && usize::from(e.slot) < n_slots) {
        // The game skips a slot or a picture that is not loaded; so does this.
        let row: Vec<Option<Box<dyn ClassInstance>>> = jvm.load_array(&slots, usize::from(e.slot), 1).await?;
        let Some(row) = row.into_iter().next().flatten() else { continue };
        if usize::from(e.picture) >= jvm.array_length(&row).await? {
            continue;
        }
        let cw: Vec<Option<Box<dyn ClassInstance>>> = jvm.load_array(&row, usize::from(e.picture), 1).await?;
        let Some(cw) = cw.into_iter().next().flatten() else { continue };
        if let Some((w, h, px)) = picture(jvm, &cw, None).await? {
            pieces.push(portrait::Piece { x: e.dx.into(), y: e.dy.into(), w, h, px, head: e.part == portrait::HEAD });
        }
    }
    Ok(portrait::compose(&pieces))
}

/// (`ed.d` if the game screen exists, a message box is up, the 「# 확인」 notice is up).
///
/// The canvas's screen `f.a:Lbj;` is the logo `ch` until the game screen `ce` replaces it, and
/// `ce`'s constructor is what first sets `ed.d`; reading `ed` before that would initialise the
/// class early, so until then there is nothing to report.
async fn scene(jvm: &Jvm) -> JvmResult<(Option<u8>, bool, bool)> {
    let Some(screen) = object(jvm.get_static_field("f", "a", "Lbj;").await?) else {
        return Ok((None, false, false));
    };
    if !jvm.is_instance(&*screen, "ce") {
        return Ok((None, false, false));
    }
    let mode: i8 = jvm.get_static_field("ed", "d", "B").await?;
    let notice: bool = jvm.get_static_field("ed", "f", "Z").await?;
    let popup = match object(jvm.get_static_field("ax", "a", "Ljava/util/Vector;").await?) {
        Some(queue) => {
            let n: i32 = jvm.invoke_virtual(&queue, "size", "()I", ()).await?;
            n > 0
        }
        None => false,
    };
    Ok((Some(mode as u8), popup, notice))
}

// ------------------------------------------------------- balance (HZ-039 ..) ----

/// HZ-039 `table(String)[B`, in place of `rpg/e.a(String)[B` where the skill tables load: the game's
/// loader, then the host's edits laid over what it returned (`tables::overlay`).
struct Table;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for Table {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let path = object(args[0].clone());
        let loaded: Option<Box<dyn ClassInstance>> = jvm.invoke_static("rpg/e", "a", "(Ljava/lang/String;)[B", [JavaValue::Object(path.clone())]).await?;
        let (Some(p), Some(bytes)) = (path, loaded) else {
            return Ok(JavaValue::Object(None));
        };
        let name = JavaLangString::to_rust_string(jvm, &p).await?;
        let n = jvm.array_length(&bytes).await?;
        let game: Vec<i8> = jvm.load_array(&bytes, 0, n).await?;
        let game: Vec<u8> = game.into_iter().map(|b| b as u8).collect();
        match tables::overlay(&name, &game) {
            Some(host) => {
                tracing::info!("hz: table {name}: {n} bytes, the host's numbers laid over");
                Ok(JavaValue::Object(Some(byte_array(jvm, &host).await?)))
            }
            None => Ok(JavaValue::Object(Some(bytes))),
        }
    }
}

/// HZ-040: the player's stat `index` of `ek.c:[I` (0 힘 · 1 민첩 · 2 건강 · 3 의지).
async fn stat(jvm: &Jvm, ek: &Box<dyn ClassInstance>, index: usize) -> JvmResult<i32> {
    let Some(c) = object(jvm.get_field_from(ek, "ek", "c", "[I").await?) else {
        return Ok(0);
    };
    Ok(jvm.load_array::<i32>(&c, index, 1).await?.pop().unwrap_or(0))
}

/// HZ-040: one of `ek`'s private helpers the formulas use — `g(B)I` a worn option's sum, `h(B)I` a slot's
/// armour, `i(B)I` an equipped passive's value at x = 1, `a(BI)I` the same at x.
async fn ek_private(jvm: &Jvm, ek: &Box<dyn ClassInstance>, name: &str, descriptor: &str, args: Vec<JavaValue>) -> JvmResult<i32> {
    jvm.invoke_special(ek, "ek", name, descriptor, args).await
}

/// HZ-040: the spirit's passive `id`'s value (`p.a(B)I`, 0 unequipped) or buff `id`'s (`p.b(B)I`).
async fn spirit_value(jvm: &Jvm, ek: &Box<dyn ClassInstance>, buff: bool, id: i8) -> JvmResult<i32> {
    let Some(p) = object(jvm.get_field_from(ek, "ek", "a", "Lp;").await?) else {
        return Ok(0);
    };
    jvm.invoke_virtual(&p, if buff { "b" } else { "a" }, "(B)I", [JavaValue::Byte(id)]).await
}

/// HZ-040 `attack(Object ek, int)V`, in place of `putfield ek.h:I` in `ek.i()V`: the attack before the
/// weapon, by the host's formula.
struct Attack;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for Attack {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(mut ek) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.attack").await);
        };
        let game = int(args[1].clone());
        let (str, dex, wil) = (stat(jvm, &ek, 0).await?, stat(jvm, &ek, 1).await?, stat(jvm, &ek, 3).await?);
        let options = ek_private(jvm, &ek, "g", "(B)I", vec![JavaValue::Byte(6)]).await?;
        let cheat: bool = jvm.get_static_field("ce", "b", "Z").await?;
        let extra = if cheat {
            9999
        } else {
            ek_private(jvm, &ek, "a", "(BI)I", vec![JavaValue::Byte(15), JavaValue::Int(dex)]).await? + spirit_value(jvm, &ek, false, 1).await?
        };
        let host = stats::PATCH.attack_base(job(jvm).await?, str, dex, wil) + options + extra;
        jvm.put_field_from(&mut ek, "ek", "h", "I", host).await?;
        tracing::info!("hz: attack {game} -> {host} (힘 {str} 민첩 {dex} 의지 {wil})");
        Ok(JavaValue::Void)
    }
}

/// HZ-040 `defence(Object ek, int)V`, in place of `putfield ek.a:I` in `ek.i()V`.
struct Defence;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for Defence {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(mut ek) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.defence").await);
        };
        let game = int(args[1].clone());
        let j = job(jvm).await?;
        let mut sum = 0;
        for slot in [1i8, 4, 6, 0] {
            sum += ek_private(jvm, &ek, "h", "(B)I", vec![JavaValue::Byte(slot)]).await?;
        }
        sum += ek_private(jvm, &ek, "g", "(B)I", vec![JavaValue::Byte(7)]).await?;
        if j == stats::KNIGHT {
            sum += ek_private(jvm, &ek, "h", "(B)I", vec![JavaValue::Byte(5)]).await?;
        }
        sum += stats::PATCH.stat_defence(j, stat(jvm, &ek, 0).await?, stat(jvm, &ek, 3).await?);
        let armour = ek_private(jvm, &ek, "a", "(BI)I", vec![JavaValue::Byte(9), JavaValue::Int(sum)]).await?;
        let host = sum + armour + spirit_value(jvm, &ek, false, 0).await?;
        jvm.put_field_from(&mut ek, "ek", "a", "I", host).await?;
        tracing::info!("hz: defence {game} -> {host}");
        Ok(JavaValue::Void)
    }
}

/// HZ-040 `hpBonus(Object p, byte)I`, in place of `p.a(B)I` (내성) where `ek.i()V` sums max HP.
struct HpBonus;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for HpBonus {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(p) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.hpBonus").await);
        };
        let id: i8 = args[1].clone().into();
        let base: i32 = jvm.invoke_virtual(&p, "a", "(B)I", [JavaValue::Byte(id)]).await?;
        let extra = match player(jvm).await? {
            Some(ek) if id == 2 => stats::PATCH.hp_extra(stat(jvm, &ek, 2).await?),
            _ => 0,
        };
        Ok(JavaValue::Int(base + extra))
    }
}

/// HZ-055 `spLevel(Object ek)I`, in place of `getfield ek.n:B` where `ek.i()V` sums max SP: the level × 3,
/// so the game's `5 × level` is 15 a level.
struct SpLevel;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for SpLevel {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(ek) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.spLevel").await);
        };
        let level: i8 = jvm.get_field_from(&ek, "ek", "n", "B").await?;
        Ok(JavaValue::Int(i32::from(level) * balance::SP_LEVEL_TIMES))
    }
}

/// HZ-059 `walk(byte arrow)V`, in place of `ed.b(B)V` where the field takes an arrow (`ce.a(II)Z`): the
/// diagonal the host's arrows make when this arrow is one of its two, else the arrow. `ed.b(B)V` uses it
/// in four states: 1 · 2 the walk's start and turn, 3 · 9 an attack's buffered turn `ek.g(B)V` (`ek.z`,
/// which the game only tests `> 0` and walks on with `ed.b(2, z)` when the attack ends — an arrow there
/// walked a diagonal held through an OK straight on, probe 2026-10-31); the others drop it.
struct Walk(Arc<GuestState>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for Walk {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let arrow: i8 = args[0].clone().into();
        let mut dir = arrow;
        if let Some((h, v)) = input::dir::split(self.0.walk_diag())
            && (arrow == h || arrow == v)
        {
            dir = input::dir::diagonal(h, v).unwrap_or(arrow);
        }
        if dir != arrow {
            tracing::debug!("hz: walk {arrow} -> {dir}");
        }
        let _: () = jvm.invoke_static("ed", "b", "(B)V", [JavaValue::Byte(dir)]).await?;
        Ok(JavaValue::Void)
    }
}

/// HZ-059 `camera(boolean, boolean)V`, in place of `ed.a(ZZ)V` in `ce.d()V`: the game's follow, which
/// reads `ed.a:[Z[ar.c]` (1..4 only) when its second argument is set. Facing a diagonal, it runs once
/// with the horizontal arrow as the facing and once with the vertical — its look-ahead moves by
/// `V` / `W` of the facing and its follow takes the facing's axis, so the two make both — and the
/// facing goes back. The field is put directly: the setter `ar.c(B)V` also writes `ar.d:B`.
struct Camera;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for Camera {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let (look, facing): (bool, bool) = (args[0].clone().into(), args[1].clone().into());
        let follow = |look: bool, facing: bool| async move {
            jvm.invoke_static::<_, ()>("ed", "a", "(ZZ)V", [JavaValue::Boolean(look), JavaValue::Boolean(facing)]).await
        };
        let player = if facing { object(jvm.get_static_field("ed", "a", "Lek;").await?) } else { None };
        let Some(mut player) = player else {
            follow(look, facing).await?;
            return Ok(JavaValue::Void);
        };
        let d: i8 = jvm.get_field_from(&player, "ar", "c", "B").await?;
        let Some((h, v)) = input::dir::split(d) else {
            follow(look, facing).await?;
            return Ok(JavaValue::Void);
        };
        for arrow in [h, v] {
            jvm.put_field_from(&mut player, "ar", "c", "B", arrow).await?;
            follow(look, facing).await?;
        }
        jvm.put_field_from(&mut player, "ar", "c", "B", d).await?;
        Ok(JavaValue::Void)
    }
}

/// HZ-059 `face(Object ar, byte dir)V`, in place of `ar.c(B)V` in the state-and-direction setter
/// `ed.b(BB)V` (the state already set): a diagonal stays only for a walk (state 2) — dozens of the
/// game's reads of the facing index tables of five — else its horizontal arrow.
///
/// It also drops a side-step's way back (`ar.e:B`): every caller is a direction the player chose (the
/// pending turn at a tile's centre `ed.d()V`, the walk's start, an attack and the walk after it), and
/// `ar.w()V` — which `ek.C()V` runs right after `ed.d()V`, at the same centre — would put the old way
/// back over it. The game's own side-step is rare (a corner), but a diagonal along a wall slides every
/// tile, so a turn there was lost and the walk went on the old way (user 2026-10-31: 「남동쪽으로
/// 이동하다가 북서쪽으로 조이스틱 옮겨도 남동쪽으로 계속」; probe: ↙ sliding ↓ along a wall, ↗ held —
/// slid on to the wall's end and stood).
struct Face;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for Face {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(mut ar) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.face").await);
        };
        let mut d: i8 = args[1].clone().into();
        let back: i8 = jvm.get_field_from(&ar, "ar", "e", "B").await?;
        if back != 0 {
            tracing::debug!("hz: face {d}: the side-step's way back {back} dropped");
            jvm.put_field_from(&mut ar, "ar", "e", "B", 0i8).await?;
        }
        if let Some((h, _)) = input::dir::split(d) {
            let state: i8 = jvm.get_field_from(&ar, "ar", "b", "B").await?;
            if state != 2 {
                tracing::debug!("hz: face {d} -> {h} (state {state})");
                d = h;
            }
        }
        let _: () = jvm.invoke_virtual(&ar, "c", "(B)V", [JavaValue::Byte(d)]).await?;
        Ok(JavaValue::Void)
    }
}

/// HZ-059 `passable(Object g, Object ar, byte dir)Z`, in place of `g.a(Lar;B)Z` in the step's check
/// `ar.e()Z`: the next tile open — for a diagonal, its two arrows' tiles too (the game's check reads the
/// target tile alone, so a diagonal slipped between two walls meeting at a corner).
struct Passable;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for Passable {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(g) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.passable").await);
        };
        let d: i8 = args[2].clone().into();
        let open = |d: i8| {
            let (g, ar) = (&g, args[1].clone());
            async move { jvm.invoke_virtual::<_, bool>(g, "a", "(Lar;B)Z", [ar, JavaValue::Byte(d)]).await }
        };
        let ok = match input::dir::split(d) {
            Some((h, v)) => open(h).await? && open(v).await? && open(d).await?,
            None => open(d).await?,
        };
        Ok(JavaValue::Boolean(ok))
    }
}

/// HZ-059 `slide(Object ar, byte dir)Z`, in place of `ar.c(B)Z` (the side-step past a corner) in
/// `ar.f()Z`, which a blocked walk tries: for a diagonal, a tile along whichever of its arrows is open
/// (horizontal first), set up as the game's side-step sets its own — `e:B` = the diagonal to come back
/// to, state 2, facing the arrow; `ar.w()V` turns it back at the tile's centre. An arrow: the game's.
struct Slide;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for Slide {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(mut ar) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.slide").await);
        };
        let d: i8 = args[1].clone().into();
        let Some((h, v)) = input::dir::split(d) else {
            let stepped: bool = jvm.invoke_virtual(&ar, "c", "(B)Z", [JavaValue::Byte(d)]).await?;
            return Ok(JavaValue::Boolean(stepped));
        };
        let Some(g) = object(jvm.get_static_field("ed", "a", "Lg;").await?) else {
            return Ok(JavaValue::Boolean(false));
        };
        for arrow in [h, v] {
            let open: bool = jvm.invoke_virtual(&g, "a", "(Lar;B)Z", [JavaValue::Object(Some(ar.clone())), JavaValue::Byte(arrow)]).await?;
            if open {
                tracing::debug!("hz: slide {d} along {arrow}");
                jvm.put_field_from(&mut ar, "ar", "e", "B", d).await?;
                let _: () = jvm.invoke_virtual(&ar, "a", "(B)V", [JavaValue::Byte(2)]).await?;
                let _: () = jvm.invoke_virtual(&ar, "c", "(B)V", [JavaValue::Byte(arrow)]).await?;
                return Ok(JavaValue::Boolean(true));
            }
        }
        Ok(JavaValue::Boolean(false))
    }
}

/// HZ-059 `drawFacing(Object ek)B`, in place of `getfield ar.c:B` in the player's picture `ek.b(III)V`: the
/// animation's direction — its sheets hold four (`t.a(BBIII)I` steps over four blocks a frame), so a
/// diagonal draws as its horizontal arrow, the one the walk stops facing.
struct DrawFacing;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for DrawFacing {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(ek) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.drawFacing").await);
        };
        let d: i8 = jvm.get_field_from(&ek, "ar", "c", "B").await?;
        Ok(JavaValue::Byte(input::dir::split(d).map_or(d, |(h, _)| h)))
    }
}

/// HZ-059, from HZ-006's field tick: a diagonal facing outside a walk — left by a way out of state 2
/// that does not pass `ed.b(BB)V` (a hit, a scene) — becomes its horizontal arrow.
async fn straighten_facing(jvm: &Jvm) -> JvmResult<()> {
    let Some(player) = object(jvm.get_static_field("ed", "a", "Lek;").await?) else {
        return Ok(());
    };
    let d: i8 = jvm.get_field_from(&player, "ar", "c", "B").await?;
    if let Some((h, _)) = input::dir::split(d) {
        let state: i8 = jvm.get_field_from(&player, "ar", "b", "B").await?;
        if state != 2 {
            tracing::debug!("hz: facing {d} -> {h} (state {state}, field tick)");
            let _: () = jvm.invoke_virtual(&player, "c", "(B)V", [JavaValue::Byte(h)]).await?;
        }
    }
    Ok(())
}

/// HZ-056 `spiritCastEnd(Object ek)V`, in place of `ek.D()V` where the spirit cast (state 10) ends in `ek.C()V`:
/// the game's `D()`, then `B()` (the action queue emptied) when that left the player idle (`ar.b` 1, `l` −1)
/// with a step still in `ac[0]` — an OK pressed during the cast in 동화 leaves a transformed step (6) there,
/// and `ek.d(B)Z` refuses every key while `ac[l + 1] >= 4`.
struct SpiritCastEnd;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for SpiritCastEnd {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(ek) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.spiritCastEnd").await);
        };
        let _: () = jvm.invoke_special(&ek, "ek", "D", "()V", ()).await?;
        let state: i8 = jvm.get_field_from(&ek, "ar", "b", "B").await?;
        let at: i8 = jvm.get_field_from(&ek, "ek", "l", "B").await?;
        if state == 1 && at < 0
            && let Some(queue) = object(jvm.get_field_from(&ek, "ek", "ac", "[B").await?)
        {
            let first: i8 = jvm.load_array::<i8>(&queue, 0, 1).await?.pop().unwrap_or(-1);
            if first >= 0 {
                tracing::info!("hz: spirit cast end: stale step {first} cleared");
                let _: () = jvm.invoke_special(&ek, "ek", "B", "()V", ()).await?;
            }
        }
        Ok(JavaValue::Void)
    }
}

/// HZ-040 `crit(Object ek, byte, int, boolean)V`, in place of `ek.a(BIZ)V` at the end of `G()`: the
/// store, with the host's crit for the hand.
struct Crit;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for Crit {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(ek) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.crit").await);
        };
        let game = int(args[2].clone());
        let off: bool = args[3].clone().into();
        let dex = stat(jvm, &ek, 1).await?;
        let options = ek_private(jvm, &ek, "g", "(B)I", vec![JavaValue::Byte(5)]).await?;
        let trick = ek_private(jvm, &ek, "i", "(B)I", vec![JavaValue::Byte(5)]).await?;
        let weapon = match object(jvm.invoke_virtual(&ek, "a", "(B)Lbx;", [JavaValue::Byte(if off { 5 } else { 3 })]).await?) {
            Some(item) => i32::from(jvm.get_field_from::<i8>(&item, "bx", "h", "B").await?),
            None => 0,
        };
        let storm = if jvm.get_field_from::<bool>(&ek, "ek", "h", "Z").await? { spirit_value(jvm, &ek, true, 1).await? } else { 0 };
        let host = stats::PATCH.crit(job(jvm).await?, dex, options, trick, weapon, storm);
        let _: () = jvm.invoke_special(&ek, "ek", "a", "(BIZ)V", vec![args[1].clone(), JavaValue::Int(host), JavaValue::Boolean(off)]).await?;
        tracing::info!("hz: crit {game} -> {host} (민첩 {dex}, off {off})");
        Ok(JavaValue::Void)
    }
}

/// HZ-040 `evasion(Object ek, byte, int, boolean)V`, in place of `ek.a(BIZ)V` at the end of `H()`.
struct Evasion;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for Evasion {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(ek) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.evasion").await);
        };
        let game = int(args[2].clone());
        let off: bool = args[3].clone().into();
        let dex = stat(jvm, &ek, 1).await?;
        let options = ek_private(jvm, &ek, "g", "(B)I", vec![JavaValue::Byte(8)]).await?;
        let dodge = ek_private(jvm, &ek, "i", "(B)I", vec![JavaValue::Byte(4)]).await?;
        let ghost = if jvm.get_field_from::<bool>(&ek, "ek", "i", "Z").await? { spirit_value(jvm, &ek, true, 2).await? } else { 0 };
        let host = stats::PATCH.evasion(job(jvm).await?, dex, stat(jvm, &ek, 3).await?, options, dodge, ghost);
        let _: () = jvm.invoke_special(&ek, "ek", "a", "(BIZ)V", vec![args[1].clone(), JavaValue::Int(host), JavaValue::Boolean(off)]).await?;
        tracing::info!("hz: evasion {game} -> {host} (민첩 {dex})");
        Ok(JavaValue::Void)
    }
}

/// HZ-042 `levelCap(Object cb)I`, in place of `getfield cb.j:B` in the skill window: 5 on the job
/// window's active tab, the field elsewhere.
struct LevelCap;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for LevelCap {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(window) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.levelCap").await);
        };
        let cap: i8 = jvm.get_field_from(&window, "cb", "j", "B").await?;
        let tab: i32 = jvm.get_field_from(&window, "cb", "e", "I").await?;
        let actives = tab == 0 && jvm.is_instance(&*window, "ad");
        Ok(JavaValue::Int(if actives { balance::ACTIVE_CAP } else { cap.into() }))
    }
}

/// HZ-044 `skillPoints(Object ek)V`, in place of the level-up's `if (n % 2 == 0) b++`.
struct SkillPoints;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for SkillPoints {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(mut ek) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.skillPoints").await);
        };
        let level: i8 = jvm.get_field_from(&ek, "ek", "n", "B").await?;
        let add = balance::skill_points_at(level.into());
        if add > 0 {
            let points: i16 = jvm.get_field_from(&ek, "ek", "b", "S").await?;
            jvm.put_field_from(&mut ek, "ek", "b", "S", points.saturating_add(add as i16)).await?;
            tracing::info!("hz: LV {level}: skill points {points} + {add}");
        }
        Ok(JavaValue::Void)
    }
}

/// HZ-045 `bossReflect(Object attacker, Object part, int fifth, byte, byte)V`, in place of
/// `ar.a(Lar;IBB)V` in a boss part's damage taker: a tenth, mitigated and capped for the player —
/// nothing while HZ-057's stance does the reflecting.
struct BossReflect(Arc<GuestState>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for BossReflect {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        if self.0.patch_on("HZ-057") {
            return Ok(JavaValue::Void);
        }
        let args = args.into_vec();
        let Some(attacker) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.bossReflect").await);
        };
        let fifth = int(args[2].clone());
        let reflected = balance::reflect(fifth);
        let dealt = if jvm.is_instance(&*attacker, "ek") {
            let defence: i32 = jvm.invoke_virtual(&attacker, "b", "()I", ()).await?;
            let max_hp = match object(jvm.get_field_from(&attacker, "ek", "b", "[I").await?) {
                Some(top) => jvm.load_array::<i32>(&top, 0, 1).await?.pop().unwrap_or(0),
                None => 0,
            };
            balance::reflected_hit(reflected, defence, max_hp)
        } else {
            reflected.max(1)
        };
        tracing::info!("hz: boss reflection {} -> {dealt}", fifth * 5);
        let _: () = jvm.invoke_virtual(&attacker, "a", "(Lar;IBB)V", vec![args[1].clone(), JavaValue::Int(dealt), args[3].clone(), args[4].clone()]).await?;
        Ok(JavaValue::Void)
    }
}

// --------------------------------------------------------------- HZ-057 stance ----

/// The hooks class's static that keeps [`Boss`]'s pixel buffer.
const BOSS_PIXELS: &str = "bossPixels";

/// How far past last frame's silhouette a boss part is read (they sway and lunge a little; one that
/// leaves the region is read whole the next frame).
const BOSS_MARGIN: i32 = 24;

/// What the reflect stance's hooks share.
struct Boss {
    state: Arc<GuestState>,
    stance: std::sync::Mutex<boss::Stance>,
    /// Each part (by kind, `cq.c:S`): where its silhouette was last drawn, on the screen.
    parts: std::sync::Mutex<std::collections::HashMap<i16, Rect>>,
    /// The parts' box this frame, for the badge.
    frame_box: std::sync::Mutex<Option<Rect>>,
    /// 「반사」 in the game's font, lettered the first time (`Some(None)`: it could not be).
    label: std::sync::Mutex<Option<Option<Icon>>>,
}

/// x, y, w, h on the screen.
type Rect = (i32, i32, i32, i32);

impl Boss {
    fn new(state: Arc<GuestState>) -> Boss {
        Boss { state, stance: Default::default(), parts: Default::default(), frame_box: Default::default(), label: Default::default() }
    }

    fn on(&self) -> bool {
        self.state.patch_on("HZ-057")
    }

    fn stance(&self) -> std::sync::MutexGuard<'_, boss::Stance> {
        self.stance.lock().unwrap()
    }

    /// A field tick (HZ-006's hook): the stance's clock — and HZ-058's plates' cooldowns'.
    fn tick(&self) {
        self.state.count_field_tick();
        if !self.on() {
            return;
        }
        let moved = self.stance().tick();
        if let Some(phase) = moved {
            tracing::info!("hz: boss stance -> {phase:?}");
        }
    }

    /// The buffer the stance reads and draws through, at least `n` long.
    async fn pixels(jvm: &Jvm, n: usize) -> JvmResult<Box<dyn ClassInstance>> {
        if let Some(a) = object(jvm.get_static_field(hooks::CLASS, BOSS_PIXELS, "[I").await?)
            && jvm.array_length(&a).await? >= n
        {
            return Ok(a);
        }
        let a = jvm.instantiate_array("I", n).await?;
        jvm.put_static_field(hooks::CLASS, BOSS_PIXELS, "[I", JavaValue::Object(Some(a.clone()))).await?;
        Ok(a)
    }

    /// One boss part's picture, in the stance: the back buffer around it before and after its draw,
    /// and the coat over what changed. The buffer is `createImage(II)`'s, four bytes a pixel (the
    /// vendor's `0xAARRGGBB`, native order) in `imgData`, read and written a row at a time — one
    /// `getRGB` and `drawRGB` a part would copy every pixel through a `JavaValue` each way.
    #[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
    async fn draw(&self, jvm: &Jvm, part: &Box<dyn ClassInstance>, x: i32, y: i32, look: boss::Look) -> JvmResult<()> {
        let draw = || async { jvm.invoke_virtual::<_, ()>(part, "a", "(II)V", [JavaValue::Int(x), JavaValue::Int(y)]).await };
        let image = object(jvm.get_static_field("dd", "a", "Ljavax/microedition/lcdui/Image;").await?);
        let g = object(jvm.get_static_field("dd", "a", "Ljavax/microedition/lcdui/Graphics;").await?);
        let (Some(image), Some(g)) = (image, g) else {
            return draw().await;
        };
        let iw: i32 = jvm.get_field(&image, "w", "I").await?;
        let ih: i32 = jvm.get_field(&image, "h", "I").await?;
        let bpl: i32 = jvm.get_field(&image, "bpl", "I").await?;
        let data = object(jvm.get_field(&image, "imgData", "[B").await?);
        let Some(mut data) = data.filter(|_| bpl == iw * 4) else {
            return draw().await;
        };
        let kind: i16 = jvm.get_field_from(part, "cq", "c", "S").await?;
        let last = self.parts.lock().unwrap().get(&kind).copied();
        let (rx, ry, rw, rh) = match last {
            Some((bx, by, bw, bh)) => {
                let (x0, y0) = ((bx - BOSS_MARGIN).max(0), (by - BOSS_MARGIN).max(0));
                let (x1, y1) = ((bx + bw + BOSS_MARGIN).min(iw), (by + bh + BOSS_MARGIN).min(ih));
                (x0, y0, x1 - x0, y1 - y0)
            }
            None => (0, 0, iw, ih),
        };
        if rw <= 0 || rh <= 0 {
            self.parts.lock().unwrap().remove(&kind);
            return draw().await;
        }
        let (row_bytes, stride) = (rw as usize * 4, bpl as usize);
        let at = |row: i32| (ry + row) as usize * stride + rx as usize * 4;
        let read = |data: Box<dyn ClassInstance>| async move {
            let raw = jvm.array_raw_buffer(&data).await?;
            let mut bytes = vec![0u8; row_bytes * rh as usize];
            for (row, chunk) in bytes.chunks_exact_mut(row_bytes).enumerate() {
                raw.read(at(row as i32), chunk)?;
            }
            JvmResult::Ok(bytes.chunks_exact(4).map(|b| i32::from_ne_bytes([b[0], b[1], b[2], b[3]])).collect::<Vec<_>>())
        };
        let before = read(data.clone()).await?;
        draw().await?;
        let after = read(data.clone()).await?;
        let Some(c) = boss::coat(&before, &after, rw as usize, rh as usize, rx, ry, look) else {
            self.parts.lock().unwrap().remove(&kind);
            return Ok(());
        };
        let (sx, sy, sw, sh) = (c.silhouette.0 as i32, c.silhouette.1 as i32, c.silhouette.2 as i32, c.silhouette.3 as i32);
        let seen = (rx + sx, ry + sy, sw, sh);
        // Up against a side of the region that is not the screen's: it may go on past it.
        let cut = last.is_some() && ((sx == 0 && rx > 0) || (sy == 0 && ry > 0) || (sx + sw == rw && rx + rw < iw) || (sy + sh == rh && ry + rh < ih));
        if cut {
            self.parts.lock().unwrap().remove(&kind);
        } else {
            self.parts.lock().unwrap().insert(kind, seen);
        }
        {
            let mut frame = self.frame_box.lock().unwrap();
            *frame = Some(match *frame {
                Some((fx, fy, fw, fh)) => {
                    let (x0, y0) = (fx.min(seen.0), fy.min(seen.1));
                    (x0, y0, (fx + fw).max(seen.0 + seen.2) - x0, (fy + fh).max(seen.1 + seen.3) - y0)
                }
                None => seen,
            });
        }
        // Written where the game may draw: the clip ce.e(dd) set on the buffer's Graphics.
        let clip_x: i32 = jvm.get_field(&g, "clipX", "I").await?;
        let clip_y: i32 = jvm.get_field(&g, "clipY", "I").await?;
        let clip_w: i32 = jvm.get_field(&g, "clipWidth", "I").await?;
        let clip_h: i32 = jvm.get_field(&g, "clipHeight", "I").await?;
        let (dx, dy, dw, dh) = (c.drawn.0 as i32, c.drawn.1 as i32, c.drawn.2 as i32, c.drawn.3 as i32);
        let (x0, x1) = ((rx + dx).max(clip_x), (rx + dx + dw).min(clip_x + clip_w));
        let (y0, y1) = ((ry + dy).max(clip_y), (ry + dy + dh).min(clip_y + clip_h));
        if x0 >= x1 || y0 >= y1 {
            return Ok(());
        }
        let mut raw = jvm.array_raw_buffer_mut(&mut data).await?;
        for sy in y0..y1 {
            let from = ((sy - ry) * rw + x0 - rx) as usize;
            let row: Vec<u8> = (from..from + (x1 - x0) as usize).flat_map(|i| (if c.px[i] != 0 { c.px[i] } else { after[i] }).to_ne_bytes()).collect();
            raw.write(sy as usize * stride + x0 as usize * 4, &row)?;
        }
        Ok(())
    }

    /// The badge over a drawn frame (`g` the screen's, the frame at `ox`, `oy`): 「반사」 and its bar by
    /// the parts drawn this frame, on the bare field only.
    #[allow(clippy::borrowed_box)]
    async fn badge(&self, jvm: &Jvm, g: &Box<dyn ClassInstance>, ox: i32, oy: i32) -> JvmResult<()> {
        let frame = self.frame_box.lock().unwrap().take();
        let (Some(parts), Some(gauge)) = (frame, if self.on() { self.stance().gauge() } else { None }) else {
            return Ok(());
        };
        let v = self.state.view();
        if v.scene != Scene::Field || v.popup || v.notice || v.window {
            return Ok(());
        }
        let label = self.label(jvm).await?;
        let Some(image) = object(jvm.get_static_field("dd", "a", "Ljavax/microedition/lcdui/Image;").await?) else {
            return Ok(());
        };
        let iw: i32 = jvm.invoke_virtual(&image, "getWidth", "()I", ()).await?;
        let ih: i32 = jvm.invoke_virtual(&image, "getHeight", "()I", ()).await?;
        let (warn, clock) = {
            let s = self.stance();
            (s.phase() == boss::Phase::Warn, s.clock())
        };
        let ink = label.as_ref().map(|l| (&l.px[..], l.w as usize, l.h as usize));
        let (bw, bh, px) = boss::badge(ink, gauge, warn, clock);
        let (bx, by) = boss::badge_at(parts, (bw as i32, bh as i32), (iw, ih));
        let mut buf = Self::pixels(jvm, (iw * ih) as usize).await?;
        jvm.store_array(&mut buf, 0, px).await?;
        jvm.invoke_virtual(g, "drawRGB", "([IIIIIIIZ)V", rgb_args(buf, (ox + bx, oy + by, bw as i32, bh as i32))).await
    }

    /// 「반사」, lettered the first time it is wanted.
    async fn label(&self, jvm: &Jvm) -> JvmResult<Option<Icon>> {
        if let Some(l) = self.label.lock().unwrap().clone() {
            return Ok(l);
        }
        let l = match crate::euc_kr(boss::BADGE_TEXT) {
            Some(text) => label(jvm, 0, &text).await?,
            None => None,
        };
        *self.label.lock().unwrap() = Some(l.clone());
        Ok(l)
    }
}

/// `drawRGB(buf, 0, w, x, y, w, h, true)`: `buf`'s first `w` × `h` over the rectangle, alpha kept.
fn rgb_args(buf: Box<dyn ClassInstance>, (x, y, w, h): Rect) -> [JavaValue; 8] {
    [JavaValue::Object(Some(buf)), JavaValue::Int(0), JavaValue::Int(w), JavaValue::Int(x), JavaValue::Int(y), JavaValue::Int(w), JavaValue::Int(h), JavaValue::Boolean(true)]
}

/// HZ-057 `monsterDraw(Object monster, int x, int y)V`, in place of `cq.a(II)V` in `cq.b(II)V`: the
/// monster's picture — a boss's, in the stance, with the mirror coat over it.
struct MonsterDraw(Arc<Boss>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for MonsterDraw {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let mut args = args.into_vec().into_iter();
        let (this, x, y) = (object(args.next().unwrap()), int(args.next().unwrap()), int(args.next().unwrap()));
        let Some(this) = this else {
            return Err(null_pointer(jvm, "Hooks.monsterDraw").await);
        };
        let look = if jvm.is_instance(&*this, "ak") && self.0.on() { self.0.stance().look() } else { None };
        match look {
            Some(look) => self.0.draw(jvm, &this, x, y, look).await?,
            None => jvm.invoke_virtual::<_, ()>(&this, "a", "(II)V", [JavaValue::Int(x), JavaValue::Int(y)]).await?,
        }
        Ok(JavaValue::Void)
    }
}

/// HZ-057 `monsterThink(Object monster)V`, in place of `cq.k()V` in `cq.l()V`: the monster's next
/// move — and a boss whose cooldown `l.a:S` it reloaded (an attack picked) may take the stance.
struct MonsterThink(Arc<Boss>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for MonsterThink {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(this) = object(args.into_vec().remove(0)) else {
            return Err(null_pointer(jvm, "Hooks.monsterThink").await);
        };
        if !jvm.is_instance(&*this, "ak") || !self.0.on() {
            let _: () = jvm.invoke_virtual(&this, "k", "()V", ()).await?;
            return Ok(JavaValue::Void);
        }
        let before: i16 = jvm.get_field_from(&this, "l", "a", "S").await?;
        let _: () = jvm.invoke_virtual(&this, "k", "()V", ()).await?;
        let after: i16 = jvm.get_field_from(&this, "l", "a", "S").await?;
        let started = {
            let mut s = self.0.stance();
            s.thought();
            after > before && s.picked()
        };
        if started {
            let kind: i16 = jvm.get_field_from(&this, "cq", "c", "S").await?;
            tracing::info!("hz: boss stance -> Warn (kind {kind} picked its next attack, cooldown {after})");
        }
        Ok(JavaValue::Void)
    }
}

/// HZ-057 `bossHit(Object boss, Object player, byte, int, boolean, byte, byte, boolean, boolean,
/// boolean)Z`, in place of `invokespecial cq.a(Lek;BIZBBZZZ)Z` in `ak`'s: the blow — and in the
/// stance a share of the HP it took back at the player ([`boss::reflected_hit`]).
struct BossHit(Arc<Boss>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for BossHit {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let args = args.into_vec();
        let Some(this) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.bossHit").await);
        };
        let hp: i32 = jvm.get_field_from(&this, "cq", "i", "I").await?;
        let landed: bool = jvm.invoke_special(&this, "cq", "a", "(Lek;BIZBBZZZ)Z", args[1..].to_vec()).await?;
        if !self.0.on() || !self.0.stance().reflecting() {
            return Ok(JavaValue::Boolean(landed));
        }
        let Some(player) = object(args[1].clone()) else {
            return Ok(JavaValue::Boolean(landed));
        };
        let took = hp - jvm.get_field_from::<i32>(&this, "cq", "i", "I").await?;
        let defence: i32 = jvm.invoke_virtual(&player, "b", "()I", ()).await?;
        let max_hp = match object(jvm.get_field_from(&player, "ek", "b", "[I").await?) {
            Some(top) => jvm.load_array::<i32>(&top, 0, 1).await?.pop().unwrap_or(0),
            None => 0,
        };
        let dealt = boss::reflected_hit(took, defence, max_hp);
        if dealt > 0 {
            tracing::info!("hz: boss reflection (stance) took {took} -> {dealt}");
            let _: () = jvm
                .invoke_virtual(&player, "a", "(Lar;IBB)V", vec![JavaValue::Object(Some(this)), JavaValue::Int(dealt), JavaValue::Byte(0), JavaValue::Byte(2)])
                .await?;
            self.0.stance().hit();
        }
        Ok(JavaValue::Boolean(landed))
    }
}

/// HZ-047 `guard(int damage, int y)I`, in place of 전력방어's `damage × (y / 10)`: the damage cut by the
/// level's percent — the level read back from `y` as HZ-039's table (or the game's) wrote it.
struct Guard(Arc<GuestState>);

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for Guard {
    async fn call(&self, _: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let (damage, y) = (int(args[0].clone()), int(args[1].clone()));
        let level = balance::guard_level(y, self.0.patch_on("HZ-039"));
        let out = balance::guard(damage, level);
        tracing::info!("hz: 전력방어 L{level}: {damage} -> {out}");
        Ok(JavaValue::Int(out))
    }
}

/// HZ-047 `guardStatus(Object ek, byte status, int frames)V`, in place of `ar.b(BI)V` where 전력방어
/// starts: `4 + L` seconds instead of the game's 80 frames.
struct GuardStatus;

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for GuardStatus {
    async fn call(&self, jvm: &Jvm, _: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let Some(ek) = object(args[0].clone()) else {
            return Err(null_pointer(jvm, "Hooks.guardStatus").await);
        };
        let level: i8 = jvm.invoke_virtual(&ek, "a", "(B)B", [JavaValue::Byte(9)]).await?;
        let frames = balance::guard_frames(level.into());
        let _: () = jvm.invoke_virtual(&ek, "b", "(BI)V", vec![args[1].clone(), JavaValue::Int(frames)]).await?;
        tracing::info!("hz: 전력방어 L{level} for {frames} frames");
        Ok(JavaValue::Void)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// HZ-058 (review 2026-10-30): another character's — or the same slot reloaded from the title —
    /// skills of the same numbers do not inherit the last one's cooldowns or seats.
    #[test]
    fn quick_cooldowns_do_not_carry_to_another_character() {
        use quickpage::{Entry, GAME_SLOTS, Owner};
        let (warrior, rogue) = (Owner { slot: 1, class: 0 }, Owner { slot: 2, class: 1 });
        let mut q = QuickInner::default();
        assert!(q.own(warrior));
        let mut game = [-1i8; GAME_SLOTS];
        game[0] = 5;
        q.cooldowns.observe(&game, &[300, 0, 0, 0], &[400, 0, 0, 0], 10);
        q.last_used[0] = 99;
        q.page = 1;
        assert_eq!(q.cooldowns.of(Entry::Skill(5)), (300, 400));
        assert!(!q.own(warrior), "the same character keeps them");
        assert_eq!(q.cooldowns.of(Entry::Skill(5)), (300, 400));
        assert!(q.own(rogue));
        assert_eq!(q.cooldowns.of(Entry::Skill(5)), (0, 0), "the rogue's skill 5 is ready");
        assert_eq!((q.last_used, q.page), ([0; GAME_SLOTS], 0));
        q.cooldowns.observe(&game, &[200, 0, 0, 0], &[400, 0, 0, 0], 20);
        q.leave();
        assert!(q.own(rogue), "back from the title: a fresh start, even for the same owner");
        assert_eq!(q.cooldowns.of(Entry::Skill(5)), (0, 0));
    }

    /// HZ-025: a notice is reported on its first drawn frame and never again, whatever its count
    /// started at (the game's 15, the guardian's 45) — and one that takes over a slot is new.
    #[test]
    fn a_pickup_notice_is_reported_once() {
        let first = hooks::PICKUP_FIRST_COUNT;
        let mut last = None;
        let mut reported = 0;
        for count in (1..=first).rev().chain((1..=44).rev()).chain([first, first - 1, 7, 6]) {
            if pickup_is_new(last, count) {
                reported += 1;
            }
            last = Some(count);
        }
        assert_eq!(reported, 4, "the game's, the guardian's, a new one at 14 after 44's 1, and one taking over at 7");
        assert!(!pickup_is_new(Some(5), 4) && pickup_is_new(Some(5), 5), "one lower = the same notice; a count that did not go down = a new one");
        let state = GuestState::new();
        state.report_pickup(120, 16, vec![0; 120 * 16]);
        state.report_pickup(120, 16, vec![0; 120 * 16]);
        let got = state.take_pickups();
        assert_eq!(got.iter().map(|i| i.key).collect::<Vec<_>>(), vec![IconKey::Pickup(0), IconKey::Pickup(1)]);
        assert!(state.take_pickups().is_empty());
    }

    /// HZ-028: the translucent picture's mask, as `dd.a(Lcw;III)V`'s loop applies it (`dd.b(I)V`'s mask
    /// = `alpha << 24 | 0xFFFFFF`): the colour kept, the alpha byte ANDed — a bitwise AND, not a
    /// product — and a fully transparent pixel left as it is, whatever its colour bits.
    #[test]
    fn the_alpha_mask_is_the_games_loop() {
        let mask = |alpha: u32| ((alpha << 24) | 0xFF_FFFF) as i32;
        assert_eq!(mask_alpha(0xFF12_3456u32 as i32, mask(0x80)), 0x8012_3456u32 as i32, "opaque takes the alpha");
        assert_eq!(mask_alpha(0xC012_3456u32 as i32, mask(0x80)), 0x8012_3456u32 as i32, "0xC0 & 0x80");
        assert_eq!(mask_alpha(0x4012_3456, mask(0x80)), 0x0012_3456, "0x40 & 0x80 = 0: the game's AND makes it clear");
        assert_eq!(mask_alpha(0x00AB_CDEF, mask(0x80)), 0x00AB_CDEF, "a fully transparent pixel stays");
        assert_eq!(mask_alpha(0x0100_0000, mask(0)), 0, "any alpha against 0");
        assert_eq!(mask_alpha(-1, mask(0xFF)), -1, "an opaque mask changes nothing");
    }

    #[test]
    fn menu_rows_shift_up_with_the_storage_before_the_system() {
        // HZ-012: ten rows.
        assert_eq!(menu_label_index(0, 10), Some(1), "캐릭터");
        assert_eq!(menu_label_index(7, 10), Some(8), "미니맵");
        assert_eq!(menu_label_index(8, 10), None, "계정창고");
        assert_eq!(menu_label_index(9, 10), Some(9), "시스템");
        assert_eq!(menu_label_index(57, 10), Some(57), "other tables' indices untouched");
        assert_eq!(menu_label_index(71 + 1, 10), Some(72), "de's rows (base 71)");
        // HZ-026: nine rows, 미니맵 gone.
        assert_eq!((0..9).map(|r| menu_label_index(r, 9)).collect::<Vec<_>>(), [1, 2, 3, 4, 5, 6, 7].map(Some).into_iter().chain([None, Some(9)]).collect::<Vec<_>>());
        assert_eq!(menu_label_index(49, 9), Some(49), "ec's rows (base 49)");
    }

    #[test]
    fn the_target_panel_moves_only_by_the_shift_and_stays_on_the_canvas() {
        assert_eq!(target_x(520, 0), 440, "no shift = the game's width - 80");
        assert_eq!(target_x(240, 0), 160, "the original 240 canvas");
        assert_eq!(target_x(520, 62), 378);
        assert_eq!(target_x(520, 9999), 0, "never off the left edge");
        assert_eq!(target_x(520, -5), 440, "a negative shift is none");
        let st = GuestState::new();
        st.set_target_shift(-3);
        assert_eq!(st.target_shift(), 0);
        st.set_target_shift(40);
        assert_eq!(target_x(520, st.target_shift()), 400);
    }

    #[test]
    fn the_target_panel_stays_inside_a_narrow_map() {
        // Map 135 (20 tiles = 320 px) centred on 520: the picture is 100..420.
        assert_eq!(target_x_on_map(520, 0, 100, 420), 340, "at the picture's right edge");
        assert_eq!(target_x_on_map(520, 69, 100, 420), 340, "the minimap's shift is already covered");
        assert_eq!(target_x_on_map(520, 200, 100, 420), 240, "a larger shift still moves it");
        assert_eq!(target_x_on_map(520, 9999, 100, 420), 100, "never left of the picture");
        // A map as wide as the canvas or wider (scrolled: d <= 0, the span reaches the edge).
        assert_eq!(target_x_on_map(520, 0, -60, 520), 440, "= HZ-014 alone");
        assert_eq!(target_x_on_map(520, 69, 0, 520), 371);
        // The original 240 canvas clips to the whole canvas: the orphanage room (160 px) keeps it.
        assert_eq!(target_x_on_map(240, 0, 40, 200), 160, "the game's own width - 80");
        // The host's copy of the span (`g.c()`): map 135 and a 30-tile ground centred, wider maps whole.
        assert_eq!(map_span(520, FIGHT_MAP_MIN_W), (100, 420));
        assert_eq!(map_span(520, 30 * MAP_TILE), (20, 500));
        assert_eq!(map_span(520, 40 * MAP_TILE), (0, 520));
        assert_eq!(map_span(320, FIGHT_MAP_MIN_W), (0, 320), "as wide as the canvas");
        assert_eq!(map_span(427, FIGHT_MAP_MIN_W), (53, 373), "an odd margin rounds left, as `>> 1` does");
    }

    #[test]
    fn proto_provides_every_hook_the_patches_call() {
        let p = proto(Arc::new(GameOptions::new()), Arc::new(GuestState::new()));
        assert_eq!(p.name, hooks::CLASS);
        for h in hooks::ALL {
            let m = p
                .methods
                .iter()
                .find(|m| m.name == h.name)
                .unwrap_or_else(|| panic!("{} missing", h.name));
            assert_eq!(m.descriptor, h.descriptor, "{}", h.name);
            assert!(
                m.access_flags.contains(MethodAccessFlags::STATIC),
                "{} must be static: the patches use invokestatic",
                h.name
            );
        }
        assert_eq!(p.methods.len(), hooks::ALL.len());
    }

    /// The one page (2026-10-29): every row the two pages had but *키 단축키, each once; more than
    /// either window shows, so it scrolls — and the scroll follows the cursor both ways, never past
    /// the end.
    #[test]
    fn the_one_page_has_every_row_and_scrolls_to_the_cursor() {
        let game: Vec<u8> = OPT_ROWS.iter().filter_map(|r| if let OptRow::Game(g) = r { Some(*g) } else { None }).collect();
        assert_eq!(game, [0, 1, 2, 3, 4, 6], "the game's rows less row 5, *키 단축키");
        let host: Vec<Row> = OPT_ROWS.iter().filter_map(|r| if let OptRow::Host(h) = r { Some(*h) } else { None }).collect();
        assert_eq!(host, Row::ALL);
        for visible in [visible_rows(true), visible_rows(false)] {
            assert!(visible < OPT_ROWS.len(), "it scrolls");
            let (mut cursor, mut scroll) = (0usize, 0usize);
            for _ in 0..OPT_ROWS.len() * 2 {
                cursor = (cursor + 1) % OPT_ROWS.len();
                scroll = scroll_to(cursor, scroll, visible);
                assert!((scroll..scroll + visible).contains(&cursor) && scroll + visible <= OPT_ROWS.len(), "{cursor} {scroll}");
            }
            assert_eq!(scroll_to(OPT_ROWS.len() - 1, 0, visible), OPT_ROWS.len() - visible, "the last row at the bottom");
            assert_eq!(scroll_to(3, 2, visible), 2, "a row on screen moves nothing");
            assert_eq!(scroll_to(0, 4, visible), 0);
        }
        assert!(visible_rows(true) < hooks::OPTIONS_SLOTS as usize, "a slot under the rows for 「다음 실행부터 적용」");
        assert!(visible_rows(false) < hooks::OPTIONS_SLOTS as usize);
    }

    #[test]
    fn rate_one_is_the_original() {
        for a in [0, 1, 100, 3_400, 65_535, -7] {
            assert_eq!(exp(a, 1), a.abs(), "ek.j adds |amt|");
        }
        assert_eq!(exp(i32::MIN, 1), i32::MAX, "saturates instead of wrapping");
        assert_eq!(exp(1_000_000_000, 5), i32::MAX);
        for t in [0, 4_833, 9_666, 14_500, 65_535] {
            assert_eq!(drop_threshold(t, 1), t);
        }
        assert_eq!(drop_threshold(14_500, 2), 29_000);
        assert_eq!(drop_threshold(30_000, 3), 65_535, "R is below 65535, so this is 'always'");
        assert_eq!(drop_threshold(0, 3), 0, "an empty slot stays empty");
    }

    #[test]
    fn monster_scaling_saturates_and_respawn_keeps_never() {
        assert_eq!((scale_hp(1_234, 100), scale_attack(321, 100)), (1_234, 321), "100 % is the game");
        assert_eq!((scale_hp(1_234, 150), scale_attack(321, 150)), (1_851, 481));
        assert_eq!(scale_hp(i32::MAX, 200), i32::MAX);
        assert_eq!(scale_attack(30_000, 200), i16::MAX);
        assert_eq!(scale_hp(1, 150), 1, "a monster keeps at least 1 HP");
        assert_eq!(respawn_delay(600, 1), 600);
        assert_eq!(respawn_delay(600, 4), 150);
        assert_eq!(respawn_delay(5, 6), 1, "a monster that comes back still does");
        assert_eq!(respawn_delay(0, 6), 0, "one that never comes back never does");
        assert_eq!(respawn_delay(-1, 3), -1);
        assert_eq!(notice_bottom(240, 240), 240 - hooks::NOTICE_LIFT, "the bar gone: over the EXP bar");
        assert_eq!(notice_bottom(207, 240), 207, "the game's bar there: as the game draws it");
    }

    #[test]
    fn keys_match_the_games_predicates() {
        // o.h / o.i / o.f / o.g / o.c, javap 2026-10-22.
        assert_eq!(classify(2, 0), Key::Left);
        assert_eq!(classify(0, 52), Key::Left, "'4'");
        assert_eq!(classify(5, 0), Key::Right);
        assert_eq!(classify(0, 54), Key::Right, "'6'");
        assert_eq!(classify(8, 0), Key::Fire);
        assert_eq!(classify(0, 53), Key::Fire, "'5'");
        assert_eq!(classify(-7, -7), Key::Rsk);
        assert_eq!(classify(-6, -6), Key::Other, "LSK does nothing on an options row");
        assert_eq!(classify(1, 0), Key::Move);
        assert_eq!(classify(6, 0), Key::Move);
        assert_eq!(classify(0, 50), Key::Move, "'2'");
        assert_eq!(classify(0, 56), Key::Move, "'8'");
    }
}
