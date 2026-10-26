// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The threads a link's calls run on, and the bound on calls in flight.

use alloc::sync::Arc;
use core::time::Duration;
use std::io;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Condvar, Mutex, PoisonError};
use std::thread;
use std::time::Instant;

use super::waiting::Waiting;

/// A job for a pool thread.
pub(crate) type Job = Box<dyn FnOnce() + Send>;

/// How long an idle pool thread waits for work before it ends.
const IDLE: Duration = Duration::from_secs(30);

/// Starts a thread; a test swaps in one that fails.
type Spawn = fn(thread::Builder, Box<dyn FnOnce() + Send>) -> io::Result<()>;

fn spawn(builder: thread::Builder, f: Box<dyn FnOnce() + Send>) -> io::Result<()> {
    builder.spawn(f).map(drop)
}

/// Where a test stops a pool's threads to order a race between them.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Point {
    /// `run` has taken an idle thread's place and not yet sent it the job.
    Taken,
    /// An idle thread's wait has just timed out.
    TimedOut,
    /// A thread has given up its place and is about to end.
    Leaving,
}

#[cfg(test)]
type Hook = Arc<dyn Fn(Point) + Send + Sync>;

/// Why the pool did not take a job, which the caller then refuses as BUSY.
#[derive(Debug)]
pub(crate) struct Busy(Option<io::Error>);

impl Busy {
    /// The refusal's words: `full` when every thread is busy and no more
    /// may start, else why a thread would not start.
    pub(crate) fn words(&self, full: &str) -> String {
        match &self.0 {
            None => full.to_string(),
            Some(e) => format!("the peer could not start a thread for it: {e}"),
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What arrives on `rx` within `spin`, polled rather than slept on: a
/// sender that finds its receiver awake wakes nobody, which on a busy
/// link is most of a hand-off's cost. `None` when nothing came (or the
/// sender is gone: the blocking receive that follows says which).
pub(crate) fn spin_recv<T>(rx: &Receiver<T>, spin: Duration) -> Option<T> {
    if spin.is_zero() {
        return None;
    }
    let until = Instant::now() + spin;
    loop {
        match rx.try_recv() {
            Ok(v) => return Some(v),
            Err(TryRecvError::Disconnected) => return None,
            Err(TryRecvError::Empty) => {
                if Instant::now() >= until {
                    return None;
                }
                core::hint::spin_loop();
            }
        }
    }
}

/// At most `max` threads, started as calls arrive and kept while busy.
pub(crate) struct Pool {
    state: Mutex<PoolState>,
    max: usize,
    name: String,
    /// How long an idle thread polls for its next job.
    waiting: Waiting,
    /// How long an idle thread waits for work before it ends.
    idle: Duration,
    spawn: Spawn,
    #[cfg(test)]
    hook: Option<Hook>,
}

struct PoolState {
    /// Threads waiting for work, by a number of their own.
    idle: Vec<(u64, Sender<Job>)>,
    threads: usize,
    next: u64,
}

impl Pool {
    pub(crate) fn new(max: usize, name: &str, spin: Duration) -> Pool {
        Pool {
            state: Mutex::new(PoolState {
                idle: Vec::new(),
                threads: 0,
                next: 0,
            }),
            max: max.max(1),
            name: name.to_string(),
            waiting: Waiting::new(spin),
            idle: IDLE,
            spawn,
            #[cfg(test)]
            hook: None,
        }
    }

    #[cfg(test)]
    fn at(&self, point: Point) {
        if let Some(hook) = &self.hook {
            hook(point);
        }
    }

    /// Runs `job` on an idle thread or a new one. Refuses it, dropping
    /// it, when every thread is busy and no more may start, or when a new
    /// thread will not start.
    pub(crate) fn run(self: &Arc<Self>, job: Job) -> Result<(), Busy> {
        let mut job = job;
        loop {
            let mut state = lock(&self.state);
            if let Some((_, tx)) = state.idle.pop() {
                // Immediately before the send, which the race test orders.
                #[cfg(test)]
                self.at(Point::Taken);
                // Sent under the lock (an unbounded send never blocks): a
                // thread whose wait timed out gives up its place under the
                // same lock and then checks its channel once, so the job is
                // either there when it checks or its place is already gone.
                let sent = tx.send(job);
                drop(state);
                match sent {
                    Ok(()) => return Ok(()),
                    // Not expected (a thread gives up its place before it
                    // ends); the next thread or a new one takes the job.
                    Err(mpsc::SendError(back)) => {
                        job = back;
                        continue;
                    }
                }
            }
            if state.threads >= self.max {
                return Err(Busy(None));
            }
            state.threads += 1;
            state.next += 1;
            let me = state.next;
            drop(state);
            // The job follows the spawn through the thread's channel, so a
            // thread that will not start leaves it here to refuse.
            let (tx, rx) = mpsc::channel::<Job>();
            let pool = Arc::clone(self);
            let theirs = tx.clone();
            let builder = thread::Builder::new().name(self.name.clone());
            if let Err(e) = (self.spawn)(builder, Box::new(move || pool.work(me, theirs, rx))) {
                lock(&self.state).threads -= 1;
                return Err(Busy(Some(e)));
            }
            // The new thread keeps its receiver until it has had this job.
            let _ = tx.send(job);
            return Ok(());
        }
    }

    fn work(self: Arc<Self>, me: u64, tx: Sender<Job>, rx: Receiver<Job>) {
        let _unwinding = Unwinding { pool: &self, me };
        let mut job = rx.recv().ok();
        loop {
            if let Some(j) = job.take() {
                j();
            }
            lock(&self.state).idle.push((me, tx.clone()));
            let idle = Instant::now();
            let (spin, polling) = self.waiting.poll();
            let polled = spin_recv(&rx, spin);
            drop(polling);
            if let Some(j) = polled {
                self.waiting.saw(idle.elapsed());
                job = Some(j);
                continue;
            }
            match rx.recv_timeout(self.idle) {
                Ok(j) => {
                    self.waiting.saw(idle.elapsed());
                    job = Some(j);
                }
                Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => {
                    #[cfg(test)]
                    self.at(Point::TimedOut);
                    let mut state = lock(&self.state);
                    state.idle.retain(|(id, _)| *id != me);
                    // A job sent after the timeout and before the retain.
                    if let Ok(j) = rx.try_recv() {
                        drop(state);
                        job = Some(j);
                        continue;
                    }
                    state.threads -= 1;
                    drop(state);
                    #[cfg(test)]
                    self.at(Point::Leaving);
                    return;
                }
            }
        }
    }
}

/// Gives a pool thread's place back when a job unwinds it, so a panic
/// does not shrink the pool for the link's life.
struct Unwinding<'a> {
    pool: &'a Pool,
    me: u64,
}

impl Drop for Unwinding<'_> {
    fn drop(&mut self) {
        if thread::panicking() {
            let mut state = lock(&self.pool.state);
            state.idle.retain(|(id, _)| *id != self.me);
            state.threads -= 1;
        }
    }
}

/// A count of calls in flight that blocks a caller past `max`.
pub(crate) struct Permits {
    held: Mutex<usize>,
    max: usize,
    freed: Condvar,
}

/// One call's place, given back when dropped.
pub(crate) struct Permit<'a>(&'a Permits);

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        *lock(&self.0.held) -= 1;
        self.0.freed.notify_one();
    }
}

impl Permits {
    pub(crate) fn new(max: usize) -> Permits {
        Permits {
            held: Mutex::new(0),
            max: max.max(1),
            freed: Condvar::new(),
        }
    }

    /// Waits for a place, at most until `deadline`.
    pub(crate) fn acquire(&self, deadline: Option<Instant>) -> Option<Permit<'_>> {
        let mut held = lock(&self.held);
        while *held >= self.max {
            held = match deadline {
                None => self
                    .freed
                    .wait(held)
                    .unwrap_or_else(PoisonError::into_inner),
                Some(d) => {
                    let now = Instant::now();
                    if now >= d {
                        return None;
                    }
                    self.freed
                        .wait_timeout(held, d - now)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0
                }
            };
        }
        *held += 1;
        Some(Permit(self))
    }

    /// Wakes every waiter, so a link that ended answers them.
    pub(crate) fn wake_all(&self) {
        self.freed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_threads(_: thread::Builder, _: Box<dyn FnOnce() + Send>) -> io::Result<()> {
        Err(io::Error::other("no threads left"))
    }

    #[test]
    fn a_job_whose_thread_will_not_start_is_refused_and_the_count_restored() {
        let mut pool = Pool::new(2, "pool-test", Duration::ZERO);
        pool.spawn = no_threads;
        let pool = Arc::new(pool);
        for _ in 0..3 {
            let busy = pool.run(Box::new(|| {})).expect_err("no thread ran it");
            assert!(busy.words("full").contains("no threads left"), "{busy:?}");
        }
        assert_eq!(lock(&pool.state).threads, 0);
    }

    /// The points a race has reached, and the test's own marks.
    #[derive(Default)]
    struct Race {
        reached: Mutex<Vec<&'static str>>,
        moved: Condvar,
    }

    impl Race {
        fn mark(&self, what: &'static str) {
            lock(&self.reached).push(what);
            self.moved.notify_all();
        }

        /// Whether `what` was marked within `within`.
        fn wait(&self, what: &'static str, within: Duration) -> bool {
            let reached = lock(&self.reached);
            let (reached, _) = self
                .moved
                .wait_timeout_while(reached, within, |r| !r.contains(&what))
                .unwrap_or_else(PoisonError::into_inner);
            reached.contains(&what)
        }
    }

    #[test]
    fn a_job_sent_to_a_thread_whose_wait_just_timed_out_still_runs() {
        let race = Arc::new(Race::default());
        let mut pool = Pool::new(1, "pool-test", Duration::ZERO);
        pool.idle = Duration::from_millis(1);
        let seen = Arc::clone(&race);
        pool.hook = Some(Arc::new(move |point| match point {
            // Hold the timed-out thread until `run` has taken its place.
            Point::TimedOut => {
                if !lock(&seen.reached).contains(&"timed out") {
                    seen.mark("timed out");
                    seen.wait("taken", Duration::from_secs(10));
                }
            }
            // Let the thread give up its place before the job is sent; a
            // pool sending under its lock waits here in vain, and goes on.
            Point::Taken => {
                seen.mark("taken");
                seen.wait("leaving", Duration::from_millis(300));
            }
            // Keep the thread's receiver alive until the job was sent.
            Point::Leaving => {
                if !lock(&seen.reached).contains(&"leaving") {
                    seen.mark("leaving");
                    seen.wait("sent", Duration::from_secs(10));
                }
            }
        }));
        let pool = Arc::new(pool);
        let (done_tx, done_rx) = mpsc::channel::<u32>();
        let done = done_tx.clone();
        pool.run(Box::new(move || done.send(1).unwrap())).unwrap();
        assert_eq!(done_rx.recv_timeout(Duration::from_secs(10)), Ok(1));
        assert!(race.wait("timed out", Duration::from_secs(10)));
        pool.run(Box::new(move || done_tx.send(2).unwrap()))
            .expect("the idle thread takes it");
        race.mark("sent");
        assert_eq!(
            done_rx.recv_timeout(Duration::from_secs(5)),
            Ok(2),
            "the job was lost with the thread that was ending"
        );
    }

    #[test]
    fn a_job_that_panics_gives_its_threads_place_back() {
        let pool = Arc::new(Pool::new(2, "pool-test", Duration::ZERO));
        pool.run(Box::new(|| panic!("a job that panics")))
            .expect("a thread takes it");
        let deadline = Instant::now() + Duration::from_secs(10);
        while lock(&pool.state).threads > 0 {
            assert!(
                Instant::now() < deadline,
                "the panicked thread still counts against the pool"
            );
            thread::sleep(Duration::from_millis(5));
        }
        // Every place is free again: `max` jobs run at once.
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let go_rx = Arc::new(Mutex::new(go_rx));
        let (done_tx, done_rx) = mpsc::channel::<()>();
        for _ in 0..2 {
            let (go, done) = (Arc::clone(&go_rx), done_tx.clone());
            pool.run(Box::new(move || {
                lock(&go).recv().unwrap();
                done.send(()).unwrap();
            }))
            .expect("a place is free");
        }
        for _ in 0..2 {
            go_tx.send(()).unwrap();
        }
        for _ in 0..2 {
            done_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        }
    }

    #[test]
    fn jobs_run_and_a_full_pool_refuses_in_its_callers_words() {
        let pool = Arc::new(Pool::new(1, "pool-test", Duration::ZERO));
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let (done_tx, done_rx) = mpsc::channel::<u32>();
        let done = done_tx.clone();
        pool.run(Box::new(move || {
            go_rx.recv().unwrap();
            done.send(1).unwrap();
        }))
        .unwrap();
        let busy = pool
            .run(Box::new(|| {}))
            .expect_err("its one thread is busy");
        assert_eq!(busy.words("full"), "full");
        go_tx.send(()).unwrap();
        assert_eq!(done_rx.recv_timeout(Duration::from_secs(10)), Ok(1));
        // The thread, idle again, takes the next job.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let done = done_tx.clone();
            match pool.run(Box::new(move || done.send(2).unwrap())) {
                Ok(()) => break,
                Err(_) if Instant::now() < deadline => thread::yield_now(),
                Err(e) => panic!("{e:?}"),
            }
        }
        assert_eq!(done_rx.recv_timeout(Duration::from_secs(10)), Ok(2));
    }
}
