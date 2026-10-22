# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# Secrets hidden by what a schema marks.

case("redaction", with_library({
  schema <- '{"type": "object", "properties": {"user": {"type": "string"},
    "password": {"type": "string", "writeOnly": true, "x-sensitive": true},
    "tls": {"type": "object", "properties": {"ca": {"type": "string"}, "key": {"$ref": "#/$defs/Key"}}},
    "recovery": {"type": "array", "items": {"type": "string", "x-sensitive": true}}},
    "$defs": {"Key": {"type": "string", "x-sensitive": true}}}'
  s <- value(read_json(schema))
  login <- list(user = "ana", password = "hunter2", tls = list(ca = "/ca", key = "k"),
    recovery = list("r1", "r2"), extra = "?")
  shown <- redact(s, login)
  expect_equal(value_data(shown), list(user = "ana", password = "[redacted]", tls = list(ca = "/ca", key = "[redacted]"),
    recovery = list("[redacted]", "[redacted]"), extra = "[redacted]"))
  close(shown)
  doc <- value(list(inner = login))
  expect_equal(value_data(redact(s, doc$inner, placeholder = "***")$password), "***")
  close(doc)
  expect_error(redact(list(1), login), "guatiao_error")
  close(s)
}))
