# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# guatiao::intake: <guatiao/intake.hpp>, <guatiao_intake.h> and the
# guatiao_intake library, over guatiao::guatiao.

include_guard(GLOBAL)
include("${CMAKE_CURRENT_LIST_DIR}/../../guatiao/cmake/guatiao-targets.cmake")

guatiao_native(guatiao_native_intake NAME guatiao_intake VARIABLE GUATIAO_INTAKE_LIBRARY CRATE guatiao-intake)
set_property(GLOBAL APPEND PROPERTY GUATIAO_RUNTIME_DIRS "${guatiao_native_intake_DIRECTORY}")

get_filename_component(_guatiao_intake_cpp_include "${CMAKE_CURRENT_LIST_DIR}/../include" ABSOLUTE)
get_filename_component(_guatiao_intake_c_include "${_GUATIAO_WORKSPACE}/crates/guatiao-intake/include" ABSOLUTE)

add_library(guatiao::intake INTERFACE IMPORTED GLOBAL)
target_include_directories(guatiao::intake INTERFACE "${_guatiao_intake_cpp_include}" "${_guatiao_intake_c_include}")
target_link_libraries(guatiao::intake INTERFACE guatiao::guatiao guatiao_native_intake)
