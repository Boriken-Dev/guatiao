// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A source's limits, kept by Rhai's own: its operation count and call
//! levels, the sizes of its strings, arrays and maps, and a progress
//! callback for time. Rhai recurses on the native stack, for calls and
//! for the nesting of its values, so every call runs on a stack sized
//! from the limits.

use alloc::rc::Rc;
use core::cell::RefCell;
use std::time::Instant;

use guatiao_bridge::{BridgeError, Limits};
use rhai::Engine;
use rhai::module_resolvers::DummyModuleResolver;

/// The deepest call limit a configuration may set.
pub(crate) const MAX_DEPTH: u64 = 8192;

/// The most operations a configuration may set: the stack a call needs for
/// the values it can nest grows with them (see [`PER_NEST`]).
pub(crate) const MAX_OPERATIONS: u64 = 16_777_216;

/// Call levels when the configuration sets none.
const DEFAULT_LEVELS: usize = 32;

/// The deepest expression nesting in a function body or a closure. With
/// it, one level of calls takes at most 14 KiB of native stack in a release
/// build and 85 KiB in a debug one, over every construct measured.
const FUNCTION_EXPR_DEPTH: usize = 16;

/// The native stack one level of script calls is given: twice what the
/// deepest level measured takes.
const PER_LEVEL: usize = if cfg!(debug_assertions) {
    192 * 1024
} else {
    32 * 1024
};

/// The native stack one level of nested values is given. Rhai copies and
/// drops a nested array, map or closure by recursion, 213 bytes a level in
/// a release build and 559 in a debug one, and no size limit bounds how
/// deep values nest.
const PER_NEST: usize = if cfg!(debug_assertions) { 1024 } else { 320 };

/// Operations that building one more level of nesting takes at least (the
/// fewest measured is four).
const OPS_PER_NEST: u64 = 3;

/// Stack for what runs under a call: Rhai's own frames, the bridge's.
const BASE: usize = 256 * 1024;

/// What one array element, and one map entry, are counted as when the
/// memory limit is turned into Rhai's size limits.
const ELEMENT: u64 = 16;
const ENTRY: u64 = 64;

/// The limits an engine runs under, and the time the running calls have.
pub(crate) struct Meter {
    /// The limits as they are kept: a depth always, the configured or the
    /// default one.
    pub(crate) limits: Limits,
    /// The deadline of each call running, innermost last.
    deadlines: Rc<RefCell<Vec<Instant>>>,
    /// The native stack a call needs.
    stack: usize,
}

impl Meter {
    /// Puts `limits` on `engine`, which also has no way to import a module.
    pub(crate) fn install(engine: &mut Engine, mut limits: Limits) -> Result<Meter, BridgeError> {
        engine.set_module_resolver(DummyModuleResolver::new());
        // A script's own output is not the host's: its standard output may
        // be a protocol.
        engine.on_print(|_| {});
        engine.on_debug(|_, _, _| {});
        engine.set_max_expr_depths(64, FUNCTION_EXPR_DEPTH);
        let mut nested = 0usize;
        if let Some(n) = limits.operations {
            if n > MAX_OPERATIONS {
                return Err(BridgeError::config(
                    "limits",
                    format!("`operations` is at most {MAX_OPERATIONS} here, not {n}"),
                ));
            }
            engine.set_max_operations(n.max(1));
            nested = usize::try_from(n / OPS_PER_NEST + 1)
                .unwrap_or(usize::MAX)
                .saturating_mul(PER_NEST);
        }
        let levels = match limits.depth {
            Some(d) if d > MAX_DEPTH => {
                return Err(BridgeError::config(
                    "limits",
                    format!("`depth` is at most {MAX_DEPTH} here, not {d}"),
                ));
            }
            Some(d) => usize::try_from(d).unwrap_or(usize::MAX).max(1),
            None => DEFAULT_LEVELS,
        };
        engine.set_max_call_levels(levels);
        limits.depth = Some(levels as u64);
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
            stack: BASE
                .saturating_add(levels.saturating_mul(PER_LEVEL))
                .saturating_add(nested),
        })
    }

    /// Runs `f` as one call: its time starts now, within the time of the
    /// call it is inside, and its stack holds the call levels and the
    /// nesting the limits allow, mapped only when the thread's own would
    /// not hold them.
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
        stacker::maybe_grow(self.stack, self.stack.saturating_add(1 << 20), f)
    }

    /// Runs `f`, which drops values a library kept, on a stack that holds
    /// the nesting of a hundred thousand levels: Rhai drops a nested value
    /// by recursion.
    pub(crate) fn dropping<R>(&self, f: impl FnOnce() -> R) -> R {
        stacker::grow(BASE + 100_000 * PER_NEST, f)
    }
}
