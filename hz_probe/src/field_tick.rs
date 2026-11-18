//! Scripted guest calls for verifying the hook patches without playing to the event that
//! triggers them.
//!
//! The probe wraps the HZ-006 hook (`Hooks.autoLoot`, which the patched field tick `ed.d()V`
//! calls once a frame on the game thread) and, at the frames the command line names, calls the
//! game's OWN methods on the live objects before handing over to the real hook:
//!
//! * `--exp-test F:N` — `ek.j(N)` on the player (`ed.a:Lek;`): the patched method, the real
//!   `Hooks.exp`, the game's own level-up loop. Prints level / EXP before and after.
//! * `--drop-money F:DX:DY:N` — `g.a(BBI)V`, the game's own "money on the ground" (what a kill
//!   drops), DX/DY tiles from the player. With `--opt loot=on` the hook then picks it up.
//! * `--drop-item F:DX:DY:CAT:ID` — `g.a(BBBB)V`, the same for an item (bag-room check path).
//! * `--drop-test F` — `Hooks.dropThreshold` on a monster record whose slot 0 holds 4833 (126 of
//!   132 monsters): the hook as the patched `cq.o()V` calls it, through the game's `rpg/g.a`.
//! * `--stat-test F:IDX:VALUE` — `ek.a(B,I)` on the player, the game's own setter of its stat
//!   array (`a[IDX] = VALUE`; 0 HP, 1 SP, 2 EXP, 3 MP). A stand-in for a monster's hit, to watch
//!   the STATS readout (`hz: stats …`, HZ-008's blit hook reading `ek.e/f(B)`) follow a drop in HP.
//!
//! * `--hit F:N` — `ek.c(−N)` on the player, the game's own "take N damage" (what a monster's
//!   blow calls): at 0 HP it puts the player in state 6 (dying), and `ek.C()` runs the death —
//!   for the death flow without a fight.
//! * `--quick-set F:SLOT:ID` — `ed.b:[B[SLOT] = ID` (the quick slots: 0..3 skill ids, 4/5 the
//!   `0`/`#` potion ids) and `ce.c = true`, the game's own "redraw the whole bar" flag: the game's
//!   bar then draws that slot the way it draws a skill the player put there, beside what the
//!   HZ-008 hook reads for the host plates (Phase 6-2) — an A/B of the icon on one frame. For the
//!   spirit slots 2 · 3 also the spirit's `p.b(slot)`, the sheet load the game's setter does.
//! * `--warp F:MAP:X:Y` — `ed.b(MAP,X,Y,0)`, the game's own "go to map" (the scenario engine's
//!   `eb.c()` calls it for a map change): it sets the map number and asks the state machine for
//!   the loading mode, which loads `/m/m/MAP.m` and puts the player on tile (X,Y). For the
//!   minimap (Phase 6-5) on maps with monsters and chests without walking there.
//! * `--equip-test F:SLOT:CAT:ID` — the item `ba.a(CAT,ID)` (the game's own factory, from
//!   `/itm/CAT`) worn in equipment slot SLOT the way the equipment window `bv.a(BB)` does it:
//!   `ek.a(SLOT, item)`, `ek.i()`, then the doll's part for the slot reloaded (`rpg/e.a(cn,
//!   ea.a[SLOT])`). The item is not taken from the bag. For the portrait (Phase 6-6): slot 1 is the
//!   head, category 10 the head gear (`bx.g` = the `/hero/0/NN` sprite).
//! * `--quest-set F:Q:STATE` — quest Q's two bits in `ed.ad:[B` (what `ed.a(I)B` reads: 0 not
//!   begun, 1 under way, 2 done) set to STATE. The game picks its guide tile when a map loads
//!   (`g.a()V`, printed as `taskX = …`), so follow it with a `--warp` to compare that tile and the
//!   world map's trail (`hz: guide quest …`) for any quest without playing to it (Phase 7-6).
//!
//! * `--skill-level F:ID:L` / `--spirit-level F:ID:L` / `--base-stat F:IDX:VALUE` — a bench build without the
//!   windows (tools/hz_rotation.py): a job skill (its action number 4..9, the passives 10·11) learned to L through
//!   the window's own `ek.a(BB)V`, a spirit skill's level `p.c:[B[ID]`, a stat (0 힘 · 1 민첩 · 2 건강 · 3 의지)
//!   made VALUE through the allocation `ek.a:[S` + `ek.i()`.
//! * `--job F:N` — the job window's change `ek.e(B)V`, the gear off and the paper doll reloaded, as `bi` does.
//! * `--refill F` — HP, SP and MP to their tops (`--exp-test` does not heal).
//! * `--item-icons F` — every picture of the item pack `rpg/e.k:[Lcw;` (`/img/it`, what `ba.a(dd,
//!   IIZZ)V` draws at `k[cat + (cat == 20 ? id : 0)]`), through the HZ-016 icon hook when it is on,
//!   written as `<out>/item_NN.png` (4x) with its size printed — for the item icons without opening
//!   the bag.
//!
//! * `--mobs F` — the living monsters on the map's draw list (`g.a:Ler` → `er.b`, the `cq`s not
//!   dying) with the stats `cq.i()V` gave them (`cq.c` kind, `n` level, `i`/`h` HP, `f` attack, `e`
//!   defence), and the respawn queue `cq.c:Vector` with each one's countdown `cq.b:S` — for HZ-017
//!   (난이도: run twice, `--opt diff=100` / `diff=200`, same frame) and HZ-018 (리젠).
//! * `--kill F` — the first living monster's own death `cq.g()V` (what its HP reaching 0 leads to:
//!   the drop, the EXP, off the map, the countdown from `eq.a:S` through `Hooks.respawnDelay`, the
//!   queue), then the countdown it got. Follow with `--mobs` to watch it come back.
//!
//! * `--item-grade F:INDEX:GRADE` — the equipment bag's item at INDEX (`ed.a(0)`'s Vector) given
//!   grade GRADE (`bx.e:B`, 0 일반 .. 3 세트) — for HZ-021's bulk sale of a grade the drops never give.
//!
//! * `--give F:CAT:ID` — the item `ba.a(CAT,ID)` put in its bag the way a pickup does (`g.a(BB)Z`:
//!   `ed.a(CAT)`, `bt.a(CAT,ID,1)` for the room, `bt.a(item, 1)`); with the bag full, in the first
//!   계정창고 page with room (`dz.a:[Lbt;`, loaded by `dz.h()` if it is not, written to `HOL.storage`
//!   by `dz.i()` — what the storage window does). For a save made with given items (2026-10-30: the
//!   85제 and the 가디언 장비 into a phone's slot 1); follow with `--save-ask` and `--rms-out`.
//! * `--bags F` — the equipment bag's and the 계정창고 pages' items as `(cat,id)`, with counts.
//! * `--item-check F` — every new piece (the 85제 `legend::ALL`, the 가디언 장비 number 40) and a few of the
//!   game's own as controls, made by the game's factory `ba.a(CAT,ID)` and worn alone in its slot on the
//!   stripped player (`ek.j(slot)` for all seven, `ek.a(slot, item)`, `ek.i()`), then taken off again: one
//!   `item-check` line each with the fields the game parsed (`bx` level · grade · set · stats · `h` · the
//!   six options after their roll), the item's own option sums `bx.a(k)`, and the player before / worn /
//!   after (stats `c`, tops `b`, attack `h`, defence `a`, the worn option sums `ek.g(k)`) —
//!   `tools/hz_item_test.py` checks them.
//!
//! Only the trigger is synthetic; everything it runs is the game's code as patched. Probe-only:
//! nothing here is in the app.

use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

/// Where `--item-icons` writes (the probe's `--out`).
pub static OUT: OnceLock<PathBuf> = OnceLock::new();

use java_class_proto::{JavaMethodProto, MethodBody};
use jvm::{ClassInstance, JavaError, JavaValue, Jvm, Result as JvmResult};
use wie_jvm_support::{WieJavaClassProto, WieJvmContext};

// The 계정창고's pages `dz.a:[Lbt;`, read from `HOL.storage` by `dz.h()` when nothing has yet — the hooks' own.
use hz_java::hooks::storage_pages;

#[derive(Clone, Copy, Debug)]
pub enum Action {
    Exp { at: u64, amount: i32 },
    Money { at: u64, dx: i32, dy: i32, amount: i32 },
    Item { at: u64, dx: i32, dy: i32, cat: i8, id: i8 },
    DropTest { at: u64 },
    Stat { at: u64, index: i8, value: i32 },
    Quick { at: u64, slot: usize, id: i8 },
    Warp { at: u64, map: u8, x: u8, y: u8 },
    Equip { at: u64, slot: i8, cat: i8, id: i8 },
    Quest { at: u64, quest: u8, state: u8 },
    Hit { at: u64, damage: i32 },
    ItemIcons { at: u64 },
    Mobs { at: u64 },
    Kill { at: u64 },
    Grade { at: u64, index: i32, grade: i8 },
    Round { at: u64, round: i8 },
    /// `--job F:N`: the player made job N (`ed.b`, the game's `ek.m(B)V` + `ek.i()V`) — for trying a job's skills.
    Job { at: u64, job: i8 },
    /// `--skill-level F:ID:L` — the player's skill ID (its action number, 4..9) raised to level L through
    /// `ek.a(BB)V` — the skill window's "learn one level" (`d[job * 8 + ID - 4] += 1`); the cap is the window's.
    SkillLevel { at: u64, skill: i8, level: i8 },
    /// `--base-stat F:IDX:VALUE` — the player's stat (0 힘 · 1 민첩 · 2 건강 · 3 의지) made VALUE: the allocation
    /// `ek.a:[S[IDX]` = VALUE − 10 (`ek.i()V` rebuilds `c:[I` as 10 + it), then `ek.i()V` — a bench's build
    /// without the stat window.
    BaseStat { at: u64, index: usize, value: i32 },
    /// `--spirit-level F:ID:L` — the spirit's skill ID (`/s/d/a` order, 0..9) at level L: `p.c:[B[ID]`
    /// (what `p.a(B)B` reads) on the player's spirit `ek.a()Lp;`.
    SpiritLevel { at: u64, skill: usize, level: i8 },
    /// `--refill F` — HP, SP and MP to their tops (`ek.a:[I` = `ek.b:[I` at 0, 1, 3): a bench starts full after
    /// `--exp-test` (which does not heal).
    Refill { at: u64 },
    /// `--walk-dir F:DIR` — the field's walk in direction DIR through the game's own `ed.b(B)V` (what
    /// the arrows call: 1 up, 2 down, 3 left, 4 right; `rpg/d.V/W` also hold 5..8, the diagonals) —
    /// one call a field tick, so a list of frames holds it. DIR 0 only prints where the player is
    /// (tile px, state `ar.b`, facing `ar.c`) and the camera (`ed.c:I`, `ed.d:I`) — for HZ-059's
    /// arrows held through `--keys`.
    WalkDir { at: u64, dir: i8 },
    /// `--recipe-test F:ID`: the game's own `ba.a(18, ID)` — the recipe object `es` and its three ingredients
    /// and result as the alchemist reads them (`es.d:[B`, `es.c:[B`), with their names.
    Recipe { at: u64, id: i8 },
    /// `--give F:CAT:ID`: the item into its bag as a pickup puts it, or the 계정창고 when the bag is full.
    Give { at: u64, cat: i8, id: i8 },
    /// `--bags F`: what the equipment bag and the 계정창고 hold.
    Bags { at: u64 },
    /// `--item-check F`: every new piece made and worn alone, its fields and what it does to the player.
    ItemCheck { at: u64 },
    /// `--hp-pct F:P`: the player's HP made P % of its top (`ek.a:[I[0]`) — the item test's low-HP phases.
    HpPct { at: u64, pct: i32 },
    /// `--flag F:N`: story flag N set by the game's own `ed.c(I)V` — 250 / 251 also unlock 총검사 / 정령사
    /// (`ed.g:Z` / `ed.h:Z`, written to `HOL.gamestore` at once, what finishing a round does).
    Flag { at: u64, flag: i32 },
    /// `--stash F:CAT:ID:N`: N of the item straight into the first 계정창고 page with room (the game's
    /// `bt.a(Lba;I)I`, no record written) — a storage holding a stack of the kind before HZ-065's save
    /// puts a reward away (the review's merged-stack take-back, 2026-11-03).
    Stash { at: u64, cat: i8, id: i8, count: i32 },
}

impl Action {
    fn at(&self) -> u64 {
        match *self {
            Action::Exp { at, .. }
            | Action::Money { at, .. }
            | Action::Item { at, .. }
            | Action::DropTest { at }
            | Action::Stat { at, .. }
            | Action::Quick { at, .. }
            | Action::Warp { at, .. }
            | Action::Equip { at, .. }
            | Action::Quest { at, .. }
            | Action::Hit { at, .. }
            | Action::ItemIcons { at }
            | Action::Mobs { at }
            | Action::Kill { at }
            | Action::Grade { at, .. }
            | Action::Round { at, .. }
            | Action::Job { at, .. }
            | Action::SkillLevel { at, .. }
            | Action::BaseStat { at, .. }
            | Action::SpiritLevel { at, .. }
            | Action::Refill { at }
            | Action::WalkDir { at, .. }
            | Action::Recipe { at, .. }
            | Action::Give { at, .. }
            | Action::Bags { at }
            | Action::ItemCheck { at }
            | Action::HpPct { at, .. }
            | Action::Flag { at, .. }
            | Action::Stash { at, .. } => at,
        }
    }

    /// `F:INDEX:GRADE` for `--item-grade`.
    pub fn parse_grade(s: &str) -> Option<Action> {
        let v: Vec<&str> = s.split(':').collect();
        let [at, index, grade] = v.as_slice() else { return None };
        Some(Action::Grade { at: at.parse().ok()?, index: index.parse().ok()?, grade: grade.parse().ok()? })
    }

    /// `F:N` for `--round` (the game's replay round `ed.h`: 0..2, and 3 = 가디언 모드 with HZ-030).
    /// `F:N` for `--job` (0 워리어 · 1 로그 · 2 나이트 · 3 건너 · 4 총검사 · 5 정령사).
    pub fn parse_job(s: &str) -> Option<Action> {
        let (at, job) = s.split_once(':')?;
        let job: i8 = job.parse().ok()?;
        (0..=5).contains(&job).then_some(())?;
        Some(Action::Job { at: at.parse().ok()?, job })
    }

    /// `F:ID:L` for `--skill-level`.
    pub fn parse_skill_level(s: &str) -> Option<Action> {
        let v: Vec<&str> = s.split(':').collect();
        let [at, skill, level] = v.as_slice() else { return None };
        Some(Action::SkillLevel { at: at.parse().ok()?, skill: skill.parse().ok()?, level: level.parse().ok()? })
    }

    /// `F:IDX:VALUE` for `--base-stat`.
    pub fn parse_base_stat(s: &str) -> Option<Action> {
        let v: Vec<&str> = s.split(':').collect();
        let [at, index, value] = v.as_slice() else { return None };
        let index: usize = index.parse().ok()?;
        (index < 4).then_some(())?;
        Some(Action::BaseStat { at: at.parse().ok()?, index, value: value.parse().ok()? })
    }

    /// `F:ID:L` for `--spirit-level`.
    pub fn parse_spirit_level(s: &str) -> Option<Action> {
        let v: Vec<&str> = s.split(':').collect();
        let [at, skill, level] = v.as_slice() else { return None };
        let skill: usize = skill.parse().ok()?;
        (skill < 10).then_some(())?;
        Some(Action::SpiritLevel { at: at.parse().ok()?, skill, level: level.parse().ok()? })
    }

    /// `F` for `--refill`.
    /// `F:DIR` for `--walk-dir`.
    pub fn parse_walk_dir(s: &str) -> Option<Action> {
        let (at, dir) = s.split_once(':')?;
        Some(Action::WalkDir { at: at.parse().ok()?, dir: dir.parse().ok()? })
    }

    pub fn parse_refill(s: &str) -> Option<Action> {
        Some(Action::Refill { at: s.parse().ok()? })
    }

    pub fn parse_recipe(s: &str) -> Option<Action> {
        let (at, id) = s.split_once(':')?;
        Some(Action::Recipe { at: at.parse().ok()?, id: id.parse().ok()? })
    }

    /// `F:CAT:ID` for `--give`.
    pub fn parse_give(s: &str) -> Option<Action> {
        let v: Vec<&str> = s.split(':').collect();
        let [at, cat, id] = v.as_slice() else { return None };
        Some(Action::Give { at: at.parse().ok()?, cat: cat.parse().ok()?, id: id.parse().ok()? })
    }

    /// `F` for `--bags`.
    pub fn parse_bags(s: &str) -> Option<Action> {
        Some(Action::Bags { at: s.parse().ok()? })
    }

    /// `F:CAT:ID:N` for `--stash`.
    pub fn parse_stash(s: &str) -> Option<Action> {
        let v: Vec<&str> = s.split(':').collect();
        let [at, cat, id, count] = v.as_slice() else { return None };
        Some(Action::Stash { at: at.parse().ok()?, cat: cat.parse().ok()?, id: id.parse().ok()?, count: count.parse().ok()? })
    }

    /// `F` for `--item-check`.
    pub fn parse_item_check(s: &str) -> Option<Action> {
        Some(Action::ItemCheck { at: s.parse().ok()? })
    }

    /// `F:P` for `--hp-pct`.
    pub fn parse_hp_pct(s: &str) -> Option<Action> {
        let (at, pct) = s.split_once(':')?;
        Some(Action::HpPct { at: at.parse().ok()?, pct: pct.parse().ok()? })
    }

    /// `F:N` for `--flag`.
    pub fn parse_flag(s: &str) -> Option<Action> {
        let (at, flag) = s.split_once(':')?;
        let flag: i32 = flag.parse().ok()?;
        (0..256).contains(&flag).then_some(())?;
        Some(Action::Flag { at: at.parse().ok()?, flag })
    }

    pub fn parse_round(s: &str) -> Option<Action> {
        let (at, round) = s.split_once(':')?;
        let round: i8 = round.parse().ok()?;
        (0..=hz_patches::hooks::GUARDIAN_ROUND).contains(&round).then_some(())?;
        Some(Action::Round { at: at.parse().ok()?, round })
    }

    /// `F` for `--mobs`.
    pub fn parse_mobs(s: &str) -> Option<Action> {
        Some(Action::Mobs { at: s.parse().ok()? })
    }

    /// `F` for `--kill`.
    pub fn parse_kill(s: &str) -> Option<Action> {
        Some(Action::Kill { at: s.parse().ok()? })
    }

    /// `F` for `--item-icons`.
    pub fn parse_item_icons(s: &str) -> Option<Action> {
        Some(Action::ItemIcons { at: s.parse().ok()? })
    }

    /// `F:N` for `--hit`.
    pub fn parse_hit(s: &str) -> Option<Action> {
        let (at, damage) = s.split_once(':')?;
        Some(Action::Hit { at: at.parse().ok()?, damage: damage.parse().ok()? })
    }

    /// `F:Q:STATE` for `--quest-set`.
    pub fn parse_quest(s: &str) -> Option<Action> {
        let v: Vec<&str> = s.split(':').collect();
        let [at, quest, state] = v.as_slice() else { return None };
        let (quest, state): (u8, u8) = (quest.parse().ok()?, state.parse().ok()?);
        ((1..=35).contains(&quest) && state <= 3).then_some(())?;
        Some(Action::Quest { at: at.parse().ok()?, quest, state })
    }

    /// `F:SLOT:CAT:ID` for `--equip-test`.
    pub fn parse_equip(s: &str) -> Option<Action> {
        let v: Vec<&str> = s.split(':').collect();
        let [at, slot, cat, id] = v.as_slice() else { return None };
        Some(Action::Equip {
            at: at.parse().ok()?,
            slot: slot.parse().ok()?,
            cat: cat.parse().ok()?,
            id: id.parse().ok()?,
        })
    }

    /// `F:MAP:X:Y` for `--warp`.
    pub fn parse_warp(s: &str) -> Option<Action> {
        let v: Vec<&str> = s.split(':').collect();
        let [at, map, x, y] = v.as_slice() else { return None };
        Some(Action::Warp {
            at: at.parse().ok()?,
            map: map.parse().ok()?,
            x: x.parse().ok()?,
            y: y.parse().ok()?,
        })
    }

    /// `F:SLOT:ID` for `--quick-set`.
    pub fn parse_quick(s: &str) -> Option<Action> {
        let v: Vec<&str> = s.split(':').collect();
        let [at, slot, id] = v.as_slice() else { return None };
        let slot: usize = slot.parse().ok()?;
        (slot < 6).then_some(())?;
        Some(Action::Quick {
            at: at.parse().ok()?,
            slot,
            id: id.parse().ok()?,
        })
    }

    /// `F:N` for `--exp-test`.
    pub fn parse_exp(s: &str) -> Option<Action> {
        let (at, amount) = s.split_once(':')?;
        Some(Action::Exp {
            at: at.parse().ok()?,
            amount: amount.parse().ok()?,
        })
    }

    /// `F:DX:DY:CAT:ID` for `--drop-item`.
    pub fn parse_item(s: &str) -> Option<Action> {
        let v: Vec<&str> = s.split(':').collect();
        let [at, dx, dy, cat, id] = v.as_slice() else { return None };
        Some(Action::Item {
            at: at.parse().ok()?,
            dx: dx.parse().ok()?,
            dy: dy.parse().ok()?,
            cat: cat.parse().ok()?,
            id: id.parse().ok()?,
        })
    }

    /// `F` for `--drop-test`.
    pub fn parse_drop_test(s: &str) -> Option<Action> {
        Some(Action::DropTest { at: s.parse().ok()? })
    }

    /// `F:IDX:VALUE` for `--stat-test`.
    pub fn parse_stat(s: &str) -> Option<Action> {
        let v: Vec<&str> = s.split(':').collect();
        let [at, index, value] = v.as_slice() else { return None };
        Some(Action::Stat {
            at: at.parse().ok()?,
            index: index.parse().ok()?,
            value: value.parse().ok()?,
        })
    }

    /// `F:DX:DY:N` for `--drop-money`.
    pub fn parse_money(s: &str) -> Option<Action> {
        let v: Vec<&str> = s.split(':').collect();
        let [at, dx, dy, amount] = v.as_slice() else { return None };
        Some(Action::Money {
            at: at.parse().ok()?,
            dx: dx.parse().ok()?,
            dy: dy.parse().ok()?,
            amount: amount.parse().ok()?,
        })
    }
}

/// Replace the `autoLoot` method of the hooks class with one that runs `actions` first.
pub fn wrap(mut hooks: WieJavaClassProto, actions: Vec<Action>, frame: Arc<AtomicU64>) -> WieJavaClassProto {
    if actions.is_empty() {
        return hooks;
    }
    let name = hz_patches::hooks::AUTO_LOOT.name;
    let i = hooks.methods.iter().position(|m| m.name == name).expect("the hooks class has autoLoot");
    let m = hooks.methods.remove(i);
    hooks.methods.push(JavaMethodProto {
        body: Box::new(FieldTick {
            inner: m.body,
            actions: Mutex::new(actions),
            frame,
        }),
        ..m
    });
    hooks
}

struct FieldTick {
    inner: Box<dyn MethodBody<JavaError, WieJvmContext>>,
    actions: Mutex<Vec<Action>>,
    frame: Arc<AtomicU64>,
}

#[async_trait::async_trait]
impl MethodBody<JavaError, WieJvmContext> for FieldTick {
    async fn call(&self, jvm: &Jvm, context: &mut WieJvmContext, args: Box<[JavaValue]>) -> Result<JavaValue, JavaError> {
        let f = self.frame.load(Ordering::SeqCst);
        let due: Vec<Action> = {
            let mut a = self.actions.lock().unwrap();
            let (due, later) = a.drain(..).partition(|x| x.at() <= f);
            *a = later;
            due
        };
        for action in due {
            if let Err(e) = run(jvm, &args, action, f).await {
                eprintln!("[probe] f={f} {action:?} FAILED: {e:?}");
            }
        }
        self.inner.call(jvm, context, args).await
    }
}

async fn run(jvm: &Jvm, args: &[JavaValue], action: Action, f: u64) -> JvmResult<()> {
    match action {
        Action::Exp { amount, .. } => {
            let player: Option<Box<dyn ClassInstance>> = jvm.get_static_field("ed", "a", "Lek;").await?;
            let player = player.expect("ed.a (the player) is set in the field");
            let (lv0, exp0, next0) = level(jvm, &player).await?;
            let _: () = jvm.invoke_virtual(&player, "j", "(I)V", (amount,)).await?;
            let (lv1, exp1, next1) = level(jvm, &player).await?;
            eprintln!("[probe] f={f} ek.j({amount}): LV {lv0} EXP {exp0}/{next0} -> LV {lv1} EXP {exp1}/{next1}");
        }
        Action::Money { dx, dy, amount, .. } => {
            let g: Option<Box<dyn ClassInstance>> = args[0].clone().into();
            let g = g.expect("the field tick passes g");
            let (x, y): (i32, i32) = (args[1].clone().into(), args[2].clone().into());
            let (tx, ty) = ((x + dx) as i8, (y + dy) as i8);
            let _: () = jvm
                .invoke_virtual(&g, "a", "(BBI)V", [JavaValue::Byte(tx), JavaValue::Byte(ty), JavaValue::Int(amount)])
                .await?;
            eprintln!("[probe] f={f} g.a({tx},{ty},{amount}): money on the ground, player at ({x},{y})");
        }
        Action::Item { dx, dy, cat, id, .. } => {
            let g: Option<Box<dyn ClassInstance>> = args[0].clone().into();
            let g = g.expect("the field tick passes g");
            let (x, y): (i32, i32) = (args[1].clone().into(), args[2].clone().into());
            let (tx, ty) = ((x + dx) as i8, (y + dy) as i8);
            let _: () = jvm
                .invoke_virtual(
                    &g,
                    "a",
                    "(BBBB)V",
                    [JavaValue::Byte(tx), JavaValue::Byte(ty), JavaValue::Byte(cat), JavaValue::Byte(id)],
                )
                .await?;
            eprintln!("[probe] f={f} g.a({tx},{ty},{cat},{id}): item on the ground, player at ({x},{y})");
        }
        Action::Stat { index, value, .. } => {
            let player: Option<Box<dyn ClassInstance>> = jvm.get_static_field("ed", "a", "Lek;").await?;
            let player = player.expect("ed.a (the player) is set in the field");
            let before: i32 = jvm.invoke_virtual(&player, "e", "(B)I", [JavaValue::Byte(index)]).await?;
            let max: i32 = jvm.invoke_virtual(&player, "f", "(B)I", [JavaValue::Byte(index)]).await?;
            let _: () = jvm.invoke_virtual(&player, "a", "(BI)V", [JavaValue::Byte(index), JavaValue::Int(value)]).await?;
            let after: i32 = jvm.invoke_virtual(&player, "e", "(B)I", [JavaValue::Byte(index)]).await?;
            eprintln!("[probe] f={f} ek.a({index},{value}): stat {index} {before}/{max} -> {after}/{max}");
        }
        Action::Quick { slot, id, .. } => {
            let ids: Option<Box<dyn ClassInstance>> = jvm.get_static_field("ed", "b", "[B").await?;
            let mut ids = ids.expect("ed.b (the quick slots) is set in the field");
            let before: Vec<i8> = jvm.load_array(&ids, slot, 1).await?;
            jvm.store_array(&mut ids, slot, vec![id]).await?;
            jvm.put_static_field("ce", "c", "Z", true).await?;
            // The game's setter `ed.d(IB)V` ends with `ek.a().b(slot)` for the spirit slots 2 · 3: it loads the
            // skill's effect sheet `/s/<id>` into `rpg/e.a:[Lt;`. Without it the spirit's effect `dj.<init>` reads a
            // null sheet and the game thread dies (2026-10-29, a spirit skill cast during 동화). The setter itself is
            // not called: its `ed.f(slot)` starts the slot's full cooldown (the game's swap rule).
            if (2..4).contains(&slot) {
                let player: Option<Box<dyn ClassInstance>> = jvm.get_static_field("ed", "a", "Lek;").await?;
                let player = player.expect("ed.a (the player) is set in the field");
                let spirit: Option<Box<dyn ClassInstance>> = jvm.invoke_virtual(&player, "a", "()Lp;", ()).await?;
                if let Some(spirit) = spirit {
                    let _: () = jvm.invoke_virtual(&spirit, "b", "(I)V", (slot as i32,)).await?;
                }
            }
            eprintln!("[probe] f={f} ed.b[{slot}] {} -> {id} (ce.c = true: the bar redraws)", before[0]);
        }
        Action::Warp { map, x, y, .. } => {
            let _: () = jvm
                .invoke_static("ed", "b", "(BBBB)V", [JavaValue::Byte(map as i8), JavaValue::Byte(x as i8), JavaValue::Byte(y as i8), JavaValue::Byte(0)])
                .await?;
            eprintln!("[probe] f={f} ed.b({map},{x},{y},0): go to map {map}");
        }
        Action::Equip { slot, cat, id, .. } => {
            let item: Option<Box<dyn ClassInstance>> = jvm.invoke_static("ba", "a", "(BB)Lba;", [JavaValue::Byte(cat), JavaValue::Byte(id)]).await?;
            let Some(item) = item.filter(|i| jvm.is_instance(&**i, "bx")) else {
                eprintln!("[probe] f={f} ba.a({cat},{id}): no equipment record");
                return Ok(());
            };
            let sprite: i8 = jvm.get_field(&item, "g", "B").await?;
            let palette: i16 = jvm.get_field(&item, "a", "S").await?;
            let player: Option<Box<dyn ClassInstance>> = jvm.get_static_field("ed", "a", "Lek;").await?;
            let player = player.expect("ed.a (the player) is set in the field");
            let _: () = jvm.invoke_virtual(&player, "a", "(BLbx;)V", [JavaValue::Byte(slot), JavaValue::Object(Some(item))]).await?;
            let _: () = jvm.invoke_virtual(&player, "i", "()V", ()).await?;
            let parts: Box<dyn ClassInstance> = jvm.get_static_field("ea", "a", "[B").await?;
            let part: Vec<i8> = jvm.load_array(&parts, slot.max(0) as usize, 1).await?;
            if part[0] >= 0 {
                let doll: Option<Box<dyn ClassInstance>> = jvm.get_static_field("rpg/e", "a", "Lcn;").await?;
                let reloaded: Result<(), _> = jvm.invoke_static("rpg/e", "a", "(Lcn;I)V", [JavaValue::Object(doll), JavaValue::Int(i32::from(part[0]))]).await;
                if let Err(e) = reloaded {
                    eprintln!("[probe] f={f} equip slot {slot}: the doll part {} did not load ({e:?}) — the stats are on", part[0]);
                }
            }
            eprintln!("[probe] f={f} equip slot {slot} <- ba.a({cat},{id}): sprite {sprite} palette {palette}, doll part {}", part[0]);
        }
        Action::Hit { damage, .. } => {
            let player: Option<Box<dyn ClassInstance>> = jvm.get_static_field("ed", "a", "Lek;").await?;
            let player = player.expect("ed.a (the player) is set in the field");
            let before: i32 = jvm.invoke_virtual(&player, "e", "(B)I", [JavaValue::Byte(0)]).await?;
            let _: () = jvm.invoke_virtual(&player, "c", "(I)V", (-damage,)).await?;
            let after: i32 = jvm.invoke_virtual(&player, "e", "(B)I", [JavaValue::Byte(0)]).await?;
            let state: i8 = jvm.get_field(&player, "b", "B").await?;
            eprintln!("[probe] f={f} ek.c(-{damage}): HP {before} -> {after}, state ar.b = {state}");
        }
        Action::Quest { quest, state, .. } => {
            let bits: Option<Box<dyn ClassInstance>> = jvm.get_static_field("ed", "ad", "[B").await?;
            let mut bits = bits.expect("ed.ad (the quest states) is set in the field");
            let (i, shift) = (usize::from(quest / 4), (3 - quest % 4) * 2);
            let old: Vec<i8> = jvm.load_array(&bits, i, 1).await?;
            let new = (old[0] as u8 & !(3 << shift)) | (state << shift);
            jvm.store_array(&mut bits, i, vec![new as i8]).await?;
            let read: i8 = jvm.invoke_static("ed", "a", "(I)B", [JavaValue::Int(i32::from(quest))]).await?;
            eprintln!("[probe] f={f} quest {quest}: {} -> {state} (ed.a({quest}) = {read})", (old[0] as u8 >> shift) & 3);
        }
        Action::ItemIcons { .. } => {
            let list: Option<Box<dyn ClassInstance>> = jvm.get_static_field("rpg/e", "k", "[Lcw;").await?;
            let list = list.expect("rpg/e.k (the item pictures) is loaded in the field");
            let n = jvm.array_length(&list).await?;
            let out = OUT.get().cloned().unwrap_or_else(|| PathBuf::from("out"));
            for i in 0..n {
                let cw: Vec<Option<Box<dyn ClassInstance>>> = jvm.load_array(&list, i, 1).await?;
                let Some(cw) = cw.into_iter().next().flatten() else {
                    eprintln!("[probe] f={f} item picture {i}: null");
                    continue;
                };
                let image: Option<Box<dyn ClassInstance>> = jvm.get_field(&cw, "a", "Ljavax/microedition/lcdui/Image;").await?;
                let Some(image) = image else { continue };
                let (w, h, px) = rgb(jvm, &image).await?;
                let s = 4u32;
                let mut rgba = Vec::with_capacity((w * h * s * s * 4) as usize);
                for y in 0..h * s {
                    for x in 0..w * s {
                        let p = px[((y / s) * w + x / s) as usize];
                        rgba.extend([(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8]);
                    }
                }
                let path = out.join(format!("item_{i:02}.png"));
                match image::save_buffer(&path, &rgba, w * s, h * s, image::ExtendedColorType::Rgba8) {
                    Ok(()) => eprintln!("[probe] f={f} item picture {i}: {w}x{h} -> {}", path.display()),
                    Err(e) => eprintln!("[probe] f={f} item picture {i}: {e}"),
                }
            }
        }
        Action::Mobs { .. } => {
            let mobs = living(jvm).await?;
            eprintln!("[probe] f={f} {} monsters:", mobs.len());
            for m in &mobs {
                eprintln!("[probe]   {}", describe(jvm, m).await?);
            }
            eprintln!("[probe] f={f} respawn queue: {}", queue(jvm).await?);
        }
        Action::Kill { .. } => {
            let Some(m) = living(jvm).await?.into_iter().next() else {
                eprintln!("[probe] f={f} kill: no living monster");
                return Ok(());
            };
            let who = describe(jvm, &m).await?;
            let before = ground(jvm).await?;
            let _: () = jvm.invoke_virtual(&m, "g", "()V", ()).await?;
            let countdown: i16 = jvm.get_field_from(&m, "cq", "b", "S").await?;
            eprintln!("[probe] f={f} kill {who}: cq.g() -> countdown {countdown}; queue {}", queue(jvm).await?);
            // The kill's drops: an item goes in at the head of the ground list (g.a(BBBB)V insertElementAt 0), money
            // at its end (g.a(BBI)V addElement), and since HZ-033's 2026-11-06 fix a slot's equipment and the band's
            // item may come in the same kill — so what is new is the list less what was there (nothing else runs
            // between the two reads), the oldest first. (Until 2026-11-06 this printed the head whenever the list
            // grew: a money-only kill reported the item before it again.)
            let mut old = before;
            let mut new = Vec::new();
            for e in ground(jvm).await? {
                match old.iter().position(|o| *o == e) {
                    Some(i) => {
                        old.swap_remove(i);
                    }
                    None => new.push(e),
                }
            }
            // In list order the new items run newest first and the money is last: reversed, the drop's own order.
            for e in new.iter().rev() {
                let what = if e[2] == 1 { "money".to_string() } else { format!("item {}:{}", e[3], e[4]) };
                eprintln!("[probe] f={f} drop: {what} at ({},{})", e[0], e[1]);
            }
        }
        Action::Job { job, .. } => {
            let was: i8 = jvm.get_static_field("ed", "b", "B").await?;
            jvm.put_static_field("ed", "b", "B", job).await?;
            let player: Option<Box<dyn ClassInstance>> = jvm.get_static_field("ed", "a", "Lek;").await?;
            if let Some(player) = player {
                // The job window `bi` (javap 2026-10-29 @504..569): `ek.e(B)V` is the change itself; it then takes
                // the armour (0, 4, 6) and the weapons (3, 5) off and reloads the paper doll
                // `rpg/e.a:Lcn = rpg/e.a().a()` — without that the old job's doll cannot show a dagger or a shield
                // (NegativeArraySizeException / ArrayIndexOutOfBounds on the next equip).
                let _: () = jvm.invoke_virtual(&player, "e", "(B)V", [JavaValue::Byte(job)]).await?;
                for slot in [0i8, 4, 6, 3, 5] {
                    let worn: Option<Box<dyn ClassInstance>> = jvm.invoke_virtual(&player, "a", "(B)Lbx;", [JavaValue::Byte(slot)]).await?;
                    if let Some(worn) = worn {
                        let _: () = jvm.invoke_virtual(&player, "a", "(Lbx;)V", [JavaValue::Object(Some(worn))]).await?;
                    }
                }
                let factory: Option<Box<dyn ClassInstance>> = jvm.invoke_static("rpg/e", "a", "()Lrpg/e;", ()).await?;
                if let Some(factory) = factory {
                    let doll: Option<Box<dyn ClassInstance>> = jvm.invoke_virtual(&factory, "a", "()Lcn;", ()).await?;
                    jvm.put_static_field("rpg/e", "a", "Lcn;", doll).await?;
                }
                let _: () = jvm.invoke_virtual(&player, "i", "()V", ()).await?;
                // `ek.A()V` (what `s()` / `b(Z)` run at a map entry) sizes the action queue `ac:[B` for the job —
                // `(ed.b == 5 ? 6 : 4) + 2`. The bench warps before it changes the job, so without this a 정령사 kept
                // the warrior's 6 and the end of 동화's 6th step read ac[6] (ArrayIndexOutOfBounds every frame, the
                // player stuck in it — 2026-10-29). In the game a map entry or a load comes between.
                let _: () = jvm.invoke_special(&player, "ek", "A", "()V", ()).await?;
            }
            let now: i8 = jvm.get_static_field("ed", "b", "B").await?;
            eprintln!("[probe] f={f} job ed.b {was} -> {now} (asked {job}; gear off, doll reloaded)");
        }
        Action::SkillLevel { skill, level, .. } => {
            let player: Option<Box<dyn ClassInstance>> = jvm.get_static_field("ed", "a", "Lek;").await?;
            let player = player.expect("ed.a (the player) is set in the field");
            // `ek.a(BB)V` (javap 2026-10-29) is the window's "learn one level": d[job * 8 + skill - 4] += 1.
            let job: i8 = jvm.get_static_field("ed", "b", "B").await?;
            let was: i8 = jvm.invoke_virtual(&player, "a", "(B)B", [JavaValue::Byte(skill)]).await?;
            for _ in was..level {
                let _: () = jvm.invoke_virtual(&player, "a", "(BB)V", [JavaValue::Byte(job), JavaValue::Byte(skill)]).await?;
            }
            let now: i8 = jvm.invoke_virtual(&player, "a", "(B)B", [JavaValue::Byte(skill)]).await?;
            eprintln!("[probe] f={f} skill {skill} level {was} -> {now}");
        }
        Action::BaseStat { index, value, .. } => {
            let player: Option<Box<dyn ClassInstance>> = jvm.get_static_field("ed", "a", "Lek;").await?;
            let player = player.expect("ed.a (the player) is set in the field");
            // `ek.i()V` @128..147 rebuilds the stats `c:[I` as 10 + the allocation `a:[S` (+ options, passives).
            let alloc: Option<Box<dyn ClassInstance>> = jvm.get_field(&player, "a", "[S").await?;
            let mut alloc = alloc.expect("ek.a:[S (the stat allocation) is set");
            let was: Vec<i16> = jvm.load_array(&alloc, index, 1).await?;
            jvm.store_array(&mut alloc, index, vec![(value - 10).clamp(0, i32::from(i16::MAX)) as i16]).await?;
            let _: () = jvm.invoke_virtual(&player, "i", "()V", ()).await?;
            let stats: Option<Box<dyn ClassInstance>> = jvm.get_field(&player, "c", "[I").await?;
            let now: Vec<i32> = match stats {
                Some(c) => jvm.load_array(&c, 0, 4).await?,
                None => Vec::new(),
            };
            let attack: i32 = jvm.get_field(&player, "h", "I").await?;
            let defence: i32 = jvm.get_field(&player, "a", "I").await?;
            eprintln!("[probe] f={f} base stat {index}: allocation {} -> {}; stats {now:?}, attack {attack} defence {defence}", was[0], value - 10);
        }
        Action::SpiritLevel { skill, level, .. } => {
            let player: Option<Box<dyn ClassInstance>> = jvm.get_static_field("ed", "a", "Lek;").await?;
            let player = player.expect("ed.a (the player) is set in the field");
            let spirit: Option<Box<dyn ClassInstance>> = jvm.invoke_virtual(&player, "a", "()Lp;", ()).await?;
            let Some(spirit) = spirit else {
                eprintln!("[probe] f={f} spirit level: the player has no spirit");
                return Ok(());
            };
            let levels: Option<Box<dyn ClassInstance>> = jvm.get_field(&spirit, "c", "[B").await?;
            let mut levels = levels.expect("p.c (the spirit's skill levels) is set");
            let was: Vec<i8> = jvm.load_array(&levels, skill, 1).await?;
            jvm.store_array(&mut levels, skill, vec![level]).await?;
            let now: i8 = jvm.invoke_virtual(&spirit, "a", "(B)B", [JavaValue::Byte(skill as i8)]).await?;
            eprintln!("[probe] f={f} spirit skill {skill} level {} -> {now}", was[0]);
        }
        Action::WalkDir { dir, .. } => {
            if dir != 0 {
                let _: () = jvm.invoke_static("ed", "b", "(B)V", [JavaValue::Byte(dir)]).await?;
            }
            let player: Option<Box<dyn ClassInstance>> = jvm.get_static_field("ed", "a", "Lek;").await?;
            if let Some(p) = player {
                let (x, y): (i16, i16) = (jvm.get_field_from(&p, "er", "h", "S").await?, jvm.get_field_from(&p, "er", "i", "S").await?);
                let (state, facing): (i8, i8) = (jvm.get_field_from(&p, "ar", "b", "B").await?, jvm.get_field_from(&p, "ar", "c", "B").await?);
                let (cx, cy): (i32, i32) = (jvm.get_static_field("ed", "c", "I").await?, jvm.get_static_field("ed", "d", "I").await?);
                // The key the game holds (`f.c`, 0 = none): a block keeps it, a press the game dropped does not.
                let key: i32 = jvm.get_static_field("f", "c", "I").await?;
                eprintln!("[probe] f={f} walk {dir}: at ({x},{y}) state {state} dir {facing} camera ({cx},{cy}) key {key}");
            }
        }
        Action::Refill { .. } => {
            let player: Option<Box<dyn ClassInstance>> = jvm.get_static_field("ed", "a", "Lek;").await?;
            let player = player.expect("ed.a (the player) is set in the field");
            let now: Option<Box<dyn ClassInstance>> = jvm.get_field(&player, "a", "[I").await?;
            let top: Option<Box<dyn ClassInstance>> = jvm.get_field(&player, "b", "[I").await?;
            let (Some(mut now), Some(top)) = (now, top) else {
                eprintln!("[probe] f={f} refill: no stat arrays");
                return Ok(());
            };
            let tops: Vec<i32> = jvm.load_array(&top, 0, 4).await?;
            for i in [0usize, 1, 3] {
                jvm.store_array(&mut now, i, vec![tops[i]]).await?;
            }
            eprintln!("[probe] f={f} refill: HP {} SP {} MP {}", tops[0], tops[1], tops[3]);
        }
        Action::Recipe { id, .. } => {
            let made: Option<Box<dyn ClassInstance>> = jvm.invoke_static("ba", "a", "(BB)Lba;", [JavaValue::Byte(18), JavaValue::Byte(id)]).await?;
            let Some(r) = made else {
                eprintln!("[probe] f={f} recipe {id}: no item");
                return Ok(());
            };
            let mut fields = Vec::new();
            for field in ["d", "c"] {
                let arr: Option<Box<dyn ClassInstance>> = jvm.get_field_from(&r, "es", field, "[B").await?;
                fields.push(match arr {
                    Some(a) => {
                        let n = jvm.array_length(&a).await?;
                        jvm.load_array::<i8>(&a, 0, n).await?
                    }
                    None => Vec::new(),
                });
            }
            let (d, c) = (&fields[0], &fields[1]);
            let mut names = Vec::new();
            for pair in d.chunks(2).chain(c.chunks(2)) {
                let item: Option<Box<dyn ClassInstance>> = jvm.invoke_static("ba", "a", "(BB)Lba;", [JavaValue::Byte(pair[0]), JavaValue::Byte(pair[1])]).await?;
                let name = match item {
                    Some(it) => {
                        let bytes: Option<Box<dyn ClassInstance>> = jvm.get_field_from(&it, "ba", "a", "[B").await?;
                        match bytes {
                            Some(b) => {
                                let n = jvm.array_length(&b).await?;
                                let raw: Vec<i8> = jvm.load_array(&b, 0, n).await?;
                                raw.iter().map(|&x| format!("{:02x}", x as u8)).collect::<String>()
                            }
                            None => "?".into(),
                        }
                    }
                    None => "(none)".into(),
                };
                names.push(format!("({},{}) {name}", pair[0], pair[1]));
            }
            eprintln!("[probe] f={f} recipe {id}: class {} ingredients {:?} -> {:?}", r.class_definition().name(), &names[..names.len().saturating_sub(1)], names.last());
        }
        Action::Round { round, .. } => {
            let was: i8 = jvm.get_static_field("ed", "h", "B").await?;
            jvm.put_static_field("ed", "h", "B", round).await?;
            eprintln!("[probe] f={f} replay round ed.h {was} -> {round} (monsters made from now on)");
        }
        Action::Grade { index, grade, .. } => {
            let bag: Option<Box<dyn ClassInstance>> = jvm.invoke_static("ed", "a", "(B)Lbt;", [JavaValue::Byte(0)]).await?;
            let bag = bag.expect("the equipment bag");
            let items: Box<dyn ClassInstance> = jvm.get_field(&bag, "a", "Ljava/util/Vector;").await?;
            let item: Option<Box<dyn ClassInstance>> = jvm.invoke_virtual(&items, "elementAt", "(I)Ljava/lang/Object;", (index,)).await?;
            match item.filter(|i| jvm.is_instance(&**i, "bx")) {
                Some(mut it) => {
                    let before: i8 = jvm.get_field_from(&it, "bx", "e", "B").await?;
                    jvm.put_field_from(&mut it, "bx", "e", "B", grade).await?;
                    eprintln!("[probe] f={f} equipment bag [{index}]: grade {before} -> {grade}");
                }
                None => eprintln!("[probe] f={f} equipment bag [{index}]: no equipment there"),
            }
        }
        Action::DropTest { .. } => {
            // Slot 0 of the 24-byte /en/edf drop block: cat, id, then the threshold as a BIG-ENDIAN
            // INT32 (rpg/g.a([BI)I reads 4 bytes; tools_re/edf_drops.py agrees).
            let record: [i8; 6] = [0, 0, 0, 0, 0x12, 0xe1u8 as i8];
            let mut array = jvm.instantiate_array("B", record.len()).await?;
            jvm.store_array(&mut array, 0, record.to_vec()).await?;
            let t: i32 = jvm
                .invoke_static(
                    hz_patches::hooks::CLASS,
                    hz_patches::hooks::DROP_THRESHOLD.name,
                    hz_patches::hooks::DROP_THRESHOLD.descriptor,
                    (array, 2),
                )
                .await?;
            eprintln!("[probe] f={f} Hooks.dropThreshold(slot 0 = 4833) = {t}");
        }
        Action::Give { cat, id, .. } => {
            let room = |bag: Box<dyn ClassInstance>| async move {
                let ok: bool = jvm.invoke_virtual(&bag, "a", "(BBI)Z", [JavaValue::Byte(cat), JavaValue::Byte(id), JavaValue::Int(1)]).await?;
                JvmResult::Ok(ok.then_some(bag))
            };
            let item: Option<Box<dyn ClassInstance>> = jvm.invoke_static("ba", "a", "(BB)Lba;", [JavaValue::Byte(cat), JavaValue::Byte(id)]).await?;
            let Some(item) = item else {
                eprintln!("[probe] f={f} give ({cat},{id}): no item record");
                return Ok(());
            };
            let bag: Option<Box<dyn ClassInstance>> = jvm.invoke_static("ed", "a", "(B)Lbt;", [JavaValue::Byte(cat)]).await?;
            if let Some(bag) = room(bag.expect("the bag of the category")).await? {
                let _: i32 = jvm.invoke_virtual(&bag, "a", "(Lba;I)I", [JavaValue::Object(Some(item)), JavaValue::Int(1)]).await?;
                eprintln!("[probe] f={f} give ({cat},{id}) -> bag ({} held)", held(jvm, &bag).await?.len());
                return Ok(());
            }
            for (i, page) in storage_pages(jvm).await?.into_iter().enumerate() {
                if let Some(page) = room(page).await? {
                    let _: i32 = jvm.invoke_virtual(&page, "a", "(Lba;I)I", [JavaValue::Object(Some(item)), JavaValue::Int(1)]).await?;
                    let _: () = jvm.invoke_static("dz", "i", "()V", ()).await?;
                    eprintln!("[probe] f={f} give ({cat},{id}) -> 계정창고 page {} ({} held there)", i + 1, held(jvm, &page).await?.len());
                    return Ok(());
                }
            }
            eprintln!("[probe] f={f} give ({cat},{id}): no room in the bag nor the 계정창고");
        }
        Action::Bags { .. } => {
            let bag: Option<Box<dyn ClassInstance>> = jvm.invoke_static("ed", "a", "(B)Lbt;", [JavaValue::Byte(0)]).await?;
            let bag = bag.expect("the equipment bag");
            let size: i8 = jvm.get_field_from(&bag, "bt", "a", "B").await?;
            let items = held(jvm, &bag).await?;
            eprintln!("[probe] f={f} equipment bag {}/{size}: {items:?}", items.len());
            dump_bag(jvm, f, "bag", &bag).await?;
            // The 조합 and 특수 bags (16 · 18 → b, 17 · 19 · 20 → c) with each stack's count `ba.c:B`.
            for (name, cat) in [("craft", 16i8), ("special", 20i8)] {
                let bag: Option<Box<dyn ClassInstance>> = jvm.invoke_static("ed", "a", "(B)Lbt;", [JavaValue::Byte(cat)]).await?;
                let bag = bag.expect("the bag of the category");
                let items: Box<dyn ClassInstance> = jvm.get_field_from(&bag, "bt", "a", "Ljava/util/Vector;").await?;
                let n: i32 = jvm.invoke_virtual(&items, "size", "()I", ()).await?;
                let mut held = Vec::new();
                for i in 0..n {
                    let it: Option<Box<dyn ClassInstance>> = jvm.invoke_virtual(&items, "elementAt", "(I)Ljava/lang/Object;", (i,)).await?;
                    if let Some(it) = it {
                        let (c, d, k): (i8, i8, i8) =
                            (jvm.get_field_from(&it, "ba", "a", "B").await?, jvm.get_field_from(&it, "ba", "b", "B").await?, jvm.get_field_from(&it, "ba", "c", "B").await?);
                        held.push(format!("{c}:{d}x{k}"));
                    }
                }
                eprintln!("[probe] f={f} {name} bag {n}: {}", held.join(" "));
            }
            for (i, page) in storage_pages(jvm).await?.iter().enumerate() {
                let size: i8 = jvm.get_field_from(page, "bt", "a", "B").await?;
                let items = stacks(jvm, page).await?;
                eprintln!("[probe] f={f} 계정창고 page {} {}/{size}: {}", i + 1, items.len(), items.join(" "));
                dump_bag(jvm, f, &format!("storage{}", i + 1), page).await?;
            }
        }
        Action::ItemCheck { .. } => item_check(jvm, f).await?,
        Action::HpPct { pct, .. } => {
            let player: Option<Box<dyn ClassInstance>> = jvm.get_static_field("ed", "a", "Lek;").await?;
            let player = player.expect("ed.a (the player) is set in the field");
            let mut now: Box<dyn ClassInstance> = jvm.get_field_from(&player, "ek", "a", "[I").await?;
            let top: Box<dyn ClassInstance> = jvm.get_field_from(&player, "ek", "b", "[I").await?;
            let top: i32 = jvm.load_array::<i32>(&top, 0, 1).await?.pop().unwrap_or_default();
            let hp = (top * pct / 100).max(1);
            jvm.store_array(&mut now, 0, vec![hp]).await?;
            eprintln!("[probe] f={f} hp-pct {pct}: HP {hp}/{top}");
        }
        Action::Flag { flag, .. } => {
            let _: () = jvm.invoke_static("ed", "c", "(I)V", (flag,)).await?;
            let (gunner, spirit): (bool, bool) = (jvm.get_static_field("ed", "g", "Z").await?, jvm.get_static_field("ed", "h", "Z").await?);
            eprintln!("[probe] f={f} flag {flag} set (총검사 {gunner} · 정령사 {spirit})");
        }
        Action::Stash { cat, id, count, .. } => {
            let item: Option<Box<dyn ClassInstance>> = jvm.invoke_static("ba", "a", "(BB)Lba;", [JavaValue::Byte(cat), JavaValue::Byte(id)]).await?;
            let Some(item) = item else {
                eprintln!("[probe] f={f} stash ({cat},{id}): no item record");
                return Ok(());
            };
            for (i, page) in storage_pages(jvm).await?.iter().enumerate() {
                let room: bool = jvm.invoke_virtual(page, "a", "(BBI)Z", [JavaValue::Byte(cat), JavaValue::Byte(id), JavaValue::Int(count)]).await?;
                if room {
                    let took: i32 = jvm.invoke_virtual(page, "a", "(Lba;I)I", [JavaValue::Object(Some(item)), JavaValue::Int(count)]).await?;
                    eprintln!("[probe] f={f} stash ({cat},{id}) x{count} -> 계정창고 page {} took {took}: {}", i + 1, stacks(jvm, page).await?.join(" "));
                    return Ok(());
                }
            }
            eprintln!("[probe] f={f} stash ({cat},{id}) x{count}: no room in the 계정창고");
        }
    }
    Ok(())
}

/// The game's own pieces `--item-check` also wears, as controls for its expectations: LV 65 블러드캘러미티 ·
/// 붉은사자 방패 · 봉인의 고글 · 봉인의 줄 · 흑암의 흉갑 · 빙해의 신발.
const CONTROLS: [(i8, i8); 6] = [(0, 30), (7, 30), (10, 30), (11, 3), (12, 30), (14, 28)];

/// The equipment slot (`ek.a:[Lbx;`, the paper doll's 0 장갑 · 1 머리 · 2 장신구 · 3 무기 · 4 갑옷 · 5 보조 ·
/// 6 신발) a category is worn in.
fn slot_of(cat: i8) -> i8 {
    match cat {
        0..=6 => 3,
        7 => 5,
        8 | 9 => 0,
        10 => 1,
        11 => 2,
        12 | 13 => 4,
        _ => 6,
    }
}

/// What `--item-check` reads of the player: stats `c` (힘 민첩 건강 의지), tops `b` (HP SP EXP MP), attack
/// `h`, defence `a` and the worn option sums `ek.g(0..15)`.
#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
async fn player_numbers(jvm: &Jvm, p: &Box<dyn ClassInstance>) -> JvmResult<String> {
    let c: Box<dyn ClassInstance> = jvm.get_field_from(p, "ek", "c", "[I").await?;
    let b: Box<dyn ClassInstance> = jvm.get_field_from(p, "ek", "b", "[I").await?;
    let c: Vec<i32> = jvm.load_array(&c, 0, 4).await?;
    let b: Vec<i32> = jvm.load_array(&b, 0, 4).await?;
    let attack: i32 = jvm.get_field_from(p, "ek", "h", "I").await?;
    let defence: i32 = jvm.get_field_from(p, "ek", "a", "I").await?;
    let mut g = Vec::new();
    for k in 0..16i8 {
        let v: i32 = jvm.invoke_special(p, "ek", "g", "(B)I", vec![JavaValue::Byte(k)]).await?;
        g.push(v);
    }
    Ok(format!("c {c:?} b {b:?} atk {attack} def {defence} g {g:?}"))
}

/// A piece's fields as the game holds them: name (hex EUC-KR), level · grade · set · sprite (`bx.d e f g`), palette
/// `bx.a:S`, the three stats `bx.b c d:S`, `h`, the six options `bx.c:[B`.
#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
async fn item_fields(jvm: &Jvm, item: &Box<dyn ClassInstance>) -> JvmResult<String> {
    let bytes = |a: Option<Box<dyn ClassInstance>>| async move {
        JvmResult::Ok(match a {
            Some(a) => {
                let len = jvm.array_length(&a).await?;
                jvm.load_array::<i8>(&a, 0, len).await?
            }
            None => Vec::new(),
        })
    };
    let name = bytes(jvm.get_field_from(item, "ba", "a", "[B").await?).await?;
    let name: String = name.iter().map(|&b| format!("{:02x}", b as u8)).collect();
    let lv: i8 = jvm.get_field_from(item, "bx", "d", "B").await?;
    let grade: i8 = jvm.get_field_from(item, "bx", "e", "B").await?;
    let set: i8 = jvm.get_field_from(item, "bx", "f", "B").await?;
    let sprite: i8 = jvm.get_field_from(item, "bx", "g", "B").await?;
    let palette: i16 = jvm.get_field_from(item, "bx", "a", "S").await?;
    let stats: [i16; 3] = [
        jvm.get_field_from(item, "bx", "b", "S").await?,
        jvm.get_field_from(item, "bx", "c", "S").await?,
        jvm.get_field_from(item, "bx", "d", "S").await?,
    ];
    let h: i8 = jvm.get_field_from(item, "bx", "h", "B").await?;
    let options = bytes(jvm.get_field_from(item, "bx", "c", "[B").await?).await?;
    Ok(format!("name {name} lv {lv} grade {grade} set {set} sprite {sprite} pal {palette} stats {stats:?} h {h} opts {options:?}"))
}

async fn item_check(jvm: &Jvm, f: u64) -> JvmResult<()> {
    let player: Option<Box<dyn ClassInstance>> = jvm.get_static_field("ed", "a", "Lek;").await?;
    let player = player.expect("ed.a (the player) is set in the field");
    let job: i8 = jvm.get_static_field("ed", "b", "B").await?;
    for slot in 0..7i8 {
        let _: () = jvm.invoke_virtual(&player, "j", "(B)V", [JavaValue::Byte(slot)]).await?;
    }
    let _: () = jvm.invoke_virtual(&player, "i", "()V", ()).await?;
    let base = player_numbers(jvm, &player).await?;
    eprintln!("[probe] f={f} item-check job {job} base {base}");
    let pieces = hz_java::legend::ALL
        .iter()
        .map(|l| (l.category, l.id, "legend"))
        .chain((0..16i8).map(|c| (c, hz_patches::hooks::GEAR_ID, "guardian")))
        .chain(CONTROLS.iter().map(|&(c, i)| (c, i, "control")));
    for (cat, id, kind) in pieces {
        let made: Option<Box<dyn ClassInstance>> = jvm.invoke_static("ba", "a", "(BB)Lba;", [JavaValue::Byte(cat), JavaValue::Byte(id)]).await?;
        let Some(item) = made.filter(|i| jvm.is_instance(&**i, "bx")) else {
            eprintln!("[probe] f={f} item-check {kind} {cat} {id} NO-RECORD");
            continue;
        };
        let fields = item_fields(jvm, &item).await?;
        let mut own = Vec::new();
        for k in 0..16 {
            let v: i32 = jvm.invoke_virtual(&item, "a", "(I)I", (k,)).await?;
            own.push(v);
        }
        let slot = slot_of(cat);
        let _: () = jvm.invoke_virtual(&player, "a", "(BLbx;)V", [JavaValue::Byte(slot), JavaValue::Object(Some(item.clone()))]).await?;
        let _: () = jvm.invoke_virtual(&player, "i", "()V", ()).await?;
        let worn_slot: i8 = jvm.get_field_from(&item, "bx", "i", "B").await?;
        let worn = player_numbers(jvm, &player).await?;
        let _: () = jvm.invoke_virtual(&player, "j", "(B)V", [JavaValue::Byte(slot)]).await?;
        let _: () = jvm.invoke_virtual(&player, "i", "()V", ()).await?;
        let back = player_numbers(jvm, &player).await?;
        eprintln!(
            "[probe] f={f} item-check {kind} {cat} {id} {fields} own {own:?} slot {slot} worn_at {worn_slot} | worn {worn} | back {back} | same {}",
            back == base
        );
    }
    // The player is left with nothing worn: the test wears its set after.
    Ok(())
}

/// `(cat, id)` of every item a bag holds (its Vector's non-null entries, in order).
#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
async fn held(jvm: &Jvm, bag: &Box<dyn ClassInstance>) -> JvmResult<Vec<(i8, i8)>> {
    let items: Box<dyn ClassInstance> = jvm.get_field_from(bag, "bt", "a", "Ljava/util/Vector;").await?;
    let n: i32 = jvm.invoke_virtual(&items, "size", "()I", ()).await?;
    let mut out = Vec::new();
    for i in 0..n {
        let it: Option<Box<dyn ClassInstance>> = jvm.invoke_virtual(&items, "elementAt", "(I)Ljava/lang/Object;", (i,)).await?;
        if let Some(it) = it {
            out.push((jvm.get_field_from(&it, "ba", "a", "B").await?, jvm.get_field_from(&it, "ba", "b", "B").await?));
        }
    }
    Ok(out)
}

/// Every cell of a bag as `cat:id xcount` (`ba.c:B` — the stack's count, 1 for gear), in the bag's order.
#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
async fn stacks(jvm: &Jvm, bag: &Box<dyn ClassInstance>) -> JvmResult<Vec<String>> {
    let items: Box<dyn ClassInstance> = jvm.get_field_from(bag, "bt", "a", "Ljava/util/Vector;").await?;
    let n: i32 = jvm.invoke_virtual(&items, "size", "()I", ()).await?;
    let mut out = Vec::new();
    for i in 0..n {
        let it: Option<Box<dyn ClassInstance>> = jvm.invoke_virtual(&items, "elementAt", "(I)Ljava/lang/Object;", (i,)).await?;
        if let Some(it) = it {
            let (c, d, k): (i8, i8, i8) =
                (jvm.get_field_from(&it, "ba", "a", "B").await?, jvm.get_field_from(&it, "ba", "b", "B").await?, jvm.get_field_from(&it, "ba", "c", "B").await?);
            out.push(format!("{c}:{d}x{k}"));
        }
    }
    Ok(out)
}

/// One `bag-item` line for every piece of equipment a bag holds, with [`item_fields`] (`--bags`; the item test's
/// round trip compares them before a save and after the load).
#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
async fn dump_bag(jvm: &Jvm, f: u64, place: &str, bag: &Box<dyn ClassInstance>) -> JvmResult<()> {
    let items: Box<dyn ClassInstance> = jvm.get_field_from(bag, "bt", "a", "Ljava/util/Vector;").await?;
    let n: i32 = jvm.invoke_virtual(&items, "size", "()I", ()).await?;
    for i in 0..n {
        let it: Option<Box<dyn ClassInstance>> = jvm.invoke_virtual(&items, "elementAt", "(I)Ljava/lang/Object;", (i,)).await?;
        let Some(it) = it.filter(|it| jvm.is_instance(&**it, "bx")) else { continue };
        let cat: i8 = jvm.get_field_from(&it, "ba", "a", "B").await?;
        let id: i8 = jvm.get_field_from(&it, "ba", "b", "B").await?;
        // `bx.i:B` — the paper-doll slot the piece is worn in, −1 in the bag.
        let worn: i8 = jvm.get_field_from(&it, "bx", "i", "B").await?;
        eprintln!("[probe] f={f} bag-item {place} {i} {cat} {id} worn {worn} {}", item_fields(jvm, &it).await?);
    }
    Ok(())
}

/// The monsters on the map's draw lists that are not dying (`ar.b:B` 6 / 7), list order: the
/// y-sorted list `g.a:Ler;`, then the one `g.c:Ler;` that the big bosses' parts (kinds 101..108 and
/// 124..131 — `ak.a(BB)Z` puts them there with `g.b(Ler;)V`, maps 124 126 132 133) are drawn from.
async fn living(jvm: &Jvm) -> JvmResult<Vec<Box<dyn ClassInstance>>> {
    let map: Option<Box<dyn ClassInstance>> = jvm.get_static_field("ed", "a", "Lg;").await?;
    let Some(map) = map else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for head in ["a", "c"] {
        let mut next: Option<Box<dyn ClassInstance>> = jvm.get_field(&map, head, "Ler;").await?;
        for _ in 0..512 {
            let Some(e) = next else { break };
            if jvm.is_instance(&*e, "cq") {
                let st: i8 = jvm.get_field(&e, "b", "B").await?;
                if st != 6 && st != 7 {
                    out.push(e.clone());
                }
            }
            next = jvm.get_field(&e, "b", "Ler;").await?;
        }
    }
    Ok(out)
}

/// `class kind K LV n HP i/h ATK f DEF e` — through `cq`'s own fields (subclasses shadow names).
#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
async fn describe(jvm: &Jvm, m: &Box<dyn ClassInstance>) -> JvmResult<String> {
    let kind: i16 = jvm.get_field_from(m, "cq", "c", "S").await?;
    let lv: i8 = jvm.get_field_from(m, "cq", "n", "B").await?;
    let hp: i32 = jvm.get_field_from(m, "cq", "i", "I").await?;
    let max: i32 = jvm.get_field_from(m, "cq", "h", "I").await?;
    let attack: i16 = jvm.get_field_from(m, "cq", "f", "S").await?;
    let defence: i16 = jvm.get_field_from(m, "cq", "e", "S").await?;
    Ok(format!("{} kind {kind} LV {lv} HP {hp}/{max} ATK {attack} DEF {defence}", m.class_definition().name()))
}

/// The ground list `ed.a:Lg;` → `g.b:Vector` of 6-byte entries (x, y, kind 0 item / 1 money, cat,
/// id, timer), head first.
async fn ground(jvm: &Jvm) -> JvmResult<Vec<Vec<i8>>> {
    let g: Option<Box<dyn ClassInstance>> = jvm.get_static_field("ed", "a", "Lg;").await?;
    let Some(g) = g else { return Ok(Vec::new()) };
    let v: Option<Box<dyn ClassInstance>> = jvm.get_field(&g, "b", "Ljava/util/Vector;").await?;
    let Some(v) = v else { return Ok(Vec::new()) };
    let n: i32 = jvm.invoke_virtual(&v, "size", "()I", ()).await?;
    let mut out = Vec::new();
    for i in 0..n {
        let e: Option<Box<dyn ClassInstance>> = jvm.invoke_virtual(&v, "elementAt", "(I)Ljava/lang/Object;", (i,)).await?;
        if let Some(e) = e {
            out.push(jvm.load_array(&e, 0, 6).await?);
        }
    }
    Ok(out)
}

/// The respawn queue `cq.c:Vector` as `kind:countdown` pairs.
async fn queue(jvm: &Jvm) -> JvmResult<String> {
    let v: Option<Box<dyn ClassInstance>> = jvm.get_static_field("cq", "c", "Ljava/util/Vector;").await?;
    let Some(v) = v else { return Ok("(none)".into()) };
    let n: i32 = jvm.invoke_virtual(&v, "size", "()I", ()).await?;
    let mut items = Vec::new();
    for i in 0..n {
        let m: Option<Box<dyn ClassInstance>> = jvm.invoke_virtual(&v, "elementAt", "(I)Ljava/lang/Object;", (i,)).await?;
        if let Some(m) = m {
            let kind: i16 = jvm.get_field_from(&m, "cq", "c", "S").await?;
            let b: i16 = jvm.get_field_from(&m, "cq", "b", "S").await?;
            items.push(format!("{kind}:{b}"));
        }
    }
    Ok(format!("[{}]", items.join(" ")))
}

/// An `Image`'s pixels (ARGB) through `Image.getRGB`.
#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
async fn rgb(jvm: &Jvm, image: &Box<dyn ClassInstance>) -> JvmResult<(u32, u32, Vec<u32>)> {
    let w: i32 = jvm.invoke_virtual(image, "getWidth", "()I", ()).await?;
    let h: i32 = jvm.invoke_virtual(image, "getHeight", "()I", ()).await?;
    let n = (w.max(0) * h.max(0)) as usize;
    let buf = jvm.instantiate_array("I", n).await?;
    let args = [
        JavaValue::Object(Some(buf.clone())),
        JavaValue::Int(0),
        JavaValue::Int(w),
        JavaValue::Int(0),
        JavaValue::Int(0),
        JavaValue::Int(w),
        JavaValue::Int(h),
    ];
    let _: () = jvm.invoke_virtual(image, "getRGB", "([IIIIIII)V", args).await?;
    let px: Vec<i32> = jvm.load_array(&buf, 0, n).await?;
    Ok((w as u32, h as u32, px.into_iter().map(|p| p as u32).collect()))
}

/// (`ek.n:B` level, `ek.a:[I`[2] EXP, `ek.b:[I`[2] EXP to the next level).
#[allow(clippy::borrowed_box)] // the jvm API takes `&Box<dyn ClassInstance>`
async fn level(jvm: &Jvm, player: &Box<dyn ClassInstance>) -> JvmResult<(i8, i32, i32)> {
    let lv: i8 = jvm.get_field(player, "n", "B").await?;
    let a: Box<dyn ClassInstance> = jvm.get_field(player, "a", "[I").await?;
    let b: Box<dyn ClassInstance> = jvm.get_field(player, "b", "[I").await?;
    let exp: Vec<i32> = jvm.load_array(&a, 2, 1).await?;
    let next: Vec<i32> = jvm.load_array(&b, 2, 1).await?;
    Ok((lv, exp[0], next[0]))
}
