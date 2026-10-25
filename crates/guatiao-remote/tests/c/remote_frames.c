/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * Every frame of tests/vectors/remote.json, encoded from its fields and
 * decoded from its bytes by remote_codec.h. `remote_vectors.h` is written
 * from the JSON by tests/c_peers.rs.
 */

#include <stdio.h>

#include "remote_codec.h"
#include "remote_vectors.h"

static int failures = 0;

static void fail(const char *name, const char *what) {
  printf("FAILED %s: %s\n", name, what);
  failures++;
}

static void encode(const gr_vector *v, gr_buf *b) {
  size_t at = gr_start(b, v->type);
  switch (v->type) {
  case 0x01:
    gr_raw(b, "GTRM", 4);
    gr_byte(b, 1);
    gr_byte(b, v->trust);
    gr_raw(b, v->nonce, 16);
    gr_bytes(b, v->options, v->options_len);
    break;
  case 0x02:
    gr_raw(b, "GTRM", 4);
    gr_byte(b, 1);
    gr_raw(b, v->nonce, 16);
    gr_bytes(b, v->blob, v->blob_len);
    gr_bytes(b, v->options, v->options_len);
    break;
  case 0x03:
    gr_bytes(b, v->blob, v->blob_len);
    break;
  case 0x04:
  case 0x12:
    gr_uint(b, v->call);
    break;
  case 0x05:
    gr_uint(b, v->call);
    gr_text(b, v->s1);
    gr_text(b, v->s2);
    break;
  case 0x06:
    gr_text(b, v->s1);
    gr_text(b, v->s2);
    break;
  case 0x10:
    gr_uint(b, v->call);
    gr_uint(b, v->parent);
    gr_uint(b, v->target_code);
    gr_uint(b, v->slot);
    gr_raw(b, v->blob, v->blob_len);
    break;
  case 0x11:
    gr_uint(b, v->call);
    gr_uint(b, v->status);
    gr_raw(b, v->blob, v->blob_len);
    break;
  case 0x13:
    gr_uint(b, v->object);
    break;
  case 0x14:
  case 0x15:
    gr_uint(b, v->n);
    break;
  case 0x16:
    gr_uint(b, v->status);
    gr_text(b, v->s1);
    break;
  default:
    break;
  }
  gr_finish(b, at);
}

static int same(const uint8_t *a, size_t an, const uint8_t *b, size_t bn) {
  return an == bn && (an == 0 || memcmp(a, b, an) == 0);
}

static int text_is(const uint8_t *p, size_t n, const char *s) {
  return same(p, n, (const uint8_t *)s, strlen(s));
}

static void decode(const gr_vector *v) {
  gr_reader r = {v->frame, v->frame_len, 0};
  uint8_t len[4], type, b8[16];
  uint64_t x;
  const uint8_t *p;
  size_t n;
  if (gr_get_fixed(&r, len, 4) || (uint32_t)(len[0] | len[1] << 8 | len[2] << 16 |
                                             (uint32_t)len[3] << 24) != v->frame_len - 4) {
    fail(v->name, "length");
    return;
  }
  if (gr_get_byte(&r, &type) || type != v->type) {
    fail(v->name, "type");
    return;
  }
  switch (type) {
  case 0x01:
  case 0x02:
    if (gr_get_fixed(&r, b8, 5) || memcmp(b8, "GTRM\x01", 5)) {
      fail(v->name, "magic and version");
      return;
    }
    if (type == 0x01) {
      uint8_t trust;
      if (gr_get_byte(&r, &trust) || trust != v->trust) {
        fail(v->name, "trust");
      }
    }
    if (gr_get_fixed(&r, b8, 16) || memcmp(b8, v->nonce, 16)) {
      fail(v->name, "nonce");
    }
    if (type == 0x02 && (gr_get_bytes(&r, &p, &n) || !same(p, n, v->blob, v->blob_len))) {
      fail(v->name, "proof");
    }
    if (gr_get_bytes(&r, &p, &n) || !same(p, n, v->options, v->options_len)) {
      fail(v->name, "options");
    }
    break;
  case 0x03:
    if (gr_get_bytes(&r, &p, &n) || !same(p, n, v->blob, v->blob_len)) {
      fail(v->name, "proof");
    }
    break;
  case 0x04:
  case 0x12:
    if (gr_get_uint(&r, &x) || x != v->call) {
      fail(v->name, "call");
    }
    break;
  case 0x05:
    if (gr_get_uint(&r, &x) || x != v->call) {
      fail(v->name, "call");
    }
    /* fall through to the two texts */
    if (gr_get_bytes(&r, &p, &n) || !text_is(p, n, v->s1)) {
      fail(v->name, "library");
    }
    if (gr_get_bytes(&r, &p, &n) || !text_is(p, n, v->s2)) {
      fail(v->name, "stamp");
    }
    break;
  case 0x06:
    if (gr_get_bytes(&r, &p, &n) || !text_is(p, n, v->s1)) {
      fail(v->name, "library");
    }
    if (gr_get_bytes(&r, &p, &n) || !text_is(p, n, v->s2)) {
      fail(v->name, "stamp");
    }
    break;
  case 0x10:
    if (gr_get_uint(&r, &x) || x != v->call) {
      fail(v->name, "call");
    }
    if (gr_get_uint(&r, &x) || x != v->parent) {
      fail(v->name, "parent");
    }
    if (gr_get_uint(&r, &x) || x != v->target_code) {
      fail(v->name, "target");
    }
    if (gr_get_uint(&r, &x) || x != v->slot) {
      fail(v->name, "slot");
    }
    if (!same(r.p + r.at, r.len - r.at, v->blob, v->blob_len)) {
      fail(v->name, "arguments");
    }
    r.at = r.len;
    break;
  case 0x11:
    if (gr_get_uint(&r, &x) || x != v->call) {
      fail(v->name, "call");
    }
    if (gr_get_uint(&r, &x) || x != v->status) {
      fail(v->name, "status");
    }
    if (!same(r.p + r.at, r.len - r.at, v->blob, v->blob_len)) {
      fail(v->name, "body");
    }
    r.at = r.len;
    break;
  case 0x13:
    if (gr_get_uint(&r, &x) || x != v->object) {
      fail(v->name, "object");
    }
    break;
  case 0x14:
  case 0x15:
    if (gr_get_uint(&r, &x) || x != v->n) {
      fail(v->name, "n");
    }
    break;
  case 0x16:
    if (gr_get_uint(&r, &x) || x != v->status) {
      fail(v->name, "status");
    }
    if (gr_get_bytes(&r, &p, &n) || !text_is(p, n, v->s1)) {
      fail(v->name, "message");
    }
    break;
  default:
    fail(v->name, "an unknown type in the vectors");
  }
  if (gr_end(&r) != GR_OK) {
    fail(v->name, "bytes after the last field");
  }
}

int main(void) {
  size_t i;
  for (i = 0; i < GR_VECTOR_COUNT; i++) {
    const gr_vector *v = &GR_VECTORS[i];
    gr_buf b = {NULL, 0, 0};
    encode(v, &b);
    if (!same(b.p, b.len, v->frame, v->frame_len)) {
      fail(v->name, "encoded bytes differ");
    }
    gr_free(&b);
    decode(v);
  }
  /* The trusted level's proofs, computed by the vectors' own encoder. */
  for (i = 0; i < GR_PROOF_COUNT; i++) {
    const gr_proof_vector *v = &GR_PROOFS[i];
    uint8_t mac[32];
    gr_proof(v->key, v->key_len, v->label, v->client, v->server, mac);
    if (memcmp(mac, v->proof, 32)) {
      fail(v->name, "the proof differs");
    }
  }
  if (failures) {
    printf("%d failures\n", failures);
    return 1;
  }
  printf("all %u frames match\n", (unsigned)GR_VECTOR_COUNT);
  return 0;
}
