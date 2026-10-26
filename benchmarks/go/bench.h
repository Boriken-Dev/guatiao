/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

/* The callback paths: a visitor whose callbacks are Go functions, and the
 * calls a Go walker makes to the visitor it is handed. */

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "guatiao_serde.h"

enum { SLOT_ABSENT, SLOT_NULL, SLOT_LIST_END, SLOT_MAP_END, SLOT_NUMBER, SLOT_STRING, SLOT_KEY, SLOT_LIST, SLOT_MAP };

guatiao_status bench_visit(void *fn, const char *text, size_t len, const guatiao_alloc *alloc);
guatiao_status bench_emit_walker(void *fn, const guatiao_alloc *alloc, guatiao_value *out);
guatiao_status bench_event(const guatiao_visitor *v, int slot);
guatiao_status bench_boolean(const guatiao_visitor *v, bool b);
guatiao_status bench_text(const guatiao_visitor *v, int slot, const char *p, size_t n);
guatiao_status bench_count(const guatiao_visitor *v, int slot, uint64_t n);
const char *bench_text_of(const guatiao_value *v, size_t *len);
void bench_release(void *free_fn, guatiao_value *v);
