# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# The native library a C++ package links: found where the user says, or
# built from this checkout's Rust sources with cargo. No library is
# downloaded: building from source is the only distribution.
#
#   guatiao_native(<target>
#     NAME <file stem>          # guatiao, guatiao_serde, guatiao_intake
#     VARIABLE <variable>       # its own variable, GUATIAO_SERDE_LIBRARY ...
#     CRATE <cargo package>     # guatiao, guatiao-serde, guatiao-intake
#     [FEATURES <feature>...])  # what the cargo build turns on
#
# The library is, in order:
#   1. <VARIABLE> (a CMake variable, else the environment): a file, taken as
#      it is, or a directory holding the platform's file for NAME;
#   2. GUATIAO_LIBRARY: a directory holding it; or a file, taken as it is
#      when it is not the core's own file (an application's library
#      carrying every surface), else the file for NAME beside it;
#   3. built by cargo from the workspace this package sits in, into
#      <build>/guatiao-cargo, once per configure (GUATIAO_CARGO_PROFILE,
#      default release; GUATIAO_CARGO_TARGET for another target triple).
#
# Defines <target>, an IMPORTED SHARED library, and sets
# <target>_DIRECTORY in the caller's scope to the directory holding the
# runtime file, for a program's PATH or rpath.

include_guard(GLOBAL)

set(_GUATIAO_WORKSPACE "${CMAKE_CURRENT_LIST_DIR}/../../../.." CACHE INTERNAL "")

function(_guatiao_setting out name)
  if(DEFINED ${name} AND NOT "${${name}}" STREQUAL "")
    set(${out} "${${name}}" PARENT_SCOPE)
  elseif(DEFINED ENV{${name}} AND NOT "$ENV{${name}}" STREQUAL "")
    set(${out} "$ENV{${name}}" PARENT_SCOPE)
  else()
    set(${out} "" PARENT_SCOPE)
  endif()
endfunction()

function(_guatiao_file_names stem out_file out_implibs)
  if(WIN32)
    set(${out_file} "${stem}.dll" PARENT_SCOPE)
    set(${out_implibs} "${stem}.dll.lib;${stem}.lib;lib${stem}.dll.a" PARENT_SCOPE)
  elseif(APPLE)
    set(${out_file} "lib${stem}.dylib" PARENT_SCOPE)
    set(${out_implibs} "" PARENT_SCOPE)
  else()
    set(${out_file} "lib${stem}.so" PARENT_SCOPE)
    set(${out_implibs} "" PARENT_SCOPE)
  endif()
endfunction()

function(_guatiao_cargo_build crate features out_dir)
  find_program(GUATIAO_CARGO cargo
    HINTS "$ENV{CARGO_HOME}/bin" "$ENV{HOME}/.cargo/bin" "$ENV{USERPROFILE}/.cargo/bin")
  if(NOT GUATIAO_CARGO)
    message(FATAL_ERROR
      "guatiao: no ${crate} library found and no cargo to build one. Set GUATIAO_LIBRARY to the "
      "directory holding a build of it (cargo build -p ${crate}), or install Rust from https://rustup.rs "
      "so this package builds it.")
  endif()
  get_filename_component(workspace "${_GUATIAO_WORKSPACE}" ABSOLUTE)
  if(NOT EXISTS "${workspace}/Cargo.toml")
    message(FATAL_ERROR
      "guatiao: no ${crate} library found, and this package is not inside the guatiao repository to "
      "build it from (${workspace}). Set GUATIAO_LIBRARY.")
  endif()
  set(profile "${GUATIAO_CARGO_PROFILE}")
  if(profile STREQUAL "")
    set(profile release)
  endif()
  set(target_dir "${CMAKE_BINARY_DIR}/guatiao-cargo")
  set(args build --manifest-path "${workspace}/Cargo.toml" -p "${crate}" --profile "${profile}"
           --target-dir "${target_dir}")
  if(features)
    list(TRANSFORM features PREPEND "${crate}/")
    list(JOIN features "," joined)
    list(APPEND args --features "${joined}")
  endif()
  set(dir "${target_dir}")
  if(NOT "${GUATIAO_CARGO_TARGET}" STREQUAL "")
    list(APPEND args --target "${GUATIAO_CARGO_TARGET}")
    set(dir "${dir}/${GUATIAO_CARGO_TARGET}")
  endif()
  if(profile STREQUAL "dev" OR profile STREQUAL "test")
    set(dir "${dir}/debug")
  else()
    set(dir "${dir}/${profile}")
  endif()
  message(STATUS "guatiao: building ${crate} with cargo (${profile})")
  execute_process(COMMAND "${GUATIAO_CARGO}" ${args} RESULT_VARIABLE rc)
  if(NOT rc EQUAL 0)
    message(FATAL_ERROR "guatiao: cargo could not build ${crate} (exit ${rc}); its output is above.")
  endif()
  set(${out_dir} "${dir}" PARENT_SCOPE)
endfunction()

function(guatiao_native target)
  cmake_parse_arguments(arg "" "NAME;VARIABLE;CRATE" "FEATURES" ${ARGN})
  if(TARGET ${target})
    get_target_property(location ${target} IMPORTED_LOCATION)
    get_filename_component(dir "${location}" DIRECTORY)
    set(${target}_DIRECTORY "${dir}" PARENT_SCOPE)
    return()
  endif()
  _guatiao_file_names("${arg_NAME}" file implibs)
  _guatiao_file_names(guatiao core_file core_implibs)

  set(found "")
  _guatiao_setting(own "${arg_VARIABLE}")
  _guatiao_setting(shared GUATIAO_LIBRARY)
  if(NOT own STREQUAL "")
    if(IS_DIRECTORY "${own}")
      set(found "${own}/${file}")
    else()
      set(found "${own}")
    endif()
  elseif(NOT shared STREQUAL "")
    if(IS_DIRECTORY "${shared}")
      set(found "${shared}/${file}")
    else()
      get_filename_component(name "${shared}" NAME)
      get_filename_component(dir "${shared}" DIRECTORY)
      if(name STREQUAL core_file)
        set(found "${dir}/${file}")
      else()
        set(found "${shared}")
      endif()
    endif()
  else()
    # One cargo build per surface into a shared target directory, each
    # file copied out at once: building another surface rebuilds the
    # core's file there with that surface's features.
    _guatiao_cargo_build("${arg_CRATE}" "${arg_FEATURES}" built)
    set(copy_dir "${CMAKE_BINARY_DIR}/guatiao-native/${arg_NAME}")
    file(MAKE_DIRECTORY "${copy_dir}")
    file(COPY "${built}/${file}" DESTINATION "${copy_dir}")
    foreach(implib IN LISTS implibs)
      if(EXISTS "${built}/${implib}")
        file(COPY "${built}/${implib}" DESTINATION "${copy_dir}")
      endif()
    endforeach()
    set(found "${copy_dir}/${file}")
  endif()

  get_filename_component(found "${found}" ABSOLUTE)
  if(NOT EXISTS "${found}")
    message(FATAL_ERROR "guatiao: ${found} does not exist (looking for the ${arg_NAME} library; "
                        "${arg_VARIABLE}, then GUATIAO_LIBRARY, then a cargo build)")
  endif()
  get_filename_component(dir "${found}" DIRECTORY)
  get_filename_component(found_name "${found}" NAME)

  add_library(${target} SHARED IMPORTED GLOBAL)
  set_target_properties(${target} PROPERTIES IMPORTED_LOCATION "${found}")
  if(WIN32)
    # The import library of the file found, by that file's own name: one
    # beside it for another library must not be taken.
    set(implib "")
    string(REGEX REPLACE "\\.dll$" "" stem "${found_name}")
    foreach(candidate "${found_name}.lib" "${stem}.lib" "lib${stem}.dll.a")
      if(implib STREQUAL "" AND EXISTS "${dir}/${candidate}")
        set(implib "${dir}/${candidate}")
      endif()
    endforeach()
    if(implib STREQUAL "")
      message(FATAL_ERROR "guatiao: no import library beside ${found} (cargo writes ${found_name}.lib)")
    endif()
    set_target_properties(${target} PROPERTIES IMPORTED_IMPLIB "${implib}")
  else()
    # A cargo cdylib names no SONAME: link it by name, so a program records
    # the file name and finds it through its rpath, not this build's path.
    set_target_properties(${target} PROPERTIES IMPORTED_NO_SONAME TRUE)
  endif()
  message(STATUS "guatiao: ${arg_NAME} is ${found}")
  set(${target}_DIRECTORY "${dir}" PARENT_SCOPE)
endfunction()
