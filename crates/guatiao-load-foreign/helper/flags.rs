// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Included by `build.rs` and by `src/helper.rs`, so a helper built while the
// host runs is built the way the one built into the crate was.

/// What the compiler is given before `-o OUT SOURCE`: a position-independent
/// program with no start files (the source has its own `_start`), no stack
/// protector or fortified calls (each would add a newer symbol version), and
/// no loop turned into a `memcpy` call (GLIBC_2.14 on x86_64).
const HELPER_FLAGS: &[&str] = &[
    "-std=gnu11",
    "-O2",
    "-fPIE",
    "-pie",
    "-nostartfiles",
    "-fno-stack-protector",
    "-U_FORTIFY_SOURCE",
    "-fno-builtin",
    "-fno-tree-loop-distribute-patterns",
    "-fno-asynchronous-unwind-tables",
    "-Wl,--hash-style=both",
    "-Wl,-z,noexecstack",
    "-Wl,--no-as-needed",
];

/// What follows the source: the C library, and before glibc 2.34 the two
/// libraries holding `dlopen` and `pthread_create`, named by soname so they
/// stay needed where they are empty.
const HELPER_LIBS: &[&str] = &["-l:libdl.so.2", "-l:libpthread.so.0"];
