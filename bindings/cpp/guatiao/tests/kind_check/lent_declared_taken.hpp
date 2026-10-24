// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Must disagree with greeter_kind.kinds.json: `check` takes its map lent
// (`map_ref`), declared here as handed over (`map`). It compiles, since
// both are the same C pointer, and would free the caller's map.

GUATIAO_KIND(greeter, greeter_vtable,
             (required, greet, guatiao::map(std::string_view)),
             (optional, shout, std::string(std::string_view)),
             (optional, start, guatiao::object<conversation>(guatiao::object<listener>)),
             (optional, greet_into, void(std::string_view, guatiao::lent<listener>)),
             (optional, greet_from, guatiao::map(guatiao::map)),
             (optional, check, bool(guatiao::map)));
