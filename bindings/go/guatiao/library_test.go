// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guatiao_test

// A library written in Go, over a kind described by hand rather than
// generated: what a descriptor says, how its providers are refused and
// built, and what LibraryOf reads from the build.

import (
	"errors"
	"reflect"
	"strings"
	"testing"
	"unsafe"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"
)

// pinger is the one slot of a kind no C function serves here: enough for
// a descriptor, which a host lists without calling.
type pinger interface{ Ping() error }

type probeRemote struct{ guatiao.Remote }

var probeKind = guatiao.NewKind(guatiao.KindSpec{
	Name:      "go-probe",
	FloorHash: 12345,
	Slots:     []guatiao.SlotSpec{{Name: "ping", Required: true, Iface: reflect.TypeFor[pinger]()}},
}, func(r guatiao.Remote) *probeRemote { return &probeRemote{Remote: r} })

type ping struct{ closed *int }

func (p *ping) Ping() error { return nil }
func (p *ping) Close() error {
	if p.closed != nil {
		*p.closed++
	}
	return nil
}

type probeConfig struct {
	Port uint16 `guatiao:"port" schema:"minimum=1"`
	Name string `guatiao:"name,omitempty"`
}

func TestALibraryWrittenInGoIsListedLikeAnyOther(t *testing.T) {
	closed := 0
	lib, err := guatiao.NewLibrary("go_probe_lib", "1.2.3",
		guatiao.Single("one", &ping{closed: &closed}, probeKind, guatiao.DisplayName("The one"),
			guatiao.ProviderMeta(map[string]any{"flavour": "plain"}),
			guatiao.AvailableIf(func() error { return errors.New("the device is unplugged") })),
		guatiao.Configured("built", func(c probeConfig) (*ping, error) {
			if c.Port == 13 {
				panic("unlucky")
			}
			if c.Port == 7 {
				return nil, guatiao.Errorf(guatiao.Busy, "port %d is taken", c.Port)
			}
			return &ping{}, nil
		}, probeKind),
		guatiao.Summary("a probe"),
	)
	if err != nil {
		t.Fatal(err)
	}
	if got := lib.Providers(); !reflect.DeepEqual(got, []string{"go_probe_lib_one", "go_probe_lib_built"}) {
		t.Fatalf("%v", got)
	}
	if got := lib.Kinds(); !reflect.DeepEqual(got, []string{"go-probe"}) {
		t.Fatalf("%v", got)
	}
	reg := registry(t)
	report, err := reg.RegisterLibrary(lib)
	if err != nil || report["loaded"] == nil {
		t.Fatalf("%v %v", report, err)
	}
	one, err := reg.Provider("go_probe_lib_one")
	if err != nil {
		t.Fatal(err)
	}
	if one["display_name"] != "The one" || one["version"] != "1.2.3" || one["library"] != "go_probe_lib" {
		t.Fatalf("%v", one)
	}
	ok, why, _ := reg.ProviderAvailable("go_probe_lib_one")
	if ok || why != "the device is unplugged" {
		t.Fatalf("%v %q", ok, why)
	}
	if lib.Meta()["summary"] != "a probe" {
		t.Fatalf("the library's meta: %v", lib.Meta())
	}

	// The table has the slot, but no C function serves it here: a remote
	// says so rather than calling a null pointer.
	remote, err := probeKind.Bind(reg, "go_probe_lib_one")
	if err != nil {
		t.Fatal(err)
	}
	if remote.Has("ping") {
		t.Fatal("ping is served by nothing")
	}
	if _, err := remote.Begin(0); !errors.Is(err, guatiao.ErrUnsupported) {
		t.Fatalf("%v", err)
	}

	// A configuration checked against the struct's schema, then decoded.
	schema, okSchema, _ := reg.ProviderConfig("go_probe_lib_built")
	if !okSchema || schema.Key("properties").Key("port").Key("minimum").IntOr(0) != 1 {
		t.Fatal("the schema the struct gives")
	}
	if _, err := reg.Create("go_probe_lib_built", map[string]any{"port": 0}); !errors.Is(err, guatiao.ErrBadValue) {
		t.Fatalf("below the minimum: %v", err)
	}
	if _, err := reg.Create("go_probe_lib_built", map[string]any{"port": 7}); !errors.Is(err, guatiao.ErrBusy) || !strings.Contains(err.Error(), "port 7 is taken") {
		t.Fatalf("a build that refuses: %v", err)
	}
	if _, err := reg.Create("go_probe_lib_built", map[string]any{"port": 13}); !errors.Is(err, guatiao.ErrInternal) || !strings.Contains(err.Error(), "unlucky") {
		t.Fatalf("a build that panics: %v", err)
	}
	inst, err := reg.Create("go_probe_lib_built", map[string]any{"port": 80})
	if err != nil {
		t.Fatal(err)
	}
	if lib.LiveInstances() != 1 {
		t.Fatalf("%d", lib.LiveInstances())
	}
	inst.Close()
	if lib.LiveInstances() != 0 {
		t.Fatalf("%d", lib.LiveInstances())
	}
	// Linked, not mapped.
	if err := reg.Unload("go_probe_lib"); !errors.Is(err, guatiao.ErrWrongKind) {
		t.Fatalf("%v", err)
	}
	reg.Close()
	lib.Close()
	if closed != 1 {
		t.Fatalf("Close of the library closes its providers' own instances: %d", closed)
	}
	if lib.Answer(nil) != nil {
		t.Fatal("a closed library declines")
	}
}

type notPinger struct{}

func TestALibraryRefusesWhatItCannotDescribe(t *testing.T) {
	cases := map[string][]guatiao.LibraryPart{
		"no provider": {},
		"no kind":     {guatiao.Single("p", &ping{})},
		"a value that does not implement its kind": {guatiao.Single("p", notPinger{}, probeKind)},
		"two providers sharing an id": {guatiao.Single("p", &ping{}, probeKind),
			guatiao.Single("q", &ping{}, probeKind, guatiao.ID("go_probe_p"))},
		"no instance": {guatiao.Single("p", nil, probeKind)},
	}
	for name, parts := range cases {
		if _, err := guatiao.NewLibrary("go_probe", "1.0.0", parts...); err == nil {
			t.Errorf("%s: built", name)
		}
	}
}

func TestLibraryOfReadsTheBuild(t *testing.T) {
	// A test binary records no version for its module, which is refused
	// rather than invented.
	_, err := guatiao.LibraryOf(guatiao.Single("p", &ping{}, probeKind))
	if err == nil || !strings.Contains(err.Error(), "names no version") {
		t.Fatalf("%v", err)
	}
	lib, err := guatiao.LibraryOf(guatiao.Single("p", &ping{}, probeKind), guatiao.LibraryVersion("2.0.0"))
	if err != nil {
		t.Fatal(err)
	}
	if lib.ID() != "github.com/Boriken-Dev/guatiao/bindings/go/guatiao_test" || lib.Version() != "2.0.0" {
		t.Fatalf("%s %s", lib.ID(), lib.Version())
	}
	if got := lib.Providers(); !reflect.DeepEqual(got, []string{"guatiao_test_p"}) {
		t.Fatalf("%v", got)
	}
	if lib.Meta()["module"] != "github.com/Boriken-Dev/guatiao/bindings/go/guatiao" {
		t.Fatalf("%v", lib.Meta())
	}
	if _, err := guatiao.LibraryOf(guatiao.Single("p", &ping{}, probeKind),
		guatiao.Single("q", &guatiao.Value{}, probeKind), guatiao.LibraryVersion("1.0.0")); err == nil {
		t.Fatal("providers from two packages")
	}
}

type listened struct {
	heard []string
}

func (l *listened) Ping() error { return nil }

func TestServingAnObjectNeedsItsRequiredSlots(t *testing.T) {
	objectKind := guatiao.NewKind(guatiao.KindSpec{
		Name: "go-probe-object", FloorHash: 1, Object: true,
		Slots: []guatiao.SlotSpec{{Name: "ping", Required: true, Iface: reflect.TypeFor[pinger]()}},
	}, func(r guatiao.Remote) guatiao.Remote { return r })
	if word := unsafe.Sizeof(uintptr(0)); objectKind.TableSize() != 8+2*word {
		t.Fatalf("an object kind's table: the header, destroy, ping: %d", objectKind.TableSize())
	}
	if _, err := guatiao.NewLibrary("go_probe", "1.0.0", guatiao.Single("p", &listened{}, objectKind)); err == nil {
		t.Fatal("a provider serving an object kind")
	}
}
