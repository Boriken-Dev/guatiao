// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A source's limits, kept by QuickJS's own hooks: its interrupt handler
//! for operations and time, its memory limit, and its stack size for
//! depth.

use alloc::rc::Rc;
use core::cell::RefCell;

use guatiao_bridge::{BridgeError, Budget, Limits};
use rquickjs::Runtime;

/// How many operations one interrupt poll stands for: QuickJS polls once
/// per this many calls and loop iterations, and the poll is its only
/// look at the budget.
pub(crate) const POLL: u64 = 10_000;

/// The stack one level of nested calls takes, in bytes, as measured on
/// Linux x86-64 (an empty function recurses 354 times in 256 KiB).
const STACK_PER_LEVEL: u64 = 740;
/// The least and most stack QuickJS may be given: the engine thread has
/// 2 MiB, and the conversion code and the host's frames sit on it too.
const STACK_LEAST: u64 = 16 * 1024;
const STACK_MOST: u64 = 1024 * 1024;

#[derive(Default)]
struct State {
    /// The calls running, innermost last: each spends from its own budget.
    calls: RefCell<Vec<Budget>>,
    /// Why QuickJS was told to stop the call, until that call ends.
    stop: RefCell<Option<BridgeError>>,
}

/// The limits a runtime runs under, and the calls spending against them.
pub(crate) struct Meter {
    limits: Limits,
    state: Rc<State>,
}

impl Meter {
    /// Puts `limits` on `runtime`: memory and stack at once, operations
    /// and time through an interrupt handler that counts only while a call
    /// runs ([`Meter::run`]).
    pub(crate) fn install(runtime: &Runtime, limits: Limits) -> Meter {
        let state = Rc::new(State::default());
        if let Some(bytes) = limits.memory {
            runtime.set_memory_limit(usize::try_from(bytes).unwrap_or(usize::MAX));
        }
        let stack = limits.depth.map_or(STACK_MOST, |depth| {
            depth
                .saturating_mul(STACK_PER_LEVEL)
                .clamp(STACK_LEAST, STACK_MOST)
        });
        runtime.set_max_stack_size(usize::try_from(stack).unwrap_or(usize::MAX));
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
                        true
                    }
                }
            })));
        }
        Meter { limits, state }
    }

    /// The memory limit, in bytes, 0 for none: what QuickJS is told.
    pub(crate) fn memory(&self) -> usize {
        self.limits
            .memory
            .map_or(0, |m| usize::try_from(m).unwrap_or(usize::MAX))
    }

    /// Whether the call running was told to stop.
    pub(crate) fn stopped(&self) -> bool {
        self.state.stop.borrow().is_some()
    }

    /// Runs `f` as one call, counted from nothing against every limit. A
    /// call the interrupt handler stopped answers the limit, whatever `f`
    /// came to: not QuickJS's "interrupted", and not success.
    pub(crate) fn run<R, E: From<BridgeError>>(
        &self,
        f: impl FnOnce() -> Result<R, E>,
    ) -> Result<R, E> {
        self.state.calls.borrow_mut().push(self.limits.budget());
        let answer = f();
        self.state.calls.borrow_mut().pop();
        let stop = self.state.stop.borrow_mut().take();
        match stop {
            Some(stop) => Err(stop.into()),
            None => answer,
        }
    }
}
