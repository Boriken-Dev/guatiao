// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Must not compile: the view would read a tree freed at the end of the
// loop's first line.

#include <guatiao/guatiao.hpp>

int main() {
  std::size_t n = 0;
  for (guatiao::entry e : guatiao::try_make(guatiao::item::map({{"a", 1}})).value().as_map()) n += e.key.size();
  return static_cast<int>(n);
}
