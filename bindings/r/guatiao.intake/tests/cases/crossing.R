# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# A tree crosses between guatiao_intake and guatiao: built by one library,
# read and freed through the other, every block accounted for.

case("a tree guatiao built is read by intake, and its view refuses once it closes", with_library({
  c <- counting()
  v <- value(list(agent = list(list(name = "a"), list(name = "b"))), alloc = c)
  found <- intake_at(v, "agent[1].name")
  expect_equal(value_data(found), "b")
  close(v)
  expect_error(value_data(found), "guatiao_closed")
  expect_equal(live(c), 0)
}))

case("a tree intake built is changed and freed through guatiao", with_library({
  schema <- value(read_json('{"type": "object", "properties": {"name": {"type": "string", "x-section": "who"}}}'))
  form <- intake_for_schema(schema)
  expect_equal(value_data(form$sections), list(list(id = "who")))
  close(form)
  close(schema)
}))

case("a provider's form, read from its meta, fits its schema", with_library({
  reg <- guatiao::registry("r-intake-tests", "0.1")
  reg$scan_dir(LIBRARY_DIR, rules = "kind=greeter")
  meta <- reg$provider_meta("derived_greeter_shouter")
  form <- meta[[intake_provider_form_key]]
  schema <- reg$provider_config("derived_greeter_shouter")
  expect_true(is.null(intake_check(schema, form)))
  expect_equal(value_data(form$fields$prefix$placeholder), "hey")
  expect_equal(value_data(form$fields$marks$unit), "marks")
  expect_true(!intake_is_visible(schema, form, "marks", list(exclaim = FALSE)))
  expect_true(intake_is_visible(schema, form, "marks", list(exclaim = TRUE)))
  close(schema)
  close(meta)
  reg$close()
}))

# The glue compiles against copies of the headers: they must be the
# committed ones.
case("the header copies are the committed headers", with_repo({
  for (h in c("guatiao/include/guatiao.h", "guatiao-intake/include/guatiao_intake.h")) {
    committed <- readBin(repo_file("crates", h), "raw", file.size(repo_file("crates", h)))
    copy <- repo_file("bindings", "r", "guatiao.intake", "src", basename(h))
    expect_true(identical(committed, readBin(copy, "raw", file.size(copy))), paste(basename(h), "matches"))
  }
}))
