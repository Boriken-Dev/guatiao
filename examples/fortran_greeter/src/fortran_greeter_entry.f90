! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> What makes the library a file any host's scan finds: the declaration
!> a scan reads as data before it maps the file, and the entry a host
!> calls. Both are emitted from Fortran; on Windows `library.def` exports
!> them.
module fortran_greeter_entry
  use, intrinsic :: iso_c_binding
  use guatiao, only: guatiao_library
  use fortran_greeter, only: fortran_greeter_library
  implicit none
  private

  public :: guatiao_declares, guatiao_library_entry

  ! One KEY=VALUE per NUL-terminated string, an empty one last.
  character(*), parameter :: DECLARES = 'kind=greeter' // c_null_char // 'kind=counter' // c_null_char // &
                                        c_null_char
  character(kind=c_char), bind(c, name='guatiao_declares') :: guatiao_declares(len(DECLARES)) = &
    transfer(DECLARES, 'a', len(DECLARES))

  type(guatiao_library), save :: library
  logical, save :: made = .false.

contains

  !> The library's descriptor for `host`, built on the first call.
  recursive function guatiao_library_entry(host) bind(c, name='guatiao_library_entry') result(info)
    type(c_ptr), value :: host
    type(c_ptr) :: info
    if (.not. made) then
      library = fortran_greeter_library()
      made = .true.
    end if
    info = library%answer(host)
  end function

end module fortran_greeter_entry
