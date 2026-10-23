# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# A plugin host written in C++, and its test calling the greeters written
# in Rust, C, Python, Go and C++:
#
#   cmake -S examples/cpp_host -B build && cmake --build build && ctest --test-dir build
#
# It builds the C greeter (from the C consumer tests) and the C++ greeter
# (examples/cpp_greeter) as library files of their own; the Rust greeters
# are the workspace's build (GUATIAO_RUST_GREETERS, default: where the
# guatiao library was found), the Go greeter GUATIAO_GO_LIBRARY, the
# Python ones served by the bridge GUATIAO_PY_BRIDGE names.
cmake_minimum_required(VERSION 3.22)
project(cpp_host VERSION 1.0.0 LANGUAGES C CXX)

find_package(guatiao CONFIG REQUIRED PATHS "${CMAKE_CURRENT_SOURCE_DIR}/../../bindings/cpp/guatiao" NO_DEFAULT_PATH)
find_package(Threads REQUIRED)

set(kinds_include "${CMAKE_CURRENT_SOURCE_DIR}/../greeter_kind/include" "${CMAKE_CURRENT_SOURCE_DIR}/../greeter_kind/cpp")

add_executable(cpp_host src/cpp_host.cpp)
target_include_directories(cpp_host PRIVATE ${kinds_include})
target_link_libraries(cpp_host PRIVATE guatiao::guatiao)
guatiao_copy_runtime(cpp_host)

# The C++ greeter, as a file of its own a scan finds.
add_subdirectory("${CMAKE_CURRENT_SOURCE_DIR}/../cpp_greeter" cpp_greeter)
set(cpp_greeter_dir "${CMAKE_BINARY_DIR}/greeters/cpp")
set_target_properties(cpp_greeter PROPERTIES
  LIBRARY_OUTPUT_DIRECTORY "${cpp_greeter_dir}" RUNTIME_OUTPUT_DIRECTORY "${cpp_greeter_dir}")

# The C greeter: a library written in C that links nothing.
set(c_greeter_dir "${CMAKE_BINARY_DIR}/greeters/c")
add_library(c_greeter SHARED "${CMAKE_CURRENT_SOURCE_DIR}/../../crates/guatiao/tests/c_consumer/greeter_in_c.c")
target_include_directories(c_greeter PRIVATE ${kinds_include} "${CMAKE_CURRENT_SOURCE_DIR}/../../crates/guatiao/include")
set_target_properties(c_greeter PROPERTIES
  C_STANDARD 11
  LIBRARY_OUTPUT_DIRECTORY "${c_greeter_dir}" RUNTIME_OUTPUT_DIRECTORY "${c_greeter_dir}")
if(WIN32)
  set_target_properties(c_greeter PROPERTIES PREFIX "")
endif()

option(CPP_HOST_TESTS "Build the C++ host's test" ON)
if(CPP_HOST_TESTS)
  enable_testing()
  get_filename_component(repo "${CMAKE_CURRENT_SOURCE_DIR}/../.." ABSOLUTE)
  add_executable(test_greeters test/test_greeters.cpp)
  target_include_directories(test_greeters PRIVATE ${kinds_include})
  target_link_libraries(test_greeters PRIVATE guatiao::guatiao Threads::Threads ${CMAKE_DL_LIBS})
  target_compile_definitions(test_greeters PRIVATE
    "GUATIAO_REPO=\"${repo}\""
    "RUST_GREETERS_DIR=\"${guatiao_native_core_DIRECTORY}\""
    "C_GREETER_DIR=\"${c_greeter_dir}\""
    "CPP_GREETER_DIR=\"${cpp_greeter_dir}\"")
  add_dependencies(test_greeters cpp_greeter c_greeter)
  add_test(NAME cpp_host_greeters COMMAND test_greeters)
  guatiao_runtime_dirs(runtime_dirs)
  if(WIN32)
    set(path_mods "")
    foreach(dir IN LISTS runtime_dirs)
      list(APPEND path_mods "PATH=path_list_prepend:${dir}")
    endforeach()
    set_tests_properties(cpp_host_greeters PROPERTIES ENVIRONMENT_MODIFICATION "${path_mods}")
  endif()
endif()
