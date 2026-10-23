# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# A tree crosses between guatiao_serde and guatiao: built by one library,
# read and freed through the other, every block accounted for.

case("a tree serde built is changed and freed through guatiao", with_library({
  c <- counting()
  v <- serde_parse('{"a": [1, 2], "b": "x"}', alloc = c)
  expect_true(live(c) > 0)
  push(v$a, 3L)
  expect_equal(value_data(v), list(a = list(1L, 2L, 3L), b = "x"))
  close(v)
  expect_equal(live(c), 0)
  expect_error(serde_parse("{", alloc = c), "guatiao_parse_error")
  expect_equal(live(c), 0)
}))

case("a tree guatiao built is written by serde", with_library({
  c <- counting()
  v <- value(list(port = 5900L), alloc = c)
  expect_equal(serde_emit(v), '{"port":5900}')
  close(v)
  expect_equal(live(c), 0)
}))

case("R data is written as guatiao builds it", with_library({
  expect_equal(serde_emit(list(a = 0.1, b = 1/3, c = 2^1000, d = 2^60)), '{"a":0.1,"b":0.3333333333333333,"c":1.0715086071862673e+301,"d":1152921504606846976}')
  expect_equal(serde_emit(list(a = NA, b = NA_integer_, c = NA_real_, d = NA_character_)), '{"a":null,"b":null,"c":null,"d":null}')
  expect_equal(serde_emit(c(x = 1, y = 2)), '{"x":1,"y":2}')
  expect_equal(serde_emit(factor(c("lo", "hi"))), '["lo","hi"]')
}))

case("the library is guatiao_serde's own, found through its own variable", with_library({
  path <- gt$.gt_state$opened[2L]
  # Without GUATIAO_SERDE_LIBRARY, a GUATIAO_LIBRARY file serves serde when
  # it carries it; otherwise guatiao_serde's own file does.
  shared <- !nzchar(Sys.getenv("GUATIAO_SERDE_LIBRARY")) && !identical(LIBRARY, LIBRARY_DIR) &&
    .Call(gt$C_file_carries, enc2utf8(normalizePath(LIBRARY, winslash = "/")), 1L)
  if (shared) {
    expect_equal(path, normalizePath(LIBRARY, winslash = "/"))
  } else {
    expect_true(grepl("guatiao_serde", basename(path), fixed = TRUE), path)
  }
  expect_true(has_export("guatiao_value_free", "guatiao_serde"))
}))

# The glue compiles against copies of the headers: they must be the
# committed ones.
case("the header copies are the committed headers", with_repo({
  for (h in c("guatiao/include/guatiao.h", "guatiao-serde/include/guatiao_serde.h")) {
    committed <- readBin(repo_file("crates", h), "raw", file.size(repo_file("crates", h)))
    copy <- repo_file("bindings", "r", "guatiao.serde", "src", basename(h))
    expect_true(identical(committed, readBin(copy, "raw", file.size(copy))), paste(basename(h), "matches"))
  }
}))
