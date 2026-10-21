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

    def __reduce__(self):
        return (type(self), (self.status, self.message))


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


class ParseError(BadValue, ValueError):
    """A document that does not parse, and where it stopped.

    `format` is `"json"`, `"toml"` or `"yaml"`; `line` and `column` count
    from 1 (the column in characters) and are 0 when the parser did not
    say where; `message` says what it expected, without quoting the
    document. For text that is not UTF-8 they name its first bad byte.
    """

    def __init__(self, message: str, *, format: str, line: int, column: int) -> None:
        where = f" at line {line}, column {column}" if line else ""
        super().__init__(int(_abi.Status.ERR_BAD_VALUE), f"{message}{where}")
        self.message = message
        self.format = format
        self.line = line
        self.column = column

    def __reduce__(self):
        return (_parse_error, (self.message, self.format, self.line, self.column))


def _parse_error(message: str, format: str, line: int, column: int) -> ParseError:
    return ParseError(message, format=format, line=line, column=column)


class Unparsed(BadValue, ValueError):
    """A date, time, instant or span whose text its grammar refused.

    `form` is `"date"`, `"time"`, `"datetime"`, `"timestamp"` or
    `"span"`; `at` is the byte offset, in the UTF-8 text, where reading
    stopped; `expected` says what would have been accepted there. Neither
    quotes the text.
    """

    def __init__(self, expected: str, *, form: str, at: int) -> None:
        super().__init__(int(_abi.Status.ERR_BAD_VALUE), f"{expected} at byte {at}")
        self.expected = expected
        self.form = form
        self.at = at

    def __reduce__(self):
        return (_unparsed, (self.expected, self.form, self.at))


def _unparsed(expected: str, form: str, at: int) -> Unparsed:
    return Unparsed(expected, form=form, at=at)


class Unsupported(GuatiaoError):
    """What was asked is not something the other side offers."""


class Busy(GuatiaoError):
    """Not now: something is still in use."""


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
    _abi.Status.ERR_UNSUPPORTED: Unsupported,
    _abi.Status.ERR_BUSY: Busy,
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
