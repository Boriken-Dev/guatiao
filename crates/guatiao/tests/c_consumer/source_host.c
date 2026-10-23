/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * A HOST written in C that adds a source written in Rust
 * (examples/source_library, exported with guatiao::source!) and calls a
 * greeter it serves through the greeter kind's table.
 *
 *   source_host <source file>
 */

#include <stdio.h>
#include <string.h>

#include "greeter_kind.h"
#include "guatiao.h"

static guatiao_alloc ALLOC = GUATIAO_ALLOC_MALLOC;

static int fail(const char *what, guatiao_status status) {
  printf("FAILED: %s (%d)\n", what, (int)status);
  return 1;
}

int main(int argc, char **argv) {
  if (argc != 2) {
    printf("usage: source_host <source file>\n");
    return 2;
  }
  guatiao_registry *reg =
      guatiao_registry_new(guatiao_cstr("c-source-host"), guatiao_cstr("1.0"), &ALLOC);
  if (!reg) {
    return fail("guatiao_registry_new", GUATIAO_ERR_NULL);
  }

  /* {"greetings": {"hi": "hello"}} */
  guatiao_value config, greetings, word;
  guatiao_value_map(&ALLOC, &config);
  guatiao_value_map(&ALLOC, &greetings);
  guatiao_value_string(&ALLOC, guatiao_cstr("hello"), &word);
  guatiao_map_set(&ALLOC, &greetings, guatiao_cstr("hi"), &word);
  guatiao_map_set(&ALLOC, &config, guatiao_cstr("greetings"), &greetings);
  guatiao_value_free(&word);
  guatiao_value_free(&greetings);

  guatiao_provider_error err;
  memset(&err, 0, sizeof(err));
  guatiao_status status =
      guatiao_registry_add_source_file(reg, guatiao_cstr(argv[1]), &config, &err);
  guatiao_value_free(&config);
  if (status != GUATIAO_OK) {
    printf("the source was refused: %.*s\n", (int)err.message.len, (const char *)err.message.ptr);
    return fail("guatiao_registry_add_source_file", status);
  }

  guatiao_value answer;
  status = guatiao_registry_refresh(reg, &ALLOC, &answer);
  if (status != GUATIAO_OK) {
    return fail("guatiao_registry_refresh", status);
  }
  guatiao_values loaded = guatiao_list_items(guatiao_map_find(&answer, guatiao_cstr("loaded")));
  size_t served = loaded.len;
  guatiao_value_free(&answer);
  if (served != 1) {
    return fail("one library served", GUATIAO_ERR_NOT_FOUND);
  }

  size_t size = 0;
  const void *table = guatiao_registry_provider_table(reg, guatiao_cstr("hi_greeter"),
                                                      guatiao_cstr("greeter"), &size);
  void *ctx = guatiao_registry_provider_ctx(reg, guatiao_cstr("hi_greeter"));
  if (!table || size < sizeof(greeter_vtable)) {
    return fail("no greeter table for hi_greeter", GUATIAO_ERR_NOT_FOUND);
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
    return fail("greet", status);
  }
  guatiao_value greeting;
  memset(&greeting, 0, sizeof(greeting));
  greeting.tag = (uint32_t)GUATIAO_MAP;
  greeting.payload.map = out;
  guatiao_str text = guatiao_string_text(guatiao_map_find(&greeting, guatiao_cstr("greeting")));
  printf("hi_greeter greets: %.*s\n", (int)text.len, (const char *)text.ptr);
  int ok = text.len == 10 && memcmp(text.ptr, "hello, ana", 10) == 0;
  guatiao_value_free(&greeting);

  guatiao_registry_free(reg);
  if (!ok) {
    return fail("the greeting", GUATIAO_ERR_BAD_VALUE);
  }
  printf("all checks passed\n");
  return 0;
}
