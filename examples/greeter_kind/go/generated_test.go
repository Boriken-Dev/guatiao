// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package greeterkind

import (
	"os/exec"
	"testing"
)

// The glue is what guatiao-kind writes from the kinds' description and the
// interfaces as they are now: a kind that changed shape, or an interface
// that did, fails here rather than in a call.
func TestTheGlueIsCurrent(t *testing.T) {
	out, err := exec.Command("go", "run", "github.com/Boriken-Dev/guatiao/bindings/go/guatiao/cmd/guatiao-kind",
		"-description", "../include/greeter_kind.kinds.json", "-check").CombinedOutput()
	if err != nil {
		t.Fatalf("%v\n%s", err, out)
	}
}
