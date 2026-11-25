/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * The program the system's dynamic linker starts inside a statically linked
 * host. ld.so maps the system C library for it; its first thread never
 * returns: it starts the other workers, reports ready through the control
 * block whose address is argv[1], and then runs jobs like every worker. A job
 * is a function of this C library's world and one pointer, so library code
 * only ever runs on threads this C library made.
 *
 * Every C library symbol is pinned to the oldest version its architecture's
 * glibc has (GLIBC_2.2.5 on x86_64, where dladdr1 is GLIBC_2.3.3; GLIBC_2.17
 * on aarch64), and _start is our own, so a helper built on a new glibc runs on
 * an old one. Before glibc 2.34 the dl and pthread functions live in libdl
 * and libpthread, which the build links by soname for that reason.
 *
 * The layouts below are mirrored in src/worker.rs and checked by size on
 * both sides.
 */

#define _GNU_SOURCE
#include <dlfcn.h>
#include <link.h>
#include <pthread.h>
#include <signal.h>
#include <stddef.h>
#include <stdint.h>
#include <sys/syscall.h>
#include <unistd.h>

#if defined(__x86_64__)
#define GT_BASE "GLIBC_2.2.5"
#define GT_DLADDR1 "GLIBC_2.3.3"
#elif defined(__aarch64__)
#define GT_BASE "GLIBC_2.17"
#define GT_DLADDR1 "GLIBC_2.17"
#else
#error "the helper is written for x86_64 and aarch64"
#endif

#define GT_PIN(symbol, version) __asm__(".symver " #symbol "," #symbol "@" version)
GT_PIN(__libc_start_main, GT_BASE);
GT_PIN(dlopen, GT_BASE);
GT_PIN(dlsym, GT_BASE);
GT_PIN(dlerror, GT_BASE);
GT_PIN(dlclose, GT_BASE);
GT_PIN(dladdr1, GT_DLADDR1);
GT_PIN(pthread_create, GT_BASE);
GT_PIN(pthread_attr_init, GT_BASE);
GT_PIN(pthread_attr_setstacksize, GT_BASE);
GT_PIN(pthread_attr_destroy, GT_BASE);
GT_PIN(pthread_sigmask, GT_BASE);
GT_PIN(syscall, GT_BASE);

#define GT_FUTEX_WAIT 128 /* FUTEX_WAIT | FUTEX_PRIVATE_FLAG */
#define GT_FUTEX_WAKE 129 /* FUTEX_WAKE | FUTEX_PRIVATE_FLAG */

enum { GT_IDLE = 0, GT_POSTED = 1, GT_DONE = 2 };
enum { GT_STARTING = 0, GT_READY = 1, GT_FAILED = 2 };
enum { GT_FAIL_LAYOUT = -1 };
enum { GT_DL_OPEN = 1, GT_DL_SYMBOL = 2, GT_DL_CLOSE = 3 };

/* One worker's mailbox. `state` is the futex word both sides wait on. */
struct gt_slot {
    uint32_t state;
    uint32_t worker_sleeps;
    uint32_t host_sleeps;
    int32_t tid;
    void (*function)(void *);
    void *argument;
};

/* `ready` and `failure` come first so a helper can report a layout it does
 * not know. */
struct gt_control {
    uint32_t size;
    uint32_t ready;
    int32_t failure;
    uint32_t slot_size;
    uint32_t workers;
    uint32_t spin;
    uint64_t blocked; /* signal n is bit n - 1 */
    uint64_t worker_stack; /* 0: the C library's default */
    struct gt_slot *slots;
    uint32_t started;
    uint32_t pad;
    void (*dl)(void *);
};

/* A request for the dynamic linker, run as a job. */
struct gt_dl {
    uint32_t op;
    int32_t flags;
    void *handle;
    const char *name;
    void *address;
    uint64_t size;
    char *error;
    uint64_t error_cap;
    uint64_t error_len;
    int32_t status;
    uint32_t pad;
};

static struct gt_control *gt_control;
static uint32_t gt_never;

static long gt_wait(uint32_t *word, uint32_t expected) {
    return syscall(SYS_futex, word, GT_FUTEX_WAIT, expected, (void *)0, (void *)0, 0);
}

static void gt_wake(uint32_t *word) {
    syscall(SYS_futex, word, GT_FUTEX_WAKE, 0x7fffffff, (void *)0, (void *)0, 0);
}

static void gt_relax(void) {
#if defined(__x86_64__)
    __asm__ volatile("pause");
#else
    __asm__ volatile("yield");
#endif
}

static void __attribute__((noreturn)) gt_park(void) {
    for (;;)
        gt_wait(&gt_never, 0);
}

static void gt_fail(struct gt_control *c, int32_t failure) {
    __atomic_store_n(&c->failure, failure, __ATOMIC_SEQ_CST);
    __atomic_store_n(&c->ready, GT_FAILED, __ATOMIC_SEQ_CST);
    gt_wake(&c->ready);
}

/* Only signals 1 to 64 exist on Linux; glibc removes its own two from any
 * mask it is given. */
static void gt_block(uint64_t blocked) {
    sigset_t set;
    volatile unsigned long *word = set.__val;
    for (size_t i = 0; i < sizeof set.__val / sizeof set.__val[0]; i++)
        word[i] = 0;
    word[0] = (unsigned long)blocked;
    pthread_sigmask(SIG_SETMASK, &set, (sigset_t *)0);
}

static void __attribute__((noreturn)) gt_serve(struct gt_slot *s) {
    uint32_t spin = gt_control->spin;
    for (;;) {
        uint32_t left = spin;
        for (;;) {
            uint32_t seen = __atomic_load_n(&s->state, __ATOMIC_ACQUIRE);
            if (seen == GT_POSTED)
                break;
            if (left) {
                left--;
                gt_relax();
                continue;
            }
            __atomic_store_n(&s->worker_sleeps, 1, __ATOMIC_SEQ_CST);
            seen = __atomic_load_n(&s->state, __ATOMIC_SEQ_CST);
            if (seen != GT_POSTED)
                gt_wait(&s->state, seen);
            __atomic_store_n(&s->worker_sleeps, 0, __ATOMIC_SEQ_CST);
        }
        s->function(s->argument);
        __atomic_store_n(&s->state, GT_DONE, __ATOMIC_SEQ_CST);
        if (__atomic_load_n(&s->host_sleeps, __ATOMIC_SEQ_CST))
            gt_wake(&s->state);
    }
}

static void gt_note(struct gt_dl *r, const char *message) {
    uint64_t n = 0;
    r->status = 1;
    if (message && r->error)
        while (n < r->error_cap && message[n]) {
            r->error[n] = message[n];
            n++;
        }
    r->error_len = n;
}

static void gt_dl(void *argument) {
    struct gt_dl *r = argument;
    r->status = 0;
    r->error_len = 0;
    switch (r->op) {
    case GT_DL_OPEN:
        dlerror();
        r->handle = dlopen(r->name, r->flags);
        if (!r->handle)
            gt_note(r, dlerror());
        break;
    case GT_DL_SYMBOL: {
        const char *message;
        Dl_info info;
        const ElfW(Sym) *symbol = 0;
        dlerror();
        r->address = dlsym(r->handle, r->name);
        r->size = 0;
        message = dlerror();
        if (message) {
            gt_note(r, message);
            break;
        }
        if (r->address && dladdr1(r->address, &info, (void **)&symbol, RTLD_DL_SYMENT) && symbol)
            r->size = symbol->st_size;
        break;
    }
    case GT_DL_CLOSE:
        if (dlclose(r->handle))
            gt_note(r, dlerror());
        break;
    default:
        gt_note(r, "unknown request");
    }
}

static void *gt_worker(void *argument) {
    struct gt_slot *s = argument;
    __atomic_store_n(&s->tid, (int32_t)syscall(SYS_gettid), __ATOMIC_SEQ_CST);
    __atomic_add_fetch(&gt_control->started, 1, __ATOMIC_SEQ_CST);
    gt_wake(&gt_control->started);
    gt_serve(s);
}

static struct gt_control *gt_parse(const char *text) {
    uintptr_t value = 0;
    if (!text || !*text)
        return 0;
    for (; *text; text++) {
        unsigned digit;
        if (*text >= '0' && *text <= '9')
            digit = (unsigned)(*text - '0');
        else if (*text >= 'a' && *text <= 'f')
            digit = (unsigned)(*text - 'a' + 10);
        else
            return 0;
        value = value * 16 + digit;
    }
    return (struct gt_control *)value;
}

int main(int argc, char **argv) {
    struct gt_control *c = argc > 1 ? gt_parse(argv[1]) : 0;
    if (!c)
        gt_park();
    if (c->size != sizeof *c || c->slot_size != sizeof(struct gt_slot) || c->workers == 0) {
        gt_fail(c, GT_FAIL_LAYOUT);
        gt_park();
    }
    gt_control = c;
    gt_block(c->blocked);
    c->dl = gt_dl;
    __atomic_store_n(&c->slots[0].tid, (int32_t)syscall(SYS_gettid), __ATOMIC_SEQ_CST);
    for (uint32_t i = 1; i < c->workers; i++) {
        pthread_attr_t attr;
        pthread_t thread;
        int err = pthread_attr_init(&attr);
        if (err) {
            gt_fail(c, err);
            gt_park();
        }
        if (c->worker_stack)
            err = pthread_attr_setstacksize(&attr, (size_t)c->worker_stack);
        if (!err)
            err = pthread_create(&thread, &attr, gt_worker, &c->slots[i]);
        pthread_attr_destroy(&attr);
        if (err) {
            gt_fail(c, err);
            gt_park();
        }
    }
    for (;;) {
        uint32_t started = __atomic_load_n(&c->started, __ATOMIC_SEQ_CST);
        if (started == c->workers - 1)
            break;
        gt_wait(&c->started, started);
    }
    __atomic_store_n(&c->ready, GT_READY, __ATOMIC_SEQ_CST);
    gt_wake(&c->ready);
    gt_serve(&c->slots[0]);
}

/* What crt1.o does, against the pinned __libc_start_main: argc and argv from
 * the stack ld.so hands over, ld.so's finaliser from rdx or x0. */
#if defined(__x86_64__)
__asm__(".text\n"
        ".globl _start\n"
        ".type _start,@function\n"
        "_start:\n"
        "  xorl %ebp, %ebp\n"
        "  movq %rdx, %r9\n"
        "  popq %rsi\n"
        "  movq %rsp, %rdx\n"
        "  andq $-16, %rsp\n"
        "  pushq %rax\n"
        "  pushq %rsp\n"
        "  xorl %r8d, %r8d\n"
        "  xorl %ecx, %ecx\n"
        "  leaq main(%rip), %rdi\n"
        "  call *__libc_start_main@GOTPCREL(%rip)\n"
        "  hlt\n"
        ".size _start, .-_start\n");
#else
__asm__(".text\n"
        ".globl _start\n"
        ".type _start,%function\n"
        "_start:\n"
        "  mov x29, #0\n"
        "  mov x30, #0\n"
        "  mov x5, x0\n"
        "  ldr x1, [sp]\n"
        "  add x2, sp, #8\n"
        "  mov x6, sp\n"
        "  mov x3, #0\n"
        "  mov x4, #0\n"
        "  adrp x0, main\n"
        "  add x0, x0, :lo12:main\n"
        "  bl __libc_start_main\n"
        "  brk #0\n"
        ".size _start, .-_start\n");
#endif
