/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/* MATLAB data and trees, both ways. */

#include <errno.h>
#include <float.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "gt.h"

static void set_absent(guatiao_value *v) { memset(v, 0, sizeof *v); }

/* ---- numbers as text --------------------------------------------------------------- */

/* The shortest text that reads back as `d`; a whole number below 1e21 as
 * the integer it is. */
static void double_text(double d, char *buf, size_t n) {
  if (d == floor(d) && fabs(d) < 1e21) {
    snprintf(buf, n, "%.0f", d);
    if (d == 0 && signbit(d)) snprintf(buf, n, "-0");
    return;
  }
  /* A normal double is within half a unit of the 15th digit of its shortest
     text, so 15 digits with `%g`'s trailing zeros dropped is that text whenever
     it reads back; a subnormal has fewer digits and is searched from one. */
  for (int p = fabs(d) >= DBL_MIN ? 15 : 1; p <= 17; p++) {
    snprintf(buf, n, "%.*g", p, d);
    if (strtod(buf, NULL) == d) return;
  }
}

static void single_text(float f, char *buf, size_t n) {
  double d = (double)f;
  if (d == floor(d) && fabs(d) < 1e21) {
    double_text(d, buf, n);
    return;
  }
  for (int p = 1; p <= 9; p++) {
    snprintf(buf, n, "%.*g", p, d);
    if (strtof(buf, NULL) == f) return;
  }
}

/* ---- building: one walk over MATLAB data, two sinks ------------------------------------- */

/* What the walk over MATLAB data hands its events to, in order: a
 * container's `open` (with how many items it was given), then each item (a
 * map's after its `key`) as one event or a nested container, then `close`.
 * Each answers 1, or 0 with the refusal recorded in the call. An absent
 * item of a map is no entry. */
typedef struct gt_sink gt_sink;
typedef struct gt_sink_ops {
  int (*absent)(gt_sink *s);
  int (*null)(gt_sink *s);
  int (*boolean)(gt_sink *s, int b);
  int (*number)(gt_sink *s, guatiao_str text);
  int (*text)(gt_sink *s, guatiao_str t);
  int (*bytes)(gt_sink *s, const void *p, size_t n);
  int (*node)(gt_sink *s, const guatiao_value *src);
  int (*open)(gt_sink *s, int is_map, size_t n);
  int (*key)(gt_sink *s, guatiao_str k);
  int (*close)(gt_sink *s);
} gt_sink_ops;

struct gt_sink {
  const gt_sink_ops *ops;
  gt_call *c;
};

/* Containers open at once: a matrix adds a row's list to each level. */
#define GT_FRAMES (2 * GT_DEPTH + 4)

/* Element `i` of a numeric or logical array as a scalar. */
static int scalar_at(gt_sink *s, const mxArray *x, size_t i) {
  char buf[64];
  const void *d = mxGetData(x);
  switch (mxGetClassID(x)) {
  case mxLOGICAL_CLASS: return s->ops->boolean(s, ((const mxLogical *)d)[i] != 0);
  case mxDOUBLE_CLASS: {
    /* Octave 10 hands a MEX file the data of a literal 9007199254740992
     * inside a cell or struct as an int64's bits; mxGetScalar converts. */
    double v = mxGetNumberOfElements(x) == 1 ? mxGetScalar(x) : ((const double *)d)[i];
    if (!isfinite(v)) return gt_fail(s->c, GUATIAO_ERR_BAD_VALUE, "a value holds no NaN or infinity");
    double_text(v, buf, sizeof buf);
    break;
  }
  case mxSINGLE_CLASS: {
    float v = ((const float *)d)[i];
    if (!isfinite(v)) return gt_fail(s->c, GUATIAO_ERR_BAD_VALUE, "a value holds no NaN or infinity");
    single_text(v, buf, sizeof buf);
    break;
  }
  case mxINT8_CLASS: snprintf(buf, sizeof buf, "%d", (int)((const int8_t *)d)[i]); break;
  case mxINT16_CLASS: snprintf(buf, sizeof buf, "%d", (int)((const int16_t *)d)[i]); break;
  case mxINT32_CLASS: snprintf(buf, sizeof buf, "%ld", (long)((const int32_t *)d)[i]); break;
  case mxINT64_CLASS: snprintf(buf, sizeof buf, "%lld", (long long)((const int64_t *)d)[i]); break;
  case mxUINT8_CLASS: snprintf(buf, sizeof buf, "%u", (unsigned)((const uint8_t *)d)[i]); break;
  case mxUINT16_CLASS: snprintf(buf, sizeof buf, "%u", (unsigned)((const uint16_t *)d)[i]); break;
  case mxUINT32_CLASS: snprintf(buf, sizeof buf, "%lu", (unsigned long)((const uint32_t *)d)[i]); break;
  case mxUINT64_CLASS: snprintf(buf, sizeof buf, "%llu", (unsigned long long)((const uint64_t *)d)[i]); break;
  default: return gt_fail(s->c, GUATIAO_ERR_BAD_VALUE, "cannot write a %s", mxGetClassName(x));
  }
  return s->ops->number(s, guatiao_str_from(buf, strlen(buf)));
}

static int emit_rec(gt_sink *s, const mxArray *x, int depth);

/* A numeric or logical array of more than one element: a list, or a list
 * of rows for a matrix. */
static int numeric_list(gt_sink *s, const mxArray *x) {
  size_t m = mxGetM(x), n = mxGetN(x);
  int matrix = m > 1 && n > 1;
  size_t rows = matrix ? m : 1, cols = matrix ? n : m * n;
  if (!s->ops->open(s, 0, matrix ? rows : cols)) return 0;
  for (size_t r = 0; r < rows; r++) {
    if (matrix && !s->ops->open(s, 0, cols)) return 0;
    for (size_t k = 0; k < cols; k++)
      if (!scalar_at(s, x, matrix ? r + k * m : k)) return 0; /* column-major storage */
    if (matrix && !s->ops->close(s)) return 0;
  }
  return s->ops->close(s);
}

static int cell_list(gt_sink *s, const mxArray *x, int depth) {
  size_t m = mxGetM(x), n = mxGetN(x);
  int matrix = m > 1 && n > 1;
  size_t rows = matrix ? m : 1, cols = matrix ? n : m * n;
  if (!s->ops->open(s, 0, matrix ? rows : cols)) return 0;
  for (size_t r = 0; r < rows; r++) {
    if (matrix && !s->ops->open(s, 0, cols)) return 0;
    for (size_t k = 0; k < cols; k++) {
      const mxArray *e = mxGetCell(x, (mwIndex)(matrix ? r + k * m : k));
      if (!(e ? emit_rec(s, e, depth + 1) : s->ops->null(s))) return 0;
    }
    if (matrix && !s->ops->close(s)) return 0;
  }
  return s->ops->close(s);
}

/* Element `i` of a struct array as a map of its fields, in order. */
static int struct_map(gt_sink *s, const mxArray *x, size_t i, int depth) {
  int nf = mxGetNumberOfFields(x);
  if (!s->ops->open(s, 1, (size_t)nf)) return 0;
  for (int f = 0; f < nf; f++) {
    const char *name = mxGetFieldNameByNumber(x, f);
    const mxArray *e = mxGetFieldByNumber(x, (mwIndex)i, f);
    if (!s->ops->key(s, guatiao_str_from(name, strlen(name)))) return 0;
    if (!(e ? emit_rec(s, e, depth + 1) : s->ops->null(s))) return 0;
  }
  return s->ops->close(s);
}

static int is_plain_class(mxClassID id) {
  switch (id) {
  case mxCELL_CLASS:
  case mxSTRUCT_CLASS:
  case mxLOGICAL_CLASS:
  case mxCHAR_CLASS:
  case mxDOUBLE_CLASS:
  case mxSINGLE_CLASS:
  case mxINT8_CLASS:
  case mxUINT8_CLASS:
  case mxINT16_CLASS:
  case mxUINT16_CLASS:
  case mxINT32_CLASS:
  case mxUINT32_CLASS:
  case mxINT64_CLASS:
  case mxUINT64_CLASS: return 1;
  default: return 0;
  }
}

static int lowered_text(gt_call *c, const mxArray *x, const char *what, guatiao_str *out) {
  return x ? gt_mx_utf8(c, x, what, out) : gt_fail(c, GUATIAO_ERR_INTERNAL, "%s is missing", what);
}

/* An object: guatiao.internal.as_tree says what it is. */
static int emit_object(gt_sink *s, const mxArray *x, int depth) {
  gt_call *c = s->c;
  mxArray *in[1] = {(mxArray *)x};
  mxArray *r = NULL;
  if (!gt_call_m(c, "guatiao.internal.as_tree", 1, &r, 1, in)) return 0;
  int ok = 0;
  if (!r || !mxIsCell(r) || mxGetNumberOfElements(r) < 1 || !mxIsChar(mxGetCell(r, 0))) {
    ok = gt_fail(c, GUATIAO_ERR_INTERNAL, "guatiao.internal.as_tree answered something else");
    goto done;
  }
  char kind[16];
  mxGetString(mxGetCell(r, 0), kind, sizeof kind);
  const mxArray *p1 = mxGetNumberOfElements(r) > 1 ? mxGetCell(r, 1) : NULL;
  const mxArray *p2 = mxGetNumberOfElements(r) > 2 ? mxGetCell(r, 2) : NULL;
  if (!strcmp(kind, "data")) {
    ok = p1 ? emit_rec(s, p1, depth + 1) : s->ops->null(s);
  } else if (!strcmp(kind, "absent")) {
    ok = s->ops->absent(s);
  } else if (!strcmp(kind, "number")) {
    guatiao_str t;
    if (lowered_text(c, p1, "a number's text", &t)) ok = s->ops->number(s, t);
  } else if (!strcmp(kind, "node")) {
    guatiao_value *src = NULL;
    if (gt_walk_mx(c, p1, p2, &src)) ok = s->ops->node(s, src);
  } else if (!strcmp(kind, "map")) {
    /* p1 the keys (a cell of text), p2 the values (a cell). */
    if (!p1 || !p2 || !mxIsCell(p1) || !mxIsCell(p2) || mxGetNumberOfElements(p1) != mxGetNumberOfElements(p2)) {
      ok = gt_fail(c, GUATIAO_ERR_INTERNAL, "a lowered map is two cells of one length");
    } else if (s->ops->open(s, 1, mxGetNumberOfElements(p1))) {
      ok = 1;
      for (size_t i = 0; ok && i < mxGetNumberOfElements(p1); i++) {
        guatiao_str k;
        ok = gt_mx_utf8(c, mxGetCell(p1, (mwIndex)i), "a map key", &k) && s->ops->key(s, k) &&
             emit_rec(s, mxGetCell(p2, (mwIndex)i), depth + 1);
      }
      ok = ok && s->ops->close(s);
    }
  } else if (!strcmp(kind, "refuse")) {
    guatiao_str t;
    if (lowered_text(c, p1, "a reason", &t)) ok = gt_fail(c, GUATIAO_ERR_BAD_VALUE, "%.*s", (int)t.len, t.ptr);
  } else {
    ok = gt_fail(c, GUATIAO_ERR_INTERNAL, "guatiao.internal.as_tree answered '%s'", kind);
  }
done:
  if (r) mxDestroyArray(r);
  return ok;
}

static int emit_rec(gt_sink *s, const mxArray *x, int depth) {
  gt_call *c = s->c;
  if (depth > GT_DEPTH) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "deeper than 128 nested containers");
  mxClassID id = mxGetClassID(x);
  if (!is_plain_class(id)) {
    if (id == mxFUNCTION_CLASS) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "cannot write a function handle");
    return emit_object(s, x, depth);
  }
  if (mxIsSparse(x)) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "cannot write a sparse array");
  if (mxIsComplex(x)) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "cannot write a complex number");
  if (mxGetNumberOfDimensions(x) > 2)
    return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "cannot write an array of more than two dimensions");
  size_t n = mxGetNumberOfElements(x);
  switch (id) {
  case mxCHAR_CLASS: {
    guatiao_str t;
    if (n > 0 && mxGetM(x) != 1)
      return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "a character matrix is not text: pass a cell of rows");
    return gt_mx_utf8(c, x, "text", &t) && s->ops->text(s, t);
  }
  case mxUINT8_CLASS: return s->ops->bytes(s, n ? mxGetData(x) : NULL, n);
  case mxCELL_CLASS: return cell_list(s, x, depth);
  case mxSTRUCT_CLASS:
    if (n == 1) return struct_map(s, x, 0, depth);
    if (mxGetM(x) > 1 && mxGetN(x) > 1) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "cannot write a matrix of structs");
    if (!s->ops->open(s, 0, n)) return 0;
    for (size_t i = 0; i < n; i++)
      if (!struct_map(s, x, i, depth + 1)) return 0;
    return s->ops->close(s);
  default:
    if (n == 0) return s->ops->null(s);
    if (n == 1) return scalar_at(s, x, 0);
    return numeric_list(s, x);
  }
}

/* ---- the tree sink: each event built into a node and moved into its container */

typedef struct tree_frame {
  guatiao_value *node; /* the open container */
  guatiao_value child; /* the item being built, until it is moved in */
  int is_map;
  guatiao_str key;
} tree_frame;

typedef struct tree_sink {
  gt_sink base;
  const guatiao_alloc *a;
  guatiao_value *root;
  int depth;
  tree_frame frames[GT_FRAMES];
} tree_sink;

static guatiao_value *tree_target(tree_sink *t) { return t->depth ? &t->frames[t->depth - 1].child : t->root; }

/* Moves the item just built into the innermost open container. */
static int tree_placed(tree_sink *t) {
  if (!t->depth) return 1;
  tree_frame *f = &t->frames[t->depth - 1];
  gt_call *c = t->base.c;
  if (f->is_map) {
    if (f->child.tag == GUATIAO_ABSENT) return 1; /* an absent entry is no entry */
    size_t before = guatiao_map_entries(f->node).len;
    guatiao_status st = gt_core.guatiao_map_set(t->a, f->node, f->key, &f->child);
    if (st) {
      gt_free(&f->child);
      set_absent(&f->child);
      return gt_fail(c, st, "the key '%.*s' was refused", (int)f->key.len, f->key.ptr);
    }
    set_absent(&f->child);
    if (guatiao_map_entries(f->node).len == before)
      return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "two entries are named '%.*s'", (int)f->key.len, f->key.ptr);
    return 1;
  }
  guatiao_status st = gt_core.guatiao_list_push(t->a, f->node, &f->child);
  if (st) {
    gt_free(&f->child);
    set_absent(&f->child);
    return gt_fail(c, st, "a list refused an element");
  }
  set_absent(&f->child);
  return 1;
}

static int tree_made(tree_sink *t, guatiao_status st, const char *why) {
  if (st) return gt_fail(t->base.c, st, "%s", why);
  return tree_placed(t);
}

static int tree_absent(gt_sink *s) { return tree_placed((tree_sink *)s); }
static int tree_null(gt_sink *s) {
  tree_sink *t = (tree_sink *)s;
  return tree_made(t, gt_core.guatiao_value_null(tree_target(t)), "the allocator refused");
}
static int tree_bool(gt_sink *s, int b) {
  tree_sink *t = (tree_sink *)s;
  return tree_made(t, gt_core.guatiao_value_bool(b ? true : false, tree_target(t)), "the allocator refused");
}
static int tree_number(gt_sink *s, guatiao_str text) {
  tree_sink *t = (tree_sink *)s;
  guatiao_status st = gt_core.guatiao_value_number(t->a, text, tree_target(t));
  if (st) return gt_fail(s->c, st, "'%.*s' is not a number", (int)text.len, text.ptr);
  return tree_placed(t);
}
static int tree_text(gt_sink *s, guatiao_str text) {
  tree_sink *t = (tree_sink *)s;
  return tree_made(t, gt_core.guatiao_value_string(t->a, text, tree_target(t)), "text that is not UTF-8");
}
static int tree_bytes(gt_sink *s, const void *p, size_t n) {
  tree_sink *t = (tree_sink *)s;
  return tree_made(t, gt_core.guatiao_value_bytes(t->a, guatiao_bytes_from(p, n), tree_target(t)),
                   "the allocator refused");
}
static int tree_node(gt_sink *s, const guatiao_value *src) {
  tree_sink *t = (tree_sink *)s;
  return tree_made(t, gt_core.guatiao_value_clone(t->a, src, tree_target(t)), "the copy was refused");
}
static int tree_open(gt_sink *s, int is_map, size_t n) {
  (void)n;
  tree_sink *t = (tree_sink *)s;
  if (t->depth >= GT_FRAMES) return gt_fail(s->c, GUATIAO_ERR_BAD_VALUE, "deeper than 128 nested containers");
  guatiao_value *into = tree_target(t);
  guatiao_status st = is_map ? gt_core.guatiao_value_map(t->a, into) : gt_core.guatiao_value_list(t->a, into);
  if (st) return gt_fail(s->c, st, "the allocator refused");
  tree_frame *f = &t->frames[t->depth++];
  f->node = into;
  set_absent(&f->child);
  f->is_map = is_map;
  f->key = guatiao_str_from(NULL, 0);
  return 1;
}
static int tree_key(gt_sink *s, guatiao_str k) {
  tree_sink *t = (tree_sink *)s;
  t->frames[t->depth - 1].key = k;
  return 1;
}
static int tree_close(gt_sink *s) {
  tree_sink *t = (tree_sink *)s;
  t->depth--;
  return tree_placed(t);
}

static const gt_sink_ops tree_ops = {tree_absent, tree_null, tree_bool, tree_number, tree_text,
                                     tree_bytes,  tree_node, tree_open, tree_key,    tree_close};

int gt_build(gt_call *c, const mxArray *x, const guatiao_alloc *a, guatiao_value *out) {
  set_absent(out);
  if (!GT_NEED_CORE(c, guatiao_value_map)) return 0;
  tree_sink *t = (tree_sink *)gt_scratch(c, sizeof *t);
  if (!t) return 0;
  t->base.ops = &tree_ops;
  t->base.c = c;
  t->a = a ? a : &gt_binding_alloc;
  t->root = out;
  t->depth = 0;
  if (!emit_rec(&t->base, x, 0)) {
    /* The items not yet moved in, innermost first; each holds the next. */
    for (int i = t->depth - 1; i >= 0; i--) gt_free(&t->frames[i].child);
    gt_free(out);
    set_absent(out);
    return 0;
  }
  return 1;
}

/* ---- the wire sink: the same events as wire bytes ------------------------------------- */

typedef struct wire_frame {
  size_t at;         /* where the container's count starts */
  size_t want, have; /* the count written, the items written */
  size_t key_at;     /* where the current item's key starts */
  int is_map;
} wire_frame;

typedef struct wire_sink {
  gt_sink base;
  uint8_t *buf;
  size_t len, cap;
  int depth;
  wire_frame frames[GT_FRAMES];
} wire_sink;

static int wire_room(wire_sink *w, size_t more) {
  if (more <= w->cap - w->len) return 1;
  size_t cap = w->cap ? w->cap : 256;
  while (cap - w->len < more) {
    if (cap > SIZE_MAX / 2) return gt_fail(w->base.c, GUATIAO_ERR_ALLOC, "out of memory");
    cap *= 2;
  }
  uint8_t *grown = (uint8_t *)realloc(w->buf, cap);
  if (!grown) return gt_fail(w->base.c, GUATIAO_ERR_ALLOC, "out of memory");
  w->buf = grown;
  w->cap = cap;
  return 1;
}

static size_t leb_size(uint64_t n) {
  size_t k = 1;
  while (n >= 0x80) {
    n >>= 7;
    k++;
  }
  return k;
}

static void leb_at(uint8_t *p, uint64_t n) {
  while (n >= 0x80) {
    *p++ = (uint8_t)(n | 0x80);
    n >>= 7;
  }
  *p = (uint8_t)n;
}

static int wire_byte(wire_sink *w, uint8_t b) {
  if (!wire_room(w, 1)) return 0;
  w->buf[w->len++] = b;
  return 1;
}

static int wire_leb(wire_sink *w, uint64_t n) {
  if (!wire_room(w, 10)) return 0;
  leb_at(w->buf + w->len, n);
  w->len += leb_size(n);
  return 1;
}

static int wire_body(wire_sink *w, const void *p, size_t n) {
  if (!wire_leb(w, n) || !wire_room(w, n)) return 0;
  if (n) memcpy(w->buf + w->len, p, n);
  w->len += n;
  return 1;
}

/* Counts an item into the innermost open container. */
static int wire_item(wire_sink *w) {
  if (w->depth) w->frames[w->depth - 1].have++;
  return 1;
}

/* Whether `p` is UTF-8 (RFC 3629: no overlongs, no surrogates, at most U+10FFFF). */
static int utf8_valid(const uint8_t *p, size_t n) {
  size_t i = 0;
  while (i < n) {
    uint8_t ch = p[i];
    if (ch < 0x80) {
      i++;
      continue;
    }
    size_t k;
    uint32_t cp;
    if ((ch & 0xE0) == 0xC0) k = 1, cp = ch & 0x1F;
    else if ((ch & 0xF0) == 0xE0) k = 2, cp = ch & 0x0F;
    else if ((ch & 0xF8) == 0xF0) k = 3, cp = ch & 0x07;
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
static int number_grammar(guatiao_str t) {
  const char *p = (const char *)t.ptr, *end = p + t.len;
#define GT_DIGIT(q) ((q) < end && *(q) >= '0' && *(q) <= '9')
  if (p < end && *p == '-') p++;
  if (p < end && *p == '0') p++;
  else if (p < end && *p >= '1' && *p <= '9')
    while (GT_DIGIT(p)) p++;
  else return 0;
  if (p < end && *p == '.') {
    p++;
    if (!GT_DIGIT(p)) return 0;
    while (GT_DIGIT(p)) p++;
  }
  if (p < end && (*p == 'e' || *p == 'E')) {
    p++;
    if (p < end && (*p == '+' || *p == '-')) p++;
    if (!GT_DIGIT(p)) return 0;
    while (GT_DIGIT(p)) p++;
  }
#undef GT_DIGIT
  return p == end;
}

static int wire_absent(gt_sink *s) {
  wire_sink *w = (wire_sink *)s;
  if (w->depth && w->frames[w->depth - 1].is_map) {
    w->len = w->frames[w->depth - 1].key_at; /* an absent entry is no entry: its key goes too */
    return 1;
  }
  return wire_byte(w, GUATIAO_ABSENT) && wire_item(w);
}
static int wire_null(gt_sink *s) {
  wire_sink *w = (wire_sink *)s;
  return wire_byte(w, GUATIAO_NULL) && wire_item(w);
}
static int wire_bool(gt_sink *s, int b) {
  wire_sink *w = (wire_sink *)s;
  return wire_byte(w, GUATIAO_BOOL) && wire_byte(w, b ? 1 : 0) && wire_item(w);
}
static int wire_number(gt_sink *s, guatiao_str text) {
  wire_sink *w = (wire_sink *)s;
  if (!number_grammar(text)) return gt_fail(s->c, GUATIAO_ERR_BAD_VALUE, "'%.*s' is not a number", (int)text.len, text.ptr);
  return wire_byte(w, GUATIAO_NUMBER) && wire_body(w, text.ptr, text.len) && wire_item(w);
}
static int wire_text(gt_sink *s, guatiao_str text) {
  wire_sink *w = (wire_sink *)s;
  if (!utf8_valid((const uint8_t *)text.ptr, text.len)) return gt_fail(s->c, GUATIAO_ERR_BAD_VALUE, "text that is not UTF-8");
  return wire_byte(w, GUATIAO_STRING) && wire_body(w, text.ptr, text.len) && wire_item(w);
}
static int wire_bytes(gt_sink *s, const void *p, size_t n) {
  wire_sink *w = (wire_sink *)s;
  return wire_byte(w, GUATIAO_BYTES) && wire_body(w, p, n) && wire_item(w);
}
static int wire_node(gt_sink *s, const guatiao_value *src) {
  wire_sink *w = (wire_sink *)s;
  if (guatiao_tag_of(src) == GUATIAO_ABSENT) return wire_absent(s);
  if (!GT_NEED_CORE(s->c, guatiao_wire_encode)) return 0;
  guatiao_value out;
  set_absent(&out);
  guatiao_status st = gt_core.guatiao_wire_encode(&gt_binding_alloc, src, &out);
  if (st) {
    gt_free(&out);
    return gt_fail(s->c, st, "the copy was refused");
  }
  guatiao_bytes b = guatiao_bytes_data(&out);
  int ok = wire_room(w, b.len);
  if (ok && b.len) memcpy(w->buf + w->len, b.ptr, b.len);
  if (ok) w->len += b.len;
  gt_free(&out);
  return ok && wire_item(w);
}
static int wire_open(gt_sink *s, int is_map, size_t n) {
  wire_sink *w = (wire_sink *)s;
  if (w->depth >= GT_FRAMES) return gt_fail(s->c, GUATIAO_ERR_BAD_VALUE, "deeper than 128 nested containers");
  if (!wire_byte(w, is_map ? GUATIAO_MAP : GUATIAO_LIST)) return 0;
  wire_frame *f = &w->frames[w->depth];
  f->at = w->len;
  f->want = n;
  f->have = 0;
  f->key_at = 0;
  f->is_map = is_map;
  if (!wire_leb(w, n)) return 0;
  w->depth++;
  return 1;
}
static int wire_key(gt_sink *s, guatiao_str k) {
  wire_sink *w = (wire_sink *)s;
  if (!utf8_valid((const uint8_t *)k.ptr, k.len))
    return gt_fail(s->c, GUATIAO_ERR_BAD_VALUE, "the key '%.*s' was refused", (int)k.len, k.ptr);
  w->frames[w->depth - 1].key_at = w->len;
  return wire_body(w, k.ptr, k.len);
}
static int wire_close(gt_sink *s) {
  wire_sink *w = (wire_sink *)s;
  wire_frame *f = &w->frames[--w->depth];
  if (f->have != f->want) {
    /* Absent entries were dropped: the count is rewritten, shorter or as long. */
    size_t was = leb_size(f->want), now = leb_size(f->have);
    if (now != was) memmove(w->buf + f->at + now, w->buf + f->at + was, w->len - f->at - was);
    w->len -= was - now;
    leb_at(w->buf + f->at, f->have);
  }
  return wire_item(w);
}

static const gt_sink_ops wire_ops = {wire_absent, wire_null, wire_bool, wire_number, wire_text,
                                     wire_bytes,  wire_node, wire_open, wire_key,    wire_close};

int gt_wire_of(gt_call *c, const mxArray *x, uint8_t **bytes, size_t *len) {
  *bytes = NULL;
  *len = 0;
  wire_sink *w = (wire_sink *)gt_scratch(c, sizeof *w);
  if (!w) return 0;
  w->base.ops = &wire_ops;
  w->base.c = c;
  w->buf = NULL;
  w->len = w->cap = 0;
  w->depth = 0;
  if (!emit_rec(&w->base, x, 0)) {
    free(w->buf);
    return 0;
  }
  *bytes = w->buf;
  *len = w->len;
  return 1;
}

/* ---- reading -------------------------------------------------------------------------- */

static mxArray *call_maker(gt_call *c, const char *fn, int nin, mxArray **in) {
  mxArray *out = NULL;
  if (!gt_call_m(c, fn, 1, &out, nin, in)) return NULL;
  if (!out) gt_fail(c, GUATIAO_ERR_INTERNAL, "%s answered nothing", fn);
  return out;
}

mxArray *gt_number_mx(gt_call *c, guatiao_str t, int numbers) {
  char buf[400];
  int integral = 1;
  for (size_t i = 0; i < t.len; i++) {
    char ch = (char)t.ptr[i];
    if (ch == '.' || ch == 'e' || ch == 'E') integral = 0;
  }
  if (numbers != 1 && t.len > 0 && t.len < sizeof buf) {
    memcpy(buf, t.ptr, t.len);
    buf[t.len] = '\0';
    char *end = NULL;
    double d = strtod(buf, &end);
    if (numbers == 2) return mxCreateDoubleScalar(d);
    if (end == buf + t.len && isfinite(d)) {
      if (!integral) return mxCreateDoubleScalar(d);
      /* An integer spelling: a double up to 2^53, past it an int64 or uint64
       * that holds it, else a double only when that is exact. */
      int neg = buf[0] == '-';
      errno = 0;
      unsigned long long mag = strtoull(buf + neg, &end, 10);
      if (errno == 0 && end == buf + t.len) {
        if (mag <= 9007199254740992ULL) return mxCreateDoubleScalar(d);
        if (neg && mag <= 9223372036854775808ULL) {
          mxArray *x = mxCreateNumericMatrix(1, 1, mxINT64_CLASS, mxREAL);
          *(int64_t *)mxGetData(x) = mag == 9223372036854775808ULL ? INT64_MIN : -(int64_t)mag;
          return x;
        }
        if (!neg) {
          int fits = mag <= 9223372036854775807ULL;
          mxArray *x = mxCreateNumericMatrix(1, 1, fits ? mxINT64_CLASS : mxUINT64_CLASS, mxREAL);
          if (fits) *(int64_t *)mxGetData(x) = (int64_t)mag;
          else *(uint64_t *)mxGetData(x) = (uint64_t)mag;
          return x;
        }
      }
      char back[400];
      snprintf(back, sizeof back, "%.0f", d);
      if (strcmp(back, buf) == 0) return mxCreateDoubleScalar(d);
    }
  }
  mxArray *text = NULL;
  if (!gt_text_out(c, t, &text)) return NULL;
  mxArray *in[1] = {text};
  mxArray *n = call_maker(c, "guatiao.Number", 1, in);
  mxDestroyArray(text);
  return n;
}

static int is_field_name(guatiao_str k) {
  if (k.len == 0 || k.len > 63) return 0;
  for (size_t i = 0; i < k.len; i++) {
    unsigned char ch = k.ptr[i];
    int alpha = (ch >= 'A' && ch <= 'Z') || (ch >= 'a' && ch <= 'z');
    int digit = ch >= '0' && ch <= '9';
    if (i == 0 ? !alpha : !(alpha || digit || ch == '_')) return 0;
  }
  return 1;
}

static int read_rec(gt_call *c, const guatiao_value *v, const gt_read_opts *o, mxArray **out, int depth) {
  *out = NULL;
  if (depth > GT_DEPTH) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "deeper than 128 nested containers");
  switch (guatiao_tag_of(v)) {
  case GUATIAO_ABSENT: *out = call_maker(c, "guatiao.Absent", 0, NULL); return *out != NULL;
  case GUATIAO_NULL: *out = mxCreateDoubleMatrix(0, 0, mxREAL); return 1;
  case GUATIAO_BOOL: *out = mxCreateLogicalScalar(v->payload.b ? true : false); return 1;
  case GUATIAO_NUMBER: *out = gt_number_mx(c, guatiao_number_text(v), o->numbers); return *out != NULL;
  case GUATIAO_STRING: return gt_text_out(c, guatiao_string_text(v), out);
  case GUATIAO_BYTES: {
    guatiao_bytes b = guatiao_bytes_data(v);
    *out = mxCreateNumericMatrix(1, (mwSize)b.len, mxUINT8_CLASS, mxREAL);
    if (b.len) memcpy(mxGetData(*out), b.ptr, b.len);
    return 1;
  }
  case GUATIAO_LIST: {
    guatiao_values items = guatiao_list_items(v);
    mxArray *x = mxCreateCellMatrix(1, (mwSize)items.len);
    for (size_t i = 0; i < items.len; i++) {
      mxArray *e = NULL;
      if (!read_rec(c, &items.ptr[i], o, &e, depth + 1)) {
        mxDestroyArray(x);
        return 0;
      }
      mxSetCell(x, (mwIndex)i, e);
    }
    *out = x;
    return 1;
  }
  case GUATIAO_MAP: {
    guatiao_entries es = guatiao_map_entries(v);
    int as_struct = o->maps == 0;
    for (size_t i = 0; as_struct && i < es.len; i++) as_struct = is_field_name(guatiao_entry_key(&es.ptr[i]));
    if (as_struct) {
      const char **names = (const char **)gt_scratch(c, sizeof(char *) * (es.len + 1));
      if (!names) return 0;
      for (size_t i = 0; i < es.len; i++) {
        guatiao_str k = guatiao_entry_key(&es.ptr[i]);
        char *n = (char *)gt_scratch(c, k.len + 1);
        if (!n) return 0;
        memcpy(n, k.ptr, k.len);
        n[k.len] = '\0';
        names[i] = n;
      }
      mxArray *x = mxCreateStructMatrix(1, 1, (int)es.len, names);
      for (size_t i = 0; i < es.len; i++) {
        mxArray *e = NULL;
        if (!read_rec(c, &es.ptr[i].value, o, &e, depth + 1)) {
          mxDestroyArray(x);
          return 0;
        }
        mxSetFieldByNumber(x, 0, (int)i, e);
      }
      *out = x;
      return 1;
    }
    mxArray *keys = mxCreateCellMatrix(1, (mwSize)es.len);
    mxArray *vals = mxCreateCellMatrix(1, (mwSize)es.len);
    for (size_t i = 0; i < es.len; i++) {
      mxArray *k = NULL, *e = NULL;
      if (!gt_text_out(c, guatiao_entry_key(&es.ptr[i]), &k) || !read_rec(c, &es.ptr[i].value, o, &e, depth + 1)) {
        if (k) mxDestroyArray(k);
        mxDestroyArray(keys);
        mxDestroyArray(vals);
        return 0;
      }
      mxSetCell(keys, (mwIndex)i, k);
      mxSetCell(vals, (mwIndex)i, e);
    }
    mxArray *in[2] = {keys, vals};
    *out = call_maker(c, "guatiao.internal.make_map", 2, in);
    mxDestroyArray(keys);
    mxDestroyArray(vals);
    return *out != NULL;
  }
  default: *out = mxCreateDoubleMatrix(0, 0, mxREAL); return 1; /* a tag this build does not know */
  }
}

int gt_read(gt_call *c, const guatiao_value *v, const gt_read_opts *o, mxArray **out) {
  gt_read_opts def = {0, 0};
  return read_rec(c, v, o ? o : &def, out, 0);
}

/* ---- reading wire bytes: a whole document as MATLAB data, decoded here ------------------ */

typedef struct wire_reader {
  gt_call *c;
  const uint8_t *p;
  size_t n, at;
  const gt_read_opts *o;
} wire_reader;

static int wire_short(wire_reader *r) { return gt_fail(r->c, GUATIAO_ERR_BAD_VALUE, "the bytes end inside a node"); }

static int wire_count(wire_reader *r, uint64_t *out) {
  uint64_t v = 0;
  for (int shift = 0;; shift += 7) {
    if (r->at >= r->n) return wire_short(r);
    uint8_t b = r->p[r->at++];
    if (shift > 63 || (shift == 63 && b > 1)) return gt_fail(r->c, GUATIAO_ERR_BAD_VALUE, "a length past 64 bits");
    v |= (uint64_t)(b & 0x7F) << shift;
    if (b < 0x80) {
      if (b == 0 && shift > 0) return gt_fail(r->c, GUATIAO_ERR_BAD_VALUE, "a length not in its shortest form");
      *out = v;
      return 1;
    }
  }
}

static int wire_text_at(wire_reader *r, guatiao_str *t) {
  uint64_t len;
  if (!wire_count(r, &len)) return 0;
  if (len > r->n - r->at) return wire_short(r);
  *t = guatiao_str_from((const char *)r->p + r->at, (size_t)len);
  r->at += (size_t)len;
  return 1;
}

static int by_name(const void *a, const void *b) { return strcmp(*(const char *const *)a, *(const char *const *)b); }

/* A map's keys (into the bytes) and its values read: a struct when every
 * key is a field name and the reader may, else a containers.Map. */
static int wire_map_mx(wire_reader *r, const guatiao_str *keys, mxArray **vals, size_t n, mxArray **out) {
  gt_call *c = r->c;
  int as_struct = r->o->maps == 0;
  size_t bytes = 0;
  for (size_t i = 0; as_struct && i < n; i++) {
    as_struct = is_field_name(keys[i]);
    bytes += keys[i].len + 1;
  }
  if (as_struct) {
    const char **names = (const char **)malloc(2 * sizeof(char *) * (n ? n : 1) + bytes);
    if (!names) return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
    const char **sorted = names + n;
    char *text = (char *)(names + 2 * n);
    for (size_t i = 0; i < n; i++) {
      memcpy(text, keys[i].ptr, keys[i].len);
      text[keys[i].len] = '\0';
      names[i] = sorted[i] = text;
      text += keys[i].len + 1;
    }
    /* A struct cannot hold a field twice; the encoding never repeats a key. */
    qsort((void *)sorted, n, sizeof(char *), by_name);
    for (size_t i = 1; i < n; i++)
      if (!strcmp(sorted[i - 1], sorted[i])) {
        free((void *)names);
        return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "a key repeated in its map");
      }
    mxArray *x = mxCreateStructMatrix(1, 1, (int)n, names);
    free((void *)names);
    for (size_t i = 0; i < n; i++) {
      mxSetFieldByNumber(x, 0, (int)i, vals[i]);
      vals[i] = NULL;
    }
    *out = x;
    return 1;
  }
  mxArray *kc = mxCreateCellMatrix(1, (mwSize)n);
  mxArray *vc = mxCreateCellMatrix(1, (mwSize)n);
  for (size_t i = 0; i < n; i++) {
    mxArray *k = NULL;
    if (!gt_text_out(c, keys[i], &k)) {
      mxDestroyArray(kc);
      mxDestroyArray(vc);
      return 0;
    }
    mxSetCell(kc, (mwIndex)i, k);
    mxSetCell(vc, (mwIndex)i, vals[i]);
    vals[i] = NULL;
  }
  mxArray *in[2] = {kc, vc};
  *out = call_maker(c, "guatiao.internal.make_map", 2, in);
  mxDestroyArray(kc);
  mxDestroyArray(vc);
  return *out != NULL;
}

static int wire_node_at(wire_reader *r, mxArray **out, int depth) {
  gt_call *c = r->c;
  *out = NULL;
  if (depth > GT_DEPTH) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "deeper than 128 nested containers");
  if (r->at >= r->n) return wire_short(r);
  uint8_t tag = r->p[r->at++];
  guatiao_str t;
  switch (tag) {
  case GUATIAO_ABSENT: *out = call_maker(c, "guatiao.Absent", 0, NULL); return *out != NULL;
  case GUATIAO_NULL: *out = mxCreateDoubleMatrix(0, 0, mxREAL); return 1;
  case GUATIAO_BOOL: {
    if (r->at >= r->n) return wire_short(r);
    uint8_t b = r->p[r->at++];
    if (b > 1) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "a bool that is neither 0 nor 1");
    *out = mxCreateLogicalScalar(b ? true : false);
    return 1;
  }
  case GUATIAO_NUMBER:
    if (!wire_text_at(r, &t)) return 0;
    *out = gt_number_mx(c, t, r->o->numbers);
    return *out != NULL;
  case GUATIAO_STRING: return wire_text_at(r, &t) && gt_text_out(c, t, out);
  case GUATIAO_BYTES:
    if (!wire_text_at(r, &t)) return 0;
    *out = mxCreateNumericMatrix(1, (mwSize)t.len, mxUINT8_CLASS, mxREAL);
    if (t.len) memcpy(mxGetData(*out), t.ptr, t.len);
    return 1;
  case GUATIAO_LIST: {
    uint64_t len;
    if (!wire_count(r, &len)) return 0;
    /* Every item takes a byte at least: a longer count is refused before anything that large is made. */
    if (len > r->n - r->at) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "a count past what remains");
    mxArray *x = mxCreateCellMatrix(1, (mwSize)len);
    for (uint64_t i = 0; i < len; i++) {
      mxArray *e = NULL;
      if (!wire_node_at(r, &e, depth + 1)) {
        mxDestroyArray(x);
        return 0;
      }
      mxSetCell(x, (mwIndex)i, e);
    }
    *out = x;
    return 1;
  }
  case GUATIAO_MAP: {
    uint64_t len;
    if (!wire_count(r, &len)) return 0;
    /* Every entry takes two bytes at least. */
    if (len > (r->n - r->at) / 2) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "a count past what remains");
    size_t n = (size_t)len;
    guatiao_str *keys = (guatiao_str *)malloc((sizeof(guatiao_str) + sizeof(mxArray *)) * (n ? n : 1));
    if (!keys) return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
    mxArray **vals = (mxArray **)(keys + n);
    size_t done = 0;
    int ok = 1;
    for (; ok && done < n; done++) {
      vals[done] = NULL;
      ok = wire_text_at(r, &keys[done]) && wire_node_at(r, &vals[done], depth + 1);
    }
    if (ok) ok = wire_map_mx(r, keys, vals, n, out);
    for (size_t i = 0; i < done; i++)
      if (vals[i]) mxDestroyArray(vals[i]);
    free(keys);
    return ok;
  }
  default: return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "unknown tag %d", (int)tag);
  }
}

int gt_read_wire(gt_call *c, guatiao_bytes data, const gt_read_opts *o, mxArray **out) {
  gt_read_opts def = {0, 0};
  wire_reader r = {c, data.ptr, data.len, 0, o ? o : &def};
  if (!wire_node_at(&r, out, 0)) return 0;
  if (r.at != r.n) {
    mxDestroyArray(*out);
    *out = NULL;
    return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "bytes after the value");
  }
  return 1;
}

/* ---- tree arguments ----------------------------------------------------------------------- */

/* Whether `x` is a guatiao value or view; its owner id and path when so. */
int gt_node_ref(gt_call *c, const mxArray *x, mxArray **ref) {
  *ref = NULL;
  if (is_plain_class(mxGetClassID(x)) || mxGetClassID(x) == mxFUNCTION_CLASS) return 1;
  mxArray *in[1] = {(mxArray *)x};
  mxArray *r = NULL;
  if (!gt_call_m(c, "guatiao.internal.as_tree", 1, &r, 1, in)) return 0;
  if (r && mxIsCell(r) && mxGetNumberOfElements(r) == 3 && mxIsChar(mxGetCell(r, 0))) {
    char kind[16];
    mxGetString(mxGetCell(r, 0), kind, sizeof kind);
    if (!strcmp(kind, "node")) {
      *ref = r;
      return 1;
    }
  }
  if (r) mxDestroyArray(r);
  return 1;
}

int gt_tree_args(gt_call *c, int n, const int *at, guatiao_value *tmp, const guatiao_value **out) {
  mxArray *refs[8] = {NULL};
  if (n > 8) return gt_fail(c, GUATIAO_ERR_INTERNAL, "too many tree arguments");
  for (int i = 0; i < n; i++) set_absent(&tmp[i]);
  int ok = 1;
  for (int i = 0; ok && i < n; i++) {
    const mxArray *x = gt_arg(c, at[i]);
    if (!x) {
      ok = gt_fail(c, GUATIAO_ERR_BAD_VALUE, "a value is missing");
      break;
    }
    if (!gt_node_ref(c, x, &refs[i])) ok = 0;
    else if (!refs[i]) {
      if (!gt_build(c, x, &gt_binding_alloc, &tmp[i])) ok = 0;
      out[i] = &tmp[i];
    }
  }
  /* Nodes are found after all building, which may have run M code. */
  for (int i = 0; ok && i < n; i++) {
    if (!refs[i]) continue;
    guatiao_value *found = NULL;
    if (!gt_walk_mx(c, mxGetCell(refs[i], 1), mxGetCell(refs[i], 2), &found)) ok = 0;
    else out[i] = found;
  }
  for (int i = 0; i < n; i++)
    if (refs[i]) mxDestroyArray(refs[i]);
  if (!ok)
    for (int i = 0; i < n; i++) gt_free(&tmp[i]);
  return ok;
}
