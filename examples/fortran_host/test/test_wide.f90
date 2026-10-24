! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> 64-bit integers cross a slot exactly: through a provider written in
!> Rust, C or Fortran, and from Rust into the Fortran one. Fortran has no
!> unsigned integer, so a `uint64` is the `int64` holding its bits: every
!> `int64` is a `uint64`, and nothing is out of range, but a value past
!> 2^63 - 1 reads as negative (2^64 - 1 is -1).
program test_wide
  use, intrinsic :: iso_c_binding
  use, intrinsic :: iso_fortran_env, only: int64
  use guatiao
  use guatiao_c, only: gt_str, gt_value
  use guatiao_loader, only: gt_dl_open, gt_dl_sym
  use greeter_kinds
  use host_helpers
  implicit none

  integer(int64), parameter :: I64_MIN = -huge(0_int64) - 1_int64
  integer(int64), parameter :: I64_MAX = huge(0_int64)
  integer(int64), parameter :: TWO53_1 = 2_int64**53 + 1_int64
  integer(int64), parameter :: SIGNED(3) = [TWO53_1, I64_MIN, I64_MAX]
  ! 2^53 + 1, INT64_MAX + 1 and UINT64_MAX, as their bits.
  integer(int64), parameter :: UNSIGNED(3) = [TWO53_1, I64_MIN, -1_int64]
  character(*), parameter :: SIGNED_TEXT(3) = [character(24) :: '9007199254740993', '-9223372036854775808', &
                                                              '9223372036854775807']
  character(*), parameter :: UNSIGNED_TEXT(3) = [character(24) :: '9007199254740993', '9223372036854775808', &
                                                                '18446744073709551615']
  character(*), parameter :: WHICH(3) = [character(8) :: 'rust', 'c', 'fortran']
  character(*), parameter :: KEYS(3) = [character(24) :: 'derived_greeter_hello', 'c_greeter', 'fortran_wide_echo']

  type(guatiao_registry) :: host
  logical :: have(3)
  integer :: i

  call setup()
  call the_kinds_are_the_headers()
  have = .false.
  host = guatiao_open_registry('fortran-wide', '0.1')
  call add_providers()
  do i = 1, 3
    if (have(i)) then
      call a_slot_carries_64_bit_integers_exactly(i)
      call the_rust_proxy_calls_it(i)
    end if
  end do
  if (have(3)) call a_fortran_provider_refuses_with_its_words()
  call finish('wide')

contains

  subroutine add_providers()
    type(guatiao_library) :: lib
    type(echo), pointer :: e
    character(:), allocatable :: dir
    call host%watch_dir(library_dir(), rules=['kind=greeter'])
    have(1) = .true.
    dir = build_c('../../crates/guatiao/tests/c_consumer/greeter_in_c.c', 'build/c-greeter', 'c_greeter')
    if (len(dir) == 0) then
      call skip('no C compiler: the greeter written in C is not built')
    else
      call host%watch_dir('build/c-greeter', rules=['kind=greeter'])
      have(2) = .true.
    end if
    call host%refresh()
    allocate(e)
    lib = guatiao_new_library('fortran_wide', '0.1.0')
    call lib%single('echo', [wide_serving(e)])
    call guatiao_register_library(host, lib)
    have(3) = .true.
  end subroutine

  !> Every table's floor hash and size are the header's.
  subroutine the_kinds_are_the_headers()
    character(:), allocatable :: text
    text = read_file('../greeter_kind/include/greeter_kind.h')
    call check(len(text) > 0, 'the header is read')
    call kind_is('greeter', greeter_kind(), GREETER_FLOOR_HASH, text)
    call kind_is('counter', counter_kind(), COUNTER_FLOOR_HASH, text)
    call kind_is('conversation', conversation_kind(), CONVERSATION_FLOOR_HASH, text)
    call kind_is('listener', listener_kind(), LISTENER_FLOOR_HASH, text)
    call kind_is('wide', wide_kind(), WIDE_FLOOR_HASH, text)
  end subroutine

  subroutine kind_is(name, k, hash, text)
    character(*), intent(in) :: name, text
    type(guatiao_kind), intent(in) :: k
    integer(c_int32_t), intent(in) :: hash
    character(:), allocatable :: table, tail, body
    character(32) :: digits
    integer :: at, to, slots, n
    table = name // '_vtable'
    at = index(text, '#define ' // table // '_FLOOR_HASH ')
    call check(at > 0, name // ': the header defines a floor hash')
    if (at == 0) return
    tail = text(at + len('#define ' // table // '_FLOOR_HASH '):)
    to = scan(tail, new_line('a'))
    digits = tail(1:to - 1)
    call check(iand(int(hash, int64), 4294967295_int64) == to_int(trim(digits)), name // ': the floor hash is the header''s')
    at = index(text, 'typedef struct ' // table // ' {')
    call check(at > 0, name // ': the header declares a table')
    if (at == 0) return
    body = text(at:)
    body = body(:index(body, '} ' // table // ';'))
    slots = 0
    n = index(body, '(*')
    do while (n > 0)
      slots = slots + 1
      body = body(n + 2:)
      n = index(body, '(*')
    end do
    call check(int(k%size) == 8 + 8 * slots, name // ': the table is the header''s size')
  end subroutine

  function to_int(digits) result(v)
    character(*), intent(in) :: digits
    integer(int64) :: v
    integer :: j
    v = 0
    do j = 1, len(digits)
      v = v * 10_int64 + int(iachar(digits(j:j)) - iachar('0'), int64)
    end do
  end function

  function read_file(path) result(text)
    character(*), intent(in) :: path
    character(:), allocatable :: text
    integer :: unit, size
    open (newunit=unit, file=path, access='stream', form='unformatted', status='old', action='read')
    inquire (unit=unit, size=size)
    allocate(character(size) :: text)
    read (unit) text
    close (unit)
  end function

  subroutine a_slot_carries_64_bit_integers_exactly(i)
    integer, intent(in) :: i
    type(wide_remote) :: w
    type(guatiao_error), allocatable :: error
    integer(int64) :: got
    character(:), allocatable :: who
    integer :: st, k
    who = trim(WHICH(i))
    call wide_bind(host, trim(KEYS(i)), w, st)
    call check(st == GUATIAO_OK, who // ': bound as a wide provider')
    if (st /= GUATIAO_OK) return
    do k = 1, 3
      call w%echo_i64(SIGNED(k), got, error)
      call check(.not. allocated(error) .and. got == SIGNED(k), who // ': echo_i64 ' // trim(SIGNED_TEXT(k)))
      call w%echo_u64(UNSIGNED(k), got, error)
      call check(.not. allocated(error) .and. got == UNSIGNED(k), who // ': echo_u64 ' // trim(UNSIGNED_TEXT(k)))
    end do
  end subroutine

  subroutine a_fortran_provider_refuses_with_its_words()
    type(wide_remote) :: w
    type(guatiao_error), allocatable :: error
    integer(int64) :: got
    call wide_bind(host, 'fortran_wide_echo', w)
    call w%echo_i64(7_int64, got, error)
    call check(allocated(error), 'echo_i64 refuses seven')
    if (allocated(error)) call check(error%status == GUATIAO_ERR_BAD_VALUE .and. &
                                     index(error%message, 'seven is no number') > 0, 'with BAD_VALUE and its words')
    call w%echo_u64(7_int64, got, error)
    call check(allocated(error), 'echo_u64 refuses seven')
    if (allocated(error)) call check(error%status == GUATIAO_ERR_BAD_VALUE .and. &
                                     index(error%message, 'seven is no number') > 0, 'with BAD_VALUE and its words')
  end subroutine

  subroutine the_rust_proxy_calls_it(i)
    integer, intent(in) :: i
    abstract interface
      function caller_i(h, key, out) bind(c) result(status)
        import :: c_ptr, gt_str, c_int32_t
        type(c_ptr), value :: h
        type(gt_str), value :: key
        type(c_ptr), value :: out
        integer(c_int32_t) :: status
      end function
    end interface
    procedure(caller_i), pointer :: run
    type(c_ptr) :: lib
    type(gt_value), pointer :: node
    type(guatiao_value) :: report, item
    character(:), allocatable, target :: key
    character(:), allocatable :: who
    integer(c_int32_t) :: st
    type(gt_str) :: k
    integer :: j
    who = trim(WHICH(i))
    lib = gt_dl_open(library_file(library_dir(), 'greeter_caller'))
    call check(c_associated(lib), who // ': the Rust caller opens')
    if (.not. c_associated(lib)) return
    call c_f_procpointer(gt_dl_sym(lib, 'greeter_caller_wide'), run)
    key = trim(KEYS(i))
    k%ptr = c_loc(key)
    k%len = len(key, kind=c_size_t)
    allocate(node)
    st = run(host%host(), k, c_loc(node))
    report = guatiao_value_from_raw(c_loc(node))
    deallocate(node)
    call check(st == GUATIAO_OK, who // ': the Rust proxy ran')
    do j = 1, 3
      item = report%at('i64', j)
      call check_text(item%number_text(), trim(SIGNED_TEXT(j)), who // ' through Rust: i64 ' // trim(SIGNED_TEXT(j)))
      item = report%at('u64', j)
      call check_text(item%number_text(), trim(UNSIGNED_TEXT(j)), who // ' through Rust: u64 ' // trim(UNSIGNED_TEXT(j)))
    end do
  end subroutine

end program test_wide
