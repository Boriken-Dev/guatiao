// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A source's limits, kept by Lua's own hooks: an instruction count
//! (Luau's interrupt) for operations, time and depth, mlua's allocator
//! for memory.

use alloc::rc::Rc;
use core::cell::RefCell;

use guatiao_bridge::{BridgeError, Budget, Limit, Limits};
use mlua::prelude::*;

/// How many instructions pass between two looks at the budget, at most.
const EVERY: u32 = 1000;

/// One call's spending.
struct Spend {
    budget: Budget,
    /// Past its limit: the hook runs on every instruction, so the next
    /// one outside a `pcall` stops the call.
    escalated: bool,
}

/// The limits a state runs under, and the calls spending against them.
pub(crate) struct Meter {
    limits: Limits,
    every: u32,
    calls: Rc<RefCell<Vec<Spend>>>,
}

impl Meter {
    /// Puts `limits` on `lua`: the memory limit at once, the rest through a
    /// hook that counts only while a call runs ([`Meter::run`]).
    pub(crate) fn install(lua: &Lua, limits: Limits) -> LuaResult<Meter> {
        let every = limits
            .operations
            .map_or(EVERY, |n| u32::try_from(n).unwrap_or(EVERY).clamp(1, EVERY));
        let meter = Meter {
            limits,
            every,
            calls: Rc::new(RefCell::new(Vec::new())),
        };
        if let Some(bytes) = limits.memory {
            lua.set_memory_limit(usize::try_from(bytes).unwrap_or(usize::MAX))?;
        }
        if meter.counts() {
            meter.hook(lua, meter.every)?;
        }
        Ok(meter)
    }

    /// The memory limit, for telling Lua's own memory error from words.
    pub(crate) fn memory(&self) -> Option<u64> {
        self.limits.memory
    }

    fn counts(&self) -> bool {
        self.limits.operations.is_some()
            || self.limits.time.is_some()
            || self.limits.depth.is_some()
    }

    /// Runs `f` as one call: counted from nothing, against every limit.
    pub(crate) fn run<R>(&self, lua: &Lua, f: impl FnOnce() -> R) -> R {
        if !self.counts() {
            return f();
        }
        self.calls.borrow_mut().push(Spend {
            budget: self.limits.budget(),
            escalated: false,
        });
        let answer = f();
        let ended = self.calls.borrow_mut().pop();
        if ended.is_some_and(|s| s.escalated) {
            // Back to the usual rate, unless the call underneath is past its
            // limit too.
            let outer = self.calls.borrow().last().is_some_and(|s| s.escalated);
            let _ = self.hook(lua, if outer { 1 } else { self.every });
        }
        answer
    }

    #[cfg(not(feature = "luau"))]
    fn hook(&self, lua: &Lua, every: u32) -> LuaResult<()> {
        counting(lua, Rc::clone(&self.calls), self.limits.depth, every)
    }
}

/// The instruction hook, every `every` instructions: spends from the
/// running call's budget and looks at the stack's depth. Past a limit it
/// installs itself again at every instruction, since a `pcall` around
/// the code that was stopped would otherwise let the loop around it spin
/// on.
#[cfg(not(feature = "luau"))]
fn counting(
    lua: &Lua,
    calls: Rc<RefCell<Vec<Spend>>>,
    depth: Option<u64>,
    every: u32,
) -> LuaResult<()> {
    lua.set_global_hook(
        mlua::HookTriggers::new().every_nth_instruction(every),
        move |lua, _| {
            let stop = {
                let mut running = calls.borrow_mut();
                let Some(call) = running.last_mut() else {
                    return Ok(mlua::VmState::Continue);
                };
                let mut stop = call.budget.spend(u64::from(every)).err();
                if stop.is_none()
                    && let Some(d) = depth
                    && lua
                        .inspect_stack(usize::try_from(d).unwrap_or(usize::MAX), |_| ())
                        .is_some()
                {
                    stop = Some(BridgeError::Limit {
                        limit: Limit::Depth,
                        at: d,
                    });
                }
                let first = stop.is_some() && !call.escalated;
                if first {
                    call.escalated = true;
                }
                stop.map(|e| (e, first))
            };
            match stop {
                None => Ok(mlua::VmState::Continue),
                Some((e, first)) => {
                    if first && every != 1 {
                        counting(lua, Rc::clone(&calls), depth, 1)?;
                    }
                    Err(LuaError::external(e))
                }
            }
        },
    )
}

#[cfg(feature = "luau")]
impl Meter {
    /// Luau's interrupt runs at every call and loop iteration, so a stop
    /// caught by `pcall` meets the next one at once: no escalation.
    fn hook(&self, lua: &Lua, _every: u32) -> LuaResult<()> {
        let calls = Rc::clone(&self.calls);
        let depth = self.limits.depth;
        lua.set_interrupt(move |lua| {
            let mut calls = calls.borrow_mut();
            let Some(call) = calls.last_mut() else {
                return Ok(mlua::VmState::Continue);
            };
            if let Err(e) = call.budget.spend(1) {
                return Err(LuaError::external(e));
            }
            if let Some(d) = depth
                && lua
                    .inspect_stack(usize::try_from(d).unwrap_or(usize::MAX), |_| ())
                    .is_some()
            {
                return Err(LuaError::external(BridgeError::Limit {
                    limit: Limit::Depth,
                    at: d,
                }));
            }
            Ok(mlua::VmState::Continue)
        });
        Ok(())
    }
}
