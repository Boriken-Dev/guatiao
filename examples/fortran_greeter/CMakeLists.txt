# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# The greeters as a library file any host's scan finds:
#
#   cmake -S examples/fortran_greeter -B build && cmake --build build
#
# writes fortran_greeter.dll / libfortran_greeter.so / .dylib into the
# build directory, holding the binding, the greeter kinds and the
# providers, and exporting guatiao_library_entry and guatiao_declares.
cmake_minimum_required(VERSION 3.20)
project(fortran_greeter VERSION 1.0.0 LANGUAGES Fortran)

# CMake knows no position-independent flag for every Fortran compiler
# (LLVM flang): a shared library built from these objects needs one.
if(NOT CMAKE_Fortran_COMPILE_OPTIONS_PIC AND NOT WIN32)
  set(CMAKE_Fortran_COMPILE_OPTIONS_PIC "-fPIC")
endif()

add_subdirectory(${CMAKE_CURRENT_SOURCE_DIR}/../../bindings/fortran/guatiao guatiao_fortran)

add_library(greeter_kind_fortran STATIC ${CMAKE_CURRENT_SOURCE_DIR}/../greeter_kind/fortran/src/greeter_kinds.F90)
target_link_libraries(greeter_kind_fortran PUBLIC guatiao::fortran)
set_target_properties(greeter_kind_fortran PROPERTIES
  Fortran_MODULE_DIRECTORY ${CMAKE_CURRENT_BINARY_DIR}/kind-modules
  POSITION_INDEPENDENT_CODE ON
  # ifx on Windows preprocesses no file by its extension alone.
  Fortran_PREPROCESS ON)
target_include_directories(greeter_kind_fortran PUBLIC ${CMAKE_CURRENT_BINARY_DIR}/kind-modules)

add_library(fortran_greeter SHARED src/fortran_greeter.f90 src/fortran_greeter_entry.f90)
target_link_libraries(fortran_greeter PRIVATE greeter_kind_fortran)
set_target_properties(fortran_greeter PROPERTIES
  Fortran_MODULE_DIRECTORY ${CMAKE_CURRENT_BINARY_DIR}/greeter-modules)
if(WIN32)
  set_target_properties(fortran_greeter PROPERTIES PREFIX "")
  # The two symbols a host looks for, and nothing else.
  target_sources(fortran_greeter PRIVATE library.def)
  if(CMAKE_Fortran_COMPILER_ID STREQUAL "GNU")
    # No libgfortran DLL for a host to find beside the library.
    target_link_options(fortran_greeter PRIVATE -static)
  elseif(CMAKE_Fortran_COMPILER_ID MATCHES "^Intel")
    # Nor Intel's runtime DLLs, which a host would look for on PATH.
    set_target_properties(fortran_greeter PROPERTIES MSVC_RUNTIME_LIBRARY MultiThreaded)
  endif()
endif()
