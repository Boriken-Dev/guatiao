# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# Redaction and schema checks, through guatiao.

# ---- schemas ----------------------------------------------------------------------

redact <- function(schema, value, placeholder = "[redacted]", alloc = NULL) {
  .gt_root(.Call(C_redact, schema, value, enc2utf8(placeholder), alloc))
}

validate <- function(schema, value) .Call(C_schema_validate, schema, value)
