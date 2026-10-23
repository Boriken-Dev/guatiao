// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Must not compile: the slot takes a 64-bit integer, and a pointer is not
// one.

#include "greeter_kind.hpp"

std::int64_t echo(const guatiao::remote<greeter_kind::wide>& w) {
  int x = 0;
  return w.echo_i64(&x);
}
