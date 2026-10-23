! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> Counted holds on what C keeps: a registry, an instance a provider
!> built, an object another side handed over. Every copy of a handle
!> shares one box; the thing is released once, when the last copy goes
!> or when it is closed and nothing else holds it.
module guatiao_holds
  use, intrinsic :: iso_c_binding
  use guatiao_c
  use guatiao_native
  implicit none
  private

  public :: gt_hold, gt_hold_box, gt_new_hold, gt_hold_parent, GT_HOLD_REGISTRY, GT_HOLD_INSTANCE, &
            GT_HOLD_OBJECT, GT_HOLD_LENT, GT_HOLD_RECEIVER

  integer, parameter :: GT_HOLD_REGISTRY = 1   ! guatiao_registry_free
  integer, parameter :: GT_HOLD_INSTANCE = 2   ! guatiao_registry_provider_destroy
  integer, parameter :: GT_HOLD_OBJECT = 3     ! the object table's destroy slot
  integer, parameter :: GT_HOLD_LENT = 4       ! nothing: another side owns it
  integer, parameter :: GT_HOLD_RECEIVER = 5   ! guatiao_wire_receiver_free (ctx)

  type :: gt_hold_box
    integer :: what = 0
    integer :: refs = 0
    logical :: closed = .false.     ! no call goes through it any more
    logical :: released = .false.   ! the C side was told
    type(c_ptr) :: reg = c_null_ptr
    type(c_ptr) :: table = c_null_ptr
    integer(c_size_t) :: size = 0
    type(c_ptr) :: ctx = c_null_ptr
    character(:), allocatable :: key
    type(gt_hold_box), pointer :: parent => null()
  end type

  type :: gt_hold
    type(gt_hold_box), pointer :: box => null()
  contains
    procedure, private :: assign
    generic :: assignment(=) => assign
    final :: finalize
    procedure :: close => hold_close
    procedure :: is_closed => hold_is_closed
    procedure :: give_up => hold_give_up
  end type

  ! The first two slots of an object kind's table: its header and destroy.
  type, bind(c) :: object_head
    type(gt_kind_header) :: header
    type(c_funptr) :: destroy = c_null_funptr
  end type

  abstract interface
    subroutine destroy_i(ctx) bind(c)
      import :: c_ptr
      type(c_ptr), value :: ctx
    end subroutine
  end interface

contains

  !> A hold of kind `what`, with the first count.
  recursive function gt_new_hold(what, reg, table, size, ctx, key) result(h)
    integer, intent(in) :: what
    type(c_ptr), intent(in), optional :: reg, table, ctx
    integer(c_size_t), intent(in), optional :: size
    character(*), intent(in), optional :: key
    type(gt_hold) :: h
    allocate(h%box)
    h%box%what = what
    h%box%refs = 1
    if (present(reg)) h%box%reg = reg
    if (present(table)) h%box%table = table
    if (present(size)) h%box%size = size
    if (present(ctx)) h%box%ctx = ctx
    if (present(key)) h%box%key = key
  end function

  !> Makes `h` keep `parent` alive (an instance its registry) until `h`
  !> is released.
  recursive subroutine gt_hold_parent(h, parent)
    type(gt_hold), intent(inout) :: h
    type(gt_hold), intent(in) :: parent
    if (.not. associated(h%box) .or. .not. associated(parent%box)) return
    h%box%parent => parent%box
    parent%box%refs = parent%box%refs + 1
  end subroutine

  recursive subroutine assign(lhs, rhs)
    class(gt_hold), intent(inout) :: lhs
    type(gt_hold), intent(in) :: rhs
    type(gt_hold_box), pointer :: keep
    keep => rhs%box
    if (associated(keep)) keep%refs = keep%refs + 1
    call drop(lhs%box)
    lhs%box => keep
  end subroutine

  impure elemental subroutine finalize(h)
    type(gt_hold), intent(inout) :: h
    call drop(h%box)
  end subroutine

  recursive subroutine drop(b)
    type(gt_hold_box), pointer, intent(inout) :: b
    type(gt_hold_box), pointer :: parent
    if (.not. associated(b)) return
    b%refs = b%refs - 1
    if (b%refs > 0) then
      nullify(b)
      return
    end if
    call release(b)
    parent => b%parent
    deallocate(b)
    call drop(parent)
  end subroutine

  ! Tells the C side, once.
  recursive subroutine release(b)
    type(gt_hold_box), pointer, intent(in) :: b
    type(object_head), pointer :: head
    procedure(destroy_i), pointer :: destroy
    character(:), allocatable, target :: key
    if (b%released) return
    b%released = .true.
    b%closed = .true.
    select case (b%what)
    case (GT_HOLD_REGISTRY)
      if (c_associated(b%reg)) call gt_registry_free(b%reg)
    case (GT_HOLD_INSTANCE)
      if (c_associated(b%reg) .and. c_associated(b%ctx)) then
        key = b%key
        call gt_registry_provider_destroy(b%reg, gt_view(key), b%ctx)
      end if
    case (GT_HOLD_RECEIVER)
      if (c_associated(b%ctx)) call gt_wire_receiver_free(b%ctx)
    case (GT_HOLD_OBJECT)
      if (c_associated(b%table) .and. b%size >= storage_size(head) / 8) then
        call c_f_pointer(b%table, head)
        if (c_associated(head%destroy)) then
          call c_f_procpointer(head%destroy, destroy)
          call destroy(b%ctx)
        end if
      end if
    end select
  end subroutine

  !> Releases now for every copy (a registry once nothing else holds it):
  !> every call through it then answers GUATIAO_STAT_CLOSED. Safe twice.
  recursive subroutine hold_close(self)
    class(gt_hold), intent(inout) :: self
    if (.not. associated(self%box)) return
    self%box%closed = .true.
    if (self%box%what == GT_HOLD_REGISTRY .and. self%box%refs > 1) return
    call release(self%box)
  end subroutine

  recursive logical function hold_is_closed(self)
    class(gt_hold), intent(in) :: self
    hold_is_closed = .true.
    if (associated(self%box)) hold_is_closed = self%box%closed
  end function

  !> Hands the held object to another side, which destroys it from now:
  !> nothing is released here, and every copy reads closed.
  recursive subroutine hold_give_up(self)
    class(gt_hold), intent(inout) :: self
    if (.not. associated(self%box)) return
    self%box%released = .true.
    self%box%closed = .true.
  end subroutine

end module guatiao_holds
