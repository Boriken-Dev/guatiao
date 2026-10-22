! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> Finding and opening guatiao, guatiao_serde and guatiao_intake, the
!> allocators trees are built in, and the text helpers every call uses.
!>
!> Nothing native is touched until a call needs it. The library is then
!> found as guatiao_use_library named it, else through GUATIAO_LIBRARY (a
!> directory, or a file taken as it is for guatiao), else by the
!> platform's own search; serde and intake beside it under their own
!> names. A library stays open for the process.
module guatiao_native
  use, intrinsic :: iso_c_binding
  use guatiao_loader
  use guatiao_c
  use guatiao_serde_c, only: gt_serde_exports, gt_resolve_serde
  use guatiao_intake_c, only: gt_intake_exports, gt_resolve_intake
  use guatiao_errors
  implicit none
  private

  public :: guatiao_use_library, guatiao_library_path, guatiao_allocator, guatiao_default_allocator
  public :: GT_CORE, GT_SERDE, GT_INTAKE, gt_need, gt_need_symbol, gt_alloc_of, gt_default_alloc
  public :: gt_view, gt_copy_text, gt_text_equal, gt_bytes_view, gt_text_of_owned

  integer, parameter :: GT_CORE = 1, GT_SERDE = 2, GT_INTAKE = 3
  character(*), parameter :: STEMS(3) = [character(14) :: 'guatiao', 'guatiao_serde', 'guatiao_intake']

  !> An allocator a tree may be built in: the address of a
  !> `guatiao_alloc` that outlives every tree built through it.
  type :: guatiao_allocator
    type(c_ptr) :: ptr = c_null_ptr
  end type

  type(c_ptr), save :: handles(3) = c_null_ptr
  character(len=4096), save :: paths(3) = ''
  character(len=4096), save :: named = ''
  type(gt_alloc), target, save :: default_alloc

contains

  !> Names the guatiao library to use: a file exporting the ABI, whatever
  !> it is called, or a directory holding it. Before the first call that
  !> needs it; naming another file once one is open is GUATIAO_ERR_BUSY.
  subroutine guatiao_use_library(path, stat, errmsg)
    character(*), intent(in) :: path
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    if (c_associated(handles(GT_CORE))) then
      if (trim(named) == path) then
        call gt_report(GUATIAO_OK, '', stat, errmsg)
      else
        call gt_report(GUATIAO_ERR_BUSY, 'guatiao is already open from ' // trim(paths(GT_CORE)), &
                       stat, errmsg)
      end if
      return
    end if
    named = path
    call gt_report(GUATIAO_OK, '', stat, errmsg)
  end subroutine

  !> The file guatiao, guatiao_serde or guatiao_intake (GT_CORE, GT_SERDE,
  !> GT_INTAKE) was opened from, or '' before it was.
  function guatiao_library_path(which) result(path)
    integer, intent(in) :: which
    character(:), allocatable :: path
    path = trim(paths(which))
  end function

  !> The allocator over guatiao's own heap, which every tree this binding
  !> builds uses unless told otherwise. Opens guatiao.
  function guatiao_default_allocator(stat, errmsg) result(alloc)
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_allocator) :: alloc
    integer :: st
    character(:), allocatable :: msg
    call gt_need(GT_CORE, st, msg)
    if (st == GUATIAO_OK) alloc%ptr = c_loc(default_alloc)
    call gt_report(st, msg, stat, errmsg)
  end function

  !> The default allocator's address; guatiao must be open.
  function gt_default_alloc() result(p)
    type(c_ptr) :: p
    p = c_loc(default_alloc)
  end function

  !> The allocator a call builds in: `alloc` when given and set, else the
  !> default.
  function gt_alloc_of(alloc) result(p)
    type(guatiao_allocator), intent(in), optional :: alloc
    type(c_ptr) :: p
    p = c_loc(default_alloc)
    if (present(alloc)) then
      if (c_associated(alloc%ptr)) p = alloc%ptr
    end if
  end function

  !> Opens library `which` if it is not open yet: GUATIAO_OK, or
  !> GUATIAO_STAT_LIBRARY_NOT_FOUND / GUATIAO_STAT_MISSING_SYMBOL with why.
  recursive subroutine gt_need(which, stat, msg)
    integer, intent(in) :: which
    integer, intent(out) :: stat
    character(:), allocatable, intent(out) :: msg
    character(:), allocatable :: path, looked
    type(c_ptr) :: h
    stat = GUATIAO_OK
    msg = ''
    if (c_associated(handles(which))) return
    if (which /= GT_CORE) then
      call gt_need(GT_CORE, stat, msg)
      if (stat /= GUATIAO_OK) return
    end if
    call locate(which, path, looked)
    if (len(path) == 0) then
      call open_by_name(which, h, path, looked)
    else
      h = gt_dl_open(path)
      if (.not. c_associated(h)) then
        call gt_dl_error(msg)
        msg = 'guatiao: cannot open ' // path // ': ' // msg
        stat = GUATIAO_STAT_LIBRARY_NOT_FOUND
        return
      end if
    end if
    if (.not. c_associated(h)) then
      stat = GUATIAO_STAT_LIBRARY_NOT_FOUND
      msg = 'guatiao: ' // trim(STEMS(which)) // ' not found (looked: ' // looked // &
            '); set GUATIAO_LIBRARY to the directory holding it'
      return
    end if
    call check_required(which, h, path, stat, msg)
    if (stat /= GUATIAO_OK) return
    select case (which)
    case (GT_CORE)
      call gt_resolve_core(h)
      stat = gt_alloc_default(c_loc(default_alloc))
      if (stat /= GUATIAO_OK) then
        msg = 'guatiao: no default allocator from ' // path
        return
      end if
    case (GT_SERDE)
      call gt_resolve_serde(h)
    case (GT_INTAKE)
      call gt_resolve_intake(h)
    end select
    handles(which) = h
    paths(which) = path
  end subroutine

  !> GUATIAO_OK when an optional export resolved, else
  !> GUATIAO_STAT_MISSING_SYMBOL naming it and the feature that has it.
  subroutine gt_need_symbol(resolved, name, feature, stat, msg)
    logical, intent(in) :: resolved
    character(*), intent(in) :: name, feature
    integer, intent(out) :: stat
    character(:), allocatable, intent(out) :: msg
    if (resolved) then
      stat = GUATIAO_OK
      msg = ''
    else
      stat = GUATIAO_STAT_MISSING_SYMBOL
      msg = 'guatiao: the loaded library has no ' // name // ' (built without the "' // feature // &
            '" feature)'
    end if
  end subroutine

  ! The file to open for `which`, or '' to search by name; `looked` says
  ! where it was looked for.
  subroutine locate(which, path, looked)
    integer, intent(in) :: which
    character(:), allocatable, intent(out) :: path, looked
    character(4096) :: env
    character(:), allocatable :: base, first, second
    integer :: n, st
    logical :: exists
    path = ''
    looked = ''
    if (which /= GT_CORE) then
      base = trim(paths(GT_CORE))
      call beside(base, which, path)
      looked = path
      return
    end if
    if (len_trim(named) > 0) then
      base = trim(named)
    else
      call get_environment_variable('GUATIAO_LIBRARY', env, n, st)
      if (st /= 0 .or. n == 0) then
        looked = 'GUATIAO_LIBRARY unset, the platform search'
        return
      end if
      base = env(1:n)
    end if
    call gt_library_file_names(trim(STEMS(GT_CORE)), first, second)
    inquire (file=base // '/' // first, exist=exists)
    if (exists) then
      path = base // '/' // first
    else if (len(second) > 0) then
      inquire (file=base // '/' // second, exist=exists)
      if (exists) path = base // '/' // second
    end if
    if (len(path) == 0) then
      inquire (file=base, exist=exists)
      if (exists) path = base
    end if
    looked = base
  end subroutine

  ! The file `which` has beside the core library at `core`.
  subroutine beside(core, which, path)
    character(*), intent(in) :: core
    integer, intent(in) :: which
    character(:), allocatable, intent(out) :: path
    character(:), allocatable :: dir, first, second
    integer :: cut
    logical :: exists
    cut = max(index(core, '/', back=.true.), index(core, '\', back=.true.))
    if (cut > 0) then
      dir = core(1:cut)
    else
      dir = ''
    end if
    call gt_library_file_names(trim(STEMS(which)), first, second)
    path = dir // first
    if (len(second) > 0) then
      inquire (file=path, exist=exists)
      if (.not. exists) path = dir // second
    end if
  end subroutine

  ! The platform's own search, by file name.
  subroutine open_by_name(which, h, path, looked)
    integer, intent(in) :: which
    type(c_ptr), intent(out) :: h
    character(:), allocatable, intent(inout) :: path, looked
    character(:), allocatable :: first, second
    call gt_library_file_names(trim(STEMS(which)), first, second)
    path = first
    h = gt_dl_open(first)
    if (.not. c_associated(h) .and. len(second) > 0) then
      path = second
      h = gt_dl_open(second)
    end if
    looked = looked // ': ' // first
    if (len(second) > 0) looked = looked // ', ' // second
  end subroutine

  ! Every export a build always has must be there.
  subroutine check_required(which, h, path, stat, msg)
    integer, intent(in) :: which
    type(c_ptr), intent(in) :: h
    character(*), intent(in) :: path
    integer, intent(out) :: stat
    character(:), allocatable, intent(out) :: msg
    integer :: i
    stat = GUATIAO_OK
    msg = ''
    select case (which)
    case (GT_CORE)
      do i = 1, size(gt_core_exports)
        if (len_trim(gt_core_exports(i)%feature) > 0) cycle
        if (.not. c_associated(gt_dl_sym(h, trim(gt_core_exports(i)%name)))) then
          stat = GUATIAO_STAT_MISSING_SYMBOL
          msg = 'guatiao: ' // path // ' does not export ' // trim(gt_core_exports(i)%name)
          return
        end if
      end do
    case (GT_SERDE)
      do i = 1, size(gt_serde_exports)
        if (len_trim(gt_serde_exports(i)%feature) > 0) cycle
        if (.not. c_associated(gt_dl_sym(h, trim(gt_serde_exports(i)%name)))) then
          stat = GUATIAO_STAT_MISSING_SYMBOL
          msg = 'guatiao: ' // path // ' does not export ' // trim(gt_serde_exports(i)%name)
          return
        end if
      end do
    case (GT_INTAKE)
      do i = 1, size(gt_intake_exports)
        if (.not. c_associated(gt_dl_sym(h, trim(gt_intake_exports(i)%name)))) then
          stat = GUATIAO_STAT_MISSING_SYMBOL
          msg = 'guatiao: ' // path // ' does not export ' // trim(gt_intake_exports(i)%name)
          return
        end if
      end do
    end select
  end subroutine

  !> A view of Fortran text for one call. `s` must be associated with the
  !> caller's text for as long as the view is used: every caller passes
  !> its own `target` dummy, so the view lasts the call it makes.
  function gt_view(s) result(v)
    character(kind=c_char, len=*), intent(in), target :: s
    type(gt_str) :: v
    v%len = len(s, kind=c_size_t)
    if (v%len > 0) v%ptr = c_loc(s)
  end function

  !> A view of bytes for one call, as gt_view.
  function gt_bytes_view(b) result(v)
    integer(c_int8_t), intent(in), target :: b(:)
    type(gt_str) :: v
    v%len = size(b, kind=c_size_t)
    if (v%len > 0) v%ptr = c_loc(b)
  end function

  !> Copies `n` bytes at `ptr` into `s`. A subroutine rather than a
  !> function: gfortran 14 keeps a function's deferred-length result
  !> length in static storage, which threads would share.
  subroutine gt_copy_text(ptr, n, s)
    type(c_ptr), intent(in) :: ptr
    integer(c_size_t), intent(in) :: n
    character(:), allocatable, intent(out) :: s
    character(kind=c_char), pointer :: bytes(:)
    integer(c_size_t) :: i
    allocate(character(n) :: s)
    if (n == 0 .or. .not. c_associated(ptr)) return
    call c_f_pointer(ptr, bytes, [n])
    do i = 1, n
      s(i:i) = bytes(i)
    end do
  end subroutine

  !> The text an owned string (a key, a STRING or NUMBER arm) holds.
  subroutine gt_text_of_owned(o, s)
    type(gt_owned), intent(in) :: o
    character(:), allocatable, intent(out) :: s
    call gt_copy_text(o%ptr, o%len, s)
  end subroutine

  !> Whether the `n` bytes at `ptr` are exactly `s`.
  function gt_text_equal(ptr, n, s) result(same)
    type(c_ptr), intent(in) :: ptr
    integer(c_size_t), intent(in) :: n
    character(*), intent(in) :: s
    logical :: same
    character(kind=c_char), pointer :: bytes(:)
    integer(c_size_t) :: i
    same = .false.
    if (n /= len(s, kind=c_size_t)) return
    same = .true.
    if (n == 0) return
    call c_f_pointer(ptr, bytes, [n])
    do i = 1, n
      if (bytes(i) /= s(i:i)) then
        same = .false.
        return
      end if
    end do
  end function

end module guatiao_native
