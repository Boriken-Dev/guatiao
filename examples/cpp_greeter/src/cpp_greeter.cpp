// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Greeters written in C++: the same kinds the Rust, C, Python, Go and
// Fortran greeters serve, offered by a library any host calls whatever
// language wrote it. `hello` is its one instance and serves the greeter
// and the counter; `shouter` is built from a configuration, one instance
// per configuration. GUATIAO_LIBRARY makes the shared library a guatiao
// library a scan finds by its declared kinds.

#include "cpp_greeter.hpp"

#include <stdexcept>

namespace cpp_greeter {

namespace gk = greeter_kind;
using guatiao::item;

// ---- a conversation ------------------------------------------------------

conversation::conversation(guatiao::object<gk::listener> listener) : listener_(std::move(listener)) {}

void conversation::say(std::string_view what) {
  if (what.empty()) throw guatiao::error(guatiao::status::bad_value, "nothing to say");
  said_.emplace_back(what);
  listener_.heard(what);
}

std::int64_t conversation::read(guatiao::bytes_mut dst) {
  std::string all;
  for (const std::string& s : said_) {
    if (!all.empty()) all += '\n';
    all += s;
  }
  std::size_t n = all.size() < dst.size() ? all.size() : dst.size();
  if (n) std::memcpy(dst.data(), all.data(), n);
  return static_cast<std::int64_t>(n);
}

std::int64_t conversation::turns() { return static_cast<std::int64_t>(said_.size()); }

// ---- hello -------------------------------------------------------------------

guatiao::map hello::greet(std::string_view name) {
  if (name == "panic") throw std::logic_error("a provider bug, on request");
  if (name.empty()) throw guatiao::error(guatiao::status::bad_value, "nobody to greet");
  return guatiao::map{{"greeting", "hello, " + std::string(name)}};
}

std::string hello::shout(std::string_view name) {
  std::string loud = "hello, " + std::string(name);
  for (char& c : loud)
    if (c >= 'a' && c <= 'z') c = static_cast<char>(c - 'a' + 'A');
  return loud;
}

guatiao::object<gk::conversation> hello::start(guatiao::object<gk::listener> listener) {
  return guatiao::object<gk::conversation>::make<conversation>(std::move(listener));
}

void hello::greet_into(std::string_view name, guatiao::lent<gk::listener> listener) {
  if (name.empty()) throw guatiao::error(guatiao::status::bad_value, "nobody to greet");
  listener.heard("hello, " + std::string(name));
}

guatiao::map hello::greet_from(guatiao::map config) { return greet(config["name"].string_or("")); }

bool hello::check(guatiao::map_ref config) {
  std::string name(config["name"].string_or("world"));
  config.set("name", name);
  config.set("greeting", "hello, " + name);
  return !name.empty();
}

std::int64_t hello::count() { return asked_.fetch_add(1) + 1; }

// ---- shouter ---------------------------------------------------------------------

shouter::shouter(guatiao::view config) : prefix_(config["prefix"].string_or("")) {
  if (prefix_.empty()) throw guatiao::error(guatiao::status::bad_value, "a shouter needs a prefix");
}

item shouter::schema() {
  return item::map({
      {"type", "object"},
      {"properties",
       item::map({{"prefix", item::map({{"type", "string"},
                                         {"minLength", 1},
                                         {"description", "What every greeting starts with."}})}})},
      {"required", item::list({"prefix"})},
      {"additionalProperties", false},
  });
}

guatiao::map shouter::greet(std::string_view name) {
  std::string loud(name);
  for (char& c : loud)
    if (c >= 'a' && c <= 'z') c = static_cast<char>(c - 'a' + 'A');
  return guatiao::map{{"greeting", prefix_ + " " + loud}};
}

// ---- the library -------------------------------------------------------------------

guatiao::library library() {
  guatiao::library lib(LIBRARY_ID, LIBRARY_VERSION);
  lib.meta(item::map({{"summary", "Greeters written in C++"}, {"license", "MPL-2.0"}}));
  lib.single<hello, gk::greeter, gk::counter>("hello").display_name("Hello, in C++");
  lib.configured<shouter, gk::greeter>("shouter").display_name("Shouter, in C++");
  return lib;
}

}  // namespace cpp_greeter

GUATIAO_LIBRARY(cpp_greeter::library, greeter_kind::greeter, greeter_kind::counter)
