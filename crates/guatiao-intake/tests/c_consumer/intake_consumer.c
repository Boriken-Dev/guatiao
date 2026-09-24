/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

/*
 * Checking, laying out and evaluating a form from C.
 *
 * This is the claim: a program that links no Rust holds a schema and a
 * form as ordinary literal trees, asks whether the form fits, gets the
 * fields back grouped and in order, and asks whether a field is showing --
 * with the same answers the Rust API gives.
 *
 * Prints "all checks passed" on success. Any failure returns non-zero with
 * a line saying which check and what it saw.
 */

#include <stdio.h>

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

/* ---- the schema: host and port in "net", verify and ca ungrouped ---- */

static guatiao_entry HOST_S[] = {
    GUATIAO_ENTRY_LIT("type", GUATIAO_VALUE_STRING_LIT("string")),
    GUATIAO_ENTRY_LIT("x-section", GUATIAO_VALUE_STRING_LIT("net")),
};
static guatiao_entry PORT_S[] = {
    GUATIAO_ENTRY_LIT("type", GUATIAO_VALUE_STRING_LIT("integer")),
    GUATIAO_ENTRY_LIT("x-section", GUATIAO_VALUE_STRING_LIT("net")),
    GUATIAO_ENTRY_LIT("x-order", GUATIAO_VALUE_NUMBER_LIT("1")),
};
static guatiao_entry VERIFY_S[] = {
    GUATIAO_ENTRY_LIT("type", GUATIAO_VALUE_STRING_LIT("boolean")),
};
static guatiao_entry CA_S[] = {
    GUATIAO_ENTRY_LIT("type", GUATIAO_VALUE_STRING_LIT("string")),
};
static guatiao_entry PROPERTIES[] = {
    GUATIAO_ENTRY_LIT("host", GUATIAO_VALUE_MAP_LIT(HOST_S)),
    GUATIAO_ENTRY_LIT("port", GUATIAO_VALUE_MAP_LIT(PORT_S)),
    GUATIAO_ENTRY_LIT("verify", GUATIAO_VALUE_MAP_LIT(VERIFY_S)),
    GUATIAO_ENTRY_LIT("ca", GUATIAO_VALUE_MAP_LIT(CA_S)),
};
static guatiao_entry SCHEMA_E[] = {
    GUATIAO_ENTRY_LIT("type", GUATIAO_VALUE_STRING_LIT("object")),
    GUATIAO_ENTRY_LIT("properties", GUATIAO_VALUE_MAP_LIT(PROPERTIES)),
};
static const guatiao_value SCHEMA = GUATIAO_VALUE_MAP_LIT(SCHEMA_E);

/* ---- the form: "net" is called Network; ca shows while verify is true - */

static guatiao_entry NET[] = {
    GUATIAO_ENTRY_LIT("id", GUATIAO_VALUE_STRING_LIT("net")),
    GUATIAO_ENTRY_LIT("title", GUATIAO_VALUE_STRING_LIT("Network")),
};
static guatiao_value SECTIONS[] = {GUATIAO_VALUE_MAP_LIT(NET)};
static guatiao_entry PORT_H[] = {
    GUATIAO_ENTRY_LIT("widget", GUATIAO_VALUE_STRING_LIT("number")),
};
static guatiao_entry WHEN_VERIFIED[] = {
    GUATIAO_ENTRY_LIT("field", GUATIAO_VALUE_STRING_LIT("verify")),
    GUATIAO_ENTRY_LIT("equals", GUATIAO_VALUE_BOOL_LIT(1)),
};
static guatiao_entry CA_H[] = {
    GUATIAO_ENTRY_LIT("placeholder", GUATIAO_VALUE_STRING_LIT("/etc/ssl/ca.pem")),
    GUATIAO_ENTRY_LIT("visibleWhen", GUATIAO_VALUE_MAP_LIT(WHEN_VERIFIED)),
};
static guatiao_entry FIELDS[] = {
    GUATIAO_ENTRY_LIT("port", GUATIAO_VALUE_MAP_LIT(PORT_H)),
    GUATIAO_ENTRY_LIT("ca", GUATIAO_VALUE_MAP_LIT(CA_H)),
};
static guatiao_entry FORM_E[] = {
    GUATIAO_ENTRY_LIT("sections", GUATIAO_VALUE_LIST_LIT(SECTIONS)),
    GUATIAO_ENTRY_LIT("fields", GUATIAO_VALUE_MAP_LIT(FIELDS)),
};
static const guatiao_value FORM = GUATIAO_VALUE_MAP_LIT(FORM_E);

/* ---- a form naming a field the schema does not declare ---------------- */

static guatiao_entry TYPO_H[] = {
    GUATIAO_ENTRY_LIT("widget", GUATIAO_VALUE_STRING_LIT("text")),
};
static guatiao_entry TYPO_FIELDS[] = {
    GUATIAO_ENTRY_LIT("hots", GUATIAO_VALUE_MAP_LIT(TYPO_H)),
};
static guatiao_entry TYPO_E[] = {
    GUATIAO_ENTRY_LIT("fields", GUATIAO_VALUE_MAP_LIT(TYPO_FIELDS)),
};
static const guatiao_value TYPO_FORM = GUATIAO_VALUE_MAP_LIT(TYPO_E);

/* ---- what a person has entered ------------------------------------------ */

static guatiao_entry VERIFY_OFF[] = {
    GUATIAO_ENTRY_LIT("verify", GUATIAO_VALUE_BOOL_LIT(0)),
};
static guatiao_entry VERIFY_ON[] = {
    GUATIAO_ENTRY_LIT("verify", GUATIAO_VALUE_BOOL_LIT(1)),
};
static const guatiao_value ENTERED_OFF = GUATIAO_VALUE_MAP_LIT(VERIFY_OFF);
static const guatiao_value ENTERED_ON = GUATIAO_VALUE_MAP_LIT(VERIFY_ON);

int main(void) {
  guatiao_alloc alloc = GUATIAO_ALLOC_MALLOC;
  guatiao_status st;

  /* ---- check ----------------------------------------------------------- */

  guatiao_value detail = {0};
  st = guatiao_intake_check(&SCHEMA, &FORM, &alloc, &detail);
  CHECK(st == GUATIAO_OK, "a form that fits its schema checks, saw status %d", (int)st);

  st = guatiao_intake_check(&SCHEMA, &TYPO_FORM, &alloc, &detail);
  CHECK(st == GUATIAO_ERR_BAD_VALUE, "a misspelled field is refused, saw %d", (int)st);
  if (st == GUATIAO_ERR_BAD_VALUE) {
    CHECK(text_is(guatiao_map_find(&detail, s("kind")), "unknown_field"),
          "the detail says what kind of mistake");
    CHECK(text_is(guatiao_map_find(&detail, s("path")), "hots"),
          "and which path");
    CHECK(guatiao_string_text(guatiao_map_find(&detail, s("message"))).len > 0,
          "and says it in a sentence");
    guatiao_value_free(&detail);
  }

  st = guatiao_intake_check(NULL, &FORM, &alloc, NULL);
  CHECK(st == GUATIAO_ERR_NULL, "a null schema is refused rather than read");

  /* ---- layout ------------------------------------------------------------ */

  guatiao_value groups = {0};
  st = guatiao_intake_layout(&SCHEMA, &FORM, &alloc, &groups);
  CHECK(st == GUATIAO_OK, "layout, saw status %d", (int)st);
  if (st == GUATIAO_OK) {
    guatiao_values g = guatiao_list_items(&groups);
    CHECK(g.len == 2, "the default group and Network, saw %zu groups", g.len);
    if (g.len == 2) {
      /* The default group first, holding the ungrouped fields in order. */
      const guatiao_value *first = guatiao_map_find(&g.ptr[0], s("section"));
      CHECK(first != NULL && first->tag == GUATIAO_NULL,
            "the default group has no declared section");
      guatiao_values f0 = guatiao_list_items(guatiao_map_find(&g.ptr[0], s("fields")));
      CHECK(f0.len == 2 && text_is(&f0.ptr[0], "verify") && text_is(&f0.ptr[1], "ca"),
            "verify then ca, in the schema's order");

      /* Then Network, with the explicitly ordered port ahead of host. */
      const guatiao_value *net = guatiao_map_find(&g.ptr[1], s("section"));
      CHECK(text_is(guatiao_map_find(net, s("title")), "Network"),
            "the second group is the declared section, as the form wrote it");
      guatiao_values f1 = guatiao_list_items(guatiao_map_find(&g.ptr[1], s("fields")));
      CHECK(f1.len == 2 && text_is(&f1.ptr[0], "port") && text_is(&f1.ptr[1], "host"),
            "port (x-order 1) before host");
    }
    guatiao_value_free(&groups);
  }

  /* ---- visibility --------------------------------------------------------- */

  bool shown = true;
  st = guatiao_intake_is_visible(&SCHEMA, &FORM, s("ca"), &ENTERED_OFF, &shown);
  CHECK(st == GUATIAO_OK && !shown, "ca is hidden while verify is false");

  shown = false;
  st = guatiao_intake_is_visible(&SCHEMA, &FORM, s("ca"), &ENTERED_ON, &shown);
  CHECK(st == GUATIAO_OK && shown, "ca shows once verify is true");

  shown = false;
  st = guatiao_intake_is_visible(&SCHEMA, &FORM, s("host"), &ENTERED_OFF, &shown);
  CHECK(st == GUATIAO_OK && shown, "a field with no condition shows");

  st = guatiao_intake_is_visible(&SCHEMA, &FORM, s("ca"), &ENTERED_ON, NULL);
  CHECK(st == GUATIAO_ERR_NULL, "a null answer slot is refused rather than written");

  if (failures) {
    printf("%d check(s) failed\n", failures);
    return 1;
  }
  printf("all checks passed\n");
  return 0;
}
