# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# The wire encoding and channels against the shared vectors every
# implementation reads: each value both ways, each malformed input refused,
# the frames the builders write, and a conversation with every refusal.

# A NUL in the vectors' text is written \u0000, which R strings cannot
# hold: it is read as U+2400 to mark the trees R cannot build or read.
wire_nul_mark <- intToUtf8(0x2400)
wire_vectors <- function() {
  text <- readLines(repo_file("crates", "guatiao", "tests", "vectors", "wire.json"), encoding = "UTF-8", warn = FALSE)
  read_json(gsub("\\u0000", "\\u2400", text, fixed = TRUE))
}

wire_hex <- function(h) {
  h <- text_of(h)
  if (!nzchar(h)) return(raw(0))
  as.raw(strtoi(substring(h, seq(1L, nchar(h), 2L), seq(2L, nchar(h), 2L)), 16L))
}
wire_hex_of <- function(r) paste(sprintf("%02x", as.integer(r)), collapse = "")

wire_has_nul <- function(t) {
  if (is.character(t)) return(any(grepl(wire_nul_mark, t, fixed = TRUE)))
  if (is.list(t)) return(any(vapply(t, wire_has_nul, TRUE)))
  FALSE
}

# R data, as value_data(numbers = "text") reads it, written as a vector tree.
wire_tree_of <- function(d) {
  if (inherits(d, "guatiao_absent")) return(list("absent"))
  if (is.null(d)) return(list("null"))
  if (inherits(d, "guatiao_number")) return(list("number", unclass(d)))
  if (is.logical(d)) return(list("bool", d))
  if (is.character(d)) return(list("string", d))
  if (is.raw(d)) return(list("bytes", wire_hex_of(d)))
  if (!is.null(names(d))) return(list("map", unname(Map(function(k, v) list(k, wire_tree_of(v)), names(d), d))))
  list("list", lapply(d, wire_tree_of))
}

# The R data a vector tree names, for value().
wire_data_of <- function(t) {
  switch(t[[1L]],
    absent = absent,
    null = NULL,
    bool = t[[2L]],
    number = number(text_of(t[[2L]])),
    string = t[[2L]],
    bytes = wire_hex(t[[2L]]),
    list = {
      out <- list()
      for (item in t[[2L]]) out[length(out) + 1L] <- list(wire_data_of(item))
      out
    },
    map = {
      out <- structure(list(), names = character())
      for (pair in t[[2L]]) out[pair[[1L]]] <- list(wire_data_of(pair[[2L]]))
      out
    })
}

wire_number <- function(x) if (is.null(x)) 0 else as.numeric(text_of(x))

case("every value both ways", with_library(with_repo({
  vs <- wire_vectors()
  expect_true(length(vs$values) > 10L)
  for (v in vs$values) {
    bytes <- wire_hex(v$hex)
    decoded <- from_wire(bytes)
    expect_equal(to_wire(decoded), bytes, paste("re-encodes:", v$name))
    if (wire_has_nul(v$tree)) {
      # R cannot hold the text: reading it is refused, the bytes survive.
      expect_error(value_data(decoded, numbers = "text"), "guatiao_error")
    } else {
      expect_equal(wire_tree_of(value_data(decoded, numbers = "text")), v$tree, paste("decodes:", v$name))
      built <- value(wire_data_of(v$tree))
      expect_equal(to_wire(built), bytes, paste("encodes:", v$name))
      close(built)
    }
    close(decoded)
  }
  expect_equal(wire_hex_of(to_wire(list(n = number("1.5"), b = TRUE))), "0702016e0303312e3501620201")
})))

case("a child encodes alone", with_library({
  v <- value(list(outer = list(inner = list(1L, "two"))))
  back <- from_wire(to_wire(v$outer))
  expect_equal(value_data(back), list(inner = list(1L, "two")))
  close(back)
  close(v)
}))

case("every malformed input is refused", with_library(with_repo({
  for (m in wire_vectors()$malformed) {
    e <- expect_error(from_wire(wire_hex(m$hex)), "guatiao_error", m$name)
    expect_equal(e$status, "BadValue", m$name)
  }
  expect_error(from_wire("not raw"), "guatiao_error")
})))

case("the frame builders write the documented bytes", with_library(with_repo({
  for (f in wire_vectors()$frames) {
    built <- switch(f$build,
      announce = wire_announce(f$channel, wire_data_of(f$tree)),
      send = wire_send(f$channel, wire_data_of(f$tree)),
      close = wire_close(f$channel))
    expect_equal(wire_hex_of(built), text_of(f$hex), paste(f$build, text_of(f$channel)))
  }
  expect_equal(wire_hex_of(wire_close("18446744073709551615")), "02ffffffffffffffffff01")
  expect_error(wire_close(-1), "guatiao_error")
  expect_error(wire_close("18446744073709551616"), "guatiao_error")
  v <- value(list(greeting = "hello"))
  expect_equal(wire_send(7L, v), wire_send(7, list(greeting = "hello")))
  close(v)
})))

case("the conversation runs through one receiver", with_library(with_repo({
  vs <- wire_vectors()
  statuses <- c(malformed = "BadValue", no_schema = "NotFound", not_a_schema = "WrongKind", invalid = "BadValue")
  rcv <- wire_receiver()
  expect_true(rcv$is_open())
  for (step in vs$conversation) {
    frame <- wire_hex(step$frame)
    channel <- wire_number(step$channel)
    if (!is.null(step$event)) {
      ev <- rcv$accept(frame)
      expect_true(inherits(ev, "guatiao_wire_event"))
      expect_equal(ev$kind, step$event)
      expect_equal(as.numeric(ev$channel), channel)
      if (!is.null(step$tree)) {
        expect_equal(wire_tree_of(value_data(ev$value, numbers = "text")), step$tree, paste("event value", step$event))
        close(ev$value)
      } else {
        expect_true(is.null(ev$value))
      }
      next
    }
    e <- expect_error(rcv$accept(frame), paste0("guatiao_wire_", step$refusal), step$refusal)
    expect_true(inherits(e, "guatiao_wire_refused") && inherits(e, "guatiao_error"))
    expect_equal(e$status, unname(statuses[step$refusal]))
    expect_equal(e$refusal, step$refusal)
    expect_equal(as.numeric(e$channel), channel)
    expect_true(nzchar(e$message))
    expect_true(!grepl("hunter2", conditionMessage(e), fixed = TRUE))
    if (!is.null(step$kind)) expect_equal(e$kind, as.integer(wire_number(step$kind)))
    if (!is.null(step$at)) expect_equal(e$at, wire_number(step$at))
    if (!is.null(step$key)) expect_equal(e$key, step$key)
  }
  rcv$close()
  rcv$close()
  expect_true(!rcv$is_open())
  expect_error(rcv$accept(wire_close(1)), "guatiao_closed")
})))

case("a channel past 2^53 is answered exactly", with_library({
  schema <- list(type = "object")
  rcv <- wire_receiver()
  ev <- rcv$accept(wire_announce("18446744073709551615", schema))
  expect_true(inherits(ev$channel, "guatiao_number"))
  expect_equal(unclass(ev$channel), "18446744073709551615")
  close(ev$value)
  ev <- rcv$accept(wire_close(2^53))
  expect_equal(ev$channel, 2^53)
  rcv$close()
}))

case("a receiver keeps its schemas through refusals", with_library({
  rcv <- wire_receiver()
  for (i in 1:50) {
    ev <- rcv$accept(wire_announce(i, list(type = "object")))
    close(ev$value)
    e <- tryCatch(rcv$accept(wire_send(i + 100L, list(a = 1L))), guatiao_wire_no_schema = function(e) e)
    expect_true(inherits(e, "guatiao_wire_no_schema"))
  }
  ev <- rcv$accept(wire_send(50L, structure(list(), names = character())))
  expect_equal(ev$kind, "value")
  close(ev$value)
  rcv$close()
}))

case("wire bytes read as R data and written from it, with no tree", with_library(with_repo({
  for (v in wire_vectors()$values) {
    bytes <- wire_hex(v$hex)
    decoded <- from_wire(bytes)
    for (numbers in 0:2) {
      if (wire_has_nul(v$tree)) {
        expect_error(.Call(gt$C_wire_read, bytes, numbers), "guatiao_error", paste("refuses:", v$name))
        next
      }
      tree <- value_data(decoded, numbers = c("auto", "text", "double")[numbers + 1L])
      expect_true(identical(.Call(gt$C_wire_read, bytes, numbers), tree), paste("reads:", v$name, numbers))
      built <- value(tree)
      expect_true(identical(.Call(gt$C_wire_of, tree), to_wire(built)), paste("writes:", v$name, numbers))
      close(built)
    }
    close(decoded)
  }
  for (end in seq_len(6L) - 1L) {
    expect_error(.Call(gt$C_wire_read, wire_hex("0702016103")[seq_len(end)], 0L), "guatiao_error")
  }
  expect_error(.Call(gt$C_wire_read, as.raw(c(1, 1)), 0L), "guatiao_error")
  expect_error(.Call(gt$C_wire_read, as.raw(c(6, 0xff, 0xff, 0xff, 0x0f)), 0L), "guatiao_error")
})))

case("R data writes as wire bytes what it builds as a tree, refusals included", with_library({
  kept <- value(list(k = list(1L, 2L)))
  data <- list(s = "\u00fc\u20ac", n = number("1.10"), i = -3L, f = 0.1, b = TRUE, na = NA, none = NULL,
    bytes = as.raw(1:2), v = c(1.5, 2), named = c(x = 1L, y = 2L), asis = I("one"), f2 = factor("lo"),
    absent = absent, ref = kept, nested = list(list(a = list())))
  built <- value(data)
  expect_true(identical(.Call(gt$C_wire_of, data), to_wire(built)))
  close(built)
  close(kept)
  deep <- list(1)
  for (i in 1:200) deep <- list(deep)
  for (bad in list(NaN, number("1."), list(a = 1, a = 2), list(1, b = 2), deep, quote(x), sum)) {
    tree <- tryCatch({ value(bad); NULL }, error = function(e) conditionMessage(e))
    tape <- tryCatch({ .Call(gt$C_wire_of, bad); NULL }, error = function(e) conditionMessage(e))
    expect_true(identical(tape, tree), paste("refuses", deparse(bad)[1L]))
  }
}))

