# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

NAME_SCHEMA <- '{"type": "object", "properties": {"name": {"type": "string", "title": "Name"}},
  "required": ["name"], "additionalProperties": false}'

FLAT_SCHEMA <- '{
  "type": "object",
  "properties": {
    "auth": {"type": "object", "x-variant-tag": "auth", "oneOf": [
      {"type": "object", "properties": {"auth": {"const": "sso"}}, "required": ["auth"]},
      {"type": "object", "properties": {"auth": {"const": "userpass"}, "username": {"type": "string"},
        "password": {"type": "string", "x-sensitive": true}}, "required": ["auth"]}]},
    "agent": {"type": "array", "items": {"type": "object", "properties": {"name": {"type": "string"}}}},
    "env": {"type": "object", "additionalProperties": {"type": "string"}},
    "host": {"type": "string", "title": "Host"}
  }
}'

empty_map <- structure(list(), names = character())

case("checking a form and laying it out", with_library({
  schema <- serde_parse(NAME_SCHEMA)
  check(is.null(intake_check(schema, empty_map)))
  err <- intake_check(schema, list(sections = list(), fields = list(nope = empty_map)))
  check_equal(err$kind, "unknown_field")
  check_equal(unlist(lapply(intake_layout(schema, empty_map), function(s) unlist(s$fields))), "name")
  check(intake_is_visible(schema, empty_map, "name", list(name = "ana")))
  e <- check_error(intake_is_visible(schema, empty_map, "nope", empty_map), "guatiao_error")
  check_equal(e$status, "NotFound")
  close(schema)
}))

case("a path names one place inside a value", with_library({
  v <- value(list(agent = list(list(name = "one"), list(name = "two")),
    env = list(PATH = "/bin", "a.b" = "dotted", "1" = "keyed")))
  check_equal(value_data(intake_at(v, "agent[0].name")), "one")
  check_equal(value_data(intake_at(v, "agent[1].name")), "two")
  check_equal(value_data(intake_at(v, "env[PATH]")), "/bin")
  check_equal(value_data(intake_at(v, "env[a.b]")), "dotted")
  check_equal(value_data(intake_at(v, 'env["1"]')), "keyed")
  check(is.null(intake_at(v, "agent[9].name")))
  check(is.null(intake_at(v, "nonesuch")))
  check(is.null(intake_at(v, "agent[")))
  view <- intake_at(v, "agent[0]")
  check_equal(node_path(view), "agent[0]")
  copy <- intake_at(list(a = list(b = 1)), "a.b")
  check_equal(value_data(copy), 1L)
  close(v)
  check_error(value_data(view), "guatiao_closed")
}))

case("a form made out of a schema", with_library({
  schema <- value(list(type = "object", additionalProperties = FALSE, properties = list(
    host = list(type = "string", "x-section" = "net"),
    tls = list(type = "object", additionalProperties = FALSE,
      properties = list(verify = list(type = "boolean", "x-section" = "trust"))),
    label = list(type = "string"))))
  made <- intake_for_schema(schema)
  doc <- value_data(made)
  check_equal(vapply(doc$sections, function(s) s$id, ""), "net")
  check_equal(doc$fields$tls$form$sections, list(list(id = "trust")))
  check(is.null(intake_check(schema, made)))
  close(made)
  implied <- intake_form_for(empty_map, schema, "tls")
  check(inherits(implied, "guatiao_value"))
  close(implied)
  check(is.null(intake_form_for(empty_map, schema, "label")))
  check(is.null(intake_form_for(empty_map, schema, "nonesuch")))
  close(schema)
}))

case("the flat projection", with_library({
  schema <- serde_parse(FLAT_SCHEMA)
  host <- intake_resolve(schema, "host")
  check_equal(value_data(host$title), "Host")
  check(isTRUE(value_data(intake_resolve(schema, "auth.password")[["x-sensitive"]])))
  check(is.null(intake_resolve(schema, "nonesuch")))
  check_equal(intake_flat_keys(schema, "auth"), c("auth", "auth.username", "auth.password"))
  check_error(intake_flat_keys(schema, "nonesuch"), "guatiao_error")
  rows <- list(
    list("auth", list(auth = "userpass", username = "alice", password = "s3cret"),
      c(auth = "userpass", auth.username = "alice", auth.password = "s3cret")),
    list("auth", list(auth = "sso"), c(auth = "sso")),
    list("agent", list(list(name = "one"), list(name = "two")), c("agent[0].name" = "one", "agent[1].name" = "two")),
    list("env", list(PATH = "/bin", "a.b" = "dotted"), c("env[PATH]" = "/bin", "env[a.b]" = "dotted")),
    list("host", "example.org", c(host = "example.org")))
  for (r in rows) {
    flat <- intake_flatten(schema, r[[1]], r[[2]])
    check_equal(flat[order(names(flat))], r[[3]][order(names(r[[3]]))], paste("flatten", r[[1]]))
    back <- intake_unflatten(schema, r[[1]], r[[3]])
    check_equal(value_data(back), r[[2]], paste("unflatten", r[[1]]))
    close(back)
  }
  check(is.null(intake_unflatten(schema, "auth", c(auth = "nonesuch"))))
  check(is.null(intake_unflatten(schema, "nonesuch", character())))
  check(is.null(intake_unflatten(schema, "host", character())))
  check_error(intake_unflatten(schema, "host", list(host = 1)))
  check_error(intake_flatten(schema, "auth", list(1)), "guatiao_error")
  close(schema)
  check_error(value_data(host), "guatiao_closed")
}))

case("redaction", with_library({
  schema <- '{"type": "object", "properties": {"user": {"type": "string"},
    "password": {"type": "string", "writeOnly": true, "x-sensitive": true},
    "tls": {"type": "object", "properties": {"ca": {"type": "string"}, "key": {"$ref": "#/$defs/Key"}}},
    "recovery": {"type": "array", "items": {"type": "string", "x-sensitive": true}}},
    "$defs": {"Key": {"type": "string", "x-sensitive": true}}}'
  s <- serde_parse(schema)
  login <- list(user = "ana", password = "hunter2", tls = list(ca = "/ca", key = "k"),
    recovery = list("r1", "r2"), extra = "?")
  shown <- redact(s, login)
  check_equal(value_data(shown), list(user = "ana", password = "[redacted]", tls = list(ca = "/ca", key = "[redacted]"),
    recovery = list("[redacted]", "[redacted]"), extra = "[redacted]"))
  close(shown)
  doc <- value(list(inner = login))
  check_equal(value_data(redact(s, doc$inner, placeholder = "***")$password), "***")
  close(doc)
  check_error(redact(list(1), login), "guatiao_error")
  close(s)
}))
