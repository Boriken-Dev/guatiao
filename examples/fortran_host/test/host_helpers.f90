! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> What the host's tests share: counted checks, stated skips, the
!> workspace's build found when GUATIAO_LIBRARY is unset, C built for the
!> greeter written in it, and a listener that keeps what it heard.
module host_helpers
  use, intrinsic :: iso_c_binding
  use, intrinsic :: iso_fortran_env, only: output_unit
  use guatiao
  use guatiao_loader, only: gt_windows, gt_library_file_names
  use greeter_kinds
  implicit none
  private

  public :: check, check_text, skip, finish, setup, env, library_file, build_c, ear, ear_state, new_ear, &
            upper_of, require_all

  integer :: passed = 0, failed = 0, skipped = 0

  !> What an ear heard, shared by every copy of it.
  type :: ear_state
    type(guatiao_text), allocatable :: heard(:)
    logical :: closed = .false.
  end type

  type, extends(listener) :: ear
    type(ear_state), pointer :: state => null()
  contains
    procedure :: heard => ear_heard
    procedure :: close => ear_close
  end type

contains

  function new_ear() result(e)
    type(ear) :: e
    allocate(e%state)
    allocate(e%state%heard(0))
  end function

  subroutine ear_heard(self, what, error)
    class(ear), intent(inout) :: self
    character(*), intent(in) :: what
    type(guatiao_error), allocatable, intent(out) :: error
    type(guatiao_text), allocatable :: grown(:)
    integer :: n
    n = size(self%state%heard)
    allocate(grown(n + 1))
    if (n > 0) grown(1:n) = self%state%heard
    grown(n + 1)%text = what
    call move_alloc(grown, self%state%heard)
  end subroutine

  subroutine ear_close(self)
    class(ear), intent(inout) :: self
    self%state%closed = .true.
  end subroutine

  subroutine setup()
    if (len(env('GUATIAO_LIBRARY')) == 0) call guatiao_use_library('../../target/debug')
  end subroutine

  !> An environment variable's value, '' when unset.
  function env(name) result(value)
    character(*), intent(in) :: name
    character(:), allocatable :: value
    integer :: n, st
    call get_environment_variable(name, length=n, status=st)
    if (st /= 0 .or. n == 0) then
      value = ''
      return
    end if
    allocate(character(n) :: value)
    call get_environment_variable(name, value)
  end function

  logical function require_all()
    require_all = env('GUATIAO_REQUIRE_ALL') == '1'
  end function

  subroutine check(ok, what)
    logical, intent(in) :: ok
    character(*), intent(in) :: what
    if (ok) then
      passed = passed + 1
    else
      failed = failed + 1
      write (output_unit, '(a)') '  FAIL ' // what
    end if
  end subroutine

  subroutine check_text(got, want, what)
    character(*), intent(in) :: got, want, what
    call check(got == want .and. len(got) == len(want), what // ': got "' // got // '", want "' // want // '"')
  end subroutine

  !> A stated skip; a failure under GUATIAO_REQUIRE_ALL=1.
  subroutine skip(what)
    character(*), intent(in) :: what
    if (require_all()) then
      failed = failed + 1
      write (output_unit, '(a)') '  FAIL (GUATIAO_REQUIRE_ALL) skipped: ' // what
    else
      skipped = skipped + 1
      write (output_unit, '(a)') '  SKIP ' // what
    end if
  end subroutine

  subroutine finish(name)
    character(*), intent(in) :: name
    write (output_unit, '(a,a,i0,a,i0,a,i0,a)') name, ': ', passed, ' passed, ', failed, ' failed, ', &
      skipped, ' skipped'
    if (failed > 0) error stop 1
  end subroutine

  logical function on_macos()
    inquire (file='/usr/lib/libSystem.B.dylib', exist=on_macos)
  end function

  !> The file the library `stem` has in `dir` on this platform.
  function library_file(dir, stem) result(path)
    character(*), intent(in) :: dir, stem
    character(:), allocatable :: path
    character(:), allocatable :: first, second
    call gt_library_file_names(stem, first, second)
    path = dir // '/' // first
    if (on_macos()) path = dir // '/' // second
  end function

  function upper_of(s) result(u)
    character(*), intent(in) :: s
    character(len(s)) :: u
    integer :: i
    u = s
    do i = 1, len(u)
      if (u(i:i) >= 'a' .and. u(i:i) <= 'z') u(i:i) = achar(iachar(u(i:i)) - 32)
    end do
  end function

  !> The first C compiler this machine has (CC first), or ''.
  function c_compiler() result(cc)
    character(:), allocatable :: cc
    character(8), parameter :: tries(3) = [character(8) :: 'clang', 'cc', 'gcc']
    character(:), allocatable :: null
    integer :: i, code
    null = '/dev/null'
    if (gt_windows) null = 'NUL'
    cc = env('CC')
    if (len(cc) > 0) return
    do i = 1, size(tries)
      call execute_command_line(trim(tries(i)) // ' --version > ' // null // ' 2>&1', exitstat=code)
      if (code == 0) then
        cc = trim(tries(i))
        return
      end if
    end do
    cc = ''
  end function

  !> `source` built as the library `stem` in `dir`, with the repository's
  !> headers; '' without a C compiler.
  function build_c(source, dir, stem) result(out)
    character(*), intent(in) :: source, dir, stem
    character(:), allocatable :: out
    character(:), allocatable :: cc, command, native_dir
    integer :: code, i
    out = ''
    cc = c_compiler()
    if (len(cc) == 0) return
    native_dir = dir
    if (gt_windows) then
      do i = 1, len(native_dir)
        if (native_dir(i:i) == '/') native_dir(i:i) = '\'
      end do
      call execute_command_line('if not exist ' // native_dir // ' mkdir ' // native_dir, exitstat=code)
    else
      call execute_command_line('mkdir -p ' // dir, exitstat=code)
    end if
    out = library_file(dir, stem)
    command = cc // ' -shared -std=c11 -Wall -Wextra'
    if (.not. gt_windows) command = command // ' -fPIC -pthread'
    command = command // ' -I ../../crates/guatiao/include -I ../greeter_kind/include -o ' // out // ' ' // source
    call execute_command_line(command, exitstat=code)
    if (code /= 0) then
      write (output_unit, '(a)') '  the C build failed: ' // command
      out = ''
    end if
  end function

end module host_helpers
