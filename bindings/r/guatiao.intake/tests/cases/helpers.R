# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# Where the library is, and an allocator that counts the blocks it handed
# out and has not had back: the guatiao package's own, which its tests use
# the same way.

gt <- asNamespace("guatiao")

LIBRARY <- Sys.getenv("GUATIAO_LIBRARY")
HAVE_LIBRARY <- nzchar(LIBRARY)

with_library <- function(code) {
  if (!HAVE_LIBRARY) return(skip_case("GUATIAO_LIBRARY names no build of guatiao"))
  force(code)
}

# The directory holding the workspace's libraries: GUATIAO_LIBRARY when it
# is one, else the directory of the file it names.
LIBRARY_DIR <- if (HAVE_LIBRARY && !dir.exists(LIBRARY)) dirname(LIBRARY) else LIBRARY

# A checkout: GUATIAO_REPO, else the directory two levels above
# LIBRARY_DIR when that is a target directory.
find_repo <- function() {
  candidates <- c(Sys.getenv("GUATIAO_REPO"),
    if (HAVE_LIBRARY) normalizePath(file.path(LIBRARY_DIR, "..", ".."), winslash = "/", mustWork = FALSE))
  for (c in candidates) {
    if (nzchar(c) && file.exists(file.path(c, "crates", "guatiao", "include", "guatiao.h"))) {
      return(normalizePath(c, winslash = "/"))
    }
  }
  ""
}
REPO <- find_repo()
repo_file <- function(...) file.path(REPO, ...)

with_repo <- function(code) {
  if (!nzchar(REPO)) return(skip_case("no checkout found: set GUATIAO_REPO"))
  force(code)
}

counting <- function() gt$.gt_counting()
live <- function(a) gt$.gt_live(a)
