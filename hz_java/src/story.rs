//! HZ-061 — a boss's story waits five seconds (user 2026-11-01: 「보스잡고나서 아이템이 드랍되는데
//! 스토리로 바로 넘어가서 아이템을 못먹고있어. 이거 hero2보스레이드 때처럼 카운트다운 5초뒤에 스토리
//! 진행되도록」).
//!
//! Every boss room (121..134) arms one kill counter on entry — its tree 0's condition 0
//! `ed.b(kind, 1)` on the boss's head (`hz_maps`, 2026-11-01: 109 · 110 · 117 · 101 · 115 · 105 · 112 ·
//! 118 · 119 · 114 · 99 · 124 · 128 · 122, all [`crate::balance::BOSS_HEADS`]). The death that spends
//! it starts the room's story at once: `cq.a(B)V` (state 7) → `ed.c(IB)V` → `eb.a(B)Z` with the
//! monster's script group — before the same death's drop (`cq.g()V`: `a(7)`, then `o()`), so the next
//! frame is the dialogue and its warp leaves what fell behind. The arena's counters (투기장 101..109,
//! ten kills) and the tutorial's (kind 0, three) are not bosses and go on as they were.
//!
//! The hook takes that one death: the counter is spent as the game spends it (−1), and the story
//! waits [`WAIT_TICKS`] field ticks — the game's 20 a second (HZ-006's hook), so it stands still under
//! the menu, a 「# 확인」 box or the revive question, as hero2's raid countdown does — then starts
//! through the game's own `eb.a(B)Z`, from the field tick, while the field is up and the player is
//! not falling. The host shows the seconds left in the left notice stack, one line replaced in place
//! (hero2's 「다음 보스로 이동 중...5」). A boss room has no door (no step trigger), and its list 0 turns
//! off saving and the return scroll (actions 45:00 · 57:00, `g.e` / `g.d`), so the wait cannot be
//! left but by a game over — a wait whose map is gone is dropped. What still stands in the room while
//! it waits (거인신's two 돌기둥, a summon) goes without a reward, as the story's warp took it before —
//! hero2's raid clears its stage the same way — so the wait is for picking up, not a new fight.
//!
//! Pure: the hooks feed [`Wait::tick`] the map and whether the story may start.

/// The game's field ticks a second (`rpg/c` 50 ms frames; HZ-057's clock too).
pub const TICKS_PER_SECOND: u16 = 20;
/// How long the story waits.
pub const WAIT_SECONDS: u8 = 5;
pub const WAIT_TICKS: u16 = WAIT_SECONDS as u16 * TICKS_PER_SECOND;

/// Does the death of a monster of `kind`, its counter `ed.ae[kind]` at `counter` before the death,
/// start a boss's story — the last kill a boss room's counter waits for?
pub fn waits(kind: i16, counter: i8) -> bool {
    counter == 1 && crate::balance::is_boss_head(kind)
}

/// A boss's story, waiting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wait {
    /// The script group the game would have started (the dead monster's `eq.f`).
    pub group: i8,
    /// The map it was killed on (`g.b:B`) — the group is that map's.
    pub map: i8,
    /// Field ticks since.
    pub ticks: u16,
}

/// What a field tick does with a [`Wait`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Still waiting; the seconds to show.
    Wait(u8),
    /// Start the story: `eb.a(group)`.
    Go(i8),
    /// The player is on another map (a game over and a load): forget it.
    Drop,
}

impl Wait {
    pub fn new(group: i8, map: i8) -> Wait {
        Wait { group, map, ticks: 0 }
    }

    /// The seconds left as shown: 5 from the kill, … 1 for the last second.
    pub fn seconds(&self) -> u8 {
        let left = WAIT_TICKS.saturating_sub(self.ticks);
        (left.div_ceil(TICKS_PER_SECOND) as u8).max(1)
    }

    /// One field tick on `map`. The story starts once the wait is over and it `may` — the field up and
    /// the player standing (a fall, `ar.b` 6 · 7, is the death's, then the revive question's or the game
    /// over's); until then it waits on at its last second.
    pub fn tick(&mut self, map: i8, may: bool) -> Step {
        if map != self.map {
            return Step::Drop;
        }
        self.ticks = self.ticks.saturating_add(1).min(WAIT_TICKS);
        if self.ticks >= WAIT_TICKS && may {
            return Step::Go(self.group);
        }
        Step::Wait(self.seconds())
    }
}

/// HZ-063: the quest that sends Karmen and Lucian apart — 「정령(2)」, 「출발하기 전에 카르멘을 찾아보자」
/// (map 8's choice from round 1 on: 2 = the Karmen route; 12 = the Elpis route). Its writers (every
/// map script, `hz_maps.py` 2026-11-03): map 8 lists 0 / 1 set it to 1, map 0 list 25 to 2 — never
/// higher, never back (the ending clears every quest with every flag).
pub const KARMEN_ROUTE_QUEST: i32 = 11;
/// HZ-063: 「결전」 — only ever set to 1 (map 2 lists 1 and 4, map 114 list 1), never back.
pub const FINAL_BATTLE_QUEST: i32 = 31;
/// HZ-063: the one map whose script turns flag 48 on as the story — 은거지, list 1, the masked
/// figure's scene. The other writer of 48, turning down 「수상한 남자」, is map 4 (게힌시티) list 34.
pub const KARMEN_AWAY_MAP: i8 = 2;

/// HZ-063, the write: is the script turning flag 48 ([`hz_patches::hooks::KARMEN_AWAY`], Karmen
/// spent) on running on the story's map? Scripts run on the map the player is on (`ed.a:Lg;` →
/// `g.b:B`), so the story's write comes from [`KARMEN_AWAY_MAP`] and 「수상한 남자」's from map 4 — the
/// site, not the quests, tells them apart (code review 2026-11-03, 7).
pub fn karmen_away_site(map: Option<i8>) -> bool {
    map == Some(KARMEN_AWAY_MAP)
}

/// HZ-063, the read: could the story have turned flag 48 on by now — a save made before the write
/// was gated may hold 「수상한 남자」's 48. The masked figure's scene needs a round past the first and
/// the Karmen route's quest at 2 (its branch), and starts 「결전」 just before the flag. A legitimate
/// 48 therefore always comes with all three; one without them is the refusal's and reads as off.
/// `round` is `ed.h`, the quests' states `ed.a(I)B` (2 bits: `>= 2` so a state past 2, were one
/// ever written, would not turn a real 48 off — code review 2026-11-03, 6).
pub fn karmen_away_real(round: i8, karmen_route: i8, final_battle: i8) -> bool {
    round > 0 && karmen_route >= 2 && final_battle >= 1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// HZ-063: flag 48 is the story's only after the masked figure's scene — never in round 0, never on
    /// the Elpis route, never before 「결전」.
    #[test]
    fn karmen_away_is_the_story_s_only_after_its_scene() {
        assert!(karmen_away_real(1, 2, 1), "하드코어, Karmen route, 「결전」 under way");
        assert!(karmen_away_real(3, 2, 1), "가디언 모드 too");
        assert!(karmen_away_real(1, 3, 1), "a quest state past 2 (never written, but 2 bits) keeps a real 48");
        assert!(!karmen_away_real(0, 2, 1), "노말: the scene needs round > 0");
        assert!(!karmen_away_real(2, 0, 1), "the Elpis route (quest 11 never taken)");
        assert!(!karmen_away_real(2, 1, 0), "the Karmen route still under way");
        assert!(!karmen_away_real(2, 2, 0), "before the masked figure's scene");
        assert_eq!(hz_patches::hooks::KARMEN_AWAY, 48);
    }

    /// HZ-063: the write is the story's only from its own map — 「수상한 남자」's refusal (map 4) is not,
    /// whatever the quests say (the Karmen route past 「결전」 could otherwise refuse him into a real 48).
    #[test]
    fn karmen_away_is_written_only_from_its_map() {
        assert!(karmen_away_site(Some(2)), "은거지, the masked figure's scene");
        assert!(!karmen_away_site(Some(4)), "게힌시티, 「수상한 남자」");
        assert!(!karmen_away_site(None), "no map loaded: not the story's scene");
        assert_eq!(KARMEN_AWAY_MAP, 2);
    }

    #[test]
    fn only_a_boss_heads_last_counted_kill_waits() {
        // The boss rooms' counters: one kill of the head.
        for kind in [109, 110, 117, 101, 115, 105, 112, 118, 119, 114, 99, 124, 128, 122] {
            assert!(waits(kind, 1), "{kind}");
            assert!(!waits(kind, 2), "{kind}: not the last");
            assert!(!waits(kind, -1), "{kind}: spent, or never armed");
            assert!(!waits(kind, 0), "{kind}");
        }
        // The arena's ten (81, 83 … 97), the tutorial's kind 0, a boss's other parts.
        for kind in [81, 83, 85, 89, 97, 0, 102, 103, 104, 116, 121, 123] {
            assert!(!waits(kind, 1), "{kind}");
        }
    }

    #[test]
    fn five_seconds_counted_down_then_the_story() {
        let mut w = Wait::new(3, 121u8 as i8);
        assert_eq!(w.seconds(), 5, "shown at the kill");
        let mut shown = vec![];
        let mut went = None;
        for t in 1..=200 {
            match w.tick(121u8 as i8, true) {
                Step::Wait(s) => {
                    if shown.last() != Some(&s) {
                        shown.push(s);
                    }
                }
                Step::Go(g) => {
                    went = Some((t, g));
                    break;
                }
                Step::Drop => panic!("same map"),
            }
        }
        assert_eq!(shown, [5, 4, 3, 2, 1]);
        assert_eq!(went, Some((WAIT_TICKS, 3)), "five seconds of field ticks");
    }

    #[test]
    fn a_fall_holds_the_story_at_its_last_second() {
        let map = 125u8 as i8;
        let mut w = Wait::new(1, map);
        for _ in 0..WAIT_TICKS - 1 {
            assert!(matches!(w.tick(map, true), Step::Wait(_)));
        }
        for _ in 0..50 {
            assert_eq!(w.tick(map, false), Step::Wait(1), "falling: not yet");
        }
        assert_eq!(w.tick(map, true), Step::Go(1), "stood up");
    }

    #[test]
    fn another_map_drops_it() {
        let mut w = Wait::new(1, 127u8 as i8);
        assert!(matches!(w.tick(127u8 as i8, true), Step::Wait(5)));
        assert_eq!(w.tick(8, true), Step::Drop);
    }
}
