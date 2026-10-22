/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/* The text forms (dates, times, instants, spans, exact lengths), serde,
 * intake, redaction and schema checks. */

#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "gt.h"

static void set_absent(guatiao_value *v) { memset(v, 0, sizeof *v); }

/* ---- the text forms --------------------------------------------------------------- */

enum { F_DATE, F_TIME, F_DATETIME, F_TIMESTAMP, F_SPAN };
static const char *const form_names[] = {"date", "time", "datetime", "timestamp", "span"};

static int arg_form(gt_call *c, int i, int *f) {
  guatiao_str t;
  if (!gt_arg_text(c, i, "the form", &t)) return 0;
  for (int k = 0; k < 5; k++)
    if (t.len == strlen(form_names[k]) && !memcmp(t.ptr, form_names[k], t.len)) {
      *f = k;
      return 1;
    }
  return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "no such form: '%.*s' (date, time, datetime, timestamp, span)", (int)t.len,
                 t.ptr);
}

/* A grammar's refusal: an error, or with `soft` a problem struct in the
 * second answer (form, at, expected) and [] in the first. */
static int unparsed(gt_call *c, int f, guatiao_text_error *e, int soft) {
  guatiao_str t = guatiao_string_text(&e->expected);
  char expected[256];
  snprintf(expected, sizeof expected, "%.*s", (int)(t.len < 255 ? t.len : 255), t.ptr ? (const char *)t.ptr : "");
  gt_free(&e->expected);
  if (soft) {
    const char *names[3] = {"form", "at", "expected"};
    mxArray *p = mxCreateStructMatrix(1, 1, 3, names);
    mxSetField(p, 0, "form", gt_cstr_mx(form_names[f]));
    mxSetField(p, 0, "at", mxCreateDoubleScalar((double)e->at));
    mxSetField(p, 0, "expected", gt_cstr_mx(expected));
    gt_answer(c, 0, mxCreateDoubleMatrix(0, 0, mxREAL));
    gt_answer(c, 1, p);
    return 1;
  }
  return gt_fail_detail(c, GUATIAO_ERR_BAD_VALUE, "unparsed", "not a %s: at byte %llu, expected %s", form_names[f],
                        (unsigned long long)e->at, expected);
}

#define TWO53 9007199254740992.0

/* Fields as doubles (a unit past 2^53 refused), or as a cell of exact text. */
static int fields_out(gt_call *c, const int64_t *v, int n, int exact, mxArray **out) {
  if (exact) {
    mxArray *x = mxCreateCellMatrix(1, (mwSize)n);
    char buf[32];
    for (int i = 0; i < n; i++) {
      snprintf(buf, sizeof buf, "%lld", (long long)v[i]);
      mxSetCell(x, (mwIndex)i, mxCreateString(buf));
    }
    *out = x;
    return 1;
  }
  for (int i = 0; i < n; i++)
    if (fabs((double)v[i]) > TWO53) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "a unit past 2^53 is not held exactly by a double");
  mxArray *x = mxCreateDoubleMatrix(1, (mwSize)n, mxREAL);
  double *d = (double *)mxGetData(x);
  for (int i = 0; i < n; i++) d[i] = (double)v[i];
  *out = x;
  return 1;
}

/* Fields in: n doubles, or a cell of n exact texts (units past 2^53). */
static int fields_in(gt_call *c, int i, int n, const char *what, int64_t *v) {
  const mxArray *a = gt_arg(c, i);
  if (a && mxIsCell(a) && mxGetNumberOfElements(a) == (size_t)n) {
    for (int k = 0; k < n; k++) {
      const mxArray *e = mxGetCell(a, (mwIndex)k);
      char buf[40];
      if (!e || !mxIsChar(e) || mxGetString(e, buf, sizeof buf))
        return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "%s is %d whole numbers", what, n);
      char *end = NULL;
      long long x = strtoll(buf, &end, 10);
      if (end == buf || *end) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "%s is %d whole numbers", what, n);
      v[k] = (int64_t)x;
    }
    return 1;
  }
  if (!a || !mxIsDouble(a) || mxIsComplex(a) || mxGetNumberOfElements(a) != (size_t)n)
    return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "%s is %d numbers", what, n);
  const double *d = (const double *)mxGetData(a);
  for (int k = 0; k < n; k++) {
    if (!isfinite(d[k]) || d[k] != floor(d[k]) || fabs(d[k]) >= 9223372036854775808.0)
      return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "%s is %d whole numbers", what, n);
    v[k] = (int64_t)d[k];
  }
  return 1;
}

static int in_range(gt_call *c, int64_t v, int64_t lo, int64_t hi, const char *what) {
  if (v < lo || v > hi) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "%s is out of range", what);
  return 1;
}

static int date_of(gt_call *c, const int64_t *v, guatiao_date *d) {
  if (!in_range(c, v[0], INT32_MIN, INT32_MAX, "the year") || !in_range(c, v[1], 0, 255, "the month") ||
      !in_range(c, v[2], 0, 255, "the day"))
    return 0;
  d->year = (int32_t)v[0];
  d->month = (uint8_t)v[1];
  d->day = (uint8_t)v[2];
  return 1;
}

static int time_of(gt_call *c, const int64_t *v, guatiao_time *t) {
  if (!in_range(c, v[0], 0, 255, "the hour") || !in_range(c, v[1], 0, 255, "the minute") ||
      !in_range(c, v[2], 0, 255, "the second") || !in_range(c, v[3], 0, UINT32_MAX, "the nanosecond"))
    return 0;
  t->hour = (uint8_t)v[0];
  t->minute = (uint8_t)v[1];
  t->second = (uint8_t)v[2];
  t->nanosecond = (uint32_t)v[3];
  return 1;
}

static int span_of(gt_call *c, const int64_t *v, guatiao_span *s) {
  if (!in_range(c, v[7], -2147483647LL, 2147483647LL, "nanoseconds")) return 0;
  s->years = v[0];
  s->months = v[1];
  s->weeks = v[2];
  s->days = v[3];
  s->hours = v[4];
  s->minutes = v[5];
  s->seconds = v[6];
  s->nanoseconds = (int32_t)v[7];
  return 1;
}

/* time_parse(form, text, exact, soft) -> fields */
int cmd_time_parse(gt_call *c) {
  int f = 0, exact = 0, soft = 0;
  guatiao_str t;
  if (!arg_form(c, 0, &f) || !gt_arg_text(c, 1, "the text", &t)) return 0;
  if (gt_arg(c, 2) && !gt_arg_bool(c, 2, "exact", &exact)) return 0;
  if (gt_arg(c, 3) && !gt_arg_bool(c, 3, "soft", &soft)) return 0;
  if (!GT_NEED_CORE(c, guatiao_date_parse)) return 0;
  guatiao_text_error e;
  memset(&e, 0, sizeof e);
  guatiao_status st;
  int64_t v[8];
  int n = 0;
  switch (f) {
  case F_DATE: {
    guatiao_date d;
    memset(&d, 0, sizeof d);
    st = gt_core.guatiao_date_parse(&gt_binding_alloc, t, &d, &e);
    v[0] = d.year, v[1] = d.month, v[2] = d.day, n = 3;
    break;
  }
  case F_TIME: {
    guatiao_time x;
    memset(&x, 0, sizeof x);
    st = gt_core.guatiao_time_parse(&gt_binding_alloc, t, &x, &e);
    v[0] = x.hour, v[1] = x.minute, v[2] = x.second, v[3] = x.nanosecond, n = 4;
    break;
  }
  case F_DATETIME: {
    guatiao_datetime x;
    memset(&x, 0, sizeof x);
    st = gt_core.guatiao_datetime_parse(&gt_binding_alloc, t, &x, &e);
    v[0] = x.date.year, v[1] = x.date.month, v[2] = x.date.day;
    v[3] = x.time.hour, v[4] = x.time.minute, v[5] = x.time.second, v[6] = x.time.nanosecond, n = 7;
    break;
  }
  case F_TIMESTAMP: {
    guatiao_timestamp x;
    memset(&x, 0, sizeof x);
    st = gt_core.guatiao_timestamp_parse(&gt_binding_alloc, t, &x, &e);
    v[0] = x.seconds, v[1] = x.nanoseconds, n = 2;
    break;
  }
  default: {
    guatiao_span x;
    memset(&x, 0, sizeof x);
    st = gt_core.guatiao_span_parse(&gt_binding_alloc, t, &x, &e);
    v[0] = x.years, v[1] = x.months, v[2] = x.weeks, v[3] = x.days;
    v[4] = x.hours, v[5] = x.minutes, v[6] = x.seconds, v[7] = x.nanoseconds, n = 8;
    break;
  }
  }
  if (st == GUATIAO_ERR_BAD_VALUE) return unparsed(c, f, &e, soft);
  gt_free(&e.expected);
  if (st) return gt_fail(c, st, "the %s was refused", form_names[f]);
  mxArray *out = NULL;
  if (!fields_out(c, v, n, exact, &out)) return 0;
  gt_answer(c, 0, out);
  return 1;
}

static int take_text(gt_call *c, guatiao_value *v) {
  mxArray *out = NULL;
  int ok = gt_text_out(c, guatiao_string_text(v), &out);
  gt_free(v);
  if (ok) gt_answer(c, 0, out);
  return ok;
}

/* time_format(form, fields) -> text */
int cmd_time_format(gt_call *c) {
  int f = 0;
  if (!arg_form(c, 0, &f) || !GT_NEED_CORE(c, guatiao_date_format)) return 0;
  static const int counts[] = {3, 4, 7, 2, 8};
  int64_t v[8];
  if (!fields_in(c, 1, counts[f], form_names[f], v)) return 0;
  guatiao_value out;
  set_absent(&out);
  guatiao_status st;
  switch (f) {
  case F_DATE: {
    guatiao_date d;
    if (!date_of(c, v, &d)) return 0;
    st = gt_core.guatiao_date_format(&gt_binding_alloc, &d, &out);
    break;
  }
  case F_TIME: {
    guatiao_time x;
    if (!time_of(c, v, &x)) return 0;
    st = gt_core.guatiao_time_format(&gt_binding_alloc, &x, &out);
    break;
  }
  case F_DATETIME: {
    guatiao_datetime x;
    if (!date_of(c, v, &x.date) || !time_of(c, v + 3, &x.time)) return 0;
    st = gt_core.guatiao_datetime_format(&gt_binding_alloc, &x, &out);
    break;
  }
  case F_TIMESTAMP: {
    guatiao_timestamp x;
    if (!in_range(c, v[1], 0, UINT32_MAX, "nanoseconds")) return 0;
    x.seconds = v[0];
    x.nanoseconds = (uint32_t)v[1];
    st = gt_core.guatiao_timestamp_format(&gt_binding_alloc, &x, &out);
    break;
  }
  default: {
    guatiao_span s;
    if (!span_of(c, v, &s)) return 0;
    st = gt_core.guatiao_span_format(&gt_binding_alloc, &s, &out);
    break;
  }
  }
  if (st) {
    gt_free(&out);
    return gt_fail(c, st, "not a %s", form_names[f]);
  }
  return take_text(c, &out);
}

/* time_normalize(form, text, soft) -> canonical text */
int cmd_time_normalize(gt_call *c) {
  int f = 0, soft = 0;
  guatiao_str t;
  if (!arg_form(c, 0, &f) || !gt_arg_text(c, 1, "the text", &t)) return 0;
  if (gt_arg(c, 2) && !gt_arg_bool(c, 2, "soft", &soft)) return 0;
  if (!GT_NEED_CORE(c, guatiao_date_normalize)) return 0;
  guatiao_text_error e;
  memset(&e, 0, sizeof e);
  guatiao_value out;
  set_absent(&out);
  guatiao_status st;
  switch (f) {
  case F_DATE: st = gt_core.guatiao_date_normalize(&gt_binding_alloc, t, &out, &e); break;
  case F_TIME: st = gt_core.guatiao_time_normalize(&gt_binding_alloc, t, &out, &e); break;
  case F_DATETIME: st = gt_core.guatiao_datetime_normalize(&gt_binding_alloc, t, &out, &e); break;
  case F_TIMESTAMP: st = gt_core.guatiao_timestamp_normalize(&gt_binding_alloc, t, &out, &e); break;
  default: st = gt_core.guatiao_span_normalize(&gt_binding_alloc, t, &out, &e); break;
  }
  if (st == GUATIAO_ERR_BAD_VALUE) {
    gt_free(&out);
    return unparsed(c, f, &e, soft);
  }
  gt_free(&e.expected);
  if (st) {
    gt_free(&out);
    return gt_fail(c, st, "the %s was refused", form_names[f]);
  }
  return take_text(c, &out);
}

/* date_add_span(date fields, span fields) -> date fields */
int cmd_date_add_span(gt_call *c) {
  int64_t dv[3], sv[8];
  guatiao_date d, out;
  guatiao_span s;
  if (!fields_in(c, 0, 3, "a date", dv) || !fields_in(c, 1, 8, "a span", sv) || !date_of(c, dv, &d) ||
      !span_of(c, sv, &s) || !GT_NEED_CORE(c, guatiao_date_add_span))
    return 0;
  guatiao_status st = gt_core.guatiao_date_add_span(&d, &s, &out);
  if (st) return gt_fail(c, st, "the span does not apply to the date");
  int64_t r[3] = {out.year, out.month, out.day};
  mxArray *x = NULL;
  if (!fields_out(c, r, 3, 0, &x)) return 0;
  gt_answer(c, 0, x);
  return 1;
}

int cmd_datetime_add_span(gt_call *c) {
  int64_t dv[7], sv[8];
  guatiao_datetime d, out;
  guatiao_span s;
  if (!fields_in(c, 0, 7, "a date and time", dv) || !fields_in(c, 1, 8, "a span", sv) || !date_of(c, dv, &d.date) ||
      !time_of(c, dv + 3, &d.time) || !span_of(c, sv, &s) || !GT_NEED_CORE(c, guatiao_datetime_add_span))
    return 0;
  guatiao_status st = gt_core.guatiao_datetime_add_span(&d, &s, &out);
  if (st) return gt_fail(c, st, "the span does not apply to the date and time");
  int64_t r[7] = {out.date.year, out.date.month, out.date.day, out.time.hour, out.time.minute, out.time.second,
                  out.time.nanosecond};
  mxArray *x = NULL;
  if (!fields_out(c, r, 7, 0, &x)) return 0;
  gt_answer(c, 0, x);
  return 1;
}

/* duration_read(tree) -> {seconds as exact text, nanoseconds} */
int cmd_duration_read(gt_call *c) {
  if (!GT_NEED_CORE(c, guatiao_duration_read)) return 0;
  guatiao_value tmp;
  const guatiao_value *v = NULL;
  int at[1] = {0};
  if (!gt_tree_args(c, 1, at, &tmp, &v)) return 0;
  uint64_t s = 0;
  uint32_t ns = 0;
  guatiao_status st = gt_core.guatiao_duration_read(v, &s, &ns);
  gt_free(&tmp);
  if (st) return gt_fail(c, st, "not an exact length: [seconds, nanoseconds], seconds, or {secs, nanos}");
  char buf[32];
  snprintf(buf, sizeof buf, "%llu", (unsigned long long)s);
  mxArray *out = mxCreateCellMatrix(1, 2);
  mxSetCell(out, 0, mxCreateString(buf));
  mxSetCell(out, 1, mxCreateDoubleScalar((double)ns));
  gt_answer(c, 0, out);
  return 1;
}

/* ---- serde ------------------------------------------------------------------------------ */

static const char *const format_names[] = {"json", "toml", "yaml"};

static int arg_format(gt_call *c, int i, int *f) {
  guatiao_str t;
  if (!gt_arg_text(c, i, "the format", &t)) return 0;
  for (int k = 0; k < 3; k++)
    if (t.len == strlen(format_names[k]) && !memcmp(t.ptr, format_names[k], t.len)) {
      *f = k;
      return 1;
    }
  return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "no such format: '%.*s' (json, toml, yaml)", (int)t.len, t.ptr);
}

typedef guatiao_status (*parse_fn)(guatiao_str, uint32_t, const guatiao_alloc *, guatiao_value *,
                                   guatiao_parse_error *);
typedef guatiao_status (*emit_fn)(const guatiao_value *, uint32_t, const guatiao_alloc *, guatiao_value *);

/* serde_parse(format, text, how, alloc, soft) -> id (and a problem with soft) */
int cmd_serde_parse(gt_call *c) {
  int f = 0, soft = 0;
  double how = 0;
  guatiao_str t;
  const guatiao_alloc *a = NULL;
  if (!arg_format(c, 0, &f) || !gt_arg_text(c, 1, "the document", &t) || !gt_arg_double(c, 2, "how", &how) ||
      !gt_arg_alloc(c, 3, &a))
    return 0;
  if (gt_arg(c, 4) && !gt_arg_bool(c, 4, "soft", &soft)) return 0;
  int have = f == 0 ? GT_NEED_SERDE(c, guatiao_json_parse_ex)
             : f == 1 ? GT_NEED_SERDE(c, guatiao_toml_parse_ex)
                      : GT_NEED_SERDE(c, guatiao_yaml_parse_ex);
  if (!have || !gt_need(c, GT_LIB_CORE)) return 0;
  parse_fn fn = f == 0 ? gt_serde.guatiao_json_parse_ex : f == 1 ? gt_serde.guatiao_toml_parse_ex : gt_serde.guatiao_yaml_parse_ex;
  guatiao_value out;
  set_absent(&out);
  guatiao_parse_error e;
  memset(&e, 0, sizeof e);
  guatiao_status st = fn(t, (uint32_t)how, a, &out, &e);
  if (st == GUATIAO_ERR_BAD_VALUE) {
    guatiao_str m = guatiao_string_text(&e.message);
    char reason[512];
    snprintf(reason, sizeof reason, "%.*s", (int)(m.len < 511 ? m.len : 511), m.ptr ? (const char *)m.ptr : "");
    gt_free(&e.message);
    gt_free(&out);
    const char *fmt = e.format <= 2 ? format_names[e.format] : format_names[f];
    if (soft) {
      const char *names[4] = {"format", "line", "column", "reason"};
      mxArray *p = mxCreateStructMatrix(1, 1, 4, names);
      mxSetField(p, 0, "format", gt_cstr_mx(fmt));
      mxSetField(p, 0, "line", mxCreateDoubleScalar((double)e.line));
      mxSetField(p, 0, "column", mxCreateDoubleScalar((double)e.column));
      mxSetField(p, 0, "reason", gt_cstr_mx(reason));
      gt_answer(c, 0, mxCreateDoubleMatrix(0, 0, mxREAL));
      gt_answer(c, 1, p);
      return 1;
    }
    char upper[8];
    snprintf(upper, sizeof upper, "%s", fmt);
    for (char *p = upper; *p; p++) *p = (char)(*p - 32);
    if (e.line > 0)
      return gt_fail_detail(c, GUATIAO_ERR_BAD_VALUE, "parse", "%s does not parse at line %u, column %u: %s", upper,
                            (unsigned)e.line, (unsigned)e.column, reason);
    return gt_fail_detail(c, GUATIAO_ERR_BAD_VALUE, "parse", "%s does not parse: %s", upper, reason);
  }
  gt_free(&e.message);
  if (st) {
    gt_free(&out);
    return gt_fail(c, st, "the document was refused");
  }
  uint64_t id = gt_owner_new(&out, a);
  if (!id) {
    gt_free(&out);
    return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
  }
  gt_answer(c, 0, mxCreateDoubleScalar((double)id));
  return 1;
}

/* serde_emit(format, tree, how, pretty) -> text */
int cmd_serde_emit(gt_call *c) {
  int f = 0, pretty = 0;
  double how = 0;
  if (!arg_format(c, 0, &f) || !gt_arg_double(c, 2, "how", &how) || !gt_arg_bool(c, 3, "pretty", &pretty)) return 0;
  int have = f == 0 ? (pretty ? GT_NEED_SERDE(c, guatiao_json_emit_pretty) : GT_NEED_SERDE(c, guatiao_json_emit))
             : f == 1 ? GT_NEED_SERDE(c, guatiao_toml_emit)
                      : GT_NEED_SERDE(c, guatiao_yaml_emit);
  if (!have) return 0;
  emit_fn fn = f == 0 ? (pretty ? gt_serde.guatiao_json_emit_pretty : gt_serde.guatiao_json_emit)
               : f == 1 ? gt_serde.guatiao_toml_emit
                        : gt_serde.guatiao_yaml_emit;
  guatiao_value tmp;
  const guatiao_value *v = NULL;
  int at[1] = {1};
  if (!gt_tree_args(c, 1, at, &tmp, &v)) return 0;
  guatiao_value out;
  set_absent(&out);
  guatiao_status st = fn(v, (uint32_t)how, &gt_binding_alloc, &out);
  gt_free(&tmp);
  if (st) {
    gt_free(&out);
    return gt_fail(c, st, "%s", f == 1 ? "TOML writes only a map, and no null" : "the value cannot be written");
  }
  return take_text(c, &out);
}

/* ---- intake ---------------------------------------------------------------------------- */

static int answer_plain(gt_call *c, guatiao_status st, guatiao_value *out, const char *what) {
  if (st) {
    gt_free(out);
    return gt_fail(c, st, "%s", what);
  }
  mxArray *x = NULL;
  int ok = gt_read(c, out, NULL, &x);
  gt_free(out);
  if (ok) gt_answer(c, 0, x);
  return ok;
}

static int answer_owner(gt_call *c, guatiao_value *out, const guatiao_alloc *a) {
  if (out->tag == GUATIAO_NULL || out->tag == GUATIAO_ABSENT) {
    gt_free(out);
    gt_answer(c, 0, mxCreateDoubleMatrix(0, 0, mxREAL));
    return 1;
  }
  uint64_t id = gt_owner_new(out, a);
  if (!id) {
    gt_free(out);
    return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
  }
  gt_answer(c, 0, mxCreateDoubleScalar((double)id));
  return 1;
}

static void free_all(guatiao_value *t, int n) {
  for (int i = 0; i < n; i++) gt_free(&t[i]);
}

/* intake_check(schema, form) -> [] or the error as data */
int cmd_intake_check(gt_call *c) {
  if (!GT_NEED_INTAKE(c, guatiao_intake_check)) return 0;
  guatiao_value t[2], out;
  const guatiao_value *p[2];
  int at[2] = {0, 1};
  set_absent(&out);
  if (!gt_tree_args(c, 2, at, t, p)) return 0;
  guatiao_status st = gt_intake.guatiao_intake_check(p[0], p[1], &gt_binding_alloc, &out);
  free_all(t, 2);
  if (!st) {
    gt_free(&out);
    gt_answer(c, 0, mxCreateDoubleMatrix(0, 0, mxREAL));
    return 1;
  }
  if (st == GUATIAO_ERR_BAD_VALUE && out.tag == GUATIAO_MAP) return answer_plain(c, GUATIAO_OK, &out, "");
  return answer_plain(c, st, &out, "the form could not be checked");
}

int cmd_intake_layout(gt_call *c) {
  if (!GT_NEED_INTAKE(c, guatiao_intake_layout)) return 0;
  guatiao_value t[2], out;
  const guatiao_value *p[2];
  int at[2] = {0, 1};
  set_absent(&out);
  if (!gt_tree_args(c, 2, at, t, p)) return 0;
  guatiao_status st = gt_intake.guatiao_intake_layout(p[0], p[1], &gt_binding_alloc, &out);
  free_all(t, 2);
  return answer_plain(c, st, &out, "the form was refused");
}

/* intake_is_visible(schema, form, key, values) -> logical */
int cmd_intake_is_visible(gt_call *c) {
  guatiao_str k;
  if (!gt_arg_text(c, 2, "the key", &k) || !GT_NEED_INTAKE(c, guatiao_intake_is_visible)) return 0;
  guatiao_value t[3];
  const guatiao_value *p[3];
  int at[3] = {0, 1, 3};
  if (!gt_tree_args(c, 3, at, t, p)) return 0;
  bool out = false;
  guatiao_status st = gt_intake.guatiao_intake_is_visible(p[0], p[1], k, p[2], &out);
  free_all(t, 3);
  if (st) return gt_fail(c, st, "visibility could not be judged");
  gt_answer(c, 0, mxCreateLogicalScalar(out));
  return 1;
}

int cmd_intake_for_schema(gt_call *c) {
  if (!GT_NEED_INTAKE(c, guatiao_intake_for_schema)) return 0;
  guatiao_value t[1], out;
  const guatiao_value *p[1];
  int at[1] = {0};
  set_absent(&out);
  if (!gt_tree_args(c, 1, at, t, p)) return 0;
  guatiao_status st = gt_intake.guatiao_intake_for_schema(p[0], &gt_binding_alloc, &out);
  free_all(t, 1);
  if (st) {
    gt_free(&out);
    return gt_fail(c, st, "no form for that schema");
  }
  return answer_owner(c, &out, &gt_binding_alloc);
}

/* intake_form_for(form, schema, path) -> id or [] */
int cmd_intake_form_for(gt_call *c) {
  guatiao_str path;
  if (!gt_arg_text(c, 2, "the path", &path) || !GT_NEED_INTAKE(c, guatiao_intake_form_for)) return 0;
  guatiao_value t[2], out;
  const guatiao_value *p[2];
  int at[2] = {0, 1};
  set_absent(&out);
  if (!gt_tree_args(c, 2, at, t, p)) return 0;
  guatiao_status st = gt_intake.guatiao_intake_form_for(p[0], p[1], path, &gt_binding_alloc, &out);
  free_all(t, 2);
  if (st) {
    gt_free(&out);
    return gt_fail(c, st, "no form at that path");
  }
  return answer_owner(c, &out, &gt_binding_alloc);
}

/* Appends to `steps` (a cell, from `depth`) the path from `root` to
 * `target`; answers 1 when it is there. Positions are from 0. */
static int path_to(gt_call *c, const guatiao_value *root, const guatiao_value *target, mxArray *steps, int depth) {
  if (root == target) return 1;
  if (depth >= GT_DEPTH) return 0;
  if (root->tag == GUATIAO_MAP) {
    guatiao_entries es = guatiao_map_entries(root);
    for (size_t i = 0; i < es.len; i++)
      if (path_to(c, &es.ptr[i].value, target, steps, depth + 1)) {
        mxArray *k = NULL;
        if (!gt_text_out(c, guatiao_entry_key(&es.ptr[i]), &k)) return 0;
        mxSetCell(steps, (mwIndex)depth, k);
        return 1;
      }
  } else if (root->tag == GUATIAO_LIST) {
    guatiao_values items = guatiao_list_items(root);
    for (size_t i = 0; i < items.len; i++)
      if (path_to(c, &items.ptr[i], target, steps, depth + 1)) {
        mxSetCell(steps, (mwIndex)depth, mxCreateDoubleScalar((double)i));
        return 1;
      }
  }
  return 0;
}

typedef const guatiao_value *(*lookup_fn)(const guatiao_value *, guatiao_str);

/* What a lookup into an argument answers: {'view', owner id, path} for a
 * node of the argument's own tree, {'value', id, {}} for a copy when the
 * argument was MATLAB data, or [] when nothing is there. */
static int lookup(gt_call *c, lookup_fn fn, const char *what) {
  guatiao_str key;
  if (!gt_arg_text(c, 1, what, &key)) return 0;
  mxArray *ref = NULL;
  if (!gt_node_ref(c, gt_arg(c, 0), &ref)) return 0;
  guatiao_value tmp;
  set_absent(&tmp);
  const guatiao_value *root = NULL;
  if (ref) {
    guatiao_value *r = NULL;
    if (!gt_walk_mx(c, mxGetCell(ref, 1), mxGetCell(ref, 2), &r)) {
      mxDestroyArray(ref);
      return 0;
    }
    root = r;
  } else {
    if (!gt_build(c, gt_arg(c, 0), &gt_binding_alloc, &tmp)) return 0;
    root = &tmp;
  }
  const guatiao_value *found = fn(root, key);
  if (!found) {
    gt_free(&tmp);
    if (ref) mxDestroyArray(ref);
    gt_answer(c, 0, mxCreateDoubleMatrix(0, 0, mxREAL));
    return 1;
  }
  mxArray *out = mxCreateCellMatrix(1, 3);
  if (ref) {
    mxArray *steps = mxCreateCellMatrix(1, GT_DEPTH + 1);
    if (!path_to(c, root, found, steps, 0)) {
      mxDestroyArray(steps);
      mxDestroyArray(out);
      mxDestroyArray(ref);
      if (c->status) return 0;
      gt_answer(c, 0, mxCreateDoubleMatrix(0, 0, mxREAL));
      return 1;
    }
    const mxArray *base = mxGetCell(ref, 2);
    size_t nb = base && mxIsCell(base) ? mxGetNumberOfElements(base) : 0, extra = 0;
    while (extra < GT_DEPTH + 1 && mxGetCell(steps, (mwIndex)extra)) extra++;
    mxArray *path = mxCreateCellMatrix(1, (mwSize)(nb + extra));
    for (size_t i = 0; i < nb; i++) mxSetCell(path, (mwIndex)i, mxDuplicateArray(mxGetCell(base, (mwIndex)i)));
    for (size_t i = 0; i < extra; i++) {
      mxSetCell(path, (mwIndex)(nb + i), mxGetCell(steps, (mwIndex)i));
      mxSetCell(steps, (mwIndex)i, NULL);
    }
    mxDestroyArray(steps);
    mxSetCell(out, 0, mxCreateString("view"));
    mxSetCell(out, 1, mxDuplicateArray(mxGetCell(ref, 1)));
    mxSetCell(out, 2, path);
    mxDestroyArray(ref);
  } else {
    guatiao_value copy;
    set_absent(&copy);
    guatiao_status st = gt_core.guatiao_value_clone(&gt_binding_alloc, found, &copy);
    gt_free(&tmp);
    if (st) {
      mxDestroyArray(out);
      return gt_fail(c, st, "the copy was refused");
    }
    uint64_t id = gt_owner_new(&copy, &gt_binding_alloc);
    if (!id) {
      gt_free(&copy);
      mxDestroyArray(out);
      return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
    }
    mxSetCell(out, 0, mxCreateString("value"));
    mxSetCell(out, 1, mxCreateDoubleScalar((double)id));
    mxSetCell(out, 2, mxCreateCellMatrix(1, 0));
  }
  gt_answer(c, 0, out);
  return 1;
}

int cmd_intake_at(gt_call *c) {
  if (!GT_NEED_INTAKE(c, guatiao_intake_path_get)) return 0;
  return lookup(c, gt_intake.guatiao_intake_path_get, "the path");
}

int cmd_intake_resolve(gt_call *c) {
  if (!GT_NEED_INTAKE(c, guatiao_intake_resolve)) return 0;
  return lookup(c, gt_intake.guatiao_intake_resolve, "the key");
}

int cmd_intake_flat_keys(gt_call *c) {
  guatiao_str k;
  if (!gt_arg_text(c, 1, "the key", &k) || !GT_NEED_INTAKE(c, guatiao_intake_flat_keys)) return 0;
  guatiao_value t[1], out;
  const guatiao_value *p[1];
  int at[1] = {0};
  set_absent(&out);
  if (!gt_tree_args(c, 1, at, t, p)) return 0;
  guatiao_status st = gt_intake.guatiao_intake_flat_keys(p[0], k, &gt_binding_alloc, &out);
  free_all(t, 1);
  return answer_plain(c, st, &out, "no such key in the schema");
}

int cmd_intake_flatten(gt_call *c) {
  guatiao_str k;
  if (!gt_arg_text(c, 1, "the key", &k) || !GT_NEED_INTAKE(c, guatiao_intake_flatten)) return 0;
  guatiao_value t[2], out;
  const guatiao_value *p[2];
  int at[2] = {0, 2};
  set_absent(&out);
  if (!gt_tree_args(c, 2, at, t, p)) return 0;
  guatiao_status st = gt_intake.guatiao_intake_flatten(p[0], k, p[1], &gt_binding_alloc, &out);
  free_all(t, 2);
  gt_read_opts maps = {0, 1}; /* flat keys are rarely field names */
  if (st) {
    gt_free(&out);
    return gt_fail(c, st, "the value does not fit the schema");
  }
  mxArray *x = NULL;
  int ok = gt_read(c, &out, &maps, &x);
  gt_free(&out);
  if (ok) gt_answer(c, 0, x);
  return ok;
}

int cmd_intake_unflatten(gt_call *c) {
  guatiao_str k;
  if (!gt_arg_text(c, 1, "the key", &k) || !GT_NEED_INTAKE(c, guatiao_intake_unflatten)) return 0;
  guatiao_value t[2], out;
  const guatiao_value *p[2];
  int at[2] = {0, 2};
  set_absent(&out);
  if (!gt_tree_args(c, 2, at, t, p)) return 0;
  guatiao_status st = gt_intake.guatiao_intake_unflatten(p[0], k, p[1], &gt_binding_alloc, &out);
  free_all(t, 2);
  if (st == GUATIAO_ERR_WRONG_KIND) { /* a key that does not apply */
    gt_free(&out);
    gt_answer(c, 0, mxCreateDoubleMatrix(0, 0, mxREAL));
    return 1;
  }
  if (st) {
    gt_free(&out);
    return gt_fail(c, st, "the store does not fit the schema");
  }
  return answer_owner(c, &out, &gt_binding_alloc);
}

/* ---- schemas --------------------------------------------------------------------------- */

/* redact(schema, value, placeholder, alloc) -> id */
int cmd_redact(gt_call *c) {
  guatiao_str ph;
  const guatiao_alloc *a = NULL;
  if (!gt_arg_text(c, 2, "the placeholder", &ph) || !gt_arg_alloc(c, 3, &a) || !GT_NEED_CORE(c, guatiao_schema_redact))
    return 0;
  guatiao_value t[2], out;
  const guatiao_value *p[2];
  int at[2] = {0, 1};
  set_absent(&out);
  if (!gt_tree_args(c, 2, at, t, p)) return 0;
  guatiao_status st = gt_core.guatiao_schema_redact(p[0], p[1], ph, a, &out);
  free_all(t, 2);
  if (st) {
    gt_free(&out);
    return gt_fail(c, st, "%s", st == GUATIAO_ERR_WRONG_KIND ? "the schema is not a map" : "the value could not be redacted");
  }
  uint64_t id = gt_owner_new(&out, a);
  if (!id) {
    gt_free(&out);
    return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
  }
  gt_answer(c, 0, mxCreateDoubleScalar((double)id));
  return 1;
}

/* schema_validate(schema, value) -> [] or the refusal as data */
int cmd_schema_validate(gt_call *c) {
  if (!GT_NEED_CORE(c, guatiao_schema_validate)) return 0;
  guatiao_value t[2], err;
  const guatiao_value *p[2];
  int at[2] = {0, 1};
  set_absent(&err);
  if (!gt_tree_args(c, 2, at, t, p)) return 0;
  guatiao_status st = gt_core.guatiao_schema_validate(p[0], p[1], &gt_binding_alloc, &err);
  free_all(t, 2);
  if (!st) {
    gt_free(&err);
    gt_answer(c, 0, mxCreateDoubleMatrix(0, 0, mxREAL));
    return 1;
  }
  if (err.tag == GUATIAO_ABSENT) return gt_fail(c, st, "the value could not be checked");
  return answer_plain(c, GUATIAO_OK, &err, "");
}
