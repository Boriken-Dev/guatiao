# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# The greeters as a library file any host's scan finds:
#
#   cmake -S examples/cpp_greeter -B build && cmake --build build
#
# writes cpp_greeter.dll / libcpp_greeter.so / .dylib into the build
# directory, exporting guatiao_library_entry and guatiao_declares and
# linking the guatiao library the package found.
cmake_minimum_required(VERSION 3.22)
project(cpp_greeter VERSION 1.0.0 LANGUAGES CXX)

find_package(guatiao CONFIG REQUIRED PATHS "${CMAKE_CURRENT_SOURCE_DIR}/../../bindings/cpp/guatiao" NO_DEFAULT_PATH)

add_library(cpp_greeter SHARED src/cpp_greeter.cpp)
target_include_directories(cpp_greeter PUBLIC
  src "${CMAKE_CURRENT_SOURCE_DIR}/../greeter_kind/include" "${CMAKE_CURRENT_SOURCE_DIR}/../greeter_kind/cpp")
# Its two entry symbols exported and nothing else, named as a Rust cdylib.
guatiao_library_target(cpp_greeter)
guatiao_copy_runtime(cpp_greeter)
