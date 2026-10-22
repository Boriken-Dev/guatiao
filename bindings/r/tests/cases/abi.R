# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# The glue compiles against copies of the headers: they must be the
# committed ones. The status table must be the header's.

case("the header copies are the committed headers", with_repo({
  for (h in c("guatiao/include/guatiao.h", "guatiao-serde/include/guatiao_serde.h",
    "guatiao-intake/include/guatiao_intake.h")) {
    committed <- readBin(repo_file("crates", h), "raw", file.size(repo_file("crates", h)))
    copy <- repo_file("bindings", "r", "src", basename(h))
    expect_true(identical(committed, readBin(copy, "raw", file.size(copy))), paste(basename(h), "matches"))
  }
}))

case("the status names and codes are the header's", with_repo({
  header <- paste(readLines(repo_file("crates", "guatiao", "include", "guatiao.h"), warn = FALSE), collapse = "\n")
  found <- regmatches(header, gregexpr("GUATIAO_(OK|ERR_[A-Z_]+) = [0-9]+", header))[[1L]]
  codes <- as.integer(sub(".* = ", "", found))
  expect_equal(sort(unname(statuses)), sort(codes))
  expect_equal(unname(statuses[["BadValue"]]), codes[grepl("ERR_BAD_VALUE", found)])
  expect_equal(unname(statuses[["Internal"]]), codes[grepl("ERR_INTERNAL", found)])
}))

case("a guatiao error carries its status and words", {
  e <- guatiao_error("Busy", "still in use")
  expect_equal(e$status, "Busy")
  expect_equal(e$code, 14L)
  expect_equal(conditionMessage(e), "Busy: still in use")
  expect_equal(conditionMessage(guatiao_error(7)), "BadValue")
  expect_error(guatiao_error("Nonesuch"))
  got <- expect_error(stop(guatiao_error("NotFound", "x")), "guatiao_error")
  expect_equal(got$status, "NotFound")
})

case("the library is found through GUATIAO_LIBRARY", with_library({
  expect_true(has_export("guatiao_value_map"))
  expect_true(!has_export("guatiao_no_such_export"))
  expect_true(has_export("guatiao_json_parse_ex", "guatiao_serde"))
  expect_error(has_export("x", "nonesuch"))
}))
