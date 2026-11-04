// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guatiao_test

// Registry against a real --all-features build: scanning, answers,
// instances, retire, collect and unload, and sources.

import (
	"crypto/sha256"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"runtime"
	"slices"
	"strings"
	"testing"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"
	"github.com/Boriken-Dev/guatiao/bindings/go/guatiao/internal/testenv"
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

func TestAProvidersMetaIsACopyCarryingItsForm(t *testing.T) {
	reg := scanned(t)
	meta, ok, err := reg.ProviderMeta("derived_greeter_shouter")
	if err != nil || !ok {
		t.Fatalf("%v %v", ok, err)
	}
	reg.Close()
	var form struct {
		Sections []struct {
			ID string `guatiao:"id"`
		} `guatiao:"sections"`
	}
	if err := meta.Key("form").Decode(&form); err != nil {
		t.Fatal(err)
	}
	meta.Close()
	if len(form.Sections) != 2 || form.Sections[0].ID != "words" || form.Sections[1].ID != "volume" {
		t.Fatalf("%+v", form)
	}
	reg = scanned(t)
	if _, ok, err := reg.ProviderMeta("derived_greeter_hello"); ok || err != nil {
		t.Fatalf("a provider declaring nothing answered meta: %v %v", ok, err)
	}
	if _, _, err := reg.ProviderMeta("nobody"); !errors.Is(err, guatiao.ErrNotFound) {
		t.Fatalf("%v", err)
	}
}

func TestAProviderCarriesItsKindsDescriptions(t *testing.T) {
	reg := scanned(t)
	document, ok, err := reg.KindDescriptions("derived_greeter_hello")
	if err != nil || !ok {
		t.Fatalf("%v %v", ok, err)
	}
	defer document.Close()
	var read struct {
		Kinds []struct {
			Name      string `guatiao:"name"`
			FloorHash uint32 `guatiao:"floor_hash"`
		} `guatiao:"kinds"`
	}
	if err := document.Decode(&read); err != nil {
		t.Fatal(err)
	}
	var names []string
	for _, k := range read.Kinds {
		names = append(names, k.Name)
	}
	if strings.Join(names, ",") != "greeter,listener,conversation,counter,wide" {
		t.Fatalf("%+v", read)
	}
	if _, ok, err := reg.KindDescriptions("hello_library_greeter"); ok || err != nil {
		t.Fatalf("a provider written by hand carries none: %v %v", ok, err)
	}
	if _, _, err := reg.KindDescriptions("nobody"); !errors.Is(err, guatiao.ErrNotFound) {
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

func TestTheImportSearchIsTheLibrarysUntilTheHostNamesPath(t *testing.T) {
	reg, err := guatiao.NewRegistry("go-tests", "0.1")
	if err != nil {
		t.Fatal(err)
	}
	defer reg.Close()
	if s, err := reg.ImportSearch(); err != nil || s != guatiao.ImportSearchLibrary {
		t.Fatalf("the default: %v %v", s, err)
	}
	if err := reg.SetImportSearch(guatiao.ImportSearchPath); err != nil {
		t.Fatal(err)
	}
	if s, _ := reg.ImportSearch(); s != guatiao.ImportSearchPath {
		t.Fatalf("after path: %v", s)
	}
	if err := reg.SetImportSearch(2); !errors.Is(err, guatiao.ErrBadValue) {
		t.Fatalf("an unknown search: %v", err)
	}
	if s, _ := reg.ImportSearch(); s != guatiao.ImportSearchPath {
		t.Fatalf("a refusal changed it: %v", s)
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

// catalogLibrary is the C library of four providers a settings screen
// lists, built into a directory of its own.
func catalogLibrary(t *testing.T) (dir, file string) {
	t.Helper()
	dir = mappedDir(t)
	file, err := testenv.BuildC("catalog_library.c", dir, "go_catalog_library")
	if err != nil {
		t.Skip("the catalog library written in C is not built: " + err.Error())
	}
	return dir, file
}

func TestTheCatalogListsEveryProviderOfADirectoryAndBuildsNone(t *testing.T) {
	dir, file := catalogLibrary(t)
	reg := registry(t)
	report, err := reg.ScanDir(dir)
	if err != nil {
		t.Fatal(err)
	}
	if loaded, _ := report["loaded"].([]any); len(loaded) != 1 {
		t.Fatalf("%v", report)
	}
	list, err := reg.Catalog()
	if err != nil {
		t.Fatal(err)
	}
	catalog := map[string]map[string]any{}
	for _, entry := range list {
		catalog[entry["id"].(string)] = entry
	}
	want := []string{"catalog_configured", "catalog_implied", "catalog_plain", "catalog_unavailable"}
	if got := ids(list); !reflect.DeepEqual(sorted(got), want) {
		t.Fatalf("%v", got)
	}
	bytes, err := os.ReadFile(file)
	if err != nil {
		t.Fatal(err)
	}
	digest := fmt.Sprintf("sha256:%x", sha256.Sum256(bytes))

	configured := catalog["catalog_configured"]
	if !reflect.DeepEqual(configured["kinds"], []any{"counter"}) ||
		configured["display_name"] != "Configured, with a form" ||
		configured["version"] != "2.0" ||
		configured["library"] != "catalog_library" ||
		configured["library_version"] != "1.0" ||
		configured["digest"] != digest ||
		configured["available"] != true {
		t.Fatalf("%v", configured)
	}
	if _, has := configured["reason"]; has {
		t.Fatalf("%v", configured)
	}
	if !reflect.DeepEqual(configured["config"].(map[string]any)["required"], []any{"host", "port"}) {
		t.Fatalf("%v", configured["config"])
	}
	form := configured["meta"].(map[string]any)["form"].(map[string]any)
	if !reflect.DeepEqual(form["sections"], []any{map[string]any{"id": "net", "title": "Network"}}) {
		t.Fatalf("%v", form)
	}

	implied := catalog["catalog_implied"]
	if _, has := implied["meta"]; has || implied["version"] != "1.0" {
		t.Fatalf("%v", implied)
	}
	token := implied["config"].(map[string]any)["properties"].(map[string]any)["token"].(map[string]any)
	if token["x-sensitive"] != true {
		t.Fatalf("%v", token)
	}

	unavailable := catalog["catalog_unavailable"]
	if _, has := unavailable["config"]; has || unavailable["available"] != false ||
		unavailable["reason"] != "no device here" || unavailable["has_config"] != false {
		t.Fatalf("%v", unavailable)
	}
	if catalog["catalog_plain"]["available"] != true {
		t.Fatalf("%v", catalog["catalog_plain"])
	}

	// Listing asked the library for nothing: its count says so, and so does
	// its refusal of the first instance asked for afterwards.
	counted, err := testenv.OpenC(file)
	if err != nil {
		t.Fatal(err)
	}
	if built := counted.IntVoid("catalog_built"); built != 0 {
		t.Fatalf("listing built %d", built)
	}
	_, err = reg.Create("catalog_configured", map[string]any{"host": "localhost", "port": 8080})
	if err == nil || !strings.Contains(err.Error(), "built nothing: the first create this library was asked") {
		t.Fatalf("%v", err)
	}
	if built := counted.IntVoid("catalog_built"); built != 1 {
		t.Fatalf("built %d", built)
	}
}

func TestACatalogNamesWhyALibraryHasNoDigest(t *testing.T) {
	dir, file := catalogLibrary(t)
	reg := registry(t)
	if _, err := reg.LoadFile(file); err != nil {
		t.Fatal(err)
	}
	// Another file at the path, before anything asked: renaming a mapped
	// file works on every platform, where overwriting one does not.
	if err := os.Rename(file, filepath.Join(dir, "moved_away.bin")); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(file, []byte("not the library"), 0o644); err != nil {
		t.Fatal(err)
	}
	list, err := reg.Catalog()
	if err != nil || len(list) != 4 {
		t.Fatalf("%v %v", list, err)
	}
	for _, entry := range list {
		if _, has := entry["digest"]; has || entry["digest_error"] != "the file changed since it was loaded" {
			t.Fatalf("%v", entry)
		}
	}
}

func sorted(in []string) []string {
	out := slices.Clone(in)
	slices.Sort(out)
	return out
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

func TestASourceNoteIsToldOnce(t *testing.T) {
	// The shelf written in C answers its configuration's `note` from its
	// table's `notes` slot.
	shelf, err := testenv.BuildC("source_in_c.c", mappedDir(t), "go_shelf_note")
	if err != nil {
		t.Skip("the source written in C is not built: " + err.Error())
	}
	reg := registry(t)
	if err := reg.AddSourceFile(shelf, map[string]any{"ids": []string{}, "note": "the shelf is dusty, says C"}); err != nil {
		t.Fatal(err)
	}
	first, err := reg.Refresh()
	if err != nil {
		t.Fatal(err)
	}
	want := []any{map[string]any{
		"source": shelf, "subject": "shelf", "text": "the shelf is dusty, says C",
		"detail": map[string]any{"said": "by C"},
	}}
	if !reflect.DeepEqual(first["notes"], want) {
		t.Fatalf("%#v", first["notes"])
	}
	if again, _ := reg.Refresh(); again["notes"] != nil {
		t.Fatalf("told again: %v", again)
	}
}

// mappedDir is a directory for a library this process maps, which stays
// mapped, so on Windows it is not removed after the test: Windows refuses
// to delete a mapped image.
func mappedDir(t *testing.T) string {
	t.Helper()
	dir, err := os.MkdirTemp("", "guatiao-go-test-")
	if err != nil {
		t.Fatal(err)
	}
	if runtime.GOOS != "windows" {
		t.Cleanup(func() { os.RemoveAll(dir) })
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
