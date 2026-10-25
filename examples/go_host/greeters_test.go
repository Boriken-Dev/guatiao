// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package main

// One behaviour, four languages: a Go host calls the greeters written in
// Rust, C, Python and Go through one kind, and a Rust caller calls each of
// them through the kind's Rust proxy.

import (
	"bytes"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"runtime"
	"strings"
	"testing"
	"time"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"
	gogreeter "github.com/Boriken-Dev/guatiao/examples/go_provider/go_greeter"
	greeterkind "github.com/Boriken-Dev/guatiao/examples/greeter_kind/go"
	"github.com/Boriken-Dev/guatiao/examples/internal/rustcaller"
	"github.com/Boriken-Dev/guatiao/examples/internal/threads"
	"github.com/Boriken-Dev/guatiao/examples/internal/valuegreeter"
)

// repo is the repository's root, two above this package.
var repo, _ = filepath.Abs(filepath.Join("..", ".."))

// workspace is where the workspace's libraries are: GUATIAO_LIBRARY when it
// names a directory, the directory of the file it names, else the
// workspace's debug build.
func workspace() string {
	if named := os.Getenv("GUATIAO_LIBRARY"); named != "" {
		if info, err := os.Stat(named); err == nil && !info.IsDir() {
			return filepath.Dir(named)
		}
		return named
	}
	return filepath.Join(repo, "target", "debug")
}

func libName(base string) string {
	switch runtime.GOOS {
	case "windows":
		return base + ".dll"
	case "darwin":
		return "lib" + base + ".dylib"
	}
	return "lib" + base + ".so"
}

type greeter struct {
	key, greeting string
}

var greeters = map[string]greeter{
	"rust":   {"derived_greeter_hello", "hello, ana"},
	"c":      {"c_greeter", "hello from C, ana"},
	"python": {"py_greeter_hello", "hello, ana"},
	"go":     {"go_greeter_hello", "hello, ana"},
}

var (
	reg     *guatiao.Registry
	present = map[string]string{} // which greeter -> "" when present, else why not
	goLib   *guatiao.Library
	// Where the greeter written in C was built, removed after the run.
	cGreeterDir string
)

// compiler is the C compiler cgo uses.
func compiler() string {
	if cc := os.Getenv("CC"); cc != "" {
		return cc
	}
	out, err := exec.Command("go", "env", "CC").Output()
	if err != nil {
		return ""
	}
	return strings.TrimSpace(string(out))
}

// buildCGreeter builds the greeter written in C into a directory of its
// own, with -D for each of defines.
func buildCGreeter(dir string, defines ...string) error {
	cc := compiler()
	if cc == "" {
		return errors.New("no C compiler")
	}
	args := []string{"-shared", "-std=c11", "-Wall", "-Wextra", "-Werror"}
	for _, d := range defines {
		args = append(args, "-D"+d)
	}
	if runtime.GOOS != "windows" {
		args = append(args, "-fPIC")
	}
	args = append(args, "-I", filepath.Join(repo, "crates", "guatiao", "include"),
		"-I", filepath.Join(repo, "examples", "greeter_kind", "include"),
		"-o", filepath.Join(dir, libName("c_greeter")),
		filepath.Join(repo, "crates", "guatiao", "tests", "c_consumer", "greeter_in_c.c"))
	if out, err := exec.Command(cc, args...).CombinedOutput(); err != nil {
		return fmt.Errorf("%s: %v\n%s", cc, err, out)
	}
	return nil
}

// bridge is the guatiao-source-py build: GUATIAO_PY_BRIDGE, else its own debug
// build; "" when there is none.
func bridge() string {
	named := os.Getenv("GUATIAO_PY_BRIDGE")
	if named == "" {
		named = filepath.Join(repo, "crates", "guatiao-source-py", "target", "debug", libName("guatiao_source_py"))
	}
	if _, err := os.Stat(named); err != nil {
		return ""
	}
	return named
}

func bridgeConfig() map[string]any {
	return map[string]any{
		"dirs":  []string{filepath.Join(repo, "examples", "python_provider")},
		"paths": []string{filepath.Join(repo, "bindings", "python", "guatiao", "src"), filepath.Join(repo, "examples", "greeter_kind", "python")},
	}
}

func TestMain(m *testing.M) {
	if os.Getenv("GUATIAO_LIBRARY") == "" {
		os.Setenv("GUATIAO_LIBRARY", workspace())
	}
	code, err := setup()
	if err != nil {
		fmt.Fprintln(os.Stderr, "setup:", err)
		os.Exit(1)
	}
	if code == 0 {
		code = m.Run()
	}
	reg.Close()
	goLib.Close()
	if wideLib != nil {
		wideLib.Close()
	}
	// Windows refuses to delete a library this process still maps.
	if cGreeterDir != "" && runtime.GOOS != "windows" {
		os.RemoveAll(cGreeterDir)
	}
	os.Exit(code)
}

func setup() (int, error) {
	var err error
	if reg, err = guatiao.NewRegistry("go-greeters", "0.1"); err != nil {
		return 1, err
	}
	if _, err := reg.ScanDir(workspace(), guatiao.Rules("kind=greeter")); err != nil {
		return 1, err
	}
	cDir, err := os.MkdirTemp("", "guatiao-go-c-greeter-")
	if err != nil {
		return 1, err
	}
	cGreeterDir = cDir
	if err := buildCGreeter(cDir); err != nil {
		present["c"] = "the greeter written in C is not built: " + err.Error()
	} else if _, err := reg.ScanDir(cDir, guatiao.Rules("kind=greeter")); err != nil {
		return 1, err
	}
	if py := bridge(); py == "" {
		present["python"] = "no guatiao-source-py build: set GUATIAO_PY_BRIDGE"
	} else {
		if err := reg.AddSourceFile(py, bridgeConfig()); err != nil {
			if os.Getenv("GUATIAO_PY_BRIDGE") != "" {
				return 1, err
			}
			// A build found by its default path that cannot be mapped here
			// (its Python not on the search path) is not the one asked for.
			present["python"] = "the guatiao-source-py build found by default does not load: " + err.Error()
		}
	}
	if present["python"] == "" {
		changed, err := reg.Refresh()
		if err != nil {
			return 1, err
		}
		if refused, _ := changed["refused"].([]any); len(refused) > 0 {
			return 1, fmt.Errorf("the bridge refused %v", refused)
		}
	}
	if goLib, err = gogreeter.Library(); err != nil {
		return 1, err
	}
	if _, err := reg.RegisterLibrary(goLib); err != nil {
		return 1, err
	}
	if err := registerWide(); err != nil {
		return 1, err
	}
	return 0, nil
}

func each(t *testing.T, body func(t *testing.T, which string, g *greeterkind.GreeterRemote)) {
	for _, which := range []string{"rust", "c", "python", "go"} {
		t.Run(which, func(t *testing.T) {
			if why := present[which]; why != "" {
				t.Skip(why)
			}
			g, err := greeterkind.GreeterKind.Bind(reg, greeters[which].key)
			if err != nil {
				t.Fatal(err)
			}
			body(t, which, g)
		})
	}
}

type tape struct {
	heard  []string
	closed int
}

func (e *tape) Heard(what string) error { e.heard = append(e.heard, what); return nil }
func (e *tape) Close() error            { e.closed++; return nil }

func TestTheKindDescribedInGoIsTheHeaders(t *testing.T) {
	header, err := os.ReadFile(filepath.Join(repo, "examples", "greeter_kind", "include", "greeter_kind.h"))
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(header, []byte(fmt.Sprintf("#define greeter_vtable_FLOOR_HASH %d", greeterkind.GreeterKind.FloorHash()))) {
		t.Fatal("the floor hash is not the header's")
	}
	if got := greeterkind.GreeterKind.TableSize(); got != 8+6*8 {
		t.Fatalf("the greeter table is %d bytes", got)
	}
}

func TestEveryGreeterGreets(t *testing.T) {
	each(t, func(t *testing.T, which string, g *greeterkind.GreeterRemote) {
		answer, err := g.Greet("ana")
		if err != nil || answer.Greeting != greeters[which].greeting {
			t.Fatalf("%q %v", answer.Greeting, err)
		}
	})
}

func TestEveryGreeterRefusesWithItsOwnWords(t *testing.T) {
	each(t, func(t *testing.T, which string, g *greeterkind.GreeterRemote) {
		_, err := g.Greet("")
		if !errors.Is(err, guatiao.ErrBadValue) || !strings.Contains(err.Error(), "nobody to greet") {
			t.Fatalf("%v", err)
		}
	})
}

func TestAFailureInsideTheMethodIsInternal(t *testing.T) {
	each(t, func(t *testing.T, which string, g *greeterkind.GreeterRemote) {
		_, err := g.Greet("panic")
		if which == "c" {
			// The greeter written in C greets anyone, panic included.
			if err != nil {
				t.Fatal(err)
			}
			return
		}
		if !errors.Is(err, guatiao.ErrInternal) {
			t.Fatalf("%v", err)
		}
	})
}

func TestEveryGreeterFillsALentMapInPlace(t *testing.T) {
	each(t, func(t *testing.T, which string, g *greeterkind.GreeterRemote) {
		config, err := guatiao.NewValue(map[string]any{"name": "bo"})
		if err != nil {
			t.Fatal(err)
		}
		defer config.Close()
		m, _ := config.Map()
		usable, err := g.Check(m)
		if err != nil || !usable {
			t.Fatalf("%v %v", usable, err)
		}
		if m.Key("name").StringOr("") != "bo" || m.Len() != 2 {
			t.Fatalf("%v", m.Keys())
		}
	})
}

// ownGreeter scans the greeter written in C, built with defines in a
// directory of its own, into a registry of its own: no other registry
// keeps it mapped when a test unloads it.
func ownGreeter(t *testing.T, defines ...string) (*guatiao.Registry, *greeterkind.GreeterRemote) {
	t.Helper()
	if why := present["c"]; why != "" {
		t.Skip(why)
	}
	dir := t.TempDir()
	if err := buildCGreeter(dir, defines...); err != nil {
		t.Fatal(err)
	}
	own, err := guatiao.NewRegistry("go-greeters-own", "0.1")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { own.Close() })
	if _, err := own.ScanDir(dir, guatiao.Rules("kind=greeter")); err != nil {
		t.Fatal(err)
	}
	g, err := greeterkind.GreeterKind.Bind(own, "c_greeter")
	if err != nil {
		t.Fatal(err)
	}
	return own, g
}

func TestAnAnswerBuiltInTheLibrarysAllocatorOutlivesTheUnload(t *testing.T) {
	// This greeter ignores the carrier; a *Value answer is moved home on
	// arrival, or closing it after the unload calls into unmapped code.
	own, g := ownGreeter(t, "GREETER_IGNORES_CARRIER")
	answer, err := valuegreeter.Greet(g.Remote, "hal")
	if err != nil {
		t.Fatal(err)
	}
	// The C greeter has no unload function: nothing it handed out is
	// alive, which the caller states.
	if err := own.UnloadUnchecked("c_greeter_library"); err != nil {
		t.Fatal(err)
	}
	if libraries, err := own.Libraries(); err != nil || len(libraries) != 0 {
		t.Fatalf("%v %v", libraries, err)
	}
	var got greeterkind.Greeting
	if err := answer.Decode(&got); err != nil || got.Greeting != "hello from C, hal" {
		t.Fatalf("%q %v", got.Greeting, err)
	}
	answer.Close()
}

func TestWhatASlotGrewALentMapWithOutlivesTheUnload(t *testing.T) {
	// This greeter's `check` rebuilds the lent map in its own allocator;
	// the map is settled into the one it recorded before the call.
	own, g := ownGreeter(t, "GREETER_IGNORES_CARRIER")
	config, err := guatiao.NewValue(map[string]any{"name": "ivy"})
	if err != nil {
		t.Fatal(err)
	}
	defer config.Close()
	m, _ := config.Map()
	if usable, err := g.Check(m); err != nil || !usable {
		t.Fatalf("%v %v", usable, err)
	}
	if err := own.UnloadUnchecked("c_greeter_library"); err != nil {
		t.Fatal(err)
	}
	var got map[string]any
	if err := config.Decode(&got); err != nil || !reflect.DeepEqual(got, map[string]any{"name": "ivy", "checked_by": "C"}) {
		t.Fatalf("%v %v", got, err)
	}
}

func TestEveryGreeterGreetsFromAMapHandedOver(t *testing.T) {
	each(t, func(t *testing.T, which string, g *greeterkind.GreeterRemote) {
		answer, err := g.GreetFrom(greeterkind.Named{Name: "ana"})
		if err != nil || answer.Greeting != greeters[which].greeting {
			t.Fatalf("%q %v", answer.Greeting, err)
		}
	})
}

func TestEveryGreeterTellsALentListener(t *testing.T) {
	each(t, func(t *testing.T, which string, g *greeterkind.GreeterRemote) {
		before := guatiao.LiveObjects()
		e := &tape{}
		if err := g.GreetInto("ana", e); err != nil {
			t.Fatal(err)
		}
		if !reflect.DeepEqual(e.heard, []string{greeters[which].greeting}) || e.closed != 0 {
			t.Fatalf("%v %d", e.heard, e.closed)
		}
		if guatiao.LiveObjects() != before {
			t.Fatalf("a lent listener is let go when the call returns")
		}
	})
}

func TestEveryGreeterHoldsAConversation(t *testing.T) {
	each(t, func(t *testing.T, which string, g *greeterkind.GreeterRemote) {
		before := guatiao.LiveObjects()
		e := &tape{}
		if which == "c" {
			if g.Has("start") {
				t.Fatal("the C greeter has no start")
			}
			if _, err := g.Start(e); !errors.Is(err, guatiao.ErrUnsupported) {
				t.Fatalf("%v", err)
			}
			return
		}
		conversation, err := g.Start(e)
		if err != nil {
			t.Fatal(err)
		}
		for _, line := range []string{"hi", "again"} {
			if err := conversation.Say(line); err != nil {
				t.Fatal(err)
			}
		}
		if turns, err := conversation.Turns(); err != nil || turns != 2 {
			t.Fatalf("%d %v", turns, err)
		}
		transcript := make([]byte, 16)
		n, err := conversation.Read(transcript)
		if err != nil || string(transcript[:n]) != "hi\nagain" {
			t.Fatalf("%q %v", transcript[:n], err)
		}
		if err := conversation.Say(""); !errors.Is(err, guatiao.ErrBadValue) {
			t.Fatalf("%v", err)
		}
		conversation.Close()
		if !reflect.DeepEqual(e.heard, []string{"hi", "again"}) {
			t.Fatalf("%v", e.heard)
		}
		if _, err := conversation.Turns(); !errors.Is(err, guatiao.ErrClosed) {
			t.Fatalf("a closed conversation: %v", err)
		}
		if e.closed != 1 || guatiao.LiveObjects() != before {
			t.Fatalf("the listener a closed conversation held: closed %d, live %d (was %d)", e.closed, guatiao.LiveObjects(), before)
		}
	})
}

func TestAProviderServesTwoKinds(t *testing.T) {
	counter, err := greeterkind.CounterKind.Bind(reg, "go_greeter_hello")
	if err != nil {
		t.Fatal(err)
	}
	first, err := counter.Count()
	if err != nil {
		t.Fatal(err)
	}
	if next, _ := counter.Count(); next != first+1 {
		t.Fatalf("%d then %d", first, next)
	}
	info, err := reg.Provider("go_greeter_hello")
	if err != nil {
		t.Fatal(err)
	}
	if kinds := info["kinds"]; !reflect.DeepEqual(kinds, []any{"greeter", "counter"}) {
		t.Fatalf("%v", kinds)
	}
}

func TestAConfiguredGoProvider(t *testing.T) {
	config, ok, err := reg.ProviderConfig("go_greeter_shouter")
	if err != nil || !ok {
		t.Fatalf("%v %v", ok, err)
	}
	if config.Key("properties").Key("prefix").Key("minLength").IntOr(0) != 1 {
		t.Fatal("the schema the struct's tags give")
	}
	if _, err := reg.Create("go_greeter_shouter", map[string]any{"prefix": 5}); !errors.Is(err, guatiao.ErrBadValue) {
		t.Fatalf("a configuration the schema refuses: %v", err)
	}
	inst, err := reg.Create("go_greeter_shouter", map[string]any{"prefix": "hey"})
	if err != nil {
		t.Fatal(err)
	}
	if goLib.LiveInstances() != 1 {
		t.Fatalf("%d instances", goLib.LiveInstances())
	}
	g, err := greeterkind.GreeterKind.BindInstance(inst)
	if err != nil {
		t.Fatal(err)
	}
	if answer, err := g.Greet("ana"); err != nil || answer.Greeting != "hey ANA" {
		t.Fatalf("%v %v", answer, err)
	}
	if g.Has("shout") {
		t.Fatal("a Shouter has no shout")
	}
	if _, err := g.Shout("ana"); !errors.Is(err, guatiao.ErrUnsupported) {
		t.Fatalf("%v", err)
	}
	inst.Close()
	if goLib.LiveInstances() != 0 {
		t.Fatalf("%d instances after close", goLib.LiveInstances())
	}
	if _, err := greeterkind.GreeterKind.BindInstance(inst); !errors.Is(err, guatiao.ErrClosed) {
		t.Fatalf("%v", err)
	}
}

func TestRustCallsEveryGreeterThroughItsProxy(t *testing.T) {
	caller, err := rustcaller.Open(filepath.Join(workspace(), libName("greeter_caller")))
	if err != nil {
		t.Fatal(err)
	}
	each(t, func(t *testing.T, which string, _ *greeterkind.GreeterRemote) {
		report, err := caller.Run(reg.Host(), greeters[which].key, "ana")
		if err != nil {
			t.Fatal(err)
		}
		defer report.Close()
		got, _ := report.Any()
		r := got.(map[string]any)
		greeting := greeters[which].greeting
		if r["greet"] != greeting || r["shout"] != strings.ToUpper(greeting) || r["checked"] != true || r["greet_from"] != greeting {
			t.Fatalf("%v", r)
		}
		refusal, _ := r["refusal"].(map[string]any)
		if refusal["status"] != guatiao.Number("7") || !strings.Contains(fmt.Sprint(refusal["message"]), "nobody to greet") {
			t.Fatalf("refusal %v", refusal)
		}
		failure := "100"
		if which == "c" {
			failure = "0"
		}
		if r["failure"] != guatiao.Number(failure) {
			t.Fatalf("failure %v", r["failure"])
		}
		if !reflect.DeepEqual(r["heard"], []any{greeting}) {
			t.Fatalf("heard %v", r["heard"])
		}
		conversation, _ := r["conversation"].(map[string]any)
		if which == "c" {
			if conversation["status"] != guatiao.Number("11") {
				t.Fatalf("%v", conversation)
			}
			return
		}
		if conversation["turns"] != guatiao.Number("2") || conversation["transcript"] != "hi\nagain" ||
			!reflect.DeepEqual(conversation["heard"], []any{"hi", "again"}) {
			t.Fatalf("%v", conversation)
		}
		if conversation["dropped"] != true {
			t.Fatalf("the listener a dropped conversation held: %v", conversation)
		}
	})
}

func TestTheHostProgramRuns(t *testing.T) {
	var out bytes.Buffer
	var pyDirs, pyPaths []string
	py := bridge()
	if present["python"] != "" {
		py = ""
	}
	if py != "" {
		config := bridgeConfig()
		pyDirs, pyPaths = config["dirs"].([]string), config["paths"].([]string)
	}
	if err := run(&out, []string{workspace()}, py, pyDirs, pyPaths, true); err != nil {
		t.Fatalf("%v\n%s", err, out.String())
	}
	text := out.String()
	for _, want := range []string{
		"loaded " + libName("derived_greeter"),
		"go_greeter_hello (github.com/Boriken-Dev/guatiao/examples/go_provider/go_greeter 1.0.0)",
		`conversation: 2 turns, transcript "hi\nagain", listener heard ["hi" "again"]`,
		"go_greeter_shouter",
		`greet: "hey ANA"`,
		"not this greeter kind",
		"objects a library can still reach: 0",
	} {
		if !strings.Contains(text, want) {
			t.Errorf("no %q in\n%s", want, text)
		}
	}
}

func TestAGoGreeterAnswersNativeThreads(t *testing.T) {
	g, err := greeterkind.GreeterKind.Bind(reg, "go_greeter_hello")
	if err != nil {
		t.Fatal(err)
	}
	table := g.Table()
	library := filepath.Join(workspace(), libName("guatiao"))
	start := time.Now()
	const threadCount, calls = 8, 2000
	bad, err := threads.Greet(library, table.Pointer, table.Ctx, threadCount, calls, len("hello, ana"))
	if err != nil {
		t.Fatal(err)
	}
	if bad != 0 {
		t.Fatalf("%d of %d calls from native threads answered wrong", bad, threadCount*calls)
	}
	t.Logf("%d calls from %d native threads: %v a call", threadCount*calls, threadCount, time.Since(start)/(threadCount*calls))
}

func TestAGoGreeterAnswersInTheCallersAllocator(t *testing.T) {
	g, err := greeterkind.GreeterKind.Bind(reg, "go_greeter_hello")
	if err != nil {
		t.Fatal(err)
	}
	table := g.Table()
	home, live, err := threads.GreetInCarrier(filepath.Join(workspace(), libName("guatiao")), table.Pointer, table.Ctx)
	if err != nil {
		t.Fatal(err)
	}
	if !home || live != 0 {
		t.Fatalf("the answer was built in the carrier's allocator: %v, blocks still out after freeing it: %d", home, live)
	}
}
