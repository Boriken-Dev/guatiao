// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Telling CPython which stack a call runs on.
//!
//! CPython 3.14 judges how deep its C stack is by address, against the
//! bounds of the thread's own stack. A host may run its calls into native
//! code on a stack of its own (koffi, Node's FFI, maps one per thread), and
//! there, below the thread's stack, every deallocation looks like an
//! imminent overflow and is put off for good: objects a call drops are never
//! freed. Before each call into Python, the thread state's stack protection
//! is pointed at the mapping the call runs on, and put back once a call runs
//! on the thread's own stack again. Linux only: the mapping is read from
//! `/proc/self/maps`. Before 3.14 there is nothing to do, and the functions
//! are not there.

#[cfg(target_os = "linux")]
mod linux {
    use core::cell::Cell;
    use core::ffi::{c_int, c_void};
    use std::sync::OnceLock;

    use pyo3::Python;
    use pyo3::ffi::{PyErr_Clear, PyThreadState, PyThreadState_Get};

    type SetProtection = unsafe extern "C" fn(*mut PyThreadState, *mut c_void, usize) -> c_int;
    type ResetProtection = unsafe extern "C" fn(*mut PyThreadState);

    /// The two functions, when this Python has them (3.14 and later).
    fn functions() -> Option<(SetProtection, ResetProtection)> {
        static FOUND: OnceLock<Option<(SetProtection, ResetProtection)>> = OnceLock::new();
        *FOUND.get_or_init(|| {
            // SAFETY: looks a name up in what the process has loaded;
            // libpython's symbols are global (`exports`' `share_python_symbols`).
            let (set, reset) = unsafe {
                (
                    libc::dlsym(
                        libc::RTLD_DEFAULT,
                        c"PyUnstable_ThreadState_SetStackProtection".as_ptr(),
                    ),
                    libc::dlsym(
                        libc::RTLD_DEFAULT,
                        c"PyUnstable_ThreadState_ResetStackProtection".as_ptr(),
                    ),
                )
            };
            if set.is_null() || reset.is_null() {
                return None;
            }
            // SAFETY: CPython's own functions, with these signatures
            // (`cpython/pystate.h`, 3.14).
            Some(unsafe {
                (
                    core::mem::transmute::<*mut c_void, SetProtection>(set),
                    core::mem::transmute::<*mut c_void, ResetProtection>(reset),
                )
            })
        })
    }

    thread_local! {
        /// The mapping this thread's state was last pointed at, if any.
        static FITTED: Cell<Option<(usize, usize)>> = const { Cell::new(None) };
    }

    /// The bounds of the calling thread's own stack.
    fn thread_stack() -> Option<(usize, usize)> {
        let mut attr = core::mem::MaybeUninit::<libc::pthread_attr_t>::uninit();
        // SAFETY: fills `attr` for this thread, destroyed below.
        if unsafe { libc::pthread_getattr_np(libc::pthread_self(), attr.as_mut_ptr()) } != 0 {
            return None;
        }
        let mut base: *mut c_void = core::ptr::null_mut();
        let mut size = 0usize;
        // SAFETY: `attr` was initialised above.
        let got = unsafe { libc::pthread_attr_getstack(attr.as_ptr(), &mut base, &mut size) };
        // SAFETY: as above, once.
        unsafe { libc::pthread_attr_destroy(attr.as_mut_ptr()) };
        (got == 0).then(|| (base as usize, base as usize + size))
    }

    /// The mapping in `/proc/self/maps` holding `address`.
    fn mapping_of(address: usize) -> Option<(usize, usize)> {
        let maps = std::fs::read_to_string("/proc/self/maps").ok()?;
        maps.lines().find_map(|line| {
            let range = line.split_whitespace().next()?;
            let (lo, hi) = range.split_once('-')?;
            let lo = usize::from_str_radix(lo, 16).ok()?;
            let hi = usize::from_str_radix(hi, 16).ok()?;
            (lo <= address && address < hi).then_some((lo, hi))
        })
    }

    pub(crate) fn fit(_py: Python<'_>) {
        let Some((set, reset)) = functions() else {
            return;
        };
        let marker = 0u8;
        let here = core::ptr::addr_of!(marker) as usize;
        let fitted = FITTED.get();
        if fitted.is_some_and(|(lo, hi)| lo <= here && here < hi) {
            return;
        }
        // SAFETY: the GIL is held (`_py`), so this thread has a state.
        let state = unsafe { PyThreadState_Get() };
        if thread_stack().is_some_and(|(lo, hi)| lo <= here && here < hi) {
            if fitted.is_some() {
                // SAFETY: this thread's state, the GIL held.
                unsafe { reset(state) };
                FITTED.set(None);
            }
            return;
        }
        let Some((lo, hi)) = mapping_of(here) else {
            return;
        };
        // SAFETY: this thread's state, the GIL held; the mapping holds the
        // stack this call runs on, whole.
        if unsafe { set(state, lo as *mut c_void, hi - lo) } == 0 {
            FITTED.set(Some((lo, hi)));
        } else {
            // SAFETY: the GIL is held; what it raised is not this call's.
            unsafe { PyErr_Clear() };
        }
    }
}

#[cfg(target_os = "linux")]
pub(crate) use linux::fit;

/// Nothing to do where the mapping cannot be read.
#[cfg(not(target_os = "linux"))]
pub(crate) fn fit(_py: pyo3::Python<'_>) {}
