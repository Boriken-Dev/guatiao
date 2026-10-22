# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# JSON, TOML and YAML, through guatiao_serde, over guatiao's values.

.gt_formats <- c("json", "toml", "yaml")

.gt_serde_format <- function(format) {
  f <- match(format, .gt_formats) - 1L
  if (length(format) != 1L || is.na(f)) stop("format is json, toml or yaml", call. = FALSE)
  f
}

.gt_bytes <- c(data_uri = 0L, base64 = 1L, array = 2L, refuse = 3L)

.gt_parse_error <- function(info, message) {
  format <- .gt_formats[info[[1L]] + 1L]
  where <- if (info[[2L]] > 0) sprintf(" at line %d, column %d", as.integer(info[[2L]]), as.integer(info[[3L]])) else ""
  stop(structure(class = c("guatiao_parse_error", "guatiao_error", "error", "condition"),
    list(message = sprintf("%s does not parse%s: %s", toupper(format), where, message), call = NULL,
      status = "BadValue", code = 7L, format = format, line = as.integer(info[[2L]]),
      column = as.integer(info[[3L]]), reason = message)))
}

serde_parse <- function(text, format = "json", read_data_uris = FALSE, alloc = NULL) {
  how <- if (isTRUE(read_data_uris)) 256L else 0L
  r <- .Call(C_serde_parse, .gt_serde_format(format), enc2utf8(text), how, alloc)
  if (inherits(r, "guatiao_serde_refusal")) .gt_parse_error(.subset2(r, 1L), .subset2(r, 2L))
  r
}

serde_emit <- function(x, format = "json", pretty = FALSE, bytes = c("data_uri", "base64", "array", "refuse")) {
  bytes <- match.arg(bytes)
  .Call(C_serde_emit, .gt_serde_format(format), x, .gt_bytes[[bytes]], isTRUE(pretty))
}
