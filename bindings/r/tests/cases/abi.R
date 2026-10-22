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
    check(identical(committed, readBin(copy, "raw", file.size(copy))), paste(basename(h), "matches"))
  }
}))

case("the status names and codes are the header's", with_repo({
  header <- paste(readLines(repo_file("crates", "guatiao", "include", "guatiao.h"), warn = FALSE), collapse = "\n")
  found <- regmatches(header, gregexpr("GUATIAO_(OK|ERR_[A-Z_]+) = [0-9]+", header))[[1L]]
  codes <- as.integer(sub(".* = ", "", found))
  check_equal(sort(unname(statuses)), sort(codes))
  check_equal(unname(statuses[["BadValue"]]), codes[grepl("ERR_BAD_VALUE", found)])
  check_equal(unname(statuses[["Internal"]]), codes[grepl("ERR_INTERNAL", found)])
}))

case("a guatiao error carries its status and words", {
  e <- guatiao_error("Busy", "still in use")
  check_equal(e$status, "Busy")
  check_equal(e$code, 14L)
  check_equal(conditionMessage(e), "Busy: still in use")
  check_equal(conditionMessage(guatiao_error(7)), "BadValue")
  check_error(guatiao_error("Nonesuch"))
  got <- check_error(stop(guatiao_error("NotFound", "x")), "guatiao_error")
  check_equal(got$status, "NotFound")
})

case("the library is found through GUATIAO_LIBRARY", with_library({
  check(has_export("guatiao_value_map"))
  check(!has_export("guatiao_no_such_export"))
  check(has_export("guatiao_json_parse_ex", "guatiao_serde"))
  check_error(has_export("x", "nonesuch"))
}))
