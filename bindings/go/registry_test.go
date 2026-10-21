// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guatiao_test

// Registry against a real --all-features build: scanning, answers,
// instances, retire, collect and unload, and sources.

import (
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"slices"
	"strings"
	"testing"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go"
	"github.com/Boriken-Dev/guatiao/bindings/go/internal/testenv"
)

func registry(t *testing.T) *guatiao.Registry {
	t.Helper()
	reg, err := guatiao.NewRegistry("go-tests", "0.1")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { reg.Close() })
	return reg
}

func scanned(t *testing.T) *guatiao.Registry {
	t.Helper()
	reg := registry(t)
	if _, err := reg.ScanDir(testenv.Workspace(), guatiao.Rules("kind=greeter")); err != nil {
		t.Fatal(err)
	}
	return reg
}

func ids(list []map[string]any) []string {
	var out []string
	for _, p := range list {
		out = append(out, p["id"].(string))
	}
	return out
}

func TestScansByRuleAndListsTheProviders(t *testing.T) {
	reg := registry(t)
	report, err := reg.ScanDir(testenv.Workspace(), guatiao.Rules("kind=greeter"))
	if err != nil {
		t.Fatal(err)
	}
	loaded := toText(report["loaded"])
	for _, want := range []string{"derived_greeter", "hello_library"} {
		if !strings.Contains(loaded, want) {
			t.Errorf("no %s in %v", want, report["loaded"])
		}
	}
	providers, err := reg.Providers("greeter")
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{"derived_greeter_hello", "hello_library_greeter"} {
		if !slices.Contains(ids(providers), want) {
			t.Errorf("no %s in %v", want, ids(providers))
		}
	}
}

func toText(x any) string {
	var b strings.Builder
	for _, item := range x.([]any) {
		b.WriteString(" " + item.(string))
	}
	return b.String()
}

func TestAnswersTheDocumentedFields(t *testing.T) {
	reg := scanned(t)
	best, err := reg.Best("greeter")
	if err != nil {
		t.Fatal(err)
	}
	for _, field := range []string{"key", "id", "version", "library", "display_name", "from", "kinds", "has_config", "vtable_size"} {
		if _, ok := best[field]; !ok {
			t.Errorf("no %s in %v", field, best)
		}
	}
	why, err := reg.WhyNot("nothing-claims-this-kind")
	if err != nil || why["available"] != false || why["why"] != "nothing-claims-it" {
		t.Fatalf("%v %v", why, err)
	}
	if _, err := reg.Provider("nonesuch"); !errors.Is(err, guatiao.ErrNotFound) {
		t.Fatalf("%v", err)
	}
	ok, why2, err := reg.ProviderAvailable("derived_greeter_hello")
	if err != nil || !ok || why2 != "" {
		t.Fatalf("%v %q %v", ok, why2, err)
	}
	libraries, err := reg.Libraries()
	if err != nil || len(libraries) < 2 {
		t.Fatalf("%v %v", libraries, err)
	}
}

func TestAnswersASingleTableAndATablePerKind(t *testing.T) {
	reg := scanned(t)
	single, err := reg.ProviderTable("hello_library_greeter", "")
	if err != nil || single.Pointer == nil || single.Size == 0 {
		t.Fatalf("%v %v", single, err)
	}
	if _, err := reg.ProviderTable("derived_greeter_hello", ""); !errors.Is(err, guatiao.ErrNotFound) {
		t.Fatalf("a provider with a table per kind has no single one: %v", err)
	}
	if per, err := reg.ProviderTable("derived_greeter_hello", "greeter"); err != nil || per.Pointer == nil {
		t.Fatalf("%v %v", per, err)
	}
	if _, err := reg.ProviderTable("derived_greeter_hello", "codec"); !errors.Is(err, guatiao.ErrNotFound) {
		t.Fatalf("%v", err)
	}
}

func TestBuildsAnInstanceAndLendsItsSchema(t *testing.T) {
	reg := scanned(t)
	schema, ok, err := reg.ProviderConfig("derived_greeter_shouter")
	if err != nil || !ok || schema.Tag() != guatiao.TagMap {
		t.Fatalf("%v %v", ok, err)
	}
	m, _ := schema.Map()
	if err := m.Set("x", 1); !errors.Is(err, guatiao.ErrReadOnly) {
		t.Fatalf("%v", err)
	}
	if _, ok, _ := reg.ProviderConfig("derived_greeter_hello"); ok {
		t.Fatal("a provider with no configuration lends no schema")
	}
	inst, err := reg.Create("derived_greeter_shouter", map[string]any{"prefix": "hey"})
	if err != nil {
		t.Fatal(err)
	}
	if inst.Ctx() == 0 {
		t.Fatal("no instance")
	}
	inst.Close()
	inst.Close()
	if _, err := reg.Create("derived_greeter_shouter", map[string]any{"prefix": 1}); !errors.Is(err, guatiao.ErrBadValue) {
		t.Fatalf("a configuration the schema refuses: %v", err)
	}
	if _, err := reg.Create("derived_greeter_hello", map[string]any{}); !errors.Is(err, guatiao.ErrNullArgument) {
		t.Fatalf("a provider that builds no instances: %v", err)
	}
}

func TestKeysRanksAndReKeys(t *testing.T) {
	reg := scanned(t)
	if err := reg.SetPriority("derived_greeter_hello", 7); err != nil {
		t.Fatal(err)
	}
	if p, _ := reg.Priority("derived_greeter_hello"); p != 7 {
		t.Fatalf("%d", p)
	}
	if err := reg.KeyedBy("%id@%version"); err != nil {
		t.Fatal(err)
	}
	providers, _ := reg.Providers("greeter")
	var keys []string
	for _, p := range providers {
		keys = append(keys, p["key"].(string))
	}
	if !slices.ContainsFunc(keys, func(k string) bool { return strings.HasPrefix(k, "derived_greeter_hello@") }) {
		t.Fatalf("%v", keys)
	}
	if err := reg.KeyedBy("%nonsense"); !errors.Is(err, guatiao.ErrBadValue) {
		t.Fatalf("%v", err)
	}
}

func TestRetiresALibraryAndFreesItsKey(t *testing.T) {
	reg := scanned(t)
	if err := reg.Retire("derived_greeter"); err != nil {
		t.Fatal(err)
	}
	providers, _ := reg.Providers("greeter")
	if slices.Contains(ids(providers), "derived_greeter_hello") || !slices.Contains(ids(providers), "hello_library_greeter") {
		t.Fatalf("%v", ids(providers))
	}
	again, _ := reg.ScanDir(testenv.Workspace(), guatiao.Rules("kind=greeter"))
	if !strings.Contains(toText(again["loaded"]), "derived_greeter") {
		t.Fatalf("%v", again)
	}
	if err := reg.Retire("derived_greeter_hello"); !errors.Is(err, guatiao.ErrNotFound) {
		t.Fatalf("%v", err)
	}
}

func TestUnloadsOnlyWhenTheLibraryAgrees(t *testing.T) {
	reg := scanned(t)
	inst, err := reg.Create("derived_greeter_shouter", map[string]any{"prefix": "hey"})
	if err != nil {
		t.Fatal(err)
	}
	if err := reg.Unload("derived_greeter"); !errors.Is(err, guatiao.ErrBusy) {
		t.Fatalf("%v", err)
	}
	inst.Close()
	if err := reg.Unload("derived_greeter"); err != nil {
		t.Fatal(err)
	}
	if err := reg.Unload("derived_greeter"); !errors.Is(err, guatiao.ErrNotFound) {
		t.Fatalf("%v", err)
	}
}

func TestCollectsARetiredLibraryOnceNothingOfItIsAlive(t *testing.T) {
	reg := scanned(t)
	if got, err := reg.Collect(); err != nil || len(got) != 0 {
		t.Fatalf("%v %v", got, err)
	}
	inst, err := reg.Create("derived_greeter_shouter", map[string]any{"prefix": "hey"})
	if err != nil {
		t.Fatal(err)
	}
	if err := reg.Retire("derived_greeter"); err != nil {
		t.Fatal(err)
	}
	if got, _ := reg.Collect(); len(got) != 0 {
		t.Fatalf("collected while an instance lives: %v", got)
	}
	inst.Close()
	if got, _ := reg.Collect(); !reflect.DeepEqual(got, []string{"derived_greeter"}) {
		t.Fatalf("%v", got)
	}
}

func TestAClosedRegistryRefusesUse(t *testing.T) {
	reg, err := guatiao.NewRegistry("go-tests", "0.1")
	if err != nil {
		t.Fatal(err)
	}
	reg.Close()
	if _, err := reg.Libraries(); !errors.Is(err, guatiao.ErrClosed) {
		t.Fatalf("%v", err)
	}
	reg.Close()
}

func TestServesLibrariesThatComeAndGoFromASource(t *testing.T) {
	shelf, err := testenv.BuildC("source_in_c.c", mappedDir(t), "go_shelf_a")
	if err != nil {
		t.Skip("the source written in C is not built: " + err.Error())
	}
	controls, err := testenv.OpenC(shelf)
	if err != nil {
		t.Fatal(err)
	}
	reg := registry(t)
	if err := reg.AddSourceFile(shelf, map[string]any{"ids": []string{"shelf_a", "shelf_b"}}); err != nil {
		t.Fatal(err)
	}
	if libs, _ := reg.Libraries(); len(libs) != 0 {
		t.Fatalf("a source serves nothing before a refresh: %v", libs)
	}
	first, err := reg.Refresh()
	if err != nil {
		t.Fatal(err)
	}
	if got := servedIDs(first["loaded"]); !reflect.DeepEqual(got, []string{"shelf_a", "shelf_b"}) {
		t.Fatalf("%v", first)
	}
	if again, _ := reg.Refresh(); len(again["loaded"].([]any))+len(again["retired"].([]any)) != 0 {
		t.Fatalf("%v", again)
	}
	controls.VoidInt("shelf_take", 1)
	changed, _ := reg.Refresh()
	if got := servedIDs(changed["retired"]); !reflect.DeepEqual(got, []string{"shelf_b"}) {
		t.Fatalf("%v", changed)
	}
	reg.Collect()
	if controls.IntInt("shelf_closed", 1) != 1 {
		t.Fatal("the source was not told")
	}
	controls.VoidBool("shelf_refuse", true)
	controls.VoidInt("shelf_put", 2)
	refused, _ := reg.Refresh()
	list := refused["refused"].([]any)
	if len(list) != 1 || list[0].(map[string]any)["why"] != "the shelf refuses, says C" {
		t.Fatalf("%v", refused)
	}
}

// mappedDir is a directory for a library this process maps, which stays
// mapped, so it is not removed after the test: Windows refuses to delete a
// mapped image.
func mappedDir(t *testing.T) string {
	t.Helper()
	dir, err := os.MkdirTemp("", "guatiao-go-test-")
	if err != nil {
		t.Fatal(err)
	}
	return dir
}

func servedIDs(x any) []string {
	var out []string
	for _, item := range x.([]any) {
		out = append(out, item.(map[string]any)["id"].(string))
	}
	return out
}

func TestASourceRefusesWithItsOwnWords(t *testing.T) {
	shelf, err := testenv.BuildC("source_in_c.c", mappedDir(t), "go_shelf_b")
	if err != nil {
		t.Skip("the source written in C is not built: " + err.Error())
	}
	reg := registry(t)
	if err := reg.AddSourceFile(shelf, nil); err == nil || !strings.Contains(err.Error(), "`ids`") {
		t.Fatalf("%v", err)
	}
	if err := reg.AddSourceFile(shelf, map[string]any{"ids": []string{}}); err != nil {
		t.Fatal(err)
	}
	if err := reg.AddSourceFile(shelf, map[string]any{"ids": []string{}}); err == nil || !strings.Contains(err.Error(), "already added") {
		t.Fatalf("%v", err)
	}
	if err := reg.AddSourceFile(filepath.Join(testenv.Workspace(), testenv.Filename("derived_greeter")), nil); !errors.Is(err, guatiao.ErrWrongKind) {
		t.Fatalf("%v", err)
	}
	if err := reg.AddSourceFile(filepath.Join(testenv.Workspace(), "no-such-file"), nil); !errors.Is(err, guatiao.ErrNotFound) {
		t.Fatalf("%v", err)
	}
}

func TestFollowsTheFilesOfAWatchedDirectory(t *testing.T) {
	dir := mappedDir(t)
	watched, aside := filepath.Join(dir, "watched"), filepath.Join(dir, "aside")
	os.Mkdir(watched, 0o755)
	os.Mkdir(aside, 0o755)
	reg := registry(t)
	if err := reg.WatchDir(watched, "kind=greeter"); err != nil {
		t.Fatal(err)
	}
	if err := reg.WatchDir(watched); !errors.Is(err, guatiao.ErrBadValue) {
		t.Fatalf("%v", err)
	}
	if first, _ := reg.Refresh(); len(first["loaded"].([]any)) != 0 {
		t.Fatalf("%v", first)
	}
	data, err := os.ReadFile(filepath.Join(testenv.Workspace(), testenv.Filename("hello_library")))
	if err != nil {
		t.Fatal(err)
	}
	copied := filepath.Join(watched, testenv.Filename("hello_go_watched"))
	if err := os.WriteFile(copied, data, 0o755); err != nil {
		t.Fatal(err)
	}
	changed, _ := reg.Refresh()
	if keys := keysOf(changed["loaded"]); !reflect.DeepEqual(keys, []string{"hello_library"}) {
		t.Fatalf("%v", changed)
	}
	// A mapped image cannot be deleted on Windows; moved aside, it is gone.
	if err := os.Rename(copied, filepath.Join(aside, filepath.Base(copied))); err != nil {
		t.Fatal(err)
	}
	changed, _ = reg.Refresh()
	if keys := keysOf(changed["retired"]); !reflect.DeepEqual(keys, []string{"hello_library"}) {
		t.Fatalf("%v", changed)
	}
}

func keysOf(x any) []string {
	var out []string
	for _, item := range x.([]any) {
		out = append(out, item.(map[string]any)["key"].(string))
	}
	return out
}
