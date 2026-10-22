# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# A small test harness, so the package needs no testing dependency: each
# check counts as passed or failed, a case that signals counts as failed,
# and a skip is stated with its reason.

.h <- new.env()
.h$passed <- 0L
.h$failed <- 0L
.h$skipped <- 0L
.h$case <- ""
.h$failures <- character()

.fail <- function(what) {
  .h$failed <- .h$failed + 1L
  line <- sprintf("FAIL [%s] %s", .h$case, what)
  .h$failures <- c(.h$failures, line)
  message(line)
}

expect_true <- function(ok, what = deparse(substitute(ok))[1L]) {
  if (isTRUE(ok)) .h$passed <- .h$passed + 1L else .fail(what)
  invisible(ok)
}

expect_equal <- function(got, want, what = deparse(substitute(got))[1L]) {
  ok <- isTRUE(all.equal(got, want, check.attributes = TRUE))
  if (ok) {
    .h$passed <- .h$passed + 1L
  } else {
    .fail(sprintf("%s: got %s, want %s", what, paste(deparse(got), collapse = " "),
      paste(deparse(want), collapse = " ")))
  }
  invisible(ok)
}

# Passes when `expr` signals a condition of class `class`; answers it.
expect_error <- function(expr, class = "error", what = deparse(substitute(expr))[1L]) {
  got <- tryCatch({
    expr
    NULL
  }, condition = function(e) e)
  if (inherits(got, class)) {
    .h$passed <- .h$passed + 1L
  } else {
    .fail(sprintf("%s: wanted a %s, got %s", what, class,
      if (is.null(got)) "no condition" else paste(class(got), collapse = "/")))
  }
  invisible(got)
}

skip_case <- function(why) {
  .h$skipped <- .h$skipped + 1L
  message(sprintf("SKIP [%s] %s", .h$case, why))
  if (identical(Sys.getenv("GUATIAO_REQUIRE_ALL"), "1")) .fail(paste("required here, but skipped:", why))
  invisible(NULL)
}

case <- function(name, code) {
  .h$case <- name
  ok <- tryCatch({
    force(code)
    TRUE
  }, error = function(e) {
    .fail(paste("signalled:", conditionMessage(e)))
    FALSE
  })
  invisible(ok)
}

run_file <- function(path) {
  message("== ", basename(path))
  sys.source(path, envir = globalenv())
}

finish <- function() {
  cat(sprintf("\n%d passed, %d failed, %d skipped\n", .h$passed, .h$failed, .h$skipped))
  if (.h$failed > 0L) {
    cat(.h$failures, sep = "\n")
    stop(sprintf("%d checks failed", .h$failed))
  }
}
