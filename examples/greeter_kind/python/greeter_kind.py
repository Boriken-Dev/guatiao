# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""The kinds of `include/greeter_kind.h`, described for Python.

Written by hand from the header: each method in the order its table
declares it, each `FLOOR_HASH` copied from the `#define` beside the
table. A test compares the literals with the header, so a kind that
changes shape fails there rather than in a call. These are what a Python
host calls a greeter through, and what a Python greeter is served as.
"""

from __future__ import annotations

from guatiao.kinds import BOOL, BYTES_MUT, I64, MAP, STR, TEXT, Kind, Method, lent, taken

#: `listener_vtable`: what a conversation tells, implemented by the host.
LISTENER = Kind(
    "listener",
    floor_hash=1867921573,
    is_object=True,
    methods=[Method("heard", [STR], required=True)],
)

#: `conversation_vtable`: one conversation a greeter started.
CONVERSATION = Kind(
    "conversation",
    floor_hash=2769624503,
    is_object=True,
    methods=[
        Method("say", [STR], fallible=True, required=True),
        Method("read", [BYTES_MUT], I64, required=True),
        Method("turns", [], I64, required=True),
    ],
)

#: `greeter_vtable`: greets by name. `greet` is the one slot every table
#: has; the rest were appended, and an older table may lack them.
GREETER = Kind(
    "greeter",
    floor_hash=2695856491,
    methods=[
        Method("greet", [STR], MAP, fallible=True, required=True),
        Method("shout", [STR], TEXT),
        Method("start", [taken(LISTENER)], CONVERSATION, fallible=True),
        Method("greet_into", [STR, lent(LISTENER)], fallible=True),
        Method("greet_from", [taken(MAP)], MAP, fallible=True),
        Method("check", [lent(MAP)], BOOL, fallible=True),
    ],
)

#: `counter_vtable`: counts how often it was asked.
COUNTER = Kind(
    "counter",
    floor_hash=1227528937,
    methods=[Method("count", [], I64, required=True)],
)
