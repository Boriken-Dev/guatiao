# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# bit64's integer64, when it is installed: taken wherever a 64-bit integer
# is, answered when int64 or numbers asks for it, and never otherwise. An
# integer64 holds every int64 but INT64_MIN, its NA; nothing past
# INT64_MAX. Without bit64 the cases that need it are stated skips, and
# asking for one is an error naming the package.

HAVE_BIT64 <- requireNamespace("bit64", quietly = TRUE)
NO_BIT64 <- "bit64 is not installed: integer64 is its class"

with_bit64 <- function(code) {
  if (!HAVE_BIT64) return(skip_case(NO_BIT64))
  force(code)
}

i64 <- function(t) bit64::as.integer64(t)

case("without bit64, asking for an integer64 is an error naming the package", with_library({
  asks <- list(
    function() value_data(value(5L), numbers = "integer64"),
    function() kind("never", floor_hash = 1, int64 = "integer64", where = globalenv()),
    function() as.list(span(days = 1), int64 = "integer64"),
    function() as.list(exact_length(1), int64 = "integer64"))
  for (ask in asks) {
    if (HAVE_BIT64) {
      expect_true(!inherits(tryCatch(ask(), error = function(e) e), "error"), "with bit64, integer64 is answered")
    } else {
      e <- expect_error(ask())
      expect_true(grepl("needs the bit64 package", conditionMessage(e)), conditionMessage(e))
    }
  }
  # Without it, the default answers never depend on what is installed.
  expect_true(identical(value_data(value(number(I64_MAX))), number(I64_MAX)))
}))

case("without bit64, a call asking for an integer64 is refused before it runs", with_examples({
  w <- HOST$bind("derived_greeter_hello", Wide)
  if (HAVE_BIT64) {
    expect_true(inherits(call_slot(w, 1L, list(5L), int64 = "integer64"), "integer64"))
  } else {
    e <- expect_error(call_slot(w, 1L, list(5L), int64 = "integer64"))
    expect_true(grepl("needs the bit64 package", conditionMessage(e)), conditionMessage(e))
  }
}))

# The vectors through integer64, in both directions, for a remote `w`.
crosses_as_integer64 <- function(w, which) {
  i <- match("echo_i64", names(Wide$slots))
  u <- match("echo_u64", names(Wide$slots))
  for (t in c(TWO53_1, I64_MAX, "-9223372036854775807", "5")) {
    expect_true(identical(echo_i64(w, i64(t)), if (t == "5") 5L else number(t)), paste(which, "auto answers", t))
    got <- call_slot(w, i, list(i64(t)), int64 = "integer64")
    expect_true(identical(got, i64(t)), paste(which, "echo_i64 as integer64", t))
  }
  # INT64_MIN is bit64's NA: it crosses as a number, and an NA is no argument.
  expect_true(identical(call_slot(w, i, list(number(I64_MIN)), int64 = "integer64"), number(I64_MIN)))
  e <- expect_error(echo_i64(w, bit64::NA_integer64_), "guatiao_error")
  expect_equal(e$message, "wide.echo_i64(x): want a number, not NA")
  # An unsigned value past INT64_MAX has no integer64.
  expect_true(identical(call_slot(w, u, list(i64(I64_MAX)), int64 = "integer64"), i64(I64_MAX)))
  for (t in c(U64_TOP, U64_MAX)) {
    expect_true(identical(call_slot(w, u, list(number(t)), int64 = "integer64"), number(t)), paste(which, t))
  }
  e <- expect_error(echo_u64(w, i64("-1")), "guatiao_error")
  expect_equal(e$message, "wide.echo_u64(x): out of range for uint64")
}

case("an integer64 crosses a provider in Rust, C or R", with_bit64(with_examples({
  for (gr in list(list(which = "rust", key = "derived_greeter_hello"), list(which = "c", key = "c_greeter"),
    list(which = "r", key = "r_wide_gt_wide"))) {
    if (gr$which %in% PRESENT) crosses_as_integer64(HOST$bind(gr$key, Wide), gr$which)
  }
})))

case("a kind declared with int64 = \"integer64\" hands its R methods integer64s", with_bit64(with_examples({
  Wide64 <- kind("wide", floor_hash = 3673689684, class = "GtWide64", int64 = "integer64",
    w64_i64 = method(x = "int64", returns = "int64", required = TRUE, fallible = TRUE),
    w64_u64 = method(x = "uint64", returns = "uint64", required = TRUE, fallible = TRUE), where = globalenv())
  methods::setClass("GtWide64R", contains = "GtWide64", where = globalenv())
  # What no integer64 holds (INT64_MIN, past INT64_MAX) arrives as a number.
  methods::setMethod("w64_i64", "GtWide64R", function(self, x) {
    if (!inherits(x, "integer64") && !identical(x, number(I64_MIN))) stop("handed a ", class(x)[1L])
    x
  }, where = globalenv())
  methods::setMethod("w64_u64", "GtWide64R", function(self, x) {
    if (!inherits(x, "integer64") && !inherits(x, "guatiao_number")) stop("handed a ", class(x)[1L])
    x
  }, where = globalenv())
  HOST$register_library(guatiao_library("r_wide64", "0.1.0", providers = list("GtWide64R")))
  r <- HOST$bind("r_wide64_gt_wide64_r", Wide64)
  expect_true(identical(w64_i64(r, 5L), i64("5")))
  expect_true(identical(w64_i64(r, number(I64_MIN)), number(I64_MIN)))
  expect_true(identical(w64_u64(r, number(U64_MAX)), number(U64_MAX)))
  if (is.null(fixture())) {
    skip_case("no C compiler: the fixture is not built")
  } else {
    out <- value()
    st <- fx("fx_wide_caller", workspace_library("greeter_caller"), HOST$host(), "r_wide64_gt_wide64_r",
      .Call(gt$C_node_pointer, out))
    expect_equal(st, 0L)
    report <- value_data(out, numbers = "text")
    close(out)
    expect_equal(vapply(report$i64, unclass, ""), c(TWO53_1, I64_MIN, I64_MAX))
    expect_equal(vapply(report$u64, unclass, ""), c(TWO53_1, U64_TOP, U64_MAX))
  }
})))

case("values take an integer64 exactly, and read one when asked", with_bit64(with_library({
  v <- value(list(a = i64(TWO53_1), b = i64(c("1", I64_MAX)), c = bit64::NA_integer64_,
    d = stats::setNames(i64(c("2", "3")), c("x", "y"))))
  expect_equal(value_data(v, numbers = "text"),
    list(a = number(TWO53_1), b = list(number("1"), number(I64_MAX)), c = NULL, d = list(x = number("2"), y = number("3"))))
  close(v)
  expect_equal(value_data(value(lower_value(i64(I64_MAX))), numbers = "text"), number(I64_MAX))
  v <- value(list(a = number(I64_MAX), b = 5L, c = 1.5, d = number(U64_MAX), e = number(I64_MIN), f = number("1e3")))
  got <- value_data(v, numbers = "integer64")
  expect_true(identical(got$a, i64(I64_MAX)))
  expect_true(identical(got$b, i64("5")))
  expect_true(identical(got$c, 1.5))
  expect_true(identical(got$d, number(U64_MAX)), "past INT64_MAX: a number")
  expect_true(identical(got$e, -2^63), "INT64_MIN is bit64's NA: read as auto reads it, a double that holds it")
  expect_true(identical(got$f, 1000), "an exponent is no integer spelling")
  expect_true(identical(value_data(v$a), number(I64_MAX)), "auto never answers an integer64")
  close(v)
})))

case("exact lengths and spans take an integer64 unit", with_bit64(with_library({
  l <- exact_length(i64(I64_MAX), 5)
  expect_equal(l, exact_length(number(I64_MAX), 5))
  expect_true(identical(as.list(l, int64 = "integer64")$seconds, i64(I64_MAX)))
  expect_true(identical(as.list(exact_length(number(U64_MAX)), int64 = "integer64")$seconds, number(U64_MAX)))
  s <- span(years = i64(I64_MAX))
  expect_true(s == time_parse("span", paste0("P", I64_MAX, "Y")))
  expect_true(identical(as.list(s, int64 = "integer64")$years, i64(I64_MAX)))
  expect_true(identical(as.list(s)$years, number(I64_MAX)))
  e <- expect_error(exact_length(i64("-1")), "guatiao_error")
  expect_equal(e$message, "seconds: out of range for uint64")
})))
