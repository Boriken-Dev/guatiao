# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# What several test files share: where the library and a checkout are, an
# allocator that counts the blocks it handed out and has not had back, the
# shared vectors, and C built for the tests that need it.

gt <- asNamespace("guatiao")

LIBRARY <- Sys.getenv("GUATIAO_LIBRARY")
HAVE_LIBRARY <- nzchar(LIBRARY)

# A checkout: GUATIAO_REPO, else the directory two levels above a
# GUATIAO_LIBRARY that is a target directory.
find_repo <- function() {
  named <- Sys.getenv("GUATIAO_REPO")
  candidates <- c(named, if (HAVE_LIBRARY) normalizePath(file.path(LIBRARY, "..", ".."), winslash = "/", mustWork = FALSE))
  for (c in candidates) {
    if (nzchar(c) && file.exists(file.path(c, "crates", "guatiao", "include", "guatiao.h"))) {
      return(normalizePath(c, winslash = "/"))
    }
  }
  ""
}
REPO <- find_repo()
HAVE_REPO <- nzchar(REPO)
repo_file <- function(...) file.path(REPO, ...)

# Runs `code` when the library is named, else states a skip.
with_library <- function(code) {
  if (!HAVE_LIBRARY) return(skip_case("GUATIAO_LIBRARY names no build of guatiao"))
  force(code)
}

with_repo <- function(code) {
  if (!HAVE_REPO) return(skip_case("no checkout found: set GUATIAO_REPO"))
  force(code)
}

counting <- function() gt$.gt_counting()
live <- function(a) gt$.gt_live(a)

vectors <- function() value_data(serde_parse(paste(readLines(repo_file("crates", "guatiao", "tests", "vectors", "time.json"),
  encoding = "UTF-8", warn = FALSE), collapse = "\n")), numbers = "text")

text_of <- function(n) if (inherits(n, "guatiao_number")) unclass(n) else n

lib_file <- function(base) gt$.gt_filename(base)
workspace_library <- function(base) file.path(LIBRARY, lib_file(base))

# ---- C, built with R's own compiler -----------------------------------------

scratch <- function(name) {
  d <- tempfile(paste0("guatiao-r-", name, "-"))
  dir.create(d, recursive = TRUE)
  normalizePath(d, winslash = "/")
}

r_cmd <- function() file.path(R.home("bin"), if (.Platform$OS.type == "windows") "R.exe" else "R")

# `source` built with `R CMD SHLIB` as `name` in `dir`, with the warnings
# CI turns into errors; NULL when there is no compiler.
build_c <- function(source, dir, name, includes = character(), libs = "") {
  out <- file.path(dir, paste0(name, .Platform$dynlib.ext))
  incs <- c(repo_file("crates", "guatiao", "include"), repo_file("examples", "greeter_kind", "include"), includes)
  env <- c(sprintf("PKG_CPPFLAGS=%s", paste(sprintf("-I\"%s\"", incs), collapse = " ")),
    "PKG_CFLAGS=-Wall -Wextra -Werror",
    sprintf("PKG_LIBS=%s %s", if (Sys.info()[["sysname"]] == "Linux") "-lpthread" else "", libs))
  src <- file.path(dir, basename(source))
  file.copy(source, src, overwrite = TRUE)
  old <- setwd(dir)
  on.exit(setwd(old))
  status <- withr_env(env, system2(r_cmd(), c("CMD", "SHLIB", "-o", shQuote(basename(out)), shQuote(basename(src))),
    stdout = TRUE, stderr = TRUE))
  if (!file.exists(out)) {
    message(paste(status, collapse = "\n"))
    return(NULL)
  }
  out
}

withr_env <- function(vars, code) {
  keys <- sub("=.*", "", vars)
  old <- Sys.getenv(keys, unset = NA, names = TRUE)
  do.call(Sys.setenv, as.list(setNames(sub("^[^=]*=", "", vars), keys)))
  on.exit({
    for (k in keys) if (is.na(old[[k]])) Sys.unsetenv(k) else do.call(Sys.setenv, setNames(list(old[[k]]), k))
  })
  force(code)
}

C_CONSUMER <- if (HAVE_REPO) repo_file("crates", "guatiao", "tests", "c_consumer") else ""
FIXTURES <- normalizePath(file.path(here, "..", "fixtures"), winslash = "/", mustWork = FALSE)
