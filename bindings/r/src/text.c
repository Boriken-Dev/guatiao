/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/* Opening the libraries, the text forms (dates, times, instants, spans,
 * exact lengths), redaction and schema checks. */

#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "glue.h"

static void set_absent(guatiao_value *v) { memset(v, 0, sizeof *v); }

/* ---- opening ---------------------------------------------------------------- */

SEXP C_open_library(SEXP lib, SEXP path) {
  char err[512];
  int l = Rf_asInteger(lib);
  const char *p = Rf_translateCharUTF8(STRING_ELT(path, 0));
  int r = gt_open(l, p, err, sizeof err);
  SEXP out = PROTECT(Rf_allocVector(VECSXP, 2));
  SET_VECTOR_ELT(out, 0, Rf_ScalarInteger(r));
  SET_VECTOR_ELT(out, 1, Rf_mkString(r ? err : ""));
  UNPROTECT(1);
  return out;
}

SEXP C_library_is_open(SEXP lib) { return Rf_ScalarLogical(gt_is_open(Rf_asInteger(lib))); }

SEXP C_file_carries(SEXP path, SEXP lib) {
  const char *p = Rf_translateCharUTF8(STRING_ELT(path, 0));
  return Rf_ScalarLogical(gt_file_carries(p, gt_probe_of(Rf_asInteger(lib))));
}

SEXP C_has_export(SEXP lib, SEXP name) {
  int l = Rf_asInteger(lib);
  gt_need(l);
  return Rf_ScalarLogical(gt_symbol_of(l, CHAR(STRING_ELT(name, 0))) != NULL);
}

/* ---- the text forms ------------------------------------------------------------ */

enum { F_DATE, F_TIME, F_DATETIME, F_TIMESTAMP, F_SPAN };

static const char *form_name(int f) {
  static const char *names[] = {"date", "time", "datetime", "timestamp", "span"};
  return f >= 0 && f <= 4 ? names[f] : "?";
}

static void raise_unparsed(int form, guatiao_text_error *e) {
  SEXP f = PROTECT(Rf_mkString(form_name(form)));
  SEXP at = PROTECT(Rf_ScalarReal((double)e->at));
  guatiao_str t = guatiao_string_text(&e->expected);
  SEXP ex = PROTECT(gt_mkchar(t) ? Rf_ScalarString(gt_mkchar(t)) : Rf_mkString(""));
  gt_free(&e->expected);
  gt_call_r(".gt_unparsed", 3, f, at, ex);
  UNPROTECT(3);
  Rf_error("unreachable");
}

static SEXP doubles(int n, const double *v) {
  SEXP x = Rf_allocVector(REALSXP, n);
  for (int i = 0; i < n; i++) REAL(x)[i] = v[i];
  return x;
}

#define TWO53 9007199254740992.0

static double from_i64(int64_t v) {
  double d = (double)v;
  if (fabs(d) > TWO53) gt_raise(GUATIAO_ERR_BAD_VALUE, "a unit past 2^53 cannot be an R number");
  return d;
}

static int64_t to_i64(double d, const char *what) {
  if (!R_FINITE(d) || d != floor(d) || fabs(d) >= 9223372036854775808.0)
    gt_raise(GUATIAO_ERR_BAD_VALUE, "%s is a whole number", what);
  return (int64_t)d;
}

static int64_t text_i64(SEXP c, const char *what) {
  gt_wide w = {0, 0, 0};
  int64_t v = 0;
  const char *why = c == NA_STRING ? "want a number, not NA" : gt_wide_text(CHAR(c), &w);
  if (!why) why = gt_wide_signed(w, 64, &v);
  if (why) gt_raise(GUATIAO_ERR_BAD_VALUE, "%s: %s", what, why);
  return v;
}

/* A span from eight numbers, or eight exact texts (units past 2^53). */
static void span_of(SEXP x, guatiao_span *s) {
  if (TYPEOF(x) == STRSXP && XLENGTH(x) == 8) {
    static const char *names[] = {"years", "months", "weeks", "days", "hours", "minutes", "seconds", "nanoseconds"};
    int64_t v[8];
    for (int i = 0; i < 8; i++) v[i] = text_i64(STRING_ELT(x, i), names[i]);
    s->years = v[0];
    s->months = v[1];
    s->weeks = v[2];
    s->days = v[3];
    s->hours = v[4];
    s->minutes = v[5];
    s->seconds = v[6];
    if (v[7] < -2147483647LL || v[7] > 2147483647LL) gt_raise(GUATIAO_ERR_BAD_VALUE, "nanoseconds is out of range");
    s->nanoseconds = (int32_t)v[7];
    return;
  }
  if (TYPEOF(x) != REALSXP || XLENGTH(x) != 8) gt_raise(GUATIAO_ERR_BAD_VALUE, "a span is eight numbers");
  const double *v = REAL(x);
  s->years = to_i64(v[0], "years");
  s->months = to_i64(v[1], "months");
  s->weeks = to_i64(v[2], "weeks");
  s->days = to_i64(v[3], "days");
  s->hours = to_i64(v[4], "hours");
  s->minutes = to_i64(v[5], "minutes");
  s->seconds = to_i64(v[6], "seconds");
  double ns = v[7];
  if (!R_FINITE(ns) || ns != floor(ns) || fabs(ns) > 2147483647.0)
    gt_raise(GUATIAO_ERR_BAD_VALUE, "nanoseconds is a whole number");
  s->nanoseconds = (int32_t)ns;
}

static int field_int(double d, double lo, double hi, const char *what) {
  if (!R_FINITE(d) || d != floor(d) || d < lo || d > hi) gt_raise(GUATIAO_ERR_BAD_VALUE, "%s is out of range", what);
  return (int)d;
}

static void date_of(const double *v, guatiao_date *d) {
  d->year = field_int(v[0], -2147483648.0, 2147483647.0, "the year");
  d->month = (uint8_t)field_int(v[1], 0, 255, "the month");
  d->day = (uint8_t)field_int(v[2], 0, 255, "the day");
}

static void time_of(const double *v, guatiao_time *t) {
  t->hour = (uint8_t)field_int(v[0], 0, 255, "the hour");
  t->minute = (uint8_t)field_int(v[1], 0, 255, "the minute");
  t->second = (uint8_t)field_int(v[2], 0, 255, "the second");
  t->nanosecond = (uint32_t)field_int(v[3], 0, 4294967295.0, "the nanosecond");
}

static const double *fields(SEXP x, int n, const char *what) {
  if (TYPEOF(x) != REALSXP || XLENGTH(x) != n) gt_raise(GUATIAO_ERR_BAD_VALUE, "%s is %d numbers", what, n);
  return REAL(x);
}

/* Fields as numbers (refusing a unit past 2^53), or as exact text. */
static SEXP fields_out(const int64_t *v, int n, int exact) {
  if (exact) {
    SEXP x = PROTECT(Rf_allocVector(STRSXP, n));
    char buf[32];
    for (int i = 0; i < n; i++) {
      snprintf(buf, sizeof buf, "%lld", (long long)v[i]);
      SET_STRING_ELT(x, i, Rf_mkChar(buf));
    }
    UNPROTECT(1);
    return x;
  }
  SEXP x = PROTECT(Rf_allocVector(REALSXP, n));
  for (int i = 0; i < n; i++) REAL(x)[i] = from_i64(v[i]);
  UNPROTECT(1);
  return x;
}

SEXP C_time_parse(SEXP form, SEXP text, SEXP exact_) {
  int f = Rf_asInteger(form);
  int exact = Rf_asLogical(exact_) == 1;
  guatiao_str t = gt_str_arg(text, "the text");
  guatiao_text_error e;
  memset(&e, 0, sizeof e);
  guatiao_status st;
  switch (f) {
  case F_DATE: {
    guatiao_date d;
    st = GT_CORE_FN(guatiao_date_parse)(&gt_binding_alloc, t, &d, &e);
    if (st == GUATIAO_ERR_BAD_VALUE) raise_unparsed(f, &e);
    if (st) gt_raise(st, "the date was refused");
    int64_t v[3] = {d.year, d.month, d.day};
    return fields_out(v, 3, exact);
  }
  case F_TIME: {
    guatiao_time c;
    st = GT_CORE_FN(guatiao_time_parse)(&gt_binding_alloc, t, &c, &e);
    if (st == GUATIAO_ERR_BAD_VALUE) raise_unparsed(f, &e);
    if (st) gt_raise(st, "the time was refused");
    int64_t v[4] = {c.hour, c.minute, c.second, c.nanosecond};
    return fields_out(v, 4, exact);
  }
  case F_DATETIME: {
    guatiao_datetime c;
    st = GT_CORE_FN(guatiao_datetime_parse)(&gt_binding_alloc, t, &c, &e);
    if (st == GUATIAO_ERR_BAD_VALUE) raise_unparsed(f, &e);
    if (st) gt_raise(st, "the date and time were refused");
    int64_t v[7] = {c.date.year, c.date.month, c.date.day, c.time.hour, c.time.minute, c.time.second,
                    c.time.nanosecond};
    return fields_out(v, 7, exact);
  }
  case F_TIMESTAMP: {
    guatiao_timestamp c;
    st = GT_CORE_FN(guatiao_timestamp_parse)(&gt_binding_alloc, t, &c, &e);
    if (st == GUATIAO_ERR_BAD_VALUE) raise_unparsed(f, &e);
    if (st) gt_raise(st, "the instant was refused");
    int64_t v[2] = {c.seconds, c.nanoseconds};
    return fields_out(v, 2, exact);
  }
  case F_SPAN: {
    guatiao_span s;
    st = GT_CORE_FN(guatiao_span_parse)(&gt_binding_alloc, t, &s, &e);
    if (st == GUATIAO_ERR_BAD_VALUE) raise_unparsed(f, &e);
    if (st) gt_raise(st, "the span was refused");
    int64_t v[8] = {s.years, s.months, s.weeks, s.days, s.hours, s.minutes, s.seconds, s.nanoseconds};
    return fields_out(v, 8, exact);
  }
  default: gt_raise(GUATIAO_ERR_BAD_VALUE, "no such form");
  }
}

SEXP C_time_format(SEXP form, SEXP x) {
  int f = Rf_asInteger(form);
  guatiao_value out;
  set_absent(&out);
  guatiao_status st;
  switch (f) {
  case F_DATE: {
    guatiao_date d;
    date_of(fields(x, 3, "a date"), &d);
    st = GT_CORE_FN(guatiao_date_format)(&gt_binding_alloc, &d, &out);
    break;
  }
  case F_TIME: {
    guatiao_time c;
    time_of(fields(x, 4, "a time"), &c);
    st = GT_CORE_FN(guatiao_time_format)(&gt_binding_alloc, &c, &out);
    break;
  }
  case F_DATETIME: {
    guatiao_datetime c;
    const double *v = fields(x, 7, "a date and time");
    date_of(v, &c.date);
    time_of(v + 3, &c.time);
    st = GT_CORE_FN(guatiao_datetime_format)(&gt_binding_alloc, &c, &out);
    break;
  }
  case F_TIMESTAMP: {
    guatiao_timestamp c;
    const double *v = fields(x, 2, "an instant");
    c.seconds = to_i64(v[0], "seconds");
    c.nanoseconds = (uint32_t)field_int(v[1], 0, 4294967295.0, "nanoseconds");
    st = GT_CORE_FN(guatiao_timestamp_format)(&gt_binding_alloc, &c, &out);
    break;
  }
  case F_SPAN: {
    guatiao_span s;
    span_of(x, &s);
    st = GT_CORE_FN(guatiao_span_format)(&gt_binding_alloc, &s, &out);
    break;
  }
  default: gt_raise(GUATIAO_ERR_BAD_VALUE, "no such form");
  }
  if (st) gt_raise(st, "not a %s", form_name(f));
  return gt_take_text(&out);
}

SEXP C_time_normalize(SEXP form, SEXP text) {
  int f = Rf_asInteger(form);
  guatiao_str t = gt_str_arg(text, "the text");
  guatiao_text_error e;
  memset(&e, 0, sizeof e);
  guatiao_value out;
  set_absent(&out);
  guatiao_status st;
  switch (f) {
  case F_DATE: st = GT_CORE_FN(guatiao_date_normalize)(&gt_binding_alloc, t, &out, &e); break;
  case F_TIME: st = GT_CORE_FN(guatiao_time_normalize)(&gt_binding_alloc, t, &out, &e); break;
  case F_DATETIME: st = GT_CORE_FN(guatiao_datetime_normalize)(&gt_binding_alloc, t, &out, &e); break;
  case F_TIMESTAMP: st = GT_CORE_FN(guatiao_timestamp_normalize)(&gt_binding_alloc, t, &out, &e); break;
  case F_SPAN: st = GT_CORE_FN(guatiao_span_normalize)(&gt_binding_alloc, t, &out, &e); break;
  default: gt_raise(GUATIAO_ERR_BAD_VALUE, "no such form");
  }
  if (st == GUATIAO_ERR_BAD_VALUE) raise_unparsed(f, &e);
  if (st) gt_raise(st, "the %s was refused", form_name(f));
  return gt_take_text(&out);
}

SEXP C_date_add_span(SEXP date, SEXP span) {
  guatiao_date d, out;
  guatiao_span s;
  date_of(fields(date, 3, "a date"), &d);
  span_of(span, &s);
  guatiao_status st = GT_CORE_FN(guatiao_date_add_span)(&d, &s, &out);
  if (st) gt_raise(st, "the span does not apply to the date");
  double v[3] = {out.year, out.month, out.day};
  return doubles(3, v);
}

SEXP C_datetime_add_span(SEXP at, SEXP span) {
  guatiao_datetime d, out;
  guatiao_span s;
  const double *v = fields(at, 7, "a date and time");
  date_of(v, &d.date);
  time_of(v + 3, &d.time);
  span_of(span, &s);
  guatiao_status st = GT_CORE_FN(guatiao_datetime_add_span)(&d, &s, &out);
  if (st) gt_raise(st, "the span does not apply to the date and time");
  double r[7] = {out.date.year, out.date.month, out.date.day, out.time.hour, out.time.minute, out.time.second,
                 out.time.nanosecond};
  return doubles(7, r);
}

/* An exact length: its seconds as exact text, its nanoseconds as a number. */
SEXP C_duration_read(SEXP x) {
  guatiao_value tmp;
  const guatiao_value *v = gt_tree_arg(x, &tmp);
  uint64_t s = 0;
  uint32_t ns = 0;
  guatiao_status st = GT_CORE_FN(guatiao_duration_read)(v, &s, &ns);
  gt_free(&tmp);
  if (st) gt_raise(st, "not an exact length: [seconds, nanoseconds], seconds, or {secs, nanos}");
  char buf[32];
  snprintf(buf, sizeof buf, "%llu", (unsigned long long)s);
  SEXP out = PROTECT(Rf_allocVector(VECSXP, 2));
  SET_VECTOR_ELT(out, 0, Rf_mkString(buf));
  SET_VECTOR_ELT(out, 1, Rf_ScalarReal((double)ns));
  UNPROTECT(1);
  return out;
}

/* ---- schemas --------------------------------------------------------------------- */

SEXP C_redact(SEXP schema, SEXP value, SEXP placeholder, SEXP alloc) {
  const guatiao_alloc *a = gt_alloc_arg(alloc);
  guatiao_value ts, tv, out;
  set_absent(&out);
  guatiao_str p = gt_str_arg(placeholder, "the placeholder");
  const guatiao_value *s, *v;
  gt_tree_args(schema, &ts, &s, value, &tv, &v, R_NilValue, NULL, NULL);
  guatiao_status st = GT_CORE_FN(guatiao_schema_redact)(s, v, p, a, &out);
  gt_free(&ts);
  gt_free(&tv);
  if (st) {
    gt_free(&out);
    gt_raise(st, st == GUATIAO_ERR_WRONG_KIND ? "the schema is not a map" : "the value could not be redacted");
  }
  SEXP owner = gt_owner_new(&out, a, alloc);
  if (!owner) gt_raise(GUATIAO_ERR_ALLOC, "out of memory");
  return owner;
}

SEXP C_schema_validate(SEXP schema, SEXP value) {
  guatiao_value ts, tv, err;
  set_absent(&err);
  const guatiao_value *s, *v;
  gt_tree_args(schema, &ts, &s, value, &tv, &v, R_NilValue, NULL, NULL);
  guatiao_status st = GT_CORE_FN(guatiao_schema_validate)(s, v, &gt_binding_alloc, &err);
  gt_free(&ts);
  gt_free(&tv);
  if (!st) {
    gt_free(&err);
    return R_NilValue;
  }
  if (err.tag == GUATIAO_ABSENT) gt_raise(st, "the value could not be checked");
  return gt_read_and_free(&err, 0);
}
