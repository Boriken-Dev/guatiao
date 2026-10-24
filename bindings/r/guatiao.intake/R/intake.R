# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# Forms, paths and the flat store, through guatiao_intake, over guatiao's
# values. A lookup into a value or view answers a view of it; into R data,
# a copy.

#' The key of a provider's meta holding its form: read it with a registry's
#' `provider_meta()`, then `intake_check()` it against `provider_config()`.
intake_provider_form_key <- "form"

intake_check <- function(schema, form) .Call(C_intake_check, schema, form)

intake_layout <- function(schema, form) .Call(C_intake_layout, schema, form)

intake_is_visible <- function(schema, form, key, values) .Call(C_intake_is_visible, schema, form, key, values)

intake_at <- function(value, path) .Call(C_intake_at, value, path)

intake_for_schema <- function(schema) .Call(C_intake_for_schema, schema)

intake_form_for <- function(form, schema, path) .Call(C_intake_form_for, form, schema, path)

intake_resolve <- function(schema, key) .Call(C_intake_resolve, schema, key)

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
  .Call(C_intake_unflatten, schema, key, store)
}
