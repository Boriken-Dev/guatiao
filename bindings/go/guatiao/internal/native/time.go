// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package native

/*
#include "native.h"
*/
import "C"

import "unsafe"

// Date, Time, DateTime, Timestamp and Span are the header's structs of
// the same names, as Go values.
type Date struct {
	Year       int32
	Month, Day uint8
}

type Time struct {
	Hour, Minute, Second uint8
	Nanosecond           uint32
}

type DateTime struct {
	Date Date
	Time Time
}

type Timestamp struct {
	Seconds     int64
	Nanoseconds uint32
}

type Span struct {
	Years, Months, Weeks, Days, Hours, Minutes, Seconds int64
	Nanoseconds                                         int32
}

// Refusal is a guatiao_text_error read and freed: the byte offset the
// reading stopped at, and what it expected there.
type Refusal struct {
	At       uintptr
	Expected string
}

func takeRefusal(e *C.guatiao_text_error) Refusal {
	r := Refusal{At: uintptr(e.at)}
	node := unsafe.Pointer(&e.expected)
	if Tag(node) == TagString {
		r.Expected = TextOf(node)
	}
	ValueFree(node)
	return r
}

func cDate(d Date) C.guatiao_date {
	return C.guatiao_date{year: C.int32_t(d.Year), month: C.uint8_t(d.Month), day: C.uint8_t(d.Day)}
}

func goDate(d C.guatiao_date) Date {
	return Date{Year: int32(d.year), Month: uint8(d.month), Day: uint8(d.day)}
}

func cTime(t Time) C.guatiao_time {
	return C.guatiao_time{hour: C.uint8_t(t.Hour), minute: C.uint8_t(t.Minute), second: C.uint8_t(t.Second),
		nanosecond: C.uint32_t(t.Nanosecond)}
}

func goTime(t C.guatiao_time) Time {
	return Time{Hour: uint8(t.hour), Minute: uint8(t.minute), Second: uint8(t.second), Nanosecond: uint32(t.nanosecond)}
}

func cSpan(s Span) C.guatiao_span {
	return C.guatiao_span{years: C.int64_t(s.Years), months: C.int64_t(s.Months), weeks: C.int64_t(s.Weeks),
		days: C.int64_t(s.Days), hours: C.int64_t(s.Hours), minutes: C.int64_t(s.Minutes),
		seconds: C.int64_t(s.Seconds), nanoseconds: C.int32_t(s.Nanoseconds)}
}

func goSpan(s C.guatiao_span) Span {
	return Span{Years: int64(s.years), Months: int64(s.months), Weeks: int64(s.weeks), Days: int64(s.days),
		Hours: int64(s.hours), Minutes: int64(s.minutes), Seconds: int64(s.seconds), Nanoseconds: int32(s.nanoseconds)}
}

// DateParse reads text as a date.
func DateParse(alloc unsafe.Pointer, text string) (Date, Refusal, uint32) {
	var out C.guatiao_date
	var e C.guatiao_text_error
	st := uint32(C.gt_core_guatiao_date_parse(a(alloc), str(text), &out, &e))
	return goDate(out), takeRefusal(&e), st
}

// TimeParse reads text as a time of day.
func TimeParse(alloc unsafe.Pointer, text string) (Time, Refusal, uint32) {
	var out C.guatiao_time
	var e C.guatiao_text_error
	st := uint32(C.gt_core_guatiao_time_parse(a(alloc), str(text), &out, &e))
	return goTime(out), takeRefusal(&e), st
}

// DateTimeParse reads text as a civil date and time.
func DateTimeParse(alloc unsafe.Pointer, text string) (DateTime, Refusal, uint32) {
	var out C.guatiao_datetime
	var e C.guatiao_text_error
	st := uint32(C.gt_core_guatiao_datetime_parse(a(alloc), str(text), &out, &e))
	return DateTime{Date: goDate(out.date), Time: goTime(out.time)}, takeRefusal(&e), st
}

// TimestampParse reads text as an instant.
func TimestampParse(alloc unsafe.Pointer, text string) (Timestamp, Refusal, uint32) {
	var out C.guatiao_timestamp
	var e C.guatiao_text_error
	st := uint32(C.gt_core_guatiao_timestamp_parse(a(alloc), str(text), &out, &e))
	return Timestamp{Seconds: int64(out.seconds), Nanoseconds: uint32(out.nanoseconds)}, takeRefusal(&e), st
}

// SpanParse reads text as a span.
func SpanParse(alloc unsafe.Pointer, text string) (Span, Refusal, uint32) {
	var out C.guatiao_span
	var e C.guatiao_text_error
	st := uint32(C.gt_core_guatiao_span_parse(a(alloc), str(text), &out, &e))
	return goSpan(out), takeRefusal(&e), st
}

// DateFormat writes a date's text into out, a string node.
func DateFormat(alloc unsafe.Pointer, d Date, out unsafe.Pointer) uint32 {
	c := cDate(d)
	return uint32(C.gt_core_guatiao_date_format(a(alloc), &c, n(out)))
}

// TimeFormat writes a time of day's text into out.
func TimeFormat(alloc unsafe.Pointer, t Time, out unsafe.Pointer) uint32 {
	c := cTime(t)
	return uint32(C.gt_core_guatiao_time_format(a(alloc), &c, n(out)))
}

// DateTimeFormat writes a civil date and time's text into out.
func DateTimeFormat(alloc unsafe.Pointer, d DateTime, out unsafe.Pointer) uint32 {
	c := C.guatiao_datetime{date: cDate(d.Date), time: cTime(d.Time)}
	return uint32(C.gt_core_guatiao_datetime_format(a(alloc), &c, n(out)))
}

// TimestampFormat writes an instant's text into out.
func TimestampFormat(alloc unsafe.Pointer, t Timestamp, out unsafe.Pointer) uint32 {
	c := C.guatiao_timestamp{seconds: C.int64_t(t.Seconds), nanoseconds: C.uint32_t(t.Nanoseconds)}
	return uint32(C.gt_core_guatiao_timestamp_format(a(alloc), &c, n(out)))
}

// SpanFormat writes a span's canonical text into out.
func SpanFormat(alloc unsafe.Pointer, s Span, out unsafe.Pointer) uint32 {
	c := cSpan(s)
	return uint32(C.gt_core_guatiao_span_format(a(alloc), &c, n(out)))
}

// Normalize rewrites text in its form's canonical spelling, into out.
func Normalize(form string, alloc unsafe.Pointer, text string, out unsafe.Pointer) (Refusal, uint32) {
	var e C.guatiao_text_error
	var st C.guatiao_status
	switch form {
	case "date":
		st = C.gt_core_guatiao_date_normalize(a(alloc), str(text), n(out), &e)
	case "time":
		st = C.gt_core_guatiao_time_normalize(a(alloc), str(text), n(out), &e)
	case "datetime":
		st = C.gt_core_guatiao_datetime_normalize(a(alloc), str(text), n(out), &e)
	case "timestamp":
		st = C.gt_core_guatiao_timestamp_normalize(a(alloc), str(text), n(out), &e)
	case "span":
		st = C.gt_core_guatiao_span_normalize(a(alloc), str(text), n(out), &e)
	default:
		return Refusal{}, ErrWrongKind
	}
	return takeRefusal(&e), uint32(st)
}

// DateAddSpan applies a span to a date.
func DateAddSpan(d Date, s Span) (Date, uint32) {
	cd, cs := cDate(d), cSpan(s)
	var out C.guatiao_date
	st := uint32(C.gt_core_guatiao_date_add_span(&cd, &cs, &out))
	return goDate(out), st
}

// DateTimeAddSpan applies a span to a civil date and time.
func DateTimeAddSpan(d DateTime, s Span) (DateTime, uint32) {
	cd := C.guatiao_datetime{date: cDate(d.Date), time: cTime(d.Time)}
	cs := cSpan(s)
	var out C.guatiao_datetime
	st := uint32(C.gt_core_guatiao_datetime_add_span(&cd, &cs, &out))
	return DateTime{Date: goDate(out.date), Time: goTime(out.time)}, st
}

// DurationRead reads an exact length in any of its three spellings.
func DurationRead(node unsafe.Pointer) (uint64, uint32, uint32) {
	var secs C.uint64_t
	var nanos C.uint32_t
	st := uint32(C.gt_core_guatiao_duration_read(n(node), &secs, &nanos))
	return uint64(secs), uint32(nanos), st
}
