! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> The host's own listener: an object kind a library calls back.
module host_listener
  use guatiao
  use greeter_kinds
  implicit none
  private
  public :: echo

  type, extends(listener) :: echo
  contains
    procedure :: heard => echo_heard
  end type

contains

  recursive subroutine echo_heard(self, what, error)
    class(echo), intent(inout) :: self
    character(*), intent(in) :: what
    type(guatiao_error), allocatable, intent(out) :: error
    print '(a)', '    the listener heard: ' // what
  end subroutine

end module host_listener
