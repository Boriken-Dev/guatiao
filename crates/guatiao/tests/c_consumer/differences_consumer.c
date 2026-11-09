/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * Where a value differs from its schema's defaults or from a baseline,
 * from C: every case of tests/vectors/differences.json through
 * guatiao_schema_differences, each result checked for the keys it lists,
 * in order, and which sides carry a value.
 *
 * `differences_vectors.h` is generated from the vector file by the test
 * that compiles this file; the trees travel as their wire bytes in hex.
 *
 * Prints "all checks passed" on success. Any failure returns non-zero
 * with a line saying which vector and what it saw.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "guatiao.h"
#include "differences_vectors.h"

static int failures = 0;
static size_t checked = 0;

#define CHECK(cond, ...)                          \
  do {                                            \
    if (!(cond)) {                                \
      printf("FAIL %s:%d: ", __FILE__, __LINE__); \
      printf(__VA_ARGS__);                        \
      printf("\n");                               \
      failures++;                                 \
    }                                             \
  } while (0)

static guatiao_alloc ALLOC = GUATIAO_ALLOC_MALLOC;

static int nibble(char c) {
  return c >= 'a' ? c - 'a' + 10 : c - '0';
}

/* The tree whose wire bytes `hex` spells. */
static int decode(const char *hex, guatiao_value *out) {
  size_t n = strlen(hex) / 2;
  size_t i;
  uint8_t *bytes = malloc(n ? n : 1);
  guatiao_status status;
  if (bytes == NULL) {
    exit(2);
  }
  for (i = 0; i < n; i++) {
    bytes[i] = (uint8_t)(nibble(hex[2 * i]) << 4 | nibble(hex[2 * i + 1]));
  }
  status = guatiao_wire_decode(&ALLOC, guatiao_bytes_from(bytes, n), out);
  free(bytes);
  return status == GUATIAO_OK;
}

static int is_text(const guatiao_value *v, const char *expected) {
  guatiao_str s;
  if (v == NULL || guatiao_tag_of(v) != GUATIAO_STRING) {
    return 0;
  }
  s = guatiao_string_text(v);
  return s.len == strlen(expected) && (s.len == 0 || memcmp(s.ptr, expected, s.len) == 0);
}

static void run_vector(const diff_vector *c) {
  guatiao_value schema, value, baseline, out;
  guatiao_values items;
  guatiao_status status;
  size_t i;
  int has_baseline = c->baseline != NULL;

  CHECK(decode(c->schema, &schema), "%s: schema", c->name);
  CHECK(decode(c->value, &value), "%s: value", c->name);
  if (has_baseline) {
    CHECK(decode(c->baseline, &baseline), "%s: baseline", c->name);
  }
  status = guatiao_schema_differences(&schema, &value, has_baseline ? &baseline : NULL,
                                      guatiao_cstr("***"), &ALLOC, &out);
  CHECK(status == GUATIAO_OK, "%s: status %d", c->name, (int)status);
  if (status == GUATIAO_OK) {
    CHECK(guatiao_tag_of(&out) == GUATIAO_LIST, "%s: a list", c->name);
    items = guatiao_list_items(&out);
    CHECK(items.len == c->count, "%s: %zu differences, expected %zu", c->name, items.len,
          c->count);
    for (i = 0; i < items.len && i < c->count; i++) {
      const guatiao_value *entry = &items.ptr[i];
      const guatiao_value *path = guatiao_map_find(entry, guatiao_cstr("path"));
      CHECK(is_text(guatiao_map_find(entry, guatiao_cstr("key")), c->keys[i]),
            "%s: key %zu is not %s", c->name, i, c->keys[i]);
      CHECK(path != NULL && guatiao_tag_of(path) == GUATIAO_LIST, "%s: path %zu", c->name, i);
      CHECK((guatiao_map_find(entry, guatiao_cstr("value")) != NULL) == ((c->sides[i] & 1) != 0),
            "%s: value side of %s", c->name, c->keys[i]);
      CHECK((guatiao_map_find(entry, guatiao_cstr("default")) != NULL) == ((c->sides[i] & 2) != 0),
            "%s: default side of %s", c->name, c->keys[i]);
    }
    guatiao_value_free(&out);
  }
  guatiao_value_free(&schema);
  guatiao_value_free(&value);
  if (has_baseline) {
    guatiao_value_free(&baseline);
  }
  checked++;
}

static void refusals(void) {
  guatiao_value schema, value, out;
  CHECK(decode(DIFF_VECTORS[0].schema, &schema), "schema");
  CHECK(decode(DIFF_VECTORS[0].value, &value), "value");
  CHECK(guatiao_schema_differences(NULL, &value, NULL, guatiao_cstr("*"), &ALLOC, &out) ==
            GUATIAO_ERR_NULL,
        "a null schema");
  CHECK(guatiao_schema_differences(&schema, &value, NULL, guatiao_cstr("*"), NULL, &out) ==
            GUATIAO_ERR_ALLOC,
        "a null allocator");
  CHECK(guatiao_tag_of(&out) == GUATIAO_ABSENT, "out stays ABSENT");
  guatiao_value_free(&schema);
  guatiao_value_free(&value);
}

int main(void) {
  size_t i;
  for (i = 0; i < DIFF_COUNT; i++) {
    run_vector(&DIFF_VECTORS[i]);
  }
  refusals();
  if (failures != 0) {
    printf("%d checks failed\n", failures);
    return 1;
  }
  printf("all checks passed (%zu vectors)\n", checked);
  return 0;
}
