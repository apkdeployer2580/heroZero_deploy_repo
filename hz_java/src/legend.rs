//! HZ-031 85제 「전설」 장비 — the end-game tier of 가디언 모드 (user 2026-10-28: *"추가 아이템은 모두 85제
//! 아이템으로 만들꺼야. hero2처럼 신규아이템을 개성있는 아이템들로 구성할꺼야. 스킬을 바꾸기도 할꺼고"*;
//! the 36 pieces and their effects are HEROZERO_NEW_ITEM_DRAFT.md §2, chosen as drafted; the second
//! draft's 60 more — user 2026-10-29 「훨씬 더 많이 추가해줘」 — are §9). The pure parts; the JVM side is
//! `hooks`, the effects themselves HZ-032.
//!
//! Each piece is a host record in its equipment file's layout, as HZ-023's guardian pieces are
//! ([`crate::gear`]), numbered from [`LEGEND_FIRST_ID`] in its file. Stats are the category's guardian
//! piece × [`STAT_PCT`] %, its options, sprite and palette; the save keeps category, number, stats and
//! options, so a name, price or effect can change later — the number never.
//!
//! ⚠ Names, numbers and effects are Claude's draft (hero2's way: 「권장대로」, then tuned by measurement).

use hz_patches::hooks::{GROUND_TOP_ID, LEGEND_FIRST_ID, LEGEND_GRADE};

use crate::{
    gear::{self, Piece},
    text::euc_kr,
};

/// Every piece's level.
pub const LEVEL: i8 = 85;
/// Stats × this percent of the category's guardian piece (LV 80), rounded half up.
pub const STAT_PCT: i32 = 110;
/// Price × this percent of the guardian piece's.
pub const PRICE_PCT: i32 = 120;

/// What a piece does when worn (HZ-032 — none of it before that patch). The draft's numbers are in the
/// item text; the effect code takes them from here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// W1 회전베기 fires twice (SP and cooldown once).
    WhirlwindTwice,
    /// W2 강타 hits three tiles ahead, +40 %.
    SmashWide,
    /// W3 회전베기's cooldown −50 %.
    WhirlwindCooldown,
    /// W4 SP +8 % a kill; 광기 costs no HP.
    FrenzyKills,
    /// W5 돌격 +60 %, its cooldown −50 %.
    ChargeBoost,
    /// R1 분신 lasts twice as long; +20 % damage while it stands.
    CloneLong,
    /// R2 참격 fires twice.
    SlashTwice,
    /// R3 the first hit after 신속 is a critical; 신속's SP −50 %.
    DashCrit,
    /// R4 독수's chance +30, its poison ×2.
    PoisonBoost,
    /// R5 −30 % damage taken while the clone stands.
    CloneGuard,
    /// K1 반격 never ends (cast again without SP when it runs out).
    CounterForever,
    /// K2 반격's damage +80 %.
    CounterPower,
    /// K3 −25 % damage taken while 반격 is up.
    CounterGuard,
    /// K4 6 % of 반격's damage back as HP.
    CounterLeech,
    /// K5 방패치기 hits the 3×3 round the player and stuns.
    ShieldBashWide,
    /// G1 저격's instant kill twice as likely (not bosses).
    SnipeDouble,
    /// G2 난사 fires twice.
    SprayTwice,
    /// G3 트릭샷 lasts three times as long.
    TrickLong,
    /// G4 조준사격 +60 %, its SP −50 %.
    AimBoost,
    /// G5 −40 % damage taken for 2 s after 연사.
    VolleyGuard,
    /// B1 돌진 fires twice.
    RushTwice,
    /// B2 포격 +50 %, its cooldown −30 %.
    BarrageBoost,
    /// B3 폭발탄's knockback chance +30, +30 % on a knocked-back monster — the 총검사's knockbacks only (`hooks::Knock`;
    /// the text says so since the item test, 2026-10-30).
    BlastBoost,
    /// B4 −15 % damage taken; no knockback.
    BayonetGuard,
    /// S1 동화 starts twice as long; +30 % damage while transformed.
    FusionLong,
    /// S2 each hit while transformed adds 0.25 s to 동화.
    FusionOnHit,
    /// S3 −20 % damage taken while transformed; a kill adds 1 s.
    FusionGuard,
    /// S4 spirit skills' MP −30 %.
    SpiritMana,
    /// S5 spirit skills +50 %, their cooldown −20 %.
    SpiritPower,
    /// C1 every 10 basic hits take 1 s off every cooling skill (spirit skills too).
    TimeThief,
    /// C2 SP +2 a basic hit.
    FlowSp,
    /// C3 skills' SP −20 %.
    FocusSp,
    /// C4 skills' cooldowns −15 % (spirit skills too).
    TimeCloak,
    /// C5 job skills +20 %.
    ChainSkill,
    /// F1 +0.5 % damage for every piece of equipment in the bags and worn.
    Gravity,
    /// F2 +50 % move speed for 10 s after a potion or wine.
    Drunkard,

    // ---- The second draft (2026-10-29, HEROZERO_NEW_ITEM_DRAFT.md §9): 60 more on the same hooks. ----
    /// 전장의 도끼 — 강타 fires twice.
    StrikeTwice,
    /// 피의 도끼 — 5 % of a basic hit's damage back as HP; drains (`effects::DRAIN_PIECES`).
    BasicLeech,
    /// 광란의 도끼 — +50 % damage under 40 % HP.
    LowHpRage,
    /// 충격의 도끼 — 충격 +80 %, its cooldown −50 %.
    ShockBoost,
    /// 외침의 도끼 — 외침's cooldown −60 %, its SP −50 %.
    ShoutBoost,
    /// 돌격대검 — 돌격 fires twice.
    ChargeTwice,
    /// 회전의 대검 — 회전베기 +100 %.
    WhirlwindPower,
    /// 처형의 대검 — SP +5 a skill hit.
    SkillSpGain,
    /// 거인의 대검 — critical chance +15.
    CritPlus15,
    /// 수호자의 대검 — −25 % damage taken at 70 % HP and up.
    HighHpGuard,
    /// 수호의 검 — 방패치기 fires twice.
    BashTwice,
    /// 돌격의 검 — 방패 돌격 +80 %, its cooldown −50 %.
    ShieldChargeBoost,
    /// 밀어내는 검 — 방패 밀기 +80 %, its SP −50 %.
    PushBoost,
    /// 흡혈의 검 — +40 % damage while 반격 is up; drains.
    CounterStrike,
    /// 회복의 검 — 회복's SP −60 %.
    HealCheap,
    /// 파쇄의 철퇴 — 방패 돌격 fires twice.
    ShieldChargeTwice,
    /// 산의 철퇴 — −10 % damage taken; 반격's reflection +40 %.
    MountainMace,
    /// 맥동의 철퇴 — a kill takes 2 s off every cooldown.
    KillCooldown,
    /// 성채의 철퇴 — 반격 lasts twice as long.
    CounterLong,
    /// 밀기의 철퇴 — 방패 밀기 fires twice.
    PushTwice,
    /// 연격의 단검 — 연격 fires twice.
    ComboTwice,
    /// 기습의 단검 — 기습 +80 %, its cooldown −50 %; drains.
    AmbushBoost,
    /// 정령의 단검 — MP +5 a basic hit; spirit skills +30 % (`effects::spirit_power_pct` — in the text since the item
    /// test, 2026-10-30, which found it unlisted), their cooldown −20 %.
    SpiritDagger,
    /// 연사의 총 — 연사 fires twice.
    VolleyTwice,
    /// 근접의 총 — 근접사격 +80 %, its cooldown −50 %; drains.
    PointBlankBoost,
    /// 저격수의 총 — 저격's SP −60 %, its cooldown −50 %.
    SnipeCheap,
    /// 조준의 총 — 조준사격 fires twice.
    AimTwice,
    /// 포격 총검 — 포격 fires twice.
    BarrageTwice,
    /// 흡혈 총검 — HP +3 % a skill hit; drains.
    SkillHeal,
    /// 연쇄 총검 — 돌진 +60 %, its cooldown −50 %.
    RushBoost,
    /// 화염 총검 — 포격's SP −50 %.
    BarrageCheap,
    /// 전장의 총검 — +30 % damage at full SP.
    FullSpRage,
    /// 철벽의 방패 — −20 % damage taken.
    IronWall,
    /// 반사의 방패 — 반격's cooldown −50 %, its SP −50 %.
    CounterCheap,
    /// 생명의 방패 — 20 % of damage taken comes as SP.
    DamageToSp,
    /// 거북의 방패 — −50 % damage taken under 30 % HP.
    LowHpGuard,
    /// 충격의 방패 — 방패치기 +60 %, its SP −50 %.
    BashBoost,
    /// 급소의 장갑 — critical chance +20.
    CritGlove,
    /// 사수의 장갑 — 트릭샷's cooldown −50 %; 연사 +40 %.
    GunnerGlove,
    /// 영매의 장갑 — 동화's SP −50 %, its cooldown −50 %; drains.
    FusionCheap,
    /// 투사의 장갑 — basic hits' critical chance +25.
    BasicCrit,
    /// 강철의 장갑 — −30 % damage taken for 2 s after a skill's start.
    SkillGuard,
    /// 분노의 장갑 — critical chance +40 under 40 % HP.
    LowHpCrit,
    /// 현자의 투구 — spirit skills' MP −20 %; MP +10 % a kill.
    SageHelm,
    /// 행운의 반지 — the 85제 and recipe drops twice as likely.
    Lucky,
    /// 생명의 반지 — HP +15 % a kill; drains.
    LifeRing,
    /// 복수의 목걸이 — skills' SP −50 % under 50 % HP.
    LowHpCheap,
    /// 인내의 목걸이 — −10 % damage taken; every cooldown −10 %.
    Endurance,
    /// 유령의 외투 — critical chance +30 while the clone stands; 분신 lasts half again.
    GhostCloak,
    /// 사냥꾼의 외투 — +25 % damage at 70 % HP and up.
    HighHpRage,
    /// 불굴의 갑주 — −30 % damage taken for 3 s after a kill.
    KillGuard,
    /// 전쟁의 갑주 — basic hits +30 %.
    BasicPower,
    /// 질풍의 신발 — walks half again as fast.
    SwiftBoots,
    /// 그림자 신발 — 신속 fires twice.
    DashTwice,
    /// 술사의 신발 — walks a fifth faster, not while transformed (동화's own walk is 16 px a frame, twice the 8 the
    /// walking hook speeds up; the item test, 2026-10-30 — the text's 「동화 중에는 +50%」 never happened).
    FusionSwift,
    /// 사수의 신발 — 연사 +30 %, its SP −50 %.
    VolleyBoost,
    /// 돌진의 장화 — 돌진's SP −50 %.
    RushCheap,
    /// 철벽의 장화 — a potion fills SP.
    PotionSp,
    /// 광전사의 장화 — 회전베기's SP −50 %.
    WhirlwindCheap,
    /// 기사의 장화 — 전력방어's SP −50 %, its cooldown −50 %.
    FullGuardCheap,

    // ---- The 2nd balance pass (HEROZERO_CLASS_BALANCE.md §6-C, 2026-10-29): six more for the jobs short of pieces. ----
    /// 정령사의 반지 — half of 의지 on every hit.
    SpiritRing,
    /// 서약의 관 — spirit skills +30 %, their MP −10 %.
    OathCrown,
    /// 암살자의 반지 — +40 % damage for 3 s after a 기습.
    AssassinRing,
    /// 그림자 투구 — critical chance +20 while the clone stands.
    ShadowHelm,
    /// 폭발의 반지 — +50 % on a monster the 총검사 knocks back (`hooks::Knock`; 「총검사 전용」 in the text since the
    /// item test, 2026-10-30).
    BlastRing,
    /// 포수의 장화 — 포격's cooldown −30 %.
    GunnerBoots,
}

/// One 85제 piece: where it is, its name, what it does and the two lines the item window shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Legend {
    pub category: i8,
    pub id: i8,
    pub name: &'static str,
    pub effect: Effect,
    pub text: [&'static str; 2],
}

const fn l(category: i8, id: i8, name: &'static str, effect: Effect, text: [&'static str; 2]) -> Legend {
    Legend { category, id, name, effect, text }
}

use Effect::*;

/// How many pieces there are: the first draft's 36, the second's 60 and the 2nd balance pass's 6.
pub const COUNT: usize = 102;
/// How many of them have a recipe: `/itm/18` 28..127 — a number is a signed byte, so the pieces past the
/// hundredth are drop-only.
pub const RECIPE_COUNT: usize = 100;

/// The pieces, by category (`/itm/00` 도끼 … `/itm/15` 중신발), numbered from 41 in each — the first
/// draft's 36, then the second draft's 60 (a category's numbers go on where the first draft left them;
/// a recipe's number is the piece's index here + [`RECIPE_FIRST`], so the order never changes).
pub const ALL: [Legend; COUNT] = [
    l(0, 41, "선풍의 도끼", WhirlwindTwice, ["회전베기가 두 번", "나간다"]),
    l(1, 41, "분쇄자의 대검", SmashWide, ["강타가 전방 3칸 공격", "강타 피해 +40%"]),
    l(2, 41, "심판의 검", CounterPower, ["반격 피해 +80%", ""]),
    l(3, 41, "울림의 철퇴", ShieldBashWide, ["방패치기가 주변 3x3", "공격하고 기절시킨다"]),
    l(4, 41, "그림자 쌍검", CloneLong, ["분신 지속시간 2배", "분신이 있으면 피해 +35%"]),
    l(4, 42, "바람가르개", SlashTwice, ["참격이 두 번", "발사된다"]),
    l(4, 43, "맹약의 단검", FusionLong, ["동화 지속 2배 피해 +15%", "동화 중에는 피해 +40%"]),
    l(5, 41, "사신의 총", SnipeDouble, ["저격 즉사 확률 2배", "(보스 제외)"]),
    l(5, 42, "탄막의 총", SprayTwice, ["난사가 두 번", "나간다"]),
    l(6, 41, "연쇄 폭뢰", RushTwice, ["돌진이 두 번", "나간다"]),
    l(7, 41, "응징의 방패", CounterForever, ["반격이 꺼지지 않는다", "(SP 없이 다시 걸림)"]),
    l(8, 41, "맹독의 손", PoisonBoost, ["독수 확률 +30", "독 피해 3배"]),
    l(8, 42, "사냥꾼의 장갑", AimBoost, ["조준사격 피해 +60%", "조준사격 SP -50%"]),
    l(8, 43, "공허의 장갑", SpiritMana, ["정령 스킬 MP -30%", ""]),
    l(9, 41, "되갚는 손", CounterLeech, ["반격 피해의 6%만큼", "HP 회복"]),
    l(9, 42, "화약 장갑", BlastBoost, ["(총검사) 폭발탄 확률 +30", "넉백된 적 피해 +30%"]),
    l(9, 43, "연환의 장갑", ChainSkill, ["직업 스킬 피해 +20%", ""]),
    l(10, 41, "폭풍의 투구", WhirlwindCooldown, ["회전베기 대기시간", "-50%"]),
    l(10, 42, "매의 눈", TrickLong, ["트릭샷 지속시간 3배", ""]),
    l(10, 43, "포격수의 투구", BarrageBoost, ["포격 피해 +50%", "포격 대기시간 -30%"]),
    l(10, 44, "정령왕의 관", FusionOnHit, ["동화 중 타격마다", "동화 시간 +0.25초"]),
    l(10, 45, "집중의 관", FocusSp, ["스킬 SP 소모 -20%", ""]),
    l(11, 41, "원소의 반지", SpiritPower, ["정령 스킬 피해 +80%", "정령 대기시간 -20%"]),
    l(11, 42, "시간도둑", TimeThief, ["기본공격 10회마다", "모든 대기시간 -1초"]),
    l(12, 41, "밤의 장막", CloneGuard, ["분신이 있는 동안", "받는 피해 -30%"]),
    l(12, 42, "바람의 코트", VolleyGuard, ["연사 뒤 2초 동안", "받는 피해 -40%"]),
    l(12, 43, "영혼의 외투", FusionGuard, ["받는 피해 -10%(동화 -20%)", "처치마다 동화 +1초"]),
    l(12, 44, "시간의 외투", TimeCloak, ["스킬 대기시간 -15%", ""]),
    l(13, 41, "광전사의 흉갑", FrenzyKills, ["처치마다 SP +6%", "광기가 HP를 쓰지 않음"]),
    l(13, 42, "가시 판금", CounterGuard, ["반격 중 받는 피해", "-25%"]),
    l(13, 43, "철벽 총검갑", BayonetGuard, ["받는 피해 -15%", "넉백되지 않는다"]),
    l(13, 44, "중력의 갑주", Gravity, ["가방 장비 1개마다", "피해 +0.5%"]),
    l(14, 41, "그림자 걸음", DashCrit, ["신속 뒤 첫 공격 크리", "신속 SP -50%"]),
    l(14, 42, "흐름의 장화", FlowSp, ["기본공격 적중마다", "SP +2"]),
    l(15, 41, "돌격대장의 장화", ChargeBoost, ["돌격 피해 +60%", "돌격 대기시간 -50%"]),
    l(15, 42, "술꾼의 장화", Drunkard, ["물약을 마시면 10초", "이동속도 +50%"]),
    // ---- The second draft. ----
    l(0, 42, "전장의 도끼", StrikeTwice, ["강타가 두 번", "나간다"]),
    l(0, 43, "피의 도끼", BasicLeech, ["기본공격 피해 5% HP 회복", "모든 피해의 2% HP 회복"]),
    l(0, 44, "광란의 도끼", LowHpRage, ["HP 40% 미만이면", "피해 +50%"]),
    l(0, 45, "충격의 도끼", ShockBoost, ["충격 피해 +80%", "충격 대기시간 -50%"]),
    l(0, 46, "외침의 도끼", ShoutBoost, ["외침 대기시간 -60%", "외침 SP -50%"]),
    l(1, 42, "돌격대검", ChargeTwice, ["돌격이 두 번", "나간다"]),
    l(1, 43, "회전의 대검", WhirlwindPower, ["회전베기 피해 +60%", ""]),
    l(1, 44, "처형의 대검", SkillSpGain, ["스킬 적중마다", "SP +5"]),
    l(1, 45, "거인의 대검", CritPlus15, ["크리티컬 확률 +15", ""]),
    l(1, 46, "수호자의 대검", HighHpGuard, ["HP 70% 이상이면", "받는 피해 -25%"]),
    l(2, 42, "수호의 검", BashTwice, ["방패치기가 두 번", "나간다"]),
    l(2, 43, "돌격의 검", ShieldChargeBoost, ["방패 돌격 피해 +80%", "방패 돌격 대기시간 -50%"]),
    l(2, 44, "밀어내는 검", PushBoost, ["방패 밀기 피해 +80%", "방패 밀기 SP -50%"]),
    l(2, 45, "흡혈의 검", CounterStrike, ["반격 중 피해 +40%", "모든 피해의 2% HP 회복"]),
    l(2, 46, "회복의 검", HealCheap, ["회복 SP -60%", ""]),
    l(3, 42, "파쇄의 철퇴", ShieldChargeTwice, ["방패 돌격이 두 번", "나간다"]),
    l(3, 43, "산의 철퇴", MountainMace, ["받는 피해 -10%", "반격 피해 +40%"]),
    l(3, 44, "맥동의 철퇴", KillCooldown, ["처치마다 모든", "대기시간 -2초"]),
    l(3, 45, "성채의 철퇴", CounterLong, ["반격 지속시간 2배", ""]),
    l(3, 46, "밀기의 철퇴", PushTwice, ["방패 밀기가 두 번", "나간다"]),
    l(4, 44, "연격의 단검", ComboTwice, ["연격이 두 번", "나간다"]),
    l(4, 45, "기습의 단검", AmbushBoost, ["기습 피해 +80% 대기 -50%", "모든 피해의 2% HP 회복"]),
    l(4, 46, "정령의 단검", SpiritDagger, ["기본공격 적중마다 MP +5", "정령 피해 +30% 대기 -20%"]),
    l(5, 43, "연사의 총", VolleyTwice, ["연사가 두 번", "나간다"]),
    l(5, 44, "근접의 총", PointBlankBoost, ["근접사격 +80% 대기 -50%", "모든 피해의 2% HP 회복"]),
    l(5, 45, "저격수의 총", SnipeCheap, ["저격 SP -60%", "저격 대기시간 -50%"]),
    l(5, 46, "조준의 총", AimTwice, ["조준사격이 두 번", "나간다"]),
    l(6, 42, "포격 총검", BarrageTwice, ["포격이 두 번", "나간다"]),
    l(6, 43, "흡혈 총검", SkillHeal, ["스킬 적중마다 HP +3%", "모든 피해의 2% HP 회복"]),
    l(6, 44, "연쇄 총검", RushBoost, ["돌진 피해 +60%", "돌진 대기시간 -50%"]),
    l(6, 45, "화염 총검", BarrageCheap, ["포격 SP -50%", ""]),
    l(6, 46, "전장의 총검", FullSpRage, ["SP가 가득이면", "피해 +30%"]),
    l(7, 42, "철벽의 방패", IronWall, ["받는 피해 -20%", ""]),
    l(7, 43, "반사의 방패", CounterCheap, ["반격 대기시간 -50%", "반격 SP -50%"]),
    l(7, 44, "생명의 방패", DamageToSp, ["받은 피해의 20%를", "SP로"]),
    l(7, 45, "거북의 방패", LowHpGuard, ["HP 30% 미만이면", "받는 피해 -50%"]),
    l(7, 46, "충격의 방패", BashBoost, ["방패치기 피해 +60%", "방패치기 SP -50%"]),
    l(8, 44, "급소의 장갑", CritGlove, ["크리티컬 확률 +20", ""]),
    l(8, 45, "사수의 장갑", GunnerGlove, ["트릭샷 대기시간 -50%", "연사 피해 +40%"]),
    l(8, 46, "영매의 장갑", FusionCheap, ["동화 SP -50% 대기 -50%", "모든 피해의 2% HP 회복"]),
    l(9, 44, "투사의 장갑", BasicCrit, ["기본공격 크리티컬", "확률 +25"]),
    l(9, 45, "강철의 장갑", SkillGuard, ["스킬을 쓰면 2초 동안", "받는 피해 -30%"]),
    l(9, 46, "분노의 장갑", LowHpCrit, ["HP 40% 미만이면", "크리티컬 확률 +40"]),
    l(10, 46, "현자의 투구", SageHelm, ["정령 스킬 MP -20%", "처치마다 MP +10%"]),
    l(11, 43, "행운의 반지", Lucky, ["가디언 모드의 85제와", "조합서 드랍 2배"]),
    l(11, 44, "생명의 반지", LifeRing, ["처치마다 HP +15%", "모든 피해의 2% HP 회복"]),
    l(11, 45, "복수의 목걸이", LowHpCheap, ["HP 50% 미만이면", "스킬 SP -50%"]),
    l(11, 46, "인내의 목걸이", Endurance, ["받는 피해 -10%", "모든 대기시간 -10%"]),
    l(12, 45, "유령의 외투", GhostCloak, ["분신 중 크리티컬 +30", "분신 지속시간 +50%"]),
    l(12, 46, "사냥꾼의 외투", HighHpRage, ["HP 70% 이상이면", "피해 +25%"]),
    l(13, 45, "불굴의 갑주", KillGuard, ["처치 뒤 3초 동안", "받는 피해 -30%"]),
    l(13, 46, "전쟁의 갑주", BasicPower, ["기본공격 피해 +30%", ""]),
    l(14, 43, "질풍의 신발", SwiftBoots, ["이동속도 +50%", ""]),
    l(14, 44, "그림자 신발", DashTwice, ["신속이 두 번", "나간다"]),
    l(14, 45, "술사의 신발", FusionSwift, ["이동속도 +20%", "(동화 중에는 제외)"]),
    l(14, 46, "사수의 신발", VolleyBoost, ["연사 피해 +30%", "연사 SP -50%"]),
    l(15, 43, "돌진의 장화", RushCheap, ["돌진 SP -50%", ""]),
    l(15, 44, "철벽의 장화", PotionSp, ["물약을 마시면", "SP가 가득 찬다"]),
    l(15, 45, "광전사의 장화", WhirlwindCheap, ["회전베기 SP -50%", ""]),
    l(15, 46, "기사의 장화", FullGuardCheap, ["전력방어 SP -50%", "전력방어 대기시간 -50%"]),
    // ---- The 2nd balance pass: six more (the last two have no recipe — RECIPE_COUNT). ----
    l(11, 47, "정령사의 반지", SpiritRing, ["의지의 절반이", "피해에 더해진다"]),
    l(10, 47, "서약의 관", OathCrown, ["정령 스킬 피해 +30%", "정령 스킬 MP -10%"]),
    l(11, 48, "암살자의 반지", AssassinRing, ["기습 뒤 3초 동안", "피해 +40%"]),
    l(10, 48, "그림자 투구", ShadowHelm, ["분신 중 크리티컬", "확률 +20"]),
    l(11, 49, "폭발의 반지", BlastRing, ["넉백된 적 피해 +50%", "(총검사 전용)"]),
    l(15, 47, "포수의 장화", GunnerBoots, ["포격 대기시간 -30%", ""]),
];

/// The piece at (`category`, `id`), if one is.
pub fn find(category: i8, id: i8) -> Option<&'static Legend> {
    ALL.iter().find(|l| l.category == category && l.id == id)
}

/// The piece `path` / `index` asks the factory for, if it is one.
pub fn asked(path: &str, index: i32) -> Option<&'static Legend> {
    let id = i8::try_from(index).ok()?;
    (LEGEND_FIRST_ID..GROUND_TOP_ID).contains(&id).then_some(())?;
    find(gear::category_of(path)?, id)
}

fn scaled(v: i32, pct: i32) -> i32 {
    (v * pct + 50) / 100
}

/// The piece's record fields: its category's guardian piece raised to this tier.
pub fn piece(l: &Legend) -> Piece {
    let base = gear::piece(l.category).expect("every equipment category has a guardian piece");
    let stat = |s: i16| if s < 0 { s } else { scaled(s.into(), STAT_PCT) as i16 };
    Piece {
        name: l.name,
        price: scaled(base.price, PRICE_PCT),
        stats: base.stats.map(stat),
        h: if base.h < 0 { base.h } else { scaled(base.h.into(), STAT_PCT) as i8 },
        ..*base
    }
}

/// The record `rpg/e.a(String,I)[B` returns for the piece.
pub fn record(l: &Legend) -> Vec<u8> {
    gear::record_at(&piece(l), LEVEL, LEGEND_GRADE)
}

/// The item window's lines for the piece (EUC-KR, the empty ones left out).
pub fn lines(l: &Legend) -> Vec<Vec<u8>> {
    l.text.iter().filter(|t| !t.is_empty()).map(|t| euc_kr(t).expect("in the game's font")).collect()
}

/// 「전설」, the grade's name.
pub const GRADE_NAME: &str = "전설";

/// A piece drops from a 가디언 모드 kill this many times in 1,000 — twice that at 난이도 어려움 (the first
/// draft's 36 dropped 15 in 1,000; the second draft's 96 drop 25, so one piece comes about as often).
pub const DROP_PER_MILLE: i32 = 25;

// ------------------------------------------------------------------------- who drops what ----
//
// Until 2026-10-31 every 가디언 모드 monster drew from all 102 pieces. The user: 「모든몬스터 말고 다른 난이도에
// 맞춰서 몬스터별로 드랍이 다르게」 — so the 85제 follow the game's own rule of the other modes (`cq.o()V`,
// HEROZERO_GAME_OPTIONS_RE.md §3·§18): a monster's drop block names the categories it drops, its level the number
// (common `LV/4`, rare `LV/5 + 13`). Here: the categories of its equipment slots, and in each category the piece
// whose share of 가디언 levels [`DROP_LOW`]..=[`DROP_HIGH`] holds its level — a category's pieces split the span in
// number order. Whether a kill drops one at all is unchanged ([`DROP_PER_MILLE`], [`RECIPE_PER_MILLE`]).

/// The 가디언 monster levels the pieces are laid over: the kinds' 85..110 (`guardian_mode`; the game's +0..2 of a
/// plain monster stays in it — base 43 is 108).
pub const DROP_LOW: i32 = crate::guardian_mode::LOW;
pub const DROP_HIGH: i32 = crate::guardian_mode::HIGH;

/// The equipment slots of a drop block `eq.b` (24 bytes: four slots of category · category-or-number · threshold
/// int) that name a category range — slots 0 and 1 (the commons), 2 (the rare band); slot 3 is one fixed item.
const RANGE_SLOTS: usize = 3;

/// The equipment categories (bit n = category n) a monster's drop block names in its slots 0..2 — the game draws
/// a slot's category anywhere in `lo..=hi` (`rpg/g.a(Random, lo, hi)`), and skips a slot of category −1
/// (돌기둥, kind 121: none at all).
pub fn slot_categories(block: &[i8]) -> u16 {
    let mut mask = 0u16;
    for slot in block.chunks_exact(6).take(RANGE_SLOTS) {
        let (lo, hi) = (i32::from(slot[0]), i32::from(slot[1]));
        if lo < 0 {
            continue;
        }
        for c in lo..=hi.min(i32::from(hz_patches::hooks::GEAR_CATEGORIES) - 1) {
            mask |= 1 << c;
        }
    }
    mask
}

/// Category `category`'s pieces, in number order.
fn of_category(category: i8) -> impl Iterator<Item = &'static Legend> {
    ALL.iter().filter(move |l| l.category == category)
}

/// Which of a category's `n` pieces a level-`level` monster drops: the span cut in `n` even shares.
fn share(level: i32, n: usize) -> usize {
    let span = (DROP_HIGH - DROP_LOW + 1) as usize;
    let at = (level.clamp(DROP_LOW, DROP_HIGH) - DROP_LOW) as usize;
    (at * n / span).min(n.saturating_sub(1))
}

/// The piece of `category` a level-`level` 가디언 monster drops.
pub fn at_level(category: i8, level: i32) -> Option<&'static Legend> {
    let n = of_category(category).count();
    (n > 0).then(|| of_category(category).nth(share(level, n))).flatten()
}

/// The levels (first, last) whose monsters drop `piece` — for the drop table and the tests.
pub fn levels(piece: &Legend) -> (i32, i32) {
    let all: Vec<i32> = (DROP_LOW..=DROP_HIGH).filter(|&l| at_level(piece.category, l).is_some_and(|p| p.id == piece.id)).collect();
    (all[0], *all.last().unwrap())
}

/// What a monster of `categories` ([`slot_categories`]) at `level` can drop: its level's piece of each category.
pub fn pool(categories: u16, level: i32) -> Vec<&'static Legend> {
    (0..hz_patches::hooks::GEAR_CATEGORIES).filter(|&c| categories & (1 << c) != 0).filter_map(|c| at_level(c, level)).collect()
}

/// The piece one draw of the game's Random (`abs(nextInt())`, `roll`) gives a 가디언 모드 kill of a monster whose
/// [`pool`] is `pool`, if any: its low three digits decide whether (× `luck`, 행운의 반지), the rest which.
pub fn roll(roll: i32, hard: bool, luck: u32, pool: &[&'static Legend]) -> Option<&'static Legend> {
    let roll = roll.unsigned_abs();
    let chance = (DROP_PER_MILLE * if hard { 2 } else { 1 }) as u32 * luck.max(1);
    (roll % 1000 < chance && !pool.is_empty()).then(|| pool[(roll / 1000) as usize % pool.len()])
}

/// A boss's piece (HZ-046, [`crate::balance::BOSS_LEGEND_PCT`] decided by the roll's low two digits): one of its
/// [`pool`], by the rest.
pub fn boss_pick(roll: i32, pool: &[&'static Legend]) -> Option<&'static Legend> {
    (!pool.is_empty()).then(|| pool[(roll.unsigned_abs() / 100) as usize % pool.len()])
}

// ------------------------------------------------------------------------------------ recipes ----

/// HZ-035: the 85제 recipes' numbers in `/itm/18` (the game's are 0..27), one a piece in [`ALL`]'s order.
pub const RECIPE_FIRST: i8 = 28;
/// The equipment categories' names in a recipe's text (`t/it` [0..15]).
const CATEGORY_NAMES: [&str; 16] = [
    "도끼", "양손검", "도검", "둔기", "단검", "총", "총검", "방패", "경장갑", "중장갑", "투구", "장신구", "경갑옷", "중갑옷", "경신발", "중신발",
];
/// The top materials (`/itm/16`): 미스릴 9, 아다만티움 10, 현자의돌 11.
const MITHRIL: (i8, &str) = (9, "미스릴");
const ADAMANTIUM: (i8, &str) = (10, "아다만티움");
const SAGES_STONE: (i8, &str) = (11, "현자의돌");
/// The game's LV 55 세트 accessories (`/itm/11` 3..5), one in turn for each recipe.
const SET_ACCESSORIES: [(i8, &str); 3] = [(3, "봉인의 줄"), (4, "붉은사자의심장"), (5, "폭풍의 펜던트")];

/// One 85제 recipe: the piece it makes and its three ingredients (category, number, name) — hero2's
/// rule (HERO2_NEW_ITEM_DRAFT.md §6.3): the category's guardian piece + a top material + an end-game
/// accessory.
#[derive(Clone, Copy, Debug)]
pub struct Recipe {
    pub id: i8,
    pub makes: &'static Legend,
    pub ingredients: [(i8, i8, &'static str); 3],
}

/// The recipe number `id`, if it is an 85제 one.
pub fn recipe(id: i8) -> Option<Recipe> {
    let i = usize::try_from(id.checked_sub(RECIPE_FIRST)?).ok()?;
    (i < RECIPE_COUNT).then_some(())?;
    let makes = ALL.get(i)?;
    let guardian = gear::piece(makes.category)?;
    let material = match makes.category {
        0..=7 => ADAMANTIUM,
        11 => SAGES_STONE,
        _ => MITHRIL,
    };
    let accessory = SET_ACCESSORIES[i % SET_ACCESSORIES.len()];
    Some(Recipe {
        id,
        makes,
        ingredients: [(makes.category, hz_patches::hooks::GEAR_ID, guardian.name), (16, material.0, material.1), (11, accessory.0, accessory.1)],
    })
}

/// The game's colour codes in its font (`ah`, javap 2026-10-28): `{` orange, `}` tan (the game's epic
/// results), `^` green (세트), `&` back to white; `;` is the line break.
const ORANGE: char = '{';
const TAN: char = '}';
const GREEN: char = '^';

/// The record `rpg/e.a(String,I)[B` returns for `/itm/18` number `r.id`, in the game's recipe layout
/// (`es.a([BI)I`): name, price, text, the three ingredients and the result as (category, number)
/// bytes, then the check byte (the byte sum before it).
pub fn recipe_record(r: &Recipe) -> Vec<u8> {
    let name = euc_kr(&format!("{ORANGE}{}&;", r.makes.name)).expect("in the game's font");
    let [(_, _, guardian), (_, _, material), (_, _, accessory)] = r.ingredients;
    let text = format!("LV{LEVEL} {};{TAN}{guardian}&;{material};{GREEN}{accessory}", CATEGORY_NAMES[r.makes.category as usize]);
    let text = euc_kr(&text).expect("in the game's font");
    let mut out = vec![name.len() as u8];
    out.extend(&name);
    out.extend(10i32.to_be_bytes());
    out.push(text.len() as u8);
    out.extend(&text);
    for (cat, id, _) in r.ingredients {
        out.extend([cat as u8, id as u8]);
    }
    out.extend([r.makes.category as u8, r.makes.id as u8]);
    out.push(out.iter().fold(0u8, |s, &b| s.wrapping_add(b)));
    out
}

/// The recipe `path` / `index` asks the factory for, if it is an 85제 one.
pub fn recipe_asked(path: &str, index: i32) -> Option<Recipe> {
    (path == "/itm/18").then_some(())?;
    recipe(i8::try_from(index).ok()?)
}

/// A 가디언 모드 kill drops a recipe this many times in 1,000 (hero2's 조합서 2 %; 25 since the second
/// draft's 96), any of them.
pub const RECIPE_PER_MILLE: u32 = 25;

/// The recipe number of `piece`, if it has one (the first [`RECIPE_COUNT`] of [`ALL`]).
pub fn recipe_of(piece: &Legend) -> Option<i8> {
    let i = ALL.iter().position(|l| (l.category, l.id) == (piece.category, piece.id))?;
    (i < RECIPE_COUNT).then(|| RECIPE_FIRST + i as i8)
}

/// The recipe one draw of the game's Random gives a 가디언 모드 kill, if any (× `luck`, 행운의 반지): the recipe of
/// one of the pieces the monster drops ([`pool`] — those that have one).
pub fn recipe_roll(roll: i32, luck: u32, pool: &[&'static Legend]) -> Option<i8> {
    let roll = roll.unsigned_abs();
    let recipes: Vec<i8> = pool.iter().filter_map(|p| recipe_of(p)).collect();
    (roll % 1000 < RECIPE_PER_MILLE * luck.max(1) && !recipes.is_empty()).then(|| recipes[(roll / 1000) as usize % recipes.len()])
}

const _: () = assert!(RECIPE_COUNT <= COUNT);
const _: () = assert!(RECIPE_FIRST as usize + RECIPE_COUNT <= i8::MAX as usize + 1, "recipe numbers are the game's signed bytes");
const _: () = assert!(RECIPE_FIRST as usize + RECIPE_COUNT <= hz_patches::hooks::RECIPE_GROUND_TOP as usize + 1, "the ground list takes every recipe");

/// The random box's recipe number (script action 18: `n / 4` of `n` in 0..182 — up to 45, past the
/// game's 27): the game's own recipes only, the 85제 ones are 가디언 모드's.
pub fn box_recipe(n: i32) -> i32 {
    let id = n.max(0) / 4;
    if id < i32::from(RECIPE_FIRST) { id } else { n.max(0) % i32::from(RECIPE_FIRST) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the game's font draws besides KS X 1001 syllables (CLAUDE.md: `$ ^ { | } \` change the colour,
    /// `& ;` end the line, `_` is a wide space — none of them, nor `,` or `·`, which it lacks).
    fn font_ok(s: &str) -> bool {
        s.chars().all(|c| {
            ('가'..='힣').contains(&c) || c.is_ascii_alphanumeric() || " !\"#%()*+-./:<=>?@[]~'`".contains(c)
        }) && euc_kr(s).is_some()
    }

    #[test]
    fn the_pieces_have_their_own_numbers() {
        for (i, a) in ALL.iter().enumerate() {
            assert!(ALL[..i].iter().all(|b| (b.category, b.id) != (a.category, a.id)), "{} twice", a.name);
            assert!((0..hz_patches::hooks::GEAR_CATEGORIES).contains(&a.category));
            assert!((LEGEND_FIRST_ID..GROUND_TOP_ID).contains(&a.id));
            // Numbered from 41 in each file, no gaps.
            let before = ALL[..i].iter().filter(|b| b.category == a.category).count() as i8;
            assert_eq!(a.id, LEGEND_FIRST_ID + before, "{}", a.name);
            assert!(ALL[..i].iter().all(|b| b.effect != a.effect), "{:?} twice", a.effect);
        }
    }

    #[test]
    fn names_and_lines_are_the_games_letters() {
        for a in &ALL {
            assert!(font_ok(a.name), "{}", a.name);
            assert!(!a.text[0].is_empty());
            for t in a.text {
                assert!(font_ok(t), "{t:?}");
                // The item window's text column is 148 px; the font's syllable is 11 px, ASCII 6.
                let w: usize = t.chars().map(|c| if c.is_ascii() { 6 } else { 11 }).sum();
                assert!(w <= 148, "{t:?} is {w} px");
            }
        }
        assert!(font_ok(GRADE_NAME));
    }

    #[test]
    fn a_piece_is_its_guardian_piece_raised() {
        let axe = find(0, 41).unwrap();
        let p = piece(axe);
        assert_eq!((p.stats, p.h, p.price, p.sprite), ([149, 598, -1], 18, 35_100, 15), "가디언 액스 135-544/16 % x1.1");
        let r = record(axe);
        assert_eq!(r[0] as usize, euc_kr(axe.name).unwrap().len());
        let p = 1 + r[0] as usize + 4;
        let p = p + 1 + r[p] as usize;
        assert_eq!((r[p] as i8, r[p + 1] as i8), (LEVEL, LEGEND_GRADE));
        assert_eq!(*r.last().unwrap(), r[..r.len() - 1].iter().fold(0u8, |s, &b| s.wrapping_add(b)));
        assert_eq!(asked("/itm/00", 41).map(|l| l.name), Some("선풍의 도끼"));
        assert!(asked("/itm/00", 40).is_none() && asked("/itm/16", 41).is_none() && asked("/itm/00", 49).is_none());
    }

    #[test]
    fn every_piece_has_a_recipe_of_three_ingredients() {
        for (i, piece) in ALL.iter().take(RECIPE_COUNT).enumerate() {
            let id = RECIPE_FIRST + i as i8;
            let r = recipe(id).unwrap();
            assert_eq!((r.makes.category, r.makes.id), (piece.category, piece.id));
            assert!(r.ingredients.iter().all(|&(c, n, _)| c >= 0 && n >= 0), "three real ingredients");
            assert_ne!((r.ingredients[0].0, r.ingredients[0].1), (r.ingredients[2].0, r.ingredients[2].1), "distinct");
            let rec = recipe_record(&r);
            let text_at = 1 + rec[0] as usize + 4;
            assert!(rec[0] < 128 && rec[text_at] < 128, "lengths are the game's signed bytes");
            assert_eq!(rec.len(), text_at + 1 + rec[text_at] as usize + 8 + 1, "name, price, text, 8 bytes, check");
            assert_eq!(*rec.last().unwrap(), rec[..rec.len() - 1].iter().fold(0u8, |s, &b| s.wrapping_add(b)));
            let n = rec.len();
            assert_eq!(&rec[n - 3..n - 1], &[piece.category as u8, piece.id as u8], "the result");
            assert_eq!(&rec[n - 9..n - 7], &[piece.category as u8, 40], "the guardian piece first");
        }
        assert!(recipe(RECIPE_FIRST - 1).is_none() && recipe_asked("/itm/18", 128).is_none(), "the pieces past the hundredth have no recipe");
        assert_eq!(recipe(127).map(|r| r.makes.name), Some("그림자 투구"), "the hundredth piece's recipe is 127");
        assert_eq!(recipe_asked("/itm/18", 28).map(|r| r.makes.name), Some("선풍의 도끼"));
        assert_eq!(recipe_asked("/itm/18", 28 + 36).map(|r| r.makes.name), Some("전장의 도끼"), "the second draft's first");
        assert_eq!(recipe_asked("/itm/18", 123).map(|r| r.makes.name), Some("기사의 장화"), "the second draft's last is 123");
        assert!(recipe_asked("/itm/18", 127).is_some() && recipe_asked("/itm/17", 28).is_none());
    }

    /// `/en/edf` drop blocks (tools_re/hz_drops.py, 2026-10-31): most kinds' (weapons 0..6 · armour 7..15 · rare
    /// 0..15), 머미 (13)'s, 고르곤 (31)'s and 돌기둥 (121)'s.
    const MOST: [i8; 24] = [0, 6, 0, 0, 18, -31, 7, 15, 0, 0, 37, -62, 0, 15, 0, 0, 56, -92, -1, 0, 0, 0, 56, -92];
    const MUMMY: [i8; 24] = [0, 5, 0, 0, 18, -31, 12, 15, 0, 0, 37, -62, 0, 5, 0, 0, 56, -92, -1, 0, 0, 0, 56, -92];
    const GORGON: [i8; 24] = [0, 5, 0, 0, 18, -31, 7, 10, 0, 0, 37, -62, 7, 15, 0, 0, 56, -92, -1, 0, 0, 0, 56, -92];
    const PILLAR: [i8; 24] = [-1, -1, 0, 0, 18, -31, -1, -1, 0, 0, 37, -62, -1, -1, 0, 0, 56, -92, -1, 0, 0, 0, 56, -92];
    const EVERY: u16 = 0xFFFF;

    #[test]
    fn a_monster_drops_the_categories_of_its_slots() {
        assert_eq!(slot_categories(&MOST), EVERY);
        assert_eq!(slot_categories(&MUMMY), 0b1111_0000_0011_1111, "weapons 0..5 and 12..15 — no 6..11");
        assert_eq!(slot_categories(&GORGON), 0xFFFF & !(1 << 6), "no 총검");
        assert_eq!(slot_categories(&PILLAR), 0, "돌기둥 drops nothing in any mode");
        assert_eq!(slot_categories(&[19, 1, 0, 0, 56, -92]), 0, "a quest item's (19, 1) is no equipment range");
        assert_eq!(slot_categories(&[3, 3, 0, 0, 0, 0, -1, 0, 0, 0, 0, 0, -1, 0, 0, 0, 0, 0, 19, 1, 0, 0, 0, 0]), 1 << 3, "slot 3 is a fixed item, not a range");
    }

    #[test]
    fn a_monsters_level_picks_the_number_in_each_category() {
        for cat in 0..hz_patches::hooks::GEAR_CATEGORIES {
            let ids: Vec<i8> = (DROP_LOW..=DROP_HIGH).map(|l| at_level(cat, l).unwrap().id).collect();
            assert_eq!(ids[0], LEGEND_FIRST_ID, "category {cat}: the first piece at the bottom");
            assert_eq!(*ids.last().unwrap(), of_category(cat).last().unwrap().id, "category {cat}: the last at the top");
            assert!(ids.windows(2).all(|w| w[1] == w[0] || w[1] == w[0] + 1), "category {cat}: in number order, none skipped");
            // Even shares: no piece holds more than one level over another of its category.
            let widths: Vec<i32> = of_category(cat).map(|p| { let (a, b) = levels(p); b - a + 1 }).collect();
            assert!(widths.iter().max().unwrap() - widths.iter().min().unwrap() <= 1, "category {cat}: {widths:?}");
            assert_eq!(widths.iter().sum::<i32>(), DROP_HIGH - DROP_LOW + 1);
        }
        // 도끼's six: 85..89 · 90..93 · 94..97 · 98..102 · 103..106 · 107..110.
        let axe: Vec<(i32, i32)> = of_category(0).map(levels).collect();
        assert_eq!(axe, [(85, 89), (90, 93), (94, 97), (98, 102), (103, 106), (107, 110)]);
        assert_eq!((at_level(0, 0).unwrap().id, at_level(0, 127).unwrap().id), (41, 46), "outside the span: clamped");
        assert_eq!(at_level(16, 100).map(|l| l.id), None, "not an equipment category");
        // A 가디언 슬라임 (LV 85..87) and a 투기장 9층 아이스트롤 (108..110) share nothing.
        let slime = pool(EVERY, 86);
        let troll = pool(EVERY, 109);
        assert_eq!((slime.len(), troll.len()), (16, 16), "one piece a category");
        assert!(slime.iter().all(|s| troll.iter().all(|t| (s.category, s.id) != (t.category, t.id))));
        assert_eq!(slime[0].name, "선풍의 도끼");
        assert_eq!(troll[0].name, "외침의 도끼");
        assert_eq!(pool(slot_categories(&MUMMY), 93).len(), 10);
        assert!(pool(0, 100).is_empty());
    }

    #[test]
    fn every_piece_has_monsters_from_85_to_110() {
        // Every piece drops at some level of the span, and a monster at every level drops one of every category —
        // the kinds' 가디언 levels cover 85..110 (tools_re/hz_drops.py names them).
        for p in &ALL {
            let (a, b) = levels(p);
            assert!(DROP_LOW <= a && a <= b && b <= DROP_HIGH, "{}", p.name);
        }
        for l in DROP_LOW..=DROP_HIGH {
            assert_eq!(pool(EVERY, l).len(), 16, "LV {l}");
        }
    }

    #[test]
    fn recipes_follow_the_monsters_pieces_and_the_box_keeps_to_the_games() {
        let slime = pool(EVERY, 86);
        assert_eq!(recipe_roll(24, 1, &slime), Some(28), "선풍의 도끼's");
        assert!(recipe_roll(25, 1, &slime).is_none() && recipe_roll(25, 2, &slime).is_some() && recipe_roll(50, 2, &slime).is_none());
        for r in 0..2_000 {
            if let Some(id) = recipe_roll(r * 1000 + 3, 1, &slime) {
                let makes = recipe(id).unwrap().makes;
                assert!(slime.iter().any(|p| (p.category, p.id) == (makes.category, makes.id)), "{}", makes.name);
            }
        }
        // The drop-only pieces (폭발의 반지 · 포수의 장화) have no recipe: a monster whose pool is only them drops none.
        let blast = find(11, 49).unwrap();
        assert_eq!((recipe_of(blast), recipe_of(find(15, 47).unwrap())), (None, None));
        assert_eq!(recipe_roll(0, 1, &[blast]), None);
        assert_eq!(recipe_of(find(10, 48).unwrap()), Some(127), "the hundredth piece's recipe is 127");
        let top = pool(EVERY, 110);
        assert!(top.contains(&blast), "the 장신구 at the top is 폭발의 반지");
        let picked: std::collections::BTreeSet<i8> = (0..5_000).filter_map(|r| recipe_roll(r * 1000, 1, &top)).collect();
        assert_eq!(picked.len(), 14, "16 pieces, two without a recipe");
        assert_eq!((box_recipe(0), box_recipe(111), box_recipe(112), box_recipe(182)), (0, 27, 0, 14));
        assert!((0..183).all(|n| (0..28).contains(&box_recipe(n))));
    }

    #[test]
    fn a_kill_draws_a_piece_of_its_pool_one_in_40() {
        let slime = pool(EVERY, 86);
        assert_eq!(roll(24, false, 1, &slime).map(|l| l.name), Some("선풍의 도끼"));
        assert!(roll(25, false, 1, &slime).is_none() && roll(25, true, 1, &slime).is_some() && roll(50, true, 1, &slime).is_none());
        assert!(roll(25, false, 2, &slime).is_some() && roll(49, false, 2, &slime).is_some() && roll(50, false, 2, &slime).is_none(), "행운의 반지 doubles");
        assert!(roll(99, true, 2, &slime).is_some() && roll(100, true, 2, &slime).is_none());
        assert_eq!(roll(0, false, 1, &[]), None, "돌기둥 drops none");
        let top = pool(EVERY, 110);
        assert_eq!(roll(15_000 + 3, false, 1, &top).map(|l| l.name), Some("포수의 장화"), "the drop-only ones too");
        let hits: Vec<&Legend> = (0..100_000).filter_map(|r| roll(r * 7919, false, 1, &slime)).collect();
        assert!((2_200..2_800).contains(&hits.len()), "{}", hits.len());
        assert!(hits.iter().all(|h| slime.contains(h)), "only the monster's own");
        let kinds: std::collections::BTreeSet<(i8, i8)> = hits.iter().map(|h| (h.category, h.id)).collect();
        assert_eq!(kinds.len(), 16, "and every one of them");
        assert_eq!(boss_pick(7, &top).map(|p| p.category), Some(0));
        assert_eq!(boss_pick(1_500, &top).map(|p| p.name), Some("포수의 장화"));
        assert_eq!(boss_pick(7, &[]), None);
    }

    #[test]
    fn the_second_draft_goes_on_from_the_first() {
        assert_eq!(ALL.len(), 36 + 60 + 6);
        assert_eq!(ALL[36].name, "전장의 도끼");
        assert_eq!(find(0, 42).map(|l| l.effect), Some(StrikeTwice));
        assert_eq!(find(15, 46).map(|l| l.name), Some("기사의 장화"));
        assert_eq!((find(11, 49).map(|l| l.name), find(10, 48).map(|l| l.effect)), (Some("폭발의 반지"), Some(ShadowHelm)));
        // At least six a category, every category (투구 8 · 장신구 9 · 중신발 7 since the 2nd pass).
        for cat in 0..hz_patches::hooks::GEAR_CATEGORIES {
            let n = ALL.iter().filter(|l| l.category == cat).count();
            let want = match cat { 10 => 8, 11 => 9, 15 => 7, _ => 6 };
            assert_eq!(n, want, "category {cat}");
        }
        assert!(ALL.iter().all(|l| l.id < hz_patches::hooks::GROUND_TOP_ID), "the ground list takes every number");
    }
}
