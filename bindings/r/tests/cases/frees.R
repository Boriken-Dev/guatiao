# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# Everything built is freed on every path, counted by an allocator that
# knows how many blocks it handed out and has not had back.

unwritable <- function() new.env()

case("a clean build and close leaves nothing", with_library({
  c <- counting()
  v <- value(list(a = list(1L, "two", as.raw(3)), m = list(x = NULL)), alloc = c)
  expect_true(live(c) > 0)
  v$b <- "more"
  push(v$a, 4L)
  insert(v$a, 1, "zero")
  v$a[[2]] <- "one"
  close(v)
  expect_equal(live(c), 0)
}))

case("a conversion that fails part way frees what it built", with_library({
  c <- counting()
  for (bad in list(list(a = list(1, 2, unwritable())), list(1, list(x = NaN)), list(a = 1, 2),
    list(a = 1, a = 2), list(d = structure(1, class = "gt_unknown")))) {
    expect_error(value(bad, alloc = c))
    expect_equal(live(c), 0)
  }
}))

case("a failed write leaves the tree as it was", with_library({
  c <- counting()
  v <- value(list(a = list(1L)), alloc = c)
  before <- live(c)
  expect_error(v$b <- list(1, unwritable()))
  expect_error(push(v$a, list(x = Inf)))
  expect_error(v$a[[1]] <- list(unwritable()))
  expect_error(insert(v$a, 1, unwritable()))
  expect_equal(live(c), before)
  expect_equal(value_data(v), list(a = list(1L)))
  close(v)
  expect_equal(live(c), 0)
}))

case("answers are built in the allocator named, and freed", with_library({
  c <- counting()
  r <- redact(list(type = "object"), list(a = 1L), alloc = c)
  expect_true(live(c) > 0)
  close(r)
  expect_equal(live(c), 0)
  w <- value(list(a = list(1L)), alloc = c)
  x <- value(w$a)
  close(w)
  expect_equal(live(c), 0)
  close(x)
}))

case("an unclosed value is freed by the collector", with_library({
  c <- counting()
  local({
    value(list(a = list(1L, 2L, 3L)), alloc = c)
    NULL
  })
  for (i in 1:5) {
    invisible(gc())
    if (live(c) == 0) break
  }
  expect_equal(live(c), 0)
}))
