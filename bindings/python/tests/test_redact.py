# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""`guatiao.redact`: a copy with every secret the schema marks hidden."""

from __future__ import annotations

import pytest

from guatiao import Value, WrongKind, redact

SCHEMA = {
    "type": "object",
    "properties": {
        "user": {"type": "string"},
        "password": {"type": "string", "writeOnly": True, "x-sensitive": True},
        "token": {"type": "string", "writeOnly": True},
        "tls": {
            "type": "object",
            "properties": {
                "ca": {"type": "string"},
                "key": {"$ref": "#/$defs/Key"},
            },
        },
        "recovery": {"type": "array", "items": {"type": "string", "x-sensitive": True}},
    },
    "$defs": {"Key": {"type": "string", "x-sensitive": True}},
}

LOGIN = {
    "user": "ana",
    "password": "hunter2",
    "token": "tok-9f",
    "tls": {"ca": "/etc/ca.pem", "key": "-----BEGIN KEY-----"},
    "recovery": ["r1", "r2"],
}


def test_every_secret_is_hidden_at_any_depth():
    with redact(SCHEMA, LOGIN) as shown:
        assert shown.to_python() == {
            "user": "ana",
            "password": "[redacted]",
            "token": "[redacted]",
            "tls": {"ca": "/etc/ca.pem", "key": "[redacted]"},
            "recovery": ["[redacted]", "[redacted]"],
        }


def test_values_and_refs_are_taken_as_they_are():
    with Value(SCHEMA) as schema, Value({"inner": LOGIN}) as doc:
        with redact(schema, doc.as_map()["inner"], "***") as shown:
            assert shown.as_map()["password"].to_python() == "***"
            assert shown.as_map()["user"].to_python() == "ana"


def test_an_undeclared_key_is_hidden_and_a_non_schema_refused():
    with redact(SCHEMA, {"user": "ana", "extra": "?"}, "*") as shown:
        assert shown.to_python() == {"user": "ana", "extra": "*"}
    with pytest.raises(WrongKind):
        redact(5, LOGIN)
