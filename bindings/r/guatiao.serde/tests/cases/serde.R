# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

case("JSON keeps key order and exact numbers", with_library({
  text <- '{"z":1,"a":[1,2.50,"x"],"big":123456789012345678901234567890.5}'
  v <- serde_parse(text)
  expect_equal(names(v), c("z", "a", "big"))
  expect_equal(serde_emit(v), text)
  expect_true(nchar(serde_emit(v, pretty = TRUE)) > nchar(text))
  close(v)
}))

case("TOML and YAML round-trip", with_library({
  if (has_export("guatiao_toml_emit", "guatiao_serde")) {
    v <- serde_parse('name = "ana"\nn = 3\n', format = "toml")
    expect_equal(value_data(v), list(name = "ana", n = 3L))
    expect_true(grepl('name = "ana"', serde_emit(v, format = "toml"), fixed = TRUE))
    expect_error(serde_emit(list(1), format = "toml"), "guatiao_error")
    close(v)
  } else {
    skip_case("guatiao_serde built without toml")
  }
  if (has_export("guatiao_yaml_emit", "guatiao_serde")) {
    v <- serde_parse("a: [1, 2]\n", format = "yaml")
    expect_equal(value_data(v), list(a = list(1L, 2L)))
    expect_equal(value_data(serde_parse(serde_emit(v, format = "yaml"), format = "yaml")), value_data(v))
    close(v)
  } else {
    skip_case("guatiao_serde built without yaml")
  }
}))

case("a malformed document names its format and position", with_library({
  e <- expect_error(serde_parse('{\n  "a": }'), "guatiao_parse_error")
  expect_equal(e$format, "json")
  expect_equal(e$line, 2L)
  expect_true(e$column > 0)
  expect_true(nzchar(e$reason))
  expect_equal(e$status, "BadValue")
  expect_error(serde_parse("{}", format = "xml"))
}))

case("plain data emits too, and bytes as asked", with_library({
  expect_equal(serde_emit(list(a = list(1L, NULL))), '{"a":[1,null]}')
  expect_equal(serde_emit(list(b = as.raw(1:3))), '{"b":"data:;base64,AQID"}')
  expect_equal(serde_emit(list(b = as.raw(1:3)), bytes = "array"), '{"b":[1,2,3]}')
  expect_error(serde_emit(as.raw(1), bytes = "refuse"), "guatiao_error")
  back <- serde_parse('{"b":"data:;base64,AQID"}', read_data_uris = TRUE)
  expect_equal(value_data(back$b), as.raw(1:3))
  close(back)
}))

case("serde_data reads what value_data(serde_parse()) reads", with_library({
  docs <- list(json = '{"z":1,"a":[1,2.50,"x",null,true],"big":123456789012345678901234567890.5,"o":{}}',
    toml = 'name = "ana"\nn = 3\n[t]\nx = [1.5, 2]\n', yaml = "a: [1, 2]\nb: {c: text}\n")
  for (format in names(docs)) {
    if (format != "json" && !has_export(paste0("guatiao_", format, "_emit"), "guatiao_serde")) next
    for (numbers in c("auto", "text", "double")) {
      v <- serde_parse(docs[[format]], format = format)
      expect_true(identical(serde_data(docs[[format]], format = format, numbers = numbers), value_data(v, numbers = numbers)), paste(format, numbers))
      close(v)
    }
  }
  a <- tryCatch(serde_parse('{"a":\n 1,,}'), error = function(e) e)
  b <- tryCatch(serde_data('{"a":\n 1,,}'), error = function(e) e)
  expect_true(inherits(b, "guatiao_parse_error"))
  expect_true(identical(c(b$line, b$column), c(a$line, a$column)))
  expect_true(identical(b$reason, a$reason))
  expect_error(serde_data("{}", format = "xml"), "error")
}))

case("serde_emit of R data writes what it writes for the value", with_library({
  data <- list(s = "\u00fc\u20ac", n = number("1.10"), i = -3L, f = 0.1, b = TRUE, na = NA,
    bytes = as.raw(1:2), v = c(1.5, 2), named = c(x = 1L, y = 2L), nested = list(list(a = list())))
  for (format in c("json", "yaml")) {
    if (format != "json" && !has_export(paste0("guatiao_", format, "_emit"), "guatiao_serde")) next
    for (pretty in c(FALSE, TRUE)) {
      v <- value(data)
      expect_true(identical(serde_emit(data, format = format, pretty = pretty, bytes = "base64"), serde_emit(v, format = format, pretty = pretty, bytes = "base64")), paste(format, pretty))
      close(v)
    }
  }
  expect_error(serde_emit(list(1, 2), format = "toml"), "guatiao_error")
  expect_error(serde_emit(list(a = NaN)), "guatiao_error")
}))

