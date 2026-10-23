// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Must not compile: `greet` takes an int where the greeter kind's slot takes
// a name, so the class cannot serve the kind.

#include "greeter_kind.hpp"

struct wrong {
  guatiao::map greet(int times);
};

guatiao::library make() {
  guatiao::library lib("wrong", "1.0.0");
  lib.single<wrong, greeter_kind::greeter>("wrong");
  return lib;
}

GUATIAO_LIBRARY(make, greeter_kind::greeter)
