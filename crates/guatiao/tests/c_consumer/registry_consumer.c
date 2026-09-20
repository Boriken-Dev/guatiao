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

/* The directory `path` is in, or "." when it names no directory. */
static void dir_of(const char *path, char *out, size_t cap) {
  size_t n = strlen(path);
  while (n > 0 && path[n - 1] != '/' && path[n - 1] != '\\') {
    n--;
  }
  if (n == 0) {
    snprintf(out, cap, ".");
    return;
  }
  if (n >= cap) {
    n = cap - 1;
  }
  memcpy(out, path, n);
  out[n] = 0;
}

/* Whether `text` ends with `tail`. */
static bool ends_with(guatiao_str text, const char *tail) {
  size_t n = strlen(tail);
  return text.len >= n && memcmp(text.ptr + text.len - n, tail, n) == 0;
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
      CHECK(guatiao_int_or(guatiao_map_find(loaded, s("providers")), -1) == 4,
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

  /* ---- a scan with rules keeps a library out BEFORE mapping it ------ */

  {
    char dir[4096];
    const char *base = argv[1] + strlen(argv[1]);
    dir_of(argv[1], dir, sizeof dir);
    while (base > argv[1] && base[-1] != '/' && base[-1] != '\\') {
      base--;
    }

    /* A rule that is not one is refused, not applied. */
    st = guatiao_registry_scan_dir_rules(reg, s(dir), false, s("nonsense"),
                                         &alloc, &answer);
    CHECK(st == GUATIAO_ERR_BAD_VALUE, "a bad rule returned %d", (int)st);

    /* The library declares kind=greeter and HELLO_EXAMPLE=1; a host that
       wants none of that writes one rule and the file is never mapped
       (it is already loaded here, and the report says `filtered`, not
       `already-loaded`, because the rule runs first). The second rule
       keeps every other library in the directory out too, so this
       registry stays exactly what the checks below expect. */
    st = guatiao_registry_scan_dir_rules(reg, s(dir), false,
                                         s("!HELLO_EXAMPLE=1\nkind=nonesuch\n"),
                                         &alloc, &answer);
    CHECK(st == GUATIAO_OK, "scan_dir_rules returned %d", (int)st);
    if (st == GUATIAO_OK) {
      guatiao_values loaded =
          guatiao_list_items(guatiao_map_find(&answer, s("loaded")));
      CHECK(loaded.len == 0, "nothing passes both rules, saw %zu loaded",
            loaded.len);
      guatiao_values skipped =
          guatiao_list_items(guatiao_map_find(&answer, s("skipped")));
      bool seen = false;
      for (size_t i = 0; i < skipped.len; i++) {
        const guatiao_value *one = &skipped.ptr[i];
        if (ends_with(field(one, "path"), base)) {
          seen = true;
          CHECK(text_is(field(one, "skipped"), "filtered"),
                "the library should be filtered by its declaration");
          CHECK(text_is(field(one, "by"), "!HELLO_EXAMPLE=1"),
                "a filtered skip names the rule");
        }
      }
      CHECK(seen, "the scan report names the filtered library");
      guatiao_value_free(&answer);
    }
  }

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
      CHECK(ks.len == 3, "expected three kinds, saw %zu", ks.len);
      if (ks.len == 3) {
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

  /* ---- ranking: the host's priority decides the order ---------------- */

  /* Unranked, the key decides: almanac, greeter, sundial. */
  st = guatiao_registry_providers(reg, s("everything"), &alloc, &answer);
  CHECK(st == GUATIAO_OK, "providers(everything) returned %d", (int)st);
  {
    guatiao_values items = guatiao_list_items(&answer);
    CHECK(items.len == 3, "expected three, saw %zu", items.len);
    if (items.len == 3) {
      CHECK(text_is(field(&items.ptr[0], "id"), "hello_library_almanac") &&
                text_is(field(&items.ptr[2], "id"), "hello_library_sundial"),
            "unranked providers come back by key");
    }
  }
  guatiao_value_free(&answer);

  /* Ranked, the raised one goes first -- in this listing, not only in
     `available`. */
  st = guatiao_registry_set_priority(reg, s("hello_library_sundial"), 10);
  CHECK(st == GUATIAO_OK, "set_priority returned %d", (int)st);
  CHECK(guatiao_registry_priority(reg, s("hello_library_sundial")) == 10,
        "the rank reads back");
  st = guatiao_registry_providers(reg, s("everything"), &alloc, &answer);
  CHECK(st == GUATIAO_OK, "providers(everything) returned %d", (int)st);
  {
    guatiao_values items = guatiao_list_items(&answer);
    CHECK(items.len == 3 &&
              text_is(field(&items.ptr[0], "id"), "hello_library_sundial"),
          "a raised provider is listed first");
  }
  guatiao_value_free(&answer);
  /* And the empty kind lists everything, in the same order. */
  st = guatiao_registry_providers(reg, s(""), &alloc, &answer);
  CHECK(st == GUATIAO_OK &&
            guatiao_list_items(&answer).len == 4 &&
            text_is(field(&guatiao_list_items(&answer).ptr[0], "id"),
                    "hello_library_sundial"),
        "an empty kind lists every provider, ranked");
  guatiao_value_free(&answer);
  st = guatiao_registry_set_priority(reg, s("hello_library_sundial"), 0);
  CHECK(st == GUATIAO_OK, "set_priority returned %d", (int)st);

  /* ---- what a library sees of this host ----------------------------- */

  {
    /* The block a library keeps. A host driving a library by hand passes
       this to guatiao_library_entry; here it is read the way a library
       would read it. */
    const guatiao_host_info *host = guatiao_registry_host(reg);
    CHECK(host != NULL, "the registry hands out a host block");
    if (host != NULL) {
      CHECK(host->struct_size >= sizeof(guatiao_host_info),
            "the block declares this build's size");
      CHECK(guatiao_str_eq(host->host_id, s("c-host")), "the host's own id");
      CHECK(host->services != NULL, "a registry offers services");
      if (host->services != NULL && host->services->list != NULL) {
        size_t total = 0;
        st = host->services->list(host->services->ctx, s("greeter"), NULL, 0,
                                  &total);
        CHECK(st == GUATIAO_OK && total == 1, "one greeter, counted");
        const guatiao_provider_info *found[4] = {0};
        size_t written = 0;
        st = host->services->list(host->services->ctx, s(""), found, 4,
                                  &written);
        CHECK(st == GUATIAO_OK && written == 4, "every provider listed");
        const guatiao_provider_info *one = NULL;
        st = host->services->get(host->services->ctx,
                                 s("hello_library_greeter"), &one);
        CHECK(st == GUATIAO_OK && one != NULL &&
                  guatiao_str_eq(one->id, s("hello_library_greeter")),
              "get hands back the library's own descriptor");
        CHECK(host->services->alloc(host->services->ctx) == &alloc,
              "the same allocator the registry was given");
      }
    }
  }

  /* ---- the provider that declares no kind at all --------------------- */

  st = guatiao_registry_provider(reg, s("hello_library_almanac"), &alloc,
                                 &answer);
  CHECK(st == GUATIAO_OK, "provider by key returned %d", (int)st);
  CHECK(text_is(field(&answer, "version"), "1.0.0"),
        "a provider that declares its own version keeps it");
  CHECK(guatiao_list_items(guatiao_map_find(&answer, s("kinds"))).len == 1,
        "the almanac serves the kind every provider here shares");
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
    /* Borrowed from the library's image: read it, never free it.

       A schema IS a JSON Schema, so a C consumer walks the keys the
       specification already names -- `properties` keyed by the field's
       own name -- with the header's own map helpers and no library
       call. */
    if (schema) {
      CHECK(text_is(field(schema, "type"), "object"),
            "a schema is an object schema");
      const guatiao_value *properties =
          guatiao_map_find(schema, s("properties"));
      CHECK(guatiao_map_entries(properties).len == 1,
            "the schema declares one field");
      CHECK(guatiao_map_find(properties, s("name")) != NULL,
            "and its name is the key it is filed under");
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

  /* ---- taking a library back out ------------------------------------ */

  /* Both take the LIBRARY key -- the template `libraries_keyed_by` sets,
     `%id` by default -- not a provider key. The re-key above moved the
     PROVIDER keys and left this one alone. */
  st = guatiao_registry_retire(reg, s("nonesuch"));
  CHECK(st == GUATIAO_ERR_NOT_FOUND, "retiring an unknown library returned %d",
        (int)st);
  st = guatiao_registry_retire(reg, s("hello_library_greeter"));
  CHECK(st == GUATIAO_ERR_NOT_FOUND, "a provider key is not a library key");

  st = guatiao_registry_retire(reg, s("hello_library"));
  CHECK(st == GUATIAO_OK, "retire returned %d", (int)st);
  st = guatiao_registry_provider(reg, s("hello_library_almanac@1.0.0"), &alloc,
                                 &answer);
  CHECK(st == GUATIAO_ERR_NOT_FOUND, "its providers left with it");
  CHECK(guatiao_registry_provider_config(reg, s("hello_library_greeter")) ==
            NULL,
        "and the schema the registry held for them");

  /* A retired library is not "already loaded": its key is free. */
  st = guatiao_registry_load_file(reg, s(argv[1]), &alloc, &answer);
  CHECK(st == GUATIAO_OK, "load_file after a retire returned %d", (int)st);
  if (st == GUATIAO_OK) {
    CHECK(guatiao_map_find(&answer, s("loaded")) != NULL,
          "a retired library loads again rather than being skipped");
    guatiao_value_free(&answer);
  }

  /* And unloading it: the library is asked first, and agrees here because
     the greeting above was freed. Nothing this host still holds came from
     it. */
  st = guatiao_registry_unload(reg, s("hello_library"));
  CHECK(st == GUATIAO_OK, "unload returned %d", (int)st);
  st = guatiao_registry_unload(reg, s("hello_library"));
  CHECK(st == GUATIAO_ERR_NOT_FOUND, "it left with the first call");

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
