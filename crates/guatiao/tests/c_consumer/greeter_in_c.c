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
 * - The map `greet` answers with travels WITH THIS FILE'S ALLOCATOR: the
 *   host frees it through the `guatiao_alloc` the value records, so no
 *   `free` here ever meets a block from over there. The allocator is the
 *   header's own malloc wrapper, `static inline`, nothing linked.
 *
 * Only `greet` is filled. `shout` and `start` are left null: to the host
 * that is an older table, and its proxy runs the trait's default bodies
 * -- `shout` through this file's `greet`.
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

/* An owned string: `cap > 0`, so the host frees it through ALLOC. */
static bool owned_string(guatiao_string *out, const char *a, guatiao_str b) {
  size_t la = strlen(a);
  size_t cap = la + b.len;
  uint8_t *buf = cap ? (uint8_t *)ALLOC.alloc(ALLOC.ctx, cap, 1) : NULL;
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
  out->alloc = &ALLOC;
  return true;
}

/* ---- the provider ------------------------------------------------------ */

static int64_t GREETED = 0;

static guatiao_status greet(void *ctx, guatiao_str name, guatiao_map *out,
                            guatiao_provider_error *err) {
  int64_t *greeted = (int64_t *)ctx;
  if (name.len == 0) {
    err->status = GUATIAO_ERR_BAD_VALUE;
    err->message.ptr = (uint8_t *)"nobody to greet, says C";
    err->message.len = strlen("nobody to greet, says C");
    err->message.cap = 0; /* a literal: never freed */
    err->message.alloc = NULL;
    return GUATIAO_ERR_BAD_VALUE;
  }

  /* One entry, in a block ALLOC owns, holding a string ALLOC owns. */
  guatiao_entry *entries = (guatiao_entry *)ALLOC.alloc(
      ALLOC.ctx, sizeof(guatiao_entry), _Alignof(guatiao_entry));
  if (!entries) {
    return GUATIAO_ERR_ALLOC;
  }
  guatiao_string key = GUATIAO_STRING_LIT("greeting");
  entries[0].key = key;
  entries[0].value.tag = (uint32_t)GUATIAO_STRING;
  entries[0].value._pad = 0;
  if (!owned_string(&entries[0].value.payload.text, "hello from C, ", name)) {
    ALLOC.free(ALLOC.ctx, entries, sizeof(guatiao_entry), _Alignof(guatiao_entry));
    return GUATIAO_ERR_ALLOC;
  }
  out->ptr = entries;
  out->len = 1;
  out->cap = 1;
  out->alloc = &ALLOC;
  (*greeted)++;
  return GUATIAO_OK;
}

/* The table: size and hash from the header, one slot filled. */
static const greeter_vtable TABLE = {
    {(uint32_t)sizeof(greeter_vtable), greeter_vtable_FLOOR_HASH},
    greet,
    NULL, /* shout: the host's default body runs, through `greet` */
    NULL, /* start: likewise -- this greeter holds no conversations */
};

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
