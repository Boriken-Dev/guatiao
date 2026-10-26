/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "glue.h"



#define GT_DEPTH 128

static void set_absent(guatiao_value *v) { memset(v, 0, sizeof *v); }

static guatiao_status failf(char *msg, size_t n, guatiao_status st, const char *fmt, const char *arg) {
  snprintf(msg, n, fmt, arg ? arg : "");
  return st;
}

/* The shortest text that reads back as `d`. */
static void double_text(double d, char *buf, size_t n) {
  /* A whole number below 1e21 is written as the integer it is exactly. */
  if (d == floor(d) && fabs(d) < 1e21) {
    snprintf(buf, n, "%.0f", d);
    if (d == 0 && signbit(d)) snprintf(buf, n, "-0");
    return;
  }
  for (int p = 1; p <= 17; p++) {
    snprintf(buf, n, "%.*g", p, d);
    if (strtod(buf, NULL) == d) return;
  }
}

/* ---- building: one walk over R data, two sinks ------------------------------------ */

/* What the walk over R data hands its events to, in order: a list's or
 * map's opening with its count, its items (each of a map's after its
 * key), then its closing. Text and keys arrive as NUL-terminated UTF-8,
 * numbers as their text; each sink refuses what it cannot carry, writing
 * why into `msg`. */
typedef struct gt_sink gt_sink;
typedef struct gt_sink_ops {
  guatiao_status (*absent)(gt_sink *s);
  guatiao_status (*null)(gt_sink *s);
  guatiao_status (*boolean)(gt_sink *s, int b);
  guatiao_status (*number)(gt_sink *s, const char *text);
  guatiao_status (*text)(gt_sink *s, const char *text);
  guatiao_status (*bytes)(gt_sink *s, const uint8_t *p, size_t n);
  guatiao_status (*node)(gt_sink *s, const guatiao_value *src);
  guatiao_status (*open)(gt_sink *s, int is_map, R_xlen_t n);
  guatiao_status (*key)(gt_sink *s, const char *key);
  guatiao_status (*close)(gt_sink *s);
} gt_sink_ops;

struct gt_sink {
  const gt_sink_ops *ops;
  char *msg;
  size_t n;
};

static guatiao_status emit_rec(SEXP x, gt_sink *s, int depth);

/* Element i of an atomic vector, as a scalar. */
static guatiao_status scalar(SEXP x, R_xlen_t i, gt_sink *s) {
  char buf[64];
  switch (TYPEOF(x)) {
  case LGLSXP: {
    int b = LOGICAL(x)[i];
    if (b == NA_LOGICAL) return s->ops->null(s);
    return s->ops->boolean(s, b != 0);
  }
  case INTSXP: {
    int v = INTEGER(x)[i];
    if (v == NA_INTEGER) return s->ops->null(s);
    snprintf(buf, sizeof buf, "%d", v);
    return s->ops->number(s, buf);
  }
  case REALSXP: {
    double d = REAL(x)[i];
    if (ISNA(d)) return s->ops->null(s);
    if (!R_FINITE(d)) return failf(s->msg, s->n, GUATIAO_ERR_BAD_VALUE, "%sa value holds no NaN or infinity", NULL);
    double_text(d, buf, sizeof buf);
    return s->ops->number(s, buf);
  }
  case STRSXP: {
    SEXP c = STRING_ELT(x, i);
    if (c == NA_STRING) return s->ops->null(s);
    return s->ops->text(s, Rf_translateCharUTF8(c));
  }
  default: return failf(s->msg, s->n, GUATIAO_ERR_BAD_VALUE, "cannot write a %s", Rf_type2char(TYPEOF(x)));
  }
}

static int all_named(SEXP names, R_xlen_t len) {
  if (names == R_NilValue || XLENGTH(names) != len) return 0;
  for (R_xlen_t i = 0; i < len; i++) {
    SEXP c = STRING_ELT(names, i);
    if (c == NA_STRING || CHAR(c)[0] == '\0') return -1;
  }
  return 1;
}

/* An R vector (atomic or list) as a map (named) or a list. */
static guatiao_status sequence(SEXP x, gt_sink *s, int depth, int force_list) {
  R_xlen_t len = XLENGTH(x);
  SEXP names = Rf_getAttrib(x, R_NamesSymbol);
  int named = force_list ? 0 : all_named(names, len);
  if (named < 0 && len > 0)
    return failf(s->msg, s->n, GUATIAO_ERR_BAD_VALUE, "%sa list is either named in full (a map) or not at all", NULL);
  if (named < 0) named = 1;
  int is_map = named || (len == 0 && names != R_NilValue && !force_list);
  if (is_map && len > 1) {
    R_xlen_t dup = Rf_any_duplicated(names, FALSE);
    if (dup > 0)
      return failf(s->msg, s->n, GUATIAO_ERR_BAD_VALUE, "two entries are named '%s'",
                   Rf_translateCharUTF8(STRING_ELT(names, dup - 1)));
  }
  int is_list = TYPEOF(x) == VECSXP;
  guatiao_status st = s->ops->open(s, is_map, len);
  if (st) return st;
  for (R_xlen_t i = 0; i < len; i++) {
    if (is_map) {
      SEXP key = STRING_ELT(names, i);
      if (key == NA_STRING) return failf(s->msg, s->n, GUATIAO_ERR_BAD_VALUE, "%sa map key is not NA", NULL);
      st = s->ops->key(s, Rf_translateCharUTF8(key));
      if (st) return st;
    }
    st = is_list ? emit_rec(VECTOR_ELT(x, i), s, depth + 1) : scalar(x, i, s);
    if (st) return st;
  }
  return s->ops->close(s);
}

/* `.gt_lower(x)` in R, protected: list(TRUE, data) or list(FALSE, message). */
static SEXP lower_safely(SEXP x) {
  SEXP f = PROTECT(Rf_install(".gt_lower_safely"));
  SEXP call = PROTECT(Rf_lang2(f, x));
  int failed = 0;
  SEXP r = R_tryEvalSilent(call, gt_namespace(), &failed);
  UNPROTECT(2);
  return failed ? R_NilValue : r;
}

static guatiao_status emit_rec(SEXP x, gt_sink *s, int depth) {
  if (depth > GT_DEPTH) return failf(s->msg, s->n, GUATIAO_ERR_BAD_VALUE, "%sdeeper than 128 nested containers", NULL);
  if (x == R_NilValue) return s->ops->null(s);
  if (Rf_isObject(x)) {
    if (Rf_inherits(x, "guatiao_node")) {
      gt_owner *o = NULL;
      guatiao_value *src = NULL;
      int r = gt_node_try(x, &o, &src);
      if (r == 1) return failf(s->msg, s->n, GUATIAO_ERR_GONE, "%sthe value is closed", NULL);
      if (r) return failf(s->msg, s->n, GUATIAO_ERR_NOT_FOUND, "%sthe view no longer addresses a node", NULL);
      return s->ops->node(s, src);
    }
    if (Rf_inherits(x, "guatiao_absent")) return s->ops->absent(s);
    if (Rf_inherits(x, "guatiao_number")) {
      if (TYPEOF(x) != STRSXP || XLENGTH(x) != 1 || STRING_ELT(x, 0) == NA_STRING)
        return failf(s->msg, s->n, GUATIAO_ERR_BAD_VALUE, "%sa number is one text", NULL);
      return s->ops->number(s, Rf_translateCharUTF8(STRING_ELT(x, 0)));
    }
    if (Rf_inherits(x, "AsIs") && (Rf_isVectorAtomic(x) || TYPEOF(x) == VECSXP) && TYPEOF(x) != RAWSXP)
      return sequence(x, s, depth, 1);
    SEXP r = PROTECT(lower_safely(x));
    if (r == R_NilValue) {
      UNPROTECT(1);
      return failf(s->msg, s->n, GUATIAO_ERR_INTERNAL, "%swriting an object failed", NULL);
    }
    if (!LOGICAL(VECTOR_ELT(r, 0))[0]) {
      const char *why = Rf_translateCharUTF8(STRING_ELT(VECTOR_ELT(r, 1), 0));
      snprintf(s->msg, s->n, "%s", why);
      UNPROTECT(1);
      return GUATIAO_ERR_BAD_VALUE;
    }
    guatiao_status st = emit_rec(VECTOR_ELT(r, 1), s, depth + 1);
    UNPROTECT(1);
    return st;
  }
  switch (TYPEOF(x)) {
  case RAWSXP: return s->ops->bytes(s, XLENGTH(x) ? RAW(x) : NULL, (size_t)XLENGTH(x));
  case LGLSXP:
  case INTSXP:
  case REALSXP:
  case STRSXP:
    if (XLENGTH(x) == 1 && Rf_getAttrib(x, R_NamesSymbol) == R_NilValue) return scalar(x, 0, s);
    return sequence(x, s, depth, 0);
  case VECSXP: return sequence(x, s, depth, 0);
  default: return failf(s->msg, s->n, GUATIAO_ERR_BAD_VALUE, "cannot write a %s", Rf_type2char(TYPEOF(x)));
  }
}

/* ---- the tree sink: each item built in its container's scratch node, then moved in */

typedef struct tree_frame {
  guatiao_value *node;
  guatiao_value child;
  int is_map;
  const char *key;
} tree_frame;

typedef struct tree_sink {
  gt_sink base;
  const guatiao_alloc *a;
  guatiao_value *root;
  int depth;
  tree_frame frames[GT_DEPTH + 2];
} tree_sink;

static guatiao_value *tree_target(tree_sink *t) { return t->depth ? &t->frames[t->depth - 1].child : t->root; }

/* Moves what the target holds into the innermost open container. */
static guatiao_status tree_placed(tree_sink *t, guatiao_status st, const char *fmt, const char *arg) {
  if (st) return failf(t->base.msg, t->base.n, st, fmt, arg);
  if (!t->depth) return GUATIAO_OK;
  tree_frame *f = &t->frames[t->depth - 1];
  if (f->is_map) {
    size_t before = guatiao_map_entries(f->node).len;
    st = gt_core.guatiao_map_set(t->a, f->node, guatiao_str_from(f->key, strlen(f->key)), &f->child);
    if (st) {
      gt_free(&f->child);
      return failf(t->base.msg, t->base.n, st, "the key '%s' was refused", f->key);
    }
    if (guatiao_map_entries(f->node).len == before)
      return failf(t->base.msg, t->base.n, GUATIAO_ERR_BAD_VALUE, "two entries are named '%s'", f->key);
  } else {
    st = gt_core.guatiao_list_push(t->a, f->node, &f->child);
    if (st) {
      gt_free(&f->child);
      return failf(t->base.msg, t->base.n, st, "%sa list refused an element", NULL);
    }
  }
  set_absent(&f->child);
  return GUATIAO_OK;
}

static guatiao_status tree_absent(gt_sink *s) { return tree_placed((tree_sink *)s, GUATIAO_OK, "%s", NULL); }
static guatiao_status tree_null(gt_sink *s) {
  tree_sink *t = (tree_sink *)s;
  return tree_placed(t, gt_core.guatiao_value_null(tree_target(t)), "%sthe allocator refused", NULL);
}
static guatiao_status tree_bool(gt_sink *s, int b) {
  tree_sink *t = (tree_sink *)s;
  return tree_placed(t, gt_core.guatiao_value_bool(b, tree_target(t)), "%sthe allocator refused", NULL);
}
static guatiao_status tree_number(gt_sink *s, const char *text) {
  tree_sink *t = (tree_sink *)s;
  return tree_placed(t, gt_core.guatiao_value_number(t->a, guatiao_str_from(text, strlen(text)), tree_target(t)),
                     "'%s' is not a number", text);
}
static guatiao_status tree_text(gt_sink *s, const char *text) {
  tree_sink *t = (tree_sink *)s;
  return tree_placed(t, gt_core.guatiao_value_string(t->a, guatiao_str_from(text, strlen(text)), tree_target(t)),
                     "text that is not UTF-8%s", NULL);
}
static guatiao_status tree_bytes(gt_sink *s, const uint8_t *p, size_t n) {
  tree_sink *t = (tree_sink *)s;
  return tree_placed(t, gt_core.guatiao_value_bytes(t->a, guatiao_bytes_from(p, n), tree_target(t)),
                     "%sthe allocator refused", NULL);
}
static guatiao_status tree_node(gt_sink *s, const guatiao_value *src) {
  tree_sink *t = (tree_sink *)s;
  return tree_placed(t, gt_core.guatiao_value_clone(t->a, src, tree_target(t)), "%sthe copy was refused", NULL);
}
static guatiao_status tree_open(gt_sink *s, int is_map, R_xlen_t n) {
  (void)n;
  tree_sink *t = (tree_sink *)s;
  guatiao_value *into = tree_target(t);
  guatiao_status st = is_map ? gt_core.guatiao_value_map(t->a, into) : gt_core.guatiao_value_list(t->a, into);
  if (st) return failf(s->msg, s->n, st, "%sthe allocator refused", NULL);
  tree_frame *f = &t->frames[t->depth++];
  f->node = into;
  set_absent(&f->child);
  f->is_map = is_map;
  f->key = NULL;
  return GUATIAO_OK;
}
static guatiao_status tree_key(gt_sink *s, const char *key) {
  tree_sink *t = (tree_sink *)s;
  t->frames[t->depth - 1].key = key;
  return GUATIAO_OK;
}
static guatiao_status tree_close(gt_sink *s) {
  tree_sink *t = (tree_sink *)s;
  t->depth--;
  return tree_placed(t, GUATIAO_OK, "%s", NULL);
}

static const gt_sink_ops tree_ops = {tree_absent, tree_null, tree_bool,  tree_number, tree_text,
                                     tree_bytes,  tree_node, tree_open, tree_key,    tree_close};

guatiao_status gt_build(SEXP x, const guatiao_alloc *a, guatiao_value *out, char *msg, size_t msglen) {
  gt_need(GT_LIB_CORE);
  msg[0] = '\0';
  set_absent(out);
  tree_sink t;
  t.base.ops = &tree_ops;
  t.base.msg = msg;
  t.base.n = msglen;
  t.a = a;
  t.root = out;
  t.depth = 0;
  guatiao_status st = emit_rec(x, &t.base, 0);
  if (st) {
    /* The items still in scratch, innermost first, then what was moved in. */
    for (int i = t.depth - 1; i >= 0; i--) gt_free(&t.frames[i].child);
    gt_free(out);
    set_absent(out);
  }
  return st;
}

/* ---- the wire sink: the same events as wire bytes, in memory R_alloc keeps for the call */

typedef struct wire_sink {
  gt_sink base;
  uint8_t *buf;
  size_t len, cap;
} wire_sink;

static void wire_room(wire_sink *w, size_t more) {
  if (w->len + more <= w->cap) return;
  size_t cap = w->cap ? w->cap * 2 : 256;
  while (cap < w->len + more) cap *= 2;
  uint8_t *grown = (uint8_t *)R_alloc(cap, 1);
  if (w->len) memcpy(grown, w->buf, w->len);
  w->buf = grown;
  w->cap = cap;
}

static void wire_byte(wire_sink *w, uint8_t b) {
  wire_room(w, 1);
  w->buf[w->len++] = b;
}

static void wire_leb(wire_sink *w, uint64_t n) {
  wire_room(w, 10);
  while (n >= 0x80) {
    w->buf[w->len++] = (uint8_t)(n | 0x80);
    n >>= 7;
  }
  w->buf[w->len++] = (uint8_t)n;
}

static void wire_body(wire_sink *w, const void *p, size_t n) {
  wire_leb(w, n);
  wire_room(w, n);
  if (n) memcpy(w->buf + w->len, p, n);
  w->len += n;
}

/* Whether `p` is UTF-8 (RFC 3629: no overlongs, no surrogates, at most U+10FFFF). */
static int utf8_valid(const uint8_t *p, size_t n) {
  size_t i = 0;
  while (i < n) {
    uint8_t c = p[i];
    if (c < 0x80) {
      i++;
      continue;
    }
    size_t k;
    uint32_t cp;
    if ((c & 0xE0) == 0xC0) k = 1, cp = c & 0x1F;
    else if ((c & 0xF0) == 0xE0) k = 2, cp = c & 0x0F;
    else if ((c & 0xF8) == 0xF0) k = 3, cp = c & 0x07;
    else return 0;
    if (i + k >= n) return 0;
    for (size_t j = 1; j <= k; j++) {
      if ((p[i + j] & 0xC0) != 0x80) return 0;
      cp = (cp << 6) | (p[i + j] & 0x3F);
    }
    if ((k == 1 && cp < 0x80) || (k == 2 && cp < 0x800) || (k == 3 && cp < 0x10000) || cp > 0x10FFFF ||
        (cp >= 0xD800 && cp <= 0xDFFF))
      return 0;
    i += k + 1;
  }
  return 1;
}

/* Whether `t` is in the JSON number grammar (RFC 8259, section 6). */
static int number_grammar(const char *t) {
  const char *p = t;
  if (*p == '-') p++;
  if (*p == '0') p++;
  else if (*p >= '1' && *p <= '9')
    while (*p >= '0' && *p <= '9') p++;
  else return 0;
  if (*p == '.') {
    p++;
    if (!(*p >= '0' && *p <= '9')) return 0;
    while (*p >= '0' && *p <= '9') p++;
  }
  if (*p == 'e' || *p == 'E') {
    p++;
    if (*p == '+' || *p == '-') p++;
    if (!(*p >= '0' && *p <= '9')) return 0;
    while (*p >= '0' && *p <= '9') p++;
  }
  return *p == '\0';
}

static guatiao_status wire_absent(gt_sink *s) {
  wire_byte((wire_sink *)s, GUATIAO_ABSENT);
  return GUATIAO_OK;
}
static guatiao_status wire_null(gt_sink *s) {
  wire_byte((wire_sink *)s, GUATIAO_NULL);
  return GUATIAO_OK;
}
static guatiao_status wire_bool(gt_sink *s, int b) {
  wire_byte((wire_sink *)s, GUATIAO_BOOL);
  wire_byte((wire_sink *)s, b ? 1 : 0);
  return GUATIAO_OK;
}
static guatiao_status wire_number(gt_sink *s, const char *text) {
  if (!number_grammar(text)) return failf(s->msg, s->n, GUATIAO_ERR_BAD_VALUE, "'%s' is not a number", text);
  wire_byte((wire_sink *)s, GUATIAO_NUMBER);
  wire_body((wire_sink *)s, text, strlen(text));
  return GUATIAO_OK;
}
static guatiao_status wire_text(gt_sink *s, const char *text) {
  size_t n = strlen(text);
  if (!utf8_valid((const uint8_t *)text, n))
    return failf(s->msg, s->n, GUATIAO_ERR_BAD_VALUE, "text that is not UTF-8%s", NULL);
  wire_byte((wire_sink *)s, GUATIAO_STRING);
  wire_body((wire_sink *)s, text, n);
  return GUATIAO_OK;
}
static guatiao_status wire_bytes(gt_sink *s, const uint8_t *p, size_t n) {
  wire_byte((wire_sink *)s, GUATIAO_BYTES);
  wire_body((wire_sink *)s, p, n);
  return GUATIAO_OK;
}
static guatiao_status wire_node(gt_sink *s, const guatiao_value *src) {
  gt_need(GT_LIB_CORE);
  guatiao_value out;
  set_absent(&out);
  guatiao_status st = GT_CORE_FN(guatiao_wire_encode)(&gt_binding_alloc, src, &out);
  if (st) {
    gt_free(&out);
    return failf(s->msg, s->n, st, "%sthe value has a node the wire encoding cannot carry", NULL);
  }
  guatiao_bytes b = guatiao_bytes_data(&out);
  wire_room((wire_sink *)s, b.len);
  if (b.len) memcpy(((wire_sink *)s)->buf + ((wire_sink *)s)->len, b.ptr, b.len);
  ((wire_sink *)s)->len += b.len;
  gt_free(&out);
  return GUATIAO_OK;
}
static guatiao_status wire_open(gt_sink *s, int is_map, R_xlen_t n) {
  wire_byte((wire_sink *)s, is_map ? GUATIAO_MAP : GUATIAO_LIST);
  wire_leb((wire_sink *)s, (uint64_t)n);
  return GUATIAO_OK;
}
static guatiao_status wire_key(gt_sink *s, const char *key) {
  size_t n = strlen(key);
  if (!utf8_valid((const uint8_t *)key, n)) return failf(s->msg, s->n, GUATIAO_ERR_BAD_VALUE, "the key '%s' was refused", key);
  wire_body((wire_sink *)s, key, n);
  return GUATIAO_OK;
}
static guatiao_status wire_close(gt_sink *s) {
  (void)s;
  return GUATIAO_OK;
}

static const gt_sink_ops wire_ops = {wire_absent, wire_null, wire_bool,  wire_number, wire_text,
                                     wire_bytes,  wire_node, wire_open, wire_key,    wire_close};

guatiao_status gt_wire_of(SEXP x, guatiao_bytes *out, char *msg, size_t msglen) {
  msg[0] = '\0';
  wire_sink w;
  w.base.ops = &wire_ops;
  w.base.msg = msg;
  w.base.n = msglen;
  w.buf = NULL;
  w.len = w.cap = 0;
  guatiao_status st = emit_rec(x, &w.base, 0);
  *out = guatiao_bytes_from(w.buf, st ? 0 : w.len);
  return st;
}

void gt_build_or_raise(SEXP x, const guatiao_alloc *a, guatiao_value *out) {
  char msg[1024];
  guatiao_status st = gt_build(x, a, out, msg, sizeof msg);
  if (st) gt_raise(st, "%s", msg);
}

const guatiao_value *gt_tree_arg(SEXP x, guatiao_value *tmp) {
  set_absent(tmp);
  if (Rf_inherits(x, "guatiao_node")) return gt_node_ptr(x, NULL);
  gt_build_or_raise(x, &gt_binding_alloc, tmp);
  return tmp;
}

/* Up to three tree arguments: R data is built first, since building may
 * run R code, and nodes are found after it, so no address taken can be
 * moved or freed by that code. A NULL `t` ends the list. */
void gt_tree_args(SEXP a, guatiao_value *ta, const guatiao_value **pa, SEXP b, guatiao_value *tb,
                  const guatiao_value **pb, SEXP c, guatiao_value *tc, const guatiao_value **pc) {
  SEXP xs[3] = {a, b, c};
  guatiao_value *ts[3] = {ta, tb, tc};
  const guatiao_value **ps[3] = {pa, pb, pc};
  int n = tc ? 3 : tb ? 2 : 1;
  char msg[1024];
  for (int i = 0; i < n; i++) set_absent(ts[i]);
  for (int i = 0; i < n; i++) {
    if (Rf_inherits(xs[i], "guatiao_node")) continue;
    guatiao_status st = gt_build(xs[i], &gt_binding_alloc, ts[i], msg, sizeof msg);
    if (st) {
      for (int j = 0; j < n; j++) gt_free(ts[j]);
      gt_raise(st, "%s", msg);
    }
    *ps[i] = ts[i];
  }
  for (int i = 0; i < n; i++) {
    if (!Rf_inherits(xs[i], "guatiao_node")) continue;
    guatiao_value *found = NULL;
    int r = gt_node_try(xs[i], NULL, &found);
    if (r) {
      for (int j = 0; j < n; j++) gt_free(ts[j]);
      if (r == 1) gt_call_r(".gt_closed", 0, R_NilValue, R_NilValue, R_NilValue);
      gt_raise(GUATIAO_ERR_NOT_FOUND, "the node a view addressed is no longer there");
    }
    *ps[i] = found;
  }
}

/* The node again after code ran that may have changed the tree; frees
 * `tmp` before signalling when it is gone. */
static guatiao_value *relocate(SEXP node, guatiao_value *tmp) {
  guatiao_value *v = NULL;
  int r = gt_node_try(node, NULL, &v);
  if (r) {
    gt_free(tmp);
    if (r == 1) gt_call_r(".gt_closed", 0, R_NilValue, R_NilValue, R_NilValue);
    gt_raise(GUATIAO_ERR_NOT_FOUND, "the node a view addressed is no longer there");
  }
  return v;
}

/* ---- reading ------------------------------------------------------------------- */

static SEXP absent_sexp(void) {
  SEXP x = PROTECT(Rf_allocVector(VECSXP, 0));
  Rf_setAttrib(x, R_ClassSymbol, Rf_mkString("guatiao_absent"));
  UNPROTECT(1);
  return x;
}

/* `numbers`: 0 auto, 1 text, 2 double, 3 integer64 for an integer
 * spelling an integer64 holds, auto for the rest. */
static SEXP number_sexp(guatiao_str t, int numbers) {
  char buf[400];
  int integral = 1;
  for (size_t i = 0; i < t.len; i++) {
    char c = (char)t.ptr[i];
    if (c == '.' || c == 'e' || c == 'E') integral = 0;
  }
  if (numbers == 3) {
    if (integral && t.len > 0 && t.len < sizeof buf) {
      memcpy(buf, t.ptr, t.len);
      buf[t.len] = '\0';
      gt_wide w;
      int64_t v = 0;
      if (!gt_wide_text(buf, &w) && !gt_wide_signed(w, 64, &v) && v != INT64_MIN) return gt_integer64_sexp(v);
    }
    numbers = 0;
  }
  if (numbers != 1 && t.len > 0 && t.len < sizeof buf) {
    memcpy(buf, t.ptr, t.len);
    buf[t.len] = '\0';
    char *end = NULL;
    if (integral && numbers == 0 && t.len <= 11) {
      long long v = strtoll(buf, &end, 10);
      if (end == buf + t.len && v > -2147483648LL && v <= 2147483647LL) return Rf_ScalarInteger((int)v);
    }
    double d = strtod(buf, &end);
    if (numbers == 2) return Rf_ScalarReal(d);
    if (end == buf + t.len && R_FINITE(d)) {
      if (!integral) return Rf_ScalarReal(d);
      /* An integer spelling: a double only when it holds it exactly. */
      char back[400];
      snprintf(back, sizeof back, "%.0f", d);
      const char *want = buf;
      if (want[0] == '-' && d == 0) want++;
      if (strcmp(back, want) == 0) return Rf_ScalarReal(d);
    }
  }
  SEXP c = PROTECT(gt_mkchar(t));
  SEXP x = PROTECT(Rf_ScalarString(c));
  Rf_setAttrib(x, R_ClassSymbol, Rf_mkString("guatiao_number"));
  UNPROTECT(2);
  return x;
}

static SEXP read_rec(const guatiao_value *v, int numbers, int depth) {
  if (depth > GT_DEPTH) gt_raise(GUATIAO_ERR_BAD_VALUE, "deeper than 128 nested containers");
  switch (guatiao_tag_of(v)) {
  case GUATIAO_ABSENT: return absent_sexp();
  case GUATIAO_NULL: return R_NilValue;
  case GUATIAO_BOOL: return Rf_ScalarLogical(v->payload.b ? 1 : 0);
  case GUATIAO_NUMBER: return number_sexp(guatiao_number_text(v), numbers);
  case GUATIAO_STRING: return gt_text_sexp(guatiao_string_text(v));
  case GUATIAO_BYTES: {
    guatiao_bytes b = guatiao_bytes_data(v);
    SEXP x = Rf_allocVector(RAWSXP, (R_xlen_t)b.len);
    if (b.len) memcpy(RAW(x), b.ptr, b.len);
    return x;
  }
  case GUATIAO_LIST: {
    guatiao_values items = guatiao_list_items(v);
    SEXP x = PROTECT(Rf_allocVector(VECSXP, (R_xlen_t)items.len));
    for (size_t i = 0; i < items.len; i++) SET_VECTOR_ELT(x, (R_xlen_t)i, read_rec(&items.ptr[i], numbers, depth + 1));
    UNPROTECT(1);
    return x;
  }
  case GUATIAO_MAP: {
    guatiao_entries es = guatiao_map_entries(v);
    SEXP x = PROTECT(Rf_allocVector(VECSXP, (R_xlen_t)es.len));
    SEXP names = PROTECT(Rf_allocVector(STRSXP, (R_xlen_t)es.len));
    for (size_t i = 0; i < es.len; i++) {
      SEXP k = gt_mkchar(guatiao_entry_key(&es.ptr[i]));
      if (!k) gt_raise(GUATIAO_ERR_BAD_VALUE, "a key holding a NUL cannot be an R name");
      SET_STRING_ELT(names, (R_xlen_t)i, k);
      SET_VECTOR_ELT(x, (R_xlen_t)i, read_rec(&es.ptr[i].value, numbers, depth + 1));
    }
    Rf_setAttrib(x, R_NamesSymbol, names);
    UNPROTECT(2);
    return x;
  }
  default: return R_NilValue; /* a tag this build does not know reads as nothing */
  }
}

SEXP gt_read(const guatiao_value *v, int numbers) { return read_rec(v, numbers, 0); }

/* Reads an answer and frees it whether or not reading signals. */
typedef struct read_job {
  guatiao_value *v;
  int numbers;
} read_job;

static SEXP read_job_run(void *p) {
  read_job *j = (read_job *)p;
  return gt_read(j->v, j->numbers);
}

static void read_job_clean(void *p, Rboolean jump) {
  (void)jump;
  gt_free(((read_job *)p)->v);
}

SEXP gt_read_and_free(guatiao_value *v, int numbers) {
  SEXP token = PROTECT(R_MakeUnwindCont());
  read_job j = {v, numbers};
  SEXP out = R_UnwindProtect(read_job_run, &j, read_job_clean, &j, token);
  UNPROTECT(1);
  return out;
}

/* ---- reading wire bytes: a whole document as R data, decoded here --------------- */

typedef struct wire_reader {
  const uint8_t *p;
  size_t n, at;
  int numbers;
} wire_reader;

static void wire_short(void) { gt_raise(GUATIAO_ERR_BAD_VALUE, "the bytes end inside a node"); }

static uint64_t wire_count(wire_reader *r) {
  uint64_t v = 0;
  for (int shift = 0;; shift += 7) {
    if (r->at >= r->n) wire_short();
    uint8_t b = r->p[r->at++];
    if (shift > 63 || (shift == 63 && b > 1)) gt_raise(GUATIAO_ERR_BAD_VALUE, "a length past 64 bits");
    v |= (uint64_t)(b & 0x7F) << shift;
    if (b < 0x80) {
      if (b == 0 && shift > 0) gt_raise(GUATIAO_ERR_BAD_VALUE, "a length not in its shortest form");
      return v;
    }
  }
}

static guatiao_str wire_text_at(wire_reader *r) {
  uint64_t len = wire_count(r);
  if (len > r->n - r->at) wire_short();
  guatiao_str t = guatiao_str_from((const char *)r->p + r->at, (size_t)len);
  r->at += (size_t)len;
  return t;
}

static SEXP wire_node_at(wire_reader *r, int depth) {
  if (r->at >= r->n) wire_short();
  uint8_t tag = r->p[r->at++];
  switch (tag) {
  case GUATIAO_ABSENT: return absent_sexp();
  case GUATIAO_NULL: return R_NilValue;
  case GUATIAO_BOOL: {
    if (r->at >= r->n) wire_short();
    uint8_t b = r->p[r->at++];
    if (b > 1) gt_raise(GUATIAO_ERR_BAD_VALUE, "a bool that is neither 0 nor 1");
    return Rf_ScalarLogical(b);
  }
  case GUATIAO_NUMBER: return number_sexp(wire_text_at(r), r->numbers);
  case GUATIAO_STRING: return gt_text_sexp(wire_text_at(r));
  case GUATIAO_BYTES: {
    guatiao_str t = wire_text_at(r);
    SEXP x = Rf_allocVector(RAWSXP, (R_xlen_t)t.len);
    if (t.len) memcpy(RAW(x), t.ptr, t.len);
    return x;
  }
  case GUATIAO_LIST:
  case GUATIAO_MAP: {
    uint64_t len = wire_count(r);
    /* Every item takes a byte at least: a longer count is refused before anything that large is made. */
    if (len > r->n - r->at) gt_raise(GUATIAO_ERR_BAD_VALUE, "a count past what remains");
    if (len && depth >= GT_DEPTH) gt_raise(GUATIAO_ERR_BAD_VALUE, "deeper than 128 nested containers");
    SEXP x = PROTECT(Rf_allocVector(VECSXP, (R_xlen_t)len));
    SEXP names = tag == GUATIAO_MAP ? Rf_allocVector(STRSXP, (R_xlen_t)len) : R_NilValue;
    PROTECT(names);
    for (uint64_t i = 0; i < len; i++) {
      if (tag == GUATIAO_MAP) {
        SEXP k = gt_mkchar(wire_text_at(r));
        if (!k) gt_raise(GUATIAO_ERR_BAD_VALUE, "a key holding a NUL cannot be an R name");
        SET_STRING_ELT(names, (R_xlen_t)i, k);
      }
      SET_VECTOR_ELT(x, (R_xlen_t)i, wire_node_at(r, depth + 1));
    }
    if (tag == GUATIAO_MAP) Rf_setAttrib(x, R_NamesSymbol, names);
    UNPROTECT(2);
    return x;
  }
  default: gt_raise(GUATIAO_ERR_BAD_VALUE, "unknown tag %d", (int)tag); return R_NilValue;
  }
}

SEXP gt_read_wire(guatiao_bytes data, int numbers) {
  wire_reader r = {data.ptr, data.len, 0, numbers};
  SEXP x = PROTECT(wire_node_at(&r, 0));
  if (r.at != r.n) gt_raise(GUATIAO_ERR_BAD_VALUE, "bytes after the value");
  UNPROTECT(1);
  return x;
}

static SEXP read_wire_job_run(void *p) {
  read_job *j = (read_job *)p;
  return gt_read_wire(guatiao_bytes_data(j->v), j->numbers);
}

SEXP gt_read_wire_and_free(guatiao_value *bytes, int numbers) {
  SEXP token = PROTECT(R_MakeUnwindCont());
  read_job j = {bytes, numbers};
  SEXP out = R_UnwindProtect(read_wire_job_run, &j, read_job_clean, &j, token);
  UNPROTECT(1);
  return out;
}

/* ---- .Call entry points: values -------------------------------------------------- */

static int numbers_arg(SEXP s) { return Rf_asInteger(s); }

SEXP C_value_new(SEXP x, SEXP alloc) {
  const guatiao_alloc *a = gt_alloc_arg(alloc);
  guatiao_value v;
  set_absent(&v);
  if (!Rf_inherits(x, "guatiao_absent")) gt_build_or_raise(x, a, &v);
  SEXP owner = gt_owner_new(&v, a, alloc);
  if (!owner) {
    gt_free(&v);
    gt_raise(GUATIAO_ERR_ALLOC, "out of memory");
  }
  return owner;
}

SEXP C_value_close(SEXP owner) {
  if (TYPEOF(owner) == EXTPTRSXP) {
    gt_owner *o = (gt_owner *)R_ExternalPtrAddr(owner);
    if (o && !(o->flags & GT_OWNED)) return R_NilValue; /* a borrowed view is not closed by its reader */
    gt_owner_clear(owner);
  }
  return R_NilValue;
}

SEXP C_value_is_open(SEXP owner) {
  return Rf_ScalarLogical(TYPEOF(owner) == EXTPTRSXP && R_ExternalPtrAddr(owner) != NULL);
}

/* The tape both ways, for the suite: wire bytes as R data, R data as wire bytes. */
SEXP C_wire_read(SEXP bytes, SEXP numbers) {
  if (TYPEOF(bytes) != RAWSXP) gt_raise(GUATIAO_ERR_BAD_VALUE, "the bytes are a raw vector");
  return gt_read_wire(guatiao_bytes_from(XLENGTH(bytes) ? RAW(bytes) : NULL, (size_t)XLENGTH(bytes)),
                      numbers_arg(numbers));
}

SEXP C_wire_of(SEXP x) {
  char msg[1024];
  guatiao_bytes b;
  guatiao_status st = gt_wire_of(x, &b, msg, sizeof msg);
  if (st) gt_raise(st, "%s", msg);
  SEXP out = Rf_allocVector(RAWSXP, (R_xlen_t)b.len);
  if (b.len) memcpy(RAW(out), b.ptr, b.len);
  return out;
}

SEXP C_node_read(SEXP node, SEXP numbers) { return gt_read(gt_node_ptr(node, NULL), numbers_arg(numbers)); }

SEXP C_node_tag(SEXP node) { return Rf_ScalarInteger((int)guatiao_tag_of(gt_node_ptr(node, NULL))); }

SEXP C_node_length(SEXP node) {
  const guatiao_value *v = gt_node_ptr(node, NULL);
  switch (guatiao_tag_of(v)) {
  case GUATIAO_MAP: return Rf_ScalarReal((double)guatiao_map_entries(v).len);
  case GUATIAO_LIST: return Rf_ScalarReal((double)guatiao_list_items(v).len);
  case GUATIAO_ABSENT:
  case GUATIAO_NULL: return Rf_ScalarReal(0);
  default: return Rf_ScalarReal(1);
  }
}

SEXP C_node_names(SEXP node) {
  const guatiao_value *v = gt_node_ptr(node, NULL);
  if (guatiao_tag_of(v) != GUATIAO_MAP) return R_NilValue;
  guatiao_entries es = guatiao_map_entries(v);
  SEXP names = PROTECT(Rf_allocVector(STRSXP, (R_xlen_t)es.len));
  for (size_t i = 0; i < es.len; i++) {
    SEXP k = gt_mkchar(guatiao_entry_key(&es.ptr[i]));
    if (!k) gt_raise(GUATIAO_ERR_BAD_VALUE, "a key holding a NUL cannot be an R name");
    SET_STRING_ELT(names, (R_xlen_t)i, k);
  }
  UNPROTECT(1);
  return names;
}

static const char *tag_name(uint32_t tag) {
  switch (tag) {
  case GUATIAO_ABSENT: return "absent";
  case GUATIAO_NULL: return "null";
  case GUATIAO_BOOL: return "bool";
  case GUATIAO_NUMBER: return "number";
  case GUATIAO_STRING: return "string";
  case GUATIAO_BYTES: return "bytes";
  case GUATIAO_LIST: return "list";
  case GUATIAO_MAP: return "map";
  default: return "unknown";
  }
}

/* A position from R (1-based) as an index, or -1. */
static long long position(SEXP sel) {
  double d = Rf_asReal(sel);
  if (ISNAN(d) || d < 1 || d != floor(d)) return -1;
  return (long long)d - 1;
}

/* The path step `sel` names below `node`: a key (character) or a position
 * (number, from 1; a map's position names its key). NULL for a key the map
 * does not hold. */
SEXP C_node_child(SEXP node, SEXP sel) {
  const guatiao_value *v = gt_node_ptr(node, NULL);
  uint32_t tag = guatiao_tag_of(v);
  if (TYPEOF(sel) == STRSXP) {
    if (tag != GUATIAO_MAP) gt_raise(GUATIAO_ERR_WRONG_KIND, "a %s has no keys", tag_name(tag));
    guatiao_str k = gt_str_arg(sel, "a key");
    if (!guatiao_map_find(v, k)) return R_NilValue;
    return Rf_ScalarString(STRING_ELT(sel, 0));
  }
  long long at = position(sel);
  if (tag == GUATIAO_LIST) {
    if (at < 0 || (size_t)at >= guatiao_list_items(v).len) gt_raise(GUATIAO_ERR_NOT_FOUND, "subscript out of bounds");
    return Rf_ScalarInteger((int)at);
  }
  if (tag == GUATIAO_MAP) {
    guatiao_entries es = guatiao_map_entries(v);
    if (at < 0 || (size_t)at >= es.len) gt_raise(GUATIAO_ERR_NOT_FOUND, "subscript out of bounds");
    SEXP k = gt_mkchar(guatiao_entry_key(&es.ptr[at]));
    if (!k) gt_raise(GUATIAO_ERR_BAD_VALUE, "a key holding a NUL cannot be an R name");
    return Rf_ScalarString(k);
  }
  gt_raise(GUATIAO_ERR_WRONG_KIND, "a %s has no elements", tag_name(tag));
}

static guatiao_value *writable(SEXP node, gt_owner **o) {
  guatiao_value *v = gt_node_ptr(node, o);
  if ((*o)->flags & GT_READONLY) gt_raise(GUATIAO_ERR_UNSUPPORTED, "this view is read-only");
  return v;
}

/* Moves `tmp` over element `at` of a list, freeing what was there. */
static void replace_element(guatiao_value *list, size_t at, guatiao_value *tmp) {
  guatiao_value *slot = &list->payload.list.ptr[at];
  gt_free(slot);
  *slot = *tmp;
  set_absent(tmp);
}

SEXP C_node_set(SEXP node, SEXP sel, SEXP x) {
  gt_owner *o = NULL;
  guatiao_value *v = writable(node, &o);
  uint32_t tag = guatiao_tag_of(v);
  const guatiao_alloc *a = gt_alloc_for(v, o);
  if (TYPEOF(sel) == STRSXP || tag == GUATIAO_MAP) {
    if (tag != GUATIAO_MAP) gt_raise(GUATIAO_ERR_WRONG_KIND, "a %s has no keys", tag_name(tag));
    guatiao_str k;
    if (TYPEOF(sel) == STRSXP) {
      k = gt_str_arg(sel, "a key");
    } else {
      long long at = position(sel);
      guatiao_entries es = guatiao_map_entries(v);
      if (at < 0 || (size_t)at >= es.len) gt_raise(GUATIAO_ERR_NOT_FOUND, "subscript out of bounds");
      k = guatiao_entry_key(&es.ptr[at]);
    }
    /* The key is copied before the tree changes: it may view the map's own. */
    char *key = (char *)R_alloc(k.len + 1, 1);
    if (k.len) memcpy(key, k.ptr, k.len);
    guatiao_value tmp;
    gt_build_or_raise(x, a, &tmp);
    v = relocate(node, &tmp); /* building may have run R code */
    guatiao_status st = GT_CORE_FN(guatiao_map_set)(a, v, guatiao_str_from(key, k.len), &tmp);
    if (st) {
      gt_free(&tmp);
      gt_raise(st, "the map refused the key");
    }
    return R_NilValue;
  }
  if (tag != GUATIAO_LIST) gt_raise(GUATIAO_ERR_WRONG_KIND, "a %s has no elements", tag_name(tag));
  long long at = position(sel);
  size_t len = guatiao_list_items(v).len;
  if (at < 0 || (size_t)at > len) gt_raise(GUATIAO_ERR_NOT_FOUND, "subscript out of bounds");
  guatiao_value tmp;
  gt_build_or_raise(x, a, &tmp);
  v = relocate(node, &tmp);
  if ((size_t)at == guatiao_list_items(v).len) {
    guatiao_status st = GT_CORE_FN(guatiao_list_push)(a, v, &tmp);
    if (st) {
      gt_free(&tmp);
      gt_raise(st, "the list refused the element");
    }
  } else {
    replace_element(v, (size_t)at, &tmp);
  }
  return R_NilValue;
}

SEXP C_node_push(SEXP node, SEXP items) {
  gt_owner *o = NULL;
  guatiao_value *v = writable(node, &o);
  if (guatiao_tag_of(v) != GUATIAO_LIST) gt_raise(GUATIAO_ERR_WRONG_KIND, "only a list takes elements");
  const guatiao_alloc *a = gt_alloc_for(v, o);
  for (R_xlen_t i = 0; i < XLENGTH(items); i++) {
    guatiao_value tmp;
    gt_build_or_raise(VECTOR_ELT(items, i), a, &tmp);
    v = relocate(node, &tmp);
    guatiao_status st = GT_CORE_FN(guatiao_list_push)(a, v, &tmp);
    if (st) {
      gt_free(&tmp);
      gt_raise(st, "the list refused the element");
    }
  }
  return R_NilValue;
}

SEXP C_node_insert(SEXP node, SEXP sel, SEXP x) {
  gt_owner *o = NULL;
  guatiao_value *v = writable(node, &o);
  if (guatiao_tag_of(v) != GUATIAO_LIST) gt_raise(GUATIAO_ERR_WRONG_KIND, "only a list takes elements");
  long long at = position(sel);
  if (at < 0 || (size_t)at > guatiao_list_items(v).len) gt_raise(GUATIAO_ERR_NOT_FOUND, "subscript out of bounds");
  const guatiao_alloc *a = gt_alloc_for(v, o);
  guatiao_value tmp;
  gt_build_or_raise(x, a, &tmp);
  v = relocate(node, &tmp);
  guatiao_status st = GT_CORE_FN(guatiao_list_push)(a, v, &tmp);
  if (st) {
    gt_free(&tmp);
    gt_raise(st, "the list refused the element");
  }
  /* Elements are plain structs: the new last one moves to `at`. */
  guatiao_value *arr = v->payload.list.ptr;
  size_t len = v->payload.list.len;
  guatiao_value last = arr[len - 1];
  memmove(&arr[at + 1], &arr[at], (len - 1 - (size_t)at) * sizeof(guatiao_value));
  arr[at] = last;
  return R_NilValue;
}

SEXP C_node_delete(SEXP node, SEXP sel) {
  gt_owner *o = NULL;
  guatiao_value *v = writable(node, &o);
  uint32_t tag = guatiao_tag_of(v);
  guatiao_status st;
  if (tag == GUATIAO_MAP && TYPEOF(sel) == STRSXP) {
    st = GT_CORE_FN(guatiao_map_discard)(v, gt_str_arg(sel, "a key"));
  } else if (tag == GUATIAO_MAP) {
    long long at = position(sel);
    guatiao_entries es = guatiao_map_entries(v);
    if (at < 0 || (size_t)at >= es.len) return R_NilValue;
    guatiao_str k = guatiao_entry_key(&es.ptr[at]);
    char *key = (char *)R_alloc(k.len + 1, 1);
    if (k.len) memcpy(key, k.ptr, k.len);
    st = GT_CORE_FN(guatiao_map_discard)(v, guatiao_str_from(key, k.len));
  } else if (tag == GUATIAO_LIST) {
    long long at = position(sel);
    if (at < 0) return R_NilValue;
    st = GT_CORE_FN(guatiao_list_discard)(v, (size_t)at);
  } else {
    gt_raise(GUATIAO_ERR_WRONG_KIND, "a %s holds nothing to delete", tag_name(tag));
  }
  if (st && st != GUATIAO_ERR_NOT_FOUND) gt_raise(st, "the delete was refused");
  return R_NilValue;
}

SEXP C_node_clear(SEXP node) {
  gt_owner *o = NULL;
  guatiao_value *v = writable(node, &o);
  uint32_t tag = guatiao_tag_of(v);
  guatiao_status st;
  if (tag == GUATIAO_MAP) st = GT_CORE_FN(guatiao_map_clear)(v);
  else if (tag == GUATIAO_LIST) st = GT_CORE_FN(guatiao_list_clear)(v);
  else gt_raise(GUATIAO_ERR_WRONG_KIND, "a %s holds nothing to clear", tag_name(tag));
  if (st) gt_raise(st, "the clear was refused");
  return R_NilValue;
}

SEXP C_node_merge(SEXP node, SEXP src) {
  gt_owner *o = NULL;
  guatiao_value *v = writable(node, &o);
  if (guatiao_tag_of(v) != GUATIAO_MAP) gt_raise(GUATIAO_ERR_WRONG_KIND, "only a map merges entries");
  const guatiao_alloc *a = gt_alloc_for(v, o);
  guatiao_value tmp;
  const guatiao_value *s = gt_tree_arg(src, &tmp);
  v = relocate(node, &tmp);
  guatiao_status st = GT_CORE_FN(guatiao_map_copy_from)(a, v, s);
  gt_free(&tmp);
  if (st) gt_raise(st, "the merge was refused");
  return R_NilValue;
}

SEXP C_node_copy(SEXP node, SEXP alloc) {
  const guatiao_alloc *a = gt_alloc_arg(alloc);
  const guatiao_value *v = gt_node_ptr(node, NULL);
  guatiao_value out;
  set_absent(&out);
  guatiao_status st = GT_CORE_FN(guatiao_value_clone)(a, v, &out);
  if (st) gt_raise(st, "the copy was refused");
  SEXP owner = gt_owner_new(&out, a, alloc);
  if (!owner) {
    gt_free(&out);
    gt_raise(GUATIAO_ERR_ALLOC, "out of memory");
  }
  return owner;
}

/* The node's own address, for C code a test calls; valid while its owner is. */
SEXP C_node_pointer(SEXP node) {
  guatiao_value *v = gt_node_ptr(node, NULL);
  return R_MakeExternalPtr(v, R_NilValue, VECTOR_ELT(node, 0));
}

/* ---- allocators ------------------------------------------------------------------ */

SEXP C_alloc_counting(void) {
  gt_counting *c = gt_counting_new();
  if (!c) gt_raise(GUATIAO_ERR_ALLOC, "out of memory");
  return R_MakeExternalPtr(&c->alloc, Rf_install("guatiao_counting"), R_NilValue);
}

SEXP C_alloc_live(SEXP a) {
  if (TYPEOF(a) != EXTPTRSXP || R_ExternalPtrTag(a) != Rf_install("guatiao_counting"))
    gt_raise(GUATIAO_ERR_WRONG_KIND, "not a counting allocator");
  return Rf_ScalarReal((double)gt_counting_live((const gt_counting *)R_ExternalPtrAddr(a)));
}

SEXP C_alloc_refuse(SEXP a, SEXP on) {
  if (TYPEOF(a) != EXTPTRSXP || R_ExternalPtrTag(a) != Rf_install("guatiao_counting"))
    gt_raise(GUATIAO_ERR_WRONG_KIND, "not a counting allocator");
  ((gt_counting *)R_ExternalPtrAddr(a))->refusing = Rf_asLogical(on) == TRUE;
  return R_NilValue;
}

SEXP C_alloc_binding(void) { return R_MakeExternalPtr((void *)&gt_binding_alloc, R_NilValue, R_NilValue); }

/* The allocator a node's container records, as an address for a test. */
SEXP C_node_alloc_address(SEXP node) {
  const guatiao_value *v = gt_node_ptr(node, NULL);
  const guatiao_alloc *a = NULL;
  switch (guatiao_tag_of(v)) {
  case GUATIAO_MAP: a = v->payload.map.alloc; break;
  case GUATIAO_LIST: a = v->payload.list.alloc; break;
  case GUATIAO_STRING: a = v->payload.text.alloc; break;
  case GUATIAO_NUMBER: a = v->payload.number.alloc; break;
  case GUATIAO_BYTES: a = v->payload.bytes.alloc; break;
  default: break;
  }
  char buf[32];
  snprintf(buf, sizeof buf, "%p", (const void *)a);
  return Rf_mkString(buf);
}

SEXP C_alloc_address(SEXP a) {
  char buf[32];
  snprintf(buf, sizeof buf, "%p", gt_alloc_arg(a));
  return Rf_mkString(buf);
}

/* A number's exact text read as `numbers = "auto"` reads it. */
SEXP C_number_read(SEXP text) {
  guatiao_str t = gt_str_arg(text, "a number");
  return number_sexp(t, 0);
}
