// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Must not compile: the counter's C slot answers an int64_t, and this
// declaration says it answers text.

#include <greeter_kind.h>
#include <guatiao/guatiao.hpp>

namespace misread {
GUATIAO_KIND(counter, counter_vtable, (required, count, std::string()));
}

int main() { return 0; }
