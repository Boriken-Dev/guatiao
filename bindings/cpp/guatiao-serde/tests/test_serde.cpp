// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// guatiao-serde from C++: JSON, TOML and YAML both ways, where a document
// stops parsing, byte strings, documents straight to wire bytes, and a
// tree crossing between the core and serde libraries. A counting
// allocator is the default, so every path is checked to free.

#include "support/guatiao_counting_default.hpp"  // first: it names the default allocator

#include <guatiao/serde.hpp>

#include "support/harness.hpp"

namespace {

using guatiao::item;
namespace serde = guatiao::serde;
using serde::format;
using gt_test::counted;

#define CHECK_NONE_LEFT(what) \
  CHECK_MSG(counted().live() == 0, std::string(what) + ": " + std::to_string(counted().live()) + " left")

void json_both_ways() {
  {
    guatiao::value v = MUST(serde::try_parse(format::json, R"({"z": 3, "a": [1.5, true, null, "x", {}, []], "n": 1.10})"));
    CHECK(v.as_map().keys() == (std::vector<std::string_view>{"z", "a", "n"}));
    CHECK(v["n"].number_text() == std::string_view("1.10"));
    CHECK(v["a"][2].is_null());
    std::string text = MUST(serde::try_emit(format::json, v));
    CHECK(text == R"({"z":3,"a":[1.5,true,null,"x",{},[]],"n":1.10})");
    std::string pretty = MUST(serde::try_emit(format::json, v, serde::how::pretty));
    CHECK(pretty.size() > text.size() && pretty.find('\n') != std::string::npos);
    guatiao::value again = MUST(serde::try_parse(format::json, pretty));
    CHECK(again == v);
    guatiao::value big = MUST(serde::try_parse(format::json, "12345678901234567890123"));
    CHECK(big.number_text() == std::string_view("12345678901234567890123"));
  }
  CHECK_NONE_LEFT("JSON read and written");
}

void where_a_document_stops() {
  {
    auto bad = serde::try_parse(format::json, "{\"a\":\n 1,,}");
    CHECK(bad.code() == guatiao::status::bad_value);
    if (!bad.ok()) {
      guatiao::view d = bad.error().detail();
      CHECK(d["format"].string_or("") == "json");
      CHECK(d["line"].int_or(0) == 2);
      CHECK(d["column"].int_or(0) == 4);
      CHECK(!bad.error().message().empty());
    }
#if defined(GUATIAO_SERDE_TOML)
    auto bad_toml = serde::try_parse(format::toml, "a = = 1");
    CHECK(!bad_toml.ok() && bad_toml.error().detail()["line"].int_or(0) == 1 &&
          bad_toml.error().detail()["column"].int_or(0) == 5);
#endif
#if defined(GUATIAO_SERDE_YAML)
    auto bad_yaml = serde::try_parse(format::yaml, "a: [1");
    CHECK(!bad_yaml.ok() && bad_yaml.error().detail()["format"].string_or("") == "yaml");
#endif
    // The message never quotes the document.
    auto secret = serde::try_parse(format::json, R"({"password": "hunter2" oops})");
    CHECK(!secret.ok() && secret.error().message().find("hunter2") == std::string::npos);
  }
  CHECK_NONE_LEFT("refused documents");
}

void toml_and_yaml() {
#if defined(GUATIAO_SERDE_TOML)
  {
    guatiao::value v = MUST(guatiao::try_make(item::map({{"a", 1}, {"b", "x"}, {"t", item::map({{"c", true}})}})));
    std::string text = MUST(serde::try_emit(format::toml, v));
    guatiao::value back = MUST(serde::try_parse(format::toml, text));
    CHECK(back == v);
    guatiao::value list = MUST(guatiao::try_make(item::list({1, 2})));
    CHECK(serde::try_emit(format::toml, list).code() == guatiao::status::wrong_kind);
  }
  CHECK_NONE_LEFT("TOML");
#else
  gt_test::skip("this build was told the library has no TOML (GUATIAO_SERDE_FEATURES)");
#endif
#if defined(GUATIAO_SERDE_YAML)
  {
    guatiao::value v = MUST(serde::try_parse(format::yaml, "a: 1\nb: [x, 2.5, null]\nc:\n  d: true\n"));
    CHECK(v["b"][1].number_text() == std::string_view("2.5"));
    CHECK(v["c"]["d"].bool_or(false));
    std::string text = MUST(serde::try_emit(format::yaml, v));
    CHECK(MUST(serde::try_parse(format::yaml, text)) == v);
  }
  CHECK_NONE_LEFT("YAML");
#else
  gt_test::skip("this build was told the library has no YAML (GUATIAO_SERDE_FEATURES)");
#endif
}

void byte_strings() {
  {
    std::vector<std::uint8_t> raw{0, 1, 2};
    guatiao::value v = MUST(guatiao::try_make(item::map({{"blob", item::bytes(raw)}})));
    CHECK(MUST(serde::try_emit(format::json, v)) == R"({"blob":"data:;base64,AAEC"})");
    CHECK(MUST(serde::try_emit(format::json, v, serde::how::base64)) == R"({"blob":"AAEC"})");
    CHECK(MUST(serde::try_emit(format::json, v, serde::how::array)) == R"({"blob":[0,1,2]})");
    CHECK(serde::try_emit(format::json, v, serde::how::refuse).code() == guatiao::status::bad_value);
    guatiao::value read = MUST(serde::try_parse(format::json, R"({"blob":"data:;base64,AAEC"})", serde::how::read_data_uris));
    CHECK(read["blob"].bytes().has_value() && *read["blob"].bytes() == guatiao::bytes_view(raw));
    guatiao::value plain = MUST(serde::try_parse(format::json, R"({"blob":"data:;base64,AAEC"})"));
    CHECK(plain["blob"].is_string());
  }
  CHECK_NONE_LEFT("byte strings");
}

void documents_and_wire_bytes() {
  {
    std::vector<std::uint8_t> bytes = MUST(serde::try_to_wire(format::json, R"({"greeting": "hello"})"));
    guatiao::value v = MUST(guatiao::try_from_wire(bytes));  // decoded by the core library
    CHECK(v["greeting"].string_or("") == "hello");
    std::vector<std::uint8_t> encoded = MUST(guatiao::try_to_wire(v));
    CHECK(encoded == bytes);
    CHECK(MUST(serde::try_from_wire(format::json, encoded)) == R"({"greeting":"hello"})");
    CHECK(!serde::try_to_wire(format::json, "{").ok());
    CHECK(!serde::try_from_wire(format::json, std::vector<std::uint8_t>{0x09}).ok());
  }
  CHECK_NONE_LEFT("documents to and from wire bytes");
}

void crossing() {
  {
    // A tree the serde library built, changed and freed by the core's
    // functions; one the core built, written by serde.
    guatiao::value parsed = MUST(serde::try_parse(format::json, R"({"list": [1]})"));
    MUST(parsed["list"].try_push("two"));
    MUST(parsed.try_set("added", item::map({{"x", nullptr}})));
    CHECK(MUST(serde::try_emit(format::json, parsed)) == R"({"list":[1,"two"],"added":{"x":null}})");
    guatiao::value copy = MUST(parsed.try_clone());
    CHECK(copy == parsed);
  }
  CHECK_NONE_LEFT("a tree crossing between the libraries");
}

}  // namespace

int main() {
  json_both_ways();
  where_a_document_stops();
  toml_and_yaml();
  byte_strings();
  documents_and_wire_bytes();
  crossing();
  return gt_test::finish("serde");
}
