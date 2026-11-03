/*
 * A HOST, written in C, drawing a settings screen for every provider a
 * directory holds, before building any: it scans the directory, lists the
 * registry's catalog (kinds, id, name, version, the library's digest,
 * whether each can run here and why not, the configuration schema and
 * meta), asks guatiao_intake for each one's form (its own, or the one its
 * schema implies), and only then asks for one instance, whose refusal
 * says whether it was the first the library was asked for.
 *
 * argv[1] is the directory: derived_greeter and catalog_library.c, built.
 *
 * Prints "all checks passed" on success. Any failure returns non-zero
 * with a line saying which check and what it saw.
 */

#include <stdbool.h>
#include <stdio.h>
#include <string.h>

#include "guatiao.h"
#include "guatiao_intake.h"

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

static guatiao_str s(const char *lit) { return guatiao_cstr(lit); }

static bool text_is(const guatiao_value *v, const char *want) {
  return guatiao_str_eq(guatiao_string_text(v), s(want));
}

/* The catalog entry whose `id` is `id`, or NULL. */
static const guatiao_value *entry(const guatiao_value *catalog, const char *id) {
  guatiao_values all = guatiao_list_items(catalog);
  for (size_t i = 0; i < all.len; i++) {
    if (text_is(guatiao_map_find(&all.ptr[i], s("id")), id)) {
      return &all.ptr[i];
    }
  }
  return NULL;
}

/* The form `one` is shown with, written to `form`: ABSENT for none. */
static guatiao_status form_of(const guatiao_value *one, guatiao_alloc *alloc,
                              guatiao_value *form) {
  return guatiao_intake_provider_form(guatiao_map_find(one, s("config")),
                                      guatiao_map_find(one, s("meta")), alloc, form);
}

/* The id of the first section of `form`, or an empty view. */
static guatiao_str first_section(const guatiao_value *form) {
  guatiao_values sections = guatiao_list_items(guatiao_map_find(form, s("sections")));
  return sections.len ? guatiao_string_text(guatiao_map_find(&sections.ptr[0], s("id")))
                      : s("");
}

/* Whether `digest` reads `sha256:` and 64 lowercase hex digits. */
static bool is_digest(const guatiao_value *digest) {
  guatiao_str text = guatiao_string_text(digest);
  if (text.len != 71 || memcmp(text.ptr, "sha256:", 7) != 0) {
    return false;
  }
  for (size_t i = 7; i < text.len; i++) {
    uint8_t c = text.ptr[i];
    if (!((c >= '0' && c <= '9') || (c >= 'a' && c <= 'f'))) {
      return false;
    }
  }
  return true;
}

int main(int argc, char **argv) {
  guatiao_alloc alloc = GUATIAO_ALLOC_MALLOC;
  guatiao_value answer = {0};
  guatiao_value catalog = {0};
  guatiao_value form = {0};
  guatiao_status st;

  if (argc < 2) {
    printf("FAIL: usage: %s <directory>\n", argv[0]);
    return 2;
  }

  guatiao_registry *reg = guatiao_registry_new(s("catalog-host"), s("1.0"), &alloc);
  CHECK(reg != NULL, "a registry could not be created");
  if (reg == NULL) {
    return 1;
  }
  st = guatiao_registry_scan_dir(reg, s(argv[1]), false, &alloc, &answer);
  CHECK(st == GUATIAO_OK, "scan_dir returned %d", (int)st);
  CHECK(guatiao_list_items(guatiao_map_find(&answer, s("loaded"))).len == 2,
        "the scan did not load both libraries");
  guatiao_value_free(&answer);

  st = guatiao_registry_catalog(reg, &alloc, &catalog);
  CHECK(st == GUATIAO_OK, "catalog returned %d", (int)st);
  CHECK(guatiao_list_items(&catalog).len == 6, "expected six providers, saw %zu",
        guatiao_list_items(&catalog).len);

  /* ---- its own form, at a version of its own --------------------------- */

  const guatiao_value *one = entry(&catalog, "catalog_configured");
  CHECK(one != NULL, "catalog_configured is not listed");
  if (one != NULL) {
    guatiao_values kinds = guatiao_list_items(guatiao_map_find(one, s("kinds")));
    CHECK(kinds.len == 1 && text_is(&kinds.ptr[0], "counter"), "its kinds are not [counter]");
    CHECK(text_is(guatiao_map_find(one, s("display_name")), "Configured, with a form"),
          "its name is wrong");
    CHECK(text_is(guatiao_map_find(one, s("version")), "2.0"), "its own version is lost");
    CHECK(text_is(guatiao_map_find(one, s("library_version")), "1.0"),
          "its library's version is wrong");
    CHECK(text_is(guatiao_map_find(one, s("library")), "catalog_library"),
          "its library is wrong");
    CHECK(is_digest(guatiao_map_find(one, s("digest"))), "its library has no digest");
    CHECK(guatiao_bool_or(guatiao_map_find(one, s("available")), false),
          "it says it cannot run");
    CHECK(guatiao_map_find(one, s("reason")) == NULL, "an available one has a reason");
    const guatiao_value *schema = guatiao_map_find(one, s("config"));
    CHECK(text_is(guatiao_map_find(schema, s("type")), "object"), "no schema copy");

    st = form_of(one, &alloc, &form);
    CHECK(st == GUATIAO_OK && guatiao_tag_of(&form) == GUATIAO_MAP,
          "no form: %d, tag %u", (int)st, (unsigned)guatiao_tag_of(&form));
    CHECK(guatiao_str_eq(first_section(&form), s("net")), "its own form's section is lost");
    guatiao_value why = {0};
    st = guatiao_intake_check(schema, &form, &alloc, &why);
    CHECK(st == GUATIAO_OK, "its form does not fit its schema: %d", (int)st);
    guatiao_value_free(&why);
    guatiao_value_free(&form);
  }

  /* ---- no form: the one its schema implies ----------------------------- */

  one = entry(&catalog, "catalog_implied");
  CHECK(one != NULL, "catalog_implied is not listed");
  if (one != NULL) {
    CHECK(guatiao_map_find(one, s("meta")) == NULL, "it declares no meta");
    st = form_of(one, &alloc, &form);
    CHECK(st == GUATIAO_OK && guatiao_tag_of(&form) == GUATIAO_MAP,
          "no implied form: %d, tag %u", (int)st, (unsigned)guatiao_tag_of(&form));
    CHECK(guatiao_str_eq(first_section(&form), s("account")),
          "the implied form has no `account` section");
    const guatiao_value *token = guatiao_map_find(guatiao_map_find(&form, s("fields")), s("token"));
    CHECK(text_is(guatiao_map_find(token, s("widget")), "password"),
          "the secret is not masked");
    guatiao_value_free(&form);
  }

  /* ---- cannot run here, and nothing to configure ----------------------- */

  one = entry(&catalog, "catalog_unavailable");
  CHECK(one != NULL, "catalog_unavailable is not listed");
  if (one != NULL) {
    CHECK(!guatiao_bool_or(guatiao_map_find(one, s("available")), true),
          "it says it can run");
    CHECK(text_is(guatiao_map_find(one, s("reason")), "no device here"), "its reason is lost");
    CHECK(guatiao_map_find(one, s("config")) == NULL, "it takes no configuration");
    st = form_of(one, &alloc, &form);
    CHECK(st == GUATIAO_OK && guatiao_tag_of(&form) == GUATIAO_ABSENT,
          "nothing to configure answered %d, tag %u", (int)st, (unsigned)guatiao_tag_of(&form));
    guatiao_value_free(&form);
  }

  /* ---- the Rust library: the derive's form ----------------------------- */

  one = entry(&catalog, "derived_greeter_shouter");
  CHECK(one != NULL, "derived_greeter_shouter is not listed");
  if (one != NULL) {
    CHECK(is_digest(guatiao_map_find(one, s("digest"))), "its library has no digest");
    st = form_of(one, &alloc, &form);
    CHECK(st == GUATIAO_OK && guatiao_str_eq(first_section(&form), s("words")),
          "the derive's form is lost: %d", (int)st);
    guatiao_value_free(&form);
  }
  guatiao_value_free(&catalog);

  /* ---- and now one instance: the first the library was asked for ------- */

  guatiao_value config = {0};
  guatiao_value field = {0};
  st = guatiao_value_map(&alloc, &config);
  if (st == GUATIAO_OK) {
    st = guatiao_value_string(&alloc, s("localhost"), &field);
  }
  if (st == GUATIAO_OK) {
    st = guatiao_map_set(&alloc, &config, s("host"), &field);
  }
  if (st == GUATIAO_OK) {
    st = guatiao_value_number(&alloc, s("8080"), &field);
  }
  if (st == GUATIAO_OK) {
    st = guatiao_map_set(&alloc, &config, s("port"), &field);
  }
  CHECK(st == GUATIAO_OK, "the configuration could not be built: %d", (int)st);
  void *instance = NULL;
  guatiao_provider_error err;
  memset(&err, 0, sizeof(err));
  st = guatiao_registry_provider_create(reg, s("catalog_configured"), &config, &instance, &err);
  guatiao_str said = {err.message.ptr, err.message.len};
  CHECK(st == GUATIAO_ERR_UNSUPPORTED && instance == NULL, "create answered %d", (int)st);
  CHECK(guatiao_str_eq(said, s("built nothing: the first create this library was asked")),
        "listing asked for an instance: %.*s", (int)said.len, (const char *)said.ptr);
  if (err.message.cap && err.message.alloc) {
    err.message.alloc->free(err.message.alloc->ctx, err.message.ptr, err.message.cap, 1);
  }
  guatiao_value_free(&config);

  guatiao_registry_free(reg);
  if (failures) {
    printf("%d check(s) failed\n", failures);
    return 1;
  }
  printf("all checks passed\n");
  return 0;
}
