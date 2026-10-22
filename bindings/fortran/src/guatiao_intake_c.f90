! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> guatiao_intake.h in Fortran: an abstract interface and a procedure
!> pointer for every export this binding calls.
module guatiao_intake_c
  use, intrinsic :: iso_c_binding
  use guatiao_loader, only: gt_dl_sym
  use guatiao_c, only: gt_str, gt_export
  implicit none
  public

  type(gt_export), parameter :: gt_intake_exports(10) = [ &
    gt_export('guatiao_intake_check     ', 4, ''), &
    gt_export('guatiao_intake_layout    ', 4, ''), &
    gt_export('guatiao_intake_for_schema', 3, ''), &
    gt_export('guatiao_intake_form_for  ', 5, ''), &
    gt_export('guatiao_intake_is_visible', 5, ''), &
    gt_export('guatiao_intake_path_get  ', 2, ''), &
    gt_export('guatiao_intake_resolve   ', 2, ''), &
    gt_export('guatiao_intake_flat_keys ', 4, ''), &
    gt_export('guatiao_intake_flatten   ', 5, ''), &
    gt_export('guatiao_intake_unflatten ', 5, '') ]

  abstract interface
    function guatiao_intake_check_i(schema, form, alloc, out_error) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: schema
      type(c_ptr), value :: form
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out_error
      integer(c_int32_t) :: r
    end function
    function guatiao_intake_layout_i(schema, form, alloc, out) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: schema
      type(c_ptr), value :: form
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_intake_for_schema_i(schema, alloc, out) bind(c) result(r)
      import :: c_int32_t, c_ptr
      type(c_ptr), value :: schema
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_intake_form_for_i(form, schema, path, alloc, out) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: form
      type(c_ptr), value :: schema
      type(gt_str), value :: path
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_intake_is_visible_i(schema, form, key, values, out) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: schema
      type(c_ptr), value :: form
      type(gt_str), value :: key
      type(c_ptr), value :: values
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_intake_path_get_i(value, path) bind(c) result(r)
      import :: c_ptr, gt_str
      type(c_ptr), value :: value
      type(gt_str), value :: path
      type(c_ptr) :: r
    end function
    function guatiao_intake_resolve_i(schema, key) bind(c) result(r)
      import :: c_ptr, gt_str
      type(c_ptr), value :: schema
      type(gt_str), value :: key
      type(c_ptr) :: r
    end function
    function guatiao_intake_flat_keys_i(schema, key, alloc, out) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: schema
      type(gt_str), value :: key
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_intake_flatten_i(schema, key, value, alloc, out) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: schema
      type(gt_str), value :: key
      type(c_ptr), value :: value
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
    function guatiao_intake_unflatten_i(schema, key, flat, alloc, out) bind(c) result(r)
      import :: c_int32_t, c_ptr, gt_str
      type(c_ptr), value :: schema
      type(gt_str), value :: key
      type(c_ptr), value :: flat
      type(c_ptr), value :: alloc
      type(c_ptr), value :: out
      integer(c_int32_t) :: r
    end function
  end interface

  procedure(guatiao_intake_check_i), pointer :: gt_intake_check => null()
  procedure(guatiao_intake_layout_i), pointer :: gt_intake_layout => null()
  procedure(guatiao_intake_for_schema_i), pointer :: gt_intake_for_schema => null()
  procedure(guatiao_intake_form_for_i), pointer :: gt_intake_form_for => null()
  procedure(guatiao_intake_is_visible_i), pointer :: gt_intake_is_visible => null()
  procedure(guatiao_intake_path_get_i), pointer :: gt_intake_path_get => null()
  procedure(guatiao_intake_resolve_i), pointer :: gt_intake_resolve => null()
  procedure(guatiao_intake_flat_keys_i), pointer :: gt_intake_flat_keys => null()
  procedure(guatiao_intake_flatten_i), pointer :: gt_intake_flatten => null()
  procedure(guatiao_intake_unflatten_i), pointer :: gt_intake_unflatten => null()

contains

  !> Points every export of this header at library `h`.
  subroutine gt_resolve_intake(h)
    type(c_ptr), intent(in) :: h
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_intake_check'), gt_intake_check)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_intake_layout'), gt_intake_layout)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_intake_for_schema'), gt_intake_for_schema)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_intake_form_for'), gt_intake_form_for)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_intake_is_visible'), gt_intake_is_visible)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_intake_path_get'), gt_intake_path_get)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_intake_resolve'), gt_intake_resolve)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_intake_flat_keys'), gt_intake_flat_keys)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_intake_flatten'), gt_intake_flatten)
    call c_f_procpointer(gt_dl_sym(h, 'guatiao_intake_unflatten'), gt_intake_unflatten)
  end subroutine

end module guatiao_intake_c
