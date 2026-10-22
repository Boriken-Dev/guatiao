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

end program test_frees
