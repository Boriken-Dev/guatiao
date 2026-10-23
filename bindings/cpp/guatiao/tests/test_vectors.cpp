// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The shared vectors every implementation reads: values and frames on the
// wire (`tests/vectors/wire.json`) and the text forms of time
// (`tests/vectors/time.json`). Through the `try_` forms only, so this
// suite runs with exceptions on and off.

#include <guatiao/guatiao.hpp>

#include "support/harness.hpp"
#include "support/json.hpp"

namespace {

using guatiao::item;
using guatiao::view;

/// A tree in the vectors' notation: `[tag]` or `[tag, x]`.
item tree_item(view t) {
  std::string_view tag = t[0].string_or("");
  view x = t[1];
  if (tag == "absent") return item::absent();
  if (tag == "null") return item(nullptr);
  if (tag == "bool") return item(x.bool_or(false));
  if (tag == "number") return item::number(x.string_or(""));
  if (tag == "string") return item(x.string_or(""));
  if (tag == "bytes") {
    auto b = gt_test::bytes_of_hex(x.string_or(""));
    return item::bytes(guatiao::bytes_view(b));
  }
  if (tag == "list") {
    std::vector<item> items;
    for (view e : x.as_list()) items.push_back(tree_item(e));
    return item::list_from(std::move(items));
  }
  std::vector<std::pair<std::string, item>> entries;
  for (view e : x.as_list())
    entries.emplace_back(std::string(e[0].string_or("")), tree_item(e[1]));
  return item::map_from(std::move(entries));
}

guatiao::value tree(view t) { return MUST(guatiao::try_make(tree_item(t))); }

std::uint64_t u64(view n) { return n.as<std::uint64_t>().value_or(0); }

void wire_values(view doc) {
  for (view c : doc["values"].as_list()) {
    std::string name(c["name"].string_or("?"));
    guatiao::value v = tree(c["tree"]);
    auto bytes = guatiao::try_to_wire(v);
    CHECK_MSG(bytes.ok(), name + ": encodes");
    if (bytes.ok()) CHECK_MSG(gt_test::hex_of(*bytes) == c["hex"].string_or(""), name + ": its bytes");
    auto raw = gt_test::bytes_of_hex(c["hex"].string_or(""));
    auto back = guatiao::try_from_wire(raw);
    CHECK_MSG(back.ok(), name + ": decodes");
    if (back.ok()) CHECK_MSG(*back == v, name + ": to its tree");
  }
  for (view c : doc["malformed"].as_list()) {
    std::string name(c["name"].string_or("?"));
    auto raw = gt_test::bytes_of_hex(c["hex"].string_or(""));
    auto back = guatiao::try_from_wire(raw);
    CHECK_MSG(!back.ok() && back.code() == guatiao::status::bad_value, name + ": refused as BAD_VALUE");
  }
}

void wire_frames(view doc) {
  for (view c : doc["frames"].as_list()) {
    std::string_view build = c["build"].string_or("");
    std::uint64_t channel = u64(c["channel"]);
    guatiao::result<std::vector<std::uint8_t>> got = guatiao::error(guatiao::status::internal);
    if (build == "announce") got = guatiao::try_wire_announce(channel, tree(c["tree"]));
    if (build == "send") got = guatiao::try_wire_send(channel, tree(c["tree"]));
    if (build == "close") got = guatiao::try_wire_close(channel);
    std::string what = std::string(build) + " on " + std::to_string(channel);
    CHECK_MSG(got.ok() && gt_test::hex_of(*got) == c["hex"].string_or(""), what + ": its bytes");
  }
}

void wire_conversation(view doc) {
  guatiao::receiver r;
  CHECK(static_cast<bool>(r));
  std::size_t step = 0;
  for (view c : doc["conversation"].as_list()) {
    std::string what = "conversation step " + std::to_string(step++);
    auto frame = gt_test::bytes_of_hex(c["frame"].string_or(""));
    auto got = r.try_accept(frame);
    if (auto event = c["event"].string()) {
      CHECK_MSG(got.ok(), what + ": accepted");
      if (!got.ok()) continue;
      guatiao::frame kind = *event == "schema"  ? guatiao::frame::schema
                            : *event == "value" ? guatiao::frame::value
                                                : guatiao::frame::close;
      CHECK_MSG(got->kind == kind, what + ": its kind");
      CHECK_MSG(got->channel == u64(c["channel"]), what + ": its channel");
      if (c.contains("tree")) CHECK_MSG(got->data == tree(c["tree"]), what + ": its tree");
      else CHECK_MSG(got->data.is_absent(), what + ": nothing carried");
      continue;
    }
    std::string_view refusal = c["refusal"].string_or("");
    CHECK_MSG(!got.ok(), what + ": refused");
    if (got.ok()) continue;
    view statuses = doc["statuses"];
    std::string_view want = statuses[refusal].string_or("");
    guatiao::status code = want == "GUATIAO_ERR_NOT_FOUND"    ? guatiao::status::not_found
                           : want == "GUATIAO_ERR_WRONG_KIND" ? guatiao::status::wrong_kind
                                                              : guatiao::status::bad_value;
    CHECK_MSG(got.code() == code, what + ": its status");
    view d = got.error().detail();
    CHECK_MSG(d["refusal"].string_or("") == refusal, what + ": named " + std::string(refusal));
    if (c.contains("channel")) CHECK_MSG(u64(d["channel"]) == u64(c["channel"]), what + ": its channel");
    if (c.contains("at")) CHECK_MSG(u64(d["at"]) == u64(c["at"]), what + ": the byte");
    if (c.contains("kind")) CHECK_MSG(u64(d["kind"]) == u64(c["kind"]), what + ": the frame kind");
    if (c.contains("key")) CHECK_MSG(d["detail"]["key"].string_or("") == c["key"].string_or(""), what + ": the field");
    CHECK_MSG(!got.error().message().empty(), what + ": in words");
  }
}

template <class T, class Fields>
void accepted_and_refused(view cases, guatiao::form f, const char* name, Fields fields) {
  for (view c : cases.as_list()) {
    std::string text(c["text"].string_or(""));
    std::string what = std::string(name) + " \"" + text + "\"";
    auto got = T::try_parse(text);
    auto normal = guatiao::try_normalize(f, text);
    if (c.contains("refused_at")) {
      CHECK_MSG(!got.ok() && got.code() == guatiao::status::bad_value, what + ": refused");
      if (!got.ok()) {
        CHECK_MSG(u64(got.error().detail()["at"]) == u64(c["refused_at"]), what + ": at its byte");
        CHECK_MSG(!got.error().detail()["expected"].string_or("").empty(), what + ": saying what it expected");
      }
      CHECK_MSG(!normal.ok() && u64(normal.error().detail()["at"]) == u64(c["refused_at"]),
                what + ": refused by normalize too");
      continue;
    }
    CHECK_MSG(got.ok(), what + ": read");
    if (!got.ok()) continue;
    CHECK_MSG(fields(*got) == c["fields"], what + ": its fields");
    auto written = got->try_to_string();
    CHECK_MSG(written.ok() && *written == c["canonical"].string_or(""), what + ": written canonically");
    CHECK_MSG(normal.ok() && *normal == c["canonical"].string_or(""), what + ": normalized");
  }
}

guatiao::value fields_of(std::initializer_list<item> xs) { return MUST(guatiao::try_make(item::list(xs))); }

void time_vectors(view doc) {
  accepted_and_refused<guatiao::date>(doc["date"], guatiao::form::date, "date", [](const guatiao::date& d) {
    return fields_of({d.year, d.month, d.day});
  });
  accepted_and_refused<guatiao::time_of_day>(doc["time"], guatiao::form::time, "time",
                                             [](const guatiao::time_of_day& t) {
                                               return fields_of({t.hour, t.minute, t.second, t.nanosecond});
                                             });
  accepted_and_refused<guatiao::date_time>(
      doc["datetime"], guatiao::form::datetime, "datetime", [](const guatiao::date_time& dt) {
        return fields_of({dt.date.year, dt.date.month, dt.date.day, dt.time.hour, dt.time.minute, dt.time.second,
                          dt.time.nanosecond});
      });
  accepted_and_refused<guatiao::timestamp>(doc["timestamp"], guatiao::form::timestamp, "timestamp",
                                           [](const guatiao::timestamp& t) {
                                             return fields_of({t.seconds, t.nanoseconds});
                                           });
  accepted_and_refused<guatiao::span>(doc["span"], guatiao::form::span, "span", [](const guatiao::span& s) {
    return fields_of({s.years, s.months, s.weeks, s.days, s.hours, s.minutes, s.seconds, s.nanoseconds});
  });

  for (view c : doc["date_add_span"].as_list()) {
    std::string what = std::string(c["start"].string_or("")) + " + " + std::string(c["span"].string_or(""));
    auto start = guatiao::date::try_parse(c["start"].string_or(""));
    auto sp = guatiao::span::try_parse(c["span"].string_or(""));
    CHECK_MSG(start.ok() && sp.ok(), what + ": its inputs read");
    if (!start.ok() || !sp.ok()) continue;
    auto got = start->try_add(*sp);
    if (c["result"].is_null()) {
      CHECK_MSG(!got.ok() && got.code() == guatiao::status::bad_value, what + ": refused");
    } else {
      auto text = got.ok() ? got->try_to_string() : guatiao::result<std::string>(got.error());
      CHECK_MSG(text.ok() && *text == c["result"].string_or(""), what + ": is " + std::string(c["result"].string_or("")));
    }
  }
  for (view c : doc["datetime_add_span"].as_list()) {
    std::string what = std::string(c["start"].string_or("")) + " + " + std::string(c["span"].string_or(""));
    auto start = guatiao::date_time::try_parse(c["start"].string_or(""));
    auto sp = guatiao::span::try_parse(c["span"].string_or(""));
    CHECK_MSG(start.ok() && sp.ok(), what + ": its inputs read");
    if (!start.ok() || !sp.ok()) continue;
    auto got = start->try_add(*sp);
    if (c["result"].is_null()) {
      CHECK_MSG(!got.ok() && got.code() == guatiao::status::bad_value, what + ": refused");
    } else {
      auto text = got.ok() ? got->try_to_string() : guatiao::result<std::string>(got.error());
      CHECK_MSG(text.ok() && *text == c["result"].string_or(""), what + ": is " + std::string(c["result"].string_or("")));
    }
  }
  std::size_t n = 0;
  for (view c : doc["duration"].as_list()) {
    std::string what = "exact length " + std::to_string(n++);
    auto got = guatiao::length::try_read(c["value"]);
    if (c["duration"].is_null()) {
      CHECK_MSG(!got.ok() && got.code() == guatiao::status::bad_value, what + ": refused");
      continue;
    }
    CHECK_MSG(got.ok(), what + ": read");
    if (got.ok()) {
      CHECK_MSG(got->seconds == u64(c["duration"][0]) && got->nanoseconds == u64(c["duration"][1]),
                what + ": its seconds and nanoseconds");
      guatiao::value written = MUST(guatiao::try_make(got->to_item()));
      auto again = guatiao::length::try_read(written);
      CHECK_MSG(again.ok() && *again == *got, what + ": written as [s, ns] and read back");
    }
  }
}

}  // namespace

int main() {
  guatiao::value wire = MUST(gt_test::read_json(gt_test::read_repo_file("crates/guatiao/tests/vectors/wire.json")));
  wire_values(wire);
  wire_frames(wire);
  wire_conversation(wire);
  guatiao::value time = MUST(gt_test::read_json(gt_test::read_repo_file("crates/guatiao/tests/vectors/time.json")));
  time_vectors(time);
  return gt_test::finish("vectors");
}
