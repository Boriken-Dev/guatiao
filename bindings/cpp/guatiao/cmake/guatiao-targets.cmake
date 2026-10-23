# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# guatiao::guatiao: the C++ headers, <guatiao.h>, and the guatiao library
# they call, found as GuatiaoNative.cmake says. Read by find_package
# (guatiaoConfig.cmake) and by add_subdirectory (CMakeLists.txt) alike.

include_guard(GLOBAL)
include("${CMAKE_CURRENT_LIST_DIR}/GuatiaoNative.cmake")

guatiao_native(guatiao_native_core NAME guatiao VARIABLE GUATIAO_LIBRARY CRATE guatiao FEATURES load regex)
set_property(GLOBAL APPEND PROPERTY GUATIAO_RUNTIME_DIRS "${guatiao_native_core_DIRECTORY}")

get_filename_component(_guatiao_cpp_include "${CMAKE_CURRENT_LIST_DIR}/../include" ABSOLUTE)
get_filename_component(_guatiao_c_include "${_GUATIAO_WORKSPACE}/crates/guatiao/include" ABSOLUTE)

add_library(guatiao::guatiao INTERFACE IMPORTED GLOBAL)
target_include_directories(guatiao::guatiao INTERFACE "${_guatiao_cpp_include}" "${_guatiao_c_include}")
target_link_libraries(guatiao::guatiao INTERFACE guatiao_native_core)
target_compile_features(guatiao::guatiao INTERFACE cxx_std_17)

# The directories holding every guatiao library found so far: what a
# program's PATH needs on Windows to start.
function(guatiao_runtime_dirs out)
  get_property(dirs GLOBAL PROPERTY GUATIAO_RUNTIME_DIRS)
  list(REMOVE_DUPLICATES dirs)
  set(${out} "${dirs}" PARENT_SCOPE)
endfunction()

# Makes `target` (a SHARED or MODULE library holding GUATIAO_LIBRARY) a
# guatiao library file: only its two entry symbols exported, so no inline
# function, table or static of it is merged with the host's or another
# plugin's; named as a Rust cdylib names its file (no `lib` on Windows).
function(guatiao_library_target target)
  set_target_properties(${target} PROPERTIES CXX_VISIBILITY_PRESET hidden VISIBILITY_INLINES_HIDDEN ON)
  if(WIN32)
    set_target_properties(${target} PROPERTIES PREFIX "")
  endif()
  target_link_libraries(${target} PRIVATE guatiao::guatiao)
endfunction()

# Copies the guatiao libraries `target` links beside it after each build:
# on Windows a program finds a DLL there.
function(guatiao_copy_runtime target)
  if(WIN32)
    set(dlls "$<TARGET_RUNTIME_DLLS:${target}>")
    add_custom_command(TARGET ${target} POST_BUILD
      COMMAND "$<IF:$<BOOL:${dlls}>,${CMAKE_COMMAND};-E;copy_if_different;${dlls};$<TARGET_FILE_DIR:${target}>,${CMAKE_COMMAND};-E;true>"
      COMMAND_EXPAND_LISTS)
  endif()
endfunction()
