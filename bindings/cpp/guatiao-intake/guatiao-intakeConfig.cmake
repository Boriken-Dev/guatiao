# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# find_package(guatiao-intake CONFIG), straight from a checkout:
#   cmake -Dguatiao-intake_DIR=<repo>/bindings/cpp/guatiao-intake ...
# then target_link_libraries(app PRIVATE guatiao::intake).
include("${CMAKE_CURRENT_LIST_DIR}/cmake/guatiao-intake-targets.cmake")
set(guatiao-intake_FOUND TRUE)
