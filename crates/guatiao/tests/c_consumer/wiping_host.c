/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * A C caller holding a secret. It takes the wiping allocator from
 * `guatiao_alloc_wiping` and asks whether trees live there; and it writes
 * an allocator of its own that wipes, which works because guatiao hands
 * every block back to `free` with the exact size it was allocated with:
 * this allocator records each block's size and checks it on release,
 * across a string grown many times and a map holding it.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "guatiao.h"

static int failures = 0;

#define CHECK(cond, what)                       \
  do {                                          \
    if (!(cond)) {                              \
      printf("FAIL: %s\n", what);               \
      failures++;                               \
    }                                           \
  } while (0)

static const char SECRET[] = "hunter2-SECRET";

/* Every block this allocator has out, with the size it was asked for. */
#define MAX_BLOCKS 256
static struct {
  void *ptr;
  size_t size;
} out_now[MAX_BLOCKS];
static size_t allocated = 0, freed = 0, wrong_size = 0, still_secret = 0;

static void *mine_alloc(void *ctx, size_t size, size_t align) {
  (void)ctx;
  (void)align;
  void *p = malloc(size);
  for (size_t i = 0; p && i < MAX_BLOCKS; i++) {
    if (!out_now[i].ptr) {
      out_now[i].ptr = p;
      out_now[i].size = size;
      allocated++;
      return p;
    }
  }
  free(p);
  return NULL;
}

/* The wipe a C author writes: a volatile store per byte of `size`. */
static void mine_free(void *ctx, void *p, size_t size, size_t align) {
  (void)ctx;
  (void)align;
  for (size_t i = 0; i < MAX_BLOCKS; i++) {
    if (out_now[i].ptr == p) {
      if (out_now[i].size != size) {
        wrong_size++;
      }
      out_now[i].ptr = NULL;
    }
  }
  volatile unsigned char *bytes = (volatile unsigned char *)p;
  for (size_t i = 0; i < size; i++) {
    bytes[i] = 0;
  }
  for (size_t i = 0; i + sizeof SECRET - 1 <= size; i++) {
    if (memcmp((const void *)(bytes + i), SECRET, sizeof SECRET - 1) == 0) {
      still_secret++;
    }
  }
  freed++;
  free(p);
}

static guatiao_str str_of(const char *s) {
  guatiao_str out = {(const uint8_t *)s, strlen(s)};
  return out;
}

/* A string holding the secret, grown past its first block several times,
 * stored in a map: every block it leaves behind goes back to `free`. */
static void build_and_free(const guatiao_alloc *alloc, bool *sensitive) {
  guatiao_value text, map;
  CHECK(guatiao_value_string(alloc, str_of(SECRET), &text) == GUATIAO_OK, "string");
  for (int i = 0; i < 6; i++) {
    CHECK(guatiao_string_push(alloc, &text, str_of("-and-a-little-more-text")) == GUATIAO_OK,
          "push");
  }
  CHECK(guatiao_value_map(alloc, &map) == GUATIAO_OK, "map");
  CHECK(guatiao_map_set(alloc, &map, str_of(SECRET), &text) == GUATIAO_OK, "set");
  *sensitive = guatiao_value_is_sensitive(&map);
  CHECK(guatiao_value_free(&map) == GUATIAO_OK, "free");
}

int main(void) {
  guatiao_alloc wiping, plain;
  CHECK(guatiao_alloc_wiping(&wiping) == GUATIAO_OK, "guatiao_alloc_wiping");
  CHECK(guatiao_alloc_default(&plain) == GUATIAO_OK, "guatiao_alloc_default");
  guatiao_alloc mine = {(uint32_t)sizeof(guatiao_alloc), NULL, mine_alloc, mine_free, NULL};

  CHECK(guatiao_alloc_is_wiping(&wiping), "the wiping allocator is recognised");
  CHECK(!guatiao_alloc_is_wiping(&plain), "the default one is not");
  CHECK(!guatiao_alloc_is_wiping(&mine), "nor one of the caller's own");
  CHECK(!guatiao_alloc_is_wiping(NULL), "nor a null one");
  CHECK(!guatiao_value_is_sensitive(NULL), "a null value is not sensitive");

  bool sensitive = false;
  build_and_free(&wiping, &sensitive);
  CHECK(sensitive, "a tree in the wiping allocator is sensitive");
  build_and_free(&plain, &sensitive);
  CHECK(!sensitive, "a tree in the default allocator is not");

  build_and_free(&mine, &sensitive);
  CHECK(!sensitive, "a tree in the caller's own allocator is not recognised");
  CHECK(allocated >= 4, "the string grew through the caller's allocator");
  CHECK(freed == allocated, "every block came back to free");
  CHECK(wrong_size == 0, "every block came back with the size it was allocated with");
  CHECK(still_secret == 0, "the caller's wipe covered every block");

  if (failures) {
    printf("%d check(s) failed\n", failures);
    return 1;
  }
  printf("blocks: %zu allocated, %zu freed, each with its size\n", allocated, freed);
  printf("all checks passed\n");
  return 0;
}
