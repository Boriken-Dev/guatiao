// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guatiao

import (
	"unsafe"

	"github.com/Boriken-Dev/guatiao/bindings/go/guatiao/internal/native"
)

// Redact is a copy of value, as a Value to close, with every field
// schema marks secret (x-sensitive or writeOnly) replaced by placeholder,
// at any depth; whatever the schema does not describe is hidden too.
// Local $refs are followed. ErrWrongKind for a schema that is not a map.
// schema and value are Values, views or plain data.
func Redact(schema, value any, placeholder string) (*Value, error) {
	lib, err := core()
	if err != nil {
		return nil, err
	}
	var out *Value
	err = withNodes([]any{schema, value}, func(nodes []unsafe.Pointer) error {
		root := native.Calloc(native.SizeValue)
		if err := check(native.SchemaRedact(nodes[0], nodes[1], placeholder, lib.Alloc(), root), ""); err != nil {
			native.ValueFree(root)
			native.Free(root)
			return err
		}
		out = adopt(root, lib.Alloc())
		return nil
	})
	return out, err
}
