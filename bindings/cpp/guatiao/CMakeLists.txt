# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# The C++ binding over guatiao, header-only. As a subdirectory (or
# FetchContent with SOURCE_SUBDIR bindings/cpp/guatiao) it defines
# guatiao::guatiao; on its own it builds and runs its tests:
#
#   cmake -S bindings/cpp/guatiao -B build/cpp && cmake --build build/cpp && ctest --test-dir build/cpp
cmake_minimum_required(VERSION 3.22)
project(guatiao_cpp VERSION 0.0.0 LANGUAGES CXX)

include("${CMAKE_CURRENT_SOURCE_DIR}/cmake/guatiao-targets.cmake")

option(GUATIAO_CPP_TESTS "Build the guatiao C++ binding's tests" ${PROJECT_IS_TOP_LEVEL})
if(GUATIAO_CPP_TESTS)
  enable_testing()
  add_subdirectory(tests)
endif()
