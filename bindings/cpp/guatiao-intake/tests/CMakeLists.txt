# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

include("${CMAKE_CURRENT_SOURCE_DIR}/../../guatiao/cmake/GuatiaoTests.cmake")

# The library of four providers a settings screen lists, written in C: the
# test scans the folder it is built into and gives each catalog entry its form.
enable_language(C)
add_library(catalog_library SHARED "${GUATIAO_REPO}/crates/guatiao/tests/c_consumer/catalog_library.c")
target_include_directories(catalog_library PRIVATE "${GUATIAO_REPO}/crates/guatiao/include"
                           "${GUATIAO_REPO}/examples/greeter_kind/include")
set_target_properties(catalog_library PROPERTIES C_STANDARD 11 LIBRARY_OUTPUT_DIRECTORY
                      "${CMAKE_CURRENT_BINARY_DIR}/catalog" RUNTIME_OUTPUT_DIRECTORY
                      "${CMAKE_CURRENT_BINARY_DIR}/catalog")
if(WIN32)
  set_target_properties(catalog_library PROPERTIES PREFIX "")
endif()

guatiao_cpp_test(cpp_intake SOURCES test_intake.cpp LIBS guatiao::intake BOTH_EXCEPTION_MODES
                 DEFINITIONS "GUATIAO_CATALOG_DIR=\"${CMAKE_CURRENT_BINARY_DIR}/catalog\"")
add_dependencies(cpp_intake catalog_library)
add_dependencies(cpp_intake_noexcept catalog_library)
