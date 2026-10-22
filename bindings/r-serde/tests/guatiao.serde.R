# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# The suite drives a real build: GUATIAO_LIBRARY names it. Without it
# every case that needs the library is a stated skip, and
# GUATIAO_REQUIRE_ALL=1 turns any skip into a failure.

library(guatiao.serde)

Sys.setenv(R_TESTS = "")

.tests_dir <- if (dir.exists("cases")) "cases" else file.path("tests", "cases")
source(file.path(.tests_dir, "harness.R"))
source(file.path(.tests_dir, "helpers.R"))

for (f in c("serde.R", "crossing.R")) run_file(file.path(.tests_dir, f))
finish()
