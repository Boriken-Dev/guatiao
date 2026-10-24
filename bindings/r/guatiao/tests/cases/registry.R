# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# The registry against a real build: scanning, answers, instances, retire,
# collect and unload, and sources.

scanned <- function() {
  reg <- registry("r-tests", "0.1")
  reg$scan_dir(LIBRARY_DIR, rules = "kind=greeter")
  reg
}
ids <- function(list) vapply(list, function(p) p$id, "")

case("scans by rule and lists the providers", with_examples({
  reg <- registry("r-tests", "0.1")
  report <- reg$scan_dir(LIBRARY_DIR, rules = "kind=greeter")
  expect_true(any(grepl("derived_greeter", unlist(report$loaded))))
  expect_true(any(grepl("hello_library", unlist(report$loaded))))
  expect_true("derived_greeter_hello" %in% ids(reg$providers("greeter")))
  expect_true("hello_library_greeter" %in% ids(reg$providers("greeter")))
  reg$close()
}))

case("answers the documented fields for the best provider", with_examples({
  reg <- scanned()
  best <- reg$best("greeter")
  for (f in c("key", "id", "version", "library", "display_name", "from", "kinds", "has_config", "vtable_size")) {
    expect_true(f %in% names(best), f)
  }
  expect_true("greeter" %in% unlist(best$kinds))
  why <- reg$why_not("nothing-claims-this-kind")
  expect_true(identical(why$available, FALSE) && identical(why$why, "nothing-claims-it"))
  e <- expect_error(reg$provider("nonesuch"), "guatiao_error")
  expect_equal(e$status, "NotFound")
  reg$close()
}))

case("answers a single table, and a table per kind", with_examples({
  reg <- scanned()
  t <- reg$provider_table("hello_library_greeter")
  expect_true(!is.null(t$table) && t$size > 0)
  expect_true(is.null(reg$provider_table("derived_greeter_hello")$table))
  expect_true(!is.null(reg$provider_table("derived_greeter_hello", "greeter")$table))
  expect_true(is.null(reg$provider_table("derived_greeter_hello", "codec")$table))
  expect_true(is.numeric(count(reg$bind("derived_greeter_hello", Counter))))
  reg$close()
}))

case("a provider's meta is a copy carrying its form", with_examples({
  reg <- scanned()
  meta <- reg$provider_meta("derived_greeter_shouter")
  reg$close()
  sections <- vapply(value_data(meta$form$sections), function(s) s$id, "")
  expect_equal(sections, c("words", "volume"))
  close(meta)
  reg <- scanned()
  expect_true(is.null(reg$provider_meta("derived_greeter_hello")))
  e <- expect_error(reg$provider_meta("nobody"), "guatiao_error")
  expect_equal(e$status, "NotFound")
  reg$close()
}))

case("builds an instance, and gives a copy of its configuration schema", with_examples({
  reg <- scanned()
  schema <- reg$provider_config("derived_greeter_shouter")
  expect_equal(tag(schema), "map")
  expect_true(is.null(reg$provider_config("derived_greeter_hello")))
  inst <- reg$create("derived_greeter_shouter", list(prefix = "hey"))
  g <- inst$bind(Greeter)
  expect_true(startsWith(value_data(greet(g, "ana")$greeting), "hey"))
  inst$close()
  inst$close()
  expect_error(greet(g, "ana"), "guatiao_closed")
  reg$close()
  expect_equal(tag(schema), "map") # a copy outlives the registry
  close(schema)
}))

case("keys, ranks and re-keys", with_examples({
  reg <- scanned()
  reg$set_priority("derived_greeter_hello", 7)
  expect_equal(reg$priority("derived_greeter_hello"), 7L)
  reg$keyed_by("%id@%version")
  expect_true(any(startsWith(vapply(reg$providers("greeter"), function(p) p$key, ""), "derived_greeter_hello@")))
  expect_error(reg$keyed_by("%nonsense"), "guatiao_error")
  reg$close()
}))

case("retires a library and frees its key", with_examples({
  reg <- scanned()
  reg$retire("derived_greeter")
  expect_true(!("derived_greeter_hello" %in% ids(reg$providers("greeter"))))
  expect_true("hello_library_greeter" %in% ids(reg$providers("greeter")))
  again <- reg$scan_dir(LIBRARY_DIR, rules = "kind=greeter")
  expect_true(any(grepl("derived_greeter", unlist(again$loaded))))
  e <- expect_error(reg$retire("derived_greeter_hello"), "guatiao_error")
  expect_equal(e$status, "NotFound")
  reg$close()
}))

case("unloads only when the library agrees", with_examples({
  reg <- scanned()
  inst <- reg$create("derived_greeter_shouter", list(prefix = "hey"))
  e <- expect_error(reg$unload("derived_greeter"), "guatiao_error")
  expect_equal(e$status, "Busy")
  inst$close()
  reg$unload("derived_greeter")
  expect_true(!("derived_greeter" %in% ids(reg$libraries())))
  e <- expect_error(reg$unload("derived_greeter"), "guatiao_error")
  expect_equal(e$status, "NotFound")
  reg$close()
}))

case("collects a retired library once nothing of it is alive", with_examples({
  reg <- scanned()
  expect_true(is.null(reg$collect()))
  inst <- reg$create("derived_greeter_shouter", list(prefix = "hey"))
  reg$retire("derived_greeter")
  expect_true(is.null(reg$collect()))
  inst$close()
  expect_equal(reg$collect(), "derived_greeter")
  expect_true(is.null(reg$collect()))
  reg$close()
}))

case("refuses use once closed, and closes twice quietly", with_library({
  reg <- registry("r-tests", "0.1")
  reg$close()
  expect_error(reg$libraries(), "guatiao_closed")
  reg$close()
  expect_true(!reg$is_open())
  expect_true(!is_open(reg))
}))

case("serves libraries that come and go from a source file", with_examples({
  shelf <- build_c_source("r_shelf_a")
  if (is.null(shelf) || is.null(fixture())) {
    absent_greeter("c_source", "no C compiler: the source written in C is not built")
  } else {
    reg <- registry("r-sources", "0.1")
    reg$add_source_file(shelf, list(ids = list("shelf_a", "shelf_b")))
    expect_equal(length(reg$libraries()), 0)
    first <- reg$refresh()
    expect_equal(vapply(first$loaded, function(s) s$id, ""), c("shelf_a", "shelf_b"))
    expect_equal(count(reg$bind("shelf_a_counter", Counter)), 1)
    quiet <- reg$refresh()
    expect_true(all(lengths(quiet) == 0))
    fx("fx_shelf", shelf, "take", 1L)
    expect_equal(vapply(reg$refresh()$retired, function(s) s$id, ""), "shelf_b")
    reg$collect()
    expect_equal(fx("fx_shelf", shelf, "closed", 1L), 1L)
    fx("fx_shelf", shelf, "refuse", TRUE)
    fx("fx_shelf", shelf, "put", 2L)
    refused <- reg$refresh()$refused
    expect_equal(length(refused), 1)
    expect_equal(refused[[1]]$id, "shelf_c")
    expect_equal(refused[[1]]$why, "the shelf refuses, says C")
    reg$close()
  }
}))

case("refuses with the source's own words, and a file that is no source", with_examples({
  shelf <- build_c_source("r_shelf_b")
  if (is.null(shelf)) {
    absent_greeter("c_source", "no C compiler: the source written in C is not built")
  } else {
    reg <- registry("r-sources", "0.1")
    e <- expect_error(reg$add_source_file(shelf, list()), "guatiao_error")
    expect_true(grepl("`ids`", e$message, fixed = TRUE))
    reg$add_source_file(shelf, list(ids = list()))
    e <- expect_error(reg$add_source_file(shelf, list(ids = list())), "guatiao_error")
    expect_true(grepl("already added", e$message))
    e <- expect_error(reg$add_source_file(workspace_library("derived_greeter")), "guatiao_error")
    expect_equal(e$status, "WrongKind")
    e <- expect_error(reg$add_source_file(file.path(LIBRARY_DIR, "no-such-file")), "guatiao_error")
    expect_equal(e$status, "NotFound")
    reg$close()
  }
}))

case("follows the files of a watched directory", with_examples({
  dir <- scratch("watched")
  watched <- file.path(dir, "watched")
  aside <- file.path(dir, "aside")
  dir.create(watched)
  dir.create(aside)
  reg <- registry("r-sources", "0.1")
  reg$watch_dir(watched, rules = "kind=greeter")
  expect_error(reg$watch_dir(watched), "guatiao_error")
  expect_equal(length(reg$refresh()$loaded), 0)
  copy <- file.path(watched, lib_file("hello_r_watched"))
  file.copy(workspace_library("hello_library"), copy)
  expect_equal(vapply(reg$refresh()$loaded, function(s) s$key, ""), "hello_library")
  # A mapped image cannot be deleted on Windows; moved aside, it is gone.
  file.rename(copy, file.path(aside, basename(copy)))
  expect_equal(vapply(reg$refresh()$retired, function(s) s$key, ""), "hello_library")
  expect_equal(length(reg$libraries()), 0)
  reg$close()
}))
