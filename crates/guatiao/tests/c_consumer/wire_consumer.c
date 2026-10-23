/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * Channels from C: every frame of tests/vectors/wire.json built with
 * guatiao_wire_announce, _send and _close, and its conversation run
 * through one guatiao_wire_receiver, each refusal checked for its status,
 * its code, its offset and its words.
 *
 * `wire_vectors.h` is generated from the vector file by the test that
 * compiles this file, so C reads the vectors Rust and every binding read.
 *
 * Prints "all checks passed" on success. Any failure returns non-zero
 * with a line saying which vector and what it saw.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "guatiao.h"
#include "wire_vectors.h"

static int failures = 0;
static size_t checked = 0;

#define CHECK(cond, ...)                          \
  do {                                            \
    if (!(cond)) {                                \
      printf("FAIL %s:%d: ", __FILE__, __LINE__); \
      printf(__VA_ARGS__);                        \
      printf("\n");                               \
      failures++;                                 \
    }                                             \
  } while (0)

static guatiao_alloc ALLOC = GUATIAO_ALLOC_MALLOC;

static int nibble(char c) {
  return c >= 'a' ? c - 'a' + 10 : c - '0';
}

/* Hex text into `out`, answering the byte count. */
static size_t unhex(const char *hex, uint8_t *out, size_t cap) {
  size_t n = strlen(hex) / 2;
  size_t i;
  if (n > cap) {
    printf("FAIL: a vector longer than %zu bytes\n", cap);
    exit(2);
  }
  for (i = 0; i < n; i++) {
    out[i] = (uint8_t)(nibble(hex[2 * i]) << 4 | nibble(hex[2 * i + 1]));
  }
  return n;
}

/* Whether `v` is BYTES holding exactly what `hex` spells. */
static int holds(const guatiao_value *v, const char *hex) {
  uint8_t want[1024];
  size_t n = unhex(hex, want, sizeof want);
  guatiao_bytes got = guatiao_bytes_data(v);
  return guatiao_tag_of(v) == GUATIAO_BYTES && got.len == n &&
         (n == 0 || memcmp(got.ptr, want, n) == 0);
}

/* Whether the wire encoding of `v` is what `hex` spells. */
static int encodes_to(const guatiao_value *v, const char *hex) {
  guatiao_value bytes;
  int same;
  if (guatiao_wire_encode(&ALLOC, v, &bytes) != GUATIAO_OK) {
    return 0;
  }
  same = holds(&bytes, hex);
  guatiao_value_free(&bytes);
  return same;
}

/* Whether STRING `v` contains `needle`. */
static int contains(const guatiao_value *v, const char *needle) {
  guatiao_str s = guatiao_string_text(v);
  size_t n = strlen(needle);
  size_t i;
  for (i = 0; n <= s.len && i + n <= s.len; i++) {
    if (memcmp(s.ptr + i, needle, n) == 0) {
      return 1;
    }
  }
  return 0;
}

static void frames(void) {
  size_t i;
  for (i = 0; i < FRAME_COUNT; i++) {
    const frame_vector *f = &FRAME_VECTORS[i];
    uint8_t payload[1024];
    guatiao_value value;
    guatiao_value frame;
    guatiao_status status;
    size_t n = unhex(f->payload, payload, sizeof payload);
    if (f->build == 2) {
      status = guatiao_wire_close(&ALLOC, f->channel, &frame);
    } else {
      CHECK(guatiao_wire_decode(&ALLOC, guatiao_bytes_from(payload, n), &value) == GUATIAO_OK,
            "frame %zu: its payload decodes", i);
      status = f->build == 0 ? guatiao_wire_announce(&ALLOC, f->channel, &value, &frame)
                             : guatiao_wire_send(&ALLOC, f->channel, &value, &frame);
      guatiao_value_free(&value);
    }
    CHECK(status == GUATIAO_OK, "frame %zu: status %u", i, (unsigned)status);
    CHECK(holds(&frame, f->frame), "frame %zu: the documented bytes", i);
    guatiao_value_free(&frame);
    checked++;
  }
}

static void conversation(void) {
  size_t i;
  guatiao_wire_receiver *recv = guatiao_wire_receiver_new(&ALLOC);
  CHECK(recv != NULL, "a receiver");
  CHECK(guatiao_wire_receiver_new(NULL) == NULL, "no allocator, no receiver");
  for (i = 0; i < STEP_COUNT; i++) {
    const step_vector *s = &STEP_VECTORS[i];
    uint8_t frame[1024];
    guatiao_wire_event event;
    size_t n = unhex(s->frame, frame, sizeof frame);
    guatiao_status status =
        guatiao_wire_receiver_accept(recv, guatiao_bytes_from(frame, n), &event);
    CHECK(status == s->status, "step %zu: status %u, wanted %u", i, (unsigned)status,
          s->status);
    CHECK(event.refusal == s->refusal, "step %zu: refusal %u, wanted %u", i, event.refusal,
          s->refusal);
    CHECK(event.channel == s->channel, "step %zu: channel %llu", i,
          (unsigned long long)event.channel);
    if (s->kind >= 0) {
      CHECK(event.kind == (uint32_t)s->kind, "step %zu: kind %u", i, event.kind);
    }
    CHECK(event.at == s->at, "step %zu: at %zu, wanted %zu", i, event.at, s->at);
    if (s->tree) {
      CHECK(encodes_to(&event.value, s->tree), "step %zu: the value is the vector's", i);
    } else {
      CHECK(guatiao_tag_of(&event.value) == GUATIAO_ABSENT, "step %zu: no value", i);
    }
    if (s->refusal == GUATIAO_WIRE_ACCEPTED) {
      CHECK(guatiao_tag_of(&event.message) == GUATIAO_ABSENT, "step %zu: no words", i);
    } else {
      CHECK(guatiao_string_text(&event.message).len > 0, "step %zu: the refusal's words", i);
      CHECK(!contains(&event.message, "hunter2"), "step %zu: the words quote no value", i);
    }
    if (s->key) {
      const guatiao_value *key = guatiao_map_find(&event.detail, guatiao_cstr("key"));
      CHECK(key != NULL && guatiao_str_eq(guatiao_string_text(key), guatiao_cstr(s->key)),
            "step %zu: the refused field is %s", i, s->key);
    } else {
      CHECK(guatiao_tag_of(&event.detail) == GUATIAO_ABSENT, "step %zu: no detail", i);
    }
    guatiao_value_free(&event.value);
    guatiao_value_free(&event.message);
    guatiao_value_free(&event.detail);
    checked++;
  }
  guatiao_wire_receiver_free(recv);
  guatiao_wire_receiver_free(NULL);
}

int main(void) {
  frames();
  conversation();
  if (failures) {
    printf("%d failures in %zu vectors\n", failures, checked);
    return 1;
  }
  printf("all checks passed (%zu vectors)\n", checked);
  return 0;
}
