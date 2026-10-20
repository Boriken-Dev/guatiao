// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What the generated glue promises, probed from the side a C caller
//! stands on: each test calls a generated shim or proxy the way a foreign
//! table or caller would, with input only C can produce, and checks the
//! promise holds.

#![cfg(feature = "provider")]
extern crate alloc;

use alloc::sync::Arc;
use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};

use guatiao::library::{BytesMut, Object, ObjectRaw, ProviderError, Remote};
use guatiao::value::alloc::{Alloc, Allocator, rust_alloc};
use guatiao::{Buffer, List, Map, Status, Str, Text, Value};

/// Counts this thread's heap allocations, so a test can say a call made
/// none: other tests run on other threads and are not counted.
///
/// Not installed under Miri: there, `System` runs the standard library's
/// own Windows allocator code, which reads an over-aligned block's header
/// outside the `Box` being freed (the test harness's channel), and Miri
/// refuses that. The count is a native measurement; Miri checks the same
/// calls for undefined behaviour.
#[cfg_attr(miri, allow(dead_code))]
struct PerThread;

std::thread_local! {
    static ALLOCATIONS: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

// SAFETY: every call is forwarded to the system allocator unchanged.
unsafe impl core::alloc::GlobalAlloc for PerThread {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|n| n.set(n.get() + 1));
        // SAFETY: forwarded.
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        // SAFETY: forwarded.
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
}

#[cfg(not(miri))]
#[global_allocator]
static HEAP: PerThread = PerThread;

/// How many allocations `f` made on this thread; `None` under Miri, where
/// nothing counts them.
fn allocations<R>(f: impl FnOnce() -> R) -> (R, Option<usize>) {
    let before = ALLOCATIONS.with(|n| n.get());
    let r = f();
    let made = ALLOCATIONS.with(|n| n.get()) - before;
    (r, (!cfg!(miri)).then_some(made))
}

/// `f` made no allocation on this thread (natively; Miri counts nothing).
fn assert_allocates_nothing<R>(f: impl FnOnce() -> R) -> R {
    let (r, made) = allocations(f);
    assert!(made.is_none_or(|n| n == 0), "{made:?} allocations");
    r
}

#[guatiao::kind]
pub trait Probe: Send + Sync {
    /// Text in, text out.
    fn name(&self, text: &str) -> Result<String, ProviderError>;
    /// An object, then a text: the object is owned before the text is read.
    fn keep(&self, sink: Object<dyn Sink>, text: &str) -> Result<(), ProviderError>;
    /// Infallible.
    fn quiet(&self) -> i64;
    /// A sink lent for the call, then a text: puts the text, answers how
    /// many bytes the sink filled.
    fn lend(&self, sink: &mut dyn Sink, text: &str) -> Result<i64, ProviderError>;
    /// A gauge lent shared: answers its level.
    fn peek(&self, gauge: &dyn Gauge) -> Result<i64, ProviderError>;
}

#[guatiao::kind(object)]
pub trait Gauge: Send {
    /// Read.
    fn level(&self) -> i64;
    /// Written: refused when lent as `&dyn Gauge`.
    fn bump(&mut self) -> Result<(), ProviderError>;
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
    fn lend(&self, sink: &mut dyn Sink, text: &str) -> Result<i64, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        sink.put(text);
        let mut buf = [0u8; 3];
        Ok(sink.fill(&mut buf))
    }
    fn peek(&self, gauge: &dyn Gauge) -> Result<i64, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(gauge.level())
    }
}

/// A sink that keeps what it was told in place, allocating nothing.
#[derive(Default)]
struct Recorder {
    puts: usize,
    last: [u8; 8],
    last_len: usize,
}

impl Sink for Recorder {
    fn put(&mut self, what: &str) {
        self.puts += 1;
        let n = what.len().min(self.last.len());
        self.last[..n].copy_from_slice(&what.as_bytes()[..n]);
        self.last_len = n;
    }
    fn fill(&mut self, dst: &mut [u8]) -> i64 {
        dst.fill(b'x');
        dst.len() as i64
    }
}

struct Level(i64);

impl Gauge for Level {
    fn level(&self) -> i64 {
        self.0
    }
    fn bump(&mut self) -> Result<(), ProviderError> {
        self.0 += 1;
        Ok(())
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
            BytesMut::from_raw_parts(core::ptr::null_mut(), 5),
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
    unsafe { real(core::ptr::null_mut(), size, align) }
}

unsafe extern "C" fn count_free(ctx: *mut c_void, p: *mut c_void, size: usize, align: usize) {
    // SAFETY: as above.
    unsafe { &*ctx.cast::<Count>() }
        .outstanding
        .fetch_sub(1, Ordering::SeqCst);
    let real = rust_alloc().free.expect("a free");
    // SAFETY: the block `count_alloc` handed out, with its layout.
    unsafe { real(core::ptr::null_mut(), p, size, align) }
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

// --- lent objects: `&mut dyn K` and `&dyn K` arguments --------------------

static COUNTED: ProbeVtable = ProbeVtable::of::<Counted>();

/// The generated proxy over a `Counted`, as a host holds one.
fn probe(counted: &Counted) -> Remote<dyn Probe> {
    // SAFETY: the table `of::<Counted>()` built, over a `&Counted` that
    // outlives every call the test makes.
    unsafe {
        Remote::<dyn Probe>::from_raw(
            (&COUNTED as *const ProbeVtable).cast(),
            size_of::<ProbeVtable>(),
            ctx(counted),
        )
    }
    .expect("the table validates")
}

#[test]
fn a_lent_object_crosses_and_comes_back_with_nothing_allocated() {
    let counted = Counted::default();
    let remote = probe(&counted);
    let mut recorder = Recorder::default();
    // The proxy lends the recorder through the lent table, the shim
    // validates it and hands the method a `&mut dyn Sink` whose calls come
    // back through the lent slots to `recorder` itself.
    let filled = assert_allocates_nothing(|| remote.lend(&mut recorder, "hey"));
    assert_eq!(filled.expect("the call succeeds"), 3);
    assert_eq!(recorder.puts, 1);
    assert_eq!(&recorder.last[..recorder.last_len], b"hey");
    // Still the caller's, and still usable: nothing was destroyed.
    recorder.put("again");
    assert_eq!(recorder.puts, 2);

    let level = Level(41);
    let seen = assert_allocates_nothing(|| remote.peek(&level));
    assert_eq!(seen.expect("the call succeeds"), 41);

    // The counter does count: a call answering a `String` allocates.
    let (_, allocated) = allocations(|| remote.name("abc"));
    assert!(
        allocated.is_none_or(|n| n > 0),
        "the counter sees an allocation"
    );
    assert_eq!(counted.calls.load(Ordering::SeqCst), 3);
}

/// A greeter whose `greet_into` tells the listener the name it was given.
struct Plain;

impl greeter_kind::Greeter for Plain {
    fn greet(&self, _: &str) -> Result<guatiao::Map, ProviderError> {
        Ok(Map::new())
    }
    fn greet_into(
        &self,
        name: &str,
        listener: &mut dyn greeter_kind::Listener,
    ) -> Result<(), ProviderError> {
        listener.heard(name);
        Ok(())
    }
}

/// A listener that counts what it heard, allocating nothing.
#[derive(Default)]
struct Ear {
    heard: usize,
    bytes: usize,
}

impl greeter_kind::Listener for Ear {
    fn heard(&mut self, what: &str) {
        self.heard += 1;
        self.bytes += what.len();
    }
}

static PLAIN: greeter_kind::GreeterVtable = greeter_kind::GreeterVtable::of::<Plain>();

#[test]
fn greet_into_lends_its_listener_rust_to_rust() {
    use greeter_kind::Greeter;

    // SAFETY: the table `of::<Plain>()` built; `Plain` is a ZST read only
    // through `&Plain`, so any aligned non-null address serves as its
    // `ctx`.
    let remote = unsafe {
        Remote::<dyn Greeter>::from_raw(
            (&PLAIN as *const greeter_kind::GreeterVtable).cast(),
            size_of::<greeter_kind::GreeterVtable>(),
            core::ptr::NonNull::<Plain>::dangling().as_ptr().cast(),
        )
    }
    .expect("the table validates");
    let mut ear = Ear::default();
    assert_allocates_nothing(|| remote.greet_into("ana", &mut ear)).expect("the call succeeds");
    assert_eq!((ear.heard, ear.bytes), (1, 3));
}

#[test]
fn a_lent_table_frees_nothing_and_a_shared_one_refuses_mut_methods() {
    let mut level = Level(1);
    // `destroy` on either lent table does nothing: the object is the
    // lender's, even if a callee breaks the rule and calls it.
    let slot: *mut dyn Gauge = &mut level;
    let ctx = core::ptr::addr_of!(slot).cast_mut().cast::<c_void>();
    let destroy = GaugeVtable::lent_mut().destroy.expect("a slot");
    // SAFETY: a no-op on any pointer.
    unsafe { destroy(ctx) };
    let destroy = GaugeVtable::lent().destroy.expect("a slot");
    // SAFETY: as above.
    unsafe { destroy(ctx) };
    assert_eq!(level.level(), 1, "the lender's object is untouched");

    // Lent shared, a `&mut self` slot answers UNSUPPORTED and leaves the
    // object alone; a `&self` slot answers.
    let shared: *const dyn Gauge = &level;
    let ctx = core::ptr::addr_of!(shared).cast_mut().cast::<c_void>();
    let table = GaugeVtable::lent();
    let mut err = ProviderError::none();
    // SAFETY: the lent table's slot, with the address of a `*const dyn
    // Gauge` as `ctx` and a writable error slot.
    let status = unsafe { (table.bump.expect("a slot"))(ctx, &mut err) };
    assert_eq!(status, Status::GUATIAO_ERR_UNSUPPORTED);
    assert_eq!(err.status, Status::GUATIAO_ERR_UNSUPPORTED);
    assert!(err.message().contains("bump"), "{}", err.message());
    let mut out = 0i64;
    // SAFETY: as above.
    let status = unsafe { (table.level.expect("a slot"))(ctx, &mut out) };
    assert_eq!(status, Status::GUATIAO_OK);
    assert_eq!(out, 1);
    assert_eq!(level.level(), 1);
}

#[test]
fn a_callee_lent_an_owned_object_never_destroys_it() {
    // What a C caller lends is its own table, `destroy` and all: the shim
    // takes it as a borrow and leaves it to its owner.
    let counted = Counted::default();
    let (sink, drops, asked) = tracked();
    let raw = sink.into_raw();
    let mut err = ProviderError::none();
    let mut out = 0i64;
    // SAFETY: `ctx` is a `&Counted`; `raw` is an object of this kind,
    // lent for the call; the out-slots are writable.
    let status = unsafe {
        ProbeVtable::__guatiao_shim_lend::<Counted>(
            ctx(&counted),
            &raw,
            Str::new("lent"),
            &mut out,
            &mut err,
        )
    };
    assert_eq!(status, Status::GUATIAO_OK);
    assert_eq!(out, 3);
    assert_eq!(asked.load(Ordering::SeqCst), 3);
    assert_eq!(
        drops.load(Ordering::SeqCst),
        0,
        "a lent object is not destroyed"
    );
    // SAFETY: `raw` came from `into_raw` and is still owned here.
    drop(unsafe { Object::<dyn Sink>::from_raw(raw) }.expect("the same table"));
    assert_eq!(
        drops.load(Ordering::SeqCst),
        1,
        "its owner destroys it, once"
    );
}

#[test]
fn a_null_or_foreign_lent_object_is_refused_before_the_method_runs() {
    let counted = Counted::default();
    let mut err = ProviderError::none();
    let mut out = 0i64;
    // SAFETY: as above, with a null lent object.
    let status = unsafe {
        ProbeVtable::__guatiao_shim_lend::<Counted>(
            ctx(&counted),
            core::ptr::null(),
            Str::new("x"),
            &mut out,
            &mut err,
        )
    };
    assert_eq!(status, Status::GUATIAO_ERR_NULL);
    // A gauge's table lent where a sink is expected.
    let mut level = Level(0);
    let slot: *mut dyn Gauge = &mut level;
    let wrong =
        guatiao::library::kind::lend_mut::<dyn Gauge>(core::ptr::addr_of!(slot).cast_mut().cast());
    let mut err = ProviderError::none();
    // SAFETY: as above; the table is readable for its size.
    let status = unsafe {
        ProbeVtable::__guatiao_shim_lend::<Counted>(
            ctx(&counted),
            &wrong,
            Str::new("x"),
            &mut out,
            &mut err,
        )
    };
    assert_eq!(status, Status::GUATIAO_ERR_WRONG_KIND);
    assert_eq!(
        counted.calls.load(Ordering::SeqCst),
        0,
        "the method never ran"
    );
}

/// An object kind whose trait names `Sync`: shared, its `&self` methods
/// may run on two threads at once.
#[guatiao::kind(object)]
pub trait Meter: Send + Sync {
    /// Read.
    fn read(&self) -> i64;
    /// Written.
    fn add(&mut self, n: i64);
}

#[guatiao::kind]
pub trait Reader: Send + Sync {
    /// Reads the meter it was lent from two threads at once.
    fn twice(&self, meter: &mut dyn Meter) -> Result<i64, ProviderError>;
}

/// A meter whose `read` waits for the other reader, so the two calls
/// overlap: under Miri, a shim taking `&mut` for a `&self` method is a
/// data race.
struct Gated {
    value: i64,
    gate: Arc<std::sync::Barrier>,
}

impl Meter for Gated {
    fn read(&self) -> i64 {
        self.gate.wait();
        self.value
    }
    fn add(&mut self, n: i64) {
        self.value += n;
    }
}

fn read_on_two_threads(meter: &dyn Meter) -> i64 {
    std::thread::scope(|s| {
        let a = s.spawn(|| meter.read());
        let b = s.spawn(|| meter.read());
        a.join().expect("a reader") + b.join().expect("a reader")
    })
}

struct Twice;

impl Reader for Twice {
    fn twice(&self, meter: &mut dyn Meter) -> Result<i64, ProviderError> {
        Ok(read_on_two_threads(meter))
    }
}

static TWICE: ReaderVtable = ReaderVtable::of::<Twice>();

#[test]
fn a_sync_object_is_read_from_two_threads_at_once_owned_or_lent() {
    // The handle of a `Sync` kind is `Sync`; of a kind naming `Send`
    // alone it is not (a `compile_fail` example on `Object`).
    fn shared<T: Sync>(_: &T) {}
    let gate = Arc::new(std::sync::Barrier::new(2));
    // Owned: the handle derefs to `&dyn Meter`, which is `Sync`.
    let mut meter = Gated {
        value: 20,
        gate: gate.clone(),
    }
    .into_object();
    shared(&meter);
    assert_eq!(read_on_two_threads(&*meter), 40);
    meter.add(1);
    assert_eq!(read_on_two_threads(&*meter), 42);

    // Lent `&mut` through a proxy: the callee shares what it was lent.
    // SAFETY: the table `of::<Twice>()` built; `Twice` is a ZST read only
    // through `&Twice`, so any aligned non-null address serves as `ctx`.
    let reader = unsafe {
        Remote::<dyn Reader>::from_raw(
            (&TWICE as *const ReaderVtable).cast(),
            size_of::<ReaderVtable>(),
            core::ptr::NonNull::<Twice>::dangling().as_ptr().cast(),
        )
    }
    .expect("the table validates");
    let mut lent = Gated { value: 21, gate };
    assert_eq!(reader.twice(&mut lent).expect("the call succeeds"), 42);
}

// --- trees handed over by value ---------------------------------------------

/// Takes trees by value and keeps the last map it was handed.
#[guatiao::kind]
pub trait Keeper: Send + Sync {
    /// A map handed over: answers how many entries it holds.
    fn keep(&self, map: Map) -> Result<i64, ProviderError>;
    /// A value and a list handed over, then a text: the trees are owned
    /// before the text is read.
    fn hold(&self, value: Value, list: List, text: &str) -> Result<(), ProviderError>;
}

/// Keeps what it is handed, so a test can compare it with what was sent.
struct Kept {
    map: std::sync::Mutex<Option<Map>>,
    calls: AtomicUsize,
}

impl Keeper for Kept {
    fn keep(&self, map: Map) -> Result<i64, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let n = map.len() as i64;
        *self.map.lock().expect("not poisoned") = Some(map);
        Ok(n)
    }
    fn hold(&self, _: Value, _: List, _: &str) -> Result<(), ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

static KEEPER: KeeperVtable = KeeperVtable::of::<Kept>();

fn keeper(kept: &Kept) -> Remote<dyn Keeper> {
    // SAFETY: the table `of::<Kept>()` built, over a `&Kept` that outlives
    // every call the test makes.
    unsafe {
        Remote::<dyn Keeper>::from_raw(
            (&KEEPER as *const KeeperVtable).cast(),
            size_of::<KeeperVtable>(),
            (kept as *const Kept).cast_mut().cast(),
        )
    }
    .expect("the table validates")
}

#[test]
fn a_map_by_value_crosses_as_a_move_with_nothing_copied() {
    let kept = Kept {
        map: std::sync::Mutex::new(None),
        calls: AtomicUsize::new(0),
    };
    let remote = keeper(&kept);
    let mut map = Map::new();
    map.set("host", "10.0.0.1");
    map.set("port", 5900);
    let sent = map.entries().as_ptr();
    // `std::sync::Mutex` allocates its lock on first use on some platforms
    // (macOS), so the provider's mutex is taken once outside the count.
    drop(kept.map.lock().expect("not poisoned"));
    let n = assert_allocates_nothing(|| remote.keep(map));
    assert_eq!(n.expect("kept"), 2);
    let held = kept.map.lock().expect("not poisoned").take().expect("kept");
    assert_eq!(
        held.entries().as_ptr(),
        sent,
        "the very storage, not a copy"
    );
    assert!(held.is_local());
}

#[test]
fn a_tree_handed_to_a_refusing_slot_is_freed_by_it() {
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
    let kept = Kept {
        map: std::sync::Mutex::new(None),
        calls: AtomicUsize::new(0),
    };
    let tree = |alloc| {
        let mut map = Map::new_in(alloc);
        map.set_in("k", Text::new_in(alloc, "v").unwrap(), alloc)
            .unwrap();
        map
    };

    // No provider behind the slot: refused, and the map is still freed.
    let mut map = core::mem::ManuallyDrop::new(tree(alloc));
    let mut err = ProviderError::none();
    let mut out = 0i64;
    // SAFETY: a null ctx, a map handed over, writable out slots.
    let status = unsafe {
        KeeperVtable::__guatiao_shim_keep::<Kept>(
            core::ptr::null_mut(),
            &mut *map,
            &mut out,
            &mut err,
        )
    };
    assert_eq!(status, Status::GUATIAO_ERR_NULL);
    assert_eq!(
        count.outstanding.load(Ordering::SeqCst),
        0,
        "freed by the slot"
    );

    // A text refused after two trees: both are freed, the method never ran.
    let mut value = core::mem::ManuallyDrop::new(Value::from(tree(alloc)));
    let mut list = core::mem::ManuallyDrop::new({
        let mut list = List::new_in(alloc);
        list.push_in(tree(alloc), alloc).unwrap();
        list
    });
    let bytes = [0xfe];
    let mut err = ProviderError::none();
    // SAFETY: the `&Kept` the table was built for, trees handed over.
    let status = unsafe {
        KeeperVtable::__guatiao_shim_hold::<Kept>(
            (&kept as *const Kept).cast_mut().cast(),
            &mut *value,
            &mut *list,
            not_utf8(&bytes),
            &mut err,
        )
    };
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(kept.calls.load(Ordering::SeqCst), 0, "the method never ran");
    assert_eq!(count.outstanding.load(Ordering::SeqCst), 0, "both freed");

    // A null tree is refused and owns nothing.
    let mut err = ProviderError::none();
    // SAFETY: as above, the map pointer null.
    let status = unsafe {
        KeeperVtable::__guatiao_shim_keep::<Kept>(
            (&kept as *const Kept).cast_mut().cast(),
            core::ptr::null_mut(),
            &mut out,
            &mut err,
        )
    };
    assert_eq!(status, Status::GUATIAO_ERR_NULL);
}
