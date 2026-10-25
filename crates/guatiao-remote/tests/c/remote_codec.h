/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * The remote protocol's framing in C, as docs/remote.md states it: a
 * growing buffer, LEB128 and zigzag integers, text and bytes behind their
 * length, a frame's u32 little-endian length, and HMAC-SHA256 for the
 * trusted level's proof. Values inside a frame are guatiao's wire encoding
 * (guatiao_wire_encode / guatiao_wire_decode). Header-only.
 */

#ifndef GUATIAO_REMOTE_CODEC_H
#define GUATIAO_REMOTE_CODEC_H

#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

/* ---- writing --------------------------------------------------------- */

typedef struct gr_buf {
  uint8_t *p;
  size_t len;
  size_t cap;
} gr_buf;

static inline void gr_reserve(gr_buf *b, size_t more) {
  if (b->len + more <= b->cap) {
    return;
  }
  size_t cap = b->cap ? b->cap : 64;
  while (cap < b->len + more) {
    cap *= 2;
  }
  uint8_t *p = (uint8_t *)realloc(b->p, cap);
  if (!p) {
    abort();
  }
  b->p = p;
  b->cap = cap;
}

static inline void gr_raw(gr_buf *b, const void *p, size_t n) {
  gr_reserve(b, n);
  if (n) {
    memcpy(b->p + b->len, p, n);
  }
  b->len += n;
}

static inline void gr_byte(gr_buf *b, uint8_t x) { gr_raw(b, &x, 1); }

static inline void gr_uint(gr_buf *b, uint64_t n) {
  for (;;) {
    uint8_t byte = (uint8_t)(n & 0x7f);
    n >>= 7;
    if (n == 0) {
      gr_byte(b, byte);
      return;
    }
    gr_byte(b, (uint8_t)(byte | 0x80));
  }
}

static inline void gr_int(gr_buf *b, int64_t n) {
  uint64_t u = (uint64_t)n;
  gr_uint(b, (u << 1) ^ (n < 0 ? UINT64_MAX : 0));
}

static inline void gr_bytes(gr_buf *b, const void *p, size_t n) {
  gr_uint(b, (uint64_t)n);
  gr_raw(b, p, n);
}

static inline void gr_text(gr_buf *b, const char *s) { gr_bytes(b, s, strlen(s)); }

/* Starts a frame: four bytes for its length, then its type. */
static inline size_t gr_start(gr_buf *b, uint8_t kind) {
  size_t at = b->len;
  uint8_t head[5] = {0, 0, 0, 0, kind};
  gr_raw(b, head, sizeof head);
  return at;
}

/* Writes the length of the frame started at `at`. */
static inline void gr_finish(gr_buf *b, size_t at) {
  uint32_t n = (uint32_t)(b->len - at - 4);
  b->p[at] = (uint8_t)n;
  b->p[at + 1] = (uint8_t)(n >> 8);
  b->p[at + 2] = (uint8_t)(n >> 16);
  b->p[at + 3] = (uint8_t)(n >> 24);
}

static inline void gr_free(gr_buf *b) {
  free(b->p);
  b->p = NULL;
  b->len = b->cap = 0;
}

/* ---- reading --------------------------------------------------------- */

enum { GR_OK = 0, GR_TRUNCATED = 1, GR_OVERLONG = 2, GR_TRAILING = 3 };

typedef struct gr_reader {
  const uint8_t *p;
  size_t len;
  size_t at;
} gr_reader;

static inline int gr_get_byte(gr_reader *r, uint8_t *out) {
  if (r->at >= r->len) {
    return GR_TRUNCATED;
  }
  *out = r->p[r->at++];
  return GR_OK;
}

/* A LEB128 in its shortest form, at most 64 bits. */
static inline int gr_get_uint(gr_reader *r, uint64_t *out) {
  uint64_t n = 0;
  unsigned shift;
  for (shift = 0; shift < 64; shift += 7) {
    uint8_t byte;
    if (gr_get_byte(r, &byte) != GR_OK) {
      return GR_TRUNCATED;
    }
    uint64_t bits = (uint64_t)(byte & 0x7f);
    if (shift == 63 && bits > 1) {
      return GR_OVERLONG;
    }
    n |= bits << shift;
    if (!(byte & 0x80)) {
      if (byte == 0 && shift > 0) {
        return GR_OVERLONG;
      }
      *out = n;
      return GR_OK;
    }
  }
  return GR_OVERLONG;
}

static inline int gr_get_int(gr_reader *r, int64_t *out) {
  uint64_t z;
  int e = gr_get_uint(r, &z);
  if (e == GR_OK) {
    *out = (int64_t)(z >> 1) ^ -(int64_t)(z & 1);
  }
  return e;
}

/* Bytes behind their length, borrowed from the reader's buffer. */
static inline int gr_get_bytes(gr_reader *r, const uint8_t **p, size_t *n) {
  uint64_t len;
  int e = gr_get_uint(r, &len);
  if (e != GR_OK) {
    return e;
  }
  if (len > r->len - r->at) {
    return GR_TRUNCATED;
  }
  *p = r->p + r->at;
  *n = (size_t)len;
  r->at += (size_t)len;
  return GR_OK;
}

static inline int gr_get_fixed(gr_reader *r, uint8_t *out, size_t n) {
  if (n > r->len - r->at) {
    return GR_TRUNCATED;
  }
  memcpy(out, r->p + r->at, n);
  r->at += n;
  return GR_OK;
}

static inline int gr_end(const gr_reader *r) {
  return r->at == r->len ? GR_OK : GR_TRAILING;
}

/* ---- SHA-256 and HMAC (FIPS 180-4, RFC 2104) -------------------------- */

typedef struct gr_sha256 {
  uint32_t h[8];
  uint64_t bits;
  uint8_t block[64];
  size_t used;
} gr_sha256;

static const uint32_t GR_K[64] = {
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
    0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
    0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
    0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
    0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
    0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
    0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
    0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
    0xc67178f2};

#define GR_ROR(x, n) (((x) >> (n)) | ((x) << (32 - (n))))

static inline void gr_sha256_block(gr_sha256 *s, const uint8_t *p) {
  uint32_t w[64], a, b, c, d, e, f, g, h;
  int i;
  for (i = 0; i < 16; i++) {
    w[i] = (uint32_t)p[4 * i] << 24 | (uint32_t)p[4 * i + 1] << 16 |
           (uint32_t)p[4 * i + 2] << 8 | (uint32_t)p[4 * i + 3];
  }
  for (i = 16; i < 64; i++) {
    uint32_t s0 = GR_ROR(w[i - 15], 7) ^ GR_ROR(w[i - 15], 18) ^ (w[i - 15] >> 3);
    uint32_t s1 = GR_ROR(w[i - 2], 17) ^ GR_ROR(w[i - 2], 19) ^ (w[i - 2] >> 10);
    w[i] = w[i - 16] + s0 + w[i - 7] + s1;
  }
  a = s->h[0], b = s->h[1], c = s->h[2], d = s->h[3];
  e = s->h[4], f = s->h[5], g = s->h[6], h = s->h[7];
  for (i = 0; i < 64; i++) {
    uint32_t t1 = h + (GR_ROR(e, 6) ^ GR_ROR(e, 11) ^ GR_ROR(e, 25)) + ((e & f) ^ (~e & g)) +
                  GR_K[i] + w[i];
    uint32_t t2 = (GR_ROR(a, 2) ^ GR_ROR(a, 13) ^ GR_ROR(a, 22)) + ((a & b) ^ (a & c) ^ (b & c));
    h = g, g = f, f = e, e = d + t1, d = c, c = b, b = a, a = t1 + t2;
  }
  s->h[0] += a, s->h[1] += b, s->h[2] += c, s->h[3] += d;
  s->h[4] += e, s->h[5] += f, s->h[6] += g, s->h[7] += h;
}

static inline void gr_sha256_init(gr_sha256 *s) {
  static const uint32_t iv[8] = {0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
                                 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19};
  memcpy(s->h, iv, sizeof iv);
  s->bits = 0;
  s->used = 0;
}

static inline void gr_sha256_update(gr_sha256 *s, const void *data, size_t n) {
  const uint8_t *p = (const uint8_t *)data;
  s->bits += (uint64_t)n * 8;
  while (n) {
    size_t take = 64 - s->used < n ? 64 - s->used : n;
    memcpy(s->block + s->used, p, take);
    s->used += take, p += take, n -= take;
    if (s->used == 64) {
      gr_sha256_block(s, s->block);
      s->used = 0;
    }
  }
}

static inline void gr_sha256_final(gr_sha256 *s, uint8_t out[32]) {
  uint64_t bits = s->bits;
  uint8_t pad = 0x80, zero = 0, len[8];
  int i;
  gr_sha256_update(s, &pad, 1);
  while (s->used != 56) {
    gr_sha256_update(s, &zero, 1);
  }
  for (i = 0; i < 8; i++) {
    len[i] = (uint8_t)(bits >> (56 - 8 * i));
  }
  gr_sha256_update(s, len, 8);
  for (i = 0; i < 8; i++) {
    out[4 * i] = (uint8_t)(s->h[i] >> 24);
    out[4 * i + 1] = (uint8_t)(s->h[i] >> 16);
    out[4 * i + 2] = (uint8_t)(s->h[i] >> 8);
    out[4 * i + 3] = (uint8_t)s->h[i];
  }
}

/* HMAC-SHA256(key, label || a || b): the trusted level's proof. */
static inline void gr_proof(const uint8_t *key, size_t klen, const char *label,
                            const uint8_t a[16], const uint8_t b[16], uint8_t out[32]) {
  uint8_t k[64], pad[64], inner[32];
  gr_sha256 s;
  int i;
  memset(k, 0, sizeof k);
  if (klen > 64) {
    gr_sha256_init(&s);
    gr_sha256_update(&s, key, klen);
    gr_sha256_final(&s, k);
  } else if (klen) {
    memcpy(k, key, klen);
  }
  for (i = 0; i < 64; i++) {
    pad[i] = (uint8_t)(k[i] ^ 0x36);
  }
  gr_sha256_init(&s);
  gr_sha256_update(&s, pad, 64);
  gr_sha256_update(&s, label, strlen(label));
  gr_sha256_update(&s, a, 16);
  gr_sha256_update(&s, b, 16);
  gr_sha256_final(&s, inner);
  for (i = 0; i < 64; i++) {
    pad[i] = (uint8_t)(k[i] ^ 0x5c);
  }
  gr_sha256_init(&s);
  gr_sha256_update(&s, pad, 64);
  gr_sha256_update(&s, inner, 32);
  gr_sha256_final(&s, out);
}

#endif /* GUATIAO_REMOTE_CODEC_H */
