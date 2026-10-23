# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# How this repository's C++ suites are built and run: warnings as errors,
# the C++ standard CMAKE_CXX_STANDARD names (17 unless set), each suite
# written against the `try_` forms built twice, with exceptions on and
# off, and run with every guatiao library on PATH.
#
#   guatiao_cpp_test(<name> SOURCES <file>... LIBS <target>...
#                    [BOTH_EXCEPTION_MODES] [DEFINITIONS <def>...])

include_guard(GLOBAL)

if(NOT DEFINED CMAKE_CXX_STANDARD)
  set(CMAKE_CXX_STANDARD 17)
endif()
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

# Whether exceptions are on is per suite, so the global default goes.
string(REPLACE "/EHsc" "" CMAKE_CXX_FLAGS "${CMAKE_CXX_FLAGS}")

get_filename_component(GUATIAO_CPP_TEST_SUPPORT "${CMAKE_CURRENT_LIST_DIR}/../tests" ABSOLUTE)
get_filename_component(GUATIAO_REPO "${CMAKE_CURRENT_LIST_DIR}/../../../.." ABSOLUTE)

function(guatiao_cpp_test name)
  cmake_parse_arguments(arg "BOTH_EXCEPTION_MODES" "" "SOURCES;LIBS;DEFINITIONS" ${ARGN})
  set(modes exceptions)
  if(arg_BOTH_EXCEPTION_MODES)
    list(APPEND modes noexcept)
  endif()
  guatiao_runtime_dirs(dirs)
  foreach(mode IN LISTS modes)
    set(t ${name})
    if(mode STREQUAL "noexcept")
      set(t ${name}_noexcept)
    endif()
    add_executable(${t} ${arg_SOURCES})
    target_link_libraries(${t} PRIVATE ${arg_LIBS})
    target_include_directories(${t} PRIVATE "${GUATIAO_CPP_TEST_SUPPORT}")
    target_compile_definitions(${t} PRIVATE "GUATIAO_REPO=\"${GUATIAO_REPO}\"" _CRT_SECURE_NO_WARNINGS
                               ${arg_DEFINITIONS})
    # The binding's headers are what is under test: their warnings count.
    set_target_properties(${t} PROPERTIES NO_SYSTEM_FROM_IMPORTED TRUE)
    if(MSVC)
      target_compile_options(${t} PRIVATE /W4 /WX /permissive- /utf-8)
      if(mode STREQUAL "noexcept")
        target_compile_options(${t} PRIVATE /EHs-c-)
        target_compile_definitions(${t} PRIVATE _HAS_EXCEPTIONS=0)
      else()
        target_compile_options(${t} PRIVATE /EHsc)
      endif()
    else()
      target_compile_options(${t} PRIVATE -Wall -Wextra -Wpedantic -Werror)
      if(mode STREQUAL "noexcept")
        target_compile_options(${t} PRIVATE -fno-exceptions)
      endif()
    endif()
    add_test(NAME ${t} COMMAND ${t})
    if(WIN32)
      set(path_mods "")
      foreach(dir IN LISTS dirs)
        list(APPEND path_mods "PATH=path_list_prepend:${dir}")
      endforeach()
      set_tests_properties(${t} PROPERTIES ENVIRONMENT_MODIFICATION "${path_mods}")
    endif()
  endforeach()
endfunction()
