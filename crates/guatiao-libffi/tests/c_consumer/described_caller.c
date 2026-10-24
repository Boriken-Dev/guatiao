/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * Calls a greeter's and a wide kind's tables as a C host does: each slot
 * through its typed pointer, as greeter_kind.h declares it. Links nothing:
 * the trees are the caller's, which reads and frees them after.
 */

#include <stdbool.h>
#include <stdint.h>
#include <string.h>

#include <guatiao.h>
#include <greeter_kind.h>

#ifdef _WIN32
#define EXPORT __declspec(dllexport)
#else
#define EXPORT __attribute__((visibility("default")))
#endif

EXPORT guatiao_status drive_greet(const greeter_vtable *t, void *ctx, const char *name,
                                  guatiao_map *out, guatiao_provider_error *err) {
  guatiao_str s = {(const uint8_t *)name, strlen(name)};
  return t->greet(ctx, s, out, err);
}

EXPORT guatiao_status drive_check(const greeter_vtable *t, void *ctx, guatiao_map *lent,
                                  bool *out, guatiao_provider_error *err) {
  return t->check(ctx, lent, out, err);
}

/* Whether a slot the table leaves out is null, as a C caller tests it. */
EXPORT int drive_shout_is_null(const greeter_vtable *t) { return t->shout == NULL; }

EXPORT int drive_wide(const wide_vtable *t, void *ctx) {
  int64_t i = 0;
  uint64_t u = 0;
  guatiao_provider_error err;
  memset(&err, 0, sizeof err);
  if (t->echo_i64(ctx, INT64_MIN, &i, &err) != 0 || i != INT64_MIN) {
    return 1;
  }
  if (t->echo_u64(ctx, UINT64_MAX, &u, &err) != 0 || u != UINT64_MAX) {
    return 2;
  }
  return 0;
}
