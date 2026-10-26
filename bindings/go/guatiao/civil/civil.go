// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Package civil holds the dates, times, spans and exact lengths of time
// guatiao's encodings define, read and written by the guatiao library's
// own functions so that every language spells them alike.
//
// A Date, Time or DateTime is a calendar date, a time of day, or both,
// with no time zone: what a wall clock shows. A Span is a length in
// calendar units, each counted on its own (P1D is not PT24H). A Length is
// an exact length, [seconds, nanoseconds]. An instant is a time.Time; an
// exact length short enough is a time.Duration, and the guatiao package
// converts both.
//
// Each type is its text in a value: they implement encoding.TextMarshaler
// and TextUnmarshaler, which guatiao.NewValue and Ref.Decode use.
package civil

import (
	"fmt"
	"math"
	"time"
	"unsafe"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"
	"github.com/Boriken-Dev/guatiao/bindings/go/guatiao/internal/native"
)

// Form names one of the text forms.
type Form string

// The forms.
const (
	DateForm      Form = "date"
	TimeForm      Form = "time"
	DateTimeForm  Form = "datetime"
	TimestampForm Form = "timestamp"
	SpanForm      Form = "span"
)

// Unparsed is text the form's grammar refused: the byte offset in the
// UTF-8 text where reading stopped, and what would have been accepted
// there. It never quotes the text. errors.Is matches it against
// guatiao.ErrBadValue.
type Unparsed struct {
	Form     Form
	At       int
	Expected string
}

func (e *Unparsed) Error() string {
	return fmt.Sprintf("guatiao: not a %s: expected %s at byte %d", e.Form, e.Expected, e.At)
}

// Unwrap is guatiao.ErrBadValue.
func (e *Unparsed) Unwrap() error { return guatiao.ErrBadValue }

func coreAlloc() (unsafe.Pointer, error) {
	if err := guatiao.Load(); err != nil {
		return nil, err
	}
	lib, err := native.Core()
	if err != nil {
		return nil, err
	}
	return lib.Alloc(), nil
}

func refused(form Form, r native.Refusal, st uint32) error {
	if st == native.OK {
		return nil
	}
	if st == native.ErrBadValue {
		expected := r.Expected
		if expected == "" {
			expected = "a valid text"
		}
		return &Unparsed{Form: form, At: int(r.At), Expected: expected}
	}
	return guatiao.StatusError(st, r.Expected)
}

// textOf runs a formatter into a scratch node and answers its text.
func textOf(form Form, write func(alloc, out unsafe.Pointer) uint32) (string, error) {
	alloc, err := coreAlloc()
	if err != nil {
		return "", err
	}
	out := native.Calloc(native.SizeValue)
	defer func() {
		native.ValueFree(out)
		native.Free(out)
	}()
	st := write(alloc, out)
	if st == native.ErrBadValue {
		return "", &guatiao.Error{Status: guatiao.BadValue, Message: "the fields are not a " + string(form)}
	}
	if err := guatiao.StatusError(st, ""); err != nil {
		return "", err
	}
	return native.TextOf(out), nil
}

// Normalize rewrites text in its form's canonical spelling, every
// nanosecond kept.
func Normalize(form Form, text string) (string, error) {
	alloc, err := coreAlloc()
	if err != nil {
		return "", err
	}
	switch form {
	case DateForm, TimeForm, DateTimeForm, TimestampForm, SpanForm:
	default:
		return "", fmt.Errorf("guatiao: unknown form %q", form)
	}
	out := native.Calloc(native.SizeValue)
	defer func() {
		native.ValueFree(out)
		native.Free(out)
	}()
	r, st := native.Normalize(string(form), alloc, text, out)
	if err := refused(form, r, st); err != nil {
		return "", err
	}
	return native.TextOf(out), nil
}

// ---- dates and times ---------------------------------------------------------

// Date is a day on the proleptic Gregorian calendar, year 0 to 9999.
type Date struct {
	Year  int
	Month time.Month
	Day   int
}

func (d Date) fields() (native.Date, error) {
	if d.Year < math.MinInt32 || d.Year > math.MaxInt32 || d.Month < 0 || d.Month > 255 || d.Day < 0 || d.Day > 255 {
		return native.Date{}, &guatiao.Error{Status: guatiao.BadValue, Message: "the fields are not a date"}
	}
	return native.Date{Year: int32(d.Year), Month: uint8(d.Month), Day: uint8(d.Day)}, nil
}

func dateOf(f native.Date) Date {
	return Date{Year: int(f.Year), Month: time.Month(f.Month), Day: int(f.Day)}
}

// NewDate is the date, checked: guatiao.ErrBadValue for a day the month
// lacks or a year outside 0 to 9999.
func NewDate(year int, month time.Month, day int) (Date, error) {
	d := Date{Year: year, Month: month, Day: day}
	if _, err := d.MarshalText(); err != nil {
		return Date{}, err
	}
	return d, nil
}

// ParseDate reads text as a date.
func ParseDate(text string) (Date, error) {
	alloc, err := coreAlloc()
	if err != nil {
		return Date{}, err
	}
	f, r, st := native.DateParse(alloc, text)
	if err := refused(DateForm, r, st); err != nil {
		return Date{}, err
	}
	return dateOf(f), nil
}

// MarshalText is the date's canonical text, YYYY-MM-DD.
func (d Date) MarshalText() ([]byte, error) {
	f, err := d.fields()
	if err != nil {
		return nil, err
	}
	text, err := textOf(DateForm, func(alloc, out unsafe.Pointer) uint32 { return native.DateFormat(alloc, f, out) })
	return []byte(text), err
}

// UnmarshalText reads the date's text.
func (d *Date) UnmarshalText(text []byte) error {
	parsed, err := ParseDate(string(text))
	if err == nil {
		*d = parsed
	}
	return err
}

// String is the canonical text, or a note saying the fields are no date.
func (d Date) String() string { return stringOf(d.MarshalText()) }

func stringOf(b []byte, err error) string {
	if err != nil {
		return "!(" + err.Error() + ")"
	}
	return string(b)
}

// Add is the date moved by span (years and months first, the month's end
// clamped, then weeks and days); guatiao.ErrBadValue for a span with
// hours or less, or a step leaving the years 0 to 9999.
func (d Date) Add(span Span) (Date, error) {
	f, err := d.fields()
	if err != nil {
		return Date{}, err
	}
	if _, err := coreAlloc(); err != nil {
		return Date{}, err
	}
	out, st := native.DateAddSpan(f, span.fields())
	if err := guatiao.StatusError(st, ""); err != nil {
		return Date{}, err
	}
	return dateOf(out), nil
}

// Compare is -1, 0 or +1 as d is before, the same as, or after o.
func (d Date) Compare(o Date) int {
	switch {
	case d.Year != o.Year:
		return sign(d.Year - o.Year)
	case d.Month != o.Month:
		return sign(int(d.Month) - int(o.Month))
	}
	return sign(d.Day - o.Day)
}

func sign(x int) int {
	switch {
	case x < 0:
		return -1
	case x > 0:
		return 1
	}
	return 0
}

// GuatiaoSchema is the form's schema.
func (Date) GuatiaoSchema() map[string]any { return map[string]any{"type": "string", "format": "date"} }

// Time is a time of day with no offset: hour 0 to 23, minute and second 0
// to 59 (no leap second), and the nanoseconds below the second.
type Time struct {
	Hour, Minute, Second int
	Nanosecond           int
}

func (t Time) fields() (native.Time, error) {
	if t.Hour < 0 || t.Hour > 255 || t.Minute < 0 || t.Minute > 255 || t.Second < 0 || t.Second > 255 ||
		t.Nanosecond < 0 || uint64(t.Nanosecond) > math.MaxUint32 {
		return native.Time{}, &guatiao.Error{Status: guatiao.BadValue, Message: "the fields are not a time"}
	}
	return native.Time{Hour: uint8(t.Hour), Minute: uint8(t.Minute), Second: uint8(t.Second), Nanosecond: uint32(t.Nanosecond)}, nil
}

func timeOf(f native.Time) Time {
	return Time{Hour: int(f.Hour), Minute: int(f.Minute), Second: int(f.Second), Nanosecond: int(f.Nanosecond)}
}

// NewTime is the time of day, checked.
func NewTime(hour, minute, second, nanosecond int) (Time, error) {
	t := Time{Hour: hour, Minute: minute, Second: second, Nanosecond: nanosecond}
	if _, err := t.MarshalText(); err != nil {
		return Time{}, err
	}
	return t, nil
}

// ParseTime reads text as a time of day.
func ParseTime(text string) (Time, error) {
	alloc, err := coreAlloc()
	if err != nil {
		return Time{}, err
	}
	f, r, st := native.TimeParse(alloc, text)
	if err := refused(TimeForm, r, st); err != nil {
		return Time{}, err
	}
	return timeOf(f), nil
}

// MarshalText is the canonical text, HH:MM:SS and a fraction when there
// is one.
func (t Time) MarshalText() ([]byte, error) {
	f, err := t.fields()
	if err != nil {
		return nil, err
	}
	text, err := textOf(TimeForm, func(alloc, out unsafe.Pointer) uint32 { return native.TimeFormat(alloc, f, out) })
	return []byte(text), err
}

// UnmarshalText reads the time's text.
func (t *Time) UnmarshalText(text []byte) error {
	parsed, err := ParseTime(string(text))
	if err == nil {
		*t = parsed
	}
	return err
}

func (t Time) String() string { return stringOf(t.MarshalText()) }

// Compare is -1, 0 or +1 as t is before, the same as, or after o.
func (t Time) Compare(o Time) int {
	switch {
	case t.Hour != o.Hour:
		return sign(t.Hour - o.Hour)
	case t.Minute != o.Minute:
		return sign(t.Minute - o.Minute)
	case t.Second != o.Second:
		return sign(t.Second - o.Second)
	}
	return sign(t.Nanosecond - o.Nanosecond)
}

// GuatiaoSchema is the form's schema.
func (Time) GuatiaoSchema() map[string]any {
	return map[string]any{"type": "string", "pattern": `^([01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9](\.[0-9]{1,9})?$`}
}

// DateTime is a civil date and time.
type DateTime struct {
	Date Date
	Time Time
}

func (d DateTime) fields() (native.DateTime, error) {
	date, err := d.Date.fields()
	if err != nil {
		return native.DateTime{}, err
	}
	t, err := d.Time.fields()
	if err != nil {
		return native.DateTime{}, err
	}
	return native.DateTime{Date: date, Time: t}, nil
}

// ParseDateTime reads text as a civil date and time.
func ParseDateTime(text string) (DateTime, error) {
	alloc, err := coreAlloc()
	if err != nil {
		return DateTime{}, err
	}
	f, r, st := native.DateTimeParse(alloc, text)
	if err := refused(DateTimeForm, r, st); err != nil {
		return DateTime{}, err
	}
	return DateTime{Date: dateOf(f.Date), Time: timeOf(f.Time)}, nil
}

// MarshalText is the canonical text: the date, T, and the time.
func (d DateTime) MarshalText() ([]byte, error) {
	f, err := d.fields()
	if err != nil {
		return nil, err
	}
	text, err := textOf(DateTimeForm, func(alloc, out unsafe.Pointer) uint32 { return native.DateTimeFormat(alloc, f, out) })
	return []byte(text), err
}

// UnmarshalText reads the text.
func (d *DateTime) UnmarshalText(text []byte) error {
	parsed, err := ParseDateTime(string(text))
	if err == nil {
		*d = parsed
	}
	return err
}

func (d DateTime) String() string { return stringOf(d.MarshalText()) }

// Add is the date and time moved by span, carried across midnight.
func (d DateTime) Add(span Span) (DateTime, error) {
	f, err := d.fields()
	if err != nil {
		return DateTime{}, err
	}
	if _, err := coreAlloc(); err != nil {
		return DateTime{}, err
	}
	out, st := native.DateTimeAddSpan(f, span.fields())
	if err := guatiao.StatusError(st, ""); err != nil {
		return DateTime{}, err
	}
	return DateTime{Date: dateOf(out.Date), Time: timeOf(out.Time)}, nil
}

// Compare is -1, 0 or +1 as d is before, the same as, or after o.
func (d DateTime) Compare(o DateTime) int {
	if c := d.Date.Compare(o.Date); c != 0 {
		return c
	}
	return d.Time.Compare(o.Time)
}

// GuatiaoSchema is the form's schema.
func (DateTime) GuatiaoSchema() map[string]any {
	return map[string]any{"type": "string",
		"pattern": `^[0-9]{4}-(0[1-9]|1[0-2])-(0[1-9]|[12][0-9]|3[01])[Tt]([01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9](\.[0-9]{1,9})?$`}
}

// ---- instants --------------------------------------------------------------------

// ParseTimestamp reads an instant's text, with any offset, as the
// instant it names, in UTC, to the nanosecond.
func ParseTimestamp(text string) (time.Time, error) {
	alloc, err := coreAlloc()
	if err != nil {
		return time.Time{}, err
	}
	ts, r, st := native.TimestampParse(alloc, text)
	if err := refused(TimestampForm, r, st); err != nil {
		return time.Time{}, err
	}
	return time.Unix(ts.Seconds, int64(ts.Nanoseconds)).UTC(), nil
}

// FormatTimestamp is the instant's canonical text, in UTC with a Z;
// guatiao.ErrBadValue outside the years 0 to 9999.
func FormatTimestamp(t time.Time) (string, error) {
	ts := native.Timestamp{Seconds: t.Unix(), Nanoseconds: uint32(t.Nanosecond())}
	return textOf(TimestampForm, func(alloc, out unsafe.Pointer) uint32 { return native.TimestampFormat(alloc, ts, out) })
}

// ---- exact lengths -------------------------------------------------------------

// Length is an exact length of time: whole seconds, then the nanoseconds
// below a second. It is written [seconds, nanoseconds], and read from that,
// a number of seconds, or {"secs", "nanos"}.
type Length struct {
	Seconds     uint64
	Nanoseconds uint32
}

// ReadLength reads an exact length from a value, a view or plain data, in
// any of its three spellings.
func ReadLength(x any) (Length, error) {
	var l Length
	err := guatiao.WithNodes([]any{x}, func(nodes []unsafe.Pointer) error {
		secs, nanos, st := native.DurationRead(nodes[0])
		if st == native.ErrBadValue {
			return &guatiao.Error{Status: guatiao.BadValue,
				Message: "not an exact length: [seconds, nanoseconds], a number of seconds, or {secs, nanos}"}
		}
		l = Length{Seconds: secs, Nanoseconds: nanos}
		return guatiao.StatusError(st, "")
	})
	return l, err
}

// LengthOf is d as an exact length; guatiao.ErrBadValue for a negative d.
func LengthOf(d time.Duration) (Length, error) {
	if d < 0 {
		return Length{}, &guatiao.Error{Status: guatiao.BadValue, Message: "an exact length is never negative"}
	}
	return Length{Seconds: uint64(d / time.Second), Nanoseconds: uint32(d % time.Second)}, nil
}

// Duration is the length as a time.Duration; guatiao.ErrBadValue past
// what one holds (about 292 years).
func (l Length) Duration() (time.Duration, error) {
	const most = uint64(math.MaxInt64)
	if l.Nanoseconds > 999_999_999 || l.Seconds > most/uint64(time.Second) ||
		l.Seconds*uint64(time.Second) > most-uint64(l.Nanoseconds) {
		return 0, &guatiao.Error{Status: guatiao.BadValue, Message: "the length is longer than a time.Duration holds"}
	}
	return time.Duration(l.Seconds*uint64(time.Second) + uint64(l.Nanoseconds)), nil
}

// MarshalGuatiao is the length as [seconds, nanoseconds].
func (l Length) MarshalGuatiao() (any, error) {
	if l.Nanoseconds > 999_999_999 {
		return nil, &guatiao.Error{Status: guatiao.BadValue, Message: "nanoseconds are 0 to 999999999"}
	}
	return [2]uint64{l.Seconds, uint64(l.Nanoseconds)}, nil
}

// UnmarshalGuatiao reads the length in any of its three spellings.
func (l *Length) UnmarshalGuatiao(r guatiao.Ref) error {
	got, err := ReadLength(r)
	if err == nil {
		*l = got
	}
	return err
}

// GuatiaoSchema is the form's schema.
func (Length) GuatiaoSchema() map[string]any {
	return map[string]any{
		"type": "array",
		"prefixItems": []any{
			map[string]any{"type": "integer", "minimum": 0, "maximum": guatiao.Number("18446744073709551615")},
			map[string]any{"type": "integer", "minimum": 0, "maximum": 999999999},
		},
		"items": false, "minItems": 2, "maxItems": 2,
	}
}

// ---- spans ---------------------------------------------------------------------

// Span is a length in calendar units with one sign for the whole: every
// non-zero field carries it, each at most 9223372036854775807 in
// magnitude, Nanoseconds below a second. The zero Span is PT0S. Two spans
// are equal unit by unit, which == compares.
type Span struct {
	Years, Months, Weeks, Days, Hours, Minutes, Seconds int64
	Nanoseconds                                         int32
}

func (s Span) fields() native.Span {
	return native.Span{Years: s.Years, Months: s.Months, Weeks: s.Weeks, Days: s.Days, Hours: s.Hours,
		Minutes: s.Minutes, Seconds: s.Seconds, Nanoseconds: s.Nanoseconds}
}

// ParseSpan reads ISO 8601 (P1DT2H) or the friendly form (1d 2h).
func ParseSpan(text string) (Span, error) {
	alloc, err := coreAlloc()
	if err != nil {
		return Span{}, err
	}
	f, r, st := native.SpanParse(alloc, text)
	if err := refused(SpanForm, r, st); err != nil {
		return Span{}, err
	}
	return Span{Years: f.Years, Months: f.Months, Weeks: f.Weeks, Days: f.Days, Hours: f.Hours,
		Minutes: f.Minutes, Seconds: f.Seconds, Nanoseconds: f.Nanoseconds}, nil
}

// MarshalText is the span's canonical ISO 8601 text; guatiao.ErrBadValue
// for fields of both signs or a unit out of range.
func (s Span) MarshalText() ([]byte, error) {
	f := s.fields()
	text, err := textOf(SpanForm, func(alloc, out unsafe.Pointer) uint32 { return native.SpanFormat(alloc, f, out) })
	return []byte(text), err
}

// UnmarshalText reads either form.
func (s *Span) UnmarshalText(text []byte) error {
	parsed, err := ParseSpan(string(text))
	if err == nil {
		*s = parsed
	}
	return err
}

func (s Span) String() string { return stringOf(s.MarshalText()) }

// Negated is the span with its sign turned.
func (s Span) Negated() Span {
	return Span{Years: -s.Years, Months: -s.Months, Weeks: -s.Weeks, Days: -s.Days, Hours: -s.Hours,
		Minutes: -s.Minutes, Seconds: -s.Seconds, Nanoseconds: -s.Nanoseconds}
}

// IsZero reports whether every unit is zero.
func (s Span) IsZero() bool { return s == Span{} }

// IsNegative reports whether the span's sign is negative.
func (s Span) IsNegative() bool {
	for _, x := range []int64{s.Years, s.Months, s.Weeks, s.Days, s.Hours, s.Minutes, s.Seconds, int64(s.Nanoseconds)} {
		if x != 0 {
			return x < 0
		}
	}
	return false
}

// GuatiaoSchema is the form's schema.
func (Span) GuatiaoSchema() map[string]any {
	return map[string]any{"type": "string", "format": "duration"}
}
