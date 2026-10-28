# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# A plugin host written in R.
#
# Scans the directories it is given for libraries declaring the greeter
# kind, whatever language wrote them, and calls each greeter through the
# kind's table: a map back, a map lent and changed in place, a map handed
# over, an object handed in and one handed back, an object lent for one
# call. A provider that builds instances from a configuration is given
# one its schema refuses, then one it accepts. The greeters written in R
# are registered beside them, and the greeters written in Python are
# served by the guatiao-source-py bridge when one is named. Everything it opened
# is closed before it exits, and it prints what it did.
#
#   Rscript examples/r_host/host.R DIR [DIR ...] [--python-source BRIDGE --python-dir DIR ... --python-path DIR ...]
#
# The guatiao, greeterkind and rgreeter packages are installed first (see
# the README beside this file).

suppressPackageStartupMessages({
  library(guatiao)
  library(greeterkind)
  library(rgreeter)
})

# A listener: what a conversation tells, kept.
setClass("Ear", contains = "Listener", slots = c(state = "environment"))
setMethod("heard", "Ear", function(self, what) {
  self@state$heard <- c(self@state$heard, what)
  invisible(NULL)
})
close.Ear <- function(con, ...) con@state$closed <- TRUE
registerS3method("close", "Ear", close.Ear)
ear <- function() {
  state <- new.env()
  state$heard <- character()
  state$closed <- FALSE
  new("Ear", state = state)
}

exercise <- function(g, tell) {
  answer <- greet(g, "ana")
  tell("greet: ", value_data(answer$greeting))
  close(answer)
  tryCatch(close(greet(g, "")), guatiao_error = function(e) tell("greet(\"\") refused: ", e$status, ": ", e$message))
  # The other slots are optional: a provider may leave them out.
  missing <- Filter(function(s) !has_slot(g, s), c("shout", "check", "greet_from", "greet_into", "start"))
  if (length(missing)) tell("a table leaving out ", paste(missing, collapse = ", "))
  if (has_slot(g, "shout")) tell("shout: ", shout(g, "ana"))
  if (has_slot(g, "check")) {
    config <- value(list(name = "bo"))
    ok <- check(g, config)
    tell("check lent a map and it came back ", if (ok) "accepted" else "refused", ": ", paste(sort(names(config)), collapse = ", "))
    close(config)
  }
  if (has_slot(g, "greet_from")) {
    handed <- value(list(name = "cy"))
    answer <- greet_from(g, handed)
    tell("greet_from took a map: ", value_data(answer$greeting))
    close(answer)
    close(handed)
  }
  if (has_slot(g, "greet_into")) {
    e <- ear()
    greet_into(g, "di", e)
    tell("greet_into told a listener: ", paste(e@state$heard, collapse = ", "))
  }
  if (has_slot(g, "start")) {
    listener <- ear()
    chat <- tryCatch(start(g, listener), guatiao_error = function(e) {
      tell("start refused: ", e$message)
      NULL
    })
    if (is.null(chat)) return(invisible(NULL))
    say(chat, "hi")
    say(chat, "again")
    buf <- byte_buffer(64)
    n <- read(chat, buf)
    say_line <- sprintf("a conversation of %d turns: \"%s\"", as.integer(turns(chat)),
      gsub("\n", "\\\\n", rawToChar(as.raw(buf)[seq_len(n)])))
    tell(say_line)
    close(chat)
    tell("closed it; the listener heard ", paste(listener@state$heard, collapse = ", "),
      if (isTRUE(listener@state$closed)) " and was let go" else "")
  }
}

main <- function(args = commandArgs(trailingOnly = TRUE)) {
  dirs <- character()
  python_source <- NULL
  python_dirs <- list()
  python_paths <- list()
  i <- 1
  while (i <= length(args)) {
    a <- args[i]
    if (a == "--python-source") {
      python_source <- args[i + 1]
      i <- i + 2
    } else if (a == "--python-dir") {
      python_dirs[[length(python_dirs) + 1]] <- args[i + 1]
      i <- i + 2
    } else if (a == "--python-path") {
      python_paths[[length(python_paths) + 1]] <- args[i + 1]
      i <- i + 2
    } else {
      dirs <- c(dirs, a)
      i <- i + 1
    }
  }
  reg <- registry("r-host", "1.0")
  lib <- NULL
  on.exit({
    reg$close()
    # What this host built for itself, once no registry can call it.
    if (!is.null(lib)) close(lib)
    cat("objects a library can still reach: ", live_objects(), "\n", sep = "")
  })
  for (dir in dirs) {
    report <- reg$scan_dir(dir, rules = "kind=greeter")
    for (file in unlist(report$loaded)) cat("loaded ", basename(file), "\n", sep = "")
  }
  lib <- library_of("rgreeter")
  reg$register_library(lib)
  cat("registered ", lib$id, " ", lib$version, " from R\n", sep = "")
  if (!is.null(python_source)) {
    # The bridge imports python3.dll from the Python installation, on PATH.
    reg$set_import_search("path")
    reg$add_source_file(python_source, list(dirs = python_dirs, paths = python_paths))
    changed <- reg$refresh()
    for (served in changed$loaded) cat("served ", served$id, " from Python\n", sep = "")
    for (refused in changed$refused) cat("refused ", if (is.null(refused$id)) "?" else refused$id, ": ", refused$why, "\n", sep = "")
  }
  for (p in reg$providers("greeter")) {
    key <- p$key
    cat("\n", key, " (", p$library, " ", p$version, ")\n", sep = "")
    tell <- function(...) cat("  ", ..., "\n", sep = "")
    g <- tryCatch(reg$bind(key, Greeter), guatiao_floor_mismatch = function(e) {
      tell("not this greeter kind: ", conditionMessage(e))
      NULL
    })
    if (is.null(g)) next
    if (!isTRUE(p$has_config)) {
      exercise(g, tell)
      next
    }
    # Built from a configuration: one the schema refuses, then one it accepts.
    tryCatch(reg$create(key, list())$close(), guatiao_error = function(e) {
      tell("an empty configuration is refused: ", e$message)
    })
    inst <- reg$create(key, list(prefix = "hey"))
    tell("instance from prefix = hey")
    exercise(inst$bind(Greeter), tell)
    inst$close()
  }
  # Unmapped where the library agrees; one with no say in it is retired, and stays mapped.
  cat("\n")
  for (l in reg$libraries()) {
    key <- l$key
    tryCatch({
      reg$unload(key)
      cat("unloaded ", key, "\n", sep = "")
    }, guatiao_error = function(e) {
      reg$retire(key)
      cat("retired ", key, ", still mapped: unload said ", e$status, "\n", sep = "")
    })
  }
  invisible(NULL)
}

main()
