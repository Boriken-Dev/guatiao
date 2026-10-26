! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> Providers of the kinds guatiao-kind writes from shape_kind.kinds.json
!> (shape_kinds.F90): every shape a slot takes or answers.
module shape_providers
  use, intrinsic :: iso_c_binding
  use, intrinsic :: iso_fortran_env, only: int8, int16, int32, int64
  use guatiao
  use shape_kinds
  implicit none
  private

  public :: pad, every_shape, heard

  !> What a note served from here was told last, wherever it went.
  character(64), save :: heard = ''

  type, extends(note) :: pad
    character(:), allocatable :: said
  contains
    procedure :: told => pad_told
    procedure :: fill => pad_fill
    procedure :: last => pad_last
  end type

  type, extends(shapes) :: every_shape
    class(note), allocatable :: kept
  contains
    procedure :: scalars => shape_scalars
    procedure :: flag => shape_flag
    procedure :: split => shape_split
    procedure :: sum => shape_sum
    procedure :: borrow => shape_borrow
    procedure :: lend => shape_lend
    procedure :: take => shape_take
    procedure :: number => shape_number
    procedure :: keep => shape_keep
    procedure :: poke => shape_poke
    procedure :: make => shape_make
    procedure :: echo => shape_echo
  end type

contains

  recursive subroutine pad_told(self, text, error)
    class(pad), intent(inout) :: self
    character(*), intent(in) :: text
    type(guatiao_error), allocatable, intent(out) :: error
    self%said = text
    heard = text
  end subroutine

  recursive subroutine pad_fill(self, dst, answer, error)
    class(pad), intent(inout) :: self
    integer(int8), intent(inout) :: dst(:)
    integer(int64), intent(out) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    integer :: i
    do i = 1, min(3, size(dst))
      dst(i) = int(i, int8)
    end do
    answer = min(3, size(dst))
  end subroutine

  recursive subroutine pad_last(self, answer, error)
    class(pad), intent(inout) :: self
    character(:), allocatable, intent(out) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    answer = ''
    if (allocated(self%said)) answer = self%said
  end subroutine

  recursive subroutine shape_scalars(self, b, a, c, d, e, f, g, h, i, j, k, l, m, answer, error)
    class(every_shape), intent(inout) :: self
    logical, intent(in) :: b
    integer(int8), intent(in) :: a, f
    integer(int16), intent(in) :: c, g
    integer(int32), intent(in) :: d, h
    integer(int64), intent(in) :: e, i
    integer(c_intptr_t), intent(in) :: j
    integer(c_size_t), intent(in) :: k
    real(c_float), intent(in) :: l
    real(c_double), intent(in) :: m
    real(c_double), intent(out) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    if (a < 0) then
      call guatiao_fail(error, GUATIAO_ERR_BAD_VALUE, 'no negative a')
      answer = 0
      return
    end if
    answer = merge(1d0, 0d0, b) + a + c + d + e + f + g + h + i + j + k + l + m
  end subroutine

  recursive subroutine shape_flag(self, on, answer, error)
    class(every_shape), intent(inout) :: self
    logical, intent(in) :: on
    logical, intent(out) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    answer = .not. on
  end subroutine

  recursive subroutine shape_split(self, a, b, c, s, answer, error)
    class(every_shape), intent(inout) :: self
    integer(int32), intent(in) :: a, b, c
    character(*), intent(in) :: s
    character(:), allocatable, intent(out) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    character(12) :: digits
    write (digits, '(i0)') a + b + c
    answer = s // ' ' // trim(digits)
  end subroutine

  recursive subroutine shape_sum(self, data, answer, error)
    class(every_shape), intent(inout) :: self
    integer(int8), intent(in) :: data(:)
    integer(int32), intent(out) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    integer :: i
    answer = 0
    do i = 1, size(data)
      answer = answer + iand(int(data(i), int32), 255)
    end do
  end subroutine

  recursive subroutine shape_borrow(self, m, v, n, answer, error)
    class(every_shape), intent(inout) :: self
    type(guatiao_value), intent(in) :: m, v, n
    type(guatiao_value), intent(inout) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    call answer%set('m', int(m%length(), int64))
    call answer%set('v', int(v%tag(), int64))
    call answer%set('n', n%is_absent())
  end subroutine

  recursive subroutine shape_lend(self, m, l, answer, error)
    class(every_shape), intent(inout) :: self
    type(guatiao_value), intent(inout) :: m, l
    logical, intent(out) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    call m%set('seen', .true.)
    call l%push(7_int64)
    answer = .true.
  end subroutine

  recursive subroutine shape_take(self, m, l, answer, error)
    class(every_shape), intent(inout) :: self
    type(guatiao_value), intent(inout) :: m, l
    type(guatiao_value), intent(inout) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    call answer%push(int(m%length(), int64))
    call answer%push(int(l%length(), int64))
    call m%close()
    call l%close()
  end subroutine

  recursive subroutine shape_number(self, x, answer, error)
    class(every_shape), intent(inout) :: self
    integer(int64), intent(in) :: x
    type(guatiao_value), intent(inout) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    answer = guatiao_of(2 * x)
  end subroutine

  recursive subroutine shape_keep(self, n, error)
    class(every_shape), intent(inout) :: self
    class(note), allocatable, intent(inout) :: n
    type(guatiao_error), allocatable, intent(out) :: error
    call n%told('kept', error)
    call move_alloc(n, self%kept)
  end subroutine

  recursive subroutine shape_poke(self, n, answer, error)
    class(every_shape), intent(inout) :: self
    class(note), intent(inout), target :: n
    integer(int64), intent(out) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    call n%told('poked', error)
    answer = 3
  end subroutine

  recursive subroutine shape_make(self, answer, error)
    class(every_shape), intent(inout) :: self
    class(note), allocatable, intent(out) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    allocate(pad :: answer)
  end subroutine

  recursive subroutine shape_echo(self, s, answer, error)
    class(every_shape), intent(inout) :: self
    character(*), intent(in) :: s
    character(:), allocatable, intent(out) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    answer = s // s
  end subroutine

end module shape_providers

program test_shape_kinds
  use, intrinsic :: iso_c_binding
  use, intrinsic :: iso_fortran_env, only: int8, int16, int32, int64
  use guatiao
  use shape_kinds
  use shape_providers
  use harness
  implicit none

  type(guatiao_registry) :: reg
  type(guatiao_library) :: lib
  type(every_shape), pointer :: provider
  type(shapes_remote) :: g
  type(guatiao_error), allocatable :: error
  integer :: st

  call setup()
  reg = guatiao_open_registry('fortran-shapes', '0.1')
  allocate(provider)
  lib = guatiao_new_library('shapes_test', '0.1.0')
  call lib%single('all', [shapes_serving(provider, echo=.true.)])
  call guatiao_register_library(reg, lib)
  call shapes_bind(reg, 'shapes_test_all', g, st)
  call check(st == GUATIAO_OK, 'the provider binds as the kind it serves')
  if (st == GUATIAO_OK) then
    call scalars_cross_exactly()
    call text_bytes_and_flags()
    call trees_borrowed_lent_and_taken()
    call objects_handed_lent_and_answered()
    call appended_slots()
  end if
  call reg%close()
  call finish('shape_kinds')

contains

  subroutine scalars_cross_exactly()
    real(c_double) :: total
    total = 0
    call g%scalars(.true., 1_int8, 2_int16, 3_int32, 4_int64, 5_int8, 6_int16, 7_int32, 8_int64, &
                   9_c_intptr_t, 10_c_size_t, 0.5_c_float, 0.25_c_double, total, error)
    call check(.not. allocated(error), 'scalars: every one crosses')
    call check(abs(total - 56.75d0) < 1d-9, 'scalars: the sum of every one')
    call g%scalars(.false., -1_int8, 0_int16, 0_int32, 0_int64, 0_int8, 0_int16, 0_int32, 0_int64, &
                   0_c_intptr_t, 0_c_size_t, 0.0_c_float, 0.0_c_double, total, error)
    call check(allocated(error), 'scalars: a refusal crosses')
    if (allocated(error)) call check(error%status == GUATIAO_ERR_BAD_VALUE .and. &
                                     index(error%message, 'no negative a') > 0, 'scalars: with its words')
  end subroutine

  subroutine text_bytes_and_flags()
    logical :: answer
    character(:), allocatable :: text
    integer(int32) :: total
    call g%flag(.true., answer, error)
    call check(.not. allocated(error) .and. .not. answer, 'flag: a bool crosses both ways')
    call g%split(1_int32, 2_int32, 3_int32, 'six', text, error)
    call check(.not. allocated(error), 'split: a view at the fifth register crosses')
    if (allocated(text)) call check_text(text, 'six 6', 'split: and its answer')
    call g%sum([10_int8, 20_int8, -1_int8], total, error)
    call check(total == 285, 'sum: bytes cross')
  end subroutine

  subroutine trees_borrowed_lent_and_taken()
    type(guatiao_value) :: m, v, answer, l, got, item
    logical :: ok
    m = guatiao_map()
    call m%set('a', 1_int64)
    call m%set('b', 2_int64)
    v = guatiao_of('text')
    call g%borrow(m, v, guatiao_absent(), answer, error)
    call check(.not. allocated(error), 'borrow: a map, a value and a null value are borrowed')
    item = answer%key('m')
    call check(item%int_or(-1_int64) == 2, 'borrow: the map is read')
    item = answer%key('v')
    call check(item%int_or(-1_int64) == GUATIAO_TAG_STRING, 'borrow: the value is read')
    item = answer%key('n')
    call check(item%bool_or(.false.), 'borrow: the null value is null')
    call check(m%length() == 2, 'borrow: what was borrowed is left as it was')
    l = guatiao_list()
    call g%lend(m, l, ok, error)
    call check(.not. allocated(error) .and. ok, 'lend: a map and a list are lent')
    call check(m%has('seen') .and. l%length() == 1, 'lend: changed where they stand')
    call g%take(m, l, got, error)
    call check(.not. allocated(error), 'take: a map and a list are handed over')
    item = got%item(1)
    call check(item%int_or(-1_int64) == 3, 'take: the map arrives')
    item = got%item(2)
    call check(item%int_or(-1_int64) == 1, 'take: the list arrives')
    m = guatiao_map()
    call m%set('kept', .true.)
    l = guatiao_map()
    call g%take(m, l, got, error)
    call check(allocated(error), 'take: a map where a list belongs is refused')
    if (allocated(error)) call check(error%status == GUATIAO_ERR_WRONG_KIND, 'take: as the wrong kind')
    call check(.not. m%is_closed() .and. m%has('kept'), 'take: and the map before it is left where it was')
    call g%number(21_int64, got, error)
    call check(.not. allocated(error) .and. got%int_or(-1_int64) == 42, 'number: a value is answered')
  end subroutine

  subroutine objects_handed_lent_and_answered()
    class(note), allocatable :: handed, made
    type(pad), target :: lent
    integer(int64) :: n
    integer(int8) :: buf(5)
    character(:), allocatable :: text
    allocate(pad :: handed)
    call g%keep(handed, error)
    call check(.not. allocated(error) .and. .not. allocated(handed), 'keep: an object is handed over')
    call check(trim(heard) == 'kept', 'keep: and the provider calls it')
    call g%poke(lent, n, error)
    call check(.not. allocated(error) .and. n == 3, 'poke: an object is lent')
    call check(allocated(lent%said), 'poke: and called in place')
    if (allocated(lent%said)) call check_text(lent%said, 'poked', 'poke: told what it was')
    call g%make(made, error)
    call check(.not. allocated(error) .and. allocated(made), 'make: an object is answered')
    if (.not. allocated(made)) return
    call made%told('hello', error)
    call check(.not. allocated(error), 'make: the answer is called')
    buf = 0_int8
    call made%fill(buf, n, error)
    call check(n == 3 .and. all(buf == [1_int8, 2_int8, 3_int8, 0_int8, 0_int8]), 'fill: bytes written in place')
    call made%last(text, error)
    call check(.not. allocated(error), 'last: an appended slot of an object kind')
    if (allocated(text)) call check_text(text, 'hello', 'last: answers what it was told')
    call made%close()
  end subroutine

  subroutine appended_slots()
    character(:), allocatable :: text
    call check(g%has('echo') .and. .not. g%has('quiet') .and. .not. g%has('nothing'), &
               'has: the appended slots served, and no others')
    call g%echo('ab', text, error)
    call check(.not. allocated(error), 'echo: an appended slot served')
    if (allocated(text)) call check_text(text, 'abab', 'echo: and its answer')
    call g%quiet('x', error)
    call check(allocated(error), 'quiet: an appended slot nobody serves refuses')
    if (allocated(error)) call check(error%status == GUATIAO_ERR_UNSUPPORTED, 'quiet: as unsupported')
  end subroutine

end program
