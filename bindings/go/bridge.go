// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guatiao

import (
	"fmt"
	"unsafe"

	"github.com/Boriken-Dev/guatiao/bindings/go/internal/bridge"
)

func init() {
	bridge.Core = func() error {
		_, err := core()
		return err
	}
	bridge.Lift = lift
	bridge.WithNodes = withNodes
	bridge.NodeOf = func(item any) (unsafe.Pointer, bool, error) {
		r, ok := asRef(item)
		if !ok {
			return nil, false, nil
		}
		node, err := r.requireNode()
		return node, true, err
	}
	bridge.Adopt = func(root, alloc unsafe.Pointer) any { return adopt(root, alloc) }
	bridge.Borrow = func(find func() unsafe.Pointer, keep any) (any, error) {
		k, ok := asRef(keep)
		if !ok {
			return nil, fmt.Errorf("guatiao: %T is not a value", keep)
		}
		return Ref{o: &borrowed{find: find, keep: k.o}}, nil
	}
	bridge.Check = check
}
