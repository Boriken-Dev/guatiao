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
