// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Package testenv is what the binding's tests share: where the repository
// is, and the workspace build they run against.
package testenv

import (
	"os"
	"path/filepath"
)

// Repo is the repository's root: the first directory above this one that
// holds crates/guatiao/Cargo.toml, or "" outside a checkout.
func Repo() string {
	dir, err := os.Getwd()
	if err != nil {
		return ""
	}
	for {
		if _, err := os.Stat(filepath.Join(dir, "crates", "guatiao", "Cargo.toml")); err == nil {
			return dir
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			return ""
		}
		dir = parent
	}
}

// Setup points GUATIAO_LIBRARY at the workspace's debug build when it is
// unset, as the other bindings' suites do.
func Setup() {
	if os.Getenv("GUATIAO_LIBRARY") != "" {
		return
	}
	if repo := Repo(); repo != "" {
		target := os.Getenv("CARGO_TARGET_DIR")
		if target == "" {
			target = filepath.Join(repo, "target")
		}
		os.Setenv("GUATIAO_LIBRARY", filepath.Join(target, "debug"))
	}
}
