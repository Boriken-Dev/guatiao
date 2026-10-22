# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# JSON, TOML and YAML, through guatiao_serde; intake, through
# guatiao_intake; redaction and schema checks, through guatiao.

.gt_serde_format <- function(format) {
  f <- match(format, .gt_formats) - 1L
  if (length(format) != 1L || is.na(f)) stop("format is json, toml or yaml", call. = FALSE)
  f
}

.gt_bytes <- c(data_uri = 0L, base64 = 1L, array = 2L, refuse = 3L)

serde_parse <- function(text, format = "json", read_data_uris = FALSE, alloc = NULL) {
  how <- if (isTRUE(read_data_uris)) 256L else 0L
  .gt_root(.Call(C_serde_parse, .gt_serde_format(format), enc2utf8(text), how, alloc))
}

serde_emit <- function(x, format = "json", pretty = FALSE, bytes = c("data_uri", "base64", "array", "refuse")) {
  bytes <- match.arg(bytes)
  .Call(C_serde_emit, .gt_serde_format(format), x, .gt_bytes[[bytes]], isTRUE(pretty))
}

# ---- intake ----------------------------------------------------------------------

# A lookup into a node answers a view of it; into R data, a copy.
.gt_view <- function(arg, r) {
  if (is.null(r)) return(NULL)
  if (inherits(arg, "guatiao_node")) .gt_node(.subset2(r, 1L), .subset2(r, 2L)) else .gt_root(.subset2(r, 1L))
}

intake_check <- function(schema, form) .Call(C_intake_check, schema, form)

intake_layout <- function(schema, form) .Call(C_intake_layout, schema, form)

intake_is_visible <- function(schema, form, key, values) .Call(C_intake_is_visible, schema, form, key, values)

intake_at <- function(value, path) .gt_view(value, .Call(C_intake_at, value, path))

intake_for_schema <- function(schema) .gt_root(.Call(C_intake_for_schema, schema))

intake_form_for <- function(form, schema, path) {
  r <- .Call(C_intake_form_for, form, schema, path)
  if (is.null(r)) NULL else .gt_root(r)
}

intake_resolve <- function(schema, key) .gt_view(schema, .Call(C_intake_resolve, schema, key))

intake_flat_keys <- function(schema, key) unlist(.Call(C_intake_flat_keys, schema, key), use.names = FALSE)

intake_flatten <- function(schema, key, value) {
  m <- .Call(C_intake_flatten, schema, key, value)
  vapply(m, as.character, character(1))
}

intake_unflatten <- function(schema, key, store) {
  if (is.character(store)) store <- as.list(store)
  if (!is.list(store) || (length(store) && (is.null(names(store)) ||
    !all(vapply(store, function(s) is.character(s) && length(s) == 1L, logical(1)))))) {
    stop("store maps text to text", call. = FALSE)
  }
  if (length(store) == 0L) store <- structure(list(), names = character())
  r <- .Call(C_intake_unflatten, schema, key, store)
  if (is.null(r)) NULL else .gt_root(r)
}

# ---- schemas ----------------------------------------------------------------------

redact <- function(schema, value, placeholder = "[redacted]", alloc = NULL) {
  .gt_root(.Call(C_redact, schema, value, enc2utf8(placeholder), alloc))
}

validate <- function(schema, value) .Call(C_schema_validate, schema, value)
