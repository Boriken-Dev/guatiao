/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * A LIBRARY whose table ignores the carrier: its one slot answers a map
 * built through this file's own allocator, whatever allocator the out
 * slot names. The tree then records an allocator whose code and data go
 * away with this library, so a caller that frees it after the unload
 * calls into nothing. `make_local_host.c` is that caller, and keeps the
 * answer by moving it home with `guatiao_value_make_local` first.
 *
 * It links nothing: the allocator is the header's malloc wrapper, whose
 * functions are compiled into this file.
 */

#include <string.h>

#include "guatiao.h"

#if defined(_WIN32)
#define EXPORT __declspec(dllexport)
#else
#define EXPORT __attribute__((visibility("default")))
#endif

static guatiao_alloc ALLOC = GUATIAO_ALLOC_MALLOC;

typedef struct carrier_ignored_table {
  guatiao_kind_header header;
  guatiao_status (*answer)(void *ctx, guatiao_value *out);
} carrier_ignored_table;

static bool owned_string(guatiao_string *out, const char *s) {
  size_t len = strlen(s);
  uint8_t *buf = (uint8_t *)ALLOC.alloc(ALLOC.ctx, len, 1);
  if (!buf) {
    return false;
  }
  memcpy(buf, s, len);
  out->ptr = buf;
  out->len = len;
  out->cap = len;
  out->alloc = &ALLOC;
  return true;
}

/* {"greeting": "hello"}, every block through ALLOC; the carrier's
 * allocator is never looked at. */
static guatiao_status answer(void *ctx, guatiao_value *out) {
  (void)ctx;
  guatiao_entry *entries = (guatiao_entry *)ALLOC.alloc(
      ALLOC.ctx, sizeof(guatiao_entry), _Alignof(guatiao_entry));
  if (!entries) {
    return GUATIAO_ERR_ALLOC;
  }
  entries[0].value.tag = (uint32_t)GUATIAO_STRING;
  entries[0].value._pad = 0;
  if (!owned_string(&entries[0].key, "greeting") ||
      !owned_string(&entries[0].value.payload.text, "hello")) {
    return GUATIAO_ERR_ALLOC;
  }
  out->tag = (uint32_t)GUATIAO_MAP;
  out->_pad = 0;
  out->payload.map.ptr = entries;
  out->payload.map.len = 1;
  out->payload.map.cap = 1;
  out->payload.map.alloc = &ALLOC;
  return GUATIAO_OK;
}

EXPORT const carrier_ignored_table CARRIER_IGNORED_TABLE = {
    {(uint32_t)sizeof(carrier_ignored_table), 0},
    answer,
};

/* The allocator the answer records, so the host can tell it moved. */
EXPORT const guatiao_alloc *carrier_ignored_alloc(void) { return &ALLOC; }
