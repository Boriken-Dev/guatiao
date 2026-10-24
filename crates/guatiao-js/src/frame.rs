// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Entering a context while a call into the same runtime is on the stack.
//!
//! `Context::with` takes the runtime's lock, which a thread cannot take
//! twice; but a host the script called may call another provider of this
//! engine on this very thread. Such a call finds the lock held by the
//! frame it is nested in, and enters its context through the pointer.

use core::cell::Cell;
use core::ptr::NonNull;

use rquickjs::qjs::JSContext;
use rquickjs::{Context, Ctx};

/// A context, and how to enter it.
pub(crate) struct Entry {
    /// Keeps the context alive; `raw` points into it.
    context: Context,
    raw: NonNull<JSContext>,
}

impl Entry {
    /// An entry to `context`, whose pointer `raw` the caller read from a
    /// `Ctx` of it.
    pub(crate) fn new(context: Context, raw: NonNull<JSContext>) -> Entry {
        Entry { context, raw }
    }
}

/// The engine's frames: how many `enter` calls are on this thread's stack.
#[derive(Default)]
pub(crate) struct Frames {
    open: Cell<u32>,
}

impl Frames {
    /// Whether a frame holds the runtime's lock.
    pub(crate) fn inside(&self) -> bool {
        self.open.get() > 0
    }

    /// Runs `f` in `entry`'s context.
    pub(crate) fn enter<R>(&self, entry: &Entry, f: impl for<'js> FnOnce(Ctx<'js>) -> R) -> R {
        self.open.set(self.open.get() + 1);
        let _leave = Leave(&self.open);
        if self.open.get() == 1 {
            return entry.context.with(f);
        }
        // SAFETY: the frame below this one on this thread's stack is inside
        // `Context::with`, so the runtime is locked, and stays locked until
        // this call returns; `raw` points into `entry.context`, alive here.
        // That the runtime is locked is not that script may run: the engine
        // never calls here from a finalizer, which is the one place QuickJS
        // cannot run it (handles hand what they own to `Shared::bury`
        // instead of destroying it).
        let ctx = unsafe { Ctx::from_raw(entry.raw) };
        f(ctx)
    }
}

struct Leave<'a>(&'a Cell<u32>);

impl Drop for Leave<'_> {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}

/// Sets the memory limit of `ctx`'s runtime, 0 for none. `Runtime`'s own
/// setter takes the lock a frame already holds.
pub(crate) fn set_memory_limit(ctx: &Ctx<'_>, bytes: usize) {
    // SAFETY: `ctx` is a live context, so its runtime is too; the call
    // stores the limit and touches nothing else.
    unsafe {
        let runtime = rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr());
        rquickjs::qjs::JS_SetMemoryLimit(runtime, bytes as _);
    }
}
