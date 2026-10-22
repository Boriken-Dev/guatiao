# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# Values: building from R data, reading back, views walked fresh, writes.

case("R data becomes a value and reads back", with_library({
  v <- value(list(host = "10.0.0.1", port = 5900L, ratio = 0.25, on = TRUE, none = NULL,
    tags = c("a", "b"), one = I("x"), raw = as.raw(c(0, 255)), nested = list(list(k = 1))))
  check_equal(tag(v), "map")
  check_equal(length(v), 9)
  check_equal(names(v), c("host", "port", "ratio", "on", "none", "tags", "one", "raw", "nested"))
  d <- value_data(v)
  check_equal(d$host, "10.0.0.1")
  check_equal(d$port, 5900L)
  check_equal(d$ratio, 0.25)
  check(isTRUE(d$on))
  check(is.null(d$none) && "none" %in% names(d))
  check_equal(d$tags, list("a", "b"))
  check_equal(d$one, list("x"))
  check_equal(d$raw, as.raw(c(0, 255)))
  check_equal(d$nested, list(list(k = 1L)))
  check_equal(tag(v$none), "null")
  check(is_null(v$none))
  check(is.null(v$nonesuch))
  check_equal(as.vector(v$port), 5900L)
  check_equal(as.character(v$port), "5900")
  check_equal(as.numeric(v$ratio), 0.25)
  check_equal(as.list(v$tags), list("a", "b"))
  close(v)
}))

case("numbers keep their exact text", with_library({
  v <- value(list(big = number("18446744073709551615"), frac = number("1.10"), int = 3000000000,
    neg = -2147483647L, exp = number("1e400"), close = number("9007199254740993"),
    held = number("9007199254740992"), negzero = -0))
  check_equal(as.character(v$big), "18446744073709551615")
  check(inherits(value_data(v$big), "guatiao_number"))
  check_equal(value_data(v$frac), 1.1)
  check_equal(value_data(v$frac, numbers = "text"), number("1.10"))
  check_equal(value_data(v$int), 3e9)
  check_equal(value_data(v$neg), -2147483647L)
  check(inherits(value_data(v$exp), "guatiao_number"))
  check(inherits(value_data(v$close), "guatiao_number"))
  check_equal(value_data(v$held), 9007199254740992)
  check_equal(value_data(v$big, numbers = "double"), 18446744073709551615)
  check_equal(serde_emit(v), '{"big":18446744073709551615,"frac":1.10,"int":3000000000,"neg":-2147483647,"exp":1e400,"close":9007199254740993,"held":9007199254740992,"negzero":0}')
  check_equal(value_data(v$negzero, numbers = "text"), number("-0"))
  check_equal(serde_emit(list(a = 0.1, b = 1/3, c = 2^1000, d = 2^60)), '{"a":0.1,"b":0.3333333333333333,"c":1.0715086071862673e+301,"d":1152921504606846976}')
  check_error(value(NaN), "guatiao_error")
  check_error(value(Inf), "guatiao_error")
  check_error(value(number("1.2.3")), "guatiao_error")
  close(v)
}))

case("absent, null and NA", with_library({
  check(is_absent(value()))
  check(is_absent(absent))
  check_equal(tag(value(NULL)), "null")
  check_equal(serde_emit(list(a = NA, b = NA_integer_, c = NA_real_, d = NA_character_)), '{"a":null,"b":null,"c":null,"d":null}')
  check(inherits(value_data(value()), "guatiao_absent"))
}))

case("a list is named in full or not at all", with_library({
  check_error(value(list(a = 1, 2)), "guatiao_error")
  check_error(value(list(a = 1, a = 2)), "guatiao_error")
  check_equal(tag(value(structure(list(), names = character()))), "map")
  check_equal(tag(value(list())), "list")
  check_equal(serde_emit(c(x = 1, y = 2)), '{"x":1,"y":2}')
  check_equal(serde_emit(factor(c("lo", "hi"))), '["lo","hi"]')
  check_error(value(data.frame(a = 1)), "guatiao_error")
  check_error(value(new.env()), "guatiao_error")
}))

case("views are walked again on every access", with_library({
  v <- value(list(a = list(1, 2, 3), m = list(x = "y")))
  second <- v$a[[2]]
  check_equal(node_path(second), "a[1]")
  check_equal(value_data(second), 2L)
  delete(v$a, 1)
  check_equal(value_data(second), 3L)
  delete(v$a, 1)
  check_error(value_data(second), "guatiao_error")
  check_equal(node_path(v$m$x), "m.x")
  check_error(v$m[[5]], "guatiao_error")
  check_error(v$a$x, "guatiao_error")
  close(v)
  check(!is_open(v))
  check_error(value_data(v$a), "guatiao_closed")
  check_error(v$a, "guatiao_closed")
}))

case("writes: set, push, insert, delete, clear, merge", with_library({
  v <- value(list(port = 1L, tags = list("b")))
  v$port <- 5901L
  v[["host"]] <- "h"
  push(v$tags, "c", "d")
  insert(v$tags, 1, "a")
  v$tags[[2]] <- "B"
  v$tags[[5]] <- "e"
  check_equal(value_data(v), list(port = 5901L, tags = list("a", "B", "c", "d", "e"), host = "h"))
  v[[1]] <- 7L
  check_equal(value_data(v$port), 7L)
  delete(v, "host")
  delete(v, "nonesuch")
  delete(v$tags, 9)
  check_equal(names(v), c("port", "tags"))
  merge_from(v, list(port = 8L, extra = TRUE))
  check_equal(value_data(v), list(port = 8L, tags = list("a", "B", "c", "d", "e"), extra = TRUE))
  v$copy <- v$tags
  check_equal(value_data(v$copy), value_data(v$tags))
  v$tags[[1]] <- v$tags
  check_equal(length(v$tags[[1]]), 5)
  clear(v$tags)
  check_equal(length(v$tags), 0)
  check_error(push(v, 1), "guatiao_error")
  check_error(v$port$x <- 1, "guatiao_error")
  close(v)
}))

case("a copy is its own value", with_library({
  v <- value(list(a = list(1)))
  w <- value(v$a)
  push(w, 2)
  check_equal(length(v$a), 1)
  check_equal(length(w), 2)
  close(v)
  check_equal(value_data(w), list(1L, 2L))
  close(w)
}))

case("strings are UTF-8 both ways", with_library({
  v <- value(list("é" = "héllo 世界"))
  check_equal(names(v), "é")
  check_equal(value_data(v)[["é"]], "héllo 世界")
  check_equal(Encoding(value_data(v)[[1]]), "UTF-8")
  latin <- iconv("café", "UTF-8", "latin1")
  check_equal(value_data(value(latin)), "café")
  close(v)
}))

case("writing a class goes through lower_value", with_library({
  setClass("GtPoint", slots = c(x = "numeric", y = "numeric"), where = globalenv())
  check_equal(serde_emit(new("GtPoint", x = 1, y = 2)), '{"x":1,"y":2}')
  registerS3method("lower_value", "gt_celsius", function(x) list(celsius = unclass(x)), envir = gt)
  check_equal(serde_emit(structure(21.5, class = "gt_celsius")), '{"celsius":21.5}')
  check_error(value(structure(1, class = "gt_unknown")), "guatiao_error")
}))
