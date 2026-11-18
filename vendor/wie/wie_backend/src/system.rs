pub mod atr;
mod audio;
mod event_queue;
mod file_system;
pub mod ma2;

use alloc::{borrow::ToOwned, boxed::Box, string::String, sync::Arc};

use spin::{RwLock, RwLockWriteGuard};

use wie_util::Result;

use crate::{
    AsyncCallable,
    executor::Executor,
    platform::Platform,
    task::{SleepFuture, YieldFuture},
    task_runner::TaskRunner,
};

use self::{audio::Audio, event_queue::EventQueue};

pub use self::{
    event_queue::{Event, KeyCode, TimerKey},
    file_system::FilesystemOverlay,
};

#[derive(Clone)]
pub struct System {
    pid: String,
    aid: String,
    executor: Executor,
    platform: Arc<Box<dyn Platform>>,
    filesystem: FilesystemOverlay,
    event_queue: Arc<RwLock<EventQueue>>,
    audio: Arc<RwLock<Audio>>,
    task_runner: Arc<dyn TaskRunner>,
}

impl System {
    pub fn new<T>(platform: Box<dyn Platform>, pid: &str, aid: &str, task_runner: T) -> Self
    where
        T: TaskRunner + 'static,
    {
        let audio_sink = platform.audio_sink();
        let platform = Arc::new(platform);

        Self {
            pid: pid.to_owned(),
            aid: aid.to_owned(), // TODO create metadata dictionary or something
            executor: Executor::new(),
            filesystem: FilesystemOverlay::new(platform.clone(), aid),
            platform,
            event_queue: Arc::new(RwLock::new(EventQueue::new())),
            audio: Arc::new(RwLock::new(Audio::new(audio_sink))),
            task_runner: Arc::new(task_runner),
        }
    }

    pub fn tick(&mut self) -> Result<()> {
        let platform = self.platform.clone();
        self.executor.tick(move || platform.now())
    }

    pub fn spawn<C>(&self, callable: C)
    where
        C: AsyncCallable<Result<()>> + 'static + Send,
    {
        let runner_clone = self.task_runner.clone();
        self.executor.spawn(async move || runner_clone.run(Box::pin(callable.call())).await);
    }

    /// [`Self::spawn`] for a task that runs GUEST code (the MIDlet's start task, a
    /// `Thread.start`, a WIPI-C timer): one [`Self::park_guest`] stops it while the host's
    /// own tasks — the music player — keep running. See `Executor::park`.
    pub fn spawn_guest<C>(&self, callable: C)
    where
        C: AsyncCallable<Result<()>> + 'static + Send,
    {
        let runner_clone = self.task_runner.clone();
        self.executor
            .spawn_parkable(async move || runner_clone.run(Box::pin(callable.call())).await);
    }

    /// Hold every guest task (`true`) or let them run again. The executor still ticks;
    /// only the tasks spawned with [`Self::spawn_guest`] stand still.
    pub fn park_guest(&self, on: bool) {
        self.executor.park(on);
    }

    pub fn guest_parked(&self) -> bool {
        self.executor.parked()
    }

    pub fn sleep(&self, timeout: u64) -> SleepFuture {
        SleepFuture::new(timeout, &self.executor)
    }

    /// ★ heroZero fork (2026-10-28): [`Self::sleep`] for the event pump — a nap that
    /// [`Self::push_event`] ends early. The caller clears the waiter when it is up again
    /// (`EventQueue::clear_waiter`), so a later push does not wake a task that is not asleep.
    pub fn sleep_for_event(&self, timeout: u64) -> SleepFuture {
        let task_id = self.executor.current_task_id() as usize;
        self.event_queue.write().set_waiter(task_id);
        SleepFuture::new(timeout, &self.executor)
    }

    /// ★ heroZero fork (2026-10-28): queue an event from the host (a key, a repaint) and wake the
    /// pump if it is napping — measured: the pump looked at an empty queue every 16 ms, so a key
    /// waited up to that long before `keyPressed`, and the game's `repaint()` as long again before
    /// `paint`. Lock order: the queue, released, then the executor.
    pub fn push_event(&self, event: Event) {
        let waiter = {
            let mut queue = self.event_queue.write();
            queue.push(event);
            queue.take_waiter()
        };
        if let Some(task_id) = waiter {
            self.executor.wake_sleeper(task_id);
        }
    }

    /// See [`Executor::next_wakeup_in`] — how long the host may sleep before this
    /// system has work again.
    pub fn next_wakeup_ms(&self) -> Option<u64> {
        self.executor.next_wakeup_in(self.platform.now())
    }

    pub fn current_task_id(&self) -> u64 {
        self.executor.current_task_id()
    }

    pub fn yield_now(&self) -> YieldFuture {
        YieldFuture::new()
    }

    /// Unified filesystem view. Reads consult the persistent platform
    /// backend first and fall back to the in-memory virtual layer loaded
    /// from archives; writes always hit the platform backend.
    pub fn filesystem(&self) -> &FilesystemOverlay {
        &self.filesystem
    }

    pub fn pid(&self) -> &str {
        &self.pid
    }

    pub fn aid(&self) -> &str {
        &self.aid
    }

    pub fn platform(&self) -> &dyn Platform {
        self.platform.as_ref().as_ref()
    }

    pub fn audio(&self) -> RwLockWriteGuard<'_, Audio> {
        self.audio.as_ref().write()
    }

    pub fn event_queue(&self) -> RwLockWriteGuard<'_, EventQueue> {
        self.event_queue.write()
    }
}
