\name{time_parse}
\alias{time_parse}
\alias{time_format}
\alias{time_normalize}
\alias{read_length}
\alias{add_span}
\alias{civil_time}
\alias{civil_datetime}
\alias{instant}
\alias{span}
\alias{exact_length}
\title{Dates, times of day, civil dates and times, instants, spans, exact lengths}
\usage{
time_parse(form, text)
time_format(x)
time_normalize(form, text)
read_length(x)
add_span(x, span)
civil_time(hour, minute, second = 0, nanosecond = 0)
civil_datetime(date, time)
instant(seconds, nanoseconds = 0)
span(years = 0, months = 0, weeks = 0, days = 0, hours = 0, minutes = 0,
  seconds = 0, nanoseconds = 0)
exact_length(seconds, nanoseconds = 0)
}
\arguments{
\item{form}{\code{"date"}, \code{"time"}, \code{"datetime"}, \code{"timestamp"} or \code{"span"}.}
\item{text}{The text to read.}
\item{x}{For \code{time_format}, a \code{Date}, \code{POSIXct}, \code{civil_time},
\code{civil_datetime}, \code{instant} or \code{span}; for \code{read_length}, a
value or R data; for \code{add_span}, a \code{Date} or \code{civil_datetime}.}
\item{span}{A \code{span}.}
\item{hour, minute, second, nanosecond}{A time of day: 0 to 23, 0 to 59, 0 to 59
(no leap second), 0 to 999999999.}
\item{date}{A \code{Date}.}
\item{time}{A \code{civil_time}.}
\item{seconds}{For \code{instant}, whole seconds from 1970-01-01T00:00:00Z, or a
\code{POSIXct}; for \code{span}, seconds; for \code{exact_length}, whole seconds
from 0 to 2^53, or a \code{difftime}.}
\item{nanoseconds}{Below a second, 0 to 999999999 (counted forward for an
instant; with the span's sign for a span).}
\item{years, months, weeks, days, hours, minutes}{A span's units, one sign for the whole.}
}
\value{\code{time_parse} returns a \code{Date}, \code{civil_time},
\code{civil_datetime}, \code{instant} or \code{span}; \code{time_format} and
\code{time_normalize} a string; \code{read_length} an \code{exact_length};
\code{add_span} what it was given.}
\description{
The forms are text in a value, fixed at
\url{https://boriken-dev.github.io/guatiao/encodings/}, and every text is read
and written by the library's own functions. Text a grammar refuses signals a
\code{guatiao_unparsed} (\code{form}, \code{at} the byte offset, \code{expected}),
which never quotes the text.

A date is R's \code{Date}, years 0000 to 9999. R has no time of day or civil
date and time of its own: \code{civil_time} and \code{civil_datetime} hold one,
to the nanosecond. An instant is \code{instant} (seconds and nanoseconds, read
in UTC; \code{as.POSIXct} converts); a \code{POSIXct} written as a value is
rounded to the microsecond, about what its double holds today. A span counts
calendar units, checked by the library (mixed signs and a second or more of
nanoseconds refused); \code{==} compares unit by unit (\code{P1D} is not
\code{PT24H}), \code{-} negates. \code{add_span} applies one to a date (time
units refused) or a civil date and time. An exact length is never negative;
\code{read_length} reads any of its three spellings. Units and seconds past
2^53 are refused: an R number holds whole numbers exactly only that far.
}
\examples{
if (nzchar(Sys.getenv("GUATIAO_LIBRARY"))) {
  time_parse("date", "2026-10-21")
  time_normalize("span", "1h 30m")
  add_span(as.Date("2026-01-31"), span(months = 1))
  format(time_parse("timestamp", "2026-10-21T19:15:00+05:30"))
}
}
