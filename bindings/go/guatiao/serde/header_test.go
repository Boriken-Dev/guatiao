// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package serde_test

import (
	"bytes"
	"os"
	"path/filepath"
	"testing"

	"github.com/Boriken-Dev/guatiao/bindings/go/guatiao/serde/internal/testenv"
)

// The module carries copies of the committed headers, since a module is
// the files under its directory; each must be the committed one.
func TestTheHeadersAreTheCommittedOnes(t *testing.T) {
	repo := testenv.Repo()
	if repo == "" {
		t.Skip("outside a checkout of the repository there is nothing to compare with")
	}
	for name, crate := range map[string]string{"guatiao.h": "guatiao", "guatiao_serde.h": "guatiao-serde"} {
		copied, err := os.ReadFile(filepath.Join("include", name))
		if err != nil {
			t.Fatal(err)
		}
		committed, err := os.ReadFile(filepath.Join(repo, "crates", crate, "include", name))
		if err != nil {
			t.Fatal(err)
		}
		if !bytes.Equal(copied, committed) {
			t.Errorf("include/%s differs from crates/%s/include/%s: copy it again", name, crate, name)
		}
	}
}
