# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# A project outside guatiao consuming its three C++ packages the way any
# CMake project does, knowing nothing of the repository's layout:
#
#   cmake -S examples/cpp_consumer -B build \
#     -Dguatiao_DIR=<repo>/bindings/cpp/guatiao \
#     -Dguatiao-serde_DIR=<repo>/bindings/cpp/guatiao-serde \
#     -Dguatiao-intake_DIR=<repo>/bindings/cpp/guatiao-intake
#   cmake --build build && ctest --test-dir build
#
# With GUATIAO_LIBRARY naming a build of the libraries they are linked;
# without it, configuring builds them with cargo.
cmake_minimum_required(VERSION 3.22)
project(cpp_consumer LANGUAGES CXX)

find_package(guatiao CONFIG REQUIRED)
find_package(guatiao-serde CONFIG REQUIRED)
find_package(guatiao-intake CONFIG REQUIRED)

add_executable(cpp_consumer main.cpp)
target_link_libraries(cpp_consumer PRIVATE guatiao::guatiao guatiao::serde guatiao::intake)
guatiao_copy_runtime(cpp_consumer)

enable_testing()
add_test(NAME cpp_consumer COMMAND cpp_consumer)
