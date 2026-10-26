// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package main

// 64-bit integers cross a slot exactly: through a provider written in
// Rust, C or Go, and from Rust into the Go one. Go's int64 and uint64 are
// the slot's own types, so a value that does not fit cannot be written.

import (
	"errors"
	"math"
	"path/filepath"
	"reflect"
	"testing"
	"unsafe"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"
	greeterkind "github.com/Boriken-Dev/guatiao/examples/greeter_kind/go"
	"github.com/Boriken-Dev/guatiao/examples/internal/rustcaller"
)

const twoPow53Plus1 = 1<<53 + 1

var (
	signedLimits   = []int64{twoPow53Plus1, math.MinInt64, math.MaxInt64}
	unsignedLimits = []uint64{twoPow53Plus1, 1 << 63, math.MaxUint64}
)

// echo answers what it is handed, and refuses 7.
type echo struct{}

var _ greeterkind.Wide = echo{}

func (echo) EchoI64(x int64) (int64, error) {
	if x == 7 {
		return 0, guatiao.Errorf(guatiao.BadValue, "seven is no number")
	}
	return x, nil
}

func (echo) EchoU64(x uint64) (uint64, error) {
	if x == 7 {
		return 0, guatiao.Errorf(guatiao.BadValue, "seven is no number")
	}
	return x, nil
}

var wideProviders = map[string]string{
	"rust": "derived_greeter_hello",
	"c":    "c_greeter",
	"go":   "go_wide_echo",
}

var wideLib *guatiao.Library

// registerWide registers the library of the provider written here.
func registerWide() error {
	var err error
	wideLib, err = guatiao.LibraryOf(
		guatiao.Single("echo", echo{}, greeterkind.WideKind),
		guatiao.LibraryID("go_wide"),
		guatiao.LibraryVersion("0.1.0"),
	)
	if err != nil {
		return err
	}
	_, err = reg.RegisterLibrary(wideLib)
	return err
}

func wideEach(t *testing.T, body func(t *testing.T, which string, w *greeterkind.WideRemote)) {
	for _, which := range []string{"rust", "c", "go"} {
		t.Run(which, func(t *testing.T) {
			if why := present[which]; why != "" {
				t.Skip(why)
			}
			w, err := greeterkind.WideKind.Bind(reg, wideProviders[which])
			if err != nil {
				t.Fatal(err)
			}
			body(t, which, w)
		})
	}
}

func TestTheWideKindDescribedInGoIsTheHeaders(t *testing.T) {
	if got := greeterkind.WideKind.TableSize(); got != 8+2*unsafe.Sizeof(uintptr(0)) {
		t.Fatalf("the wide table is %d bytes", got)
	}
	if got := greeterkind.WideKind.FloorHash(); got != 3673689684 {
		t.Fatalf("the wide floor hash is %d", got)
	}
}

func TestEverySlotCarries64BitIntegersExactly(t *testing.T) {
	wideEach(t, func(t *testing.T, which string, w *greeterkind.WideRemote) {
		for _, x := range signedLimits {
			if got, err := w.EchoI64(x); err != nil || got != x {
				t.Fatalf("echo_i64(%d) = %d, %v", x, got, err)
			}
		}
		for _, x := range unsignedLimits {
			if got, err := w.EchoU64(x); err != nil || got != x {
				t.Fatalf("echo_u64(%d) = %d, %v", x, got, err)
			}
		}
	})
}

func TestAGoProviderRefusesWithItsOwnWords(t *testing.T) {
	w, err := greeterkind.WideKind.Bind(reg, wideProviders["go"])
	if err != nil {
		t.Fatal(err)
	}
	if _, err := w.EchoI64(7); !errors.Is(err, guatiao.ErrBadValue) {
		t.Fatalf("%v", err)
	}
	if _, err := w.EchoU64(7); !errors.Is(err, guatiao.ErrBadValue) {
		t.Fatalf("%v", err)
	}
}

func TestRustCallsEveryWideProviderExactly(t *testing.T) {
	caller, err := rustcaller.Open(filepath.Join(workspace(), libName("greeter_caller")))
	if err != nil {
		t.Fatal(err)
	}
	wideEach(t, func(t *testing.T, which string, _ *greeterkind.WideRemote) {
		report, err := caller.RunWide(reg.Host(), wideProviders[which])
		if err != nil {
			t.Fatal(err)
		}
		defer report.Close()
		got, _ := report.Any()
		r := got.(map[string]any)
		wantSigned := []any{guatiao.Number("9007199254740993"), guatiao.Number("-9223372036854775808"), guatiao.Number("9223372036854775807")}
		wantUnsigned := []any{guatiao.Number("9007199254740993"), guatiao.Number("9223372036854775808"), guatiao.Number("18446744073709551615")}
		if !reflect.DeepEqual(r["i64"], wantSigned) || !reflect.DeepEqual(r["u64"], wantUnsigned) {
			t.Fatalf("%v", r)
		}
	})
}
