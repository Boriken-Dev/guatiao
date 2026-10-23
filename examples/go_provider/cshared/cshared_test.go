// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package main

// The Go greeters as an ordinary library file: built with
// -buildmode=c-shared, found by a scan of the Rust example host
// (greeter_host, from the workspace's build), and called there as the
// greeter kind's Rust trait.

import (
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

func TestARustHostScansAndCallsTheGoLibrary(t *testing.T) {
	repo, _ := filepath.Abs(filepath.Join("..", "..", ".."))
	// The library passes on as named; the host sits in its directory, or
	// beside the one file it names.
	library := os.Getenv("GUATIAO_LIBRARY")
	if library == "" {
		library = filepath.Join(repo, "target", "debug")
	}
	workspace := library
	if info, err := os.Stat(library); err == nil && !info.IsDir() {
		workspace = filepath.Dir(library)
	}
	host := filepath.Join(workspace, "greeter_host")
	if runtime.GOOS == "windows" {
		host += ".exe"
	}
	if _, err := os.Stat(host); err != nil {
		t.Skipf("no Rust host at %s: build the workspace first", host)
	}
	name := map[string]string{"windows": "go_greeter.dll", "darwin": "libgo_greeter.dylib"}[runtime.GOOS]
	if name == "" {
		name = "libgo_greeter.so"
	}
	dir := t.TempDir()
	build := exec.Command("go", "build", "-buildmode=c-shared", "-o", filepath.Join(dir, name), ".")
	if out, err := build.CombinedOutput(); err != nil {
		t.Fatalf("go build -buildmode=c-shared: %v\n%s", err, out)
	}
	run := exec.Command(host)
	run.Env = append(os.Environ(), "GREETER_HOST_PATH="+dir, "GUATIAO_LIBRARY="+library)
	out, err := run.CombinedOutput()
	text := string(out)
	if err != nil {
		t.Fatalf("the host failed: %v\n%s", err, text)
	}
	for _, want := range []string{
		"loaded (1):",
		"go_greeter_hello [greeter, counter] from github.com/Boriken-Dev/guatiao/examples/go_provider/go_greeter",
		"go_greeter_hello greets: hello, ana",
		"go_greeter_hello shouts: HELLO, ANA",
		"go_greeter_hello checked a configuration, added [name, greeting], usable: true",
		"go_greeter_hello greets from it: hello, world",
		"go_greeter_hello has been asked 1 time(s)",
		`go_greeter_shouter configured with ShoutSettings { prefix: "hey" } greets: hey ANA`,
		"go_greeter_shouter given nothing: the configuration does not fit",
		"go_greeter_hello held a conversation of 2 turn(s)",
		`the listener heard ["good morning", "how are you"] and was dropped with the conversation: true`,
	} {
		if !strings.Contains(text, want) {
			t.Errorf("no %q in\n%s", want, text)
		}
	}
}
