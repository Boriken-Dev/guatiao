# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# R code runs only on R's main thread. A slot an R provider serves runs
# when a host calls it on that thread; called from any other, it answers
# Unsupported at once, touching no R object, and a destroy it is asked for
# there waits for R's thread.

case("an R provider runs on R's thread and refuses every other", with_examples({
  if (is.null(fixture())) {
    skip_case("no C compiler: the fixture is not built")
  } else {
    reg <- registry("r-threads", "0.1")
    reg$register_library(library_of("rgreeter"))
    t <- reg$provider_table("rgreeter_hello", "greeter")
    lib <- workspace_library("guatiao")
    on_r <- fx("fx_greet_here", lib, t$table, t$ctx, "ana", "hello, ana", 200L)
    expect_equal(on_r, c(200L, 0L, 0L, 0L))
    away <- fx("fx_greet_threads", lib, t$table, t$ctx, "ana", "hello, ana", 8L, 250L)
    expect_equal(away, c(0L, 2000L, 0L))
    # A thread left running while R goes on is refused the same way.
    fx("fx_greet_detached", lib, t$table, t$ctx, "ana", "hello, ana")
    for (i in 1:500) {
      if (!is.na(fx("fx_detached"))) break
      Sys.sleep(0.01)
    }
    expect_equal(fx("fx_detached"), 13L)
    # R is still sound after all that: the provider answers on its thread.
    expect_equal(value_data(greet(reg$bind("rgreeter_hello", Greeter), "bo")), list(greeting = "hello, bo"))
    reg$close()
  }
}))

case("an object destroyed on another thread is let go on R's", with_examples({
  if (is.null(fixture())) {
    skip_case("no C compiler: the fixture is not built")
  } else {
    before <- live_objects()
    e <- new.env()
    e$closed <- FALSE
    listener <- methods::new("GtListener", state = e)
    parts <- gt$.gt_object_parts(Listener, listener)
    expect_equal(live_objects(), before + 1)
    fx("fx_destroy_on_thread", parts$table, parts$size, parts$ctx)
    # Queued, not run: the thread that asked could not touch R.
    expect_equal(length(ls(gt$.gt_state$objects, all.names = TRUE)), before + 1)
    expect_true(!isTRUE(e$closed))
    expect_equal(live_objects(), before) # drains the queue on R's thread
    expect_true(isTRUE(e$closed))
  }
}))
