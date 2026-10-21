// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guatiao_test

import (
	"os"
	"regexp"
	"strconv"
	"strings"
	"testing"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go"
)

// enumMembers are the members of a C enum in the header: name to value.
func enumMembers(t *testing.T, enum string) map[string]int {
	t.Helper()
	text, err := os.ReadFile("internal/native/include/guatiao.h")
	if err != nil {
		t.Fatal(err)
	}
	body := string(text)
	start := strings.Index(body, "enum "+enum+"\n")
	if start < 0 {
		t.Fatalf("no enum %s in the header", enum)
	}
	body = body[start:]
	body = body[:strings.Index(body, "};")]
	out := map[string]int{}
	for _, m := range regexp.MustCompile(`(?m)^\s*(GUATIAO_[A-Z_]+) = (\d+),`).FindAllStringSubmatch(body, -1) {
		n, _ := strconv.Atoi(m[2])
		out[m[1]] = n
	}
	return out
}

func TestEveryStatusTheHeaderDeclaresHasAName(t *testing.T) {
	members := enumMembers(t, "guatiao_status_code")
	if len(members) < 10 {
		t.Fatalf("read %d statuses from the header", len(members))
	}
	for name, code := range members {
		if got := guatiao.Status(code).String(); strings.HasPrefix(got, "status ") {
			t.Errorf("%s (%d) has no name here", name, code)
		}
	}
	for _, s := range []guatiao.Status{guatiao.OK, guatiao.BadValue, guatiao.AllocFailed, guatiao.WrongKind,
		guatiao.NotFound, guatiao.NullArgument, guatiao.Gone, guatiao.Unsupported, guatiao.Busy, guatiao.Internal} {
		found := false
		for _, code := range members {
			found = found || code == int(s)
		}
		if !found {
			t.Errorf("%v (%d) is not in the header", s, uint32(s))
		}
	}
}

func TestEveryTagTheHeaderDeclaresHasAName(t *testing.T) {
	members := enumMembers(t, "guatiao_tag")
	if len(members) != 8 {
		t.Fatalf("read %d tags from the header", len(members))
	}
	for name, code := range members {
		if got := guatiao.Tag(code).String(); strings.HasPrefix(got, "tag ") || "GUATIAO_"+strings.ToUpper(got) != name {
			t.Errorf("%s (%d) reads as %q", name, code, got)
		}
	}
}
