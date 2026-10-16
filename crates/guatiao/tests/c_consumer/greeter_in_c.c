/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * A LIBRARY, written in C, offering a greeter.
 *
 * The claim: a kind declared as a Rust trait is a C ABI. This file fills
 * the table `greeter_kind.h` renders for that trait, offers it through a
 * guatiao_provider_info of its own, and exports the two symbols every
 * library exports -- and links NOTHING. The Rust host that loads it
 * (`tests/c_library.rs`) calls `greet` through the very proxy a Rust
 * library's table is called through, and cannot tell the difference.
 *
 * Two things are worth seeing here:
 *
 * - `header.floor_hash` is a macro the header carries. A table built
 *   from a different declaration of the kind hashes differently and is
 *   refused before anything in it runs.
 * - The map `greet` answers is built IN THE ALLOCATOR THE CALLER NAMED:
 *   the out slot arrives holding an empty map whose `alloc` is the host's,
 *   so the host keeps the answer as it is, with no copy. A caller that
 *   names none (an older host, or a zeroed slot) gets one built through
 *   this file's own allocator, the header's malloc wrapper, which the
 *   host copies from and frees through the `guatiao_alloc` it records.
 *
 * `greet` and `greet_into` are filled. `shout` and `start` are left null:
 * to the host that is an older table, and its proxy runs the trait's
 * default bodies -- `shout` through this file's `greet`.
 *
 * `greet_into` takes its listener LENT: a `const guatiao_object *` on the
 * caller's stack, valid for the call. This file calls the listener's
 * slots through it and neither keeps it nor calls its `destroy`. And
 * `c_greeter_lend_tape` is the other direction: C lending an object of
 * its own to a greeter's `greet_into`, whoever wrote the greeter.
 */

#include <stdlib.h>
#include <string.h>

#include "greeter_kind.h"
#include "guatiao.h"

#if defined(_WIN32)
#define EXPORT __declspec(dllexport)
#else
#define EXPORT __attribute__((visibility("default")))
#endif

/* ---- this library's allocator ------------------------------------------ */

static guatiao_alloc ALLOC = GUATIAO_ALLOC_MALLOC;

/* The allocator to build an answer in: the one the caller's carrier names
 * -- an empty map whose `alloc` is set -- or this file's own. */
static const guatiao_alloc *answer_alloc(const guatiao_map *out) {
  const guatiao_alloc *a = out ? out->alloc : NULL;
  if (out && out->len == 0 && out->cap == 0 && a &&
      a->struct_size >= offsetof(guatiao_alloc, free) + sizeof(a->free) &&
      a->alloc && a->free) {
    return a;
  }
  return &ALLOC;
}

/* An owned string, `a` then `b`, through `alloc`: `cap > 0`, so the host
 * frees it through the allocator it records. */
static bool owned_string(guatiao_string *out, const guatiao_alloc *alloc,
                         const char *a, guatiao_str b) {
  size_t la = strlen(a);
  size_t cap = la + b.len;
  uint8_t *buf = cap ? (uint8_t *)alloc->alloc(alloc->ctx, cap, 1) : NULL;
  if (cap && !buf) {
    return false;
  }
  if (la) {
    memcpy(buf, a, la);
  }
  if (b.len) {
    memcpy(buf + la, b.ptr, b.len);
  }
  out->ptr = buf;
  out->len = cap;
  out->cap = cap;
  out->alloc = alloc;
  return true;
}

static void free_string(guatiao_string *s) {
  if (s->cap) {
    s->alloc->free(s->alloc->ctx, s->ptr, s->cap, 1);
  }
}

/* The entries `greet` built last, so the host can tell its answer is the
 * very block built here and not a copy. */
static const guatiao_entry *LAST_BUILT = NULL;

EXPORT const guatiao_entry *c_greeter_last_built(void) { return LAST_BUILT; }

/* ---- the provider ------------------------------------------------------ */

/* A refusal whose message is a literal: never freed. */
static guatiao_status refuse(guatiao_provider_error *err, guatiao_status status,
                             const char *why) {
  err->status = status;
  err->message.ptr = (uint8_t *)why;
  err->message.len = strlen(why);
  err->message.cap = 0;
  err->message.alloc = NULL;
  return status;
}

static int64_t GREETED = 0;

static guatiao_status greet(void *ctx, guatiao_str name, guatiao_map *out,
                            guatiao_provider_error *err) {
  int64_t *greeted = (int64_t *)ctx;
  if (name.len == 0) {
    return refuse(err, GUATIAO_ERR_BAD_VALUE, "nobody to greet, says C");
  }

  /* One entry, its key and its string, all through one allocator. */
  const guatiao_alloc *alloc = answer_alloc(out);
  guatiao_entry *entries = (guatiao_entry *)alloc->alloc(
      alloc->ctx, sizeof(guatiao_entry), _Alignof(guatiao_entry));
  if (!entries) {
    return GUATIAO_ERR_ALLOC;
  }
  guatiao_str none = {NULL, 0};
  if (!owned_string(&entries[0].key, alloc, "greeting", none)) {
    alloc->free(alloc->ctx, entries, sizeof(guatiao_entry), _Alignof(guatiao_entry));
    return GUATIAO_ERR_ALLOC;
  }
  entries[0].value.tag = (uint32_t)GUATIAO_STRING;
  entries[0].value._pad = 0;
  if (!owned_string(&entries[0].value.payload.text, alloc, "hello from C, ", name)) {
    free_string(&entries[0].key);
    alloc->free(alloc->ctx, entries, sizeof(guatiao_entry), _Alignof(guatiao_entry));
    return GUATIAO_ERR_ALLOC;
  }
  /* Written over the carrier, which owns nothing. */
  out->ptr = entries;
  out->len = 1;
  out->cap = 1;
  out->alloc = alloc;
  LAST_BUILT = entries;
  (*greeted)++;
  return GUATIAO_OK;
}

/* The greeting, told to a listener the caller lent for this call. */
static guatiao_status greet_into(void *ctx, guatiao_str name,
                                 const guatiao_object *listener,
                                 guatiao_provider_error *err) {
  (void)ctx;
  if (!listener || !listener->table ||
      listener->size < sizeof(listener_vtable)) {
    return refuse(err, GUATIAO_ERR_WRONG_KIND, "no listener was lent, says C");
  }
  const listener_vtable *table = (const listener_vtable *)listener->table;
  if (table->header.floor_hash != listener_vtable_FLOOR_HASH || !table->heard) {
    return refuse(err, GUATIAO_ERR_WRONG_KIND, "not a listener, says C");
  }
  /* Built on this stack: the listener reads it during the call only. */
  uint8_t buf[64];
  const char *hello = "hello from C, ";
  size_t len = strlen(hello);
  memcpy(buf, hello, len);
  size_t room = sizeof(buf) - len;
  size_t n = name.len < room ? name.len : room;
  if (n) {
    memcpy(buf + len, name.ptr, n);
  }
  guatiao_str what = {buf, len + n};
  return table->heard(listener->ctx, what);
}

/* The table: size and hash from the header, two slots filled. */
static const greeter_vtable TABLE = {
    {(uint32_t)sizeof(greeter_vtable), greeter_vtable_FLOOR_HASH},
    greet,
    NULL, /* shout: the host's default body runs, through `greet` */
    NULL, /* start: likewise -- this greeter holds no conversations */
    greet_into,
};

/* ---- C lending an object of its own ------------------------------------ */

/* What the test reads back: laid out as the Rust test declares it. */
typedef struct tape {
  int64_t heard;
  int64_t destroyed;
  size_t len;
  uint8_t last[32];
} tape;

/* Never called on a lent object; counted, so a test can say so. */
static void tape_destroy(void *ctx) { ((tape *)ctx)->destroyed++; }

static guatiao_status tape_heard(void *ctx, guatiao_str what) {
  tape *t = (tape *)ctx;
  t->heard++;
  t->len = what.len < sizeof(t->last) ? what.len : sizeof(t->last);
  if (t->len) {
    memcpy(t->last, what.ptr, t->len);
  }
  return GUATIAO_OK;
}

static const listener_vtable TAPE = {
    {(uint32_t)sizeof(listener_vtable), listener_vtable_FLOOR_HASH},
    tape_destroy,
    tape_heard,
};

/* Calls a greeter's `greet_into` -- any greeter's table, Rust's included
 * -- lending `t` as the listener. The object lives on this stack. */
EXPORT guatiao_status c_greeter_lend_tape(const greeter_vtable *greeter,
                                          void *ctx, guatiao_str name,
                                          tape *t) {
  if (!greeter || greeter->header.struct_size < sizeof(greeter_vtable) ||
      !greeter->greet_into) {
    return GUATIAO_ERR_NULL;
  }
  guatiao_object lent = {&TAPE, sizeof(listener_vtable), t};
  guatiao_provider_error err;
  memset(&err, 0, sizeof(err));
  guatiao_status status = greeter->greet_into(ctx, name, &lent, &err);
  if (err.message.cap && err.message.alloc) {
    err.message.alloc->free(err.message.alloc->ctx, err.message.ptr,
                            err.message.cap, 1);
  }
  return status;
}

static const guatiao_str KINDS[] = {{(const uint8_t *)"greeter", 7}};

static const guatiao_kind_table TABLES[] = {{
    (uint32_t)sizeof(guatiao_kind_table),
    (uint32_t)sizeof(greeter_vtable),
    {(const uint8_t *)"greeter", 7},
    &TABLE,
}};

static guatiao_provider_info PROVIDER;
static guatiao_library_info LIBRARY;

/* ---- the envelope ------------------------------------------------------ */

/* What a scanner reads before mapping this file. */
EXPORT const char guatiao_declares[] = "kind=greeter\0";

EXPORT const guatiao_library_info *
guatiao_library_entry(const guatiao_host_info *host) {
  if (!host || host->abi_version != GUATIAO_ABI_VERSION) {
    return NULL; /* decline a host this file was not built for */
  }
  memset(&PROVIDER, 0, sizeof(PROVIDER));
  PROVIDER.struct_size = (uint32_t)sizeof(guatiao_provider_info);
  PROVIDER.kinds.ptr = KINDS;
  PROVIDER.kinds.len = 1;
  PROVIDER.id = guatiao_cstr("c_greeter");
  PROVIDER.display_name = guatiao_cstr("Greeter, in C");
  PROVIDER.ctx = &GREETED;
  PROVIDER.version = guatiao_cstr("1.0");
  PROVIDER.tables.ptr = TABLES;
  PROVIDER.tables.len = 1;
  PROVIDER.tables.stride = sizeof(guatiao_kind_table);

  memset(&LIBRARY, 0, sizeof(LIBRARY));
  LIBRARY.struct_size = (uint32_t)sizeof(guatiao_library_info);
  LIBRARY.abi_version = GUATIAO_ABI_VERSION;
  LIBRARY.id = guatiao_cstr("c_greeter_library");
  LIBRARY.version = guatiao_cstr("1.0");
  LIBRARY.providers.ptr = &PROVIDER;
  LIBRARY.providers.len = 1;
  LIBRARY.providers.stride = sizeof(guatiao_provider_info);
  return &LIBRARY;
}
