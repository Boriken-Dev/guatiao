! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> A plugin host written in Fortran.
!>
!> Scans the directories it is given for libraries declaring the greeter
!> kind, whatever language wrote them, registers the greeters written in
!> Fortran beside them, and calls each greeter through the kind's table:
!> a map back, a map lent and changed in place, a map handed over, an
!> object lent for one call, an object handed in and one handed back. A
!> provider building instances is given a configuration its schema
!> accepts. With `--python-source BRIDGE --python-dir DIR`, the guatiao-source-py
!> bridge serves the Python greeters described under DIR too.
!>
!>     fortran_host DIR [DIR ...] [--python-source BRIDGE --python-dir DIR]
program fortran_host
  use, intrinsic :: iso_fortran_env, only: int8, int64
  use guatiao
  use guatiao_serde
  use greeter_kinds
  use fortran_greeter, only: fortran_greeter_library
  use host_listener
  implicit none

  type(guatiao_registry) :: reg
  type(guatiao_value) :: providers, provider, item, config, refreshed
  character(:), allocatable :: arg, bridge, pydir, key, text
  integer :: i, n

  reg = guatiao_open_registry('fortran-host', '1.0')
  bridge = ''
  pydir = ''
  n = command_argument_count()
  i = 1
  do while (i <= n)
    call argument(i, arg)
    if (arg == '--python-source' .and. i < n) then
      call argument(i + 1, bridge)
      i = i + 2
      cycle
    end if
    if (arg == '--python-dir' .and. i < n) then
      call argument(i + 1, pydir)
      i = i + 2
      cycle
    end if
    call reg%scan_dir(arg, rules=['kind=greeter'], answer=item)
    call guatiao_emit(item%key('loaded'), GUATIAO_FORMAT_JSON, text)
    print '(a)', 'scanned ' // arg // ': loaded ' // text
    i = i + 1
  end do
  if (len(bridge) > 0) then
    config = guatiao_map()
    call config%set('dirs', guatiao_list())
    item = config%key('dirs')
    if (len(pydir) > 0) call item%push(pydir)
    ! The bridge imports python3.dll from the Python installation, on PATH.
    call reg%set_import_search(GUATIAO_IMPORT_SEARCH_PATH)
    call reg%add_source_file(bridge, config)
    call reg%refresh(refreshed)
    call guatiao_emit(refreshed%key('loaded'), GUATIAO_FORMAT_JSON, text)
    print '(a)', 'served from Python: ' // text
  end if
  call guatiao_register_library(reg, fortran_greeter_library())

  call reg%providers('greeter', providers)
  do i = 1, providers%length()
    provider = providers%item(i)
    item = provider%key('key')
    call item%get(key)
    print '(/,a)', key
    item = provider%key('has_config')
    if (item%bool_or(.false.)) then
      call configured(key)
    else
      call exercise_key(key)
    end if
  end do
  call reg%close()

contains

  recursive subroutine argument(i, value)
    integer, intent(in) :: i
    character(:), allocatable, intent(out) :: value
    integer :: length
    call get_command_argument(i, length=length)
    allocate(character(length) :: value)
    call get_command_argument(i, value)
  end subroutine

  recursive subroutine exercise_key(key)
    character(*), intent(in) :: key
    type(greeter_remote) :: g
    integer :: st
    call greeter_bind(reg, key, g, st)
    if (st /= GUATIAO_OK) then
      print '(a)', '  not this greeter kind'
      return
    end if
    call exercise(g)
  end subroutine

  recursive subroutine configured(key)
    character(*), intent(in) :: key
    type(guatiao_instance) :: inst
    type(greeter_remote) :: g
    type(guatiao_value) :: config
    character(:), allocatable :: msg
    integer :: st
    config = guatiao_map()
    call reg%create(key, config, inst, st, msg)
    print '(a)', '  an empty configuration is refused: ' // msg
    call config%set('prefix', 'hey')
    call reg%create(key, config, inst, st, msg)
    if (st /= GUATIAO_OK) then
      ! A provider whose schema takes no prefix: its own table, as it is.
      print '(a)', '  nor is {"prefix": "hey"}: ' // msg
      call exercise_key(key)
      return
    end if
    call greeter_bind_instance(inst, g)
    print '(a)', '  an instance from {"prefix": "hey"}'
    call exercise(g)
    call inst%close()
  end subroutine

  !> Every slot of the greeter kind the table carries.
  recursive subroutine exercise(g)
    type(greeter_remote), intent(inout) :: g
    type(guatiao_value) :: answer, config
    type(guatiao_error), allocatable :: error
    character(:), allocatable :: text
    logical :: usable
    type(echo), target :: ear
    class(listener), allocatable :: given
    class(conversation), allocatable :: chat
    integer(int8) :: buf(64)
    integer(int64) :: n
    call g%greet('ana', answer, error)
    call guatiao_emit(answer, GUATIAO_FORMAT_JSON, text)
    print '(a)', '  greet: ' // text
    call g%greet('', answer, error)
    if (allocated(error)) print '(a)', '  greet refused: ' // error%message
    if (g%has('shout')) then
      call g%shout('ana', text, error)
      print '(a)', '  shout: ' // text
    end if
    if (g%has('check')) then
      config = guatiao_map()
      call config%set('name', 'bo')
      call g%check(config, usable, error)
      call guatiao_emit(config, GUATIAO_FORMAT_JSON, text)
      print '(a,l1,a)', '  check: ', usable, ', the lent map now ' // text
    end if
    if (g%has('greet_from')) then
      config = guatiao_map()
      call config%set('name', 'cy')
      call g%greet_from(config, answer, error)
      call guatiao_emit(answer, GUATIAO_FORMAT_JSON, text)
      print '(a)', '  greet_from: ' // text
    end if
    if (g%has('greet_into')) then
      call g%greet_into('dee', ear, error)
      print '(a)', '  greet_into: the lent listener was told'
    end if
    if (g%has('start')) then
      allocate(echo :: given)
      call g%start(given, chat, error)
      if (allocated(chat)) then
        call chat%say('hi', error)
        call chat%say('again', error)
        buf = 0
        call chat%read(buf, n, error)
        text = transfer(buf(1:n), repeat(' ', int(n)))
        call chat%turns(n, error)
        print '(a,i0,a)', '  conversation: ', n, ' turns, transcript "' // text // '"'
        call chat%close()
      end if
    end if
  end subroutine

end program fortran_host
