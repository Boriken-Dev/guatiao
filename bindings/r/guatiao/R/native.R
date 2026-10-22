# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# Finding the native libraries, and the conditions this package signals.

.gt_state <- new.env(parent = emptyenv())
.gt_state$chosen <- character(3)
.gt_state$opened <- character(3)

.gt_bases <- c("guatiao", "guatiao_serde", "guatiao_intake")

# Each surface's own environment variable, consulted before GUATIAO_LIBRARY.
.gt_variables <- c("", "GUATIAO_SERDE_LIBRARY", "GUATIAO_INTAKE_LIBRARY")

.gt_filename <- function(base) {
  if (.Platform$OS.type == "windows") return(paste0(base, ".dll"))
  if (Sys.info()[["sysname"]] == "Darwin") return(paste0("lib", base, ".dylib"))
  paste0("lib", base, ".so")
}

# Name the library this session uses for a surface
#
# Called before the first call that needs the library: a file exporting
# the surface, whatever it is called, or a directory holding
# `guatiao.dll` / `libguatiao.so` / `libguatiao.dylib` (or the surface's
# own file name). It wins over the environment. Naming another file once
# one is loaded is an error; naming the same one is not.
#
# @param path A file or a directory.
# @param library `"guatiao"`, `"guatiao_serde"` or `"guatiao_intake"`.
# @return `NULL`, invisibly.
# @export
use_library <- function(path, library = "guatiao") {
  stopifnot(is.character(path), length(path) == 1L, !is.na(path))
  lib <- match(library, .gt_bases)
  if (is.na(lib)) stop("no such library: ", library, call. = FALSE)
  path <- normalizePath(path, winslash = "/", mustWork = TRUE)
  opened <- .gt_state$opened[lib]
  if (nzchar(opened)) {
    want <- if (dir.exists(path)) file.path(path, .gt_filename(library)) else path
    if (!identical(normalizePath(want, winslash = "/", mustWork = FALSE), opened)) {
      stop(sprintf("%s is already loaded from %s", library, opened), call. = FALSE)
    }
  }
  .gt_state$chosen[lib] <- path
  invisible(NULL)
}

# Where to look for library `lib` (0 core, 1 serde, 2 intake), best first:
# what use_library named, the surface's own variable, GUATIAO_LIBRARY, then
# the platform's search. A directory holds the surface's file under its
# platform name; a file named for the surface is taken as it is; a
# GUATIAO_LIBRARY file serves every surface it exports, else the surface's
# own file beside it does.
.gt_candidates <- function(lib) {
  base <- .gt_bases[lib + 1L]
  file <- .gt_filename(base)
  from <- function(p, shared) {
    if (dir.exists(p)) return(file.path(p, file))
    if (!shared || lib == 0L || !file.exists(p)) return(p)
    if (.Call(C_file_carries, enc2utf8(normalizePath(p, winslash = "/", mustWork = FALSE)), lib)) return(p)
    file.path(dirname(p), file)
  }
  out <- character()
  if (nzchar(.gt_state$chosen[lib + 1L])) out <- c(out, from(.gt_state$chosen[lib + 1L], FALSE))
  own <- if (nzchar(.gt_variables[lib + 1L])) Sys.getenv(.gt_variables[lib + 1L]) else ""
  if (nzchar(own)) out <- c(out, from(own, FALSE))
  env <- Sys.getenv("GUATIAO_LIBRARY")
  if (nzchar(env)) out <- c(out, from(env, TRUE))
  # The platform's own search, by file name.
  unique(c(out, file))
}

.gt_ensure <- function(lib) {
  lib <- as.integer(lib)
  if (.Call(C_library_is_open, lib)) return(invisible(NULL))
  looked <- .gt_candidates(lib)
  reasons <- character()
  for (p in looked) {
    path <- if (grepl("[/\\\\]", p)) normalizePath(p, winslash = "/", mustWork = FALSE) else p
    r <- .Call(C_open_library, lib, enc2utf8(path))
    if (r[[1L]] == 0L) {
      .gt_state$opened[lib + 1L] <- path
      return(invisible(NULL))
    }
    if (r[[1L]] == 2L) {
      stop(structure(class = c("guatiao_library_not_found", "error", "condition"),
        list(message = sprintf("%s at %s lacks the export %s: not a build of %s", .gt_bases[lib + 1L],
          path, r[[2L]], .gt_bases[lib + 1L]), call = NULL, basename = .gt_bases[lib + 1L], looked = path)))
    }
    reasons <- c(reasons, sprintf("%s (%s)", path, r[[2L]]))
  }
  variable <- if (nzchar(.gt_variables[lib + 1L])) paste0(.gt_variables[lib + 1L], " or ") else ""
  stop(structure(class = c("guatiao_library_not_found", "error", "condition"),
    list(message = sprintf("%s not found; looked at %s. Set %sGUATIAO_LIBRARY or call use_library().",
      .gt_bases[lib + 1L], paste(reasons, collapse = ", "), variable), call = NULL,
      basename = .gt_bases[lib + 1L], looked = looked)))
}

# Whether the loaded build has an export
#
# @param symbol The export's C name.
# @param library `"guatiao"`, `"guatiao_serde"` or `"guatiao_intake"`.
# @return `TRUE` or `FALSE`; loads the library first.
# @export
has_export <- function(symbol, library = "guatiao") {
  lib <- match(library, .gt_bases) - 1L
  if (is.na(lib)) stop("no such library: ", library, call. = FALSE)
  .Call(C_has_export, lib, symbol)
}

# ---- conditions --------------------------------------------------------------

.gt_statuses <- c(OK = 0L, BadValue = 7L, AllocFailed = 8L, WrongKind = 9L, NotFound = 10L,
  NullArgument = 11L, Gone = 12L, Unsupported = 13L, Busy = 14L, Internal = 100L)

# The status codes a guatiao call answers
#
# A named integer vector: `OK` 0, `BadValue` 7, `AllocFailed` 8,
# `WrongKind` 9, `NotFound` 10, `NullArgument` 11, `Gone` 12,
# `Unsupported` 13, `Busy` 14, `Internal` 100.
#
# @export
statuses <- .gt_statuses

.gt_status_name <- function(code) {
  hit <- names(.gt_statuses)[match(as.integer(code), .gt_statuses)]
  if (is.na(hit)) sprintf("Status%d", as.integer(code)) else hit
}

# A guatiao error
#
# The condition a failing status becomes, and what a provider's method
# signals to answer one: `stop(guatiao_error("BadValue", "nobody to greet"))`.
#
# @param status A status name (see [statuses]) or code.
# @param message The words; may be empty.
# @return A condition of class `guatiao_error`, with `status` (the name),
#   `code` (the number) and `message`.
# @export
guatiao_error <- function(status, message = "") {
  code <- if (is.character(status)) unname(.gt_statuses[status]) else as.integer(status)
  if (length(code) != 1L || is.na(code)) stop("no such status: ", status, call. = FALSE)
  structure(class = c("guatiao_error", "error", "condition"),
    list(message = as.character(message), call = NULL, status = .gt_status_name(code), code = code))
}

# @export
conditionMessage.guatiao_error <- function(c) {
  if (nzchar(c$message)) paste0(c$status, ": ", c$message) else c$status
}

.gt_raise <- function(status, message) stop(guatiao_error(as.integer(status), message))

.gt_closed <- function() {
  stop(structure(class = c("guatiao_closed", "error", "condition"),
    list(message = "the value, registry or handle is closed", call = NULL)))
}

.gt_missing_symbol <- function(lib, name, feature) {
  path <- .gt_state$opened[lib + 1L]
  msg <- if (nzchar(feature)) {
    sprintf("%s at %s lacks %s: build it with the '%s' feature", .gt_bases[lib + 1L], path, name, feature)
  } else {
    sprintf("%s at %s lacks %s", .gt_bases[lib + 1L], path, name)
  }
  stop(structure(class = c("guatiao_missing_symbol", "error", "condition"),
    list(message = msg, call = NULL, path = path, symbol = name, feature = feature)))
}

.gt_unparsed <- function(form, at, expected) {
  stop(structure(class = c("guatiao_unparsed", "guatiao_error", "error", "condition"),
    list(message = sprintf("not a %s: at byte %d, expected %s", form, as.integer(at), expected), call = NULL,
      status = "BadValue", code = 7L, form = form, at = as.integer(at), expected = expected)))
}

