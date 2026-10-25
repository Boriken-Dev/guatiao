// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A source's limits, kept by Rhai's own: its operation count and call
//! levels, the sizes of its strings, arrays and maps, and a progress
//! callback for time. Rhai recurses on the native stack, for calls and
//! for the nesting of its values, so every call runs on a stack sized
//! from the limits.

use alloc::rc::Rc;
use core::cell::{Cell, RefCell};
use std::time::Instant;

use guatiao_bridge::{BridgeError, Limits};
use rhai::module_resolvers::DummyModuleResolver;
use rhai::{Dynamic, Engine, EvalAltResult, FnPtr, Map};

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

/// The native stack one level of nested values is given. Rhai copies,
/// drops, measures and formats a nested array, map or closure by
/// recursion within one operation, and no size limit bounds how deep
/// values nest. A level costs at least four operations, so each level has
/// four thirds of this: at most 337 bytes were measured in a release build
/// on Windows ARM64 (formatting; copying 273, dropping 96 for arrays and
/// 240 for closures).
const PER_NEST: usize = if cfg!(debug_assertions) { 1024 } else { 320 };

/// Stack kept free below a call's script beyond what its nested values
/// need: what one step of Rhai's evaluator takes between two operations.
/// Below it the call stops at its depth, however it recursed.
const RED: usize = if cfg!(debug_assertions) {
    1024 * 1024
} else {
    256 * 1024
};

/// The nesting a value a library kept is dropped for when the operations
/// limit allows less, or there is none.
const KEPT_LEVELS: usize = 100_000;

/// What the stack guard's stop is called, for the error it makes.
pub(crate) const DEPTH_TOKEN: &str = "depth";

/// The deepest map `to_json` writes, and the stack a level of it takes
/// (851 bytes in a release build, 4 599 in a debug one, measured).
const JSON_DEPTH: usize = 512;
const PER_JSON_LEVEL: usize = if cfg!(debug_assertions) {
    8 * 1024
} else {
    2 * 1024
};

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
    /// The native stack dropping what a library kept needs.
    kept: usize,
    /// Set when the stack guard stopped the running call: every operation
    /// after it stops too, so that no fallback a built-in takes on an error
    /// goes on with the script.
    exhausted: Rc<Cell<bool>>,
}

/// Whether `d` nests containers more than `left` levels deep, looking no
/// deeper than that.
fn nests_deeper(d: &Dynamic, left: usize) -> bool {
    fn deeper<'a>(mut children: impl Iterator<Item = &'a Dynamic>, left: usize) -> bool {
        left == 0 || children.any(|c| nests_deeper(c, left - 1))
    }
    if let Some(map) = d.read_lock::<Map>() {
        return deeper(map.values(), left);
    }
    if let Some(array) = d.read_lock::<rhai::Array>() {
        return deeper(array.iter(), left);
    }
    if let Some(f) = d.read_lock::<FnPtr>()
        && f.is_curried()
    {
        return deeper(f.iter_curry(), left);
    }
    false
}

/// `map.to_json()`, refusing a map nested deeper than [`JSON_DEPTH`]: Rhai
/// writes JSON by recursion within one operation.
fn to_json(map: &mut Map) -> Result<String, Box<EvalAltResult>> {
    let need = JSON_DEPTH * PER_JSON_LEVEL;
    stacker::maybe_grow(need, need, || {
        if map.values().any(|v| nests_deeper(v, JSON_DEPTH)) {
            return Err(format!(
                "a map nested deeper than {JSON_DEPTH} levels is not written as JSON"
            )
            .into());
        }
        Ok(rhai::format_map_as_json(map))
    })
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
        let exhausted = Rc::new(Cell::new(false));
        let running = Rc::clone(&deadlines);
        let stopped = Rc::clone(&exhausted);
        let timed = limits.time.is_some();
        // What the values a call can nest need below the deepest point its
        // script reaches.
        let floor = nested.saturating_add(RED);
        engine.on_progress(move |count| {
            if stopped.get() || stacker::remaining_stack().is_some_and(|left| left < floor) {
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
            stack: BASE
                .saturating_add(levels.saturating_mul(PER_LEVEL))
                .saturating_add(floor),
            kept: BASE.saturating_add(nested.max(KEPT_LEVELS * PER_NEST)),
            exhausted,
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
        stacker::maybe_grow(self.stack, self.stack.saturating_add(1 << 20), f)
    }

    /// Runs `f`, which drops values a library kept, on a stack that holds
    /// what one call can nest under the limits, and at least a hundred
    /// thousand levels: Rhai drops a nested value by recursion.
    pub(crate) fn dropping<R>(&self, f: impl FnOnce() -> R) -> R {
        stacker::grow(self.kept, f)
    }
}
