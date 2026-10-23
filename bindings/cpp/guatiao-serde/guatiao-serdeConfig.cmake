# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# find_package(guatiao-serde CONFIG), straight from a checkout:
#   cmake -Dguatiao-serde_DIR=<repo>/bindings/cpp/guatiao-serde ...
# then target_link_libraries(app PRIVATE guatiao::serde).
include("${CMAKE_CURRENT_LIST_DIR}/cmake/guatiao-serde-targets.cmake")
set(guatiao-serde_FOUND TRUE)
