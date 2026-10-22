! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> Statuses and how they reach a caller: the optional `stat`/`errmsg`
!> pair of the binding's own calls, and the allocatable `guatiao_error`
!> a kind's slot methods report through.
module guatiao_errors
  use, intrinsic :: iso_c_binding, only: c_int32_t
  use guatiao_c, only: GUATIAO_OK, GUATIAO_ERR_BAD_VALUE, GUATIAO_ERR_ALLOC, GUATIAO_ERR_WRONG_KIND, &
                       GUATIAO_ERR_NOT_FOUND, GUATIAO_ERR_NULL, GUATIAO_ERR_GONE, GUATIAO_ERR_UNSUPPORTED, &
                       GUATIAO_ERR_BUSY, GUATIAO_ERR_INTERNAL
  implicit none
  private

  public :: GUATIAO_OK, GUATIAO_ERR_BAD_VALUE, GUATIAO_ERR_ALLOC, GUATIAO_ERR_WRONG_KIND, &
            GUATIAO_ERR_NOT_FOUND, GUATIAO_ERR_NULL, GUATIAO_ERR_GONE, GUATIAO_ERR_UNSUPPORTED, &
            GUATIAO_ERR_BUSY, GUATIAO_ERR_INTERNAL
  public :: GUATIAO_STAT_CLOSED, GUATIAO_STAT_READ_ONLY, GUATIAO_STAT_LIBRARY_NOT_FOUND, &
            GUATIAO_STAT_MISSING_SYMBOL, GUATIAO_STAT_FLOOR_MISMATCH
  public :: guatiao_error, guatiao_fail, guatiao_status_name, guatiao_raise, gt_report, gt_report_error

  !> This binding's own conditions, outside the range of a guatiao_status.
  integer, parameter :: GUATIAO_STAT_CLOSED = 1001          ! the tree, registry or object was closed
  integer, parameter :: GUATIAO_STAT_READ_ONLY = 1002       ! a borrowed view refuses writes
  integer, parameter :: GUATIAO_STAT_LIBRARY_NOT_FOUND = 1003
  integer, parameter :: GUATIAO_STAT_MISSING_SYMBOL = 1004  ! the loaded build lacks an export
  integer, parameter :: GUATIAO_STAT_FLOOR_MISMATCH = 1005  ! a table too small, or another floor hash

  !> Why a slot method failed: a status and the provider's own words.
  !> Unallocated is success.
  type :: guatiao_error
    integer :: status = GUATIAO_ERR_INTERNAL
    character(:), allocatable :: message
  end type

contains

  !> Refuses: allocates `error` with `status` and `message`.
  subroutine guatiao_fail(error, status, message)
    type(guatiao_error), allocatable, intent(inout) :: error
    integer, intent(in) :: status
    character(*), intent(in), optional :: message
    if (allocated(error)) deallocate(error)
    allocate(error)
    error%status = status
    if (present(message)) then
      error%message = message
    else
      error%message = ''
    end if
  end subroutine

  !> The constant's name of a status (`GUATIAO_ERR_BAD_VALUE`), or
  !> `status <n>` for one nobody here defines.
  function guatiao_status_name(status) result(name)
    integer, intent(in) :: status
    character(:), allocatable :: name
    character(24) :: buf
    select case (status)
    case (GUATIAO_OK); name = 'GUATIAO_OK'
    case (GUATIAO_ERR_BAD_VALUE); name = 'GUATIAO_ERR_BAD_VALUE'
    case (GUATIAO_ERR_ALLOC); name = 'GUATIAO_ERR_ALLOC'
    case (GUATIAO_ERR_WRONG_KIND); name = 'GUATIAO_ERR_WRONG_KIND'
    case (GUATIAO_ERR_NOT_FOUND); name = 'GUATIAO_ERR_NOT_FOUND'
    case (GUATIAO_ERR_NULL); name = 'GUATIAO_ERR_NULL'
    case (GUATIAO_ERR_GONE); name = 'GUATIAO_ERR_GONE'
    case (GUATIAO_ERR_UNSUPPORTED); name = 'GUATIAO_ERR_UNSUPPORTED'
    case (GUATIAO_ERR_BUSY); name = 'GUATIAO_ERR_BUSY'
    case (GUATIAO_ERR_INTERNAL); name = 'GUATIAO_ERR_INTERNAL'
    case (GUATIAO_STAT_CLOSED); name = 'GUATIAO_STAT_CLOSED'
    case (GUATIAO_STAT_READ_ONLY); name = 'GUATIAO_STAT_READ_ONLY'
    case (GUATIAO_STAT_LIBRARY_NOT_FOUND); name = 'GUATIAO_STAT_LIBRARY_NOT_FOUND'
    case (GUATIAO_STAT_MISSING_SYMBOL); name = 'GUATIAO_STAT_MISSING_SYMBOL'
    case (GUATIAO_STAT_FLOOR_MISMATCH); name = 'GUATIAO_STAT_FLOOR_MISMATCH'
    case default
      write (buf, '(a,i0)') 'status ', status
      name = trim(buf)
    end select
  end function

  !> Hands a status to an optional `stat`/`errmsg` pair: `stat` receives
  !> it, `errmsg` the message when it failed; with no `stat` a failure
  !> stops the program with the message.
  subroutine gt_report(status, message, stat, errmsg)
    integer, intent(in) :: status
    character(*), intent(in) :: message
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    if (present(stat)) stat = status
    if (status == GUATIAO_OK) return
    if (present(errmsg)) errmsg = message
    if (.not. present(stat)) call guatiao_raise(status, message)
  end subroutine

  !> As gt_report, for a slot method's error (unallocated is success).
  subroutine gt_report_error(error, stat, errmsg)
    type(guatiao_error), allocatable, intent(in) :: error
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    if (.not. allocated(error)) then
      if (present(stat)) stat = GUATIAO_OK
      return
    end if
    if (allocated(error%message)) then
      call gt_report(error%status, error%message, stat, errmsg)
    else
      call gt_report(error%status, '', stat, errmsg)
    end if
  end subroutine

  !> Stops the program: what an absent `stat` asks for on a failure.
  subroutine guatiao_raise(status, message)
    integer, intent(in) :: status
    character(*), intent(in) :: message
    character(:), allocatable :: name
    name = guatiao_status_name(status)
    if (len(message) > 0) then
      error stop message // ' (' // name // ')'
    else
      error stop 'guatiao: ' // name
    end if
  end subroutine

end module guatiao_errors
