# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# The suite drives a real build: GUATIAO_LIBRARY names it. Without it
# every case that needs the library is a stated skip, and
# GUATIAO_REQUIRE_ALL=1 turns any skip into a failure.

library(guatiao)

# R CMD check names a startup file for this process; the R processes the
# tests start (R CMD SHLIB, R CMD INSTALL, Rscript) must not look for it.
Sys.setenv(R_TESTS = "")

.tests_dir <- if (dir.exists("cases")) "cases" else file.path("tests", "cases")
source(file.path(.tests_dir, "harness.R"))
source(file.path(.tests_dir, "helpers.R"))
source(file.path(.tests_dir, "json.R"))

files <- c("abi.R", "values.R", "typed.R", "redact.R", "time.R", "frees.R",
  "greeters.R", "provide.R", "wide.R", "integer64.R", "threads.R", "registry.R", "examples.R")
for (f in files) {
  path <- file.path(.tests_dir, f)
  if (file.exists(path)) run_file(path)
}
finish()
