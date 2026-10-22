/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * Calls a greeter's `greet` slot from a thread Node did not start, as a
 * library with threads of its own would: `foreign_greet` starts the
 * thread and returns at once; `foreign_done` says whether the call came
 * back, `foreign_status` and `foreign_greeting` what it answered. The
 * caller passes `guatiao_value_free`, so this links nothing.
 */

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

static const greeter_vtable *TABLE;
static value_free_fn FREE;
static void *CTX;
static char NAME[64];
static volatile int DONE;
static volatile guatiao_status STATUS;
static char GREETING[128];

static void call(void) {
  guatiao_str name = {(const uint8_t *)NAME, strlen(NAME)};
  guatiao_map out = {0};
  guatiao_provider_error err = {0};
  guatiao_status status = TABLE->greet(CTX, name, &out, &err);
  GREETING[0] = 0;
  if (status == GUATIAO_OK) {
    for (size_t i = 0; i < out.len; i++) {
      guatiao_str key = {out.ptr[i].key.ptr, out.ptr[i].key.len};
      if (key.len == 8 && memcmp(key.ptr, "greeting", 8) == 0 && out.ptr[i].value.tag == GUATIAO_STRING) {
        size_t n = out.ptr[i].value.payload.text.len;
        if (n >= sizeof GREETING) n = sizeof GREETING - 1;
        memcpy(GREETING, out.ptr[i].value.payload.text.ptr, n);
        GREETING[n] = 0;
      }
    }
    guatiao_value answer = {GUATIAO_MAP, 0, {0}};
    answer.payload.map = out;
    FREE(&answer);
  } else if (err.message.cap) {
    guatiao_value message = {GUATIAO_STRING, 0, {0}};
    message.payload.text = err.message;
    FREE(&message);
  }
  STATUS = status;
  DONE = 1;
}

#ifdef _WIN32
static DWORD WINAPI run(LPVOID unused) {
  (void)unused;
  call();
  return 0;
}
#else
static void *run(void *unused) {
  (void)unused;
  call();
  return 0;
}
#endif

EXPORT int foreign_greet(const greeter_vtable *table, void *ctx, const char *name, value_free_fn free_value) {
  TABLE = table;
  FREE = free_value;
  CTX = ctx;
  size_t len = strlen(name);
  if (len >= sizeof NAME) len = sizeof NAME - 1;
  memcpy(NAME, name, len);
  NAME[len] = 0;
  DONE = 0;
#ifdef _WIN32
  HANDLE thread = CreateThread(NULL, 0, run, NULL, 0, NULL);
  if (!thread) return 0;
  CloseHandle(thread);
#else
  pthread_t thread;
  if (pthread_create(&thread, NULL, run, NULL) != 0) return 0;
  pthread_detach(thread);
#endif
  return 1;
}

EXPORT int foreign_done(void) { return DONE; }
EXPORT guatiao_status foreign_status(void) { return STATUS; }
EXPORT const char *foreign_greeting(void) { return GREETING; }
