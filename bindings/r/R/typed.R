# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# Typed reading and schemas: an S4 class describes a configuration, its
# typed slots giving the schema; from_value() reads a value back as a type.

field <- function(type, default, doc = "", ...) {
  structure(list(type = type, has_default = !missing(default), default = if (missing(default)) NULL else default,
    doc = doc, keywords = list(...)), class = "guatiao_field")
}

.gt_config_var <- function(Class) paste0(".__guatiao_config_", Class)

config_class <- function(Class, ..., where = topenv(parent.frame())) {
  fields <- list(...)
  if (length(fields) && (is.null(names(fields)) || any(!nzchar(names(fields))))) {
    stop("every field of a configuration is named", call. = FALSE)
  }
  for (f in fields) if (!inherits(f, "guatiao_field")) stop("each field is a field()", call. = FALSE)
  slots <- vapply(fields, function(f) f$type, character(1))
  proto <- list()
  for (n in names(fields)) if (fields[[n]]$has_default && !is.null(fields[[n]]$default)) proto[[n]] <- fields[[n]]$default
  gen <- methods::setClass(Class, slots = slots, prototype = proto, where = where)
  assign(.gt_config_var(Class), fields, envir = where)
  invisible(gen)
}

# The environment a class was defined in, where config_class() left its fields.
.gt_class_env <- function(def) {
  pkg <- def@package
  if (identical(pkg, ".GlobalEnv")) return(globalenv())
  tryCatch(asNamespace(pkg), error = function(e) globalenv())
}

.gt_config_fields <- function(def) {
  env <- .gt_class_env(def)
  var <- .gt_config_var(def@className)
  if (exists(var, envir = env, inherits = FALSE)) get(var, envir = env, inherits = FALSE) else list()
}

.gt_class_name <- function(type) {
  if (methods::is(type, "classGeneratorFunction")) return(type@className)
  if (methods::is(type, "refObjectGenerator")) return(type$className)
  if (is.character(type) && length(type) == 1L) return(type)
  stop("a type is a class name or generator", call. = FALSE)
}

# The member of a class union with NULL that is not NULL, or NULL.
.gt_nullable <- function(type) {
  def <- methods::getClassDef(type)
  if (is.null(def) || !methods::is(def, "ClassUnionRepresentation")) return(NULL)
  direct <- names(Filter(function(e) e@distance == 1, def@subclasses))
  members <- setdiff(direct, "NULL")
  if (!("NULL" %in% direct) || length(members) != 1L) return(NULL)
  members
}

.gt_time_pattern <- "^([01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9](\\.[0-9]{1,9})?$"
.gt_length_schema <- list(type = "array",
  prefixItems = list(list(type = "integer", minimum = 0, maximum = structure("18446744073709551615", class = "guatiao_number")),
    list(type = "integer", minimum = 0, maximum = 999999999)),
  items = FALSE, minItems = 2, maxItems = 2)

# A slot's prototype is its default when the slot is a basic type whose
# prototype holds something; any other class's prototype is no default.
.gt_basic <- c("character", "integer", "numeric", "double", "logical", "raw", "list")
.gt_proto_default <- function(def, n) {
  if (!(def@slots[[n]] %in% .gt_basic)) return(NULL)
  p <- tryCatch(methods::slot(def@prototype, n), error = function(e) NULL)
  if (is.null(p) || length(p) == 0L) NULL else p
}

schema_of <- function(type) {
  type <- .gt_class_name(type)
  switch(type,
    ANY = , list = return(structure(list(), names = character())),
    character = return(list(type = "string")),
    integer = return(list(type = "integer", minimum = -2147483647L, maximum = 2147483647L)),
    numeric = , double = return(list(type = "number")),
    logical = return(list(type = "boolean")),
    raw = return(list(type = "bytes")),
    Date = return(list(type = "string", format = "date")),
    POSIXct = return(list(type = "string", format = "date-time")),
    difftime = return(.gt_length_schema),
    NULL)
  inner <- .gt_nullable(type)
  if (!is.null(inner)) return(list(anyOf = list(schema_of(inner), list(type = "null"))))
  def <- methods::getClassDef(type)
  if (is.null(def) || def@virtual) stop(sprintf("no schema for %s", type), call. = FALSE)
  slots <- def@slots
  fields <- .gt_config_fields(def)
  props <- list()
  required <- character()
  for (n in names(slots)) {
    s <- schema_of(slots[[n]])
    f <- fields[[n]]
    default <- NULL
    has_default <- FALSE
    if (!is.null(f) && f$has_default) {
      has_default <- TRUE
      default <- f$default
    } else {
      p <- .gt_proto_default(def, n)
      if (!is.null(p)) {
        has_default <- TRUE
        default <- p
      }
    }
    if (!is.null(f) && nzchar(f$doc)) s$description <- f$doc
    if (has_default && !is.null(default)) s$default <- default
    if (!is.null(f)) for (k in names(f$keywords)) s[[k]] <- f$keywords[[k]]
    props[[n]] <- s
    if (!has_default && is.null(.gt_nullable(slots[[n]]))) required <- c(required, n)
  }
  out <- list(type = "object", properties = if (length(props)) props else structure(list(), names = character()),
    additionalProperties = FALSE)
  if (length(required)) out$required <- as.list(required)
  out
}

# ---- reading a value as a type ----------------------------------------------------

.gt_got <- function(d) {
  if (is.null(d)) return("null")
  if (inherits(d, "guatiao_absent")) return("absent")
  if (inherits(d, "guatiao_number")) return("number")
  if (is.logical(d)) return("bool")
  if (is.character(d)) return("string")
  if (is.raw(d)) return("bytes")
  if (is.list(d) && !is.null(names(d))) return("map")
  if (is.list(d)) return("list")
  class(d)[1L]
}

.gt_refuse <- function(at, want, d) {
  stop(guatiao_error("BadValue", sprintf("at %s: want %s, got %s", at, want, .gt_got(d))))
}

.gt_auto <- function(d) {
  if (inherits(d, "guatiao_number")) return(.Call(C_number_read, unclass(d)))
  if (is.list(d) && !inherits(d, "guatiao_absent")) {
    out <- lapply(d, .gt_auto)
    names(out) <- names(d)
    return(out)
  }
  d
}

.gt_at <- function(at, k) {
  if (is.numeric(k)) return(sprintf("%s[%d]", if (identical(at, "the root")) "" else at, as.integer(k)))
  if (identical(at, "the root")) k else paste0(at, ".", k)
}

.gt_decode <- function(d, type, at) {
  switch(type,
    ANY = , list = return(.gt_auto(d)),
    character = {
      if (!is.character(d) || inherits(d, "guatiao_number")) .gt_refuse(at, "a string", d)
      return(d)
    },
    integer = {
      if (!inherits(d, "guatiao_number") || !grepl("^-?[0-9]+$", unclass(d))) .gt_refuse(at, "an integer", d)
      v <- suppressWarnings(as.numeric(unclass(d)))
      if (abs(v) > 2147483647) stop(guatiao_error("BadValue", sprintf("at %s: %s is out of range", at, unclass(d))))
      return(as.integer(v))
    },
    numeric = , double = {
      if (!inherits(d, "guatiao_number")) .gt_refuse(at, "a number", d)
      return(as.numeric(unclass(d)))
    },
    logical = {
      if (!is.logical(d)) .gt_refuse(at, "a bool", d)
      return(d)
    },
    raw = {
      if (!is.raw(d)) .gt_refuse(at, "bytes", d)
      return(d)
    },
    Date = {
      if (!is.character(d) || inherits(d, "guatiao_number")) .gt_refuse(at, "a date", d)
      return(time_parse("date", d))
    },
    POSIXct = {
      if (!is.character(d) || inherits(d, "guatiao_number")) .gt_refuse(at, "an instant", d)
      return(as.POSIXct(time_parse("timestamp", d)))
    },
    difftime = return(.gt_length_difftime(read_length(.gt_auto(d)))),
    NULL)
  inner <- .gt_nullable(type)
  if (!is.null(inner)) {
    if (is.null(d) || inherits(d, "guatiao_absent")) return(NULL)
    return(.gt_decode(d, inner, at))
  }
  def <- methods::getClassDef(type)
  if (is.null(def) || def@virtual) stop(sprintf("cannot read a value as %s", type), call. = FALSE)
  if (!is.list(d) || inherits(d, "guatiao_absent") || (length(d) && is.null(names(d)))) .gt_refuse(at, "a map", d)
  slots <- def@slots
  fields <- .gt_config_fields(def)
  args <- list()
  for (n in names(slots)) {
    if (n %in% names(d)) {
      args[[n]] <- .gt_decode(d[[n]], slots[[n]], .gt_at(at, n))
    } else {
      f <- fields[[n]]
      has_proto <- !is.null(.gt_proto_default(def, n))
      if (!is.null(f) && f$has_default || has_proto || !is.null(.gt_nullable(slots[[n]]))) next
      stop(guatiao_error("BadValue", sprintf("at %s: missing", .gt_at(at, n))))
    }
  }
  do.call(methods::new, c(list(type), args))
}

from_value <- function(x, type) {
  d <- if (inherits(x, "guatiao_node")) value_data(x, numbers = "text") else {
    v <- value(x)
    on.exit(close(v))
    value_data(v, numbers = "text")
  }
  .gt_decode(d, .gt_class_name(type), "the root")
}
