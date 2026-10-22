! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> guatiao_serde.h in Fortran: the parse error, the `how` flags, and an
!> abstract interface and a procedure pointer for every export this
!> binding calls. TOML and YAML are features of a build: their pointers
!> stay null when the library lacks them.
module guatiao_serde_c
  use, intrinsic :: iso_c_binding
  use guatiao_loader, only: gt_dl_sym
  use guatiao_c, only: gt_str, gt_value, gt_export
  implicit none
  public

  integer(c_int32_t), parameter :: GUATIAO_FORMAT_JSON = 0, GUATIAO_FORMAT_TOML = 1, &
                                   GUATIAO_FORMAT_YAML = 2
  integer(c_int32_t), parameter :: GUATIAO_BYTES_DATA_URI = 0, GUATIAO_BYTES_BASE64 = 1, &
                                   GUATIAO_BYTES_ARRAY = 2, GUATIAO_BYTES_REFUSE = 3
  integer(c_int32_t), parameter :: GUATIAO_READ_DATA_URIS = 256, GUATIAO_PRETTY = 512

  !> `guatiao_parse_error`: 56 bytes.
  type, bind(c) :: gt_parse_error
    integer(c_int32_t) :: format = 0
    integer(c_int32_t) :: line = 0
    integer(c_int32_t) :: column = 0
    type(gt_value) :: message
  end type

  type(gt_export), parameter :: gt_serde_exports(7) = [ &
    gt_export('guatiao_json_parse_ex   ', 5, ''), &
    gt_export('guatiao_json_emit       ', 4, ''), &
    gt_export('guatiao_json_emit_pretty', 4, ''), &
    gt_export('guatiao_toml_parse_ex   ', 5, 'toml'), &
    gt_export('guatiao_toml_emit       ', 4, 'toml'), &
    gt_export('guatiao_yaml_parse_ex   ', 5, 'yaml'), &
    gt_export('guatiao_yaml_emit       ', 4, 'yaml') ]

  abstract interface
    function guatiao_json_parse_ex_i(text, how, alloc, out, out_error) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(gt_str), value :: text
      integer(c_int32_t), value :: how
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      type(c_ptr), value :: out_error
      integer(c_int32_t) :: r
    end function
    function guatiao_json_emit_i(value, how, alloc, out) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: value
      integer(c_int32_t), value :: how
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_json_emit_pretty_i(value, how, alloc, out) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: value
      integer(c_int32_t), value :: how
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_toml_parse_ex_i(text, how, alloc, out, out_error) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(gt_str), value :: text
      integer(c_int32_t), value :: how
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      type(c_ptr), value :: out_error
      integer(c_int32_t) :: r
    end function
    function guatiao_toml_emit_i(value, how, alloc, out) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: value
      integer(c_int32_t), value :: how
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_yaml_parse_ex_i(text, how, alloc, out, out_error) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(gt_str), value :: text
      integer(c_int32_t), value :: how
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      type(c_ptr), value :: out_error
      integer(c_int32_t) :: r
    end function
    function guatiao_yaml_emit_i(value, how, alloc, out) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: value
      integer(c_int32_t), value :: how
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
  end interface

  procedure(guatiao_json_parse_ex_i), pointer :: gt_json_parse_ex => null()
  procedure(guatiao_json_emit_i), pointer :: gt_json_emit => null()
  procedure(guatiao_json_emit_pretty_i), pointer :: gt_json_emit_pretty => null()
  procedure(guatiao_toml_parse_ex_i), pointer :: gt_toml_parse_ex => null()
  procedure(guatiao_toml_emit_i), pointer :: gt_toml_emit => null()
  procedure(guatiao_yaml_parse_ex_i), pointer :: gt_yaml_parse_ex => null()
  procedure(guatiao_yaml_emit_i), pointer :: gt_yaml_emit => null()

contains

  !> Points every export of this header at library `h`.
  subroutine gt_resolve_serde(h)
    type(c_ptr), intent(in) :: h
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_json_parse_ex'), gt_json_parse_ex)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_json_emit'), gt_json_emit)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_json_emit_pretty'), gt_json_emit_pretty)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_toml_parse_ex'), gt_toml_parse_ex)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_toml_emit'), gt_toml_emit)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_yaml_parse_ex'), gt_yaml_parse_ex)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_yaml_emit'), gt_yaml_emit)
  end subroutine

end module guatiao_serde_c
