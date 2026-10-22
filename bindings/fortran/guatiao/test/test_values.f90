! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> Values: building, reading, the map and list protocols, exact numbers,
!> handles that share a tree, closing, and what is refused.
program test_values
  use, intrinsic :: iso_fortran_env, only: int8, int32, int64, real32, real64
  use, intrinsic :: ieee_arithmetic, only: ieee_value, ieee_positive_inf
  use harness
  use guatiao
  implicit none

  call setup()
  call every_kind_round_trips()
  call numbers_keep_their_text()
  call reads_with_the_header_rules()
  call the_map_protocol()
  call the_list_protocol()
  call a_handle_stays_valid_while_the_map_grows()
  call copies_share_a_tree_and_close_frees_it()
  call clones_are_independent()
  call refuses_the_wrong_container()
  call absent_and_null()
  call paths_are_spelled_in_the_intake_grammar()
  call reals_are_written_shortest()
  call from_raw_takes_a_tree_over()
  call finish('values')

contains

  subroutine every_kind_round_trips()
    type(guatiao_value) :: v, item
    logical :: b
    integer(int64) :: n
    real(real64) :: x
    character(:), allocatable :: s
    integer(int8), allocatable :: bytes(:)
    integer :: st
    v = guatiao_map()
    call v%set('flag', .true.)
    call v%set('port', 5900)
    call v%set('big', 9007199254740993_int64)
    call v%set('ratio', 0.25_real64)
    call v%set('host', '10.0.0.1')
    call v%set('raw', [0_int8, 1_int8, -1_int8])
    call v%set('nothing', guatiao_null())
    call v%set('tags', guatiao_list())
    call check(v%length() == 8, 'eight entries')
    call v%get_key('flag', b)
    call check(b, 'a boolean')
    call v%get_key('port', n)
    call check(n == 5900, 'an integer')
    call v%get_key('big', n)
    call check(n == 9007199254740993_int64, 'an integer past 2**53, exact')
    call v%get_key('ratio', x)
    call check(x == 0.25_real64, 'a real')
    call v%get_key('host', s)
    call check_text(s, '10.0.0.1', 'a string')
    call v%get_key('raw', bytes)
    call check(size(bytes) == 3, 'bytes')
    if (size(bytes) == 3) call check(all(bytes == [0_int8, 1_int8, -1_int8]), 'bytes, every value')
    item = v%key('nothing')
    call check(item%is_null(), 'null')
    item = v%key('tags')
    call check(item%tag() == GUATIAO_TAG_LIST .and. item%length() == 0, 'an empty list')
    call item%get(n, st)
    call check_status(st, GUATIAO_ERR_WRONG_KIND, 'a list is not a number')
    call v%close()
  end subroutine

  subroutine numbers_keep_their_text()
    type(guatiao_value) :: v, n
    integer(int64) :: i
    integer :: st
    v = guatiao_map()
    call v%set('a', guatiao_number('1.10'))
    call v%set('b', guatiao_number('18446744073709551615.5'))
    call v%set('c', guatiao_number('-0'))
    n = v%key('a')
    call check_text(n%number_text(), '1.10', 'a number keeps its trailing zero')
    n = v%key('b')
    call check_text(n%number_text(), '18446744073709551615.5', 'and its every digit')
    call n%get(i, st)
    call check_status(st, GUATIAO_ERR_BAD_VALUE, 'a number with a fraction is no integer')
    n = guatiao_number('1.2.3', stat=st)
    call check_status(st, GUATIAO_ERR_BAD_VALUE, 'text outside the grammar is refused')
    n = v%key('a')
    call check_text(n%string_or('fallback'), 'fallback', 'a number is not a string')
  end subroutine

  subroutine reads_with_the_header_rules()
    type(guatiao_value) :: v, n
    integer(int32) :: small
    integer :: st
    v = guatiao_list()
    call v%push(guatiao_number('9223372036854775807'))
    call v%push(guatiao_number('9223372036854775808'))
    call v%push(guatiao_number('-9223372036854775808'))
    call v%push(guatiao_number('1e2'))
    call v%push(guatiao_number('2147483648'))
    call v%push(guatiao_number('1e400'))
    n = v%item(1)
    call check(n%int_or(0_int64) == huge(0_int64), 'the largest int64')
    n = v%item(2)
    call check(n%int_or(-7_int64) == -7, 'one past it falls back')
    n = v%item(3)
    call check(n%int_or(0_int64) == -huge(0_int64) - 1, 'the smallest int64')
    n = v%item(4)
    call check(n%int_or(-7_int64) == -7, 'an exponent is not an integer spelling')
    call check(n%real_or(0d0) == 100d0, 'but is a real')
    n = v%item(5)
    call n%get(small, st)
    call check_status(st, GUATIAO_ERR_BAD_VALUE, 'past int32 is refused, not wrapped')
    n = v%item(6)
    call check(n%real_or(-1d0) == -1d0, 'no finite real falls back')
  end subroutine

  subroutine the_map_protocol()
    type(guatiao_value) :: m, other
    type(guatiao_text), allocatable :: keys(:)
    integer :: st
    m = guatiao_map()
    call m%set('b', 1)
    call m%set('a', 2)
    call m%set('c', 3)
    call m%set('a', 20)
    keys = m%keys()
    call check(size(keys) == 3, 'three keys')
    if (size(keys) == 3) then
      call check(keys(1)%text == 'b' .and. keys(2)%text == 'a' .and. keys(3)%text == 'c', &
                 'insertion order, a replaced in place')
    end if
    call check(m%has('a') .and. .not. m%has('z'), 'has')
    call m%remove('b')
    call check(m%length() == 2 .and. .not. m%has('b'), 'removed')
    call m%remove('b', st)
    call check_status(st, GUATIAO_ERR_NOT_FOUND, 'removing what is not there')
    other = guatiao_map()
    call other%set('d', 4)
    call other%set('a', 200)
    call m%copy_from(other)
    call check(m%length() == 3, 'copied one map into another')
    other = m%key('a')
    call check(other%int_or(0_int64) == 200, 'collisions replaced')
    call m%clear()
    call check(m%length() == 0, 'cleared')
  end subroutine

  subroutine the_list_protocol()
    type(guatiao_value) :: l, last
    integer :: st
    l = guatiao_list()
    call l%push('a')
    call l%push('b')
    call l%push('c')
    call check(l%length() == 3, 'three elements')
    last = l%item(0)
    call check_text(last%string_or(''), 'c', 'item(0) is the last')
    last = l%item(-1)
    call check_text(last%string_or(''), 'b', 'item(-1) the one before')
    call l%remove_item(1)
    last = l%item(1)
    call check_text(last%string_or(''), 'b', 'the first removed')
    call l%remove_item(9, st)
    call check_status(st, GUATIAO_ERR_NOT_FOUND, 'removing past the end')
    call l%clear()
    call check(l%length() == 0, 'cleared')
  end subroutine

  subroutine a_handle_stays_valid_while_the_map_grows()
    type(guatiao_value) :: m, first
    integer :: i
    character(8) :: k
    m = guatiao_map()
    call m%set('first', 'kept')
    first = m%key('first')
    do i = 1, 200
      write (k, '(a,i0)') 'k', i
      call m%set(trim(k), i)
    end do
    call check_text(first%string_or(''), 'kept', 'a handle walks again after the array moved')
  end subroutine

  subroutine copies_share_a_tree_and_close_frees_it()
    type(guatiao_value) :: a, b, view
    integer :: st
    a = guatiao_map()
    call a%set('x', 1)
    b = a
    call b%set('y', 2)
    call check(a%length() == 2, 'a copy is another handle on the same tree')
    view = a%key('x')
    call a%close()
    call check(b%is_closed() .and. view%is_absent(), 'closed for every handle')
    call b%set('z', 3, st)
    call check_status(st, GUATIAO_STAT_CLOSED, 'a write after close')
    call a%close()
  end subroutine

  subroutine clones_are_independent()
    type(guatiao_value) :: a, b
    a = guatiao_map()
    call a%set('x', 1)
    b = a%clone()
    call b%set('y', 2)
    call check(a%length() == 1 .and. b%length() == 2, 'a clone is its own tree')
  end subroutine

  subroutine refuses_the_wrong_container()
    type(guatiao_value) :: m, l, below
    integer :: st
    m = guatiao_map()
    l = guatiao_list()
    call m%push(1, st)
    call check_status(st, GUATIAO_ERR_WRONG_KIND, 'push on a map')
    call l%set('a', 1, st)
    call check_status(st, GUATIAO_ERR_WRONG_KIND, 'set on a list')
    below = m%key('nope')
    call below%set('a', 1, st)
    call check_status(st, GUATIAO_ERR_NOT_FOUND, 'set below nothing')
  end subroutine

  subroutine absent_and_null()
    type(guatiao_value) :: m, gone
    integer(int64) :: n
    integer :: st
    m = guatiao_map()
    gone = m%key('missing')
    call check(gone%is_absent() .and. .not. gone%is_null(), 'a missing key is absent')
    call gone%get(n, st)
    call check_status(st, GUATIAO_ERR_NOT_FOUND, 'reading nothing')
    gone = guatiao_null()
    call check(gone%is_null(), 'the stored nothing')
    gone = guatiao_absent()
    call check(gone%is_absent(), 'no value')
  end subroutine

  subroutine paths_are_spelled_in_the_intake_grammar()
    type(guatiao_value) :: v, at
    v = guatiao_map()
    at = v%at('agent', 2, 'name')
    call check_text(at%path_text(), 'agent[1].name', 'keys and positions from 0')
    at = v%at('a.b', 'c d', 'x"y')
    call check_text(at%path_text(), '["a.b"]["c d"]["x\"y"]', 'keys that are not plain are quoted')
  end subroutine

  subroutine reals_are_written_shortest()
    type(guatiao_value) :: l, n
    real(real64) :: back
    integer :: i, st
    real(real64), parameter :: xs(8) = [0.1_real64, 1.5_real64, 100.0_real64, 1e21_real64, 1.25e-7_real64, &
                                        -2.5_real64, 123456789.125_real64, 5e-324_real64]
    character(*), parameter :: texts(8) = [character(24) :: '0.1', '1.5', '100', '1e21', '1.25e-7', '-2.5', &
                                             '123456789.125', '5e-324']
    l = guatiao_list()
    do i = 1, size(xs)
      call l%push(xs(i))
      n = l%item(i)
      call check_text(n%number_text(), trim(texts(i)), 'real written as ' // trim(texts(i)))
      call n%get(back)
      call check(back == xs(i), 'and read back the same')
    end do
    call l%push(0.1_real32)
    n = l%item(0)
    call check_text(n%number_text(), '0.1', 'a real32 written as its own shortest')
    call l%push(-0d0)
    n = l%item(0)
    call check_text(n%number_text(), '-0', 'negative zero keeps its sign')
    call l%push(ieee_value(0d0, ieee_positive_inf), st)
    call check_status(st, GUATIAO_ERR_BAD_VALUE, 'an infinity has no number form')
  end subroutine

  subroutine from_raw_takes_a_tree_over()
    type(guatiao_value) :: a, b, k
    a = guatiao_map()
    call a%set('k', 'v')
    b = guatiao_value_from_raw(a%raw())
    call check(a%tag() == GUATIAO_TAG_ABSENT, 'the source node is left ABSENT')
    k = b%key('k')
    call check(k%string_or('') == 'v', 'the tree moved')
  end subroutine

end program test_values
