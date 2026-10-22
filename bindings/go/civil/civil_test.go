// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package civil_test

// The civil package against the shared vectors every implementation
// reads, then the mappings to Go's own types and what they lose.

import (
	"bytes"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go"
	"github.com/Boriken-Dev/guatiao/bindings/go/civil"
	"github.com/Boriken-Dev/guatiao/bindings/go/internal/testenv"
)

func TestMain(m *testing.M) {
	testenv.Setup()
	os.Exit(m.Run())
}

// vectors are the shared vectors, every number kept as its text so a
// 20-digit one stays exact.
func vectors(t *testing.T) *guatiao.Value {
	t.Helper()
	repo := testenv.Repo()
	if repo == "" {
		t.Skip("the shared vectors live in the repository")
	}
	text, err := os.ReadFile(filepath.Join(repo, "crates", "guatiao", "tests", "vectors", "time.json"))
	if err != nil {
		t.Fatal(err)
	}
	dec := json.NewDecoder(bytes.NewReader(text))
	dec.UseNumber()
	var plain any
	if err := dec.Decode(&plain); err != nil {
		t.Fatal(err)
	}
	v, err := guatiao.NewValue(exact(plain))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { v.Close() })
	return v
}

// exact turns each json.Number into a guatiao.Number, which keeps its text.
func exact(x any) any {
	switch x := x.(type) {
	case json.Number:
		return guatiao.Number(x)
	case []any:
		for i := range x {
			x[i] = exact(x[i])
		}
	case map[string]any:
		for k := range x {
			x[k] = exact(x[k])
		}
	}
	return x
}

func cases(t *testing.T, v *guatiao.Value, name string) []guatiao.Ref {
	t.Helper()
	l, err := v.Key(name).List()
	if err != nil {
		t.Fatal(err)
	}
	var out []guatiao.Ref
	for _, c := range l.All() {
		out = append(out, c)
	}
	if len(out) == 0 {
		t.Fatalf("no %s vectors", name)
	}
	return out
}

// parsed reads text as form and answers its fields as the C struct holds
// them, and its canonical text.
func parsed(form civil.Form, text string) ([]int64, string, error) {
	switch form {
	case civil.DateForm:
		d, err := civil.ParseDate(text)
		if err != nil {
			return nil, "", err
		}
		b, err := d.MarshalText()
		return []int64{int64(d.Year), int64(d.Month), int64(d.Day)}, string(b), err
	case civil.TimeForm:
		x, err := civil.ParseTime(text)
		if err != nil {
			return nil, "", err
		}
		b, err := x.MarshalText()
		return []int64{int64(x.Hour), int64(x.Minute), int64(x.Second), int64(x.Nanosecond)}, string(b), err
	case civil.DateTimeForm:
		x, err := civil.ParseDateTime(text)
		if err != nil {
			return nil, "", err
		}
		b, err := x.MarshalText()
		return []int64{int64(x.Date.Year), int64(x.Date.Month), int64(x.Date.Day), int64(x.Time.Hour),
			int64(x.Time.Minute), int64(x.Time.Second), int64(x.Time.Nanosecond)}, string(b), err
	case civil.TimestampForm:
		x, err := civil.ParseTimestamp(text)
		if err != nil {
			return nil, "", err
		}
		s, err := civil.FormatTimestamp(x)
		return []int64{x.Unix(), int64(x.Nanosecond())}, s, err
	default:
		x, err := civil.ParseSpan(text)
		if err != nil {
			return nil, "", err
		}
		b, err := x.MarshalText()
		return []int64{x.Years, x.Months, x.Weeks, x.Days, x.Hours, x.Minutes, x.Seconds, int64(x.Nanoseconds)}, string(b), err
	}
}

func TestReadsAndWritesEveryForm(t *testing.T) {
	v := vectors(t)
	for _, form := range []civil.Form{civil.DateForm, civil.TimeForm, civil.DateTimeForm, civil.TimestampForm, civil.SpanForm} {
		for _, c := range cases(t, v, string(form)) {
			text := c.Key("text").StringOr("")
			if at := c.Key("refused_at"); !at.IsAbsent() {
				_, _, readErr := parsed(form, text)
				_, normErr := civil.Normalize(form, text)
				for _, err := range []error{readErr, normErr} {
					var u *civil.Unparsed
					if !errors.As(err, &u) || !errors.Is(err, guatiao.ErrBadValue) {
						t.Errorf("%s %q: %v", form, text, err)
						continue
					}
					if u.At != int(at.IntOr(-1)) || u.Form != form || u.Expected == "" {
						t.Errorf("%s %q: %#v", form, text, u)
					}
					if len(text) > 2 && strings.Contains(u.Error(), text) {
						t.Errorf("%s: the refusal quotes the text: %s", form, u)
					}
				}
				continue
			}
			var want []int64
			if err := c.Key("fields").Decode(&want); err != nil {
				t.Fatal(err)
			}
			fields, canonical, err := parsed(form, text)
			if err != nil {
				t.Errorf("%s %q: %v", form, text, err)
				continue
			}
			if !reflect.DeepEqual(fields, want) {
				t.Errorf("%s %q: fields %v, want %v", form, text, fields, want)
			}
			wantText := c.Key("canonical").StringOr("")
			if canonical != wantText {
				t.Errorf("%s %q: written %q, want %q", form, text, canonical, wantText)
			}
			if got, err := civil.Normalize(form, text); err != nil || got != wantText {
				t.Errorf("%s %q: normalized %q %v", form, text, got, err)
			}
		}
	}
}

func TestAppliesEverySpan(t *testing.T) {
	v := vectors(t)
	for _, kind := range []string{"date", "datetime"} {
		for _, c := range cases(t, v, kind+"_add_span") {
			start, spanText := c.Key("start").StringOr(""), c.Key("span").StringOr("")
			span, err := civil.ParseSpan(spanText)
			if err != nil {
				t.Fatal(err)
			}
			var got string
			if kind == "date" {
				d, _ := civil.ParseDate(start)
				var moved civil.Date
				if moved, err = d.Add(span); err == nil {
					got = moved.String()
				}
			} else {
				d, _ := civil.ParseDateTime(start)
				var moved civil.DateTime
				if moved, err = d.Add(span); err == nil {
					got = moved.String()
				}
			}
			if c.Key("result").IsNull() {
				if !errors.Is(err, guatiao.ErrBadValue) {
					t.Errorf("%s + %s: %q %v", start, spanText, got, err)
				}
			} else if want := c.Key("result").StringOr(""); err != nil || got != want {
				t.Errorf("%s + %s: %q %v, want %q", start, spanText, got, err, want)
			}
		}
	}
}

func TestReadsEveryExactLength(t *testing.T) {
	v := vectors(t)
	for _, c := range cases(t, v, "duration") {
		got, err := civil.ReadLength(c.Key("value"))
		if c.Key("duration").IsNull() {
			if !errors.Is(err, guatiao.ErrBadValue) {
				t.Errorf("%v: %v %v", plainOf(c.Key("value")), got, err)
			}
			continue
		}
		var want [2]uint64
		if err := c.Key("duration").Decode(&want); err != nil {
			t.Fatal(err)
		}
		if err != nil || got.Seconds != want[0] || uint64(got.Nanoseconds) != want[1] {
			t.Errorf("%v: %v %v, want %v", plainOf(c.Key("value")), got, err, want)
		}
	}
}

func plainOf(r guatiao.Ref) any {
	x, _ := r.Any()
	return x
}

func TestHoldsDatesAndTimesToTheNanosecond(t *testing.T) {
	d, err := civil.NewDate(0, time.January, 1)
	if err != nil || d.String() != "0000-01-01" {
		t.Fatalf("%v %v", d, err)
	}
	if _, err := civil.NewDate(2026, time.February, 29); !errors.Is(err, guatiao.ErrBadValue) {
		t.Fatalf("%v", err)
	}
	tm, err := civil.NewTime(13, 45, 0, 1)
	if err != nil || tm.String() != "13:45:00.000000001" {
		t.Fatalf("%v %v", tm, err)
	}
	if _, err := civil.NewTime(24, 0, 0, 0); !errors.Is(err, guatiao.ErrBadValue) {
		t.Fatalf("%v", err)
	}
	if _, err := civil.NewTime(300, 0, 0, 0); !errors.Is(err, guatiao.ErrBadValue) {
		t.Fatalf("a field past its C width: %v", err)
	}
	dt, _ := civil.ParseDateTime("2026-10-21T13:45:00.5")
	if dt.Date.Compare(d) != 1 || dt.Compare(dt) != 0 {
		t.Fatalf("Compare")
	}
}

func TestAnInstantIsATimeTime(t *testing.T) {
	at, err := civil.ParseTimestamp("1969-12-31T23:59:59.999999999Z")
	if err != nil || at.Unix() != -1 || at.Nanosecond() != 999999999 || at.Location() != time.UTC {
		t.Fatalf("%v %v", at, err)
	}
	text, err := civil.FormatTimestamp(time.Date(2026, 10, 21, 19, 15, 0, 0, time.FixedZone("", 5*3600+1800)))
	if err != nil || text != "2026-10-21T13:45:00Z" {
		t.Fatalf("%q %v", text, err)
	}
	if _, err := civil.FormatTimestamp(time.Date(10000, 1, 1, 0, 0, 0, 0, time.UTC)); !errors.Is(err, guatiao.ErrBadValue) {
		t.Fatalf("year 10000: %v", err)
	}
}

func TestAnExactLengthAndADuration(t *testing.T) {
	l, err := civil.LengthOf(90*time.Second + 5)
	if err != nil || l != (civil.Length{Seconds: 90, Nanoseconds: 5}) {
		t.Fatalf("%v %v", l, err)
	}
	if _, err := civil.LengthOf(-1); !errors.Is(err, guatiao.ErrBadValue) {
		t.Fatalf("%v", err)
	}
	if _, err := (civil.Length{Seconds: 1 << 63}).Duration(); !errors.Is(err, guatiao.ErrBadValue) {
		t.Fatalf("%v", err)
	}
	v, err := guatiao.NewValue(map[string]any{"len": civil.Length{Seconds: 18446744073709551615, Nanoseconds: 1}})
	if err != nil {
		t.Fatal(err)
	}
	defer v.Close()
	var back struct {
		Len civil.Length `guatiao:"len"`
	}
	if err := v.Decode(&back); err != nil || back.Len.Seconds != 18446744073709551615 {
		t.Fatalf("%v %v", back, err)
	}
}

func TestASpanIsItsOwnTypeBackedByTheLibrary(t *testing.T) {
	s, err := civil.ParseSpan("1y 2mo 3w 4d 5h 6m 7s 500ms 1us 1ns")
	if err != nil || s.String() != "P1Y2M3W4DT5H6M7.500001001S" {
		t.Fatalf("%v %v", s, err)
	}
	if n := s.Negated(); !n.IsNegative() || n.String() != "-P1Y2M3W4DT5H6M7.500001001S" {
		t.Fatalf("%v", n)
	}
	if !(civil.Span{}).IsZero() || (civil.Span{}).String() != "PT0S" {
		t.Fatal("zero")
	}
	if _, err := (civil.Span{Days: 1, Hours: -1}).MarshalText(); !errors.Is(err, guatiao.ErrBadValue) {
		t.Fatalf("mixed signs: %v", err)
	}
	day, _ := civil.ParseSpan("P1D")
	hours, _ := civil.ParseSpan("PT24H")
	if day == hours {
		t.Fatal("a day is not 24 hours")
	}
}

func TestTheCivilTypesCrossAsTheirText(t *testing.T) {
	d, _ := civil.ParseDate("2026-10-21")
	s, _ := civil.ParseSpan("PT1H30M")
	v, err := guatiao.NewValue(map[string]any{"d": d, "s": s})
	if err != nil {
		t.Fatal(err)
	}
	defer v.Close()
	if v.Key("d").StringOr("") != "2026-10-21" || v.Key("s").StringOr("") != "PT1H30M" {
		t.Fatalf("%v", plainOf(v.Ref))
	}
	var back struct {
		D civil.Date `guatiao:"d"`
		S civil.Span `guatiao:"s"`
	}
	if err := v.Decode(&back); err != nil || back.D != d || back.S != s {
		t.Fatalf("%v %v", back, err)
	}
	if _, err := civil.Normalize("week", "x"); err == nil {
		t.Fatal("an unknown form")
	}
}
