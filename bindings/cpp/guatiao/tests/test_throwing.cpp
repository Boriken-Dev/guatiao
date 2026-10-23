// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The throwing forms: each is its `try_` form, throwing the error that
// form carries. Built with exceptions on only.

#include <guatiao/guatiao.hpp>

#include "support/harness.hpp"

namespace {

using guatiao::item;

template <class F>
guatiao::status thrown(F&& f) {
  try {
    f();
  } catch (const guatiao::error& e) {
    return e.code();
  }
  return guatiao::status::ok;
}

void values() {
  guatiao::value v = guatiao::make(item::map({{"a", 1}}));
  v.set("b", "two").set("c", item::list({1, 2}));
  v["c"].push(3);
  CHECK(v["c"].size() == 3);
  CHECK(v.erase("a"));
  guatiao::map m{{"k", "v"}, {"n", 2}};
  CHECK(m["n"].int_or(0) == 2);
  guatiao::list l{1, "x", nullptr};
  CHECK(l.size() == 3);
  CHECK(v.clone() == v);

  CHECK(thrown([] { guatiao::make(item::number("nope")); }) == guatiao::status::bad_value);
  CHECK(thrown([&] { v["b"].push(1); }) == guatiao::status::wrong_kind);
  CHECK(thrown([] { guatiao::from_wire(std::vector<std::uint8_t>{0x09}); }) == guatiao::status::bad_value);
  try {
    guatiao::date::parse("2026-02-30");
    CHECK(false);
  } catch (const guatiao::error& e) {
    CHECK(e.code() == guatiao::status::bad_value);
    CHECK(e.detail()["at"].int_or(-1) == 8);
    CHECK(std::string(e.what()).find("expected") != std::string::npos);
  }
  // `result::value()` throws what the result carries.
  auto r = guatiao::try_make(item::number("x"));
  CHECK(thrown([&] { (void)r.value(); }) == guatiao::status::bad_value);
}

void schemas_and_times() {
  guatiao::value schema = guatiao::make(item::map({{"type", "object"},
                                                   {"properties", item::map({{"n", item::map({{"type", "integer"}})}})},
                                                   {"additionalProperties", false}}));
  guatiao::validate(schema, guatiao::make(item::map({{"n", 1}})));
  CHECK(thrown([&] { guatiao::validate(schema, guatiao::make(item::map({{"x", 1}}))); }) ==
        guatiao::status::bad_value);
  CHECK(guatiao::date::parse("2026-01-31").add(guatiao::span::parse("P1M")).to_string() == "2026-02-28");
  CHECK(guatiao::normalize(guatiao::form::span, "1h 30m") == "PT1H30M");
  guatiao::receiver r;
  CHECK(thrown([&] { r.accept(std::vector<std::uint8_t>{0x01, 0x07, 0x00}); }) == guatiao::status::not_found);
  CHECK((guatiao::timestamp{1, 0}.to_time_point() > std::chrono::system_clock::time_point()));
  auto frame = guatiao::wire_send(3, guatiao::make(nullptr));
  CHECK(frame.size() == 3);
}

}  // namespace

int main() {
  values();
  schemas_and_times();
  return gt_test::finish("throwing");
}
