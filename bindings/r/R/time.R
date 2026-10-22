# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# Dates, times of day, civil dates and times, instants, spans and exact
# lengths. Every text is read and written by the library's own functions.

.gt_forms <- c(date = 0L, time = 1L, datetime = 2L, timestamp = 3L, span = 4L)

.gt_form <- function(form) {
  f <- .gt_forms[form]
  if (length(form) != 1L || is.na(f)) {
    stop("form is one of date, time, datetime, timestamp, span", call. = FALSE)
  }
  unname(f)
}

# Days from 1970-01-01 for a civil date, and back (Hinnant's algorithms).
.gt_days_from_civil <- function(y, m, d) {
  y <- y - (m <= 2)
  era <- y %/% 400
  yoe <- y - era * 400
  doy <- (153 * (m + (if (m > 2) -3 else 9)) + 2) %/% 5 + d - 1
  doe <- yoe * 365 + yoe %/% 4 - yoe %/% 100 + doy
  era * 146097 + doe - 719468
}

.gt_civil_from_days <- function(z) {
  z <- z + 719468
  era <- z %/% 146097
  doe <- z - era * 146097
  yoe <- (doe - doe %/% 1460 + doe %/% 36524 - doe %/% 146096) %/% 365
  y <- yoe + era * 400
  doy <- doe - (365 * yoe + yoe %/% 4 - yoe %/% 100)
  mp <- (5 * doy + 2) %/% 153
  d <- doy - (153 * mp + 2) %/% 5 + 1
  m <- mp + (if (mp < 10) 3 else -9)
  c(y + (m <= 2), m, d)
}

.gt_date_fields <- function(x) {
  days <- as.numeric(unclass(x))
  if (length(days) != 1L || !is.finite(days)) stop("one date at a time", call. = FALSE)
  as.numeric(.gt_civil_from_days(floor(days)))
}

.gt_date_of <- function(f) {
  structure(.gt_days_from_civil(f[[1L]], f[[2L]], f[[3L]]), class = "Date")
}

civil_time <- function(hour, minute, second = 0, nanosecond = 0) {
  x <- structure(as.numeric(c(hour, minute, second, nanosecond)), class = "guatiao_civil_time")
  .Call(C_time_format, .gt_forms[["time"]], unclass(x))
  x
}

civil_datetime <- function(date, time) {
  if (!inherits(date, "Date")) date <- as.Date(date)
  if (!inherits(time, "guatiao_civil_time")) stop("time is a civil_time()", call. = FALSE)
  x <- structure(c(.gt_date_fields(date), unclass(time)), class = "guatiao_civil_datetime")
  .Call(C_time_format, .gt_forms[["datetime"]], unclass(x))
  x
}

instant <- function(seconds, nanoseconds = 0) {
  if (inherits(seconds, "POSIXct")) {
    t <- as.numeric(seconds)
    s <- floor(t)
    us <- round((t - s) * 1e6)
    if (us >= 1e6) {
      s <- s + 1
      us <- us - 1e6
    }
    return(instant(s, us * 1000))
  }
  x <- structure(as.numeric(c(seconds, nanoseconds)), class = "guatiao_instant")
  .Call(C_time_format, .gt_forms[["timestamp"]], unclass(x))
  x
}

span <- function(years = 0, months = 0, weeks = 0, days = 0, hours = 0, minutes = 0, seconds = 0,
                 nanoseconds = 0) {
  x <- structure(as.numeric(c(years, months, weeks, days, hours, minutes, seconds, nanoseconds)),
    names = c("years", "months", "weeks", "days", "hours", "minutes", "seconds", "nanoseconds"),
    class = "guatiao_span")
  .Call(C_time_format, .gt_forms[["span"]], unname(unclass(x)))
  x
}

exact_length <- function(seconds, nanoseconds = 0) {
  if (inherits(seconds, "difftime")) {
    t <- as.numeric(seconds, units = "secs")
    if (t < 0) stop("an exact length is never negative", call. = FALSE)
    s <- floor(t)
    return(exact_length(s, round((t - s) * 1e9)))
  }
  s <- as.numeric(seconds)
  ns <- as.numeric(nanoseconds)
  if (length(s) != 1L || length(ns) != 1L || !is.finite(s) || s < 0 || s != floor(s) || s > 2^53) {
    stop("seconds is a whole number from 0 to 2^53", call. = FALSE)
  }
  if (!is.finite(ns) || ns < 0 || ns > 999999999 || ns != floor(ns)) {
    stop("nanoseconds is a whole number from 0 to 999999999", call. = FALSE)
  }
  structure(c(seconds = s, nanoseconds = ns), class = "guatiao_length")
}

# ---- text in, text out ----------------------------------------------------------

time_parse <- function(form, text) {
  f <- .gt_form(form)
  fields <- .Call(C_time_parse, f, enc2utf8(text), FALSE)
  switch(form,
    date = .gt_date_of(fields),
    time = structure(fields, class = "guatiao_civil_time"),
    datetime = structure(fields, class = "guatiao_civil_datetime"),
    timestamp = structure(fields, class = "guatiao_instant"),
    span = structure(fields, names = names(span()), class = "guatiao_span"))
}

time_format <- function(x) {
  if (inherits(x, "Date")) return(.Call(C_time_format, .gt_forms[["date"]], .gt_date_fields(x)))
  if (inherits(x, "POSIXct")) x <- instant(x)
  form <- if (inherits(x, "guatiao_civil_time")) "time"
    else if (inherits(x, "guatiao_civil_datetime")) "datetime"
    else if (inherits(x, "guatiao_instant")) "timestamp"
    else if (inherits(x, "guatiao_span")) "span"
    else stop("time_format takes a Date, POSIXct, civil_time, civil_datetime, instant or span", call. = FALSE)
  .Call(C_time_format, .gt_forms[[form]], unname(unclass(x)))
}

time_normalize <- function(form, text) .Call(C_time_normalize, .gt_form(form), enc2utf8(text))

read_length <- function(x) {
  r <- .Call(C_duration_read, x)
  s <- as.numeric(r[[1L]])
  if (s > 2^53) stop(sprintf("%s seconds is past what an R number holds exactly", r[[1L]]), call. = FALSE)
  exact_length(s, r[[2L]])
}

add_span <- function(x, span) {
  if (!inherits(span, "guatiao_span")) stop("span is a span()", call. = FALSE)
  s <- unname(unclass(span))
  if (inherits(x, "Date")) return(.gt_date_of(.Call(C_date_add_span, .gt_date_fields(x), s)))
  if (inherits(x, "guatiao_civil_datetime")) {
    return(structure(.Call(C_datetime_add_span, unclass(x), s), class = "guatiao_civil_datetime"))
  }
  stop("add_span takes a Date or a civil_datetime", call. = FALSE)
}

# ---- conversions and printing ----------------------------------------------------

as.POSIXct.guatiao_instant <- function(x, tz = "UTC", ...) {
  as.POSIXct(unclass(x)[[1L]] + unclass(x)[[2L]] / 1e9, origin = "1970-01-01", tz = tz)
}

as.Date.guatiao_civil_datetime <- function(x, ...) .gt_date_of(unclass(x)[1:3])

.gt_length_difftime <- function(x) {
  as.difftime(unclass(x)[["seconds"]] + unclass(x)[["nanoseconds"]] / 1e9, units = "secs")
}

Ops.guatiao_span <- function(e1, e2) {
  if (.Generic == "==" || .Generic == "!=") {
    same <- identical(unname(unclass(e1)), unname(unclass(e2)))
    return(if (.Generic == "==") same else !same)
  }
  if (.Generic == "-" && missing(e2)) return(structure(-unclass(e1), class = "guatiao_span"))
  stop(sprintf("%s is not defined for spans", .Generic), call. = FALSE)
}

format.guatiao_civil_time <- function(x, ...) time_format(x)
format.guatiao_civil_datetime <- function(x, ...) time_format(x)
format.guatiao_instant <- function(x, ...) time_format(x)
format.guatiao_span <- function(x, ...) time_format(x)
format.guatiao_length <- function(x, ...) {
  sprintf("%.0f.%09.0fs", unclass(x)[["seconds"]], unclass(x)[["nanoseconds"]])
}
as.character.guatiao_civil_time <- function(x, ...) time_format(x)
as.character.guatiao_civil_datetime <- function(x, ...) time_format(x)
as.character.guatiao_instant <- function(x, ...) time_format(x)
as.character.guatiao_span <- function(x, ...) time_format(x)

.gt_print_text <- function(x, ...) {
  cat("<", sub("^guatiao_", "", class(x)[1L]), " ", format(x), ">\n", sep = "")
  invisible(x)
}
print.guatiao_civil_time <- .gt_print_text
print.guatiao_civil_datetime <- .gt_print_text
print.guatiao_instant <- .gt_print_text
print.guatiao_span <- .gt_print_text
print.guatiao_length <- .gt_print_text

lower_value.Date <- function(x) {
  if (length(x) == 1L) return(time_format(x))
  lapply(seq_along(x), function(i) time_format(x[i]))
}
lower_value.POSIXct <- function(x) {
  if (length(x) == 1L) return(time_format(x))
  lapply(seq_along(x), function(i) time_format(x[i]))
}
lower_value.difftime <- function(x) lower_value(exact_length(x))
lower_value.guatiao_civil_time <- function(x) time_format(x)
lower_value.guatiao_civil_datetime <- function(x) time_format(x)
lower_value.guatiao_instant <- function(x) time_format(x)
lower_value.guatiao_span <- function(x) time_format(x)
lower_value.guatiao_length <- function(x) {
  list(unname(unclass(x)[["seconds"]]), unname(unclass(x)[["nanoseconds"]]))
}
