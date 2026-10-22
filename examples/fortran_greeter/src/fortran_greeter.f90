! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> Greeters written in Fortran: the same kinds the Rust, C, Python and Go
!> greeters serve, offered by a library any host calls whatever language
!> wrote it. `hello` is its one instance and serves two kinds (with
!> `hello_count`, which shares its state); `shouter` is built from a
!> configuration, one instance per configuration.
module fortran_greeter
  use, intrinsic :: iso_fortran_env, only: int8, int64
  use guatiao
  use greeter_kinds
  implicit none
  private

  public :: hello, hello_count, chat, shouter, shouter_config, fortran_greeter_library
  public :: LIBRARY_ID, LIBRARY_VERSION

  ! As fpm.toml names and versions this package; its test checks they agree.
  character(*), parameter :: LIBRARY_ID = 'fortran_greeter'
  character(*), parameter :: LIBRARY_VERSION = '1.0.0'

  !> Greets, shouts, holds conversations; `asked` is what `hello_count`
  !> counts.
  type, extends(greeter) :: hello
    integer(int64) :: asked = 0
  contains
    procedure :: greet => hello_greet
    procedure :: shout => hello_shout
    procedure :: start => hello_start
    procedure :: greet_into => hello_greet_into
    procedure :: greet_from => hello_greet_from
    procedure :: check => hello_check
  end type

  !> The counter kind of the same provider: one object per kind, sharing
  !> state through a pointer.
  type, extends(counter) :: hello_count
    type(hello), pointer :: of => null()
  contains
    procedure :: count => hello_count_count
  end type

  !> One conversation `start` hands back, holding the listener it was
  !> handed; closing it closes the listener.
  type, extends(conversation) :: chat
    class(listener), allocatable :: heard_by
    type(guatiao_text), allocatable :: transcript(:)
  contains
    procedure :: say => chat_say
    procedure :: read => chat_read
    procedure :: turns => chat_turns
    procedure :: close => chat_close
  end type

  !> What a shouter is built from.
  type, extends(guatiao_config) :: shouter_config
    character(:), allocatable :: prefix
  contains
    procedure :: describe => shouter_describe
    procedure :: decode => shouter_decode
  end type

  type, extends(greeter) :: shouter
    character(:), allocatable :: prefix
  contains
    procedure :: greet => shouter_greet
  end type

contains

  !> The library this package is.
  recursive function fortran_greeter_library() result(lib)
    type(guatiao_library) :: lib
    type(hello), pointer :: h
    type(hello_count), pointer :: c
    type(shouter_config) :: proto
    allocate(h)
    allocate(c)
    c%of => h
    lib = guatiao_new_library(LIBRARY_ID, LIBRARY_VERSION, summary='Greeters written in Fortran', &
                              license='MPL-2.0')
    call lib%single('hello', [greeter_serving(h, shout=.true., start=.true., greet_into=.true., &
                                              greet_from=.true., check=.true.), counter_serving(c)], &
                    display_name='Hello, in Fortran')
    call lib%configured('shouter', proto, build_shouter, [greeter_serving()])
  end function

  ! ---- hello -----------------------------------------------------------

  recursive subroutine hello_greet(self, name, answer, error)
    class(hello), intent(inout) :: self
    character(*), intent(in) :: name
    type(guatiao_value), intent(inout) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    integer :: st
    if (name == 'panic') then
      call guatiao_fail(error, GUATIAO_ERR_INTERNAL, 'a provider bug, on request')
      return
    end if
    if (len(name) == 0) then
      call guatiao_fail(error, GUATIAO_ERR_BAD_VALUE, 'nobody to greet')
      return
    end if
    call answer%set('greeting', 'hello, ' // name, st)
    if (st /= GUATIAO_OK) call guatiao_fail(error, st, 'the answer was refused')
  end subroutine

  recursive subroutine hello_shout(self, name, answer, error)
    class(hello), intent(inout) :: self
    character(*), intent(in) :: name
    character(:), allocatable, intent(out) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    answer = 'hello, ' // name
    call upper(answer)
  end subroutine

  recursive subroutine hello_start(self, listener_in, answer, error)
    class(hello), intent(inout) :: self
    class(listener), allocatable, intent(inout) :: listener_in
    class(conversation), allocatable, intent(out) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    type(chat), allocatable :: c
    allocate(c)
    allocate(c%transcript(0))
    call move_alloc(listener_in, c%heard_by)
    call move_alloc(c, answer)
  end subroutine

  recursive subroutine hello_greet_into(self, name, listener_in, error)
    class(hello), intent(inout) :: self
    character(*), intent(in) :: name
    class(listener), intent(inout), target :: listener_in
    type(guatiao_error), allocatable, intent(out) :: error
    if (len(name) == 0) then
      call guatiao_fail(error, GUATIAO_ERR_BAD_VALUE, 'nobody to greet')
      return
    end if
    call listener_in%heard('hello, ' // name, error)
  end subroutine

  recursive subroutine hello_greet_from(self, config, answer, error)
    class(hello), intent(inout) :: self
    type(guatiao_value), intent(inout) :: config
    type(guatiao_value), intent(inout) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    character(:), allocatable :: name
    integer :: st
    ! The subroutine form: safe from several threads on every compiler.
    call config%get_key('name', name, st)
    if (st /= GUATIAO_OK) name = ''
    call config%close()
    call self%greet(name, answer, error)
  end subroutine

  recursive subroutine hello_check(self, config, answer, error)
    class(hello), intent(inout) :: self
    type(guatiao_value), intent(inout) :: config
    logical, intent(out) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    character(:), allocatable :: name
    integer :: st
    answer = .false.
    call config%get_key('name', name, st)
    if (st /= GUATIAO_OK) name = 'world'
    call config%set('name', name, st)
    if (st == GUATIAO_OK) call config%set('greeting', 'hello, ' // name, st)
    if (st /= GUATIAO_OK) then
      call guatiao_fail(error, st, 'the lent map refused the change')
      return
    end if
    answer = len(name) > 0
  end subroutine

  recursive subroutine hello_count_count(self, answer, error)
    class(hello_count), intent(inout) :: self
    integer(int64), intent(out) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    self%of%asked = self%of%asked + 1
    answer = self%of%asked
  end subroutine

  ! ---- chat ------------------------------------------------------------

  recursive subroutine chat_say(self, what, error)
    class(chat), intent(inout) :: self
    character(*), intent(in) :: what
    type(guatiao_error), allocatable, intent(out) :: error
    type(guatiao_text), allocatable :: grown(:)
    integer :: n
    if (len(what) == 0) then
      call guatiao_fail(error, GUATIAO_ERR_BAD_VALUE, 'nothing to say')
      return
    end if
    n = size(self%transcript)
    allocate(grown(n + 1))
    if (n > 0) grown(1:n) = self%transcript
    grown(n + 1)%text = what
    call move_alloc(grown, self%transcript)
    call self%heard_by%heard(what, error)
  end subroutine

  recursive subroutine chat_read(self, dst, answer, error)
    class(chat), intent(inout) :: self
    integer(int8), intent(inout) :: dst(:)
    integer(int64), intent(out) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    character(:), allocatable :: text
    integer :: i, n
    text = ''
    do i = 1, size(self%transcript)
      if (i > 1) text = text // new_line('a')
      text = text // self%transcript(i)%text
    end do
    n = min(len(text), size(dst))
    do i = 1, n
      dst(i) = int(transfer(text(i:i), 0_int8), int8)
    end do
    answer = n
  end subroutine

  recursive subroutine chat_turns(self, answer, error)
    class(chat), intent(inout) :: self
    integer(int64), intent(out) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    answer = size(self%transcript)
  end subroutine

  recursive subroutine chat_close(self)
    class(chat), intent(inout) :: self
    if (allocated(self%heard_by)) call self%heard_by%close()
  end subroutine

  ! ---- shouter ---------------------------------------------------------

  recursive subroutine shouter_describe(self, schema)
    class(shouter_config), intent(in) :: self
    type(guatiao_schema_builder), intent(inout) :: schema
    call schema%string_field('prefix', description='What every greeting starts with.', min_length=1)
  end subroutine

  recursive subroutine shouter_decode(self, value, error)
    class(shouter_config), intent(inout) :: self
    type(guatiao_value), intent(in) :: value
    type(guatiao_error), allocatable, intent(out) :: error
    integer :: st
    call value%get_key('prefix', self%prefix, st)
    if (st /= GUATIAO_OK) call guatiao_fail(error, st, 'a shouter needs a prefix')
  end subroutine

  recursive subroutine build_shouter(config, made, error)
    class(guatiao_config), intent(in) :: config
    type(guatiao_serving), allocatable, intent(out) :: made(:)
    type(guatiao_error), allocatable, intent(out) :: error
    type(shouter), pointer :: s
    select type (config)
    type is (shouter_config)
      allocate(s)
      s%prefix = config%prefix
      made = [greeter_serving(s)]
    class default
      call guatiao_fail(error, GUATIAO_ERR_WRONG_KIND, 'not a shouter''s configuration')
    end select
  end subroutine

  recursive subroutine shouter_greet(self, name, answer, error)
    class(shouter), intent(inout) :: self
    character(*), intent(in) :: name
    type(guatiao_value), intent(inout) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    character(:), allocatable :: loud
    integer :: st
    loud = name
    call upper(loud)
    call answer%set('greeting', self%prefix // ' ' // loud, st)
    if (st /= GUATIAO_OK) call guatiao_fail(error, st, 'the answer was refused')
  end subroutine

  recursive subroutine upper(s)
    character(*), intent(inout) :: s
    integer :: i
    do i = 1, len(s)
      if (s(i:i) >= 'a' .and. s(i:i) <= 'z') s(i:i) = achar(iachar(s(i:i)) - 32)
    end do
  end subroutine

end module fortran_greeter
