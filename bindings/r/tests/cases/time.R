# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# The shared vectors every implementation reads, then the mappings to R's
# own types and what they lose.

form_code <- function(form) gt$.gt_forms[[form]]
fields_text <- function(form, text) .Call(gt$C_time_parse, form_code(form), text, TRUE)

case("every vector of every form", with_library(with_repo({
  vecs <- vectors()
  for (form in names(gt$.gt_forms)) {
    cases <- vecs[[form]]
    expect_true(length(cases) > 0, paste(form, "has vectors"))
    for (c in cases) {
      text <- c$text
      if (!is.null(c$refused_at)) {
        at <- as.integer(text_of(c$refused_at))
        for (reader in list(function() fields_text(form, text), function() time_normalize(form, text))) {
          e <- expect_error(reader(), "guatiao_unparsed", paste(form, text))
          expect_equal(e$at, at, paste(form, text, "at"))
          expect_equal(e$form, form)
          expect_true(nzchar(e$expected))
          if (nchar(text) > 2) expect_true(!grepl(text, conditionMessage(e), fixed = TRUE), "the message never quotes the text")
        }
      } else {
        expect_equal(fields_text(form, text), vapply(c$fields, text_of, ""), paste(form, text))
        expect_equal(time_normalize(form, text), c$canonical, paste(form, text, "canonical"))
      }
    }
  }
})))

case("every span applied", with_library(with_repo({
  vecs <- vectors()
  # Spans as exact text: a vector's units may be past what an R number holds.
  for (form in c("date", "datetime")) {
    add <- if (form == "date") gt$C_date_add_span else gt$C_datetime_add_span
    for (c in vecs[[paste0(form, "_add_span")]]) {
      start <- as.numeric(fields_text(form, c$start))
      s <- fields_text("span", c$span)
      if (is.null(c$result)) {
        expect_error(.Call(add, start, s), "guatiao_error", paste(c$start, c$span))
      } else {
        got <- .Call(gt$C_time_format, form_code(form), .Call(add, start, s))
        expect_equal(got, c$result, paste(c$start, c$span))
      }
    }
  }
  expect_equal(time_format(add_span(time_parse("date", "2026-01-31"), time_parse("span", "P1M"))), "2026-02-28")
})))

case("every exact length", with_library(with_repo({
  vecs <- vectors()
  for (c in vecs$duration) {
    if (is.null(c$duration)) {
      expect_error(.Call(gt$C_duration_read, c$value), "guatiao_error")
    } else {
      got <- .Call(gt$C_duration_read, c$value)
      expect_equal(got[[1L]], text_of(c$duration[[1L]]))
      expect_equal(got[[2L]], as.numeric(text_of(c$duration[[2L]])))
      if (as.numeric(got[[1L]]) <= 2^53) {
        expect_equal(read_length(c$value), exact_length(as.numeric(got[[1L]]), got[[2L]]))
      } else {
        expect_error(read_length(c$value))
      }
    }
  }
})))

case("the mappings to R's types", with_library({
  expect_equal(time_parse("date", "0000-02-29"), as.Date("0000-02-29"))
  expect_equal(time_format(as.Date("2026-10-21")), "2026-10-21")
  t <- time_parse("time", "13:45:00.000000001")
  expect_equal(unclass(t)[[4L]], 1)
  expect_equal(time_format(t), "13:45:00.000000001")
  dt <- time_parse("datetime", "2026-10-21T13:45:00.0019")
  expect_equal(time_format(dt), "2026-10-21T13:45:00.0019")
  expect_equal(as.Date(dt), as.Date("2026-10-21"))
  ts <- time_parse("timestamp", "2026-10-21T19:15:00.5+05:30")
  expect_equal(unclass(ts), c(1792590300, 5e8))
  expect_equal(format(ts), "2026-10-21T13:45:00.5Z")
  expect_equal(as.numeric(as.POSIXct(ts)), 1792590300.5)
  expect_equal(time_format(as.POSIXct("1969-12-31 23:59:59.999", tz = "UTC")), "1969-12-31T23:59:59.999Z")
  expect_equal(time_format(instant(-1, 999000000)), "1969-12-31T23:59:59.999Z")
  expect_error(time_parse("date", "2026-10-21Z"), "guatiao_unparsed")
  expect_equal(time_normalize("timestamp", "2026-10-21T13:45:00.000000001+00:00"), "2026-10-21T13:45:00.000000001Z")
  expect_error(civil_time(24, 0), "guatiao_error")
  expect_error(time_parse("week", "x"))
}))

case("a span is its own type, backed by the library", with_library({
  s <- span(days = 1, hours = 2)
  expect_equal(format(s), "P1DT2H")
  expect_equal(format(-s), "-P1DT2H")
  expect_true(span(days = 1) != span(hours = 24))
  expect_true(span(days = 1) == time_parse("span", "P1D"))
  expect_error(span(days = 1, hours = -1), "guatiao_error")
  expect_error(span(nanoseconds = 1e9), "guatiao_error")
  expect_true(time_parse("span", "1h 30m") == span(hours = 1, minutes = 30))
  expect_equal(add_span(as.Date("2026-01-31"), span(months = 1)), as.Date("2026-02-28"))
  expect_equal(add_span(as.Date("2026-01-31"), span(months = 1, days = 1)), as.Date("2026-03-01"))
  expect_equal(format(add_span(time_parse("datetime", "2026-10-21T23:30:00"), span(hours = 1))), "2026-10-22T00:30:00")
  expect_error(add_span(as.Date("2026-10-21"), span(hours = 1)), "guatiao_error")
  expect_error(time_parse("span", "P9223372036854775807Y"), "guatiao_error")
}))

case("dates and lengths as values", with_library({
  v <- value(list(d = as.Date("2026-10-21"), t = civil_time(13, 45), dt = civil_datetime(as.Date("2026-10-21"), civil_time(13, 45)),
    ts = instant(0), s = span(weeks = 1), l = exact_length(90, 5e8), n = as.difftime(90, units = "secs"),
    p = as.POSIXct("2026-10-21 13:45:00", tz = "UTC")))
  expect_equal(value_data(v), list(d = "2026-10-21", t = "13:45:00", dt = "2026-10-21T13:45:00",
    ts = "1970-01-01T00:00:00Z", s = "P1W", l = list(90L, 500000000L), n = list(90L, 0L),
    p = "2026-10-21T13:45:00Z"))
  expect_equal(from_value(v$d, "Date"), as.Date("2026-10-21"))
  expect_equal(as.numeric(from_value(v$p, "POSIXct")), as.numeric(as.POSIXct("2026-10-21 13:45:00", tz = "UTC")))
  expect_equal(as.numeric(from_value(v$l, "difftime"), units = "secs"), 90.5)
  expect_equal(read_length(v$l), exact_length(90, 5e8))
  expect_equal(read_length(1.5), exact_length(1, 5e8))
  expect_error(from_value(v$t, "Date"), "guatiao_error")
  expect_error(value(as.difftime(-1, units = "secs")))
  close(v)
}))
