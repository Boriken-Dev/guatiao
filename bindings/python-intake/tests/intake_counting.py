# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""An allocator counting the blocks it handed out and has not taken back,
over a library's own: a tree built in it by one library and freed through
another comes back to zero."""

from __future__ import annotations

import ctypes

from guatiao import ffi


class Counting:
    """Counts over `base`, a library's default allocator."""

    def __init__(self, base: ffi.Alloc) -> None:
        self.live = 0
        fields = dict(ffi.Alloc._fields_)

        def _alloc(_ctx, size, align):
            ptr = base.alloc(base.ctx, size, align)
            if ptr:
                self.live += 1
            return ptr

        def _free(_ctx, ptr, size, align):
            if ptr:
                self.live -= 1
            base.free(base.ctx, ptr, size, align)

        # Held on self: ctypes drops a callback nothing references.
        self._callbacks = (fields["alloc"](_alloc), fields["free"](_free))
        self.alloc = ffi.Alloc(
            ctypes.sizeof(ffi.Alloc), None, self._callbacks[0], self._callbacks[1], fields["release"]()
        )
