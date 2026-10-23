# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# The C++ binding over guatiao-intake, header-only: guatiao::intake as a
# subdirectory, its tests on its own.
cmake_minimum_required(VERSION 3.22)
project(guatiao_intake_cpp VERSION 0.0.0 LANGUAGES CXX)

include("${CMAKE_CURRENT_SOURCE_DIR}/cmake/guatiao-intake-targets.cmake")

option(GUATIAO_CPP_TESTS "Build the guatiao C++ binding's tests" ${PROJECT_IS_TOP_LEVEL})
if(GUATIAO_CPP_TESTS)
  enable_testing()
  add_subdirectory(tests)
endif()
