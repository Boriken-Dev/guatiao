# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# guatiao::serde: <guatiao/serde.hpp>, <guatiao_serde.h> and the
# guatiao_serde library, over guatiao::guatiao.
#
# GUATIAO_SERDE_FEATURES (default "toml;yaml"; JSON is always there) is
# what a cargo build turns on, and which of GUATIAO_SERDE_TOML and
# GUATIAO_SERDE_YAML the headers are told: name the features the library
# was built with when GUATIAO_SERDE_LIBRARY or GUATIAO_LIBRARY names one.

include_guard(GLOBAL)
include("${CMAKE_CURRENT_LIST_DIR}/../../guatiao/cmake/guatiao-targets.cmake")

if(NOT DEFINED GUATIAO_SERDE_FEATURES)
  set(GUATIAO_SERDE_FEATURES toml yaml)
endif()

guatiao_native(guatiao_native_serde NAME guatiao_serde VARIABLE GUATIAO_SERDE_LIBRARY CRATE guatiao-serde
               FEATURES ${GUATIAO_SERDE_FEATURES})
set_property(GLOBAL APPEND PROPERTY GUATIAO_RUNTIME_DIRS "${guatiao_native_serde_DIRECTORY}")

get_filename_component(_guatiao_serde_cpp_include "${CMAKE_CURRENT_LIST_DIR}/../include" ABSOLUTE)
get_filename_component(_guatiao_serde_c_include "${_GUATIAO_WORKSPACE}/crates/guatiao-serde/include" ABSOLUTE)

add_library(guatiao::serde INTERFACE IMPORTED GLOBAL)
target_include_directories(guatiao::serde INTERFACE "${_guatiao_serde_cpp_include}" "${_guatiao_serde_c_include}")
target_link_libraries(guatiao::serde INTERFACE guatiao::guatiao guatiao_native_serde)
if("toml" IN_LIST GUATIAO_SERDE_FEATURES)
  target_compile_definitions(guatiao::serde INTERFACE GUATIAO_SERDE_TOML)
endif()
if("yaml" IN_LIST GUATIAO_SERDE_FEATURES)
  target_compile_definitions(guatiao::serde INTERFACE GUATIAO_SERDE_YAML)
endif()
