// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Included before <guatiao/guatiao.hpp>: makes a counting allocator the
// binding's default, so a test sees every block the binding builds.

#ifndef GT_TEST_COUNTING_DEFAULT_HPP
#define GT_TEST_COUNTING_DEFAULT_HPP

#include <guatiao.h>

namespace gt_test {
inline const guatiao_alloc* counting_default() noexcept;
}  // namespace gt_test

#define GUATIAO_CPP_DEFAULT_ALLOC ::gt_test::counting_default

#include "counting.hpp"

namespace gt_test {

inline counting& counted() {
  static counting c;
  return c;
}

inline const guatiao_alloc* counting_default() noexcept { return counted().get(); }

}  // namespace gt_test

#endif  // GT_TEST_COUNTING_DEFAULT_HPP
