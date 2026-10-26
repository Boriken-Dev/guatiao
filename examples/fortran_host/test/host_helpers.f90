! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> What the host's tests share: counted checks, stated skips, the
!> workspace's build found when GUATIAO_LIBRARY is unset and the directory
!> holding it, C built for the greeter written in it, a listener that
!> keeps what it heard, and a provider of the wide kind.
module host_helpers
  use, intrinsic :: iso_c_binding
  use, intrinsic :: iso_fortran_env, only: output_unit, int64
  use guatiao
  use guatiao_loader, only: gt_windows, gt_library_file_names
  use greeter_kinds
  implicit none
  private

  public :: check, check_text, skip, finish, setup, env, library_dir, library_file, build_c, ear, ear_state, &
            new_ear, upper_of, require_all, echo, refusing, refusing_new, refusing_allocator

  !> An allocator that counts the blocks it has out and refuses every
  !> allocation while `refuse` is set: the guatiao_alloc first.
  type, bind(c) :: refusing
    integer(c_int32_t) :: struct_size = 0
    type(c_ptr) :: ctx = c_null_ptr
    type(c_funptr) :: alloc = c_null_funptr
    type(c_funptr) :: free = c_null_funptr
    type(c_funptr) :: release = c_null_funptr
    integer(c_int64_t) :: live = 0
    logical(c_bool) :: refuse = .false.
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

  !> Answers the 64-bit integer it is given, and refuses 7.
  type, extends(wide) :: echo
  contains
    procedure :: echo_i64 => echo_answers
    procedure :: echo_u64 => echo_answers
  end type

contains

  subroutine echo_answers(self, x, answer, error)
    class(echo), intent(inout) :: self
    integer(int64), intent(in) :: x
    integer(int64), intent(out) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    answer = x
    if (x == 7_int64) call guatiao_fail(error, GUATIAO_ERR_BAD_VALUE, 'seven is no number')
  end subroutine

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

  function refusing_alloc(ctx, size, align) bind(c) result(p)
    type(c_ptr), value :: ctx
    integer(c_size_t), value :: size, align
    type(c_ptr) :: p
    type(refusing), pointer :: r
    p = c_null_ptr
    call c_f_pointer(ctx, r)
    if (r%refuse .or. size == 0 .or. align > 16) return
    p = c_malloc(size)
    if (c_associated(p)) r%live = r%live + 1
  end function

  subroutine refusing_release(ctx, p, size, align) bind(c)
    type(c_ptr), value :: ctx, p
    integer(c_size_t), value :: size, align
    type(refusing), pointer :: r
    if (.not. c_associated(p)) return
    call c_free(p)
    call c_f_pointer(ctx, r)
    r%live = r%live - 1
  end subroutine

  !> A new refusing allocator, not refusing yet; kept for the process.
  function refusing_new() result(r)
    type(refusing), pointer :: r
    allocate(r)
    r%struct_size = 40
    r%ctx = c_loc(r)
    r%alloc = c_funloc(refusing_alloc)
    r%free = c_funloc(refusing_release)
  end function

  function refusing_allocator(r) result(a)
    type(refusing), pointer, intent(in) :: r
    type(guatiao_allocator) :: a
    a%ptr = c_loc(r)
  end function

  !> The directory holding the workspace's libraries: GUATIAO_LIBRARY, or
  !> the directory of the file it names (an application's own library
  !> carrying every surface, the example libraries beside it).
  function library_dir() result(dir)
    character(:), allocatable :: dir
    integer :: cut
    logical :: holds
    dir = env('GUATIAO_LIBRARY')
    if (len(dir) == 0) then
      dir = '../../target/debug'
      return
    end if
    ! A directory is told by the guatiao file it holds, as the lookup does:
    ! what inquire answers for a directory differs between compilers.
    inquire (file=library_file(dir, 'guatiao'), exist=holds)
    if (holds) return
    ! achar(92) is the backslash: flang's preprocessing pass (CMake runs
    ! one on every source) doubles one written as a literal.
    cut = max(index(dir, '/', back=.true.), index(dir, achar(92), back=.true.))
    if (cut > 1) then
      dir = dir(1:cut - 1)
    else if (cut == 1) then
      dir = dir(1:1)
    else
      dir = '.'
    end if
  end function

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

  !> macOS keeps its system libraries in the dyld cache rather than as
  !> files; this file is on every release.
  logical function on_macos()
    inquire (file='/System/Library/CoreServices/SystemVersion.plist', exist=on_macos)
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
  !> headers and `-D<define>` when one is given; '' without a C compiler.
  function build_c(source, dir, stem, define) result(out)
    character(*), intent(in) :: source, dir, stem
    character(*), intent(in), optional :: define
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
    if (present(define)) command = command // ' -D' // define
    command = command // ' -I ../../crates/guatiao/include -I ../greeter_kind/include -o ' // out // ' ' // source
    call execute_command_line(command, exitstat=code)
    if (code /= 0) then
      write (output_unit, '(a)') '  the C build failed: ' // command
      out = ''
    end if
  end function

end module host_helpers
