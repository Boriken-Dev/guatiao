# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# Finding the native libraries, and the conditions this package signals.

.gt_state <- new.env(parent = emptyenv())
.gt_state$chosen <- NULL
.gt_state$opened <- character(3)

.gt_bases <- c("guatiao", "guatiao_serde", "guatiao_intake")

.gt_filename <- function(base) {
  if (.Platform$OS.type == "windows") return(paste0(base, ".dll"))
  if (Sys.info()[["sysname"]] == "Darwin") return(paste0("lib", base, ".dylib"))
  paste0("lib", base, ".so")
}

# Name the guatiao library this session uses
#
# Called before the first call that needs the library: a file exporting
# the C ABI, whatever it is called, or a directory holding
# `guatiao.dll` / `libguatiao.so` / `libguatiao.dylib`. It wins over
# `GUATIAO_LIBRARY`. Naming another file once one is loaded is an error;
# naming the same one is not.
#
# @param path A file or a directory.
# @return `NULL`, invisibly.
# @export
use_library <- function(path) {
  stopifnot(is.character(path), length(path) == 1L, !is.na(path))
  path <- normalizePath(path, winslash = "/", mustWork = TRUE)
  opened <- .gt_state$opened[1L]
  if (nzchar(opened)) {
    want <- if (dir.exists(path)) file.path(path, .gt_filename("guatiao")) else path
    if (!identical(normalizePath(want, winslash = "/", mustWork = FALSE), opened)) {
      stop(sprintf("guatiao is already loaded from %s", opened), call. = FALSE)
    }
  }
  .gt_state$chosen <- path
  invisible(NULL)
}

# Where to look for library `lib` (0 core, 1 serde, 2 intake), best first.
.gt_candidates <- function(lib) {
  base <- .gt_bases[lib + 1L]
  file <- .gt_filename(base)
  from <- function(p) {
    if (dir.exists(p)) return(file.path(p, file))
    if (lib == 0L) return(p)
    file.path(dirname(p), file)
  }
  if (!is.null(.gt_state$chosen)) return(from(.gt_state$chosen))
  env <- Sys.getenv("GUATIAO_LIBRARY")
  if (nzchar(env)) return(from(env))
  # The platform's own search, by file name.
  file
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
  stop(structure(class = c("guatiao_library_not_found", "error", "condition"),
    list(message = sprintf("%s not found; looked at %s. Set GUATIAO_LIBRARY or call use_library().",
      .gt_bases[lib + 1L], paste(reasons, collapse = ", ")), call = NULL,
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

.gt_formats <- c("json", "toml", "yaml")

.gt_parse_error <- function(info, message) {
  format <- .gt_formats[info[[1L]] + 1L]
  where <- if (info[[2L]] > 0) sprintf(" at line %d, column %d", as.integer(info[[2L]]), as.integer(info[[3L]])) else ""
  stop(structure(class = c("guatiao_parse_error", "guatiao_error", "error", "condition"),
    list(message = sprintf("%s does not parse%s: %s", toupper(format), where, message), call = NULL,
      status = "BadValue", code = 7L, format = format, line = as.integer(info[[2L]]),
      column = as.integer(info[[3L]]), reason = message)))
}
