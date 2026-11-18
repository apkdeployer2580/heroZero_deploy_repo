//! 영웅서기 Zero's own patches.
//!
//! The emulator forks under `vendor/` fix the PLATFORM (a missing MIDP method, a JVM
//! resolution bug) and are shared lineage with hero2/hero3. This crate is the other half:
//! changes to how THIS GAME behaves. Keeping the two apart is the point — a platform fix
//! must be right for any MIDlet, a game patch only has to be right for one build of one
//! jar, and mixing them is how a game decision ends up silently baked into the platform.
//!
//! A patch is one of two kinds, cheapest first:
//!
//! * [`PatchKind::AppProperty`] — an app property layered over the jar's manifest. Used
//!   when the game ALREADY has the switch: EA left QA hooks in this port that read
//!   `MIDlet.getAppProperty`, so behaviour can change with no bytecode touched at all.
//! * [`PatchKind::ClassRewrite`] — the bytes of one class file, rewritten as the JVM
//!   defines it. Keyed by the md5 of the ORIGINAL class, so a rewrite can only ever meet
//!   the exact bytes it was written against; any other build of the class passes through
//!   untouched (and [`Patches::report`] says so).
//!
//! Every patch has a stable id (`HZ-nnn`), a Korean title for the settings screen, and
//! the evidence it rests on. Ids are never reused: a withdrawn patch keeps its number.
//!
//! The jar asset itself is never modified (`tools_re/make_hz_asset.py` stages it
//! byte-for-byte), so turning every patch off gives back the original game.
//!
//! A class rewrite may also turn one 3-byte instruction into an `invokestatic` of a host
//! HOOK ([`hooks`]): a native method the app provides (`hz_java`), which reads the player's
//! options. Every hook returns the original result when its option is at the default, so
//! "patch on, option default" plays exactly like "patch off".

mod classfile;

/// The contract between the hook patches here and the host class that answers them
/// (`hz_java::hooks`, which builds the class from this same list — a test there checks it).
/// Descriptors take game objects as `Object`: the host never needs the guest type, and the
/// hook class stays free of names only this jar defines.
pub mod hooks {
    /// The class the patched bytecode calls. Defined by the host, never by the jar.
    pub const CLASS: &str = "net/wie/hz/Hooks";

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Hook {
        pub name: &'static str,
        pub descriptor: &'static str,
    }

    /// HZ-004. `|amount| × EXP rate` — what `ek.j(I)V` adds to the player's EXP.
    pub const EXP: Hook = Hook { name: "exp", descriptor: "(I)I" };
    /// HZ-005. `rpg/g.a(edf, offset) × drop rate`, at most 65535 — a monster's item-slot threshold. With
    /// HZ-033 (× 2, and its `return` after a slot's drop gone) 0 for the slots left once one dropped in the roll.
    pub const DROP_THRESHOLD: Hook = Hook { name: "dropThreshold", descriptor: "([BI)I" };
    /// HZ-006. `g.a(BB)Z` on the player's tile, then (auto-loot on) one item nearby.
    pub const AUTO_LOOT: Hook = Hook { name: "autoLoot", descriptor: "(Ljava/lang/Object;II)Z" };
    /// HZ-007. `bn` (the options window) title / row label: `this`, text-table index.
    pub const OPT_LABEL: Hook = Hook { name: "optLabel", descriptor: "(Ljava/lang/Object;I)[B" };
    /// HZ-007. `this`, Graphics wrapper `dd`, the rows' base y. Returns the value column's x,
    /// or −1 after drawing the host page's values itself (the game's are then skipped).
    pub const OPT_VALUES: Hook = Hook { name: "optValues", descriptor: "(Ljava/lang/Object;Ljava/lang/Object;I)I" };
    /// HZ-007. In place of `o.b(II)Z` (child window first) at the top of `bn.a(II)Z`: `true` =
    /// the key is consumed.
    pub const OPT_KEY: Hook = Hook { name: "optKey", descriptor: "(Ljava/lang/Object;II)Z" };
    /// HZ-008. In place of `Graphics.drawImage(Image,III)V` at the end of `f.paint` — the one
    /// call that puts the game's back buffer on the canvas, once a frame: `Graphics g`, `Image`,
    /// x, y, anchor. The hook draws exactly that, then reads the scene for the HUD.
    pub const BLIT: Hook = Hook { name: "blit", descriptor: "(Ljava/lang/Object;Ljava/lang/Object;III)V" };
    /// HZ-009. In place of `ep.a(I)Z`'s test of the QA property: `true` = the purchase is granted
    /// at once (what `EA-Porting-Test-SMS-Mode=true` does), `false` = refused at once (`=false`).
    pub const CASH_FREE: Hook = Hook { name: "cashFree", descriptor: "()Z" };
    /// HZ-011. In place of the body of `cx.a(I)[B` (one entry of a text table): `this`, index.
    /// Returns the entry, or the host's replacement for it (계정창고 wording).
    pub const TEXT: Hook = Hook { name: "text", descriptor: "(Ljava/lang/Object;I)[B" };
    /// HZ-012. In place of `bh.a.a(I)` (the menu table, `t/gm`) where the menu's rows and its
    /// children's titles are drawn: `this` (the list window, or the child), the row/label index.
    /// For the menu and every window opened under it (any depth), counted from the menu's last
    /// row: that one is gm[9] (시스템), the one before it 「계정창고」, the rest gm[r+1] (the cheat
    /// shop is gone) — ten rows (HZ-012) or nine without 미니맵 (HZ-026).
    pub const MENU_LABEL: Hook = Hook { name: "menuLabel", descriptor: "(Ljava/lang/Object;I)[B" };
    /// HZ-012. The menu's 계정창고 row (8; 7 with HZ-026): opens the game's storage chooser `bg` as
    /// the menu's child.
    pub const OPEN_STORAGE: Hook = Hook { name: "openStorage", descriptor: "(Ljava/lang/Object;)V" };
    /// HZ-012. In place of `bg`'s RSK `parent.c()`: closes the chooser, then — when the parent
    /// is the in-game menu — the game's own quick save (`bh.c(2)`), so the slot never drifts
    /// from the storage it just traded with.
    pub const STORAGE_CLOSED: Hook = Hook { name: "storageClosed", descriptor: "(Ljava/lang/Object;)V" };
    /// HZ-014. In place of `bipush 80; isub` after `getstatic rpg/a.a:I` (the canvas width) in
    /// `ce.f()V`: the target-monster panel's x. Returns `width − 80` moved left by however many
    /// guest px the host's minimap needs (0 = the original).
    pub const TARGET_X: Hook = Hook { name: "targetX", descriptor: "(I)I" };
    /// HZ-011. In place of `getstatic rpg/d.J:[B` — the one-time help before the storage (the
    /// title's `bb`, the in-game 상점및창고 `de`): `rpg/d.J`, or its 계정창고 wording.
    pub const STORAGE_HELP: Hook = Hook { name: "storageHelp", descriptor: "()[B" };
    /// HZ-016. In place of `rpg/e.a(String,I)[B` in the item factory `ba.a(BB)Lba;`: one record of
    /// an item file — the file's own, or 「부활의 구」 for `/itm/20` number [`REVIVE_ORB`].1.
    pub const ITEM_RECORD: Hook = Hook { name: "itemRecord", descriptor: "(Ljava/lang/String;I)[B" };
    /// HZ-016. In place of `rpg/e.k[index]` in `ba.a(dd,IIZZ)V` (an item's picture): the game's
    /// picture, or the orb's for [`REVIVE_ORB_ICON`] (which is otherwise the selection frame).
    pub const ITEM_ICON: Hook = Hook { name: "itemIcon", descriptor: "(I)Ljava/lang/Object;" };
    /// HZ-016. The body of `ba.a()Z` (`this`): whether the bag offers 「0번 등록 / #에 등록」 for the
    /// item — the game's rule (special bag, number 2 and up), not for the orb.
    pub const QUICK_ITEM: Hook = Hook { name: "quickItem", descriptor: "(Ljava/lang/Object;)Z" };
    /// HZ-016. In place of the death's cheat-shop test in `ek.C()V`: `true` = the game over now
    /// (no orb in the bag), `false` = the revive question is open and the death waits for it.
    pub const DEATH: Hook = Hook { name: "death", descriptor: "()Z" };
    /// HZ-016. In place of `cf`'s `parent.a(f, row)` (both of its answers): the revive question's
    /// answer when the parent is the menu, else the call as it was.
    pub const ANSWER: Hook = Hook { name: "answer", descriptor: "(Ljava/lang/Object;BB)V" };
    /// HZ-017. In place of `this.q()V` at the end of the monster's stat setup `cq.i()V` (`this`):
    /// the call as it was, then the monster's max HP / HP / attack × the 난이도 percent.
    pub const MONSTER_INIT: Hook = Hook { name: "monsterInit", descriptor: "(Ljava/lang/Object;)V" };
    /// HZ-018. In place of `getfield eq.a:S` in the death `cq.g()V` (the kind's record `eq`): its
    /// respawn delay in field ticks, divided by the 몬스터 리젠 rate (0 = never, kept).
    pub const RESPAWN_DELAY: Hook = Hook { name: "respawnDelay", descriptor: "(Ljava/lang/Object;)I" };
    /// HZ-019. In place of `getstatic ce.h:I` where `g.c()V` anchors the pickup notices: the
    /// viewport's bottom, [`NOTICE_LIFT`] higher when the game's bar is gone (`ce.h` = the canvas).
    pub const NOTICE_BOTTOM: Hook = Hook { name: "noticeBottom", descriptor: "()I" };
    /// HZ-020. In place of `getstatic rpg/b.a/b/c:[Ljava/lang/String;` (the three slots' store
    /// names, read by `rpg/b`'s slot readers and writers): the names of the slot page shown — the
    /// game's own arrays on page 0.
    pub const GAMEDATA_STORES: Hook = Hook { name: "gamedataStores", descriptor: "()[Ljava/lang/String;" };
    pub const ITEM_STORES: Hook = Hook { name: "itemStores", descriptor: "()[Ljava/lang/String;" };
    pub const SLOT_STORES: Hook = Hook { name: "slotStores", descriptor: "()[Ljava/lang/String;" };
    /// HZ-020. In place of `o.b(II)Z` (child first) at the top of the slot window's `bb.a(II)Z`:
    /// the call as it was, then LEFT / RIGHT turn the slot page (the title only).
    pub const SLOT_KEY: Hook = Hook { name: "slotKey", descriptor: "(Ljava/lang/Object;II)Z" };
    /// HZ-020. In place of `bb.a(dd)V`'s three draws: its title (`ah.a`, the page arrows round
    /// it), a slot row's box (`o.a(Ldd;IIIIII)V`) and its 「저장 N」 (`ah.a`) — renumbered for the
    /// page, and nothing for a row the last page has not.
    pub const SLOT_TITLE: Hook = Hook { name: "slotTitle", descriptor: "(Ljava/lang/Object;II[BII)V" };
    pub const SLOT_BOX: Hook = Hook { name: "slotBox", descriptor: "(Ljava/lang/Object;IIIIII)V" };
    pub const SLOT_LABEL: Hook = Hook { name: "slotLabel", descriptor: "(Ljava/lang/Object;II[BII)V" };
    /// HZ-020. In place of the title's `getstatic ed.i:B` (how many slots hold a save, which puts
    /// the carousel on 이어하기 and keeps it there): the count over every page.
    pub const SAVE_COUNT: Hook = Hook { name: "saveCount", descriptor: "()B" };
    /// HZ-021. In place of `o.b(Ldd;III)V` (the gold) at the end of the shop's painter `co.a(dd)V`
    /// (`this`, `dd`, x, y, gold): the gold as before, then — while selling with nothing open over
    /// it — the 「일괄판매」 button ([`BULK_BUTTON`]).
    pub const SHOP_GOLD: Hook = Hook { name: "shopGold", descriptor: "(Ljava/lang/Object;Ljava/lang/Object;III)V" };
    /// HZ-022. In place of `getfield ek.n:B` in the EXP loop `ek.j(I)V` @75 (`this`): the level as
    /// read — and at 99, the guardian level's work (the EXP the loop took back, the points, the next
    /// GL's start).
    pub const LEVEL_UP: Hook = Hook { name: "levelUp", descriptor: "(Ljava/lang/Object;)I" };
    /// HZ-022. In place of `getfield ek.b:[I` where `ek.i()V` stores the EXP to the next level
    /// (`this`): the array as read — at 99 its `[2]` set to the next guardian level's start and a
    /// scratch array handed back for the game's own value.
    pub const EXP_TABLE: Hook = Hook { name: "expTable", descriptor: "(Ljava/lang/Object;)[I" };
    /// HZ-022. The bodies of `ek.e(B)I` / `ek.f(B)I` (a stat now / its top): as read — at 99 the EXP
    /// into the guardian level and what it needs.
    pub const STAT_NOW: Hook = Hook { name: "statNow", descriptor: "(Ljava/lang/Object;I)I" };
    pub const STAT_MAX: Hook = Hook { name: "statMax", descriptor: "(Ljava/lang/Object;I)I" };
    /// HZ-023. [`ITEM_RECORD`]'s place without HZ-016: the file's record, or a guardian piece's for
    /// number [`GEAR_ID`] of an equipment file.
    pub const GEAR_RECORD: Hook = Hook { name: "gearRecord", descriptor: "(Ljava/lang/String;I)[B" };
    /// HZ-023 over HZ-016's [`ITEM_RECORD`]: the file's record, the orb's, or a guardian piece's.
    pub const ORB_GEAR_RECORD: Hook = Hook { name: "orbGearRecord", descriptor: "(Ljava/lang/String;I)[B" };
    /// HZ-023. In place of `Math.min(lvl / 5 + 13, 28)` — the rare band's item number in a monster's
    /// drop `cq.o()V`: as `min`, except that at the band's top (a monster of LV 75 and up — only the
    /// game's second replay round has them) it may be [`GEAR_ID`].
    pub const RARE_DROP: Hook = Hook { name: "rareDrop", descriptor: "(II)I" };
    /// HZ-025. In place of `dd.a(Lcw;III)V` where `g.c()V` draws a pickup notice, its anchor's
    /// `bipush 33` made `iload_2` (the notice's slot): the game's graphics, the picture, x, y, the
    /// slot. Draws nothing — on the notice's first drawn frame ([`PICKUP_FIRST_COUNT`]) its picture
    /// goes to the host's notice stack.
    pub const PICKUP_NOTICE: Hook = Hook { name: "pickupNotice", descriptor: "(Ljava/lang/Object;Ljava/lang/Object;III)V" };
    /// HZ-028. In place of the per-pixel store in the translucent fill `dd.a(IIII)V` (`aload 5; iload 7;
    /// iload 6; iastore; iinc 7, -1`, made `aload 5; iload 6; <this>; istore 7`): the fill's `int[w·h]`
    /// and its colour. Sets every element to the colour — what the loop stores, top index down — and
    /// returns −1 for the loop's index, so the loop's next test leaves it.
    pub const FILL_INTS: Hook = Hook { name: "fillInts", descriptor: "([II)I" };
    /// HZ-028. In place of the per-pixel mask in the translucent picture `dd.a(Lcw;III)V` (`if (p >>> 24
    /// != 0) p &= dd.a`, made `aload 7; aload_0; getfield dd.a:I; <this>; istore_1`): the picture's
    /// `getRGB` pixels and the mask. ANDs the mask into every pixel that is not fully transparent and
    /// returns 0 for the loop's index — its `iinc 1, -1` then leaves it at −1, as the loop does.
    pub const MASK_ALPHA: Hook = Hook { name: "maskAlpha", descriptor: "([II)I" };
    /// HZ-030. In place of `this.p()V` in the monster's stat setup `cq.i()V` (`this`): the kind's
    /// record copied as before — and in 가디언 모드 ([`GUARDIAN_ROUND`]) the level and stats raised from it.
    pub const GUARDIAN_MONSTER: Hook = Hook { name: "guardianMonster", descriptor: "(Ljava/lang/Object;)V" };
    /// HZ-030. In place of both `getstatic ed.h:B` (the replay round) in `cq.i()V`: the round up to 헬,
    /// 0 in 가디언 모드 — the game's own round block is skipped, [`GUARDIAN_MONSTER`] has done its work.
    pub const MONSTER_ROUND: Hook = Hook { name: "monsterRound", descriptor: "()I" };
    /// HZ-030. In place of `ed.e(I)V` (money) in script action 17, the quest reward the script picks by
    /// round: as given, more in 가디언 모드.
    pub const QUEST_MONEY: Hook = Hook { name: "questMoney", descriptor: "(I)V" };
    /// HZ-030. In place of `ek.j(I)V` (EXP) in script action 20 (the player, the amount): the same.
    pub const QUEST_EXP: Hook = Hook { name: "questExp", descriptor: "(Ljava/lang/Object;I)V" };
    /// HZ-030. In place of `cx.a(I)[B` where the ending letters 「… 모드로」 (`ce.d(dd)V`): the text
    /// table's entry, or 「가디언 모드로」 from 헬 on.
    pub const ROUND_TEXT: Hook = Hook { name: "roundText", descriptor: "(Ljava/lang/Object;I)[B" };
    /// HZ-031. [`ITEM_RECORD`]'s place when neither HZ-016 nor HZ-023 put a hook there: the file's
    /// record, or an 85제 piece's for numbers [`LEGEND_FIRST_ID`] and up of an equipment file. (Those
    /// two hooks serve the 85제 pieces too while HZ-031 is chosen.)
    pub const LEGEND_RECORD: Hook = Hook { name: "legendRecord", descriptor: "(Ljava/lang/String;I)[B" };
    /// HZ-031. In place of `getfield bx.e:B; iaload` where an item's name takes its grade's colour
    /// (`q.c[grade]` — the bag's panel `q.b(Ldd;IILba;Z)V`, the pickup notice `g.a(BBBZ)V`): the
    /// table's colour, or [`LEGEND_COLOUR`] for [`LEGEND_GRADE`] (the table has four).
    pub const GRADE_COLOR: Hook = Hook { name: "gradeColor", descriptor: "([ILjava/lang/Object;)I" };
    /// HZ-031. In place of `cx.a(I)[B` where the item window letters the grade (`t/it` [28 + grade]):
    /// the entry, or 「전설」 for [`LEGEND_GRADE`] (entry 32 is 「타입」).
    pub const GRADE_NAME: Hook = Hook { name: "gradeName", descriptor: "(Ljava/lang/Object;I)[B" };
    /// HZ-031. In place of `q.b(Ldd;IILba;)V` in the item window `cz.a(dd)V` (`this`, `dd`, x, y,
    /// the item): the window's contents as before, then an 85제 piece's effect lines below them.
    pub const ITEM_INFO: Hook = Hook { name: "itemInfo", descriptor: "(Ljava/lang/Object;Ljava/lang/Object;IILjava/lang/Object;)V" };
    /// HZ-031. In place of `this.o()V` (the drop) in a monster's death `cq.g()V`: the drop as before,
    /// then in 가디언 모드 the 85제 roll.
    pub const MONSTER_DROP: Hook = Hook { name: "monsterDrop", descriptor: "(Ljava/lang/Object;)V" };
    /// HZ-032. In place of `cq.a(II)I` (`max(5, attack − defence)`) in the hit `cq.a(Lek;BIZBBZZZ)Z`
    /// @103 — every basic and skill hit: the game's damage × the worn 85제 pieces' (and the basic hit's
    /// events: C1 · C2 · S2).
    pub const HIT_DAMAGE: Hook = Hook { name: "hitDamage", descriptor: "(II)I" };
    /// HZ-032. In place of `ek.a(BZ)I` (the critical chance) in the same hit @172: the chance, or 100
    /// for the first hit after 신속 (R3).
    pub const CRIT_CHANCE: Hook = Hook { name: "critChance", descriptor: "(Ljava/lang/Object;BZ)I" };
    /// HZ-032. In place of `ek.c(I)V` (the HP change, negative) where a monster's hit lands
    /// `ek.a(Lar;IB)Z` @338: the change × the pieces' damage taken.
    pub const TAKE_DAMAGE: Hook = Hook { name: "takeDamage", descriptor: "(Ljava/lang/Object;I)V" };
    /// HZ-032. In place of `this.n()V` (the EXP) in a monster's death `cq.g()V` @26: the EXP, then the
    /// kill's effects (W4 · S3).
    pub const MONSTER_EXP: Hook = Hook { name: "monsterExp", descriptor: "(Ljava/lang/Object;)V" };
    /// HZ-032. In place of `db.b(B)I` (a skill's SP) in the action driver `ek.C()V` @354: × the pieces'
    /// (0 for a second cast).
    pub const SKILL_SP: Hook = Hook { name: "skillSp", descriptor: "(Ljava/lang/Object;B)I" };
    /// HZ-032. In place of `db.b(B)Z` (turn to the target — once at an action's start, after its SP and
    /// cooldown) in `ek.C()V` @460: as before, then a second cast queued (`ek.f(B)V`) or a skill's mark.
    pub const CAST_START: Hook = Hook { name: "castStart", descriptor: "(Ljava/lang/Object;B)Z" };
    /// HZ-032. In place of `ek.d(B)I` / `p.d(B)I` (a job / spirit skill's cooldown) in `ed.g(I)V` @26 /
    /// @39: × the pieces'.
    pub const JOB_COOLDOWN: Hook = Hook { name: "jobCooldown", descriptor: "(Ljava/lang/Object;B)I" };
    pub const SPIRIT_COOLDOWN: Hook = Hook { name: "spiritCooldown", descriptor: "(Ljava/lang/Object;B)I" };
    /// HZ-032. In place of `ek.n(I)V` (동화's frames) in `eo.a(BI)Z` @191: × 2 with S1.
    pub const FUSION_START: Hook = Hook { name: "fusionStart", descriptor: "(Ljava/lang/Object;I)V" };
    /// HZ-032. In place of `h.a(Lek;S)V` (분신 and its lifetime) in `dn.a(BI)Z` @918: × 2 with R1.
    pub const CLONE_LIFE: Hook = Hook { name: "cloneLife", descriptor: "(Ljava/lang/Object;S)V" };
    /// HZ-032. In place of `ar.b(BI)V` where 트릭샷's mark (status 17) goes on a monster in `cq.c(bc)V`
    /// @90: × 3 with G3.
    pub const MARK_STATUS: Hook = Hook { name: "markStatus", descriptor: "(Ljava/lang/Object;BI)V" };
    /// HZ-032. In place of `ar.b(BI)V` (반격's status 20 × 100) in `ek.k()V` @5: never ending (−1) with K1.
    pub const COUNTER_STATUS: Hook = Hook { name: "counterStatus", descriptor: "(Ljava/lang/Object;BI)V" };
    /// HZ-032. In place of `p.a(BI)I` (공허의 손's cut of a spirit skill's MP) in `p.d(B)Z` @45: the cut, and
    /// S4's.
    pub const SPIRIT_COST: Hook = Hook { name: "spiritCost", descriptor: "(Ljava/lang/Object;BI)I" };
    /// HZ-032. In place of `ek.c(I)I` (a spirit skill's power) in `p.e(B)I` @22: × 1.5 with S5.
    pub const SPIRIT_POWER: Hook = Hook { name: "spiritPower", descriptor: "(Ljava/lang/Object;I)I" };
    /// HZ-033. In place of `Math.min(lvl / 4, 14)` (the common slots' item number) in `cq.o()V` @158: the
    /// number, or one of the few under it.
    pub const COMMON_DROP: Hook = Hook { name: "commonDrop", descriptor: "(II)I" };
    /// HZ-033. In place of `g.a(BBBB)V` (an item slot's drop onto the ground) in `cq.o()V` @271 (the map,
    /// x, y, category, number): the drop — in 헬 and 가디언 모드 a top rare may become a LV 60..65 piece;
    /// the roll's slots are then taken (the loop goes on to the game's bands, `dropThreshold` answers 0).
    pub const DROP_ITEM: Hook = Hook { name: "dropItem", descriptor: "(Ljava/lang/Object;BBBB)V" };
    /// HZ-034. In place of `ba.a(BB)Lba;` (the item a quest reward makes) in script action 16 `eb.b(BLn;)Z`
    /// @1021: the item — in 가디언 모드 gear becomes its category's guardian piece, a recipe a LV 65 one.
    pub const QUEST_ITEM: Hook = Hook { name: "questItem", descriptor: "(BB)Ljava/lang/Object;" };
    /// HZ-034. In place of `bt.a(Lba;I)I` (into the bag, so many) in the same action @1045: the count —
    /// doubled for materials, stones and potions in 가디언 모드. HZ-065 (the same hook, put there when HZ-034
    /// is off): what the bag does not take waits for the next save (which puts it into the 계정창고), and
    /// the whole count is answered.
    pub const QUEST_GIVE: Hook = Hook { name: "questGive", descriptor: "(Ljava/lang/Object;Ljava/lang/Object;I)I" };
    /// HZ-032, the jobs' own sites. 반격's reflection `ar.a(Lar;IBB)V` in `ek.a(Lar;IB)Z` @119 (attacker,
    /// player, damage, 0, 3): K2 · K4.
    pub const COUNTER_REFLECT: Hook = Hook { name: "counterReflect", descriptor: "(Ljava/lang/Object;Ljava/lang/Object;IBB)V" };
    /// 방패치기's hit `d.a(Lcq;BB)Z` (private) in `d.a(BI)Z` @139: K5 hits the 3×3 and stuns.
    pub const SHIELD_BASH: Hook = Hook { name: "shieldBash", descriptor: "(Ljava/lang/Object;Ljava/lang/Object;BB)Z" };
    /// 강타's front hit `ek.a(BIBZZZ)Z` in `ag.a(BI)Z` @131: W2 also the two tiles behind it.
    pub const SMASH: Hook = Hook { name: "smash", descriptor: "(Ljava/lang/Object;BIBZZZ)Z" };
    /// 광기's HP `ek.c(I)V` in `ag.a(BI)Z` @394: W4 none.
    pub const FRENZY_COST: Hook = Hook { name: "frenzyCost", descriptor: "(Ljava/lang/Object;I)V" };
    /// `db.c(B)I` read as a chance: 저격's kill % (`ai.a(BI)Z` @953, G1 ×2), 독수's % (`dn.a(BI)Z` @363,
    /// R4 +30), 폭발탄's knockback % (`av.c()Z` @9, B3 +30).
    pub const SNIPE_CHANCE: Hook = Hook { name: "snipeChance", descriptor: "(Ljava/lang/Object;B)I" };
    pub const POISON_CHANCE: Hook = Hook { name: "poisonChance", descriptor: "(Ljava/lang/Object;B)I" };
    pub const BLAST_CHANCE: Hook = Hook { name: "blastChance", descriptor: "(Ljava/lang/Object;B)I" };
    /// 독수's poison, one basic roll `ek.c(Z)I` in `dn.a(BI)Z` @374: R4 ×2.
    pub const POISON_DAMAGE: Hook = Hook { name: "poisonDamage", descriptor: "(Ljava/lang/Object;Z)I" };
    /// The hit's knockback in `cq.a(Lek;BIZBBZZZ)Z` @321..331 (`if (knock) this.b(dir)`, 11 bytes) — the
    /// monster, the hit's damage, knock, dir, fx: the push as before, B3's 30 % more on it first.
    pub const KNOCK: Hook = Hook { name: "knock", descriptor: "(Ljava/lang/Object;IZBB)V" };
    /// The player's knockback test `ek.g()Z` / `ar.g()Z` where a monster's hit pushes (`cq.a([BSZ)Z` @112,
    /// `at.b()V` @49, `di.b()V` @96, `dk.b()V` @49): B4 never for the player.
    pub const KNOCKABLE: Hook = Hook { name: "knockable", descriptor: "(Ljava/lang/Object;)Z" };
    /// `bt.a(Lba;)I` once a potion is drunk in `ek.a(Z)Z` @196 (the bag, the potion): F2's 10 s.
    pub const DRANK: Hook = Hook { name: "drank", descriptor: "(Ljava/lang/Object;Ljava/lang/Object;)I" };
    /// `getfield ar.f:B` (the px a walking frame) in `ar.d()Z` @25: F2 steps 16 · 16 · 8 from tile to tile.
    pub const MOVE_STEP: Hook = Hook { name: "moveStep", descriptor: "(Ljava/lang/Object;)I" };
    /// HZ-035. In place of `iconst_4; idiv; i2b` (the random box's recipe number `n / 4`, up to 45) in
    /// script action 18 `eb.b(BLn;)Z` @1347: the game's recipes only (0..27).
    pub const BOX_RECIPE: Hook = Hook { name: "boxRecipe", descriptor: "(I)I" };
    /// HZ-036. In place of `bt.a(I)Lba;` where a bag window reads a cell of the page it shows (`q`'s and
    /// `bz`'s grids, the cursor's item in `ds` `eg` `co` `dv` `et`): the bag and the cell, 0..27. For one of
    /// the player's three bags (`ed.a/b/c:Lbt;`) the item at page × [`BAG_PAGE`] + cell; any other bag (the
    /// shop's stock, a 계정창고 page) as it was.
    pub const BAG_AT: Hook = Hook { name: "bagAt", descriptor: "(Ljava/lang/Object;I)Ljava/lang/Object;" };
    /// HZ-036. In place of `o.d(II)Z` (LEFT / RIGHT on the grid) in a bag window's keys (`this`, the keys):
    /// the call, then — the cursor gone round (27 → 0, 0 → 27) on a player's bag — the next or last page.
    pub const BAG_SIDEWAYS: Hook = Hook { name: "bagSideways", descriptor: "(Ljava/lang/Object;II)Z" };
    /// HZ-036. The same for `o.e(II)Z` (UP / DOWN): DOWN on the bottom row is the next page's top row.
    pub const BAG_UPDOWN: Hook = Hook { name: "bagUpDown", descriptor: "(Ljava/lang/Object;II)Z" };
    /// HZ-036. In place of `q.a(Ldd;Lbt;BZBBI)V` (the grid and the cursor's item, `ds` and `eg`): the draw,
    /// then — a player's bag — its page under the tabs, ◀ n/4 ▶.
    pub const BAG_GRID: Hook = Hook { name: "bagGrid", descriptor: "(Ljava/lang/Object;Ljava/lang/Object;Ljava/lang/Object;BZBBI)V" };
    /// HZ-036. The same for `bz.a(Ldd;Lbt;)V` (the shop's, the alchemist's and the smith's grid).
    pub const BAG_GRID_BZ: Hook = Hook { name: "bagGridBz", descriptor: "(Ljava/lang/Object;Ljava/lang/Object;Ljava/lang/Object;)V" };
    /// HZ-036. In place of `getfield bt.a:B` (a bag's size) where 계정창고's window `eg.c(B)V` sizes its
    /// grid from it: at most a page, [`BAG_PAGE`].
    pub const BAG_CELLS: Hook = Hook { name: "bagCells", descriptor: "(Ljava/lang/Object;)I" };
    /// HZ-036 (2026-11-01 — user report: 계정창고 「넣기」 kept the item and lost another). In place of
    /// `bt.a(I)V` (take the item at a place out of the bag) where a bag window takes the cursor's item
    /// away: `ds.a(BB)V` @316 (인벤토리 「버리기」), `eg.a(BB)V` @162 (계정창고 「넣기」) and @473
    /// (캐릭터창고 「버리기」). The bag and the cell 0..27 — the same place [`BAG_AT`] read the item from.
    pub const BAG_TAKE: Hook = Hook { name: "bagTake", descriptor: "(Ljava/lang/Object;I)V" };
    /// HZ-036. The same for `bt.a(II)V` (take `n` off the stack at a place): `ds.a(BB)V` @342 (귀환서 사용).
    pub const BAG_TAKE_SOME: Hook = Hook { name: "bagTakeSome", descriptor: "(Ljava/lang/Object;II)V" };
    /// HZ-039. In place of `rpg/e.a(String)[B` where the skill tables load — `db.b(B)V` @2 (`/hero/hsd1`),
    /// `ek.a(B)Lek;` @34 (`/hero/hsd2`), `p.f()V` @3 / @20 (`/s/d/a`, `/s/d/p`): the file as the game's
    /// loader reads it, with the host's edits laid over it (`hz_java::tables`).
    pub const TABLE: Hook = Hook { name: "table", descriptor: "(Ljava/lang/String;)[B" };
    /// HZ-040. In place of `putfield ek.h:I` in `ek.i()V` @582 (`this`, the attack before the weapon):
    /// stores the host formula's value instead (`hz_java::stats`).
    pub const ATTACK: Hook = Hook { name: "attack", descriptor: "(Ljava/lang/Object;I)V" };
    /// HZ-040. In place of `putfield ek.a:I` in `ek.i()V` @817 (`this`, the defence): the host's.
    pub const DEFENCE: Hook = Hook { name: "defence", descriptor: "(Ljava/lang/Object;I)V" };
    /// HZ-040. In place of `p.a(B)I` (내성, spirit passive 2) where `ek.i()V` @1082 sums max HP (the spirit,
    /// 2): the passive's value plus the host's extra HP per 건강.
    pub const HP_BONUS: Hook = Hook { name: "hpBonus", descriptor: "(Ljava/lang/Object;B)I" };
    /// HZ-055. In place of `getfield ek.n:B` (the level) where `ek.i()V` @1017 sums max SP (`5 × n + 10 × 의지 +
    /// option 4`): the level times three, so SP is 15 a level (hero2's +10; user 2026-10-29).
    pub const SP_LEVEL: Hook = Hook { name: "spLevel", descriptor: "(Ljava/lang/Object;)I" };
    /// HZ-056. In place of `invokespecial ek.D()V` where the spirit cast (state 10) ends, `ek.C()V` @717
    /// (`this`): `D()`, then the action queue emptied (`ek.B()V`) when that left the player idle with a step
    /// still in it — an OK pressed during the cast in 동화 queued a transformed step (6) that blocks every OK.
    pub const SPIRIT_CAST_END: Hook = Hook { name: "spiritCastEnd", descriptor: "(Ljava/lang/Object;)V" };
    /// HZ-058. In place of `ed.d(IB)V` (the game's registration into quick slot `slot` — dedupe in its
    /// pair, store, a full cooldown) where a skill window's popup row is chosen, `cb.a(BBBBZ)V` @143 / @180
    /// (`slot` 0 · 1 = 「1번 · 3번 등록」, 2 · 3 = the spirit's 「7번 · 9번」; the skill): with the patch on
    /// the first row is 「퀵슬롯등록」 (choose a plate) and the second 「퀵슬롯해제」 (`hz_java::quickpage`).
    pub const QUICK_REGISTER: Hook = Hook { name: "quickRegister", descriptor: "(IB)V" };
    /// HZ-058. In place of `ed.a(ZB)V` (the potion into the `0` slot, `true`, or the `#` slot) where an
    /// item's popup row is chosen, `ds.a(BB)V` @72: 「퀵슬롯등록 / 퀵슬롯해제」 as above.
    pub const QUICK_POTION: Hook = Hook { name: "quickPotion", descriptor: "(ZB)V" };
    /// HZ-040. In place of `ek.a(BIZ)V` (private: `ad[2 (+3)] = v`) at the end of `ek.G()V` @171 / @178
    /// (`this`, 2, the game's crit, off-hand): the store, with the host's crit.
    pub const CRIT: Hook = Hook { name: "crit", descriptor: "(Ljava/lang/Object;BIZ)V" };
    /// HZ-040. The same at the end of `ek.H()V` @114 / @121 (`this`, 0, the game's evasion, off-hand).
    pub const EVASION: Hook = Hook { name: "evasion", descriptor: "(Ljava/lang/Object;BIZ)V" };
    /// HZ-041: the least a monster's hit chance on the player can be (`cq.a(Lar;)Z` @28 — the game's 15).
    pub const HIT_FLOOR: u8 = 30;
    /// HZ-042. In place of `getfield cb.j:B` (the skill window's level cap) at its seven reads (`this`):
    /// the cap — 5 on the job window's active tab, the field elsewhere.
    pub const LEVEL_CAP: Hook = Hook { name: "levelCap", descriptor: "(Ljava/lang/Object;)I" };
    /// HZ-044. In place of the level-up's `if (n % 2 == 0) b++` in `ek.j(I)V` @109..129 (`this`): the
    /// points the new level grants.
    pub const SKILL_POINTS: Hook = Hook { name: "skillPoints", descriptor: "(Ljava/lang/Object;)V" };
    /// HZ-045. In place of `ar.a(Lar;IBB)V` in a boss part's `ak.a(Lar;I)V` @20 (the attacker, the part,
    /// a fifth of the damage, 0, 2): the reflection, a tenth, mitigated and capped.
    pub const BOSS_REFLECT: Hook = Hook { name: "bossReflect", descriptor: "(Ljava/lang/Object;Ljava/lang/Object;IBB)V" };
    /// HZ-057. In place of `invokevirtual cq.a(II)V` (a monster's own picture) in `cq.b(II)V` @207 (`this`,
    /// x, y on the screen): the picture, then — a boss in its reflect stance — the mirror coat over what
    /// it drew (`hz_java::boss`).
    pub const MONSTER_DRAW: Hook = Hook { name: "monsterDraw", descriptor: "(Ljava/lang/Object;II)V" };
    /// HZ-057. In place of `invokevirtual cq.k()V` (the monster's next move) in `cq.l()V` @128 (`this`): the
    /// move — and a boss that just picked its next attack (its cooldown `l.a:S` reloaded) may take the
    /// reflect stance.
    pub const MONSTER_THINK: Hook = Hook { name: "monsterThink", descriptor: "(Ljava/lang/Object;)V" };
    /// HZ-057. In place of `invokespecial cq.a(Lek;BIZBBZZZ)Z` (the player's blow landing) in the boss
    /// base's `ak.a(Lek;BIZBBZZZ)Z` @23 / @53 (`this`, the player, …): the blow, then — in the stance — a
    /// share of the HP it took back at the player, mitigated and capped.
    pub const BOSS_HIT: Hook = Hook { name: "bossHit", descriptor: "(Ljava/lang/Object;Ljava/lang/Object;BIZBBZZZ)Z" };
    /// HZ-047. In place of `bipush 10; idiv; imul` in `ek.a(Lar;IB)Z` @68..72 (전력방어's `dmg × (y / 10)`,
    /// the damage and `y` on the stack): the damage cut by the level's percent.
    pub const GUARD: Hook = Hook { name: "guard", descriptor: "(II)I" };
    /// HZ-047. In place of `ar.b(BI)V` in `ek.b(BB)V` @5 (`this`, 21, 80 — 전력방어's status and frames):
    /// the status for `4 + L` seconds.
    pub const GUARD_STATUS: Hook = Hook { name: "guardStatus", descriptor: "(Ljava/lang/Object;BI)V" };
    /// HZ-054. In place of `dd.a(Lcw;III)V` in `ce.f()V` @40 — the target-monster panel's picture:
    /// `dd`, the picture, x, y, anchor. Draws it as the game does, `GuestState::target_drop` guest px
    /// lower (0 = the original).
    pub const TARGET_PICTURE: Hook = Hook { name: "targetPicture", descriptor: "(Ljava/lang/Object;Ljava/lang/Object;III)V" };
    /// HZ-054. In place of `dd.a(IIII)V` (`fillRect`) in `ce.f()V` @83 — the panel's HP bar: `dd`, x,
    /// y, width, height, as lower.
    pub const TARGET_BAR: Hook = Hook { name: "targetBar", descriptor: "(Ljava/lang/Object;IIII)V" };
    /// HZ-059. In place of `ed.b(B)V` (the field's walk) where `ce.a(II)Z` @513 / 520 / 527 / 534 takes an
    /// arrow (1 ↑ 2 ↓ 3 ← 4 →): the walk in the diagonal the host holds (5 ↖ 6 ↗ 7 ↙ 8 ↘) when the arrow
    /// is one of its two — the walk's start and turn, or an attack's buffered turn — else the arrow as it was.
    pub const WALK: Hook = Hook { name: "walk", descriptor: "(B)V" };
    /// HZ-059. In place of `ed.a(ZZ)V` (the camera's follow) in `ce.d()V` @14: facing a diagonal, the
    /// game's follow once for each of its two arrows (its own table `ed.a:[Z` has 1..4 only).
    pub const CAMERA: Hook = Hook { name: "camera", descriptor: "(ZZ)V" };
    /// HZ-059. In place of `ar.c(B)V` (the facing) in `ed.b(BB)V` @11 (the player, the direction — the
    /// state was set at @4): a diagonal only while walking (state 2), else its horizontal arrow; and a
    /// side-step's way back (`ar.e:B`) dropped, so `ar.w()V` cannot turn the chosen direction back.
    pub const FACE: Hook = Hook { name: "face", descriptor: "(Ljava/lang/Object;B)V" };
    /// HZ-059. In place of `g.a(Lar;B)Z` (the next tile open) in the step's check `ar.e()Z` @24 (the map,
    /// the walker, its facing): a diagonal is open only when its two arrows' tiles are too — no squeezing
    /// between two walls that meet at a corner.
    pub const PASSABLE: Hook = Hook { name: "passable", descriptor: "(Ljava/lang/Object;Ljava/lang/Object;B)Z" };
    /// HZ-059. In place of `ar.c(B)Z` (the game's side-step past a corner, its tables 1..4 only) in
    /// `ar.f()Z` @5 (the walker, its facing): a blocked diagonal slides a tile along its open arrow, then
    /// goes on diagonally (the game's own restore, `ar.e:B` → `ar.w()V`).
    pub const SLIDE: Hook = Hook { name: "slide", descriptor: "(Ljava/lang/Object;B)Z" };
    /// HZ-059. In place of `getfield ar.c:B` (the facing) in the player's picture `ek.b(III)V` @7 (`this`):
    /// the animation's direction — the sheets hold four, so a diagonal draws as its horizontal arrow.
    pub const DRAW_FACING: Hook = Hook { name: "drawFacing", descriptor: "(Ljava/lang/Object;)B" };
    /// HZ-060. In place of `getfield ba.b:B` (the potion's number the drink switches on) in `ek.a(Z)Z` @62
    /// (the item): 「엘릭서」 fills HP and SP and answers [`ELIXIR_DRINK`]; any other item its own number.
    pub const POTION: Hook = Hook { name: "potion", descriptor: "(Ljava/lang/Object;)I" };
    /// HZ-061. In place of `ed.c(IB)V` (a kill counted: `ed.ae[kind]` down by one, at zero the
    /// script group started with `eb.a(B)Z`) where a monster turns dead, `cq.a(B)V` @44 (its kind,
    /// its group): a boss room's last kill spends the counter the same way and its story waits five
    /// seconds (`hz_java::story`); any other kill is the game's call.
    pub const KILL_COUNT: Hook = Hook { name: "killCount", descriptor: "(IB)V" };
    /// HZ-062. In place of `bt.b(BBI)Z` (take `n` of category / number off the bag, walking it from its
    /// END, the worn skipped) where the shop sells, `co.a(BB)V` @353 (one) / @467 (`n` of a stack) — with
    /// the `getfield ba.a:B` before the number nopped, so the item the cursor chose is on the stack in
    /// its place (the bag, the chosen item, its number, `n`): the chosen item first, any rest the game's way.
    pub const SELL_TAKE: Hook = Hook { name: "sellTake", descriptor: "(Ljava/lang/Object;Ljava/lang/Object;BI)Z" };
    /// HZ-063. In place of `ed.a(I)Z` (is story flag `n` on) where a map script's condition 7 tests its
    /// flags, `eb.a(BLn;)Z` @421: the flag — but [`KARMEN_AWAY`] only when the story can have set it
    /// (`hz_java::story::karmen_away_real`), never because 「수상한 남자」 was turned down.
    pub const SCRIPT_FLAG: Hook = Hook { name: "scriptFlag", descriptor: "(I)Z" };
    /// HZ-063. In place of `ed.c(I)V` (story flag `n` on) in script action 53, `eb.b(BLn;)Z` @2836: the
    /// game's call — but [`KARMEN_AWAY`] is left off when the story cannot have set it.
    pub const SCRIPT_FLAG_ON: Hook = Hook { name: "scriptFlagOn", descriptor: "(I)V" };
    /// HZ-064. In place of `l.b(I)V` (the target held `n` frames) where 조준사격 (`ai.a(BI)Z` @330) and
    /// 저격 (@891) aim (the target, 14): the hold — unless the target never walks (`l.a:I` −1, the mark
    /// its AI needs to keep choosing attacks), which is left as it is.
    pub const AIM_HOLD: Hook = Hook { name: "aimHold", descriptor: "(Ljava/lang/Object;I)V" };
    /// HZ-065. In place of `rpg/b.a([BB)Z` (the slot's progress record, `HOL.gamedata<n>` — the quests'
    /// states are in it) in `ed.k()V` @143: the game's write, then — with rewards waiting — that record
    /// read back; on the disk, the rewards go into the 계정창고 and its record is written and read back
    /// too (code review 2026-11-03: before the slot's records, a storage written while the progress was
    /// refused left a reward on the disk the quest could earn again).
    pub const SLOT_SAVE: Hook = Hook { name: "slotSave", descriptor: "([BB)Z" };
    /// HZ-065. In place of `rpg/b.b([B)Z` where `dz.i()V` @138 writes the 계정창고 (its payload): the
    /// payload kept for that read-back, then the game's write (which answers `true` even when the record
    /// store refused it).
    pub const STORAGE_WRITE: Hook = Hook { name: "storageWrite", descriptor: "([B)Z" };
    /// HZ-066. In place of `bt.a(BBI)Z` (room for `n` of category / number) where 계정창고's 「넣기」
    /// tests the storage page, `eg.a(BB)V` @95 (the page, category, number, `n`): the game handed
    /// `dz.a[eg.g]` — the CHARACTER side's tab (0 장비 · 1 조합 · 2 특수), so equipment only ever went to
    /// page 1 — and the hook tests the 계정창고 page [`STORAGE_PUT`] will put into instead
    /// (`hz_java::bags::deposit_page`).
    pub const STORAGE_ROOM: Hook = Hook { name: "storageRoom", descriptor: "(Ljava/lang/Object;BBI)Z" };
    /// HZ-066. The same for `bt.a(Lba;I)I` (the add) @145 (the page, the item, `n`): into that page.
    pub const STORAGE_PUT: Hook = Hook { name: "storagePut", descriptor: "(Ljava/lang/Object;Ljava/lang/Object;I)I" };
    /// HZ-067. In place of `cq.a(Lar;I)V` — the one call that takes HP off a monster — in `cq.a(Lar;IBB)V`
    /// @84 (every landed hit: the player's, a spirit's, 반격's reflection, the clone's, a monster's on the
    /// clone) and `cq.b(Lbc;)V` @103 (독수's poison tick, attacker null): the monster, the attacker, the
    /// damage. The hook calls the game's method and gives the wearer of a draining 85제 piece
    /// (`hz_java::effects::drain_pct`) a share of what really came off.
    pub const MONSTER_HURT: Hook = Hook { name: "monsterHurt", descriptor: "(Ljava/lang/Object;Ljava/lang/Object;I)V" };
    /// HZ-068. In place of `getstatic rpg/e.w:Lcw;` in 캐릭터 `dy`'s stat name `dy.a(Ldd;IIB)V` @19: the four names'
    /// sheet with 의지 for 지력 (`hz_java::statname`), the game's own if it cannot be made.
    pub const STAT_NAMES: Hook = Hook { name: "statNames", descriptor: "()Ljava/lang/Object;" };
    /// HZ-063: story flag 48 — set by the story (map 2, list 1: Karmen spent, 「결전」 begins) and, by
    /// the same number, by turning down 「수상한 남자」 (map 4, list 34).
    pub const KARMEN_AWAY: i32 = 48;
    pub const ALL: [Hook; 128] = [
        EXP,
        DROP_THRESHOLD,
        AUTO_LOOT,
        OPT_LABEL,
        OPT_VALUES,
        OPT_KEY,
        BLIT,
        CASH_FREE,
        TEXT,
        MENU_LABEL,
        OPEN_STORAGE,
        STORAGE_CLOSED,
        TARGET_X,
        STORAGE_HELP,
        ITEM_RECORD,
        ITEM_ICON,
        QUICK_ITEM,
        DEATH,
        ANSWER,
        MONSTER_INIT,
        RESPAWN_DELAY,
        NOTICE_BOTTOM,
        GAMEDATA_STORES,
        ITEM_STORES,
        SLOT_STORES,
        SLOT_KEY,
        SLOT_TITLE,
        SLOT_BOX,
        SLOT_LABEL,
        SAVE_COUNT,
        SHOP_GOLD,
        LEVEL_UP,
        EXP_TABLE,
        STAT_NOW,
        STAT_MAX,
        GEAR_RECORD,
        ORB_GEAR_RECORD,
        RARE_DROP,
        PICKUP_NOTICE,
        FILL_INTS,
        MASK_ALPHA,
        GUARDIAN_MONSTER,
        MONSTER_ROUND,
        QUEST_MONEY,
        QUEST_EXP,
        ROUND_TEXT,
        LEGEND_RECORD,
        GRADE_COLOR,
        GRADE_NAME,
        ITEM_INFO,
        MONSTER_DROP,
        HIT_DAMAGE,
        CRIT_CHANCE,
        TAKE_DAMAGE,
        MONSTER_EXP,
        SKILL_SP,
        CAST_START,
        JOB_COOLDOWN,
        SPIRIT_COOLDOWN,
        FUSION_START,
        CLONE_LIFE,
        MARK_STATUS,
        COUNTER_STATUS,
        SPIRIT_COST,
        SPIRIT_POWER,
        COMMON_DROP,
        DROP_ITEM,
        QUEST_ITEM,
        QUEST_GIVE,
        COUNTER_REFLECT,
        SHIELD_BASH,
        SMASH,
        FRENZY_COST,
        SNIPE_CHANCE,
        POISON_CHANCE,
        BLAST_CHANCE,
        POISON_DAMAGE,
        KNOCK,
        KNOCKABLE,
        DRANK,
        MOVE_STEP,
        BOX_RECIPE,
        BAG_AT,
        BAG_SIDEWAYS,
        BAG_UPDOWN,
        BAG_GRID,
        BAG_GRID_BZ,
        BAG_CELLS,
        BAG_TAKE,
        BAG_TAKE_SOME,
        TABLE,
        ATTACK,
        DEFENCE,
        HP_BONUS,
        SP_LEVEL,
        SPIRIT_CAST_END,
        CRIT,
        EVASION,
        LEVEL_CAP,
        SKILL_POINTS,
        BOSS_REFLECT,
        MONSTER_DRAW,
        MONSTER_THINK,
        BOSS_HIT,
        GUARD,
        GUARD_STATUS,
        TARGET_PICTURE,
        TARGET_BAR,
        QUICK_REGISTER,
        QUICK_POTION,
        WALK,
        CAMERA,
        FACE,
        PASSABLE,
        SLIDE,
        DRAW_FACING,
        POTION,
        KILL_COUNT,
        SELL_TAKE,
        SCRIPT_FLAG,
        SCRIPT_FLAG_ON,
        AIM_HOLD,
        SLOT_SAVE,
        STORAGE_WRITE,
        STORAGE_ROOM,
        STORAGE_PUT,
        MONSTER_HURT,
        STAT_NAMES,
    ];

    /// HZ-036 (user 2026-10-28: 28 → 112): what each of the player's three bags holds, shown a page of
    /// [`BAG_PAGE`] (the game's 7 × 4 grid) at a time. Under 128 — the save writes a bag's count as one
    /// signed byte (`bt.a()[B`), and the windows keep a cell's place in a byte.
    pub const BAG_SIZE: i8 = 112;
    pub const BAG_PAGE: i8 = 28;
    pub const BAG_PAGES: u8 = (BAG_SIZE / BAG_PAGE) as u8;
    /// HZ-037: 계정창고 (`HOL.storage`) — pages of 28, the game's 84 = three. Its count is a signed byte too.
    pub const STORAGE_SIZE: i8 = 112;
    /// HZ-037: its pages — the load makes `(size − 1) / 28 + 1`, and the save walks them all.
    pub const STORAGE_PAGES: u8 = (STORAGE_SIZE / BAG_PAGE) as u8;
    /// HZ-038: the most a stack holds (`ba.c:B`, a byte — the game's 99).
    pub const STACK_MAX: i8 = 127;
    const _: () = assert!(BAG_SIZE % BAG_PAGE == 0 && STORAGE_SIZE % BAG_PAGE == 0 && STACK_MAX > 99);
    // The page loops' bound is one `iconst_n` (n ≤ 5), the game's `iconst_3`.
    const _: () = assert!(STORAGE_PAGES > 3 && STORAGE_PAGES <= 5);

    /// HZ-035: what the ground list takes (`g.a(BBBB)V`) — the 85제 recipes are `/itm/18` 28..123 (the
    /// first draft's 36 were 28..63; the second draft's 60 go on to 123, `hz_java::legend::COUNT`). The
    /// game's item numbers are signed bytes, so 127 is the end of the road.
    pub const RECIPE_GROUND_TOP: i8 = 127;

    /// HZ-031: the 85제 pieces' grade — a fifth after the game's 일반 · 레어 · 에픽 · 세트 (0..3), named
    /// 「전설」 by the host. The bulk sale (HZ-021) lists 0..3 only, so it never sells them.
    pub const LEGEND_GRADE: i8 = 4;
    /// HZ-031: 「전설」's name colour (orange gold; the game's are white, yellow, pink and green).
    pub const LEGEND_COLOUR: i32 = 0xFF_A0_20;
    /// HZ-031: the 85제 pieces' numbers start here in each equipment file (HZ-023's guardian piece is
    /// [`GEAR_ID`]).
    pub const LEGEND_FIRST_ID: i8 = 41;
    /// HZ-031: what the ground list takes (`g.a(BBBB)V`; the game's 28, HZ-023's 40).
    pub const GROUND_TOP_ID: i8 = 50;

    /// HZ-030: 가디언 모드 — the game's replay round `ed.h` after 헬 (0 노말 · 1 하드코어 · 2 헬). The ending
    /// bumps the round up to this and no further.
    pub const GUARDIAN_ROUND: i8 = 3;

    /// HZ-021: the bulk sale's grade list (the game's list popup `ee`) and its 에픽-and-up question
    /// (`cf`) answer their parent, the shop, with these tags — set on them after they are made
    /// (`ey.f`), so [`ANSWER`] knows them from the shop's own answers (it answers by its state
    /// `co.g`, with −1 / −2 from its count box; `ee`'s own tag is 6).
    pub const BULK_GRADE: i8 = 77;
    pub const BULK_ASK: i8 = 78;
    /// HZ-021: the 「일괄판매」 button, `(x, y, w, h)` guest px from the shop window's frame — the
    /// right of its title band, which the title 「아이템 판매」 (centred) leaves empty.
    pub const BULK_BUTTON: (i32, i32, i32, i32) = (112, 1, 46, 12);

    /// HZ-020: the save slots (user 2026-10-25: 3 → 10). The game keeps three (`rpg/b`'s store
    /// names, its three summaries `ed.a:[[B`, the slot window's three rows); the rest are pages of
    /// three — slot `n` (1-based) is page `(n − 1) / 3`, the game's slot `(n − 1) % 3`.
    pub const SAVE_SLOTS: u8 = 10;
    pub const SLOTS_PER_PAGE: u8 = 3;
    pub const SLOT_PAGES: u8 = SAVE_SLOTS.div_ceil(SLOTS_PER_PAGE);

    /// HZ-020: how many slots page `page` has (the last one fewer).
    pub const fn page_slots(page: u8) -> u8 {
        let first = page as u16 * SLOTS_PER_PAGE as u16;
        if first >= SAVE_SLOTS as u16 {
            0
        } else if SAVE_SLOTS as u16 - first < SLOTS_PER_PAGE as u16 {
            (SAVE_SLOTS as u16 - first) as u8
        } else {
            SLOTS_PER_PAGE
        }
    }

    /// HZ-019: guest px the pickup notices rise by over the host's EXP bar (`hz_android::hud_view`:
    /// margin 3 + bar 4 hud px, a hud px up to 5/4.5 of a guest px on a 1080-high screen) — and a
    /// little air. `hz_android` checks it against the bar.
    pub const NOTICE_LIFT: i32 = 9;

    /// HZ-025: a pickup notice's count `g.a:[I` the first time `g.c()V` draws it — `g.a(BBBZ)V` sets
    /// 15 and the painter counts down before it draws.
    pub const PICKUP_FIRST_COUNT: i32 = 14;

    /// HZ-016: 「부활의 구」 — the special bag's (category 20) tenth item. `/itm/20` holds nine (0..8,
    /// all sold by the shop's `a(20, 8)`), so number 9 is free and the shop's loop reaches it.
    pub const REVIVE_ORB: (i8, i8) = (20, 9);
    /// HZ-016: its picture index `cat + id` in `rpg/e.k` (`ba.a(dd,IIZZ)V`) — 29, the pack's last
    /// picture, the selection frame the bag draws over a chosen item.
    pub const REVIVE_ORB_ICON: i32 = REVIVE_ORB.0 as i32 + REVIVE_ORB.1 as i32;
    /// HZ-016: the `cf` question's `f` byte (handed back as `parent.a(f, row)`). Not 3, so RSK is no
    /// answer (`cf`'s ctor sets `g` only for 3) and not 4, so the cursor starts on 예.
    pub const REVIVE_ASK: i8 = 5;

    /// HZ-060: 「엘릭서」 — the special bag's eleventh item, after HZ-016's orb (number 9), so the shop's
    /// loop over the tab reaches it as it does the orb.
    pub const ELIXIR: (i8, i8) = (20, 10);
    /// HZ-060: its picture index `cat + id` in `rpg/e.k` — 30, one past the pack's 30 pictures.
    pub const ELIXIR_ICON: i32 = ELIXIR.0 as i32 + ELIXIR.1 as i32;
    /// HZ-060: the number [`POTION`] answers for the elixir once it has filled HP and SP — 포션's (HP
    /// 100, nothing at full), so the drink goes on the game's way: one off the bag, `ed.i()V`, `drank`.
    pub const ELIXIR_DRINK: i32 = 2;

    /// HZ-023: the guardian pieces' number in every equipment file (`/itm/00`..`/itm/15`, which hold
    /// 29..32 — nothing reads an id past a file's end but the factory), so the drop needs no table.
    pub const GEAR_ID: i8 = 40;
    /// HZ-023: the equipment categories (`ba.a(BB)Lba;` makes a `bx` for 0..15).
    pub const GEAR_CATEGORIES: i8 = 16;
    /// HZ-023: the rare band's top number in `cq.o()V` (`min(lvl / 5 + 13, 28)`).
    pub const RARE_TOP: i32 = 28;

    /// HZ-014: the target-monster panel's width and its x on the original canvas (`width − 80`).
    pub const TARGET_PANEL_W: i32 = 80;
    /// HZ-014: its height (the 80×34 picture `ce.a:Lcw` at y 0).
    pub const TARGET_PANEL_H: i32 = 34;

    /// HZ-007 (one page since 2026-10-29): the rows the in-game window shows at once — its row
    /// count `o.b:B` from the constructor, so the list frame `m.a(dd,IIZ)` draws them; the frame's
    /// last slot is the line under them (「다음 실행부터 적용」). The list scrolls through the rest.
    pub const OPTIONS_ROWS: u8 = 9;
    /// HZ-007: the label loop's bound — every slot the list frame draws (`m.a(dd,IIZ)`: exactly 10).
    pub const OPTIONS_SLOTS: u8 = 10;
    const _: () = assert!(OPTIONS_ROWS < OPTIONS_SLOTS && OPTIONS_SLOTS <= 10);
}

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

// ------------------------------------------------------------------ builds ----

/// A build of the game this repository knows, identified by the md5 of the whole jar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Build {
    /// `영웅서기제로(최종버전).zip` → the one jar inside it. EA Mobile's China (QQ) port of
    /// 영웅서기 Zero 진홍의 사도 with the text translated back to Korean; MIDlet-Version
    /// 1.0.0, main class `rpg.RPGHeroEx`, 163 classes, 240×320, 27,334,365 B.
    Final,
}

impl Build {
    pub const ALL: [Build; 1] = [Build::Final];

    pub fn jar_md5(self) -> &'static str {
        match self {
            Build::Final => "850ad70748b9e1e48b30f6fa60164229",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Build::Final => "영웅서기 Zero 진홍의 사도 (최종버전, EA CN 1.0.0 한글)",
        }
    }

    /// `None` = not a build this repository has patches for. The host must then apply
    /// NOTHING — a class rewrite written for one build is a corruption on another.
    pub fn identify(jar: &[u8]) -> Option<Build> {
        let md5 = format!("{:x}", md5::compute(jar));
        Build::ALL.into_iter().find(|b| b.jar_md5() == md5)
    }
}

// ----------------------------------------------------------------- patches ----

/// Rewrites one class file. Given the original bytes (already md5-verified), returns the
/// new bytes, or `None` if something the rewrite expected is not where it should be — in
/// which case the original is defined unchanged and the miss is reported.
pub type Rewrite = fn(&[u8]) -> Option<Vec<u8>>;

#[derive(Clone, Copy, Debug)]
pub enum PatchKind {
    AppProperty { key: &'static str, value: &'static str },
    ClassRewrite { class: &'static str, class_md5: &'static str, rewrite: Rewrite },
    /// One change that lives in several classes, each gated by its own md5 (HZ-010: the cheat
    /// shop is reachable from four classes). Each target is applied — or refused — on its own.
    Classes(&'static [ClassTarget]),
}

/// One class of a [`PatchKind::Classes`] patch.
#[derive(Clone, Copy, Debug)]
pub struct ClassTarget {
    pub class: &'static str,
    pub class_md5: &'static str,
    pub rewrite: Rewrite,
}

impl PatchKind {
    /// Every class this kind rewrites, as `(class, md5, rewrite)`.
    pub fn targets(self) -> Vec<(&'static str, &'static str, Rewrite)> {
        match self {
            PatchKind::AppProperty { .. } => Vec::new(),
            PatchKind::ClassRewrite { class, class_md5, rewrite } => vec![(class, class_md5, rewrite)],
            PatchKind::Classes(targets) => targets.iter().map(|t| (t.class, t.class_md5, t.rewrite)).collect(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Patch {
    pub id: &'static str,
    /// Shown to the player (settings) — Korean, one line.
    pub title: &'static str,
    /// Why this exists and what it rests on: the guest code it relies on, how it was
    /// verified. For the next person, not the player.
    pub evidence: &'static str,
    pub builds: &'static [Build],
    pub default_on: bool,
    pub kind: PatchKind,
}

/// The table. Append only; ids are never reused.
pub const PATCHES: &[Patch] = &[Patch {
    id: "HZ-001",
    title: "캐시 아이템 무료 (SMS 결제 대신 게임의 QA 경로로 즉시 지급)",
    evidence: "ep.a(I)Z is the purchase send. Its first test reads field ep.c, which ep.<init> \
               fills from MIDlet.getAppProperty(\"EA-Porting-Test-SMS-Mode\") (trimmed, \
               lower-cased, null if empty). When it is non-null the method returns WITHOUT \
               sending any SMS: false if the value is \"false\", true otherwise. EA's QA hook, \
               left in the shipped build. With no carrier billing on this platform the real \
               path can never succeed, so without this every cash item is unreachable.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::AppProperty { key: "EA-Porting-Test-SMS-Mode", value: "true" },
}, Patch {
    id: "HZ-002",
    title: "네트워크 메뉴 「우편 보내기」 봉인 (빠진 화면 — 누르면 게임이 멈추던 것)",
    evidence: "ay.a(II)Z case 1 does `new dx(this, null)`, but dx.class is not in the jar (dw and                dy are). ay's labels are rpg/e.h = t/nui.tdf[29..31] (랭킹 등록 / 캐릭터 복구 / 우편함)                and dx's help line is t/h.tdf[14] (send items, money and a message to another                player): an online mail screen, removed with the server. Choosing it raised                NoClassDefFoundError on the game thread. Bytecode 143..155 (13 bytes, verified                exactly) becomes `goto 198` + 10 nops: the switch's common exit (iconst_1;                ireturn), so the key is consumed and nothing opens. Same length, so the CLDC                StackMap offsets stay valid.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "ay", class_md5: "a3602a2b62e620e575a3ad3ecdcb7ccb", rewrite: seal_mail_send },
}, Patch {
    // HZ-003 is taken: the QA-cheat app property (HEROZERO_REMAKE_PLAN.md §4), not built yet.
    id: "HZ-004",
    title: "경험치 배율 (추가 설정: 1·2·3·5배, 기본 1배 = 원본)",
    evidence: "ek.j(I)V is the ONLY method that adds EXP (ek = the player). Its three callers are \
               cq.n()V @23 (monster types 81..97, fixed 100), @75 (other monsters, \
               18*mLv*clamp(20+mLv-pLv,0,30)/20) and eb.b(BLn;)Z script opcode 20 (quest \
               rewards), so one site covers every source. Bytes 32..40 compute |amt| for the \
               `a[2] += ` at 42 (iload_1; ifge 41; iload_1; ineg; goto 42); they become iload_1; \
               invokestatic Hooks.exp(I)I; nop; nop; goto 42, and the hook returns |amt| x rate. \
               The cash 'EXP x2' (bh.i, persisted in HOL.gamestore) and the dead ey.f branch at \
               0..24 run first, untouched. Same length; StackMap frames 41/42 stay (41 becomes \
               unreachable). Multi-level-up (@44 loop) and the cap at 99 are the game's own.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "ek", class_md5: "a33b0a941d884487e83fe61b75e7d40e", rewrite: exp_rate },
}, Patch {
    id: "HZ-005",
    title: "아이템 드랍률 배율 (추가 설정: 1·2·3배, 기본 1배 = 원본)",
    evidence: "cq.o()V is the drop roll: ONE R = abs(ar.a.nextInt()) % 65535 decides everything. \
               R < 52428 drops money; then each of the monster's 4 /en/edf item slots (6 B each) is \
               tried in order and the first with R < threshold(slot) drops that item. The \
               threshold is read by rpg/g.a([BI)I at @87 (126 of 132 monsters: 4833 / 9666 / \
               14500, cumulative, ~7.4 % a slot). That invokestatic becomes Hooks.dropThreshold \
               ([BI)I = the same read x rate, capped at 65535, so every item slot's chance scales \
               and the rest of the roll is unchanged: nextInt is still consumed exactly once, and \
               R >= 52428 (20 %: nothing, not even money) is untouched. The generic band \
               (14500..52428: potions, materials, recipes, magic stones) shrinks by what the item \
               slots take. Bosses (99..131) never reach o(). Same length, no frame moved.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "cq", class_md5: "acbac4519692a62c390389354fb998ea", rewrite: drop_rate },
}, Patch {
    id: "HZ-006",
    title: "오토루팅 (추가 설정에서 켜면 주변의 돈·아이템을 자동으로 줍는다, 기본 끔)",
    evidence: "Ground items live in g.b:Ljava/util/Vector; as byte[6] {tileX, tileY, kind (1 = \
               money), cat | amount/100, id | amount%100, timer} and never expire. The field \
               tick ed.d()V @77 already calls g.a(BB)Z on the player's tile (ek.s, ek.t) every \
               frame — pickup is automatic, but only on that exact tile. That invokevirtual \
               becomes Hooks.autoLoot(Object,II)Z: the original call first; if it took nothing \
               and auto-loot is on, the nearest entry within a few tiles that is money or fits \
               the bag (ed.a(B)Lbt -> bt.a(BBI)Z, a read-only check — g.a's own refusal pops \
               「공간 부족」 every call) is picked by calling g.a on ITS tile. One a frame. Runs \
               on the game thread inside the world tick, so the Vector cannot change under it. \
               The following `pop` stays. Same length.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "ed", class_md5: "9b90d32b52e8ddcc4a90fb8b7ac2e610", rewrite: auto_loot },
}, Patch {
    id: "HZ-007",
    title: "환경설정 한 페이지 (게임 행 + 화면 방향·오토루팅·경험치 배율·드랍률·난이도·몬스터 리젠·조작 방식, 스크롤; *키 단축키 행 없음)",
    evidence: "The options window is bn (extends m -> b -> o), opened from the title (i.a(II)Z \
               case 4) and in game (ec.a()Z case 1). Its row count is the ctor's bipush 7 -> \
               o.b:B, which the cursor wraps on (o.b(B)) and the list frame m.a(dd,IIZ) fills \
               (10 slots, so at most 3 more rows). Eight same-length edits: ctor @2 and label \
               loop @104 bipush 7 -> 8 (row 7 = 「추가 설정」); title @70/@75 and label @113/@121 \
               `getstatic rpg/e.b:Lcx; ... invokevirtual cx.a(I)[B` -> `aload_0; nop; nop; ... \
               invokestatic Hooks.optLabel`, so the hook sees the window; @137..148 (value x = \
               rpg/a.c+37) -> Hooks.optValues + `dup; istore_3; iflt 482` — on the host page the \
               hook draws the values and the game's seven are skipped (482 = the method's tail, \
               o.a(dd,2); return. It has no StackMap frame: rustjava keeps StackMap as raw bytes \
               and never verifies, a CLDC verifier would object); bn.a(II)Z @3 `invokevirtual \
               o.b(II)Z` -> Hooks.optKey, which calls o.b first and then opens / closes / drives \
               the host page. The host page is the SAME window with o.b:B set to the host row \
               count, so the game's own frame, cursor bar and wrap draw it. Arg 1 of a(II)Z is the \
               game action (or -6/-7 for the soft keys), arg 2 the raw key (o.f/o.g/o.h/o.i). \
               2026-10-29 (user: '*키 단축키 제거, 추가설정에 있는거 따로 빼지않고 환경설정 한페이지에서 \
               스크롤로'): ONE page - the game's rows less *키 단축키 (rpg/c.d:B, the field's * key; \
               the SAVE and MAP tabs do both of its jobs) and the host's seven, scrolled through the \
               same window: ctor @2 bipush 7 -> 9 (the rows shown at once; the list frame's tenth \
               slot is the line under them), label loop @104 bipush 7 -> 10 (every slot). The hooks \
               keep the scroll; o.a:B stays the cursor's slot on screen, so the game's frame and its \
               cursor bar b(dd) (both from o.a:B) and the touch layer's row taps are unchanged. A \
               game row's LEFT / RIGHT runs the game's own row code (the hook re-enters bn.a(II)Z \
               with o.a:B set to the game's row for that one call), so its music stop, vibration and \
               effect preview are the game's.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "bn", class_md5: "a9d8789baadebded3c0e63307ac8f528", rewrite: extra_options },
}, Patch {
    id: "HZ-008",
    title: "터치 HUD 가 게임 장면을 안다 (필드에서만 원호 키, 메뉴·대화에서는 숨김)",
    evidence: "The game draws into its own back buffer dd.a and the canvas f.paint(Graphics) @245 \
               puts it on screen with `invokevirtual Graphics.drawImage(Image,III)V` — the one \
               call every drawn frame goes through (javap + classbytes.py 2026-10-22; StackMap \
               frames at 231 and 248, none inside). That call becomes Hooks.blit(Object,Object,III)V, \
               same operands, same length; the hook draws the image exactly as the game did and \
               then reads what the HUD needs: the mode byte ed.d:B (ce.a(II)Z routes keys on it: \
               2 = title window i, 3 = the field — the only case with number keys, 4 = scenario \
               eb, 5 = menu bh, 8 = yes/no, 11 = game over, 12 = ct), the message-box queue \
               ax.a:Vector (keys go to ax while it is non-empty) and the 「# 확인」 notice ed.f:Z \
               (ce.a(II)Z @14: `#` dismisses it in any mode). Read only while the canvas's screen \
               f.a:Lbj; is the game screen ce (its ctor sets ed.d), never during the logo ch, so \
               no class is initialised early. Changes nothing the game does.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "f", class_md5: "1a18bda6c8a895897d7393ea1877a1f8", rewrite: scene_blit },
}, Patch {
    id: "HZ-009",
    title: "(철회) 캐시 무료 토글 — 치트상점이 HZ-010 으로 사라져 닿지 않는다; 훅은 늘 지급",
    evidence: "ep.a(I)Z @40..62 (23 bytes, verified exactly) is the QA-property test HZ-001 drives: \
               `aload_0; getfield ep.c; ifnull 63; aload_0; getfield ep.c; ldc \"false\"; \
               String.equals; ifeq 61; iconst_0; ireturn; (61) iconst_1; ireturn` — with the \
               property set the method returns here and never reaches the SMS path at 63. The \
               block becomes `invokestatic Hooks.cashFree()Z; ireturn` + 19 nops, so the host's \
               「캐시 무료」 row answers in its place: on = granted (the property's `true`), off = \
               refused (`false`). Same length; the StackMap frames at 61 and 63 stay where they are \
               (61 is now dead, 63 unreachable — rustjava keeps the map as raw bytes). The property \
               patch HZ-001 stays on underneath: with HZ-009 off the game reads it as before.",
    builds: &[Build::Final],
    default_on: false,
    kind: PatchKind::ClassRewrite { class: "ep", class_md5: "91895206afda037583408eef26ecaf68", rewrite: cash_free_toggle },
}, Patch {
    id: "HZ-010",
    title: "치트상점 제거 — 타이틀 항목 · 필드 RSK STORE · 돈·재료가 모자랄 때와 죽었을 때의 결제창 (메뉴 쪽은 HZ-012 가 행을 당긴다)",
    evidence: "ey.f:Z is the port's `network build` flag: never true here (static init false, rpg/b.a \
               @83..96 stores 0 on BOTH branches, eb clears it) and read at 18 sites. Where it is \
               false this build opens the cheat shop (ci, cash items through SMS — free with HZ-001); \
               where it is true it does what the QQ network build shipped without one. Cheat-shop \
               sites: i.a(II)Z @42 skips carousel index 2 (치트상점) when true; ce.a(II)Z @278 makes \
               the field's RSK a no-op instead of ed.a(6,3) (the store) and ce.f()V @375 draws the \
               softkey art without STORE (rpg/e.k/l images, null-checked); the menu's four sites \
               (bh.<init>, bh.a(II)Z x2, m.a) are HZ-012's, which removes the row instead of greying \
               it. And the shop opened FOR a sale (2026-10-24; first filed here as network economy): \
               the player's death ek.C()V @512 — state 7, then bh.c(3) and `(ci) bh.b` .a() = the \
               「즉시 부활」 purchase; true = cc.f(); ed.a(9), the game over a refusal leads to (probe: \
               「게임 오버」 -> title, nothing written). Since HZ-012 rotated the menu, bh.c(3) opens \
               row 0 = 캐릭터 and that cast threw ClassCastException: the player lay dead with an \
               empty menu (S9 2026-10-24, probe --hit). Not enough gold — the shop co.a(BB)V @138, \
               et.a(BB)V @59 and a(II)Z @301, bi.a(BB)V @88 and @229 — opened ci on 「20000 \
               게임머니」 (ci.g()); true = the game's own message (bz.a = the window's text, or a cd \
               popup). Missing materials at the alchemist dv.a(II)Z @1100 (ifne) opened 「빠른 합성」; \
               true = its own path @1690. The other readers stay false: ek.j @6 (network EXP, \
               HZ-004 runs past it), g.a()V @94 (guide special cases), eb.a(II)Z @387 (a network \
               prompt), rpg/b.a @74. Each site's `getstatic ey.f` (b2 cp cp) becomes `iconst_1; nop; \
               nop` — same length, the branch right after it reads a constant true; no StackMap \
               frame moves. Verified bytes (classbytes.py): i #50, ce #105, ek #159, co #48, et #45, \
               bi #39, dv #58.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::Classes(&[
        ClassTarget { class: "i", class_md5: "286944bcd3bc0a761142082bf8d1a1bf", rewrite: cheat_shop_title },
        ClassTarget { class: "ce", class_md5: "0feea535bec17058182df9c45fc1cf3c", rewrite: cheat_shop_field },
        ClassTarget { class: "ek", class_md5: "a33b0a941d884487e83fe61b75e7d40e", rewrite: cheat_shop_revive },
        ClassTarget { class: "co", class_md5: "c1a068f6d08e67c7ef490de32f4fa6c8", rewrite: cheat_shop_gold_co },
        ClassTarget { class: "et", class_md5: "135b634d43c74e31e46adf319c796f5e", rewrite: cheat_shop_gold_et },
        ClassTarget { class: "bi", class_md5: "6476a4d7f4e98b03fee2b1e5dd42bcbd", rewrite: cheat_shop_gold_bi },
        ClassTarget { class: "dv", class_md5: "85198deab2c8ef577488bbec7de405eb", rewrite: cheat_shop_synthesis },
    ]),
}, Patch {
    id: "HZ-011",
    title: "공용창고를 「계정창고」로 — 네트워크 표현을 계정창고로 (문구만; 저장은 게임의 HOL.storage 그대로)",
    evidence: "The title's 공용창고 (m[3]) is the game's own slot-shared store: dz.f (money) and \
               dz.a[3] (bags) persist through rpg/b.b([B) into RMS HOL.storage (probe 2026-10-22: \
               `HOL.storage add #1 (13 B)` at the title, no network call anywhere in dz/w/bg/eg). \
               Only its wording is the QQ build's: nui[0] 네트워크창고, [20] 네트워크로 올리기, [21] \
               싱글로 내리기, [23] 네트워크보유액, [24] 싱글보유액. Every table entry is read through \
               cx.a(I)[B (10 bytes: aload_0; getfield a; iload_1; aaload; checkcast [B; areturn), which \
               becomes `aload_0; iload_1; invokestatic Hooks.text; areturn` + 4 nops: the hook does the \
               same lookup and swaps those exact byte strings (hz_java::text::rename) for 계정창고 \
               wording. Any other entry comes back untouched, so the patch is invisible elsewhere. \
               The one-time help before the storage (「공용창고에는 … 1,3 키로 개인창고와 공용창고를 \
               전환합니다.」, flag ed.i in HOL.gamestore) is no table entry: it is rpg/d.J, which \
               rpg/d.<clinit> assigns twice — the Chinese build's dot-font glyph codes, then the \
               Korean EUC-KR bytes one bipush at a time (268 B), so no search of the jar's files \
               finds it (2026-10-24). Its two readers, bb.<init>(Lo;B)V @149 (the title flow, mode 3) \
               and de.a(II)Z @136 (in game: 상점및창고 → 창고, prepended to rpg/e.a[28]), are \
               `getstatic rpg/d.J:[B`; each becomes invokestatic Hooks.storageHelp()[B (same length, \
               same stack effect), which reads rpg/d.J and swaps it only when the bytes are exactly \
               that paragraph (공용창고 → 계정창고, 개인창고 → 캐릭터창고 as the item window's own side).",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::Classes(&[
        ClassTarget { class: "cx", class_md5: "87fb3e964b083aa1d71e457f45cb555d", rewrite: text_rename },
        ClassTarget { class: "bb", class_md5: "698c1dc125db3660962f5fec7a5807a3", rewrite: storage_help_title },
        ClassTarget { class: "de", class_md5: "6297333e39dae1318b785fd86d467d11", rewrite: storage_help_game },
    ]),
}, Patch {
    id: "HZ-012",
    title: "메뉴에서 계정창고 — 치트상점 행을 빼고 1~8 을 당긴 뒤 9번 「계정창고」, 0번 시스템 (닫으면 게임의 퀵 세이브)",
    evidence: "The in-game menu bh (10 rows, labels gm[0..9], actions bh.d(B)V's tableswitch 0..9: \
               0 치트상점 ci, 1 캐릭터 dy, 2 인벤토리 ds, 3 상점및창고 de, 4 장비 em, 5 스킬 ad, 6 정령 cr, \
               7 퀘스트 ao, 8 미니맵 ed.a(3,8,3), 9 시스템 ec). User (2026-10-22): drop 치트상점, move the \
               rest up one, put 계정창고 at 9 above 시스템 at 0. Done in place, every site same length: \
               (1) bh.d(B)V — the switch's ten 4-byte targets rotate (new r = old r+1 for r<8, new 8 = \
               old 0's block, 9 as is) and old case 0's block @177..202 (new ci + telemetry, 25 B) \
               becomes `aload_0; invokestatic Hooks.openStorage; return` + nops: the hook sets the \
               menu's child to `new bg(menu)`, the game's own 돈/아이템 chooser (what the title's \
               계정창고 opens after a slot is picked; its windows w/eg trade with the live character \
               and persist the store through dz.i() → HOL.storage). (2) bh.a(II)Z — digit shortcuts: \
               '7'→'6' for the 정령 lock (@24), '4'→'3' for the 상점 lock (@82), the ey.f/'1' test \
               @49 reads a constant false; cursor skips: 6→5 (@193), 3→2 (@266), the ey.f test @232 \
               false; bh.<init> @7 false (cursor starts on 캐릭터). (3) bh.c(B)V mode 1 (장비 from the \
               quick key) 4→3 (@29, @34). (4) m.a(dd,IIZ)I — the locked grey rows 3→2 (@331), 6→5 \
               (@343), ey.f @323 false; and both label reads @374/@612 `getstatic bh.a … cx.a(I)` \
               become `aload_0 … Hooks.menuLabel(this, index)`: for bh (base 0; bn/de/ec pass -1/71/49 \
               and are not bh) row r → gm[r+1], 8 → 「계정창고」, 9 → gm[9]. (5) o.a(dd,ZZZZ)V @6..14 — a \
               child's title is gm[bh.f]; same hook with `this` = the child, shifted when bh is \
               anywhere up its o.a parent chain — the equipment window's bag grid bv (bh → em → bv) \
               and the equipment window from the inventory (bh → ds → em) are grandchildren (2026-10-24: \
               with the parent alone the grid's title read 「상점 및 창고」 = gm[3]). (6) bg.a(II)Z @19..25 — RSK's `parent.c()` becomes Hooks.storageClosed(this): \
               parent.c(), then, if the parent is bh, bh.c(2) = the game's quick save (시스템 → \
               「저장하시겠습니까?」, the '*' quick-save path), so a trade is never left unsaved on \
               the slot side while HOL.storage already has it. (7) The children put the menu's \
               cursor back on their OWN row as they close (a(II)Z, before parent.c(): `aload_0; \
               getfield o.a:Lo; <row>; putfield o.a:B`): dy 캐릭터 1 (@19, @126), de 상점및창고 3 \
               (@33), em 장비 4 (@43, only when its parent is bh — the inventory ds opens it too), ad \
               스킬 5 (@47), cr 정령 6 (@38), ao 퀘스트 7 (@27); ec 시스템 keeps 9. bh is their only \
               parent. Each row moves up one like the labels — without it the cursor came back one \
               row low (S25 2026-10-23: 장비 → 스킬, 스킬 → the grey 정령). Verified bytes at every \
               offset (classbytes.py); the switch pad and frames are untouched. bh.c(3), the cheat \
               shop's mode, now opens row 0 = 캐릭터: its callers are the field's RSK STORE and the \
               death's revive purchase ek.C(), which casts the child to ci — both closed by HZ-010, \
               so HZ-012 without HZ-010 throws ClassCastException when the player dies. (8) The \
               blinking unspent-point marks (2026-10-27 — until then they blinked beside the wrong \
               rows: user report, probe frame of 2. 인벤토리 / 6. 정령 / 7. 퀘스트): m.a(dd,IIZ)I, under its \
               last flag (true only for bh — bn/de/ec pass false), draws the red stat mark rpg/e.c[0] \
               on row 1 while ek.a:S > 0 (@452 iconst_1) and the blue skill mark rpg/e.c[1] on row 5, \
               or row 6 while 정령 is open bh.e, while ek.b:S > 0 (@512 iconst_5, @516 bipush 6) — the \
               old rows of 캐릭터, 스킬 and 정령; they read 0, 4 and 5 like the labels.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::Classes(&[
        ClassTarget { class: "bh", class_md5: "ec7ee5341021765156cd5aee38cc1833", rewrite: menu_storage_bh },
        ClassTarget { class: "m", class_md5: "859120493326a182ea87e4631d486a74", rewrite: menu_storage_m },
        ClassTarget { class: "o", class_md5: "156653192e287c925a86054d8e15e1d2", rewrite: menu_storage_o },
        ClassTarget { class: "bg", class_md5: "2b56d31d25837e85783ba1892c01b3ff", rewrite: menu_storage_bg },
        ClassTarget { class: "dy", class_md5: "a0ce6f6b80edcff98979083bdf858341", rewrite: menu_row_dy },
        ClassTarget { class: "de", class_md5: "6297333e39dae1318b785fd86d467d11", rewrite: menu_row_de },
        ClassTarget { class: "em", class_md5: "dc75efad1ead6257ef39791ff66cbffd", rewrite: menu_row_em },
        ClassTarget { class: "ad", class_md5: "8f725b96e93f34219cb13260474513a7", rewrite: menu_row_ad },
        ClassTarget { class: "cr", class_md5: "0bb80b5b307e7dda4a237383cd783f7a", rewrite: menu_row_cr },
        ClassTarget { class: "ao", class_md5: "ab0639648d83f35a92576b2414905128", rewrite: menu_row_ao },
    ]),
}, Patch {
    id: "HZ-013",
    title: "게임 하단 바를 끄고 필드 시야를 화면 끝까지 (상태 블록·퀵슬롯 플레이트·포인트 배지·CLR 이 대신한다)",
    evidence: "The field's HUD is ce.f()V, called by the field paint ce.d() after the map: @0..104 the \
               target monster panel (top right, ce.a:Lcq + its 80x34 image, kept), @107.. the bottom \
               bar at y0 = rpg/a.b - 33: two blinking status icons above it (ek.a:S, ek.b:S), the \
               panels with gold ed.e:I and level ek.a()B, the HP/SP/EXP/MP bars and the 4 quick \
               slots (ed.b:[B, cooldown ed.a:[I / ed.b:[I) — all redrawn only on dirty flags \
               (ce.c:Z, ce.a:[Z), so the map cannot simply be let under it. The world viewport is \
               ce.h = rpg/a.b - 33 (ce.<init> @113; camera centre ce.j = h/2; read by the map g.c, \
               culling cq/bu/h.b and en). Same-length edits: ce.<init> @116 bipush 33 -> bipush 0 \
               (viewport = whole canvas, as hero2's viewport_full_patch); ce.f @107 getstatic \
               rpg/a.a -> goto 1091 (the method's tail: clip back to the viewport, ce.c = false, \
               return; StackMap frames at 107 and 1091 both exist); g.a(dd) @160 (the full-screen \
               window's black fill) and eb.a(dd) @61 / @536 (the scenario's grid and centred box, \
               measured from the viewport) bipush 33 -> 0. The scenario's bottom box is at \
               rpg/a.b - 37 already. Bytes verified with classbytes.py (2026-10-23). ON by \
               default since Phase 6-4 (2026-10-23): the host draws everything the bar showed \
               (hz_android: status block, quick-slot plates with the game's own icons, the \
               unspent-point badges, CLR for the field's menu); probe A/B of the field, the \
               minimap window (mode 8) and the opening scenario, landscape and portrait.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::Classes(&[
        ClassTarget { class: "ce", class_md5: "0feea535bec17058182df9c45fc1cf3c", rewrite: hud_off_field },
        ClassTarget { class: "g", class_md5: "4068de457fd524cea9717857c382d2a4", rewrite: hud_off_window },
        ClassTarget { class: "eb", class_md5: "517894f884b84da221685a5a118ef05c", rewrite: hud_off_scenario },
    ]),
}, Patch {
    id: "HZ-014",
    title: "타깃 몬스터 창이 미니맵을 비켜 왼쪽으로 (미니맵과 겹칠 때만, 겹치는 만큼) · 좁은 맵에서는 맵 그림 안으로",
    evidence: "The field HUD ce.f()V draws the target monster panel first (@0..104, kept by HZ-013): \
               while ce.l:I > 0 and the target ce.a:Lcq is not dead (ar.b != 7), x = rpg/a.a - 80 \
               (@23 getstatic rpg/a.a:I; @26 bipush 80; @28 isub; @29 istore_1), the 80x34 picture \
               ce.a:Lcw at (x, 0) and the HP bar from x + 44. The host's minimap (Phase 6-5, \
               hero2/hero3's: the top-right corner) would sit on it, and hero2's player asked for \
               the monster's box to its left (hero2 overlay::target_box_shift: an overlap query, \
               the box moves only when the panel covers it, by as much as it takes). @26..28 \
               `bipush 80; isub` (10 50 64) becomes invokestatic Hooks.targetX(I)I: the width in, \
               width - 80 - shift out, the shift the host sets (0 while its minimap is hidden or \
               clear of the box = the original). Same length, same stack effect, no StackMap frame \
               between 23 and 30 (classbytes.py 2026-10-23: frames at 101, 104, 107, ...). Chains \
               after HZ-010 (@375) and HZ-013 (@107, <init>) on the same class. \
               2026-10-24 (S25, map 135): on a canvas wider than 240 the game clips its field HUD to \
               the map picture (ce.e(dd): setClip(g.d, g.e, min(g.a, ce.g - g.d), ...); g.c() centres \
               a map narrower than the canvas), so at width - 80 the panel fell in the side margin - \
               cut on a 30-tile map in 520 (480 px), mostly gone on a 20-tile room. The hook also keeps \
               it inside that span (no change on a 240 canvas or a map as wide as the canvas). It does \
               not lift the clip: g repaints the margin only on a full redraw.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "ce", class_md5: "0feea535bec17058182df9c45fc1cf3c", rewrite: target_dodge },
}, Patch {
    id: "HZ-015",
    title: "메뉴 「3. 상점및창고」 를 처음부터 연다 (원래는 치트상점의 현금 상품 「휴대 상점」 을 사야 열렸다)",
    evidence: "The menu's 상점및창고 row (de: 상점 = the shop co anywhere, 창고 = the game's own 「저장 후 창고로 \
               접속하시겠습니까?」 and the 계정창고 chooser bg) is locked by bh.h:Z alone. Its only writers are \
               the cheat shop's 「휴대 상점」 purchase (ci.a(BB)V @228, a cash item: rpg/d.g[3]) and the \
               HOL.gamestore loader (rpg/b @50); with the cheat shop gone (HZ-010) a new install could \
               never open it. User (2026-10-24): open it for free. The three readers become a constant \
               true (`getstatic bh.h` b2 cp cp -> `iconst_1; nop; nop`, the branch after each kept): \
               bh.a(II)Z @75 (digit '3' -> the 'locked' notice gm[69]; HZ-012 moved it from '4'), @256 \
               (the cursor skips the row) and m.a(dd,IIZ)I @335 (the row drawn grey). Nothing is written: \
               HOL.gamestore keeps whatever bh.h it had, so with the patch off the row is as before. Same \
               length; bytes verified with classbytes.py (bh #40, m #27), no StackMap frame moves; \
               chains after HZ-012 on both classes (no site shared).",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::Classes(&[
        ClassTarget { class: "bh", class_md5: "ec7ee5341021765156cd5aee38cc1833", rewrite: shop_unlock_bh },
        ClassTarget { class: "m", class_md5: "859120493326a182ea87e4631d486a74", rewrite: shop_unlock_m },
    ]),
}, Patch {
    id: "HZ-016",
    title: "「부활의 구」 — 상점에서 사고, 쓰러지면 「부활하시겠습니까?」 를 묻고 예면 그 자리에서 일어난다",
    evidence: "User (2026-10-24): like hero2's 부활의 구 — revive only when the bag holds one, after a \
               「부활하시겠습니까?」 window. The death: ek.C()V state 6 (falling) at fall frame 8 sets state 7 \
               and, if eb.e(), @512 read ey.f — true (the network build, HZ-010) = cc.f(); ed.a(9), the \
               game over; false = the cheat shop's 「즉시 부활」 purchase (@527..562: ed.d = 5, the menu, \
               bh.c(3), (ci) child .a()). The game's own revive is that purchase's grant, ci.a(BB)V case 6: \
               ed.d = 3; bh.a().g(); player.d() — ek.d()V = 5 s untouchable, ek.A = 2, HP/SP/MP = max; the \
               next tick of state 7 stands the player up (A == 2 -> ar.a(1)). Its refusal is ek.b()V (A = \
               1 -> the game over). No item does this, so the orb is added without touching the jar: \
               (1) ba.a(BB)Lba; @107 `invokestatic rpg/e.a(String,I)[B` (the item factory reading record \
               id of /itm/CC) -> Hooks.itemRecord, which hands back a record for /itm/20 number 9 (the \
               file has 0..8; same layout: name, price int, text, checksum) and the file's own otherwise. \
               (2) ba.a(dd,IIZZ)V @76 `getstatic rpg/e.k; iload 4; aaload` (picture cat + id; 20 + 9 = \
               29 is the pack's selection frame) -> `iload 4; Hooks.itemIcon; nop`. (3) ba.a()Z (cat 20 \
               and id >= 2: the bag's 0/# registration popup, else 버리기/상세정보) -> `aload_0; \
               Hooks.quickItem; ireturn` + nops — not the orb (its 0/# use would be ek.a(Z)Z's default, \
               nothing). (4) co.h()V @315 `bipush 20; bipush 8` -> 9: the shop's special tab lists 0..9. \
               (5) ek.C()V @512 (`getstatic ey.f`, or HZ-010's iconst_1) -> Hooks.death: no orb = true \
               (the game over, as HZ-010); orb = the menu (ed.d = 5, bh.a().a()) with the game's 예/아니오 \
               window cf as its child, false; @527..562 (the old purchase) -> `return` + nops. (6) \
               cf.a(II)Z @59 / @89 `invokevirtual o.a(BB)V` (FIRE: parent.a(f, cursor); RSK when g) -> \
               Hooks.answer: parent bh (no other cf has it) = 예 -> bag.b(20,9,1), then the grant above; \
               아니오 -> ek.b() and the menu closed (ci's RSK path); any other parent -> parent.a(f, row) \
               as before. The question's f is 5, so RSK is no answer (hero2: CLR is not one). Nothing is \
               written to RMS: the orb leaves the bag in memory, as a potion does. A save holding orbs \
               loads without them when HZ-016 is off (ba.a([B) returns null, bt.a([B) skips it). Same \
               length everywhere; bytes verified with classbytes.py (2026-10-24): ba #68 #33, co, ek #159 \
               (and HZ-010's form), cf #48 x2; frames at ba.a()Z 19 and ek.C 527/563 stay (dead code).",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::Classes(&[
        ClassTarget { class: "ba", class_md5: "e1c4edbb48a1823b4486098c78780440", rewrite: revive_orb_item },
        ClassTarget { class: "co", class_md5: "c1a068f6d08e67c7ef490de32f4fa6c8", rewrite: revive_orb_shop },
        ClassTarget { class: "ek", class_md5: "a33b0a941d884487e83fe61b75e7d40e", rewrite: revive_orb_death },
        ClassTarget { class: "cf", class_md5: "91ca231a8e7efbdb2eee266400b835e8", rewrite: revive_orb_answer },
    ]),
}, Patch {
    id: "HZ-017",
    title: "난이도 (추가 설정: 쉬움·보통·어려움 = 몬스터 HP·공격력 1·1.5·2배, 기본 쉬움 = 원본)",
    evidence: "hero2's 환경설정 「난이도」 (2026-09-29: EASY / NORMAL / HARD = monster HP and attack x1 / x1.5 / \
               x2, EASY the shipped game). Every monster (cq and its subclasses: the boss base ak, x, at, be, ck, di ...) gets its \
               stats in cq.i()V: p()V copies the kind's record eq.a[kind] (n:B level <- eq.d:B, h:I max HP \
               and i:I HP <- eq.d:S, d:S <- eq.b, e:S defence <- eq.c, f:S attack <- eq.e, g:S <- eq.f); \
               then the game's own replay rounds ed.h:B (bumped after the ending, at most 2) raise level \
               and stats; then q()V adds 0..2 levels at random (ak, the boss base, overrides it empty). \
               i() runs from the constructor (placed at once) and from the respawn queue cq.C()V @63, so \
               a spawn and a respawn both pass it. Roles from the damage code: the player's hit on a \
               monster is max(5, atk - e) (cq.a(IB)Z @84 via cq.a(II)I); the monster's own hits pass \
               cq.a()S = f (x4/5 while i:Z) to cq.a([BSZ)Z / cq.b(Lar;IB)Z (at, da, dh, di, dk, be, bm, \
               ck); cq.c()I / d()I return i / h, the target panel's bar. cq.i()V @208 `invokevirtual \
               #223 q()V` (b6 00 df) becomes invokestatic Hooks.monsterInit(Object)V: the hook calls \
               q() on the monster as before (virtual, so ak's empty one stays), then at 150 / 200 % sets \
               h and i to h x pct (int, saturating) and f to f x pct (at most 32767). Read and written \
               through cq's own fields (Jvm::get_field_from: subclasses redeclare names). At 100 % it \
               writes nothing. A changed setting applies to monsters made after it (map entry, \
               respawn); RMS holds no monster. Same length; StackMap frame at 207, none at 208 \
               (classbytes.py 2026-10-25). Chains after HZ-005 (@o()V 87) on the same class.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "cq", class_md5: "acbac4519692a62c390389354fb998ea", rewrite: monster_difficulty },
}, Patch {
    id: "HZ-018",
    title: "몬스터 리젠 (추가 설정: 1~6배 빨리 다시 나온다, 기본 1배 = 원본)",
    evidence: "hero2's 「몬스터리젠」 row (기존 / x2..x6). A monster's death is cq.g()V: state 7, then b:S <- \
               eq.a:S (the kind's respawn delay), the drop o()V, the EXP n()V, m()V (off the map, g.d(er)); \
               if b > 0 the monster joins the static queue cq.c:Vector (a(Lcq;)V). The field tick ed.d()V \
               @59 runs cq.C()V once a tick: each queued monster counts b down; at 0 it is placed at its \
               home tile (a(BB)Z, retry in 10 ticks when blocked), re-made by i()V and leaves the queue. \
               A kind whose eq.a is 0 never comes back. g()V @11 `getfield #105 eq.a:S` (b4 00 69) becomes \
               invokestatic Hooks.respawnDelay(Object)I with the same eq on the stack: the delay divided \
               by the rate, at least 1 tick when it was positive, 0 and below as they were — rate 1 = \
               the field as read. Same length, same stack effect (an int either way); g()V's one StackMap \
               frame is at 44 (classbytes.py 2026-10-25). Chains after HZ-005 and HZ-017 on cq.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "cq", class_md5: "acbac4519692a62c390389354fb998ea", rewrite: monster_respawn },
}, Patch {
    id: "HZ-019",
    title: "줍기 알림이 EXP 바 위로 (게임 하단 바가 없을 때만)",
    evidence: "The game lists what the player picks up itself: g.a(BBBZ)V letters the item (or 「N금」) into a \
               120x16 picture and puts it in the first free of three slots g.b:[Lcw; with 15 ticks in g.a:[I \
               (all full: slot 0); the map painter g.c()V @1398..1484 draws slot i at (rpg/a.c, ce.h - i * (h + \
               1)), anchor 33 (bottom centre), counting each down - stacked up from the viewport's bottom. \
               With HZ-013 the viewport is the whole canvas (ce.h = rpg/a.b), so the lowest notice sat on \
               the host's EXP bar, which takes the bar's place at the bottom centre (probe map 46 + HUD \
               preview 2026-10-25: 「귀환서」 under the bar). @1439 `getstatic #88 ce.h:I` (b2 00 58) \
               becomes invokestatic Hooks.noticeBottom()I: ce.h, less NOTICE_LIFT (9 guest px) when ce.h is \
               the canvas height - the game's bar there (HZ-013 off) leaves it as it was, and the host draws \
               no EXP bar then. Same length and stack effect; no StackMap frame at 1439 (classbytes.py: \
               frames ... 1398, 1465). Chains after HZ-013's g.a(dd)V edit on the same class.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "g", class_md5: "4068de457fd524cea9717857c382d2a4", rewrite: notices_over_exp_bar },
}, Patch {
    id: "HZ-020",
    title: "세이브 슬롯 10개 (세 개씩 쪽으로 — 슬롯 창에서 ◀ ▶)",
    evidence: "User (2026-10-25): save slots 3 -> 10. The game keeps three, by index 0..2: rpg/b's static \
               String[3] a/b/c (HOL.gamedata1..3 / HOL.item1..3 / HOL.slot1..3, <clinit>) read by its six \
               readers / writers a/b/c(B)[B and ([BB)Z (getstatic; iload; aaload — each with the slot's \
               own record key rpg/b.a/b:[[B, so a record only reads back in its own index), the current \
               slot ed.j:B (set by the slot window bb, used by every save ed.k()/l()), the summaries \
               ed.m()V (three loops into ed.a:[[B, ed.c:[I, ed.a:[J, ed.af:[B; ed.i:B = how many exist) \
               and the window bb (count 3 from its ctor, rows drawn by a fixed loop of 3 in bb.a(dd)V \
               @29..181). Widening those is not same-length, so the extra slots are PAGES of the game's \
               three: a host page number (hz_java GuestState::slot_page) picks which names the six \
               getstatics see (Hooks.gamedataStores / itemStores / slotStores: page 0 = the game's own \
               arrays, page p = HOL.x(3p+1..3p+3)), keys included - slot n (1-based) is the game's slot \
               (n-1)%3 on page (n-1)/3, so slots 4, 7, 10 share slot 1's key. bb.a(II)Z @3 o.b(II)Z -> \
               Hooks.slotKey: the child first as before; then, while the title has the keys (ed.d = 2), \
               LEFT / RIGHT turn the page, re-run the game's own ed.m() and give bb the page's count \
               (3, the last page 1) - so the page can change only at the title, never while a slot is \
               loaded, and every write of a session goes to the page it was loaded from. bb.a(dd)V @24 \
               (title, ah.a) -> Hooks.slotTitle: the title, then the game's arrows o.a(Ldd;IIIIZ)V \
               round it; @93 (a row's box, o.a(Ldd;IIIIII)V; a StackMap frame target, same stack) -> \
               Hooks.slotBox and @173 (its 「저장 N」, ah.a) -> Hooks.slotLabel: the row's number for \
               the page, nothing for a row past the last page's slots. The title i: <init> @8 and \
               a(II)Z @28 `getstatic ed.i:B` -> Hooks.saveCount()B, the count over every page (the \
               game's own reader rpg/b.c(B)[B on each), so 이어하기 is offered when any slot holds a \
               save. Page 0 everywhere is the original game; slots 1..3 are the same stores as before. \
               Same length at all twelve sites (classbytes.py 2026-10-25); chains after HZ-010 (i \
               @42) and HZ-011 (bb @149).",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::Classes(&[
        ClassTarget { class: "rpg/b", class_md5: "ed0a7548ed50e09f1bb31bd2cde16dfa", rewrite: slot_pages_stores },
        ClassTarget { class: "bb", class_md5: "698c1dc125db3660962f5fec7a5807a3", rewrite: slot_pages_window },
        ClassTarget { class: "i", class_md5: "286944bcd3bc0a761142082bf8d1a1bf", rewrite: slot_pages_title },
    ]),
}, Patch {
    id: "HZ-021",
    title: "상점 판매 창의 「일괄판매」 — 고른 등급의 장비를 한 번에 판다 (에픽 이상은 한 번 더 묻는다)",
    evidence: "User (2026-10-25): bulk sell exactly as hero2 (HERO2_GUEST_RE.md §8.85: a button on the shop's \
               sell window, the game's list popup of the grades, epic and up asked again with the cursor on \
               아니오; equipment only, worn excluded, exactly the grade chosen; the game's own price; paid, then \
               removed). This game's grades are bx.e:B with the names t/it.tdf [28..31] 일반 레어 에픽 세트 \
               (rpg/e.g, the item window's own words). The shop co: selling while co.h >= 3; its own single \
               sale co.a(BB)V @329..386 = bt.b(cat, id, 1) (the first of that kind - not this instance), \
               ed.e(price / 10) (the purse, clamped 0..999,999,999, ce.c redraw) and the message ni[22] \
               판매하였습니다 through bz.a:[B + bz.f:B 15. The bulk sale pays the sum of price / 10 x stack \
               for the equipment bag ed.a(0) (categories 0..15: only bx) items of that grade with bx.i < 0 \
               (not worn - the player holds worn gear by object, ek.a:[Lbx;), through the same ed.e, then \
               removes exactly those objects (Vector.removeElement, identity) and refreshes the window \
               (co.d()), the same message. co.a(dd)V @323 `invokevirtual #97 o.b(Ldd;III)V` (the gold, the \
               painter's last draw before the message) -> Hooks.shopGold: the gold, then the button in the \
               title band's right (the game's box o.a(Ldd;IIIIII)V + the game's font). The popup ee answers \
               its parent with ey.f (6, set in its ctor) at ee.a(II)Z @36 `invokevirtual #28 o.a(BB)V` -> \
               Hooks.answer (HZ-016's, same descriptor): tags BULK_GRADE / BULK_ASK, set on the windows the \
               host made, go to the sale; any other answer is parent.a(f, row) as before. The question is \
               the game's cf(o,[B,B,Z) made with f = 4 (its ctor puts the cursor on 아니오 then) and retagged. \
               Touch only, as hero2: the button is a tap target of the game-window touch (menu_touch). \
               Same length at both sites (classbytes.py 2026-10-25); co chains after HZ-010 (@138) and \
               HZ-016 (h()V @315).",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::Classes(&[
        ClassTarget { class: "co", class_md5: "c1a068f6d08e67c7ef490de32f4fa6c8", rewrite: bulk_sell_button },
        ClassTarget { class: "ee", class_md5: "ba1a3de3de7052cef087cdd2b8740019", rewrite: bulk_sell_popup },
    ]),
}, Patch {
    id: "HZ-022",
    title: "가디언 레벨 — LV 99 뒤 1~250, 오를 때마다 스탯 포인트 4 (레벨·장비 조건은 99 그대로)",
    evidence: "User (2026-10-25): the level cap like hero2 - hero2 kept LV 99 and added 가디언 레벨 GL 1..250 \
               on its own bar (HERO2_GUARDIAN_LEVEL_PLAN.md: the level byte stays, so level formulas, unlocks \
               and requirements are unaffected; each GL gives a level-up's stat points; the next GL needs \
               +2 % of LV 99's EXP). This game's EXP loop ek.j(I)V @44..197: while a[2] >= b[2]: a[2] -= b[2]; \
               if n < 99 {n++, i(), a:S += 4, even level b:S += 1, the cg(8) effect} then the full refill and \
               again - at 99 the EXP is thrown away. b[2] = n^3 - n^2 + 80n from ek.i()V @970..1014 \
               (968,418 at 99 - hero2's own rule). ek.a()[B (the save) writes a[0..3] as ints. HZ-022 keeps \
               the EXP at 99 instead: @75 `getfield #141 n:B` -> Hooks.levelUp(Object)I returns n; at 99 it \
               adds back what the loop just took, works out the GL from the EXP since 99 (hz_java::guardian), \
               gives 4 stat points a GL gained, sets b[2] to the next GL's start (never past GL 250) and shows \
               「가디언 레벨 N 달성」 in the game's own pickup notice list (g.b:[Lcw, g.a:[I) with the level-up \
               effect cg(8); the game's refill and loop follow unchanged. ek.i()V @1009 `getfield #110 b:[I` \
               (the store of b[2]) -> Hooks.expTable(Object)[I: below 99 the array; at 99 b[2] set from the \
               EXP and a scratch array returned for the game's value. ek.e(B)I / f(B)I (aload_0; getfield; \
               iload_1; iaload; ireturn) -> aload_0; iload_1; invokestatic Hooks.statNow / statMax; ireturn; \
               nop: as read, except index 2 at 99 = the EXP into the GL and what it needs - the character \
               window and the HUD's bar show the GL's progress (index 2 is read only by displays: javap of \
               every caller, the others ask 0 1 3). So the GL is stored in the game's own saved EXP: nothing \
               new is written and it can never disagree with the save. An LV 99 save from before starts at \
               GL 1. With the patch off the game's loop takes the EXP back down below 968,418 (the points \
               already given stay). Same length at all four sites (classbytes.py 2026-10-25; e/f max_stack 2 \
               is enough); chains after HZ-004 (j @32), HZ-010 and HZ-016 (C @512) on ek.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "ek", class_md5: "a33b0a941d884487e83fe61b75e7d40e", rewrite: guardian_levels },
}, Patch {
    id: "HZ-023",
    title: "가디언 장비 — 장비 종류마다 LV 80 하나, 게임의 2회차 LV 75 이상 몬스터가 떨군다",
    evidence: "User (2026-10-25): new items like hero2 - hero2's first was a 「가디언」 tier past the old best \
               (HERO2_GUARDIAN_PLAN.md 2.6: the end-game best x1.25, dropped after the level cap). This game's \
               gear stops at LV 65 (/itm/00..15, 29..32 records each: the LV 65 pieces are recipe-only), its \
               monsters at LV 45, and its own replay rounds ed.h (cq.i()V @36..85: round 1 = 2n/5 + 40, round 2 \
               = 3n/5 + 60) put round 2 at LV 60..87. A drop (cq.o()V) rolls R once: slots 0/1 give a common \
               piece min(lvl/4, 14), slot 2 the rare band min(lvl/5 + 13, 28) (@207..215, monsters past LV 13), \
               slot 3 a fixed one (javap 2026-10-25). The tier is 16 records the jar lacks, one per equipment \
               category at number GEAR_ID (40) - an item stores category, number, the three stat shorts, h and \
               the six option bytes (bx.a(DataOutputStream)), and its name, price, level, grade, sprite and \
               palette come from the record each load, so only the number is for ever. (1) ba.a(BB)Lba; @107: \
               the game's `invokestatic #68 rpg/e.a(String,I)[B` -> Hooks.gearRecord; or, after HZ-016, its \
               Hooks.itemRecord -> Hooks.orbGearRecord (both: the orb and the tier). Same record layout as the \
               files (bx.a([BI)I: level, grade, set, sprite, palette, b c d, h, options; option 100 rolls 0..15 \
               on load). (2) cq.o()V @215 `invokestatic #259 Math.min(II)I` -> Hooks.rareDrop(II)I: min as \
               before, except at the band's top (lvl/5 + 13 >= 28, LV 75 and up) one in four (the game's own \
               Random ar.a) is GEAR_ID - in the slot's category range, so any of the 16. (3) g.a(BBBB)V @17 \
               `bipush 28` -> 40: the ground list refuses a number past 28 (and a category past 20); its only \
               callers are cq.o()V's six drops (javap of every class, rpg/ too), none past 28, so 29..40 are \
               only ever the tier (pickup g.a(BB)Z has no limit, the ground picture is k[cat]). Nothing else \
               reads an equipment number as an index (the picture is k[cat], the paper doll \
               /hero/w/<cat><sprite>). A \
               save with the tier loads without it when HZ-023 is off (ba.a([B) returns null, bt.a([B) skips \
               it - HZ-016's path). Same length; no StackMap frame at either site (classbytes.py 2026-10-25); \
               chains after HZ-016 on ba, after HZ-005 (@87) on cq and after HZ-013 / HZ-019 on g.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::Classes(&[
        ClassTarget { class: "ba", class_md5: "e1c4edbb48a1823b4486098c78780440", rewrite: gear_records },
        ClassTarget { class: "cq", class_md5: "acbac4519692a62c390389354fb998ea", rewrite: gear_drop },
        ClassTarget { class: "g", class_md5: "4068de457fd524cea9717857c382d2a4", rewrite: gear_ground },
    ]),
}, Patch {
    id: "HZ-024",
    title: "창 아래 모서리의 소프트키 그림(왼쪽 □·오른쪽 ↩, 옛 하단 바 조각)을 지운다 — OK·CLR 이 대신한다",
    evidence: "User (2026-10-26, S25): the menu showed 'strange buttons' bottom-left and bottom-right - the game's \
               own soft-key art, which hero2 and hero3 have no counterpart of (their games have no soft keys). \
               The windows draw it through one static painter o.a(Ldd;I)V (o.<clinit>: b = /sks/ok.png, a = \
               /sks/back.png): bit 0 -> b at (0, H) anchor 36 BOTTOM|LEFT, bit 1 -> a at (W, H) anchor 40 \
               BOTTOM|RIGHT; called by bb bg bh bi bn bo ci co cu dv et j s with 3 or 2. None of the thirteen \
               reads LSK -6 itself: confirm is o.f(II)Z (FIRE 8, '5' or -6), back o.g(II)Z (-7 or no game \
               action), so the host's OK and CLR do what the two pictures said (javap 2026-10-26). The game's \
               minimap window draws the back picture itself: g.a(dd)V @423..455 (getstatic o.a:Lcw; ifnull 458; \
               ... dd.a(Lcw;III)V anchor 20). Same-length edits: o.a(Ldd;I)V @34 and @53 iload_1 -> iconst_0 \
               (the bit tests read 0, both draws skipped; StackMap frame at 53 is an int either way); g.a(dd)V \
               @423 getstatic #158 -> aconst_null; nop; nop (ifnull taken; frame at 423 unchanged). The third \
               draw of the painter goes too: while o.c:Z (set by the system window ec for the help s and the \
               options bn it opens in-game) it puts rpg/e.k - /img/sui picture 4, the frame of the old bottom \
               bar's corner box (ce.f draws it there) - at the bottom-left (probe 2026-10-26: menu > 0 > 도움말, \
               an empty ornate box beside the help window); @74 getstatic #45 o.c:Z -> iconst_0; nop; nop \
               (frame at 74, an int either way). Windows that answer only -6/-7: cd with e:Z set - \
               made only by de (the 창고 question, @174) - and aj (the cheat shop's and the scenario's cash \
               questions, unreachable since HZ-010); for cd the host's OK sends LSK (hz_java::state \
               GuestView::lsk_ok). Classes verified with classbytes.py (2026-10-26); g chains after HZ-013 \
               (@157), HZ-019 (c()V) and HZ-023 (a(BBBB)V).",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::Classes(&[
        ClassTarget { class: "o", class_md5: "156653192e287c925a86054d8e15e1d2", rewrite: soft_art_off },
        ClassTarget { class: "g", class_md5: "4068de457fd524cea9717857c382d2a4", rewrite: soft_art_off_map },
    ]),
}, Patch {
    id: "HZ-025",
    title: "줍기 알림을 호스트의 왼쪽 알림 쌓기로 (hero2 식 — 게임이 만든 그림 그대로)",
    evidence: "User (2026-10-27): the game's own pickup notices join the host's left-hand notice stack, as hero2's \
               do (hero2 switched its game's notice painter off, n.d()V, and stacks copies of the game's own \
               pictures at the left edge). g.a(BBBZ)V makes a NEW 120x16 picture for every notice (cw.a(II) @4), \
               puts it in the first free of three slots g.b:[Lcw; (all full: slot 0) and sets the slot's count in \
               the static g.a:[I to 15 (@267, @297). The map painter g.c()V @1398..1484 counts each live slot down \
               and, while it is still positive, draws it: @1428 aload_1 (dd); aload 8; getfield b; iload_2; aaload; \
               getstatic rpg/a.c; <the y, HZ-019 @1439>; @1457 bipush 33; @1459 invokevirtual #209 \
               dd.a(Lcw;III)V. Two same-length edits: @1457 bipush 33 (10 21) -> iload_2; nop (1c 00) - the slot \
               rides in the anchor's place (local 2 is the loop's int, istore_2 @1397) - and @1459 invokevirtual \
               -> invokestatic Hooks.pickupNotice(Object,Object,III)V, the same stack effect. The hook draws \
               nothing; on a slot's first drawn frame (count 14) it reads the picture through Image.getRGB for the \
               host, which shows it in its notice stack for as long as its own notices (hz_android::touch \
               Notice::Picture). The game's counts and slots run as before. No StackMap frame at 1457 or 1459 \
               (classbytes.py 2026-10-27: frames ... 1396, 1398, 1465). Chains after HZ-019 (@1439) on the same \
               method and after HZ-013 / HZ-023 / HZ-024 on g.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "g", class_md5: "4068de457fd524cea9717857c382d2a4", rewrite: pickup_notices_to_host },
}, Patch {
    id: "HZ-026",
    title: "메뉴에서 「미니맵」 행을 뺀다 — 1~7 그대로, 8 계정창고, 0 시스템 (지도는 상단 MAP 탭)",
    evidence: "User (2026-10-27): take the minimap out of the menu - the host's MAP tab (Phase 7: world map + \
               the game's minimap window as it draws it) and the field's own minimap panel cover it, and the \
               game's window stays on the field's '*' (ce.a(II)Z key 42 -> @614: 환경설정's rpg/c.d 1 = the \
               quick save, otherwise ed.a(3,8,3) @646). On HZ-012's menu (rows 0..9 = 캐릭터 \
               인벤토리 상점및창고 장비 스킬 정령 퀘스트 미니맵 계정창고 시스템), same-length edits: (1) bh.<init> @2 \
               bipush 10 -> 9, the row count o.b:B (m(o, byte) -> o(o,B,1,1)): o.c(II) moves the cursor and the \
               list m.a(dd,IIZ)I draws within it; its second loop (@140..281, every m list) outlines the slots \
               left of ten, so one empty slot stays under 시스템 - the options window bn shows three the same way. \
               (2) bh.d(B)V's switch, already HZ-012's rotation (checked whole - without HZ-012 the class is \
               refused): 7 = the storage hook (old 0's block), 8 = 시스템 ec, 9 = 미니맵's block (unreachable past \
               the nine rows). bh.f:B stays the shown row, so the children's titles follow through \
               Hooks.menuLabel, which reads the menu's row count (9: 계정창고 at 7, gm[9] at 8). (3) bh.a(II)Z: \
               the digit range @17 bipush 57 -> 56 ('9' is no shortcut: it falls to o.g/o.f/o.c, which want \
               -7 / FIRE 8 / -6 / UP 1 / DOWN 6 - nothing), '0' @107 bipush 9 -> 8. (4) bh.c(B)V mode 2 (the \
               quick save: the 시스템 row and ec.a()) @48, @55 bipush 9 -> 8. (5) m.a(dd,IIZ)I numbers a row \
               「0.」 when it is 9 (@415, and @657 for the selected row) -> 8; this class only after HZ-012's \
               label site (@374 aload_0), so the game's own ten rows never get two 「0.」. (6) ec.a(II)Z @57..65 \
               `aload_0; getfield o.a:Lo; bipush 9; putfield o.a:B` (RSK closing 시스템 puts the menu's cursor \
               on 9) -> nop x9: the cursor is already on 시스템's row - bh.d(B)V is only reached with it there \
               ('0' / OK / bh.c(2)) and nothing moves it while a child is open - so this is right on any menu. \
               The host's tap on a menu row sends the row's digit, 「0」 for the last row \
               (hz_java::menu_touch::menu_digit). Bytes verified with classbytes.py (2026-10-27): no StackMap \
               frame moves (bh.a frames 11, 101, 136; c 47; m 413/653 inside; ec frame 57 = nops, stack \
               empty either way). Chains after HZ-010/012/015 on bh and m (no shared site).",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::Classes(&[
        ClassTarget { class: "bh", class_md5: "ec7ee5341021765156cd5aee38cc1833", rewrite: menu_no_minimap_bh },
        ClassTarget { class: "m", class_md5: "859120493326a182ea87e4631d486a74", rewrite: menu_no_minimap_m },
        ClassTarget { class: "ec", class_md5: "27aa2c28ca14f8d737a6c43be6f376b1", rewrite: menu_no_minimap_ec },
    ]),
}, Patch {
    id: "HZ-027",
    title: "계정창고 고르기 창의 제목 「계정창고」 가 「돈」 판에 가려지지 않게 위로",
    evidence: "User (2026-10-27): something hides behind the 「돈」 button in 계정창고. The chooser bg (title flow \
               after a slot, and the menu's 계정창고 since HZ-012) draws no frame: bg.a(dd)V puts its title com[9] \
               (네트워크 -> 계정창고, HZ-011) at y = rpg/a.d - 64 (the canvas centre - 64, @12..20) and THEN the two \
               plates at o.b:I + 25 + 17i, o.b:I = (H - 174) >> 1 = centre - 87 -> the 돈 plate (15 high) starts at \
               centre - 62 and covers the title's 12-px glyphs but their top 2 rows, on every canvas size \
               (probe 520x240 2026-10-27: title 56, plate 58). @18 bipush 64 -> 82: the title at centre - 82, \
               8 px clear above the plate (240 high: 38..50 over 58; 320 high: 78..90 over 98), on the title \
               flow's background where the picture is black. Same length, no StackMap frame (frames 36..145); \
               chains after HZ-012 (a(II)Z) on bg.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "bg", class_md5: "2b56d31d25837e85783ba1892c01b3ff", rewrite: storage_title_clear },
}, Patch {
    id: "HZ-028",
    title: "반투명 그리기를 호스트가 대신 — 컷신 페이드·어둡게 깔기가 끊기지 않게 (그림은 그대로)",
    evidence: "Measured (probe watchline tools/hz_watchline.py + a per-method bytecode count, 2026-10-27): the \
               opening's black narration screens and fades cost 40-45 ms a paint on the x86 host against ~5 ms \
               for the map 44 field, and 77-91 % of the bytecodes run there (up to 1.5 M a paint) were one \
               method, the Graphics wrapper's fillRect dd.a(IIII)V; with it gone, 89-99 % of the rest was its \
               twin, the picture draw dd.a(Lcw;III)V. Both look at the alpha mask dd.a:I (dd.b(I)V: (alpha << \
               24) | 0xFFFFFF - set by the scripts eb, the field ce.d, the map painter g.c, ch, bz, ci, \
               rpg/a.b; -1 = opaque, the plain fillRect / drawImage). (1) dd.a(IIII)V with a mask: @21 newarray \
               int[w*h], @29..41 colour = (b | 0xFF000000) & a into local 6, @43..67 a loop that stores the \
               colour into every element from the top index down (@55 aload 5; iload 7; iload 6; iastore; iinc \
               7,-1 - eight bytecodes a pixel, ~1 M for a 520x240 screen), @68 Graphics.drawRGB(arr, 0, w, x, y, \
               w, h, true). @55 (10 bytes) -> aload 5; iload 6; invokestatic Hooks.fillInts([II)I; istore 7; nop: \
               the hook fills the array and returns -1, so @65 goto 50 -> iload 7; iflt 68 leaves the loop. (2) \
               dd.a(Lcw;III)V with a mask: @44..68 newarray int[w*h] + Image.getRGB into local 7, @71..107 a loop \
               from the top index down (local 1, the cw, reused) - @81 if (p >>> 24 != 0) p &= a (twenty \
               bytecodes a pixel) - @108 Image.createRGBImage(arr, w, h, true) + drawImage. @81 (21 bytes) -> \
               aload 7; aload_0; getfield #12 a:I; invokestatic Hooks.maskAlpha([II)I; istore_1; nop x11: the \
               hook masks every pixel the loop would, returns 0, and @102 iinc 1,-1 (a branch target, kept) \
               makes it -1, so @105 goto 77 -> iflt 108 leaves the loop. Either way the array is what the loop \
               leaves, the index ends at -1 as the loop's does, and an empty array never reaches the hook - the \
               same drawRGB / createRGBImage, the same pixels (watchline bytes equal with and without). Same \
               length; StackMap frames 21, 50, 68 and 26, 77, 102, 108, 132, 138 (classbytes.py 2026-10-27), \
               none inside an edit, the loops' heads and locals untouched. A platform patch could not do this - \
               they are one game class's loops.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "dd", class_md5: "d29479a03496b1184d2e182152dcea4d", rewrite: translucent_fill },
}, Patch {
    id: "HZ-030",
    title: "가디언 모드 — 헬 다음 회차 (몬스터 LV 85~110, 퀘스트 돈·경험치 헬의 2배)",
    evidence: "User (2026-10-28): a 가디언 모드 after 헬, monsters LV 85..110 growing in proportion to what is there, \
               more quest money and EXP. The game's replay round is ed.h:B (0 노말, 1 하드코어, 2 헬 - t/com.tdf [33] \
               「하드코어 모드로」 / [34] 「헬 모드로」 + [32] 「재시작합니다.」 at the ending), saved in the slot summary \
               (ed.a(B)[B byte 16, read back by bb @115 / @393 with no check; the slot list draws round + 1 \
               marks, ed.a(B)[B's percent adds it, capped at 100). Every reader (javap of all classes, \
               HEROZERO_GAME_OPTIONS_RE.md §18): (1) the ending's command 8 ed.a()V @430 `getstatic h; iconst_2; \
               if_icmpge` -> iconst_3: 헬 goes on to 3 and 3 stays. (2) cq.i()V: p()V copies the kind's record, \
               then @36..85 r = min(h, 2) raises level and stats (r > 0: n = (r+1)n0/5 + 40|60, d and e (the \
               defences against skills / spirit) x n/n0, HP (HP-70)(n + 20|40)/n0, attack (atk-13)n/n0 + 20|40, \
               g (evasion) (g-10)n/n0), then q() adds \
               0..2 levels (not bosses). @31 invokevirtual #222 p()V -> Hooks.guardianMonster(Object)V: p() as \
               before (virtual), then at round 3 the same rule from the copied record with n = 85 + 25(n0-1)/44 \
               (base 1..45 -> 85..110) and offset 60; @36 and @47 getstatic #100 ed.h:B -> Hooks.monsterRound()I: \
               the round up to 2, 0 at 3, so @52 ifeq 207 skips the game's block. q() and HZ-017 (@208) still \
               follow. (3) eb.b(BLn;)Z script actions 17 (money, three shorts) and 20 (EXP, three u16) read \
               the value of min(h, 2): @1208 invokestatic #270 ed.e(I)V -> Hooks.questMoney(I)V and @1458 \
               invokevirtual #279 ek.j(I)V -> Hooks.questExp(Object,I)V pay 헬's value x2 at round 3 (actions \
               12 / 16, items, stay 헬's). (4) ce.d(Ldd;)V @897 invokevirtual #181 cx.a(I)[B -> \
               Hooks.roundText(Object,I)[B: entry 34 reads 「가디언 모드로」 when the round is 2 or 3 (the line is \
               shown before the bump). The script's condition 8 (round 0 / above) and the menu's bh.e flag \
               (round > 0) need nothing. Up to 헬 every hook returns what the replaced instruction did. Turned \
               off with a round-3 save the game reads it as 헬 (both min(h, 2)) and the ending leaves it. Same \
               length everywhere; StackMap frames cq.i 30 / 47 / 50, eb 1207 / 1454, ce 897 sit on unchanged \
               instructions or keep their stack types (classbytes.py 2026-10-28); ed.a has none in 421..446. \
               Chains after HZ-005 / 017 / 018 / 023 (cq), HZ-006 (ed), HZ-013 (eb), HZ-010 / 013 / 014 (ce) - \
               other methods.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::Classes(&[
        ClassTarget { class: "ed", class_md5: "9b90d32b52e8ddcc4a90fb8b7ac2e610", rewrite: guardian_round_cap },
        ClassTarget { class: "cq", class_md5: "acbac4519692a62c390389354fb998ea", rewrite: guardian_monsters },
        ClassTarget { class: "eb", class_md5: "517894f884b84da221685a5a118ef05c", rewrite: guardian_rewards },
        ClassTarget { class: "ce", class_md5: "0feea535bec17058182df9c45fc1cf3c", rewrite: guardian_ending_text },
    ]),
}, Patch {
    id: "HZ-055",
    title: "레벨당 SP 15 (게임 5 + 10) — hero2 식",
    evidence: "User (2026-10-29): 「레벨당 SP가 추가 +10씩」. ek.i()V @1015..1037 is max SP: `iconst_5; aload_0; getfield #141 n:B; \
               imul; bipush 10; aload_0; getfield c:[I; iconst_3; iaload; imul; iadd; aload_0; iconst_4; invokespecial g(B)I; \
               iadd; istore_1` (5 × level + 10 × 의지 + option 4 — classbytes.py 2026-10-29, frames 969 and 1105 outside). \
               @1017 `getfield n:B` (b4 00 8d, `this` on the stack) -> invokestatic Hooks.spLevel(Object)I = level × 3, so \
               the 5 × becomes 15 ×. Same length, same stack. Nothing saved — the game recomputes max SP in i().",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "ek", class_md5: "a33b0a941d884487e83fe61b75e7d40e", rewrite: sp_per_level },
}, Patch {
    id: "HZ-056",
    title: "게임 결함 — 동화 중 정령 스킬 시전 동안 OK 를 누르면 그 뒤로 공격이 안 나감",
    evidence: "HEROZERO_CLASS_BALANCE.md §8.8 (probe trace 2026-10-29, 정령사 T4 bench): the field OK `ed.b(I)V` queues a step with \
               ek.d(B)Z and starts it only from state 1 (idle) or 2 (moving); in any other state the step stays in the queue \
               ac:[B. Transformed (b:I > 0) the step is l + 1 + 6, so an OK during the spirit cast (state 10) leaves \
               ac[0] = 6; the cast's end C()V @698..720 (`aload_0; invokespecial #265 D()V; return`) puts the player idle \
               (l = -1) without B()V, and from then on d(B)Z refuses every OK and skill key (`ac[l + 1] >= 4`) — no attack \
               until the spirit skill can be cast again (its own B()) or a map change; the bench's 정령사 stood idle for 2,300 \
               frames. @717 `b7 01 09` -> invokestatic Hooks.spiritCastEnd(Object)V: D(), then B() only when D() left \
               state 1 with l < 0 and ac[0] set (the queue is empty there by every other path). Same length, same \
               stack; frames 698 and 721 outside (classbytes.py 2026-10-29). Chains after HZ-016 (@512) on C().",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "ek", class_md5: "a33b0a941d884487e83fe61b75e7d40e", rewrite: spirit_cast_end },
}, Patch {
    id: "HZ-057",
    title: "보스 반사 = 패턴 — 공격 패턴 중 무작위로 「반사 태세」(예고 1 초 + 4 초, 거울 코팅 표시), 그 밖엔 반사 없음",
    evidence: "User (2026-10-29): 「상시 반격모드 … 특정 패턴시에만 반격모드가 켜지도록 … 보스 주변에 표시」, look B 거울 코팅 \
               (mock-ups out/reflect_mock). Every boss is an ak (cq's factory 99..131: x, dk / at, di / ck, be, bs, dp's aw / bm \
               / dq / e). (1) The pick: a boss chooses its next attack in its k() — dk.k's z() takes the next of /t/bd [8] \
               and reloads the cooldown l.a:S from [9]; x, be and dp test the same l.a:S at @1 — called from cq.l()V @128 \
               `invokevirtual #217 k()V` (x.l calls cq.l; bs.l is empty) -> Hooks.monsterThink(Object)V: k(), then l.a:S \
               above what it was = a pick, and hz_java::boss may start the stance. Its clock is HZ-006's hook (the field \
               tick ed.d()V, once a frame, not under a window). (2) The reflection: ak.a(Lek;BIZBBZZZ)Z (final: the \
               player's blow on any boss) @23 and @53 `invokespecial #57 cq.a(Lek;BIZBBZZZ)Z` -> Hooks.bossHit: the blow \
               through invokespecial, then in the stance a fifth of the HP it took (cq.i:I before − after) less the \
               player's defence, ÷ 3, at most 20 % of max HP, through ar.a(Lar;IBB)V (…, 0, 2) as HZ-045's site; HZ-045's \
               always-on part reflection (키메라 103 / 126, 켈베로스 107 / 130) does nothing while this is chosen. (3) The look: \
               cq.b(II)V @207 `invokevirtual #196 a(II)V` (the monster's own picture, after cq.b's culling) -> \
               Hooks.monsterDraw: a boss in the stance has the back buffer dd.a:Image read around it before and after — \
               what changed is its silhouette — and the coat drawn over that with drawRGB on dd.a:Graphics (the clip \
               ce.e(dd) set). Same length everywhere, same stack; StackMap frames cq.b 188, cq.l 127, ak 27 / 40 / 41 are \
               other instructions (classbytes.py 2026-10-29). Chains after HZ-005 / 016 / 017 / 018 / 023 / 030 / 031 / 041 \
               / 046 (cq) and HZ-045 (ak) — other methods.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::Classes(&[
        ClassTarget { class: "cq", class_md5: "acbac4519692a62c390389354fb998ea", rewrite: stance_monster },
        ClassTarget { class: "ak", class_md5: "c32c38972e807c6e0f8a1e4c89c98d2a", rewrite: stance_boss_hit },
    ]),
}, Patch {
    id: "HZ-058",
    title: "퀵슬롯 7칸 × 2쪽 (hero2 식) — 스킬·정령·물약 팝업의 등록 줄이 「퀵슬롯등록 / 퀵슬롯해제」",
    evidence: "User (2026-10-30): 「퀵슬롯이 hero2처럼 7칸있어야해」, decisions in HEROZERO_QUICKSLOT_PLAN.md §6 (7 × 2 pages, potions \
               too, honest cooldowns, PC Q W E R A S D + Tab). The game's slots are ed.b:[B 6 (0 · 1 character skills, 2 · 3 \
               spirit skills, 4 · 5 potions; ed.h()V), filled only through ed.d(IB)V (skills: dedupe in the pair, store, \
               ed.f(I)V = a FULL cooldown) and ed.a(ZB)V (potions). Their only callers: the skill windows' popup cb.a(BBBBZ)V \
               — rows 「1번 등록 · 3번 등록」 (7번 · 9번 for the spirit), t/gm.tdf — @140..145 `iload_1; iconst_1; isub; …; iadd; \
               iload_2; invokestatic #77 ed.d(IB)V` and @177..182 the same, and the inventory's ds.a(BB)V @72 `invokestatic \
               #49 ed.a(ZB)V` (「0번 등록 · #에 등록」) — classbytes.py 2026-10-30: `b8 00 4d` at 143 and 180, `b8 00 31` at 72, \
               frames 140 141 177 178 / 65 78 outside. Each -> Hooks.quickRegister(IB)V / quickPotion(ZB)V: off, the original \
               call; on, the first row puts the skill or potion on a plate the player picks and the second takes it off every \
               plate and out of the game's slots (hz_java::quickpage — the rows' words through HZ-011's text hook). A plate \
               is used by seating its entry in a slot of its kind and the slot's own use (ce.a(I)V, ek.a(Z)Z), its cooldown \
               carried by the host. Same length, same stack. Chains after HZ-042 (cb, other method) and HZ-036 (ds, other \
               method).",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::Classes(&[
        ClassTarget { class: "cb", class_md5: "beb5a1ae652ea12abc4564a6236e14ff", rewrite: quick_register },
        ClassTarget { class: "ds", class_md5: "0b7c676bdf0b2bdefd92f087c9852a31", rewrite: quick_potion },
    ]),
}, Patch {
    id: "HZ-048",
    title: "방패치기 = 무기 굴림 (방패 굴림 대신) — 밸런스 2차",
    evidence: "HEROZERO_CLASS_BALANCE.md §6-B (user 2026-10-29 「권장대로」): 방패치기's roll d.a(Lcq;BB)Z @11..69 is the slot-5 \
               item's (the shield's) `bx.b + (bx.c − bx.b) × rand / 100` — `aload_0; getfield db.a; iconst_5; invokevirtual \
               ek.a(B)Lbx;` @11..15 — then db.a(B,I,Z) multiplies (×4.0 at L5, HZ-039). The bench measured 746 a hit at LV \
               99 against 강타 2097. @14 `iconst_5` -> `iconst_3`: the weapon's roll (가디언 세이버 213-255 against the shield's \
               115-173). One byte; frames 90 and 127 outside (classbytes.py 2026-10-29).",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "d", class_md5: "88c26756b980ab1cf4ab93c3371c746d", rewrite: bash_weapon_roll },
}, Patch {
    id: "HZ-031",
    title: "85제 「전설」 장비 — 가디언 모드에서만 떨어지는 102종, 몬스터마다 그 레벨대의 것 (효과는 HZ-032)",
    evidence: "User (2026-10-28): every new item an 85-tier piece, hero2-like, dropped in 가디언 모드; decisions in \
               HEROZERO_NEW_ITEM_DRAFT.md §7 (36 pieces, a new grade 「전설」; §9 the second draft, 60 more, and 6 of the 2nd \
               balance pass — 102). Pieces are host records as HZ-023's \
               (ba.a(BB)Lba; @107 reads /itm/CC record N through rpg/e.a(String,I)[B; the save keeps category, \
               number, stats, h and options, the rest is read at every load), numbers 41.. of their equipment \
               file (HZ-023 = 40). (1) ba @107: the original call -> Hooks.legendRecord; HZ-016's / HZ-023's \
               hook already there stays (they serve the pieces too while HZ-031 is chosen). (2) g.a(BBBB)V @15 \
               `iload 4; bipush 28 | 40; if_icmple` (the ground list's number limit) -> 50. (3) The grade's name \
               colour q.c[bx.e] (q.<clinit>: four ints, white / yellow / pink / green) at q.b(Ldd;IILba;Z)V @13 \
               (the bag's panel) and g.a(BBBZ)V @221 (the pickup notice): `getfield bx.e:B; iaload` -> \
               Hooks.gradeColor([I,Object)I + nop, the table's colour or 0xFFA020 for grade 4. (4) The item \
               window's grade name q.b(Ldd;IILba;)V @104 `invokevirtual cx.a(I)[B` (t/it [28 + grade]; [32] is \
               타입) -> Hooks.gradeName: 「전설」 for 32. (5) The item window cz.a(dd)V @35 `invokevirtual \
               q.b(Ldd;IILba;)V` -> Hooks.itemInfo: the contents as before, then a piece's effect lines in \
               the empty parchment under the option rows (probe 520x240: rows end 125 px under the frame's \
               top, the frame is 174 high). (6) cq.g()V @22 `invokevirtual o()V` -> Hooks.monsterDrop: o() as \
               before, then at round 3 (HZ-030) one draw of the game's Random (ar.a) - 1.5 % (3 % at 난이도 \
               어려움) a piece (2.5 % since the second draft), on the monster's tile (er.s, er.t). Which piece is the \
               monster's (2026-10-31, user 「모든몬스터 말고 다른 난이도에 맞춰서 몬스터별로」, HEROZERO_NEW_ITEM_DRAFT.md §11): \
               a category of its drop slots 0..2 (cq.a:Leq; -> eq.b:[B, what o() reads) and in it the piece whose \
               share of 가디언 levels 85..110 holds its level cq.n - the game's own rule, a slot's category and the \
               number by the level (hz_java::legend::pool); before, any piece. Below round 3 nothing \
               more is drawn, so the Random stream is the game's. The grade feeds et's refine price \
               (grade + 1) x 100 x slot - no table there. Same length everywhere; frames q.b 23/29/47 and g, \
               cz, cq.g (44) sit outside the edits (classbytes.py 2026-10-28). Turned off, a save's pieces read \
               as null and are skipped (HZ-016's path).",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::Classes(&[
        ClassTarget { class: "ba", class_md5: "e1c4edbb48a1823b4486098c78780440", rewrite: legend_records },
        ClassTarget { class: "g", class_md5: "4068de457fd524cea9717857c382d2a4", rewrite: legend_ground },
        ClassTarget { class: "q", class_md5: "5c9837362bfb3a6827831aa78463f195", rewrite: legend_grade },
        ClassTarget { class: "cz", class_md5: "bc1346c7cc82666edc4018779b64d6b1", rewrite: legend_info },
        ClassTarget { class: "cq", class_md5: "acbac4519692a62c390389354fb998ea", rewrite: legend_drop },
    ]),
}, Patch {
    id: "HZ-032",
    title: "85제 「전설」 장비의 효과 — 스킬 두 번·반격 무한·동화 무한·쿨감 등 (입은 조각마다 더함)",
    evidence: "HEROZERO_NEW_ITEM_DRAFT.md §2 / §8 (36 effects, chosen as drafted) + §9 (the second draft, 60 more on the same sites - conditions on HP, SP, guard windows, luck; part-sum, nothing saved: the \
               host reads the worn ek.a:[Lbx; every time). The sites (javap + classbytes.py 2026-10-28; two RE \
               passes over combat and skills): (1) cq.a(Lek;BIZBBZZZ)Z - every basic and skill hit - @103 \
               invokestatic #195 cq.a(II)I (max(5, attack - the monster's d)) -> Hooks.hitDamage(II)I: the game's \
               value x the pieces' percent for the action ek.ac[ek.l] (0..3 basic, 4..9 the job's actives) and \
               the basic hit's own effects; @172 invokevirtual #233 ek.a(BZ)I (crit chance) -> \
               Hooks.critChance. (2) ek.a(Lar;IB)Z @338 invokevirtual #307 ek.c(I)V (the hit's HP change, \
               negated) -> Hooks.takeDamage. (3) cq.g()V @26 invokevirtual #220 n()V (EXP) -> Hooks.monsterExp. \
               (4) ek.C()V (the action driver, frame 0) @354 invokevirtual #236 db.b(B)I (SP, its only caller) \
               -> Hooks.skillSp; @460 invokevirtual #238 db.b(B)Z (turn to the target, once a start after SP \
               and the cooldown) -> Hooks.castStart, which may queue the same action again with ek.f(B)V - the \
               game's own replay (dn @876, av @369) - whose SP is then 0. (5) ed.g(I)V @26 invokevirtual #208 \
               ek.d(B)I / @39 invokevirtual #257 p.d(B)I (the cooldown set when a skill starts) -> \
               Hooks.jobCooldown / spiritCooldown. (6) Durations: eo.a(BI)Z @191 invokevirtual #23 ek.n(I)V \
               (동화 frames) -> fusionStart; dn.a(BI)Z @918 invokestatic #52 h.a(Lek;S)V (분신 and its life; \
               the clone is the static ed.a:Lh) -> cloneLife; cq.c(Lbc;)V @90 invokevirtual #158 ar.b(BI)V \
               (트릭샷's mark 17) -> markStatus; ek.k()V @5 invokevirtual #193 ar.b(BI)V (반격 = status 20 x 100 \
               animation cycles; -1 = until removed) -> counterStatus. (7) Spirit: p.d(B)Z @45 invokevirtual \
               #114 p.a(BI)I (공허의 손's MP cut, (8, cost)) -> spiritCost; p.e(B)I @22 invokevirtual #78 \
               ek.c(I)I (power) -> spiritPower. (8) The jobs' own (third RE pass): ek.a(Lar;IB)Z @119 \
               ar.a(Lar;IBB)V (반격's reflection) -> counterReflect; d.a(BI)Z @139 invokespecial d.a(Lcq;BB)Z \
               (방패치기's hit; the hook calls it per monster of the player's eight directions ar.a(B)Lcq;, \
               stun cq.c(30)) -> shieldBash; ag.a(BI)Z @131 ek.a(BIBZZZ)Z (강타's front hit; + dirs facing+8 / \
               +12 through ek.a(Lcq;BIBBZZZ)Z) -> smash and @394 ek.c(I)V (광기's HP) -> frenzyCost; ai @953 / \
               dn @363 / av.c()Z @9 db.c(B)I read as chances (저격 x2, 독수 +30, 폭발탄 +30); dn @374 ek.c(Z)I \
               (독수's poison roll) x2; cq.a(Lek;BIZBBZZZ)Z @321..331 (11 bytes: if (knock) this.b(dir) -> \
               aload_0 iload_3 iload 4 iload 5 iload 6 invokestatic knock; frame 321 at its first byte, 332 \
               after it, stack empty) - the push as before, 30 % more first; the player's knockback test \
               ek.g()Z / ar.g()Z at cq.a([BSZ)Z @112, at.b()V @49, di.b()V @96, dk.b()V @49 -> knockable; \
               ek.a(Z)Z @196 bt.a(Lba;)I (a potion drunk) -> drank; ar.d()Z @25 getfield f:B (the walking \
               step) -> moveStep: 16 / 16 / 8 from a tile (er.k / er.l clear) for 10 s, so every tile is \
               still entered through ar.e()Z's passability test (probe: x1.5 walking in town). With no 85제 \
               piece worn every hook returns the replaced \
               call's value and draws no Random. Same length, same stack effect; no StackMap frame inside an \
               edit (cq.a 94/154/178, ek.a 328, ek.C 450/470, ed.g 32/42, eo 180/194, dn 906, cq.c 72/94, p \
               25). Chains after HZ-016 / 017 / 018 / 022 / 030 / 031 (other sites).",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::Classes(&[
        ClassTarget { class: "cq", class_md5: "acbac4519692a62c390389354fb998ea", rewrite: effects_cq },
        ClassTarget { class: "ek", class_md5: "a33b0a941d884487e83fe61b75e7d40e", rewrite: effects_ek },
        ClassTarget { class: "ed", class_md5: "9b90d32b52e8ddcc4a90fb8b7ac2e610", rewrite: effects_ed },
        ClassTarget { class: "eo", class_md5: "524e821309d17b3fcecfcc6df8e07a0e", rewrite: effects_eo },
        ClassTarget { class: "dn", class_md5: "0665c61b5e4b443e566a667792e07d0f", rewrite: effects_dn },
        ClassTarget { class: "p", class_md5: "dc69afc6de21ca946a6ca975cfd8699d", rewrite: effects_p },
        ClassTarget { class: "d", class_md5: "88c26756b980ab1cf4ab93c3371c746d", rewrite: effects_d },
        ClassTarget { class: "ag", class_md5: "8f90afed3f3ed75316673f3475461074", rewrite: effects_ag },
        ClassTarget { class: "ai", class_md5: "c522fe51754091ecd9b34d3917601a76", rewrite: effects_ai },
        ClassTarget { class: "av", class_md5: "c3e51434064c3cbcf8444a1f0c0ca10b", rewrite: effects_av },
        ClassTarget { class: "at", class_md5: "cc869f321e3d9d6a39387fd502dc1c79", rewrite: effects_at },
        ClassTarget { class: "di", class_md5: "d4536ca51629cc665125dbf96b6dd39a", rewrite: effects_di },
        ClassTarget { class: "dk", class_md5: "311e7c8d8fc51b535135fe4363567804", rewrite: effects_dk },
        ClassTarget { class: "ar", class_md5: "bdd5d638e0e6c5893781df244dcdb226", rewrite: effects_ar },
    ]),
}, Patch {
    id: "HZ-033",
    title: "그 전 아이템 드랍률 대폭 상승 — 장비 2배·같은 레벨대 여러 종류·헬부터 LV 65 조합 장비·에픽/세트 칸",
    evidence: "User (2026-10-28): with the 85-tier the goal, the game's own items drop far more readily (hero2's way); \
               plan A of HEROZERO_NEW_ITEM_DRAFT.md §5. cq.o()V (the drop, one roll R, HEROZERO_GAME_OPTIONS_RE.md \
               §18): (1) @87 invokestatic #282 rpg/g.a([BI)I (an item slot's cumulative threshold) -> \
               Hooks.dropThreshold (HZ-005's; put here when HZ-005 is not chosen), x2 while HZ-033 is \
               chosen (the 「추가 설정」 rate only with HZ-005). (2) @158 invokestatic #259 Math.min (common \
               number min(lvl/4, 14)) -> Hooks.commonDrop: min, then one draw of the game's Random takes it \
               0..3 under. (3) @215 the rare band's Math.min -> Hooks.rareDrop (HZ-023's, or put here): 0..4 \
               under, never below 13; HZ-023's guardian piece only while HZ-023 is chosen. (4) @271 \
               invokevirtual #250 g.a(BBBB)V (the slot's drop onto the ground) -> Hooks.dropItem: a rare of 26 \
               and up in round 2 or 3 is one in 4 a LV 60..65 piece past 28 of its category (hz_items: the \
               recipe-only LV 65 gear and the price-10 LV 60 rares, 23); g.a(BBBB)V @15's limit 28 -> 31 when \
               no patch raised it. (5) cq.g()V @22 o()V -> Hooks.monsterDrop (HZ-031's, or put here): below \
               round 2 one kill in 50 an 에픽 / 세트 of the monster's level band (LV - 10 .. + 3; 97 of them \
               numbered 28 and under), on its tile. This changes the drop - and the game's Random stream - \
               in every round: the watchline moves (new values with the commit). (6) 2026-11-06 (user: 「제련석이나 \
               포션같은게 안나오는거같은데」): the wider slots ate the game's fixed bands past them - R 14500..22750 \
               potion (20/2, 20/6), 22750..30250 material (16), 30250..43428 제련석 (20/0, 20/1), 43428..47928 recipe \
               (18), 47928..52428 magic stone (17) - a slot's drop @271 being followed by `return` @274: at the \
               「드랍률」 x1 potions 12.6 % -> 0 and materials 11.4 -> 1.9 % a kill (LV 30), at x2 every band 0. @274 \
               return -> nop: after a slot's drop the loop goes on, Hooks.dropThreshold answers 0 for the slots left \
               in that roll (drops::SLOT_TAKEN), and @281 the game's own `R < 14500 -> return` or its band for the \
               same R - every band back at the game's odds, the equipment's boost kept (a kill may give both). No \
               monster's last threshold is past 14500 (96 4833/9666/14500/14500, 3 x 3086.., 3 x 3304..), so with \
               the slots at x1 this is the game's drop exactly. Same length everywhere.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::Classes(&[
        ClassTarget { class: "cq", class_md5: "acbac4519692a62c390389354fb998ea", rewrite: drop_boost },
        ClassTarget { class: "g", class_md5: "4068de457fd524cea9717857c382d2a4", rewrite: drop_boost_ground },
    ]),
}, Patch {
    id: "HZ-034",
    title: "가디언 모드 퀘스트 보상 아이템 — 장비는 가디언 장비, 조합서는 LV 65, 재료·물약 2배",
    evidence: "User (2026-10-28), HEROZERO_NEW_ITEM_DRAFT.md §6 plan A: 가디언 모드 repeats forever (its ending \
               leads back to it), so its quest gear is the guardian piece (HZ-023, LV 80), the 85-tier staying a \
               drop. Script action 16 (eb.b(BLn;)Z case 16, the item reward, three (category, number, count) \
               of which min(round, 2)'s is taken - 헬's in 가디언 모드): @1021 invokestatic #180 ba.a(BB)Lba; \
               (the item) -> Hooks.questItem: at round 3 categories 0..15 -> number 40 (HZ-023's record; \
               the game's own when HZ-023 is off), a recipe (18) not among the LV 65 ones (7..13, 16, 17, \
               26) -> one of them; else as given. @1045 invokevirtual #190 bt.a(Lba;I)I (into the bag) -> \
               Hooks.questGive: the count x2 for categories 16 / 17 / 20 at round 3. The bag is the one of \
               the reward's category (@1032 ed.a(B)Lbt;), the same after the change. Below round 3 both \
               hooks are the calls they replace. Same length; frames 993 / 1089 outside (classbytes.py \
               2026-10-28). Turn-ins (action 12) stay 헬's.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "eb", class_md5: "517894f884b84da221685a5a118ef05c", rewrite: guardian_quest_items },
}, Patch {
    id: "HZ-035",
    title: "85제 조합서 100장 — 가디언 장비 + 최상급 재료 + 종결 장신구, 가디언 모드에서 그 조각을 떨어뜨리는 몬스터가 떨어뜨린다",
    evidence: "User (2026-10-28): every 85제 piece gets its recipe (hero2's rule HERO2_NEW_ITEM_DRAFT.md §6.3: the \
               category's guardian piece + a top material + an end-game accessory). The alchemist dv (javap \
               2026-10-28) lists the 조합 bag's category-18 items (no number limit), the factory ba.a(BB)Lba; \
               makes an es that reads name, price, text, three ingredient pairs and the result pair from the \
               record (es.a([BI)I), and crafting checks one of each unworn ingredient, makes ba.a(result), \
               removes the three and keeps the recipe. So the pieces are host records /itm/18 28.. (28..63 the first 36, 64..123 the second draft's 60 - hz_java::legend::COUNT) \
               (hz_java::legend::recipe_record), served by the record hook at ba @107 (HZ-016's / HZ-023's / \
               HZ-031's while HZ-031 and HZ-035 are chosen, or HZ-031's legendRecord put here). g.a(BBBB)V @15 \
               (the ground list's number limit; 28, or 40 / 50 after HZ-023 / HZ-031) -> 64. cq.g()V @22 o()V -> \
               Hooks.monsterDrop (or kept): at round 3 one kill in 50 (40 since the second draft) a recipe - since \
               2026-10-31 the recipe of one of the pieces that monster drops (HZ-031's hz_java::legend::pool), not any \
               of them. Script action 18, \
               the random box (maps 22 / 32 / 110), takes category 18's number as n/4 of 0..182 - up to 45, \
               which the game never had a record for: @1346 `iload_1; iconst_4; idiv; i2b; istore_1` -> \
               `iload_1; invokestatic Hooks.boxRecipe(I)I; istore_1`, the game's 0..27 (n/4 below 28, n mod 28 \
               past it); frames 1340 / 1351 outside. The monsters' recipe band stays min(3*kind/10, 27).",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::Classes(&[
        ClassTarget { class: "ba", class_md5: "e1c4edbb48a1823b4486098c78780440", rewrite: legend_records },
        ClassTarget { class: "g", class_md5: "4068de457fd524cea9717857c382d2a4", rewrite: recipe_ground },
        ClassTarget { class: "cq", class_md5: "acbac4519692a62c390389354fb998ea", rewrite: recipe_drop },
        ClassTarget { class: "eb", class_md5: "517894f884b84da221685a5a118ef05c", rewrite: recipe_box },
    ]),
}, Patch {
    id: "HZ-036",
    title: "가방 112칸 — 장비·조합·특수 가방이 28칸씩 4쪽 (격자 끝에서 넘기거나 ◀ ▶)",
    evidence: "User (2026-10-28): each bag 28 -> 112. A bag is bt (javap 2026-10-28): a Vector and its size bt.a:B, \
               the constructor's argument; the player's three are ed.a/b/c:Lbt;, made in ed.f()V @4/@16/@28 \
               `bipush 28` (ed.a(B)Lbt;: 0..15 equipment -> a, 16 조합재료 / 18 조합서 -> b, 17 마법석 / 19 퀘스트 / \
               20 특수 -> c). The size only refuses an add (bt.a(Lba;B)Z, bt.a(BBI)Z; buffers sized by it); the \
               save writes the Vector's size as one signed byte and every item (bt.a()[B), the load reads it back \
               with no size check (bt.a([B)V) - so 112 < 128 keeps every save readable, and with the patch off \
               the game keeps every item (it shows the first 28 and writes all). The windows (ds 인벤토리, eg \
               계정창고's 캐릭터창고 side, co selling, dv / et picking) show a fixed 7 x 4 from item 0: q.a(Ldd;\
               Lbt;BZBBI)V @134 and bz.a(Ldd;Lbt;)V @53 read cell i as bt.a(I)Lba; and the cursor's item is \
               bt.a(o.a) (q @309, ds @164, eg @197, co @354 / @172, dv @210 / @107, et @110 / @92). All eleven \
               -> Hooks.bagAt(bag, i): + page x 28 for a player's bag (identity against ed.a/b/c), else as it was. \
               The cursor moves by o.d(II)Z / o.e(II)Z (o.b(B) / o.a(B): 0..27 going round), called by ds @388 / \
               @399, eg @335 / @346, co @555 / @513, dv @773 / @780, et @173 / @180 -> Hooks.bagSideways / \
               bagUpDown: the call, then a cursor that went round on a player's bag turns the page (DOWN on the \
               last row = the next page's first row, UP / LEFT back). ds @44 and eg @118 q.a(...) -> Hooks.bagGrid, \
               co @135, dv @79, et @64 bz.a(Ldd;Lbt;)V -> Hooks.bagGridBz: the draw, then ◀ n/4 ▶ under the tabs \
               (taps there turn it, hz_java::menu_touch). eg.c(B)V @89 `getfield bt.a:B` (the grid's rows = \
               size / 7) -> Hooks.bagCells, at most 28. ea's and bv's equipment lists keep bag places in byte \
               arrays as long as the bag (bt.a(BBZB)[B), whole-bag loops (ed.l, ek.j, eb, dz) are by size(). \
               2026-11-01 (user report: 계정창고 「넣기」 left the item in the bag, copied it into the storage and lost \
               another): the answers that take the cursor's item away remove it by place, bt.a(I)V / bt.a(II)V \
               with o.a (javap: every caller) - ds.a(BB)V @316 (버리기) / @342 (귀환서, n 1), eg.a(BB)V @162 (넣기) / \
               @473 (캐릭터창고 버리기); on a page past the first they took the first page's item at that cell \
               (probe: page 2 cell 0 deposited, page 1 cell 0 gone, and a worn weapon when the cell was its). \
               All four -> Hooks.bagTake / bagTakeSome, the same place as bagAt; eg @396 / @493 take from a 계정창고 \
               page and stay. co / dv / et take by category and number (bt.b(BBI)Z), not by place. \
               Same length at all 31 sites (classbytes.py 2026-10-28 / 2026-11-01; eg @118 is a frame target, same stack); \
               co chains after HZ-010 / HZ-016 / HZ-021, dv / et after HZ-010, ed after its other patches.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::Classes(&[
        ClassTarget { class: "ed", class_md5: "9b90d32b52e8ddcc4a90fb8b7ac2e610", rewrite: bag_size },
        ClassTarget { class: "q", class_md5: "5c9837362bfb3a6827831aa78463f195", rewrite: bag_pages_q },
        ClassTarget { class: "ds", class_md5: "0b7c676bdf0b2bdefd92f087c9852a31", rewrite: bag_pages_ds },
        ClassTarget { class: "eg", class_md5: "527addee529651c1e376570a8e093e4a", rewrite: bag_pages_eg },
        ClassTarget { class: "bz", class_md5: "1911bcba1424b10d6673f4ed67c6f616", rewrite: bag_pages_bz },
        ClassTarget { class: "co", class_md5: "c1a068f6d08e67c7ef490de32f4fa6c8", rewrite: bag_pages_co },
        ClassTarget { class: "dv", class_md5: "85198deab2c8ef577488bbec7de405eb", rewrite: bag_pages_dv },
        ClassTarget { class: "et", class_md5: "135b634d43c74e31e46adf319c796f5e", rewrite: bag_pages_et },
    ]),
}, Patch {
    id: "HZ-037",
    title: "계정창고 112칸 — 28칸씩 4쪽 (원래 3쪽)",
    evidence: "User (2026-10-28): 계정창고 84 -> 112. HOL.storage (dz, javap 2026-10-28) = gold int, a size byte, an \
               item count byte, the items; the load dz.h()V makes (size - 1) / 28 + 1 pages bt(min(28, left)) - \
               the window eg already turns as many pages as there are (dz.a.length, its tabs) - but every writer \
               puts 84 back: dz.h @25 / dz.i @25 and eb.a([[I)V @31 / @381 `bipush 84` (eb's from z, a dated \
               gift into the storage) -> 112. The two loads @50 (dz.h) / @54 (eb) `aload; iconst_4; iinc; baload` \
               (the stored size) -> `bipush 112; iinc; nop`, so a storage saved at 84 opens with four pages at \
               once. Count and size stay under 128. With the patch off the game still reads 112 (four pages) but \
               writes 84 again; a storage of more than 84 items would then not load whole - the overwritten \
               record is in the host's save vault (saves.rs). 2026-11-01: the save dz.i()V walks the pages with \
               `iconst_3` twice (@35 the count, @64 the items) and the gift eb.a([[I)V three times (@292 the room, \
               @392 / @424 the write) - the fourth page was never written -> `iconst_4` (the pages dz.h makes \
               from 112). Same length; no frame inside a span.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::Classes(&[
        ClassTarget { class: "dz", class_md5: "3c527bf0dbb20ea5b419d2f22a3d03eb", rewrite: storage_size_dz },
        ClassTarget { class: "eb", class_md5: "517894f884b84da221685a5a118ef05c", rewrite: storage_size_eb },
    ]),
}, Patch {
    id: "HZ-038",
    title: "아이템 겹치기 127개 (원래 99개 — 물약·재료·조합서 등)",
    evidence: "User (2026-10-28): a stack 99 -> 127, the most its count ba.c:B (a byte, saved as one) holds. The \
               bag bt keeps 99 in three places (javap 2026-10-28): the room test a(BBI)Z @152 / @160, the add \
               a(Lba;I)I @177 / @193 / @198, the merge b()V @45 / @124 / @144 - each `bipush 99` -> `bipush 127`. \
               Stacks are categories 16 and up (bt.a(B)Z). The count is drawn right-aligned in the cell \
               (o.b(Ldd;IIIII)V, digits rpg/e.s), three digits fit. The shop sells at most 99 at a time \
               (co.a(BB)V @223, kept) and the game's own quick bar shows at most 99 (ce, gone with HZ-013). \
               Same length; bt.b @144 is a frame target, same stack.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "bt", class_md5: "2ee41cecf8a7e7d0fc29e90f94fbef73", rewrite: stack_max },
}, Patch {
    id: "HZ-039",
    title: "스킬 표 재조정 — 액티브 상향·SP 인하, 패시브 값·신경가속, 정령 광역 +20 % (HEROZERO_BALANCE_PLAN.md §4-B·C)",
    evidence: "User 2026-10-29 (cafe feedback, the plan's recommended options). The skill numbers are four files the \
               game reads through rpg/e.a(String)[B: /hero/hsd1 (db.b(B)V @0 ldc, @2 invokestatic #36 — six jobs x \
               eight skills x 7 B A·B·C SP cool, value (A + B·L)/C), /hero/hsd2 (ek.a(B)Lek; @32 ldc, @34 \
               invokestatic #382 — twenty passives x A·B·C, x·(A + B·L)/C), /s/d/a and /s/d/p (p.f()V @1/@3 and \
               @18/@20, invokestatic #127 — the spirit's actives 8 B and passives 3 B). Each file ends in a check \
               byte = the sum of the rest mod 256. The jar stays as it is (rule 3): each invokestatic becomes \
               Hooks.table(String)[B, which calls the game's loader and lays hz_java::tables' edits over the \
               bytes it returned, check byte redone (same descriptor, same length; db.b frames 11/26/55/108/117, \
               ek.a 93.., p.f none — classbytes.py 2026-10-29). The level cap is 3 (cb.j), so the values are set \
               for L3 and L5 (HZ-042). Changes the game's numbers in every round: the watchline moves.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::Classes(&[
        ClassTarget { class: "db", class_md5: "30e4ca82f2e0b8abb8a0817570de3d3f", rewrite: tables_db },
        ClassTarget { class: "ek", class_md5: "a33b0a941d884487e83fe61b75e7d40e", rewrite: tables_ek },
        ClassTarget { class: "p", class_md5: "dc69afc6de21ca946a6ca975cfd8699d", rewrite: tables_p },
    ]),
}, Patch {
    id: "HZ-040",
    title: "스탯 공식 — 회피·크리의 민첩 몫 하향, 힘의 공격·방어 몫 상향, HP 건강당 14, 재생 3 초 (HEROZERO_BALANCE_PLAN.md §4-A)",
    evidence: "ek.i()V recomputes the derived stats from c:[I (힘 민첩 건강 의지): attack h @422..582 = 힘 + 민첩/4 + \
               option 6 (+ the job's adjustment: 로그 +민첩/6 −힘/3, 나이트 −힘/2, 건너 −민첩/6 −힘/2) + 살쾡이 \
               a(15, 민첩) + 쇄도 p.a(1) (or 9999 with ce.b), stored by @582 putfield #127 h:I; defence @726..817 = \
               slots 1 4 6 0 (h(B)I) + option 7 (+ 나이트 h(5), 워리어 힘/12) + 갑옷숙련 a(9, sum) + 오러 p.a(0), stored \
               by @817 putfield #93 a:I; max HP @1051..1117 = 10·LV + 12·건강 + option 9 + 내성 p.a(2) (@1082 \
               invokevirtual #368). G()V: crit = 민첩/8 (+건너 민첩/16) + option 5, ≤ 50, + 속임수 i(5), + the hand's \
               bx.h, ≤ 70, + 폭풍 p.b(1) while h:Z, stored by @171/@178 invokespecial #285 a(BIZ)V (2, v, off). \
               H()V: evasion = 민첩/5 (+로그 민첩/10, −나이트 민첩/10) + option 8, ≤ 75, + 회피 i(4), ≤ 80, + 영체화 \
               p.b(2) while i:Z, ≤ 85, stored by @114/@121 a(BIZ)V (0, v, off). c()V (the field tick) refills HP and \
               SP by l:I (의지 + 재생) when p:B / v:B count down from 100 (@163 / @189 bipush 100 — 5 s). The two \
               putfields become Hooks.attack / defence (Object, I)V (same stack: this, value), p.a(2) becomes \
               Hooks.hpBonus(Object, B)I, the four a(BIZ)V calls Hooks.crit / evasion (Object, B, I, Z)V: each hook \
               reads the same inputs (ek's private g/h/i/a(BI) through invokespecial, the spirit's p.a/p.b) and \
               stores hz_java::stats::PATCH's value — whose GAME twin reproduces this bytecode in the tests. The \
               two bipush 100 become bipush 60 (3 s). Same length; no frame inside an edit (i() 574/624/798/848/ \
               1105, G() 167, H() 110, c() 147/168/194 — classbytes.py 2026-10-29). Chains after HZ-004 / 010 / \
               016 / 022 / 032 / 039 on ek (other sites).",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "ek", class_md5: "a33b0a941d884487e83fe61b75e7d40e", rewrite: stat_formulas },
}, Patch {
    id: "HZ-041",
    title: "몬스터 명중 하한 15 % → 30 % (안 맞는 빌드 폐지)",
    evidence: "cq.a(Lar;)Z is the monster's hit roll on the player: 90 + its level − the target's − the target's \
               evasion (ar.a()I = ek.ad[0]), floored by @26..35 `dup; istore_1; bipush 15; if_icmpge 36; bipush 15; \
               istore_1` — a player past 75 evasion was hit 15 % of the time, so 민첩 alone made a character safe \
               (HEROZERO_BALANCE_PLAN.md §2.6). @28 and @33 bipush 15 -> bipush HIT_FLOOR (30) — both: the compare \
               alone set a chance of 15..29 down to 15 (code review 2026-10-29). Same length; frames 9/36/57.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "cq", class_md5: "acbac4519692a62c390389354fb998ea", rewrite: hit_floor },
}, Patch {
    id: "HZ-042",
    title: "직업 스킬 레벨 상한 3 → 5 (패시브·정령은 그대로)",
    evidence: "The skill windows ad (job) and cr (spirit) extend cb, whose level cap is the field cb.j:B — 3 from cb's \
               constructor (@77 iconst_3; putfield #35), 5 from cr's — compared with a skill's level before a raise \
               and for the draw: cb.a(Ldd;IIBZ)V @90, cb.a(ZB)Z @90 and @142, cb.a(BBBBZ)V @18 and @98, cb.b(BBBBZ)V \
               @18 and @82, each `aload_0; getfield #35 j:B` (javap 2026-10-29). ek.a(BB)V (the raise) has no cap of \
               its own. Each getfield becomes invokestatic Hooks.levelCap(Object)I — the same `this` on the stack: \
               5 when the window is an ad on its active tab (cb.e:I == 0), else the field. The passives (tab 1, 2) \
               and the spirit window keep 3 and 5. Same length; no frame on any of the seven.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "cb", class_md5: "beb5a1ae652ea12abc4564a6236e14ff", rewrite: level_cap },
}, Patch {
    id: "HZ-043",
    title: "기본공격 콤보가 SP 를 번다 (3·4·5·6) — 타 피해 100·80·80·120 % (hero2 의 콤보 SP 획득)",
    evidence: "The basic combo (숙련 L3 = four hits) was four full hits for nothing: every hit goes through \
               cq.a(Lek;BIZBBZZZ)Z @103, HZ-032's Hooks.hitDamage site, where the action ek.ac[ek.l] is 0..3 for \
               the combo's steps. The hook (hz_java::hooks HitDamage) weights the step's damage and adds the \
               step's SP to ek.a:[I[1] while HZ-043 is chosen (GuestState::patch_on). Site: HZ-032's hook, or put \
               here when HZ-032 is not chosen.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "cq", class_md5: "acbac4519692a62c390389354fb998ea", rewrite: combo_sp },
}, Patch {
    id: "HZ-044",
    title: "스킬 포인트 — 짝수 레벨 1 + 10 의 배수 레벨 1 + 보스 처치 1",
    evidence: "ek.j(I)V (the EXP loop) @109..129: `aload_0; getfield #141 n:B; iconst_2; irem; ifne 129; aload_0; dup; \
               getfield #107 b:S; iconst_1; iadd; i2s; putfield #107 b:S` — one point at an even level (49 by LV \
               99). 2nd pass (user 2026-10-29, hero2's curve): one more at every level up to 20 (balance::EARLY_LEVELS). \
               99). The twenty bytes become `aload_0; invokestatic Hooks.skillPoints(Object)V` + 16 nops: the hook \
               adds hz_java::balance::skill_points_at(level). The frame at 129 (the ifne's target) stays, now \
               only fallen into. A boss head's death adds one more in HZ-046's hook. Chains after HZ-004 (@32) \
               and HZ-022 (@75) on the same method.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "ek", class_md5: "a33b0a941d884487e83fe61b75e7d40e", rewrite: skill_point_curve },
}, Patch {
    id: "HZ-045",
    title: "보스 반사 피해 — 1/5 무방어에서 1/10·방어 적용·최대 HP 15 % 상한으로",
    evidence: "ak (the boss base) overrides the damage taker a(Lar;I)V: after cq's, `if (d) attacker.a(this, dmg / 5, \
               0, 2)` — ar.a(Lar;IBB)V, which for the player is ek.a(Lar;IBB)V = HP −= dmg outright (no defence, no \
               ÷3, no evasion). The flag d is set in di's constructor for 키메라 part 103 / 126 and ck's for 켈베로스 \
               part 107 / 130, and a(ak) folds a dead part's flags into the head. A 3,000 skill cost 600 HP \
               (유저G: 「액티브 스킬이 봉인되는 보스」). @20 invokevirtual #52 ar.a(Lar;IBB)V becomes \
               Hooks.bossReflect(Object, Object, I, B, B)V: a tenth, less the player's defence (ek.b()I), ÷ 3, at \
               most 15 % of max HP, then the same call. Same length; frame 23.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "ak", class_md5: "c32c38972e807c6e0f8a1e4c89c98d2a", rewrite: boss_reflection },
}, Patch {
    id: "HZ-046",
    title: "보스 드랍 — 레벨대의 레어 50 %·에픽 35 %·세트 15 % 하나 확정, 가디언 모드 보스는 85제 10 % 추가",
    evidence: "ak.o()V (the boss base's drop) is `return`: no boss ever dropped anything (유저F 「5처럼 보스 잡으면 \
               레어템」). Its death still runs cq.g()V @22 o()V — HZ-031 / 033 / 035's Hooks.monsterDrop site; the \
               hook (hz_java::hooks MonsterDrop) draws the game's Random for an `instanceof ak` and puts one \
               piece of hz_java::balance::GRADED on its tile, a boss head also paying the skill point of HZ-044; in \
               가디언 모드 10 % more an 85제 of its own level band (HZ-031's hz_java::legend::pool since 2026-10-31, any \
               piece before). \
               Site: the hook already there, or put here.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "cq", class_md5: "acbac4519692a62c390389354fb998ea", rewrite: boss_drops },
}, Patch {
    id: "HZ-047",
    title: "게임 결함 둘 — 정령 죽음의 검의 거꾸로 된 확률, 나이트 전력방어의 정수 나눗셈 (받는 피해 −(40+10L) %, 4+L 초)",
    evidence: "p.a(Lcq;)Z (죽음의 검 on a hit): `r = rpg/g.a(1, 100); if (r <= b(5, 1)) return false; (boss: miss) kill` — \
               the kill happens when the roll is ABOVE the chance: 100 − 3L % (L1 97 %). @18 if_icmple (a4) -> \
               if_icmpgt (a3): now the chance is the chance (HZ-039 sets it 10 + 5L). ek.a(Lar;IB)Z (a monster's hit \
               on the player) @42..79 for a 나이트 with y:B > 1 (전력방어 up): `dmg *= y / 10` — int, 0 for every y \
               under 10, so the damage became the floor's 5 (÷3 → 1) at any level, and the y of L4+ (9 − 2L ≤ 1) \
               turned the skill off. @63..72 `iload_2; aload_0; getfield #148 y:B; bipush 10; idiv; imul` becomes \
               `iload_2; aload_0; getfield y; invokestatic Hooks.guard(II)I; nop`: the damage × (100 − 40 − 10·L) %, \
               L read back from y (HZ-039's table makes y = 1 + L; the game's 9 − 2L otherwise). ek.b(BB)V @5 \
               invokevirtual #193 ar.b(BI)V (status 21 for 80 frames) becomes Hooks.guardStatus(Object, B, I)V: \
               (4 + L) seconds. Same length; frames 20/35/79 and none in b(BB)V. Chains after HZ-032 (@119, @338) \
               on ek, HZ-032 / 039 on p.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::Classes(&[
        ClassTarget { class: "p", class_md5: "dc69afc6de21ca946a6ca975cfd8699d", rewrite: death_sword },
        ClassTarget { class: "ek", class_md5: "a33b0a941d884487e83fe61b75e7d40e", rewrite: guard_percent },
    ]),
}, Patch {
    id: "HZ-054",
    title: "세로: 타깃 몬스터 창(이름·HP)이 미니맵을 따라간다 (기본 = 게임 자리 맨 위, 미니맵이 그 밑)",
    evidence: "User 2026-10-29: \"세로버전에서 … 우측상단에 미니맵나오고 그 밑에 몬스터 이름과 피 나오게\".                The field HUD ce.f()V draws the target panel (HZ-014) as two calls on rpg/a.a:Ldd:                @40 invokevirtual #185 dd.a(Lcw;III)V - the 80x34 picture ce.a:Lcw (name, frame) at                (x, 0) anchor 20 - and @83 invokevirtual #184 dd.a(IIII)V - the HP bar's fillRect at                (x + 44, 24, w, 2) (classbytes.py 2026-10-29: b6 00 b9 / b6 00 b8; StackMap frames at                101, 104, 107, ... - none between 30 and 86). Each becomes invokestatic of a hook taking                the receiver first (same stack effect, same length) that draws the same thing                GuestState::target_drop guest px lower: the host sets it from its minimap in                portrait - over the minimap, which is the game's own place (drop 0) at its built-in                seat (user, same day: '몬스터 피와 이름이 위에 나오고 그 밑에 미니맵이 나오게'), under it                when the player puts it at the very top - and 0 in landscape and while the minimap is                hidden (= the original). Every fight                map is at least 20 tiles = 320 px tall (hz_maps 2026-10-25), so on the 240x320                portrait canvas the field HUD's clip (ce.e(dd)) is the whole height. Chains after                HZ-010 (@375), HZ-013 (@107, <init>) and HZ-014 (@26) on the same class.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "ce", class_md5: "0feea535bec17058182df9c45fc1cf3c", rewrite: target_drop },
}, Patch {
    id: "HZ-059",
    title: "대각선 이동 — 두 방향을 함께 누르면 8 방향으로 걷는다 (PC 키보드·폰 스틱·십자키·패드)",
    evidence: "User (2026-10-30): 「PC버전에서는 대각선으로도 이동가능하게」 → 「모바일도」 → 십자키도 「어 넣어」 (HEROZERO_DIAGONAL_PLAN.md). \
               The game's direction tables rpg/d.V / W (25 long) already hold 5 ↖ (−1,−1) 6 ↗ 7 ↙ 8 ↘, and the step ar.e(I)V \
               moves er.h / er.i by V[c] · f, W[c] · f: ed.b(B)V with 8 walks the player diagonally (probe --walk-dir, \
               2026-10-30: (208,16) -> (240,48)). The field's arrows reach it only through ce.a(II)Z @513 / 520 / 527 / 534 \
               `iconst_n; invokestatic #205 ed.b(B)V` (b8 00 cd, n = 1 ↑ 2 ↓ 3 ← 4 →; the keypad's 2 4 6 8 map there too) -> \
               Hooks.walk(B)V: the host's diagonal (GuestState::walk_diag) when the arrow is one of its two, else \
               ed.b(arrow) — ed.b(B)V walks it in ar.b 1 · 2 and buffers it in 3 · 9 (ek.g(B)V -> ek.z, only tested \
               > 0 and walked with ed.b(2, z) when the attack ends; an arrow there turned a diagonal held through an \
               OK straight, probe 2026-10-31). The game keeps one key (f.a()V), so the host holds a \
               diagonal as one of its arrows and presses it again when the diagonal changes — the walk, its stop \
               (release -> ed.e()V) and OK cutting in stay the game's. Only thrower: the camera ed.a(ZZ)V reads \
               ed.a:[Z (5 long, 「horizontal?」 for 1..4) with ar.c — AIOOBE every frame (probe log, ce.d()V <- rpg/c.run). \
               ce.d()V @14 `invokestatic #203 ed.a(ZZ)V` (b8 00 cb; eb.a(dd) @40 calls it with false, false — no table) -> \
               Hooks.camera(ZZ)V: facing 5..8, the call once with the horizontal arrow and once with the vertical as ar.c \
               (its @11..52 moves the look-ahead by V / W of the facing, its @55..138 follows the facing's axis), then \
               the diagonal back; 1..4 the call as it was. Dozens of skill and monster reads of ar.c index 5-long tables, \
               so a diagonal facing lives only while walking: ed.b(BB)V @11 `invokevirtual #141 ar.c(B)V` (b6 00 8d — \
               every state-and-direction set: the pending command at a tile's centre ed.d()V @30, the walk's start, \
               the attack from standing ed.b(I)V @62) -> Hooks.face(Object, B)V: a diagonal with any state but 2 \
               becomes its horizontal arrow, and the side-step's way back ar.e:B is dropped — ek.C()V runs ar.w()V \
               (@97, state 2) right after ed.d()V at the same tile centre and would put it over the chosen direction, \
               which a diagonal sliding along a wall (below) meets every tile (probe 2026-10-31: ↙ sliding ↓, ↗ held, \
               slid on to the wall's end — user: 「북서쪽으로 조이스틱 옮겨도 남동쪽으로 계속」); and HZ-006's field tick does the same for the other ways out of a walk \
               (a hit, a scene). A blocked walk tries the game's side-step ar.f()Z = ar.c(ar.c)Z, whose tables \
               rpg/d.X / Y / Z / aa are 5 long — AIOOBE once a block (probe 2026-10-30, ek.h <- ar.d <- ek.c): ar.f()Z @5 \
               `invokevirtual #49 c(B)Z` (b6 00 31) -> Hooks.slide(Object, B)Z: a diagonal steps a tile along whichever \
               of its arrows is open (horizontal first) the way ar.c(B)Z steps aside — e:B = the diagonal, state 2, \
               facing the arrow — and ar.w()V (ek @97) turns it back at the tile's centre; an arrow as it was. The \
               step's check ar.e()Z @24 `invokevirtual #66 g.a(Lar;B)Z` (b6 00 42) reads only the target tile \
               (g.a(Lar;II)Z) -> Hooks.passable(Object, Object, B)Z: a diagonal also needs both arrows' tiles open, \
               so no slipping between two walls that meet at a corner; an arrow as it was (monsters' walks too). \
               The player's picture ek.b(III)V @7 `getfield #63 ar.c:B` (b4 00 3f, `this` on the stack) is the \
               direction t.a(BBIII)I skips to — four blocks a frame, so 5..8 read the next frame's parts (wrong \
               picture; AIOOBE 130 > 14 in cn.a, probe 2026-10-30) -> Hooks.drawFacing(Object)B: a diagonal's \
               horizontal arrow (only monsters read ar.d:B, the setter's copy). The save never holds the facing (ed.a()[B writes ah — the scenario's save point, \
               eb @3040 — and ed.g is the map entry's), so no diagonal reaches a save. Same length, same stack; ce.d \
               frames 20 / 26, ce.a frames 509..540, ar.e frames 14 / 16 / 37, ed.b(BB) and ar.f none are other \
               instructions (classbytes.py 2026-10-30). Chains after HZ-010 / 013 / 014 / 030 / 054 (ce), HZ-006 / \
               030 / 032 / 036 (ed), HZ-032 (ar.d @25) and HZ-004 / 016 / 022 / 032 / 039 / 040 / 044 / 047 / 055 / 056 \
               (ek, ek.b(III)V has no frames) — other methods or other offsets.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::Classes(&[
        ClassTarget { class: "ce", class_md5: "0feea535bec17058182df9c45fc1cf3c", rewrite: diagonal_field },
        ClassTarget { class: "ed", class_md5: "9b90d32b52e8ddcc4a90fb8b7ac2e610", rewrite: diagonal_facing },
        ClassTarget { class: "ar", class_md5: "bdd5d638e0e6c5893781df244dcdb226", rewrite: diagonal_step },
        ClassTarget { class: "ek", class_md5: "a33b0a941d884487e83fe61b75e7d40e", rewrite: diagonal_picture },
    ]),
}, Patch {
    id: "HZ-060",
    title: "「엘릭서」 — 상점 기타 탭에서 1,000 에 사고, 마시면 HP·SP 를 끝까지 채운다",
    evidence: "User (2026-10-30): 「1000원짜리 HP와 SP 100%회복하는 엘릭서 아이템 … 포션 아이콘에서 하얀색계열로」. The \
               game drinks a potion only in ek.a(Z)Z (the 0 / # slots, HZ-058's plates): past the dying states and \
               the potion cooldown ed.a[4], `aload_2; getfield #69 ba.b:B; dup; istore_3; tableswitch 2..8` (@61..67) \
               picks the effect by number — 2 포션 c(100) · 3 포도주 c(1, 80) · 6 하이 포션 c(600) · 7 상급 회복 포션 \
               c(1000) then c(1, 80) · 8 브랜디 c(1, 300) (c(I)V = HP through the game's own rules, c(BI)V adds to \
               a[i] capped at b[i]) — then @186 takes one off the bag (bt.a(Lba;)I — HZ-032's drank — and bt.a(II)V) \
               and ed.i()V. Any other number goes to @183 `goto 209`: true, nothing drunk, nothing taken. No item does \
               both to full, so the elixir is added as HZ-016 added the orb, after it: (1) /itm/20 number 10 (the \
               file has 0..8, the orb is 9) is served by HZ-016's record hook (ba @107; name, price 1000, text, \
               checksum) and its picture by HZ-016's itemIcon (index 20 + 10 = 30, past the pack's 30: 포션's, \
               picture 22, in white — hz_java::elixir); ba.a()Z's 0/# rule (cat 20, id >= 2, not the orb) offers it \
               to the potion keys as it is. (2) co.h()V @315 `bipush 20; bipush 9` (HZ-016's; the file's own 8 \
               refuses — no orb hook, no record) -> 10: the shop's special tab lists 0..10. (3) ek.a(Z)Z @62 \
               `getfield #69 ba.b:B` (b4 00 45; the item on the stack, its number pushed) -> Hooks.potion(Object)I: \
               the elixir fills HP (c(I)V with b[0]) and SP (c(BI)V 1, b[1]) and answers 2 — 포션's c(100) at full \
               HP is nothing — so the drink ends the game's way; any other item its number, the original read. A \
               save holding elixirs loads without them when HZ-060 is off (the file's reader returns null for 10, \
               bt.a([B) skips it — HZ-016's path). Same length; ek.a(Z)Z frames 58 / 61 / 108.. (62 is inside the \
               getfield after the frame at 61), co.h frames 308 / 323 (classbytes.py 2026-10-30). Chains after \
               HZ-016 on co and after HZ-004 / 016 / 022 / 032 / 039 / 040 / 044 / 047 / 055 / 056 / 059 on ek \
               (other methods, or ek.a(Z)Z @196 — HZ-032). \
               2026-11-02 (user report: 헬모드 게힌시티에서 정령사로 직업변경 → 무한로딩, then 이어하기 failed): a quick \
               slot's cooldown is set by ed.g(I)V - slots 0 / 1 from the job's skill (ek.d(B)I), every other slot \
               from the SPIRIT's table, p.d(B)I = the short at id * 8 + 5 of /s/d/a (81 bytes, ten spirit skills) - \
               the 0 / # potion slots 4 / 5 included, by the potion's number: the game's 2..8 land inside the table, \
               the elixir's 10 at 85 -> ArrayIndexOutOfBoundsException (probe: 86 > 81). ed.j()V runs it for all six \
               slots from ek.a(Z)V, which the job change ek.e(B)V (bi's paint, f = 3) and every load (ed.a(Z)Z, \
               이어하기) call - so once a plate (HZ-058) or the 0/# registration had seated the elixir, the job change \
               threw on every paint and stayed on 「직업 변경 중...」 and the save could not be continued (probe: both, \
               the same trace). ed.g(I)V @39 `invokevirtual #257 p.d(B)I` - HZ-032's Hooks.spiritCooldown when that \
               is on (kept), else put here - reads the elixir as the potion it drinks as, 2 (ELIXIR_DRINK); any \
               other number as it was (hz_java::elixir::cooldown_number). Saves already holding it load again.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::Classes(&[
        ClassTarget { class: "co", class_md5: "c1a068f6d08e67c7ef490de32f4fa6c8", rewrite: elixir_shop },
        ClassTarget { class: "ek", class_md5: "a33b0a941d884487e83fe61b75e7d40e", rewrite: elixir_drink },
        ClassTarget { class: "ed", class_md5: "9b90d32b52e8ddcc4a90fb8b7ac2e610", rewrite: elixir_cooldown },
    ]),
}, Patch {
    id: "HZ-061",
    title: "보스 처치 뒤 5 초 카운트다운 — 떨어진 아이템을 줍고 나서 스토리가 이어진다",
    evidence: "User (2026-11-01): 「보스잡고나서 아이템이 드랍되는데 스토리로 바로 넘어가서 아이템을 못먹고있어 … hero2보스레이드 \
               때처럼 카운트다운 5초뒤에 스토리 진행되도록」. Every boss room 121..134 arms one kill counter on entry: tree 0's \
               condition 0 = ed.b(IB)V `ae[kind] = 1` on the boss's head (hz_maps 2026-11-01: 109 110 117 101 115 105 \
               112 118 119 114 99 124 128 122 = balance::BOSS_HEADS; the other counters are the arena's 81..97 × 10 \
               and the tutorial's kind 0 × 3). cq.a(B)V, the state setter, on state 7 calls `ed.c(this.c, eq.f)` \
               @44 (b8 00 e6 invokestatic #230): ae[kind] < 0 → nothing; −1; at 0 → −1 and eb.a(B)Z — the room's \
               story group (dw.a(I)Z loads it, ek.q()V stops the player, eb.c()Z starts it). cq.g()V sets state 7 \
               (@1) BEFORE its drop o()V (@22, HZ-046's hook), so the story's first frame came before anything \
               could be picked up and its warp left the drop behind. @44 becomes Hooks.killCount(IB)V: a boss \
               head's last counted kill writes ae[kind] = −1 as the game does and keeps the group; HZ-006's field \
               tick (ed.d()V @77 — stopped under windows, the 「# 확인」 box, the revive question) counts 100 \
               ticks (5 s at the game's 20) and then calls the game's eb.a(B)Z while the scene is the field \
               (ed.d:B 3) and the player is not falling (ar.b 6 / 7); any other kill calls ed.c(IB)V. The \
               host shows 「스토리 진행까지...N」 in its notice stack, one line replaced in place (hero2's raid \
               countdown). While it waits, every other cq still standing in the room goes by its own m()V (off \
               the grid and the draw list — no drop, EXP, respawn or count): 거인신's (129) two 돌기둥 kind 121 \
               outlive its head and hit through the wait (probe: HP 1103 -> 748), as the story's warp never let \
               them. Nothing else can happen in the wait: the rooms have no step trigger and their list 0 \
               turns saving and the return scroll off (actions 45:00 g.e, 57:00 g.d); a game over (title, scene \
               2 / 7) or another map drops the wait. Needs HZ-006 (without its tick the story would never come: \
               the hook then calls the game at once). Same length, same stack ((IB)V for (IB)V); the frame at \
               44 is before the call. Chains after HZ-005 / 017 / 018 / 023 / 030 / 031 / 032 / 033 / 035 / 041 / \
               043 / 046 / 057 on cq (other methods).",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "cq", class_md5: "acbac4519692a62c390389354fb998ea", rewrite: boss_story_wait },
}, Patch {
    id: "HZ-062",
    title: "게임 결함 — 상점에서 같은 이름의 장비가 둘 이상이면 고른 것이 아니라 뒤쪽 것이 팔리던 것 (메뉴 상점·NPC 상점)",
    evidence: "User (2026-11-02): 「상점에 장비 판매시 같은 이름의 장비가 있으면 판매하려는 장비 우측의 장비가 팔리는 버그 … \
               메뉴 상점, npc상점 동일」. Both shops are co (javap 2026-11-02). Selling keeps the cursor's item in co.a:Lba \
               (co.a(II)Z @354 bt.a(I) - HZ-036's bagAt; a worn item is refused right there, text 23) and the answer \
               co.a(BB)V takes it with ed.a(cat).b(cat, id, n) - @353 one, @467 n of a stack - and bt.b(BBI)Z walks the \
               bag from its END for the first unworn item of that category and number: of two of one name the later \
               one (to the right, or on a later page) went, whatever its grade, options or 강화, and the chosen one \
               stayed. An original-game defect (patches none: the same walk). @342 / @456 `getfield #28 ba.a:B` \
               (b4 00 1c, the category under the number) -> three nops, so the chosen item itself is on the stack, \
               and @353 / @467 `invokevirtual #64 bt.b(BBI)Z` (b6 00 40) -> Hooks.sellTake(Object bag, Object item, \
               B id, I n)Z: the chosen item, found by identity in the bag, gives min(n, its count) by place \
               (bt.a(II)V - off the bag at 0), any rest goes the game's way (bt.b(BBI)Z - a stack sold past the \
               chosen one); not found, the game's call as it was. bt.b's own tidy (its private b()V, merging part \
               stacks) still follows, as bt.b(cat, id, 0). The money, ed.e(a.a / 10) per item, was the chosen \
               item's already. Same length; the frames are at 328 / 389 / 442 / 471 - none inside 338..356 or \
               452..470 (classbytes.py 2026-11-02). Chains after HZ-010 / 016 / 021 / 036 / 060 on co (other \
               methods or offsets).",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "co", class_md5: "c1a068f6d08e67c7ef490de32f4fa6c8", rewrite: sell_chosen },
}, Patch {
    id: "HZ-063",
    title: "게임 결함 — 「수상한 남자」 의뢰를 거절하면 카르멘이 사라지고 미행(엘피스 루트)·정령3 이 멈추던 것",
    evidence: "User (2026-11-03): 「헬모드 게힌시티 치안관이랑 대화한 뒤에 미행이 따라와야 하는데 안 따라온다」, and the \
               known 「정령3 퀘스트 진행 중 카르멘 실종」. Map scripts (conditions eb.a(BLn;)Z, actions eb.b(BLn;)Z, javap; \
               decoded with tools_re/hz_maps.py's tables, 2026-11-03): story flag 48 is turned on in two places - map 2 \
               (은거지) list 1, the masked figure's scene (tree 0 branch 1: round > 0, the Karmen route's quest 11 = 2; \
               QUEST[31] = 1, then FLAG 48 / 62 / 28: Karmen spent, 「결전」 begins), and map 4 (게힌시티) list 34 option \
               84, turning down 「수상한 남자」 (NPC (23,6) tree 19, sub quest 60 「수상한 일」). About thirty maps' entry \
               trees read 48 as 'Karmen is gone' and hide the companion (a52 00); map 22 (중앙도서관) tree 2 tests it before \
               flag 42, so the Elpis route's library scene (list 9: the man who tails, flag 43) becomes the book text \
               (list 8) and map 4 tree 17 (the Elpis route's 미행, flags 17 + 43 at (3,18) / (8,20..22)) never runs; map 2 \
               tree 0 tests it before branch 5 (round 0, quest 28 = 1: Karmen waiting outside), so 정령(3) never ends. Probe \
               2026-11-03: after the refusal the man says '.....' (tree 19 branch 3), the library is the book only with no \
               Karmen, leaving the 은거지 shows no Karmen and no scene; the same runs without the refusal play them. An \
               original-game defect (no patch involved). Quest 31 is only ever set to 1 (maps 2 and 114) and the flags \
               only reset at the ending (ed.a()V command 8 clears ac[0..31)), so the story's 48 always comes with round \
               > 0 (ed.h), quest 11 = 2 and quest 31 >= 1 (ed.a(I)B) - hz_java::story::karmen_away_real. @421 \
               invokestatic #260 ed.a(I)Z (b8 01 04, condition 7's flag test) -> Hooks.scriptFlag(I)Z: 48 reads on only \
               with those three (quest 11 >= 2: its writers are 1 and 2 only, hz_maps 2026-11-03 - a save that refused \
               before the write was gated heals), any other flag the game's. @2836 invokestatic #268 ed.c(I)V (b8 01 0c, \
               action 53's 'on') -> Hooks.scriptFlagOn(I)V: 48 is written only from the story's own map (the player's \
               map ed.a:Lg; -> g.b:B is 2, 은거지 - the refusal runs on map 4, so whatever the quests say it is not \
               written: the man offers again next time, his '!' back; code review 2026-11-03: a refusal on the Karmen \
               route past 「결전」 would otherwise pass the quest test), any other the game's call. The Karmen route's \
               미행 (map 4 tree 15 at (6,23)) never needed the library and is unchanged. Same length; no frame starts at 421..423 \
               (398 / 429) nor 2836..2838 (2835 / 2839) (classbytes.py 2026-11-03). Chains after HZ-013 / 030 / 034 / \
               035 / 037 on eb (other offsets).",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "eb", class_md5: "517894f884b84da221685a5a118ef05c", rewrite: karmen_flag },
}, Patch {
    id: "HZ-064",
    title: "게임 결함 — 건너의 조준사격·저격을 맞은 거인신(움직이지 않는 몬스터)이 다시는 공격하지 않던 것",
    evidence: "The known tip (2026-11-03): 건너's 조준사격 on 거인신 leaves it unable to act. ai.a(BI)Z (javap): 조준사격 \
               (skill 5) stage 0 picks its target (ai.a()Lcq;) and calls target.l.b(14) @330 (invokevirtual #52 \
               l.b(I)V: l.a:I = 14, a hold cq.c()V counts down to 0) with an aim mark aq(14, 14); 저격 (skill 9) does \
               the same at @891. A monster that never walks (ar.f <= 0 - 거인신 be, kind 119, among them) is made \
               with l.a:I = -1 (cq.<init> @103 l.b(-1)), and cq.l()V calls its AI k()V while it is off a tile only \
               when l.a:I is -1 (@115..128). The hold writes 14, counts to 0 and never back to -1: the boss never \
               picks another attack. Probe 2026-11-03 (gunner LV 99 beside it in map 129, 30 s): standing, hits \
               taken through the whole run (15); basic attacks, the same (12); one 조준사격 at the start, none after \
               it - the boss stands with its arms down. The same with HZ-045 / 046 / 057 / 061 off: the game's own. \
               @330 / @891 invokevirtual #52 l.b(I)V (b6 00 34) -> Hooks.aimHold(Object, I)V: a target whose l.a:I \
               is -1 keeps it, any other the game's l.b(I)V. The mark and the shot stay the game's (the boss is still \
               only hit in its open phase: be.a(ek, ...) aims at -90 unless cq.h is 7). Same length; no frame starts \
               at 330..332 (304 / 354) nor 891..893 (864 / 915). Chains after HZ-032 on ai (@953).",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "ai", class_md5: "c522fe51754091ecd9b34d3917601a76", rewrite: aim_hold },
}, Patch {
    id: "HZ-065",
    title: "게임 결함 — 가방이 가득 차면 퀘스트 보상을 못 받고 퀘스트가 멈추던 것 (보상은 다음 저장 때 계정창고로, 대본은 끝까지)",
    evidence: "The known defect (2026-11-03): a quest reward into a full bag gives the EXP but not the item and the \
               quest does not end. Script action 16 (eb.b(BLn;)Z @970..1158, javap): the round's (category, number, \
               count) goes into its bag with bt.a(Lba;I)I @1045; fewer than asked -> the added ones taken back, \
               rpg/e.g [53] 「가방에 공간이 부족합니다.」 (eb.a([B)V) and a yield - and closing that box (eb.a(II)Z, \
               keys 8 / 53) runs eb.e()V -> dw.c()V, which throws the rest of the action list away (dw.b()Z: an \
               action that answers false is resumed next frame; the list is only ever dropped there). What ran \
               before the give stays: map 25 (게힌 여관) list 29, 듀라이's 「케네스의 일기 4」 (sub quest 77, round > 0), \
               takes the diary (action 12) and gives the EXP before the accessory, so the quest stays open with the \
               diary gone for good - probe 2026-11-03 (a save crafted with quest 77 under way, the equipment bag \
               112 / 112): LV 1 -> 15, no accessory, no diary, his default line after. The same shape: map 22 list 15 \
               (a second give after the quest's end), map 23 list 11 and map 86 list 8 (gives after gives). Most \
               lists give first and only lose a turn. The fix (hz_java::reward): @1045 -> Hooks.questGive (HZ-034's, \
               or put here): what the bag did not take waits for the next save - only when the 계정창고 has room for \
               it now (else the game's failure as it was) - and the whole count is answered, so the list goes on; \
               the host says 「가방이 가득 차 보상은 저장할 때 계정창고로 들어갑니다」. The save: ed.k()V @143 \
               invokestatic #261 rpg/b.a([BB)Z (b8 01 05, the slot's progress record HOL.gamedata<n> - the \
               quests' states; the header rpg/b.c @27 before it, the item record l()V @147 after) -> \
               Hooks.slotSave([BB)Z: the game's write, then with rewards waiting the record READ BACK with the \
               game's reader rpg/b.a(B)[B against the payload (rpg/b's writer catches the record store's exception, \
               @383 printStackTrace, and still answers true, @397 iconst_1, so only the record can say) - not on \
               the disk: the rewards wait as they were, the progress is unsaved too. On the disk: each waiting \
               reward made again (ba.a(BB)) into the first page with room (bt.a(BBI)Z, bt.a(Lba;I)I - what it took \
               counted, the page's cells snapshotted before and after so exactly the change is known: the game \
               merges into the stacks of the kind first and puts only the rest into a cell of its own, inserted in \
               order), the storage written with the game's dz.i()V and read back (rpg/b.b()[B against the payload \
               dz.i()V @138 handed rpg/b.b([B)Z - invokestatic #68, b8 00 44 -> Hooks.storageWrite([B)Z keeps it \
               while the save has armed it). Read back the same: 「보상을 계정창고에 넣었습니다」; refused: exactly \
               what this save added is taken back (bt.a(II)V cell by cell, highest first - the rest of the storage \
               stays, it may hold what only it holds after an earlier refused write), the rewards keep waiting for \
               the next save, 「보상을 계정창고에 저장하지 못했습니다 - 다시 저장하세요」 (probe: \
               HZ_PROBE_RMS_FAIL=HOL.storage); the write made but the read-back itself failing: taken as written, \
               nothing queued twice; no room by then: they wait, 「계정창고가 가득 차 보상을 넣지 못했습니다 - \
               창고를 비우고 저장하세요」. Then the rest of the game's save. So the reward reaches the disk only \
               with the progress that earned it, never before it (code review 2026-11-03, twice: writing the \
               storage at the reward, or before the slot's records, put it on disk while the quest stayed \
               unsaved - quit without saving and it could be had again; a refused write was taken for a stored \
               reward; a take-back by kind took the wrong cells). A reward a save went past is owed: it outlives \
               the process in files/hz_rewards.txt (hz_java::reward::FILE) until a save puts it away; one no save \
               went past is dropped with its progress when the game is left (the title, ed.d 2, or a game over, 7 - \
               HZ-008's blit hook). Same length everywhere; frames: eb 993 / 1089, ed.k has none, dz.i 134 / 143 \
               (classbytes.py 2026-11-03). Chains after HZ-034 on eb (@1045 kept), HZ-036 / 059 / 060 on ed (other \
               methods), HZ-037 on dz (dz.i @25 / 35 / 64).",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::Classes(&[
        ClassTarget { class: "eb", class_md5: "517894f884b84da221685a5a118ef05c", rewrite: reward_overflow },
        ClassTarget { class: "ed", class_md5: "9b90d32b52e8ddcc4a90fb8b7ac2e610", rewrite: reward_slot_save },
        ClassTarget { class: "dz", class_md5: "3c527bf0dbb20ea5b419d2f22a3d03eb", rewrite: reward_storage_write },
    ]),
}, Patch {
    id: "HZ-066",
    title: "게임 결함 — 계정창고 「넣기」 가 캐릭터 가방 탭 번호의 쪽에만 들어가던 것 (장비는 1쪽 28칸뿐, 빈 쪽이 있어도 「창고의 공간이 부족합니다」)",
    evidence: "User (2026-11-03): 「계정창고가 최대 4페이지인데 실제로는 1페이지로 동작하는 것 같다」. eg.a(BB)V (javap \
               2026-11-03), 「넣기」 on the 캐릭터창고 side (eg.g:Z, answer 6 / 0): @66 `getstatic dz.a:[Lbt; aload_0 \
               getfield eg.g:B aaload` and the same @126 - eg.g:B is the tab the window shows, on this side the \
               CHARACTER bag's (eg.c(B)V: 0 ed.a() 장비 · 1 ed.c() 조합 · 2 ed.b() 특수) - so a deposit goes to \
               storage page tab + 1: equipment page 1 only, 조합 page 2, 특수 page 3, page 4 (HZ-037) never; @95 \
               bt.a(BBI)Z on that page fails at 28 -> nui[55] 「창고의 공간이 부족합니다」 with the other pages empty. \
               The storage is flat everywhere else: its tabs draw one icon for every page (o.a(Ldd;BZIZ)V with the \
               last arg false: rpg/e.b[0 / 3], the character side's are per kind), the load dz.h()V fills page \
               after page in the record's order (a reload moves a page-3 potion to page 2 behind 28 equipment, and \
               a page 1 of 10 equipment + 20 potion stacks on page 3 reloads as a full page 1 - equipment refused \
               with 82 cells free), the gift eb and HZ-065's rewards take the first page with room. Probe \
               2026-11-03 (a new game, 29 equipment given, menu 8 -> 아이템 -> OK OK OK x 29): 28 to page 1, the \
               29th 「창고의 공간이 부족합니다」 with pages 2-4 empty; a potion from the 특수 tab to page 3. An \
               original-game defect (no patch touches @66..145; the original's 84 gave equipment 28). @95 \
               invokevirtual #53 bt.a(BBI)Z (b6 00 35) -> Hooks.storageRoom(Object page, BBI)Z and @145 \
               invokevirtual #56 bt.a(Lba;I)I (b6 00 38) -> Hooks.storagePut(Object page, Object item, I)I: both \
               take the 계정창고 page hz_java::bags::deposit_page picks - for a stack kind (category 16 and up, \
               bt.a(B)Z) the first page holding that kind with room (bt.a(BB)I > 0, bt.a(BBI)Z), else the first \
               page with room - and call the game's own method on it; no page with room, or a page that is not \
               one of dz.a's: the page the game handed (its refusal as it was). The same storage state at both \
               calls (nothing between them writes it), so the same page. The rest is the game's: the worn test, \
               dz.i()V's write of every page, HZ-036's bagTake out of the bag, 「아이템이 창고로 옮겨졌습니다」. \
               Same length; StackMap frames at 66 / 116 / 198 - none inside 95..97 or 145..147 (classbytes.py \
               2026-11-03). Chains after HZ-036 on eg (@162 / @473 / other methods).",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "eg", class_md5: "527addee529651c1e376570a8e093e4a", rewrite: storage_deposit },
}, Patch {
    id: "HZ-067",
    title: "85제 일곱 종의 「모든 피해의 2% HP 회복」 — 몬스터 HP 가 실제로 깎이는 자리에서 (정령·반격·분신·독까지)",
    evidence: "User (2026-11-04): 각 직업 85제 일부에 모든 피해 흡혈 — 흡혈의 검 · 피의 도끼 · 흡혈 총검 · 생명의 반지, 로그 \
               · 건너 · 정령사 것도 찾아서. The two lifesteals there are do not cover 「모든 피해」 (javap 2026-11-04): \
               the game's own (set 10 %) is ek.q(I)V, called only from the player's hit cq.a(Lek;BIZBBZZZ)Z @149 / \
               @301, and 피의 도끼's is HZ-032's Hooks.hitDamage in the same hit; a spirit's attack cq.a(IB)Z @89, \
               반격's reflection ek.a(Lar;IB)Z @119, the clone's h -> cq.b(Lar;IB)Z -> cq.a(Lar;IB)Z @54 and 독수's \
               tick cq.b(Lbc;)V pass neither. Every one of them lands on cq.a(Lar;I)V, which alone takes a monster's \
               HP (cq.i:I): called from cq.a(Lar;IBB)V @84 (invokevirtual #198; the four hit paths, attacker on the \
               stack: the player, or the clone h) and cq.b(Lbc;)V @103 (the same; aconst_null - the tick; only dn @384 \
               sets cq.b:I, 독수), and am.e()V @11 (a monster ending itself: not hooked, the player did not do it). \
               Both -> Hooks.monsterHurt(Object cq, Object attacker, I)V: the stack is the call's. It calls the game's \
               method virtually (ak's reflection, cy, bs passing the hit to the body be.a all stay) and heals by \
               the HP that really came off - cq.c()I before and after, virtual because bs answers for be.a, so a \
               killing blow counts what was left, not its size. Attacker the player, the clone or null only: a \
               monster's hit (on the clone) gives nothing. Nothing worn: the game's call alone, so no change to \
               anything without the seven pieces. The heal is the game's ek.c(I)V, as ek.q's; not at HP 0 (ak's \
               reflection or a poison tick can land after the player's fall, and a heal would stand them up in \
               the game's death). Same length; no StackMap frame inside 84..86 or 103..105.",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "cq", class_md5: "acbac4519692a62c390389354fb998ea", rewrite: monster_hurt },
}, Patch {
    id: "HZ-068",
    title: "넷째 스탯 이름을 「의지」 로 통일 — 캐릭터 창의 「지력」 그림 · 패시브 「고양」 의 「정신력」",
    evidence: "User (2026-11-04): 「의지로 맞추자」. One stat, three names (scan of every t/*.tdf and jar member, EUC-KR, \
               2026-11-04): ek.i()V @172..232 adds a piece's options n, n + 10 and 15 to stat n, so t/it.tdf [38] 「의지+5」, \
               [48] 「의지+8」 and the 마석 /itm/17 「의지의 마석 · 영석」 raise ek.c:[I[3], the value dy.a(Ldd;IIB)V draws \
               beside 지력. 의지 also in t/h.tdf [11], t/lot.tdf [11] [15], t/sp.tdf [13] and HZ-031's 정령사의 반지; \
               지력 in no text at all - dy draws its four names from the picture rpg/e.w (@19 getstatic #38; dd.b clip \
               26 x 14, the sheet slid by ((n + 3) % 4) x 26), and no other class reads that field; 정신력 only in t/s5.tdf \
               [15], the 정령사's 고양, which ek.i()V @370..388 adds to stat 3 (ed.b = 5). Fix to the word nearly all \
               of it says: @19 getstatic #38 rpg/e.w:Lcw; (b2 00 26) -> invokestatic Hooks.statNames()Object: a copy of \
               the sheet with cell 2 redrawn (hz_java::statname::will_sheet: the game's 지 moved to the right half, 의 in \
               the sheet's two inks on the left), made once through the game's cw.a([IIIZ)Lcw;, as HZ-060's elixir; a \
               sheet of another shape stays the game's. The line through HZ-011's Hooks.text (with HZ-011 off it stays \
               정신력). A picture and a line only: no stat, record or save changes. Same length; no StackMap frame in \
               dy.a(Ldd;IIB)V (classbytes.py 2026-11-04). Chains after HZ-012 on dy (another method).",
    builds: &[Build::Final],
    default_on: true,
    kind: PatchKind::ClassRewrite { class: "dy", class_md5: "a0ce6f6b80edcff98979083bdf858341", rewrite: stat_names },
}];

/// HZ-068 `dy`: the stat names' sheet goes through the hook.
fn stat_names(class: &[u8]) -> Option<Vec<u8>> {
    // getstatic #38 rpg/e.w:Lcw;
    call_hook(class, "a", "(Ldd;IIB)V", 19, &[0xb2, 0x00, 0x26], hooks::STAT_NAMES, invokestatic)
}

/// HZ-067 `cq`: the two calls that take HP off a monster go through the hook.
fn monster_hurt(class: &[u8]) -> Option<Vec<u8>> {
    // invokevirtual #198 a(Lar;I)V
    let c = call_hook(class, "a", "(Lar;IBB)V", 84, &[0xb6, 0x00, 0xc6], hooks::MONSTER_HURT, invokestatic)?;
    call_hook(&c, "b", "(Lbc;)V", 103, &[0xb6, 0x00, 0xc6], hooks::MONSTER_HURT, invokestatic)
}

/// HZ-066 `eg` (계정창고's window): 「넣기」's room test and its add go through the hooks, which take the
/// 계정창고 page with room instead of the one numbered like the character side's tab.
fn storage_deposit(class: &[u8]) -> Option<Vec<u8>> {
    // invokevirtual #53 bt.a(BBI)Z
    let c = call_hook(class, "a", "(BB)V", 95, &[0xb6, 0x00, 0x35], hooks::STORAGE_ROOM, invokestatic)?;
    // invokevirtual #56 bt.a(Lba;I)I
    call_hook(&c, "a", "(BB)V", 145, &[0xb6, 0x00, 0x38], hooks::STORAGE_PUT, invokestatic)
}

/// HZ-063 `eb`: condition 7's flag test and action 53's flag set go through the hooks.
fn karmen_flag(class: &[u8]) -> Option<Vec<u8>> {
    // invokestatic #260 ed.a(I)Z
    let c = call_hook(class, "a", "(BLn;)Z", 421, &[0xb8, 0x01, 0x04], hooks::SCRIPT_FLAG, invokestatic)?;
    // invokestatic #268 ed.c(I)V
    call_hook(&c, "b", "(BLn;)Z", 2836, &[0xb8, 0x01, 0x0c], hooks::SCRIPT_FLAG_ON, invokestatic)
}

/// HZ-064 `ai` (건너): 조준사격's and 저격's hold on their target goes through the hook.
fn aim_hold(class: &[u8]) -> Option<Vec<u8>> {
    let mut c = class.to_vec();
    for at in [330, 891] {
        // invokevirtual #52 l.b(I)V
        c = call_hook(&c, "a", "(BI)Z", at, &[0xb6, 0x00, 0x34], hooks::AIM_HOLD, invokestatic)?;
    }
    Some(c)
}

/// HZ-065 `eb`: script action 16's give goes through the hook - HZ-034's when it is there.
fn reward_overflow(class: &[u8]) -> Option<Vec<u8>> {
    // invokevirtual #190 bt.a(Lba;I)I
    hook_or_keep(class, "b", "(BLn;)Z", 1045, &[0xb6, 0x00, 0xbe], hooks::QUEST_GIVE, hooks::QUEST_GIVE)
}

/// HZ-065 `ed`: a save of the slot puts the waiting rewards away once its progress record is written.
fn reward_slot_save(class: &[u8]) -> Option<Vec<u8>> {
    // invokestatic #261 rpg/b.a([BB)Z
    call_hook(class, "k", "()V", 143, &[0xb8, 0x01, 0x05], hooks::SLOT_SAVE, invokestatic)
}

/// HZ-065 `dz`: the storage's write hands its payload to the hook.
fn reward_storage_write(class: &[u8]) -> Option<Vec<u8>> {
    // invokestatic #68 rpg/b.b([B)Z
    call_hook(class, "i", "()V", 138, &[0xb8, 0x00, 0x44], hooks::STORAGE_WRITE, invokestatic)
}

/// HZ-062 `co`: the sale takes the item the cursor chose — the category read under its number is
/// dropped so the item itself is on the stack, and the take goes through the hook.
fn sell_chosen(class: &[u8]) -> Option<Vec<u8>> {
    let mut c = class.to_vec();
    for (category, take) in [(342, 353), (456, 467)] {
        // getfield #28 ba.a:B -> nop x3
        c = classfile::rewrite_code(&c, "a", "(BB)V", category, &[0xb4, 0x00, 0x1c], &[0x00, 0x00, 0x00])?;
        // invokevirtual #64 bt.b(BBI)Z
        c = call_hook(&c, "a", "(BB)V", take, &[0xb6, 0x00, 0x40], hooks::SELL_TAKE, invokestatic)?;
    }
    Some(c)
}

/// HZ-060 `ed`: a quick slot's cooldown reads the elixir as the potion it drinks as — HZ-032's hook
/// already there, or put here.
fn elixir_cooldown(class: &[u8]) -> Option<Vec<u8>> {
    // invokevirtual #257 p.d(B)I
    hook_or_keep(class, "g", "(I)V", 39, &[0xb6, 0x01, 0x01], hooks::SPIRIT_COOLDOWN, hooks::SPIRIT_COOLDOWN)
}

/// HZ-061 `cq`: a monster's death counts its kill through the hook.
fn boss_story_wait(class: &[u8]) -> Option<Vec<u8>> {
    // invokestatic #230 ed.c(IB)V
    call_hook(class, "a", "(B)V", 44, &[0xb8, 0x00, 0xe6], hooks::KILL_COUNT, invokestatic)
}

/// HZ-060 `co` (the shop): the special tab lists numbers 0..10 — the elixir after HZ-016's orb.
fn elixir_shop(class: &[u8]) -> Option<Vec<u8>> {
    // bipush 20; bipush 9 (HZ-016's last number)
    classfile::rewrite_code(class, "h", "()V", 315, &[0x10, 0x14, 0x10, hooks::REVIVE_ORB.1 as u8], &[0x10, 0x14, 0x10, hooks::ELIXIR.1 as u8])
}

/// HZ-060 `ek`: the drink reads the potion's number through the hook.
fn elixir_drink(class: &[u8]) -> Option<Vec<u8>> {
    // getfield #69 ba.b:B
    call_hook(class, "a", "(Z)Z", 62, &[0xb4, 0x00, 0x45], hooks::POTION, invokestatic)
}

/// HZ-059 `ek`: the player's picture reads its direction through the hook.
fn diagonal_picture(class: &[u8]) -> Option<Vec<u8>> {
    // getfield #63 ar.c:B
    call_hook(class, "b", "(III)V", 7, &[0xb4, 0x00, 0x3f], hooks::DRAW_FACING, invokestatic)
}

/// HZ-059 `ce`: the field's four arrows call the walk hook, the camera's follow its hook.
fn diagonal_field(class: &[u8]) -> Option<Vec<u8>> {
    // invokestatic #205 ed.b(B)V, after iconst_1 · 2 · 3 · 4
    let mut c = class.to_vec();
    for at in [513, 520, 527, 534] {
        c = call_hook(&c, "a", "(II)Z", at, &[0xb8, 0x00, 0xcd], hooks::WALK, invokestatic)?;
    }
    // invokestatic #203 ed.a(ZZ)V
    call_hook(&c, "d", "()V", 14, &[0xb8, 0x00, 0xcb], hooks::CAMERA, invokestatic)
}

/// HZ-059 `ar`: the step's open-tile check and the blocked walk's side-step call the hooks.
fn diagonal_step(class: &[u8]) -> Option<Vec<u8>> {
    // invokevirtual #66 g.a(Lar;B)Z
    let c = call_hook(class, "e", "()Z", 24, &[0xb6, 0x00, 0x42], hooks::PASSABLE, invokestatic)?;
    // invokevirtual #49 c(B)Z
    call_hook(&c, "f", "()Z", 5, &[0xb6, 0x00, 0x31], hooks::SLIDE, invokestatic)
}

/// HZ-059 `ed`: the state-and-direction setter's facing calls the hook.
fn diagonal_facing(class: &[u8]) -> Option<Vec<u8>> {
    // invokevirtual #141 ar.c(B)V
    call_hook(class, "b", "(BB)V", 11, &[0xb6, 0x00, 0x8d], hooks::FACE, invokestatic)
}

/// HZ-039 `db`: the job skill table.
fn tables_db(class: &[u8]) -> Option<Vec<u8>> {
    // invokestatic #36 rpg/e.a(Ljava/lang/String;)[B
    call_hook(class, "b", "(B)V", 2, &[0xb8, 0x00, 0x24], hooks::TABLE, invokestatic)
}

/// HZ-039 `ek`: the passive table.
fn tables_ek(class: &[u8]) -> Option<Vec<u8>> {
    // invokestatic #382 rpg/e.a(Ljava/lang/String;)[B
    call_hook(class, "a", "(B)Lek;", 34, &[0xb8, 0x01, 0x7e], hooks::TABLE, invokestatic)
}

/// HZ-039 `p`: the spirit's two tables.
fn tables_p(class: &[u8]) -> Option<Vec<u8>> {
    // invokestatic #127 rpg/e.a(Ljava/lang/String;)[B
    let c = call_hook(class, "f", "()V", 3, &[0xb8, 0x00, 0x7f], hooks::TABLE, invokestatic)?;
    call_hook(&c, "f", "()V", 20, &[0xb8, 0x00, 0x7f], hooks::TABLE, invokestatic)
}

/// HZ-040 `ek`: attack, defence, max HP, crit, evasion — and the refill every 3 s.
fn stat_formulas(class: &[u8]) -> Option<Vec<u8>> {
    // putfield #127 h:I; putfield #93 a:I; invokevirtual #368 p.a(B)I
    let c = call_hook(class, "i", "()V", 582, &[0xb5, 0x00, 0x7f], hooks::ATTACK, invokestatic)?;
    let c = call_hook(&c, "i", "()V", 817, &[0xb5, 0x00, 0x5d], hooks::DEFENCE, invokestatic)?;
    let c = call_hook(&c, "i", "()V", 1082, &[0xb6, 0x01, 0x70], hooks::HP_BONUS, invokestatic)?;
    // invokespecial #285 a(BIZ)V
    let c = call_hook(&c, "G", "()V", 171, &[0xb7, 0x01, 0x1d], hooks::CRIT, invokestatic)?;
    let c = call_hook(&c, "G", "()V", 178, &[0xb7, 0x01, 0x1d], hooks::CRIT, invokestatic)?;
    let c = call_hook(&c, "H", "()V", 114, &[0xb7, 0x01, 0x1d], hooks::EVASION, invokestatic)?;
    let c = call_hook(&c, "H", "()V", 121, &[0xb7, 0x01, 0x1d], hooks::EVASION, invokestatic)?;
    // bipush 100 -> bipush 60: the refill counters p:B / v:B
    let c = classfile::rewrite_code(&c, "c", "()V", 163, &[0x10, 100], &[0x10, REFILL_FRAMES])?;
    classfile::rewrite_code(&c, "c", "()V", 189, &[0x10, 100], &[0x10, REFILL_FRAMES])
}

/// HZ-040: HP and SP refill by (의지 + 재생) every so many field ticks (the game's 100).
pub const REFILL_FRAMES: u8 = 60;

/// HZ-041 `cq`: the monster's hit floor.
fn hit_floor(class: &[u8]) -> Option<Vec<u8>> {
    // bipush 15 twice: the compare @28 and the value stored @33
    let c = classfile::rewrite_code(class, "a", "(Lar;)Z", 28, &[0x10, 15], &[0x10, hooks::HIT_FLOOR])?;
    classfile::rewrite_code(&c, "a", "(Lar;)Z", 33, &[0x10, 15], &[0x10, hooks::HIT_FLOOR])
}

/// HZ-042 `cb`: the seven reads of the level cap.
fn level_cap(class: &[u8]) -> Option<Vec<u8>> {
    let mut c = class.to_vec();
    for (method, descriptor, at) in [
        ("a", "(Ldd;IIBZ)V", 90),
        ("a", "(ZB)Z", 90),
        ("a", "(ZB)Z", 142),
        ("a", "(BBBBZ)V", 18),
        ("a", "(BBBBZ)V", 98),
        ("b", "(BBBBZ)V", 18),
        ("b", "(BBBBZ)V", 82),
    ] {
        // getfield #35 cb.j:B
        c = call_hook(&c, method, descriptor, at, &[0xb4, 0x00, 0x23], hooks::LEVEL_CAP, invokestatic)?;
    }
    Some(c)
}

/// HZ-043 `cq`: the hit's damage — HZ-032's hook, or put here.
fn combo_sp(class: &[u8]) -> Option<Vec<u8>> {
    hook_or_keep(class, "a", "(Lek;BIZBBZZZ)Z", 103, &[0xb8, 0x00, 0xc3], hooks::HIT_DAMAGE, hooks::HIT_DAMAGE)
}

/// HZ-044 `ek`: the level-up's point.
fn skill_point_curve(class: &[u8]) -> Option<Vec<u8>> {
    const EVEN_LEVEL: [u8; 20] = [0x2a, 0xb4, 0x00, 0x8d, 0x05, 0x70, 0x9a, 0x00, 0x0e, 0x2a, 0x59, 0xb4, 0x00, 0x6b, 0x04, 0x60, 0x93, 0xb5, 0x00, 0x6b];
    call_hook(class, "j", "(I)V", 109, &EVEN_LEVEL, hooks::SKILL_POINTS, |[hi, lo]| {
        let mut with = vec![0x2a, 0xb8, hi, lo];
        with.resize(EVEN_LEVEL.len(), 0);
        with
    })
}

/// HZ-045 `ak`: a part's reflection.
fn boss_reflection(class: &[u8]) -> Option<Vec<u8>> {
    // invokevirtual #52 ar.a(Lar;IBB)V
    call_hook(class, "a", "(Lar;I)V", 20, &[0xb6, 0x00, 0x34], hooks::BOSS_REFLECT, invokestatic)
}

/// HZ-057 `cq`: a monster's own picture and its next move call the hooks.
fn stance_monster(class: &[u8]) -> Option<Vec<u8>> {
    // invokevirtual #196 a(II)V
    let c = call_hook(class, "b", "(II)V", 207, &[0xb6, 0x00, 0xc4], hooks::MONSTER_DRAW, invokestatic)?;
    // invokevirtual #217 k()V
    call_hook(&c, "l", "()V", 128, &[0xb6, 0x00, 0xd9], hooks::MONSTER_THINK, invokestatic)
}

/// HZ-057 `ak`: the player's blow on a boss calls the hook, both ways in.
fn stance_boss_hit(class: &[u8]) -> Option<Vec<u8>> {
    // invokespecial #57 cq.a(Lek;BIZBBZZZ)Z
    let c = call_hook(class, "a", "(Lek;BIZBBZZZ)Z", 23, &[0xb7, 0x00, 0x39], hooks::BOSS_HIT, invokestatic)?;
    call_hook(&c, "a", "(Lek;BIZBBZZZ)Z", 53, &[0xb7, 0x00, 0x39], hooks::BOSS_HIT, invokestatic)
}

/// HZ-046 `cq`: the death's drop — the hook already there, or put here.
fn boss_drops(class: &[u8]) -> Option<Vec<u8>> {
    hook_or_keep(class, "g", "()V", 22, &[0xb6, 0x00, 0xdd], hooks::MONSTER_DROP, hooks::MONSTER_DROP)
}

/// HZ-047 `p`: 죽음의 검 kills when the roll is under the chance.
fn death_sword(class: &[u8]) -> Option<Vec<u8>> {
    // if_icmple 60 -> if_icmpgt 60
    classfile::rewrite_code(class, "a", "(Lcq;)Z", 18, &[0xa4, 0x00, 0x2a], &[0xa3, 0x00, 0x2a])
}

/// HZ-047 `ek`: 전력방어's cut and its length.
fn guard_percent(class: &[u8]) -> Option<Vec<u8>> {
    // iload_2; aload_0; getfield #148 y:B; bipush 10; idiv; imul -> ... invokestatic; nop
    const DIVIDE: [u8; 9] = [0x1c, 0x2a, 0xb4, 0x00, 0x94, 0x10, 0x0a, 0x6c, 0x68];
    let c = call_hook(class, "a", "(Lar;IB)Z", 63, &DIVIDE, hooks::GUARD, |[hi, lo]| vec![0x1c, 0x2a, 0xb4, 0x00, 0x94, 0xb8, hi, lo, 0x00])?;
    // invokevirtual #193 ar.b(BI)V
    call_hook(&c, "b", "(BB)V", 5, &[0xb6, 0x00, 0xc1], hooks::GUARD_STATUS, invokestatic)
}

/// HZ-036 `ed`: the player's three bags are made 112.
fn bag_size(class: &[u8]) -> Option<Vec<u8>> {
    let mut c = class.to_vec();
    for at in [4, 16, 28] {
        c = classfile::rewrite_code(&c, "f", "()V", at, &[0x10, hooks::BAG_PAGE as u8], &[0x10, hooks::BAG_SIZE as u8])?;
    }
    Some(c)
}

/// HZ-036 `q`: its grid's cells and the cursor's item, on the page.
fn bag_pages_q(class: &[u8]) -> Option<Vec<u8>> {
    // invokevirtual #70 bt.a(I)Lba;
    let c = call_hook(class, "a", "(Ldd;Lbt;BZBBI)V", 134, &[0xb6, 0x00, 0x46], hooks::BAG_AT, invokestatic)?;
    call_hook(&c, "a", "(Ldd;Lbt;BZBBI)V", 309, &[0xb6, 0x00, 0x46], hooks::BAG_AT, invokestatic)
}

/// HZ-036 `ds` (인벤토리): the item chosen, the cursor's moves, the grid's draw — and the answers that
/// take the chosen item away (「버리기」 @316, a 귀환서 used @342).
fn bag_pages_ds(class: &[u8]) -> Option<Vec<u8>> {
    // #35 bt.a(I)Lba; · #59 o.d(II)Z · #60 o.e(II)Z · #65 q.a(Ldd;Lbt;BZBBI)V · #36 bt.a(I)V · #37 bt.a(II)V
    let c = call_hook(class, "a", "(II)Z", 164, &[0xb6, 0x00, 0x23], hooks::BAG_AT, invokestatic)?;
    let c = call_hook(&c, "a", "(II)Z", 388, &[0xb6, 0x00, 0x3b], hooks::BAG_SIDEWAYS, invokestatic)?;
    let c = call_hook(&c, "a", "(II)Z", 399, &[0xb6, 0x00, 0x3c], hooks::BAG_UPDOWN, invokestatic)?;
    let c = call_hook(&c, "a", "(Ldd;)V", 44, &[0xb6, 0x00, 0x41], hooks::BAG_GRID, invokestatic)?;
    let c = call_hook(&c, "a", "(BB)V", 316, &[0xb6, 0x00, 0x24], hooks::BAG_TAKE, invokestatic)?;
    call_hook(&c, "a", "(BB)V", 342, &[0xb6, 0x00, 0x25], hooks::BAG_TAKE_SOME, invokestatic)
}

/// HZ-036 `eg` (계정창고's window): the same, its grid a page at most — and the answers that take the
/// chosen item out of the player's bag (「넣기」 @162, 「버리기」 @473; the 계정창고 side's @396 / @493
/// take from a 계정창고 page, which has no pages of its own).
fn bag_pages_eg(class: &[u8]) -> Option<Vec<u8>> {
    // #54 bt.a(I)Lba; · #81 o.d(II)Z · #82 o.e(II)Z · #27 bt.a:B · #87 q.a(Ldd;Lbt;BZBBI)V · #55 bt.a(I)V
    let c = call_hook(class, "a", "(II)Z", 197, &[0xb6, 0x00, 0x36], hooks::BAG_AT, invokestatic)?;
    let c = call_hook(&c, "a", "(II)Z", 335, &[0xb6, 0x00, 0x51], hooks::BAG_SIDEWAYS, invokestatic)?;
    let c = call_hook(&c, "a", "(II)Z", 346, &[0xb6, 0x00, 0x52], hooks::BAG_UPDOWN, invokestatic)?;
    let c = call_hook(&c, "c", "(B)V", 89, &[0xb4, 0x00, 0x1b], hooks::BAG_CELLS, invokestatic)?;
    let c = call_hook(&c, "a", "(Ldd;)V", 118, &[0xb6, 0x00, 0x57], hooks::BAG_GRID, invokestatic)?;
    let c = call_hook(&c, "a", "(BB)V", 162, &[0xb6, 0x00, 0x37], hooks::BAG_TAKE, invokestatic)?;
    call_hook(&c, "a", "(BB)V", 473, &[0xb6, 0x00, 0x37], hooks::BAG_TAKE, invokestatic)
}

/// HZ-036 `bz` (the shop's, alchemist's and smith's grid): its cells on the page.
fn bag_pages_bz(class: &[u8]) -> Option<Vec<u8>> {
    // #28 bt.a(I)Lba;
    call_hook(class, "a", "(Ldd;Lbt;)V", 53, &[0xb6, 0x00, 0x1c], hooks::BAG_AT, invokestatic)
}

/// HZ-036 `co` (the shop, selling from a bag).
fn bag_pages_co(class: &[u8]) -> Option<Vec<u8>> {
    // #62 bt.a(I)Lba; · #101 o.e(II)Z · #100 o.d(II)Z · #68 bz.a(Ldd;Lbt;)V
    let c = call_hook(class, "a", "(II)Z", 354, &[0xb6, 0x00, 0x3e], hooks::BAG_AT, invokestatic)?;
    let c = call_hook(&c, "a", "(II)Z", 513, &[0xb6, 0x00, 0x65], hooks::BAG_UPDOWN, invokestatic)?;
    let c = call_hook(&c, "a", "(II)Z", 555, &[0xb6, 0x00, 0x64], hooks::BAG_SIDEWAYS, invokestatic)?;
    let c = call_hook(&c, "a", "(Ldd;)V", 135, &[0xb6, 0x00, 0x44], hooks::BAG_GRID_BZ, invokestatic)?;
    call_hook(&c, "a", "(Ldd;)V", 172, &[0xb6, 0x00, 0x3e], hooks::BAG_AT, invokestatic)
}

/// HZ-036 `dv` (연금술사, picking from the 조합 bag).
fn bag_pages_dv(class: &[u8]) -> Option<Vec<u8>> {
    // #76 bt.a(I)Lba; · #115 o.d(II)Z · #116 o.e(II)Z · #81 bz.a(Ldd;Lbt;)V
    let c = call_hook(class, "a", "(II)Z", 210, &[0xb6, 0x00, 0x4c], hooks::BAG_AT, invokestatic)?;
    let c = call_hook(&c, "a", "(II)Z", 773, &[0xb6, 0x00, 0x73], hooks::BAG_SIDEWAYS, invokestatic)?;
    let c = call_hook(&c, "a", "(II)Z", 780, &[0xb6, 0x00, 0x74], hooks::BAG_UPDOWN, invokestatic)?;
    let c = call_hook(&c, "a", "(Ldd;)V", 79, &[0xb6, 0x00, 0x51], hooks::BAG_GRID_BZ, invokestatic)?;
    call_hook(&c, "a", "(Ldd;)V", 107, &[0xb6, 0x00, 0x4c], hooks::BAG_AT, invokestatic)
}

/// HZ-036 `et` (대장간, picking from a bag).
fn bag_pages_et(class: &[u8]) -> Option<Vec<u8>> {
    // #56 bt.a(I)Lba; · #90 o.d(II)Z · #91 o.e(II)Z · #60 bz.a(Ldd;Lbt;)V
    let c = call_hook(class, "a", "(II)Z", 110, &[0xb6, 0x00, 0x38], hooks::BAG_AT, invokestatic)?;
    let c = call_hook(&c, "a", "(II)Z", 173, &[0xb6, 0x00, 0x5a], hooks::BAG_SIDEWAYS, invokestatic)?;
    let c = call_hook(&c, "a", "(II)Z", 180, &[0xb6, 0x00, 0x5b], hooks::BAG_UPDOWN, invokestatic)?;
    let c = call_hook(&c, "a", "(Ldd;)V", 64, &[0xb6, 0x00, 0x3c], hooks::BAG_GRID_BZ, invokestatic)?;
    call_hook(&c, "a", "(Ldd;)V", 92, &[0xb6, 0x00, 0x38], hooks::BAG_AT, invokestatic)
}

/// HZ-037: `bipush 84` (계정창고's size as written) -> 112.
fn storage_written(class: &[u8], method: &str, descriptor: &str, at: usize) -> Option<Vec<u8>> {
    classfile::rewrite_code(class, method, descriptor, at, &[0x10, 84], &[0x10, hooks::STORAGE_SIZE as u8])
}

/// HZ-037: `aload; iconst_4; iinc local, 1; baload` (the size as read) -> `bipush 112; iinc local, 1; nop`.
fn storage_read(class: &[u8], method: &str, descriptor: &str, at: usize, aload: u8, local: u8) -> Option<Vec<u8>> {
    let was = [aload, 0x07, 0x84, local, 0x01, 0x33];
    classfile::rewrite_code(class, method, descriptor, at, &was, &[0x10, hooks::STORAGE_SIZE as u8, 0x84, local, 0x01, 0x00])
}

/// HZ-037: a page loop's bound `iconst_3` (the game's three pages) -> the pages there are now.
fn storage_pages_walked(class: &[u8], method: &str, descriptor: &str, at: usize) -> Option<Vec<u8>> {
    classfile::rewrite_code(class, method, descriptor, at, &[0x06], &[0x03 + hooks::STORAGE_PAGES])
}

/// HZ-037 `dz` (계정창고): its load and its save — the size, and the save's two walks over the pages
/// (`dz.i()V` @35 the count, @64 the items: `iconst_3` left the fourth page out of the record).
fn storage_size_dz(class: &[u8]) -> Option<Vec<u8>> {
    let c = storage_written(class, "h", "()V", 25)?;
    let c = storage_read(&c, "h", "()V", 50, 0x2a, 1)?;
    let c = storage_written(&c, "i", "()V", 25)?;
    let c = storage_pages_walked(&c, "i", "()V", 35)?;
    storage_pages_walked(&c, "i", "()V", 64)
}

/// HZ-037 `eb`: the dated gift into the storage reads and writes it the same way — the room it looks
/// for (@292) and the two walks of the write (@392, @424) over every page.
fn storage_size_eb(class: &[u8]) -> Option<Vec<u8>> {
    let c = storage_written(class, "a", "([[I)V", 31)?;
    let c = storage_read(&c, "a", "([[I)V", 54, 0x2b, 2)?;
    let c = storage_written(&c, "a", "([[I)V", 381)?;
    let c = storage_pages_walked(&c, "a", "([[I)V", 292)?;
    let c = storage_pages_walked(&c, "a", "([[I)V", 392)?;
    storage_pages_walked(&c, "a", "([[I)V", 424)
}

/// HZ-038 `bt`: a stack's most, 99 -> 127, in the room test, the add and the merge.
fn stack_max(class: &[u8]) -> Option<Vec<u8>> {
    let mut c = class.to_vec();
    for (method, descriptor, at) in [("a", "(BBI)Z", 152), ("a", "(BBI)Z", 160), ("a", "(Lba;I)I", 177), ("a", "(Lba;I)I", 193), ("a", "(Lba;I)I", 198), ("b", "()V", 45), ("b", "()V", 124), ("b", "()V", 144)] {
        c = classfile::rewrite_code(&c, method, descriptor, at, &[0x10, 99], &[0x10, hooks::STACK_MAX as u8])?;
    }
    Some(c)
}

/// HZ-055 `ek`: max SP's level read becomes the hook (level × 3).
fn sp_per_level(class: &[u8]) -> Option<Vec<u8>> {
    call_hook(class, "i", "()V", 1017, &[0xb4, 0x00, 0x8d], hooks::SP_LEVEL, invokestatic)
}

/// HZ-056 `ek`: the spirit cast's end empties a stale action queue.
fn spirit_cast_end(class: &[u8]) -> Option<Vec<u8>> {
    // invokespecial #265 D()V
    call_hook(class, "C", "()V", 717, &[0xb7, 0x01, 0x09], hooks::SPIRIT_CAST_END, invokestatic)
}

/// HZ-058 `cb`: the skill popup's two registration rows call the plates' hook.
fn quick_register(class: &[u8]) -> Option<Vec<u8>> {
    // invokestatic #77 ed.d(IB)V
    let c = call_hook(class, "a", "(BBBBZ)V", 143, &[0xb8, 0x00, 0x4d], hooks::QUICK_REGISTER, invokestatic)?;
    call_hook(&c, "a", "(BBBBZ)V", 180, &[0xb8, 0x00, 0x4d], hooks::QUICK_REGISTER, invokestatic)
}

/// HZ-058 `ds`: the potion popup's two registration rows call the plates' hook.
fn quick_potion(class: &[u8]) -> Option<Vec<u8>> {
    // invokestatic #49 ed.a(ZB)V
    call_hook(class, "a", "(BB)V", 72, &[0xb8, 0x00, 0x31], hooks::QUICK_POTION, invokestatic)
}

/// HZ-048 `d`: 방패치기 rolls the weapon (slot 3), not the shield (slot 5).
fn bash_weapon_roll(class: &[u8]) -> Option<Vec<u8>> {
    classfile::rewrite_code(class, "a", "(Lcq;BB)Z", 14, &[0x08], &[0x06])
}

/// HZ-035 `g`: the ground list takes the recipes' numbers (after whatever limit is there).
fn recipe_ground(class: &[u8]) -> Option<Vec<u8>> {
    let top = [0x15, 0x04, 0x10, hooks::RECIPE_GROUND_TOP as u8, 0xa4];
    for was in [0x1c, 31, hooks::GEAR_ID as u8, hooks::GROUND_TOP_ID as u8] {
        if let Some(c) = classfile::rewrite_code(class, "a", "(BBBB)V", 15, &[0x15, 0x04, 0x10, was, 0xa4], &top) {
            return Some(c);
        }
    }
    None
}

/// HZ-035 `cq`: the kill's recipe roll (HZ-031's / HZ-033's hook, or put here).
fn recipe_drop(class: &[u8]) -> Option<Vec<u8>> {
    hook_or_keep(class, "g", "()V", 22, &[0xb6, 0x00, 0xdd], hooks::MONSTER_DROP, hooks::MONSTER_DROP)
}

/// HZ-035 `eb`: the random box keeps to the game's recipes.
fn recipe_box(class: &[u8]) -> Option<Vec<u8>> {
    // iload_1; iconst_4; idiv; i2b; istore_1 -> iload_1; invokestatic; istore_1
    call_hook(class, "b", "(BLn;)Z", 1346, &[0x1b, 0x07, 0x6c, 0x91, 0x3c], hooks::BOX_RECIPE, |[hi, lo]| vec![0x1b, 0xb8, hi, lo, 0x3c])
}

/// HZ-034 `eb`: the item a quest reward makes, and how many go into the bag.
fn guardian_quest_items(class: &[u8]) -> Option<Vec<u8>> {
    // invokestatic #180 ba.a(BB)Lba;
    let c = call_hook(class, "b", "(BLn;)Z", 1021, &[0xb8, 0x00, 0xb4], hooks::QUEST_ITEM, invokestatic)?;
    // invokevirtual #190 bt.a(Lba;I)I
    call_hook(&c, "b", "(BLn;)Z", 1045, &[0xb6, 0x00, 0xbe], hooks::QUEST_GIVE, invokestatic)
}

/// `hook` at `at` in place of the `original` instruction, or — when another patch's hook (`theirs`) is
/// already there — that one kept (it answers for this patch too, by `GuestState::patch_on`).
fn hook_or_keep(class: &[u8], method: &str, descriptor: &str, at: usize, original: &[u8], hook: hooks::Hook, theirs: hooks::Hook) -> Option<Vec<u8>> {
    if let Some(c) = call_hook(class, method, descriptor, at, original, hook, invokestatic) {
        return Some(c);
    }
    let code = &class[classfile::method_code(class, method, descriptor)?];
    let (pool, index) = classfile::methodref(class, hooks::CLASS, theirs.name, theirs.descriptor)?;
    let [hi, lo] = index.to_be_bytes();
    (pool.len() == class.len() && code.get(at..at + 3) == Some(&[0xb8, hi, lo][..])).then(|| class.to_vec())
}

/// HZ-033 `cq`: the drop's slot widths, numbers, ground drop and the kill's extra.
fn drop_boost(class: &[u8]) -> Option<Vec<u8>> {
    // invokestatic #282 rpg/g.a([BI)I — or HZ-005's hook
    let c = hook_or_keep(class, "o", "()V", 87, &[0xb8, 0x01, 0x1a], hooks::DROP_THRESHOLD, hooks::DROP_THRESHOLD)?;
    // invokestatic #259 java/lang/Math.min(II)I — the common band's
    let c = call_hook(&c, "o", "()V", 158, &[0xb8, 0x01, 0x03], hooks::COMMON_DROP, invokestatic)?;
    // the rare band's min — or HZ-023's hook
    let c = hook_or_keep(&c, "o", "()V", 215, &[0xb8, 0x01, 0x03], hooks::RARE_DROP, hooks::RARE_DROP)?;
    // invokevirtual #250 g.a(BBBB)V
    let c = call_hook(&c, "o", "()V", 271, &[0xb6, 0x00, 0xfa], hooks::DROP_ITEM, invokestatic)?;
    // return; iinc 7, 1 -> nop; iinc 7, 1: after a slot's drop the loop goes on (Hooks.dropThreshold
    // answers 0 for the slots left) to the game's own bands for the same R (2026-11-06)
    let c = classfile::rewrite_code(&c, "o", "()V", 274, &[0xb1, 0x84, 0x07, 0x01], &[0x00, 0x84, 0x07, 0x01])?;
    // invokevirtual #221 o()V in the death — or HZ-031's hook
    hook_or_keep(&c, "g", "()V", 22, &[0xb6, 0x00, 0xdd], hooks::MONSTER_DROP, hooks::MONSTER_DROP)
}

/// HZ-033 `g`: the ground list takes the top pieces (numbers up to 31) when nothing raised its limit.
fn drop_boost_ground(class: &[u8]) -> Option<Vec<u8>> {
    // iload 4; bipush 28; if_icmple — HZ-023 (40) / HZ-031 (50) already take 31
    classfile::rewrite_code(class, "a", "(BBBB)V", 15, &[0x15, 0x04, 0x10, 0x1c, 0xa4], &[0x15, 0x04, 0x10, 31, 0xa4]).or_else(|| {
        let code = &class[classfile::method_code(class, "a", "(BBBB)V")?];
        (code.get(15..17) == Some(&[0x15, 0x04][..]) && code.get(17) == Some(&0x10) && code.get(18).is_some_and(|&n| n >= 31)).then(|| class.to_vec())
    })
}

/// HZ-032 `cq`: the hit's damage and critical, the kill's EXP, 트릭샷's mark.
fn effects_cq(class: &[u8]) -> Option<Vec<u8>> {
    const HIT: &str = "(Lek;BIZBBZZZ)Z";
    // invokestatic #195 cq.a(II)I
    let c = call_hook(class, "a", HIT, 103, &[0xb8, 0x00, 0xc3], hooks::HIT_DAMAGE, invokestatic)?;
    // invokevirtual #233 ek.a(BZ)I
    let c = call_hook(&c, "a", HIT, 172, &[0xb6, 0x00, 0xe9], hooks::CRIT_CHANCE, invokestatic)?;
    // invokevirtual #220 n()V
    let c = call_hook(&c, "g", "()V", 26, &[0xb6, 0x00, 0xdc], hooks::MONSTER_EXP, invokestatic)?;
    // invokevirtual #158 ar.b(BI)V
    let c = call_hook(&c, "c", "(Lbc;)V", 90, &[0xb6, 0x00, 0x9e], hooks::MARK_STATUS, invokestatic)?;
    // iload 4; ifeq +9; aload_0; iload 5; invokevirtual #157 ar.b(B)V -> aload_0; iload_3; iload 4; iload 5;
    // iload 6; invokestatic (11 bytes; the old branch target 332 follows with the stack empty)
    let push = [0x15, 0x04, 0x99, 0x00, 0x09, 0x2a, 0x15, 0x05, 0xb6, 0x00, 0x9d];
    let c = call_hook(&c, "a", HIT, 321, &push, hooks::KNOCK, |[hi, lo]| vec![0x2a, 0x1d, 0x15, 0x04, 0x15, 0x05, 0x15, 0x06, 0xb8, hi, lo])?;
    // invokevirtual #240 ek.g()Z — a monster's push on the player
    call_hook(&c, "a", "([BSZ)Z", 112, &[0xb6, 0x00, 0xf0], hooks::KNOCKABLE, invokestatic)
}

/// HZ-032 `ek`: damage taken, a skill's SP and start, 반격's status.
fn effects_ek(class: &[u8]) -> Option<Vec<u8>> {
    // invokevirtual #307 ek.c(I)V
    let c = call_hook(class, "a", "(Lar;IB)Z", 338, &[0xb6, 0x01, 0x33], hooks::TAKE_DAMAGE, invokestatic)?;
    // invokevirtual #236 db.b(B)I; #238 db.b(B)Z
    let c = call_hook(&c, "C", "()V", 354, &[0xb6, 0x00, 0xec], hooks::SKILL_SP, invokestatic)?;
    let c = call_hook(&c, "C", "()V", 460, &[0xb6, 0x00, 0xee], hooks::CAST_START, invokestatic)?;
    // invokevirtual #193 ar.b(BI)V
    let c = call_hook(&c, "k", "()V", 5, &[0xb6, 0x00, 0xc1], hooks::COUNTER_STATUS, invokestatic)?;
    // invokevirtual #189 ar.a(Lar;IBB)V — 반격's reflection
    let c = call_hook(&c, "a", "(Lar;IB)Z", 119, &[0xb6, 0x00, 0xbd], hooks::COUNTER_REFLECT, invokestatic)?;
    // invokevirtual #217 bt.a(Lba;)I — the potion drunk
    call_hook(&c, "a", "(Z)Z", 196, &[0xb6, 0x00, 0xd9], hooks::DRANK, invokestatic)
}

/// HZ-032 `ed`: the cooldown a skill starts with.
fn effects_ed(class: &[u8]) -> Option<Vec<u8>> {
    // invokevirtual #208 ek.d(B)I; #257 p.d(B)I
    let c = call_hook(class, "g", "(I)V", 26, &[0xb6, 0x00, 0xd0], hooks::JOB_COOLDOWN, invokestatic)?;
    call_hook(&c, "g", "(I)V", 39, &[0xb6, 0x01, 0x01], hooks::SPIRIT_COOLDOWN, invokestatic)
}

/// HZ-032 `eo` (정령사): 동화's frames.
fn effects_eo(class: &[u8]) -> Option<Vec<u8>> {
    // invokevirtual #23 ek.n(I)V
    call_hook(class, "a", "(BI)Z", 191, &[0xb6, 0x00, 0x17], hooks::FUSION_START, invokestatic)
}

/// HZ-032 `dn` (로그): 분신's life.
fn effects_dn(class: &[u8]) -> Option<Vec<u8>> {
    // invokestatic #52 h.a(Lek;S)V
    let c = call_hook(class, "a", "(BI)Z", 918, &[0xb8, 0x00, 0x34], hooks::CLONE_LIFE, invokestatic)?;
    // invokevirtual #39 db.c(B)I (독수's %); #45 ek.c(Z)I (its poison)
    let c = call_hook(&c, "a", "(BI)Z", 363, &[0xb6, 0x00, 0x27], hooks::POISON_CHANCE, invokestatic)?;
    call_hook(&c, "a", "(BI)Z", 374, &[0xb6, 0x00, 0x2d], hooks::POISON_DAMAGE, invokestatic)
}

/// HZ-032 `d` (나이트): 방패치기's hit.
fn effects_d(class: &[u8]) -> Option<Vec<u8>> {
    // invokespecial #28 d.a(Lcq;BB)Z
    call_hook(class, "a", "(BI)Z", 139, &[0xb7, 0x00, 0x1c], hooks::SHIELD_BASH, invokestatic)
}

/// HZ-032 `ag` (워리어): 강타's front hit, 광기's HP.
fn effects_ag(class: &[u8]) -> Option<Vec<u8>> {
    // invokevirtual #37 ek.a(BIBZZZ)Z; #39 ek.c(I)V
    let c = call_hook(class, "a", "(BI)Z", 131, &[0xb6, 0x00, 0x25], hooks::SMASH, invokestatic)?;
    call_hook(&c, "a", "(BI)Z", 394, &[0xb6, 0x00, 0x27], hooks::FRENZY_COST, invokestatic)
}

/// HZ-032 `ai` (건너): 저격's kill chance.
fn effects_ai(class: &[u8]) -> Option<Vec<u8>> {
    // invokevirtual #39 db.c(B)I
    call_hook(class, "a", "(BI)Z", 953, &[0xb6, 0x00, 0x27], hooks::SNIPE_CHANCE, invokestatic)
}

/// HZ-032 `av` (총검사): 폭발탄's chance.
fn effects_av(class: &[u8]) -> Option<Vec<u8>> {
    // invokevirtual #21 db.c(B)I
    call_hook(class, "c", "()Z", 9, &[0xb6, 0x00, 0x15], hooks::BLAST_CHANCE, invokestatic)
}

/// HZ-032 `at` / `di` / `dk` (the big bosses' pushes): the player's knockback test.
fn effects_at(class: &[u8]) -> Option<Vec<u8>> {
    call_hook(class, "b", "()V", 49, &[0xb6, 0x00, 0x13], hooks::KNOCKABLE, invokestatic)
}

fn effects_di(class: &[u8]) -> Option<Vec<u8>> {
    call_hook(class, "b", "()V", 96, &[0xb6, 0x00, 0x28], hooks::KNOCKABLE, invokestatic)
}

fn effects_dk(class: &[u8]) -> Option<Vec<u8>> {
    call_hook(class, "b", "()V", 49, &[0xb6, 0x00, 0x26], hooks::KNOCKABLE, invokestatic)
}

/// HZ-032 `ar` (anything that walks): a walking frame's step.
fn effects_ar(class: &[u8]) -> Option<Vec<u8>> {
    // getfield #23 f:B
    call_hook(class, "d", "()Z", 25, &[0xb4, 0x00, 0x17], hooks::MOVE_STEP, invokestatic)
}

/// HZ-032 `p` (the spirit): a spirit skill's MP cut and power.
fn effects_p(class: &[u8]) -> Option<Vec<u8>> {
    // invokevirtual #114 p.a(BI)I; #78 ek.c(I)I
    let c = call_hook(class, "d", "(B)Z", 45, &[0xb6, 0x00, 0x72], hooks::SPIRIT_COST, invokestatic)?;
    call_hook(&c, "e", "(B)I", 22, &[0xb6, 0x00, 0x4e], hooks::SPIRIT_POWER, invokestatic)
}

/// HZ-031 `ba`: the factory's record read serves the 85제 pieces — through the hook already there
/// (HZ-016 / HZ-023 serve them too while HZ-031 is chosen), or a new one.
fn legend_records(class: &[u8]) -> Option<Vec<u8>> {
    // invokestatic #68 rpg/e.a(Ljava/lang/String;I)[B
    if let Some(c) = call_hook(class, "a", "(BB)Lba;", 107, &[0xb8, 0x00, 0x44], hooks::LEGEND_RECORD, invokestatic) {
        return Some(c);
    }
    let code = &class[classfile::method_code(class, "a", "(BB)Lba;")?];
    for h in [hooks::ITEM_RECORD, hooks::GEAR_RECORD, hooks::ORB_GEAR_RECORD] {
        if let Some((pool, index)) = classfile::methodref(class, hooks::CLASS, h.name, h.descriptor)
            && pool.len() == class.len()
            && code.get(107..110) == Some(&[0xb8, index.to_be_bytes()[0], index.to_be_bytes()[1]][..])
        {
            return Some(class.to_vec());
        }
    }
    None
}

/// HZ-031 `g`: the ground list takes the 85제 numbers; the pickup notice's grade colour.
fn legend_ground(class: &[u8]) -> Option<Vec<u8>> {
    // iload 4; bipush 28 (or HZ-023's 40); if_icmple
    let top = [0x15, 0x04, 0x10, hooks::GROUND_TOP_ID as u8, 0xa4];
    let c = classfile::rewrite_code(class, "a", "(BBBB)V", 15, &[0x15, 0x04, 0x10, 0x1c, 0xa4], &top)
        .or_else(|| classfile::rewrite_code(class, "a", "(BBBB)V", 15, &[0x15, 0x04, 0x10, hooks::GEAR_ID as u8, 0xa4], &top))?;
    // getfield #84 bx.e:B; iaload
    call_hook(&c, "a", "(BBBZ)V", 221, &[0xb4, 0x00, 0x54, 0x2e], hooks::GRADE_COLOR, |[hi, lo]| vec![0xb8, hi, lo, 0x00])
}

/// HZ-031 `q`: the bag panel's name colour and the item window's grade name.
fn legend_grade(class: &[u8]) -> Option<Vec<u8>> {
    // getfield #48 bx.e:B; iaload
    let c = call_hook(class, "b", "(Ldd;IILba;Z)V", 13, &[0xb4, 0x00, 0x30, 0x2e], hooks::GRADE_COLOR, |[hi, lo]| vec![0xb8, hi, lo, 0x00])?;
    // invokevirtual #72 cx.a(I)[B
    call_hook(&c, "b", "(Ldd;IILba;)V", 104, &[0xb6, 0x00, 0x48], hooks::GRADE_NAME, invokestatic)
}

/// HZ-031 `cz` (the item window): its contents, then the piece's effect lines.
fn legend_info(class: &[u8]) -> Option<Vec<u8>> {
    // invokevirtual #18 q.b(Ldd;IILba;)V
    call_hook(class, "a", "(Ldd;)V", 35, &[0xb6, 0x00, 0x12], hooks::ITEM_INFO, invokestatic)
}

/// HZ-031 `cq`: the drop, then the 85제 roll.
fn legend_drop(class: &[u8]) -> Option<Vec<u8>> {
    // invokevirtual #221 o()V
    call_hook(class, "g", "()V", 22, &[0xb6, 0x00, 0xdd], hooks::MONSTER_DROP, invokestatic)
}

/// HZ-030 `ed`: the ending's round bump goes one further — 헬 to 가디언 모드, which stays.
fn guardian_round_cap(class: &[u8]) -> Option<Vec<u8>> {
    // getstatic #109 h:B; iconst_2; if_icmpge -> iconst_3
    let top = 0x03 + hooks::GUARDIAN_ROUND as u8;
    classfile::rewrite_code(class, "a", "()V", 430, &[0xb2, 0x00, 0x6d, 0x05, 0xa2], &[0xb2, 0x00, 0x6d, top, 0xa2])
}

/// HZ-030 `cq`: the kind's numbers, then 가디언 모드's in place of the game's round block.
fn guardian_monsters(class: &[u8]) -> Option<Vec<u8>> {
    // invokevirtual #222 p()V
    let c = call_hook(class, "i", "()V", 31, &[0xb6, 0x00, 0xde], hooks::GUARDIAN_MONSTER, invokestatic)?;
    // getstatic #100 ed.h:B — the round's two reads (`min(h, 2)`)
    let c = call_hook(&c, "i", "()V", 36, &[0xb2, 0x00, 0x64], hooks::MONSTER_ROUND, invokestatic)?;
    call_hook(&c, "i", "()V", 47, &[0xb2, 0x00, 0x64], hooks::MONSTER_ROUND, invokestatic)
}

/// HZ-030 `eb`: the quest money and EXP the script picks by round.
fn guardian_rewards(class: &[u8]) -> Option<Vec<u8>> {
    // invokestatic #270 ed.e(I)V (action 17)
    let c = call_hook(class, "b", "(BLn;)Z", 1208, &[0xb8, 0x01, 0x0e], hooks::QUEST_MONEY, invokestatic)?;
    // invokevirtual #279 ek.j(I)V (action 20)
    call_hook(&c, "b", "(BLn;)Z", 1458, &[0xb6, 0x01, 0x17], hooks::QUEST_EXP, invokestatic)
}

/// HZ-030 `ce`: the ending's 「… 모드로」.
fn guardian_ending_text(class: &[u8]) -> Option<Vec<u8>> {
    // invokevirtual #181 cx.a(I)[B
    call_hook(class, "d", "(Ldd;)V", 897, &[0xb6, 0x00, 0xb5], hooks::ROUND_TEXT, invokestatic)
}

/// HZ-028. See its `evidence`.
fn translucent_fill(class: &[u8]) -> Option<Vec<u8>> {
    // aload 5; iload 7; iload 6; iastore; iinc 7, -1 -> aload 5; iload 6; invokestatic; istore 7; nop
    let c = call_hook(class, "a", "(IIII)V", 55, &[0x19, 0x05, 0x15, 0x07, 0x15, 0x06, 0x4f, 0x84, 0x07, 0xff], hooks::FILL_INTS, |[hi, lo]| {
        vec![0x19, 0x05, 0x15, 0x06, 0xb8, hi, lo, 0x36, 0x07, 0x00]
    })?;
    // aload 7; iload_1; iaload; bipush 24; iushr; ifeq 102; aload 7; iload_1; dup2; iaload; aload_0;
    // getfield #12 a:I; iand; iastore -> aload 7; aload_0; getfield #12; invokestatic; istore_1; nop x11
    let mask = [0x19, 0x07, 0x1b, 0x2e, 0x10, 0x18, 0x7c, 0x99, 0x00, 0x0e, 0x19, 0x07, 0x1b, 0x5c, 0x2e, 0x2a, 0xb4, 0x00, 0x0c, 0x7e, 0x4f];
    call_hook(&c, "a", "(Lcw;III)V", 81, &mask, hooks::MASK_ALPHA, |[hi, lo]| {
        let mut out = vec![0x19, 0x07, 0x2a, 0xb4, 0x00, 0x0c, 0xb8, hi, lo, 0x3c];
        out.resize(mask.len(), 0x00);
        out
    })
}

/// HZ-025. See its `evidence`.
fn pickup_notices_to_host(class: &[u8]) -> Option<Vec<u8>> {
    // bipush 33 (the anchor); invokevirtual #209 dd.a(Lcw;III)V -> iload_2; nop; invokestatic
    call_hook(class, "c", "()V", 1457, &[0x10, 0x21, 0xb6, 0x00, 0xd1], hooks::PICKUP_NOTICE, |[hi, lo]| vec![0x1c, 0x00, 0xb8, hi, lo])
}

/// HZ-024 `o`: the windows' soft-key pictures are never drawn (both bit tests read 0), nor the old
/// bar's corner box the in-game help and options put beside them (`o.c` reads false).
fn soft_art_off(class: &[u8]) -> Option<Vec<u8>> {
    // iload_1 (the flags) -> iconst_0, before `iconst_1; iand; ifeq` and `iconst_2; iand; ifeq`
    let c = classfile::rewrite_code(class, "a", "(Ldd;I)V", 34, &[0x1b, 0x04, 0x7e, 0x99], &[0x03, 0x04, 0x7e, 0x99])?;
    let c = classfile::rewrite_code(&c, "a", "(Ldd;I)V", 53, &[0x1b, 0x05, 0x7e, 0x99], &[0x03, 0x05, 0x7e, 0x99])?;
    // getstatic #45 o.c:Z; ifeq -> iconst_0; nop; nop; ifeq
    classfile::rewrite_code(&c, "a", "(Ldd;I)V", 74, &[0xb2, 0x00, 0x2d, 0x99], &[0x03, 0x00, 0x00, 0x99])
}

/// HZ-024 `g`: the game's minimap window draws no back picture.
fn soft_art_off_map(class: &[u8]) -> Option<Vec<u8>> {
    // getstatic #158 o.a:Lcw; ifnull -> aconst_null; nop; nop; ifnull
    classfile::rewrite_code(class, "a", "(Ldd;)V", 423, &[0xb2, 0x00, 0x9e, 0xc6], &[0x01, 0x00, 0x00, 0xc6])
}

/// HZ-023 `ba`: the item factory's read of a record. Alone it replaces the game's read; after HZ-016
/// it replaces HZ-016's hook with the one that serves both.
fn gear_records(class: &[u8]) -> Option<Vec<u8>> {
    // invokestatic #68 rpg/e.a(Ljava/lang/String;I)[B
    if let Some(c) = call_hook(class, "a", "(BB)Lba;", 107, &[0xb8, 0x00, 0x44], hooks::GEAR_RECORD, invokestatic) {
        return Some(c);
    }
    // HZ-016's hook: its Methodref is already in the pool (methodref finds it, so nothing grows).
    let (pool, index) = classfile::methodref(class, hooks::CLASS, hooks::ITEM_RECORD.name, hooks::ITEM_RECORD.descriptor)?;
    (pool.len() == class.len()).then_some(())?;
    let [hi, lo] = index.to_be_bytes();
    call_hook(class, "a", "(BB)Lba;", 107, &[0xb8, hi, lo], hooks::ORB_GEAR_RECORD, invokestatic)
}

/// HZ-023 `g`: the ground list takes numbers up to the tier's.
fn gear_ground(class: &[u8]) -> Option<Vec<u8>> {
    // iload 4; bipush 28; if_icmple +4 (id <= 28, else return)
    classfile::rewrite_code(class, "a", "(BBBB)V", 15, &[0x15, 0x04, 0x10, 0x1c, 0xa4], &[0x15, 0x04, 0x10, hooks::GEAR_ID as u8, 0xa4])
}

/// HZ-023 `cq`: the rare band's number in a drop.
fn gear_drop(class: &[u8]) -> Option<Vec<u8>> {
    // invokestatic #259 java/lang/Math.min(II)I — the second of o()V's two (the first, @158, is the
    // common band's min(lvl / 4, 14))
    call_hook(class, "o", "()V", 215, &[0xb8, 0x01, 0x03], hooks::RARE_DROP, invokestatic)
}

/// HZ-020 `rpg/b`: the six reads of the store-name arrays.
fn slot_pages_stores(class: &[u8]) -> Option<Vec<u8>> {
    // getstatic #47 a / #49 b / #51 c :[Ljava/lang/String;
    let mut c = class.to_vec();
    for (name, cp, hook) in [("a", 0x2f, hooks::GAMEDATA_STORES), ("b", 0x31, hooks::ITEM_STORES), ("c", 0x33, hooks::SLOT_STORES)] {
        c = call_hook(&c, name, "(B)[B", 0, &[0xb2, 0x00, cp], hook, invokestatic)?;
        c = call_hook(&c, name, "([BB)Z", 0, &[0xb2, 0x00, cp], hook, invokestatic)?;
    }
    Some(c)
}

/// HZ-020 `bb` (the slot window): its keys and its three draws.
fn slot_pages_window(class: &[u8]) -> Option<Vec<u8>> {
    // invokevirtual #90 o.b(II)Z
    let c = call_hook(class, "a", "(II)Z", 3, &[0xb6, 0x00, 0x5a], hooks::SLOT_KEY, invokestatic)?;
    // invokestatic #63 ah.a(Ldd;II[BII)V (the title, then a row's label); #89 o.a(Ldd;IIIIII)V (a row's box)
    let c = call_hook(&c, "a", "(Ldd;)V", 24, &[0xb8, 0x00, 0x3f], hooks::SLOT_TITLE, invokestatic)?;
    let c = call_hook(&c, "a", "(Ldd;)V", 93, &[0xb8, 0x00, 0x59], hooks::SLOT_BOX, invokestatic)?;
    call_hook(&c, "a", "(Ldd;)V", 173, &[0xb8, 0x00, 0x3f], hooks::SLOT_LABEL, invokestatic)
}

/// HZ-020 `i` (the title): its two reads of the save count.
fn slot_pages_title(class: &[u8]) -> Option<Vec<u8>> {
    // getstatic #49 ed.i:B
    let c = call_hook(class, "<init>", "()V", 8, &[0xb2, 0x00, 0x31], hooks::SAVE_COUNT, invokestatic)?;
    call_hook(&c, "a", "(II)Z", 28, &[0xb2, 0x00, 0x31], hooks::SAVE_COUNT, invokestatic)
}

/// HZ-021 `co` (the shop): the gold's draw, then the button.
fn bulk_sell_button(class: &[u8]) -> Option<Vec<u8>> {
    // invokevirtual #97 o.b(Ldd;III)V
    call_hook(class, "a", "(Ldd;)V", 323, &[0xb6, 0x00, 0x61], hooks::SHOP_GOLD, invokestatic)
}

/// HZ-021 `ee` (the list popup): its answer to the parent goes through [`hooks::ANSWER`].
fn bulk_sell_popup(class: &[u8]) -> Option<Vec<u8>> {
    // invokevirtual #28 o.a(BB)V
    call_hook(class, "a", "(II)Z", 36, &[0xb6, 0x00, 0x1c], hooks::ANSWER, invokestatic)
}

/// HZ-022 `ek` (the player): the EXP loop, the next-level store and the two stat accessors.
fn guardian_levels(class: &[u8]) -> Option<Vec<u8>> {
    // getfield #141 n:B (the level, before `bipush 99; if_icmpge`)
    let c = call_hook(class, "j", "(I)V", 75, &[0xb4, 0x00, 0x8d], hooks::LEVEL_UP, invokestatic)?;
    // getfield #110 b:[I (the store of b[2])
    let c = call_hook(&c, "i", "()V", 1009, &[0xb4, 0x00, 0x6e], hooks::EXP_TABLE, invokestatic)?;
    // aload_0; getfield a / b; iload_1; iaload; ireturn -> aload_0; iload_1; invokestatic; ireturn; nop
    let c = call_hook(&c, "e", "(B)I", 0, &[0x2a, 0xb4, 0x00, 0x64, 0x1b, 0x2e, 0xac], hooks::STAT_NOW, |[hi, lo]| vec![0x2a, 0x1b, 0xb8, hi, lo, 0xac, 0x00])?;
    call_hook(&c, "f", "(B)I", 0, &[0x2a, 0xb4, 0x00, 0x6e, 0x1b, 0x2e, 0xac], hooks::STAT_MAX, |[hi, lo]| vec![0x2a, 0x1b, 0xb8, hi, lo, 0xac, 0x00])
}

/// HZ-019. See its `evidence`.
fn notices_over_exp_bar(class: &[u8]) -> Option<Vec<u8>> {
    // getstatic #88 ce.h:I
    call_hook(class, "c", "()V", 1439, &[0xb2, 0x00, 0x58], hooks::NOTICE_BOTTOM, invokestatic)
}

/// HZ-017. See its `evidence`.
fn monster_difficulty(class: &[u8]) -> Option<Vec<u8>> {
    // invokevirtual #223 q()V
    call_hook(class, "i", "()V", 208, &[0xb6, 0x00, 0xdf], hooks::MONSTER_INIT, invokestatic)
}

/// HZ-018. See its `evidence`.
fn monster_respawn(class: &[u8]) -> Option<Vec<u8>> {
    // getfield #105 eq.a:S
    call_hook(class, "g", "()V", 11, &[0xb4, 0x00, 0x69], hooks::RESPAWN_DELAY, invokestatic)
}

/// HZ-015 `bh`: the digit and the cursor no longer treat 상점및창고 as locked.
fn shop_unlock_bh(class: &[u8]) -> Option<Vec<u8>> {
    let c = flag_true(class, "a", "(II)Z", 75, [0x00, 0x28])?;
    flag_true(&c, "a", "(II)Z", 256, [0x00, 0x28])
}

/// HZ-015 `m`: the row is not drawn grey.
fn shop_unlock_m(class: &[u8]) -> Option<Vec<u8>> {
    flag_true(class, "a", "(Ldd;IIZ)I", 335, [0x00, 0x1b])
}

/// HZ-016 `ba` (the item): the orb's record, its picture, and no 0/# registration for it.
fn revive_orb_item(class: &[u8]) -> Option<Vec<u8>> {
    // invokestatic #68 rpg/e.a(Ljava/lang/String;I)[B
    let c = call_hook(class, "a", "(BB)Lba;", 107, &[0xb8, 0x00, 0x44], hooks::ITEM_RECORD, invokestatic)?;
    // getstatic #33 rpg/e.k:[Lcw;; iload 4; aaload -> iload 4; invokestatic Hooks.itemIcon; nop
    let c = call_hook(&c, "a", "(Ldd;IIZZ)V", 76, &[0xb2, 0x00, 0x21, 0x15, 0x04, 0x32], hooks::ITEM_ICON, |[hi, lo]| {
        vec![0x15, 0x04, 0xb8, hi, lo, 0x00]
    })?;
    // a()Z: cat == 20 && id >= 2 -> aload_0; invokestatic Hooks.quickItem; ireturn; nop x16
    const QUICK: [u8; 21] = [
        0x2a, 0xb4, 0x00, 0x17, 0x10, 0x14, 0xa0, 0x00, 0x0d, 0x2a, 0xb4, 0x00, 0x1c, 0x05, 0xa1, 0x00, 0x05, 0x04, 0xac, 0x03,
        0xac,
    ];
    call_hook(&c, "a", "()Z", 0, &QUICK, hooks::QUICK_ITEM, |[hi, lo]| {
        let mut with = vec![0x2a, 0xb8, hi, lo, 0xac];
        with.resize(QUICK.len(), 0);
        with
    })
}

/// HZ-016 `co` (the shop): the special tab lists numbers 0..9 — the orb is the tenth.
fn revive_orb_shop(class: &[u8]) -> Option<Vec<u8>> {
    // bipush 20; bipush 8 (co.a(BI)V: category, last number)
    classfile::rewrite_code(class, "h", "()V", 315, &[0x10, 0x14, 0x10, 0x08], &[0x10, 0x14, 0x10, hooks::REVIVE_ORB.1 as u8])
}

/// HZ-016 `ek` (the death): the orb's question instead of the cheat shop's purchase.
fn revive_orb_death(class: &[u8]) -> Option<Vec<u8>> {
    // getstatic #159 ey.f:Z — or HZ-010's iconst_1; nop; nop at the same site.
    let c = call_hook(class, "C", "()V", 512, &[0xb2, 0x00, 0x9f], hooks::DEATH, invokestatic)
        .or_else(|| call_hook(class, "C", "()V", 512, &[0x04, 0x00, 0x00], hooks::DEATH, invokestatic))?;
    // ed.d = 5; ek.d = true; bh.a().a(); bh.a().c(3); ((ci) bh.a().b).a(); return
    const PURCHASE: [u8; 36] = [
        0x08, 0xb3, 0x00, 0x59, 0x04, 0xb3, 0x00, 0x76, 0xb8, 0x00, 0xd3, 0xb6, 0x00, 0xd4, 0xb8, 0x00, 0xd3, 0x06, 0xb6, 0x00,
        0xd5, 0xb8, 0x00, 0xd3, 0xb4, 0x00, 0xa3, 0xc0, 0x00, 0x16, 0x59, 0x4c, 0xb6, 0x00, 0xe1, 0xb1,
    ];
    let mut done = [0u8; PURCHASE.len()];
    done[0] = 0xb1; // return: the hook opened the question
    classfile::rewrite_code(&c, "C", "()V", 527, &PURCHASE, &done)
}

/// HZ-016 `cf` (the 예/아니오 window): both answers go through the hook.
fn revive_orb_answer(class: &[u8]) -> Option<Vec<u8>> {
    // invokevirtual #48 o.a(BB)V
    let c = call_hook(class, "a", "(II)Z", 59, &[0xb6, 0x00, 0x30], hooks::ANSWER, invokestatic)?;
    call_hook(&c, "a", "(II)Z", 89, &[0xb6, 0x00, 0x30], hooks::ANSWER, invokestatic)
}

/// HZ-014. See its `evidence`.
fn target_dodge(class: &[u8]) -> Option<Vec<u8>> {
    // bipush 80; isub
    call_hook(class, "f", "()V", 26, &[0x10, 80, 0x64], hooks::TARGET_X, invokestatic)
}

/// HZ-054. See its `evidence`. Both draws or neither.
fn target_drop(class: &[u8]) -> Option<Vec<u8>> {
    // invokevirtual #185 dd.a(Lcw;III)V, then #184 dd.a(IIII)V
    let c = call_hook(class, "f", "()V", 40, &[0xb6, 0x00, 0xb9], hooks::TARGET_PICTURE, invokestatic)?;
    call_hook(&c, "f", "()V", 83, &[0xb6, 0x00, 0xb8], hooks::TARGET_BAR, invokestatic)
}

/// HZ-013: `getstatic rpg/a.b; bipush 33` (`cp` = rpg/a.b in that class) at `at` measures from the
/// whole canvas: `bipush 0`.
fn canvas_not_bar(class: &[u8], method: &str, descriptor: &str, at: usize, cp: [u8; 2]) -> Option<Vec<u8>> {
    classfile::rewrite_code(class, method, descriptor, at, &[0xb2, cp[0], cp[1], 0x10, 33], &[0xb2, cp[0], cp[1], 0x10, 0])
}

/// HZ-013 `ce`: the viewport is the whole canvas and the bottom bar is never drawn.
fn hud_off_field(class: &[u8]) -> Option<Vec<u8>> {
    let c = canvas_not_bar(class, "<init>", "()V", 113, [0x00, 0x75])?;
    // getstatic #114 rpg/a.a:I -> goto +984 (107 -> 1091)
    classfile::rewrite_code(&c, "f", "()V", 107, &[0xb2, 0x00, 0x72], &[0xa7, 0x03, 0xd8])
}

/// HZ-013 `g`: the full-screen window fills black down to the bottom.
fn hud_off_window(class: &[u8]) -> Option<Vec<u8>> {
    canvas_not_bar(class, "a", "(Ldd;)V", 157, [0x00, 0xa3])
}

/// HZ-013 `eb`: the scenario's grid and centred box are measured from the whole canvas.
fn hud_off_scenario(class: &[u8]) -> Option<Vec<u8>> {
    let c = canvas_not_bar(class, "a", "(Ldd;)V", 58, [0x00, 0x98])?;
    canvas_not_bar(&c, "a", "(Ldd;)V", 533, [0x00, 0x98])
}

/// HZ-008. See its `evidence`.
fn scene_blit(class: &[u8]) -> Option<Vec<u8>> {
    // invokevirtual #82 javax/microedition/lcdui/Graphics.drawImage(Ljavax/microedition/lcdui/Image;III)V
    call_hook(class, "paint", "(Ljavax/microedition/lcdui/Graphics;)V", 245, &[0xb6, 0x00, 0x52], hooks::BLIT, invokestatic)
}

/// HZ-002. See its `evidence`.
fn seal_mail_send(class: &[u8]) -> Option<Vec<u8>> {
    // aload_0; new #16 (dx); dup; aload_0; aconst_null; invokespecial #49 (dx.<init>); putfield #36 (o.b)
    const EXPECT: [u8; 13] = [0x2a, 0xbb, 0x00, 0x10, 0x59, 0x2a, 0x01, 0xb7, 0x00, 0x31, 0xb5, 0x00, 0x24];
    // goto +55 (143 -> 198); nop x10
    const WITH: [u8; 13] = [0xa7, 0x00, 0x37, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    classfile::rewrite_code(class, "a", "(II)Z", 143, &EXPECT, &WITH)
}

/// `expect` at `at` becomes `with(index of Methodref Hooks.<hook>)`.
fn call_hook(class: &[u8], method: &str, descriptor: &str, at: usize, expect: &[u8], hook: hooks::Hook, with: impl Fn([u8; 2]) -> Vec<u8>) -> Option<Vec<u8>> {
    classfile::rewrite_with_methodref(class, method, descriptor, at, expect, (hooks::CLASS, hook.name, hook.descriptor), |i| with(i.to_be_bytes()))
}

/// The common case: a 3-byte invoke becomes `invokestatic` of the hook.
fn invokestatic([hi, lo]: [u8; 2]) -> Vec<u8> {
    vec![0xb8, hi, lo]
}

/// HZ-010: one `getstatic ey.f` (b2 cp cp) at `at` becomes `iconst_1; nop; nop` (HZ-015: the same for
/// the 상점및창고 lock `bh.h`).
fn flag_true(class: &[u8], method: &str, descriptor: &str, at: usize, cp: [u8; 2]) -> Option<Vec<u8>> {
    classfile::rewrite_code(class, method, descriptor, at, &[0xb2, cp[0], cp[1]], &[0x04, 0x00, 0x00])
}

/// HZ-010 `i` (title carousel). See the patch's `evidence`.
fn cheat_shop_title(class: &[u8]) -> Option<Vec<u8>> {
    flag_true(class, "a", "(II)Z", 42, [0x00, 0x32])
}

/// HZ-012: one `getstatic ey.f` (b2 cp cp) at `at` becomes `iconst_0; nop; nop` — the flag as it
/// always is here, with the cheat-shop row gone there is nothing left for it to lock.
fn flag_false(class: &[u8], method: &str, descriptor: &str, at: usize, cp: [u8; 2]) -> Option<Vec<u8>> {
    classfile::rewrite_code(class, method, descriptor, at, &[0xb2, cp[0], cp[1]], &[0x03, 0x00, 0x00])
}

/// `bh.d(B)V` @0, as the game has it: `iload_1; putstatic bh.f:B; iload_1; tableswitch` 0..9 —
/// pad, default 197, then the ten 4-byte targets of the rows (0 치트상점 … 8 미니맵, 9 시스템).
const MENU_SWITCH: [u8; 60] = [
    0x1b, 0xb3, 0x00, 0x23, 0x1b, 0xaa, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc5, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x09,
    0x00, 0x00, 0x00, 0xac, 0x00, 0x00, 0x00, 0x37, 0x00, 0x00, 0x00, 0x44, 0x00, 0x00, 0x00, 0x9f, 0x00, 0x00, 0x00, 0x52,
    0x00, 0x00, 0x00, 0x5f, 0x00, 0x00, 0x00, 0x6c, 0x00, 0x00, 0x00, 0x79, 0x00, 0x00, 0x00, 0x93, 0x00, 0x00, 0x00, 0x86,
];

/// HZ-012's menu: row r runs the game's row `[r]` — 캐릭터 … 미니맵 moved up one, the cheat shop's
/// block (the storage hook) at 8, 시스템 at 9.
const MENU_ROWS_HZ012: [usize; 10] = [1, 2, 3, 4, 5, 6, 7, 8, 0, 9];

/// HZ-026's menu: 미니맵 gone — 계정창고 at 7, 시스템 at 8; row 9 (past the nine rows) keeps the
/// minimap's block so every target is still one of the game's.
const MENU_ROWS_HZ026: [usize; 10] = [1, 2, 3, 4, 5, 6, 7, 0, 9, 8];

/// [`MENU_SWITCH`] with row r jumping where the game's row `rows[r]` did.
fn menu_switch(rows: [usize; 10]) -> [u8; 60] {
    let mut out = MENU_SWITCH;
    for (r, old) in rows.into_iter().enumerate() {
        out[20 + 4 * r..24 + 4 * r].copy_from_slice(&MENU_SWITCH[20 + 4 * old..24 + 4 * old]);
    }
    out
}

/// HZ-012 `bh`. See the patch's `evidence`.
fn menu_storage_bh(class: &[u8]) -> Option<Vec<u8>> {
    let c = flag_false(class, "<init>", "()V", 7, [0x00, 0x32])?;
    // a(II)Z: '7' -> '6' (정령 lock), ey.f/'1' false, '4' -> '3' (상점 lock), cursor skip 6 -> 5,
    // ey.f cursor test false, cursor skip 3 -> 2.
    let c = classfile::rewrite_code(&c, "a", "(II)Z", 23, &[0x1c, 0x10, 0x37, 0xa0], &[0x1c, 0x10, 0x36, 0xa0])?;
    let c = flag_false(&c, "a", "(II)Z", 49, [0x00, 0x32])?;
    let c = classfile::rewrite_code(&c, "a", "(II)Z", 81, &[0x1c, 0x10, 0x34, 0xa0], &[0x1c, 0x10, 0x33, 0xa0])?;
    let c = classfile::rewrite_code(&c, "a", "(II)Z", 189, &[0x2a, 0xb4, 0x00, 0x33, 0x10, 0x06, 0xa0], &[0x2a, 0xb4, 0x00, 0x33, 0x10, 0x05, 0xa0])?;
    let c = flag_false(&c, "a", "(II)Z", 232, [0x00, 0x32])?;
    let c = classfile::rewrite_code(&c, "a", "(II)Z", 262, &[0x2a, 0xb4, 0x00, 0x33, 0x06, 0xa0], &[0x2a, 0xb4, 0x00, 0x33, 0x05, 0xa0])?;
    // c(B)V mode 1: row 4 (장비) -> 3.
    let c = classfile::rewrite_code(&c, "c", "(B)V", 28, &[0x2a, 0x07, 0xb5, 0x00, 0x33, 0x2a, 0x07, 0xb7], &[0x2a, 0x06, 0xb5, 0x00, 0x33, 0x2a, 0x06, 0xb7])?;
    // d(B)V: the switch targets rotate; old case 0 (치트상점) becomes the storage hook.
    let c = classfile::rewrite_code(&c, "d", "(B)V", 0, &MENU_SWITCH, &menu_switch(MENU_ROWS_HZ012))?;
    const SHOP_BLOCK: [u8; 25] = [
        0x2a, 0xbb, 0x00, 0x0a, 0x59, 0x2a, 0xb7, 0x00, 0x3e, 0xb5, 0x00, 0x35, 0xb2, 0x00, 0x31, 0x9a, 0x00, 0x0a, 0x10, 0x3d,
        0x12, 0x03, 0xb8, 0x00, 0x40,
    ];
    call_hook(&c, "d", "(B)V", 177, &SHOP_BLOCK, hooks::OPEN_STORAGE, |[hi, lo]| {
        let mut with = vec![0x2a, 0xb8, hi, lo, 0xb1];
        with.resize(SHOP_BLOCK.len(), 0);
        with
    })
}

/// HZ-012 `m`: locked rows move up with the labels, and the labels come from the hook.
fn menu_storage_m(class: &[u8]) -> Option<Vec<u8>> {
    let c = flag_false(class, "a", "(Ldd;IIZ)I", 323, [0x00, 0x20])?;
    let c = classfile::rewrite_code(&c, "a", "(Ldd;IIZ)I", 329, &[0x15, 0x0a, 0x06, 0xa0], &[0x15, 0x0a, 0x05, 0xa0])?;
    let c = classfile::rewrite_code(&c, "a", "(Ldd;IIZ)I", 341, &[0x15, 0x0a, 0x10, 0x06, 0xa0], &[0x15, 0x0a, 0x10, 0x05, 0xa0])?;
    // The blinking point marks: stat on row 1 -> 0 (캐릭터), skill on 5 / 6 -> 4 / 5 (스킬 / 정령).
    let c = classfile::rewrite_code(&c, "a", "(Ldd;IIZ)I", 452, &[0x15, 0x0a, 0x04, 0xa0], &[0x15, 0x0a, 0x03, 0xa0])?;
    let c = classfile::rewrite_code(&c, "a", "(Ldd;IIZ)I", 510, &[0x15, 0x0a, 0x08, 0x9f], &[0x15, 0x0a, 0x07, 0x9f])?;
    let c = classfile::rewrite_code(&c, "a", "(Ldd;IIZ)I", 516, &[0x15, 0x0a, 0x10, 0x06, 0xa0], &[0x15, 0x0a, 0x10, 0x05, 0xa0])?;
    // getstatic bh.a -> aload_0 (this), and the cx.a(I) call -> Hooks.menuLabel(this, index).
    let c = classfile::rewrite_code(&c, "a", "(Ldd;IIZ)I", 374, &[0xb2, 0x00, 0x19], &[0x2a, 0x00, 0x00])?;
    let c = call_hook(&c, "a", "(Ldd;IIZ)I", 381, &[0xb6, 0x00, 0x30], hooks::MENU_LABEL, invokestatic)?;
    let c = classfile::rewrite_code(&c, "a", "(Ldd;IIZ)I", 612, &[0xb2, 0x00, 0x19], &[0x2a, 0x00, 0x00])?;
    call_hook(&c, "a", "(Ldd;IIZ)I", 621, &[0xb6, 0x00, 0x30], hooks::MENU_LABEL, invokestatic)
}

/// HZ-012 `o`: a menu child's title is the menu's selected label — through the hook.
fn menu_storage_o(class: &[u8]) -> Option<Vec<u8>> {
    let c = classfile::rewrite_code(class, "a", "(Ldd;ZZZZ)V", 6, &[0xb2, 0x00, 0x1d], &[0x2a, 0x00, 0x00])?;
    call_hook(&c, "a", "(Ldd;ZZZZ)V", 12, &[0xb6, 0x00, 0x45], hooks::MENU_LABEL, invokestatic)
}

/// HZ-012 `bg`: RSK closes the chooser through the hook (which quick-saves in game).
fn menu_storage_bg(class: &[u8]) -> Option<Vec<u8>> {
    // aload_0; getfield #21 (o.a); invokevirtual #38 (o.c()V)
    call_hook(class, "a", "(II)Z", 19, &[0x2a, 0xb4, 0x00, 0x15, 0xb6, 0x00, 0x26], hooks::STORAGE_CLOSED, |[hi, lo]| {
        vec![0x2a, 0xb8, hi, lo, 0x00, 0x00, 0x00]
    })
}

/// HZ-012: a menu child closing puts the menu's cursor back on its row, `aload_0; getfield
/// <parent> (o.a:Lo); <row> (iconst_n, or bipush n when `wide`); putfield <cursor> (o.a:B)` at
/// `at` of `a(II)Z` — one row up, like the labels.
fn menu_row(class: &[u8], at: usize, parent: [u8; 2], cursor: [u8; 2], row: u8, wide: bool) -> Option<Vec<u8>> {
    let site = |row: u8| {
        let push = if wide { vec![0x10, row] } else { vec![0x03 + row] };
        [&[0x2a, 0xb4, parent[0], parent[1]][..], &push, &[0xb5, cursor[0], cursor[1]]].concat()
    };
    classfile::rewrite_code(class, "a", "(II)Z", at, &site(row), &site(row - 1))
}

/// HZ-012 `dy` (캐릭터): both of its closes.
fn menu_row_dy(class: &[u8]) -> Option<Vec<u8>> {
    let c = menu_row(class, 19, [0x00, 0x19], [0x00, 0x17], 1, false)?;
    menu_row(&c, 126, [0x00, 0x19], [0x00, 0x17], 1, false)
}

/// HZ-012 `de` (상점 및 창고).
fn menu_row_de(class: &[u8]) -> Option<Vec<u8>> {
    menu_row(class, 33, [0x00, 0x11], [0x00, 0x10], 3, false)
}

/// HZ-012 `em` (장비).
fn menu_row_em(class: &[u8]) -> Option<Vec<u8>> {
    menu_row(class, 43, [0x00, 0x26], [0x00, 0x24], 4, false)
}

/// HZ-012 `ad` (스킬).
fn menu_row_ad(class: &[u8]) -> Option<Vec<u8>> {
    menu_row(class, 47, [0x00, 0x26], [0x00, 0x24], 5, false)
}

/// HZ-012 `cr` (정령).
fn menu_row_cr(class: &[u8]) -> Option<Vec<u8>> {
    menu_row(class, 38, [0x00, 0x2b], [0x00, 0x29], 6, true)
}

/// HZ-012 `ao` (퀘스트).
fn menu_row_ao(class: &[u8]) -> Option<Vec<u8>> {
    menu_row(class, 27, [0x00, 0x22], [0x00, 0x20], 7, true)
}

/// HZ-026 `bh`: nine rows, the switch past 퀘스트 re-rotated, '0' and the quick save on row 8, and
/// '9' no shortcut. Only on HZ-012's menu (the switch must be its rotation).
fn menu_no_minimap_bh(class: &[u8]) -> Option<Vec<u8>> {
    // <init>: bipush 10 (the rows, m(o, byte)) -> 9
    let c = classfile::rewrite_code(class, "<init>", "()V", 2, &[0x10, 0x0a, 0xb7], &[0x10, 0x09, 0xb7])?;
    // a(II)Z: the digit range '0'..'9' -> '0'..'8'; '0' puts the cursor on row 9 -> 8
    let c = classfile::rewrite_code(&c, "a", "(II)Z", 17, &[0x1c, 0x10, 0x39, 0xa3], &[0x1c, 0x10, 0x38, 0xa3])?;
    let c = classfile::rewrite_code(&c, "a", "(II)Z", 107, &[0x2a, 0x10, 0x09, 0xb5, 0x00, 0x33], &[0x2a, 0x10, 0x08, 0xb5, 0x00, 0x33])?;
    // c(B)V mode 2 (시스템 + quick save): row 9 -> 8, cursor and dispatch
    let c = classfile::rewrite_code(
        &c,
        "c",
        "(B)V",
        47,
        &[0x2a, 0x10, 0x09, 0xb5, 0x00, 0x33, 0x2a, 0x10, 0x09, 0xb7],
        &[0x2a, 0x10, 0x08, 0xb5, 0x00, 0x33, 0x2a, 0x10, 0x08, 0xb7],
    )?;
    classfile::rewrite_code(&c, "d", "(B)V", 0, &menu_switch(MENU_ROWS_HZ012), &menu_switch(MENU_ROWS_HZ026))
}

/// HZ-026 `m`: the menu's numbers put 「0.」 on row 8 (was 9). Only on HZ-012's list — its label
/// read at @374 — so the game's own ten rows never number 「0.」 twice.
fn menu_no_minimap_m(class: &[u8]) -> Option<Vec<u8>> {
    let c = classfile::rewrite_code(class, "a", "(Ldd;IIZ)I", 374, &[0x2a, 0x00, 0x00], &[0x2a, 0x00, 0x00])?;
    let c = classfile::rewrite_code(&c, "a", "(Ldd;IIZ)I", 413, &[0x15, 0x0a, 0x10, 0x09, 0xa0], &[0x15, 0x0a, 0x10, 0x08, 0xa0])?;
    classfile::rewrite_code(&c, "a", "(Ldd;IIZ)I", 653, &[0x2a, 0xb4, 0x00, 0x21, 0x10, 0x09, 0xa0], &[0x2a, 0xb4, 0x00, 0x21, 0x10, 0x08, 0xa0])
}

/// HZ-026 `ec` (시스템): closing no longer puts the menu's cursor on row 9 — it is already on 시스템's
/// row (`bh.d(B)V` is only reached with the cursor there), whichever menu this is.
fn menu_no_minimap_ec(class: &[u8]) -> Option<Vec<u8>> {
    // aload_0; getfield #21 (o.a:Lo); bipush 9; putfield #20 (o.a:B) -> nop x9
    classfile::rewrite_code(class, "a", "(II)Z", 57, &[0x2a, 0xb4, 0x00, 0x15, 0x10, 0x09, 0xb5, 0x00, 0x14], &[0; 9])
}

/// HZ-027 `bg`: its title 「계정창고」 one row pitch above the 「돈」 plate it was drawn under.
fn storage_title_clear(class: &[u8]) -> Option<Vec<u8>> {
    // getstatic rpg/a.c; getstatic rpg/a.d; bipush 64 -> 82; isub
    classfile::rewrite_code(class, "a", "(Ldd;)V", 12, &[0xb2, 0x00, 0x18, 0xb2, 0x00, 0x19, 0x10, 0x40, 0x64], &[0xb2, 0x00, 0x18, 0xb2, 0x00, 0x19, 0x10, 0x52, 0x64])
}

/// HZ-010 `ce` (the field): RSK no longer opens the store, and the softkey art loses STORE.
fn cheat_shop_field(class: &[u8]) -> Option<Vec<u8>> {
    let class = flag_true(class, "a", "(II)Z", 278, [0x00, 0x69])?;
    flag_true(&class, "f", "()V", 375, [0x00, 0x69])
}

/// HZ-010 `ek` (the player's death): no 「즉시 부활」 purchase — the game over a refusal leads to.
fn cheat_shop_revive(class: &[u8]) -> Option<Vec<u8>> {
    flag_true(class, "C", "()V", 512, [0x00, 0x9f])
}

/// HZ-010 `co` (the shop): not enough gold shows the game's message, not the gold purchase.
fn cheat_shop_gold_co(class: &[u8]) -> Option<Vec<u8>> {
    flag_true(class, "a", "(BB)V", 138, [0x00, 0x30])
}

/// HZ-010 `et`: the same, at both of its price checks.
fn cheat_shop_gold_et(class: &[u8]) -> Option<Vec<u8>> {
    let class = flag_true(class, "a", "(BB)V", 59, [0x00, 0x2d])?;
    flag_true(&class, "a", "(II)Z", 301, [0x00, 0x2d])
}

/// HZ-010 `bi`: the same, at both of its price checks (a popup with the game's text).
fn cheat_shop_gold_bi(class: &[u8]) -> Option<Vec<u8>> {
    let class = flag_true(class, "a", "(BB)V", 88, [0x00, 0x27])?;
    flag_true(&class, "a", "(BB)V", 229, [0x00, 0x27])
}

/// HZ-010 `dv` (the alchemist): missing materials take the game's own path, not 「빠른 합성」.
fn cheat_shop_synthesis(class: &[u8]) -> Option<Vec<u8>> {
    flag_true(class, "a", "(II)Z", 1100, [0x00, 0x3a])
}

/// HZ-011. See its `evidence`.
fn text_rename(class: &[u8]) -> Option<Vec<u8>> {
    // aload_0; getfield #9 (a:[Ljava/lang/Object;); iload_1; aaload; checkcast #2 ([B); areturn
    const EXPECT: [u8; 10] = [0x2a, 0xb4, 0x00, 0x09, 0x1b, 0x32, 0xc0, 0x00, 0x02, 0xb0];
    // aload_0; iload_1; invokestatic Hooks.text(Object,I)[B; areturn; nop x4
    call_hook(class, "a", "(I)[B", 0, &EXPECT, hooks::TEXT, |[hi, lo]| vec![0x2a, 0x1b, 0xb8, hi, lo, 0xb0, 0, 0, 0, 0])
}

/// HZ-011 `bb` (the title's storage flow): `getstatic #54 rpg/d.J:[B` -> the help hook.
fn storage_help_title(class: &[u8]) -> Option<Vec<u8>> {
    call_hook(class, "<init>", "(Lo;B)V", 149, &[0xb2, 0x00, 0x36], hooks::STORAGE_HELP, invokestatic)
}

/// HZ-011 `de` (상점및창고 → 창고): `getstatic #20 rpg/d.J:[B` -> the help hook.
fn storage_help_game(class: &[u8]) -> Option<Vec<u8>> {
    call_hook(class, "a", "(II)Z", 136, &[0xb2, 0x00, 0x14], hooks::STORAGE_HELP, invokestatic)
}

/// HZ-009. See its `evidence`.
fn cash_free_toggle(class: &[u8]) -> Option<Vec<u8>> {
    // aload_0; getfield #58 (ep.c); ifnull +19 (63); aload_0; getfield #58; ldc #15 ("false");
    // invokevirtual #79 (String.equals); ifeq +5 (61); iconst_0; ireturn; iconst_1; ireturn
    const EXPECT: [u8; 23] = [
        0x2a, 0xb4, 0x00, 0x3a, 0xc6, 0x00, 0x13, 0x2a, 0xb4, 0x00, 0x3a, 0x12, 0x0f, 0xb6, 0x00, 0x4f, 0x99, 0x00, 0x05, 0x03,
        0xac, 0x04, 0xac,
    ];
    // invokestatic Hooks.cashFree()Z; ireturn; nop x19
    call_hook(class, "a", "(I)Z", 40, &EXPECT, hooks::CASH_FREE, |[hi, lo]| {
        let mut with = vec![0xb8, hi, lo, 0xac];
        with.resize(EXPECT.len(), 0);
        with
    })
}

/// HZ-004. See its `evidence`.
fn exp_rate(class: &[u8]) -> Option<Vec<u8>> {
    // iload_1; ifge +8 (41); iload_1; ineg; goto +4 (42)
    const EXPECT: [u8; 9] = [0x1b, 0x9c, 0x00, 0x08, 0x1b, 0x74, 0xa7, 0x00, 0x04];
    // iload_1; invokestatic Hooks.exp(I)I; nop; nop; goto +4 (42)
    call_hook(class, "j", "(I)V", 32, &EXPECT, hooks::EXP, |[hi, lo]| vec![0x1b, 0xb8, hi, lo, 0, 0, 0xa7, 0x00, 0x04])
}

/// HZ-005. See its `evidence`.
fn drop_rate(class: &[u8]) -> Option<Vec<u8>> {
    // invokestatic #282 rpg/g.a([BI)I
    call_hook(class, "o", "()V", 87, &[0xb8, 0x01, 0x1a], hooks::DROP_THRESHOLD, invokestatic)
}

/// HZ-006. See its `evidence`.
fn auto_loot(class: &[u8]) -> Option<Vec<u8>> {
    // invokevirtual #221 g.a(BB)Z
    call_hook(class, "d", "()V", 77, &[0xb6, 0x00, 0xdd], hooks::AUTO_LOOT, invokestatic)
}

/// HZ-007. See its `evidence`. All eight edits or none.
fn extra_options(class: &[u8]) -> Option<Vec<u8>> {
    const DRAW: (&str, &str) = ("a", "(Ldd;)V");
    const ROWS: [u8; 2] = [0x10, 7]; // bipush 7
    const PAGE_ROWS: [u8; 2] = [0x10, hooks::OPTIONS_ROWS];
    const SLOTS: [u8; 2] = [0x10, hooks::OPTIONS_SLOTS];
    const TEXT_TABLE: [u8; 3] = [0xb2, 0x00, 0x27]; // getstatic #39 rpg/e.b:Lcx;
    const THIS: [u8; 3] = [0x2a, 0, 0]; // aload_0; nop; nop
    const LABEL: [u8; 3] = [0xb6, 0x00, 0x33]; // invokevirtual #51 cx.a(I)[B
    // getstatic #29 rpg/a.c:I; bipush 24; iadd; iconst_5; iadd; bipush 8; iadd; istore_3
    const VALUE_X: [u8; 12] = [0xb2, 0x00, 0x1d, 0x10, 0x18, 0x60, 0x08, 0x60, 0x10, 0x08, 0x60, 0x3e];
    const CHILD_FIRST: [u8; 3] = [0xb6, 0x00, 0x40]; // invokevirtual #64 o.b(II)Z

    let c = classfile::rewrite_code(class, "<init>", "(Lo;Z)V", 2, &ROWS, &PAGE_ROWS)?;
    let c = classfile::rewrite_code(&c, DRAW.0, DRAW.1, 104, &ROWS, &SLOTS)?;
    let c = classfile::rewrite_code(&c, DRAW.0, DRAW.1, 70, &TEXT_TABLE, &THIS)?;
    let c = call_hook(&c, DRAW.0, DRAW.1, 75, &LABEL, hooks::OPT_LABEL, invokestatic)?;
    let c = classfile::rewrite_code(&c, DRAW.0, DRAW.1, 113, &TEXT_TABLE, &THIS)?;
    let c = call_hook(&c, DRAW.0, DRAW.1, 121, &LABEL, hooks::OPT_LABEL, invokestatic)?;
    // aload_0; aload_1; iload_2; invokestatic Hooks.optValues; dup; istore_3; iflt +337 (145 -> 482); nop
    let c = call_hook(&c, DRAW.0, DRAW.1, 137, &VALUE_X, hooks::OPT_VALUES, |[hi, lo]| {
        vec![0x2a, 0x2b, 0x1c, 0xb8, hi, lo, 0x59, 0x3e, 0x9b, 0x01, 0x51, 0x00]
    })?;
    call_hook(&c, "a", "(II)Z", 3, &CHILD_FIRST, hooks::OPT_KEY, invokestatic)
}

/// Which patches to apply. Parsed from the probe's `--patches` and the app's settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selection {
    /// Every patch whose `default_on` is set.
    Default,
    /// Nothing — the original game.
    None,
    /// Exactly these ids.
    Only(Vec<String>),
}

impl Selection {
    /// `default` | `none` | `all` | `HZ-001,HZ-003` | `default,HZ-013` (a `default` in a list is
    /// every `default_on` id) | `default,-HZ-028` (a `-id` takes one out of what came before it —
    /// the A/B of one patch, 2026-10-28). Unknown ids are an error, so a typo cannot silently turn
    /// into "no patch".
    pub fn parse(s: &str) -> Result<Selection, String> {
        match s.trim() {
            "" | "default" => Ok(Selection::Default),
            "none" => Ok(Selection::None),
            "all" => Ok(Selection::Only(PATCHES.iter().map(|p| p.id.to_string()).collect())),
            list => {
                let mut ids: Vec<String> = Vec::new();
                for item in list.split(',').map(|x| x.trim().to_uppercase()).filter(|x| !x.is_empty()) {
                    let (out, item) = match item.strip_prefix('-') {
                        Some(rest) => (true, rest.to_string()),
                        None => (false, item),
                    };
                    if item == "DEFAULT" && !out {
                        ids.extend(PATCHES.iter().filter(|p| p.default_on).map(|p| p.id.to_string()));
                    } else if PATCHES.iter().any(|p| p.id == item.as_str()) {
                        if out {
                            ids.retain(|id| *id != item);
                        } else {
                            ids.push(item);
                        }
                    } else {
                        return Err(format!("unknown patch id {item} (known: {})", PATCHES.iter().map(|p| p.id).collect::<Vec<_>>().join(", ")));
                    }
                }
                ids.sort();
                ids.dedup();
                Ok(Selection::Only(ids))
            }
        }
    }

    /// Whether the patch `id` is chosen.
    pub fn wants_id(&self, id: &str) -> bool {
        PATCHES.iter().find(|p| p.id == id).is_some_and(|p| self.wants(p))
    }

    fn wants(&self, p: &Patch) -> bool {
        match self {
            Selection::Default => p.default_on,
            Selection::None => false,
            Selection::Only(ids) => ids.iter().any(|id| id == p.id),
        }
    }
}

/// The patches chosen for one boot of one build.
pub struct Patches {
    build: Build,
    chosen: Vec<&'static Patch>,
    /// Class rewrites that fired, in order (`id`), and the ones whose rewrite refused
    /// (`!id`). Filled by the transformer as the JVM loads classes.
    log: Arc<Mutex<Vec<String>>>,
}

impl Patches {
    pub fn new(build: Build, selection: &Selection) -> Patches {
        let chosen = PATCHES.iter().filter(|p| p.builds.contains(&build) && selection.wants(p)).collect();
        Patches { build, chosen, log: Arc::default() }
    }

    pub fn build(&self) -> Build {
        self.build
    }

    pub fn chosen(&self) -> impl Iterator<Item = &'static Patch> + '_ {
        self.chosen.iter().copied()
    }

    /// App properties to layer over the manifest (`J2meOptions::property_overrides`).
    pub fn property_overrides(&self) -> Vec<(String, String)> {
        self.chosen
            .iter()
            .filter_map(|p| match p.kind {
                PatchKind::AppProperty { key, value } => Some((key.to_string(), value.to_string())),
                _ => None,
            })
            .collect()
    }

    /// The class transformer (`J2meOptions::class_transformer`), or `None` when no chosen
    /// patch rewrites a class. A miss costs one md5 of the class bytes, once per class load.
    ///
    /// Several patches may target one class: they run in table order, each on the previous
    /// one's output (the md5 gate is on the ORIGINAL bytes; after that each rewrite checks
    /// its own expected bytes). A refusing rewrite is skipped and the rest still apply.
    #[allow(clippy::type_complexity)]
    pub fn class_transformer(&self) -> Option<Box<dyn Fn(&[u8]) -> Option<Vec<u8>> + Send + Sync>> {
        let mut table: HashMap<&'static str, Vec<(&'static str, Rewrite)>> = HashMap::new();
        for p in &self.chosen {
            for (class, class_md5, rewrite) in p.kind.targets() {
                // Logged as `id/class`, so a multi-class patch reports each class on its own.
                let tag: &'static str = Box::leak(format!("{}/{class}", p.id).into_boxed_str());
                table.entry(class_md5).or_default().push((tag, rewrite));
            }
        }
        if table.is_empty() {
            return None;
        }
        let log = self.log.clone();
        Some(Box::new(move |class: &[u8]| {
            let md5 = format!("{:x}", md5::compute(class));
            let rewrites = table.get(md5.as_str())?;
            let mut out: Option<Vec<u8>> = None;
            let mut log = log.lock().unwrap();
            for (tag, rewrite) in rewrites {
                match rewrite(out.as_deref().unwrap_or(class)) {
                    Some(new) => {
                        log.push(tag.to_string());
                        out = Some(new);
                    }
                    None => log.push(format!("!{tag}")),
                }
            }
            out
        }))
    }

    /// One line per chosen patch: applied, refused, or (for a class rewrite) not yet met.
    pub fn report(&self) -> Vec<String> {
        let log = self.log.lock().unwrap();
        self.chosen
            .iter()
            .map(|p| {
                let state = match p.kind {
                    PatchKind::AppProperty { key, value } => format!("property {key}={value}"),
                    _ => p
                        .kind
                        .targets()
                        .into_iter()
                        .map(|(class, _, _)| {
                            let tag = format!("{}/{class}", p.id);
                            if log.iter().any(|x| x == &tag) {
                                format!("class {class}: rewritten")
                            } else if log.iter().any(|x| x == &format!("!{tag}")) {
                                format!("class {class}: REFUSED (bytes not as expected) — original kept")
                            } else {
                                format!("class {class}: not loaded yet")
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", "),
                };
                format!("{} {} — {}", p.id, p.title, state)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_well_formed() {
        for (i, p) in PATCHES.iter().enumerate() {
            assert!(p.id.len() == 6 && p.id.starts_with("HZ-") && p.id[3..].chars().all(|c| c.is_ascii_digit()), "{}", p.id);
            assert!(PATCHES[..i].iter().all(|q| q.id != p.id), "duplicate {}", p.id);
            assert!(!p.builds.is_empty(), "{} applies to no build", p.id);
            assert!(!p.title.is_empty() && !p.evidence.is_empty(), "{} undocumented", p.id);
        }
    }

    /// The key is matched by the guest with `String.equals` after `getAppProperty`, so one
    /// wrong character turns the patch into a silent no-op. And the guest reads "false" as
    /// "fail every purchase" — the one value this must never be.
    #[test]
    fn free_cash_uses_the_games_own_switch() {
        let p = PATCHES.iter().find(|p| p.id == "HZ-001").unwrap();
        match p.kind {
            PatchKind::AppProperty { key, value } => {
                assert_eq!(key, "EA-Porting-Test-SMS-Mode");
                assert!(!value.trim().is_empty() && value.trim().to_lowercase() != "false");
            }
            _ => panic!("HZ-001 must stay an app property"),
        }
        let on = Patches::new(Build::Final, &Selection::Default);
        assert_eq!(on.property_overrides(), vec![("EA-Porting-Test-SMS-Mode".to_string(), "true".to_string())]);
        assert!(Patches::new(Build::Final, &Selection::None).property_overrides().is_empty());
    }

    /// HZ-010: each site turns `getstatic ey.f` into a constant true, same length, and refuses
    /// anything else. `ce` touches two methods (synth_class_n).
    #[test]
    fn cheat_shop_sites_read_the_flag_as_true() {
        let mut i_code = vec![0u8; 60];
        i_code[42..48].copy_from_slice(&[0xb2, 0x00, 0x32, 0x99, 0x00, 0x2a]);
        let out = cheat_shop_title(&classfile::tests::synth_class("i", "a", "(II)Z", &i_code)).unwrap();
        let r = classfile::method_code(&out, "a", "(II)Z").unwrap();
        assert_eq!(&out[r][42..48], &[0x04, 0x00, 0x00, 0x99, 0x00, 0x2a]);
        assert_eq!(cheat_shop_title(&out), None, "never applies twice");

        let mut ce_keys = vec![0u8; 844];
        ce_keys[278..281].copy_from_slice(&[0xb2, 0x00, 0x69]);
        let mut ce_f = vec![0u8; 1102];
        ce_f[375..378].copy_from_slice(&[0xb2, 0x00, 0x69]);
        let ce = classfile::tests::synth_class_n("ce", &[("a", "(II)Z", &ce_keys), ("f", "()V", &ce_f)]);
        let out = cheat_shop_field(&ce).unwrap();
        assert_eq!(&out[classfile::method_code(&out, "f", "()V").unwrap()][375..378], &[0x04, 0x00, 0x00]);
        ce_f[376] = 0x01;
        let ce = classfile::tests::synth_class_n("ce", &[("a", "(II)Z", &ce_keys), ("f", "()V", &ce_f)]);
        assert_eq!(cheat_shop_field(&ce), None, "one site off = the whole class refused");
    }

    /// HZ-010's sale sites on synthetic classes with the real bytes (classbytes.py 2026-10-24):
    /// each `getstatic ey.f` reads a constant true and the branch after it is kept; `ek` chains
    /// after HZ-004 (same original class), and one site off refuses the whole class.
    #[test]
    fn cheat_shop_sales_take_the_network_builds_branch() {
        let site = |len: usize, at: usize, bytes: &[u8]| {
            let mut c = vec![0u8; len];
            c[at..at + bytes.len()].copy_from_slice(bytes);
            c
        };
        let mut j = vec![0u8; 207];
        j[32..41].copy_from_slice(&[0x1b, 0x9c, 0x00, 0x08, 0x1b, 0x74, 0xa7, 0x00, 0x04]);
        let death = site(733, 512, &[0xb2, 0x00, 0x9f, 0x99, 0x00, 0x0c]);
        let ek = classfile::tests::synth_class_n("ek", &[("j", "(I)V", &j), ("C", "()V", &death)]);
        let out = cheat_shop_revive(&exp_rate(&ek).unwrap()).expect("after HZ-004");
        assert_eq!(&out[classfile::method_code(&out, "C", "()V").unwrap()][512..518], &[0x04, 0x00, 0x00, 0x99, 0x00, 0x0c]);
        assert_eq!(callee(&out, "j", "(I)V", 33), hook(hooks::EXP), "HZ-004's edit kept");
        let order = |id: &str| PATCHES.iter().position(|p| p.id == id).unwrap();
        assert!(order("HZ-004") < order("HZ-010"));

        let co = classfile::tests::synth_class("co", "a", "(BB)V", &site(532, 138, &[0xb2, 0x00, 0x30, 0x99, 0x00, 0x19]));
        let out = cheat_shop_gold_co(&co).unwrap();
        assert_eq!(&out[classfile::method_code(&out, "a", "(BB)V").unwrap()][138..144], &[0x04, 0x00, 0x00, 0x99, 0x00, 0x19]);

        let et = classfile::tests::synth_class_n(
            "et",
            &[("a", "(BB)V", &site(235, 59, &[0xb2, 0x00, 0x2d, 0x99, 0x00, 0x16])), ("a", "(II)Z", &site(504, 301, &[0xb2, 0x00, 0x2d, 0x99, 0x00, 0x18]))],
        );
        let out = cheat_shop_gold_et(&et).unwrap();
        assert_eq!(out[classfile::method_code(&out, "a", "(BB)V").unwrap()][59], 0x04);
        assert_eq!(out[classfile::method_code(&out, "a", "(II)Z").unwrap()][301], 0x04);

        let mut bi = site(424, 88, &[0xb2, 0x00, 0x27, 0x99, 0x00, 0x22]);
        bi[229..235].copy_from_slice(&[0xb2, 0x00, 0x27, 0x99, 0x00, 0x22]);
        let out = cheat_shop_gold_bi(&classfile::tests::synth_class("bi", "a", "(BB)V", &bi)).unwrap();
        let a = &out[classfile::method_code(&out, "a", "(BB)V").unwrap()];
        assert_eq!((a[88], a[229]), (0x04, 0x04));
        bi[230] = 0x01;
        assert_eq!(cheat_shop_gold_bi(&classfile::tests::synth_class("bi", "a", "(BB)V", &bi)), None, "one site off = refused");

        let dv = classfile::tests::synth_class("dv", "a", "(II)Z", &site(1711, 1100, &[0xb2, 0x00, 0x3a, 0x9a, 0x02, 0x4b]));
        let out = cheat_shop_synthesis(&dv).unwrap();
        let a = &out[classfile::method_code(&out, "a", "(II)Z").unwrap()];
        assert_eq!(&a[1100..1106], &[0x04, 0x00, 0x00, 0x9a, 0x02, 0x4b]);
        assert_eq!(1103 + 0x024b, 1690, "ifne -> the alchemist's own path");
        assert_eq!(cheat_shop_synthesis(&out), None, "never applies twice");
    }

    /// HZ-013 on synthetic classes with the real bytes: the viewport and the windows measure the
    /// whole canvas, the bar painter jumps to its tail, and it chains after HZ-010 on `ce`.
    #[test]
    fn hud_off_frees_the_bottom_rows() {
        let mut init = vec![0u8; 187];
        init[113..119].copy_from_slice(&[0xb2, 0x00, 0x75, 0x10, 0x21, 0x64]);
        let mut keys = vec![0u8; 844];
        keys[278..281].copy_from_slice(&[0xb2, 0x00, 0x69]);
        let mut f = vec![0u8; 1102];
        f[104..110].copy_from_slice(&[0xb8, 0x00, 0xaa, 0xb2, 0x00, 0x72]);
        f[375..378].copy_from_slice(&[0xb2, 0x00, 0x69]);
        let ce = classfile::tests::synth_class_n("ce", &[("<init>", "()V", &init), ("a", "(II)Z", &keys), ("f", "()V", &f)]);
        let out = hud_off_field(&cheat_shop_field(&ce).unwrap()).expect("after HZ-010");
        assert_eq!(&out[classfile::method_code(&out, "<init>", "()V").unwrap()][113..119], &[0xb2, 0x00, 0x75, 0x10, 0x00, 0x64]);
        let f = &out[classfile::method_code(&out, "f", "()V").unwrap()];
        assert_eq!(&f[107..110], &[0xa7, 0x03, 0xd8]);
        assert_eq!(107 + 0x3d8, 1091, "the tail");
        assert_eq!(&f[375..378], &[0x04, 0x00, 0x00], "HZ-010's edit kept");
        assert_eq!(hud_off_field(&out), None, "never applies twice");

        let mut paint = vec![0u8; 914];
        paint[58..64].copy_from_slice(&[0xb2, 0x00, 0x98, 0x10, 0x21, 0x64]);
        paint[533..541].copy_from_slice(&[0xb2, 0x00, 0x98, 0x10, 0x21, 0x64, 0x10, 0x25]);
        let out = hud_off_scenario(&classfile::tests::synth_class("eb", "a", "(Ldd;)V", &paint)).unwrap();
        let a = &out[classfile::method_code(&out, "a", "(Ldd;)V").unwrap()];
        assert_eq!((a[62], a[537], a[540]), (0, 0, 0x25));
        paint[533] = 0x00;
        assert_eq!(hud_off_scenario(&classfile::tests::synth_class("eb", "a", "(Ldd;)V", &paint)), None);

        let mut window = vec![0u8; 485];
        window[157..163].copy_from_slice(&[0xb2, 0x00, 0xa3, 0x10, 0x21, 0x64]);
        let out = hud_off_window(&classfile::tests::synth_class("g", "a", "(Ldd;)V", &window)).unwrap();
        assert_eq!(out[classfile::method_code(&out, "a", "(Ldd;)V").unwrap()][161], 0);
        assert!(PATCHES.iter().find(|p| p.id == "HZ-013").unwrap().default_on, "on: the host HUD replaces the bar (6-4)");
    }

    /// HZ-024 on synthetic classes with the real bytes: both bit tests of the soft-key painter read
    /// 0 and so does `o.c` (the old bar's corner box), and the minimap window's back picture is
    /// skipped — after HZ-013's edit on the same `g` method.
    #[test]
    fn soft_art_off_skips_both_pictures() {
        let mut paint = vec![0u8; 126];
        paint[34..40].copy_from_slice(&[0x1b, 0x04, 0x7e, 0x99, 0x00, 0x13]);
        paint[53..59].copy_from_slice(&[0x1b, 0x05, 0x7e, 0x99, 0x00, 0x14]);
        paint[74..80].copy_from_slice(&[0xb2, 0x00, 0x2d, 0x99, 0x00, 0x1e]);
        let out = soft_art_off(&classfile::tests::synth_class("o", "a", "(Ldd;I)V", &paint)).unwrap();
        let a = &out[classfile::method_code(&out, "a", "(Ldd;I)V").unwrap()];
        assert_eq!((&a[34..40], &a[53..59]), (&[0x03, 0x04, 0x7e, 0x99, 0x00, 0x13][..], &[0x03, 0x05, 0x7e, 0x99, 0x00, 0x14][..]));
        assert_eq!(&a[74..80], &[0x03, 0x00, 0x00, 0x99, 0x00, 0x1e], "o.c reads false");
        paint[53] = 0x1c;
        assert_eq!(soft_art_off(&classfile::tests::synth_class("o", "a", "(Ldd;I)V", &paint)), None, "a byte not as expected");

        let mut window = vec![0u8; 485];
        window[157..163].copy_from_slice(&[0xb2, 0x00, 0xa3, 0x10, 0x21, 0x64]);
        window[423..427].copy_from_slice(&[0xb2, 0x00, 0x9e, 0xc6]);
        let g = classfile::tests::synth_class("g", "a", "(Ldd;)V", &window);
        let out = soft_art_off_map(&hud_off_window(&g).unwrap()).expect("after HZ-013");
        let a = &out[classfile::method_code(&out, "a", "(Ldd;)V").unwrap()];
        assert_eq!(&a[423..427], &[0x01, 0x00, 0x00, 0xc6]);
        assert_eq!(a[161], 0, "HZ-013's edit kept");
        let p = PATCHES.iter().find(|p| p.id == "HZ-024").unwrap();
        assert!(p.default_on);
        assert_eq!(p.kind.targets()[1].1, PATCHES.iter().find(|p| p.id == "HZ-013").unwrap().kind.targets()[1].1, "the same original g");
    }

    /// HZ-012 `bh` on a synthetic class with the real bytes at every site: the switch rotates,
    /// old case 0 becomes the hook, the shortcuts and cursor skips move up one row.
    #[test]
    fn menu_storage_rotates_the_dispatch_and_moves_the_locks() {
        let mut init = vec![0u8; 26];
        init[7..10].copy_from_slice(&[0xb2, 0x00, 0x32]);
        let mut keys = vec![0u8; 280];
        keys[23..27].copy_from_slice(&[0x1c, 0x10, 0x37, 0xa0]);
        keys[49..52].copy_from_slice(&[0xb2, 0x00, 0x32]);
        keys[81..85].copy_from_slice(&[0x1c, 0x10, 0x34, 0xa0]);
        keys[189..196].copy_from_slice(&[0x2a, 0xb4, 0x00, 0x33, 0x10, 0x06, 0xa0]);
        keys[232..235].copy_from_slice(&[0xb2, 0x00, 0x32]);
        keys[262..268].copy_from_slice(&[0x2a, 0xb4, 0x00, 0x33, 0x06, 0xa0]);
        let mut mode = vec![0u8; 94];
        mode[28..36].copy_from_slice(&[0x2a, 0x07, 0xb5, 0x00, 0x33, 0x2a, 0x07, 0xb7]);
        let mut dispatch = vec![0u8; 203];
        dispatch[0..60].copy_from_slice(&[
            0x1b, 0xb3, 0x00, 0x23, 0x1b, 0xaa, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc5, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x09, 0x00, 0x00, 0x00, 0xac, 0x00, 0x00, 0x00, 0x37, 0x00, 0x00, 0x00, 0x44, 0x00, 0x00, 0x00, 0x9f, 0x00, 0x00,
            0x00, 0x52, 0x00, 0x00, 0x00, 0x5f, 0x00, 0x00, 0x00, 0x6c, 0x00, 0x00, 0x00, 0x79, 0x00, 0x00, 0x00, 0x93, 0x00,
            0x00, 0x00, 0x86,
        ]);
        dispatch[177..202].copy_from_slice(&[
            0x2a, 0xbb, 0x00, 0x0a, 0x59, 0x2a, 0xb7, 0x00, 0x3e, 0xb5, 0x00, 0x35, 0xb2, 0x00, 0x31, 0x9a, 0x00, 0x0a, 0x10,
            0x3d, 0x12, 0x03, 0xb8, 0x00, 0x40,
        ]);
        dispatch[202] = 0xb1;
        let bh = classfile::tests::synth_class_n(
            "bh",
            &[("<init>", "()V", &init), ("a", "(II)Z", &keys), ("c", "(B)V", &mode), ("d", "(B)V", &dispatch)],
        );
        let out = menu_storage_bh(&bh).expect("all sites as expected");
        let d = &out[classfile::method_code(&out, "d", "(B)V").unwrap()];
        assert_eq!(d.len(), 203, "same length");
        let target = |k: usize| u32::from_be_bytes(d[20 + 4 * k..24 + 4 * k].try_into().unwrap()) as usize + 5;
        assert_eq!(target(0), 60, "row 0 = old 캐릭터");
        assert_eq!(target(7), 152, "row 7 = old 미니맵");
        assert_eq!(target(8), 177, "row 8 = the old shop block, now the storage hook");
        assert_eq!(target(9), 139, "row 9 = 시스템 as before");
        assert_eq!(&d[..5], &[0x1b, 0xb3, 0x00, 0x23, 0x1b], "bh.f = row is kept");
        assert_eq!((d[177], d[178], d[181]), (0x2a, 0xb8, 0xb1));
        assert!(d[182..202].iter().all(|&b| b == 0));
        assert_eq!(d[202], 0xb1);
        let a = &out[classfile::method_code(&out, "a", "(II)Z").unwrap()];
        assert_eq!((a[25], a[83], a[194], a[266]), (0x36, 0x33, 0x05, 0x05));
        assert_eq!(&a[49..52], &[0x03, 0x00, 0x00]);
        assert_eq!(&a[232..235], &[0x03, 0x00, 0x00]);
        let c = &out[classfile::method_code(&out, "c", "(B)V").unwrap()];
        assert_eq!((c[29], c[34]), (0x06, 0x06));
        assert_eq!(menu_storage_bh(&out), None, "never applies twice");
    }

    /// HZ-012 `m`, `o`, `bg` on synthetic classes: label reads go through the hook with `this`,
    /// the grey rows move up, and bg's RSK becomes the close hook.
    #[test]
    fn menu_storage_labels_and_close_go_through_the_hooks() {
        let mut list = vec![0u8; 694];
        list[323..326].copy_from_slice(&[0xb2, 0x00, 0x20]);
        list[329..333].copy_from_slice(&[0x15, 0x0a, 0x06, 0xa0]);
        list[341..346].copy_from_slice(&[0x15, 0x0a, 0x10, 0x06, 0xa0]);
        list[374..384].copy_from_slice(&[0xb2, 0x00, 0x19, 0x1c, 0x15, 0x0a, 0x60, 0xb6, 0x00, 0x30]);
        list[452..458].copy_from_slice(&[0x15, 0x0a, 0x04, 0xa0, 0x00, 0x37]);
        list[510..523].copy_from_slice(&[0x15, 0x0a, 0x08, 0x9f, 0x00, 0x10, 0x15, 0x0a, 0x10, 0x06, 0xa0, 0x00, 0x3a]);
        list[612..624].copy_from_slice(&[0xb2, 0x00, 0x19, 0x1c, 0x2a, 0xb4, 0x00, 0x21, 0x60, 0xb6, 0x00, 0x30]);
        let out = menu_storage_m(&classfile::tests::synth_class("m", "a", "(Ldd;IIZ)I", &list)).unwrap();
        let m = &out[classfile::method_code(&out, "a", "(Ldd;IIZ)I").unwrap()];
        assert_eq!(&m[323..326], &[0x03, 0x00, 0x00]);
        assert_eq!((m[331], m[344]), (0x05, 0x05));
        assert_eq!((m[454], m[512], m[519]), (0x03, 0x07, 0x05), "the point marks on 캐릭터 0, 스킬 4, 정령 5");
        assert_eq!((&m[455..458], &m[513..516], &m[520..523]), (&[0xa0, 0x00, 0x37][..], &[0x9f, 0x00, 0x10][..], &[0xa0, 0x00, 0x3a][..]), "branches kept");
        assert_eq!(&m[374..377], &[0x2a, 0x00, 0x00]);
        assert_eq!(m[381], 0xb8);
        assert_eq!(&m[612..615], &[0x2a, 0x00, 0x00]);
        assert_eq!(m[621], 0xb8);
        assert_eq!(u16::from_be_bytes([m[382], m[383]]), u16::from_be_bytes([m[622], m[623]]), "one Methodref");

        let mut title = vec![0u8; 34];
        title[4..16].copy_from_slice(&[0x2a, 0x2b, 0xb2, 0x00, 0x1d, 0xb2, 0x00, 0x1e, 0xb6, 0x00, 0x45, 0x15]);
        let out = menu_storage_o(&classfile::tests::synth_class("o", "a", "(Ldd;ZZZZ)V", &title)).unwrap();
        let o = &out[classfile::method_code(&out, "a", "(Ldd;ZZZZ)V").unwrap()];
        assert_eq!(&o[6..9], &[0x2a, 0x00, 0x00]);
        assert_eq!(&o[9..12], &[0xb2, 0x00, 0x1e], "bh.f stays the index");
        assert_eq!(o[12], 0xb8);

        let mut keys = vec![0u8; 98];
        keys[16..28].copy_from_slice(&[0x99, 0x00, 0x0c, 0x2a, 0xb4, 0x00, 0x15, 0xb6, 0x00, 0x26, 0x04, 0xac]);
        let out = menu_storage_bg(&classfile::tests::synth_class("bg", "a", "(II)Z", &keys)).unwrap();
        let g = &out[classfile::method_code(&out, "a", "(II)Z").unwrap()];
        assert_eq!((g[19], g[20], g[23], g[24], g[25], g[26], g[27]), (0x2a, 0xb8, 0, 0, 0, 0x04, 0xac));
    }

    /// HZ-012's children on synthetic classes with the real bytes: closing puts the menu's cursor
    /// one row up (the row the child now sits on), same length, and only once.
    #[test]
    fn menu_children_come_back_to_their_moved_row() {
        let site = |code: &mut Vec<u8>, at: usize, bytes: &[u8]| code[at..at + bytes.len()].copy_from_slice(bytes);
        let cases: [(&str, Rewrite, usize, &[u8], u8); 5] = [
            ("de", menu_row_de, 33, &[0x2a, 0xb4, 0x00, 0x11, 0x06, 0xb5, 0x00, 0x10], 0x05),
            ("em", menu_row_em, 43, &[0x2a, 0xb4, 0x00, 0x26, 0x07, 0xb5, 0x00, 0x24], 0x06),
            ("ad", menu_row_ad, 47, &[0x2a, 0xb4, 0x00, 0x26, 0x08, 0xb5, 0x00, 0x24], 0x07),
            ("cr", menu_row_cr, 38, &[0x2a, 0xb4, 0x00, 0x2b, 0x10, 0x06, 0xb5, 0x00, 0x29], 5),
            ("ao", menu_row_ao, 27, &[0x2a, 0xb4, 0x00, 0x22, 0x10, 0x07, 0xb5, 0x00, 0x20], 6),
        ];
        for (class, rewrite, at, bytes, row) in cases {
            let mut code = vec![0u8; 64];
            site(&mut code, at, bytes);
            let out = rewrite(&classfile::tests::synth_class(class, "a", "(II)Z", &code)).unwrap_or_else(|| panic!("{class}"));
            let a = &out[classfile::method_code(&out, "a", "(II)Z").unwrap()];
            let wide = bytes[4] == 0x10;
            assert_eq!(a.len(), 64, "{class}: same length");
            assert_eq!(a[at + 4 + wide as usize], row, "{class}: one row up");
            assert_eq!(&a[at + 5 + wide as usize..at + bytes.len()], &bytes[5 + wide as usize..], "{class}: the putfield stays");
            assert_eq!(rewrite(&out), None, "{class}: never applies twice");
        }
        let mut code = vec![0u8; 172];
        site(&mut code, 19, &[0x2a, 0xb4, 0x00, 0x19, 0x04, 0xb5, 0x00, 0x17]);
        site(&mut code, 126, &[0x2a, 0xb4, 0x00, 0x19, 0x04, 0xb5, 0x00, 0x17]);
        let out = menu_row_dy(&classfile::tests::synth_class("dy", "a", "(II)Z", &code)).unwrap();
        let a = &out[classfile::method_code(&out, "a", "(II)Z").unwrap()];
        assert_eq!((a[23], a[130]), (0x03, 0x03), "캐릭터 = row 0, both closes");
        code[126] = 0;
        assert_eq!(menu_row_dy(&classfile::tests::synth_class("dy", "a", "(II)Z", &code)), None, "one close off = refused");
    }

    /// HZ-011 on a synthetic `cx`: the ten-byte body becomes the hook call, same length.
    #[test]
    fn text_rename_replaces_the_lookup_body() {
        let code = [0x2a, 0xb4, 0x00, 0x09, 0x1b, 0x32, 0xc0, 0x00, 0x02, 0xb0];
        let out = text_rename(&classfile::tests::synth_class("cx", "a", "(I)[B", &code)).unwrap();
        let r = classfile::method_code(&out, "a", "(I)[B").unwrap();
        let new = &out[r];
        assert_eq!(new.len(), 10);
        assert_eq!((new[0], new[1], new[2], new[5]), (0x2a, 0x1b, 0xb8, 0xb0));
        assert!(new[6..].iter().all(|&b| b == 0));
        assert_eq!(text_rename(&out), None);
    }

    /// HZ-011's help sites on synthetic classes with the real bytes (classbytes.py 2026-10-24):
    /// each `getstatic rpg/d.J` becomes the hook, the operands around it stay, and `de` still
    /// takes HZ-012's cursor edit after it — the transformer runs them in table order.
    #[test]
    fn storage_help_reads_go_through_the_hook() {
        let mut init = vec![0u8; 180];
        init[140..160].copy_from_slice(&[
            0x99, 0x00, 0x1b, 0x2a, 0xbb, 0x00, 0x13, 0x59, 0x2a, 0xb2, 0x00, 0x36, 0x03, 0xb7, 0x00, 0x48, 0xb5, 0x00, 0x33, 0x03,
        ]);
        let out = storage_help_title(&classfile::tests::synth_class("bb", "<init>", "(Lo;B)V", &init)).unwrap();
        assert_eq!(callee(&out, "<init>", "(Lo;B)V", 149), hook(hooks::STORAGE_HELP));
        let code = &out[classfile::method_code(&out, "<init>", "(Lo;B)V").unwrap()];
        assert_eq!((code[148], code[152]), (0x2a, 0x03), "this, then the char 0 for cd's ctor");
        assert_eq!(storage_help_title(&out), None, "never applies twice");

        let mut keys = vec![0u8; 216];
        keys[33..41].copy_from_slice(&[0x2a, 0xb4, 0x00, 0x11, 0x06, 0xb5, 0x00, 0x10]);
        keys[130..146].copy_from_slice(&[0xb2, 0x00, 0x0f, 0x99, 0x00, 0x13, 0xb2, 0x00, 0x14, 0x2c, 0xb8, 0x00, 0x23, 0x4d, 0x03, 0xb3]);
        let de = classfile::tests::synth_class("de", "a", "(II)Z", &keys);
        let out = menu_row_de(&storage_help_game(&de).unwrap()).expect("HZ-012 after HZ-011");
        assert_eq!(callee(&out, "a", "(II)Z", 136), hook(hooks::STORAGE_HELP));
        let code = &out[classfile::method_code(&out, "a", "(II)Z").unwrap()];
        assert_eq!(&code[139..141], &[0x2c, 0xb8], "aload_2; rpg/g.a joins the help to the message");
        assert_eq!(code[37], 0x05, "HZ-012's row kept");
        let order = |id: &str| PATCHES.iter().position(|p| p.id == id).unwrap();
        assert!(order("HZ-011") < order("HZ-012"));
        let de_md5 = |id: &str| PATCHES.iter().find(|p| p.id == id).unwrap().kind.targets().into_iter().find(|t| t.0 == "de").unwrap().1;
        assert_eq!(de_md5("HZ-011"), de_md5("HZ-012"), "the same original de");
    }

    /// A multi-class patch reports every class on its own line part.
    #[test]
    fn a_classes_patch_reports_each_class() {
        let p = PATCHES.iter().find(|p| p.id == "HZ-012").unwrap();
        assert_eq!(p.kind.targets().len(), 10);
        let report = Patches::new(Build::Final, &Selection::Default).report();
        let line = report.iter().find(|l| l.starts_with("HZ-012")).unwrap();
        assert!(line.contains("class bh: not loaded yet") && line.contains("class bg: not loaded yet"), "{line}");
    }

    /// HZ-009 on a synthetic `ep` whose `a(I)Z` has the real bytes at 40..63: the block becomes
    /// one hook call and a return, the SMS path at 63 is untouched, and the length is the same.
    #[test]
    fn cash_free_toggle_replaces_the_property_test_in_place() {
        let mut code = vec![0u8; 80];
        code[40..63].copy_from_slice(&[
            0x2a, 0xb4, 0x00, 0x3a, 0xc6, 0x00, 0x13, 0x2a, 0xb4, 0x00, 0x3a, 0x12, 0x0f, 0xb6, 0x00, 0x4f, 0x99, 0x00, 0x05,
            0x03, 0xac, 0x04, 0xac,
        ]);
        code[63..66].copy_from_slice(&[0x2a, 0x01, 0xb5]); // aload_0; aconst_null; putfield … (the SMS path)
        let class = classfile::tests::synth_class("ep", "a", "(I)Z", &code);
        let out = cash_free_toggle(&class).expect("expected bytes are there");
        let r = classfile::method_code(&out, "a", "(I)Z").unwrap();
        let new = &out[r];
        assert_eq!(new.len(), code.len(), "same code length");
        assert_eq!(new[40], 0xb8, "invokestatic");
        assert_eq!(new[43], 0xac, "ireturn");
        assert!(new[44..63].iter().all(|&b| b == 0), "nops to 63");
        assert_eq!(&new[63..66], &[0x2a, 0x01, 0xb5], "the SMS path is untouched");
        let index = u16::from_be_bytes([new[41], new[42]]);
        assert!(index > 0);
        assert_eq!(cash_free_toggle(&out), None, "never applies twice");
    }

    /// HZ-002 on a synthetic `ay` whose `a(II)Z` has the real bytes at 143..159 (the game's
    /// own class never enters the repository). The jump must land on 198 = the switch exit.
    #[test]
    fn mail_send_seal_jumps_to_the_switch_exit() {
        let mut code = vec![0u8; 207];
        code[143..159].copy_from_slice(&[0x2a, 0xbb, 0x00, 0x10, 0x59, 0x2a, 0x01, 0xb7, 0x00, 0x31, 0xb5, 0x00, 0x24, 0xa7, 0x00, 0x2a]);
        code[198..200].copy_from_slice(&[0x04, 0xac]); // iconst_1; ireturn
        let class = classfile::tests::synth_class("ay", "a", "(II)Z", &code);
        let out = seal_mail_send(&class).expect("expected bytes are there");
        let r = classfile::method_code(&out, "a", "(II)Z").unwrap();
        let new = &out[r];
        assert_eq!(new[143], 0xa7);
        assert_eq!(143 + i16::from_be_bytes([new[144], new[145]]) as usize, 198);
        assert_eq!(&new[198..200], &[0x04, 0xac]);
        assert!(new[146..156].iter().all(|&b| b == 0));
        assert_eq!(&new[156..159], &[0xa7, 0x00, 0x2a], "the original case exit is untouched");
        assert_eq!(seal_mail_send(&out), None, "never applies twice");
    }

    /// HZ-015 on synthetic `bh` / `m` with the real bytes (classbytes.py 2026-10-24): the three
    /// `getstatic bh.h` read a constant true, the branches after them and HZ-012's neighbouring
    /// sites are untouched, and one site off refuses the class.
    #[test]
    fn shop_unlock_reads_the_lock_as_open() {
        let mut keys = vec![0u8; 280];
        keys[75..85].copy_from_slice(&[0xb2, 0x00, 0x28, 0x9a, 0x00, 0x17, 0x1c, 0x10, 0x34, 0xa0]);
        keys[256..268].copy_from_slice(&[0xb2, 0x00, 0x28, 0x9a, 0x00, 0x13, 0x2a, 0xb4, 0x00, 0x33, 0x06, 0xa0]);
        let out = shop_unlock_bh(&classfile::tests::synth_class("bh", "a", "(II)Z", &keys)).unwrap();
        let a = &out[classfile::method_code(&out, "a", "(II)Z").unwrap()];
        assert_eq!(&a[75..85], &[0x04, 0x00, 0x00, 0x9a, 0x00, 0x17, 0x1c, 0x10, 0x34, 0xa0], "HZ-012's digit site kept");
        assert_eq!(&a[256..268], &[0x04, 0x00, 0x00, 0x9a, 0x00, 0x13, 0x2a, 0xb4, 0x00, 0x33, 0x06, 0xa0]);
        assert_eq!(shop_unlock_bh(&out), None, "never applies twice");
        keys[257] = 0x01;
        assert_eq!(shop_unlock_bh(&classfile::tests::synth_class("bh", "a", "(II)Z", &keys)), None, "one site off = refused");

        let mut list = vec![0u8; 694];
        list[329..341].copy_from_slice(&[0x15, 0x0a, 0x06, 0xa0, 0x00, 0x09, 0xb2, 0x00, 0x1b, 0x99, 0x00, 0x10]);
        let out = shop_unlock_m(&classfile::tests::synth_class("m", "a", "(Ldd;IIZ)I", &list)).unwrap();
        let a = &out[classfile::method_code(&out, "a", "(Ldd;IIZ)I").unwrap()];
        assert_eq!(&a[329..341], &[0x15, 0x0a, 0x06, 0xa0, 0x00, 0x09, 0x04, 0x00, 0x00, 0x99, 0x00, 0x10]);
        let order = |id: &str| PATCHES.iter().position(|p| p.id == id).unwrap();
        assert!(order("HZ-012") < order("HZ-015"), "after the menu's rotation on the same classes");
    }

    /// HZ-026 on synthetic `bh` / `m` / `ec` with the real bytes (classbytes.py 2026-10-27): nine rows,
    /// the switch re-rotated from HZ-012's, '0' and the quick save on row 8, '9' out of the digits,
    /// 「0.」 on row 8; refused on the game's own menu (HZ-012 off).
    #[test]
    fn menu_without_minimap_has_nine_rows() {
        let mut init = vec![0u8; 26];
        init[0..6].copy_from_slice(&[0x2a, 0x01, 0x10, 0x0a, 0xb7, 0x00]);
        let mut keys = vec![0u8; 280];
        keys[11..23].copy_from_slice(&[0x1c, 0x10, 0x30, 0xa1, 0x00, 0x7a, 0x1c, 0x10, 0x39, 0xa3, 0x00, 0x74]);
        keys[101..113].copy_from_slice(&[0x1c, 0x10, 0x30, 0xa0, 0x00, 0x0c, 0x2a, 0x10, 0x09, 0xb5, 0x00, 0x33]);
        let mut mode = vec![0u8; 94];
        mode[47..59].copy_from_slice(&[0x2a, 0x10, 0x09, 0xb5, 0x00, 0x33, 0x2a, 0x10, 0x09, 0xb7, 0x00, 0x3d]);
        let mut dispatch = vec![0u8; 203];
        let mut bh = |switch: [u8; 60]| {
            dispatch[..60].copy_from_slice(&switch);
            classfile::tests::synth_class_n("bh", &[("<init>", "()V", &init), ("a", "(II)Z", &keys), ("c", "(B)V", &mode), ("d", "(B)V", &dispatch)])
        };
        assert_eq!(menu_no_minimap_bh(&bh(MENU_SWITCH)), None, "the game's own switch: HZ-012 is off");
        let out = menu_no_minimap_bh(&bh(menu_switch(MENU_ROWS_HZ012))).expect("on HZ-012's menu");
        assert_eq!(out[classfile::method_code(&out, "<init>", "()V").unwrap()][3], 9, "nine rows");
        let a = &out[classfile::method_code(&out, "a", "(II)Z").unwrap()];
        assert_eq!((a[19], a[109]), (0x38, 0x08), "digits '0'..'8', '0' = row 8");
        let c = &out[classfile::method_code(&out, "c", "(B)V").unwrap()];
        assert_eq!((c[49], c[55]), (0x08, 0x08), "the quick save's 시스템 = row 8");
        let d = &out[classfile::method_code(&out, "d", "(B)V").unwrap()];
        let target = |k: usize| u32::from_be_bytes(d[20 + 4 * k..24 + 4 * k].try_into().unwrap()) as usize + 5;
        assert_eq!((0..10).map(target).collect::<Vec<_>>(), [60, 73, 164, 87, 100, 113, 126, 177, 139, 152], "… 퀘스트, 계정창고, 시스템, (미니맵)");
        assert_eq!(menu_no_minimap_bh(&out), None, "never applies twice");

        let mut list = vec![0u8; 694];
        list[374..377].copy_from_slice(&[0xb2, 0x00, 0x19]);
        list[413..418].copy_from_slice(&[0x15, 0x0a, 0x10, 0x09, 0xa0]);
        list[653..660].copy_from_slice(&[0x2a, 0xb4, 0x00, 0x21, 0x10, 0x09, 0xa0]);
        assert_eq!(menu_no_minimap_m(&classfile::tests::synth_class("m", "a", "(Ldd;IIZ)I", &list)), None, "no HZ-012 label site");
        list[374..377].copy_from_slice(&[0x2a, 0x00, 0x00]);
        let out = menu_no_minimap_m(&classfile::tests::synth_class("m", "a", "(Ldd;IIZ)I", &list)).unwrap();
        let m = &out[classfile::method_code(&out, "a", "(Ldd;IIZ)I").unwrap()];
        assert_eq!((m[416], m[658]), (0x08, 0x08), "「0.」 on row 8");

        let mut close = vec![0u8; 97];
        close[57..73].copy_from_slice(&[0x2a, 0xb4, 0x00, 0x15, 0x10, 0x09, 0xb5, 0x00, 0x14, 0x2a, 0xb4, 0x00, 0x15, 0xb6, 0x00, 0x26]);
        let out = menu_no_minimap_ec(&classfile::tests::synth_class("ec", "a", "(II)Z", &close)).unwrap();
        let e = &out[classfile::method_code(&out, "a", "(II)Z").unwrap()];
        assert!(e[57..66].iter().all(|&b| b == 0), "the cursor is left where it is");
        assert_eq!(&e[66..73], &[0x2a, 0xb4, 0x00, 0x15, 0xb6, 0x00, 0x26], "the close itself kept");

        let p = PATCHES.iter().find(|p| p.id == "HZ-026").unwrap();
        let md5 = |id: &str, class: &str| PATCHES.iter().find(|p| p.id == id).unwrap().kind.targets().into_iter().find(|t| t.0 == class).unwrap().1;
        assert_eq!((p.kind.targets()[0].1, p.kind.targets()[1].1), (md5("HZ-012", "bh"), md5("HZ-012", "m")), "the same original classes");
        let order = |id: &str| PATCHES.iter().position(|p| p.id == id).unwrap();
        assert!(order("HZ-012") < order("HZ-026") && order("HZ-015") < order("HZ-026"));
    }

    /// HZ-027 on a synthetic `bg` with the real bytes: the title's offset from the centre 64 -> 82.
    #[test]
    fn storage_title_moves_above_its_plates() {
        let mut paint = vec![0u8; 151];
        paint[11..21].copy_from_slice(&[0x2b, 0xb2, 0x00, 0x18, 0xb2, 0x00, 0x19, 0x10, 0x40, 0x64]);
        let out = storage_title_clear(&classfile::tests::synth_class("bg", "a", "(Ldd;)V", &paint)).unwrap();
        let g = &out[classfile::method_code(&out, "a", "(Ldd;)V").unwrap()];
        assert_eq!(&g[17..21], &[0x19, 0x10, 82, 0x64], "getstatic rpg/a.d; bipush 82; isub");
        assert_eq!(storage_title_clear(&out), None, "never applies twice");
        let md5 = |id: &str| PATCHES.iter().find(|p| p.id == id).unwrap().kind.targets().into_iter().find(|t| t.0 == "bg").unwrap().1;
        assert_eq!(md5("HZ-027"), md5("HZ-012"), "the same original bg");
    }

    /// HZ-028 on a synthetic `dd` with the real bytes (classbytes.py 2026-10-27): each loop's per-pixel
    /// body becomes its hook, the loops' heads and jumps back as they were.
    #[test]
    fn translucent_draws_go_through_the_hooks() {
        let mut fill = vec![0u8; 86];
        fill[43..68].copy_from_slice(&[
            0x19, 0x05, 0xbe, 0x04, 0x64, 0x36, 0x07, 0x15, 0x07, 0x9b, 0x00, 0x10, 0x19, 0x05, 0x15, 0x07, 0x15, 0x06, 0x4f, 0x84, 0x07, 0xff,
            0xa7, 0xff, 0xf1,
        ]);
        let mut picture = vec![0u8; 139];
        picture[71..108].copy_from_slice(&[
            0x19, 0x07, 0xbe, 0x04, 0x64, 0x3c, 0x1b, 0x9b, 0x00, 0x1e, 0x19, 0x07, 0x1b, 0x2e, 0x10, 0x18, 0x7c, 0x99, 0x00, 0x0e, 0x19, 0x07,
            0x1b, 0x5c, 0x2e, 0x2a, 0xb4, 0x00, 0x0c, 0x7e, 0x4f, 0x84, 0x01, 0xff, 0xa7, 0xff, 0xe4,
        ]);
        let dd = classfile::tests::synth_class_n("dd", &[("a", "(IIII)V", &fill), ("a", "(Lcw;III)V", &picture)]);
        let out = translucent_fill(&dd).unwrap();

        let c = &out[classfile::method_code(&out, "a", "(IIII)V").unwrap()];
        assert_eq!(&c[43..55], &fill[43..55], "the head and the test");
        assert_eq!(&c[55..59], &[0x19, 0x05, 0x15, 0x06], "the array, the colour");
        assert_eq!(callee(&out, "a", "(IIII)V", 59), hook(hooks::FILL_INTS));
        assert_eq!(&c[62..65], &[0x36, 0x07, 0x00], "istore 7 (the index), nop");
        assert_eq!(&c[65..68], &[0xa7, 0xff, 0xf1], "goto 50, as it was");

        let p = &out[classfile::method_code(&out, "a", "(Lcw;III)V").unwrap()];
        assert_eq!(&p[71..81], &picture[71..81], "the head and the test");
        assert_eq!(&p[81..87], &[0x19, 0x07, 0x2a, 0xb4, 0x00, 0x0c], "the pixels, the mask dd.a:I");
        assert_eq!(callee(&out, "a", "(Lcw;III)V", 87), hook(hooks::MASK_ALPHA));
        assert_eq!(p[90], 0x3c, "istore_1 (the index)");
        assert!(p[91..102].iter().all(|&b| b == 0), "the rest of the body gone");
        assert_eq!(&p[102..108], &picture[102..108], "iinc 1, -1 (the branch target) and goto 77, as they were");

        assert_eq!(translucent_fill(&out), None, "never applies twice");
        assert_eq!(translucent_fill(&classfile::tests::synth_class("dd", "a", "(IIII)V", &fill)), None, "both or nothing");
    }

    /// HZ-023 on synthetic classes with the real bytes (classbytes.py 2026-10-25): the factory's read
    /// becomes the tier's hook alone, or the orb-and-tier hook over HZ-016's.
    #[test]
    fn gear_records_serve_the_tier_alone_or_with_the_orb() {
        let mut factory = vec![0u8; 159];
        factory[104..112].copy_from_slice(&[0x00, 0x40, 0x1b, 0xb8, 0x00, 0x44, 0x59, 0x4c]);
        let ba = classfile::tests::synth_class("ba", "a", "(BB)Lba;", &factory);
        let alone = gear_records(&ba).expect("the game's read");
        assert_eq!(callee(&alone, "a", "(BB)Lba;", 107), hook(hooks::GEAR_RECORD));
        assert_eq!(gear_records(&alone), None, "never applies twice");

        let quick = [
            0x2a, 0xb4, 0x00, 0x17, 0x10, 0x14, 0xa0, 0x00, 0x0d, 0x2a, 0xb4, 0x00, 0x1c, 0x05, 0xa1, 0x00, 0x05, 0x04, 0xac, 0x03,
            0xac,
        ];
        let mut draw = vec![0u8; 99];
        draw[72..84].copy_from_slice(&[0x60, 0x36, 0x04, 0x2b, 0xb2, 0x00, 0x21, 0x15, 0x04, 0x32, 0x1c, 0x10]);
        let ba = classfile::tests::synth_class_n("ba", &[("a", "(BB)Lba;", &factory), ("a", "(Ldd;IIZZ)V", &draw), ("a", "()Z", &quick)]);
        let orb = revive_orb_item(&ba).unwrap();
        let both = gear_records(&orb).expect("after HZ-016");
        assert_eq!(callee(&both, "a", "(BB)Lba;", 107), hook(hooks::ORB_GEAR_RECORD));
        assert_eq!(callee(&both, "a", "(Ldd;IIZZ)V", 78), hook(hooks::ITEM_ICON), "HZ-016's other edits kept");
        assert_eq!(gear_records(&both), None, "never applies twice");
        let order = |id: &str| PATCHES.iter().position(|p| p.id == id).unwrap();
        assert!(order("HZ-016") < order("HZ-023"), "the chain needs HZ-016 first");

        // Any other invokestatic at the site is not touched.
        let mut other = factory.clone();
        other[107..110].copy_from_slice(&[0xb8, 0x00, 0x07]);
        assert_eq!(gear_records(&classfile::tests::synth_class("ba", "a", "(BB)Lba;", &other)), None);
    }

    #[test]
    fn gear_drop_replaces_the_rare_bands_min() {
        let mut code = vec![0u8; 536];
        // @205: istore 4; iload_2; iconst_5; idiv; bipush 13; iadd; bipush 28; invokestatic #259; i2b
        code[205..219].copy_from_slice(&[0x36, 0x04, 0x1c, 0x08, 0x6c, 0x10, 0x0d, 0x60, 0x10, 0x1c, 0xb8, 0x01, 0x03, 0x91]);
        code[84..90].copy_from_slice(&[0x68, 0x05, 0x60, 0xb8, 0x01, 0x1a]);
        let cq = classfile::tests::synth_class("cq", "o", "()V", &code);
        let out = gear_drop(&drop_rate(&cq).unwrap()).expect("after HZ-005");
        assert_eq!(callee(&out, "o", "()V", 215), hook(hooks::RARE_DROP));
        assert_eq!(callee(&out, "o", "()V", 87), hook(hooks::DROP_THRESHOLD), "HZ-005's edit kept");
        assert_eq!(code[214] as i32, hooks::RARE_TOP);
        assert_eq!(gear_drop(&out), None, "never applies twice");
        assert!((0..hooks::GEAR_CATEGORIES).contains(&15) && hooks::GEAR_ID > 32);

        let ground = [
            0x1d, 0x9b, 0x00, 0x15, 0x15, 0x04, 0x9b, 0x00, 0x10, 0x1d, 0x10, 0x14, 0xa3, 0x00, 0x0a, 0x15, 0x04, 0x10, 0x1c, 0xa4, 0x00,
            0x04, 0xb1, 0x10,
        ];
        let out = gear_ground(&classfile::tests::synth_class("g", "a", "(BBBB)V", &ground)).unwrap();
        let c = &out[classfile::method_code(&out, "a", "(BBBB)V").unwrap()];
        assert_eq!((c[17], c[18]), (0x10, 40), "bipush 40");
        assert_eq!(&c[..17], &ground[..17]);
        assert_eq!(&c[19..], &ground[19..]);
        assert_eq!(gear_ground(&out), None, "never applies twice");
    }

    /// HZ-016 on synthetic classes with the real bytes (classbytes.py 2026-10-24): every site calls
    /// its hook with the same stack effect, the death takes either the original flag test or
    /// HZ-010's constant, the old purchase becomes a return, and nothing applies twice.
    #[test]
    fn revive_orb_sites_call_their_hooks() {
        let site = |len: usize, at: usize, bytes: &[u8]| {
            let mut c = vec![0u8; len];
            c[at..at + bytes.len()].copy_from_slice(bytes);
            c
        };
        let factory = site(159, 104, &[0x00, 0x40, 0x1b, 0xb8, 0x00, 0x44, 0x59, 0x4c]);
        let draw = site(99, 72, &[0x60, 0x36, 0x04, 0x2b, 0xb2, 0x00, 0x21, 0x15, 0x04, 0x32, 0x1c, 0x10]);
        let quick = [
            0x2a, 0xb4, 0x00, 0x17, 0x10, 0x14, 0xa0, 0x00, 0x0d, 0x2a, 0xb4, 0x00, 0x1c, 0x05, 0xa1, 0x00, 0x05, 0x04, 0xac, 0x03,
            0xac,
        ];
        let ba = classfile::tests::synth_class_n("ba", &[("a", "(BB)Lba;", &factory), ("a", "(Ldd;IIZZ)V", &draw), ("a", "()Z", &quick)]);
        let out = revive_orb_item(&ba).unwrap();
        assert_eq!(callee(&out, "a", "(BB)Lba;", 107), hook(hooks::ITEM_RECORD));
        let d = &out[classfile::method_code(&out, "a", "(Ldd;IIZZ)V").unwrap()];
        assert_eq!((d[75], &d[76..78], d[81], &d[82..84]), (0x2b, &[0x15, 0x04][..], 0x00, &[0x1c, 0x10][..]), "iload 4 first, nop, then as before");
        assert_eq!(callee(&out, "a", "(Ldd;IIZZ)V", 78), hook(hooks::ITEM_ICON));
        let q = &out[classfile::method_code(&out, "a", "()Z").unwrap()];
        assert_eq!((q[0], q[4]), (0x2a, 0xac), "aload_0 … ireturn");
        assert_eq!(callee(&out, "a", "()Z", 1), hook(hooks::QUICK_ITEM));
        assert!(q[5..].iter().all(|&b| b == 0));
        assert_eq!(revive_orb_item(&out), None, "never applies twice");

        let co = classfile::tests::synth_class("co", "h", "()V", &site(421, 312, &[0x9a, 0x00, 0x0b, 0x10, 0x14, 0x10, 0x08, 0xb8, 0x00, 0x4a]));
        let out = revive_orb_shop(&co).unwrap();
        assert_eq!(&out[classfile::method_code(&out, "h", "()V").unwrap()][312..322], &[0x9a, 0x00, 0x0b, 0x10, 0x14, 0x10, 0x09, 0xb8, 0x00, 0x4a]);

        let purchase = [
            0x08, 0xb3, 0x00, 0x59, 0x04, 0xb3, 0x00, 0x76, 0xb8, 0x00, 0xd3, 0xb6, 0x00, 0xd4, 0xb8, 0x00, 0xd3, 0x06, 0xb6, 0x00,
            0xd5, 0xb8, 0x00, 0xd3, 0xb4, 0x00, 0xa3, 0xc0, 0x00, 0x16, 0x59, 0x4c, 0xb6, 0x00, 0xe1, 0xb1,
        ];
        let mut death = site(733, 512, &[0xb2, 0x00, 0x9f, 0x99, 0x00, 0x0c, 0xb8, 0x00, 0xdd, 0x10, 0x09, 0xb8, 0x00, 0xf8, 0xb1]);
        death[527..563].copy_from_slice(&purchase);
        death[563] = 0x2a;
        let mut j = vec![0u8; 207];
        j[32..41].copy_from_slice(&[0x1b, 0x9c, 0x00, 0x08, 0x1b, 0x74, 0xa7, 0x00, 0x04]);
        let ek = classfile::tests::synth_class_n("ek", &[("j", "(I)V", &j), ("C", "()V", &death)]);
        let chained = revive_orb_death(&cheat_shop_revive(&exp_rate(&ek).unwrap()).unwrap()).expect("after HZ-004 and HZ-010");
        let alone = revive_orb_death(&ek).expect("without HZ-010");
        for out in [&chained, &alone] {
            assert_eq!(callee(out, "C", "()V", 512), hook(hooks::DEATH));
            let c = &out[classfile::method_code(out, "C", "()V").unwrap()];
            assert_eq!(&c[515..527], &[0x99, 0x00, 0x0c, 0xb8, 0x00, 0xdd, 0x10, 0x09, 0xb8, 0x00, 0xf8, 0xb1], "true = the game over, as before");
            assert_eq!(515 + 0x0c, 527);
            assert_eq!(c[527], 0xb1, "false = return: the question is open");
            assert!(c[528..563].iter().all(|&b| b == 0));
            assert_eq!(c[563], 0x2a, "state 7's code after it untouched");
            assert_eq!(revive_orb_death(out), None, "never applies twice");
        }
        assert_eq!(callee(&chained, "j", "(I)V", 33), hook(hooks::EXP), "HZ-004's edit kept");
        let order = |id: &str| PATCHES.iter().position(|p| p.id == id).unwrap();
        assert!(order("HZ-004") < order("HZ-010") && order("HZ-010") < order("HZ-016"));

        let mut keys = site(94, 55, &[0x2a, 0xb4, 0x00, 0x1d, 0xb6, 0x00, 0x30, 0xa7, 0x00, 0x1e]);
        keys[80..94].copy_from_slice(&[0x2a, 0xb4, 0x00, 0x1e, 0x2a, 0xb4, 0x00, 0x18, 0x04, 0xb6, 0x00, 0x30, 0x04, 0xac]);
        let out = revive_orb_answer(&classfile::tests::synth_class("cf", "a", "(II)Z", &keys)).unwrap();
        assert_eq!(callee(&out, "a", "(II)Z", 59), hook(hooks::ANSWER));
        assert_eq!(callee(&out, "a", "(II)Z", 89), hook(hooks::ANSWER));
        assert_eq!(revive_orb_answer(&out), None, "never applies twice");
        assert_eq!(hooks::REVIVE_ORB_ICON, 29);
    }

    /// HZ-017 / HZ-018 on a synthetic `cq` with the real bytes (classbytes.py 2026-10-25): both sites
    /// call their hooks, the neighbours and HZ-005's drop read are kept, and nothing applies twice.
    #[test]
    fn monster_sites_call_their_hooks() {
        let mut init = vec![0u8; 257];
        init[204..216].copy_from_slice(&[0xb5, 0x00, 0x4c, 0x2a, 0xb6, 0x00, 0xdf, 0x2a, 0x01, 0xb5, 0x00, 0x3d]);
        let mut death = vec![0u8; 45];
        death[6..21].copy_from_slice(&[0x2a, 0x2a, 0xb4, 0x00, 0x3e, 0xb4, 0x00, 0x69, 0x3d, 0x59, 0x4c, 0x1c, 0xb5, 0x00, 0x42]);
        let mut drop = vec![0u8; 120];
        drop[87..90].copy_from_slice(&[0xb8, 0x01, 0x1a]);
        let cq = classfile::tests::synth_class_n("cq", &[("o", "()V", &drop), ("i", "()V", &init), ("g", "()V", &death)]);
        let out = monster_respawn(&monster_difficulty(&drop_rate(&cq).unwrap()).unwrap()).expect("after HZ-005");
        assert_eq!(callee(&out, "o", "()V", 87), hook(hooks::DROP_THRESHOLD), "HZ-005's edit kept");
        assert_eq!(callee(&out, "i", "()V", 208), hook(hooks::MONSTER_INIT));
        let i = &out[classfile::method_code(&out, "i", "()V").unwrap()];
        assert_eq!((&i[204..208], &i[211..216]), (&[0xb5, 0x00, 0x4c, 0x2a][..], &[0x2a, 0x01, 0xb5, 0x00, 0x3d][..]));
        assert_eq!(callee(&out, "g", "()V", 11), hook(hooks::RESPAWN_DELAY));
        let g = &out[classfile::method_code(&out, "g", "()V").unwrap()];
        assert_eq!((&g[6..11], &g[14..21]), (&[0x2a, 0x2a, 0xb4, 0x00, 0x3e][..], &[0x3d, 0x59, 0x4c, 0x1c, 0xb5, 0x00, 0x42][..]));
        assert_eq!((monster_difficulty(&out), monster_respawn(&out)), (None, None), "never applies twice");
        let order = |id: &str| PATCHES.iter().position(|p| p.id == id).unwrap();
        assert!(order("HZ-005") < order("HZ-017") && order("HZ-017") < order("HZ-018"));
    }

    /// HZ-030 on synthetic `ed` / `cq` / `eb` / `ce` with the real bytes (classbytes.py 2026-10-28): the
    /// ending's cap is 3, the monster setup calls its hooks (after HZ-017's, which stays), the quest
    /// money and EXP and the ending's line go through theirs, and nothing applies twice.
    #[test]
    fn guardian_mode_sites_call_their_hooks() {
        let mut ending = vec![0u8; 925];
        ending[426..446].copy_from_slice(&[0x05, 0xa0, 0x00, 0x5d, 0xb2, 0x00, 0x6d, 0x05, 0xa2, 0x00, 0x0c, 0xb2, 0x00, 0x6d, 0x04, 0x60, 0x91, 0xb3, 0x00, 0x6d]);
        let ed = classfile::tests::synth_class("ed", "a", "()V", &ending);
        let out = guardian_round_cap(&ed).unwrap();
        let a = &out[classfile::method_code(&out, "a", "()V").unwrap()];
        assert_eq!(&a[426..446], &[0x05, 0xa0, 0x00, 0x5d, 0xb2, 0x00, 0x6d, 0x06, 0xa2, 0x00, 0x0c, 0xb2, 0x00, 0x6d, 0x04, 0x60, 0x91, 0xb3, 0x00, 0x6d]);
        assert_eq!(guardian_round_cap(&out), None, "never applies twice");

        let mut init = vec![0u8; 257];
        init[28..52].copy_from_slice(&[
            0x00, 0x83, 0x2a, 0xb6, 0x00, 0xde, 0x2a, 0x4c, 0xb2, 0x00, 0x64, 0x05, 0xa4, 0x00, 0x07, 0x05, 0xa7, 0x00, 0x06, 0xb2, 0x00, 0x64, 0x59, 0x3d,
        ]);
        init[204..216].copy_from_slice(&[0xb5, 0x00, 0x4c, 0x2a, 0xb6, 0x00, 0xdf, 0x2a, 0x01, 0xb5, 0x00, 0x3d]);
        let cq = classfile::tests::synth_class("cq", "i", "()V", &init);
        let out = guardian_monsters(&monster_difficulty(&cq).unwrap()).expect("after HZ-017");
        assert_eq!(callee(&out, "i", "()V", 31), hook(hooks::GUARDIAN_MONSTER));
        assert_eq!(callee(&out, "i", "()V", 36), hook(hooks::MONSTER_ROUND));
        assert_eq!(callee(&out, "i", "()V", 47), hook(hooks::MONSTER_ROUND));
        assert_eq!(callee(&out, "i", "()V", 208), hook(hooks::MONSTER_INIT), "HZ-017's edit kept");
        let i = &out[classfile::method_code(&out, "i", "()V").unwrap()];
        assert_eq!((&i[28..31], &i[34..36], &i[39..47], &i[50..52]), (&init[28..31], &init[34..36], &init[39..47], &init[50..52]));
        assert!(guardian_monsters(&cq).is_some(), "alone too");
        assert_eq!(guardian_monsters(&out), None, "never applies twice");

        let mut script = vec![0u8; 3053];
        script[1196..1212].copy_from_slice(&[0x2c, 0xb6, 0x01, 0x40, 0x57, 0x84, 0x03, 0x01, 0xa7, 0xff, 0xd9, 0x1b, 0xb8, 0x01, 0x0e, 0xa7]);
        script[1448..1462].copy_from_slice(&[0x84, 0x03, 0x01, 0xa7, 0xff, 0xd9, 0xb2, 0x00, 0x4c, 0x1b, 0xb6, 0x01, 0x17, 0xa7]);
        let eb = classfile::tests::synth_class("eb", "b", "(BLn;)Z", &script);
        let out = guardian_rewards(&eb).unwrap();
        assert_eq!(callee(&out, "b", "(BLn;)Z", 1208), hook(hooks::QUEST_MONEY));
        assert_eq!(callee(&out, "b", "(BLn;)Z", 1458), hook(hooks::QUEST_EXP));
        let b = &out[classfile::method_code(&out, "b", "(BLn;)Z").unwrap()];
        assert_eq!((&b[1196..1208], &b[1211], &b[1448..1458], &b[1461]), (&script[1196..1208], &0xa7, &script[1448..1458], &0xa7));
        assert_eq!(guardian_rewards(&out), None, "never applies twice");
        let mut reward = script.clone();
        reward[1005..1049].copy_from_slice(&[
            0xb2, 0x00, 0x5a, 0x1d, 0x06, 0x68, 0x33, 0xb2, 0x00, 0x5a, 0x1d, 0x06, 0x68, 0x04, 0x60, 0x33, 0xb8, 0x00, 0xb4, 0x4c, 0xb2, 0x00, 0x5a, 0x1d,
            0x06, 0x68, 0x33, 0xb8, 0x01, 0x02, 0x2b, 0xb2, 0x00, 0x5a, 0x1d, 0x06, 0x68, 0x05, 0x60, 0x33, 0xb6, 0x00, 0xbe, 0x59,
        ]);
        let eb = classfile::tests::synth_class("eb", "b", "(BLn;)Z", &reward);
        let out = guardian_quest_items(&guardian_rewards(&eb).unwrap()).expect("after HZ-030");
        assert_eq!(callee(&out, "b", "(BLn;)Z", 1021), hook(hooks::QUEST_ITEM));
        assert_eq!(callee(&out, "b", "(BLn;)Z", 1045), hook(hooks::QUEST_GIVE));
        let code = &out[classfile::method_code(&out, "b", "(BLn;)Z").unwrap()];
        assert_eq!(&code[1032..1035], &[0xb8, 0x01, 0x02], "the bag's ed.a(B)Lbt; kept");
        assert_eq!(callee(&out, "b", "(BLn;)Z", 1208), hook(hooks::QUEST_MONEY), "HZ-030's kept");
        assert_eq!(guardian_quest_items(&out), None, "never applies twice");

        let mut paint = vec![0u8; 932];
        paint[878..902].copy_from_slice(&[
            0x10, 0x14, 0x64, 0xb2, 0x00, 0x80, 0xb2, 0x00, 0x64, 0x9e, 0x00, 0x08, 0x10, 0x22, 0xa7, 0x00, 0x05, 0x10, 0x21, 0xb6, 0x00, 0xb5, 0x03, 0x04,
        ]);
        let ce = classfile::tests::synth_class("ce", "d", "(Ldd;)V", &paint);
        let out = guardian_ending_text(&ce).unwrap();
        assert_eq!(callee(&out, "d", "(Ldd;)V", 897), hook(hooks::ROUND_TEXT));
        let d = &out[classfile::method_code(&out, "d", "(Ldd;)V").unwrap()];
        assert_eq!((&d[878..897], &d[900..902]), (&paint[878..897], &paint[900..902]));
        assert_eq!(guardian_ending_text(&out), None, "never applies twice");

        assert_eq!(hooks::GUARDIAN_ROUND, 3);
        let order = |id: &str| PATCHES.iter().position(|p| p.id == id).unwrap();
        assert!(order("HZ-017") < order("HZ-030") && order("HZ-023") < order("HZ-030"));
    }

    /// HZ-031 on synthetic `ba` / `g` / `q` / `cz` / `cq` with the real bytes (classbytes.py 2026-10-28):
    /// the record read gets a hook only when none is there, the ground limit is 50 after the game's 28 or
    /// HZ-023's 40, the grade colour / name and the item window and the drop call theirs.
    #[test]
    fn legend_sites_call_their_hooks() {
        let mut factory = vec![0u8; 159];
        factory[104..112].copy_from_slice(&[0x00, 0x40, 0x1b, 0xb8, 0x00, 0x44, 0x59, 0x4c]);
        let ba = classfile::tests::synth_class("ba", "a", "(BB)Lba;", &factory);
        let alone = legend_records(&ba).expect("the game's read");
        assert_eq!(callee(&alone, "a", "(BB)Lba;", 107), hook(hooks::LEGEND_RECORD));
        let geared = gear_records(&ba).unwrap();
        assert_eq!(legend_records(&geared), Some(geared.clone()), "HZ-023's hook stays and serves them");
        assert_eq!(legend_records(&alone), None, "never applies twice");

        let mut ground = vec![0u8; 60];
        ground[12..20].copy_from_slice(&[0xa3, 0x00, 0x0a, 0x15, 0x04, 0x10, 0x1c, 0xa4]);
        let mut notice = vec![0u8; 304];
        notice[212..226].copy_from_slice(&[0x19, 0x06, 0xb2, 0x00, 0x9f, 0x2b, 0xc0, 0x00, 0x2b, 0xb4, 0x00, 0x54, 0x2e, 0xb6]);
        let g = classfile::tests::synth_class_n("g", &[("a", "(BBBB)V", &ground), ("a", "(BBBZ)V", &notice)]);
        for g in [g.clone(), gear_ground(&g).unwrap()] {
            let out = legend_ground(&g).unwrap();
            let a = &out[classfile::method_code(&out, "a", "(BBBB)V").unwrap()];
            assert_eq!(&a[15..20], &[0x15, 0x04, 0x10, 50, 0xa4]);
            assert_eq!(callee(&out, "a", "(BBBZ)V", 221), hook(hooks::GRADE_COLOR));
            let n = &out[classfile::method_code(&out, "a", "(BBBZ)V").unwrap()];
            assert_eq!((&n[212..221], n[224], n[225]), (&notice[212..221], 0x00, 0xb6));
            assert_eq!(legend_ground(&out), None, "never applies twice");
        }

        let mut panel = vec![0u8; 48];
        panel[5..19].copy_from_slice(&[0x2a, 0xb2, 0x00, 0x38, 0x2d, 0xc0, 0x00, 0x17, 0xb4, 0x00, 0x30, 0x2e, 0xb6, 0x00]);
        let mut window = vec![0u8; 561];
        window[99..109].copy_from_slice(&[0xb2, 0x00, 0x3c, 0x15, 0x08, 0xb6, 0x00, 0x48, 0x03, 0x10]);
        let q = classfile::tests::synth_class_n("q", &[("b", "(Ldd;IILba;Z)V", &panel), ("b", "(Ldd;IILba;)V", &window)]);
        let out = legend_grade(&q).unwrap();
        assert_eq!(callee(&out, "b", "(Ldd;IILba;Z)V", 13), hook(hooks::GRADE_COLOR));
        assert_eq!(callee(&out, "b", "(Ldd;IILba;)V", 104), hook(hooks::GRADE_NAME));
        assert_eq!(legend_grade(&out), None, "never applies twice");

        let mut info = vec![0u8; 39];
        info[30..39].copy_from_slice(&[0x60, 0x2a, 0xb4, 0x00, 0x06, 0xb6, 0x00, 0x12, 0xb1]);
        let cz = classfile::tests::synth_class("cz", "a", "(Ldd;)V", &info);
        let out = legend_info(&cz).unwrap();
        assert_eq!(callee(&out, "a", "(Ldd;)V", 35), hook(hooks::ITEM_INFO));
        assert_eq!(legend_info(&out), None, "never applies twice");

        let mut death = vec![0u8; 45];
        death[6..34].copy_from_slice(&[
            0x2a, 0x2a, 0xb4, 0x00, 0x3e, 0xb4, 0x00, 0x69, 0x3d, 0x59, 0x4c, 0x1c, 0xb5, 0x00, 0x42, 0x2a, 0xb6, 0x00, 0xdd, 0x2a, 0xb6, 0x00, 0xdc, 0x2a,
            0xb6, 0x00, 0xdb, 0x2a,
        ]);
        let cq = classfile::tests::synth_class("cq", "g", "()V", &death);
        let out = legend_drop(&monster_respawn(&cq).unwrap()).expect("after HZ-018");
        assert_eq!(callee(&out, "g", "()V", 11), hook(hooks::RESPAWN_DELAY), "HZ-018's edit kept");
        assert_eq!(callee(&out, "g", "()V", 22), hook(hooks::MONSTER_DROP));
        assert_eq!(legend_drop(&out), None, "never applies twice");

        assert!(hooks::LEGEND_FIRST_ID > hooks::GEAR_ID && hooks::GROUND_TOP_ID > hooks::LEGEND_FIRST_ID);

        // HZ-033 alone, and after HZ-005 / HZ-023 / HZ-031 (their hooks kept).
        let mut drop = vec![0u8; 536];
        drop[84..92].copy_from_slice(&[0x68, 0x05, 0x60, 0xb8, 0x01, 0x1a, 0x3e, 0x15]);
        drop[150..164].copy_from_slice(&[0x91, 0x36, 0x04, 0x1c, 0x07, 0x6c, 0x10, 0x0e, 0xb8, 0x01, 0x03, 0x91, 0x36, 0x05]);
        drop[210..220].copy_from_slice(&[0x10, 0x0d, 0x60, 0x10, 0x1c, 0xb8, 0x01, 0x03, 0x91, 0x36]);
        drop[252..281].copy_from_slice(&[
            0x02, 0x9f, 0x00, 0x16, 0xb2, 0x00, 0x60, 0x2a, 0xb7, 0x00, 0xcb, 0x2a, 0xb4, 0x00, 0x87, 0x15, 0x04, 0x15, 0x05, 0xb6, 0x00, 0xfa, 0xb1, 0x84,
            0x07, 0x01, 0xa7, 0xff, 0x2d,
        ]);
        let cq = classfile::tests::synth_class_n("cq", &[("o", "()V", &drop), ("g", "()V", &death)]);
        let check = |out: &[u8]| {
            for (m, at, h) in [
                ("o", 87, hooks::DROP_THRESHOLD),
                ("o", 158, hooks::COMMON_DROP),
                ("o", 215, hooks::RARE_DROP),
                ("o", 271, hooks::DROP_ITEM),
                ("g", 22, hooks::MONSTER_DROP),
            ] {
                assert_eq!(callee(out, m, "()V", at), hook(h), "{m} @{at}");
            }
            let o = &out[classfile::method_code(out, "o", "()V").unwrap()];
            assert_eq!(&o[274..281], &[0x00, 0x84, 0x07, 0x01, 0xa7, 0xff, 0x2d], "the slot's return -> nop, on to the loop's end");
        };
        check(&drop_boost(&cq).expect("alone"));
        let others = legend_drop(&gear_drop(&drop_rate(&cq).unwrap()).unwrap()).unwrap();
        let out = drop_boost(&others).expect("after HZ-005, HZ-023, HZ-031");
        check(&out);
        assert_eq!(drop_boost(&out), None, "never applies twice");
        let g = classfile::tests::synth_class_n("g", &[("a", "(BBBB)V", &ground)]);
        let alone = drop_boost_ground(&g).unwrap();
        assert_eq!(&alone[classfile::method_code(&alone, "a", "(BBBB)V").unwrap()][15..20], &[0x15, 0x04, 0x10, 31, 0xa4]);
        let raised = gear_ground(&g).unwrap();
        assert_eq!(drop_boost_ground(&raised), Some(raised.clone()), "HZ-023's 40 already takes 31");
        let fifty = classfile::rewrite_code(&g, "a", "(BBBB)V", 15, &[0x15, 0x04, 0x10, 0x1c, 0xa4], &[0x15, 0x04, 0x10, 50, 0xa4]).unwrap();
        for g in [g.clone(), raised.clone(), fifty, drop_boost_ground(&g).unwrap()] {
            let out = recipe_ground(&g).unwrap();
            assert_eq!(&out[classfile::method_code(&out, "a", "(BBBB)V").unwrap()][15..20], &[0x15, 0x04, 0x10, hooks::RECIPE_GROUND_TOP as u8, 0xa4]);
            assert_eq!(recipe_ground(&out), None, "never applies twice");
        }
        let out = recipe_drop(&others).expect("HZ-031's hook kept");
        assert_eq!(callee(&out, "g", "()V", 22), hook(hooks::MONSTER_DROP));
        let mut boxed = vec![0u8; 3053];
        boxed[1336..1352].copy_from_slice(&[0x3c, 0xa7, 0x00, 0x0e, 0x1d, 0x10, 0x12, 0xa0, 0x00, 0x08, 0x1b, 0x07, 0x6c, 0x91, 0x3c, 0x1d]);
        let out = recipe_box(&classfile::tests::synth_class("eb", "b", "(BLn;)Z", &boxed)).unwrap();
        assert_eq!(callee(&out, "b", "(BLn;)Z", 1347), hook(hooks::BOX_RECIPE));
        let b = &out[classfile::method_code(&out, "b", "(BLn;)Z").unwrap()];
        assert_eq!((b[1346], b[1350], &b[1340..1346]), (0x1b, 0x3c, &boxed[1340..1346]));
        assert_eq!(recipe_box(&out), None, "never applies twice");

        // HZ-032 on the same death, after HZ-018 and HZ-031, and the other classes' real bytes.
        let mut hit = vec![0u8; 333];
        hit[96..110].copy_from_slice(&[0x2a, 0xb6, 0x00, 0xcc, 0xb6, 0x00, 0xec, 0xb8, 0x00, 0xc3, 0x3e, 0x2b, 0xb4, 0x00]);
        hit[168..184].copy_from_slice(&[0x00, 0xba, 0x15, 0x07, 0xb6, 0x00, 0xe9, 0xa2, 0x00, 0x69, 0x1d, 0x2b, 0x1d, 0xb6, 0x00, 0xea]);
        hit[321..332].copy_from_slice(&[0x15, 0x04, 0x99, 0x00, 0x09, 0x2a, 0x15, 0x05, 0xb6, 0x00, 0x9d]);
        let mut pushed = vec![0u8; 125];
        pushed[112..115].copy_from_slice(&[0xb6, 0x00, 0xf0]);
        let mut mark = vec![0u8; 123];
        mark[74..93].copy_from_slice(&[0x5f, 0x10, 0x08, 0xb6, 0x00, 0xee, 0x3c, 0x2a, 0x04, 0xb5, 0x00, 0x43, 0x2a, 0x10, 0x11, 0x1b, 0xb6, 0x00, 0x9e]);
        let cq = classfile::tests::synth_class_n(
            "cq",
            &[("a", "(Lek;BIZBBZZZ)Z", &hit), ("g", "()V", &death), ("c", "(Lbc;)V", &mark), ("a", "([BSZ)Z", &pushed)],
        );
        let out = effects_cq(&legend_drop(&monster_respawn(&cq).unwrap()).unwrap()).expect("after HZ-018 and HZ-031");
        for (m, d, at, h) in [
            ("a", "(Lek;BIZBBZZZ)Z", 103, hooks::HIT_DAMAGE),
            ("a", "(Lek;BIZBBZZZ)Z", 172, hooks::CRIT_CHANCE),
            ("g", "()V", 11, hooks::RESPAWN_DELAY),
            ("g", "()V", 22, hooks::MONSTER_DROP),
            ("g", "()V", 26, hooks::MONSTER_EXP),
            ("c", "(Lbc;)V", 90, hooks::MARK_STATUS),
            ("a", "(Lek;BIZBBZZZ)Z", 329, hooks::KNOCK),
            ("a", "([BSZ)Z", 112, hooks::KNOCKABLE),
        ] {
            assert_eq!(callee(&out, m, d, at), hook(h), "{m}{d} @{at}");
        }
        let a = &out[classfile::method_code(&out, "a", "(Lek;BIZBBZZZ)Z").unwrap()];
        assert_eq!(&a[321..329], &[0x2a, 0x1d, 0x15, 0x04, 0x15, 0x05, 0x15, 0x06], "the monster, damage, knock, dir, fx");
        assert_eq!(effects_cq(&out), None, "never applies twice");

        let mut taken = vec![0u8; 376];
        taken[330..342].copy_from_slice(&[0x04, 0xb5, 0x00, 0x7c, 0x2a, 0x15, 0x04, 0x74, 0xb6, 0x01, 0x33, 0x2a]);
        taken[119..122].copy_from_slice(&[0xb6, 0x00, 0xbd]);
        let mut drink = vec![0u8; 210];
        drink[196..199].copy_from_slice(&[0xb6, 0x00, 0xd9]);
        let mut drive = vec![0u8; 733];
        drive[348..360].copy_from_slice(&[0x4e, 0xb4, 0x00, 0x5f, 0x15, 0x04, 0xb6, 0x00, 0xec, 0x36, 0x04, 0x2d]);
        drive[448..464].copy_from_slice(&[0x01, 0x1d, 0x1c, 0x04, 0xb8, 0x00, 0xfe, 0x2b, 0xb4, 0x00, 0x5f, 0x1c, 0xb6, 0x00, 0xee, 0x99]);
        let counter = [0x2a, 0x10, 0x14, 0x10, 0x64, 0xb6, 0x00, 0xc1, 0x2a, 0x04, 0xb5, 0x00, 0x7d, 0xb1];
        let ek = classfile::tests::synth_class_n("ek", &[("a", "(Lar;IB)Z", &taken), ("C", "()V", &drive), ("k", "()V", &counter), ("a", "(Z)Z", &drink)]);
        let out = effects_ek(&ek).unwrap();
        assert_eq!(callee(&out, "a", "(Lar;IB)Z", 338), hook(hooks::TAKE_DAMAGE));
        assert_eq!(callee(&out, "C", "()V", 354), hook(hooks::SKILL_SP));
        assert_eq!(callee(&out, "C", "()V", 460), hook(hooks::CAST_START));
        assert_eq!(callee(&out, "k", "()V", 5), hook(hooks::COUNTER_STATUS));
        assert_eq!(callee(&out, "a", "(Lar;IB)Z", 119), hook(hooks::COUNTER_REFLECT));
        assert_eq!(callee(&out, "a", "(Z)Z", 196), hook(hooks::DRANK));
        let c = &out[classfile::method_code(&out, "C", "()V").unwrap()];
        assert_eq!((&c[448..454], &c[455..460]), (&drive[448..454], &drive[455..460]), "the cooldown's start kept");
        assert_eq!(effects_ek(&out), None, "never applies twice");

        let mut cooldown = vec![0u8; 67];
        cooldown[18..44].copy_from_slice(&[
            0xb4, 0x99, 0x00, 0x0d, 0xb2, 0x00, 0x45, 0x1b, 0xb6, 0x00, 0xd0, 0xa7, 0x00, 0x0d, 0xb2, 0x00, 0x45, 0xb6, 0x00, 0xc7, 0x1b, 0xb6, 0x01, 0x01,
            0x4f, 0xb2,
        ]);
        let out = effects_ed(&classfile::tests::synth_class("ed", "g", "(I)V", &cooldown)).unwrap();
        assert_eq!((callee(&out, "g", "(I)V", 26), callee(&out, "g", "(I)V", 39)), (hook(hooks::JOB_COOLDOWN), hook(hooks::SPIRIT_COOLDOWN)));
        let g = &out[classfile::method_code(&out, "g", "(I)V").unwrap()];
        assert_eq!(&g[35..38], &[0xb6, 0x00, 0xc7], "the spirit's getter kept");

        let mut fusion = vec![0u8; 362];
        fusion[180..194].copy_from_slice(&[0x2b, 0x08, 0xb6, 0x00, 0x12, 0x3d, 0x2b, 0xb4, 0x00, 0x08, 0x1c, 0xb6, 0x00, 0x17]);
        let out = effects_eo(&classfile::tests::synth_class("eo", "a", "(BI)Z", &fusion)).unwrap();
        assert_eq!(callee(&out, "a", "(BI)Z", 191), hook(hooks::FUSION_START));
        let mut clone = vec![0u8; 928];
        clone[905..921].copy_from_slice(&[0xee, 0x2b, 0x10, 0x09, 0xb6, 0x00, 0x27, 0x3e, 0xb2, 0x00, 0x17, 0x1d, 0x93, 0xb8, 0x00, 0x34]);
        clone[363..366].copy_from_slice(&[0xb6, 0x00, 0x27]);
        clone[374..377].copy_from_slice(&[0xb6, 0x00, 0x2d]);
        let out = effects_dn(&classfile::tests::synth_class("dn", "a", "(BI)Z", &clone)).unwrap();
        assert_eq!(callee(&out, "a", "(BI)Z", 918), hook(hooks::CLONE_LIFE));
        assert_eq!((callee(&out, "a", "(BI)Z", 363), callee(&out, "a", "(BI)Z", 374)), (hook(hooks::POISON_CHANCE), hook(hooks::POISON_DAMAGE)));
        let one = |name: &str, m: &str, d: &str, at: usize, bytes: [u8; 3], len: usize, f: fn(&[u8]) -> Option<Vec<u8>>, h: hooks::Hook| {
            let mut code = vec![0u8; len];
            code[at..at + 3].copy_from_slice(&bytes);
            let out = f(&classfile::tests::synth_class(name, m, d, &code)).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(callee(&out, m, d, at), hook(h), "{name}");
            assert_eq!(f(&out), None, "{name} never twice");
        };
        one("d", "a", "(BI)Z", 139, [0xb7, 0x00, 0x1c], 160, effects_d, hooks::SHIELD_BASH);
        one("ai", "a", "(BI)Z", 953, [0xb6, 0x00, 0x27], 1030, effects_ai, hooks::SNIPE_CHANCE);
        one("av", "c", "()Z", 9, [0xb6, 0x00, 0x15], 20, effects_av, hooks::BLAST_CHANCE);
        one("at", "b", "()V", 49, [0xb6, 0x00, 0x13], 60, effects_at, hooks::KNOCKABLE);
        one("di", "b", "()V", 96, [0xb6, 0x00, 0x28], 110, effects_di, hooks::KNOCKABLE);
        one("dk", "b", "()V", 49, [0xb6, 0x00, 0x26], 60, effects_dk, hooks::KNOCKABLE);
        one("ar", "d", "()Z", 25, [0xb4, 0x00, 0x17], 39, effects_ar, hooks::MOVE_STEP);
        let mut warrior = vec![0u8; 420];
        warrior[131..134].copy_from_slice(&[0xb6, 0x00, 0x25]);
        warrior[392..397].copy_from_slice(&[0x1c, 0x74, 0xb6, 0x00, 0x27]);
        let out = effects_ag(&classfile::tests::synth_class("ag", "a", "(BI)Z", &warrior)).unwrap();
        assert_eq!((callee(&out, "a", "(BI)Z", 131), callee(&out, "a", "(BI)Z", 394)), (hook(hooks::SMASH), hook(hooks::FRENZY_COST)));
        let mut cast = vec![0u8; 187];
        cast[38..50].copy_from_slice(&[0x33, 0x59, 0x3e, 0x2c, 0x10, 0x08, 0x1d, 0xb6, 0x00, 0x72, 0x64, 0x59]);
        let mut power = vec![0u8; 26];
        power[14..26].copy_from_slice(&[0x79, 0x60, 0x3c, 0x2a, 0xb4, 0x00, 0x2f, 0x1b, 0xb6, 0x00, 0x4e, 0xac]);
        let out = effects_p(&classfile::tests::synth_class_n("p", &[("d", "(B)Z", &cast), ("e", "(B)I", &power)])).unwrap();
        assert_eq!((callee(&out, "d", "(B)Z", 45), callee(&out, "e", "(B)I", 22)), (hook(hooks::SPIRIT_COST), hook(hooks::SPIRIT_POWER)));
        assert_eq!(effects_p(&out), None, "never applies twice");
        let order = |id: &str| PATCHES.iter().position(|p| p.id == id).unwrap();
        assert!(order("HZ-016") < order("HZ-031") && order("HZ-023") < order("HZ-031") && order("HZ-018") < order("HZ-031"));
    }

    /// HZ-019 on a synthetic `g` with the real bytes (classbytes.py 2026-10-25): the notices' anchor
    /// reads the hook, the rest of the call is untouched, and HZ-013's edit stays.
    #[test]
    fn the_pickup_notices_read_their_bottom_from_the_hook() {
        let mut paint = vec![0u8; 1488];
        paint[1428..1462].copy_from_slice(&[
            0x2b, 0x19, 0x08, 0xb4, 0x00, 0x84, 0x1c, 0x32, 0xb2, 0x00, 0xa4, 0xb2, 0x00, 0x58, 0x1c, 0x19, 0x08, 0xb4, 0x00, 0x84, 0x1c,
            0x32, 0xb6, 0x00, 0xcc, 0x04, 0x60, 0x68, 0x64, 0x10, 0x21, 0xb6, 0x00, 0xd1,
        ]);
        let mut window = vec![0u8; 200];
        window[157..162].copy_from_slice(&[0xb2, 0x00, 0xa3, 0x10, 33]);
        let g = classfile::tests::synth_class_n("g", &[("a", "(Ldd;)V", &window), ("c", "()V", &paint)]);
        let out = notices_over_exp_bar(&hud_off_window(&g).unwrap()).expect("after HZ-013");
        assert_eq!(callee(&out, "c", "()V", 1439), hook(hooks::NOTICE_BOTTOM));
        let c = &out[classfile::method_code(&out, "c", "()V").unwrap()];
        assert_eq!(&c[1436..1439], &[0xb2, 0x00, 0xa4], "rpg/a.c, the centre, as before");
        assert_eq!(&c[1442..1462], &paint[1442..1462]);
        let a = &out[classfile::method_code(&out, "a", "(Ldd;)V").unwrap()];
        assert_eq!(&a[157..162], &[0xb2, 0x00, 0xa3, 0x10, 0], "HZ-013's edit kept");
        assert_eq!(notices_over_exp_bar(&out), None, "never applies twice");
    }

    /// HZ-025 on the same synthetic `g`: the notice's draw calls the hook with the slot where the
    /// anchor was, after HZ-019's anchor — the rest of the call as it was.
    #[test]
    fn the_pickup_notices_go_to_the_host() {
        let mut paint = vec![0u8; 1488];
        paint[1428..1462].copy_from_slice(&[
            0x2b, 0x19, 0x08, 0xb4, 0x00, 0x84, 0x1c, 0x32, 0xb2, 0x00, 0xa4, 0xb2, 0x00, 0x58, 0x1c, 0x19, 0x08, 0xb4, 0x00, 0x84, 0x1c,
            0x32, 0xb6, 0x00, 0xcc, 0x04, 0x60, 0x68, 0x64, 0x10, 0x21, 0xb6, 0x00, 0xd1,
        ]);
        let g = classfile::tests::synth_class_n("g", &[("c", "()V", &paint)]);
        let out = pickup_notices_to_host(&notices_over_exp_bar(&g).unwrap()).expect("after HZ-019");
        assert_eq!(callee(&out, "c", "()V", 1459), hook(hooks::PICKUP_NOTICE));
        assert_eq!(callee(&out, "c", "()V", 1439), hook(hooks::NOTICE_BOTTOM), "HZ-019's edit kept");
        let c = &out[classfile::method_code(&out, "c", "()V").unwrap()];
        assert_eq!(&c[1457..1459], &[0x1c, 0x00], "iload_2 (the slot); nop");
        assert_eq!(&c[1428..1439], &paint[1428..1439]);
        assert_eq!(&c[1442..1457], &paint[1442..1457]);
        assert_eq!(pickup_notices_to_host(&out), None, "never applies twice");
        let alone = pickup_notices_to_host(&g).expect("alone");
        assert_eq!(callee(&alone, "c", "()V", 1459), hook(hooks::PICKUP_NOTICE));
        let order = |id: &str| PATCHES.iter().position(|p| p.id == id).unwrap();
        assert!(order("HZ-019") < order("HZ-025"));
    }

    /// HZ-020 on synthetic `rpg/b`, `bb` and `i` with the real bytes (classbytes.py 2026-10-25): all
    /// twelve sites call their hooks, the bytes round them are kept, and nothing applies twice.
    #[test]
    fn slot_pages_sites_call_their_hooks() {
        let reader = |cp: u8| vec![0xb2, 0x00, cp, 0x1a, 0x32, 0xb2, 0, 0, 0, 0];
        let writer = |cp: u8| vec![0xb2, 0x00, cp, 0x1b, 0x32, 0x2a, 0, 0, 0, 0];
        let (ra, wa, rb, wb, rc, wc) = (reader(0x2f), writer(0x2f), reader(0x31), writer(0x31), reader(0x33), writer(0x33));
        let b = classfile::tests::synth_class_n(
            "rpg/b",
            &[("a", "(B)[B", &ra), ("a", "([BB)Z", &wa), ("b", "(B)[B", &rb), ("b", "([BB)Z", &wb), ("c", "(B)[B", &rc), ("c", "([BB)Z", &wc)],
        );
        let out = slot_pages_stores(&b).unwrap();
        for (name, want) in [("a", hooks::GAMEDATA_STORES), ("b", hooks::ITEM_STORES), ("c", hooks::SLOT_STORES)] {
            assert_eq!(callee(&out, name, "(B)[B", 0), hook(want));
            assert_eq!(callee(&out, name, "([BB)Z", 0), hook(want));
            assert_eq!(&out[classfile::method_code(&out, name, "(B)[B").unwrap()][3..5], &[0x1a, 0x32], "iload_0; aaload kept");
        }
        assert_eq!(slot_pages_stores(&out), None, "never applies twice");

        let mut keys = vec![0u8; 249];
        keys[0..8].copy_from_slice(&[0x2a, 0x1b, 0x1c, 0xb6, 0x00, 0x5a, 0x99, 0x00]);
        let mut paint = vec![0u8; 237];
        paint[0..30].copy_from_slice(&[
            0x2b, 0x04, 0xb8, 0x00, 0x62, 0x2b, 0x12, 0x07, 0xb6, 0x00, 0x4c, 0x2b, 0xb2, 0x00, 0x35, 0xb2, 0x00, 0x3d, 0x2a, 0xb4, 0x00, 0x25, 0x03,
            0x04, 0xb8, 0x00, 0x3f, 0x03, 0x3e, 0x1d,
        ]);
        paint[90..98].copy_from_slice(&[0x05, 0x12, 0x03, 0xb8, 0x00, 0x59, 0x1d, 0x2a]);
        paint[168..178].copy_from_slice(&[0xb8, 0x00, 0x65, 0x03, 0x04, 0xb8, 0x00, 0x3f, 0x1d, 0x04]);
        let bb = classfile::tests::synth_class_n("bb", &[("a", "(II)Z", &keys), ("a", "(Ldd;)V", &paint)]);
        let out = slot_pages_window(&bb).unwrap();
        assert_eq!(callee(&out, "a", "(II)Z", 3), hook(hooks::SLOT_KEY));
        assert_eq!(callee(&out, "a", "(Ldd;)V", 24), hook(hooks::SLOT_TITLE));
        assert_eq!(callee(&out, "a", "(Ldd;)V", 93), hook(hooks::SLOT_BOX));
        assert_eq!(callee(&out, "a", "(Ldd;)V", 173), hook(hooks::SLOT_LABEL));
        let p = &out[classfile::method_code(&out, "a", "(Ldd;)V").unwrap()];
        assert_eq!(&p[168..171], &[0xb8, 0x00, 0x65], "rpg/g.a, the label's concat, kept");
        assert_eq!(slot_pages_window(&out), None);

        let mut init = vec![0u8; 27];
        init[0..16].copy_from_slice(&[0x2a, 0x01, 0x10, 0x0a, 0xb7, 0x00, 0x59, 0x2a, 0xb2, 0x00, 0x31, 0x9e, 0x00, 0x07, 0x04, 0xa7]);
        let mut carousel = vec![0u8; 313];
        carousel[24..34].copy_from_slice(&[0x04, 0xa0, 0x00, 0x09, 0xb2, 0x00, 0x31, 0x9e, 0x00, 0x11]);
        let i = classfile::tests::synth_class_n("i", &[("<init>", "()V", &init), ("a", "(II)Z", &carousel)]);
        let out = slot_pages_title(&i).unwrap();
        assert_eq!(callee(&out, "<init>", "()V", 8), hook(hooks::SAVE_COUNT));
        assert_eq!(callee(&out, "a", "(II)Z", 28), hook(hooks::SAVE_COUNT));
        assert_eq!(slot_pages_title(&out), None);
        assert_eq!((hooks::SLOT_PAGES, hooks::page_slots(0), hooks::page_slots(3), hooks::page_slots(4)), (4, 3, 1, 0));
    }

    /// HZ-022 on a synthetic `ek` with the real bytes, chained after HZ-004 (classbytes.py 2026-10-25).
    #[test]
    fn guardian_sites_call_their_hooks() {
        let mut j = vec![0u8; 207];
        j[32..41].copy_from_slice(&[0x1b, 0x9c, 0x00, 0x08, 0x1b, 0x74, 0xa7, 0x00, 0x04]);
        j[72..82].copy_from_slice(&[0x64, 0x4f, 0x2a, 0xb4, 0x00, 0x8d, 0x10, 0x63, 0xa2, 0x00]);
        let mut i = vec![0u8; 1214];
        i[1000..1016].copy_from_slice(&[0x60, 0x3c, 0x2a, 0x05, 0x1b, 0x3e, 0x3d, 0x59, 0x4c, 0xb4, 0x00, 0x6e, 0x1c, 0x1d, 0x4f, 0x08]);
        let e = [0x2a, 0xb4, 0x00, 0x64, 0x1b, 0x2e, 0xac];
        let f = [0x2a, 0xb4, 0x00, 0x6e, 0x1b, 0x2e, 0xac];
        let ek = classfile::tests::synth_class_n("ek", &[("j", "(I)V", &j), ("i", "()V", &i), ("e", "(B)I", &e), ("f", "(B)I", &f)]);
        let out = guardian_levels(&exp_rate(&ek).unwrap()).expect("after HZ-004");
        assert_eq!(callee(&out, "j", "(I)V", 33), hook(hooks::EXP), "HZ-004 kept");
        assert_eq!(callee(&out, "j", "(I)V", 75), hook(hooks::LEVEL_UP));
        assert_eq!(&out[classfile::method_code(&out, "j", "(I)V").unwrap()][78..80], &[0x10, 0x63], "bipush 99 kept");
        assert_eq!(callee(&out, "i", "()V", 1009), hook(hooks::EXP_TABLE));
        for (m, h) in [("e", hooks::STAT_NOW), ("f", hooks::STAT_MAX)] {
            let c = &out[classfile::method_code(&out, m, "(B)I").unwrap()];
            assert_eq!((c[0], c[1], c[5], c[6]), (0x2a, 0x1b, 0xac, 0x00));
            assert_eq!(callee(&out, m, "(B)I", 2), hook(h));
        }
        assert_eq!(guardian_levels(&out), None, "never applies twice");
    }

    /// HZ-021 on synthetic `co` and `ee` with the real bytes (classbytes.py 2026-10-25).
    #[test]
    fn bulk_sell_sites_call_their_hooks() {
        let mut paint = vec![0u8; 355];
        paint[307..327].copy_from_slice(&[0x2a, 0x2b, 0x2a, 0xb4, 0x00, 0x32, 0x10, 0x6f, 0x60, 0x15, 0x04, 0x05, 0x60, 0xb2, 0x00, 0x2f, 0xb6, 0x00, 0x61, 0x2a]);
        let co = classfile::tests::synth_class("co", "a", "(Ldd;)V", &paint);
        let out = bulk_sell_button(&co).unwrap();
        assert_eq!(callee(&out, "a", "(Ldd;)V", 323), hook(hooks::SHOP_GOLD));
        assert_eq!(&out[classfile::method_code(&out, "a", "(Ldd;)V").unwrap()][320..323], &[0xb2, 0x00, 0x2f], "ed.e, the gold, kept");
        assert_eq!(bulk_sell_button(&out), None);
        let mut keys = vec![0u8; 59];
        keys[24..40].copy_from_slice(&[0x2a, 0xb4, 0x00, 0x11, 0x2a, 0xb4, 0x00, 0x0c, 0x2a, 0xb4, 0x00, 0x10, 0xb6, 0x00, 0x1c, 0xa7]);
        let ee = classfile::tests::synth_class("ee", "a", "(II)Z", &keys);
        let out = bulk_sell_popup(&ee).unwrap();
        assert_eq!(callee(&out, "a", "(II)Z", 36), hook(hooks::ANSWER));
        assert_eq!(bulk_sell_popup(&out), None);
        assert!(hooks::BULK_GRADE != hooks::BULK_ASK && ![0, 1, 2, 3, 4, 5, 6].contains(&hooks::BULK_GRADE) && ![0, 1, 2, 3, 4, 5, 6].contains(&hooks::BULK_ASK));
    }

    /// A method's code `len` long with the real bytes at their offsets.
    fn code_with(len: usize, bytes: &[(usize, &[u8])]) -> Vec<u8> {
        let mut code = vec![0u8; len];
        for &(at, b) in bytes {
            code[at..at + b.len()].copy_from_slice(b);
        }
        code
    }

    /// HZ-036 on synthetic classes with the real bytes (classbytes.py 2026-10-28).
    #[test]
    fn bag_pages_sites_call_their_hooks() {
        let ed = classfile::tests::synth_class("ed", "f", "()V", &code_with(37, &[(4, &[0x10, 28]), (16, &[0x10, 28]), (28, &[0x10, 28])]));
        let out = bag_size(&ed).unwrap();
        let f = &out[classfile::method_code(&out, "f", "()V").unwrap()];
        assert_eq!([f[5], f[17], f[29]], [112; 3], "three bags of 112");
        assert_eq!(bag_size(&out), None);

        let q = classfile::tests::synth_class("q", "a", "(Ldd;Lbt;BZBBI)V", &code_with(397, &[(134, &[0xb6, 0x00, 0x46]), (309, &[0xb6, 0x00, 0x46])]));
        let out = bag_pages_q(&q).unwrap();
        for at in [134, 309] {
            assert_eq!(callee(&out, "a", "(Ldd;Lbt;BZBBI)V", at), hook(hooks::BAG_AT));
        }

        let keys = |len: usize, at: usize, d: u8, side: usize, e: usize, (dc, ec): (u8, u8)| {
            code_with(len, &[(at, &[0xb6, 0x00, d]), (side, &[0xb6, 0x00, dc]), (e, &[0xb6, 0x00, ec])])
        };
        let ds = classfile::tests::synth_class_n("ds", &[
            ("a", "(II)Z", &keys(405, 164, 0x23, 388, 399, (0x3b, 0x3c))),
            ("a", "(Ldd;)V", &code_with(48, &[(44, &[0xb6, 0x00, 0x41])])),
            ("a", "(BB)V", &code_with(352, &[(316, &[0xb6, 0x00, 0x24]), (342, &[0xb6, 0x00, 0x25])])),
        ]);
        let out = bag_pages_ds(&ds).unwrap();
        assert_eq!(callee(&out, "a", "(II)Z", 164), hook(hooks::BAG_AT));
        assert_eq!(callee(&out, "a", "(II)Z", 388), hook(hooks::BAG_SIDEWAYS));
        assert_eq!(callee(&out, "a", "(II)Z", 399), hook(hooks::BAG_UPDOWN));
        assert_eq!(callee(&out, "a", "(Ldd;)V", 44), hook(hooks::BAG_GRID));
        assert_eq!(callee(&out, "a", "(BB)V", 316), hook(hooks::BAG_TAKE), "버리기 takes the page's item");
        assert_eq!(callee(&out, "a", "(BB)V", 342), hook(hooks::BAG_TAKE_SOME), "a 귀환서 off the page's stack");

        let eg = classfile::tests::synth_class_n("eg", &[
            ("a", "(II)Z", &keys(350, 197, 0x36, 335, 346, (0x51, 0x52))),
            ("c", "(B)V", &code_with(140, &[(89, &[0xb4, 0x00, 0x1b])])),
            ("a", "(Ldd;)V", &code_with(122, &[(118, &[0xb6, 0x00, 0x57])])),
            ("a", "(BB)V", &code_with(500, &[(162, &[0xb6, 0x00, 0x37]), (396, &[0xb6, 0x00, 0x37]), (473, &[0xb6, 0x00, 0x37]), (493, &[0xb6, 0x00, 0x37])])),
        ]);
        let out = bag_pages_eg(&eg).unwrap();
        assert_eq!(callee(&out, "a", "(II)Z", 197), hook(hooks::BAG_AT));
        assert_eq!(callee(&out, "a", "(II)Z", 335), hook(hooks::BAG_SIDEWAYS));
        assert_eq!(callee(&out, "a", "(II)Z", 346), hook(hooks::BAG_UPDOWN));
        assert_eq!(callee(&out, "c", "(B)V", 89), hook(hooks::BAG_CELLS));
        assert_eq!(callee(&out, "a", "(Ldd;)V", 118), hook(hooks::BAG_GRID));
        assert_eq!(callee(&out, "a", "(BB)V", 162), hook(hooks::BAG_TAKE), "넣기 takes the page's item");
        assert_eq!(callee(&out, "a", "(BB)V", 473), hook(hooks::BAG_TAKE), "버리기 too");
        let answer = &out[classfile::method_code(&out, "a", "(BB)V").unwrap()];
        for at in [396, 493] {
            assert_eq!(answer[at..at + 3], [0xb6, 0x00, 0x37], "@{at} takes from a 계정창고 page: the game's");
        }

        let bz = classfile::tests::synth_class("bz", "a", "(Ldd;Lbt;)V", &code_with(222, &[(53, &[0xb6, 0x00, 0x1c])]));
        assert_eq!(callee(&bag_pages_bz(&bz).unwrap(), "a", "(Ldd;Lbt;)V", 53), hook(hooks::BAG_AT));

        // The shop: its HZ-021 site @323 in the same painter, chained first.
        let mut paint = code_with(355, &[(135, &[0xb6, 0x00, 0x44]), (172, &[0xb6, 0x00, 0x3e])]);
        paint[307..327].copy_from_slice(&[0x2a, 0x2b, 0x2a, 0xb4, 0x00, 0x32, 0x10, 0x6f, 0x60, 0x15, 0x04, 0x05, 0x60, 0xb2, 0x00, 0x2f, 0xb6, 0x00, 0x61, 0x2a]);
        let co = classfile::tests::synth_class_n("co", &[("a", "(II)Z", &keys(561, 354, 0x3e, 555, 513, (0x64, 0x65))), ("a", "(Ldd;)V", &paint)]);
        let out = bag_pages_co(&bulk_sell_button(&co).unwrap()).expect("after HZ-021");
        assert_eq!(callee(&out, "a", "(II)Z", 354), hook(hooks::BAG_AT));
        assert_eq!(callee(&out, "a", "(II)Z", 513), hook(hooks::BAG_UPDOWN));
        assert_eq!(callee(&out, "a", "(II)Z", 555), hook(hooks::BAG_SIDEWAYS));
        assert_eq!(callee(&out, "a", "(Ldd;)V", 135), hook(hooks::BAG_GRID_BZ));
        assert_eq!(callee(&out, "a", "(Ldd;)V", 172), hook(hooks::BAG_AT));
        assert_eq!(callee(&out, "a", "(Ldd;)V", 323), hook(hooks::SHOP_GOLD), "HZ-021 kept");

        for (name, f, len, at, (d, side, e), (dc, ec), paint_len, grid, grid_cp, item) in [
            ("dv", bag_pages_dv as Rewrite, 1711, 210, (0x4c, 773, 780), (0x73, 0x74), 1626, 79, 0x51, 107),
            ("et", bag_pages_et as Rewrite, 504, 110, (0x38, 173, 180), (0x5a, 0x5b), 618, 64, 0x3c, 92),
        ] {
            let c = classfile::tests::synth_class_n(name, &[
                ("a", "(II)Z", &keys(len, at, d, side, e, (dc, ec))),
                ("a", "(Ldd;)V", &code_with(paint_len, &[(grid, &[0xb6, 0x00, grid_cp]), (item, &[0xb6, 0x00, d])])),
            ]);
            let out = f(&c).unwrap();
            assert_eq!(callee(&out, "a", "(II)Z", at), hook(hooks::BAG_AT), "{name}");
            assert_eq!(callee(&out, "a", "(II)Z", side), hook(hooks::BAG_SIDEWAYS), "{name}");
            assert_eq!(callee(&out, "a", "(II)Z", e), hook(hooks::BAG_UPDOWN), "{name}");
            assert_eq!(callee(&out, "a", "(Ldd;)V", grid), hook(hooks::BAG_GRID_BZ), "{name}");
            assert_eq!(callee(&out, "a", "(Ldd;)V", item), hook(hooks::BAG_AT), "{name}");
            assert_eq!(f(&out), None, "{name} never twice");
        }
        assert_eq!((hooks::BAG_PAGES, hooks::BAG_SIZE), (4, 112));
    }

    /// HZ-037 / HZ-038 on synthetic classes with the real bytes (classbytes.py 2026-10-28).
    #[test]
    fn storage_and_stacks_grow() {
        let dz = classfile::tests::synth_class_n("dz", &[
            ("h", "()V", &code_with(220, &[(25, &[0x10, 84]), (50, &[0x2a, 0x07, 0x84, 0x01, 0x01, 0x33])])),
            ("i", "()V", &code_with(149, &[(25, &[0x10, 84]), (35, &[0x06]), (64, &[0x06])])),
        ]);
        let out = storage_size_dz(&dz).unwrap();
        let h = &out[classfile::method_code(&out, "h", "()V").unwrap()];
        assert_eq!(&h[25..27], &[0x10, 112]);
        assert_eq!(&h[50..56], &[0x10, 112, 0x84, 0x01, 0x01, 0x00], "the stored size read as 112, the iinc kept");
        let i = &out[classfile::method_code(&out, "i", "()V").unwrap()];
        assert_eq!(&i[25..27], &[0x10, 112]);
        assert_eq!((i[35], i[64]), (0x07, 0x07), "the save walks all four pages (iconst_4), count and items");
        assert_eq!(storage_size_dz(&out), None);

        let eb = classfile::tests::synth_class(
            "eb",
            "a",
            "([[I)V",
            &code_with(509, &[(31, &[0x10, 84]), (54, &[0x2b, 0x07, 0x84, 0x02, 0x01, 0x33]), (292, &[0x06]), (381, &[0x10, 84]), (392, &[0x06]), (424, &[0x06])]),
        );
        let out = storage_size_eb(&eb).unwrap();
        let a = &out[classfile::method_code(&out, "a", "([[I)V").unwrap()];
        assert_eq!((&a[31..33], &a[54..60], &a[381..383]), (&[0x10, 112][..], &[0x10, 112, 0x84, 0x02, 0x01, 0x00][..], &[0x10, 112][..]));
        assert_eq!((a[292], a[392], a[424]), (0x07, 0x07, 0x07), "the gift's room and its write over four pages");

        let ninety_nine: &[u8] = &[0x10, 99];
        let bt = classfile::tests::synth_class_n("bt", &[
            ("a", "(BBI)Z", &code_with(175, &[(152, ninety_nine), (160, ninety_nine)])),
            ("a", "(Lba;I)I", &code_with(263, &[(177, ninety_nine), (193, ninety_nine), (198, ninety_nine)])),
            ("b", "()V", &code_with(250, &[(45, ninety_nine), (124, ninety_nine), (144, ninety_nine)])),
        ]);
        let out = stack_max(&bt).unwrap();
        for (m, d, ats) in [("a", "(BBI)Z", &[152, 160][..]), ("a", "(Lba;I)I", &[177, 193, 198]), ("b", "()V", &[45, 124, 144])] {
            let code = &out[classfile::method_code(&out, m, d).unwrap()];
            for &at in ats {
                assert_eq!(&code[at..at + 2], &[0x10, 127], "{m}{d} @{at}");
            }
        }
        assert_eq!(stack_max(&out), None, "never twice");
    }

    /// The Methodref an `invokestatic` at `at` of `code` calls.
    fn callee(class: &[u8], method: &str, descriptor: &str, at: usize) -> (String, String, String) {
        let code = &class[classfile::method_code(class, method, descriptor).unwrap()];
        assert_eq!(code[at], 0xb8, "invokestatic at {at}");
        classfile::tests::resolve(class, u16::from_be_bytes([code[at + 1], code[at + 2]]))
    }

    fn hook(h: hooks::Hook) -> (String, String, String) {
        (hooks::CLASS.into(), h.name.into(), h.descriptor.into())
    }

    #[test]
    fn hooks_are_unique() {
        for (i, h) in hooks::ALL.iter().enumerate() {
            assert!(hooks::ALL[..i].iter().all(|g| g.name != h.name), "{}", h.name);
        }
    }

    /// HZ-004 on a synthetic `ek` whose `j(I)V` starts with the real 44 bytes.
    #[test]
    fn exp_rate_calls_the_hook_for_the_amount_added() {
        let mut code = vec![0u8; 207];
        code[..44].copy_from_slice(&[
            0xb2, 0x00, 0x49, 0x9a, 0x00, 0x12, 0xb2, 0x00, 0x9f, 0x99, 0x00, 0x10, 0x2a, 0xb4, 0x00, 0x8d, 0x10, 0x14, 0xa2, 0x00, 0x07, 0x1b,
            0x04, 0x78, 0x3c, 0x2a, 0xb4, 0x00, 0x64, 0x05, 0x5c, 0x2e, 0x1b, 0x9c, 0x00, 0x08, 0x1b, 0x74, 0xa7, 0x00, 0x04, 0x1b, 0x60, 0x4f,
        ]);
        let class = classfile::tests::synth_class("ek", "j", "(I)V", &code);
        let out = exp_rate(&class).unwrap();
        let new = &out[classfile::method_code(&out, "j", "(I)V").unwrap()];
        assert_eq!(callee(&out, "j", "(I)V", 33), hook(hooks::EXP));
        assert_eq!(new[32], 0x1b, "iload_1 feeds the hook");
        assert_eq!(&new[36..39], &[0, 0, 0xa7], "nop; nop; goto");
        assert_eq!(38 + i16::from_be_bytes([new[39], new[40]]) as usize, 42, "then the iadd");
        assert_eq!(&new[..32], &code[..32], "cash x2 and everything before it untouched");
        assert_eq!(&new[41..], &code[41..]);
        assert_eq!(exp_rate(&out), None, "never applies twice");
    }

    #[test]
    fn drop_rate_and_auto_loot_replace_one_invoke() {
        let mut code = vec![0u8; 536];
        code[84..90].copy_from_slice(&[0x68, 0x05, 0x60, 0xb8, 0x01, 0x1a]);
        let out = drop_rate(&classfile::tests::synth_class("cq", "o", "()V", &code)).unwrap();
        assert_eq!(callee(&out, "o", "()V", 87), hook(hooks::DROP_THRESHOLD));

        let mut code = vec![0u8; 134];
        code[74..81].copy_from_slice(&[0xb4, 0x00, 0x7d, 0xb6, 0x00, 0xdd, 0x57]);
        let out = auto_loot(&classfile::tests::synth_class("ed", "d", "()V", &code)).unwrap();
        assert_eq!(callee(&out, "d", "()V", 77), hook(hooks::AUTO_LOOT));
        assert_eq!(out[classfile::method_code(&out, "d", "()V").unwrap()][80], 0x57, "the pop stays");
    }

    /// HZ-008 on a synthetic `f` whose `paint` ends with the real 18 bytes @231 (classbytes.py
    /// 2026-10-22): getstatic dd.a; ifnull 248; aload_1; getstatic dd.a; iconst_0; iconst_0;
    /// bipush 20; invokevirtual drawImage; return.
    #[test]
    fn scene_blit_replaces_the_one_draw_image() {
        const PAINT: (&str, &str) = ("paint", "(Ljavax/microedition/lcdui/Graphics;)V");
        let mut code = vec![0u8; 249];
        code[231..249].copy_from_slice(&[
            0xb2, 0x00, 0x4e, 0xc6, 0x00, 0x0e, 0x2b, 0xb2, 0x00, 0x4e, 0x03, 0x03, 0x10, 0x14, 0xb6, 0x00, 0x52, 0xb1,
        ]);
        let out = scene_blit(&classfile::tests::synth_class("f", PAINT.0, PAINT.1, &code)).unwrap();
        assert_eq!(callee(&out, PAINT.0, PAINT.1, 245), hook(hooks::BLIT));
        let paint = &out[classfile::method_code(&out, PAINT.0, PAINT.1).unwrap()];
        assert_eq!(&paint[237..245], &code[237..245], "the operands are pushed as before");
        assert_eq!(paint[248], 0xb1, "return");
        assert_eq!(scene_blit(&out), None, "never applies twice");
        code[246] = 0x01;
        assert_eq!(scene_blit(&classfile::tests::synth_class("f", PAINT.0, PAINT.1, &code)), None, "another build of f passes through");
    }

    /// HZ-014 on a synthetic `ce` with the real bytes @23..30 of `f` (classbytes.py 2026-10-23:
    /// getstatic rpg/a.a; bipush 80; isub; istore_1) and HZ-010's / HZ-013's sites: the panel's x
    /// comes from the hook, and it chains after both — as the transformer runs them, table order.
    #[test]
    fn the_target_panel_x_comes_from_the_hook() {
        let mut init = vec![0u8; 187];
        init[113..119].copy_from_slice(&[0xb2, 0x00, 0x75, 0x10, 0x21, 0x64]);
        let mut keys = vec![0u8; 844];
        keys[278..281].copy_from_slice(&[0xb2, 0x00, 0x69]);
        let mut f = vec![0u8; 1102];
        f[23..30].copy_from_slice(&[0xb2, 0x00, 0x72, 0x10, 0x50, 0x64, 0x3c]);
        f[104..110].copy_from_slice(&[0xb8, 0x00, 0xaa, 0xb2, 0x00, 0x72]);
        f[375..378].copy_from_slice(&[0xb2, 0x00, 0x69]);
        let ce = classfile::tests::synth_class_n("ce", &[("<init>", "()V", &init), ("a", "(II)Z", &keys), ("f", "()V", &f)]);
        let chained = hud_off_field(&cheat_shop_field(&ce).unwrap()).unwrap();
        let out = target_dodge(&chained).expect("after HZ-010 and HZ-013");
        assert_eq!(callee(&out, "f", "()V", 26), hook(hooks::TARGET_X));
        let code = &out[classfile::method_code(&out, "f", "()V").unwrap()];
        assert_eq!(&code[23..26], &[0xb2, 0x00, 0x72], "the width is still pushed");
        assert_eq!(code[29], 0x3c, "istore_1 takes the hook's x");
        assert_eq!(&code[107..110], &[0xa7, 0x03, 0xd8], "HZ-013's edit kept");
        assert_eq!(target_dodge(&out), None, "never applies twice");
        assert!(target_dodge(&ce).is_some(), "alone too");
        let p = PATCHES.iter().find(|p| p.id == "HZ-014").unwrap();
        let ce_md5 = |id: &str| PATCHES.iter().find(|p| p.id == id).unwrap().kind.targets().into_iter().find(|t| t.0 == "ce").unwrap().1;
        assert_eq!(p.kind.targets()[0].1, ce_md5("HZ-013"), "the same original ce");
        let order = |id: &str| PATCHES.iter().position(|p| p.id == id).unwrap();
        assert!(order("HZ-010") < order("HZ-014") && order("HZ-013") < order("HZ-014"), "runs after them");
    }

    /// HZ-054 on the same synthetic `ce` with the real bytes of its two draws (classbytes.py
    /// 2026-10-29): both become hooks, after HZ-010, HZ-013 and HZ-014 — and neither without the other.
    #[test]
    fn the_target_panel_draws_through_the_hooks() {
        let mut init = vec![0u8; 187];
        init[113..119].copy_from_slice(&[0xb2, 0x00, 0x75, 0x10, 0x21, 0x64]);
        let mut keys = vec![0u8; 844];
        keys[278..281].copy_from_slice(&[0xb2, 0x00, 0x69]);
        let mut f = vec![0u8; 1102];
        f[23..30].copy_from_slice(&[0xb2, 0x00, 0x72, 0x10, 0x50, 0x64, 0x3c]);
        f[40..43].copy_from_slice(&[0xb6, 0x00, 0xb9]);
        f[83..86].copy_from_slice(&[0xb6, 0x00, 0xb8]);
        f[104..110].copy_from_slice(&[0xb8, 0x00, 0xaa, 0xb2, 0x00, 0x72]);
        f[375..378].copy_from_slice(&[0xb2, 0x00, 0x69]);
        let ce = classfile::tests::synth_class_n("ce", &[("<init>", "()V", &init), ("a", "(II)Z", &keys), ("f", "()V", &f)]);
        let chained = target_dodge(&hud_off_field(&cheat_shop_field(&ce).unwrap()).unwrap()).unwrap();
        let out = target_drop(&chained).expect("after HZ-010, HZ-013 and HZ-014");
        assert_eq!(callee(&out, "f", "()V", 26), hook(hooks::TARGET_X));
        assert_eq!(callee(&out, "f", "()V", 40), hook(hooks::TARGET_PICTURE));
        assert_eq!(callee(&out, "f", "()V", 83), hook(hooks::TARGET_BAR));
        assert_eq!(target_drop(&out), None, "never applies twice");
        f[83] = 0;
        let half = classfile::tests::synth_class_n("ce", &[("<init>", "()V", &init), ("a", "(II)Z", &keys), ("f", "()V", &f)]);
        assert_eq!(target_drop(&half), None, "the bar's site off -> neither");
        let order = |id: &str| PATCHES.iter().position(|p| p.id == id).unwrap();
        assert!(order("HZ-014") < order("HZ-054"));
    }

    /// The real bytes of `bn` at every HZ-007 site (javap + tools_re/classbytes.py, 2026-10-22).
    fn synth_bn(loop_bound: u8) -> Vec<u8> {
        let init = [0x2a, 0x2b, 0x10, 0x07, 0xb7, 0x00, 0x3b, 0x2a, 0x1c, 0xb5, 0x00, 0x16, 0x2a, 0x04, 0xb5, 0x00, 0x1a, 0xb1];
        let mut draw = vec![0u8; 488];
        draw[57..83].copy_from_slice(&[
            0x2b, 0x12, 0x02, 0xb6, 0x00, 0x34, 0x2b, 0xb2, 0x00, 0x1d, 0xb2, 0x00, 0x28, 0xb2, 0x00, 0x27, 0x10, 0x07, 0xb6, 0x00, 0x33, 0x03,
            0x04, 0xb8, 0x00, 0x2a,
        ]);
        draw[102..138].copy_from_slice(&[
            0x15, 0x04, 0x10, loop_bound, 0xa2, 0x00, 0x1f, 0x2b, 0x1d, 0x15, 0x05, 0xb2, 0x00, 0x27, 0x15, 0x04, 0x10, 0x10, 0x60, 0xb6, 0x00,
            0x33, 0x03, 0xb8, 0x00, 0x29, 0x84, 0x04, 0x01, 0x84, 0x05, 0x0f, 0xa7, 0xff, 0xe0, 0xb2,
        ]);
        draw[137..149].copy_from_slice(&[0xb2, 0x00, 0x1d, 0x10, 0x18, 0x60, 0x08, 0x60, 0x10, 0x08, 0x60, 0x3e]);
        draw[482..488].copy_from_slice(&[0x2b, 0x05, 0xb8, 0x00, 0x3d, 0xb1]);
        let mut keys = vec![0u8; 335];
        keys[..11].copy_from_slice(&[0x2a, 0x1b, 0x1c, 0xb6, 0x00, 0x40, 0x99, 0x00, 0x05, 0x04, 0xac]);
        classfile::tests::synth_class_n("bn", &[("<init>", "(Lo;Z)V", &init), ("a", "(Ldd;)V", &draw), ("a", "(II)Z", &keys)])
    }

    #[test]
    fn extra_options_makes_all_eight_edits() {
        let class = synth_bn(7);
        let out = extra_options(&class).unwrap();
        let code = |m: &str, d: &str| out[classfile::method_code(&out, m, d).unwrap()].to_vec();
        let (init, draw, keys) = (code("<init>", "(Lo;Z)V"), code("a", "(Ldd;)V"), code("a", "(II)Z"));
        assert_eq!(&init[2..4], &[0x10, hooks::OPTIONS_ROWS]);
        assert_eq!(&draw[104..106], &[0x10, hooks::OPTIONS_SLOTS]);
        for (this_at, call_at) in [(70, 75), (113, 121)] {
            assert_eq!(&draw[this_at..this_at + 3], &[0x2a, 0, 0], "aload_0 in place of the text table");
            assert_eq!(callee(&out, "a", "(Ldd;)V", call_at), hook(hooks::OPT_LABEL));
        }
        assert_eq!(&draw[137..140], &[0x2a, 0x2b, 0x1c]);
        assert_eq!(callee(&out, "a", "(Ldd;)V", 140), hook(hooks::OPT_VALUES));
        assert_eq!(&draw[143..146], &[0x59, 0x3e, 0x9b], "dup; istore_3; iflt");
        assert_eq!(145 + i16::from_be_bytes([draw[146], draw[147]]) as usize, 482, "the method's tail");
        assert_eq!(&draw[482..488], &[0x2b, 0x05, 0xb8, 0x00, 0x3d, 0xb1]);
        assert_eq!(callee(&out, "a", "(II)Z", 3), hook(hooks::OPT_KEY));
        assert_eq!(&keys[6..11], &[0x99, 0x00, 0x05, 0x04, 0xac], "still: consumed -> return true");
        // Both label sites and the title share one Methodref.
        assert_eq!(&draw[76..78], &draw[122..124]);
        assert_eq!(extra_options(&out), None, "never applies twice");
    }

    #[test]
    fn extra_options_is_all_or_nothing() {
        assert_eq!(extra_options(&synth_bn(6)), None, "one site off -> the original class, not a half-patched one");
    }

    #[test]
    fn rewrites_of_one_class_chain() {
        fn first(b: &[u8]) -> Option<Vec<u8>> {
            Some([b, b"1"].concat())
        }
        fn second(b: &[u8]) -> Option<Vec<u8>> {
            b.ends_with(b"1").then(|| [b, b"2"].concat())
        }
        fn refuse(_: &[u8]) -> Option<Vec<u8>> {
            None
        }
        // md5("abc") for all three.
        const MD5: &str = "900150983cd24fb0d6963f7d28e17f72";
        static TEST: [Patch; 3] = [
            Patch { id: "HZ-910", title: "t", evidence: "e", builds: &[Build::Final], default_on: true, kind: PatchKind::ClassRewrite { class: "x", class_md5: MD5, rewrite: first } },
            Patch { id: "HZ-911", title: "t", evidence: "e", builds: &[Build::Final], default_on: true, kind: PatchKind::ClassRewrite { class: "x", class_md5: MD5, rewrite: refuse } },
            Patch { id: "HZ-912", title: "t", evidence: "e", builds: &[Build::Final], default_on: true, kind: PatchKind::ClassRewrite { class: "x", class_md5: MD5, rewrite: second } },
        ];
        let p = Patches { build: Build::Final, chosen: TEST.iter().collect(), log: Arc::default() };
        assert_eq!(p.class_transformer().unwrap()(b"abc"), Some(b"abc12".to_vec()), "the second sees the first's output");
        let r = p.report();
        assert!(r[0].ends_with("rewritten") && r[1].contains("REFUSED") && r[2].ends_with("rewritten"), "{r:?}");
    }

    #[test]
    fn unknown_jar_is_not_a_build() {
        assert_eq!(Build::identify(b"not a jar"), None);
    }

    #[test]
    fn selection_parsing() {
        assert_eq!(Selection::parse("").unwrap(), Selection::Default);
        assert_eq!(Selection::parse("none").unwrap(), Selection::None);
        assert_eq!(Selection::parse("hz-001").unwrap(), Selection::Only(vec!["HZ-001".into()]));
        assert!(Selection::parse("HZ-999").is_err(), "a typo must not become 'no patch'");
        // The default set plus one that is off by default (the dev lever, e.g. HZ-009).
        let more = Selection::parse("default, hz-009").unwrap();
        for p in PATCHES {
            assert_eq!(more.wants_id(p.id), p.default_on || p.id == "HZ-009", "{}", p.id);
        }
        assert!(!Selection::Default.wants_id("HZ-009") && Selection::Default.wants_id("HZ-013"));
        assert!(!Selection::Default.wants_id("HZ-999"), "an unknown id is never wanted");
        assert!(Selection::parse("default,HZ-999").is_err());
        // One out of the default set (the A/B of one patch).
        let less = Selection::parse("default,-hz-028").unwrap();
        for p in PATCHES {
            assert_eq!(less.wants_id(p.id), p.default_on && p.id != "HZ-028", "{}", p.id);
        }
        assert!(Selection::parse("-HZ-999").is_err(), "an unknown id, out or in");
        assert_eq!(Selection::parse("-hz-013").unwrap(), Selection::Only(vec![]), "nothing to take it from");
    }

    #[test]
    fn class_rewrite_only_meets_its_own_bytes() {
        fn upper(b: &[u8]) -> Option<Vec<u8>> {
            Some(b.to_ascii_uppercase())
        }
        fn refuse(_: &[u8]) -> Option<Vec<u8>> {
            None
        }
        static TEST: [Patch; 2] = [
            Patch {
                id: "HZ-900",
                title: "t",
                evidence: "e",
                builds: &[Build::Final],
                default_on: true,
                // md5("abc")
                kind: PatchKind::ClassRewrite { class: "x", class_md5: "900150983cd24fb0d6963f7d28e17f72", rewrite: upper },
            },
            Patch {
                id: "HZ-901",
                title: "t",
                evidence: "e",
                builds: &[Build::Final],
                default_on: true,
                // md5("def")
                kind: PatchKind::ClassRewrite { class: "y", class_md5: "4ed9407630eb1000c0f6b63842defa7d", rewrite: refuse },
            },
        ];
        let p = Patches { build: Build::Final, chosen: TEST.iter().collect(), log: Arc::default() };
        let t = p.class_transformer().unwrap();
        assert_eq!(t(b"abc"), Some(b"ABC".to_vec()));
        assert_eq!(t(b"abd"), None, "other bytes pass through");
        assert_eq!(t(b"def"), None, "a refusing rewrite keeps the original");
        let r = p.report();
        assert!(r[0].ends_with("rewritten") && r[1].contains("REFUSED"), "{r:?}");
    }

    /// HZ-039 .. HZ-047 (2026-10-29): each rewrite on a synthetic class carrying the site's real bytes.
    #[test]
    fn balance_sites_call_their_hooks() {
        let mut load = vec![0u8; 118];
        load[2..5].copy_from_slice(&[0xb8, 0x00, 0x24]);
        let out = tables_db(&classfile::tests::synth_class("db", "b", "(B)V", &load)).unwrap();
        assert_eq!(callee(&out, "b", "(B)V", 2), hook(hooks::TABLE));
        assert_eq!(tables_db(&out), None, "never applies twice");
        let mut make = vec![0u8; 306];
        make[34..37].copy_from_slice(&[0xb8, 0x01, 0x7e]);
        let out = tables_ek(&classfile::tests::synth_class("ek", "a", "(B)Lek;", &make)).unwrap();
        assert_eq!(callee(&out, "a", "(B)Lek;", 34), hook(hooks::TABLE));
        let mut spirit = vec![0u8; 27];
        spirit[3..6].copy_from_slice(&[0xb8, 0x00, 0x7f]);
        spirit[20..23].copy_from_slice(&[0xb8, 0x00, 0x7f]);
        let out = tables_p(&classfile::tests::synth_class("p", "f", "()V", &spirit)).unwrap();
        assert_eq!((callee(&out, "f", "()V", 3), callee(&out, "f", "()V", 20)), (hook(hooks::TABLE), hook(hooks::TABLE)));

        let mut i = vec![0u8; 1237];
        i[582..585].copy_from_slice(&[0xb5, 0x00, 0x7f]);
        i[817..820].copy_from_slice(&[0xb5, 0x00, 0x5d]);
        i[1082..1085].copy_from_slice(&[0xb6, 0x01, 0x70]);
        let mut g = vec![0u8; 182];
        g[171..174].copy_from_slice(&[0xb7, 0x01, 0x1d]);
        g[178..181].copy_from_slice(&[0xb7, 0x01, 0x1d]);
        let mut h = vec![0u8; 125];
        h[114..117].copy_from_slice(&[0xb7, 0x01, 0x1d]);
        h[121..124].copy_from_slice(&[0xb7, 0x01, 0x1d]);
        let mut tick = vec![0u8; 397];
        tick[163..165].copy_from_slice(&[0x10, 100]);
        tick[189..191].copy_from_slice(&[0x10, 100]);
        let ek = classfile::tests::synth_class_n("ek", &[("i", "()V", &i), ("G", "()V", &g), ("H", "()V", &h), ("c", "()V", &tick)]);
        let out = stat_formulas(&ek).unwrap();
        assert_eq!(callee(&out, "i", "()V", 582), hook(hooks::ATTACK));
        assert_eq!(callee(&out, "i", "()V", 817), hook(hooks::DEFENCE));
        assert_eq!(callee(&out, "i", "()V", 1082), hook(hooks::HP_BONUS));
        assert_eq!((callee(&out, "G", "()V", 171), callee(&out, "G", "()V", 178)), (hook(hooks::CRIT), hook(hooks::CRIT)));
        assert_eq!((callee(&out, "H", "()V", 114), callee(&out, "H", "()V", 121)), (hook(hooks::EVASION), hook(hooks::EVASION)));
        let c = &out[classfile::method_code(&out, "c", "()V").unwrap()];
        assert_eq!((&c[163..165], &c[189..191]), (&[0x10, 60][..], &[0x10, 60][..]));
        assert_eq!(stat_formulas(&out), None, "never applies twice");

        let mut roll = vec![0u8; 59];
        roll[28..30].copy_from_slice(&[0x10, 15]);
        roll[33..35].copy_from_slice(&[0x10, 15]);
        let out = hit_floor(&classfile::tests::synth_class("cq", "a", "(Lar;)Z", &roll)).unwrap();
        let c = &out[classfile::method_code(&out, "a", "(Lar;)Z").unwrap()];
        assert_eq!((&c[28..30], &c[33..35]), (&[0x10, 30][..], &[0x10, 30][..]), "the compare and the value it stores");

        let site = |len: usize, ats: &[usize]| {
            let mut v = vec![0u8; len];
            for &at in ats {
                v[at..at + 3].copy_from_slice(&[0xb4, 0x00, 0x23]);
            }
            v
        };
        let (draw, touch, raise_a, raise_b) = (site(260, &[90]), site(260, &[90, 142]), site(190, &[18, 98]), site(161, &[18, 82]));
        let cb = classfile::tests::synth_class_n("cb", &[("a", "(Ldd;IIBZ)V", &draw), ("a", "(ZB)Z", &touch), ("a", "(BBBBZ)V", &raise_a), ("b", "(BBBBZ)V", &raise_b)]);
        let out = level_cap(&cb).unwrap();
        for (m, d, at) in [("a", "(Ldd;IIBZ)V", 90), ("a", "(ZB)Z", 90), ("a", "(ZB)Z", 142), ("a", "(BBBBZ)V", 18), ("a", "(BBBBZ)V", 98), ("b", "(BBBBZ)V", 18), ("b", "(BBBBZ)V", 82)] {
            assert_eq!(callee(&out, m, d, at), hook(hooks::LEVEL_CAP), "{m}{d} @{at}");
        }
        assert_eq!(level_cap(&out), None, "never applies twice");

        let mut hit = vec![0u8; 400];
        hit[103..106].copy_from_slice(&[0xb8, 0x00, 0xc3]);
        let cq = classfile::tests::synth_class("cq", "a", "(Lek;BIZBBZZZ)Z", &hit);
        let alone = combo_sp(&cq).unwrap();
        assert_eq!(callee(&alone, "a", "(Lek;BIZBBZZZ)Z", 103), hook(hooks::HIT_DAMAGE));
        assert_eq!(combo_sp(&alone).as_deref(), Some(&alone[..]), "HZ-032's hook there: kept");
        let mut death = vec![0u8; 60];
        death[22..25].copy_from_slice(&[0xb6, 0x00, 0xdd]);
        let cq = classfile::tests::synth_class("cq", "g", "()V", &death);
        let alone = boss_drops(&cq).unwrap();
        assert_eq!(callee(&alone, "g", "()V", 22), hook(hooks::MONSTER_DROP));
        assert_eq!(boss_drops(&alone).as_deref(), Some(&alone[..]), "HZ-031's hook there: kept");

        let mut level = vec![0u8; 207];
        level[109..129].copy_from_slice(&[0x2a, 0xb4, 0x00, 0x8d, 0x05, 0x70, 0x9a, 0x00, 0x0e, 0x2a, 0x59, 0xb4, 0x00, 0x6b, 0x04, 0x60, 0x93, 0xb5, 0x00, 0x6b]);
        let out = skill_point_curve(&classfile::tests::synth_class("ek", "j", "(I)V", &level)).unwrap();
        assert_eq!(callee(&out, "j", "(I)V", 110), hook(hooks::SKILL_POINTS));
        let j = &out[classfile::method_code(&out, "j", "(I)V").unwrap()];
        assert_eq!((j[109], &j[113..129]), (0x2a, &[0u8; 16][..]));

        let mut part = vec![0u8; 24];
        part[20..23].copy_from_slice(&[0xb6, 0x00, 0x34]);
        let out = boss_reflection(&classfile::tests::synth_class("ak", "a", "(Lar;I)V", &part)).unwrap();
        assert_eq!(callee(&out, "a", "(Lar;I)V", 20), hook(hooks::BOSS_REFLECT));

        let mut sword = vec![0u8; 62];
        sword[18..21].copy_from_slice(&[0xa4, 0x00, 0x2a]);
        let out = death_sword(&classfile::tests::synth_class("p", "a", "(Lcq;)Z", &sword)).unwrap();
        assert_eq!(&out[classfile::method_code(&out, "a", "(Lcq;)Z").unwrap()][18..21], &[0xa3, 0x00, 0x2a]);
        assert_eq!(death_sword(&out), None, "never applies twice");
        let mut taken = vec![0u8; 393];
        taken[63..72].copy_from_slice(&[0x1c, 0x2a, 0xb4, 0x00, 0x94, 0x10, 0x0a, 0x6c, 0x68]);
        let mut status = vec![0u8; 14];
        status[5..8].copy_from_slice(&[0xb6, 0x00, 0xc1]);
        let ek = classfile::tests::synth_class_n("ek", &[("a", "(Lar;IB)Z", &taken), ("b", "(BB)V", &status)]);
        let out = guard_percent(&ek).unwrap();
        assert_eq!(callee(&out, "a", "(Lar;IB)Z", 68), hook(hooks::GUARD));
        let a = &out[classfile::method_code(&out, "a", "(Lar;IB)Z").unwrap()];
        assert_eq!((&a[63..68], a[71]), (&[0x1c, 0x2a, 0xb4, 0x00, 0x94][..], 0));
        assert_eq!(callee(&out, "b", "(BB)V", 5), hook(hooks::GUARD_STATUS));
        assert_eq!(guard_percent(&out), None, "never applies twice");
        assert_eq!(hooks::HIT_FLOOR, 30);
    }

    /// HZ-057 (2026-10-29): a monster's picture and move, and the player's blow on a boss, call the hooks.
    #[test]
    fn reflect_stance_calls_its_hooks() {
        let mut draw = vec![0u8; 224];
        draw[204..210].copy_from_slice(&[0x2a, 0x1b, 0x1c, 0xb6, 0x00, 0xc4]);
        let mut think = vec![0u8; 248];
        think[127..131].copy_from_slice(&[0x2a, 0xb6, 0x00, 0xd9]);
        let cq = classfile::tests::synth_class_n("cq", &[("b", "(II)V", &draw), ("l", "()V", &think)]);
        let out = stance_monster(&cq).unwrap();
        assert_eq!(callee(&out, "b", "(II)V", 207), hook(hooks::MONSTER_DRAW));
        assert_eq!(callee(&out, "l", "()V", 128), hook(hooks::MONSTER_THINK));
        assert_eq!(stance_monster(&out), None, "never applies twice");

        let mut blow = vec![0u8; 57];
        blow[23..26].copy_from_slice(&[0xb7, 0x00, 0x39]);
        blow[53..56].copy_from_slice(&[0xb7, 0x00, 0x39]);
        let out = stance_boss_hit(&classfile::tests::synth_class("ak", "a", "(Lek;BIZBBZZZ)Z", &blow)).unwrap();
        assert_eq!(callee(&out, "a", "(Lek;BIZBBZZZ)Z", 23), hook(hooks::BOSS_HIT));
        assert_eq!(callee(&out, "a", "(Lek;BIZBBZZZ)Z", 53), hook(hooks::BOSS_HIT));
        assert_eq!(stance_boss_hit(&out), None, "never applies twice");
    }

    /// HZ-058 (2026-10-30): the skill and potion popups' registration rows call the plates' hooks.
    #[test]
    fn quick_registration_calls_the_plates_hooks() {
        let mut c = vec![0u8; 195];
        c[143..146].copy_from_slice(&[0xb8, 0x00, 0x4d]);
        c[180..183].copy_from_slice(&[0xb8, 0x00, 0x4d]);
        let out = quick_register(&classfile::tests::synth_class("cb", "a", "(BBBBZ)V", &c)).unwrap();
        assert_eq!(callee(&out, "a", "(BBBBZ)V", 143), hook(hooks::QUICK_REGISTER));
        assert_eq!(callee(&out, "a", "(BBBBZ)V", 180), hook(hooks::QUICK_REGISTER));
        assert_eq!(quick_register(&out), None, "never applies twice");
        let mut d = vec![0u8; 352];
        d[72..75].copy_from_slice(&[0xb8, 0x00, 0x31]);
        let out = quick_potion(&classfile::tests::synth_class("ds", "a", "(BB)V", &d)).unwrap();
        assert_eq!(callee(&out, "a", "(BB)V", 72), hook(hooks::QUICK_POTION));
        assert_eq!(quick_potion(&out), None, "never applies twice");
    }

    /// HZ-059 (2026-10-30): the field's arrows, the camera's follow and the facing call their hooks.
    #[test]
    fn diagonal_walk_calls_its_hooks() {
        let mut a = vec![0u8; 844];
        for (i, at) in [513, 520, 527, 534].into_iter().enumerate() {
            a[at - 1] = 0x04 + i as u8; // iconst_1..4
            a[at..at + 3].copy_from_slice(&[0xb8, 0x00, 0xcd]);
        }
        let mut d = vec![0u8; 37];
        d[12..17].copy_from_slice(&[0x04, 0x04, 0xb8, 0x00, 0xcb]);
        let ce = classfile::tests::synth_class_n("ce", &[("a", "(II)Z", &a), ("d", "()V", &d)]);
        let out = diagonal_field(&ce).unwrap();
        for at in [513, 520, 527, 534] {
            assert_eq!(callee(&out, "a", "(II)Z", at), hook(hooks::WALK));
            assert_eq!(out[classfile::method_code(&out, "a", "(II)Z").unwrap()][at - 1], a[at - 1], "the arrow stays");
        }
        assert_eq!(callee(&out, "d", "()V", 14), hook(hooks::CAMERA));
        assert_eq!(diagonal_field(&out), None, "never applies twice");
        let b = [0xb2, 0x00, 0x45, 0x1a, 0xb6, 0x00, 0x8c, 0xb2, 0x00, 0x45, 0x1b, 0xb6, 0x00, 0x8d, 0xb1];
        let out = diagonal_facing(&classfile::tests::synth_class("ed", "b", "(BB)V", &b)).unwrap();
        assert_eq!(callee(&out, "b", "(BB)V", 11), hook(hooks::FACE));
        let code = &out[classfile::method_code(&out, "b", "(BB)V").unwrap()];
        assert_eq!(&code[..11], &b[..11], "the state set before it stays");
        assert_eq!(diagonal_facing(&out), None, "never applies twice");
        let e = [
            0x2a, 0xb4, 0x00, 0x1d, 0x9a, 0x00, 0x0a, 0x2a, 0xb4, 0x00, 0x1e, 0x99, 0x00, 0x05, 0x03, 0xac, 0xb2, 0x00, 0x1a, 0x2a, 0x2a, 0xb4,
            0x00, 0x12, 0xb6, 0x00, 0x42, 0x9a, 0x00, 0x0a, 0x2a, 0x04, 0xb6, 0x00, 0x2c, 0x04, 0xac, 0x03, 0xac,
        ];
        let f = [0x2a, 0x2a, 0xb4, 0x00, 0x12, 0xb6, 0x00, 0x31, 0xac];
        let ar = classfile::tests::synth_class_n("ar", &[("e", "()Z", &e), ("f", "()Z", &f)]);
        let out = diagonal_step(&ar).unwrap();
        assert_eq!(callee(&out, "e", "()Z", 24), hook(hooks::PASSABLE));
        assert_eq!(callee(&out, "f", "()Z", 5), hook(hooks::SLIDE));
        assert_eq!(diagonal_step(&out), None, "never applies twice");
        let p = [0x2a, 0xb2, 0x00, 0xac, 0x1b, 0x91, 0x2a, 0xb4, 0x00, 0x3f, 0x2a, 0xb4, 0x00, 0x40, 0x1c, 0x1d, 0xb6, 0x01, 0x82, 0xb5, 0x00, 0x40, 0xb1];
        let out = diagonal_picture(&classfile::tests::synth_class("ek", "b", "(III)V", &p)).unwrap();
        assert_eq!(callee(&out, "b", "(III)V", 7), hook(hooks::DRAW_FACING));
        assert_eq!(diagonal_picture(&out), None, "never applies twice");
    }

    /// HZ-060 (2026-10-30): the shop's special tab reaches the elixir after HZ-016's orb, and the drink
    /// reads the potion's number through the hook.
    #[test]
    fn elixir_sites_call_their_hooks() {
        let mut h = vec![0u8; 421];
        h[312..322].copy_from_slice(&[0x9a, 0x00, 0x0b, 0x10, 0x14, 0x10, 0x08, 0xb8, 0x00, 0x4a]);
        let co = classfile::tests::synth_class("co", "h", "()V", &h);
        assert_eq!(elixir_shop(&co), None, "the file's own 8: no orb before it, no elixir");
        let out = elixir_shop(&revive_orb_shop(&co).unwrap()).unwrap();
        assert_eq!(&out[classfile::method_code(&out, "h", "()V").unwrap()][312..322], &[0x9a, 0x00, 0x0b, 0x10, 0x14, 0x10, 0x0a, 0xb8, 0x00, 0x4a]);
        assert_eq!(elixir_shop(&out), None, "never applies twice");
        let mut d = vec![0u8; 211];
        d[58..70].copy_from_slice(&[0xa7, 0x00, 0x97, 0x2c, 0xb4, 0x00, 0x45, 0x59, 0x3e, 0xaa, 0x00, 0x00]);
        let out = elixir_drink(&classfile::tests::synth_class("ek", "a", "(Z)Z", &d)).unwrap();
        assert_eq!(callee(&out, "a", "(Z)Z", 62), hook(hooks::POTION));
        let code = &out[classfile::method_code(&out, "a", "(Z)Z").unwrap()];
        assert_eq!((code[61], code[65], code[66]), (0x2c, 0x59, 0x3e), "the item loaded before it, the number's dup · istore_3 after");
        assert_eq!(elixir_drink(&out), None, "never applies twice");
        assert_eq!((hooks::ELIXIR_ICON, hooks::ELIXIR_DRINK), (30, 2));

        // 2026-11-02: a quick slot's cooldown read (ed.g(I)V @39) goes through the spirit hook — put
        // there when HZ-032 is off, kept when it already is.
        let mut cooldown = vec![0u8; 67];
        cooldown[35..44].copy_from_slice(&[0xb6, 0x00, 0xc7, 0x1b, 0xb6, 0x01, 0x01, 0x4f, 0xb2]);
        let ed = classfile::tests::synth_class("ed", "g", "(I)V", &cooldown);
        let alone = elixir_cooldown(&ed).unwrap();
        assert_eq!(callee(&alone, "g", "(I)V", 39), hook(hooks::SPIRIT_COOLDOWN));
        let g = &alone[classfile::method_code(&alone, "g", "(I)V").unwrap()];
        assert_eq!((&g[35..39], g[42]), (&[0xb6, 0x00, 0xc7, 0x1b][..], 0x4f), "the spirit's getter, the number and the store kept");
        assert_eq!(elixir_cooldown(&alone).as_deref(), Some(&alone[..]), "a hook there already: kept");
        assert_eq!(elixir_cooldown(&effects_ed_only_spirit(&ed)).map(|c| callee(&c, "g", "(I)V", 39)), Some(hook(hooks::SPIRIT_COOLDOWN)), "HZ-032's: kept");
    }

    /// The spirit half of HZ-032's `ed` rewrite (its job half needs the bytes at 26 too).
    fn effects_ed_only_spirit(class: &[u8]) -> Vec<u8> {
        call_hook(class, "g", "(I)V", 39, &[0xb6, 0x01, 0x01], hooks::SPIRIT_COOLDOWN, invokestatic).unwrap()
    }

    /// HZ-062 (2026-11-02): both sales in `co.a(BB)V` hand the chosen item to the hook in place of its
    /// category — the bag, the item, the number and the count on the stack.
    #[test]
    fn a_sale_takes_the_chosen_item() {
        let mut a = vec![0u8; 532];
        a[338..356].copy_from_slice(&[0x2a, 0xb4, 0x00, 0x25, 0xb4, 0x00, 0x1c, 0x2a, 0xb4, 0x00, 0x25, 0xb4, 0x00, 0x1e, 0x04, 0xb6, 0x00, 0x40]);
        a[452..470].copy_from_slice(&[0x2a, 0xb4, 0x00, 0x25, 0xb4, 0x00, 0x1c, 0x2a, 0xb4, 0x00, 0x25, 0xb4, 0x00, 0x1e, 0x1c, 0xb6, 0x00, 0x40]);
        let out = sell_chosen(&classfile::tests::synth_class("co", "a", "(BB)V", &a)).unwrap();
        assert_eq!((callee(&out, "a", "(BB)V", 353), callee(&out, "a", "(BB)V", 467)), (hook(hooks::SELL_TAKE), hook(hooks::SELL_TAKE)));
        let code = &out[classfile::method_code(&out, "a", "(BB)V").unwrap()];
        for (at, n) in [(338, 0x04), (452, 0x1c)] {
            assert_eq!(&code[at..at + 4], &[0x2a, 0xb4, 0x00, 0x25], "the chosen item loaded");
            assert_eq!(&code[at + 4..at + 7], &[0, 0, 0], "its category no longer read");
            assert_eq!(&code[at + 7..at + 15], &[0x2a, 0xb4, 0x00, 0x25, 0xb4, 0x00, 0x1e, n], "its number and the count after it");
        }
        assert_eq!(sell_chosen(&out), None, "never applies twice");
        assert_eq!(hooks::SELL_TAKE.descriptor, "(Ljava/lang/Object;Ljava/lang/Object;BI)Z", "bt.b(BBI)Z's stack with the item for the category");
    }

    /// HZ-066 (2026-11-03): 계정창고's 「넣기」 (`eg.a(BB)V`) tests and adds through the hooks — the page
    /// the game picked by the character side's tab (@66 / @126) is still loaded and handed over; the
    /// HZ-068: the stat name reads the hook's sheet.
    #[test]
    fn the_stat_names_go_through_the_hook() {
        let dy = classfile::tests::synth_class("dy", "a", "(Ldd;IIB)V", &code_with(59, &[(19, &[0xb2, 0x00, 0x26])]));
        let out = stat_names(&dy).unwrap();
        assert_eq!(callee(&out, "a", "(Ldd;IIB)V", 19), hook(hooks::STAT_NAMES));
        assert_eq!(stat_names(&out), None, "never applies twice");
        assert_eq!(hooks::STAT_NAMES.descriptor, "()Ljava/lang/Object;", "getstatic's stack: one reference");
    }

    /// HZ-067: both calls that take a monster's HP go through the hook, with the game's stack.
    #[test]
    fn a_monsters_hurt_goes_through_the_hook() {
        let hurt = [0xb6, 0x00, 0xc6];
        let (hit, tick) = (code_with(125, &[(84, &hurt)]), code_with(112, &[(103, &hurt)]));
        let cq = classfile::tests::synth_class_n("cq", &[("a", "(Lar;IBB)V", &hit), ("b", "(Lbc;)V", &tick)]);
        let out = monster_hurt(&cq).unwrap();
        assert_eq!((callee(&out, "a", "(Lar;IBB)V", 84), callee(&out, "b", "(Lbc;)V", 103)), (hook(hooks::MONSTER_HURT), hook(hooks::MONSTER_HURT)));
        assert_eq!(monster_hurt(&out), None, "never applies twice");
        assert_eq!(hooks::MONSTER_HURT.descriptor, "(Ljava/lang/Object;Ljava/lang/Object;I)V", "cq.a(Lar;I)V's stack with the monster first");
    }

    /// window's other takes and the 계정창고 side's are left. Chains on HZ-036's `eg`.
    #[test]
    fn a_deposit_picks_its_storage_page() {
        let page = [0xb2, 0x00, 0x1d, 0x2a, 0xb4, 0x00, 0x23, 0x32];
        let eg = classfile::tests::synth_class(
            "eg",
            "a",
            "(BB)V",
            &code_with(500, &[(66, &page), (95, &[0xb6, 0x00, 0x35]), (126, &page), (145, &[0xb6, 0x00, 0x38]), (162, &[0xb6, 0x00, 0x37]), (384, &page), (396, &[0xb6, 0x00, 0x37])]),
        );
        for class in [eg.clone(), bag_pages_eg_answers_only(&eg)] {
            let out = storage_deposit(&class).unwrap();
            assert_eq!((callee(&out, "a", "(BB)V", 95), callee(&out, "a", "(BB)V", 145)), (hook(hooks::STORAGE_ROOM), hook(hooks::STORAGE_PUT)));
            let code = &out[classfile::method_code(&out, "a", "(BB)V").unwrap()];
            for at in [66, 126, 384] {
                assert_eq!(code[at..at + 8], page, "@{at}: the game's page still on the stack");
            }
            assert_eq!(code[396..399], [0xb6, 0x00, 0x37], "꺼내기 takes from the page it shows: the game's");
            assert_eq!(storage_deposit(&out), None, "never applies twice");
        }
        assert_eq!(hooks::STORAGE_ROOM.descriptor, "(Ljava/lang/Object;BBI)Z", "bt.a(BBI)Z's stack with the page first");
        assert_eq!(hooks::STORAGE_PUT.descriptor, "(Ljava/lang/Object;Ljava/lang/Object;I)I", "bt.a(Lba;I)I's");
    }

    /// HZ-036's takes on `eg.a(BB)V` alone (its other sites are in methods the synthetic class lacks).
    fn bag_pages_eg_answers_only(class: &[u8]) -> Vec<u8> {
        call_hook(class, "a", "(BB)V", 162, &[0xb6, 0x00, 0x37], hooks::BAG_TAKE, invokestatic).unwrap()
    }

    /// HZ-056 (2026-10-29): the spirit cast's end in `ek.C()V` calls its hook.
    #[test]
    fn spirit_cast_end_calls_its_hook() {
        let mut c = vec![0u8; 733];
        c[716..721].copy_from_slice(&[0x2a, 0xb7, 0x01, 0x09, 0xb1]);
        let out = spirit_cast_end(&classfile::tests::synth_class("ek", "C", "()V", &c)).unwrap();
        assert_eq!(callee(&out, "C", "()V", 717), hook(hooks::SPIRIT_CAST_END));
        let code = &out[classfile::method_code(&out, "C", "()V").unwrap()];
        assert_eq!((code[716], code[720]), (0x2a, 0xb1), "this and the return stay");
        assert_eq!(spirit_cast_end(&out), None, "never applies twice");
    }

    /// HZ-063 (2026-11-03): condition 7's flag test (`eb.a(BLn;)Z` @421) and action 53's 'on'
    /// (`eb.b(BLn;)Z` @2836) call their hooks.
    #[test]
    fn story_flag_sites_call_their_hooks() {
        let mut a = vec![0u8; 506];
        // sipush 255; iand; invokestatic #260 ed.a(I)Z; ifne
        a[417..425].copy_from_slice(&[0x11, 0x00, 0xff, 0x7e, 0xb8, 0x01, 0x04, 0x9a]);
        let mut b = vec![0u8; 3053];
        // iload_1; invokestatic #268 ed.c(I)V; invokestatic #238 dw.b()V
        b[2835..2842].copy_from_slice(&[0x1b, 0xb8, 0x01, 0x0c, 0xb8, 0x00, 0xee]);
        let eb = classfile::tests::synth_class_n("eb", &[("a", "(BLn;)Z", &a), ("b", "(BLn;)Z", &b)]);
        let out = karmen_flag(&eb).unwrap();
        assert_eq!(callee(&out, "a", "(BLn;)Z", 421), hook(hooks::SCRIPT_FLAG));
        assert_eq!(callee(&out, "b", "(BLn;)Z", 2836), hook(hooks::SCRIPT_FLAG_ON));
        let a = &out[classfile::method_code(&out, "a", "(BLn;)Z").unwrap()];
        assert_eq!((&a[417..421], a[424]), (&[0x11, 0x00, 0xff, 0x7e][..], 0x9a), "the flag's number before, the test after");
        let b = &out[classfile::method_code(&out, "b", "(BLn;)Z").unwrap()];
        assert_eq!((b[2835], &b[2839..2842]), (0x1b, &[0xb8, 0x00, 0xee][..]), "the flag loaded before, dw.b() after");
        assert_eq!(karmen_flag(&out), None, "never applies twice");
        assert_eq!((hooks::SCRIPT_FLAG.descriptor, hooks::SCRIPT_FLAG_ON.descriptor), ("(I)Z", "(I)V"), "ed.a(I)Z's and ed.c(I)V's stacks");
    }

    /// HZ-064 (2026-11-03): 조준사격's (@330) and 저격's (@891) hold on their target call the hook — with
    /// HZ-032's 저격 chance (@953) in the same method.
    #[test]
    fn aim_holds_call_their_hook() {
        let mut a = vec![0u8; 1044];
        for at in [324, 885] {
            // aload_1; getfield #16 ai.b:Lcq; bipush 14; invokevirtual #52 l.b(I)V; aload_1
            a[at..at + 10].copy_from_slice(&[0x2b, 0xb4, 0x00, 0x10, 0x10, 0x0e, 0xb6, 0x00, 0x34, 0x2b]);
        }
        a[953..956].copy_from_slice(&[0xb6, 0x00, 0x27]);
        let ai = classfile::tests::synth_class("ai", "a", "(BI)Z", &a);
        for class in [ai.clone(), effects_ai(&ai).unwrap()] {
            let out = aim_hold(&class).unwrap();
            for at in [330, 891] {
                assert_eq!(callee(&out, "a", "(BI)Z", at), hook(hooks::AIM_HOLD));
                let code = &out[classfile::method_code(&out, "a", "(BI)Z").unwrap()];
                assert_eq!(&code[at - 6..at], &[0x2b, 0xb4, 0x00, 0x10, 0x10, 0x0e], "the target and its 14 frames before");
            }
            assert_eq!(aim_hold(&out), None, "never applies twice");
        }
        assert_eq!(callee(&aim_hold(&effects_ai(&ai).unwrap()).unwrap(), "a", "(BI)Z", 953), hook(hooks::SNIPE_CHANCE), "HZ-032's kept");
        assert_eq!(hooks::AIM_HOLD.descriptor, "(Ljava/lang/Object;I)V", "l.b(I)V's stack: the target, the frames");
    }

    /// HZ-065 (2026-11-03): action 16's give (`eb.b(BLn;)Z` @1045) goes through the reward hook — put there
    /// when HZ-034 is off, HZ-034's kept when it is on.
    #[test]
    fn a_reward_give_calls_the_hook_with_or_without_hz034() {
        let mut b = vec![0u8; 3053];
        b[1021..1024].copy_from_slice(&[0xb8, 0x00, 0xb4]);
        b[1045..1048].copy_from_slice(&[0xb6, 0x00, 0xbe]);
        let eb = classfile::tests::synth_class("eb", "b", "(BLn;)Z", &b);
        let alone = reward_overflow(&eb).unwrap();
        assert_eq!(callee(&alone, "b", "(BLn;)Z", 1045), hook(hooks::QUEST_GIVE));
        assert_eq!(&alone[classfile::method_code(&alone, "b", "(BLn;)Z").unwrap()][1021..1024], &[0xb8, 0x00, 0xb4], "HZ-034's item site untouched");
        assert_eq!(reward_overflow(&alone).as_deref(), Some(&alone[..]), "a hook there already: kept");
        let theirs = guardian_quest_items(&eb).unwrap();
        assert_eq!(reward_overflow(&theirs).as_deref(), Some(&theirs[..]), "HZ-034's: kept");

        // The save's progress record (ed.k()V @143 — the header's write @27 stays the game's) and the
        // storage's write (dz.i()V @138) call their hooks.
        let mut k = vec![0u8; 151];
        // iload_0; invokestatic #266 rpg/b.c([BB)Z; pop
        k[26..31].copy_from_slice(&[0x1a, 0xb8, 0x01, 0x0a, 0x57]);
        // aload 4; getstatic #114 ed.j:B; invokestatic #261 rpg/b.a([BB)Z; pop; invokestatic #195 ed.l()V; return
        k[138..151].copy_from_slice(&[0x19, 0x04, 0xb2, 0x00, 0x72, 0xb8, 0x01, 0x05, 0x57, 0xb8, 0x00, 0xc3, 0xb1]);
        let ed = reward_slot_save(&classfile::tests::synth_class("ed", "k", "()V", &k)).unwrap();
        assert_eq!(callee(&ed, "k", "()V", 143), hook(hooks::SLOT_SAVE));
        let code = &ed[classfile::method_code(&ed, "k", "()V").unwrap()];
        assert_eq!(&code[26..31], &[0x1a, 0xb8, 0x01, 0x0a, 0x57], "the header's write before it: the game's");
        assert_eq!((&code[138..143], &code[146..151]), (&[0x19, 0x04, 0xb2, 0x00, 0x72][..], &[0x57, 0xb8, 0x00, 0xc3, 0xb1][..]), "the record and the slot before, the answer dropped and the item record after");
        assert_eq!(reward_slot_save(&ed), None, "never applies twice");
        let mut i = vec![0u8; 149];
        // aload_0; invokevirtual toByteArray; invokestatic #68 rpg/b.b([B)Z; pop; return
        i[134..143].copy_from_slice(&[0x2a, 0xb6, 0x00, 0x37, 0xb8, 0x00, 0x44, 0x57, 0xb1]);
        let dz = reward_storage_write(&classfile::tests::synth_class("dz", "i", "()V", &i)).unwrap();
        assert_eq!(callee(&dz, "i", "()V", 138), hook(hooks::STORAGE_WRITE));
        let code = &dz[classfile::method_code(&dz, "i", "()V").unwrap()];
        assert_eq!(&code[134..138], &[0x2a, 0xb6, 0x00, 0x37], "the payload made before");
        assert_eq!(&code[141..143], &[0x57, 0xb1], "its answer dropped, return");
        assert_eq!(reward_storage_write(&dz), None, "never applies twice");
        assert_eq!((hooks::SLOT_SAVE.descriptor, hooks::STORAGE_WRITE.descriptor), ("([BB)Z", "([B)Z"), "rpg/b.c([BB)Z's and rpg/b.b([B)Z's stacks");
    }
}
