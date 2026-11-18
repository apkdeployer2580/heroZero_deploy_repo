//! HZ-065 — a quest reward the full bag cannot take waits for the next save, and goes into the
//! 계정창고 as part of it (code review 2026-11-03, twice).
//!
//! Script action 16 gives a reward into its bag; when the bag is full the game takes back what it
//! put, says 「가방에 공간이 부족합니다.」 and — once that box is closed — throws the rest of the action
//! list away, so whatever ran before the give (a diary taken, the EXP) stays and the quest never ends.
//! The hook answers the whole count instead, so the list goes on, and the reward waits here.
//!
//! Why it waits rather than going into the storage at once: the 계정창고 is the account's record
//! (`HOL.storage`) and the quest's progress is the slot's (`HOL.gamedata<n>`). Writing the storage
//! then and there put the reward on disk while the progress that paid for it stayed unsaved — quit
//! without saving, and the reward was kept while the quest could be done again. So the reward is
//! written by the next save of the slot, right after the slot's progress record (`ed.k()V` @143) —
//! and only once that record reads back as written: the game's writer answers `true` even when the
//! record store refused the write (`rpg/b`: the RMS exception is caught and printed, then
//! `iconst_1`), so only the record itself can say. The storage's own write is read back the same
//! way. Not written: exactly what that save added is taken back out of the storage in memory, and
//! the reward keeps waiting for the save after. Leaving the game without a save (the title, a game
//! over) drops a reward no save went past, with the progress it belonged to; a reward a save went
//! past outlives the process in [`FILE`] (its progress is on the disk, it is owed) until a save puts
//! it away.
//!
//! Pure: the hooks hold a [`Pending`] in the guest state and act on what it says.

/// One reward waiting: `count` of item (`cat`, `id`), and whether a save of its progress has gone by
/// without the storage taking it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reward {
    pub cat: i8,
    pub id: i8,
    pub count: i32,
    pub saved: bool,
}

/// What the host tells the player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
    /// The bag was full: the reward goes to the 계정창고 with the next save.
    Waiting,
    /// A save put the waiting rewards into the 계정창고 (read back from the record).
    Stored,
    /// A save could not write the 계정창고 — the rewards wait for the next save.
    NotStored,
    /// A save found no room in the 계정창고 for some — they wait until there is.
    StorageFull,
    /// A save put the rewards into the 계정창고 but could not read its record back to be sure.
    Unverified,
}

/// How a save's write of the storage record went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Written {
    /// The record read back as written.
    Yes,
    /// The record store refused it (the record is as it was).
    No,
    /// The write was made but the read-back itself failed: on the disk or not, unknown. Taken as
    /// written — what was put into the storage in memory stays there (any later write of the
    /// storage carries it), and nothing is queued twice.
    Unknown,
}

/// The host file the rewards a save went past are kept in, a line each (`cat id count`, `#`
/// comments) — none waiting, no file.
pub const FILE: &str = "hz_rewards.txt";

/// The rewards waiting for a save.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pending(Vec<Reward>);

impl Pending {
    /// A reward the full bag could not take.
    pub fn add(&mut self, cat: i8, id: i8, count: i32) {
        self.0.push(Reward { cat, id, count, saved: false });
    }

    /// Everything waiting, for a save to put away (what it cannot goes back with [`Pending::keep`]).
    pub fn take(&mut self) -> Vec<Reward> {
        std::mem::take(&mut self.0)
    }

    /// What a save could not put away, back as it is: [`after_save`] marks what a save went past.
    pub fn keep(&mut self, rest: impl IntoIterator<Item = Reward>) {
        self.0.extend(rest);
    }

    /// The game was left without a save (the title, a game over): the rewards no save went past go
    /// with the progress that earned them. Returns how many.
    pub fn drop_unsaved(&mut self) -> usize {
        let before = self.0.len();
        self.0.retain(|r| r.saved);
        before - self.0.len()
    }

    /// The file's lines at boot: rewards whose progress reached the disk in an earlier run.
    pub fn restore(&mut self, text: &str) -> usize {
        let before = self.0.len();
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let mut words = line.split_whitespace().map(str::parse::<i32>);
            if let (Some(Ok(cat)), Some(Ok(id)), Some(Ok(count)), None) = (words.next(), words.next(), words.next(), words.next())
                && let (Ok(cat), Ok(id)) = (i8::try_from(cat), i8::try_from(id))
                && count > 0
            {
                self.0.push(Reward { cat, id, count, saved: true });
            }
        }
        self.0.len() - before
    }

    /// The file's text: the rewards a save went past (`None`: nothing to keep, no file).
    pub fn saved_text(&self) -> Option<String> {
        let saved: Vec<&Reward> = self.0.iter().filter(|r| r.saved).collect();
        if saved.is_empty() {
            return None;
        }
        let mut out = String::from("# heroZero: quest rewards owed to the 계정창고 - put there by the next save (HZ-065)\n");
        for r in saved {
            out.push_str(&format!("{} {} {}\n", r.cat, r.id, r.count));
        }
        Some(out)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A save's outcome, once the slot's progress record read back as written: `placed` rewards put
/// into the storage in memory, `left` with no room there, and how the storage record's write went.
/// Returns what keeps waiting — every one of them past a save now — and what to say.
pub fn after_save(placed: Vec<Reward>, left: Vec<Reward>, written: Written) -> (Vec<Reward>, Vec<Notice>) {
    let past = |r: Reward| Reward { saved: true, ..r };
    let mut say = Vec::new();
    let keep: Vec<Reward> = match written {
        Written::No if !placed.is_empty() => {
            say.push(Notice::NotStored);
            placed.into_iter().chain(left.iter().copied()).map(past).collect()
        }
        _ => {
            if !placed.is_empty() {
                say.push(if written == Written::Unknown { Notice::Unverified } else { Notice::Stored });
            }
            left.iter().copied().map(past).collect()
        }
    };
    if !left.is_empty() {
        say.push(Notice::StorageFull);
    }
    (keep, say)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(id: i8) -> Reward {
        Reward { cat: 11, id, count: 1, saved: false }
    }

    fn s(id: i8) -> Reward {
        Reward { saved: true, ..r(id) }
    }

    /// A reward waits until a save; leaving without one drops it (its progress is gone too).
    #[test]
    fn a_reward_no_save_went_past_goes_with_its_progress() {
        let mut p = Pending::default();
        p.add(11, 20, 1);
        assert_eq!(p.len(), 1);
        assert_eq!(p.drop_unsaved(), 1, "the title without a save");
        assert!(p.is_empty());
    }

    /// A save that could not write the storage keeps every reward — past a save now, so the title no
    /// longer drops them — and says so, with the storage's fullness in the same breath.
    #[test]
    fn a_refused_write_keeps_the_rewards_for_the_next_save() {
        let mut p = Pending::default();
        p.add(11, 20, 1);
        p.add(16, 3, 5);
        let taken = p.take();
        assert!(p.is_empty());
        let (kept, say) = after_save(taken.clone(), Vec::new(), Written::No);
        assert_eq!(say, vec![Notice::NotStored]);
        p.keep(kept);
        assert_eq!(p.len(), 2);
        assert_eq!(p.drop_unsaved(), 0, "a save went past them: kept through the title");
        assert!(p.take().iter().all(|r| r.saved));
        // Refused, and one had no room either: both said in the one save (review 2026-11-03, 8).
        let (kept, say) = after_save(vec![r(1)], vec![r(2)], Written::No);
        assert_eq!((kept, say), (vec![s(1), s(2)], vec![Notice::NotStored, Notice::StorageFull]));
    }

    /// A written save keeps only what found no room; nothing placed means nothing to write.
    #[test]
    fn a_written_save_keeps_only_what_had_no_room() {
        assert_eq!(after_save(vec![r(1)], Vec::new(), Written::Yes), (Vec::new(), vec![Notice::Stored]));
        assert_eq!(after_save(vec![r(1)], vec![r(2)], Written::Yes), (vec![s(2)], vec![Notice::Stored, Notice::StorageFull]));
        assert_eq!(after_save(Vec::new(), vec![r(2)], Written::Yes), (vec![s(2)], vec![Notice::StorageFull]));
        assert_eq!(after_save(Vec::new(), Vec::new(), Written::Yes), (Vec::new(), Vec::new()));
        assert_eq!(after_save(Vec::new(), vec![r(2)], Written::No), (vec![s(2)], vec![Notice::StorageFull]), "nothing placed: nothing was written, nothing to say of it");
    }

    /// A write whose read-back failed is taken as written: nothing is queued twice (review
    /// 2026-11-03, 2), the player hears it was not verified.
    #[test]
    fn an_unverified_write_queues_nothing_twice() {
        assert_eq!(after_save(vec![r(1)], Vec::new(), Written::Unknown), (Vec::new(), vec![Notice::Unverified]));
        assert_eq!(after_save(vec![r(1)], vec![r(2)], Written::Unknown), (vec![s(2)], vec![Notice::Unverified, Notice::StorageFull]));
    }

    /// What a save went past outlives the process through the file; what no save went past does not
    /// (review 2026-11-03, 3).
    #[test]
    fn the_file_keeps_what_a_save_went_past() {
        let mut p = Pending::default();
        assert_eq!(p.saved_text(), None, "nothing: no file");
        p.add(11, 20, 1);
        assert_eq!(p.saved_text(), None, "no save went past it: it dies with the process, as its progress does");
        p.keep([s(5), Reward { cat: 20, id: 1, count: 3, saved: true }]);
        let text = p.saved_text().unwrap();
        assert_eq!(text.lines().filter(|l| !l.starts_with('#')).collect::<Vec<_>>(), ["11 5 1", "20 1 3"]);
        let mut back = Pending::default();
        assert_eq!(back.restore(&format!("{text}\n\nbad line\n1 2\n3 4 0\n300 1 1\n")), 2, "two good lines; a short, a zero, an out-of-range one ignored");
        assert_eq!(back.take(), vec![s(5), Reward { cat: 20, id: 1, count: 3, saved: true }]);
        assert_eq!(FILE, "hz_rewards.txt");
    }
}
