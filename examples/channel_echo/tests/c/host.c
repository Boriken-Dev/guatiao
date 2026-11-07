/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * A HOST written in C that starts a channel on the echo library, through
 * the headers alone: its sink is a channel_sink_vtable of its own, called
 * from the echo's thread, and the channel it gets back a channel_vtable.
 *
 *   host <echo library file>
 */

#include <stdio.h>
#include <string.h>

#ifdef _WIN32
#include <windows.h>
typedef SRWLOCK lock_t;
static void lock_init(lock_t *l) { InitializeSRWLock(l); }
static void lock(lock_t *l) { AcquireSRWLockExclusive(l); }
static void unlock(lock_t *l) { ReleaseSRWLockExclusive(l); }
static void pause_ms(unsigned ms) { Sleep(ms); }
#else
#include <pthread.h>
#include <time.h>
typedef pthread_mutex_t lock_t;
static void lock_init(lock_t *l) { pthread_mutex_init(l, NULL); }
static void lock(lock_t *l) { pthread_mutex_lock(l); }
static void unlock(lock_t *l) { pthread_mutex_unlock(l); }
static void pause_ms(unsigned ms) {
  struct timespec t = {(time_t)(ms / 1000), (long)(ms % 1000) * 1000000L};
  nanosleep(&t, NULL);
}
#endif

#include "channel_echo.h"
#include "guatiao.h"
#include "guatiao_channel.h"

static guatiao_alloc ALLOC = GUATIAO_ALLOC_MALLOC;

/* What the sink saw, written on the echo's thread, read on this one. */
typedef struct seen {
  lock_t lock;
  char received[8][16];
  int count;
  int closed;
  uint32_t status;
  int destroyed;
  int away; /* receive answers GUATIAO_ERR_GONE, filling err */
  const char *broken;
} seen;

static void sink_destroy(void *ctx) {
  seen *s = (seen *)ctx;
  lock(&s->lock);
  s->destroyed = 1;
  unlock(&s->lock);
}

static guatiao_status sink_receive(void *ctx, guatiao_bytes bytes, guatiao_provider_error *err) {
  if (((seen *)ctx)->away) {
    err->status = GUATIAO_ERR_GONE;
    return GUATIAO_ERR_GONE;
  }
  seen *s = (seen *)ctx;
  lock(&s->lock);
  if (s->closed) {
    s->broken = "receive after closed";
  } else if (s->count < 8 && bytes.len < 16) {
    memcpy(s->received[s->count], bytes.ptr, bytes.len);
    s->received[s->count][bytes.len] = '\0';
    s->count++;
  } else {
    s->broken = "more than the host sent";
  }
  unlock(&s->lock);
  return GUATIAO_OK;
}

static guatiao_status sink_closed(void *ctx, uint32_t status) {
  seen *s = (seen *)ctx;
  lock(&s->lock);
  if (s->closed) {
    s->broken = "closed twice";
  }
  s->closed++;
  s->status = status;
  unlock(&s->lock);
  return GUATIAO_OK;
}

static const channel_sink_vtable SINK = {
    {sizeof(channel_sink_vtable), channel_sink_vtable_FLOOR_HASH},
    sink_destroy,
    sink_receive,
    sink_closed,
};

/* Waits up to 30 s for `what` of the sink's to reach `n`. */
static int wait_for(seen *s, const int *what, int n) {
  for (int i = 0; i < 3000; i++) {
    lock(&s->lock);
    int now = *what;
    unlock(&s->lock);
    if (now >= n) {
      return 1;
    }
    pause_ms(10);
  }
  return 0;
}

static void free_error(guatiao_provider_error *err) {
  if (err->message.cap && err->message.alloc) {
    err->message.alloc->free(err->message.alloc->ctx, err->message.ptr, err->message.cap, 1);
  }
  memset(err, 0, sizeof(*err));
}

static int fail(const char *what) {
  printf("FAILED: %s\n", what);
  return 1;
}

int main(int argc, char **argv) {
  if (argc != 2) {
    printf("usage: host <echo library file>\n");
    return 2;
  }
  guatiao_registry *reg =
      guatiao_registry_new(guatiao_cstr("c-channel-host"), guatiao_cstr("1.0"), &ALLOC);
  if (!reg) {
    return fail("guatiao_registry_new");
  }
  guatiao_value report;
  if (guatiao_registry_load_file(reg, guatiao_cstr(argv[1]), &ALLOC, &report) != GUATIAO_OK) {
    return fail("guatiao_registry_load_file");
  }
  guatiao_value_free(&report);

  size_t size = 0;
  const channel_echo_vtable *echo = (const channel_echo_vtable *)guatiao_registry_provider_table(
      reg, guatiao_cstr("channel_echo_echo"), guatiao_cstr("channel_echo"), &size);
  void *echo_ctx = guatiao_registry_provider_ctx(reg, guatiao_cstr("channel_echo_echo"));
  if (!echo || size < sizeof(channel_echo_vtable) ||
      echo->header.floor_hash != channel_echo_vtable_FLOOR_HASH) {
    return fail("the echo's table");
  }

  seen s;
  memset(&s, 0, sizeof(s));
  lock_init(&s.lock);
  guatiao_object sink = {&SINK, sizeof(SINK), &s};
  guatiao_value config;
  guatiao_value_map(&ALLOC, &config);
  guatiao_object chan;
  memset(&chan, 0, sizeof(chan));
  guatiao_provider_error err;
  memset(&err, 0, sizeof(err));
  /* The sink is the echo's from here, whatever start answers. */
  guatiao_status status = echo->start(echo_ctx, &config.payload.map, sink, &chan, &err);
  guatiao_value_free(&config);
  if (status != GUATIAO_OK) {
    return fail("start");
  }
  const channel_vtable *ch = (const channel_vtable *)chan.table;
  if (!ch || chan.size < sizeof(channel_vtable) ||
      ch->header.floor_hash != channel_vtable_FLOOR_HASH) {
    return fail("the channel's table");
  }

  const char *words[] = {"one", "two", "three"};
  for (int i = 0; i < 3; i++) {
    if (ch->send(chan.ctx, guatiao_bytes_from(words[i], strlen(words[i])), &err) != GUATIAO_OK) {
      return fail("send");
    }
  }
  if (!wait_for(&s, &s.count, 3)) {
    return fail("three messages back");
  }
  for (int i = 0; i < 3; i++) {
    if (strcmp(s.received[i], words[i]) != 0) {
      return fail("in the order sent");
    }
  }
  printf("echoed: %s %s %s\n", s.received[0], s.received[1], s.received[2]);

  ch->close(chan.ctx, 5);
  status = ch->send(chan.ctx, guatiao_bytes_from("late", 4), &err);
  free_error(&err);
  if (status != GUATIAO_ERR_GONE) {
    return fail("a send after close answers GUATIAO_ERR_GONE");
  }
  if (!wait_for(&s, &s.closed, 1)) {
    return fail("closed");
  }
  ch->destroy(chan.ctx);
  if (!wait_for(&s, &s.destroyed, 1)) {
    return fail("the sink destroyed");
  }
  pause_ms(200);
  lock(&s.lock);
  int ok = !s.broken && s.closed == 1 && s.status == 5 && s.count == 3;
  const char *broken = s.broken;
  unlock(&s.lock);
  /* A host that has gone: its sink answers GONE, filling err. The echo
   * closes with GONE and sends no more; send then answers GONE. */
  seen g;
  memset(&g, 0, sizeof(g));
  lock_init(&g.lock);
  g.away = 1;
  guatiao_object gone_sink = {&SINK, sizeof(SINK), &g};
  guatiao_value gone_config;
  guatiao_value_map(&ALLOC, &gone_config);
  guatiao_object gone_chan;
  memset(&gone_chan, 0, sizeof(gone_chan));
  memset(&err, 0, sizeof(err));
  status = echo->start(echo_ctx, &gone_config.payload.map, gone_sink, &gone_chan, &err);
  guatiao_value_free(&gone_config);
  if (status != GUATIAO_OK) {
    return fail("start with a sink that has gone");
  }
  const channel_vtable *gc = (const channel_vtable *)gone_chan.table;
  if (gc->send(gone_chan.ctx, guatiao_bytes_from("x", 1), &err) != GUATIAO_OK) {
    return fail("the echo takes a message its sink then refuses");
  }
  if (!wait_for(&g, &g.closed, 1)) {
    return fail("closed after the sink answered GONE");
  }
  status = gc->send(gone_chan.ctx, guatiao_bytes_from("y", 1), &err);
  free_error(&err);
  if (status != GUATIAO_ERR_GONE) {
    return fail("a send after the sink answered GONE answers GUATIAO_ERR_GONE");
  }
  gc->destroy(gone_chan.ctx);
  if (!wait_for(&g, &g.destroyed, 1)) {
    return fail("the gone sink destroyed");
  }
  lock(&g.lock);
  int gone_ok = !g.broken && g.closed == 1 && g.status == GUATIAO_ERR_GONE && g.count == 0;
  unlock(&g.lock);
  if (!gone_ok) {
    return fail("closed(GUATIAO_ERR_GONE) once, nothing received");
  }
  printf("a sink that answered GONE: closed(GONE) once, send answers GONE\n");
  guatiao_registry_free(reg);
  if (!ok) {
    return fail(broken ? broken : "closed once, with the host's status");
  }
  printf("closed(%u) once, then destroyed\n", 5u);
  printf("all checks passed\n");
  return 0;
}
