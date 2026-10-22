! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> The library names and versions itself as its fpm.toml does, and a host
!> in this process calls it through its tables, the answer built in the
!> allocator the host named.
program test_library
  use, intrinsic :: iso_c_binding
  use, intrinsic :: iso_fortran_env, only: int64, output_unit
  use guatiao
  use guatiao_serde
  use greeter_kinds
  use fortran_greeter
  use counting_alloc
  implicit none

  integer :: failed = 0, passed = 0

  call setup()
  call names_itself_as_fpm_toml_does()
  call a_host_in_this_process_calls_it()
  call answers_in_the_callers_allocator()
  write (output_unit, '(a,i0,a,i0,a)') 'fortran_greeter: ', passed, ' passed, ', failed, ' failed'
  if (failed > 0) error stop 1

contains

  subroutine check(ok, what)
    logical, intent(in) :: ok
    character(*), intent(in) :: what
    if (ok) then
      passed = passed + 1
    else
      failed = failed + 1
      write (output_unit, '(a)') '  FAIL ' // what
    end if
  end subroutine

  subroutine setup()
    integer :: n, st
    character(8) :: probe
    call get_environment_variable('GUATIAO_LIBRARY', probe, n, st)
    if (st == 1) call guatiao_use_library('../../target/debug')
  end subroutine

  ! The value of `key = "..."` in fpm.toml.
  function manifest(key) result(value)
    character(*), intent(in) :: key
    character(:), allocatable :: value
    character(512) :: line
    integer :: u, ios, a, b
    value = ''
    open (newunit=u, file='fpm.toml', action='read', status='old')
    do
      read (u, '(a)', iostat=ios) line
      if (ios /= 0) exit
      if (index(line, key // ' = "') /= 1) cycle
      a = index(line, '"')
      b = index(line(a + 1:), '"') + a
      value = line(a + 1:b - 1)
      exit
    end do
    close (u)
  end function

  subroutine names_itself_as_fpm_toml_does()
    type(guatiao_library) :: lib
    call check(manifest('name') == LIBRARY_ID, 'the id is fpm.toml''s name')
    call check(manifest('version') == LIBRARY_VERSION, 'the version is fpm.toml''s')
    lib = fortran_greeter_library()
    call check(lib%id() == LIBRARY_ID .and. lib%version() == LIBRARY_VERSION, 'the library says the same')
  end subroutine

  subroutine a_host_in_this_process_calls_it()
    type(guatiao_registry) :: reg
    type(greeter_remote) :: g
    type(counter_remote) :: c
    type(guatiao_value) :: answer, providers
    type(guatiao_error), allocatable :: error
    character(:), allocatable :: text
    integer(int64) :: n
    reg = guatiao_open_registry('fortran-greeter-test', '0.1')
    call guatiao_register_library(reg, fortran_greeter_library())
    call reg%providers('greeter', providers)
    call check(providers%length() == 2, 'two greeters registered')
    call greeter_bind(reg, 'fortran_greeter_hello', g)
    call g%greet('ana', answer, error)
    call check(.not. allocated(error), 'greeted')
    call guatiao_emit(answer, GUATIAO_FORMAT_JSON, text)
    call check(text == '{"greeting":"hello, ana"}', 'hello, ana')
    call g%greet('', answer, error)
    call check(allocated(error), 'nobody is refused')
    if (allocated(error)) call check(error%status == GUATIAO_ERR_BAD_VALUE .and. &
                                     error%message == 'nobody to greet', 'with its status and words')
    call counter_bind(reg, 'fortran_greeter_hello', c)
    call c%count(n, error)
    call check(n == 1, 'the counter kind of the same provider')
  end subroutine

  subroutine answers_in_the_callers_allocator()
    type(guatiao_registry) :: reg
    type(greeter_remote) :: g
    type(guatiao_value) :: answer
    type(guatiao_error), allocatable :: error
    type(counting), pointer :: alloc
    alloc => counting_new()
    reg = guatiao_open_registry('fortran-counting', '0.1', alloc=guatiao_allocator(c_loc(alloc)))
    call guatiao_register_library(reg, fortran_greeter_library())
    call greeter_bind(reg, 'fortran_greeter_hello', g)
    call g%greet('ana', answer, error)
    call check(alloc%live > 0, 'the answer is in the allocator the caller named')
    call answer%close()
    call check(alloc%live == 0, 'and nothing of it is left once freed')
  end subroutine

end program test_library
