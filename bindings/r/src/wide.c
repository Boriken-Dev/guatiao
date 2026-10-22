/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * 64-bit integers, which an R number holds exactly only to 2^53: read
 * exactly from an R integer, a whole double, a `number` or a string, and
 * answered as an integer, a double while it is exact, or a `number` (its
 * exact text), as `int64 =` says.
 */

#include <math.h>
#include <stdio.h>
#include <string.h>

#include "glue.h"

#define TWO53 9007199254740992ULL
#define TWO64 18446744073709551616.0

static void from_i64(int64_t v, gt_wide *w) {
  w->neg = v < 0;
  w->mag = v < 0 ? (uint64_t)0 - (uint64_t)v : (uint64_t)v;
}

const char *gt_wide_text(const char *p, gt_wide *w) {
  memset(w, 0, sizeof *w);
  if (*p == '-') {
    w->neg = 1;
    p++;
  }
  if (!*p) return "want a whole number: digits, after a - when negative";
  for (; *p; p++) {
    if (*p < '0' || *p > '9') return "want a whole number: digits, after a - when negative";
    unsigned d = (unsigned)(*p - '0');
    if (w->mag > (UINT64_MAX - d) / 10) w->over = 1;
    else w->mag = w->mag * 10 + d;
  }
  if (w->mag == 0 && !w->over) w->neg = 0;
  return NULL;
}

const char *gt_wide_in(SEXP x, gt_wide *w) {
  memset(w, 0, sizeof *w);
  if (Rf_xlength(x) != 1) return "want one number";
  switch (TYPEOF(x)) {
  case LGLSXP: {
    int b = LOGICAL(x)[0];
    if (b == NA_LOGICAL) return "want a number, not NA";
    w->mag = b ? 1 : 0;
    return NULL;
  }
  case INTSXP: {
    int v = INTEGER(x)[0];
    if (v == NA_INTEGER) return "want a number, not NA";
    from_i64(v, w);
    return NULL;
  }
  case REALSXP: {
    double d = REAL(x)[0];
    if (ISNAN(d)) return "want a number, not NA";
    if (!R_FINITE(d) || d != floor(d)) return "want a whole number";
    w->neg = d < 0;
    double m = fabs(d);
    if (m >= TWO64) w->over = 1;
    else w->mag = (uint64_t)m;
    if (w->mag == 0 && !w->over) w->neg = 0;
    return NULL;
  }
  case STRSXP: {
    SEXP c = STRING_ELT(x, 0);
    if (c == NA_STRING) return "want a number, not NA";
    return gt_wide_text(CHAR(c), w);
  }
  default: return "want a number";
  }
}

static const char *out_of_range(int is_unsigned, int bits) {
  switch (bits) {
  case 8: return is_unsigned ? "out of range for uint8" : "out of range for int8";
  case 16: return is_unsigned ? "out of range for uint16" : "out of range for int16";
  case 32: return is_unsigned ? "out of range for uint32" : "out of range for int32";
  default: return is_unsigned ? "out of range for uint64" : "out of range for int64";
  }
}

const char *gt_wide_signed(gt_wide w, int bits, int64_t *out) {
  uint64_t most = ((uint64_t)1 << (bits - 1)) - 1;
  if (w.over || w.mag > most + (w.neg ? 1 : 0)) return out_of_range(0, bits);
  *out = !w.neg ? (int64_t)w.mag : w.mag == (uint64_t)1 << 63 ? INT64_MIN : -(int64_t)w.mag;
  return NULL;
}

const char *gt_wide_unsigned(gt_wide w, int bits, uint64_t *out) {
  uint64_t most = bits == 64 ? UINT64_MAX : ((uint64_t)1 << bits) - 1;
  if (w.over || w.neg || w.mag > most) return out_of_range(1, bits);
  *out = w.mag;
  return NULL;
}

SEXP gt_number_sexp(const char *text) {
  SEXP x = PROTECT(Rf_mkString(text));
  Rf_setAttrib(x, R_ClassSymbol, Rf_mkString("guatiao_number"));
  UNPROTECT(1);
  return x;
}

static SEXP wide_sexp(gt_wide w, int mode) {
  char text[24];
  snprintf(text, sizeof text, "%s%llu", w.neg ? "-" : "", (unsigned long long)w.mag);
  if (mode == GT_INT64_NUMBER) return gt_number_sexp(text);
  if (mode == GT_INT64_DOUBLE) return Rf_ScalarReal(w.neg ? -(double)w.mag : (double)w.mag);
  /* auto: an R integer (whose INT_MIN is NA), a double while it is exact, then the text */
  if (w.mag <= (uint64_t)INT32_MAX) return Rf_ScalarInteger(w.neg ? -(int)w.mag : (int)w.mag);
  if (w.mag <= TWO53) return Rf_ScalarReal(w.neg ? -(double)w.mag : (double)w.mag);
  return gt_number_sexp(text);
}

SEXP gt_i64_sexp(int64_t v, int mode) {
  gt_wide w = {0, 0, 0};
  from_i64(v, &w);
  return wide_sexp(w, mode);
}

SEXP gt_u64_sexp(uint64_t v, int mode) {
  gt_wide w = {0, v, 0};
  return wide_sexp(w, mode);
}

/* ---- .Call entry points --------------------------------------------------- */

/* The units of a span or an exact length, as R holds them: doubles while
 * every one is within 2^53, else the exact text of each. `x` is a list of
 * R values, `names` what each is called in a refusal, `is_unsigned` a
 * logical per unit. */
SEXP C_units(SEXP x, SEXP names, SEXP is_unsigned) {
  if (TYPEOF(x) != VECSXP || TYPEOF(names) != STRSXP || TYPEOF(is_unsigned) != LGLSXP)
    gt_raise(GUATIAO_ERR_BAD_VALUE, "units are a list, with a name and a kind each");
  R_xlen_t n = XLENGTH(x);
  if (n > 16 || XLENGTH(names) != n || XLENGTH(is_unsigned) != n)
    gt_raise(GUATIAO_ERR_BAD_VALUE, "units are a list, with a name and a kind each");
  gt_wide w[16];
  int wide = 0;
  for (R_xlen_t i = 0; i < n; i++) {
    const char *what = CHAR(STRING_ELT(names, i));
    const char *why = gt_wide_in(VECTOR_ELT(x, i), &w[i]);
    if (!why) {
      int64_t s;
      uint64_t u;
      why = LOGICAL(is_unsigned)[i] == 1 ? gt_wide_unsigned(w[i], 64, &u) : gt_wide_signed(w[i], 64, &s);
    }
    if (why) gt_raise(GUATIAO_ERR_BAD_VALUE, "%s: %s", what, why);
    if (w[i].mag > TWO53) wide = 1;
  }
  if (!wide) {
    SEXP out = PROTECT(Rf_allocVector(REALSXP, n));
    for (R_xlen_t i = 0; i < n; i++) REAL(out)[i] = w[i].neg ? -(double)w[i].mag : (double)w[i].mag;
    UNPROTECT(1);
    return out;
  }
  SEXP out = PROTECT(Rf_allocVector(STRSXP, n));
  char text[24];
  for (R_xlen_t i = 0; i < n; i++) {
    snprintf(text, sizeof text, "%s%llu", w[i].neg ? "-" : "", (unsigned long long)w[i].mag);
    SET_STRING_ELT(out, i, Rf_mkChar(text));
  }
  UNPROTECT(1);
  return out;
}

/* One unit as `int64 =` answers it: `x` as C_units holds it. */
SEXP C_unit_answer(SEXP x, SEXP mode) {
  gt_wide w;
  const char *why = gt_wide_in(x, &w);
  if (why) gt_raise(GUATIAO_ERR_BAD_VALUE, "a unit: %s", why);
  if (w.over) gt_raise(GUATIAO_ERR_BAD_VALUE, "a unit: out of range for uint64");
  return wide_sexp(w, Rf_asInteger(mode));
}
