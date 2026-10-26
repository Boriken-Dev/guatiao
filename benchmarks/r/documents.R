# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# A document to plain R data and back, two ways: through a value
# (value_data(serde_parse(text)), serde_emit(value(data))) and through the
# wire bytes as a tape read and written in C (serde_data(text),
# serde_emit(data)). No callback way: an R error must never unwind through
# the library's frames, so R objects are made only once its call returns.
#
#     GUATIAO_LIBRARY=<target>/release Rscript benchmarks/r/documents.R [--save NAME]
#
# with guatiao and guatiao.serde installed. Writes
# benchmarks/results/NAME.json in the runner's shape.

suppressPackageStartupMessages({
  library(guatiao)
  library(guatiao.serde)
})

config <- function() {
  servers <- lapply(0:49, function(i) {
    list(host = sprintf("node-%d.example", i), port = 8000L + i, weight = i / 7, tags = list("edge", "eu", as.character(i)),
      enabled = i %% 3 != 0, limits = list(rps = 1000L + i, burst = 2.5, note = NULL))
  })
  list(name = "fleet", servers = servers)
}

cases <- list(config = config(), floats_10k = as.list(0:9999 * 1.25 + 0.5), text_1mb = list(blob = strrep("x", 2^20)))

parse_value <- function(text) {
  v <- serde_parse(text)
  on.exit(close(v))
  value_data(v)
}
parse_tape <- function(text) serde_data(text)
render_value <- function(data) {
  v <- value(data)
  on.exit(close(v))
  serde_emit(v)
}
render_tape <- function(data) serde_emit(data)

time_it <- function(f, target = 0.02, samples = 15L) {
  f()
  n <- 1L
  repeat {
    t <- proc.time()[["elapsed"]]
    for (i in seq_len(n)) f()
    if (proc.time()[["elapsed"]] - t >= target) break
    n <- n * 2L
  }
  vapply(seq_len(samples), function(s) {
    t <- proc.time()[["elapsed"]]
    for (i in seq_len(n)) f()
    (proc.time()[["elapsed"]] - t) / n * 1000
  }, 0)
}

args <- commandArgs(trailingOnly = TRUE)
save <- if ("--save" %in% args) args[[match("--save", args) + 1L]] else NULL
samples <- if ("--samples" %in% args) as.integer(args[[match("--samples", args) + 1L]]) else 15L
metrics <- list()
for (name in names(cases)) {
  data <- cases[[name]]
  text <- render_tape(data)
  stopifnot(identical(parse_tape(text), parse_value(text)), identical(render_value(data), text))
  force(text)
  local({
    d <- data
    t <- text
    metrics[[paste0(name, "_parse_value")]] <<- function() parse_value(t)
    metrics[[paste0(name, "_parse_tape")]] <<- function() parse_tape(t)
    metrics[[paste0(name, "_render_value")]] <<- function() render_value(d)
    metrics[[paste0(name, "_render_tape")]] <<- function() render_tape(d)
  })
}
results <- list()
for (name in names(metrics)) {
  xs <- time_it(metrics[[name]], samples = samples)
  results[[name]] <- xs
  cat(sprintf("%-32s %12.1f us\n", name, median(xs) * 1000))
}
if (!is.null(save)) {
  here <- dirname(normalizePath(sub("--file=", "", grep("--file=", commandArgs(FALSE), value = TRUE)[1L])))
  lines <- c("{", sprintf('  "name": "%s",', save), '  "metrics": {')
  body <- vapply(names(results), function(n) {
    xs <- results[[n]]
    sprintf('    "%s": {"min_ms": %.6f, "median_ms": %.6f, "max_ms": %.6f}', n, min(xs), median(xs), max(xs))
  }, "")
  lines <- c(lines, paste(body, collapse = ",\n"), "  },",
    sprintf('  "raw": {"processor": "%s", "os": "%s", "r": "%s"}', R.version$arch, R.version$os,
      paste(R.version$major, R.version$minor, sep = ".")), "}")
  path <- file.path(here, "..", "results", paste0(save, ".json"))
  writeLines(lines, path)
  message("saved ", path)
}
