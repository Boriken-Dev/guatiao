# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# The guatiao Fortran bindings for projects that build with CMake:
#
#   add_subdirectory(path/to/bindings/fortran)       # or FetchContent with
#   target_link_libraries(app PRIVATE guatiao::fortran)  # SOURCE_SUBDIR bindings/fortran
#
# Nothing of guatiao itself is linked: the library is opened at run time.
cmake_minimum_required(VERSION 3.20)
project(guatiao_fortran VERSION 0.0.0 LANGUAGES Fortran)

# CMake knows no position-independent flag for every Fortran compiler
# (LLVM flang): a shared library built from these objects needs one.
if(NOT CMAKE_Fortran_COMPILE_OPTIONS_PIC AND NOT WIN32)
  set(CMAKE_Fortran_COMPILE_OPTIONS_PIC "-fPIC")
endif()

file(GLOB GUATIAO_FORTRAN_SOURCES CONFIGURE_DEPENDS
  ${CMAKE_CURRENT_SOURCE_DIR}/src/*.f90 ${CMAKE_CURRENT_SOURCE_DIR}/src/*.F90)

add_library(guatiao_fortran ${GUATIAO_FORTRAN_SOURCES})
add_library(guatiao::fortran ALIAS guatiao_fortran)
set_target_properties(guatiao_fortran PROPERTIES
  Fortran_MODULE_DIRECTORY ${CMAKE_CURRENT_BINARY_DIR}/modules
  POSITION_INDEPENDENT_CODE ON)
target_include_directories(guatiao_fortran PUBLIC
  $<BUILD_INTERFACE:${CMAKE_CURRENT_BINARY_DIR}/modules>
  $<INSTALL_INTERFACE:include/guatiao_fortran>)
# glibc before 2.34 keeps dlopen in libdl.
if(UNIX AND NOT APPLE)
  target_link_libraries(guatiao_fortran PUBLIC ${CMAKE_DL_LIBS})
endif()

option(GUATIAO_FORTRAN_TESTS "Build the binding's tests" ${PROJECT_IS_TOP_LEVEL})
if(GUATIAO_FORTRAN_TESTS)
  enable_testing()
  add_library(guatiao_fortran_harness OBJECT test/harness.f90)
  target_link_libraries(guatiao_fortran_harness PUBLIC guatiao_fortran)
  set_target_properties(guatiao_fortran_harness PROPERTIES
    Fortran_MODULE_DIRECTORY ${CMAKE_CURRENT_BINARY_DIR}/test-modules)
  target_include_directories(guatiao_fortran_harness PUBLIC ${CMAKE_CURRENT_BINARY_DIR}/test-modules)
  file(GLOB GUATIAO_FORTRAN_TESTS_SOURCES CONFIGURE_DEPENDS ${CMAKE_CURRENT_SOURCE_DIR}/test/test_*.f90)
  foreach(source ${GUATIAO_FORTRAN_TESTS_SOURCES})
    get_filename_component(name ${source} NAME_WE)
    add_executable(${name} ${source})
    target_link_libraries(${name} PRIVATE guatiao_fortran_harness guatiao_fortran)
    set_target_properties(${name} PROPERTIES Fortran_MODULE_DIRECTORY ${CMAKE_CURRENT_BINARY_DIR}/test-modules/${name})
    # The tests read the headers and vectors relative to this directory.
    add_test(NAME ${name} COMMAND ${name} WORKING_DIRECTORY ${CMAKE_CURRENT_SOURCE_DIR})
  endforeach()
endif()

include(GNUInstallDirs)
install(TARGETS guatiao_fortran EXPORT guatiao_fortran
  ARCHIVE DESTINATION ${CMAKE_INSTALL_LIBDIR}
  LIBRARY DESTINATION ${CMAKE_INSTALL_LIBDIR})
install(DIRECTORY ${CMAKE_CURRENT_BINARY_DIR}/modules/ DESTINATION include/guatiao_fortran)
install(EXPORT guatiao_fortran NAMESPACE guatiao:: DESTINATION ${CMAKE_INSTALL_LIBDIR}/cmake/guatiao_fortran
  FILE guatiao_fortranConfig.cmake)
