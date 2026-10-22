# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# A configuration is an S4 class; its schema is read from its slots, and a
# value reads back as it.

config_class("GtAgentConfig", host = field("character", doc = "Where the agent is.", minLength = 1),
  port = field("integer", default = 5900L), labels = field("list", default = list()),
  where = globalenv())
setClassUnion("GtCharacterOrNull", c("character", "NULL"), where = globalenv())
setClass("GtPlain", slots = c(name = "character", ratio = "numeric", on = "logical", note = "GtCharacterOrNull",
  agent = "GtAgentConfig", since = "Date"), prototype = list(ratio = 0.5), where = globalenv())

case("a configuration class's schema", with_library({
  s <- schema_of("GtAgentConfig")
  check_equal(names(s$properties), c("host", "port", "labels"))
  check_equal(s$properties$host, list(type = "string", description = "Where the agent is.", minLength = 1))
  check_equal(s$properties$port$default, 5900L)
  check_equal(s$required, list("host"))
  check(identical(s$additionalProperties, FALSE))
  p <- schema_of("GtPlain")
  check_equal(p$properties$note, list(anyOf = list(list(type = "string"), list(type = "null"))))
  check_equal(p$properties$ratio$default, 0.5)
  check_equal(p$properties$since, list(type = "string", format = "date"))
  check_equal(p$properties$agent$type, "object")
  check_equal(unlist(p$required), c("name", "on", "agent", "since"))
  check(is.null(validate(s, list(host = "h"))))
  check_equal(validate(s, list(port = 1L))$key, "host")
  check(!is.null(validate(s, list(host = "h", nope = 1))))
  check(!is.null(validate(s, list(host = ""))))
  check_error(schema_of("GtNoSuchClass"))
}))

case("a value reads back as a configuration", with_library({
  a <- from_value(list(host = "h", ignored = TRUE), "GtAgentConfig")
  check(methods::is(a, "GtAgentConfig"))
  check_equal(a@host, "h")
  check_equal(a@port, 5900L)
  b <- from_value(list(host = "h", port = 22L, labels = list(x = 1L)), "GtAgentConfig")
  check_equal(b@port, 22L)
  check_equal(b@labels, list(x = 1L))
  e <- check_error(from_value(list(host = "h", port = "22"), "GtAgentConfig"), "guatiao_error")
  check_equal(conditionMessage(e), "BadValue: at port: want an integer, got string")
  check_error(from_value(list(host = "h", port = 3000000000), "GtAgentConfig"), "guatiao_error")
  check_error(from_value(list(host = "h", port = 1.5), "GtAgentConfig"), "guatiao_error")
  e <- check_error(from_value(list(port = 1L), "GtAgentConfig"), "guatiao_error")
  check_equal(conditionMessage(e), "BadValue: at host: missing")
  p <- from_value(list(name = "n", on = TRUE, note = NULL, agent = list(host = "x"), since = "2026-10-21"), "GtPlain")
  check(is.null(p@note))
  check_equal(p@ratio, 0.5)
  check_equal(p@agent@host, "x")
  check_equal(p@since, as.Date("2026-10-21"))
  e <- check_error(from_value(list(name = "n", on = TRUE, agent = list(host = 1), since = "2026-10-21"), "GtPlain"), "guatiao_error")
  check_equal(conditionMessage(e), "BadValue: at agent.host: want a string, got number")
  round <- from_value(value(a), "GtAgentConfig")
  check(identical(round@host, a@host) && identical(round@port, a@port))
  check_equal(from_value(list(1L, "x"), "ANY"), list(1L, "x"))
}))
