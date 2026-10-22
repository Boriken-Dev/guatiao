# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# 64-bit integers, which an R number holds exactly only to 2^53, cross
# exactly both ways: through a slot of a provider written in Rust, C or R,
# from Rust into an R provider, and in exact lengths and spans.

TWO53_1 <- "9007199254740993"
I64_MIN <- "-9223372036854775808"
I64_MAX <- "9223372036854775807"
U64_TOP <- "9223372036854775808"
U64_MAX <- "18446744073709551615"

# The vectors, each crossed in every spelling an argument takes.
crosses_exactly <- function(w, which) {
  for (t in c(TWO53_1, I64_MIN, I64_MAX)) {
    for (arg in list(number(t), t)) {
      got <- echo_i64(w, arg)
      expect_equal(got, number(t), paste(which, "echo_i64", t))
    }
  }
  for (t in c(TWO53_1, I64_MAX, U64_TOP, U64_MAX)) {
    expect_equal(echo_u64(w, number(t)), number(t), paste(which, "echo_u64", t))
    expect_equal(echo_u64(w, t), number(t), paste(which, "echo_u64 text", t))
  }
  # What fits an R integer answers one; what a double holds, a double.
  expect_equal(echo_i64(w, 5L), 5L)
  expect_equal(echo_i64(w, -2147483647L), -2147483647L)
  expect_true(identical(echo_i64(w, -2147483648), -2147483648), paste(which, "INT_MIN is NA in R: a double"))
  expect_true(identical(echo_i64(w, 2^53), 2^53), paste(which, "2^53 a double"))
  expect_true(identical(echo_u64(w, 2147483648), 2147483648))
  expect_true(identical(echo_i64(w, TRUE), 1L))
  # The option, on the call.
  i <- match("echo_i64", names(Wide$slots))
  expect_equal(call_slot(w, i, list(5L), int64 = "number"), number("5"))
  expect_true(identical(call_slot(w, i, list(number(I64_MAX)), int64 = "double"), 2^63))
  expect_error(call_slot(w, i, list(5L), int64 = "nonesuch"))
}

refuses_with_the_slot_named <- function(w, which) {
  for (bad in list(1.5, NA, NA_integer_, "1.5", "12a", "", " 5", "+5", number("1e3"), c(1, 2), list(1), Inf)) {
    e <- expect_error(echo_i64(w, bad), "guatiao_error", paste(which, "refuses", deparse(bad)))
    expect_true(identical(e$status, "BadValue") && startsWith(e$message, "wide.echo_i64(x): "),
      paste(which, "names the slot:", e$message))
  }
  for (bad in list(number("9223372036854775808"), "-9223372036854775809", 2^63, number("99999999999999999999999"))) {
    e <- expect_error(echo_i64(w, bad), "guatiao_error")
    expect_equal(e$message, "wide.echo_i64(x): out of range for int64")
  }
  for (bad in list(-1L, number("-1"), number("18446744073709551616"), 2^64)) {
    e <- expect_error(echo_u64(w, bad), "guatiao_error")
    expect_equal(e$message, "wide.echo_u64(x): out of range for uint64")
  }
}

case("a slot of a provider in Rust or C carries 64-bit integers exactly", with_examples({
  for (gr in list(list(which = "rust", key = "derived_greeter_hello"), list(which = "c", key = "c_greeter"))) {
    if (!(gr$which %in% PRESENT)) next
    w <- HOST$bind(gr$key, Wide)
    crosses_exactly(w, gr$which)
    refuses_with_the_slot_named(w, gr$which)
  }
}))

# A provider written in R: what it is handed arrives as the kind's int64
# option answers it, and it answers in any spelling an argument takes.
WIDE_SEEN <- list()

case("an R provider takes and answers 64-bit integers exactly", with_examples({
  methods::setClass("GtWide", contains = "Wide", where = globalenv())
  methods::setMethod("echo_i64", "GtWide", function(self, x) {
    WIDE_SEEN[[length(WIDE_SEEN) + 1L]] <<- x
    if (identical(x, 7L)) return("not a number")
    x
  }, where = globalenv())
  methods::setMethod("echo_u64", "GtWide", function(self, x) {
    if (identical(x, 7L)) return(number("18446744073709551616"))
    if (identical(x, 8L)) return(U64_MAX)
    x
  }, where = globalenv())
  HOST$register_library(guatiao_library("r_wide", "0.1.0", providers = list("GtWide")))
  w <- HOST$bind("r_wide_gt_wide", Wide)
  crosses_exactly(w, "r")
  refuses_with_the_slot_named(w, "r")
  # It was handed what int64 = "auto" answers.
  WIDE_SEEN <- list()
  echo_i64(w, 5L)
  echo_i64(w, number(I64_MAX))
  expect_true(identical(WIDE_SEEN, list(5L, number(I64_MAX))))
  # An answer is read as an argument is, and refused naming the slot.
  e <- expect_error(echo_i64(w, 7L), "guatiao_error")
  expect_true(identical(e$status, "BadValue") && grepl("wide.echo_i64 answered: want a whole number", e$message), e$message)
  e <- expect_error(echo_u64(w, 7L), "guatiao_error")
  expect_true(grepl("wide.echo_u64 answered: out of range for uint64", e$message), e$message)
  expect_equal(echo_u64(w, 8L), number(U64_MAX))
}))

case("Rust calls a wide provider in R, Rust and C, exactly", with_examples({
  if (is.null(fixture())) {
    skip_case("no C compiler: the fixture is not built")
  } else {
    for (gr in list(list(which = "r", key = "r_wide_gt_wide"), list(which = "rust", key = "derived_greeter_hello"),
      list(which = "c", key = "c_greeter"))) {
      if (!(gr$which %in% PRESENT)) next
      out <- value()
      st <- fx("fx_wide_caller", workspace_library("greeter_caller"), HOST$host(), gr$key, .Call(gt$C_node_pointer, out))
      expect_equal(st, 0L, paste(gr$which, "greeter_caller_wide"))
      report <- value_data(out, numbers = "text")
      close(out)
      expect_equal(vapply(report$i64, unclass, ""), c(TWO53_1, I64_MIN, I64_MAX), paste(gr$which, "i64"))
      expect_equal(vapply(report$u64, unclass, ""), c(TWO53_1, U64_TOP, U64_MAX), paste(gr$which, "u64"))
    }
  }
}))

case("a kind's int64 option is how its remotes answer and its methods are handed", with_examples({
  # The same table under other R names, so its generics mask nobody's.
  Wider <- kind("wide", floor_hash = 3673689684, class = "GtWider", int64 = "number",
    wider_i64 = method(x = "int64", returns = "int64", required = TRUE, fallible = TRUE),
    wider_u64 = method(x = "uint64", returns = "uint64", required = TRUE, fallible = TRUE), where = globalenv())
  w <- HOST$bind("derived_greeter_hello", Wider)
  expect_equal(wider_i64(w, 5L), number("5"))
  expect_equal(wider_u64(w, 5L), number("5"))
  expect_true(identical(call_slot(w, 1L, list(5L), int64 = "auto"), 5L))
  methods::setClass("GtWiderR", contains = "GtWider", where = globalenv())
  methods::setMethod("wider_i64", "GtWiderR", function(self, x) {
    if (!inherits(x, "guatiao_number")) stop("handed a ", class(x)[1L])
    x
  }, where = globalenv())
  methods::setMethod("wider_u64", "GtWiderR", function(self, x) x, where = globalenv())
  HOST$register_library(guatiao_library("r_wider", "0.1.0", providers = list("GtWiderR")))
  r <- HOST$bind("r_wider_gt_wider_r", Wider)
  expect_equal(wider_i64(r, 5L), number("5"))
  expect_equal(echo_i64(HOST$bind("r_wider_gt_wider_r", Wide), 5L), 5L)
  expect_error(kind("bad", floor_hash = 1, int64 = "text", where = globalenv()))
}))

case("exact lengths past 2^53", with_library({
  for (t in c(TWO53_1, I64_MAX, U64_MAX)) {
    for (s in list(number(t), t)) {
      l <- exact_length(s, 5)
      expect_true(identical(unclass(l), c(seconds = t, nanoseconds = "5")), paste("held as text", t))
      expect_equal(format(l), paste0(t, ".000000005s"))
      v <- value(l)
      expect_equal(value_data(v, numbers = "text"), list(number(t), number("5")))
      expect_equal(read_length(v), l)
      close(v)
      expect_equal(read_length(list(number(t), 5L)), l)
      expect_equal(as.list(l), list(seconds = number(t), nanoseconds = 5L))
      expect_equal(as.list(l, int64 = "double")$seconds, as.numeric(t))
    }
  }
  # A length within 2^53 is held as numbers.
  expect_true(identical(unclass(exact_length(number("90"), 5e8)), c(seconds = 90, nanoseconds = 5e8)))
  expect_true(identical(exact_length(2^53), exact_length(number("9007199254740992"))))
  expect_equal(as.list(exact_length(90, 5e8)), list(seconds = 90L, nanoseconds = 500000000L))
  expect_equal(read_length(number(paste0(U64_MAX, ".5"))), exact_length(number(U64_MAX), 5e8))
  for (bad in list(-1, number("18446744073709551616"), "1.5", NA)) {
    e <- expect_error(exact_length(bad), "guatiao_error")
    expect_true(startsWith(e$message, "seconds: "), e$message)
  }
  expect_error(exact_length(1, 1e9))
  expect_error(read_length(number("18446744073709551616")), "guatiao_error")
}))

case("span units past 2^53", with_library({
  s <- time_parse("span", paste0("P", I64_MAX, "Y"))
  expect_true(identical(unname(unclass(s))[1L], I64_MAX), "held as text")
  expect_equal(format(s), paste0("P", I64_MAX, "Y"))
  expect_equal(format(-s), paste0("-P", I64_MAX, "Y"))
  expect_true(-(-s) == s)
  expect_true(s == span(years = number(I64_MAX)))
  expect_true(s == span(years = I64_MAX))
  expect_true(s != span(years = number("9223372036854775806")))
  expect_equal(as.list(s)$years, number(I64_MAX))
  expect_equal(as.list(s)$days, 0L)
  d <- time_parse("span", paste0("P", TWO53_1, "DT1S"))
  expect_equal(format(d), paste0("P", TWO53_1, "DT1S"))
  expect_equal(time_normalize("span", format(d)), format(d))
  v <- value(list(s = d))
  expect_equal(value_data(v$s), format(d))
  close(v)
  # Within 2^53 a span holds numbers, equal however it was made.
  expect_true(identical(unclass(span(days = number("2"))), unclass(span(days = 2))))
  expect_true(span(days = 2^53) == time_parse("span", "P9007199254740992D"))
  # INT64_MIN is no unit (the library refuses it); a unit's refusal names it.
  expect_error(span(years = number(I64_MIN)), "guatiao_error")
  e <- expect_error(span(days = "1.5"), "guatiao_error")
  expect_true(startsWith(e$message, "days: "), e$message)
  e <- expect_error(span(days = number("9223372036854775808")), "guatiao_error")
  expect_equal(e$message, "days: out of range for int64")
  # Applying one far past any year 9999 is refused by the library.
  expect_error(add_span(as.Date("2026-01-01"), s), "guatiao_error")
}))
