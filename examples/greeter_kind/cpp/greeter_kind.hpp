// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The greeter kinds in C++, over the tables greeter_kind.h declares: what a
// C++ host and a C++ library both include. Each line is a slot: whether a
// table must carry it, its C field, and the C++ signature a method serving
// it has (a slot with no default in the kind's own declaration is
// required).

#ifndef GREETER_KIND_HPP
#define GREETER_KIND_HPP

#include <greeter_kind.h>
#include <guatiao/guatiao.hpp>

namespace greeter_kind {

/// What a conversation tells whoever is listening: implemented by the
/// host and handed to the library.
GUATIAO_OBJECT_KIND(listener, listener_vtable,
                    (required, heard, void(std::string_view)));

/// One conversation a greeter started: state the caller drives.
GUATIAO_OBJECT_KIND(conversation, conversation_vtable,
                    (required, say, void(std::string_view)),
                    (required, read, std::int64_t(guatiao::bytes_mut)),
                    (required, turns, std::int64_t()));

/// Greets by name.
GUATIAO_KIND(greeter, greeter_vtable,
             (required, greet, guatiao::map(std::string_view)),
             (optional, shout, std::string(std::string_view)),
             (optional, start, guatiao::object<conversation>(guatiao::object<listener>)),
             (optional, greet_into, void(std::string_view, guatiao::lent<listener>)),
             (optional, greet_from, guatiao::map(guatiao::map)),
             (optional, check, bool(guatiao::map_ref)));

/// Counts how often it was asked.
GUATIAO_KIND(counter, counter_vtable,
             (required, count, std::int64_t()));

/// Answers the 64-bit integer it is given, unchanged.
GUATIAO_KIND(wide, wide_vtable,
             (required, echo_i64, std::int64_t(std::int64_t)),
             (required, echo_u64, std::uint64_t(std::uint64_t)));

}  // namespace greeter_kind

#endif  // GREETER_KIND_HPP
