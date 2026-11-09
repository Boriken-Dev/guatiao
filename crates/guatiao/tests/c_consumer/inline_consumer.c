/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * A value as one line of text, from C: every case of
 * tests/vectors/inline.json through guatiao_value_inline_text, the line
 * compared byte for byte, then the nodes a checked builder refuses, a node
 * in the wiping allocator and the refusals.
 *
 * `inline_vectors.h` is generated from the vector file by the test that
 * compiles this file; the trees travel as their wire bytes in hex.
 *
 * Prints "all checks passed" on success. Any failure returns non-zero
 * with a line saying which vector and what it saw.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "guatiao.h"
#include "inline_vectors.h"

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

static void run_vector(const inline_vector *c) {
  guatiao_value value, out;
  guatiao_status status;
  CHECK(decode(c->tree, &value), "%s: tree", c->name);
  status = guatiao_value_inline_text(&value, &ALLOC, &out);
  CHECK(status == GUATIAO_OK, "%s: status %d", c->name, (int)status);
  CHECK(is_text(&out, c->text), "%s: not %s", c->name, c->text);
  guatiao_value_free(&out);
  guatiao_value_free(&value);
  checked++;
}

/* Nodes a checked builder refuses, filled in by hand over borrowed bytes
   (cap 0, no allocator): text, a number or a map key that is not what it
   claims. Each prints its placeholder and nothing else fails. */
static void unreadable(const inline_unreadable *c) {
  size_t n = strlen(c->hex) / 2;
  size_t i;
  uint8_t *bytes = malloc(n ? n : 1);
  guatiao_value out;
  guatiao_status status;
  if (bytes == NULL) {
    exit(2);
  }
  for (i = 0; i < n; i++) {
    bytes[i] = (uint8_t)(nibble(c->hex[2 * i]) << 4 | nibble(c->hex[2 * i + 1]));
  }
  if (c->kind == 2) {
    guatiao_entry entries[2];
    guatiao_value map;
    memset(entries, 0, sizeof entries);
    memset(&map, 0, sizeof map);
    entries[0].key = (guatiao_string)GUATIAO_STRING_LIT("ok");
    entries[0].value = (guatiao_value)GUATIAO_VALUE_NUMBER_LIT("1");
    entries[1].key.ptr = bytes;
    entries[1].key.len = n;
    entries[1].value = (guatiao_value)GUATIAO_VALUE_NUMBER_LIT("2");
    map.tag = GUATIAO_MAP;
    map.payload.map.ptr = entries;
    map.payload.map.len = 2;
    status = guatiao_value_inline_text(&map, &ALLOC, &out);
  } else {
    guatiao_value node;
    memset(&node, 0, sizeof node);
    node.tag = c->kind == 0 ? GUATIAO_STRING : GUATIAO_NUMBER;
    node.payload.text.ptr = bytes;
    node.payload.text.len = n;
    status = guatiao_value_inline_text(&node, &ALLOC, &out);
  }
  CHECK(status == GUATIAO_OK, "%s: status %d", c->name, (int)status);
  CHECK(is_text(&out, c->text), "%s: not %s", c->name, c->text);
  guatiao_value_free(&out);
  free(bytes);
  checked++;
}

static void sensitive_and_refusals(void) {
  guatiao_alloc wiping;
  guatiao_value secret, map, out;
  CHECK(guatiao_alloc_wiping(&wiping) == GUATIAO_OK, "a wiping allocator");
  CHECK(guatiao_value_string(&wiping, guatiao_cstr("hunter2"), &secret) == GUATIAO_OK, "a secret");
  CHECK(guatiao_value_inline_text(&secret, &ALLOC, &out) == GUATIAO_OK, "status of a secret");
  CHECK(is_text(&out, "[sensitive]"), "a secret is not shown");
  guatiao_value_free(&out);

  CHECK(guatiao_value_map(&ALLOC, &map) == GUATIAO_OK, "a map");
  CHECK(guatiao_map_set(&ALLOC, &map, guatiao_cstr("user"), &secret) == GUATIAO_OK, "set");
  CHECK(guatiao_value_inline_text(&map, &ALLOC, &out) == GUATIAO_OK, "status of a map");
  CHECK(is_text(&out, "{\"user\":[sensitive]}"), "a secret inside a map is not shown");
  guatiao_value_free(&out);
  guatiao_value_free(&map);

  CHECK(guatiao_value_null(&secret) == GUATIAO_OK, "null");
  CHECK(guatiao_value_inline_text(NULL, &ALLOC, &out) == GUATIAO_ERR_NULL, "a null value");
  CHECK(guatiao_value_inline_text(&secret, NULL, &out) == GUATIAO_ERR_ALLOC, "a null allocator");
  CHECK(guatiao_tag_of(&out) == GUATIAO_ABSENT, "out stays ABSENT");
}

int main(void) {
  size_t i;
  for (i = 0; i < INLINE_COUNT; i++) {
    run_vector(&INLINE_VECTORS[i]);
  }
  for (i = 0; i < INLINE_UNREADABLE_COUNT; i++) {
    unreadable(&INLINE_UNREADABLE[i]);
  }
  sensitive_and_refusals();
  if (failures != 0) {
    printf("%d checks failed\n", failures);
    return 1;
  }
  printf("all checks passed (%zu vectors)\n", checked);
  return 0;
}
