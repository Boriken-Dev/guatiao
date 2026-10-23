// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Every path frees: this file makes a counting allocator the binding's
// default, so every tree it builds (answers, scratch, an error's detail)
// is counted, then walks the ordinary paths and the refused ones, and
// checks nothing is left out. Through the `try_` forms only, so it runs
// with exceptions on and off.

#include "support/guatiao_counting_default.hpp"  // first: it names the default allocator

#include <guatiao/guatiao.hpp>

#include "support/harness.hpp"
#include "support/json.hpp"

namespace {

using guatiao::item;
using gt_test::counted;

long live() { return counted().live(); }

#define CHECK_NONE_LEFT(what) CHECK_MSG(live() == 0, std::string(what) + ": " + std::to_string(live()) + " left")

item sample() {
  return item::map({{"name", "ana"},
                    {"n", 5},
                    {"list", item::list({1, "two", item::map({{"deep", item::list({nullptr, true})}})})},
                    {"bytes", item::bytes(guatiao::bytes_view::of("xyz"))}});
}

void builds_and_drops() {
  {
    guatiao::value v = MUST(guatiao::try_make(sample()));
    CHECK(live() > 0);
    guatiao::value copy = MUST(v.try_clone());
    MUST(copy.try_set("more", item::list({1, 2, 3})));
    MUST(copy["list"].try_push("x"));
    MUST(copy["list"].try_erase_at(0));
    CHECK(MUST(copy.try_erase("name")));
    MUST(copy["list"].try_clear());
    guatiao::value moved = std::move(copy);
    guatiao::value taken = moved["n"].take();
    moved["n"].assign(std::move(taken));
  }
  CHECK_NONE_LEFT("a tree, a copy and their changes");
  {
    guatiao::map m;
    MUST(m.try_set("a", 1));
    guatiao::list l = MUST(guatiao::list::try_of({"x", "y"}));
    MUST(m.try_set("l", std::move(l).into_value()));
    guatiao_map raw = m.release();
    guatiao::map back = guatiao::map::adopt(raw);
  }
  CHECK_NONE_LEFT("owned maps and lists, released and adopted");
}

void every_refused_build_frees() {
  // Every allocation a nested build makes, refused in turn.
  long needed = 0;
  {
    long before = counted().total();
    guatiao::value v = MUST(guatiao::try_make(sample()));
    needed = counted().total() - before;
  }
  CHECK(needed > 5);
  int refused = 0;
  for (long n = 0; n < needed; ++n) {
    counted().fail_after(n);
    auto v = guatiao::try_make(sample());
    counted().fail_after(-1);
    if (!v.ok()) {
      ++refused;
      CHECK_MSG(v.code() == guatiao::status::alloc, "refusal " + std::to_string(n) + " is ALLOC");
    }
    CHECK_NONE_LEFT("a build refused after " + std::to_string(n) + " blocks");
  }
  CHECK(refused == needed);

  // A refused growth leaves the tree as it was.
  {
    guatiao::value v = MUST(guatiao::try_make(item::map({{"a", 1}})));
    guatiao::value big = MUST(guatiao::try_make(item("a value to move")));
    counted().fail_after(0);
    auto r = v.try_set("b", std::move(big));
    counted().fail_after(-1);
    CHECK(r.code() == guatiao::status::alloc);
    CHECK(big.string_or("") == "a value to move");  // still ours
    CHECK(v.size() == 1);
  }
  CHECK_NONE_LEFT("a refused set");

  // Text and numbers refused before anything is built.
  CHECK(guatiao::try_make(item::map({{"ok", "x"}, {"bad", item::number("--1")}})).code() ==
        guatiao::status::bad_value);
  CHECK_NONE_LEFT("a refused number inside a map");
}

void wire_paths() {
  {
    guatiao::value v = MUST(guatiao::try_make(sample()));
    auto bytes = MUST(guatiao::try_to_wire(v));
    guatiao::value back = MUST(guatiao::try_from_wire(bytes));
    CHECK(back == v);
    bytes.resize(bytes.size() - 1);
    CHECK(guatiao::try_from_wire(bytes).code() == guatiao::status::bad_value);
  }
  CHECK_NONE_LEFT("encoding, decoding and a truncated decode");

  guatiao::value wire = MUST(gt_test::read_json(gt_test::read_repo_file("crates/guatiao/tests/vectors/wire.json")));
  {
    guatiao::receiver r;
    for (guatiao::view c : wire["conversation"].as_list()) {
      auto frame = gt_test::bytes_of_hex(c["frame"].string_or(""));
      auto ev = r.try_accept(frame);
      (void)ev;
    }
    auto close = MUST(guatiao::try_wire_close(7));
    auto announce = MUST(guatiao::try_wire_announce(1, wire["frames"][0]["tree"]));
    (void)close;
    (void)announce;
  }
  wire = guatiao::value();
  CHECK_NONE_LEFT("a receiver through every accepted and refused frame");
}

void schema_paths() {
  {
    guatiao::value schema = MUST(guatiao::try_make(item::map({
        {"type", "object"},
        {"properties", item::map({{"name", item::map({{"type", "string"}})},
                                  {"pw", item::map({{"type", "string"}, {"writeOnly", true}})}})},
        {"additionalProperties", false},
    })));
    guatiao::value ok = MUST(guatiao::try_make(item::map({{"name", "a"}, {"pw", "b"}})));
    guatiao::value bad = MUST(guatiao::try_make(item::map({{"name", 1}, {"extra", 2}})));
    CHECK(guatiao::try_validate(schema, ok).ok());
    auto refused = guatiao::try_validate(schema, bad);
    CHECK(!refused.ok() && !refused.error().detail().is_absent());
    guatiao::value shown = MUST(guatiao::try_redact(schema, ok, "-"));
    CHECK(guatiao::try_redact(ok, ok, "\xff").code() == guatiao::status::bad_value);
    guatiao::value merged = MUST(guatiao::try_merge(guatiao::merge_mode::deep, ok, bad));
    guatiao::value list = MUST(guatiao::try_make(item::list({1})));
    auto clash = guatiao::try_merge(guatiao::merge_mode::deep, ok, list);
    (void)clash;
  }
  CHECK_NONE_LEFT("validating, redacting and merging, with their refusals");
}

void time_paths() {
  {
    CHECK(MUST(guatiao::date::try_parse("2026-10-21")).try_to_string().ok());
    CHECK(!guatiao::date::try_parse("2026-02-30").ok());
    CHECK(!guatiao::span::try_parse("1h 1h").ok());
    CHECK(guatiao::try_normalize(guatiao::form::timestamp, "2026-10-21T19:15:00+05:30").ok());
    CHECK(!guatiao::try_normalize(guatiao::form::time, "25:00:00").ok());
    guatiao::value len = MUST(guatiao::try_make(guatiao::length{90, 5}.to_item()));
    CHECK(guatiao::length::try_read(len).ok());
  }
  CHECK_NONE_LEFT("reading and writing the text forms, refusals included");
}

void settling() {
  // A tree built in another allocator moves into the one asked for; a
  // tree already there moves nothing.
  gt_test::counting other;
  {
    guatiao::value v = MUST(guatiao::try_make(sample(), other.get()));
    guatiao_value raw = v.release();
    CHECK(guatiao::detail::settle(raw, counted().get()) == GUATIAO_OK);
    CHECK(other.live() == 0);
    CHECK(guatiao::detail::lives_in(raw, counted().get()));
    long total = counted().total();
    CHECK(guatiao::detail::settle(raw, counted().get()) == GUATIAO_OK);
    CHECK(counted().total() == total);
    guatiao::value back = guatiao::value::adopt(raw);
  }
  CHECK_NONE_LEFT("a tree settled into this allocator");
}

}  // namespace

int main() {
  builds_and_drops();
  every_refused_build_frees();
  wire_paths();
  schema_paths();
  time_paths();
  settling();
  return gt_test::finish("frees");
}
