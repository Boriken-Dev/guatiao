! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> Where serde's library is looked for. The environment is read once a
!> process, so each case is this program again, run with its own: a
!> surface's variable naming a file whatever it is called, or a directory;
!> GUATIAO_LIBRARY naming a file that carries the surface, used as it is,
!> or one that does not, whose sibling is used. Serde opens the core
!> first, so the own-variable cases name a copy of the core with no serde
!> beside it: only the own variable can find serde.
program test_lookup
  use, intrinsic :: iso_fortran_env, only: output_unit
  use harness
  use guatiao_native, only: GT_SERDE, gt_need, guatiao_library_path
  use guatiao_loader, only: gt_windows, gt_library_file_names
  implicit none

  character(:), allocatable :: dir, serde, core, app, renamed, lone_core
  integer :: unit

  if (command_argument_count() > 0) then
    call report_serde()
    stop
  end if

  dir = library_dir()
  serde = file_of('guatiao_serde')
  core = file_of('guatiao')
  if (len(serde) == 0 .or. len(core) == 0) then
    call skip('the lookup cases: no guatiao and guatiao_serde built in ' // dir)
    call finish('lookup')
  end if
  renamed = temp_dir() // '/guatiao-fortran-lookup-renamed'
  call copy_file(serde, renamed)
  lone_core = temp_dir() // '/guatiao-fortran-lookup-core'
  call copy_file(core, lone_core)

  call check_text(serde_found_with('GUATIAO_SERDE_LIBRARY', renamed, 'GUATIAO_LIBRARY', lone_core), &
                  renamed, "serde's own variable names a file, whatever it is called")
  call check_text(serde_found_with('GUATIAO_SERDE_LIBRARY', dir, 'GUATIAO_LIBRARY', lone_core), &
                  serde, "serde's own variable names a directory")
  call check_text(serde_found_with('GUATIAO_LIBRARY', core, '', ''), serde, &
                  'GUATIAO_LIBRARY naming a file without serde points at its sibling')
  app = file_of('myapp')
  if (len(app) > 0) then
    call check_text(serde_found_with('GUATIAO_LIBRARY', app, '', ''), app, &
                    'GUATIAO_LIBRARY naming a file carrying serde serves it as it is')
  else
    call skip('a file carrying serde: no application library (examples/app_library) built in ' // dir)
  end if
  open (newunit=unit, file=renamed, status='old')
  close (unit, status='delete')
  open (newunit=unit, file=lone_core, status='old')
  close (unit, status='delete')
  call finish('lookup')

contains

  !> The child: opens serde from the environment it was given and prints
  !> the file it came from, or why it found none.
  subroutine report_serde()
    integer :: st
    character(:), allocatable :: msg
    call gt_need(GT_SERDE, st, msg)
    if (st == 0) then
      write (output_unit, '(a)') guatiao_library_path(GT_SERDE)
    else
      write (output_unit, '(a)') 'not found: ' // msg
    end if
  end subroutine

  !> What the child answers with only `var` (and `other`, when named) set
  !> among the lookup's variables.
  function serde_found_with(var, value, other, other_value) result(found)
    character(*), intent(in) :: var, value, other, other_value
    character(:), allocatable :: found, self, out, command, sets
    character(4096) :: buf
    integer :: n
    call get_command_argument(0, buf, n)
    self = buf(1:n)
    out = temp_dir() // '/guatiao-fortran-lookup.out'
    if (gt_windows) then
      sets = 'set "GUATIAO_SERDE_LIBRARY=" & set "GUATIAO_LIBRARY=" & set "' // var // '=' // value // '" & '
      if (len(other) > 0) sets = sets // 'set "' // other // '=' // other_value // '" & '
      command = sets // '"' // self // '" child > "' // out // '"'
    else
      sets = 'env -u GUATIAO_SERDE_LIBRARY -u GUATIAO_LIBRARY ' // var // "='" // value // "' "
      if (len(other) > 0) sets = sets // other // "='" // other_value // "' "
      command = sets // "'" // self // "' child > '" // out // "'"
    end if
    call execute_command_line(command)
    call read_file(out, found)
    found = trim(adjustl(strip_newlines(found)))
  end function

  !> `name`'s library file in the library directory, or '' when not built.
  function file_of(name) result(path)
    character(*), intent(in) :: name
    character(:), allocatable :: path, first, second
    logical :: there
    call gt_library_file_names(name, first, second)
    path = dir // '/' // first
    inquire (file=path, exist=there)
    if (there) return
    path = ''
    if (len(second) == 0) return
    path = dir // '/' // second
    inquire (file=path, exist=there)
    if (.not. there) path = ''
  end function

  !> The platform's temporary directory.
  function temp_dir() result(t)
    character(:), allocatable :: t
    character(4096) :: buf
    integer :: n, st
    if (gt_windows) then
      call get_environment_variable('TEMP', buf, n, st)
    else
      call get_environment_variable('TMPDIR', buf, n, st)
    end if
    if (st == 0 .and. n > 0) then
      t = buf(1:n)
    else if (gt_windows) then
      t = '.'
    else
      t = '/tmp'
    end if
  end function

  !> Copies `from` to `to` byte for byte.
  subroutine copy_file(from, to)
    character(*), intent(in) :: from, to
    character(:), allocatable :: bytes
    integer :: in, out, size_of
    open (newunit=in, file=from, access='stream', form='unformatted', status='old', action='read')
    inquire (unit=in, size=size_of)
    allocate (character(size_of) :: bytes)
    read (in) bytes
    close (in)
    open (newunit=out, file=to, access='stream', form='unformatted', status='replace', action='write')
    write (out) bytes
    close (out)
  end subroutine

  function strip_newlines(text) result(s)
    character(*), intent(in) :: text
    character(:), allocatable :: s
    integer :: i
    s = ''
    do i = 1, len(text)
      if (text(i:i) /= achar(10) .and. text(i:i) /= achar(13)) s = s // text(i:i)
    end do
  end function

end program
