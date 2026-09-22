// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What the generated glue promises, probed from the side a C caller
//! stands on: each test calls a generated shim or proxy the way a foreign
//! table or caller would, with input only C can produce, and checks the
//! promise holds.

#![cfg(feature = "provider")]

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use guatiao::library::{BytesMut, Object, ObjectRaw, ProviderError, Remote};
use guatiao::value::alloc::{Alloc, Allocator, rust_alloc};
use guatiao::{Buffer, Status, Str, Text};

#[guatiao::kind]
pub trait Probe: Send + Sync {
    /// Text in, text out.
    fn name(&self, text: &str) -> Result<String, ProviderError>;
    /// An object, then a text: the object is owned before the text is read.
    fn keep(&self, sink: Object<dyn Sink>, text: &str) -> Result<(), ProviderError>;
    /// Infallible.
    fn quiet(&self) -> i64;
}

#[guatiao::kind(object)]
pub trait Sink: Send {
    /// Something to keep.
    fn put(&mut self, what: &str);
    /// Fills `dst`, answering how many bytes it wrote.
    fn fill(&mut self, dst: &mut [u8]) -> i64;
}

/// Counts every call it takes.
#[derive(Default)]
struct Counted {
    calls: AtomicUsize,
}

impl Probe for Counted {
    fn name(&self, text: &str) -> Result<String, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(text.to_string())
    }
    fn keep(&self, sink: Object<dyn Sink>, _: &str) -> Result<(), ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        drop(sink);
        Ok(())
    }
    fn quiet(&self) -> i64 {
        self.calls.fetch_add(1, Ordering::SeqCst);
        7
    }
}

/// Counts how often it is destroyed, and how many bytes it was asked to fill.
struct Tracked {
    drops: Arc<AtomicUsize>,
    asked: Arc<AtomicUsize>,
}

impl Drop for Tracked {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl Sink for Tracked {
    fn put(&mut self, _: &str) {}
    fn fill(&mut self, dst: &mut [u8]) -> i64 {
        self.asked.store(dst.len(), Ordering::SeqCst);
        dst.len() as i64
    }
}

fn tracked() -> (Object<dyn Sink>, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let drops = Arc::new(AtomicUsize::new(0));
    let asked = Arc::new(AtomicUsize::new(usize::MAX));
    let sink = Tracked {
        drops: drops.clone(),
        asked: asked.clone(),
    }
    .into_object();
    (sink, drops, asked)
}

/// A view over bytes that are not UTF-8: what only a C caller can pass.
fn not_utf8(bytes: &[u8]) -> Str<'_> {
    // SAFETY: the bytes are readable for the view's lifetime. They are
    // not UTF-8, which breaks the constructor's promise on purpose: this
    // is the view a C caller hands a shim.
    unsafe { Str::from_raw_parts(bytes.as_ptr(), bytes.len()) }
}

fn ctx(probe: &Counted) -> *mut c_void {
    (probe as *const Counted).cast_mut().cast()
}

#[test]
fn text_a_c_caller_passes_that_is_not_utf8_never_reaches_the_method() {
    let probe = Counted::default();
    let bytes = [b'h', 0xff];
    let mut out = Text::new("");
    let mut err = ProviderError::none();
    // SAFETY: `ctx` is a `&Counted`, the out-slots are writable.
    let status = unsafe {
        ProbeVtable::__guatiao_shim_name::<Counted>(
            ctx(&probe),
            not_utf8(&bytes),
            &mut out,
            &mut err,
        )
    };
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(
        err.status,
        Status::GUATIAO_ERR_BAD_VALUE,
        "the error slot says so too"
    );
    assert_eq!(
        probe.calls.load(Ordering::SeqCst),
        0,
        "the method never ran"
    );
}

#[test]
fn an_object_argument_is_destroyed_once_when_a_later_argument_is_refused() {
    let probe = Counted::default();
    let (sink, drops, _) = tracked();
    let bytes = [0xfe];
    let mut err = ProviderError::none();
    // SAFETY: as above; the object crosses with its ownership.
    let status = unsafe {
        ProbeVtable::__guatiao_shim_keep::<Counted>(
            ctx(&probe),
            sink.into_raw(),
            not_utf8(&bytes),
            &mut err,
        )
    };
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(
        probe.calls.load(Ordering::SeqCst),
        0,
        "the method never ran"
    );
    assert_eq!(
        drops.load(Ordering::SeqCst),
        1,
        "owned from the call, destroyed once"
    );
}

#[test]
fn an_out_buffer_with_a_null_pointer_and_a_length_is_empty() {
    let (sink, drops, asked) = tracked();
    let raw: ObjectRaw = sink.into_raw();
    // SAFETY: the table `into_raw` handed out is a `SinkVtable`.
    let table = unsafe { &*raw.table.cast::<SinkVtable>() };
    let fill = table.fill.expect("a full table");
    let mut out = 0i64;
    // SAFETY: the object's own table and ctx; the view is null with a
    // length, a shape a C caller can pass, and it must read as empty.
    let status = unsafe {
        fill(
            raw.ctx,
            BytesMut::from_raw_parts(std::ptr::null_mut(), 5),
            &mut out,
        )
    };
    assert_eq!(status, Status::GUATIAO_OK);
    assert_eq!(out, 0);
    assert_eq!(
        asked.load(Ordering::SeqCst),
        0,
        "the method saw an empty slice"
    );
    // SAFETY: `raw` came from `into_raw` for this kind.
    drop(unsafe { Object::<dyn Sink>::from_raw(raw) }.expect("the same table"));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

unsafe extern "C" fn quiet_fails(_: *mut c_void, _: *mut i64) -> Status {
    Status::GUATIAO_ERR_BAD_VALUE
}

/// A full table whose infallible slot fails.
static FAILING_QUIET: ProbeVtable = ProbeVtable {
    quiet: Some(quiet_fails),
    ..ProbeVtable::of::<Counted>()
};

static PROBE: Counted = Counted {
    calls: AtomicUsize::new(0),
};

#[test]
fn an_infallible_method_whose_provider_fails_panics_in_the_host() {
    let table: &'static ProbeVtable = &FAILING_QUIET;
    // SAFETY: a table `of` built, one slot replaced by a function of the
    // same signature, with the `&Counted` it was built for.
    let remote = unsafe {
        Remote::<dyn Probe>::from_raw(
            (table as *const ProbeVtable).cast(),
            size_of::<ProbeVtable>(),
            ctx(&PROBE),
        )
    }
    .expect("the table validates");
    let caught = std::panic::catch_unwind(|| remote.quiet());
    assert!(caught.is_err(), "no answer to give, so the proxy panics");
}

// --- a counting allocator, so a free is observed rather than assumed -----

struct Count {
    outstanding: AtomicUsize,
}

unsafe extern "C" fn count_alloc(ctx: *mut c_void, size: usize, align: usize) -> *mut c_void {
    // SAFETY: `ctx` is the `Count` below, alive for the test.
    unsafe { &*ctx.cast::<Count>() }
        .outstanding
        .fetch_add(1, Ordering::SeqCst);
    let real = rust_alloc().alloc.expect("an alloc");
    // SAFETY: forwarded unchanged.
    unsafe { real(std::ptr::null_mut(), size, align) }
}

unsafe extern "C" fn count_free(ctx: *mut c_void, p: *mut c_void, size: usize, align: usize) {
    // SAFETY: as above.
    unsafe { &*ctx.cast::<Count>() }
        .outstanding
        .fetch_sub(1, Ordering::SeqCst);
    let real = rust_alloc().free.expect("a free");
    // SAFETY: the block `count_alloc` handed out, with its layout.
    unsafe { real(std::ptr::null_mut(), p, size, align) }
}

#[test]
fn a_text_a_shim_wrote_that_is_not_utf8_is_refused_and_freed() {
    use guatiao::library::kind::text_ret;

    let count = Count {
        outstanding: AtomicUsize::new(0),
    };
    let vtable = Allocator {
        struct_size: size_of::<Allocator>() as u32,
        ctx: (&count as *const Count).cast_mut().cast(),
        alloc: Some(count_alloc),
        free: Some(count_free),
        release: None,
    };
    // SAFETY: a complete vtable that outlives every block it hands out.
    let alloc = unsafe { Alloc::from_raw(&vtable) }.expect("a complete vtable");
    let (ptr, len, cap, a) = Buffer::new_in(alloc, &[b'o', 0xff])
        .unwrap()
        .into_raw_parts();
    assert_eq!(count.outstanding.load(Ordering::SeqCst), 1);
    // SAFETY: consistent storage; its bytes are what a shim wrote.
    let written = unsafe { Text::from_raw_parts(ptr, len, cap, a) };
    let refused = text_ret(written).expect_err("not UTF-8");
    assert_eq!(refused.status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(
        count.outstanding.load(Ordering::SeqCst),
        0,
        "the refused text was freed"
    );
}
