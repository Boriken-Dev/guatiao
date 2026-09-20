# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""`guatiao_status` as Python exceptions."""

from __future__ import annotations

import ctypes

from . import _abi, _lib


class GuatiaoError(RuntimeError):
    """One `guatiao_status` failure. `status` is the raw code."""

    def __init__(self, status: int, message: str = "") -> None:
        self.status = int(status)
        self.message = message
        try:
            name = _abi.Status(self.status).name
        except ValueError:
            name = str(self.status)
        text = f"{name}: {message}" if message else name
        super().__init__(text)


class BadValue(GuatiaoError):
    pass


class AllocFailed(GuatiaoError):
    pass


class WrongKind(GuatiaoError):
    pass


class NotFound(GuatiaoError):
    pass


class NullArgument(GuatiaoError):
    pass


class Gone(GuatiaoError):
    pass


class Internal(GuatiaoError):
    pass


_BY_STATUS = {
    _abi.Status.ERR_BAD_VALUE: BadValue,
    _abi.Status.ERR_ALLOC: AllocFailed,
    _abi.Status.ERR_WRONG_KIND: WrongKind,
    _abi.Status.ERR_NOT_FOUND: NotFound,
    _abi.Status.ERR_NULL: NullArgument,
    _abi.Status.ERR_GONE: Gone,
    _abi.Status.ERR_INTERNAL: Internal,
}


def raise_for_status(status: int, message: str = "") -> None:
    if status == _abi.Status.OK:
        return
    raise _BY_STATUS.get(status, GuatiaoError)(status, message)


def check(status: int, err: "_abi.ProviderError | None" = None) -> None:
    """Raises unless `status` is `GUATIAO_OK`, reading and freeing
    `err.message` first -- it is an owned string the provider built."""
    message = ""
    if err is not None and err.message.len:
        message = ctypes.string_at(err.message.ptr, err.message.len).decode(
            "utf-8", "replace"
        )
        wrapper = _abi.Value(tag=int(_abi.Tag.STRING), payload=_abi.Payload(text=err.message))
        _lib.core().guatiao_value_free(ctypes.byref(wrapper))
    raise_for_status(status, message)
