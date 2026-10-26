! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> `guatiao-kind DOCUMENT -o FILE [-m MODULE] [--check]` writes the
!> Fortran module of the kinds a kinds document describes (the
!> `<crate>.kinds.json` a kind crate commits, or one another language
!> wrote); `--check` writes nothing and exits 1 when FILE is not what it
!> would write. MODULE defaults to the document's name without
!> `.kinds.json`, plus `s`. The document is read by the guatiao library,
!> found as any program here finds it (GUATIAO_LIBRARY).
program guatiao_kind
  use, intrinsic :: iso_fortran_env, only: error_unit
  use guatiao_kind_writer, only: guatiao_write_kinds
  implicit none
  character(:), allocatable :: document, path, module, msg
  character(4096) :: arg
  logical :: check
  integer :: i, n, st
  document = ''
  path = ''
  module = ''
  check = .false.
  i = 1
  n = command_argument_count()
  do while (i <= n)
    call get_command_argument(i, arg)
    select case (trim(arg))
    case ('-o', '--out')
      i = i + 1
      call get_command_argument(i, arg)
      path = trim(arg)
    case ('-m', '--module')
      i = i + 1
      call get_command_argument(i, arg)
      module = trim(arg)
    case ('--check')
      check = .true.
    case default
      if (len(document) > 0) call usage()
      document = trim(arg)
    end select
    i = i + 1
  end do
  if (len(document) == 0 .or. len(path) == 0) call usage()
  msg = ''
  if (len(module) > 0) then
    call guatiao_write_kinds(document, path, module, check=check, stat=st, errmsg=msg)
  else
    call guatiao_write_kinds(document, path, check=check, stat=st, errmsg=msg)
  end if
  if (st /= 0) then
    write (error_unit, '(a)') msg
    error stop 1
  end if
contains
  subroutine usage()
    write (error_unit, '(a)') 'usage: guatiao-kind DOCUMENT -o FILE [-m MODULE] [--check]'
    error stop 2
  end subroutine
end program
