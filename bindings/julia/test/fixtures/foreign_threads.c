/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * Calls a greeter's `greet` slot from threads Julia did not start, as a
 * library with threads of its own would: `foreign_greet_joined` starts
 * them and waits for every one; `foreign_greet_detached` starts one and
 * returns at once, `foreign_done` saying when it came back. The caller
 * passes `guatiao_value_free`, so this links nothing.
 */

#ifndef _WIN32
#define _GNU_SOURCE
#endif

#include <guatiao.h>
#include <greeter_kind.h>
#include <string.h>

#ifdef _WIN32
#include <windows.h>
#define EXPORT __declspec(dllexport)
#else
#include <pthread.h>
#define EXPORT __attribute__((visibility("default")))
#endif

typedef guatiao_status (*value_free_fn)(guatiao_value *value);

typedef struct {
  const greeter_vtable *table;
  void *ctx;
  value_free_fn free_value;
  const char *name;
  const char *expect;
  int calls;
  int failures;
  volatile int done;
} job;

static void call_greet(job *j) {
  guatiao_str name = {(const uint8_t *)j->name, strlen(j->name)};
  size_t want = strlen(j->expect);
  for (int i = 0; i < j->calls; i++) {
    guatiao_map out = {0};
    guatiao_provider_error err = {0};
    guatiao_status status = j->table->greet(j->ctx, name, &out, &err);
    int ok = 0;
    if (status == GUATIAO_OK) {
      for (size_t k = 0; k < out.len; k++) {
        const guatiao_entry *e = &out.ptr[k];
        if (e->key.len == 8 && memcmp(e->key.ptr, "greeting", 8) == 0 && e->value.tag == GUATIAO_STRING &&
            e->value.payload.text.len == want && memcmp(e->value.payload.text.ptr, j->expect, want) == 0) {
          ok = 1;
        }
      }
      guatiao_value answer = {GUATIAO_MAP, 0, {0}};
      answer.payload.map = out;
      j->free_value(&answer);
    } else if (err.message.cap) {
      guatiao_value message = {GUATIAO_STRING, 0, {0}};
      message.payload.text = err.message;
      j->free_value(&message);
    }
    if (!ok) j->failures++;
  }
  j->done = 1;
}

#ifdef _WIN32
static DWORD WINAPI run(LPVOID p) {
  call_greet((job *)p);
  return 0;
}
#else
static void *run(void *p) {
  call_greet((job *)p);
  return 0;
}
#endif

/* `threads` threads (at most 64), each calling greet(name) `calls` times
 * and checking the greeting; answers the calls that failed, or -1 when a
 * thread could not start. */
EXPORT int foreign_greet_joined(const greeter_vtable *table, void *ctx, value_free_fn free_value, const char *name,
                                const char *expect, int threads, int calls) {
  job jobs[64];
  if (threads > 64) threads = 64;
#ifdef _WIN32
  HANDLE handles[64];
#else
  pthread_t handles[64];
#endif
  for (int i = 0; i < threads; i++) {
    jobs[i] = (job){table, ctx, free_value, name, expect, calls, 0, 0};
#ifdef _WIN32
    handles[i] = CreateThread(NULL, 0, run, &jobs[i], 0, NULL);
    if (!handles[i]) return -1;
#else
    if (pthread_create(&handles[i], NULL, run, &jobs[i]) != 0) return -1;
#endif
  }
  int failures = 0;
  for (int i = 0; i < threads; i++) {
#ifdef _WIN32
    WaitForSingleObject(handles[i], INFINITE);
    CloseHandle(handles[i]);
#else
    pthread_join(handles[i], NULL);
#endif
    failures += jobs[i].failures;
  }
  return failures;
}

static job detached;
static char detached_name[64];
static char detached_expect[128];

/* One thread calling greet(name) once; returns at once. */
EXPORT int foreign_greet_detached(const greeter_vtable *table, void *ctx, value_free_fn free_value, const char *name,
                                  const char *expect) {
  strncpy(detached_name, name, sizeof detached_name - 1);
  strncpy(detached_expect, expect, sizeof detached_expect - 1);
  detached = (job){table, ctx, free_value, detached_name, detached_expect, 1, 0, 0};
#ifdef _WIN32
  HANDLE thread = CreateThread(NULL, 0, run, &detached, 0, NULL);
  if (!thread) return 0;
  CloseHandle(thread);
#else
  pthread_t thread;
  if (pthread_create(&thread, NULL, run, &detached) != 0) return 0;
  pthread_detach(thread);
#endif
  return 1;
}

EXPORT int foreign_done(void) { return detached.done; }
EXPORT int foreign_failures(void) { return detached.failures; }
