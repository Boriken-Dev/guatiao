! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> An allocator counting the blocks it handed out and has not had back:
!> a `guatiao_alloc` first, then the counts.
module counting_alloc
  use, intrinsic :: iso_c_binding
  implicit none
  private

  public :: counting, counting_new

  type, bind(c) :: counting
    integer(c_int32_t) :: struct_size = 0
    type(c_ptr) :: ctx = c_null_ptr
    type(c_funptr) :: alloc = c_null_funptr
    type(c_funptr) :: free = c_null_funptr
    type(c_funptr) :: release = c_null_funptr
    integer(c_int64_t) :: live = 0
  end type

  interface
    function c_malloc(size) bind(c, name="malloc") result(p)
      import :: c_size_t, c_ptr
      integer(c_size_t), value :: size
      type(c_ptr) :: p
    end function
    subroutine c_free(p) bind(c, name="free")
      import :: c_ptr
      type(c_ptr), value :: p
    end subroutine
  end interface

contains

  function counting_new() result(c)
    type(counting), pointer :: c
    allocate(c)
    c%struct_size = 40
    c%ctx = c_loc(c)
    c%alloc = c_funloc(counted_alloc)
    c%free = c_funloc(counted_free)
  end function

  function counted_alloc(ctx, size, align) bind(c) result(p)
    type(c_ptr), value :: ctx
    integer(c_size_t), value :: size, align
    type(c_ptr) :: p
    type(counting), pointer :: c
    p = c_null_ptr
    if (size == 0 .or. align > 16) return
    p = c_malloc(size)
    if (.not. c_associated(p)) return
    call c_f_pointer(ctx, c)
    c%live = c%live + 1
  end function

  subroutine counted_free(ctx, p, size, align) bind(c)
    type(c_ptr), value :: ctx, p
    integer(c_size_t), value :: size, align
    type(counting), pointer :: c
    if (.not. c_associated(p)) return
    call c_free(p)
    call c_f_pointer(ctx, c)
    c%live = c%live - 1
  end subroutine

end module counting_alloc
