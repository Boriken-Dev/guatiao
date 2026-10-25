/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * A HOST written in C that adds guatiao-source-remote's library as a source,
 * with a server in a child process, and calls the greeters it serves
 * through the greeter kind's table, as it would a library it loaded.
 *
 *   remote_host <remote library> <server> <kinds.json> <library>... -- <provider>...
 */

#ifdef _WIN32
#define _CRT_SECURE_NO_WARNINGS
#endif

#include <stdio.h>
#include <string.h>

#include "greeter_kind.h"
#include "guatiao.h"

static guatiao_alloc ALLOC = GUATIAO_ALLOC_MALLOC;

static int fail(const char *what, guatiao_status status) {
  printf("FAILED: %s (%d)\n", what, (int)status);
  return 1;
}

static void push_text(guatiao_value *list, const char *text) {
  guatiao_value v;
  guatiao_value_string(&ALLOC, guatiao_cstr(text), &v);
  guatiao_list_push(&ALLOC, list, &v);
}

int main(int argc, char **argv) {
  if (argc < 6) {
    printf("usage: remote_host <remote library> <server> <kinds.json> <library>... -- "
           "<provider>...\n");
    return 2;
  }
  int dashes = 4;
  while (dashes < argc && strcmp(argv[dashes], "--") != 0) {
    dashes++;
  }

  /* {"child": [server, "--stdio", "--kinds", kinds, "--load", library, ...]} */
  guatiao_value config, child;
  guatiao_value_map(&ALLOC, &config);
  guatiao_value_list(&ALLOC, &child);
  push_text(&child, argv[2]);
  push_text(&child, "--stdio");
  push_text(&child, "--kinds");
  push_text(&child, argv[3]);
  for (int i = 4; i < dashes; i++) {
    push_text(&child, "--load");
    push_text(&child, argv[i]);
  }
  guatiao_map_set(&ALLOC, &config, guatiao_cstr("child"), &child);

  guatiao_registry *reg =
      guatiao_registry_new(guatiao_cstr("c-remote-host"), guatiao_cstr("1.0"), &ALLOC);
  if (!reg) {
    return fail("guatiao_registry_new", GUATIAO_ERR_NULL);
  }
  guatiao_provider_error err;
  memset(&err, 0, sizeof(err));
  guatiao_status status =
      guatiao_registry_add_source_file(reg, guatiao_cstr(argv[1]), &config, &err);
  guatiao_value_free(&config);
  if (status != GUATIAO_OK) {
    printf("the source was refused: %.*s\n", (int)err.message.len,
           (const char *)err.message.ptr);
    return fail("guatiao_registry_add_source_file", status);
  }
  guatiao_value answer;
  status = guatiao_registry_refresh(reg, &ALLOC, &answer);
  if (status != GUATIAO_OK) {
    return fail("guatiao_registry_refresh", status);
  }
  size_t loaded = guatiao_list_items(guatiao_map_find(&answer, guatiao_cstr("loaded"))).len;
  guatiao_value_free(&answer);
  printf("libraries served: %u\n", (unsigned)loaded);

  int failures = 0;
  for (int i = dashes + 1; i < argc; i++) {
    size_t size = 0;
    const void *table = guatiao_registry_provider_table(reg, guatiao_cstr(argv[i]),
                                                        guatiao_cstr("greeter"), &size);
    void *ctx = guatiao_registry_provider_ctx(reg, guatiao_cstr(argv[i]));
    if (!table || size < sizeof(greeter_vtable)) {
      printf("no greeter table for %s\n", argv[i]);
      failures++;
      continue;
    }
    const greeter_vtable *greeter = (const greeter_vtable *)table;
    if (greeter->header.floor_hash != greeter_vtable_FLOOR_HASH) {
      return fail("the table is not the greeter kind's", GUATIAO_ERR_WRONG_KIND);
    }
    guatiao_map out = {NULL, 0, 0, &ALLOC};
    guatiao_provider_error gerr;
    memset(&gerr, 0, sizeof(gerr));
    status = greeter->greet(ctx, guatiao_cstr("ana"), &out, &gerr);
    if (status != GUATIAO_OK) {
      printf("%s refused: %.*s\n", argv[i], (int)gerr.message.len,
             (const char *)gerr.message.ptr);
      failures++;
      continue;
    }
    guatiao_value greeting;
    memset(&greeting, 0, sizeof(greeting));
    greeting.tag = (uint32_t)GUATIAO_MAP;
    greeting.payload.map = out;
    guatiao_str text =
        guatiao_string_text(guatiao_map_find(&greeting, guatiao_cstr("greeting")));
    printf("%s greets: %.*s\n", argv[i], (int)text.len, (const char *)text.ptr);
    guatiao_value_free(&greeting);
  }
  guatiao_registry_free(reg);
  if (failures) {
    return fail("a greeter", GUATIAO_ERR_NOT_FOUND);
  }
  printf("all checks passed\n");
  return 0;
}
