/*
 * A HOST, written in C.
 *
 * This is the claim that matters for the envelope: a program that links
 * no Rust creates a registry, loads a real shared library into it, asks
 * what that library offers, reads the answer as an ordinary value tree,
 * and calls through the provider's own function table.
 *
 * argv[1] is the library to load.
 *
 * Prints "all checks passed" on success. Any failure returns non-zero
 * with a line saying which check and what it saw.
 */

#include <stdbool.h>
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

/* The greeter's table, as this host understands it. The envelope defines
   no vtable; whoever defines a kind defines what it looks like, and a host
   compiles against its own declaration and checks the size. */
typedef struct {
  uint32_t struct_size;
  guatiao_status (*greet)(void *ctx, const guatiao_value *config,
                          guatiao_value *out);
  int64_t (*outstanding)(void *ctx);
} greeter_vtable;

/* The frozen floor: the original slot and nothing after it. A floor that
   tracked the newest slot would refuse every library built before it. */
#define GREETER_FLOOR (offsetof(greeter_vtable, greet) + sizeof(void *))

static guatiao_str s(const char *lit) { return guatiao_cstr(lit); }

/* The value under `key` in a map, as text. Empty when absent. */
static guatiao_str field(const guatiao_value *map, const char *key) {
  const guatiao_value *v = guatiao_map_find(map, s(key));
  return guatiao_string_text(v);
}

static bool text_is(guatiao_str got, const char *want) {
  return guatiao_str_eq(got, s(want));
}

int main(int argc, char **argv) {
  guatiao_alloc alloc = GUATIAO_ALLOC_MALLOC;
  guatiao_registry *reg = NULL;
  guatiao_value answer = {0};
  guatiao_status st;

  if (argc < 2) {
    printf("FAIL: usage: %s <library>\n", argv[0]);
    return 2;
  }

  /* ---- a registry, introducing this host ---------------------------- */

  reg = guatiao_registry_new(s("c-host"), s("1.0"), &alloc);
  CHECK(reg != NULL, "a registry could not be created");
  if (reg == NULL) {
    return 1;
  }

  /* ---- load a real library ------------------------------------------ */

  st = guatiao_registry_load_file(reg, s(argv[1]), &alloc, &answer);
  CHECK(st == GUATIAO_OK, "load_file returned %d", (int)st);
  {
    const guatiao_value *loaded = guatiao_map_find(&answer, s("loaded"));
    CHECK(loaded != NULL, "the answer does not say it loaded");
    if (loaded) {
      CHECK(text_is(field(loaded, "id"), "hello_library"), "wrong library id");
      CHECK(text_is(field(loaded, "key"), "hello_library"),
            "the default library key is %%id");
      CHECK(guatiao_int_or(guatiao_map_find(loaded, s("providers")), -1) == 3,
            "expected three providers");
    }
  }
  guatiao_value_free(&answer);

  /* The same file again is a SKIP naming where it came from, not an
     error -- the case a host's search path produces on every run. */
  st = guatiao_registry_load_file(reg, s(argv[1]), &alloc, &answer);
  CHECK(st == GUATIAO_OK, "a repeat load returned %d", (int)st);
  CHECK(text_is(field(&answer, "skipped"), "already-loaded"),
        "a repeat should be reported as already-loaded");
  CHECK(guatiao_string_text(guatiao_map_find(&answer, s("from"))).len > 0,
        "an already-loaded skip names where it came from");
  guatiao_value_free(&answer);

  /* ---- ask what serves a kind --------------------------------------- */

  st = guatiao_registry_providers(reg, s("greeter"), &alloc, &answer);
  CHECK(st == GUATIAO_OK, "providers returned %d", (int)st);
  {
    guatiao_values items = guatiao_list_items(&answer);
    CHECK(items.len == 1, "expected one greeter, saw %zu", items.len);
    if (items.len == 1) {
      const guatiao_value *p = &items.ptr[0];
      CHECK(text_is(field(p, "id"), "hello_library_greeter"), "wrong id");
      CHECK(text_is(field(p, "library"), "hello_library"), "wrong library");

      /* One provider, several kinds: it answers to each. */
      const guatiao_value *kinds = guatiao_map_find(p, s("kinds"));
      guatiao_values ks = guatiao_list_items(kinds);
      CHECK(ks.len == 2, "expected two kinds, saw %zu", ks.len);
      if (ks.len == 2) {
        CHECK(guatiao_str_eq(guatiao_string_text(&ks.ptr[0]), s("greeter")) &&
                  guatiao_str_eq(guatiao_string_text(&ks.ptr[1]), s("writer")),
              "the kinds are not what the library declared");
      }
      CHECK(guatiao_bool_or(guatiao_map_find(p, s("has_config")), false),
            "the greeter declares a schema");
    }
  }
  guatiao_value_free(&answer);

  /* The same provider answers to its OTHER kind, and is the same one. */
  st = guatiao_registry_providers(reg, s("writer"), &alloc, &answer);
  CHECK(st == GUATIAO_OK, "providers(writer) returned %d", (int)st);
  {
    guatiao_values items = guatiao_list_items(&answer);
    CHECK(items.len == 1 &&
              text_is(field(&items.ptr[0], "id"), "hello_library_greeter"),
          "a provider serving two kinds answers to both");
  }
  guatiao_value_free(&answer);

  /* A kind nobody serves is empty rather than an error. */
  st = guatiao_registry_providers(reg, s("nothing-of-this-kind"), &alloc,
                                  &answer);
  CHECK(st == GUATIAO_OK && guatiao_list_items(&answer).len == 0,
        "an unserved kind should be an empty list");
  guatiao_value_free(&answer);

  /* ---- the provider that declares no kind at all --------------------- */

  st = guatiao_registry_provider(reg, s("hello_library_almanac"), &alloc,
                                 &answer);
  CHECK(st == GUATIAO_OK, "provider by key returned %d", (int)st);
  CHECK(text_is(field(&answer, "version"), "1.0.0"),
        "a provider that declares its own version keeps it");
  CHECK(guatiao_list_items(guatiao_map_find(&answer, s("kinds"))).len == 0,
        "the almanac serves no kind");
  guatiao_value_free(&answer);

  st = guatiao_registry_provider(reg, s("nobody"), &alloc, &answer);
  CHECK(st == GUATIAO_ERR_NOT_FOUND, "an unknown key should be NOT_FOUND");

  /* ---- can it actually run here, and if not why ---------------------- */

  {
    guatiao_str why = {NULL, 0};

    /* No slot means available, which is the common case. */
    CHECK(guatiao_registry_provider_available(reg, s("hello_library_greeter"),
                                              &why),
          "a provider declaring no availability slot is available");

    /* And one that refuses carries its own words about why. */
    why.ptr = NULL;
    why.len = 0;
    CHECK(!guatiao_registry_provider_available(reg, s("hello_library_almanac"),
                                               &why),
          "the almanac refuses");
    CHECK(why.len > 0, "a refusal says why");
  }

  /* THE SPLIT THAT MATTERS: a kind nobody claims and a kind claimed only
     by something that cannot run are different answers, because the
     remedies differ -- install something, versus fix what you have. */
  st = guatiao_registry_why_not(reg, s("greeter"), &alloc, &answer);
  CHECK(st == GUATIAO_OK &&
            guatiao_bool_or(guatiao_map_find(&answer, s("available")), false),
        "something can serve greeter");
  guatiao_value_free(&answer);

  st = guatiao_registry_why_not(reg, s("nothing-of-this-kind"), &alloc, &answer);
  CHECK(st == GUATIAO_OK, "why_not returned %d", (int)st);
  CHECK(!guatiao_bool_or(guatiao_map_find(&answer, s("available")), true) &&
            text_is(field(&answer, "why"), "nothing-claims-it"),
        "a kind nobody claims");
  guatiao_value_free(&answer);

  st = guatiao_registry_why_not(reg, s("timekeeper"), &alloc, &answer);
  CHECK(st == GUATIAO_OK, "why_not returned %d", (int)st);
  CHECK(text_is(field(&answer, "why"), "none-available"),
        "a kind claimed only by something that cannot run");
  {
    guatiao_values refused =
        guatiao_list_items(guatiao_map_find(&answer, s("providers")));
    CHECK(refused.len == 1, "one provider refused, saw %zu", refused.len);
    if (refused.len == 1) {
      CHECK(text_is(field(&refused.ptr[0], "id"), "hello_library_sundial"),
            "the refusal names which provider");
      CHECK(field(&refused.ptr[0], "reason").len > 0,
            "and carries its reason");
    }
  }
  guatiao_value_free(&answer);

  /* Claiming a kind and being able to serve it are different questions. */
  st = guatiao_registry_providers(reg, s("timekeeper"), &alloc, &answer);
  CHECK(st == GUATIAO_OK && guatiao_list_items(&answer).len == 1,
        "the sundial claims timekeeper");
  guatiao_value_free(&answer);
  st = guatiao_registry_available(reg, s("timekeeper"), &alloc, &answer);
  CHECK(st == GUATIAO_OK && guatiao_list_items(&answer).len == 0,
        "and cannot serve it");
  guatiao_value_free(&answer);

  /* ---- read the schema it declared, with no library call ------------- */

  {
    const guatiao_value *schema =
        guatiao_registry_provider_config(reg, s("hello_library_greeter"));
    CHECK(schema != NULL, "the greeter declares a configuration schema");
    /* Borrowed from the library's image: read it, never free it. */
    if (schema) {
      const guatiao_value *options = guatiao_map_find(schema, s("options"));
      CHECK(guatiao_list_items(options).len == 1,
            "the schema declares one option");
    }
  }

  /* ---- call through the provider's own table ------------------------- */

  {
    size_t size = 0;
    const void *table =
        guatiao_registry_provider_vtable(reg, s("hello_library_greeter"), &size);
    CHECK(table != NULL, "the greeter has a table");
    CHECK(size >= GREETER_FLOOR, "a table of %zu bytes is below the floor",
          size);

    if (table && size >= GREETER_FLOOR) {
      const greeter_vtable *v = (const greeter_vtable *)table;
      void *ctx = guatiao_registry_provider_ctx(reg, s("hello_library_greeter"));

      guatiao_entry config_entries[] = {
          GUATIAO_ENTRY_LIT("name", GUATIAO_VALUE_STRING_LIT("ana")),
      };
      guatiao_value config = GUATIAO_VALUE_MAP_LIT(config_entries);
      guatiao_value greeting = {0};

      /* The appended slot, read only when the library declared it.
         A DELTA, never an absolute: the library holds its own schema and
         metadata, allocated through this same counter and never released,
         so the count is not zero and was never meant to be. */
      bool counts = size >= offsetof(greeter_vtable, outstanding) +
                                sizeof(void *) &&
                    v->outstanding != NULL;
      int64_t before = counts ? v->outstanding(ctx) : 0;

      CHECK(v->greet != NULL, "a greeter declares a greet slot");
      if (v->greet) {
        st = v->greet(ctx, &config, &greeting);
        CHECK(st == GUATIAO_OK, "greet returned %d", (int)st);
        CHECK(text_is(field(&greeting, "greeting"), "hello, ana"),
              "the greeting is not what the library builds");
        if (counts) {
          CHECK(v->outstanding(ctx) > before,
                "the greeting was built through the library's allocator");
        }

        /* The tree came from the LIBRARY's allocator, and this host frees
           it without ever naming that allocator: every container carries
           the one that made it. THIS IS THE CLAIM THE DESIGN RESTS ON. */
        guatiao_value_free(&greeting);
      }

      if (counts) {
        CHECK(v->outstanding(ctx) == before,
              "every block the greeting used came back: %lld before, %lld "
              "after",
              (long long)before, (long long)v->outstanding(ctx));
      }
    }
  }

  /* ---- the key template is this host's policy ------------------------ */

  st = guatiao_registry_keyed_by(reg, s("%id@%version"));
  CHECK(st == GUATIAO_OK, "keyed_by returned %d", (int)st);
  st = guatiao_registry_provider(reg, s("hello_library_almanac@1.0.0"), &alloc,
                                 &answer);
  CHECK(st == GUATIAO_OK, "the re-keyed provider is reachable by its new key");
  if (st == GUATIAO_OK) {
    guatiao_value_free(&answer);
  }

  /* A template naming a field no descriptor has is refused, and changes
     nothing: the old keys still work. */
  st = guatiao_registry_keyed_by(reg, s("%id@%revision"));
  CHECK(st != GUATIAO_OK, "an unknown field should be refused");
  st = guatiao_registry_provider(reg, s("hello_library_almanac@1.0.0"), &alloc,
                                 &answer);
  CHECK(st == GUATIAO_OK, "a refused template leaves the registry as it was");
  if (st == GUATIAO_OK) {
    guatiao_value_free(&answer);
  }

  guatiao_registry_free(reg);
  /* Freeing twice is not offered; freeing NULL is a no-op. */
  guatiao_registry_free(NULL);

  if (failures == 0) {
    printf("all checks passed\n");
    return 0;
  }
  printf("%d check(s) failed\n", failures);
  return 1;
}
