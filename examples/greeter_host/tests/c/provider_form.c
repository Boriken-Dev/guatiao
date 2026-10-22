/*
 * A HOST, written in C, showing a provider's configuration the way the
 * provider asked: it loads a library, reads the provider's form out of its
 * `meta`, checks the form against the configuration schema, and asks which
 * fields to show for a given value.
 *
 * argv[1] is the library to load (the example greeters).
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

/* The value at `a.b` of a map: two lookups, NULL when either misses. */
static const guatiao_value *at(const guatiao_value *map, const char *a,
                               const char *b) {
  const guatiao_value *first = guatiao_map_find(map, s(a));
  return first ? guatiao_map_find(first, s(b)) : NULL;
}

/* Whether the field `key` is shown when `exclaim` holds `exclaim`. */
static guatiao_status shown(const guatiao_value *schema,
                            const guatiao_value *form, const char *key,
                            bool exclaim, bool *out) {
  guatiao_alloc alloc = GUATIAO_ALLOC_MALLOC;
  guatiao_value values = {0};
  guatiao_value flag = {0};
  guatiao_status st = guatiao_value_map(&alloc, &values);
  if (st == GUATIAO_OK) {
    st = guatiao_value_bool(exclaim, &flag);
  }
  if (st == GUATIAO_OK) {
    st = guatiao_map_set(&alloc, &values, s("exclaim"), &flag);
  }
  if (st == GUATIAO_OK) {
    st = guatiao_intake_is_visible(schema, form, s(key), &values, out);
  }
  guatiao_value_free(&values);
  return st;
}

int main(int argc, char **argv) {
  guatiao_alloc alloc = GUATIAO_ALLOC_MALLOC;
  guatiao_value answer = {0};
  guatiao_value meta = {0};
  guatiao_status st;

  if (argc < 2) {
    printf("FAIL: usage: %s <library>\n", argv[0]);
    return 2;
  }

  guatiao_registry *reg = guatiao_registry_new(s("form-host"), s("1.0"), &alloc);
  CHECK(reg != NULL, "a registry could not be created");
  if (reg == NULL) {
    return 1;
  }
  st = guatiao_registry_load_file(reg, s(argv[1]), &alloc, &answer);
  CHECK(st == GUATIAO_OK, "load_file returned %d", (int)st);
  guatiao_value_free(&answer);

  /* ---- the form, out of the provider's meta ----------------------------- */

  st = guatiao_registry_provider_meta(reg, s("derived_greeter_shouter"), &alloc,
                                      &meta);
  CHECK(st == GUATIAO_OK, "provider_meta returned %d", (int)st);
  const guatiao_value *form =
      guatiao_map_find(&meta, s(GUATIAO_INTAKE_PROVIDER_FORM_KEY));
  CHECK(form != NULL, "the provider's meta carries no form");

  /* Borrowed from the registry: read, never freed. */
  const guatiao_value *schema =
      guatiao_registry_provider_config(reg, s("derived_greeter_shouter"));
  CHECK(schema != NULL, "the provider declares no configuration schema");

  if (form != NULL && schema != NULL) {
    guatiao_value why = {0};
    st = guatiao_intake_check(schema, form, &alloc, &why);
    CHECK(st == GUATIAO_OK, "the form does not fit the schema: %d (%.*s)",
          (int)st, (int)guatiao_string_text(guatiao_map_find(&why, s("message"))).len,
          guatiao_string_text(guatiao_map_find(&why, s("message"))).ptr);
    guatiao_value_free(&why);

    /* Two sections, in display order, each with its label. */
    guatiao_values sections = guatiao_list_items(guatiao_map_find(form, s("sections")));
    CHECK(sections.len == 2, "expected two sections, saw %zu", sections.len);
    if (sections.len == 2) {
      CHECK(text_is(guatiao_map_find(&sections.ptr[0], s("id")), "words"),
            "the first section is not `words`");
      CHECK(text_is(guatiao_map_find(&sections.ptr[1], s("title")), "Volume"),
            "the second section is not labelled `Volume`");
    }

    /* Hints, by the field's path. */
    const guatiao_value *fields = guatiao_map_find(form, s("fields"));
    CHECK(text_is(at(fields, "prefix", "placeholder"), "hey"),
          "`prefix` has no placeholder");
    CHECK(text_is(at(fields, "marks", "unit"), "marks"), "`marks` has no unit");
    const guatiao_value *when = at(fields, "marks", "visibleWhen");
    CHECK(text_is(guatiao_map_find(when, s("field")), "exclaim"),
          "`marks` does not wait on `exclaim`");
    CHECK(guatiao_bool_or(guatiao_map_find(when, s("equals")), false),
          "`marks` is not shown on `exclaim` = true");

    /* What the condition means, asked of the library rather than read. */
    bool visible = true;
    st = shown(schema, form, "marks", false, &visible);
    CHECK(st == GUATIAO_OK && !visible, "`marks` shown without `exclaim`");
    st = shown(schema, form, "marks", true, &visible);
    CHECK(st == GUATIAO_OK && visible, "`marks` hidden with `exclaim`");
  }
  guatiao_value_free(&meta);

  /* ---- a provider with no meta, and a key nothing answers to ----------- */

  st = guatiao_registry_provider_meta(reg, s("derived_greeter_hello"), &alloc, &meta);
  CHECK(st == GUATIAO_OK && guatiao_tag_of(&meta) == GUATIAO_ABSENT,
        "a provider without meta answered %d, tag %u", (int)st,
        (unsigned)guatiao_tag_of(&meta));
  guatiao_value_free(&meta);
  st = guatiao_registry_provider_meta(reg, s("nobody"), &alloc, &meta);
  CHECK(st == GUATIAO_ERR_NOT_FOUND, "an unknown key answered %d", (int)st);
  guatiao_value_free(&meta);

  guatiao_registry_free(reg);
  if (failures) {
    printf("%d check(s) failed\n", failures);
    return 1;
  }
  printf("all checks passed\n");
  return 0;
}
