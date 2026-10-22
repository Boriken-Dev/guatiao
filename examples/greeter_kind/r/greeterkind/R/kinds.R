# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# The kinds of include/greeter_kind.h, declared for R: each slot in the
# order its table declares it, its shapes beside it, each floor hash copied
# from the #define beside the table. A test compares the literals with the
# header. A host calls a greeter through Greeter; a greeter written in R is
# a class containing Greeter with methods of these generics.

# listener_vtable: what a conversation tells, implemented by the host.
Listener <- kind("listener", floor_hash = 1867921573, object = TRUE,
  heard = method(what = "string", required = TRUE))

# conversation_vtable: one conversation a greeter started.
Conversation <- kind("conversation", floor_hash = 2769624503, object = TRUE,
  say = method(what = "string", required = TRUE, fallible = TRUE),
  read = method(dst = lent("bytes"), returns = "int64", required = TRUE),
  turns = method(returns = "int64", required = TRUE))

# greeter_vtable: greets by name. greet is the one slot every table has;
# the rest were appended, and an older table may lack them.
Greeter <- kind("greeter", floor_hash = 2695856491,
  greet = method(name = "string", returns = "map", required = TRUE, fallible = TRUE),
  shout = method(name = "string", returns = "string"),
  start = method(listener = taken(Listener), returns = Conversation, fallible = TRUE),
  greet_into = method(name = "string", listener = lent(Listener), fallible = TRUE),
  greet_from = method(config = taken("map"), returns = "map", fallible = TRUE),
  check = method(config = lent("map"), returns = "bool", fallible = TRUE))

# counter_vtable: counts how often it was asked.
Counter <- kind("counter", floor_hash = 1227528937,
  count = method(returns = "int64", required = TRUE))

# wide_vtable: answers the 64-bit integer it is given.
Wide <- kind("wide", floor_hash = 3673689684,
  echo_i64 = method(x = "int64", returns = "int64", required = TRUE, fallible = TRUE),
  echo_u64 = method(x = "uint64", returns = "uint64", required = TRUE, fallible = TRUE))
