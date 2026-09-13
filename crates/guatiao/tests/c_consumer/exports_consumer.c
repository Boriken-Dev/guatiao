/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * Builds and frees a tree from C, through the exported functions.
 *
 * The other consumer in this directory links NOTHING: it reads a literal
 * tree with the header's `static inline` helpers, which is the headline
 * claim. This one is the other half -- everything that needs code, called
 * the way a real C caller calls it, against the artifact that exports it.
 *
 * Prints "all checks passed" on success. Any failure returns non-zero
 * with a line saying which check and what it saw.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "guatiao.h"

static int failures = 0;

#define CHECK(cond, ...)                          \
  do {                                            \
    if (!(cond)) {                                \
      printf("FAIL %s:%d: ", __FILE__, __LINE__); \
      printf(__VA_ARGS__);                        \
      printf("\n");                               \
      failures++;                                 \
    }                                             \
  } while (0)

int main(void) {
  /* The allocator every tree below grows through. Declared by the header
     as a macro over malloc/free, so this file supplies no allocator of
     its own. */
  guatiao_alloc alloc = GUATIAO_ALLOC_MALLOC;

  /* ---- a map, built one key at a time ------------------------------ */

  guatiao_value map;
  CHECK(guatiao_value_map(&alloc, &map) == GUATIAO_OK, "could not create a map");

  guatiao_value host;
  CHECK(guatiao_value_string(&alloc, guatiao_cstr("10.0.0.1"), &host) == GUATIAO_OK,
        "could not create a string");
  /* MOVES `host` in and leaves it null-tagged, so freeing the map frees
     it exactly once and freeing `host` afterwards is a no-op. */
  CHECK(guatiao_map_set(&alloc, &map, guatiao_cstr("host"), &host) == GUATIAO_OK,
        "could not set a key");
  CHECK(guatiao_tag_of(&host) == GUATIAO_NULL,
        "a moved value must be left null-tagged, saw tag %u", guatiao_tag_of(&host));

  guatiao_value port;
  CHECK(guatiao_value_number(&alloc, guatiao_cstr("5900"), &port) == GUATIAO_OK,
        "could not create a number");
  CHECK(guatiao_map_set(&alloc, &map, guatiao_cstr("port"), &port) == GUATIAO_OK,
        "could not set a key");

  /* A key with a NUL in it: legal, because a key is pointer and length. */
  guatiao_value odd;
  CHECK(guatiao_value_bool(1, &odd) == GUATIAO_OK, "could not create a bool");
  CHECK(guatiao_map_set(&alloc, &map, guatiao_str_from("a\0b", 3), &odd) == GUATIAO_OK,
        "a key containing a NUL must be accepted");

  /* ---- reading it back, with nothing linked ------------------------ */

  const guatiao_value *found = guatiao_map_find(&map, guatiao_cstr("host"));
  CHECK(found != NULL, "the key just set must be found");
  if (found) {
    guatiao_str text = guatiao_string_text(found);
    CHECK(text.len == 8 && memcmp(text.ptr, "10.0.0.1", 8) == 0,
          "the stored text must read back verbatim");
  }
  CHECK(guatiao_int_or(guatiao_map_find(&map, guatiao_cstr("port")), -1) == 5900,
        "the stored number must read back as 5900");
  CHECK(guatiao_map_find(&map, guatiao_str_from("a\0b", 3)) != NULL,
        "a NUL-bearing key must be findable by its full bytes");
  CHECK(guatiao_map_find(&map, guatiao_cstr("a")) == NULL,
        "and must NOT match a prefix, which is what strcmp would do");
  CHECK(guatiao_map_entries(&map).len == 3, "three keys were set");

  /* ---- a list ------------------------------------------------------- */

  guatiao_value list;
  CHECK(guatiao_value_list(&alloc, &list) == GUATIAO_OK, "could not create a list");
  for (int i = 0; i < 3; i++) {
    char digits[2];
    guatiao_value item;
    digits[0] = (char)('1' + i);
    digits[1] = '\0';
    CHECK(guatiao_value_number(&alloc, guatiao_cstr(digits), &item) == GUATIAO_OK,
          "could not create a number");
    CHECK(guatiao_list_push(&alloc, &list, &item) == GUATIAO_OK, "could not push");
  }
  CHECK(guatiao_list_items(&list).len == 3, "three items were pushed");
  CHECK(guatiao_int_or(guatiao_list_at(&list, 2), -1) == 3, "the third item is 3");

  CHECK(guatiao_list_discard(&list, 0) == GUATIAO_OK, "could not discard");
  CHECK(guatiao_list_items(&list).len == 2, "one was removed");
  CHECK(guatiao_int_or(guatiao_list_at(&list, 0), -1) == 2,
        "removal keeps the order of the rest");

  /* Nesting: the list MOVES into the map. */
  CHECK(guatiao_map_set(&alloc, &map, guatiao_cstr("ports"), &list) == GUATIAO_OK,
        "could not nest a list");
  CHECK(guatiao_tag_of(&list) == GUATIAO_NULL, "the nested list was moved, not copied");

  /* ---- removal ------------------------------------------------------ */

  CHECK(guatiao_map_discard(&map, guatiao_cstr("port")) == GUATIAO_OK, "could not discard a key");
  CHECK(guatiao_map_find(&map, guatiao_cstr("port")) == NULL, "the key is gone");
  CHECK(guatiao_map_discard(&map, guatiao_cstr("never-there")) == GUATIAO_ERR_NOT_FOUND,
        "discarding a key that is not there says so rather than succeeding");

  /* ---- the wrong kind is refused, not guessed at -------------------- */

  guatiao_value scalar;
  CHECK(guatiao_value_bool(0, &scalar) == GUATIAO_OK, "could not create a bool");
  guatiao_value orphan;
  CHECK(guatiao_value_bool(1, &orphan) == GUATIAO_OK, "could not create a bool");
  CHECK(guatiao_map_set(&alloc, &scalar, guatiao_cstr("k"), &orphan) == GUATIAO_ERR_WRONG_KIND,
        "setting a key on a bool must be refused");
  CHECK(guatiao_tag_of(&orphan) == GUATIAO_BOOL,
        "a refused move must leave the caller's value alone");
  guatiao_value_free(&orphan);

  /* ---- null is refused, not dereferenced ---------------------------- */

  CHECK(guatiao_value_free(NULL) == GUATIAO_ERR_NULL, "free(NULL) must be a status");
  CHECK(guatiao_map_clear(NULL) == GUATIAO_ERR_NULL, "clear(NULL) must be a status");

  /* ---- freeing ------------------------------------------------------ */

  /* One call frees the whole tree, including the nested list and every
     string in it: each container carries the allocator that made it. */
  CHECK(guatiao_value_free(&map) == GUATIAO_OK, "could not free the tree");
  CHECK(guatiao_tag_of(&map) == GUATIAO_NULL, "a freed node is left null-tagged");
  /* Which is what makes a second free a no-op rather than a crash. */
  CHECK(guatiao_value_free(&map) == GUATIAO_OK, "freeing a null-tagged node again is safe");

  /* The moved-from values were null-tagged, so these free nothing. */
  guatiao_value_free(&host);
  guatiao_value_free(&port);
  guatiao_value_free(&odd);
  guatiao_value_free(&list);
  guatiao_value_free(&scalar);

  if (failures != 0) {
    printf("%d check(s) failed\n", failures);
    return 1;
  }
  printf("all checks passed\n");
  return 0;
}
