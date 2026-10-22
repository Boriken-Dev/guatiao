# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""What the binding raises survives the trip a worker pool or a cache
gives an exception, and a refusal keeps its words."""

from __future__ import annotations

import pickle

from guatiao.errors import BadValue, GuatiaoError, WrongKind


def test_a_status_error_pickles():
    for error in (GuatiaoError(99, "odd"), BadValue(7, "x"), WrongKind(9)):
        back = pickle.loads(pickle.dumps(error))
        assert type(back) is type(error)
        assert (back.status, back.message, str(back)) == (
            error.status,
            error.message,
            str(error),
        )
