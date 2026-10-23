// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Must not compile: a pointer is not a truth value.

#include <guatiao/guatiao.hpp>

int main() {
  int x = 0;
  auto v = guatiao::try_make(guatiao::item(&x));
  return v.ok() ? 0 : 1;
}
