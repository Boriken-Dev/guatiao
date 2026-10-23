# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# find_package(guatiao CONFIG), straight from a checkout:
#   cmake -Dguatiao_DIR=<repo>/bindings/cpp/guatiao ...
# then target_link_libraries(app PRIVATE guatiao::guatiao).
include("${CMAKE_CURRENT_LIST_DIR}/cmake/guatiao-targets.cmake")
set(guatiao_FOUND TRUE)
