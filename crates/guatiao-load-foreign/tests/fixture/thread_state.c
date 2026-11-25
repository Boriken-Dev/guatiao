/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * A library in C for the system C library, built by tests/foreign.rs: it
 * declares itself as a guatiao library does, and its jobs touch the thread
 * state a C library keeps behind the thread pointer, reporting what they
 * saw.
 */

#include <errno.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/syscall.h>
#include <unistd.h>

/* What the library declares, read as data; the literal's own NUL ends the
 * last pair and the extra one is the terminating empty string. */
const char guatiao_declares[] = "kind=fixture\0name=thread_state\0";

/* Present so the file has the guatiao entry symbol; never called here. */
const void *guatiao_library_entry(const void *host) {
    (void)host;
    return 0;
}

static __thread int64_t calls;

struct probe {
    int64_t tid;
    int64_t errno_seen;
    int64_t calls;
    int64_t allocated;
    uint64_t size;
    uint64_t rounds;
};

/* Sets errno, counts in a thread-local and allocates `size` bytes `rounds`
 * times, filling and checking each block. */
void fixture_probe(void *argument) {
    struct probe *p = argument;
    p->tid = (int64_t)syscall(SYS_gettid);
    errno = 0;
    errno = EDOM;
    p->errno_seen = errno;
    p->calls = ++calls;
    p->allocated = 1;
    for (uint64_t i = 0; i < p->rounds; i++) {
        unsigned char *block = malloc(p->size);
        if (!block) {
            p->allocated = 0;
            return;
        }
        memset(block, 0x5a, p->size);
        if (block[0] != 0x5a || block[p->size - 1] != 0x5a)
            p->allocated = 0;
        free(block);
    }
}
