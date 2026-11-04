! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> What every test program shares: checks that count, a stated skip, the
!> workspace's library found when GUATIAO_LIBRARY is unset and the
!> directory holding it, files read whole, and an allocator counting the
!> blocks it handed out.
module harness
  use, intrinsic :: iso_c_binding
  use, intrinsic :: iso_fortran_env, only: int64, output_unit
  use guatiao
  use guatiao_loader, only: gt_library_file_names, gt_dl_open, gt_dl_sym, gt_windows
  implicit none
  private

  public :: check, check_text, check_status, skip, finish, setup, read_file, repo_path, library_dir, &
            counting, counting_new, counting_allocator, counting_live, counting_free, require_all, build_c, sha256_of, catalog_built

  integer :: passed = 0, failed = 0, skipped = 0

  !> The allocator's state: the guatiao_alloc first, then the counts.
  type, bind(c) :: counting
    integer(c_int32_t) :: struct_size = 0
    type(c_ptr) :: ctx = c_null_ptr
    type(c_funptr) :: alloc = c_null_funptr
    type(c_funptr) :: free = c_null_funptr
    type(c_funptr) :: release = c_null_funptr
    integer(c_int64_t) :: live = 0
    integer(c_int64_t) :: total = 0
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

  !> Points the binding at the workspace's debug build unless
  !> GUATIAO_LIBRARY names one.
  subroutine setup()
    integer :: n, st
    character(16) :: probe
    call get_environment_variable('GUATIAO_LIBRARY', probe, n, st)
    if (st /= 0 .and. st /= -1) call guatiao_use_library(repo_path('target/debug'))
  end subroutine

  !> The directory holding the workspace's libraries: GUATIAO_LIBRARY, or
  !> the directory of the file it names (an application's own library
  !> carrying every surface, the example libraries beside it).
  function library_dir() result(dir)
    character(:), allocatable :: dir, first, second
    character(4096) :: buf
    integer :: n, st, cut
    logical :: holds
    call get_environment_variable('GUATIAO_LIBRARY', buf, n, st)
    if (st /= 0 .or. n == 0) then
      dir = repo_path('target/debug')
      return
    end if
    dir = buf(1:n)
    ! A directory is told by the guatiao file it holds, as the lookup does:
    ! what inquire answers for a directory differs between compilers.
    call gt_library_file_names('guatiao', first, second)
    inquire (file=dir // '/' // first, exist=holds)
    if (.not. holds .and. len(second) > 0) inquire (file=dir // '/' // second, exist=holds)
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

  !> A path inside the repository, from this package's directory.
  function repo_path(rel) result(p)
    character(*), intent(in) :: rel
    character(:), allocatable :: p
    p = '../../../' // rel
  end function

  !> Whether every greeter must be there (GUATIAO_REQUIRE_ALL=1): a skip
  !> then fails.
  logical function require_all()
    character(8) :: v
    integer :: n, st
    call get_environment_variable('GUATIAO_REQUIRE_ALL', v, n, st)
    require_all = st == 0 .and. v(1:n) == '1'
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
    if (got == want .and. len(got) == len(want)) then
      passed = passed + 1
    else
      failed = failed + 1
      write (output_unit, '(a)') '  FAIL ' // what // ': got "' // got // '", want "' // want // '"'
    end if
  end subroutine

  subroutine check_status(got, want, what)
    integer, intent(in) :: got, want
    character(*), intent(in) :: what
    if (got == want) then
      passed = passed + 1
    else
      failed = failed + 1
      write (output_unit, '(a)') '  FAIL ' // what // ': got ' // guatiao_status_name(got) // &
                                 ', want ' // guatiao_status_name(want)
    end if
  end subroutine

  !> A stated skip: counted, printed, and a failure under GUATIAO_REQUIRE_ALL=1.
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

  !> A file's bytes as text ('' when it does not open): read in blocks, then byte by byte at the
  !> end (flang on Windows answers -1 for a file's size), into a buffer
  !> that doubles, so no long concatenation lands on the stack.
  subroutine read_file(path, text)
    character(*), intent(in) :: path
    character(:), allocatable, intent(out) :: text
    character(:), allocatable :: buf, grown
    character(4096) :: block
    character :: one
    integer :: u, ios, at
    open (newunit=u, file=path, access='stream', form='unformatted', action='read', status='old', iostat=ios)
    if (ios /= 0) then
      text = ''
      return
    end if
    allocate(character(65536) :: buf)
    at = 1
    do
      read (u, pos=at, iostat=ios) block
      if (ios /= 0) exit
      call room(len(block))
      buf(at:at + len(block) - 1) = block
      at = at + len(block)
    end do
    do
      read (u, pos=at, iostat=ios) one
      if (ios /= 0) exit
      call room(1)
      buf(at:at) = one
      at = at + 1
    end do
    close (u)
    allocate(character(at - 1) :: text)
    text(:) = buf(1:at - 1)
  contains
    subroutine room(n)
      integer, intent(in) :: n
      if (at + n - 1 <= len(buf)) return
      allocate(character(2 * len(buf) + n) :: grown)
      grown(1:at - 1) = buf(1:at - 1)
      call move_alloc(grown, buf)
    end subroutine
  end subroutine

  ! ---- the counting allocator -------------------------------------------

  function counting_alloc(ctx, size, align) bind(c) result(p)
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
    c%total = c%total + 1
  end function

  subroutine counting_release(ctx, p, size, align) bind(c)
    type(c_ptr), value :: ctx, p
    integer(c_size_t), value :: size, align
    type(counting), pointer :: c
    if (.not. c_associated(p)) return
    call c_free(p)
    call c_f_pointer(ctx, c)
    c%live = c%live - 1
  end subroutine

  !> A new counting allocator, kept until counting_free.
  function counting_new() result(c)
    type(counting), pointer :: c
    allocate(c)
    c%struct_size = 40
    c%ctx = c_loc(c)
    c%alloc = c_funloc(counting_alloc)
    c%free = c_funloc(counting_release)
  end function

  function counting_allocator(c) result(a)
    type(counting), pointer, intent(in) :: c
    type(guatiao_allocator) :: a
    a%ptr = c_loc(c)
  end function

  integer(int64) function counting_live(c)
    type(counting), pointer, intent(in) :: c
    counting_live = c%live
  end function

  subroutine counting_free(c)
    type(counting), pointer, intent(inout) :: c
    deallocate(c)
  end subroutine

  ! ---- C built for the tests that call a library written in it ----------

  !> The first C compiler this machine has (CC first), or ''.
  function c_compiler() result(cc)
    character(:), allocatable :: cc
    character(8), parameter :: tries(3) = [character(8) :: 'clang', 'cc', 'gcc']
    character(:), allocatable :: null
    character(256) :: named
    integer :: i, code, st, n
    call get_environment_variable('CC', named, n, st)
    if (st == 0 .and. n > 0) then
      cc = named(1:n)
      return
    end if
    null = '/dev/null'
    if (gt_windows) null = 'NUL'
    do i = 1, size(tries)
      ! cmdstat: gfortran stops the program for a command not found without it.
      call execute_command_line(trim(tries(i)) // ' --version > ' // null // ' 2>&1', exitstat=code, cmdstat=st)
      if (st == 0 .and. code == 0) then
        cc = trim(tries(i))
        return
      end if
    end do
    cc = ''
  end function

  !> `crates/guatiao/tests/c_consumer/<source>` built as the library file
  !> `out` (its folder made first), with the repository's headers; '' without
  !> a C compiler or when the build fails.
  function build_c(source, out) result(built)
    character(*), intent(in) :: source, out
    character(:), allocatable :: built
    character(:), allocatable :: cc, command, folder
    integer :: code, cut
    built = ''
    cc = c_compiler()
    if (len(cc) == 0) return
    cut = index(out, '/', back=.true.)
    folder = out(1:cut - 1)
    if (gt_windows) then
      call execute_command_line('if not exist "' // folder // '" mkdir "' // folder // '"', exitstat=code)
    else
      call execute_command_line('mkdir -p ' // folder, exitstat=code)
    end if
    command = cc // ' -shared -std=c11 -Wall -Wextra'
    if (.not. gt_windows) command = command // ' -fPIC -pthread'
    command = command // ' -I ' // repo_path('crates/guatiao/include') // &
              ' -I ' // repo_path('examples/greeter_kind/include') // ' -o ' // out // ' ' // &
              repo_path('crates/guatiao/tests/c_consumer/' // source)
    call execute_command_line(command, exitstat=code)
    if (code /= 0) then
      write (output_unit, '(a)') '  the C build failed: ' // command
      return
    end if
    built = out
  end function

  !> The lowercase hex SHA-256 of the file at `path`, read through the
  !> platform's own tool, or '' when it has none.
  function sha256_of(path) result(hex)
    character(*), intent(in) :: path
    character(:), allocatable :: hex
    character(:), allocatable :: said, line
    integer :: code, st, i, cut
    hex = ''
    if (gt_windows) then
      call execute_command_line('certutil -hashfile "' // path // '" SHA256 > build/sha256.txt', exitstat=code, cmdstat=st)
    else
      call execute_command_line('(sha256sum "' // path // '" || shasum -a 256 "' // path // '") > build/sha256.txt 2>/dev/null', &
                                exitstat=code, cmdstat=st)
    end if
    if (st /= 0 .or. code /= 0) return
    call read_file('build/sha256.txt', said)
    if (gt_windows) then
      cut = index(said, new_line('a'))
      if (cut == 0) return
      said = said(cut + 1:)
    end if
    cut = index(said, new_line('a'))
    if (cut > 0) said = said(1:cut - 1)
    line = ''
    do i = 1, len(said)
      if (said(i:i) == ' ') then
        if (gt_windows) cycle
        exit
      end if
      if (said(i:i) == achar(13)) exit
      line = line // said(i:i)
    end do
    hex = line
  end function

  !> How many times any `create` of the catalog library at `path` was called.
  integer function catalog_built(path)
    character(*), intent(in) :: path
    interface
      function counted() bind(c) result(n)
        import :: c_int
        integer(c_int) :: n
      end function
    end interface
    procedure(counted), pointer :: fn
    type(c_ptr) :: h
    catalog_built = -1
    h = gt_dl_open(path)
    if (.not. c_associated(h)) return
    call c_f_procpointer(gt_dl_sym(h, 'catalog_built'), fn)
    catalog_built = int(fn())
  end function

end module harness
