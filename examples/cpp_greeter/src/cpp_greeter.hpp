// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The greeters this library serves, as classes: each method is a slot of
// a kind in greeter_kind.hpp, and a method that does not fit its slot is
// a compile error.

#ifndef CPP_GREETER_HPP
#define CPP_GREETER_HPP

#include "greeter_kind.hpp"

#include <atomic>
#include <string>
#include <vector>

namespace cpp_greeter {

constexpr const char* LIBRARY_ID = "cpp_greeter";
constexpr const char* LIBRARY_VERSION = "1.0.0";

/// One conversation `start` hands back, holding the listener it was
/// handed; destroying it destroys the listener.
class conversation {
 public:
  explicit conversation(guatiao::object<greeter_kind::listener> listener);
  void say(std::string_view what);
  std::int64_t read(guatiao::bytes_mut dst);
  std::int64_t turns();

 private:
  guatiao::object<greeter_kind::listener> listener_;
  std::vector<std::string> said_;
};

/// Greets, shouts, holds conversations; counts how often it was asked to
/// count.
class hello {
 public:
  guatiao::map greet(std::string_view name);
  std::string shout(std::string_view name);
  guatiao::object<greeter_kind::conversation> start(guatiao::object<greeter_kind::listener> listener);
  void greet_into(std::string_view name, guatiao::lent<greeter_kind::listener> listener);
  guatiao::map greet_from(guatiao::map config);
  bool check(guatiao::map_ref config);
  std::int64_t count();

 private:
  std::atomic<std::int64_t> asked_{0};
};

/// Built from `{"prefix": text}`: greets with the prefix and the name
/// shouted.
class shouter {
 public:
  explicit shouter(guatiao::view config);
  static guatiao::item schema();
  guatiao::map greet(std::string_view name);

 private:
  std::string prefix_;
};

/// The library: `hello` and `shouter`.
guatiao::library library();

}  // namespace cpp_greeter

#endif  // CPP_GREETER_HPP
