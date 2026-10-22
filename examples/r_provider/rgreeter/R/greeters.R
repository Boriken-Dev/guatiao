# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# Greeters written in R: the same kinds the Rust, C and Python greeters
# serve. The package is the library: its id is the package's name, its
# version and authors come from its DESCRIPTION, and its providers are the
# classes declared with provider(), in order.

# One conversation `start` hands back, holding the listener it was handed.
Chat <- setRefClass("Chat", contains = "Conversation",
  fields = list(listener = "ANY", transcript = "character"))

setMethod("say", "Chat", function(self, what) {
  if (!nzchar(what)) stop(guatiao_error("BadValue", "nothing to say"))
  self$transcript <- c(self$transcript, what)
  heard(self$listener, what)
  invisible(NULL)
})

setMethod("read", "Chat", function(self, dst) {
  text <- charToRaw(paste(self$transcript, collapse = "\n"))
  n <- min(length(text), length(dst))
  if (n > 0) dst[seq_len(n)] <- text[seq_len(n)]
  n
})

setMethod("turns", "Chat", function(self) length(self$transcript))

# The listener a conversation holds is closed when the conversation is.
close.Chat <- function(con, ...) close(con$listener)

# One provider serving two kinds: its own one instance.
Hello <- setRefClass("Hello", contains = c("Greeter", "Counter"), fields = list(asked = "numeric"),
  methods = list(initialize = function(...) {
    asked <<- 0
    callSuper(...)
  }))

setMethod("greet", "Hello", function(self, name) {
  if (name == "panic") stop("a provider bug, on request")
  if (!nzchar(name)) stop(guatiao_error("BadValue", "nobody to greet"))
  list(greeting = paste0("hello, ", name))
})

setMethod("shout", "Hello", function(self, name) toupper(greet(self, name)$greeting))

setMethod("start", "Hello", function(self, listener) Chat$new(listener = listener, transcript = character()))

setMethod("greet_into", "Hello", function(self, name, listener) {
  heard(listener, greet(self, name)$greeting)
  invisible(NULL)
})

setMethod("greet_from", "Hello", function(self, config) {
  name <- if ("name" %in% names(config)) value_data(config$name) else ""
  close(config)
  greet(self, name)
})

setMethod("check", "Hello", function(self, config) {
  name <- if ("name" %in% names(config)) as.character(value_data(config$name)) else "world"
  config$name <- name
  config$greeting <- paste0("hello, ", name)
  nzchar(name)
})

setMethod("count", "Hello", function(self) {
  self$asked <- self$asked + 1
  self$asked
})

provider(Hello, display_name = "Hello, in R")

# What a shouter is built from; its schema is read from these fields.
ShouterConfig <- config_class("ShouterConfig",
  prefix = field("character", doc = "What every greeting starts with.", minLength = 1))

# Built from a configuration: one instance per configuration.
Shouter <- setClass("Shouter", contains = "Greeter", slots = c(prefix = "character"))

setMethod("greet", "Shouter", function(self, name) list(greeting = paste(self@prefix, toupper(name))))

provider(Shouter, config = ShouterConfig, make = function(config) Shouter(prefix = config@prefix))
