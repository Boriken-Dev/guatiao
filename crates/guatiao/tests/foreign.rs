// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The foreign loader from a host's side: what it refuses, what runs on its
//! workers, that the host's own thread state is what it was, and that the
//! registry loads a guatiao library through it with nothing in the host
//! naming it.
//!
//! The point is a statically linked host: run these with a musl target
//! (`cargo test -p guatiao --features load,provider --target
//! x86_64-unknown-linux-musl --test foreign`, or aarch64). They pass in a
//! glibc host as well, the registry told to load through the foreign
//! loader. `c_consumer/thread_state.c` is built with `CC` (default `cc`).
//! `GUATIAO_LIBRARY_DIR` names a directory of guatiao libraries built for
//! the system C library (`cargo build -p derived_greeter` on the host's
//! architecture); without it the registry test says it skipped, and fails
//! under `GUATIAO_REQUIRE_ALL=1`.

#![cfg(all(guatiao_foreign, feature = "provider"))]

extern crate alloc;

use core::cell::Cell;
use core::ffi::{c_int, c_long, c_void};
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use guatiao::library::foreign::{
    Error, Foreign, ImageError, Library, OpenFlags, Settings, SignalSet,
};
use guatiao::library::{Declared, Dlopen, Registry};
use guatiao::value::convert::TryAsRef;
use guatiao::{Map, Status};

use greeter_kind::{Conversation, Counter, Greeter, Listener, Wide};

unsafe extern "C" {
    fn syscall(number: c_long, ...) -> c_long;
    fn close(fd: c_int) -> c_int;
    fn fork() -> c_int;
    fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int;
    fn _exit(code: c_int) -> !;
}

#[cfg(target_arch = "x86_64")]
const SYS_BRK: c_long = 12;
#[cfg(target_arch = "aarch64")]
const SYS_BRK: c_long = 214;

const WORKERS: usize = 3;

fn program_break() -> usize {
    // SAFETY: brk(0) only answers.
    unsafe { syscall(SYS_BRK, 0usize) as usize }
}

fn thread_pointer() -> usize {
    let tp: usize;
    // SAFETY: reads this thread's pointer register (x86_64: the C library's
    // self pointer at fs:0).
    unsafe {
        #[cfg(target_arch = "x86_64")]
        core::arch::asm!("mov {}, fs:0", out(reg) tp, options(nostack, readonly, preserves_flags));
        #[cfg(target_arch = "aarch64")]
        core::arch::asm!("mrs {}, tpidr_el0", out(reg) tp, options(nomem, nostack, preserves_flags));
    }
    tp
}

/// A field of `/proc/thread-self/status` or of another thread's, as hex.
fn status_hex(path: &str, field: &str) -> u64 {
    let text = std::fs::read_to_string(path).expect("procfs is mounted");
    let line = text
        .lines()
        .find_map(|l| l.strip_prefix(field))
        .unwrap_or_else(|| panic!("{field} in {path}"));
    u64::from_str_radix(line.trim(), 16).expect("a hex mask")
}

fn compiler() -> std::ffi::OsString {
    std::env::var_os("CC").unwrap_or_else(|| "cc".into())
}

fn scratch() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("guatiao-foreign");
    std::fs::create_dir_all(&dir).expect("a temporary directory");
    dir
}

/// `c_consumer/thread_state.c`, built once per test binary.
fn fixture() -> &'static PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT.get_or_init(|| {
        let out = scratch().join(format!("libthread_state-{}.so", std::process::id()));
        let source =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/c_consumer/thread_state.c");
        let output = Command::new(compiler())
            .args(["-shared", "-fPIC", "-O2", "-o"])
            .arg(&out)
            .arg(&source)
            .output()
            .expect("the C compiler runs");
        assert!(
            output.status.success(),
            "the fixture builds: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        out
    })
}

fn settings() -> Settings {
    let mut s = Settings::default();
    s.workers = WORKERS;
    // A glibc host's own C library already handles signal 33.
    s.allow_reserved_signal_handlers = cfg!(target_env = "gnu");
    s
}

/// The process's one loader.
fn shared() -> Foreign {
    Foreign::start(&settings()).expect("the loader starts")
}

#[repr(C)]
#[derive(Default, Debug)]
struct Probe {
    tid: i64,
    errno_seen: i64,
    calls: i64,
    allocated: i64,
    size: u64,
    rounds: u64,
}

fn probe(library: &Library, size: u64, rounds: u64) -> Probe {
    let job = library
        .symbol("fixture_probe")
        .expect("looked up")
        .expect("the fixture exports it");
    let mut probe = Probe {
        size,
        rounds,
        ..Probe::default()
    };
    // SAFETY: `fixture_probe` takes a `struct probe *`, which `Probe`
    // mirrors, alive across the call.
    unsafe {
        library
            .foreign()
            .call(job.as_job(), (&mut probe as *mut Probe).cast::<c_void>())
    }
    .expect("the job runs");
    probe
}

/// The directory of guatiao libraries built for the system C library, or
/// `None` when the run did not name one.
fn library_dir() -> Option<PathBuf> {
    match std::env::var_os("GUATIAO_LIBRARY_DIR") {
        Some(dir) => Some(PathBuf::from(dir)),
        None if std::env::var_os("GUATIAO_REQUIRE_ALL").is_some() => {
            panic!("GUATIAO_LIBRARY_DIR names no directory of libraries built for glibc")
        }
        None => {
            eprintln!(
                "skipped: GUATIAO_LIBRARY_DIR names no directory of libraries built for glibc"
            );
            None
        }
    }
}

/// Whether to run `name`'s body here: in a child process of this binary
/// running that test alone, so the loader is not started yet. In the
/// parent, runs the child and checks it passed.
fn in_a_fresh_process(name: &str) -> bool {
    if std::env::var("GUATIAO_FOREIGN_FRESH").as_deref() == Ok(name) {
        assert!(Foreign::get().is_none(), "the child starts nothing first");
        return true;
    }
    let output = Command::new(std::env::current_exe().expect("the test binary"))
        .args(["--exact", name, "--test-threads=1", "--nocapture"])
        .env("GUATIAO_FOREIGN_FRESH", name)
        .output()
        .expect("the child runs");
    assert!(
        output.status.success() && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
        "{name} in a fresh process: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    false
}

#[test]
fn a_missing_dynamic_linker_is_a_clear_error() {
    if !in_a_fresh_process("a_missing_dynamic_linker_is_a_clear_error") {
        return;
    }
    let mut s = settings();
    s.ld_so = Some("/nonexistent/ld-linux.so".into());
    let e = Foreign::start(&s).expect_err("there is no such file");
    assert!(
        matches!(&e, Error::LdSo { path, reason: ImageError::Io(io) }
            if path.to_str() == Some("/nonexistent/ld-linux.so")
                && io.kind() == std::io::ErrorKind::NotFound),
        "{e:?}"
    );
    assert!(e.to_string().contains("/nonexistent/ld-linux.so"), "{e}");
}

#[test]
fn files_that_are_not_a_dynamic_linker_are_refused_before_anything_runs() {
    if !in_a_fresh_process("files_that_are_not_a_dynamic_linker_are_refused_before_anything_runs") {
        return;
    }
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/c_consumer/thread_state.c");
    let mut s = settings();
    s.ld_so = Some(source);
    assert!(matches!(
        Foreign::start(&s),
        Err(Error::LdSo {
            reason: ImageError::NotElf,
            ..
        })
    ));
    // A program where the dynamic linker should be: the program the loader
    // writes names an interpreter.
    let program = scratch().join(format!("not-a-linker-{}", std::process::id()));
    std::fs::write(&program, Settings::default().program()).unwrap();
    let mut s = settings();
    s.ld_so = Some(program.clone());
    let refused = Foreign::start(&s).expect_err("not a dynamic linker");
    let _ = std::fs::remove_file(&program);
    assert!(
        matches!(
            refused,
            Error::LdSo {
                reason: ImageError::HasInterpreter,
                ..
            }
        ),
        "{refused:?}"
    );
}

#[test]
fn the_program_needs_no_newer_c_library_than_the_oldest_of_its_architecture() {
    let needs = Settings::default().program_needs().expect("reads back");
    eprintln!("the program needs: {needs:?}");
    assert!(!needs.is_empty());
    let floor: &[&str] = if cfg!(target_arch = "x86_64") {
        &["GLIBC_2.2.5", "GLIBC_2.3.3"]
    } else {
        &["GLIBC_2.17"]
    };
    for need in &needs {
        assert_eq!(need.file, "libc.so.6", "{need:?}");
        assert!(floor.contains(&need.version.as_str()), "{need:?}");
    }
}

#[test]
fn a_library_runs_on_foreign_workers_and_the_host_keeps_its_thread_state() {
    thread_local!(static MINE: Cell<u64> = const { Cell::new(7) });
    let tp = thread_pointer();
    let mask = status_hex("/proc/thread-self/status", "SigBlk:");
    MINE.with(|m| m.set(41));

    let foreign = shared();
    assert!(!foreign.on_foreign_thread(), "a host thread is the host's");
    let library = foreign.open(fixture()).expect("the fixture opens");

    // The library ran on the workers: its errno, allocator and thread-local
    // are the foreign C library's.
    for _ in 0..20 {
        let p = probe(&library, 4096, 100);
        // Read after the call: other tests may have started workers since.
        let ids = foreign.thread_ids();
        assert!(ids.iter().all(|&id| id > 0), "{ids:?}");
        assert!(ids.contains(&(p.tid as i32)), "ran on a worker: {p:?}");
        assert_eq!(p.errno_seen, 33, "EDOM, written and read on the worker");
        assert_eq!(p.allocated, 1);
        assert!(p.calls >= 1, "a thread-local per worker");
    }

    // Its declaration is read the way a scan reads one.
    let bytes = library.declaration().expect("read").expect("declared");
    let declared = Declared::parse(&bytes).expect("well formed");
    assert!(declared.has("kind", "fixture"));
    assert!(declared.has("name", "thread_state"));
    assert!(library.entry().expect("looked up").is_some());
    assert!(library.symbol("not_there").expect("looked up").is_none());

    // The host thread is as it was.
    assert_eq!(thread_pointer(), tp, "the thread pointer");
    assert_eq!(
        status_hex("/proc/thread-self/status", "SigBlk:"),
        mask,
        "the signal mask"
    );
    MINE.with(|m| assert_eq!(m.get(), 41, "a thread-local"));
    let e = std::fs::File::open("/nonexistent/guatiao-foreign").expect_err("absent");
    assert_eq!(e.raw_os_error(), Some(2), "errno through std");
    // SAFETY: closing -1 only sets errno.
    assert_eq!(unsafe { close(-1) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(9),
        "EBADF"
    );
    let boxes: Vec<Box<[u8; 64]>> = (0..10_000).map(|i| Box::new([i as u8; 64])).collect();
    assert!(boxes.iter().enumerate().all(|(i, b)| b[63] == i as u8));
    let big = vec![0xa5u8; 64 << 20];
    assert_eq!(big[(64 << 20) - 1], 0xa5);
    drop((boxes, big));
    let joined: Vec<u64> = (0..8u64)
        .map(|i| {
            std::thread::spawn(move || {
                MINE.with(|m| {
                    for _ in 0..1000 {
                        m.set(m.get() + i);
                    }
                    m.get()
                })
            })
        })
        .map(|t| t.join().expect("joins"))
        .collect();
    assert_eq!(joined, (0..8u64).map(|i| 7 + 1000 * i).collect::<Vec<_>>());
}

#[test]
fn many_host_threads_call_the_workers_at_once_and_more_workers_start() {
    let foreign = shared();
    let library = foreign.open(fixture()).expect("the fixture opens");
    std::thread::scope(|scope| {
        for _ in 0..6 {
            scope.spawn(|| {
                for _ in 0..500 {
                    let p = probe(&library, 256, 4);
                    assert_eq!(p.errno_seen, 33);
                    assert_eq!(p.allocated, 1);
                }
            });
        }
    });
    assert!(foreign.workers() >= WORKERS);
}

#[test]
fn workers_block_asynchronous_signals_and_glibc_takes_only_its_own() {
    let foreign = shared();
    // glibc never lets its own two (32 and 33) be blocked, nor the kernel
    // SIGKILL and SIGSTOP.
    let expected = settings()
        .blocked_signals
        .without(9)
        .without(19)
        .without(32)
        .without(33)
        .bits();
    for id in foreign.thread_ids() {
        let blocked = status_hex(&format!("/proc/self/task/{id}/status"), "SigBlk:");
        assert_eq!(blocked, expected, "thread {id}: {blocked:x}");
    }
    let taken = foreign.signals_taken();
    eprintln!(
        "handlers changed by the start: {:?}",
        taken.iter().collect::<Vec<_>>()
    );
    assert!(
        taken.iter().all(|s| s == 32 || s == 33),
        "{:?}",
        taken.iter().collect::<Vec<_>>()
    );
    assert_ne!(SignalSet::asynchronous(), SignalSet::all());
}

#[test]
fn the_program_break_stays_where_it_was() {
    let library = shared().open(fixture()).expect("the fixture opens");
    let before = program_break();
    // The thread ld.so started on is glibc's main thread, whose arena grows
    // by the break when it can; workers allocate from their own arenas or
    // fall back to it.
    let p = probe(&library, 100 << 10, 2000);
    assert_eq!(p.allocated, 1);
    let boxes: Vec<Box<[u8; 1024]>> = (0..20_000).map(|_| Box::new([1u8; 1024])).collect();
    drop(boxes);
    assert_eq!(program_break(), before);
}

#[test]
fn opening_what_is_not_there_answers_the_linker_message() {
    let foreign = shared();
    match foreign.open("/nonexistent/libnothing.so") {
        Err(Error::Open { name, message }) => {
            assert_eq!(name, "/nonexistent/libnothing.so");
            assert!(message.contains("libnothing.so"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    let library = foreign
        .open_with("libm.so.6", OpenFlags::NOW | OpenFlags::LOCAL)
        .expect("the C library's own libm opens");
    let cos = library.symbol("cos").expect("looked up").expect("there");
    assert!(!cos.address().is_null());
    assert!(
        library.declaration().expect("looked up").is_none(),
        "libm declares nothing"
    );
    // A foreign function made callable from host code: `cos` through its
    // stub, on a worker, answering in the vector register.
    let cos: extern "C" fn(f64) -> f64 =
        // SAFETY: `cos` has that signature; the stub makes the call.
        unsafe { core::mem::transmute(foreign.callable(cos.address() as usize)) };
    assert_eq!(cos(0.0), 1.0);
    assert!((cos(core::f64::consts::PI) + 1.0).abs() < 1e-12);
    library.close().expect("closes");
}

#[test]
fn a_call_after_fork_is_refused_in_the_child() {
    let foreign = shared();
    let library = foreign.open(fixture()).expect("the fixture opens");
    let job = library.symbol("fixture_probe").unwrap().unwrap();
    // SAFETY: the child only calls what returns before any lock, then exits.
    let pid = unsafe { fork() };
    if pid == 0 {
        let mut probe = Probe::default();
        // SAFETY: as in `probe`; it is refused before it runs.
        let r = unsafe { foreign.call(job.as_job(), (&mut probe as *mut Probe).cast()) };
        // SAFETY: ends the child without running the parent's destructors.
        unsafe {
            _exit(if matches!(r, Err(Error::Forked)) {
                0
            } else {
                1
            })
        };
    }
    assert!(pid > 0, "fork");
    let mut status = 0;
    // SAFETY: waits for the child just made.
    assert_eq!(unsafe { waitpid(pid, &mut status, 0) }, pid);
    assert_eq!(status, 0, "the child saw Forked");
    assert_eq!(
        probe(&library, 16, 1).allocated,
        1,
        "the parent still calls"
    );
}

/// What the host hands a conversation to tell: written on a host thread.
struct Heard(alloc::sync::Arc<std::sync::Mutex<Vec<String>>>);

impl Listener for Heard {
    fn heard(&mut self, what: &str) {
        // A thread-local of the host's, which a foreign thread would read
        // from the wrong memory.
        thread_local!(static ON_HOST: Cell<u32> = const { Cell::new(0) });
        ON_HOST.with(|c| c.set(c.get() + 1));
        self.0.lock().unwrap().push(what.to_string());
    }
}

/// The done-when: a static host's registry loads `derived_greeter`, built
/// for glibc, with no host code naming the foreign loader, and calls every
/// shape a slot takes.
#[test]
fn the_registry_loads_a_guatiao_library_built_for_glibc() {
    let Some(dir) = library_dir() else { return };
    let path = dir.join("libderived_greeter.so");
    let mut registry = Registry::new("foreign-tests", env!("CARGO_PKG_VERSION"));
    if cfg!(not(target_feature = "crt-static")) {
        // A dynamic host keeps its own loader unless told.
        assert_eq!(Dlopen::Auto.resolved(), Dlopen::System);
        registry.set_dlopen(Dlopen::Foreign);
    } else {
        assert_eq!(Dlopen::Auto.resolved(), Dlopen::Foreign);
    }
    registry.set_foreign_settings(settings());
    registry
        .load_file(&path)
        .expect("loads")
        .loaded()
        .expect("is a library for this host");

    let greeter = registry
        .offer::<dyn Greeter>("derived_greeter_hello")
        .expect("offered")
        .expect("as the trait")
        .shared()
        .expect("its own instance");
    let answer = greeter.greet("ana").expect("answers");
    assert_eq!(
        answer.get("greeting").and_then(TryAsRef::<str>::try_as_ref),
        Some("hello, ana")
    );
    let e = greeter.greet("").expect_err("an empty name is refused");
    assert_ne!(e.status, Status::GUATIAO_OK);
    assert!(!greeter.shout("bo").is_empty());

    // An object the host lends for the call, called back on this thread.
    let log = alloc::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut lent = Heard(log.clone());
    greeter.greet_into("cy", &mut lent).expect("told");
    assert_eq!(log.lock().unwrap().as_slice(), ["hello, cy"]);

    // A map handed over, and one lent to be filled in.
    let mut config = Map::new();
    config.set("name", "di");
    let from = greeter.greet_from(config.clone()).expect("greets");
    assert_eq!(
        from.get("greeting").and_then(TryAsRef::<str>::try_as_ref),
        Some("hello, di")
    );
    assert!(greeter.check(&mut config).expect("checks"));

    // An object the host owns, handed over; an object the library owns,
    // handed back and driven from here.
    let told = alloc::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    match greeter.start(Heard(told.clone()).into_object()) {
        Ok(mut conversation) => {
            let conversation: &mut dyn Conversation = &mut *conversation;
            conversation.say("hi").expect("said");
            conversation.say("bye").expect("said");
            assert_eq!(conversation.turns(), 2);
            let mut buffer = [0u8; 64];
            let n = conversation.read(&mut buffer);
            assert!(n > 0, "a transcript");
        }
        Err(e) => eprintln!("this greeter holds no conversations: {e}"),
    }

    let counter = registry
        .offer::<dyn Counter>("derived_greeter_hello")
        .expect("offered")
        .expect("as the trait")
        .shared()
        .expect("its own instance");
    let first = counter.count();
    assert_eq!(counter.count(), first + 1);
    // What a call costs here: a pure slot, and one whose answer is built
    // through the host's allocator (each allocation a call back).
    let rounds = 20_000u32;
    let started = std::time::Instant::now();
    for _ in 0..rounds {
        counter.count();
    }
    let pure = started.elapsed() / rounds;
    let started = std::time::Instant::now();
    for _ in 0..rounds / 10 {
        greeter.greet("ana").expect("answers");
    }
    let building = started.elapsed() / (rounds / 10);
    eprintln!("per call through the foreign loader: count {pure:?}, greet {building:?}");
    if let Some(Ok(wide)) = registry.offer::<dyn Wide>("derived_greeter_hello") {
        let wide = wide.shared().expect("its own instance");
        for x in [i64::MIN, -1, 0, 1, i64::MAX] {
            assert_eq!(wide.echo_i64(x).expect("echoes"), x);
        }
        assert_eq!(wide.echo_u64(u64::MAX).expect("echoes"), u64::MAX);
    }

    // Many host threads through one provider.
    std::thread::scope(|scope| {
        for t in 0..4 {
            let greeter = &greeter;
            scope.spawn(move || {
                for i in 0..200 {
                    let name = format!("t{t}-{i}");
                    let answer = greeter.greet(&name).expect("answers");
                    assert_eq!(
                        answer.get("greeting").and_then(TryAsRef::<str>::try_as_ref),
                        Some(format!("hello, {name}").as_str())
                    );
                }
            });
        }
    });
    drop((greeter, counter));
    let key = registry.loaded()[0].key.to_string();
    match registry.unload(&key) {
        Ok(_) => {}
        Err(e) => eprintln!("unload: {e}"),
    }
}

/// The fence's control: a child process with glibc held to one arena (so
/// every worker allocates from the arena that grows by the program break)
/// allocates and frees blocks glibc trims, between rounds of host
/// allocations kept alive and checked. Fenced it must come through; the
/// unfenced child's end is reported, not judged.
#[test]
#[ignore = "a control: starts children that may crash"]
fn the_program_break_fence_control() {
    let exe = std::env::current_exe().expect("the test binary");
    let mut fenced_ok = false;
    for mode in ["fenced", "unfenced"] {
        let status = Command::new(&exe)
            .args([
                "--exact",
                "control_child",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("GUATIAO_FOREIGN_CONTROL", mode)
            .status()
            .expect("the child runs");
        eprintln!("control {mode}: {status:?}");
        if mode == "fenced" {
            fenced_ok = status.success();
        }
    }
    assert!(fenced_ok, "the fenced child came through");
}

/// The control's child; a no-op unless `GUATIAO_FOREIGN_CONTROL` names a
/// mode.
#[test]
fn control_child() {
    let Some(mode) = std::env::var_os("GUATIAO_FOREIGN_CONTROL") else {
        return;
    };
    let mut s = settings();
    s.fence_program_break = mode == "fenced";
    s.environment = guatiao::library::foreign::Environment::Exact(vec![
        "GLIBC_TUNABLES=glibc.malloc.arena_max=1".into(),
    ]);
    let foreign = Foreign::start(&s).expect("starts");
    let library = foreign.open(fixture()).expect("the fixture opens");
    let start = program_break();
    let mut kept: Vec<Box<[u8; 1024]>> = Vec::new();
    let mut moved = false;
    for round in 0..50u8 {
        let p = probe(&library, 120 << 10, 50);
        assert_eq!(p.allocated, 1);
        for _ in 0..2000 {
            kept.push(Box::new([round; 1024]));
        }
        if program_break() != start {
            moved = true;
        }
        let mut at = 0;
        for r in 0..=round {
            for _ in 0..2000 {
                assert_eq!(kept[at][1023], r, "a host block survived");
                at += 1;
            }
        }
    }
    eprintln!(
        "control {}: the break {} ({:#x} -> {:#x}), {} host blocks intact",
        mode.to_string_lossy(),
        if moved { "moved" } else { "stayed" },
        start,
        program_break(),
        kept.len()
    );
}
