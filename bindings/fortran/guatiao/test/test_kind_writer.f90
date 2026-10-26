! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> The kind modules guatiao-kind writes: the committed greeter kinds and
!> shape kinds are what their documents write, and a document no module
!> can be written from is refused, naming the place.
program test_kind_writer
  use guatiao
  use guatiao_kind_writer
  use harness
  implicit none

  call setup()
  call committed_modules_are_current()
  call refusals()
  call finish('kind_writer')

contains

  subroutine committed_modules_are_current()
    integer :: st
    character(:), allocatable :: msg
    msg = ''
    call guatiao_write_kinds(repo_path('examples/greeter_kind/include/greeter_kind.kinds.json'), &
                             repo_path('examples/greeter_kind/fortran/src/greeter_kinds.F90'), check=.true., &
                             stat=st, errmsg=msg)
    call check(st == GUATIAO_OK, 'the greeter kinds are what their document writes: ' // msg)
    call guatiao_write_kinds('test/shape_kind.kinds.json', 'test/shape_kinds.F90', check=.true., stat=st, errmsg=msg)
    call check(st == GUATIAO_OK, 'the shape kinds are what their document writes: ' // msg)
    call written('examples/tally_kind/include/tally_kind.kinds.json', 'tally_kinds')
  end subroutine

  ! A document another language wrote is written as a module too.
  subroutine written(document, module)
    character(*), intent(in) :: document, module
    character(:), allocatable :: json, text, msg
    integer :: st
    msg = ''
    call read_file(repo_path(document), json)
    call guatiao_kinds_source(json, module, 'x.kinds.json', text, st, msg)
    call check(st == GUATIAO_OK .and. index(text, 'module ' // module) > 0, document // ' is written: ' // msg)
  end subroutine

  subroutine refusals()
    character(*), parameter :: HEAD = '{"kinds": [{"name": "k", "floor_hash": 1, "object": false, ' // &
                                      '"slots": [{"name": "f", "required": true, "fallible": false, "args": ['
    call refused(HEAD // '{"name": "l", "type": "list", "pass": "borrowed"}]}]}]}', 'k.f(l): a list is not borrowed')
    call refused(HEAD // '{"name": "b", "type": "bytes_mut"}]}]}]}', 'on an object kind only')
    call refused(HEAD // '{"name": "v", "type": "value", "pass": "lent"}]}]}]}', 'has no Fortran form yet')
    call refused(HEAD // '{"name": "o", "type": "object", "kind": "far", "pass": "lent"}]}]}]}', &
                 'the object kind far is not in this document')
    call refused(HEAD // '{"name": "o", "type": "object", "kind": "k", "pass": "lent"}]}]}]}', &
                 'k is a kind but no object kind')
    call refused(HEAD // '{"name": "a", "type": "i8"}, {"name": "a", "type": "i8"}]}]}]}', 'two arguments are a')
    call refused(HEAD // ']}, {"name": "f", "required": true, "fallible": false, "args": []}]}]}', 'names `f` a second time')
    call refused(HEAD // ']}, {"name": "close", "required": true, "fallible": false, "args": []}]}]}', &
                 'close names no method')
    call refused(HEAD // '], "answer": {"type": "bytes"}}]}]}', 'which no answer is')
    call refused('{"kinds": [{"name": "k", "floor_hash": 1, "object": false, "slots": [' // &
                 '{"name": "f", "required": false, "fallible": false, "args": []}, ' // &
                 '{"name": "g", "required": true, "fallible": false, "args": []}]}]}', 'the required slots come first')
    call refused(HEAD // '{"name": "a", "type": "i8"}, {"name": "A", "type": "i8"}]}]}]}', 'which ignores case')
    call refused(HEAD // ']}, {"name": "kind", "required": true, "fallible": false, "args": []}]}]}', &
                 'two things in the module would be named k_kind')
    call refused('{"kinds": [{"name": "k", "floor_hash": 1, "object": false, "slots": [' // &
                 '{"name": "k", "required": true, "fallible": false, "args": []}]}]}', 'k names no method')
    call refused('{"kinds": [', 'not a value')
    call refused('{"kinds": 3}', 'kinds')
  end subroutine

  subroutine refused(document, says)
    character(*), intent(in) :: document, says
    character(:), allocatable :: text, msg
    integer :: st
    msg = ''
    call guatiao_kinds_source(document, 'k_kinds', 'k.kinds.json', text, st, msg)
    call check(st /= GUATIAO_OK, 'refused: ' // says)
    call check(index(msg, says) > 0 .and. index(msg, 'k.kinds.json: ') == 1, 'naming the place: ' // msg)
    call check(len(text) == 0, 'nothing written: ' // says)
  end subroutine

end program
