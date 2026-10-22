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
# `file` names the output when a scan must find it: a scan probes only
# files with the platform's own library extension (`.dylib` on macOS,
# where R's own is `.so`).
build_c <- function(source, dir, name, includes = character(), libs = "", file = NULL) {
  out <- file.path(dir, if (is.null(file)) paste0(name, .Platform$dynlib.ext) else file)
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
FIXTURES <- normalizePath(file.path(.tests_dir, "..", "fixtures"), winslash = "/", mustWork = FALSE)

# ---- the example packages, installed for the tests that use them ------------------

.examples <- new.env()

# Installs greeterkind and rgreeter into a library of their own and
# attaches them; FALSE when there is no checkout to install them from.
with_examples <- function(code) {
  if (!HAVE_LIBRARY) return(skip_case("GUATIAO_LIBRARY names no build of guatiao"))
  if (!HAVE_REPO) return(skip_case("no checkout found: set GUATIAO_REPO"))
  if (is.null(.examples$lib)) {
    lib <- scratch("examples-lib")
    rlibs <- paste(c(lib, .libPaths()), collapse = .Platform$path.sep)
    for (pkg in c(repo_file("examples", "greeter_kind", "r", "greeterkind"), repo_file("examples", "r_provider", "rgreeter"))) {
      out <- withr_env(sprintf("R_LIBS=%s", rlibs),
        suppressWarnings(system2(r_cmd(), c("CMD", "INSTALL", "--no-docs", "--no-multiarch", "-l", shQuote(lib), shQuote(pkg)),
          stdout = TRUE, stderr = TRUE)))
      if (!is.null(attr(out, "status")) && attr(out, "status") != 0) {
        message(paste(out, collapse = "\n"))
        stop("installing ", basename(pkg), " failed")
      }
    }
    .libPaths(c(lib, .libPaths()))
    suppressPackageStartupMessages({
      library(greeterkind)
      library(rgreeter)
    })
    .examples$lib <- lib
  }
  force(code)
}

# An absent greeter is a stated skip, which GUATIAO_REQUIRE_ALL=1 fails.
absent_greeter <- function(which, why) skip_case(sprintf("the %s greeter: %s", which, why))

# The greeter written in C, built into a directory of its own; NULL without a compiler.
build_c_greeter <- function() {
  dir <- scratch("c-greeter")
  if (is.null(build_c(file.path(C_CONSUMER, "greeter_in_c.c"), dir, "c_greeter", file = lib_file("c_greeter")))) NULL else dir
}

build_c_source <- function(name) build_c(file.path(C_CONSUMER, "source_in_c.c"), scratch("c-shelf"), name)

# The guatiao-py bridge: GUATIAO_PY_BRIDGE, else NULL.
bridge <- function() {
  named <- Sys.getenv("GUATIAO_PY_BRIDGE")
  if (nzchar(named) && file.exists(named)) named else NULL
}

bridge_config <- function() list(dirs = list(repo_file("examples", "python_provider")),
  paths = list(repo_file("bindings", "python", "src"), repo_file("examples", "greeter_kind", "python")))

# The test fixtures in C, built once.
.fixtures <- new.env()
fixture <- function() {
  if (is.null(.fixtures$path)) {
    path <- build_c(file.path(FIXTURES, "threads.c"), scratch("fixture"), "gtfixture")
    if (is.null(path)) return(NULL)
    dyn.load(path)
    .fixtures$path <- path
  }
  .fixtures$path
}
fx <- function(name, ...) .Call(name, ..., PACKAGE = "gtfixture")
