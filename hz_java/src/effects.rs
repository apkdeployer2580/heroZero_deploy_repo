//! HZ-032 — what the worn 85제 pieces ([`crate::legend`]) do. The pure parts: which effects apply to
//! which job and skill, and by how much; the JVM side is `hooks`, which reads the worn pieces, the
//! player's job, action and states, and calls these. Nothing here is saved — effects come from what is
//! worn, every time (HEROZERO_NEW_ITEM_DRAFT.md §1).
//!
//! Skills are the game's action numbers (`ek.ac[ek.l]`): 0..3 the basic combo, 4..9 the job's six
//! actives in `t/s<job>.tdf` order; the job is `ed.b:B` (javap 2026-10-28, HEROZERO_NEW_ITEM_DRAFT.md §3).
//! The second draft (2026-10-29, 60 more pieces — §9) adds conditions the same hooks can read: the
//! player's HP and SP ([`Moment::hp_pct`], [`Moment::sp_full`]) and guard windows a cast or a kill opens.

use crate::legend::Effect::{self, *};

/// The rotation bench (`tools/hz_rotation.py`): when set, the hit, taken and kill hooks log every call at
/// info level (`hz: bench …`). The probe sets it from `HZ_BENCH`; the app never does.
pub static BENCH: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the bench logging is on.
pub fn bench() -> bool {
    BENCH.load(std::sync::atomic::Ordering::Relaxed)
}

/// `ed.b:B`, the job.
pub const WARRIOR: i8 = 0;
pub const ROGUE: i8 = 1;
pub const KNIGHT: i8 = 2;
pub const GUNNER: i8 = 3;
pub const BAYONET: i8 = 4;
pub const SPIRIT: i8 = 5;

/// The actions the pieces name, by job (HEROZERO_BALANCE_PLAN.md §2.3).
pub const STRIKE: i8 = 4; // 워리어 강타
pub const SHOCK: i8 = 5; // 워리어 충격
pub const CHARGE: i8 = 6; // 워리어 돌격
pub const SHOUT: i8 = 7; // 워리어 외침
pub const WHIRLWIND: i8 = 9; // 워리어 회전베기
pub const AMBUSH: i8 = 4; // 로그 기습
pub const COMBO: i8 = 6; // 로그 연격
pub const DASH: i8 = 7; // 로그 신속
pub const SLASH: i8 = 8; // 로그 참격
pub const BASH: i8 = 4; // 나이트 방패치기
pub const HEAL: i8 = 5; // 나이트 회복
pub const SHIELD_CHARGE: i8 = 6; // 나이트 방패 돌격
pub const COUNTER: i8 = 7; // 나이트 반격
pub const PUSH: i8 = 8; // 나이트 방패 밀기
pub const FULL_GUARD: i8 = 9; // 나이트 전력방어
pub const POINT_BLANK: i8 = 4; // 건너 근접사격
pub const AIM: i8 = 5; // 건너 조준사격
pub const VOLLEY: i8 = 6; // 건너 연사
pub const SPRAY: i8 = 7; // 건너 난사
pub const TRICK: i8 = 8; // 건너 트릭샷
pub const SNIPE: i8 = 9; // 건너 저격
pub const BARRAGE: i8 = 4; // 총검사 포격
pub const RUSH: i8 = 5; // 총검사 돌진
/// 돌진's second stage: reaching a monster, 돌진 queues action 6 (`av.a(BI)Z` @367 `ek.f(6)`), and that stage deals
/// the blast (@392 `db.a(5, false)` — 돌진's own table — then the front hit with action 6). A hit of 돌진 is a hit of 6
/// (the item test, 2026-10-30: 20 돌진 casts, every hit action 6, none 5).
pub const RUSH_BLAST: i8 = 6;
pub const FUSION: i8 = 5; // 정령사 동화

/// Whether `action` is one of the job's skills (not the basic combo).
pub fn is_skill(action: i8) -> bool {
    (4..=9).contains(&action)
}

/// The effects of the worn pieces (one per piece; the same piece can be worn once per slot).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Worn(pub Vec<Effect>);

impl Worn {
    pub fn has(&self, e: Effect) -> bool {
        self.0.contains(&e)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// What the player is doing and has up, as the hooks read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Moment {
    pub job: i8,
    pub action: i8,
    /// 동화 (`ek.b:I > 0`).
    pub transformed: bool,
    /// 분신 stands (`ed.a:Lh` set).
    pub clone: bool,
    /// 반격 is up (`ek.g:Z`).
    pub counter: bool,
    /// Within [`VOLLEY_GUARD_FRAMES`] of a 연사.
    pub volley_guard: bool,
    /// Within [`SKILL_GUARD_FRAMES`] of a skill's start (강철의 장갑).
    pub skill_guard: bool,
    /// Within [`KILL_GUARD_FRAMES`] of a kill (불굴의 갑주).
    pub kill_guard: bool,
    /// Equipment in the bag (worn pieces are in it too, `bx.i` ≥ 0).
    pub gear_count: i32,
    /// 의지 (`ek.c:[I[3]`) — 정령사의 반지 adds half of it to a hit.
    pub wil: i32,
    /// Within [`AMBUSH_FRAMES`] of a 기습 (암살자의 반지).
    pub ambush: bool,
    /// HP now as a percent of the top (`ek.a:[I[0]` / `ek.b:[I[0]`); 100 when the top is unknown.
    pub hp_pct: i32,
    /// SP is at its top.
    pub sp_full: bool,
}

impl Default for Moment {
    fn default() -> Self {
        Moment {
            job: 0,
            action: 0,
            transformed: false,
            clone: false,
            counter: false,
            volley_guard: false,
            skill_guard: false,
            kill_guard: false,
            gear_count: 0,
            wil: 0,
            ambush: false,
            hp_pct: 100,
            sp_full: false,
        }
    }
}

/// The HP thresholds the second draft's conditions use, percent of the top.
pub const LOW_HP: i32 = 40;
pub const VERY_LOW_HP: i32 = 30;
pub const HALF_HP: i32 = 50;
pub const HIGH_HP: i32 = 70;

/// Damage dealt, as a percent of the game's (every hit — the basic combo and the job's skills — after
/// the monster's defence; the game's critical doubles it after this).
pub fn hit_pct(w: &Worn, m: &Moment) -> i32 {
    let mut pct = 100;
    let skill = |job: i8, action: i8| m.job == job && m.action == action;
    let basic = !is_skill(m.action);
    // The first draft.
    if w.has(SmashWide) && skill(WARRIOR, STRIKE) {
        pct += 40;
    }
    if w.has(ChargeBoost) && skill(WARRIOR, CHARGE) {
        pct += 60;
    }
    if w.has(AimBoost) && skill(GUNNER, AIM) {
        pct += 60;
    }
    if w.has(BarrageBoost) && skill(BAYONET, BARRAGE) {
        pct += 50;
    }
    if w.has(ChainSkill) && is_skill(m.action) {
        pct += 20;
    }
    if w.has(CloneLong) && m.clone {
        pct += 35;
    }
    if w.has(FusionLong) {
        pct += if m.transformed { 40 } else { 15 };
    }
    if w.has(Gravity) {
        pct += m.gear_count.max(0) / 2;
    }
    // The second draft: a skill's own power.
    if w.has(ShockBoost) && skill(WARRIOR, SHOCK) {
        pct += 80;
    }
    if w.has(WhirlwindPower) && skill(WARRIOR, WHIRLWIND) {
        pct += 60;
    }
    if w.has(AmbushBoost) && skill(ROGUE, AMBUSH) {
        pct += 80;
    }
    if w.has(BashBoost) && skill(KNIGHT, BASH) {
        pct += 60;
    }
    if w.has(ShieldChargeBoost) && skill(KNIGHT, SHIELD_CHARGE) {
        pct += 80;
    }
    if w.has(PushBoost) && skill(KNIGHT, PUSH) {
        pct += 80;
    }
    if w.has(PointBlankBoost) && skill(GUNNER, POINT_BLANK) {
        pct += 80;
    }
    if w.has(GunnerGlove) && skill(GUNNER, VOLLEY) {
        pct += 40;
    }
    if w.has(VolleyBoost) && skill(GUNNER, VOLLEY) {
        pct += 30;
    }
    if w.has(RushBoost) && (skill(BAYONET, RUSH) || skill(BAYONET, RUSH_BLAST)) {
        pct += 60;
    }
    if w.has(BasicPower) && basic {
        pct += 30;
    }
    // The second draft: the player's state.
    if w.has(CounterStrike) && m.counter {
        pct += 40;
    }
    if w.has(LowHpRage) && m.hp_pct < LOW_HP {
        pct += 50;
    }
    if w.has(HighHpRage) && m.hp_pct >= HIGH_HP {
        pct += 25;
    }
    if w.has(FullSpRage) && m.sp_full {
        pct += 30;
    }
    if w.has(AssassinRing) && m.ambush {
        pct += 40;
    }
    pct
}

/// Damage added flat to a hit after the percent (정령사의 반지: half of 의지).
pub fn hit_flat(w: &Worn, m: &Moment) -> i32 {
    if w.has(SpiritRing) { m.wil.max(0) / 2 } else { 0 }
}

/// Damage taken, as a percent of the game's — the reductions add, and never below a tenth.
pub fn taken_pct(w: &Worn, m: &Moment) -> i32 {
    let mut cut = 0;
    if w.has(CloneGuard) && m.clone {
        cut += 30;
    }
    if w.has(CounterGuard) && m.counter {
        cut += 25;
    }
    if w.has(VolleyGuard) && m.volley_guard {
        cut += 40;
    }
    if w.has(BayonetGuard) {
        cut += 15;
    }
    if w.has(FusionGuard) {
        cut += if m.transformed { 20 } else { 10 };
    }
    if w.has(IronWall) {
        cut += 20;
    }
    if w.has(MountainMace) {
        cut += 10;
    }
    if w.has(Endurance) {
        cut += 10;
    }
    if w.has(LowHpGuard) && m.hp_pct < VERY_LOW_HP {
        cut += 50;
    }
    if w.has(HighHpGuard) && m.hp_pct >= HIGH_HP {
        cut += 25;
    }
    if w.has(SkillGuard) && m.skill_guard {
        cut += 30;
    }
    if w.has(KillGuard) && m.kill_guard {
        cut += 30;
    }
    (100 - cut).max(10)
}

/// A skill's SP, as a percent of the game's (`m.action` is the skill).
pub fn sp_pct(w: &Worn, m: &Moment) -> i32 {
    let mut pct = 100;
    let skill = |job: i8, action: i8| m.job == job && m.action == action;
    if w.has(FocusSp) && is_skill(m.action) {
        pct -= 20;
    }
    if w.has(DashCrit) && skill(ROGUE, DASH) {
        pct -= 50;
    }
    if w.has(AimBoost) && skill(GUNNER, AIM) {
        pct -= 50;
    }
    if w.has(ShoutBoost) && skill(WARRIOR, SHOUT) {
        pct -= 50;
    }
    if w.has(WhirlwindCheap) && skill(WARRIOR, WHIRLWIND) {
        pct -= 50;
    }
    if w.has(HealCheap) && skill(KNIGHT, HEAL) {
        pct -= 60;
    }
    if w.has(PushBoost) && skill(KNIGHT, PUSH) {
        pct -= 50;
    }
    if w.has(CounterCheap) && skill(KNIGHT, COUNTER) {
        pct -= 50;
    }
    if w.has(BashBoost) && skill(KNIGHT, BASH) {
        pct -= 50;
    }
    if w.has(FullGuardCheap) && skill(KNIGHT, FULL_GUARD) {
        pct -= 50;
    }
    if w.has(SnipeCheap) && skill(GUNNER, SNIPE) {
        pct -= 60;
    }
    if w.has(VolleyBoost) && skill(GUNNER, VOLLEY) {
        pct -= 50;
    }
    if w.has(BarrageCheap) && skill(BAYONET, BARRAGE) {
        pct -= 50;
    }
    if w.has(RushCheap) && skill(BAYONET, RUSH) {
        pct -= 50;
    }
    if w.has(FusionCheap) && skill(SPIRIT, FUSION) {
        pct -= 50;
    }
    if w.has(LowHpCheap) && is_skill(m.action) && m.hp_pct < HALF_HP {
        pct -= 50;
    }
    pct.max(0)
}

/// A job skill's cooldown, as a percent of the game's.
pub fn job_cooldown_pct(w: &Worn, job: i8, action: i8) -> i32 {
    let mut pct = 100;
    let skill = |j: i8, a: i8| job == j && action == a;
    if w.has(WhirlwindCooldown) && skill(WARRIOR, WHIRLWIND) {
        pct -= 50;
    }
    if w.has(ChargeBoost) && skill(WARRIOR, CHARGE) {
        pct -= 50;
    }
    if w.has(BarrageBoost) && skill(BAYONET, BARRAGE) {
        pct -= 30;
    }
    if w.has(TimeCloak) {
        pct -= 15;
    }
    if w.has(ShockBoost) && skill(WARRIOR, SHOCK) {
        pct -= 50;
    }
    if w.has(ShoutBoost) && skill(WARRIOR, SHOUT) {
        pct -= 60;
    }
    if w.has(AmbushBoost) && skill(ROGUE, AMBUSH) {
        pct -= 50;
    }
    if w.has(ShieldChargeBoost) && skill(KNIGHT, SHIELD_CHARGE) {
        pct -= 50;
    }
    if w.has(CounterCheap) && skill(KNIGHT, COUNTER) {
        pct -= 50;
    }
    if w.has(FullGuardCheap) && skill(KNIGHT, FULL_GUARD) {
        pct -= 50;
    }
    if w.has(PointBlankBoost) && skill(GUNNER, POINT_BLANK) {
        pct -= 50;
    }
    if w.has(GunnerGlove) && skill(GUNNER, TRICK) {
        pct -= 50;
    }
    if w.has(SnipeCheap) && skill(GUNNER, SNIPE) {
        pct -= 50;
    }
    if w.has(RushBoost) && skill(BAYONET, RUSH) {
        pct -= 50;
    }
    if w.has(FusionCheap) && skill(SPIRIT, FUSION) {
        pct -= 50;
    }
    if w.has(Endurance) {
        pct -= 10;
    }
    if w.has(GunnerBoots) && skill(BAYONET, BARRAGE) {
        pct -= 30;
    }
    pct.max(10)
}

/// A spirit skill's cooldown, as a percent of the game's.
pub fn spirit_cooldown_pct(w: &Worn) -> i32 {
    let mut pct = 100;
    if w.has(SpiritPower) {
        pct -= 20;
    }
    if w.has(TimeCloak) {
        pct -= 15;
    }
    if w.has(SpiritDagger) {
        pct -= 20;
    }
    if w.has(Endurance) {
        pct -= 10;
    }
    pct.max(10)
}

/// Whether a cast of `action` is followed by a second one (SP and cooldown once).
pub fn twice(w: &Worn, job: i8, action: i8) -> bool {
    let skill = |j: i8, a: i8| job == j && action == a;
    (w.has(WhirlwindTwice) && skill(WARRIOR, WHIRLWIND))
        || (w.has(StrikeTwice) && skill(WARRIOR, STRIKE))
        || (w.has(ChargeTwice) && skill(WARRIOR, CHARGE))
        || (w.has(SlashTwice) && skill(ROGUE, SLASH))
        || (w.has(ComboTwice) && skill(ROGUE, COMBO))
        || (w.has(DashTwice) && skill(ROGUE, DASH))
        || (w.has(BashTwice) && skill(KNIGHT, BASH))
        || (w.has(ShieldChargeTwice) && skill(KNIGHT, SHIELD_CHARGE))
        || (w.has(PushTwice) && skill(KNIGHT, PUSH))
        || (w.has(SprayTwice) && skill(GUNNER, SPRAY))
        || (w.has(AimTwice) && skill(GUNNER, AIM))
        || (w.has(VolleyTwice) && skill(GUNNER, VOLLEY))
        || (w.has(RushTwice) && skill(BAYONET, RUSH))
        || (w.has(BarrageTwice) && skill(BAYONET, BARRAGE))
}

/// Points on the critical chance the game rolled for a hit (the game's own is at most 70 before the
/// level difference; the hook's forced critical, R3, is 100 and outranks this).
pub fn crit_plus(w: &Worn, m: &Moment) -> i32 {
    let mut plus = 0;
    if w.has(CritPlus15) {
        plus += 15;
    }
    if w.has(CritGlove) {
        plus += 20;
    }
    if w.has(BasicCrit) && !is_skill(m.action) {
        plus += 25;
    }
    if w.has(LowHpCrit) && m.hp_pct < LOW_HP {
        plus += 40;
    }
    if w.has(GhostCloak) && m.clone {
        plus += 30;
    }
    if w.has(ShadowHelm) && m.clone {
        plus += 20;
    }
    plus
}

/// C1: basic hits per second taken off the cooldowns.
pub const THIEF_HITS: u32 = 10;
/// One second in the cooldowns' field frames (`ed.a:[I`, −1 a field tick at the game's 20 fps).
pub const SECOND: i32 = 20;
/// C2: SP a basic hit gives.
pub const FLOW_SP: i32 = 2;
/// S2: 동화 frames a hit adds while transformed (0.25 s), S3: a kill (1 s); never past [`FUSION_TOP`].
pub const FUSION_HIT: i32 = 5;
pub const FUSION_KILL: i32 = SECOND;
pub const FUSION_TOP: i32 = 60 * SECOND;
/// W4: SP a kill gives, percent of the top (2nd pass: 6, was 8).
pub const FRENZY_SP_PCT: i32 = 6;
/// G5: how long the guard after 연사 lasts, in drawn frames (2 s).
pub const VOLLEY_GUARD_FRAMES: u64 = 40;
/// S4: spirit skills' MP, percent off.
pub const SPIRIT_MANA_CUT: i32 = 30;
/// S5: spirit skills' power, percent (2nd pass: 180, was 150).
pub const SPIRIT_POWER_PCT: i32 = 180;
/// R4: 독수's poison, percent (2nd pass: ×3, was ×2).
pub const POISON_PCT: i32 = 300;
/// 암살자의 반지: the window after a 기습, drawn frames (3 s).
pub const AMBUSH_FRAMES: u64 = 60;

// The second draft's numbers.
/// 피의 도끼: HP back from a basic hit, percent of the damage.
pub const BASIC_LEECH_PCT: i32 = 5;
/// 정령의 단검: MP a basic hit gives (2nd pass: 5, was 3).
pub const BASIC_MANA: i32 = 5;
/// 처형의 대검: SP a skill hit gives.
pub const SKILL_HIT_SP: i32 = 5;
/// 흡혈 총검: HP a skill hit gives, percent of the top.
pub const SKILL_HEAL_PCT: i32 = 3;
/// 생명의 반지: HP a kill gives, percent of the top.
pub const KILL_HEAL_PCT: i32 = 15;
/// 현자의 투구: MP a kill gives, percent of the top.
pub const KILL_MANA_PCT: i32 = 10;
/// 맥동의 철퇴: frames a kill takes off every cooling skill (2 s).
pub const KILL_COOLDOWN: i32 = 2 * SECOND;
/// 강철의 장갑: the guard after a skill's start, drawn frames (2 s); 불굴의 갑주: after a kill (3 s).
pub const SKILL_GUARD_FRAMES: u64 = 40;
pub const KILL_GUARD_FRAMES: u64 = 60;
/// 생명의 방패: SP from a hit taken, percent of the damage.
pub const DAMAGE_TO_SP_PCT: i32 = 20;
/// 행운의 반지: the 85제 and recipe drop chances, times this.
pub const LUCK: u32 = 2;

/// HZ-067 모든 피해 흡혈 (user 2026-11-04): percent of the HP a hit really takes off a monster that comes back, for
/// each of [`DRAIN_PIECES`] worn — they add, and a piece in both of a 로그's or 건너's hands is one piece, like every
/// other effect.
pub const DRAIN_PCT: i32 = 2;
/// The pieces that drain, on top of what each already does: a job's own piece outside its main set
/// (HEROZERO_NEW_ITEM_DRAFT.md §12), so wearing one gives something up, and 생명의 반지 for all six.
pub const DRAIN_PIECES: [Effect; 7] = [BasicLeech, CounterStrike, AmbushBoost, PointBlankBoost, SkillHeal, FusionCheap, LifeRing];

/// What the worn pieces give back of a hit, percent.
pub fn drain_pct(w: &Worn) -> i32 {
    DRAIN_PIECES.iter().filter(|e| w.has(**e)).count() as i32 * DRAIN_PCT
}

/// The HP `removed` off a monster gives back at `pct`: at least 1 for a hit that took anything.
pub fn drain_heal(removed: i32, pct: i32) -> i32 {
    if removed > 0 && pct > 0 { scale(removed, pct).max(1) } else { 0 }
}

/// `v × pct / 100`, the JVM's int, saturating.
pub fn scale(v: i32, pct: i32) -> i32 {
    (i64::from(v) * i64::from(pct) / 100).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// K2: 반격's reflection, percent; K4: what of it comes back as HP.
pub const COUNTER_PCT: i32 = 180;
pub const COUNTER_LEECH_PCT: i32 = 6;

/// 반격's reflection, percent of the game's (K2 심판의 검 +80, 산의 철퇴 +40).
pub fn counter_pct(w: &Worn) -> i32 {
    let mut pct = 100;
    if w.has(CounterPower) {
        pct += COUNTER_PCT - 100;
    }
    if w.has(MountainMace) {
        pct += 40;
    }
    pct
}

/// K1 응징의 방패's 반격: a length no fight reaches (2³¹ drawn frames ≈ 3.4 years at 20 fps). Not −1: the
/// counter's status is an `aq`, whose tick `aq.a()V` ends it once `bc.b:I` (frames drawn) ≥ `bc.c:I` with no
/// "−1 = forever" case (only the base `bc.a()V` has one — javap 2026-11-03). With −1 the status ended on its first
/// frame, `ek.c(Lbc;)V` (status 20) put `ek.g:Z` back to false, and `ek.a(Lar;IB)Z` @79 never reflected (probe: 0
/// reflects in 30 s of 반격 against 16 for 성채의 철퇴). The game still wipes it with every buff on the maps that
/// reset them (`ed.a()V` case 2 → `ek.b(true)`: a map with no monsters, or a boss).
pub const COUNTER_FOREVER: i32 = i32::MAX;

/// 반격's duration in drawn frames (`aq`'s length; the game's 100 = 5 s): never ending with K1, twice with 성채의 철퇴.
pub fn counter_cycles(w: &Worn, cycles: i32) -> i32 {
    if w.has(CounterForever) {
        COUNTER_FOREVER
    } else if w.has(CounterLong) {
        cycles.saturating_mul(2)
    } else {
        cycles
    }
}

/// 분신's life: × 2 with R1, half again with 유령의 외투.
pub fn clone_life(w: &Worn, life: i16) -> i16 {
    let mut life = i32::from(life);
    if w.has(CloneLong) {
        life *= 2;
    }
    if w.has(GhostCloak) {
        life = life * 3 / 2;
    }
    life.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

/// A spirit skill's power, percent of the game's (S5 +50, 정령의 단검 +30).
pub fn spirit_power_pct(w: &Worn) -> i32 {
    let mut pct = 100;
    if w.has(SpiritPower) {
        pct += SPIRIT_POWER_PCT - 100;
    }
    if w.has(SpiritDagger) {
        pct += 30;
    }
    if w.has(OathCrown) {
        pct += 30;
    }
    pct
}

/// What of a spirit skill's MP the pieces take off, percent (S4 30, 현자의 투구 20, 서약의 관 10).
pub fn spirit_mana_cut_pct(w: &Worn) -> i32 {
    let mut cut = 0;
    if w.has(SpiritMana) {
        cut += SPIRIT_MANA_CUT;
    }
    if w.has(SageHelm) {
        cut += 20;
    }
    if w.has(OathCrown) {
        cut += 10;
    }
    cut.min(100)
}

/// B3 · 폭발의 반지: damage more on a knocked-back monster, percent of the hit.
pub fn blast_pct(w: &Worn) -> i32 {
    let mut pct = 0;
    if w.has(BlastBoost) {
        pct += BLAST_PCT;
    }
    if w.has(BlastRing) {
        pct += 50;
    }
    pct
}

/// The 85제 and recipe drop chances' multiplier (행운의 반지).
pub fn luck(w: &Worn) -> u32 {
    if w.has(Lucky) { LUCK } else { 1 }
}

/// How fast the player walks now: [`Walk::Fast`] (×1.5 — 질풍의 신발 always, 술꾼의 장화 within its window),
/// [`Walk::Brisk`] (×1.2 — 술사의 신발, not while transformed: 동화 walks 16 px a frame of its own, past the 8 the hook
/// speeds, and the text says 「동화 중에는 제외」), or the game's.
pub fn fast_walk(w: &Worn, transformed: bool, drunk: bool) -> Walk {
    if (w.has(Drunkard) && drunk) || w.has(SwiftBoots) {
        Walk::Fast
    } else if w.has(FusionSwift) && !transformed {
        Walk::Brisk
    } else {
        Walk::Game
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Walk {
    Game,
    Brisk,
    Fast,
}

/// [`Walk::Brisk`]'s step from a tile: 16, 8, 8 in turn (with the game's own 8 after each 8: three tiles in
/// five frames, ×1.2).
pub fn brisk_step(live: &mut Live) -> i32 {
    live.drunk_steps = (live.drunk_steps + 1) % 3;
    if live.drunk_steps == 1 { 16 } else { 8 }
}

/// R4 · B3: points on 독수's and 폭발탄's chances.
pub const CHANCE_PLUS: i32 = 30;
/// B3: damage more on a knocked-back monster, percent of the hit.
pub const BLAST_PCT: i32 = 30;
/// K5: the stun 방패치기 leaves (`cq.c(I)V` — 방패돌격's own 30).
pub const BASH_STUN: i32 = 30;
/// F2: the walking step it speeds (`ar.f:B` — 4 slowed, 8 walking, 16 the game's own fast) and how long,
/// in drawn frames (10 s).
pub const WALK_STEP: i8 = 8;
pub const DRUNK_FRAMES: u64 = 200;

/// F2: the step from a tile-aligned position — 16, 16, 8 in turn (three tiles in four frames, ×1.5; a
/// step of 8 leaves the walker mid-tile, and the game's own 8 brings it back), every tile still entered
/// through the game's passability test.
pub fn drunk_step(live: &mut Live) -> i32 {
    live.drunk_steps = (live.drunk_steps + 1) % 3;
    if live.drunk_steps == 0 { 8 } else { 16 }
}

/// How long a queued second cast stays due, in drawn frames (3 s; the first cast's animation is
/// under a second).
pub const REPLAY_FRAMES: u64 = 60;

/// The host's memory between hooks (nothing of it is saved).
#[derive(Clone, Debug, Default)]
pub struct Live {
    /// C1: basic hits since the last second taken off.
    pub basic_hits: u32,
    /// R3: the next hit is a critical.
    pub dash_crit: bool,
    /// G5: the drawn frame the guard after 연사 ends.
    pub volley_until: u64,
    /// 강철의 장갑: the drawn frame the guard after a skill's start ends.
    pub skill_guard_until: u64,
    /// 불굴의 갑주: the drawn frame the guard after a kill ends.
    pub kill_guard_until: u64,
    /// 암살자의 반지: the drawn frame the window after a 기습 ends.
    pub ambush_until: u64,
    /// The action whose second cast is queued and the drawn frame it was (its SP is 0, its start
    /// clears this; a replay the game dropped — the player hit — lapses after [`REPLAY_FRAMES`]).
    pub replay: Option<(i8, u64)>,
    /// F1: equipment counted, and the drawn frame it was.
    pub gear_count: (i32, u64),
    /// F2: the drawn frame the speed ends, and the steps taken in its 16 · 16 · 8 turn.
    pub drunk_until: u64,
    pub drunk_steps: u8,
}

impl Live {
    /// Whether a start of `action` at drawn frame `now` is the queued second cast (a lapsed one is
    /// forgotten).
    pub fn replay_due(&mut self, action: i8, now: u64) -> bool {
        match self.replay {
            Some((a, at)) if now < at + REPLAY_FRAMES => a == action,
            Some(_) => {
                self.replay = None;
                false
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worn(e: &[Effect]) -> Worn {
        Worn(e.to_vec())
    }

    #[test]
    fn draining_pieces_add_and_a_doubled_piece_is_one() {
        assert_eq!(drain_pct(&worn(&[])), 0);
        assert_eq!(drain_pct(&worn(&[WhirlwindTwice, TimeThief])), 0, "the other pieces do not drain");
        assert_eq!(drain_pct(&worn(&[LifeRing])), DRAIN_PCT);
        assert_eq!(drain_pct(&worn(&[BasicLeech, LifeRing])), 2 * DRAIN_PCT);
        assert_eq!(drain_pct(&worn(&[AmbushBoost, AmbushBoost])), DRAIN_PCT, "a 로그's two daggers");
        assert_eq!(drain_pct(&worn(&DRAIN_PIECES)), 7 * DRAIN_PCT);
    }

    #[test]
    fn a_drain_heals_by_what_came_off() {
        assert_eq!(drain_heal(2_500, 2), 50);
        assert_eq!(drain_heal(10, 2), 1, "a small hit still gives 1");
        assert_eq!(drain_heal(0, 2), 0, "a hit that took nothing gives nothing");
        assert_eq!(drain_heal(2_500, 0), 0, "nothing worn");
    }

    #[test]
    fn every_draining_piece_says_so() {
        for piece in crate::legend::ALL.iter() {
            let says = piece.text.iter().any(|t| *t == "모든 피해의 2% HP 회복");
            assert_eq!(says, DRAIN_PIECES.contains(&piece.effect), "{}", piece.name);
        }
        assert_eq!(DRAIN_PCT, 2, "the text's number");
    }

    fn all_up() -> Moment {
        Moment { transformed: true, clone: true, counter: true, volley_guard: true, skill_guard: true, kill_guard: true, gear_count: 40, hp_pct: 20, sp_full: true, ..Default::default() }
    }

    #[test]
    fn nothing_worn_is_the_game() {
        let w = Worn::default();
        for job in 0..6 {
            for action in 0..10 {
                for m in [Moment { job, action, ..all_up() }, Moment { job, action, hp_pct: 100, ..all_up() }] {
                    assert_eq!((hit_pct(&w, &m), taken_pct(&w, &m), sp_pct(&w, &m), crit_plus(&w, &m)), (100, 100, 100, 0));
                    assert_eq!((job_cooldown_pct(&w, job, action), spirit_cooldown_pct(&w), twice(&w, job, action)), (100, 100, false));
                }
            }
        }
        assert_eq!((counter_pct(&w), counter_cycles(&w, 100), clone_life(&w, 200), spirit_power_pct(&w), spirit_mana_cut_pct(&w), luck(&w)), (100, 100, 200, 100, 0, 1));
        assert_eq!((fast_walk(&w, true, true), blast_pct(&w), hit_flat(&w, &Moment { wil: 200, ..Default::default() })), (Walk::Game, 0, 0));
    }

    #[test]
    fn skill_pieces_touch_their_skill_only() {
        let w = worn(&[ChargeBoost]);
        let at = |job, action| hit_pct(&w, &Moment { job, action, ..Default::default() });
        assert_eq!((at(WARRIOR, CHARGE), at(WARRIOR, STRIKE), at(ROGUE, CHARGE)), (160, 100, 100));
        assert_eq!((job_cooldown_pct(&w, WARRIOR, CHARGE), job_cooldown_pct(&w, WARRIOR, STRIKE)), (50, 100));
        let w = worn(&[WhirlwindTwice, SlashTwice]);
        assert!(twice(&w, WARRIOR, WHIRLWIND) && twice(&w, ROGUE, SLASH));
        assert!(!twice(&w, ROGUE, WHIRLWIND) && !twice(&w, WARRIOR, SLASH));
        // The second draft's doubles, one job each.
        let w = worn(&[StrikeTwice, BashTwice, AimTwice, BarrageTwice, ComboTwice]);
        assert!(twice(&w, WARRIOR, STRIKE) && twice(&w, KNIGHT, BASH) && twice(&w, GUNNER, AIM) && twice(&w, BAYONET, BARRAGE) && twice(&w, ROGUE, COMBO));
        // The same action number is another job's other skill (4 = 강타 · 기습 · 방패치기 · 근접사격 · 포격 · 맹약).
        assert!(!twice(&w, ROGUE, STRIKE) && !twice(&w, SPIRIT, BASH) && !twice(&w, GUNNER, BARRAGE) && !twice(&w, WARRIOR, COMBO));
        let w = worn(&[PushBoost]);
        let m = Moment { job: KNIGHT, action: PUSH, ..Default::default() };
        assert_eq!((hit_pct(&w, &m), sp_pct(&w, &m)), (180, 50));
        assert_eq!(sp_pct(&w, &Moment { job: WARRIOR, ..m }), 100);
        // 돌진's blast lands as its second stage, action 6; the cast, its SP and cooldown are 5's.
        let w = worn(&[RushBoost]);
        let at = |action| hit_pct(&w, &Moment { job: BAYONET, action, ..Default::default() });
        assert_eq!((at(RUSH_BLAST), at(RUSH), at(BARRAGE)), (160, 160, 100));
        assert_eq!(hit_pct(&w, &Moment { job: WARRIOR, action: RUSH_BLAST, ..Default::default() }), 100, "6 is 워리어 돌격");
        assert_eq!((job_cooldown_pct(&w, BAYONET, RUSH), job_cooldown_pct(&w, BAYONET, RUSH_BLAST)), (50, 100));
    }

    #[test]
    fn pieces_add_up() {
        let w = worn(&[ChainSkill, FusionLong, Gravity]);
        let m = Moment { job: SPIRIT, action: 5, transformed: true, gear_count: 41, ..Default::default() };
        assert_eq!(hit_pct(&w, &m), 100 + 20 + 40 + 20);
        assert_eq!(hit_pct(&w, &Moment { action: 1, ..m }), 100 + 40 + 20, "the basic combo is no job skill");
        assert_eq!(hit_pct(&w, &Moment { action: 1, transformed: false, ..m }), 100 + 15 + 20, "맹약의 단검 gives 15 untransformed");
        let w = worn(&[CloneGuard, CounterGuard, VolleyGuard, BayonetGuard, FusionGuard]);
        let m = Moment { transformed: true, clone: true, counter: true, volley_guard: true, ..Default::default() };
        assert_eq!(taken_pct(&w, &m), 10, "130 % off is still a tenth");
        assert_eq!(taken_pct(&w, &Moment::default()), 100 - 15 - 10, "영혼의 외투 cuts 10 untransformed");
        assert_eq!(sp_pct(&worn(&[FocusSp, DashCrit]), &Moment { job: ROGUE, action: DASH, ..Default::default() }), 30);
        assert_eq!(job_cooldown_pct(&worn(&[WhirlwindCooldown, TimeCloak]), WARRIOR, WHIRLWIND), 35);
        assert_eq!(spirit_cooldown_pct(&worn(&[SpiritPower, TimeCloak])), 65);
        assert_eq!(counter_pct(&worn(&[CounterPower, MountainMace])), 220);
        assert_eq!(spirit_power_pct(&worn(&[SpiritPower, SpiritDagger, OathCrown])), 100 + 80 + 30 + 30);
        assert_eq!(spirit_mana_cut_pct(&worn(&[SpiritMana, SageHelm, OathCrown])), 60);
        assert_eq!(blast_pct(&worn(&[BlastBoost, BlastRing])), 80);
        assert_eq!(hit_flat(&worn(&[SpiritRing]), &Moment { wil: 201, ..Default::default() }), 100);
        assert_eq!(hit_pct(&worn(&[AssassinRing]), &Moment { ambush: true, ..Default::default() }), 140);
        assert_eq!(crit_plus(&worn(&[ShadowHelm, GhostCloak]), &Moment { clone: true, ..Default::default() }), 50);
        assert_eq!(job_cooldown_pct(&worn(&[GunnerBoots]), BAYONET, BARRAGE), 70);
    }

    #[test]
    fn the_second_drafts_conditions_read_the_moment() {
        let w = worn(&[LowHpRage, HighHpRage, FullSpRage, CounterStrike, BasicPower]);
        let low = Moment { hp_pct: 39, sp_full: true, counter: true, ..Default::default() };
        assert_eq!(hit_pct(&w, &low), 100 + 50 + 30 + 40 + 30);
        assert_eq!(hit_pct(&w, &Moment { hp_pct: 40, sp_full: false, counter: false, action: STRIKE, ..low }), 100);
        assert_eq!(hit_pct(&w, &Moment { hp_pct: 70, ..Moment::default() }), 100 + 25 + 30);
        let w = worn(&[IronWall, LowHpGuard, HighHpGuard, SkillGuard, KillGuard, Endurance, MountainMace]);
        assert_eq!(taken_pct(&w, &Moment { hp_pct: 50, ..Default::default() }), 100 - 20 - 10 - 10);
        assert_eq!(taken_pct(&w, &Moment { hp_pct: 29, skill_guard: true, kill_guard: true, ..Default::default() }), 10);
        assert_eq!(taken_pct(&w, &Moment::default()), 100 - 20 - 10 - 10 - 25, "full HP is high HP");
        let w = worn(&[LowHpCheap, CritPlus15, CritGlove, BasicCrit, LowHpCrit, GhostCloak]);
        assert_eq!(sp_pct(&w, &Moment { action: STRIKE, hp_pct: 49, ..Default::default() }), 50);
        assert_eq!(sp_pct(&w, &Moment { action: 1, hp_pct: 49, ..Default::default() }), 100, "the combo pays its own SP");
        assert_eq!(crit_plus(&w, &Moment { action: 1, hp_pct: 39, clone: true, ..Default::default() }), 15 + 20 + 25 + 40 + 30);
        assert_eq!(crit_plus(&w, &Moment { action: STRIKE, ..Default::default() }), 15 + 20);
        assert_eq!((counter_cycles(&worn(&[CounterLong]), 100), counter_cycles(&worn(&[CounterLong, CounterForever]), 100)), (200, COUNTER_FOREVER));
        assert!(COUNTER_FOREVER > 0, "aq's tick ends a status once frames >= length: a negative length ends it at once");
        assert_eq!((clone_life(&worn(&[GhostCloak]), 200), clone_life(&worn(&[CloneLong, GhostCloak]), 200), clone_life(&worn(&[GhostCloak]), i16::MAX)), (300, 600, i16::MAX));
        assert_eq!(luck(&worn(&[Lucky])), 2);
        assert_eq!((fast_walk(&worn(&[SwiftBoots]), false, false), fast_walk(&worn(&[FusionSwift]), true, false), fast_walk(&worn(&[FusionSwift]), false, true)), (Walk::Fast, Walk::Game, Walk::Brisk));
        assert_eq!((fast_walk(&worn(&[Drunkard]), false, true), fast_walk(&worn(&[Drunkard]), true, false)), (Walk::Fast, Walk::Game));
        let mut live = Live::default();
        assert_eq!((0..6).map(|_| brisk_step(&mut live)).collect::<Vec<_>>(), [16, 8, 8, 16, 8, 8]);
        assert_eq!(job_cooldown_pct(&worn(&[Endurance, GunnerGlove]), GUNNER, TRICK), 40);
        assert_eq!(spirit_cooldown_pct(&worn(&[Endurance, SpiritDagger])), 70);
    }

    #[test]
    fn a_second_cast_is_due_only_for_its_action_and_a_while() {
        let mut live = Live { replay: Some((WHIRLWIND, 100)), ..Default::default() };
        assert!(!live.replay_due(STRIKE, 110) && live.replay_due(WHIRLWIND, 110));
        assert!(!live.replay_due(WHIRLWIND, 100 + REPLAY_FRAMES) && live.replay.is_none(), "lapsed");
    }

    #[test]
    fn the_drunk_walk_is_half_again_as_fast() {
        let mut live = Live::default();
        let steps: Vec<i32> = (0..6).map(|_| drunk_step(&mut live)).collect();
        assert_eq!(steps, [16, 16, 8, 16, 16, 8]);
        // From aligned: 16, 16, 8 (then the game's own 8 to the next tile) = 48 px in 4 frames = 12 a frame.
        assert_eq!(16 + 16 + 8 + 8, 4 * 12);
    }

    #[test]
    fn scale_is_the_jvms_int() {
        assert_eq!((scale(77, 160), scale(-90, 85), scale(i32::MAX, 200)), (123, -76, i32::MAX));
    }
}
