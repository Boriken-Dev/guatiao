// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package main

import (
	"encoding/json"
	"fmt"
)

// The kinds document a kind crate commits beside its header
// (<crate>.kinds.json), or a provider carries: what a kind is, as data.
type kindsDocument struct {
	Format int               `json:"format"`
	Kinds  []kindDescription `json:"kinds"`
}

type kindDescription struct {
	Name      string            `json:"name"`
	FloorHash uint32            `json:"floor_hash"`
	Object    bool              `json:"object"`
	Slots     []slotDescription `json:"slots"`
}

type slotDescription struct {
	Name     string           `json:"name"`
	Fallible bool             `json:"fallible"`
	Args     []argDescription `json:"args"`
	Answer   *argDescription  `json:"answer"`
}

type argDescription struct {
	Name string `json:"name"`
	Type string `json:"type"`
	Pass string `json:"pass"`
	Kind string `json:"kind"`
}

// cScalars are the C types a description's scalar types are, as a kind
// header spells them.
var cScalars = map[string]string{
	"bool": "bool", "i8": "int8_t", "i16": "int16_t", "i32": "int32_t", "i64": "int64_t",
	"u8": "uint8_t", "u16": "uint16_t", "u32": "uint32_t", "u64": "uint64_t",
	"isize": "intptr_t", "usize": "size_t", "f32": "float", "f64": "double",
}

var cTrees = map[string]string{"value": "guatiao_value", "map": "guatiao_map", "list": "guatiao_list"}

func readDocument(text []byte) (*kindsDocument, error) {
	var doc kindsDocument
	if err := json.Unmarshal(text, &doc); err != nil {
		return nil, fmt.Errorf("not a kinds document: %w", err)
	}
	if doc.Format != 1 {
		return nil, fmt.Errorf("a kinds document is format 1, not %d", doc.Format)
	}
	return &doc, nil
}

// describedTable is the table of the kind named kind, as a kind header
// would declare it under the name table: the same slots, arguments and
// lent marks readTable finds in a header, so the glue is the same
// whichever the kind was read from.
func describedTable(doc *kindsDocument, kind, table string) (*cTable, error) {
	var found *kindDescription
	for i := range doc.Kinds {
		if doc.Kinds[i].Name == kind {
			found = &doc.Kinds[i]
		}
	}
	if found == nil {
		return nil, fmt.Errorf("the document describes no kind %s", kind)
	}
	t := &cTable{name: table, floorHash: found.FloorHash, object: found.Object}
	for _, s := range found.Slots {
		where := kind + "." + s.Name
		slot := cSlot{name: s.Name, ret: "guatiao_status", lent: map[string]bool{},
			params: []cParam{{typ: "void *", name: "ctx"}}}
		for _, a := range s.Args {
			typ, err := cArg(a, where)
			if err != nil {
				return nil, err
			}
			if a.Pass == "lent" && cTrees[a.Type] != "" {
				slot.lent[a.Name] = true
			}
			slot.params = append(slot.params, cParam{typ: typ, name: a.Name})
		}
		if s.Answer != nil {
			typ, err := cAnswer(*s.Answer, where)
			if err != nil {
				return nil, err
			}
			slot.params = append(slot.params, cParam{typ: typ, name: "out"})
		}
		if s.Fallible {
			slot.params = append(slot.params, cParam{typ: "guatiao_provider_error *", name: "err"})
		}
		t.slots = append(t.slots, slot)
	}
	return t, nil
}

func cArg(a argDescription, where string) (string, error) {
	if c, ok := cScalars[a.Type]; ok {
		return c, nil
	}
	switch a.Type {
	case "str":
		return "guatiao_str", nil
	case "bytes":
		return "guatiao_bytes", nil
	case "bytes_mut":
		return "guatiao_bytes_mut", nil
	case "object":
		switch a.Pass {
		case "taken":
			return "guatiao_object", nil
		case "lent":
			return "const guatiao_object *", nil
		}
		return "", fmt.Errorf("%s(%s): an object is passed taken or lent, not %q", where, a.Name, a.Pass)
	}
	tree, ok := cTrees[a.Type]
	if !ok {
		return "", fmt.Errorf("%s(%s): %q is no argument type", where, a.Name, a.Type)
	}
	switch a.Pass {
	case "borrowed":
		if a.Type == "list" {
			return "", fmt.Errorf("%s(%s): a list cannot be passed borrowed", where, a.Name)
		}
		return "const " + tree + " *", nil
	case "taken", "lent":
		return tree + " *", nil
	}
	return "", fmt.Errorf("%s(%s): a %s is passed borrowed, taken or lent, not %q", where, a.Name, a.Type, a.Pass)
}

func cAnswer(a argDescription, where string) (string, error) {
	if c, ok := cScalars[a.Type]; ok {
		return c + " *", nil
	}
	switch a.Type {
	case "text":
		return "guatiao_string *", nil
	case "object":
		return "guatiao_object *", nil
	}
	if tree, ok := cTrees[a.Type]; ok {
		return tree + " *", nil
	}
	return "", fmt.Errorf("%s: an answer cannot be %q", where, a.Type)
}
