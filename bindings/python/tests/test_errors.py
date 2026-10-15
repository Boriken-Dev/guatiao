# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""What the binding raises survives the trip a worker pool or a cache
gives an exception, and a refusal keeps its words."""

from __future__ import annotations

import ctypes
import pickle

from guatiao import _abi, _lib
from guatiao.errors import BadValue, GuatiaoError, ParseError, WrongKind


def test_a_status_error_pickles():
    for error in (GuatiaoError(99, "odd"), BadValue(7, "x"), WrongKind(9)):
        back = pickle.loads(pickle.dumps(error))
        assert type(back) is type(error)
        assert (back.status, back.message, str(back)) == (
            error.status,
            error.message,
            str(error),
        )


def test_a_parse_error_pickles():
    error = ParseError("expected a value", format="json", line=3, column=7)
    back = pickle.loads(pickle.dumps(error))
    assert type(back) is ParseError
    assert (back.format, back.line, back.column) == ("json", 3, 7)
    assert back.message == "expected a value"
    assert str(back) == str(error)


def test_a_refusal_that_is_not_a_syntax_error_keeps_its_message():
    from guatiao import serde

    lib = _lib.core()
    error = _abi.ParseError()
    text = b"the library's own words"
    view = _abi.Str(ptr=ctypes.cast(ctypes.c_char_p(text), ctypes.c_void_p), len=len(text))
    assert lib.guatiao_value_string(
        ctypes.byref(lib.alloc), view, ctypes.byref(error.message)
    ) == _abi.Status.OK
    try:
        serde._finish_parse(_abi.Status.ERR_WRONG_KIND, error, "json")
    except WrongKind as refusal:
        assert refusal.message == "the library's own words"
    else:
        raise AssertionError("no WrongKind raised")
