//! The writer thread's clock: when its retention gate reads the time and how
//! it waits while idle. Every shipped build has only the system clock
//! (`Instant::now()` and a real `retention::INTERVAL` wait, exactly as the
//! writer always did). With the `test-support` feature, which only
//! dev-dependencies enable, a writer can instead run on a [`SimClock`]: the
//! soak's simulated time, so the real writer thread performs its own
//! retention passes, at startup and once a simulated minute, without a real
//! minute's wait and without the harness calling maintenance itself.
use super::Bell;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// Which clock a writer thread reads.
pub(crate) enum Clock {
    System,
    #[cfg(feature = "test-support")]
    Simulated(Arc<SimClock>),
}

/// One writer thread's view of its clock, created on that thread.
pub(crate) struct WriterTime {
    #[cfg(feature = "test-support")]
    simulated: Option<(Arc<SimClock>, u64)>,
    /// The simulated time's generation when the writer's current loop
    /// iteration began.
    #[cfg(feature = "test-support")]
    read: u64,
    bell: Arc<Bell>,
}
impl WriterTime {
    /// Called by `Database::open` before the writer thread starts, so a
    /// simulated clock knows the writer from the moment the open returns.
    pub(crate) fn new(clock: Clock, bell: Arc<Bell>) -> Self {
        match clock {
            Clock::System => Self {
                #[cfg(feature = "test-support")]
                simulated: None,
                #[cfg(feature = "test-support")]
                read: 0,
                bell,
            },
            #[cfg(feature = "test-support")]
            Clock::Simulated(clock) => {
                let id = clock.register(bell.clone());
                Self {
                    simulated: Some((clock, id)),
                    read: 0,
                    bell,
                }
            }
        }
    }
    /// On the writer thread, before its first pass: a held simulated clock
    /// keeps it here until released.
    pub(crate) fn begin(&self) {
        #[cfg(feature = "test-support")]
        if let Some((clock, _)) = &self.simulated {
            clock.wait_released();
        }
    }
    /// The start of one turn of the writer's loop. On simulated time it
    /// notes the clock's generation: the turn's retention gate reads the
    /// clock no earlier, so once the writer goes idle at the end of a turn
    /// that began at the current generation, any pass due then has run (and
    /// a writer that never runs its gate still goes idle, and is seen to).
    pub(crate) fn turn(&mut self) {
        #[cfg(feature = "test-support")]
        if let Some((clock, id)) = &self.simulated {
            clock.doing(*id, "serving a call or a revocation");
            self.read = clock.read().0;
        }
    }
    /// The time the retention gate compares.
    pub(crate) fn now(&self) -> Instant {
        #[cfg(feature = "test-support")]
        if let Some((clock, id)) = &self.simulated {
            clock.doing(*id, "running its retention gate or a pass");
            return clock.read().1;
        }
        Instant::now()
    }
    /// Idle: until a send rings, and at least once a retention interval. On
    /// simulated time the interval passes only when the clock is advanced,
    /// which rings every writer: its own wait, on that clock.
    pub(crate) fn idle(&mut self, interval: Duration) {
        #[cfg(feature = "test-support")]
        if let Some((clock, id)) = &self.simulated {
            clock.idle(*id, self.read, true);
            // A real timeout only as a backstop: the gate decides, so a
            // spurious wake runs no extra pass.
            self.bell.wait(Duration::from_secs(3600));
            clock.idle(*id, self.read, false);
            return;
        }
        self.bell.wait(interval);
    }
    /// A pass ran (reported to a simulated clock's observer).
    #[cfg_attr(not(feature = "test-support"), allow(unused_variables))]
    pub(crate) fn passed(&self, pass: &super::retention::Pass) {
        #[cfg(feature = "test-support")]
        if let Some((clock, _)) = &self.simulated {
            clock.passed(pass);
        }
    }
}
#[cfg(feature = "test-support")]
impl Drop for WriterTime {
    fn drop(&mut self) {
        if let Some((clock, id)) = &self.simulated {
            clock.deregister(*id);
        }
    }
}

#[cfg(feature = "test-support")]
pub use simulated::SimClock;
#[cfg(feature = "test-support")]
mod simulated {
    use super::{Arc, Bell, Duration, Instant};
    use crate::retention::test_support::Pass;
    use std::{
        collections::BTreeMap,
        sync::{Condvar, Mutex, MutexGuard},
    };

    struct Writer {
        bell: Arc<Bell>,
        /// The generation this writer last went idle at, while it is idle.
        idle_at: Option<u64>,
        /// What it was last seen doing, for a writer that does not settle.
        doing: &'static str,
        /// The generation its current turn began at.
        turn: u64,
    }
    /// How long [`SimClock::settle`] waits, in real time, for the writers to
    /// go idle: far beyond any turn's work on a loaded CI runner (a retention
    /// pass is one batch of at most 1,024 rows), so only a writer that never
    /// goes idle reaches it.
    const SETTLE_LIMIT: Duration = Duration::from_secs(120);
    struct State {
        now: i64,
        generation: u64,
        writers: BTreeMap<u64, Writer>,
        next: u64,
        passes: Vec<Pass>,
        held: bool,
    }

    /// Simulated seconds shared by the soak and the writer threads opened on
    /// it ([`crate::test_support::open`]). It moves only when [`SimClock::set`]
    /// advances it, which rings every writer and returns once each has run
    /// its retention gate at the new time and gone idle again, so a pass due
    /// at a simulated time has run before the caller goes on.
    pub struct SimClock {
        state: Mutex<State>,
        changed: Condvar,
        origin: Instant,
        start: i64,
    }
    impl SimClock {
        pub fn new(start: i64) -> Arc<Self> {
            Arc::new(Self {
                state: Mutex::new(State {
                    now: start,
                    generation: 0,
                    writers: BTreeMap::new(),
                    next: 0,
                    passes: Vec::new(),
                    held: false,
                }),
                changed: Condvar::new(),
                origin: Instant::now(),
                start,
            })
        }
        fn lock(&self) -> MutexGuard<'_, State> {
            self.state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }
        fn wait<'a>(&self, state: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
            self.changed
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }
        /// Simulated seconds since the Unix epoch.
        pub fn now(&self) -> i64 {
            self.lock().now
        }
        /// The instant that stands for simulated time `at`: instants are only
        /// ever compared with each other.
        pub fn instant_at(&self, at: i64) -> Instant {
            self.origin + Duration::from_secs(u64::try_from(at - self.start).unwrap_or(0))
        }
        pub fn instant(&self) -> Instant {
            self.instant_at(self.now())
        }
        /// Moves the clock to `at` (never back), wakes every writer, and
        /// returns once each has run its retention gate at `at` and is idle.
        pub fn set(&self, at: i64) {
            let mut state = self.lock();
            assert!(at >= state.now, "the simulated clock never steps back");
            if at == state.now {
                return;
            }
            state.now = at;
            state.generation += 1;
            for writer in state.writers.values() {
                writer.bell.ring();
            }
            drop(state);
            self.settle();
        }
        /// Returns once every writer is idle after reading the current time.
        /// Panics, saying what each writer still busy was doing, if they are
        /// not within [`SETTLE_LIMIT`] of real time: a writer that never goes
        /// idle would otherwise hang the test until the CI job's timeout.
        pub fn settle(&self) {
            self.settle_within(SETTLE_LIMIT);
        }
        /// [`SimClock::settle`], waiting at most `limit` of real time.
        pub fn settle_within(&self, limit: Duration) {
            let deadline = Instant::now() + limit;
            let mut state = self.lock();
            loop {
                let busy: Vec<String> = state
                    .writers
                    .iter()
                    .filter(|(_, w)| w.idle_at != Some(state.generation))
                    .map(|(id, w)| {
                        format!(
                            "writer {id}: {} (its turn began at generation {}, idle at {:?})",
                            w.doing, w.turn, w.idle_at
                        )
                    })
                    .collect();
                if busy.is_empty() {
                    return;
                }
                let left = deadline.saturating_duration_since(Instant::now());
                assert!(
                    !left.is_zero(),
                    "the simulated clock's writers did not go idle within {:?} of real time after it moved to generation {} ({} s simulated): {}",
                    limit,
                    state.generation,
                    state.now,
                    busy.join("; ")
                );
                state = self
                    .changed
                    .wait_timeout(state, left)
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .0;
            }
        }
        /// Every retention pass the writers ran since the last call.
        pub fn take_passes(&self) -> Vec<Pass> {
            std::mem::take(&mut self.lock().passes)
        }
        /// Writer threads still running on this clock.
        pub fn writers(&self) -> usize {
            self.lock().writers.len()
        }
        /// Returns once no writer thread runs on this clock: each has closed
        /// its store and released the core lock.
        pub fn wait_closed(&self) {
            let mut state = self.lock();
            while !state.writers.is_empty() {
                state = self.wait(state);
            }
        }
        /// Writers wait, before their first retention pass, until
        /// [`SimClock::release`]: a test can queue work first.
        pub fn hold(&self) {
            self.lock().held = true;
        }
        pub fn release(&self) {
            self.lock().held = false;
            self.changed.notify_all();
        }

        pub(super) fn register(&self, bell: Arc<Bell>) -> u64 {
            let mut state = self.lock();
            let id = state.next;
            state.next += 1;
            state.writers.insert(
                id,
                Writer {
                    bell,
                    idle_at: None,
                    doing: "starting (waiting to be released, or its first pass)",
                    turn: 0,
                },
            );
            id
        }
        pub(super) fn wait_released(&self) {
            let mut state = self.lock();
            while state.held {
                state = self.wait(state);
            }
        }
        pub(super) fn deregister(&self, id: u64) {
            self.lock().writers.remove(&id);
            self.changed.notify_all();
        }
        pub(super) fn read(&self) -> (u64, Instant) {
            let state = self.lock();
            (
                state.generation,
                self.origin
                    + Duration::from_secs(u64::try_from(state.now - self.start).unwrap_or(0)),
            )
        }
        pub(super) fn idle(&self, id: u64, generation: u64, idle: bool) {
            let mut state = self.lock();
            if let Some(writer) = state.writers.get_mut(&id) {
                writer.idle_at = idle.then_some(generation);
                writer.doing = if idle { "idle" } else { "woken" };
            }
            drop(state);
            self.changed.notify_all();
        }
        /// What writer `id` is doing now (`settle`'s message, if it hangs).
        pub(super) fn doing(&self, id: u64, what: &'static str) {
            let mut state = self.lock();
            let generation = state.generation;
            if let Some(writer) = state.writers.get_mut(&id) {
                if what.starts_with("serving") {
                    writer.turn = generation;
                }
                writer.doing = what;
            }
        }
        pub(super) fn passed(&self, pass: &crate::retention::Pass) {
            self.lock().passes.push(Pass {
                usage: pass.usage,
                failed: pass.failed.clone(),
                removed: pass.removed.clone(),
            });
        }
    }
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::*;
    /// A writer that never goes idle fails `settle` within its bound, and the
    /// failure says what that writer was doing.
    #[test]
    fn a_writer_that_never_goes_idle_fails_settle_with_what_it_was_doing() {
        let clock = SimClock::new(1_000);
        let bell = Arc::new(Bell {
            rung: std::sync::Mutex::new(false),
            ring: std::sync::Condvar::new(),
        });
        let mut time = WriterTime::new(Clock::Simulated(clock.clone()), bell);
        time.turn();
        let _ = time.now();
        let started = Instant::now();
        let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            clock.settle_within(Duration::from_millis(50));
        }))
        .expect_err("settle returned with a busy writer");
        assert!(started.elapsed() < Duration::from_secs(10));
        let message = failure
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_default();
        assert!(
            message.contains("did not go idle") && message.contains("running its retention gate"),
            "{message}"
        );
        // Once it goes idle at the current generation, settle returns.
        let (writer, id) = time.simulated.as_ref().expect("on the simulated clock");
        writer.idle(*id, time.read, true);
        clock.settle_within(Duration::from_millis(50));
    }
}
