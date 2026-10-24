/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * A C caller of a table that ignores the carrier (`carrier_ignored.c`,
 * the library named as the first argument). The answer arrives in the
 * library's allocator; `guatiao_value_make_local` moves it into the
 * caller's while the library is loaded, and the caller then unloads the
 * library, reads the answer and frees it.
 */

#include <stdio.h>
#include <string.h>

#include "guatiao.h"

#if defined(_WIN32)
#include <windows.h>
typedef HMODULE library;
static library open_library(const char *path) { return LoadLibraryA(path); }
static void *symbol(library l, const char *name) { return (void *)GetProcAddress(l, name); }
static int close_library(library l) { return FreeLibrary(l) ? 0 : 1; }
#else
#include <dlfcn.h>
typedef void *library;
static library open_library(const char *path) { return dlopen(path, RTLD_NOW | RTLD_LOCAL); }
static void *symbol(library l, const char *name) { return dlsym(l, name); }
static int close_library(library l) { return dlclose(l); }
#endif

typedef struct carrier_ignored_table {
  guatiao_kind_header header;
  guatiao_status (*answer)(void *ctx, guatiao_value *out);
} carrier_ignored_table;

static int failures = 0;

#define CHECK(cond, what)                       \
  do {                                          \
    if (!(cond)) {                              \
      printf("FAIL: %s\n", what);               \
      failures++;                               \
    }                                           \
  } while (0)

static bool text_is(const guatiao_string *s, const char *want) {
  return s->len == strlen(want) && memcmp(s->ptr, want, s->len) == 0;
}

int main(int argc, char **argv) {
  if (argc < 2) {
    printf("usage: make_local_host <carrier_ignored library>\n");
    return 2;
  }
  library lib = open_library(argv[1]);
  if (!lib) {
    printf("FAIL: could not load %s\n", argv[1]);
    return 1;
  }
  const carrier_ignored_table *table =
      (const carrier_ignored_table *)symbol(lib, "CARRIER_IGNORED_TABLE");
  const guatiao_alloc *(*theirs_of)(void) =
      (const guatiao_alloc *(*)(void))symbol(lib, "carrier_ignored_alloc");
  if (!table || !theirs_of) {
    printf("FAIL: the library lacks its table\n");
    return 1;
  }
  const guatiao_alloc *theirs = theirs_of();

  guatiao_alloc home;
  CHECK(guatiao_alloc_default(&home) == GUATIAO_OK, "the default allocator");

  /* The carrier names `home`; the slot ignores it. */
  guatiao_value out;
  memset(&out, 0, sizeof out);
  out.tag = (uint32_t)GUATIAO_ABSENT;
  out.payload.map.alloc = &home;
  CHECK(table->answer(NULL, &out) == GUATIAO_OK, "the slot answers");
  CHECK(out.tag == (uint32_t)GUATIAO_MAP, "a map");
  CHECK(out.payload.map.alloc == theirs, "built in the library's allocator");

  CHECK(guatiao_value_make_local(NULL, &home) == GUATIAO_ERR_NULL, "a null value is refused");
  CHECK(guatiao_value_make_local(&out, NULL) == GUATIAO_ERR_ALLOC, "a null allocator is refused");
  CHECK(out.payload.map.alloc == theirs, "a refusal moves nothing");

  CHECK(guatiao_value_make_local(&out, &home) == GUATIAO_OK, "moved home");
  const guatiao_entry *e = out.payload.map.ptr;
  CHECK(out.payload.map.alloc != theirs, "the map's entries left the library");
  CHECK(e[0].key.alloc != theirs, "the key left the library");
  CHECK(e[0].value.payload.text.alloc != theirs, "the string left the library");
  CHECK(guatiao_value_make_local(&out, &home) == GUATIAO_OK, "moving a tree already home");

  CHECK(close_library(lib) == 0, "the library unloads");
  /* Unmapped, not merely released: otherwise freeing through its
   * allocator would still work and this would prove nothing. */
#if defined(_WIN32)
  CHECK(GetModuleHandleA(argv[1]) == NULL, "the library is gone");
#else
  CHECK(dlopen(argv[1], RTLD_NOW | RTLD_NOLOAD) == NULL, "the library is gone");
#endif

  CHECK(out.payload.map.len == 1, "one entry");
  CHECK(text_is(&e[0].key, "greeting"), "its key");
  CHECK(e[0].value.tag == (uint32_t)GUATIAO_STRING && text_is(&e[0].value.payload.text, "hello"),
        "its value");
  CHECK(guatiao_value_free(&out) == GUATIAO_OK, "freed after the unload");

  if (failures) {
    printf("%d check(s) failed\n", failures);
    return 1;
  }
  printf("all checks passed\n");
  return 0;
}
