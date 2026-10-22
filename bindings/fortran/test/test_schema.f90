! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> Schemas: a configuration type's schema checked by the library's own
!> validator, decoding into the type, and redacting secrets.
module schema_fixture
  use, intrinsic :: iso_fortran_env, only: int64
  use guatiao
  implicit none

  type, extends(guatiao_config) :: shout_config
    character(:), allocatable :: prefix
    integer(int64) :: times = 1
    logical :: loud = .false.
    character(:), allocatable :: token
  contains
    procedure :: describe => shout_describe
    procedure :: decode => shout_decode
  end type

contains

  subroutine shout_describe(self, schema)
    class(shout_config), intent(in) :: self
    type(guatiao_schema_builder), intent(inout) :: schema
    call schema%string_field('prefix', description='What every shout starts with', min_length=1)
    call schema%integer_field('times', default=1_int64, minimum=1_int64, maximum=10_int64)
    call schema%boolean_field('loud', default=.false.)
    call schema%string_field('token', sensitive=.true., required=.false.)
  end subroutine

  subroutine shout_decode(self, value, error)
    class(shout_config), intent(inout) :: self
    type(guatiao_value), intent(in) :: value
    type(guatiao_error), allocatable, intent(out) :: error
    integer :: st
    call value%get_key('prefix', self%prefix, st)
    if (st /= GUATIAO_OK) then
      call guatiao_fail(error, st, 'no prefix')
      return
    end if
    if (value%has('times')) call value%get_key('times', self%times)
    if (value%has('loud')) call value%get_key('loud', self%loud)
    if (value%has('token')) call value%get_key('token', self%token)
  end subroutine

end module schema_fixture

program test_schema
  use, intrinsic :: iso_fortran_env, only: int64
  use harness
  use guatiao
  use guatiao_serde
  use schema_fixture
  implicit none

  call setup()
  call a_config_type_describes_itself()
  call decodes_what_fits_and_refuses_what_does_not()
  call redacts_every_secret()
  call finish('schema')

contains

  function json(text) result(v)
    character(*), intent(in) :: text
    type(guatiao_value) :: v
    call guatiao_parse(text, GUATIAO_FORMAT_JSON, v)
  end function

  subroutine a_config_type_describes_itself()
    type(guatiao_value) :: schema
    character(:), allocatable :: text
    type(shout_config) :: proto
    schema = guatiao_schema_of(proto)
    call guatiao_emit(schema, GUATIAO_FORMAT_JSON, text)
    call check_text(text, '{"type":"object","properties":{"prefix":{"type":"string","description":' // &
      '"What every shout starts with","minLength":1},"times":{"type":"integer","default":1,"minimum":1,' // &
      '"maximum":10},"loud":{"type":"boolean","default":false},"token":{"type":"string","x-sensitive":true}},' // &
      '"additionalProperties":false,"required":["prefix"]}', 'the schema, field by field')
  end subroutine

  subroutine decodes_what_fits_and_refuses_what_does_not()
    type(shout_config) :: c
    type(guatiao_value) :: why, schema
    integer :: st
    character(:), allocatable :: msg
    call guatiao_decode_config(c, json('{"prefix": "hey", "times": 3}'))
    call check_text(c%prefix, 'hey', 'decoded')
    call check(c%times == 3 .and. .not. c%loud, 'with its defaults')
    call guatiao_decode_config(c, json('{"prefix": ""}'), st, msg)
    call check_status(st, GUATIAO_ERR_BAD_VALUE, 'too short')
    call check(index(msg, 'prefix') > 0, 'the refusal names the field')
    call guatiao_decode_config(c, json('{"prefix": "a", "extra": 1}'), st)
    call check_status(st, GUATIAO_ERR_BAD_VALUE, 'a key it does not declare')
    call guatiao_decode_config(c, json('{"times": 2}'), st)
    call check_status(st, GUATIAO_ERR_BAD_VALUE, 'a required field missing')
    schema = guatiao_schema_of(c)
    call guatiao_validate(schema, json('{"prefix": "a", "times": 11}'), why, st)
    call check_status(st, GUATIAO_ERR_BAD_VALUE, 'past the maximum')
    call check(.not. why%is_absent(), 'with a map saying why')
  end subroutine

  subroutine redacts_every_secret()
    type(guatiao_value) :: schema, shown, item
    type(shout_config) :: proto
    schema = guatiao_schema_of(proto)
    call guatiao_redact(schema, json('{"prefix": "hey", "token": "s3cret"}'), '[redacted]', shown)
    item = shown%key('token')
    call check_text(item%string_or(''), '[redacted]', 'the secret hidden')
    item = shown%key('prefix')
    call check_text(item%string_or(''), 'hey', 'the rest kept')
  end subroutine

end program test_schema
