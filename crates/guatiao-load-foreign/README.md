# guatiao-load-foreign

[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)

Native [guatiao](https://crates.io/crates/guatiao) libraries in a statically
linked Linux host. A static program (a musl build, say) has no dynamic linker
and cannot map a shared library; this crate maps the **system's own** `ld.so`
and a small helper program built against the system C library, enters `ld.so`
on a thread of its own, and from then on asks that C library to open
libraries and run their code, on threads it made.

guatiao can do this where most hosts cannot: nothing a guatiao host holds
needs a library's allocator. Values cross as C shapes and are copied into the
caller's allocator, so the host's C library and the system's never free each
other's memory.

> **Status: alpha, not published.** Linux on x86_64 and aarch64; the crate is
> empty elsewhere. It opens libraries and reads their declarations; the
> registry does not take such a library yet.

## Use

```rust
use guatiao_load_foreign::{Foreign, Settings};

let foreign = Foreign::start(&Settings::default())?;
let library = foreign.open("/opt/plugins/libgreeter.so")?;   // dlopen, on a foreign worker
let declared = library.declaration()?;                       // guatiao_declares, copied out
let entry = library.entry()?;                                // guatiao_library_entry
```

## How it works

- **Every piece of library code runs on a foreign worker**, a thread the
  system C library made: constructors, `dlopen`, `dlsym`, every call. Each C
  library keeps its per-thread state behind the thread pointer, so library
  code on a host thread would read the host's (and the other way round). The
  thread `ld.so` starts on is the first worker; it never returns to host code
  and is never joined. A host thread posts a job and waits, spinning a while
  and then sleeping on a futex.
- **Workers block asynchronous signals**, so a signal meant for the host never
  runs its handler on a foreign thread.
- **The program break is fenced** before the foreign C library starts. Both
  allocators would otherwise grow and trim it, each from its own idea of where
  it ends, and one trims the other's heap away.
- **The helper is C source built at build time** (`helper/helper.c`), its
  C library symbols pinned to the oldest versions glibc has: GLIBC_2.2.5 (and
  GLIBC_2.3.3 for `dladdr1`) on x86_64, GLIBC_2.17 on aarch64.
  `GUATIAO_FOREIGN_HELPER_CC` names the compiler (a glibc C compiler for the
  target's architecture, default `cc`); `none` builds without one.

Every bound is a setting, and every check a strict default the host may weaken
(`Settings`): the worker count, the stacks, the spin, the signals workers
block, the environment, the start timeout, the declaration window, `dlopen`'s
flags, the reserved-signal check and the fence.

## What the process shares

- glibc installs its handler on signal 33 when it starts (its set-id
  broadcast, which is musl's cancellation signal); the loader refuses to start
  when the host already handles 32 or 33, unless told to.
- After `fork` the child has no workers, and a call there is `Error::Forked`.
- A host `setuid` reaches the host's own threads, not the foreign workers.
- The host's `exit` ends every thread; the foreign C library's `atexit`
  handlers and library destructors do not run.
- Once `ld.so` is entered it is the system's: if it cannot find the C library
  it prints why and ends the process, as it would for any program.

## Credits

Entering the system `ld.so` from a static program is the technique of
[foreign-dlopen](https://github.com/pfalcon/foreign-dlopen) (MIT) and of
Cosmopolitan libc's `dlopen` (ISC); this crate takes no code from either.

## License

MPL-2.0, as the rest of guatiao.
