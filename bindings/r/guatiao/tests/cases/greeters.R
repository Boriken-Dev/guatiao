# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# One behaviour, many languages: an R host calls the greeters written in
# Rust, C, Python, Go and R through one description of the kind, and a
# Rust caller calls each of them through the kind's Rust proxy.

GREETERS <- list(
  list(which = "rust", key = "derived_greeter_hello", greeting = "hello, ana"),
  list(which = "c", key = "c_greeter", greeting = "hello from C, ana"),
  list(which = "python", key = "py_greeter_hello", greeting = "hello, ana"),
  list(which = "go", key = "go_greeter_hello", greeting = "hello, ana"),
  list(which = "r", key = "rgreeter_hello", greeting = "hello, ana"))

# A listener: what a conversation tells, kept; closed when let go.
ear <- function() {
  e <- new.env()
  e$heard <- character()
  e$closed <- FALSE
  methods::new("GtListener", state = e)
}

PRESENT <- character()
HOST <- NULL

case("the kinds copy their floor hashes and sizes from the header", with_examples({
  header <- paste(readLines(repo_file("examples", "greeter_kind", "include", "greeter_kind.h"), warn = FALSE), collapse = "\n")
  for (pair in list(list("greeter_vtable", Greeter), list("counter_vtable", Counter),
    list("conversation_vtable", Conversation), list("listener_vtable", Listener), list("wide_vtable", Wide))) {
    table <- pair[[1]]
    k <- pair[[2]]
    found <- regmatches(header, regexec(sprintf("#define %s_FLOOR_HASH ([0-9]+)", table), header))[[1]]
    expect_equal(as.numeric(found[2]), k$floor_hash, paste(table, "floor hash"))
    body <- regmatches(header, regexec(sprintf("typedef struct %s \\{(.*?)\\} %s;", table, table), header))[[1]][2]
    slots <- length(gregexpr("\\(\\*\\w+\\)", body)[[1]])
    expect_equal(gt$.gt_kind_sizes(k)[["full"]], 8 + .Machine$sizeof.pointer * slots, paste(table, "size"))
  }
}))

case("one refresh loads every greeter its sources serve", with_examples({
  methods::setClass("GtListener", contains = "Listener", slots = c(state = "environment"), where = globalenv())
  methods::setMethod("heard", "GtListener", function(self, what) {
    self@state$heard <- c(self@state$heard, what)
    invisible(NULL)
  }, where = globalenv())
  registerS3method("close", "GtListener", function(con, ...) con@state$closed <- TRUE)
  HOST <- registry("r-greeters", "0.1")
  # Every greeter written in another language comes through a source the
  # registry keeps: the workspace's directory and the C and Go greeters'
  # as watched directories, the Python greeters through the guatiao-source-py bridge.
  HOST$watch_dir(LIBRARY_DIR, rules = "kind=greeter")
  PRESENT <- c(PRESENT, "rust")
  c_dir <- build_c_greeter()
  if (is.null(c_dir)) {
    absent_greeter("c", "no C compiler: the greeter written in C is not built")
  } else {
    HOST$watch_dir(c_dir, rules = "kind=greeter")
    PRESENT <- c(PRESENT, "c")
  }
  go <- Sys.getenv("GUATIAO_GO_LIBRARY")
  if (!nzchar(go)) {
    absent_greeter("go", "GUATIAO_GO_LIBRARY names no directory holding the Go greeter built with -buildmode=c-shared")
  } else {
    HOST$watch_dir(go, rules = "kind=greeter")
    PRESENT <- c(PRESENT, "go")
  }
  py <- bridge()
  if (is.null(py)) {
    absent_greeter("python", "no guatiao-source-py build: set GUATIAO_PY_BRIDGE")
  } else {
    HOST$add_source_file(py, bridge_config())
    PRESENT <- c(PRESENT, "python")
  }
  refreshed <- HOST$refresh()
  expect_equal(length(refreshed$refused), 0)
  loaded <- vapply(refreshed$loaded, function(s) s$key, "")
  expect_true("derived_greeter" %in% loaded)
  if ("c" %in% PRESENT) expect_true("c_greeter_library" %in% loaded)
  if ("python" %in% PRESENT) expect_true("py_greeter" %in% loaded)
  if ("go" %in% PRESENT) expect_true(any(endsWith(loaded, "go_greeter")))
  HOST$register_library(library_of("rgreeter"))
  PRESENT <- c(PRESENT, "r")
}))

exercise <- function(gr) {
  g <- HOST$bind(gr$key, Greeter)
  expect_true(is(g, "Greeter"))
  answer <- greet(g, "ana")
  expect_equal(value_data(answer), list(greeting = gr$greeting))
  close(answer)
  e <- expect_error(greet(g, ""), "guatiao_error")
  expect_true(identical(e$status, "BadValue") && grepl("nobody to greet", e$message), paste(gr$which, "refuses nobody"))
  if (gr$which == "c") {
    close(greet(g, "panic")) # the greeter written in C greets anyone
  } else {
    e <- expect_error(greet(g, "panic"), "guatiao_error")
    expect_equal(e$status, "Internal")
  }
  config <- value(list(name = "bo"))
  expect_true(isTRUE(check(g, config)), paste(gr$which, "check"))
  expect_equal(value_data(config$name), "bo")
  expect_equal(length(config), 2)
  close(config)
  handed <- value(list(name = "ana"))
  answer <- greet_from(g, handed)
  expect_equal(value_data(answer), list(greeting = gr$greeting))
  expect_true(is_absent(handed))
  close(answer)
  close(handed)
  e1 <- ear()
  greet_into(g, "ana", e1)
  expect_equal(e1@state$heard, gr$greeting)
  if (has_slot(g, "shout")) {
    expect_equal(shout(g, "ana"), toupper(gr$greeting))
  } else {
    expect_error(shout(g, "ana"), "guatiao_error")
  }
  if (has_slot(g, "start")) {
    listener <- ear()
    chat <- start(g, listener)
    say(chat, "hi")
    say(chat, "again")
    e <- expect_error(say(chat, ""), "guatiao_error")
    expect_equal(e$status, "BadValue")
    buf <- byte_buffer(64)
    n <- read(chat, buf)
    expect_equal(rawToChar(as.raw(buf)[seq_len(n)]), "hi\nagain")
    expect_equal(turns(chat), 2)
    expect_equal(listener@state$heard, c("hi", "again"))
    close(chat)
    expect_true(isTRUE(listener@state$closed), paste(gr$which, "listener let go"))
  } else {
    expect_equal(gr$which, "c")
  }
}

for (gr in GREETERS) {
  case(paste("an R host calls every greeter:", gr$which), with_examples({
    if (gr$which %in% PRESENT) exercise(gr)
  }))
}

# Each of these unloads a library of its own: no other registry keeps it mapped.
case("an answer built in the library's allocator outlives the unload", with_examples({
  dir <- build_c_greeter("GREETER_IGNORES_CARRIER")
  if (is.null(dir)) {
    absent_greeter("c", "no C compiler: the greeter written in C is not built")
  } else {
    own <- registry("r-careless", "0.1")
    own$scan_dir(dir, rules = "kind=greeter")
    answer <- greet(own$bind("c_greeter", Greeter), "hal")
    # The C greeter has no `unload` function: nothing it handed out is
    # alive, which the caller states.
    own$unload("c_greeter_library", unchecked = TRUE)
    expect_equal(length(own$libraries()), 0)
    expect_equal(value_data(answer), list(greeting = "hello from C, hal"))
    close(answer)
    own$close()
  }
}))

case("what a slot grew a lent map with outlives the unload", with_examples({
  # This greeter's `check` rebuilds the lent map in its own allocator.
  dir <- build_c_greeter("GREETER_IGNORES_CARRIER")
  if (is.null(dir)) {
    absent_greeter("c", "no C compiler: the greeter written in C is not built")
  } else {
    own <- registry("r-lent", "0.1")
    own$scan_dir(dir, rules = "kind=greeter")
    config <- value(list(name = "ivy"))
    expect_true(isTRUE(check(own$bind("c_greeter", Greeter), config)))
    own$unload("c_greeter_library", unchecked = TRUE)
    expect_equal(value_data(config), list(name = "ivy", checked_by = "C"))
    close(config)
    own$close()
  }
}))

case("a refused move of a lent tree is the call's error", with_examples({
  # `check` rebuilds the lent map in its own allocator; moving it back into
  # the map's, which refuses, fails the call and leaves the map whole where
  # it stands.
  dir <- build_c_greeter("GREETER_IGNORES_CARRIER")
  if (is.null(dir)) {
    absent_greeter("c", "no C compiler: the greeter written in C is not built")
  } else {
    own <- registry("r-refused", "0.1")
    own$scan_dir(dir, rules = "kind=greeter")
    a <- counting()
    config <- value(list(name = "ivy"), alloc = a)
    g <- own$bind("c_greeter", Greeter)
    refuse(a, TRUE)
    refused <- tryCatch(check(g, config), error = function(e) e)
    refuse(a, FALSE)
    expect_true(inherits(refused, "guatiao_error") && identical(refused$status, "AllocFailed"))
    expect_true(grepl("lent tree", conditionMessage(refused), fixed = TRUE))
    expect_equal(value_data(config), list(name = "ivy", checked_by = "C"))
    close(config)
    expect_equal(live(a), 0)
    own$close()
  }
}))

case("a key a slot wrote into a lent map that is not UTF-8 is removed", with_examples({
  # `check` adds a key that is not UTF-8 to the lent map: removed and freed
  # when the call returns, and the call's error.
  dir <- build_c_greeter("GREETER_WRITES_BAD_KEY")
  if (is.null(dir)) {
    absent_greeter("c", "no C compiler: the greeter written in C is not built")
  } else {
    own <- registry("r-bad-key", "0.1")
    own$scan_dir(dir, rules = "kind=greeter")
    a <- counting()
    config <- value(list(name = "ivy"), alloc = a)
    refused <- tryCatch(check(own$bind("c_greeter", Greeter), config), error = function(e) e)
    expect_true(inherits(refused, "guatiao_error") && identical(refused$status, "BadValue"))
    expect_true(grepl("not UTF-8", conditionMessage(refused), fixed = TRUE))
    expect_equal(value_data(config), list(name = "ivy"))
    close(config)
    expect_equal(live(a), 0)
    own$close()
  }
}))

case("the Rust proxy calls every greeter", with_examples({
  if (is.null(fixture())) {
    skip_case("no C compiler: the fixture is not built")
  } else {
    for (gr in GREETERS) {
      if (!(gr$which %in% PRESENT)) next
      out <- value()
      st <- fx("fx_greeter_caller", workspace_library("greeter_caller"), HOST$host(), gr$key, "ana",
        .Call(gt$C_node_pointer, out))
      expect_equal(st, 0L, paste(gr$which, "greeter_caller_run"))
      report <- value_data(out)
      close(out)
      expect_equal(report$greet, gr$greeting)
      expect_equal(report$refusal$status, 7L)
      expect_equal(report$failure, if (gr$which == "c") 0L else 100L)
      expect_equal(report$shout, toupper(gr$greeting))
      expect_true(isTRUE(report$checked))
      expect_equal(length(report$filled), 2)
      expect_equal(report$greet_from, gr$greeting)
      expect_equal(report$heard, list(gr$greeting))
      if (gr$which == "c") {
        # No `start` in its table: the Rust proxy runs the kind's default.
        expect_equal(report$conversation, list(status = 11L, message = "this greeter holds no conversations"))
      } else {
        expect_equal(report$conversation$turns, 2L, paste(gr$which, "turns"))
        expect_equal(report$conversation$transcript, "hi\nagain")
        expect_equal(report$conversation$heard, list("hi", "again"))
        expect_true(isTRUE(report$conversation$dropped))
      }
    }
  }
}))

case("a table that is not the kind's is refused before it runs", with_examples({
  t <- HOST$provider_table("rgreeter_hello", "counter")
  expect_error(gt$.gt_check_floor(Greeter, t), "guatiao_floor_mismatch")
  expect_error(HOST$bind("c_greeter", Counter), "guatiao_error")
}))

case("nothing an R greeter served is left once its holders are done", with_examples({
  before <- live_objects()
  g <- HOST$bind("rgreeter_hello", Greeter)
  chat <- start(g, ear())
  expect_equal(live_objects(), before + 2) # the conversation, and the listener it holds
  close(chat)
  expect_equal(live_objects(), before)
}))
