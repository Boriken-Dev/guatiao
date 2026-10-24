// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A source's limits, kept by Rhai's own: its operation count and call
//! levels, the sizes of its strings, arrays and maps, and a progress
//! callback for time.

use alloc::rc::Rc;
use core::cell::RefCell;
use std::time::Instant;

use guatiao_bridge::{BridgeError, Limits};
use rhai::Engine;
use rhai::module_resolvers::DummyModuleResolver;

/// The deepest call limit a configuration may set: the stack a call that
/// deep needs is mapped from the system, up to a gigabyte in a debug build.
pub(crate) const MAX_DEPTH: u64 = 8192;

/// The native stack one level of script calls is given when the thread's
/// own does not hold the configured depth. Rhai recurses natively: a level
/// measures 7 to 12 KiB in a release build and 47 KiB in a debug one.
const PER_LEVEL: usize = if cfg!(debug_assertions) {
    128 * 1024
} else {
    24 * 1024
};

/// What one array element, and one map entry, are counted as when the
/// memory limit is turned into Rhai's size limits.
const ELEMENT: u64 = 16;
const ENTRY: u64 = 64;

/// The limits an engine runs under, and the time the running calls have.
pub(crate) struct Meter {
    pub(crate) limits: Limits,
    /// The deadline of each call running, innermost last.
    deadlines: Rc<RefCell<Vec<Instant>>>,
    /// The stack a call needs when a depth is configured; none otherwise,
    /// where Rhai's own default holds on any thread.
    stack: Option<usize>,
}

impl Meter {
    /// Puts `limits` on `engine`, which also has no way to import a module.
    pub(crate) fn install(engine: &mut Engine, limits: Limits) -> Result<Meter, BridgeError> {
        engine.set_module_resolver(DummyModuleResolver::new());
        // Rhai's defaults differ between a debug and a release build.
        engine.set_max_expr_depths(64, 32);
        if let Some(n) = limits.operations {
            engine.set_max_operations(n.max(1));
        }
        let mut stack = None;
        if let Some(d) = limits.depth {
            if d > MAX_DEPTH {
                return Err(BridgeError::config(
                    "limits",
                    format!("`depth` is at most {MAX_DEPTH} here, not {d}"),
                ));
            }
            let levels = usize::try_from(d).unwrap_or(usize::MAX).max(1);
            engine.set_max_call_levels(levels);
            stack = Some(levels.saturating_mul(PER_LEVEL));
        }
        if let Some(bytes) = limits.memory {
            let size = |per: u64| usize::try_from((bytes / per).max(1)).unwrap_or(usize::MAX);
            engine
                .set_max_string_size(size(1))
                .set_max_array_size(size(ELEMENT))
                .set_max_map_size(size(ENTRY));
        }
        let deadlines = Rc::new(RefCell::new(Vec::new()));
        if limits.time.is_some() {
            let running = Rc::clone(&deadlines);
            engine.on_progress(move |count| {
                // The clock is read every 64 operations.
                if count & 63 != 0 {
                    return None;
                }
                let past = running.borrow().last().is_some_and(|d| Instant::now() > *d);
                past.then(|| "time_ms".into())
            });
        }
        Ok(Meter {
            limits,
            deadlines,
            stack,
        })
    }

    /// Runs `f` as one call: its time starts now, within the time of the
    /// call it is inside, and its stack holds the configured depth.
    pub(crate) fn run<R>(&self, f: impl FnOnce() -> R) -> R {
        /// Ends the call's deadline however `f` ends.
        struct Deadline<'a>(Option<&'a RefCell<Vec<Instant>>>);
        impl Drop for Deadline<'_> {
            fn drop(&mut self) {
                if let Some(running) = self.0 {
                    running.borrow_mut().pop();
                }
            }
        }
        let _deadline = Deadline(self.limits.time.map(|t| {
            let own = Instant::now() + t;
            let mut running = self.deadlines.borrow_mut();
            let inside = running.last().map_or(own, |outer| own.min(*outer));
            running.push(inside);
            &*self.deadlines
        }));
        match self.stack {
            Some(need) => stacker::maybe_grow(need, need + (1 << 20), f),
            None => f(),
        }
    }
}
