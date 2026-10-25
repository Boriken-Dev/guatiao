/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * A HOST written in C that adds the guatiao-source-js source and calls a
 * greeter written in JavaScript through the greeter kind's table.
 *
 *   c_host <source> <libraries dir> <greeter_kind.kinds.json>
 *
 * It links the guatiao library (for the registry) and nothing of QuickJS:
 * the source carries its own.
 */

#include <stdio.h>
#include <string.h>

#include "greeter_kind.h"
#include "guatiao.h"

static guatiao_alloc ALLOC = GUATIAO_ALLOC_MALLOC;

static void push_text(guatiao_value *list, const char *text) {
  guatiao_value v;
  guatiao_value_string(&ALLOC, guatiao_cstr(text), &v);
  guatiao_list_push(&ALLOC, list, &v);
  guatiao_value_free(&v); /* moved into the list; what is left owns nothing */
}

static int fail(const char *what, guatiao_status status) {
  printf("FAILED: %s (%d)\n", what, (int)status);
  return 1;
}

int main(int argc, char **argv) {
  if (argc != 4) {
    printf("usage: c_host <source> <libraries dir> <kinds.json>\n");
    return 2;
  }
  guatiao_registry *reg =
      guatiao_registry_new(guatiao_cstr("c-host"), guatiao_cstr("1.0"), &ALLOC);
  if (!reg) {
    return fail("guatiao_registry_new", GUATIAO_ERR_NULL);
  }

  /* {"dirs": [<libraries>], "kinds": [<kinds.json>]} */
  guatiao_value config, dirs, kinds;
  guatiao_value_map(&ALLOC, &config);
  guatiao_value_list(&ALLOC, &dirs);
  guatiao_value_list(&ALLOC, &kinds);
  push_text(&dirs, argv[2]);
  push_text(&kinds, argv[3]);
  guatiao_map_set(&ALLOC, &config, guatiao_cstr("dirs"), &dirs);
  guatiao_map_set(&ALLOC, &config, guatiao_cstr("kinds"), &kinds);
  guatiao_value_free(&dirs);
  guatiao_value_free(&kinds);

  guatiao_provider_error err;
  memset(&err, 0, sizeof(err));
  guatiao_status status = guatiao_registry_add_source_file(reg, guatiao_cstr(argv[1]), &config, &err);
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
  for (size_t i = 0; i < loaded.len; i++) {
    guatiao_str id = guatiao_string_text(guatiao_map_find(&loaded.ptr[i], guatiao_cstr("id")));
    printf("served %.*s\n", (int)id.len, (const char *)id.ptr);
  }
  guatiao_value_free(&answer);

  size_t size = 0;
  const void *table = guatiao_registry_provider_table(reg, guatiao_cstr("quickjs_greeter_hello"),
                                                      guatiao_cstr("greeter"), &size);
  void *ctx = guatiao_registry_provider_ctx(reg, guatiao_cstr("quickjs_greeter_hello"));
  if (!table || size < sizeof(greeter_vtable)) {
    return fail("no greeter table for quickjs_greeter_hello", GUATIAO_ERR_NOT_FOUND);
  }
  const greeter_vtable *greeter = (const greeter_vtable *)table;
  if (greeter->header.floor_hash != greeter_vtable_FLOOR_HASH) {
    return fail("the table is not the greeter kind's", GUATIAO_ERR_WRONG_KIND);
  }

  /* The carrier names this host's allocator: the answer is built in it. */
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
  printf("quickjs_greeter_hello greets: %.*s\n", (int)text.len, (const char *)text.ptr);
  int ok = text.len == 10 && memcmp(text.ptr, "hello, ana", 10) == 0;
  guatiao_value_free(&greeting);

  /* A refusal crosses with its words. */
  guatiao_map nothing = {NULL, 0, 0, &ALLOC};
  memset(&gerr, 0, sizeof(gerr));
  status = greeter->greet(ctx, guatiao_cstr(""), &nothing, &gerr);
  printf("an empty name is refused (%d): %.*s\n", (int)status, (int)gerr.message.len,
         (const char *)gerr.message.ptr);
  ok = ok && status == GUATIAO_ERR_BAD_VALUE;
  if (gerr.message.cap) {
    gerr.message.alloc->free(gerr.message.alloc->ctx, gerr.message.ptr, gerr.message.cap, 1);
  }

  guatiao_registry_free(reg);
  if (!ok) {
    return fail("the greeting", GUATIAO_ERR_BAD_VALUE);
  }
  printf("all checks passed\n");
  return 0;
}
