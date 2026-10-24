! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> The registry over the workspace's own libraries: scanning by rule,
!> the documented answers, keys and ranks, instances, retiring, unloading,
!> collecting, watched directories and sources, and a closed registry.
program test_registry
  use, intrinsic :: iso_c_binding
  use, intrinsic :: iso_fortran_env, only: int64
  use harness
  use guatiao
  use guatiao_serde
  use guatiao_loader, only: gt_library_file_names
  implicit none

  call setup()
  call scans_by_rule_and_lists_the_providers()
  call answers_the_documented_fields()
  call keys_ranks_and_rekeys()
  call builds_an_instance_and_lends_its_schema()
  call retires_and_unloads()
  call follows_a_watched_directory()
  call a_source_refuses_with_its_own_words()
  call a_closed_registry_refuses_use()
  call answers_in_the_registrys_allocator()
  call finish('registry')

contains

  function scanned() result(reg)
    type(guatiao_registry) :: reg
    reg = guatiao_open_registry('fortran-registry', '0.1')
    call reg%scan_dir(library_dir(), rules=['kind=greeter'])
  end function

  ! The keys of a list of provider maps, space-separated.
  function keys_of(list) result(text)
    type(guatiao_value), intent(in) :: list
    character(:), allocatable :: text
    type(guatiao_value) :: k
    integer :: i
    text = ''
    do i = 1, list%length()
      k = list%at(i, 'key')
      text = text // ' ' // k%string_or('?')
    end do
    text = text // ' '
  end function

  subroutine scans_by_rule_and_lists_the_providers()
    type(guatiao_registry) :: reg
    type(guatiao_value) :: answer, loaded, providers
    reg = guatiao_open_registry('fortran-registry', '0.1')
    call reg%scan_dir(library_dir(), rules=['kind=greeter'], answer=answer)
    loaded = answer%key('loaded')
    call check(loaded%length() >= 1, 'the scan loads what declares a greeter')
    call reg%providers('greeter', providers)
    call check(index(keys_of(providers), ' derived_greeter_hello ') > 0, 'derived_greeter_hello is offered')
    call reg%providers('', providers)
    call check(providers%length() >= 2, 'every provider, with no kind named')
  end subroutine

  subroutine answers_the_documented_fields()
    type(guatiao_registry) :: reg
    type(guatiao_value) :: p, field, why, best
    character(*), parameter :: names(9) = [character(12) :: 'key', 'id', 'version', 'library', 'display_name', &
                                           'from', 'kinds', 'has_config', 'vtable_size']
    integer :: i, st
    reg = scanned()
    call reg%provider('derived_greeter_hello', p)
    do i = 1, size(names)
      field = p%key(trim(names(i)))
      call check(.not. field%is_absent(), 'a provider says its ' // trim(names(i)))
    end do
    call reg%provider('nonesuch', p, st)
    call check_status(st, GUATIAO_ERR_NOT_FOUND, 'no such provider')
    call reg%best('greeter', best)
    field = best%key('key')
    call check(len(field%string_or('')) > 0, 'the best greeter has a key')
    call reg%why_not('codec', why)
    field = why%key('available')
    call check(.not. field%bool_or(.true.), 'nothing serves a codec')
  end subroutine

  subroutine keys_ranks_and_rekeys()
    type(guatiao_registry) :: reg
    type(guatiao_value) :: providers, first
    logical :: ok
    character(:), allocatable :: reason
    reg = guatiao_open_registry('fortran-keys', '0.1')
    call reg%keyed_by('%id@%version')
    call reg%scan_dir(library_dir(), rules=['kind=greeter'])
    call reg%providers('greeter', providers)
    call check(index(keys_of(providers), '@') > 0, 'keys follow the template')
    call reg%set_priority('derived_greeter_shouter', 50)
    call check(reg%priority('derived_greeter_shouter') == 50, 'a rank set is read back')
    call reg%providers('greeter', providers)
    first = providers%at(1, 'id')
    call check_text(first%string_or(''), 'derived_greeter_shouter', 'the highest rank first')
    ok = .false.
    call reg%provider_available(trim(adjustl(key_of(providers, 1))), ok, reason)
    call check(ok, 'a loaded provider can run here')
  end subroutine

  function key_of(list, i) result(k)
    type(guatiao_value), intent(in) :: list
    integer, intent(in) :: i
    character(:), allocatable :: k
    type(guatiao_value) :: v
    v = list%at(i, 'key')
    k = v%string_or('')
  end function

  subroutine builds_an_instance_and_lends_its_schema()
    type(guatiao_registry) :: reg
    type(guatiao_instance) :: inst
    type(guatiao_value) :: schema, config
    type(c_ptr) :: table
    integer(c_size_t) :: size
    integer :: st
    reg = scanned()
    schema = reg%provider_config('derived_greeter_shouter')
    call check(schema%tag() == GUATIAO_TAG_MAP, 'the schema is lent')
    call schema%set('x', 1, st)
    call check_status(st, GUATIAO_STAT_READ_ONLY, 'and read-only')
    call reg%provider_table('derived_greeter_hello', 'greeter', table, size)
    call check(c_associated(table) .and. size > 8, 'a greeter table')
    call reg%provider_table('derived_greeter_hello', 'codec', table, size, st)
    call check_status(st, GUATIAO_ERR_NOT_FOUND, 'no codec table')
    call guatiao_parse('{}', GUATIAO_FORMAT_JSON, config)
    call reg%create('derived_greeter_shouter', config, inst, st)
    call check_status(st, GUATIAO_ERR_BAD_VALUE, 'a configuration the schema refuses')
    call guatiao_parse('{"prefix": "hey"}', GUATIAO_FORMAT_JSON, config)
    call reg%create('derived_greeter_shouter', config, inst, st)
    call check_status(st, GUATIAO_OK, 'an instance')
    call check(.not. inst%is_closed(), 'that is open')
    call inst%close()
    call check(inst%is_closed(), 'until closed')
  end subroutine

  subroutine retires_and_unloads()
    type(guatiao_registry) :: reg
    type(guatiao_value) :: providers, collected, libraries
    integer :: st
    reg = scanned()
    call reg%libraries(libraries)
    call check(libraries%length() >= 1, 'libraries are listed')
    call reg%retire('derived_greeter')
    call reg%providers('greeter', providers)
    call check(index(keys_of(providers), ' derived_greeter_hello ') == 0, 'a retired library offers nothing')
    call reg%collect(collected)
    call check(collected%tag() == GUATIAO_TAG_LIST, 'collect answers the keys it closed')
    call reg%unload('nonesuch', stat=st)
    call check_status(st, GUATIAO_ERR_NOT_FOUND, 'unloading what is not there')
    call reg%unload('hello_library', stat=st)
    call check(st == GUATIAO_OK .or. st == GUATIAO_ERR_UNSUPPORTED .or. st == GUATIAO_ERR_NOT_FOUND, &
               'a library unloads when it agrees: ' // guatiao_status_name(st))
  end subroutine

  subroutine follows_a_watched_directory()
    type(guatiao_registry) :: reg
    type(guatiao_value) :: changed, loaded
    character(:), allocatable :: first, second, bytes
    integer :: u, code
    call gt_library_file_names('derived_greeter', first, second)
    call read_file(library_dir() // '/' // first, bytes)
    if (len(bytes) == 0 .and. len(second) > 0) then
      first = second
      call read_file(library_dir() // '/' // first, bytes)
    end if
    call execute_command_line(mkdir('build/watched'), exitstat=code)
    open (newunit=u, file='build/watched/' // first, access='stream', form='unformatted', status='replace')
    write (u) bytes
    close (u)
    reg = guatiao_open_registry('fortran-watch', '0.1')
    call reg%watch_dir('build/watched', rules=['kind=greeter'])
    call reg%refresh(changed)
    loaded = changed%key('loaded')
    call check(loaded%length() == 1, 'a refresh loads the file the watched directory holds')
    call reg%refresh(changed)
    loaded = changed%key('loaded')
    call check(loaded%length() == 0, 'and nothing more the second time')
  end subroutine

  function mkdir(dir) result(command)
    use guatiao_loader, only: gt_windows
    character(*), intent(in) :: dir
    character(:), allocatable :: command
    integer :: i
    character(len(dir)) :: native
    native = dir
    if (gt_windows) then
      do i = 1, len(native)
        ! achar(92), not a literal: flang's preprocessing doubles one.
        if (native(i:i) == '/') native(i:i) = achar(92)
      end do
      command = 'if not exist ' // native // ' mkdir ' // native
    else
      command = 'mkdir -p ' // dir
    end if
  end function

  subroutine a_source_refuses_with_its_own_words()
    type(guatiao_registry) :: reg
    character(:), allocatable :: first, second, msg
    integer :: st
    reg = guatiao_open_registry('fortran-source', '0.1')
    call gt_library_file_names('derived_greeter', first, second)
    call reg%add_source_file(library_dir() // '/' // first, guatiao_map(), st, msg)
    call check(st /= GUATIAO_OK, 'a library that is no source is refused')
    call check(len(msg) > 0, 'with words')
  end subroutine

  subroutine a_closed_registry_refuses_use()
    type(guatiao_registry) :: reg, copy
    type(guatiao_value) :: providers
    integer :: st
    reg = scanned()
    copy = reg
    call reg%close()
    call copy%providers('greeter', providers, st)
    call check_status(st, GUATIAO_STAT_CLOSED, 'every copy of a closed registry refuses')
    call reg%close()
  end subroutine

  subroutine answers_in_the_registrys_allocator()
    type(counting), pointer :: c
    type(guatiao_registry) :: reg
    type(guatiao_value) :: providers
    integer(int64) :: before
    c => counting_new()
    reg = guatiao_open_registry('fortran-counting', '0.1', alloc=counting_allocator(c))
    call reg%scan_dir(library_dir(), rules=['kind=greeter'])
    before = counting_live(c)
    call reg%providers('greeter', providers)
    call check(counting_live(c) > before, 'an answer is built in the registry''s allocator')
    call providers%close()
    call check(counting_live(c) == before, 'and freed there')
    call reg%close()
  end subroutine

end program test_registry
