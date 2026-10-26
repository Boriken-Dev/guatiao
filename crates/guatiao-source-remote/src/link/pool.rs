// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The threads a link's calls run on, and the bound on calls in flight.

use core::time::Duration;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Condvar, Mutex, PoisonError};
use std::thread;
use std::time::Instant;

use super::waiting::Waiting;

/// A job for a pool thread.
pub(crate) type Job = Box<dyn FnOnce() + Send>;

/// How long an idle pool thread waits for work before it ends.
const IDLE: Duration = Duration::from_secs(30);

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
        }
    }

    /// Runs `job` on an idle thread or a new one; hands it back when
    /// every thread is busy and no more may start.
    pub(crate) fn run(self: &alloc::sync::Arc<Self>, job: Job) -> Result<(), Job> {
        let mut job = job;
        loop {
            let mut state = lock(&self.state);
            if let Some((_, tx)) = state.idle.pop() {
                drop(state);
                match tx.send(job) {
                    Ok(()) => return Ok(()),
                    // That thread has just ended: try the next.
                    Err(mpsc::SendError(back)) => {
                        job = back;
                        continue;
                    }
                }
            }
            if state.threads >= self.max {
                return Err(job);
            }
            state.threads += 1;
            state.next += 1;
            let me = state.next;
            drop(state);
            let pool = alloc::sync::Arc::clone(self);
            let started = thread::Builder::new()
                .name(self.name.clone())
                .spawn(move || pool.work(me, job));
            if let Err(e) = started {
                lock(&self.state).threads -= 1;
                // A thread that will not start is no place for the job.
                let _ = e;
                return Ok(());
            }
            return Ok(());
        }
    }

    fn work(self: alloc::sync::Arc<Self>, me: u64, first: Job) {
        let (tx, rx) = mpsc::channel::<Job>();
        let mut job = Some(first);
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
            match rx.recv_timeout(IDLE) {
                Ok(j) => {
                    self.waiting.saw(idle.elapsed());
                    job = Some(j);
                }
                Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => {
                    let mut state = lock(&self.state);
                    state.idle.retain(|(id, _)| *id != me);
                    // A job sent after the timeout and before the retain.
                    if let Ok(j) = rx.try_recv() {
                        drop(state);
                        job = Some(j);
                        continue;
                    }
                    state.threads -= 1;
                    return;
                }
            }
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
