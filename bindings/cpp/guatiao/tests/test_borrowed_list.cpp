// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// A list a slot borrows: a `const guatiao_list *` the caller keeps, which a
// class serving the kind reads through a `list_view`. Registered in this
// process and called through its table as any host calls it.

#include "support/guatiao_counting_default.hpp"  // first: it names the default allocator

#include "support/harness.hpp"

// The kind's C table, as a kind header would declare it.
extern "C" {
typedef struct summer_vtable {
  guatiao_kind_header header;
  guatiao_status (*sum)(void* ctx, const guatiao_list* items, int64_t* out);
} summer_vtable;
}
#define summer_vtable_FLOOR_HASH 2920396019u

namespace {

using gt_test::counted;

GUATIAO_KIND(summer, summer_vtable, (required, sum, std::int64_t(guatiao::list_view)));

struct adds {
  std::int64_t sum(guatiao::list_view items) {
    std::int64_t total = 0;
    for (auto item : items) total += item.int_or(0);
    return total;
  }
};

guatiao::library cpp_adds() {
  guatiao::library lib("cpp_adds", "1.0.0");
  lib.single<adds, summer>("adds");
  return lib;
}

}  // namespace

int main() {
  {
    guatiao::registry reg = MUST(guatiao::registry::try_open("cpp-borrowed-list", "0.1", counted().get()));
    guatiao::value loaded = MUST(reg.try_register_library<&cpp_adds>("cpp_adds"));
    CHECK(loaded.contains("loaded"));
    auto s = MUST(summer::try_bind(reg, "cpp_adds_adds"));
    guatiao::value items = MUST(guatiao::try_make(guatiao::item::list({40, 2})));
    CHECK(MUST(s.try_sum(items.as_list())) == 42);
    CHECK_MSG(items.size() == 2 && items[0].int_or(0) == 40, "the caller's list, as it was");
  }
  CHECK_MSG(counted().live() == 0, std::to_string(counted().live()) + " blocks left");
  return gt_test::finish("borrowed list");
}
