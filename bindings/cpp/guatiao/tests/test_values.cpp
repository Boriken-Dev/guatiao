// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Values from C++: building, reading, writing in place, moving, copying,
// and what is refused. Through the `try_` forms only, so this suite runs
// with exceptions on and off.

#include <guatiao/guatiao.hpp>

#include "support/harness.hpp"

namespace {

using guatiao::item;
using guatiao::view;

void building_and_reading() {
  guatiao::value v = MUST(guatiao::try_make(item::map({
      {"host", "10.0.0.1"},
      {"port", 5900},
      {"big", item::number("12345678901234567890123")},
      {"ratio", 0.25},
      {"on", true},
      {"none", nullptr},
      {"tags", item::list({"a", "b"})},
      {"blob", item::bytes(guatiao::bytes_view::of(std::string_view("a\0b", 3)))},
  })));
  CHECK(v.is_map());
  CHECK(v.size() == 8);
  CHECK(v["host"].string_or("") == "10.0.0.1");
  CHECK(v["port"].int_or(0) == 5900);
  CHECK(v["port"].as<std::uint16_t>() == std::uint16_t(5900));
  CHECK(!v["port"].as<std::int8_t>().has_value());
  CHECK(v["big"].number_text() == std::string_view("12345678901234567890123"));
  CHECK(!v["big"].as<std::int64_t>().has_value());
  CHECK(v["ratio"].as<double>() == 0.25);
  CHECK(!v["ratio"].as<std::int64_t>().has_value());
  CHECK(v["on"].bool_or(false));
  CHECK(v["none"].is_null());
  CHECK(v["nothing"].is_absent());
  CHECK(v["tags"][1].string_or("") == "b");
  CHECK(v["tags"][2].is_absent());
  CHECK(v["tags"][-1].is_absent());
  CHECK(v["host"][0].is_absent());
  CHECK(v["blob"].bytes()->size() == 3);
  CHECK(!v["host"].number_text().has_value());
  CHECK(!v["port"].string().has_value());

  std::vector<std::string_view> keys;
  for (guatiao::entry e : v.as_map()) keys.push_back(e.key);
  CHECK((keys == std::vector<std::string_view>{"host", "port", "big", "ratio", "on", "none", "tags", "blob"}));
  std::size_t n = 0;
  for (view e : v["tags"].as_list()) n += e.string_or("").size();
  CHECK(n == 2);
}

void writing_in_place() {
  guatiao::value v = MUST(guatiao::try_make(item::map({{"a", 1}})));
  MUST(v.try_set("b", "two"));
  MUST(v.try_set("a", 10));  // replaces in place, keeping the position
  CHECK(v.as_map().keys() == (std::vector<std::string_view>{"a", "b"}));
  CHECK(v["a"].int_or(0) == 10);
  CHECK(MUST(v.try_erase("b")) == true);
  CHECK(MUST(v.try_erase("b")) == false);
  CHECK(v.size() == 1);

  MUST(v.try_set("list", item::list({})));
  guatiao::node list = v["list"];
  MUST(list.try_push(1));
  MUST(list.try_push("x"));
  guatiao::value inner = MUST(guatiao::try_make(item::map({{"deep", true}})));
  MUST(v["list"].try_push(std::move(inner)));
  CHECK(inner.is_absent());  // moved in
  CHECK(v["list"].size() == 3);
  CHECK(v["list"][2]["deep"].bool_or(false));
  MUST(v["list"].try_erase_at(0));
  CHECK(v["list"][0].string_or("") == "x");
  MUST(v["list"].try_clear());
  CHECK(v["list"].size() == 0);

  MUST(v.try_set("s", "ab"));
  MUST(v["s"].try_append("cd"));
  CHECK(v["s"].string_or("") == "abcd");

  // A write the node's kind does not take is refused, and changes nothing.
  CHECK(v["s"].try_push(1).code() == guatiao::status::wrong_kind);
  CHECK(v["s"].try_set("k", 1).code() == guatiao::status::wrong_kind);
  CHECK(v["missing"].try_set("k", 1).code() == guatiao::status::null);

  // A node moved out leaves null behind; assigning replaces.
  guatiao::value taken = v["s"].take();
  CHECK(taken.string_or("") == "abcd");
  CHECK(v["s"].is_null());
  v["s"].assign(std::move(taken));
  CHECK(v["s"].string_or("") == "abcd");

  // Copying the entries of one map into another.
  guatiao::value other = MUST(guatiao::try_make(item::map({{"a", 99}, {"z", 26}})));
  MUST(v.ref().try_update(other));
  CHECK(v["a"].int_or(0) == 99);
  CHECK(v["z"].int_or(0) == 26);
}

void owned_maps_and_lists() {
  guatiao::map m;
  CHECK(m.size() == 0);
  MUST(m.try_set("k", "v"));
  CHECK(m["k"].string_or("") == "v");
  guatiao::value as_value = std::move(m).into_value();
  CHECK(as_value.is_map());
  auto back = guatiao::map::from(std::move(as_value));
  CHECK(back.ok());
  auto not_a_map = guatiao::map::from(MUST(guatiao::try_make(item(5))));
  CHECK(not_a_map.code() == guatiao::status::wrong_kind);

  guatiao::list l = MUST(guatiao::list::try_of({1, 2, 3}));
  CHECK(l.size() == 3);
  MUST(l.try_push(4));
  std::int64_t sum = 0;
  for (view x : l) sum += x.int_or(0);
  CHECK(sum == 10);

  // Released and adopted: the container crosses as its C struct.
  guatiao_list raw = l.release();
  CHECK(l.size() == 0);
  guatiao::list again = guatiao::list::adopt(raw);
  CHECK(again.size() == 4);
}

void refusals() {
  // Text that is not UTF-8, a number outside the grammar, a NaN.
  auto bad_text = guatiao::try_make(item(std::string("\xff\xfe")));
  CHECK(bad_text.code() == guatiao::status::bad_value);
  CHECK(guatiao::try_make(item::number("1.2.3")).code() == guatiao::status::bad_value);
  CHECK(guatiao::try_make(item::number("01")).code() == guatiao::status::bad_value);
  CHECK(guatiao::try_make(item(std::numeric_limits<double>::quiet_NaN())).code() == guatiao::status::bad_value);
  CHECK(guatiao::try_make(item(std::numeric_limits<double>::infinity())).code() == guatiao::status::bad_value);
  // A refused build inside a map leaves nothing built.
  CHECK(guatiao::try_make(item::map({{"ok", 1}, {"bad", item::number("x")}})).code() ==
        guatiao::status::bad_value);
  // A key that is not UTF-8.
  guatiao::value m = MUST(guatiao::try_make(item::map({})));
  CHECK(m.try_set(std::string_view("\xc3", 1), 1).code() == guatiao::status::bad_value);
  CHECK(m.size() == 0);
}

void numbers_round_trip() {
  CHECK(MUST(guatiao::try_make(item(-9223372036854775807LL - 1))).number_text() ==
        std::string_view("-9223372036854775808"));
  CHECK(MUST(guatiao::try_make(item(18446744073709551615ULL))).as<std::uint64_t>() == 18446744073709551615ULL);
  CHECK(MUST(guatiao::try_make(item(0.1))).number_text() == std::string_view("0.1"));
  CHECK(MUST(guatiao::try_make(item(-0.0))).as<double>() == 0.0);
  CHECK(MUST(guatiao::try_make(item(1e300))).as<double>() == 1e300);
  CHECK(MUST(guatiao::try_make(item::number("1e2"))).as<std::int64_t>() == std::nullopt);
  CHECK(MUST(guatiao::try_make(item::number("-0"))).as<std::uint32_t>() == std::nullopt);
}

void copies_and_equality() {
  guatiao::value a = MUST(guatiao::try_make(item::map({{"x", item::list({1, "two", item::map({{"y", nullptr}})})}})));
  guatiao::value b = MUST(a.try_clone());
  CHECK(a == b);
  MUST(b["x"].try_push(3));
  CHECK(a != b);
  guatiao::value order1 = MUST(guatiao::try_make(item::map({{"p", 1}, {"q", 2}})));
  guatiao::value order2 = MUST(guatiao::try_make(item::map({{"q", 2}, {"p", 1}})));
  CHECK(order1 != order2);  // order is part of a map
  CHECK(MUST(guatiao::try_make(item::number("1.0"))) != MUST(guatiao::try_make(item::number("1"))));
  // A copy of a tree from `item(view)`.
  guatiao::value c = MUST(guatiao::try_make(item::map({{"copy", a.get()}})));
  CHECK(c["copy"] == a);
}

void moves() {
  guatiao::value a = MUST(guatiao::try_make(item("text")));
  guatiao::value b = std::move(a);
  CHECK(a.is_absent());
  CHECK(b.string_or("") == "text");
  guatiao_value raw = b.release();
  CHECK(b.is_absent());
  guatiao::value c = guatiao::value::adopt(raw);
  CHECK(c.string_or("") == "text");
}

void schemas() {
  guatiao::value schema = MUST(guatiao::try_make(item::map({
      {"type", "object"},
      {"properties", item::map({{"name", item::map({{"type", "string"}})},
                                {"secret", item::map({{"type", "string"}, {"x-sensitive", true}})}})},
      {"required", item::list({"name"})},
      {"additionalProperties", false},
  })));
  guatiao::value good = MUST(guatiao::try_make(item::map({{"name", "ana"}, {"secret", "hunter2"}})));
  CHECK(guatiao::try_validate(schema, good).ok());
  guatiao::value bad = MUST(guatiao::try_make(item::map({{"name", 5}})));
  auto refused = guatiao::try_validate(schema, bad);
  CHECK(refused.code() == guatiao::status::bad_value);
  if (!refused.ok()) {
    CHECK(refused.error().detail()["key"].string_or("") == "name");
    CHECK(!refused.error().message().empty());
  }
  guatiao::value shown = MUST(guatiao::try_redact(schema, good, "***"));
  CHECK(shown["secret"].string_or("") == "***");
  CHECK(shown["name"].string_or("") == "ana");

  guatiao::value earlier = MUST(guatiao::try_make(item::map({{"a", 1}, {"n", item::map({{"x", 1}, {"y", 2}})}})));
  guatiao::value later = MUST(guatiao::try_make(item::map({{"n", item::map({{"y", 3}})}, {"b", 2}})));
  guatiao::value merged = MUST(guatiao::try_merge(guatiao::merge_mode::substitute, earlier, later));
  CHECK(merged["a"].int_or(0) == 1);
  CHECK(merged["n"]["x"].int_or(0) == 1);
  CHECK(merged["n"]["y"].int_or(0) == 3);
  CHECK(merged["b"].int_or(0) == 2);
  guatiao::value shallow = MUST(guatiao::try_merge(guatiao::merge_mode::simple, earlier, later));
  CHECK(shallow["n"]["x"].is_absent());
}

void times() {
  guatiao::date d = MUST(guatiao::date::try_parse("2026-01-31"));
  guatiao::date next = MUST(d.try_add(MUST(guatiao::span::try_parse("P1M"))));
  CHECK(MUST(next.try_to_string()) == "2026-02-28");
  CHECK((guatiao::date{2026, 2, 30}.try_to_string().code() == guatiao::status::bad_value));
  CHECK((guatiao::date{2026, 300, 1}.try_to_string().code() == guatiao::status::bad_value));
  guatiao::span s = MUST(guatiao::span::try_parse("1h 30m"));
  CHECK(s.hours == 1 && s.minutes == 30);
  CHECK(MUST(s.try_to_string()) == "PT1H30M");
  CHECK(MUST(s.negated().try_to_string()) == "-PT1H30M");
  guatiao::span mixed{};
  mixed.days = 1;
  mixed.hours = -1;
  CHECK(mixed.try_to_string().code() == guatiao::status::bad_value);
  guatiao::timestamp t = MUST(guatiao::timestamp::try_parse("2026-10-21T19:15:00+05:30"));
  CHECK(MUST(t.try_to_string()) == "2026-10-21T13:45:00Z");
  auto at = t.to_time_point();
  CHECK(guatiao::timestamp::from(at) == t);
  guatiao::length l = guatiao::length::from(std::chrono::milliseconds(1500));
  CHECK(l.seconds == 1 && l.nanoseconds == 500000000);
  CHECK(l.to_duration() == std::chrono::milliseconds(1500));
}

void wire_round_trip() {
  guatiao::value v = MUST(guatiao::try_make(item::map({{"n", item::number("1.10")}, {"b", item::bytes({})}})));
  auto bytes = MUST(guatiao::try_to_wire(v));
  guatiao::value back = MUST(guatiao::try_from_wire(bytes));
  CHECK(back == v);
  CHECK(back["n"].number_text() == std::string_view("1.10"));
  std::vector<std::uint8_t> trailing = bytes;
  trailing.push_back(0);
  CHECK(guatiao::try_from_wire(trailing).code() == guatiao::status::bad_value);
}

}  // namespace

int main() {
  building_and_reading();
  writing_in_place();
  owned_maps_and_lists();
  refusals();
  numbers_round_trip();
  copies_and_equality();
  moves();
  schemas();
  times();
  wire_round_trip();
  return gt_test::finish("values");
}
