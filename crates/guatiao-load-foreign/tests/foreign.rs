// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The loader from a host's side: what it refuses, what runs on its
//! workers, and that the host's own thread state is what it was.
//!
//! The point is a statically linked host: run these with a musl target
//! (`cargo test --target x86_64-unknown-linux-musl`, or aarch64). They
//! pass in a glibc host as well. `fixture/thread_state.c` is built with
//! `GUATIAO_FOREIGN_HELPER_CC` (default `cc`). The ignored test reads
//! `GUATIAO_FOREIGN_LIBRARY`, a guatiao library built for the system C
//! library (`cargo build -p derived_greeter` on the host's architecture).

#![cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]

use core::cell::Cell;
use core::ffi::{c_int, c_long, c_void};
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use guatiao::library::Declared;
use guatiao_load_foreign::{
    Error, Foreign, Helper, ImageError, Library, OpenFlags, Settings, SignalSet,
};

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
    std::env::var_os("GUATIAO_FOREIGN_HELPER_CC").unwrap_or_else(|| "cc".into())
}

/// `fixture/thread_state.c`, built once per test binary.
fn fixture() -> &'static PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("guatiao-load-foreign");
        std::fs::create_dir_all(&dir).expect("a temporary directory");
        let out = dir.join(format!("libthread_state-{}.so", std::process::id()));
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixture/thread_state.c");
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

/// One loader for the tests that only need one.
fn shared() -> &'static Foreign {
    static STARTED: OnceLock<Foreign> = OnceLock::new();
    STARTED.get_or_init(|| Foreign::start(&settings()).expect("the loader starts"))
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

fn probe_on(library: &Library, worker: Option<usize>, size: u64, rounds: u64) -> Probe {
    let job = library
        .symbol("fixture_probe")
        .expect("looked up")
        .expect("the fixture exports it");
    let mut probe = Probe {
        size,
        rounds,
        ..Probe::default()
    };
    let argument = (&mut probe as *mut Probe).cast::<c_void>();
    // SAFETY: `fixture_probe` takes a `struct probe *`, which `Probe`
    // mirrors, alive across the call.
    unsafe {
        let job = job.as_job();
        match worker {
            Some(w) => library.foreign().call_on(w, job, argument),
            None => library.foreign().call(job, argument),
        }
        .expect("the job runs");
    }
    probe
}

#[test]
fn a_missing_dynamic_linker_is_a_clear_error() {
    let mut s = settings();
    s.ld_so = Some("/nonexistent/ld-linux.so".into());
    let e = Foreign::start(&s).expect_err("there is no such file");
    assert!(
        matches!(&e, Error::LdSo { path, reason: ImageError::Io(io) }
            if path.to_str() == Some("/nonexistent/ld-linux.so")
                && io.kind() == std::io::ErrorKind::NotFound),
        "{e:?}"
    );
    let text = e.to_string();
    assert!(text.contains("/nonexistent/ld-linux.so"), "{text}");
}

#[test]
fn a_missing_helper_is_a_clear_error() {
    let mut s = settings();
    s.helper = Helper::Path("/nonexistent/helper".into());
    let e = Foreign::start(&s).expect_err("there is no such file");
    assert!(
        matches!(&e, Error::Helper { path: Some(p), reason: ImageError::Io(io) }
            if p.to_str() == Some("/nonexistent/helper") && io.kind() == std::io::ErrorKind::NotFound),
        "{e:?}"
    );
    assert!(e.to_string().contains("/nonexistent/helper"), "{e}");
}

#[test]
fn files_that_are_not_what_they_should_be_are_refused_before_anything_runs() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixture/thread_state.c");
    let mut s = settings();
    s.helper = Helper::Path(source.clone());
    assert!(matches!(
        Foreign::start(&s),
        Err(Error::Helper {
            reason: ImageError::NotElf,
            ..
        })
    ));
    // A program where the dynamic linker should be: the helper names an
    // interpreter.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("guatiao-load-foreign");
    std::fs::create_dir_all(&dir).unwrap();
    let program = dir.join(format!("not-a-linker-{}", std::process::id()));
    std::fs::write(&program, Helper::built().expect("this build has a helper")).unwrap();
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
fn the_helper_needs_no_newer_c_library_than_the_oldest_of_its_architecture() {
    let needs = Helper::Built.needs().expect("the built helper reads");
    eprintln!("the helper needs: {needs:?}");
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
fn a_library_loads_on_a_foreign_worker_and_the_host_keeps_its_thread_state() {
    thread_local!(static MINE: Cell<u64> = const { Cell::new(7) });
    let tp = thread_pointer();
    let mask = status_hex("/proc/thread-self/status", "SigBlk:");
    MINE.with(|m| m.set(41));

    let foreign = shared();
    let library = foreign.open(fixture()).expect("the fixture opens");

    // The library ran on the workers: its errno, allocator and thread-local
    // are the foreign C library's.
    let ids = foreign.worker_ids();
    assert_eq!(ids.len(), WORKERS);
    assert!(ids.iter().all(|&id| id > 0), "{ids:?}");
    for (w, &id) in ids.iter().enumerate() {
        let first = probe_on(&library, Some(w), 4096, 100);
        let second = probe_on(&library, Some(w), 4096, 100);
        assert_eq!(first.tid, i64::from(id));
        assert_eq!(first.errno_seen, 33, "EDOM, written and read on the worker");
        assert_eq!(first.allocated, 1);
        // Other tests share the workers, so the count only grows.
        assert!(second.calls > first.calls, "a thread-local per worker");
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
    let e = std::fs::File::open("/nonexistent/guatiao-load-foreign").expect_err("absent");
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
fn many_host_threads_call_the_workers_at_once() {
    let library = shared().open(fixture()).expect("the fixture opens");
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                for _ in 0..500 {
                    let p = probe_on(&library, None, 256, 4);
                    assert_eq!(p.errno_seen, 33);
                    assert_eq!(p.allocated, 1);
                }
            });
        }
    });
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
    for id in foreign.worker_ids() {
        let blocked = status_hex(&format!("/proc/self/task/{id}/status"), "SigBlk:");
        assert_eq!(blocked, expected, "worker {id}: {blocked:x}");
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
    // The first worker is glibc's main thread, whose arena grows by the
    // break when it can.
    let p = probe_on(&library, Some(0), 100 << 10, 2000);
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
        // SAFETY: as in `probe_on`; it is refused before it runs.
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
        probe_on(&library, None, 16, 1).allocated,
        1,
        "the parent still calls"
    );
}

#[test]
fn a_helper_built_while_the_host_runs_starts_like_the_built_one() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("guatiao-load-foreign");
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join(format!("helper-{}", std::process::id()));
    let helper = Helper::build(std::path::Path::new(&compiler()), &out).expect("builds");
    assert_eq!(helper.needs().unwrap(), Helper::Built.needs().unwrap());
    let mut s = settings();
    s.helper = helper;
    s.workers = 1;
    // The first loader's glibc handles signal 33 by now.
    s.allow_reserved_signal_handlers = true;
    let second = Foreign::start(&s).expect("a second loader starts");
    let library = second.open(fixture()).expect("opens");
    assert_eq!(probe_on(&library, None, 64, 10).allocated, 1);
    let _ = std::fs::remove_file(&out);
}

/// The done-when of a real guatiao library: `derived_greeter` built for the
/// system C library.
#[test]
#[ignore = "needs GUATIAO_FOREIGN_LIBRARY, a guatiao library built for the system C library"]
fn a_guatiao_library_built_for_glibc_loads_and_declares() {
    let path = std::env::var_os("GUATIAO_FOREIGN_LIBRARY")
        .expect("GUATIAO_FOREIGN_LIBRARY names the library");
    let library = shared().open(&path).expect("the library opens");
    let bytes = library.declaration().expect("read").expect("declared");
    let declared = Declared::parse(&bytes).expect("well formed");
    let kinds: Vec<&str> = declared.kinds().collect();
    eprintln!("{} declares {kinds:?}", path.to_string_lossy());
    assert!(kinds.contains(&"greeter"), "{kinds:?}");
    assert!(library.entry().expect("looked up").is_some());
}
