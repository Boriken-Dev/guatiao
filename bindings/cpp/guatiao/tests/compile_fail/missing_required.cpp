// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Must not compile: the class serves the greeter kind without `greet`, a
// slot the kind requires.

#include "greeter_kind.hpp"

struct silent {
  std::string shout(std::string_view name) { return std::string(name); }
};

guatiao::library make() {
  guatiao::library lib("silent", "1.0.0");
  lib.single<silent, greeter_kind::greeter>("silent");
  return lib;
}

GUATIAO_LIBRARY(make, greeter_kind::greeter)
