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

static int number_from(gt_call *c, const guatiao_alloc *a, const char *text, guatiao_value *out) {
  guatiao_status st = gt_core.guatiao_value_number(a, guatiao_str_from(text, strlen(text)), out);
  if (st) return gt_fail(c, st, "'%s' is not a number", text);
  return 1;
}

/* Element `i` of a numeric or logical array as a scalar node. */
static int scalar_at(gt_call *c, const mxArray *x, size_t i, const guatiao_alloc *a, guatiao_value *out) {
  char buf[64];
  const void *d = mxGetData(x);
  switch (mxGetClassID(x)) {
  case mxLOGICAL_CLASS: return gt_core.guatiao_value_bool(((const mxLogical *)d)[i] != 0, out) == 0;
  case mxDOUBLE_CLASS: {
    /* Octave 10 hands a MEX file the data of a literal 9007199254740992
     * inside a cell or struct as an int64's bits; mxGetScalar converts. */
    double v = mxGetNumberOfElements(x) == 1 ? mxGetScalar(x) : ((const double *)d)[i];
    if (!isfinite(v)) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "a value holds no NaN or infinity");
    double_text(v, buf, sizeof buf);
    break;
  }
  case mxSINGLE_CLASS: {
    float v = ((const float *)d)[i];
    if (!isfinite(v)) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "a value holds no NaN or infinity");
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
  default: return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "cannot write a %s", mxGetClassName(x));
  }
  return number_from(c, a, buf, out);
}

/* ---- building ------------------------------------------------------------------------ */

static int build_rec(gt_call *c, const mxArray *x, const guatiao_alloc *a, guatiao_value *out, int depth);

static int push_child(gt_call *c, const guatiao_alloc *a, guatiao_value *list, guatiao_value *child) {
  guatiao_status st = gt_core.guatiao_list_push(a, list, child);
  if (st) {
    gt_free(child);
    return gt_fail(c, st, "a list refused an element");
  }
  return 1;
}

static int set_child(gt_call *c, const guatiao_alloc *a, guatiao_value *map, guatiao_str key, guatiao_value *child) {
  if (child->tag == GUATIAO_ABSENT) return 1; /* an absent entry is no entry */
  size_t before = guatiao_map_entries(map).len;
  guatiao_status st = gt_core.guatiao_map_set(a, map, key, child);
  if (st) {
    gt_free(child);
    return gt_fail(c, st, "the key '%.*s' was refused", (int)key.len, key.ptr);
  }
  if (guatiao_map_entries(map).len == before)
    return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "two entries are named '%.*s'", (int)key.len, key.ptr);
  return 1;
}

static int new_container(gt_call *c, const guatiao_alloc *a, int map, guatiao_value *out) {
  guatiao_status st = map ? gt_core.guatiao_value_map(a, out) : gt_core.guatiao_value_list(a, out);
  if (st) return gt_fail(c, st, "the allocator refused");
  return 1;
}

static int fail_out(guatiao_value *out) {
  gt_free(out);
  set_absent(out);
  return 0;
}

/* A numeric or logical array of more than one element: a list, or a list
 * of rows for a matrix. */
static int numeric_list(gt_call *c, const mxArray *x, const guatiao_alloc *a, guatiao_value *out) {
  size_t m = mxGetM(x), n = mxGetN(x);
  if (!new_container(c, a, 0, out)) return 0;
  int matrix = m > 1 && n > 1;
  size_t rows = matrix ? m : 1, cols = matrix ? n : m * n;
  for (size_t r = 0; r < rows; r++) {
    guatiao_value row;
    set_absent(&row);
    guatiao_value *into = out;
    if (matrix) {
      if (!new_container(c, a, 0, &row)) return fail_out(out);
      into = &row;
    }
    for (size_t k = 0; k < cols; k++) {
      guatiao_value item;
      set_absent(&item);
      size_t at = matrix ? r + k * m : k; /* column-major storage */
      if (!scalar_at(c, x, at, a, &item) || !push_child(c, a, into, &item)) {
        if (matrix) gt_free(&row);
        return fail_out(out);
      }
    }
    if (matrix && !push_child(c, a, out, &row)) return fail_out(out);
  }
  return 1;
}

static int cell_list(gt_call *c, const mxArray *x, const guatiao_alloc *a, guatiao_value *out, int depth) {
  size_t m = mxGetM(x), n = mxGetN(x);
  if (!new_container(c, a, 0, out)) return 0;
  int matrix = m > 1 && n > 1;
  size_t rows = matrix ? m : 1, cols = matrix ? n : m * n;
  for (size_t r = 0; r < rows; r++) {
    guatiao_value row;
    set_absent(&row);
    guatiao_value *into = out;
    if (matrix) {
      if (!new_container(c, a, 0, &row)) return fail_out(out);
      into = &row;
    }
    for (size_t k = 0; k < cols; k++) {
      guatiao_value item;
      size_t at = matrix ? r + k * m : k;
      const mxArray *e = mxGetCell(x, (mwIndex)at);
      if (!e) {
        set_absent(&item);
        if (gt_core.guatiao_value_null(&item)) return fail_out(out);
      } else if (!build_rec(c, e, a, &item, depth + 1)) {
        if (matrix) gt_free(&row);
        return fail_out(out);
      }
      if (!push_child(c, a, into, &item)) {
        if (matrix) gt_free(&row);
        return fail_out(out);
      }
    }
    if (matrix && !push_child(c, a, out, &row)) return fail_out(out);
  }
  return 1;
}

/* Element `i` of a struct array as a map of its fields, in order. */
static int struct_map(gt_call *c, const mxArray *x, size_t i, const guatiao_alloc *a, guatiao_value *out, int depth) {
  if (!new_container(c, a, 1, out)) return 0;
  int nf = mxGetNumberOfFields(x);
  for (int f = 0; f < nf; f++) {
    const char *name = mxGetFieldNameByNumber(x, f);
    const mxArray *e = mxGetFieldByNumber(x, (mwIndex)i, f);
    guatiao_value item;
    set_absent(&item);
    if (!e) {
      if (gt_core.guatiao_value_null(&item)) return fail_out(out);
    } else if (!build_rec(c, e, a, &item, depth + 1)) {
      return fail_out(out);
    }
    if (!set_child(c, a, out, guatiao_str_from(name, strlen(name)), &item)) return fail_out(out);
  }
  return 1;
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
static int build_object(gt_call *c, const mxArray *x, const guatiao_alloc *a, guatiao_value *out, int depth) {
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
    ok = p1 ? build_rec(c, p1, a, out, depth + 1) : gt_core.guatiao_value_null(out) == 0;
  } else if (!strcmp(kind, "absent")) {
    set_absent(out);
    ok = 1;
  } else if (!strcmp(kind, "number")) {
    guatiao_str t;
    if (lowered_text(c, p1, "a number's text", &t)) {
      guatiao_status st = gt_core.guatiao_value_number(a, t, out);
      ok = st ? gt_fail(c, st, "'%.*s' is not a number", (int)t.len, t.ptr) : 1;
    }
  } else if (!strcmp(kind, "node")) {
    guatiao_value *src = NULL;
    if (gt_walk_mx(c, p1, p2, &src)) {
      guatiao_status st = gt_core.guatiao_value_clone(a, src, out);
      ok = st ? gt_fail(c, st, "the copy was refused") : 1;
    }
  } else if (!strcmp(kind, "map")) {
    /* p1 the keys (a cell of text), p2 the values (a cell). */
    if (!p1 || !p2 || !mxIsCell(p1) || !mxIsCell(p2) || mxGetNumberOfElements(p1) != mxGetNumberOfElements(p2)) {
      ok = gt_fail(c, GUATIAO_ERR_INTERNAL, "a lowered map is two cells of one length");
    } else if (new_container(c, a, 1, out)) {
      ok = 1;
      for (size_t i = 0; ok && i < mxGetNumberOfElements(p1); i++) {
        guatiao_str k;
        guatiao_value item;
        set_absent(&item);
        if (!gt_mx_utf8(c, mxGetCell(p1, (mwIndex)i), "a map key", &k)) ok = 0;
        else if (!build_rec(c, mxGetCell(p2, (mwIndex)i), a, &item, depth + 1)) ok = 0;
        else if (!set_child(c, a, out, k, &item)) ok = 0;
      }
      if (!ok) fail_out(out);
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

static int build_rec(gt_call *c, const mxArray *x, const guatiao_alloc *a, guatiao_value *out, int depth) {
  set_absent(out);
  if (depth > GT_DEPTH) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "deeper than 128 nested containers");
  mxClassID id = mxGetClassID(x);
  if (!is_plain_class(id)) {
    if (id == mxFUNCTION_CLASS) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "cannot write a function handle");
    return build_object(c, x, a, out, depth);
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
    if (!gt_mx_utf8(c, x, "text", &t)) return 0;
    guatiao_status st = gt_core.guatiao_value_string(a, t, out);
    return st ? gt_fail(c, st, "text that is not UTF-8") : 1;
  }
  case mxUINT8_CLASS: {
    guatiao_status st = gt_core.guatiao_value_bytes(a, guatiao_bytes_from(n ? mxGetData(x) : NULL, n), out);
    return st ? gt_fail(c, st, "the allocator refused") : 1;
  }
  case mxCELL_CLASS: return cell_list(c, x, a, out, depth);
  case mxSTRUCT_CLASS:
    if (n == 1) return struct_map(c, x, 0, a, out, depth);
    if (mxGetM(x) > 1 && mxGetN(x) > 1) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "cannot write a matrix of structs");
    if (!new_container(c, a, 0, out)) return 0;
    for (size_t i = 0; i < n; i++) {
      guatiao_value item;
      if (!struct_map(c, x, i, a, &item, depth + 1) || !push_child(c, a, out, &item)) return fail_out(out);
    }
    return 1;
  default:
    if (n == 0) return gt_core.guatiao_value_null(out) == 0 ? 1 : gt_fail(c, GUATIAO_ERR_INTERNAL, "null");
    if (n == 1) return scalar_at(c, x, 0, a, out);
    return numeric_list(c, x, a, out);
  }
}

int gt_build(gt_call *c, const mxArray *x, const guatiao_alloc *a, guatiao_value *out) {
  set_absent(out);
  if (!GT_NEED_CORE(c, guatiao_value_map)) return 0;
  if (!build_rec(c, x, a ? a : &gt_binding_alloc, out, 0)) {
    set_absent(out);
    return 0;
  }
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
