! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> Documents: JSON keeps key order and exact numbers, pretty output,
!> TOML and YAML round trips, and where a document stopped parsing.
program test_serde
  use, intrinsic :: iso_fortran_env, only: int64
  use harness
  use guatiao
  use guatiao_serde
  implicit none

  call setup()
  call json_keeps_order_and_numbers()
  call indents_when_asked()
  call toml_and_yaml_round_trip()
  call says_where_a_document_stopped()
  call finish('serde')

contains

  subroutine json_keeps_order_and_numbers()
    type(guatiao_value) :: v, n
    character(:), allocatable :: text
    call guatiao_parse('{"b": 1, "a": 18446744073709551615.5, "c": [true, null, "x"]}', GUATIAO_FORMAT_JSON, v)
    call guatiao_emit(v, GUATIAO_FORMAT_JSON, text)
    call check_text(text, '{"b":1,"a":18446744073709551615.5,"c":[true,null,"x"]}', &
                    'key order and the exact text kept')
    n = v%key('a')
    call check_text(n%number_text(), '18446744073709551615.5', 'the number read as written')
    n = v%at('c', 1)
    call check(n%bool_or(.false.), 'nested reads')
  end subroutine

  subroutine indents_when_asked()
    type(guatiao_value) :: v
    character(:), allocatable :: text
    call guatiao_parse('{"a": [1, 2]}', GUATIAO_FORMAT_JSON, v)
    call guatiao_emit(v, GUATIAO_FORMAT_JSON, text, pretty=.true.)
    call check(index(text, new_line('a')) > 0, 'pretty output has lines')
  end subroutine

  subroutine toml_and_yaml_round_trip()
    type(guatiao_value) :: v, back, n, list
    character(:), allocatable :: text
    integer :: st
    character(:), allocatable :: msg
    call guatiao_parse('{"name": "ana", "port": 5900, "tags": ["a", "b"]}', GUATIAO_FORMAT_JSON, v)
    call guatiao_emit(v, GUATIAO_FORMAT_TOML, text, stat=st, errmsg=msg)
    if (st == GUATIAO_STAT_MISSING_SYMBOL) then
      call skip('TOML: ' // msg)
    else
      call check_status(st, GUATIAO_OK, 'TOML written')
      call guatiao_parse(text, GUATIAO_FORMAT_TOML, back)
      n = back%key('port')
      call check(n%int_or(0_int64) == 5900, 'TOML read back')
      list = guatiao_list()
      call guatiao_emit(list, GUATIAO_FORMAT_TOML, text, stat=st)
      call check_status(st, GUATIAO_ERR_WRONG_KIND, 'a TOML document is a map')
    end if
    call guatiao_emit(v, GUATIAO_FORMAT_YAML, text, stat=st, errmsg=msg)
    if (st == GUATIAO_STAT_MISSING_SYMBOL) then
      call skip('YAML: ' // msg)
    else
      call check_status(st, GUATIAO_OK, 'YAML written')
      call guatiao_parse(text, GUATIAO_FORMAT_YAML, back)
      n = back%at('tags', 2)
      call check_text(n%string_or(''), 'b', 'YAML read back')
    end if
  end subroutine

  subroutine says_where_a_document_stopped()
    type(guatiao_value) :: v
    type(guatiao_parse_error) :: where
    integer :: st
    character(:), allocatable :: msg
    call guatiao_parse('{"a": 1,' // new_line('a') // '  "b": }', GUATIAO_FORMAT_JSON, v, where=where, &
                       stat=st, errmsg=msg)
    call check_status(st, GUATIAO_ERR_BAD_VALUE, 'a document that does not parse')
    call check(where%line == 2, 'the line')
    call check(where%column > 0, 'the column')
    call check(len(where%reason) > 0, 'the reason')
    call check(index(msg, 'line 2') > 0, 'the message says where')
    call check(v%is_absent(), 'nothing answered')
  end subroutine

end program test_serde
