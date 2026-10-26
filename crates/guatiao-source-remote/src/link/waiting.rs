// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! How long a waiting thread polls before it sleeps.
//!
//! Polling saves the wake-up a sleeping thread costs, but only when what
//! it waits for comes while it polls. So a thread polls just past the
//! typical wait it has seen (an answer's round trip, the gap between the
//! peer's calls), not at all when that is longer than the cap, and not
//! while as many threads poll as there are cores. It never polls less
//! than [`FLOOR`]: a wait missed by a little costs a whole wake-up, which
//! on a quick link is many times the wait.

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use core::time::Duration;

/// The typical wait a link starts from, before it has seen any.
const FIRST: Duration = Duration::from_micros(50);

/// The least a thread polls, unless the cap is less.
const FLOOR: Duration = Duration::from_micros(50);

pub(crate) struct Waiting {
    cap: Duration,
    /// The typical wait, in nanoseconds: it follows a shorter one quickly
    /// and a longer one slowly, so a slow call now and then does not
    /// stretch it.
    typical: AtomicU64,
    polling: AtomicUsize,
    cores: usize,
}

/// A thread counted as polling until dropped.
pub(crate) struct Polling<'a>(&'a AtomicUsize);

impl Drop for Polling<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Waiting {
    pub(crate) fn new(cap: Duration) -> Waiting {
        Waiting {
            cap,
            typical: AtomicU64::new(FIRST.as_nanos() as u64),
            polling: AtomicUsize::new(0),
            cores: std::thread::available_parallelism().map_or(1, |n| n.get()),
        }
    }

    /// How long to poll now, with this thread counted as polling for as
    /// long as the guard lives; zero when polling would not pay.
    pub(crate) fn poll(&self) -> (Duration, Option<Polling<'_>>) {
        let typical = Duration::from_nanos(self.typical.load(Ordering::Relaxed));
        let past = typical + typical / 2;
        if past > self.cap || self.cap.is_zero() {
            return (Duration::ZERO, None);
        }
        if self.polling.fetch_add(1, Ordering::Relaxed) >= self.cores {
            self.polling.fetch_sub(1, Ordering::Relaxed);
            return (Duration::ZERO, None);
        }
        (past.max(FLOOR.min(self.cap)), Some(Polling(&self.polling)))
    }

    /// Notes a wait that lasted `took`.
    pub(crate) fn saw(&self, took: Duration) {
        let e = self.typical.load(Ordering::Relaxed);
        let s = u64::try_from(took.as_nanos()).unwrap_or(u64::MAX);
        let next = if s < e {
            e - (e - s) / 4
        } else {
            e + (s.min(e.saturating_mul(4)) - e) / 16
        };
        self.typical.store(next.max(1), Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn micros(w: &Waiting) -> u64 {
        w.typical.load(Ordering::Relaxed) / 1000
    }

    #[test]
    fn it_settles_on_the_round_trip_and_shrugs_off_a_slow_call() {
        let w = Waiting::new(Duration::from_micros(200));
        for _ in 0..100 {
            w.saw(Duration::from_micros(80));
        }
        assert!((79..=80).contains(&micros(&w)), "{}", micros(&w));
        let spin = w.poll().0;
        assert!(spin > Duration::from_micros(115) && spin <= Duration::from_micros(120));
        w.saw(Duration::from_secs(1));
        assert!(micros(&w) < 100, "one slow call moved it to {}", micros(&w));
    }

    #[test]
    fn a_quick_link_polls_the_floor() {
        let w = Waiting::new(Duration::from_micros(200));
        for _ in 0..100 {
            w.saw(Duration::from_micros(2));
        }
        assert_eq!(w.poll().0, FLOOR);
        assert_eq!(
            Waiting::new(Duration::from_micros(20)).poll().0,
            Duration::ZERO,
            "a typical 50 us wait is past a 20 us cap"
        );
    }

    #[test]
    fn a_wait_past_the_cap_is_slept_not_polled() {
        let w = Waiting::new(Duration::from_micros(200));
        for _ in 0..200 {
            w.saw(Duration::from_millis(5));
        }
        assert_eq!(w.poll().0, Duration::ZERO);
        // And it comes back once answers are quick again.
        for _ in 0..40 {
            w.saw(Duration::from_micros(10));
        }
        assert!(w.poll().0 > Duration::ZERO);
        assert_eq!(Waiting::new(Duration::ZERO).poll().0, Duration::ZERO);
    }

    #[test]
    fn no_more_threads_poll_than_there_are_cores() {
        let w = Waiting::new(Duration::from_micros(200));
        let guards: Vec<_> = (0..w.cores).map(|_| w.poll()).collect();
        assert!(guards.iter().all(|(d, g)| !d.is_zero() && g.is_some()));
        assert_eq!(w.poll().0, Duration::ZERO);
        drop(guards);
        assert!(w.poll().0 > Duration::ZERO);
    }
}
