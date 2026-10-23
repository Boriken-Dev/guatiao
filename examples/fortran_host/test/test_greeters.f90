! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> One behaviour, five languages: a Fortran host calls the greeters
!> written in Rust, C, Python, Go and Fortran through one description of
!> the kind, every slot, and a Rust caller calls each of them through the
!> kind's Rust proxy. The greeters written in other languages come
!> through sources the registry keeps (watched directories, the
!> guatiao-py bridge), loaded by one refresh; the Fortran one is
!> registered in this process. Calls from threads the host did not start
!> reach the Fortran greeter too.
program test_greeters
  use, intrinsic :: iso_c_binding
  use, intrinsic :: iso_fortran_env, only: int8, int64
  use guatiao
  use guatiao_c, only: gt_str, gt_value
  use guatiao_loader, only: gt_dl_open, gt_dl_sym
  use guatiao_serde
  use greeter_kinds
  use fortran_greeter, only: fortran_greeter_library
  use host_helpers
  implicit none

  character(*), parameter :: WHICH(5) = [character(8) :: 'rust', 'c', 'python', 'go', 'fortran']
  character(*), parameter :: KEYS(5) = [character(24) :: 'derived_greeter_hello', 'c_greeter', &
                                        'py_greeter_hello', 'go_greeter_hello', 'fortran_greeter_hello']
  character(*), parameter :: GREETINGS(5) = [character(24) :: 'hello, ana', 'hello from C, ana', &
                                             'hello, ana', 'hello, ana', 'hello, ana']

  type(guatiao_registry) :: host
  logical :: have(5)
  integer :: i

  call setup()
  have = .false.
  host = guatiao_open_registry('fortran-greeters', '0.1')
  call add_sources()
  do i = 1, 5
    if (have(i)) then
      call a_fortran_host_calls_it(i)
      call the_rust_proxy_calls_it(i)
    end if
  end do
  call a_table_that_is_not_the_kinds_is_refused()
  call a_configured_provider_builds_instances()
  call native_threads_call_the_fortran_greeter()
  call finish('greeters')

contains

  subroutine add_sources()
    type(guatiao_value) :: changed, list, item, config
    character(:), allocatable :: dir, bridge, text
    integer :: st, k
    logical :: exists
    character(:), allocatable :: msg
    call host%watch_dir(library_dir(), rules=['kind=greeter'])
    have(1) = .true.
    dir = build_c('../../crates/guatiao/tests/c_consumer/greeter_in_c.c', 'build/c-greeter', 'c_greeter')
    if (len(dir) == 0) then
      call skip('no C compiler: the greeter written in C is not built')
    else
      call host%watch_dir('build/c-greeter', rules=['kind=greeter'])
      have(2) = .true.
    end if
    dir = env('GUATIAO_GO_LIBRARY')
    if (len(dir) == 0) then
      call skip('GUATIAO_GO_LIBRARY names no directory holding the Go greeter built with -buildmode=c-shared')
    else
      call host%watch_dir(dir, rules=['kind=greeter'])
      have(4) = .true.
    end if
    bridge = env('GUATIAO_PY_BRIDGE')
    if (len(bridge) == 0) bridge = library_file('../../crates/guatiao-py/target/debug', 'guatiao_py')
    inquire (file=bridge, exist=exists)
    if (.not. exists) then
      call skip('no guatiao-py build: set GUATIAO_PY_BRIDGE')
    else
      call guatiao_parse('{"dirs": ["../python_provider"], "paths": ["../../bindings/python/guatiao/src", ' // &
                         '"../greeter_kind/python"]}', GUATIAO_FORMAT_JSON, config)
      call host%add_source_file(bridge, config, st, msg)
      if (st == GUATIAO_OK) then
        have(3) = .true.
      else if (len(env('GUATIAO_PY_BRIDGE')) > 0) then
        call check(.false., 'the bridge named by GUATIAO_PY_BRIDGE maps: ' // msg)
      else
        call skip('the bridge at ' // bridge // ' does not map here: ' // msg)
      end if
    end if
    call host%refresh(changed)
    list = changed%key('refused')
    call check(list%length() == 0, 'one refresh refuses nothing')
    list = changed%key('loaded')
    text = ''
    do k = 1, list%length()
      item = list%at(k, 'key')
      text = text // ' ' // item%string_or('?')
    end do
    call check(index(text, ' derived_greeter') > 0, 'the refresh loads the Rust greeters')
    if (have(2)) call check(index(text, ' c_greeter_library') > 0, 'and the C one')
    if (have(3)) call check(index(text, ' py_greeter') > 0, 'and the Python ones')
    if (have(4)) call check(index(text, 'go_greeter') > 0, 'and the Go ones')
    call guatiao_register_library(host, fortran_greeter_library())
    have(5) = .true.
  end subroutine

  subroutine a_fortran_host_calls_it(i)
    integer, intent(in) :: i
    type(greeter_remote) :: g
    type(guatiao_value) :: answer, config, handed, name
    type(guatiao_error), allocatable :: error
    class(listener), allocatable :: given
    class(conversation), allocatable :: chat
    type(ear) :: lent, told
    character(:), allocatable :: text, said
    character(:), allocatable :: who, greeting
    integer(int8) :: buf(64)
    integer(int64) :: n
    logical :: usable
    integer :: st
    who = trim(WHICH(i))
    greeting = trim(GREETINGS(i))
    call greeter_bind(host, trim(KEYS(i)), g, st)
    call check(st == GUATIAO_OK, who // ': bound as a greeter')
    if (st /= GUATIAO_OK) return

    call g%greet('ana', answer, error)
    call check(.not. allocated(error), who // ': greet')
    call guatiao_emit(answer, GUATIAO_FORMAT_JSON, text)
    call check_text(text, '{"greeting":"' // greeting // '"}', who // ': the greeting')

    call g%greet('', answer, error)
    call check(allocated(error), who // ': nobody is refused')
    if (allocated(error)) then
      call check(error%status == GUATIAO_ERR_BAD_VALUE .and. index(error%message, 'nobody to greet') > 0, &
                 who // ': with BAD_VALUE and its words')
    end if

    call g%greet('panic', answer, error)
    if (who == 'c') then
      call check(.not. allocated(error), who // ': greets anyone')
    else
      call check(allocated(error), who // ': a provider bug is refused')
      if (allocated(error)) call check(error%status == GUATIAO_ERR_INTERNAL, who // ': as INTERNAL')
    end if

    call guatiao_parse('{"name": "bo"}', GUATIAO_FORMAT_JSON, config)
    call g%check(config, usable, error)
    call check(.not. allocated(error) .and. usable, who // ': check')
    name = config%key('name')
    call check_text(name%string_or(''), 'bo', who // ': the lent map keeps its name')
    call check(config%length() == 2, who // ': and gains an entry')

    call guatiao_parse('{"name": "ana"}', GUATIAO_FORMAT_JSON, handed)
    call g%greet_from(handed, answer, error)
    call check(.not. allocated(error), who // ': greet_from')
    call guatiao_emit(answer, GUATIAO_FORMAT_JSON, text)
    call check_text(text, '{"greeting":"' // greeting // '"}', who // ': greets from the map handed over')
    call check(handed%is_absent(), who // ': the map handed over is gone from this side')

    lent = new_ear()
    call g%greet_into('ana', lent, error)
    call check(.not. allocated(error), who // ': greet_into')
    call check(size(lent%state%heard) == 1, who // ': the lent listener heard once')
    if (size(lent%state%heard) == 1) call check_text(lent%state%heard(1)%text, greeting, who // ': what it heard')
    call check(.not. lent%state%closed, who // ': a lent listener is not closed')

    if (g%has('shout')) then
      call g%shout('ana', said, error)
      call check_text(said, upper_of(greeting), who // ': shout')
    else
      call g%shout('ana', said, error)
      call check(allocated(error), who // ': no shout in its table')
    end if

    if (g%has('start')) then
      told = new_ear()
      allocate(given, source=told)
      call g%start(given, chat, error)
      call check(.not. allocated(error) .and. allocated(chat), who // ': start')
      call check(.not. allocated(given), who // ': the listener was handed over')
      if (allocated(chat)) then
        call chat%say('hi', error)
        call chat%say('again', error)
        call chat%say('', error)
        call check(allocated(error), who // ': nothing to say is refused')
        if (allocated(error)) call check(error%status == GUATIAO_ERR_BAD_VALUE, who // ': as BAD_VALUE')
        buf = 0
        call chat%read(buf, n, error)
        call check(n == 8, who // ': read 8 bytes')
        text = transfer(buf(1:max(n, 1_int64)), repeat(' ', int(max(n, 1_int64))))
        call check_text(text(1:int(n)), 'hi' // new_line('a') // 'again', who // ': the transcript')
        call chat%turns(n, error)
        call check(n == 2, who // ': two turns')
        call check(size(told%state%heard) == 2, who // ': the listener heard both')
        call chat%close()
        call check(told%state%closed, who // ': closed with the conversation')
        deallocate(chat)
      end if
    else
      call check(who == 'c', who // ': every greeter but the C one holds conversations')
    end if
  end subroutine

  subroutine the_rust_proxy_calls_it(i)
    integer, intent(in) :: i
    abstract interface
      function caller_i(h, key, name, out) bind(c) result(status)
        import :: c_ptr, gt_str, c_int32_t
        type(c_ptr), value :: h
        type(gt_str), value :: key, name
        type(c_ptr), value :: out
        integer(c_int32_t) :: status
      end function
    end interface
    procedure(caller_i), pointer :: run
    type(c_ptr) :: lib
    type(gt_value), pointer :: node
    type(guatiao_value) :: report, item
    character(:), allocatable, target :: key, name
    character(:), allocatable :: who, greeting, text
    integer(c_int32_t) :: st
    type(gt_str) :: k, n
    who = trim(WHICH(i))
    greeting = trim(GREETINGS(i))
    lib = gt_dl_open(library_file(library_dir(), 'greeter_caller'))
    call check(c_associated(lib), who // ': the Rust caller opens')
    if (.not. c_associated(lib)) return
    call c_f_procpointer(gt_dl_sym(lib, 'greeter_caller_run'), run)
    key = trim(KEYS(i))
    name = 'ana'
    k%ptr = c_loc(key)
    k%len = len(key, kind=c_size_t)
    n%ptr = c_loc(name)
    n%len = len(name, kind=c_size_t)
    allocate(node)
    st = run(host%host(), k, n, c_loc(node))
    report = guatiao_value_from_raw(c_loc(node))
    deallocate(node)
    call check(st == GUATIAO_OK, who // ': the Rust proxy ran')
    item = report%key('greet')
    call check_text(item%string_or(''), greeting, who // ' through Rust: greet')
    item = report%at('refusal', 'status')
    call check(item%int_or(0_int64) == GUATIAO_ERR_BAD_VALUE, who // ' through Rust: the refusal')
    item = report%key('failure')
    if (who == 'c') then
      call check(item%int_or(-1_int64) == 0, who // ' through Rust: greets anyone')
    else
      call check(item%int_or(0_int64) == GUATIAO_ERR_INTERNAL, who // ' through Rust: the failure')
    end if
    item = report%key('shout')
    call check_text(item%string_or(''), upper_of(greeting), who // ' through Rust: shout')
    item = report%key('checked')
    call check(item%bool_or(.false.), who // ' through Rust: check')
    item = report%key('filled')
    call check(item%length() == 2, who // ' through Rust: the lent map filled')
    item = report%key('greet_from')
    call check_text(item%string_or(''), greeting, who // ' through Rust: greet_from')
    item = report%at('heard', 1)
    call check_text(item%string_or(''), greeting, who // ' through Rust: greet_into')
    item = report%key('conversation')
    call guatiao_emit(item, GUATIAO_FORMAT_JSON, text)
    if (who == 'c') then
      call check(index(text, 'this greeter holds no conversations') > 0, who // ' through Rust: the default start')
    else
      call check(index(text, '"turns":2') > 0, who // ' through Rust: two turns ' // text)
      call check(index(text, '"dropped":true') > 0, who // ' through Rust: the listener dropped ' // text)
    end if
  end subroutine

  subroutine a_table_that_is_not_the_kinds_is_refused()
    type(c_ptr) :: table
    integer(c_size_t) :: size
    type(guatiao_remote) :: r
    type(counter_remote) :: c
    integer :: st
    call host%provider_table('fortran_greeter_hello', 'counter', table, size)
    call guatiao_bind_table(table, size, c_null_ptr, greeter_kind(), r, st)
    call check(st == GUATIAO_STAT_FLOOR_MISMATCH, 'a counter table is no greeter')
    call counter_bind(host, 'derived_greeter_shouter', c, st)
    call check(st /= GUATIAO_OK, 'a provider with no counter table is refused')
  end subroutine

  subroutine a_configured_provider_builds_instances()
    type(guatiao_instance) :: inst
    type(greeter_remote) :: g
    type(guatiao_value) :: config, answer, schema
    type(guatiao_error), allocatable :: error
    character(:), allocatable :: text, msg
    integer :: st
    schema = host%provider_config('fortran_greeter_shouter')
    call check(schema%tag() == GUATIAO_TAG_MAP, 'the shouter declares its configuration')
    call guatiao_parse('{}', GUATIAO_FORMAT_JSON, config)
    call host%create('fortran_greeter_shouter', config, inst, st, msg)
    call check(st == GUATIAO_ERR_BAD_VALUE, 'an empty configuration is refused')
    call check(index(msg, 'prefix') > 0, 'naming the field')
    call guatiao_parse('{"prefix": "hey"}', GUATIAO_FORMAT_JSON, config)
    call host%create('fortran_greeter_shouter', config, inst)
    call greeter_bind_instance(inst, g)
    call g%greet('ana', answer, error)
    call guatiao_emit(answer, GUATIAO_FORMAT_JSON, text)
    call check_text(text, '{"greeting":"hey ANA"}', 'an instance built from {"prefix": "hey"}')
    call inst%close()
    call g%greet('ana', answer, error)
    call check(allocated(error), 'a closed instance answers nothing')
  end subroutine

  subroutine native_threads_call_the_fortran_greeter()
    abstract interface
      function on_threads_i(f, fr, ctx, n, calls, tids) bind(c) result(status)
        import :: c_funptr, c_ptr, c_int, c_int32_t
        type(c_funptr), value :: f, fr
        type(c_ptr), value :: ctx
        integer(c_int), value :: n, calls
        type(c_ptr), value :: tids
        integer(c_int32_t) :: status
      end function
    end interface
    procedure(on_threads_i), pointer :: on_threads
    character(:), allocatable :: built
    type(c_ptr) :: lib, table
    type(c_funptr), pointer :: slots(:)
    integer(c_size_t) :: size
    integer(c_int64_t), target :: tids(8)
    integer(c_int32_t) :: st
    built = build_c('fixtures/threads.c', 'build/threads', 'threads_fixture')
    if (len(built) == 0) then
      call skip('no C compiler: no threads to call from')
      return
    end if
    lib = gt_dl_open(built)
    call check(c_associated(lib), 'the thread fixture opens')
    if (.not. c_associated(lib)) return
    call c_f_procpointer(gt_dl_sym(lib, 'threads_call_on_threads'), on_threads)
    call host%provider_table('fortran_greeter_hello', 'greeter', table, size)
    call c_f_pointer(table, slots, [size / 8])
    tids = 0
    st = on_threads(slots(2), guatiao_value_free_ptr(), host%provider_ctx('fortran_greeter_hello'), 8, 500, &
                    c_loc(tids))
    call check(st == GUATIAO_OK, '8 native threads x 500 greetings, all answered')
    call check(all(tids /= 0), 'each thread ran')
  end subroutine

  ! guatiao_value_free, as the fixture frees each answer with it.
  function guatiao_value_free_ptr() result(f)
    use guatiao_c, only: gt_value_free
    type(c_funptr) :: f
    f = c_funloc(gt_value_free)
  end function

end program test_greeters
