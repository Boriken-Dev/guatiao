// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Windows: ending, from another thread, a read blocked on a socket or a
//! named pipe. A socket's `shutdown` does not wake a `recv` already blocked
//! on it there, and a pipe offers nothing at all; `CancelIoEx` ends both.
//! An operation started after the cancel is not ended by it.

use std::os::windows::io::{AsRawHandle, BorrowedHandle};

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::IO::CancelIoEx;

/// Ends the I/O in flight on `handle`, whichever thread started it; each
/// fails with `ERROR_OPERATION_ABORTED`. None in flight is no error.
pub(crate) fn cancel_io(handle: BorrowedHandle<'_>) {
    cancel(handle.as_raw_handle() as HANDLE);
}

fn cancel(handle: HANDLE) {
    // SAFETY: `handle` is open for as long as the borrow it came from;
    // CancelIoEx asks the system to complete the I/O on it early, and the
    // completed operations' buffers stay with the code that started them.
    unsafe {
        CancelIoEx(handle, core::ptr::null());
    }
}
