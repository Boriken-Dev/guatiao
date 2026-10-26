/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

#include "bench.h"

#include <stdlib.h>

#include "_cgo_export.h"

typedef guatiao_status (*visit_fn)(guatiao_str, uint32_t, const guatiao_alloc *, const guatiao_visitor *,
                                   guatiao_parse_error *);
typedef guatiao_status (*emit_walker_fn)(const guatiao_walker *, uint32_t, const guatiao_alloc *, guatiao_value *);

static guatiao_status on_absent(void *ctx) { return goAbsent(ctx); }
static guatiao_status on_null(void *ctx) { return goNull(ctx); }
static guatiao_status on_bool(void *ctx, bool b) { return goBool(ctx, b); }
static guatiao_status on_number(void *ctx, guatiao_str s) { return goNumber(ctx, (char *)s.ptr, s.len); }
static guatiao_status on_string(void *ctx, guatiao_str s) { return goString(ctx, (char *)s.ptr, s.len); }
static guatiao_status on_bytes(void *ctx, guatiao_bytes b) { return goBytes(ctx, (char *)b.ptr, b.len); }
static guatiao_status on_list_begin(void *ctx, uint64_t n) { return goListBegin(ctx, n); }
static guatiao_status on_list_end(void *ctx) { return goEnd(ctx); }
static guatiao_status on_map_begin(void *ctx, uint64_t n) { return goMapBegin(ctx, n); }
static guatiao_status on_key(void *ctx, guatiao_str s) { return goKey(ctx, (char *)s.ptr, s.len); }
static guatiao_status on_map_end(void *ctx) { return goEnd(ctx); }

guatiao_status bench_visit(void *fn, const char *text, size_t len, const guatiao_alloc *alloc) {
  guatiao_visitor v = {sizeof v,      0,         on_absent,  on_null,     on_bool, on_number,
                       on_string,     on_bytes,  on_list_begin, on_list_end, on_map_begin, on_key,
                       on_map_end};
  guatiao_str s = {(const uint8_t *)text, len};
  return ((visit_fn)fn)(s, 0, alloc, &v, 0);
}

static guatiao_status on_walk(void *ctx, const guatiao_visitor *into) { return goWalk(ctx, (void *)into); }

guatiao_status bench_emit_walker(void *fn, const guatiao_alloc *alloc, guatiao_value *out) {
  guatiao_walker w = {sizeof w, 0, on_walk};
  return ((emit_walker_fn)fn)(&w, 0, alloc, out);
}

guatiao_status bench_event(const guatiao_visitor *v, int slot) {
  switch (slot) {
  case SLOT_ABSENT: return v->absent(v->ctx);
  case SLOT_NULL: return v->null(v->ctx);
  case SLOT_LIST_END: return v->list_end(v->ctx);
  default: return v->map_end(v->ctx);
  }
}

guatiao_status bench_boolean(const guatiao_visitor *v, bool b) { return v->boolean(v->ctx, b); }

guatiao_status bench_text(const guatiao_visitor *v, int slot, const char *p, size_t n) {
  guatiao_str s = {(const uint8_t *)p, n};
  switch (slot) {
  case SLOT_NUMBER: return v->number(v->ctx, s);
  case SLOT_STRING: return v->string(v->ctx, s);
  default: return v->key(v->ctx, s);
  }
}

guatiao_status bench_count(const guatiao_visitor *v, int slot, uint64_t n) {
  return slot == SLOT_LIST ? v->list_begin(v->ctx, n) : v->map_begin(v->ctx, n);
}

const char *bench_text_of(const guatiao_value *v, size_t *len) {
  *len = v->tag == GUATIAO_STRING ? v->payload.text.len : 0;
  return v->tag == GUATIAO_STRING ? (const char *)v->payload.text.ptr : "";
}

void bench_release(void *free_fn, guatiao_value *v) {
  ((guatiao_status(*)(guatiao_value *))free_fn)(v);
  free(v);
}
