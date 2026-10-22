/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/* Calls a greeter's `greet` slot from the calling thread and from threads
 * the caller did not start, freeing each answer: what a host's own
 * threads do to a provider. */
#ifndef _WIN32
#define _GNU_SOURCE
#endif
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#ifdef _WIN32
#include <windows.h>
#define EXPORT __declspec(dllexport)
#else
#include <pthread.h>
#include <unistd.h>
#ifndef __APPLE__
#include <sys/syscall.h>
#endif
#define EXPORT __attribute__((visibility("default")))
#endif

typedef struct { const uint8_t *ptr; size_t len; } gstr;
typedef struct { void *ptr; size_t len, cap; const void *alloc; } gowned;
typedef struct { uint32_t tag, pad; gowned payload; } gvalue;
typedef struct { uint32_t status; gowned message; } gerr;
typedef uint32_t (*greet_fn)(void *ctx, gstr name, gowned *out, gerr *err);
typedef uint32_t (*free_fn)(gvalue *v);

typedef struct {
  greet_fn fn;
  free_fn free;
  void *ctx;
  int calls;
  volatile uint32_t status;
  volatile uint64_t tid;
  volatile int done;
} job;

static const char NAME[] = "thread";

EXPORT uint64_t threads_thread_id(void) {
#ifdef _WIN32
  return GetCurrentThreadId();
#elif defined(__APPLE__)
  uint64_t id = 0;
  pthread_threadid_np(NULL, &id);
  return id;
#else
  return (uint64_t)syscall(SYS_gettid);
#endif
}

static void run(job *j) {
  gstr n = {(const uint8_t *)NAME, 6};
  j->tid = threads_thread_id();
  uint32_t s = 0;
  for (int i = 0; i < j->calls; i++) {
    gvalue answer;
    gerr err;
    memset(&answer, 0, sizeof answer);
    memset(&err, 0, sizeof err);
    answer.tag = 7; /* a map: the slot fills the arm in place */
    s = j->fn(j->ctx, n, &answer.payload, &err);
    if (s) break;
    j->free(&answer);
  }
  j->status = s;
  j->done = 1;
}

EXPORT uint32_t threads_call_here(greet_fn f, free_fn fr, void *ctx, int calls) {
  job j = {f, fr, ctx, calls, 999, 0, 0};
  run(&j);
  return j.status;
}

/* The answer to one call with `name`, as a map value the caller frees. */
EXPORT uint32_t threads_call_named(greet_fn f, void *ctx, gstr name, gvalue *out, gerr *err) {
  memset(out, 0, sizeof *out);
  out->tag = 7;
  return f(ctx, name, &out->payload, err);
}

#ifdef _WIN32
static DWORD WINAPI thr(LPVOID p) { run((job *)p); return 0; }
#else
static void *thr(void *p) { run((job *)p); return 0; }
#endif

EXPORT uint32_t threads_call_on_threads(greet_fn f, free_fn fr, void *ctx, int n, int calls, uint64_t *tids) {
  job jobs[64];
  if (n > 64) n = 64;
#ifdef _WIN32
  HANDLE hs[64];
#else
  pthread_t hs[64];
#endif
  for (int i = 0; i < n; i++) {
    jobs[i].fn = f; jobs[i].free = fr; jobs[i].ctx = ctx; jobs[i].calls = calls;
    jobs[i].status = 999; jobs[i].tid = 0; jobs[i].done = 0;
#ifdef _WIN32
    hs[i] = CreateThread(0, 0, thr, &jobs[i], 0, 0);
#else
    pthread_create(&hs[i], 0, thr, &jobs[i]);
#endif
  }
  uint32_t status = 0;
  for (int i = 0; i < n; i++) {
#ifdef _WIN32
    WaitForSingleObject(hs[i], INFINITE);
    CloseHandle(hs[i]);
#else
    pthread_join(hs[i], 0);
#endif
    if (tids) tids[i] = jobs[i].tid;
    if (!status && jobs[i].status) status = jobs[i].status;
  }
  return status;
}

static job detached;

EXPORT void threads_call_detached(greet_fn f, free_fn fr, void *ctx, int calls) {
  detached.fn = f; detached.free = fr; detached.ctx = ctx; detached.calls = calls;
  detached.status = 999; detached.done = 0; detached.tid = 0;
#ifdef _WIN32
  HANDLE h = CreateThread(0, 0, thr, &detached, 0, 0);
  CloseHandle(h);
#else
  pthread_t t;
  pthread_create(&t, 0, thr, &detached);
  pthread_detach(t);
#endif
}

EXPORT int threads_detached_done(void) { return detached.done; }
EXPORT uint32_t threads_detached_status(void) { return detached.status; }
