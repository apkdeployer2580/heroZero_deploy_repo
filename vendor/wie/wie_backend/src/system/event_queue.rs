use alloc::{
    boxed::Box,
    collections::{BTreeMap, VecDeque},
};
use core::pin::Pin;

use wie_util::Result;

use crate::Instant;

#[allow(clippy::upper_case_acronyms, non_camel_case_types)]
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum KeyCode {
    UP,
    DOWN,
    LEFT,
    RIGHT,
    OK,
    LEFT_SOFT_KEY,
    RIGHT_SOFT_KEY,
    CLEAR,
    CALL,
    HANGUP,
    VOLUME_UP,
    VOLUME_DOWN,

    NUM0,
    NUM1,
    NUM2,
    NUM3,
    NUM4,
    NUM5,
    NUM6,
    NUM7,
    NUM8,
    NUM9,
    HASH,
    STAR,
}

impl KeyCode {
    // TODO we can use libraries like strum
    pub fn parse(string: &str) -> KeyCode {
        match string {
            "UP" => KeyCode::UP,
            "DOWN" => KeyCode::DOWN,
            "LEFT" => KeyCode::LEFT,
            "RIGHT" => KeyCode::RIGHT,
            "OK" => KeyCode::OK,
            "0" => KeyCode::NUM0,
            "1" => KeyCode::NUM1,
            "2" => KeyCode::NUM2,
            "3" => KeyCode::NUM3,
            "4" => KeyCode::NUM4,
            "5" => KeyCode::NUM5,
            "6" => KeyCode::NUM6,
            "7" => KeyCode::NUM7,
            "8" => KeyCode::NUM8,
            "9" => KeyCode::NUM9,
            "#" => KeyCode::HASH,
            "*" => KeyCode::STAR,
            "CLR" => KeyCode::CLEAR,
            _ => unimplemented!("Unknown key: {string}"),
        }
    }
}

type TimerCallback = Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = Result<()>> + Send>> + Send + Sync>;

/// Identity of one pending timer shot. `id` is the guest's timer object (the WIPI
/// `ptr_timer`); `generation` is bumped by every re-arm or cancel of that object, so a
/// shot whose generation is no longer current must NOT fire — it was replaced by a newer
/// `MC_knlSetTimer` or removed by `MC_knlUnsetTimer` while it sat in the queue.
/// (The shot cannot simply be deleted from the queue at cancel time: the event pump
/// stashes not-yet-due timers in a local Vec while guest code — which may be the very
/// code calling `MC_knlUnsetTimer` — runs, so cancellation has to work by making the
/// pending shot stale wherever it currently lives.)
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct TimerKey {
    pub id: u64,
    pub generation: u64,
}

pub enum Event {
    Redraw,
    Keydown(KeyCode),
    Keyup(KeyCode),
    Keyrepeat(KeyCode),
    Timer {
        due: Instant,
        key: Option<TimerKey>,
        callback: TimerCallback,
    },
    Notify {
        r#type: i32,
        param1: i32,
        param2: i32,
    }, // wipi notifyEvent
}

impl Event {
    pub fn timer<F, Fut>(due: Instant, callback: F) -> Self
    where
        F: FnOnce() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        Event::Timer {
            due,
            key: None,
            callback: Box::new(move || Box::pin(callback())),
        }
    }

    pub fn timer_keyed<F, Fut>(due: Instant, key: TimerKey, callback: F) -> Self
    where
        F: FnOnce() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        Event::Timer {
            due,
            key: Some(key),
            callback: Box::new(move || Box::pin(callback())),
        }
    }
}

#[derive(Default)]
pub struct EventQueue {
    events: VecDeque<Event>,
    timer_generations: BTreeMap<u64, u64>,
    /// ★ heroZero fork (2026-10-28): the task napping until something is queued (the MIDP event
    /// pump, `System::sleep_for_event`), woken by `System::push_event`.
    waiter: Option<usize>,
}

impl EventQueue {
    pub fn new() -> Self {
        Self {
            events: VecDeque::new(),
            timer_generations: BTreeMap::new(),
            waiter: None,
        }
    }

    pub fn push(&mut self, event: Event) {
        self.events.push_back(event);
    }

    /// ★ heroZero fork (2026-10-28). The task that will sleep until the queue has something.
    pub fn set_waiter(&mut self, task_id: usize) {
        self.waiter = Some(task_id);
    }

    /// The waiter, cleared — the pusher wakes it.
    pub fn take_waiter(&mut self) -> Option<usize> {
        self.waiter.take()
    }

    /// The waiter is up (its nap ran out on its own).
    pub fn clear_waiter(&mut self) {
        self.waiter = None;
    }

    pub fn pop(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    /// Arms (or re-arms) the timer object `id`: any shot still pending under the previous
    /// generation becomes stale, so a re-arm REPLACES the pending shot instead of adding a
    /// second one. Returns the key the new shot must carry.
    pub fn arm_timer(&mut self, id: u64) -> TimerKey {
        let generation = self.timer_generations.entry(id).and_modify(|g| *g += 1).or_insert(1);

        TimerKey { id, generation: *generation }
    }

    /// Cancels the pending shot of timer object `id` (WIPI `MC_knlUnsetTimer`). A no-op
    /// for an id that was never armed.
    pub fn cancel_timer(&mut self, id: u64) {
        self.timer_generations.entry(id).and_modify(|g| *g += 1);
    }

    /// Whether the shot carrying `key` is still the live one for its timer object.
    pub fn timer_key_current(&self, key: TimerKey) -> bool {
        self.timer_generations.get(&key.id).copied().unwrap_or(0) == key.generation
    }
}

#[cfg(test)]
mod tests {
    use super::EventQueue;

    #[test]
    fn arm_returns_current_key() {
        let mut queue = EventQueue::new();

        let key = queue.arm_timer(0x1234);
        assert!(queue.timer_key_current(key));
    }

    #[test]
    fn rearm_replaces_pending_shot() {
        let mut queue = EventQueue::new();

        let first = queue.arm_timer(0x1234);
        let second = queue.arm_timer(0x1234);

        assert!(!queue.timer_key_current(first));
        assert!(queue.timer_key_current(second));
    }

    #[test]
    fn cancel_makes_shot_stale() {
        let mut queue = EventQueue::new();

        let key = queue.arm_timer(0x1234);
        queue.cancel_timer(0x1234);

        assert!(!queue.timer_key_current(key));
    }

    #[test]
    fn cancel_unknown_id_is_noop() {
        let mut queue = EventQueue::new();

        queue.cancel_timer(0x9999);

        let key = queue.arm_timer(0x1234);
        assert!(queue.timer_key_current(key));
    }

    #[test]
    fn timers_are_independent() {
        let mut queue = EventQueue::new();

        let a = queue.arm_timer(1);
        let b = queue.arm_timer(2);
        queue.cancel_timer(1);

        assert!(!queue.timer_key_current(a));
        assert!(queue.timer_key_current(b));
    }
}
