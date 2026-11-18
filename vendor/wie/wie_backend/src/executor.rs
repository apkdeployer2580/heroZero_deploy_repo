use alloc::{boxed::Box, sync::Arc, task::Wake};
use core::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicUsize, Ordering},
    task::{Context, Poll, Waker},
};

use hashbrown::{HashMap, HashSet};
use spin::Mutex;

use wie_util::{Result, WieError};

use crate::time::Instant;

type Task = Pin<Box<dyn Future<Output = Result<()>> + Send>>;

static TASK_COUNT_MAX: AtomicUsize = AtomicUsize::new(0);

pub struct ExecutorInner {
    current_task_id: Option<usize>,
    tasks: HashMap<usize, Task>,
    sleeping_tasks: HashMap<usize, Instant>,
    last_task_id: usize,
    last_now: Instant,
    /// ★ 2026-10-21 (wie_app fork): the tasks that run GUEST code — the MIDlet's start task,
    /// every `Thread.start`, a WIPI-C timer. While `parked` they are not polled at all
    /// (their sleep entry stays as it is), so the executor keeps ticking the HOST tasks —
    /// the SMAF/MA-2 music player above all — with the game itself standing still. The
    /// hero2 world map: it holds the field the way a menu does, but a menu keeps the
    /// music going and a tick stop did not.
    parkable: HashSet<usize>,
    parked: bool,
    /// ★ heroZero fork (2026-10-22): tasks whose last poll returned `Pending` without
    /// waking themselves and without registering a sleep — i.e. they wait on an event (a
    /// guest `Object.wait`, `Thread.join`, a class-init lock). They are still polled every
    /// `step`, as before, but no longer count as runnable until their waker fires (see
    /// [`Executor::woken`]). Before this, ONE guest thread in `Object.wait` made every task
    /// set look runnable forever: `tick` spun its whole 8 ms every frame and
    /// `next_wakeup_in` answered `Some(0)`, so the host never slept (heat, battery) — and
    /// on a virtual clock (`hz_probe`) `tick` never returned at all. 영웅서기 Zero parks a
    /// thread in `wait(100000)` during boot.
    blocked: HashSet<usize>,
}

impl ExecutorInner {
    /// `(how many tasks may be polled right now, the earliest wake-up among the sleeping
    /// ones that may)` — parked guest tasks count for neither, nor do blocked tasks that
    /// have not been woken since.
    fn runnable(&self, woken: &HashSet<usize>) -> (usize, Option<Instant>) {
        let mut running = 0usize;
        let mut next: Option<Instant> = None;
        for id in self.tasks.keys() {
            if self.parked && self.parkable.contains(id) {
                continue;
            }
            match self.sleeping_tasks.get(id) {
                Some(until) => next = Some(next.map_or(*until, |n| if *until < n { *until } else { n })),
                None if self.blocked.contains(id) && !woken.contains(id) => {}
                None => running += 1,
            }
        }
        (running, next)
    }
}

/// ★ heroZero fork (2026-10-22): the waker each task is polled with. It only records the
/// task id in [`Executor::woken`] — a lock of its own, never the executor's, because a
/// waker may fire from inside another task's poll or from a host thread.
struct TaskWaker {
    id: usize,
    woken: Arc<Mutex<HashSet<usize>>>,
}

impl Wake for TaskWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.woken.lock().insert(self.id);
    }
}

pub trait AsyncCallable<R>: Send
where
    R: Send,
{
    fn call(self) -> impl Future<Output = R> + Send;
}

impl<F, R, Fut> AsyncCallable<R> for F
where
    F: FnOnce() -> Fut + 'static + Send,
    R: AsyncCallableResult,
    Fut: Future<Output = R> + 'static + Send,
{
    async fn call(self) -> R {
        self().await
    }
}

pub trait AsyncCallableResult: Send {
    fn err(self) -> Option<WieError>;
}

impl<R> AsyncCallableResult for core::result::Result<R, WieError>
where
    R: Send,
{
    fn err(self) -> Option<WieError> {
        self.err()
    }
}

impl AsyncCallableResult for () {
    fn err(self) -> Option<WieError> {
        None
    }
}

#[derive(Clone)]
pub struct Executor {
    inner: Arc<Mutex<ExecutorInner>>,
    /// Tasks woken since they were last polled. Lock order: `inner` before `woken`.
    woken: Arc<Mutex<HashSet<usize>>>,
}

impl Executor {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let inner = Arc::new(Mutex::new(ExecutorInner {
            current_task_id: None,
            tasks: HashMap::new(),
            sleeping_tasks: HashMap::new(),
            last_task_id: 0,
            last_now: Instant::from_epoch_millis(0),
            parkable: HashSet::new(),
            parked: false,
            blocked: HashSet::new(),
        }));

        Self {
            inner,
            woken: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn spawn<C, R>(&self, callable: C) -> usize
    where
        C: AsyncCallable<R> + 'static,
        R: AsyncCallableResult,
    {
        let fut = async move {
            let result = callable.call().await;
            if let Some(err) = result.err() {
                return Err(err);
            }

            Ok(())
        };

        let task_id = {
            let mut inner = self.inner.lock();
            inner.last_task_id += 1;
            inner.last_task_id
        };

        self.inner.lock().tasks.insert(task_id, Box::pin(fut));

        task_id
    }

    /// [`Self::spawn`] for a task that runs GUEST code — one [`Self::park`] stops.
    pub fn spawn_parkable<C, R>(&self, callable: C) -> usize
    where
        C: AsyncCallable<R> + 'static,
        R: AsyncCallableResult,
    {
        let task_id = self.spawn(callable);
        self.inner.lock().parkable.insert(task_id);
        task_id
    }

    /// Park (`true`) or release every parkable task. Parked tasks are neither polled nor
    /// counted as runnable, so a tick with only parked guest tasks and a sleeping music
    /// player sleeps out to the player's next event instead of spinning its 8 ms.
    pub fn park(&self, on: bool) {
        self.inner.lock().parked = on;
    }

    pub fn parked(&self) -> bool {
        self.inner.lock().parked
    }

    // TODO we need to remove error handling from here. we need to JoinHandle like on spawn..
    pub fn tick<T>(&mut self, now: T) -> Result<()>
    where
        T: Fn() -> Instant,
    {
        // ★ Task-count gauge (wie_app fork, 2026-08-06). Tasks are only removed on
        // `Poll::Ready`, so a spawned task that never completes stays forever — which
        // would show up here as a rising count and in RSS, separately from the poll
        // nesting the crash is about. Logs on a new maximum only.
        {
            let n = self.inner.lock().tasks.len();
            if n > TASK_COUNT_MAX.load(Ordering::Relaxed) {
                TASK_COUNT_MAX.store(n, Ordering::Relaxed);
                if n >= 8 {
                    tracing::warn!("executor task count {n}");
                }
            }
        }

        let end = now() + 8; // TODO hardcoded
        loop {
            let now = now();

            if now > end {
                break;
            }

            {
                let inner = self.inner.lock();
                let (running_task_count, next_wakeup) = inner.runnable(&self.woken.lock());
                if running_task_count == 0 {
                    match next_wakeup {
                        Some(next) if now >= next => {}
                        // Nothing runnable and nothing due: every live task sleeps past
                        // `now`, or only parked tasks are left.
                        _ => break,
                    }
                }
            }

            self.step(now)?;
        }

        Ok(())
    }

    pub fn current_task_id(&self) -> u64 {
        self.inner.lock().current_task_id.unwrap() as _
    }

    /// Milliseconds until this executor has work again: `Some(0)` when a task is
    /// runnable right now, `Some(n)` when every task is asleep and the earliest wakes
    /// in `n` ms, `None` when there is nothing at all.
    ///
    /// ★ Exists so the HOST's worker loop can stop over-sleeping (2026-08-13,
    /// `MIGRATION_LGT.md` §2.24). `tick` already computes this to decide its own early
    /// break, but it used to keep the number to itself, so `wie_android` filled the
    /// rest of its 16 ms frame regardless — a guest waiting 3 ms for its next frame
    /// timer was not looked at again for 16.
    pub fn next_wakeup_in(&self, now: Instant) -> Option<u64> {
        let inner = self.inner.lock();
        if inner.tasks.is_empty() {
            return None;
        }
        let (running, next) = inner.runnable(&self.woken.lock());
        if running > 0 {
            return Some(0);
        }
        let next = next?;
        Some(if next > now { next - now } else { 0 })
    }

    fn step(&mut self, now: Instant) -> Result<()> {
        self.inner.lock().last_now = now;

        let mut next_tasks = HashMap::new();
        let tasks = self.inner.lock().tasks.drain().collect::<HashMap<_, _>>();
        let mut sleeping_tasks = self.inner.lock().sleeping_tasks.drain().collect::<HashMap<_, _>>();

        let parked = self.inner.lock().parked;
        // ★ heroZero fork (2026-10-27): poll in task-id (spawn) order. `tasks` is a hashbrown map
        // with a randomly seeded hasher, so its iteration order differed from run to run - and
        // with it which guest thread ran first within a step: the same probe script drew the
        // player's walk at a different animation phase each time, and a frame-hash A/B (the only
        // proof an optimisation changed nothing) was impossible. Any order is a legal schedule;
        // this one is the same every time.
        let mut tasks = tasks.into_iter().collect::<alloc::vec::Vec<_>>();
        tasks.sort_unstable_by_key(|(id, _)| *id);
        for (task_id, mut task) in tasks.into_iter() {
            if parked && self.inner.lock().parkable.contains(&task_id) {
                // Not polled, not woken: the task and its sleep entry (if any) stay as they are.
                next_tasks.insert(task_id, task);
                continue;
            }
            let item = sleeping_tasks.get(&task_id);
            if let Some(item) = item {
                if *item <= now {
                    sleeping_tasks.remove(&task_id);
                } else {
                    next_tasks.insert(task_id, task);
                    continue;
                }
            }

            // A wake from here on (by this poll itself, by a later task in this step, or by a
            // host thread) means "poll me again"; one from before is answered by this poll.
            self.woken.lock().remove(&task_id);
            let waker = Waker::from(Arc::new(TaskWaker {
                id: task_id,
                woken: self.woken.clone(),
            }));
            let mut context = Context::from_waker(&waker);
            self.inner.lock().current_task_id = Some(task_id);

            match task.as_mut().poll(&mut context) {
                Poll::Ready(x) => {
                    let mut inner = self.inner.lock();
                    inner.parkable.remove(&task_id);
                    inner.blocked.remove(&task_id);
                    drop(inner);
                    self.woken.lock().remove(&task_id);
                    x?;
                }
                Poll::Pending => {
                    // Neither a yield (it wakes itself) nor a sleep (`SleepFuture::new`
                    // registers one): the task waits for someone else's wake.
                    let mut inner = self.inner.lock();
                    if !inner.sleeping_tasks.contains_key(&task_id) && !self.woken.lock().contains(&task_id) {
                        inner.blocked.insert(task_id);
                    } else {
                        inner.blocked.remove(&task_id);
                    }
                    drop(inner);
                    next_tasks.insert(task_id, task);
                }
            }

            self.inner.lock().current_task_id = None;
        }

        {
            // ★ heroZero fork (2026-10-28): a sleep ended by `wake_sleeper` DURING this step (from a
            // polled task) is an entry `inner` already holds — it wins over the one drained above.
            let mut inner = self.inner.lock();
            for (id, until) in sleeping_tasks {
                inner.sleeping_tasks.entry(id).or_insert(until);
            }
        }
        self.inner.lock().tasks.extend(next_tasks);

        Ok(())
    }

    /// ★ heroZero fork (2026-10-28): end a task's sleep now. It is due on the next `step`, so its
    /// `SleepFuture` resolves (the future itself only counts polls) and `next_wakeup_in` answers 0.
    /// For the MIDP event pump, which naps between looks at an empty queue: a key or a repaint the
    /// host pushes no longer waits for that nap to run out (`System::push_event`). Returns whether
    /// the task was asleep.
    pub fn wake_sleeper(&self, task_id: usize) -> bool {
        let mut inner = self.inner.lock();
        match inner.sleeping_tasks.get_mut(&task_id) {
            Some(until) => {
                *until = Instant::from_epoch_millis(0);
                true
            }
            // Mid-step (the entry is drained into `step`'s local map): a due entry here outlives
            // the merge at the end of the step. Not asleep at all: the entry is dropped unpolled
            // when the task's turn comes.
            None => {
                inner.sleeping_tasks.insert(task_id, Instant::from_epoch_millis(0));
                false
            }
        }
    }

    pub(crate) fn sleep(&self, timeout: u64) {
        let task_id = self.inner.lock().current_task_id.unwrap();

        let until = self.inner.lock().last_now + timeout;
        self.inner.lock().sleeping_tasks.insert(task_id, until);
    }
}

#[cfg(test)]
mod park_tests {
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicUsize, Ordering};

    use super::Executor;
    use crate::{task::YieldFuture, time::Instant};

    /// Counts its polls and yields forever.
    async fn count_forever(c: Arc<AtomicUsize>) -> wie_util::Result<()> {
        loop {
            c.fetch_add(1, Ordering::Relaxed);
            YieldFuture::new().await;
        }
    }

    fn counter(executor: &Executor, parkable: bool) -> Arc<AtomicUsize> {
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        if parkable {
            executor.spawn_parkable(move || count_forever(c));
        } else {
            executor.spawn(move || count_forever(c));
        }
        count
    }

    /// One `step`: `tick` reads `now` once for its deadline and once per loop; the third
    /// reading is past the deadline, so exactly one step runs.
    fn tick_once(executor: &mut Executor, at: u64) {
        let t = Instant::from_epoch_millis(at);
        let n = core::cell::Cell::new(0u64);
        executor
            .tick(move || {
                n.set(n.get() + 1);
                if n.get() <= 2 { t } else { t + 100 }
            })
            .unwrap();
    }

    #[test]
    fn a_parked_guest_task_is_not_polled_while_a_host_task_still_is() {
        let mut executor = Executor::new();
        let guest = counter(&executor, true);
        let host = counter(&executor, false);
        tick_once(&mut executor, 0);
        assert_eq!((guest.load(Ordering::Relaxed), host.load(Ordering::Relaxed)), (1, 1));

        executor.park(true);
        tick_once(&mut executor, 10);
        tick_once(&mut executor, 20);
        assert_eq!(
            (guest.load(Ordering::Relaxed), host.load(Ordering::Relaxed)),
            (1, 3),
            "the guest stands, the host runs"
        );

        executor.park(false);
        tick_once(&mut executor, 30);
        assert_eq!(
            (guest.load(Ordering::Relaxed), host.load(Ordering::Relaxed)),
            (2, 4),
            "released: both run again"
        );
    }

    #[test]
    fn a_parked_guest_task_does_not_count_as_runnable() {
        let executor = Executor::new();
        let _guest = counter(&executor, true);
        assert_eq!(executor.next_wakeup_in(Instant::from_epoch_millis(0)), Some(0));
        executor.park(true);
        // Only a parked task is left: nothing is due and nothing will be — the host may sleep as it likes.
        assert_eq!(executor.next_wakeup_in(Instant::from_epoch_millis(0)), None);
        executor.park(false);
        assert_eq!(executor.next_wakeup_in(Instant::from_epoch_millis(0)), Some(0));
    }

    /// Stands in for a guest `Object.wait`: `Pending` until `open`, keeping the latest waker.
    struct Gate {
        open: AtomicUsize,
        waker: spin::Mutex<Option<core::task::Waker>>,
    }

    struct GateFuture(Arc<Gate>);

    impl core::future::Future for GateFuture {
        type Output = wie_util::Result<()>;

        fn poll(self: core::pin::Pin<&mut Self>, cx: &mut core::task::Context<'_>) -> core::task::Poll<Self::Output> {
            if self.0.open.load(Ordering::SeqCst) != 0 {
                return core::task::Poll::Ready(Ok(()));
            }
            *self.0.waker.lock() = Some(cx.waker().clone());
            core::task::Poll::Pending
        }
    }

    /// ★ heroZero fork (2026-10-22): a task waiting on an event is not runnable — `tick`
    /// returns and the host may sleep — until its waker fires; then it runs to completion.
    #[test]
    fn a_task_waiting_on_an_event_is_not_runnable_until_woken() {
        let mut executor = Executor::new();
        let gate = Arc::new(Gate {
            open: AtomicUsize::new(0),
            waker: spin::Mutex::new(None),
        });
        let g = gate.clone();
        executor.spawn(move || GateFuture(g));

        assert_eq!(executor.next_wakeup_in(Instant::from_epoch_millis(0)), Some(0), "never polled yet");
        // A constant clock: before the fix this `tick` never returned.
        executor.tick(|| Instant::from_epoch_millis(0)).unwrap();
        assert_eq!(executor.next_wakeup_in(Instant::from_epoch_millis(0)), None, "blocked, nothing due");

        gate.open.store(1, Ordering::SeqCst);
        gate.waker.lock().take().unwrap().wake();
        assert_eq!(executor.next_wakeup_in(Instant::from_epoch_millis(0)), Some(0), "woken");

        executor.tick(|| Instant::from_epoch_millis(0)).unwrap();
        assert_eq!(executor.next_wakeup_in(Instant::from_epoch_millis(0)), None, "finished");
        assert!(executor.inner.lock().tasks.is_empty());
    }

    /// A yield is still runnable right away, and a sleep is due at its time.
    #[test]
    fn yield_and_sleep_are_not_mistaken_for_blocking() {
        let mut executor = Executor::new();
        let yields = counter(&executor, false);
        tick_once(&mut executor, 0);
        assert!(yields.load(Ordering::Relaxed) >= 1);
        assert_eq!(executor.next_wakeup_in(Instant::from_epoch_millis(0)), Some(0));

        let mut sleeper = Executor::new();
        let s = sleeper.clone();
        sleeper.spawn(move || async move {
            crate::task::SleepFuture::new(50, &s).await;
            Ok::<(), wie_util::WieError>(())
        });
        sleeper.tick(|| Instant::from_epoch_millis(0)).unwrap();
        assert_eq!(sleeper.next_wakeup_in(Instant::from_epoch_millis(0)), Some(50));
        sleeper.tick(|| Instant::from_epoch_millis(50)).unwrap();
        assert_eq!(sleeper.next_wakeup_in(Instant::from_epoch_millis(50)), None);
    }

    /// Counts, then sleeps `ms`, forever — the event pump's shape.
    async fn count_and_sleep(c: Arc<AtomicUsize>, executor: Executor, ms: u64) -> wie_util::Result<()> {
        loop {
            c.fetch_add(1, Ordering::Relaxed);
            crate::task::SleepFuture::new(ms, &executor).await;
        }
    }

    /// ★ heroZero fork (2026-10-28): `wake_sleeper` ends a sleep — the task runs on the next step
    /// and `next_wakeup_in` says so — and the sleep after that is a whole one again.
    #[test]
    fn a_sleeper_woken_early_runs_on_the_next_step() {
        let mut executor = Executor::new();
        let count = Arc::new(AtomicUsize::new(0));
        let (c, e) = (count.clone(), executor.clone());
        let id = executor.spawn(move || count_and_sleep(c, e, 100));
        tick_once(&mut executor, 0);
        assert_eq!(count.load(Ordering::Relaxed), 1);
        tick_once(&mut executor, 10);
        assert_eq!(count.load(Ordering::Relaxed), 1, "asleep until 100");
        assert_eq!(executor.next_wakeup_in(Instant::from_epoch_millis(10)), Some(90));

        assert!(executor.wake_sleeper(id));
        assert_eq!(executor.next_wakeup_in(Instant::from_epoch_millis(10)), Some(0), "due now");
        tick_once(&mut executor, 11);
        assert_eq!(count.load(Ordering::Relaxed), 2, "up on the next step");
        tick_once(&mut executor, 60);
        assert_eq!(count.load(Ordering::Relaxed), 2, "and asleep again, for the whole 100");
        tick_once(&mut executor, 111);
        assert_eq!(count.load(Ordering::Relaxed), 3);
    }

    /// A wake from inside a step (another task's poll) is not lost when the step merges the sleeps
    /// it drained back in.
    #[test]
    fn a_wake_from_inside_a_step_is_kept() {
        let mut executor = Executor::new();
        let count = Arc::new(AtomicUsize::new(0));
        let (c, e) = (count.clone(), executor.clone());
        let id = executor.spawn(move || count_and_sleep(c, e, 100));
        tick_once(&mut executor, 0);
        assert_eq!(count.load(Ordering::Relaxed), 1);
        // Spawned after the sleeper, so polled after it within a step: the sleeper's entry is in
        // the step's own map when this wakes it.
        let e = executor.clone();
        executor.spawn(move || async move {
            assert!(!e.wake_sleeper(id), "mid-step: not found in the executor's map");
            Ok::<(), wie_util::WieError>(())
        });
        tick_once(&mut executor, 10);
        assert_eq!(count.load(Ordering::Relaxed), 1, "this step only ran the waker");
        assert_eq!(executor.next_wakeup_in(Instant::from_epoch_millis(10)), Some(0), "the wake survived the merge");
        tick_once(&mut executor, 11);
        assert_eq!(count.load(Ordering::Relaxed), 2);
    }
}
