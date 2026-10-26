# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""An allocator that counts the blocks it has handed out and not taken back."""

from __future__ import annotations

import ctypes

from guatiao import _abi, _lib


class Counting:
    """The library's default allocator, wrapped to count live blocks."""

    def __init__(self) -> None:
        base = _lib.core().alloc
        self.live = 0
        # Set, every allocation is refused: a caller's failure paths.
        self.refusing = False
        alloc_type = _abi.Alloc._fields_[2][1]
        free_type = _abi.Alloc._fields_[3][1]
        realloc_type = _abi.Alloc._fields_[4][1]

        def _alloc(_ctx, size, align):
            if self.refusing:
                return None
            ptr = base.alloc(base.ctx, size, align)
            if ptr:
                self.live += 1
            return ptr

        def _free(_ctx, ptr, size, align):
            if ptr:
                self.live -= 1
            base.free(base.ctx, ptr, size, align)

        # Held on self: ctypes drops a callback nothing references.
        self._callbacks = (alloc_type(_alloc), free_type(_free))
        self.alloc = _abi.Alloc(
            ctypes.sizeof(_abi.Alloc),
            None,
            self._callbacks[0],
            self._callbacks[1],
            realloc_type(),
        )
