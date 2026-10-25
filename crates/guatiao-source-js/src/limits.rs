// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A source's limits, kept by QuickJS's own hooks: its interrupt handler
//! for operations and time, its memory limit, and its stack size for
//! depth.

use alloc::rc::Rc;
use core::cell::{Cell, RefCell};

use guatiao_bridge::{BridgeError, Budget, Limits};
use rquickjs::Runtime;

use crate::frame::RawRuntime;

/// How many operations one interrupt poll stands for: QuickJS polls once
/// per this many calls and loop iterations, and the poll is its only
/// look at the budget.
pub(crate) const POLL: u64 = 10_000;

/// The stack one level of nested calls takes, in bytes, as measured on
/// Linux x86-64 (an empty function recurses 354 times in 256 KiB).
const STACK_PER_LEVEL: u64 = 740;
/// The least stack QuickJS may be given for a depth limit's levels, and
/// the most it is given in all: the engine thread has 2 MiB, and the
/// conversion code and the host's frames sit on it too.
const STACK_LEAST: u64 = 16 * 1024;
const STACK_MOST: u64 = 1024 * 1024;
/// What a call needs below its entry before its script recurses at all:
/// parsing and running a module, the prelude and the module loader. A
/// small library opens in 16 to 37 KiB in a release build and 111 to 148
/// KiB in a debug one (measured on Windows ARM64).
const STACK_ENTRY: u64 = if cfg!(debug_assertions) {
    256 * 1024
} else {
    64 * 1024
};
const _: () = assert!(STACK_LEAST <= STACK_MOST - STACK_ENTRY);

/// The memory kept back from the limit so that the error which stops a
/// call can always be made: with none, QuickJS throws `null` for want of
/// memory, which a script's `catch` catches and so keeps running.
const RESERVE: u64 = 128 * 1024;

#[derive(Default)]
struct State {
    /// The calls running, innermost last: each spends from its own budget.
    calls: RefCell<Vec<Budget>>,
    /// Why QuickJS was told to stop the call, until that call ends.
    stop: RefCell<Option<BridgeError>>,
    /// Whether a stop is pending: what a host call asks before it goes.
    stopped: Rc<Cell<bool>>,
}

/// The limits a runtime runs under, and the calls spending against them.
pub(crate) struct Meter {
    limits: Limits,
    state: Rc<State>,
    raw: RawRuntime,
    /// What QuickJS is told while a call runs: the limit less the reserve.
    normal: usize,
    /// The stack QuickJS is given below a call's entry.
    stack: usize,
}

/// A runtime that finds no stack and no memory for anything, so that what
/// a stopped call left queued ends without running: each job fails at its
/// first call, and cannot queue another. Puts the usual limits back when
/// it goes, however it goes.
pub(crate) struct Starved<'a>(&'a Meter);

impl Starved<'_> {
    /// Again nothing, should anything have raised the limits.
    pub(crate) fn again(&self) {
        self.0.raw.set_memory_limit(1);
        self.0.raw.set_max_stack_size(1);
    }
}

impl Drop for Starved<'_> {
    fn drop(&mut self) {
        self.0.raw.set_memory_limit(self.0.normal);
        self.0.raw.set_max_stack_size(self.0.stack);
    }
}

/// Ends a call's accounting, however the call ends.
struct Ending<'a> {
    meter: &'a Meter,
    /// The stop of the call this one is inside, if it had stopped.
    outer: Option<BridgeError>,
}

impl Drop for Ending<'_> {
    fn drop(&mut self) {
        let state = &self.meter.state;
        state.calls.borrow_mut().pop();
        state.stop.borrow_mut().take();
        let outer = self.outer.take();
        state.stopped.set(outer.is_some());
        *state.stop.borrow_mut() = outer;
        self.meter.raw.set_memory_limit(self.meter.normal);
    }
}

impl Meter {
    /// Puts `limits` on `runtime`: memory and stack at once, operations
    /// and time through an interrupt handler that counts only while a call
    /// runs ([`Meter::run`]).
    pub(crate) fn install(runtime: &Runtime, limits: Limits) -> Result<Meter, rquickjs::Error> {
        let state = Rc::new(State::default());
        let raw = RawRuntime::of(runtime)?;
        let full = limits
            .memory
            .map_or(0, |m| usize::try_from(m).unwrap_or(usize::MAX));
        let normal = limits.memory.map_or(0, |m| {
            usize::try_from(m - RESERVE.min(m / 4)).unwrap_or(usize::MAX)
        });
        raw.set_memory_limit(normal);
        let stack = limits.depth.map_or(STACK_MOST, |depth| {
            depth
                .saturating_mul(STACK_PER_LEVEL)
                .clamp(STACK_LEAST, STACK_MOST - STACK_ENTRY)
                + STACK_ENTRY
        });
        let stack = usize::try_from(stack).unwrap_or(usize::MAX);
        runtime.set_max_stack_size(stack);
        if limits.operations.is_some() || limits.time.is_some() {
            let watching = Rc::clone(&state);
            runtime.set_interrupt_handler(Some(Box::new(move || {
                let mut calls = watching.calls.borrow_mut();
                let Some(budget) = calls.last_mut() else {
                    return false;
                };
                match budget.spend(POLL) {
                    Ok(()) => false,
                    Err(stop) => {
                        watching.stop.borrow_mut().get_or_insert(stop);
                        watching.stopped.set(true);
                        // The reserve is what the error QuickJS now makes
                        // is made of.
                        raw.set_memory_limit(full);
                        true
                    }
                }
            })));
        }
        Ok(Meter {
            limits,
            state,
            raw,
            normal,
            stack,
        })
    }

    /// Measures QuickJS's stack from here: called where a call enters the
    /// engine, above every frame the call will run QuickJS in.
    pub(crate) fn set_stack_top_here(&self) {
        self.raw.set_stack_top_here();
    }

    /// Whether the call running was told to stop.
    pub(crate) fn stopped(&self) -> bool {
        self.state.stopped.get()
    }

    /// The same, for those that cannot reach the meter.
    pub(crate) fn stopped_flag(&self) -> Rc<Cell<bool>> {
        Rc::clone(&self.state.stopped)
    }

    /// Makes the runtime find no stack below the top
    /// [`Meter::set_stack_top_here`] measured, and no memory, until the
    /// guard goes.
    pub(crate) fn starve(&self) -> Starved<'_> {
        let starved = Starved(self);
        starved.again();
        starved
    }

    /// Runs `f` as one call, counted from nothing against every limit. A
    /// call the interrupt handler stopped answers the limit, whatever `f`
    /// came to: not QuickJS's "interrupted", and not success. A call made
    /// inside a stopped one leaves its stop as it found it.
    pub(crate) fn run<R, E: From<BridgeError>>(
        &self,
        f: impl FnOnce() -> Result<R, E>,
    ) -> Result<R, E> {
        let outer = self.state.stop.borrow_mut().take();
        self.state.stopped.set(false);
        self.state.calls.borrow_mut().push(self.limits.budget());
        let _ending = Ending { meter: self, outer };
        let answer = f();
        let stop = self.state.stop.borrow_mut().take();
        match stop {
            Some(stop) => Err(stop.into()),
            None => answer,
        }
    }
}
