/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

/*
 * A document straight into a C struct, and a C struct straight into a
 * document, with no guatiao value in between.
 *
 * guatiao_json_visit hands a JSON document's events to a table of C
 * functions that fill the program's own struct; guatiao_toml_emit_walker
 * asks the program to walk its own struct into a visitor the library
 * provides and writes TOML. The same through the tape: the document as
 * wire bytes, walked with guatiao_wire_visit, and wire bytes written as
 * text with guatiao_json_from_wire.
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

/* The program's own data. */
typedef struct endpoint {
  char host[64];
  long port;
  double weight;
  int tags;
  bool verify;
} endpoint;

/* What the reader keeps between events: where it is, and the last key. */
typedef struct reader {
  endpoint *into;
  int depth;
  char key[32];
  bool in_tags;
  bool in_tls;
  int refuse_port; /* answer this status at the port, to test a stop */
} reader;

static void copy(char *dst, size_t cap, guatiao_str s) {
  size_t n = s.len < cap - 1 ? s.len : cap - 1;
  if (n) memcpy(dst, s.ptr, n);
  dst[n] = 0;
}

static bool key_is(reader *r, const char *k) { return strcmp(r->key, k) == 0; }

static guatiao_status on_absent(void *ctx) { (void)ctx; return GUATIAO_ERR_WRONG_KIND; }
static guatiao_status on_null(void *ctx) { (void)ctx; return GUATIAO_OK; }

static guatiao_status on_bool(void *ctx, bool v) {
  reader *r = ctx;
  if (r->in_tls && key_is(r, "verify")) r->into->verify = v;
  return GUATIAO_OK;
}

static guatiao_status on_number(void *ctx, guatiao_str text) {
  reader *r = ctx;
  char buf[64];
  copy(buf, sizeof buf, text);
  if (r->depth == 1 && key_is(r, "port")) {
    if (r->refuse_port) return (guatiao_status)r->refuse_port;
    r->into->port = strtol(buf, NULL, 10);
  } else if (r->depth == 1 && key_is(r, "weight")) {
    r->into->weight = strtod(buf, NULL);
  }
  return GUATIAO_OK;
}

static guatiao_status on_string(void *ctx, guatiao_str text) {
  reader *r = ctx;
  if (r->in_tags) r->into->tags++;
  else if (r->depth == 1 && key_is(r, "host")) copy(r->into->host, sizeof r->into->host, text);
  return GUATIAO_OK;
}

static guatiao_status on_bytes(void *ctx, guatiao_bytes b) { (void)ctx; (void)b; return GUATIAO_OK; }

static guatiao_status on_list_begin(void *ctx, uint64_t len) {
  reader *r = ctx;
  (void)len;
  if (r->depth == 1 && key_is(r, "tags")) r->in_tags = true;
  r->depth++;
  return GUATIAO_OK;
}

static guatiao_status on_list_end(void *ctx) {
  reader *r = ctx;
  r->depth--;
  r->in_tags = false;
  return GUATIAO_OK;
}

static guatiao_status on_map_begin(void *ctx, uint64_t len) {
  reader *r = ctx;
  (void)len;
  if (r->depth == 1 && key_is(r, "tls")) r->in_tls = true;
  r->depth++;
  return GUATIAO_OK;
}

static guatiao_status on_key(void *ctx, guatiao_str key) {
  reader *r = ctx;
  copy(r->key, sizeof r->key, key);
  return GUATIAO_OK;
}

static guatiao_status on_map_end(void *ctx) {
  reader *r = ctx;
  r->depth--;
  if (r->depth == 1) r->in_tls = false;
  return GUATIAO_OK;
}

static guatiao_visitor reader_of(reader *r) {
  guatiao_visitor v = {
      sizeof(guatiao_visitor), r,         on_absent,     on_null,     on_bool,
      on_number,               on_string, on_bytes,      on_list_begin, on_list_end,
      on_map_begin,            on_key,    on_map_end,
  };
  return v;
}

/* The program's struct, walked into the visitor the library hands it. */
static guatiao_status walk_endpoint(void *ctx, const guatiao_visitor *into) {
  const endpoint *e = ctx;
  char number[64];
  guatiao_status s;
#define DO(call) \
  if ((s = (call)) != GUATIAO_OK) return s
  DO(into->map_begin(into->ctx, 4));
  DO(into->key(into->ctx, guatiao_cstr("host")));
  DO(into->string(into->ctx, guatiao_cstr(e->host)));
  DO(into->key(into->ctx, guatiao_cstr("port")));
  snprintf(number, sizeof number, "%ld", e->port);
  DO(into->number(into->ctx, guatiao_cstr(number)));
  DO(into->key(into->ctx, guatiao_cstr("tags")));
  DO(into->list_begin(into->ctx, GUATIAO_LEN_UNKNOWN));
  DO(into->string(into->ctx, guatiao_cstr("a")));
  DO(into->string(into->ctx, guatiao_cstr("b")));
  DO(into->list_end(into->ctx));
  DO(into->key(into->ctx, guatiao_cstr("tls")));
  DO(into->map_begin(into->ctx, 1));
  DO(into->key(into->ctx, guatiao_cstr("verify")));
  DO(into->boolean(into->ctx, e->verify));
  DO(into->map_end(into->ctx));
  return into->map_end(into->ctx);
#undef DO
}

/* A walker that breaks the protocol: a key outside any map. */
static guatiao_status walk_badly(void *ctx, const guatiao_visitor *into) {
  (void)ctx;
  return into->key(into->ctx, guatiao_cstr("loose"));
}

static const char *DOCUMENT =
    "{\"host\": \"10.0.0.1\", \"port\": 5900, \"weight\": 1.5,\n"
    " \"tags\": [\"a\", \"b\", \"c\"], \"tls\": {\"verify\": true}}";

static void check_endpoint(const char *what, const endpoint *e) {
  CHECK(strcmp(e->host, "10.0.0.1") == 0, "%s: host %s", what, e->host);
  CHECK(e->port == 5900, "%s: port %ld", what, e->port);
  CHECK(e->weight == 1.5, "%s: weight %g", what, e->weight);
  CHECK(e->tags == 3, "%s: tags %d", what, e->tags);
  CHECK(e->verify, "%s: tls.verify", what);
}

static bool contains(guatiao_str haystack, const char *needle) {
  size_t n = strlen(needle);
  for (size_t i = 0; i + n <= haystack.len; i++) {
    if (memcmp(haystack.ptr + i, needle, n) == 0) return true;
  }
  return false;
}

int main(void) {
  guatiao_alloc alloc;
  CHECK(guatiao_alloc_default(&alloc) == GUATIAO_OK, "an allocator");

  /* JSON straight into the program's struct. */
  endpoint parsed = {0};
  reader r = {&parsed, 0, {0}, false, false, 0};
  guatiao_visitor v = reader_of(&r);
  guatiao_status s = guatiao_json_visit(guatiao_cstr(DOCUMENT), 0, &alloc, &v, NULL);
  CHECK(s == GUATIAO_OK, "json_visit: %u", (unsigned)s);
  check_endpoint("json_visit", &parsed);

  /* A callback's status stops the read and is what the call answers. */
  endpoint stopped = {0};
  reader rs = {&stopped, 0, {0}, false, false, GUATIAO_ERR_NOT_FOUND};
  guatiao_visitor vs = reader_of(&rs);
  s = guatiao_json_visit(guatiao_cstr(DOCUMENT), 0, &alloc, &vs, NULL);
  CHECK(s == GUATIAO_ERR_NOT_FOUND, "a callback's stop: %u", (unsigned)s);
  CHECK(stopped.tags == 0, "nothing after the stop");

  /* A document that does not parse says where. */
  guatiao_parse_error error;
  endpoint none = {0};
  reader rn = {&none, 0, {0}, false, false, 0};
  guatiao_visitor vn = reader_of(&rn);
  s = guatiao_json_visit(guatiao_cstr("{\"host\":\n  oops}"), 0, &alloc, &vn, &error);
  CHECK(s == GUATIAO_ERR_BAD_VALUE, "a bad document: %u", (unsigned)s);
  CHECK(error.format == GUATIAO_FORMAT_JSON && error.line == 2, "where: line %u", error.line);
  guatiao_value_free(&error.message);

  /* The program's struct straight into TOML. */
  endpoint mine = {"10.0.0.1", 5900, 0, 0, true};
  guatiao_walker w = {sizeof(guatiao_walker), &mine, walk_endpoint};
  guatiao_value toml;
  s = guatiao_toml_emit_walker(&w, 0, &alloc, &toml);
  CHECK(s == GUATIAO_OK, "toml_emit_walker: %u", (unsigned)s);
  guatiao_str text = guatiao_string_text(&toml);
  CHECK(contains(text, "host = \"10.0.0.1\""), "toml: host in %.*s", (int)text.len, text.ptr);
  CHECK(contains(text, "port = 5900"), "toml: port");
  CHECK(contains(text, "[tls]") && contains(text, "verify = true"), "toml: tls");
  guatiao_value_free(&toml);

  /* A walker that breaks the protocol is refused, `out` left absent. */
  guatiao_walker bad = {sizeof(guatiao_walker), NULL, walk_badly};
  guatiao_value refused;
  s = guatiao_json_emit_walker(&bad, 0, &alloc, &refused);
  CHECK(s == GUATIAO_ERR_WRONG_KIND, "a bad walker: %u", (unsigned)s);
  CHECK(guatiao_tag_of(&refused) == GUATIAO_ABSENT, "out stays absent");

  /* Through the tape: the document as wire bytes, walked; wire bytes as
   * text. */
  guatiao_value tape;
  s = guatiao_json_to_wire(guatiao_cstr(DOCUMENT), 0, &alloc, &tape, NULL);
  CHECK(s == GUATIAO_OK, "json_to_wire: %u", (unsigned)s);
  endpoint from_tape = {0};
  reader rt = {&from_tape, 0, {0}, false, false, 0};
  guatiao_visitor vt = reader_of(&rt);
  guatiao_bytes bytes = guatiao_bytes_data(&tape);
  s = guatiao_wire_visit(bytes, &vt);
  CHECK(s == GUATIAO_OK, "wire_visit: %u", (unsigned)s);
  check_endpoint("the tape", &from_tape);

  guatiao_value again;
  s = guatiao_json_from_wire(bytes, 0, &alloc, &again);
  CHECK(s == GUATIAO_OK, "json_from_wire: %u", (unsigned)s);
  guatiao_str json = guatiao_string_text(&again);
  CHECK(contains(json, "\"tags\":[\"a\",\"b\",\"c\"]"), "json_from_wire: %.*s", (int)json.len,
        json.ptr);
  guatiao_value_free(&again);
  guatiao_value_free(&tape);

  /* The program's struct as a value, and back into the struct. */
  guatiao_value built;
  s = guatiao_value_from_walker(&alloc, &w, &built);
  CHECK(s == GUATIAO_OK, "value_from_walker: %u", (unsigned)s);
  endpoint round = {0};
  reader rr = {&round, 0, {0}, false, false, 0};
  guatiao_visitor vr = reader_of(&rr);
  s = guatiao_value_visit(&built, &vr);
  CHECK(s == GUATIAO_OK, "value_visit: %u", (unsigned)s);
  CHECK(strcmp(round.host, "10.0.0.1") == 0 && round.port == 5900 && round.tags == 2 &&
            round.verify,
        "the struct came back");
  guatiao_value_free(&built);

  if (failures) {
    printf("%d check(s) failed\n", failures);
    return 1;
  }
  printf("all checks passed\n");
  return 0;
}
