! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> Frees on every path: trees built in an allocator that counts the
!> blocks it handed out leave none behind, whether the work succeeded,
!> was refused part-way, or the handle simply went out of scope.
program test_frees
  use, intrinsic :: iso_fortran_env, only: int64, int8
  use, intrinsic :: ieee_arithmetic, only: ieee_value, ieee_positive_inf
  use harness
  use guatiao
  use guatiao_serde
  implicit none

  type(counting), pointer :: c

  call setup()
  c => counting_new()
  call a_clean_build()
  call a_refused_write()
  call a_handle_out_of_scope()
  call a_document_that_does_not_parse()
  call a_clone_and_copies()
  call a_channel_conversation()
  call counting_free(c)
  call finish('frees')

contains

  subroutine a_clean_build()
    type(guatiao_value) :: v, tags
    v = guatiao_map(counting_allocator(c))
    call v%set('host', '10.0.0.1')
    call v%set('port', 5900)
    call v%set('ratio', 0.5d0)
    call v%set('raw', [1_int8, 2_int8])
    call v%set('tags', guatiao_list())
    tags = v%key('tags')
    call tags%push('a')
    call tags%push(guatiao_number('1.10'))
    call check(counting_live(c) > 5, 'the tree is in the counting allocator')
    call v%close()
    call check(counting_live(c) == 0, 'a clean build frees every block')
  end subroutine

  subroutine a_refused_write()
    type(guatiao_value) :: v
    integer :: st
    v = guatiao_map(counting_allocator(c))
    call v%set('ok', 1)
    call v%set('bad', ieee_value(0d0, ieee_positive_inf), st)
    call check_status(st, GUATIAO_ERR_BAD_VALUE, 'an infinity refused')
    call v%set('bad', 'x' // achar(255), st)
    call check_status(st, GUATIAO_ERR_BAD_VALUE, 'text that is not UTF-8 refused')
    call v%push(1, st)
    call check_status(st, GUATIAO_ERR_WRONG_KIND, 'a push on a map refused')
    call v%close()
    call check(counting_live(c) == 0, 'refused writes leave nothing behind')
  end subroutine

  subroutine a_handle_out_of_scope()
    call build()
    call check(counting_live(c) == 0, 'the last handle going frees the tree')
  end subroutine

  subroutine build()
    type(guatiao_value) :: v, w
    v = guatiao_map(counting_allocator(c))
    call v%set('k', 'v')
    w = v
    call check(counting_live(c) > 0, 'two handles, one tree')
  end subroutine

  subroutine a_document_that_does_not_parse()
    type(guatiao_value) :: v
    type(guatiao_parse_error) :: where
    integer :: st
    call guatiao_parse('{"a": [1, 2, {"b": ', GUATIAO_FORMAT_JSON, v, alloc=counting_allocator(c), &
                       where=where, stat=st)
    call check_status(st, GUATIAO_ERR_BAD_VALUE, 'a broken document')
    call check(counting_live(c) == 0, 'a document that does not parse leaves nothing behind')
    call guatiao_parse('{"a": [1, 2, {"b": 3}]}', GUATIAO_FORMAT_JSON, v, alloc=counting_allocator(c))
    call check(counting_live(c) > 0, 'a document read into the counting allocator')
    call v%close()
    call check(counting_live(c) == 0, 'and freed')
  end subroutine

  subroutine a_clone_and_copies()
    type(guatiao_value) :: a, b
    a = guatiao_map(counting_allocator(c))
    call a%set('x', 'y')
    b = a%clone(counting_allocator(c))
    call a%close()
    call check(counting_live(c) > 0, 'a clone outlives its source')
    call b%close()
    call check(counting_live(c) == 0, 'and is freed on its own')
  end subroutine

  subroutine a_channel_conversation()
    call converse()
    call check(counting_live(c) == 0, 'a receiver, its schemas and its answers leave nothing behind')
  end subroutine

  subroutine converse()
    type(guatiao_value) :: schema, props, field, hello, empty, back
    type(guatiao_receiver) :: r
    type(guatiao_wire_event) :: a, b, x
    type(guatiao_wire_refusal) :: why
    integer(int8), allocatable :: frame(:), bytes(:)
    integer :: st
    schema = guatiao_map()
    call schema%set('type', 'object')
    props = guatiao_map()
    field = guatiao_map()
    call field%set('type', 'string')
    call props%set('greeting', field)
    call schema%set('properties', props)
    call schema%set('required', guatiao_list())
    field = schema%key('required')
    call field%push('greeting')
    hello = guatiao_map()
    call hello%set('greeting', 'hello')
    empty = guatiao_map()
    r = guatiao_open_receiver(counting_allocator(c))
    call guatiao_wire_announce(5_int64, schema, frame)
    call r%accept(frame, a)
    call check(counting_live(c) > 0, 'the schema and its copy are in the counting allocator')
    call guatiao_wire_send(5_int64, hello, frame)
    call r%accept(frame, b)
    call guatiao_wire_send(5_int64, empty, frame)
    call r%accept(frame, x, why, stat=st)
    call check_status(st, GUATIAO_ERR_BAD_VALUE, 'a value its schema refuses')
    call r%accept([1_int8, 5_int8, 9_int8], x, why, stat=st)
    call check_status(st, GUATIAO_ERR_BAD_VALUE, 'a malformed frame')
    call guatiao_to_wire(hello, bytes)
    call guatiao_from_wire(bytes(1:size(bytes) - 1), back, alloc=counting_allocator(c), stat=st)
    call check_status(st, GUATIAO_ERR_BAD_VALUE, 'truncated bytes')
    call guatiao_from_wire(bytes, back, alloc=counting_allocator(c))
    call r%close()
    call check(counting_live(c) > 0, 'what the receiver answered outlives it')
  end subroutine

end program test_frees
