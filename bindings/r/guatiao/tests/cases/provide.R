# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# A library written in R: its package is the library, its classes the
# providers. What a provider answers is built in the allocator its caller
# carried in, a lent tree grows in the allocator it records, and what it
# was handed is freed.

PREG <- NULL
KEPT <- new.env()

case("a package is its library", with_examples({
  lib <- library_of("rgreeter")
  expect_equal(lib$id, "rgreeter")
  expect_equal(lib$version, "1.0.0")
  expect_equal(vapply(lib$providers, function(p) p$name, ""), c("hello", "shouter"))
  expect_equal(lib$provider_ids, c("rgreeter_hello", "rgreeter_shouter"))
  expect_equal(lib$meta$authors, list("jose-pr <jose-pr@coqui.dev>"))
  expect_equal(lib$meta$license, "MPL-2.0")
  expect_equal(lib$meta$summary, "Greeters Written in R")
  expect_true(startsWith(lib$meta$description, "Greeters written in R"))
  expect_equal(lib$meta$urls$homepage, "https://github.com/Boriken-Dev/guatiao")
  expect_equal(lib$meta$notices[[1]]$component, "rgreeter")
  expect_true(grepl("Mozilla Public", lib$meta$notices[[1]]$text))
  expect_true(identical(library_of("rgreeter"), lib))
  expect_error(library_of(new.env()))
  PREG <- registry("r-provide", "0.1")
  PREG$register_library(lib)
  p <- PREG$provider("rgreeter_hello")
  expect_equal(p$display_name, "Hello, in R")
  expect_equal(sort(unlist(p$kinds)), c("counter", "greeter"))
  expect_true(isTRUE(PREG$provider("rgreeter_shouter")$has_config))
  schema <- PREG$provider_config("rgreeter_shouter")
  expect_equal(value_data(schema$required), list("prefix"))
  expect_equal(value_data(schema$properties$prefix$minLength), 1L)
  expect_equal(value_data(schema$properties$prefix$description), "What every greeting starts with.")
  close(schema)
  expect_true(is.null(PREG$provider_config("rgreeter_hello")))
}))

case("kinds and providers declared in a script", with_examples({
  Pair <- kind("pair", floor_hash = 7, both = method(a = taken("map"), b = taken("map"), returns = "int64", required = TRUE),
    where = globalenv())
  Mirror <- kind("mirror", floor_hash = 8, show_me = method(returns = "map", required = TRUE), where = globalenv())
  methods::setClass("GtSizes", contains = "Pair", where = globalenv())
  methods::setMethod("both", "GtSizes", function(self, a, b) {
    n <- length(a) + length(b)
    close(a)
    close(b)
    n
  }, where = globalenv())
  methods::setClass("GtBuilt", contains = "Mirror", where = globalenv())
  methods::setMethod("show_me", "GtBuilt", function(self) value(list(built = "here")), where = globalenv())
  methods::setClass("GtMissing", contains = "Pair", where = globalenv())
  methods::setClass("GtPicky", contains = "Greeter", where = globalenv())
  methods::setMethod("greet", "GtPicky", function(self, name) list(greeting = name), where = globalenv())
  expect_true(inherits(Pair, "guatiao_kind"))
  expect_error(kind("bad", floor_hash = 1, close = method(), object = TRUE, where = globalenv()))
  expect_error(kind("bad", floor_hash = 1, f = method(x = "nonesuch"), where = globalenv()))
  expect_error(kind("bad", floor_hash = 1, f = method(x = taken(Greeter)), where = globalenv()))
  expect_error(kind("bad", floor_hash = 1, f = method(), g = method(required = TRUE), where = globalenv()))
}))

case("a provider is checked when its library is made", with_examples({
  expect_error(guatiao_library("lib", "1", providers = list("GtMissing")))
  expect_error(provider("GtListener", where = NULL)) # serves no kind
  expect_error(provider("GtPicky", serves = list(Listener), where = NULL)) # an object kind
  expect_error(guatiao_library("lib", "1", providers = list("GtPicky", provider("GtPicky", where = NULL))))
}))

case("what a provider answers lands in the caller's allocator", with_examples({
  if (is.null(fixture())) {
    skip_case("no C compiler: the fixture is not built")
  } else {
    c <- counting()
    t <- PREG$provider_table("rgreeter_hello", "greeter")
    lib <- core_library()
    got <- fx("fx_greet_into_alloc", lib, t$table, t$ctx, "ana", c)
    expect_equal(got, c(0L, 1L))
    expect_equal(live(c), 0)
    got <- fx("fx_greet_into_alloc", lib, t$table, t$ctx, "", c)
    expect_equal(got, c(7L, 0L))
    # A value the method built elsewhere is copied into the carrier's allocator.
    PREG$register_library(guatiao_library("r_mirror", "0.1.0", providers = list("GtBuilt")))
    m <- PREG$bind("r_mirror_gt_built", Mirror)
    answer <- show_me(m)
    expect_equal(value_data(answer), list(built = "here"))
    close(answer)
  }
}))

case("a lent map grows in the allocator it records", with_examples({
  c <- counting()
  config <- value(list(name = "bo"), alloc = c)
  before <- live(c)
  expect_true(isTRUE(check(PREG$bind("rgreeter_hello", Greeter), config)))
  expect_true(live(c) > before)
  expect_equal(value_data(config), list(name = "bo", greeting = "hello, bo"))
  close(config)
  expect_equal(live(c), 0)
}))

case("a map handed over is freed, whatever the provider answers", with_examples({
  c <- counting()
  g <- PREG$bind("rgreeter_hello", Greeter)
  config <- value(list(name = "ana"), alloc = c)
  close(greet_from(g, config))
  expect_equal(live(c), 0)
  refused <- value(list(name = ""), alloc = c)
  expect_error(greet_from(g, refused), "guatiao_error")
  invisible(gc())
  expect_equal(live(c), 0)
}))

case("a map handed over and kept is all the provider's own", with_examples({
  # Handed over in the caller's allocator, it is moved into this binding's
  # before the method keeps it.
  methods::setClass("GtKeeper", contains = "Greeter", where = globalenv())
  methods::setMethod("greet", "GtKeeper", function(self, name) list(greeting = name), where = globalenv())
  methods::setMethod("greet_from", "GtKeeper", function(self, config) {
    KEPT$maps <- c(KEPT$maps, list(config))
    list(greeting = "kept")
  }, where = globalenv())
  KEPT$maps <- list()
  reg <- registry("r-keeper", "0.1")
  reg$register_library(guatiao_library("r_keeper", "0.1.0", providers = list("GtKeeper")))
  c <- counting()
  answer <- greet_from(reg$bind("r_keeper_gt_keeper", Greeter), value(list(name = "ivy"), alloc = c))
  expect_equal(value_data(answer), list(greeting = "kept"))
  close(answer)
  expect_equal(live(c), 0)
  expect_equal(length(KEPT$maps), 1L)
  expect_equal(value_data(KEPT$maps[[1]]), list(name = "ivy"))
  close(KEPT$maps[[1]])
  KEPT$maps <- list()
  reg$close()
}))

case("nothing crosses when an argument is refused before the call", with_examples({
  c <- counting()
  PREG$register_library(guatiao_library("r_pair", "0.1.0", providers = list("GtSizes")))
  p <- PREG$bind("r_pair_gt_sizes", Pair)
  first <- value(list(a = 1L), alloc = c)
  expect_error(both(p, first, list(1L)), "guatiao_error")
  expect_equal(value_data(first), list(a = 1L))
  expect_equal(both(p, first, list(b = 2L, c = 3L)), 3)
  expect_true(is_absent(first))
  expect_equal(live(c), 0)
}))

case("an instance per configuration, closed when its host destroys it", with_examples({
  CLOSED <- 0
  config_class("GtClosingConfig", label = field("character", default = "x"), where = globalenv())
  methods::setClass("GtClosing", contains = "Greeter", slots = c(config = "GtClosingConfig"), where = globalenv())
  methods::setMethod("greet", "GtClosing", function(self, name) list(greeting = name), where = globalenv())
  registerS3method("close", "GtClosing", function(con, ...) CLOSED <<- CLOSED + 1)
  lib <- guatiao_library("r_closing", "0.1.0", providers = list(provider("GtClosing", config = "GtClosingConfig", where = NULL)))
  PREG$register_library(lib)
  e <- expect_error(PREG$create("r_closing_gt_closing", list(label = 1L)), "guatiao_error")
  expect_equal(e$status, "BadValue")
  inst <- PREG$create("r_closing_gt_closing", list())
  expect_equal(live_instances(lib), 1)
  e <- expect_error(PREG$unload("r_closing"), "guatiao_error")
  expect_equal(e$status, "WrongKind") # linked, not mapped
  if (!is.null(fixture())) {
    info <- .Call(gt$C_library_info, lib$ptr)
    expect_equal(fx("fx_library_unload", info), 14L)
  }
  inst$close()
  inst$close()
  expect_equal(CLOSED, 1)
  expect_equal(live_instances(lib), 0)
  if (!is.null(fixture())) expect_equal(fx("fx_library_unload", .Call(gt$C_library_info, lib$ptr)), 0L)
  PREG$retire("r_closing")
  expect_true(!any(vapply(PREG$providers(), function(p) p$id == "r_closing_gt_closing", logical(1))))
}))

case("create builds nothing when it has nowhere to put the instance", with_examples({
  if (is.null(fixture())) {
    skip_case("no C compiler: the fixture is not built")
  } else {
    lib <- library_of("rgreeter")
    before <- live_objects()
    config <- value(list(prefix = "hey"))
    st <- fx("fx_create_null_out", core_library(), .Call(gt$C_library_info, lib$ptr), 1L,
      .Call(gt$C_node_pointer, config))
    expect_equal(st, 11L)
    expect_equal(live_objects(), before)
    close(config)
  }
}))

case("a provider says why it cannot run here", with_examples({
  WHY <- "not on this machine"
  PREG$register_library(guatiao_library("r_picky", "0.1.0",
    providers = list(provider("GtPicky", available = function() WHY, where = NULL))))
  expect_equal(PREG$provider_available("r_picky_gt_picky"), list(available = FALSE, reason = "not on this machine"))
  expect_true(!any(vapply(PREG$available("greeter"), function(p) p$id == "r_picky_gt_picky", logical(1))))
  WHY <- NULL
  expect_equal(PREG$provider_available("r_picky_gt_picky"), list(available = TRUE, reason = NULL))
}))

case("a closed library declines a host", with_examples({
  lib <- guatiao_library("r_declines", "0.1.0", providers = list("GtPicky"))
  e <- gt$.gt_entry(lib)
  close(lib)
  if (!is.null(fixture())) expect_true(!fx("fx_entry_answers", e))
  PREG$register_entry("r_declines", e)
  expect_true(!any(vapply(PREG$providers(), function(p) identical(p$library, "r_declines"), logical(1))))
}))
