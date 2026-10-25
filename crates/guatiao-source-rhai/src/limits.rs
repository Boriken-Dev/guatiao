// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A source's limits, kept by Rhai's own: its operation count and call
//! levels, the sizes of its strings, arrays and maps, and a progress
//! callback for time. Rhai recurses on the native stack, for calls and
//! for the nesting of its values, so the engine thread's stack is sized
//! from the limits, and a call that would run past it stops instead.

use alloc::rc::Rc;
use core::cell::{Cell, RefCell};
use std::time::Instant;

use guatiao_bridge::{BridgeError, Limits};
use rhai::module_resolvers::DummyModuleResolver;
use rhai::{Dynamic, Engine, EvalAltResult, FnPtr, Map};

/// The deepest call limit a configuration may set.
pub(crate) const MAX_DEPTH: u64 = 8192;

/// The most operations a configuration may set: the stack the engine
/// thread reserves grows with them (see [`PER_NEST`]).
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

/// The native stack one level of nested values is given. Rhai copies,
/// drops, hashes and formats a nested array, map or closure by recursion
/// within one operation, which no check between operations sees. The most
/// measured is 496 bytes a level in a release build (formatting maps,
/// x86-64) and 2 353 in a debug one (copying maps, x86-64); ARM64 takes
/// less. Each level costs at least one operation: a literal is one, a
/// closure capturing the last nine.
const PER_NEST: usize = if cfg!(debug_assertions) { 3072 } else { 640 };

/// Stack kept free below a call's script beyond what its nested values
/// need: what one step of Rhai's evaluator takes between two operations.
/// Below it the call stops at its depth, however it recursed.
const RED: usize = if cfg!(debug_assertions) {
    1024 * 1024
} else {
    256 * 1024
};

/// The nesting the stack holds when no operations limit bounds it, and at
/// least: what a library keeps is dropped within it.
const KEPT_LEVELS: usize = 100_000;

/// What the stack guard's stop is called, for the error it makes.
pub(crate) const DEPTH_TOKEN: &str = "depth";

/// The deepest map `to_json` writes, and the stack a level of it takes
/// (851 bytes in a release build, 4 599 in a debug one, measured).
const JSON_DEPTH: usize = 512;
/// The most values `to_json` writes.
const JSON_VALUES: usize = 1_000_000;
const PER_JSON_LEVEL: usize = if cfg!(debug_assertions) {
    8 * 1024
} else {
    2 * 1024
};

// `to_json` is a call into Rust, so the stack guard has left it the floor.
const _: () = assert!(JSON_DEPTH * PER_JSON_LEVEL <= KEPT_LEVELS * PER_NEST);

/// Stack for what runs under a call: Rhai's own frames, the bridge's.
const BASE: usize = 256 * 1024;

/// What one array element, and one map entry, are counted as when the
/// memory limit is turned into Rhai's size limits.
const ELEMENT: u64 = 16;
const ENTRY: u64 = 64;

/// The stack a set of limits needs, a value past its cap taken at the cap.
struct Stack {
    /// What the values a call can nest need below the deepest point its
    /// script reaches: the stack guard stops a call with less left. Values
    /// a library kept are dropped between calls, with the whole stack, or
    /// inside one (the host letting go of an object), within this floor.
    floor: usize,
    /// The engine thread's whole stack.
    whole: usize,
}

impl Stack {
    fn of(limits: &Limits) -> Stack {
        let levels = limits.depth.map_or(DEFAULT_LEVELS, |d| {
            usize::try_from(d.clamp(1, MAX_DEPTH)).unwrap_or(usize::MAX)
        });
        let nested = limits.operations.map_or(KEPT_LEVELS, |n| {
            usize::try_from(n.min(MAX_OPERATIONS))
                .unwrap_or(usize::MAX)
                .max(KEPT_LEVELS)
        });
        let floor = nested.saturating_mul(PER_NEST).saturating_add(RED);
        Stack {
            floor,
            whole: BASE
                .saturating_add(levels.saturating_mul(PER_LEVEL))
                .saturating_add(floor),
        }
    }
}

/// The engine thread's stack for `limits`: every call level they allow and,
/// below the deepest, the nesting their operations can build. None for a
/// limit past its cap, which the engine refuses as it starts.
pub(crate) fn thread_stack(limits: &Limits) -> Option<usize> {
    let past = limits.operations.is_some_and(|n| n > MAX_OPERATIONS)
        || limits.depth.is_some_and(|d| d > MAX_DEPTH);
    (!past).then(|| Stack::of(limits).whole)
}

/// Where the engine thread's stack begins and how large it is, for the
/// targets where `stacker` cannot read a thread's stack (Android and iOS
/// among them). Stacks grow down on every target this builds for.
#[derive(Clone, Copy)]
struct Extent {
    top: usize,
    size: usize,
}

impl Extent {
    /// About where the stack pointer is.
    #[inline(never)]
    fn here() -> usize {
        let marker = 0u8;
        core::hint::black_box(core::ptr::addr_of!(marker)) as usize
    }

    /// The stack left at `at`.
    fn left(self, at: usize) -> usize {
        self.size.saturating_sub(self.top.saturating_sub(at))
    }

    /// The stack left here: as the system reports it, else as measured
    /// from where the engine started.
    fn remaining(self) -> usize {
        stacker::remaining_stack().unwrap_or_else(|| self.left(Extent::here()))
    }
}

/// The limits an engine runs under, and the time the running calls have.
pub(crate) struct Meter {
    /// The limits as they are kept: a depth always, the configured or the
    /// default one.
    pub(crate) limits: Limits,
    /// The deadline of each call running, innermost last.
    deadlines: Rc<RefCell<Vec<Instant>>>,
    /// Set when the stack guard stopped the running call: every operation
    /// after it stops too, so that no fallback a built-in takes on an error
    /// goes on with the script.
    exhausted: Rc<Cell<bool>>,
}

/// Whether `d` nests containers more than `left` levels deep, or holds
/// more than `nodes` values, a value shared by two containers counted in
/// each as JSON writes it twice; looking no further than that.
fn nests_beyond(d: &Dynamic, left: usize, nodes: &mut usize) -> bool {
    fn deeper<'a>(
        mut children: impl Iterator<Item = &'a Dynamic>,
        left: usize,
        nodes: &mut usize,
    ) -> bool {
        left == 0 || children.any(|c| nests_beyond(c, left - 1, nodes))
    }
    let Some(rest) = nodes.checked_sub(1) else {
        return true;
    };
    *nodes = rest;
    if let Some(map) = d.read_lock::<Map>() {
        return deeper(map.values(), left, nodes);
    }
    if let Some(array) = d.read_lock::<rhai::Array>() {
        return deeper(array.iter(), left, nodes);
    }
    if let Some(f) = d.read_lock::<FnPtr>()
        && f.is_curried()
    {
        return deeper(f.iter_curry(), left, nodes);
    }
    false
}

/// `map.to_json()`, refusing a map nested deeper than [`JSON_DEPTH`] or
/// holding more than [`JSON_VALUES`]: Rhai writes JSON by recursion within
/// one operation, and closures sharing what they capture make the text
/// grow twice as long a level.
fn to_json(map: &mut Map) -> Result<String, Box<EvalAltResult>> {
    let mut nodes = JSON_VALUES;
    if map
        .values()
        .any(|v| nests_beyond(v, JSON_DEPTH, &mut nodes))
    {
        return Err(format!(
            "a map nested deeper than {JSON_DEPTH} levels, or holding more than {JSON_VALUES} values, is not written as JSON"
        )
        .into());
    }
    Ok(rhai::format_map_as_json(map))
}

impl Meter {
    /// Puts `limits` on `engine`, which also has no way to import a module.
    pub(crate) fn install(engine: &mut Engine, mut limits: Limits) -> Result<Meter, BridgeError> {
        engine.set_module_resolver(DummyModuleResolver::new());
        // A script's own output is not the host's: its standard output may
        // be a protocol.
        engine.on_print(|_| {});
        engine.on_debug(|_, _, _| {});
        engine.register_fn("to_json", to_json);
        engine.set_max_expr_depths(64, FUNCTION_EXPR_DEPTH);
        if let Some(n) = limits.operations {
            if n > MAX_OPERATIONS {
                return Err(BridgeError::config(
                    "limits",
                    format!("`operations` is at most {MAX_OPERATIONS} here, not {n}"),
                ));
            }
            engine.set_max_operations(n.max(1));
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
        let stack = Stack::of(&limits);
        // The engine starts near the top of the thread its stack was sized
        // for (`RhaiEngine::stack_size`), on which it then runs.
        let extent = Extent {
            top: Extent::here(),
            size: stack.whole,
        };
        let deadlines = Rc::new(RefCell::new(Vec::new()));
        let exhausted = Rc::new(Cell::new(false));
        let running = Rc::clone(&deadlines);
        let stopped = Rc::clone(&exhausted);
        let timed = limits.time.is_some();
        let floor = stack.floor;
        engine.on_progress(move |count| {
            if stopped.get() || extent.remaining() < floor {
                stopped.set(true);
                return Some(DEPTH_TOKEN.into());
            }
            // The clock is read every 64 operations.
            if !timed || count & 63 != 0 {
                return None;
            }
            let past = running.borrow().last().is_some_and(|d| Instant::now() > *d);
            past.then(|| "time_ms".into())
        });
        Ok(Meter {
            limits,
            deadlines,
            exhausted,
        })
    }

    /// Runs `f` as one call: its time starts now, within the time of the
    /// call it is inside.
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
        /// Puts back whether the call this one is inside was stopped.
        struct Outer<'a>(&'a Cell<bool>, bool);
        impl Drop for Outer<'_> {
            fn drop(&mut self) {
                self.0.set(self.1);
            }
        }
        let _outer = Outer(&self.exhausted, self.exhausted.replace(false));
        let _deadline = Deadline(self.limits.time.map(|t| {
            let own = Instant::now() + t;
            let mut running = self.deadlines.borrow_mut();
            let inside = running.last().map_or(own, |outer| own.min(*outer));
            running.push(inside);
            &*self.deadlines
        }));
        f()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_thread_holds_every_call_level_and_the_nesting_below_them() {
        let unset = Stack::of(&Limits::default());
        assert!(unset.floor >= KEPT_LEVELS * PER_NEST);
        assert!(unset.whole >= unset.floor + DEFAULT_LEVELS * PER_LEVEL);
        let limits = Limits {
            operations: Some(2_000_000),
            depth: Some(1000),
            ..Limits::default()
        };
        let set = Stack::of(&limits);
        assert!(set.floor >= 2_000_000 * PER_NEST);
        assert!(set.whole >= set.floor + 1000 * PER_LEVEL);
        assert_eq!(thread_stack(&limits), Some(set.whole));
    }

    #[test]
    fn a_limit_past_its_cap_reserves_nothing() {
        for limits in [
            Limits {
                operations: Some(MAX_OPERATIONS + 1),
                ..Limits::default()
            },
            Limits {
                depth: Some(MAX_DEPTH + 1),
                ..Limits::default()
            },
        ] {
            assert_eq!(thread_stack(&limits), None, "{limits:?}");
        }
    }

    #[test]
    fn the_stack_left_is_measured_from_where_the_engine_started() {
        let size = 64 << 20;
        let (measured, reported) = std::thread::Builder::new()
            .stack_size(size)
            .spawn(move || {
                let extent = Extent {
                    top: Extent::here(),
                    size,
                };
                #[inline(never)]
                fn down(extent: Extent, levels: usize) -> (usize, Option<usize>) {
                    let pad = core::hint::black_box([0u8; 1024]);
                    if levels == 0 {
                        let left = (extent.left(Extent::here()), stacker::remaining_stack());
                        return (left.0 + usize::from(pad[0]), left.1);
                    }
                    down(extent, levels - 1)
                }
                down(extent, 8 * 1024)
            })
            .expect("spawned")
            .join()
            .expect("joined");
        assert!(measured < size - (8 << 20), "{measured}");
        // Where the system reports its own, the two agree within a frame or
        // two above the engine's start.
        if let Some(reported) = reported {
            assert!(
                measured.abs_diff(reported) < 256 * 1024,
                "{measured} {reported}"
            );
        }
    }
}
