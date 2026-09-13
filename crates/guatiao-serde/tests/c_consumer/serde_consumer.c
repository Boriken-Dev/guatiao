/*
 * Loading and dumping a value from C, in three formats.
 *
 * This is the claim: a program that links no Rust reads a configuration
 * file into a guatiao value, walks it with the header's own inline
 * readers, writes it back out in another format, and frees everything
 * with the one free function it already knew about.
 *
 * Prints "all checks passed" on success. Any failure returns non-zero
 * with a line saying which check and what it saw.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "guatiao_serde.h"

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

/* Everything the document says, however it was written down. */
static void check_shape(const char *what, const guatiao_value *v) {
  const guatiao_value *name = guatiao_map_find(v, s("name"));
  const guatiao_value *port = guatiao_map_find(v, s("port"));
  const guatiao_value *hosts = guatiao_map_find(v, s("hosts"));
  const guatiao_value *tls = guatiao_map_find(v, s("tls"));

  CHECK(guatiao_str_eq(guatiao_string_text(name), s("example")), "%s: name", what);
  CHECK(guatiao_int_or(port, -1) == 5900, "%s: port", what);
  CHECK(guatiao_list_items(hosts).len == 2, "%s: hosts", what);
  CHECK(tls != NULL && guatiao_bool_or(guatiao_map_find(tls, s("verify")), false),
        "%s: tls.verify", what);
}

int main(void) {
  guatiao_alloc alloc = GUATIAO_ALLOC_MALLOC;
  guatiao_value doc = {0};
  guatiao_value text = {0};
  guatiao_status st;

  /* ---- load a JSON document ------------------------------------------ */

  const char *json =
      "{\"name\":\"example\",\"port\":5900,"
      "\"hosts\":[\"alpha\",\"beta\"],"
      "\"tls\":{\"verify\":true,\"timeout\":30}}";

  st = guatiao_json_parse(s(json), 0, &alloc, &doc);
  CHECK(st == GUATIAO_OK, "json_parse returned %d", (int)st);
  check_shape("json", &doc);

  /* ---- dump it back out, and the answer is a VALUE ------------------- */

  st = guatiao_json_emit(&doc, 0, &alloc, &text);
  CHECK(st == GUATIAO_OK, "json_emit returned %d", (int)st);
  CHECK(guatiao_tag_of(&text) == (uint32_t)GUATIAO_STRING,
        "emitting answers a string value, freed like any other tree");
  {
    /* And it reads back as the same document. */
    guatiao_value again = {0};
    guatiao_str written = guatiao_string_text(&text);
    st = guatiao_json_parse(written, 0, &alloc, &again);
    CHECK(st == GUATIAO_OK, "the text we just wrote parses");
    check_shape("json round trip", &again);
    guatiao_value_free(&again);
  }
  guatiao_value_free(&text);

  /* ---- the same value, in the other two formats ---------------------- */

  st = guatiao_toml_emit(&doc, 0, &alloc, &text);
  CHECK(st == GUATIAO_OK, "toml_emit returned %d", (int)st);
  {
    guatiao_value again = {0};
    st = guatiao_toml_parse(guatiao_string_text(&text), 0, &alloc, &again);
    CHECK(st == GUATIAO_OK, "toml_parse returned %d", (int)st);
    check_shape("toml", &again);
    guatiao_value_free(&again);
  }
  guatiao_value_free(&text);

  st = guatiao_yaml_emit(&doc, 0, &alloc, &text);
  CHECK(st == GUATIAO_OK, "yaml_emit returned %d", (int)st);
  {
    guatiao_value again = {0};
    st = guatiao_yaml_parse(guatiao_string_text(&text), 0, &alloc, &again);
    CHECK(st == GUATIAO_OK, "yaml_parse returned %d", (int)st);
    check_shape("yaml", &again);
    guatiao_value_free(&again);
  }
  guatiao_value_free(&text);

  /* ---- a TOML document is a TABLE ------------------------------------ */

  {
    guatiao_value bare = GUATIAO_VALUE_STRING_LIT("not a table");
    st = guatiao_toml_emit(&bare, 0, &alloc, &text);
    CHECK(st == GUATIAO_ERR_WRONG_KIND,
          "a bare scalar is refused rather than wrapped, got %d", (int)st);
  }

  /* ---- the policy is one integer ------------------------------------- */

  {
    uint8_t raw[] = {0xde, 0xad};
    guatiao_value bytes = GUATIAO_VALUE_BYTES_LIT(raw);

    st = guatiao_json_emit(&bytes, GUATIAO_BYTES_ARRAY, &alloc, &text);
    CHECK(st == GUATIAO_OK, "emit with a policy returned %d", (int)st);
    CHECK(guatiao_str_eq(guatiao_string_text(&text), s("[222,173]")),
          "bytes as an array");
    guatiao_value_free(&text);

    st = guatiao_json_emit(&bytes, GUATIAO_BYTES_DATA_URI, &alloc, &text);
    CHECK(st == GUATIAO_OK && guatiao_str_eq(guatiao_string_text(&text),
                                             s("\"data:;base64,3q0=\"")),
          "bytes as a data uri, which is the default");
    guatiao_value_free(&text);

    st = guatiao_json_emit(&bytes, GUATIAO_BYTES_REFUSE, &alloc, &text);
    CHECK(st != GUATIAO_OK, "a refusal is a refusal");
  }

  /* ---- a number keeps its spelling ----------------------------------- */

  {
    guatiao_value back = {0};
    st = guatiao_json_parse(s("{\"ratio\":1.10}"), 0, &alloc, &back);
    CHECK(st == GUATIAO_OK, "parse returned %d", (int)st);
    CHECK(guatiao_str_eq(
              guatiao_number_text(guatiao_map_find(&back, s("ratio"))),
              s("1.10")),
          "a number is its exact text, not an f64's idea of it");
    guatiao_value_free(&back);
  }

  /* ---- malformed input is refused, not guessed ----------------------- */

  {
    guatiao_value nope = {0};
    st = guatiao_json_parse(s("{ this is not json"), 0, &alloc, &nope);
    CHECK(st != GUATIAO_OK, "malformed JSON is refused");
    st = guatiao_json_parse(s("{} and then some"), 0, &alloc, &nope);
    CHECK(st != GUATIAO_OK, "trailing rubbish is refused");
  }

  /* A null out-parameter is a status, never a fault. */
  CHECK(guatiao_json_parse(s("{}"), 0, &alloc, NULL) == GUATIAO_ERR_NULL,
        "a null out parameter answers rather than faults");
  CHECK(guatiao_json_emit(NULL, 0, &alloc, &text) == GUATIAO_ERR_NULL,
        "a null value answers rather than faults");

  guatiao_value_free(&doc);

  if (failures == 0) {
    printf("all checks passed\n");
    return 0;
  }
  printf("%d check(s) failed\n", failures);
  return 1;
}
