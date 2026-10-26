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
 * `greet_from` takes its configuration HANDED OVER: a `guatiao_map *`
 * without `const` is a tree this file owns from the call on, built in the
 * caller's allocator. It keeps the last one, and `c_greeter_take_kept`
 * hands it back, so the host can see it is the very map it passed.
 * `c_greeter_hand_over` is the other direction: C handing a map built
 * through this file's allocator to the greeter a host files under a key.
 *
 * `check` takes its configuration LENT: a `guatiao_map *` the slot's
 * comment calls lent is the caller's map for this call only. It adds an
 * entry, growing the map through the allocator the map records, and
 * neither frees it nor keeps it. `c_greeter_check_lending` is the other
 * direction: C lending a map of its own to any greeter's `check`. Built
 * with GREETER_IGNORES_CARRIER, `check` instead rebuilds the map in this
 * file's allocator, which the map then records, as a careless library
 * writing a fresh map over a lent one does.
 *
 * `greet`, `greet_into`, `greet_from` and `check` are filled. `shout` and `start` are left null:
 * to the host that is an older table, and its proxy runs the trait's
 * default bodies -- `shout` through this file's `greet`.
 *
 * `greet_into` takes its listener LENT: a `const guatiao_object *` on the
 * caller's stack, valid for the call. This file calls the listener's
 * slots through it and neither keeps it nor calls its `destroy`. And
 * `c_greeter_lend_tape` is the other direction: C lending an object of
 * its own to a greeter's `greet_into`, whoever wrote the greeter.
 *
 * The same provider serves the `wide` kind too: a second table, whose
 * slots answer the 64-bit integer they are given.
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
 * -- an empty map whose `alloc` is set -- or this file's own. Built with
 * GREETER_IGNORES_CARRIER, always this file's own, as an older or careless
 * library does: a caller then moves the answer home before the unload. */
static const guatiao_alloc *answer_alloc(const guatiao_map *out) {
#if defined(GREETER_IGNORES_CARRIER)
  (void)out;
#else
  const guatiao_alloc *a = out ? out->alloc : NULL;
  if (out && out->len == 0 && out->cap == 0 && a &&
      a->struct_size >= offsetof(guatiao_alloc, free) + sizeof(a->free) &&
      a->alloc && a->free) {
    return a;
  }
#endif
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

/* The configuration `greet_from` was handed last. */
static guatiao_map KEPT;
static bool HAS_KEPT = false;

/* Frees a map of text through the allocators it records: what this file
 * is handed is a flat configuration, so a nested container is not
 * walked. */
static void free_flat_map(guatiao_map *m) {
  for (size_t i = 0; i < m->len; i++) {
    guatiao_entry *e = &m->ptr[i];
    free_string(&e->key);
    if (e->value.tag == (uint32_t)GUATIAO_STRING ||
        e->value.tag == (uint32_t)GUATIAO_NUMBER) {
      free_string(&e->value.payload.text);
    }
  }
  if (m->cap) {
    m->alloc->free(m->alloc->ctx, m->ptr, m->cap * sizeof(guatiao_entry),
                   _Alignof(guatiao_entry));
  }
}

/* The greeting for `{"name": ..}`. The map is this file's from the call
 * on, on every path: kept, the one kept before it freed. */
static guatiao_status greet_from(void *ctx, guatiao_map *config, guatiao_map *out,
                                 guatiao_provider_error *err) {
  if (!config) {
    return refuse(err, GUATIAO_ERR_NULL, "no configuration, says C");
  }
  guatiao_str name = {NULL, 0};
  for (size_t i = 0; i < config->len; i++) {
    const guatiao_entry *e = &config->ptr[i];
    if (e->key.len == 4 && memcmp(e->key.ptr, "name", 4) == 0 &&
        e->value.tag == (uint32_t)GUATIAO_STRING) {
      name.ptr = e->value.payload.text.ptr;
      name.len = e->value.payload.text.len;
    }
  }
  guatiao_status status = greet(ctx, name, out, err);
  if (HAS_KEPT) {
    free_flat_map(&KEPT);
  }
  KEPT = *config;
  HAS_KEPT = true;
  return status;
}

/* Hands the kept configuration back to the caller, who owns it again. */
EXPORT bool c_greeter_take_kept(guatiao_map *out) {
  if (!HAS_KEPT || !out) {
    return false;
  }
  *out = KEPT;
  HAS_KEPT = false;
  return true;
}

/* Adds `"checked_by": "C"` to a map the caller lent for this call. The
 * array grows and the key is built through the allocator the map
 * records, as any growth of it must be; the value is built through this
 * file's own, and a Rust caller moves it into the map's once the call
 * returns. Built with GREETER_IGNORES_CARRIER, the array moves into this
 * file's allocator and the map records it, key and all: a caller then
 * moves the map back into the allocator it recorded before the call.
 * This file links nothing; a C library that links guatiao would call
 * `guatiao_map_set` on a value wrapping the map instead. */
static guatiao_status check(void *ctx, guatiao_map *config, bool *out,
                            guatiao_provider_error *err) {
  (void)ctx;
  if (!config) {
    return refuse(err, GUATIAO_ERR_NULL, "no configuration, says C");
  }
  const guatiao_alloc *a = config->alloc;
  if (!a || a->struct_size < offsetof(guatiao_alloc, free) + sizeof(a->free) ||
      !a->alloc || !a->free) {
    return refuse(err, GUATIAO_ERR_ALLOC, "the map records no allocator, says C");
  }
#if defined(GREETER_IGNORES_CARRIER)
  {
    size_t cap = config->len + 4;
    guatiao_entry *fresh = (guatiao_entry *)ALLOC.alloc(
        ALLOC.ctx, cap * sizeof(guatiao_entry), _Alignof(guatiao_entry));
    if (!fresh) {
      return GUATIAO_ERR_ALLOC;
    }
    if (config->len) {
      memcpy(fresh, config->ptr, config->len * sizeof(guatiao_entry));
    }
    if (config->cap) {
      a->free(a->ctx, config->ptr, config->cap * sizeof(guatiao_entry),
              _Alignof(guatiao_entry));
    }
    config->ptr = fresh;
    config->cap = cap;
    config->alloc = &ALLOC;
    a = &ALLOC;
  }
#else
  if (config->len >= config->cap) {
    /* `cap == 0` is storage the map does not own: copied out, never freed. */
    size_t cap = config->len < 2 ? 4 : config->len * 2;
    guatiao_entry *grown = (guatiao_entry *)a->alloc(
        a->ctx, cap * sizeof(guatiao_entry), _Alignof(guatiao_entry));
    if (!grown) {
      return GUATIAO_ERR_ALLOC;
    }
    if (config->len) {
      memcpy(grown, config->ptr, config->len * sizeof(guatiao_entry));
    }
    if (config->cap) {
      a->free(a->ctx, config->ptr, config->cap * sizeof(guatiao_entry),
              _Alignof(guatiao_entry));
    }
    config->ptr = grown;
    config->cap = cap;
  }
#endif
  guatiao_entry *e = &config->ptr[config->len];
  guatiao_str none = {NULL, 0};
  guatiao_str by = {(const uint8_t *)"C", 1};
  if (!owned_string(&e->key, a, "checked_by", none)) {
    return GUATIAO_ERR_ALLOC;
  }
  e->value.tag = (uint32_t)GUATIAO_STRING;
  e->value._pad = 0;
  if (!owned_string(&e->value.payload.text, &ALLOC, "", by)) {
    free_string(&e->key);
    return GUATIAO_ERR_ALLOC;
  }
  config->len++;
  *out = true;
  return GUATIAO_OK;
}

/* The table: size and hash from the header, four slots filled. */
static const greeter_vtable TABLE = {
    {(uint32_t)sizeof(greeter_vtable), greeter_vtable_FLOOR_HASH},
    greet,
    NULL, /* shout: the host's default body runs, through `greet` */
    NULL, /* start: likewise -- this greeter holds no conversations */
    greet_into,
    greet_from,
    check,
};

/* ---- the wide kind ----------------------------------------------------- */

static guatiao_status echo_i64(void *ctx, int64_t x, int64_t *out,
                               guatiao_provider_error *err) {
  (void)ctx;
  (void)err;
  *out = x;
  return GUATIAO_OK;
}

static guatiao_status echo_u64(void *ctx, uint64_t x, uint64_t *out,
                               guatiao_provider_error *err) {
  (void)ctx;
  (void)err;
  *out = x;
  return GUATIAO_OK;
}

static const wide_vtable WIDE = {
    {(uint32_t)sizeof(wide_vtable), wide_vtable_FLOOR_HASH},
    echo_i64,
    echo_u64,
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

/* ---- C lending a map of its own ---------------------------------------- */

/* Calls a greeter's `check` -- any greeter's table, Rust's included --
 * lending `{"name": "cy"}`, a map built through this file's allocator.
 * Reports how many entries it holds after the call, and whether every
 * one of them, and the map, still records this file's allocator: what a
 * Rust callee adds through the allocator the map records. Then frees it. */
EXPORT guatiao_status c_greeter_check_lending(const greeter_vtable *greeter,
                                              void *ctx, size_t *len,
                                              bool *all_mine) {
  if (!greeter || greeter->header.struct_size < sizeof(greeter_vtable) ||
      !greeter->check) {
    return GUATIAO_ERR_NULL;
  }
  guatiao_map config = {NULL, 0, 0, &ALLOC};
  guatiao_entry *entries = (guatiao_entry *)ALLOC.alloc(
      ALLOC.ctx, sizeof(guatiao_entry), _Alignof(guatiao_entry));
  if (!entries) {
    return GUATIAO_ERR_ALLOC;
  }
  guatiao_str none = {NULL, 0};
  guatiao_str cy = {(const uint8_t *)"cy", 2};
  if (!owned_string(&entries[0].key, &ALLOC, "name", none)) {
    ALLOC.free(ALLOC.ctx, entries, sizeof(guatiao_entry), _Alignof(guatiao_entry));
    return GUATIAO_ERR_ALLOC;
  }
  entries[0].value.tag = (uint32_t)GUATIAO_STRING;
  entries[0].value._pad = 0;
  if (!owned_string(&entries[0].value.payload.text, &ALLOC, "", cy)) {
    free_string(&entries[0].key);
    ALLOC.free(ALLOC.ctx, entries, sizeof(guatiao_entry), _Alignof(guatiao_entry));
    return GUATIAO_ERR_ALLOC;
  }
  config.ptr = entries;
  config.len = 1;
  config.cap = 1;

  bool usable = false;
  guatiao_provider_error err;
  memset(&err, 0, sizeof(err));
  guatiao_status status = greeter->check(ctx, &config, &usable, &err);
  if (err.message.cap && err.message.alloc) {
    err.message.alloc->free(err.message.alloc->ctx, err.message.ptr,
                            err.message.cap, 1);
  }
  bool mine = usable && config.alloc == &ALLOC;
  for (size_t i = 0; i < config.len; i++) {
    const guatiao_entry *e = &config.ptr[i];
    mine = mine && e->key.alloc == &ALLOC;
    if (e->value.tag == (uint32_t)GUATIAO_STRING) {
      mine = mine && e->value.payload.text.alloc == &ALLOC;
    }
  }
  *len = config.len;
  *all_mine = mine;
  free_flat_map(&config);
  return status;
}

/* ---- C handing a map of its own over ----------------------------------- */

/* Whether `s` is the kind name `greeter`. */
static bool is_greeter(guatiao_str s) {
  return s.len == 7 && memcmp(s.ptr, "greeter", 7) == 0;
}

/* The greeter table a provider descriptor carries, with `greet_from`
 * inside it: a table for the kind first, then `vtable` when `kinds`
 * names the kind; else null. */
static const greeter_vtable *greeter_of(const guatiao_provider_info *d) {
  const size_t need = offsetof(greeter_vtable, greet_from) + sizeof(void *);
  if (d->struct_size >= offsetof(guatiao_provider_info, tables) + sizeof(d->tables)) {
    for (size_t i = 0; i < d->tables.len; i++) {
      const guatiao_kind_table *t =
          (const guatiao_kind_table *)((const char *)d->tables.ptr + i * d->tables.stride);
      if (is_greeter(t->kind)) {
        return t->vtable_size >= need ? (const greeter_vtable *)t->vtable : NULL;
      }
    }
  }
  for (size_t i = 0; i < d->kinds.len; i++) {
    if (is_greeter(d->kinds.ptr[i])) {
      return d->vtable_size >= need ? (const greeter_vtable *)d->vtable : NULL;
    }
  }
  return NULL;
}

/* Finds the greeter filed under `key` through the host's services and
 * hands `{"name": name}`, a map built through this file's allocator, to
 * its `greet_from`. A callee that keeps the map holds blocks of this
 * file's allocator, which goes with this library, unless it moved them
 * into its own. The answer, a map of strings, is freed here. */
EXPORT guatiao_status c_greeter_hand_over(const guatiao_host_info *host,
                                          guatiao_str key, guatiao_str name) {
  if (!host ||
      host->struct_size < offsetof(guatiao_host_info, services) + sizeof(host->services) ||
      !host->services || !host->services->get) {
    return GUATIAO_ERR_NULL;
  }
  const guatiao_host_services *s = host->services;
  bool pinned = s->struct_size >= offsetof(guatiao_host_services, unpin) + sizeof(s->unpin) &&
                s->get_pinned && s->unpin;
  const guatiao_provider_info *desc = NULL;
  guatiao_status status = pinned ? s->get_pinned(s->ctx, key, &desc) : s->get(s->ctx, key, &desc);
  if (status != GUATIAO_OK) {
    return status;
  }
  const greeter_vtable *greeter = greeter_of(desc);
  if (!greeter || !greeter->greet_from) {
    status = GUATIAO_ERR_UNSUPPORTED;
  } else {
    guatiao_map config = {NULL, 0, 0, &ALLOC};
    guatiao_entry *entry = (guatiao_entry *)ALLOC.alloc(ALLOC.ctx, sizeof(guatiao_entry),
                                                        _Alignof(guatiao_entry));
    guatiao_str none = {NULL, 0};
    if (!entry) {
      status = GUATIAO_ERR_ALLOC;
    } else if (!owned_string(&entry->key, &ALLOC, "name", none)) {
      ALLOC.free(ALLOC.ctx, entry, sizeof(guatiao_entry), _Alignof(guatiao_entry));
      status = GUATIAO_ERR_ALLOC;
    } else {
      entry->value.tag = (uint32_t)GUATIAO_STRING;
      entry->value._pad = 0;
      if (!owned_string(&entry->value.payload.text, &ALLOC, "", name)) {
        free_string(&entry->key);
        ALLOC.free(ALLOC.ctx, entry, sizeof(guatiao_entry), _Alignof(guatiao_entry));
        status = GUATIAO_ERR_ALLOC;
      } else {
        config.ptr = entry;
        config.len = 1;
        config.cap = 1;
        guatiao_map out = {NULL, 0, 0, NULL};
        guatiao_provider_error err;
        memset(&err, 0, sizeof(err));
        /* Handed over: the slot owns `config` from here, on every path. */
        status = greeter->greet_from(desc->ctx, &config, &out, &err);
        if (err.message.cap && err.message.alloc) {
          err.message.alloc->free(err.message.alloc->ctx, err.message.ptr,
                                  err.message.cap, 1);
        }
        if (status == GUATIAO_OK) {
          free_flat_map(&out);
        }
      }
    }
  }
  if (pinned) {
    s->unpin(s->ctx, desc);
  }
  return status;
}

static const guatiao_str KINDS[] = {{(const uint8_t *)"greeter", 7},
                                     {(const uint8_t *)"wide", 4}};

static const guatiao_kind_table TABLES[] = {{
    (uint32_t)sizeof(guatiao_kind_table),
    (uint32_t)sizeof(greeter_vtable),
    {(const uint8_t *)"greeter", 7},
    &TABLE,
}, {
    (uint32_t)sizeof(guatiao_kind_table),
    (uint32_t)sizeof(wide_vtable),
    {(const uint8_t *)"wide", 4},
    &WIDE,
}};

static guatiao_provider_info PROVIDER;
static guatiao_library_info LIBRARY;

/* ---- the envelope ------------------------------------------------------ */

/* What a scanner reads before mapping this file. */
EXPORT const char guatiao_declares[] = "kind=greeter\0kind=wide\0";

EXPORT const guatiao_library_info *
guatiao_library_entry(const guatiao_host_info *host) {
  if (!host || host->abi_version != GUATIAO_ABI_VERSION) {
    return NULL; /* decline a host this file was not built for */
  }
  memset(&PROVIDER, 0, sizeof(PROVIDER));
  PROVIDER.struct_size = (uint32_t)sizeof(guatiao_provider_info);
  PROVIDER.kinds.ptr = KINDS;
  PROVIDER.kinds.len = 2;
  PROVIDER.id = guatiao_cstr("c_greeter");
  PROVIDER.display_name = guatiao_cstr("Greeter, in C");
  PROVIDER.ctx = &GREETED;
  PROVIDER.version = guatiao_cstr("1.0");
  PROVIDER.tables.ptr = TABLES;
  PROVIDER.tables.len = 2;
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
