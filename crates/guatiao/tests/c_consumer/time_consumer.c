/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * Every date, time, instant and span vector, from C: each text parsed
 * into its struct, formatted back, normalised, and each span applied, as
 * a C caller does it, against the artifact that exports the functions.
 *
 * `time_vectors.h` is generated from tests/vectors/time.json by the test
 * that compiles this file, so C reads the same vectors as Rust, Python
 * and Dart.
 *
 * Prints "all checks passed" on success. Any failure returns non-zero
 * with a line saying which vector and what it saw.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "guatiao.h"
#include "time_vectors.h"

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

/* Whether `v` is the string `expected`, byte for byte. */
static int is_text(const guatiao_value *v, const char *expected) {
  if (guatiao_tag_of(v) != GUATIAO_STRING) {
    return 0;
  }
  guatiao_str s = guatiao_string_text(v);
  return s.len == strlen(expected) && (s.len == 0 || memcmp(s.ptr, expected, s.len) == 0);
}

static int date_fields(const guatiao_date *d, long long *f) {
  f[0] = d->year;
  f[1] = d->month;
  f[2] = d->day;
  return 3;
}

static int time_fields(const guatiao_time *t, long long *f) {
  f[0] = t->hour;
  f[1] = t->minute;
  f[2] = t->second;
  f[3] = t->nanosecond;
  return 4;
}

static int datetime_fields(const guatiao_datetime *at, long long *f) {
  date_fields(&at->date, f);
  time_fields(&at->time, f + 3);
  return 7;
}

static int timestamp_fields(const guatiao_timestamp *t, long long *f) {
  f[0] = t->seconds;
  f[1] = t->nanoseconds;
  return 2;
}

static int span_fields(const guatiao_span *s, long long *f) {
  f[0] = s->years;
  f[1] = s->months;
  f[2] = s->weeks;
  f[3] = s->days;
  f[4] = s->hours;
  f[5] = s->minutes;
  f[6] = s->seconds;
  f[7] = s->nanoseconds;
  return 8;
}

/* One form's vectors through its three functions. A macro, because each
   form has its own struct type and C has no generics. */
#define CHECK_FORM(NAME, TYPE, VECTORS, COUNT, PARSE, FORMAT, NORMALIZE, FIELDS)          \
  for (size_t i = 0; i < (COUNT); i++) {                                                \
    const text_vector *v = &(VECTORS)[i];                                               \
    TYPE parsed;                                                                        \
    guatiao_text_error error;                                                           \
    guatiao_status status = PARSE(&alloc, guatiao_cstr(v->text), &parsed, &error);     \
    guatiao_value normal;                                                               \
    guatiao_text_error normal_error;                                                    \
    guatiao_status n_status =                                                           \
        NORMALIZE(&alloc, guatiao_cstr(v->text), &normal, &normal_error);              \
    if (v->refused_at < 0) {                                                            \
      long long f[8];                                                                   \
      int n = FIELDS(&parsed, f);                                                       \
      CHECK(status == GUATIAO_OK, "%s \"%s\": status %u", NAME, v->text, status);      \
      CHECK(n == v->field_count && memcmp(f, v->fields, sizeof(long long) * (size_t)n) == 0, \
            "%s \"%s\": fields", NAME, v->text);                                        \
      guatiao_value written;                                                            \
      CHECK(FORMAT(&alloc, &parsed, &written) == GUATIAO_OK, "%s format", NAME);        \
      CHECK(is_text(&written, v->canonical), "%s \"%s\": format", NAME, v->text);       \
      CHECK(n_status == GUATIAO_OK && is_text(&normal, v->canonical),                   \
            "%s \"%s\": normalize", NAME, v->text);                                     \
      guatiao_value_free(&written);                                                     \
    } else {                                                                            \
      CHECK(status == GUATIAO_ERR_BAD_VALUE && error.at == (size_t)v->refused_at,      \
            "%s \"%s\": refused at %zu, status %u", NAME, v->text, error.at, status);   \
      CHECK(n_status == GUATIAO_ERR_BAD_VALUE && normal_error.at == (size_t)v->refused_at, \
            "%s \"%s\": normalize refused at %zu", NAME, v->text, normal_error.at);     \
      CHECK(guatiao_tag_of(&error.expected) == GUATIAO_STRING,                          \
            "%s \"%s\": a refusal says what it expected", NAME, v->text);               \
      CHECK(guatiao_tag_of(&normal) == GUATIAO_ABSENT, "%s: out left ABSENT", NAME);     \
    }                                                                                   \
    guatiao_value_free(&error.expected);                                                \
    guatiao_value_free(&normal_error.expected);                                         \
    guatiao_value_free(&normal);                                                        \
    checked++;                                                                          \
  }

int main(void) {
  guatiao_alloc alloc = GUATIAO_ALLOC_MALLOC;

  CHECK_FORM("date", guatiao_date, DATE_VECTORS, DATE_COUNT, guatiao_date_parse,
             guatiao_date_format, guatiao_date_normalize, date_fields)
  CHECK_FORM("time", guatiao_time, TIME_VECTORS, TIME_COUNT, guatiao_time_parse,
             guatiao_time_format, guatiao_time_normalize, time_fields)
  CHECK_FORM("datetime", guatiao_datetime, DATETIME_VECTORS, DATETIME_COUNT,
             guatiao_datetime_parse, guatiao_datetime_format, guatiao_datetime_normalize,
             datetime_fields)
  CHECK_FORM("timestamp", guatiao_timestamp, TIMESTAMP_VECTORS, TIMESTAMP_COUNT,
             guatiao_timestamp_parse, guatiao_timestamp_format, guatiao_timestamp_normalize,
             timestamp_fields)
  CHECK_FORM("span", guatiao_span, SPAN_VECTORS, SPAN_COUNT, guatiao_span_parse,
             guatiao_span_format, guatiao_span_normalize, span_fields)

  /* ---- applying a span ---------------------------------------------- */

  for (size_t i = 0; i < DATE_ADD_COUNT; i++) {
    const add_vector *v = &DATE_ADD_VECTORS[i];
    guatiao_date date, expected;
    guatiao_span span;
    CHECK(guatiao_date_parse(NULL, guatiao_cstr(v->start), &date, NULL) == GUATIAO_OK, "start");
    CHECK(guatiao_span_parse(NULL, guatiao_cstr(v->span), &span, NULL) == GUATIAO_OK, "span");
    /* In place: the result overwrites the date it was computed from. */
    guatiao_status status = guatiao_date_add_span(&date, &span, &date);
    if (v->result) {
      CHECK(guatiao_date_parse(NULL, guatiao_cstr(v->result), &expected, NULL) == GUATIAO_OK,
            "result");
      CHECK(status == GUATIAO_OK && date.year == expected.year &&
                date.month == expected.month && date.day == expected.day,
            "%s + %s: status %u, %d-%u-%u", v->start, v->span, status, date.year,
            date.month, date.day);
    } else {
      CHECK(status == GUATIAO_ERR_BAD_VALUE, "%s + %s: refused, status %u", v->start,
            v->span, status);
    }
    checked++;
  }

  for (size_t i = 0; i < DATETIME_ADD_COUNT; i++) {
    const add_vector *v = &DATETIME_ADD_VECTORS[i];
    guatiao_datetime at, sum;
    guatiao_span span;
    CHECK(guatiao_datetime_parse(NULL, guatiao_cstr(v->start), &at, NULL) == GUATIAO_OK,
          "start");
    CHECK(guatiao_span_parse(NULL, guatiao_cstr(v->span), &span, NULL) == GUATIAO_OK, "span");
    guatiao_status status = guatiao_datetime_add_span(&at, &span, &sum);
    if (v->result) {
      guatiao_value text;
      CHECK(status == GUATIAO_OK, "%s + %s: status %u", v->start, v->span, status);
      CHECK(guatiao_datetime_format(&alloc, &sum, &text) == GUATIAO_OK, "format the sum");
      CHECK(is_text(&text, v->result), "%s + %s", v->start, v->span);
      guatiao_value_free(&text);
    } else {
      CHECK(status == GUATIAO_ERR_BAD_VALUE, "%s + %s: refused, status %u", v->start,
            v->span, status);
    }
    checked++;
  }

  /* ---- a struct that is no form is refused when written -------------- */

  guatiao_span mixed;
  memset(&mixed, 0, sizeof mixed);
  mixed.days = 1;
  mixed.hours = -2;
  guatiao_value out;
  CHECK(guatiao_span_format(&alloc, &mixed, &out) == GUATIAO_ERR_BAD_VALUE,
        "a span of both signs is refused");
  CHECK(guatiao_tag_of(&out) == GUATIAO_ABSENT, "and its out is ABSENT");

  if (failures != 0) {
    printf("%d check(s) failed\n", failures);
    return 1;
  }
  printf("%zu vectors; all checks passed\n", checked);
  return 0;
}
