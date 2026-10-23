// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// guatiao for C++: dates, times of day, civil date-times, instants, spans
// and exact lengths, every text read and written by the library's own
// functions (the forms are fixed at
// https://boriken-dev.github.io/guatiao/encodings/). Part of
// <guatiao/guatiao.hpp>.

#ifndef GUATIAO_TIME_HPP
#define GUATIAO_TIME_HPP

#include <guatiao/value.hpp>

#include <chrono>

namespace guatiao {

struct span;

namespace detail {

/// A text form's refusal: BAD_VALUE, with `{"at": <byte>, "expected":
/// <words>}` as its detail. It never quotes the text.
inline ::guatiao::error unparsed(guatiao_status s, guatiao_text_error& te, std::string_view form) {
  std::string expected;
  if (te.expected.tag == GUATIAO_STRING) expected = std::string(text(te.expected.payload.text));
  guatiao_value_free(&te.expected);
  if (s != GUATIAO_ERR_BAD_VALUE) return error_of(s);
  std::string message = std::string(form) + ": expected " + (expected.empty() ? "something else" : expected) +
                        " at byte " + text_of_integer(te.at);
  guatiao_value d = blank();
  if (item::map({{"at", te.at}, {"expected", expected}}).build(default_alloc(), &d) != GUATIAO_OK) d = blank();
  return ::guatiao::error(static_cast<status>(s), std::move(message), d);
}

/// The text of a STRING value a call wrote, the value freed.
inline result<std::string> take_text(guatiao_status s, guatiao_value& out) {
  if (s != GUATIAO_OK) {
    guatiao_value_free(&out);
    return error_of(s);
  }
  std::string t;
  if (out.tag == GUATIAO_STRING) t = std::string(text(out.payload.text));
  guatiao_value_free(&out);
  return t;
}

inline bool fits_u8(unsigned v) noexcept { return v <= 0xFFu; }

}  // namespace detail

/// A day on the calendar: year 0 to 9999, month 1 to 12, day from 1.
struct date {
  std::int32_t year = 0;
  unsigned month = 0;
  unsigned day = 0;

  /// `YYYY-MM-DD`; a refusal is BAD_VALUE with `{"at", "expected"}`.
  static result<date> try_parse(std::string_view text) {
    guatiao_date d{};
    guatiao_text_error te{};
    guatiao_status s = guatiao_date_parse(default_alloc(), detail::str(text), &d, &te);
    if (s != GUATIAO_OK) return detail::unparsed(s, te, "a date");
    guatiao_value_free(&te.expected);
    return from(d);
  }
  result<std::string> try_to_string() const {
    guatiao_date d;
    if (!raw(d)) return detail::error_of(GUATIAO_ERR_BAD_VALUE, "a date field is out of range");
    guatiao_value out = detail::blank();
    return detail::take_text(guatiao_date_format(default_alloc(), &d, &out), out);
  }
  /// `s` after this day (before it, for a negative span): years and
  /// months first, then weeks and days.
  result<date> try_add(const span& s) const;
#if GUATIAO_EXCEPTIONS
  static date parse(std::string_view text) { return try_parse(text).value(); }
  std::string to_string() const { return try_to_string().value(); }
  date add(const span& s) const { return try_add(s).value(); }
#endif

  static date from(const guatiao_date& d) noexcept { return {d.year, d.month, d.day}; }
  /// The C struct; false when a field does not fit its C width.
  bool raw(guatiao_date& out) const noexcept {
    if (!detail::fits_u8(month) || !detail::fits_u8(day)) return false;
    out.year = year;
    out.month = static_cast<std::uint8_t>(month);
    out.day = static_cast<std::uint8_t>(day);
    return true;
  }
  friend bool operator==(const date& a, const date& b) noexcept {
    return a.year == b.year && a.month == b.month && a.day == b.day;
  }
  friend bool operator!=(const date& a, const date& b) noexcept { return !(a == b); }
};

/// A time of day with no offset: hour 0 to 23, minute and second 0 to 59,
/// nanoseconds below the second.
struct time_of_day {
  unsigned hour = 0;
  unsigned minute = 0;
  unsigned second = 0;
  std::uint32_t nanosecond = 0;

  /// `HH:MM:SS[.fraction]`.
  static result<time_of_day> try_parse(std::string_view text) {
    guatiao_time t{};
    guatiao_text_error te{};
    guatiao_status s = guatiao_time_parse(default_alloc(), detail::str(text), &t, &te);
    if (s != GUATIAO_OK) return detail::unparsed(s, te, "a time of day");
    guatiao_value_free(&te.expected);
    return from(t);
  }
  result<std::string> try_to_string() const {
    guatiao_time t;
    if (!raw(t)) return detail::error_of(GUATIAO_ERR_BAD_VALUE, "a time field is out of range");
    guatiao_value out = detail::blank();
    return detail::take_text(guatiao_time_format(default_alloc(), &t, &out), out);
  }
#if GUATIAO_EXCEPTIONS
  static time_of_day parse(std::string_view text) { return try_parse(text).value(); }
  std::string to_string() const { return try_to_string().value(); }
#endif

  static time_of_day from(const guatiao_time& t) noexcept { return {t.hour, t.minute, t.second, t.nanosecond}; }
  bool raw(guatiao_time& out) const noexcept {
    if (!detail::fits_u8(hour) || !detail::fits_u8(minute) || !detail::fits_u8(second)) return false;
    out.hour = static_cast<std::uint8_t>(hour);
    out.minute = static_cast<std::uint8_t>(minute);
    out.second = static_cast<std::uint8_t>(second);
    out.nanosecond = nanosecond;
    return true;
  }
  friend bool operator==(const time_of_day& a, const time_of_day& b) noexcept {
    return a.hour == b.hour && a.minute == b.minute && a.second == b.second && a.nanosecond == b.nanosecond;
  }
  friend bool operator!=(const time_of_day& a, const time_of_day& b) noexcept { return !(a == b); }
};

/// A civil date and time, as a wall clock shows it: no zone.
struct date_time {
  ::guatiao::date date;
  ::guatiao::time_of_day time;

  /// The date, `T` or `t`, the time of day; no offset.
  static result<date_time> try_parse(std::string_view text) {
    guatiao_datetime dt{};
    guatiao_text_error te{};
    guatiao_status s = guatiao_datetime_parse(default_alloc(), detail::str(text), &dt, &te);
    if (s != GUATIAO_OK) return detail::unparsed(s, te, "a date and time");
    guatiao_value_free(&te.expected);
    return from(dt);
  }
  result<std::string> try_to_string() const {
    guatiao_datetime dt;
    if (!raw(dt)) return detail::error_of(GUATIAO_ERR_BAD_VALUE, "a field is out of range");
    guatiao_value out = detail::blank();
    return detail::take_text(guatiao_datetime_format(default_alloc(), &dt, &out), out);
  }
  /// `s` after this date-time: as `date::add`, then the clock fields
  /// carried across midnight (a day is 24 hours).
  result<date_time> try_add(const span& s) const;
#if GUATIAO_EXCEPTIONS
  static date_time parse(std::string_view text) { return try_parse(text).value(); }
  std::string to_string() const { return try_to_string().value(); }
  date_time add(const span& s) const { return try_add(s).value(); }
#endif

  static date_time from(const guatiao_datetime& dt) noexcept {
    return {::guatiao::date::from(dt.date), ::guatiao::time_of_day::from(dt.time)};
  }
  bool raw(guatiao_datetime& out) const noexcept { return date.raw(out.date) && time.raw(out.time); }
  friend bool operator==(const date_time& a, const date_time& b) noexcept {
    return a.date == b.date && a.time == b.time;
  }
  friend bool operator!=(const date_time& a, const date_time& b) noexcept { return !(a == b); }
};

/// An instant: whole seconds from 1970-01-01T00:00:00Z (negative before
/// it), then nanoseconds counted forward, 0 to 999999999.
struct timestamp {
  std::int64_t seconds = 0;
  std::uint32_t nanoseconds = 0;

  /// An RFC 3339 date-time with any offset, as the instant it names.
  static result<timestamp> try_parse(std::string_view text) {
    guatiao_timestamp t{};
    guatiao_text_error te{};
    guatiao_status s = guatiao_timestamp_parse(default_alloc(), detail::str(text), &t, &te);
    if (s != GUATIAO_OK) return detail::unparsed(s, te, "an instant");
    guatiao_value_free(&te.expected);
    return timestamp{t.seconds, t.nanoseconds};
  }
  /// In UTC: `YYYY-MM-DDTHH:MM:SS[.fraction]Z`.
  result<std::string> try_to_string() const {
    guatiao_timestamp t{seconds, nanoseconds};
    guatiao_value out = detail::blank();
    return detail::take_text(guatiao_timestamp_format(default_alloc(), &t, &out), out);
  }
#if GUATIAO_EXCEPTIONS
  static timestamp parse(std::string_view text) { return try_parse(text).value(); }
  std::string to_string() const { return try_to_string().value(); }
#endif

  /// The instant on the system clock, to the clock's resolution.
  template <class Clock = std::chrono::system_clock>
  typename Clock::time_point to_time_point() const {
    auto since = std::chrono::duration_cast<typename Clock::duration>(std::chrono::seconds(seconds) +
                                                         std::chrono::nanoseconds(nanoseconds));
    return typename Clock::time_point(since);
  }
  template <class Clock, class Duration>
  static timestamp from(std::chrono::time_point<Clock, Duration> at) {
    auto ns = std::chrono::duration_cast<std::chrono::nanoseconds>(at.time_since_epoch()).count();
    std::int64_t s = ns / 1000000000;
    std::int64_t rest = ns % 1000000000;
    if (rest < 0) rest += 1000000000, s -= 1;
    return {s, static_cast<std::uint32_t>(rest)};
  }
  friend bool operator==(const timestamp& a, const timestamp& b) noexcept {
    return a.seconds == b.seconds && a.nanoseconds == b.nanoseconds;
  }
  friend bool operator!=(const timestamp& a, const timestamp& b) noexcept { return !(a == b); }
};

/// A length of time in calendar units, each counted on its own, with one
/// sign for the whole: `-P1DT2H` is `{.days = -1, .hours = -2}`.
struct span {
  std::int64_t years = 0;
  std::int64_t months = 0;
  std::int64_t weeks = 0;
  std::int64_t days = 0;
  std::int64_t hours = 0;
  std::int64_t minutes = 0;
  std::int64_t seconds = 0;
  std::int32_t nanoseconds = 0;

  /// ISO 8601 (`P1Y2M3W4DT5H6M7.5S`) or the friendly form (`1h 30m`).
  static result<span> try_parse(std::string_view text) {
    guatiao_span sp{};
    guatiao_text_error te{};
    guatiao_status s = guatiao_span_parse(default_alloc(), detail::str(text), &sp, &te);
    if (s != GUATIAO_OK) return detail::unparsed(s, te, "a span");
    guatiao_value_free(&te.expected);
    return from(sp);
  }
  /// The canonical ISO 8601 text (`PT0S` for zero); fields of both signs
  /// are BAD_VALUE.
  result<std::string> try_to_string() const {
    guatiao_span sp = raw();
    guatiao_value out = detail::blank();
    return detail::take_text(guatiao_span_format(default_alloc(), &sp, &out), out);
  }
#if GUATIAO_EXCEPTIONS
  static span parse(std::string_view text) { return try_parse(text).value(); }
  std::string to_string() const { return try_to_string().value(); }
#endif

  bool is_zero() const noexcept {
    return !years && !months && !weeks && !days && !hours && !minutes && !seconds && !nanoseconds;
  }
  span negated() const noexcept {
    return {-years, -months, -weeks, -days, -hours, -minutes, -seconds, -nanoseconds};
  }
  static span from(const guatiao_span& s) noexcept {
    return {s.years, s.months, s.weeks, s.days, s.hours, s.minutes, s.seconds, s.nanoseconds};
  }
  guatiao_span raw() const noexcept {
    return guatiao_span{years, months, weeks, days, hours, minutes, seconds, nanoseconds};
  }
  /// Unit by unit: `P1D` is not `PT24H`.
  friend bool operator==(const span& a, const span& b) noexcept {
    return a.years == b.years && a.months == b.months && a.weeks == b.weeks && a.days == b.days &&
           a.hours == b.hours && a.minutes == b.minutes && a.seconds == b.seconds &&
           a.nanoseconds == b.nanoseconds;
  }
  friend bool operator!=(const span& a, const span& b) noexcept { return !(a == b); }
};

inline result<date> date::try_add(const span& s) const {
  guatiao_date d;
  if (!raw(d)) return detail::error_of(GUATIAO_ERR_BAD_VALUE, "a date field is out of range");
  guatiao_span sp = s.raw();
  guatiao_date out{};
  guatiao_status st = guatiao_date_add_span(&d, &sp, &out);
  if (st != GUATIAO_OK) return detail::error_of(st, "the span does not apply to this date");
  return from(out);
}

inline result<date_time> date_time::try_add(const span& s) const {
  guatiao_datetime dt;
  if (!raw(dt)) return detail::error_of(GUATIAO_ERR_BAD_VALUE, "a field is out of range");
  guatiao_span sp = s.raw();
  guatiao_datetime out{};
  guatiao_status st = guatiao_datetime_add_span(&dt, &sp, &out);
  if (st != GUATIAO_OK) return detail::error_of(st, "the span does not apply to this date and time");
  return from(out);
}

/// An exact length of time: whole seconds and the nanoseconds below one.
/// Written as the list `[seconds, nanoseconds]`.
struct length {
  std::uint64_t seconds = 0;
  std::uint32_t nanoseconds = 0;

  /// Reads any of its three spellings: `[s, ns]`, a number of seconds
  /// (`1.5`), or `{"secs": .., "nanos": ..}`.
  static result<length> try_read(view v) {
    std::uint64_t s = 0;
    std::uint32_t ns = 0;
    guatiao_status st = guatiao_duration_read(v.raw(), &s, &ns);
    if (st != GUATIAO_OK) return detail::error_of(st, "not an exact length");
    return length{s, ns};
  }
#if GUATIAO_EXCEPTIONS
  static length read(view v) { return try_read(v).value(); }
#endif
  /// The list it is written as.
  item to_item() const { return item::list({seconds, nanoseconds}); }

  template <class Rep, class Period>
  static length from(std::chrono::duration<Rep, Period> d) {
    auto ns = std::chrono::duration_cast<std::chrono::nanoseconds>(d).count();
    if (ns < 0) ns = 0;
    return {static_cast<std::uint64_t>(ns / 1000000000), static_cast<std::uint32_t>(ns % 1000000000)};
  }
  /// As nanoseconds; wraps past about 292 years, which the type holds and
  /// `std::chrono::nanoseconds` does not.
  std::chrono::nanoseconds to_duration() const noexcept {
    return std::chrono::nanoseconds(static_cast<std::int64_t>(seconds * 1000000000u + nanoseconds));
  }
  friend bool operator==(const length& a, const length& b) noexcept {
    return a.seconds == b.seconds && a.nanoseconds == b.nanoseconds;
  }
  friend bool operator!=(const length& a, const length& b) noexcept { return !(a == b); }
};

/// The forms `normalize` reads.
enum class form { date, time, datetime, timestamp, span };

/// `text` read as `f` and written in its canonical form: `1h 30m` is
/// `PT1H30M`, an instant is written in UTC.
inline result<std::string> try_normalize(form f, std::string_view text) {
  guatiao_value out = detail::blank();
  guatiao_text_error te{};
  guatiao_status s = GUATIAO_ERR_BAD_VALUE;
  const char* name = "a value";
  switch (f) {
    case form::date: s = guatiao_date_normalize(default_alloc(), detail::str(text), &out, &te), name = "a date"; break;
    case form::time: s = guatiao_time_normalize(default_alloc(), detail::str(text), &out, &te), name = "a time of day"; break;
    case form::datetime:
      s = guatiao_datetime_normalize(default_alloc(), detail::str(text), &out, &te), name = "a date and time";
      break;
    case form::timestamp:
      s = guatiao_timestamp_normalize(default_alloc(), detail::str(text), &out, &te), name = "an instant";
      break;
    case form::span: s = guatiao_span_normalize(default_alloc(), detail::str(text), &out, &te), name = "a span"; break;
  }
  if (s != GUATIAO_OK) {
    guatiao_value_free(&out);
    return detail::unparsed(s, te, name);
  }
  guatiao_value_free(&te.expected);
  return detail::take_text(s, out);
}
#if GUATIAO_EXCEPTIONS
inline std::string normalize(form f, std::string_view text) { return try_normalize(f, text).value(); }
#endif

}  // namespace guatiao

#endif  // GUATIAO_TIME_HPP
