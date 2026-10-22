# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# JSON text to and from what value() takes and value_data(numbers = "text")
# answers, for the tests' documents: an object is a named list in its key
# order, a number a guatiao_number keeping its text. The suite needs no
# other package to read or show them.

read_json <- function(text) {
  cs <- strsplit(enc2utf8(paste(text, collapse = "\n")), "")[[1L]]
  n <- length(cs)
  i <- 1L
  space <- function() while (i <= n && cs[i] %in% c(" ", "\t", "\n", "\r")) i <<- i + 1L
  expect <- function(ch) {
    if (i > n || cs[i] != ch) stop(sprintf("JSON: '%s' expected at %d", ch, i), call. = FALSE)
    i <<- i + 1L
  }
  string <- function() {
    expect('"')
    out <- character()
    repeat {
      ch <- cs[i]
      if (ch == '"') {
        i <<- i + 1L
        return(paste(out, collapse = ""))
      }
      if (ch == "\\") {
        e <- cs[i + 1L]
        if (e == "u") {
          u <- strtoi(paste(cs[(i + 2L):(i + 5L)], collapse = ""), 16L)
          i <<- i + 6L
          if (u >= 0xD800 && u <= 0xDBFF && i <= n && cs[i] == "\\") {
            low <- strtoi(paste(cs[(i + 2L):(i + 5L)], collapse = ""), 16L)
            u <- 0x10000 + (u - 0xD800) * 1024 + (low - 0xDC00)
            i <<- i + 6L
          }
          out <- c(out, intToUtf8(u))
          next
        }
        out <- c(out, switch(e, b = "\b", f = "\f", n = "\n", r = "\r", t = "\t", e))
        i <<- i + 2L
      } else {
        out <- c(out, ch)
        i <<- i + 1L
      }
    }
  }
  value <- function() {
    space()
    ch <- cs[i]
    if (ch == "{") {
      i <<- i + 1L
      out <- structure(list(), names = character())
      space()
      if (cs[i] == "}") {
        i <<- i + 1L
        return(out)
      }
      repeat {
        space()
        key <- string()
        space()
        expect(":")
        out[key] <- list(value())
        space()
        if (cs[i] == "}") {
          i <<- i + 1L
          return(out)
        }
        expect(",")
      }
    }
    if (ch == "[") {
      i <<- i + 1L
      out <- list()
      space()
      if (cs[i] == "]") {
        i <<- i + 1L
        return(out)
      }
      repeat {
        out[length(out) + 1L] <- list(value())
        space()
        if (cs[i] == "]") {
          i <<- i + 1L
          return(out)
        }
        expect(",")
      }
    }
    if (ch == '"') return(string())
    for (word in c("true", "false", "null")) {
      if (identical(paste(cs[i:min(n, i + nchar(word) - 1L)], collapse = ""), word)) {
        i <<- i + nchar(word)
        return(switch(word, true = TRUE, false = FALSE, null = NULL))
      }
    }
    j <- i
    while (j <= n && cs[j] %in% strsplit("+-0123456789.eE", "")[[1L]]) j <- j + 1L
    if (j == i) stop(sprintf("JSON: unexpected '%s' at %d", ch, i), call. = FALSE)
    text <- paste(cs[i:(j - 1L)], collapse = "")
    i <<- j
    number(text)
  }
  out <- value()
  space()
  if (i <= n) stop(sprintf("JSON: text after the value at %d", i), call. = FALSE)
  out
}

# What value_data(numbers = "text") answers, written as JSON text.
json_of <- function(d) {
  if (is.null(d)) return("null")
  if (inherits(d, "guatiao_number")) return(unclass(d))
  if (is.logical(d)) return(if (isTRUE(d)) "true" else "false")
  if (is.character(d)) return(paste0('"', gsub('(["\\\\])', "\\\\\\1", d), '"'))
  if (!is.null(names(d))) {
    return(paste0("{", paste0('"', names(d), '":', vapply(d, json_of, ""), collapse = ","), "}"))
  }
  paste0("[", paste(vapply(d, json_of, ""), collapse = ","), "]")
}

# `x` (a guatiao value, or R data made one) as JSON text, from the value
# itself: every number its exact text.
as_json <- function(x) {
  v <- if (inherits(x, "guatiao_node")) x else value(x)
  json_of(value_data(v, numbers = "text"))
}
