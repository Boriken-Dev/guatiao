// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package main

import (
	"fmt"
	"regexp"
	"strconv"
	"strings"
)

// cTable is one kind table as a C header declares it.
type cTable struct {
	name      string
	floorHash uint32
	object    bool
	slots     []cSlot
}

// cSlot is one function pointer field of a table.
type cSlot struct {
	name   string
	ret    string
	params []cParam
	// lent are the arguments the field's comment calls lent.
	lent map[string]bool
}

type cParam struct {
	typ, name string
}

var lentRe = regexp.MustCompile("Lent for the call, not handed over: ([^.]*)\\.")
var tickRe = regexp.MustCompile("`([A-Za-z_][A-Za-z0-9_]*)`")

// readTable reads the table named name, and its floor hash, from a header
// rendered the way the guatiao kinds' headers are: a typedef'd struct of a
// guatiao_kind_header and function pointer fields, each optionally after a
// comment, then `#define <name>_FLOOR_HASH <n>`.
func readTable(header, name string) (*cTable, error) {
	start := strings.Index(header, "typedef struct "+name+" {")
	if start < 0 {
		return nil, fmt.Errorf("the header declares no table %s", name)
	}
	body := header[start+len("typedef struct "+name+" {"):]
	end := strings.Index(body, "} "+name+";")
	if end < 0 {
		return nil, fmt.Errorf("the table %s does not end as the header generator ends one", name)
	}
	body = body[:end]
	hash := regexp.MustCompile(`#define ` + regexp.QuoteMeta(name) + `_FLOOR_HASH (\d+)`).FindStringSubmatch(header)
	if hash == nil {
		return nil, fmt.Errorf("the header defines no %s_FLOOR_HASH", name)
	}
	h, err := strconv.ParseUint(hash[1], 10, 32)
	if err != nil {
		return nil, fmt.Errorf("%s_FLOOR_HASH: %w", name, err)
	}
	t := &cTable{name: name, floorHash: uint32(h)}
	fields, err := splitFields(body)
	if err != nil {
		return nil, fmt.Errorf("%s: %w", name, err)
	}
	if len(fields) == 0 || squash(fields[0].text) != "guatiao_kind_header header" {
		return nil, fmt.Errorf("%s does not start with a guatiao_kind_header", name)
	}
	for _, f := range fields[1:] {
		slot, err := readSlot(f.text)
		if err != nil {
			return nil, fmt.Errorf("%s: %w", name, err)
		}
		slot.lent = map[string]bool{}
		if m := lentRe.FindStringSubmatch(f.comment); m != nil {
			for _, arg := range tickRe.FindAllStringSubmatch(m[1], -1) {
				slot.lent[arg[1]] = true
			}
		}
		t.slots = append(t.slots, slot)
	}
	if len(t.slots) > 0 && t.slots[0].name == "destroy" {
		d := t.slots[0]
		if d.ret != "void" || len(d.params) != 1 {
			return nil, fmt.Errorf("%s.destroy is not void (*destroy)(void *ctx)", name)
		}
		t.object = true
		t.slots = t.slots[1:]
	}
	// A header is no description the library could read: this is checked
	// here, as #[guatiao::kind] and the library's reader check it.
	for _, s := range t.slots {
		for _, prm := range s.params {
			if prm.typ == "guatiao_bytes_mut" && !t.object {
				return nil, fmt.Errorf("%s.%s(%s): bytes written in place cross on an object kind only", name, s.name, prm.name)
			}
		}
	}
	return t, nil
}

type field struct {
	text, comment string
}

// splitFields splits a struct body into its fields, each with the comment
// before it.
func splitFields(body string) ([]field, error) {
	var out []field
	var comment strings.Builder
	var text strings.Builder
	for i := 0; i < len(body); i++ {
		switch {
		case strings.HasPrefix(body[i:], "/*"):
			end := strings.Index(body[i+2:], "*/")
			if end < 0 {
				return nil, fmt.Errorf("an unterminated comment")
			}
			comment.WriteString(body[i+2 : i+2+end])
			i += end + 3
		case strings.HasPrefix(body[i:], "//"):
			end := strings.IndexByte(body[i:], '\n')
			if end < 0 {
				end = len(body) - i
			}
			comment.WriteString(body[i+2 : i+end])
			i += end
		case body[i] == ';':
			out = append(out, field{text: text.String(), comment: comment.String()})
			text.Reset()
			comment.Reset()
		default:
			text.WriteByte(body[i])
		}
	}
	if strings.TrimSpace(text.String()) != "" {
		return nil, fmt.Errorf("a field with no semicolon: %q", strings.TrimSpace(text.String()))
	}
	return out, nil
}

func squash(s string) string { return strings.Join(strings.Fields(s), " ") }

var slotRe = regexp.MustCompile(`^(.*?)\(\s*\*\s*([A-Za-z_][A-Za-z0-9_]*)\s*\)\s*\((.*)\)$`)

func readSlot(text string) (cSlot, error) {
	m := slotRe.FindStringSubmatch(squash(text))
	if m == nil {
		return cSlot{}, fmt.Errorf("%q is not a function pointer field", squash(text))
	}
	s := cSlot{name: m[2], ret: strings.TrimSpace(m[1])}
	for _, p := range strings.Split(m[3], ",") {
		p = strings.TrimSpace(p)
		cut := strings.LastIndexFunc(p, func(r rune) bool { return r == ' ' || r == '*' })
		if cut < 0 {
			return cSlot{}, fmt.Errorf("%s: the argument %q has no name", s.name, p)
		}
		s.params = append(s.params, cParam{typ: squash(strings.ReplaceAll(p[:cut+1], "*", " * ")), name: p[cut+1:]})
	}
	if len(s.params) == 0 || s.params[0].typ != "void *" {
		return cSlot{}, fmt.Errorf("%s does not take void *ctx first", s.name)
	}
	return s, nil
}
