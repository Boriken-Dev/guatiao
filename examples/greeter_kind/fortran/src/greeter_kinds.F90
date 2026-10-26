! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> The greeter kinds in Fortran, written once from greeter_kind.h: what a
!> host and a library both compile against.
!>
!> Each kind is an abstract type (a deferred procedure per required slot,
!> an overridable one per optional slot), a remote type calling another
!> side's table, a description (`<kind>_kind()`), a serving function
!> naming the optional slots a provider overrides, and a bind(c)
!> procedure per slot serving a table to any caller.
module greeter_kinds
  use, intrinsic :: iso_c_binding
  use, intrinsic :: iso_fortran_env, only: int8, int64
  use guatiao
  use guatiao_c, only: gt_str, gt_owned, gt_object, gt_bytes_mut, gt_provider_error, gt_kind_header, gt_value
  use guatiao_host, only: guatiao_registry, guatiao_instance
  use guatiao_kinds
  implicit none
  private

  public :: listener, conversation, greeter, counter, wide
  public :: listener_remote, conversation_remote, greeter_remote, counter_remote, wide_remote
  public :: listener_kind, conversation_kind, greeter_kind, counter_kind, wide_kind
  public :: greeter_serving, counter_serving, wide_serving
  public :: greeter_bind, greeter_bind_instance, counter_bind, wide_bind
  public :: GREETER_FLOOR_HASH, COUNTER_FLOOR_HASH, CONVERSATION_FLOOR_HASH, LISTENER_FLOOR_HASH, WIDE_FLOOR_HASH

  ! The headers' *_FLOOR_HASH, as the uint32 bits an int32 holds.
  integer(c_int32_t), parameter :: GREETER_FLOOR_HASH = -1599110805_c_int32_t       ! 2695856491
  integer(c_int32_t), parameter :: COUNTER_FLOOR_HASH = 1227528937_c_int32_t
  integer(c_int32_t), parameter :: CONVERSATION_FLOOR_HASH = -1525342793_c_int32_t  ! 2769624503
  integer(c_int32_t), parameter :: LISTENER_FLOOR_HASH = 1867921573_c_int32_t
  integer(c_int32_t), parameter :: WIDE_FLOOR_HASH = -621277612_c_int32_t           ! 3673689684

  ! ---- the tables, as greeter_kind.h lays them out ---------------------

  type, bind(c) :: greeter_vtable
    type(gt_kind_header) :: header
    type(c_funptr) :: greet = c_null_funptr
    type(c_funptr) :: shout = c_null_funptr
    type(c_funptr) :: start = c_null_funptr
    type(c_funptr) :: greet_into = c_null_funptr
    type(c_funptr) :: greet_from = c_null_funptr
    type(c_funptr) :: check = c_null_funptr
  end type

  type, bind(c) :: counter_vtable
    type(gt_kind_header) :: header
    type(c_funptr) :: count = c_null_funptr
  end type

  type, bind(c) :: wide_vtable
    type(gt_kind_header) :: header
    type(c_funptr) :: echo_i64 = c_null_funptr
    type(c_funptr) :: echo_u64 = c_null_funptr
  end type

  type, bind(c) :: conversation_vtable
    type(gt_kind_header) :: header
    type(c_funptr) :: destroy = c_null_funptr
    type(c_funptr) :: say = c_null_funptr
    type(c_funptr) :: read = c_null_funptr
    type(c_funptr) :: turns = c_null_funptr
  end type

  type, bind(c) :: listener_vtable
    type(gt_kind_header) :: header
    type(c_funptr) :: destroy = c_null_funptr
    type(c_funptr) :: heard = c_null_funptr
  end type

  ! ---- the kinds -------------------------------------------------------

  !> An object kind: what a conversation tells, as it is said.
  type, abstract, extends(guatiao_impl) :: listener
  contains
    procedure(listener_heard), deferred :: heard
  end type

  !> An object kind: one conversation a greeter started.
  type, abstract, extends(guatiao_impl) :: conversation
  contains
    procedure(conversation_say), deferred :: say
    procedure(conversation_read), deferred :: read
    procedure(conversation_turns), deferred :: turns
  end type

  !> The greeter kind: `greet` required; the rest optional (a provider
  !> declares the ones it overrides in greeter_serving).
  type, abstract, extends(guatiao_impl) :: greeter
  contains
    procedure(greeter_greet), deferred :: greet
    procedure :: shout => greeter_shout
    procedure :: start => greeter_start
    procedure :: greet_into => greeter_greet_into
    procedure :: greet_from => greeter_greet_from
    procedure :: check => greeter_check
  end type

  !> The counter kind.
  type, abstract, extends(guatiao_impl) :: counter
  contains
    procedure(counter_count), deferred :: count
  end type

  !> The wide kind: answers the 64-bit integer it is given. Fortran has no
  !> unsigned integer, so `echo_u64` carries its `uint64` as the `int64`
  !> holding the same bits: a value past 2^63 - 1 is negative, and
  !> 2^64 - 1 is -1.
  type, abstract, extends(guatiao_impl) :: wide
  contains
    procedure(wide_echo), deferred :: echo_i64
    procedure(wide_echo), deferred :: echo_u64
  end type

  abstract interface
    subroutine listener_heard(self, what, error)
      import :: listener, guatiao_error
      class(listener), intent(inout) :: self
      character(*), intent(in) :: what
      type(guatiao_error), allocatable, intent(out) :: error
    end subroutine
    subroutine conversation_say(self, what, error)
      import :: conversation, guatiao_error
      class(conversation), intent(inout) :: self
      character(*), intent(in) :: what
      type(guatiao_error), allocatable, intent(out) :: error
    end subroutine
    !> Writes into `dst` (the caller's memory, for the call) and answers
    !> how many bytes it wrote.
    subroutine conversation_read(self, dst, answer, error)
      import :: conversation, guatiao_error, int8, int64
      class(conversation), intent(inout) :: self
      integer(int8), intent(inout) :: dst(:)
      integer(int64), intent(out) :: answer
      type(guatiao_error), allocatable, intent(out) :: error
    end subroutine
    subroutine conversation_turns(self, answer, error)
      import :: conversation, guatiao_error, int64
      class(conversation), intent(inout) :: self
      integer(int64), intent(out) :: answer
      type(guatiao_error), allocatable, intent(out) :: error
    end subroutine
    !> `answer` holds an empty map to fill, in the caller's allocator.
    subroutine greeter_greet(self, name, answer, error)
      import :: greeter, guatiao_value, guatiao_error
      class(greeter), intent(inout) :: self
      character(*), intent(in) :: name
      type(guatiao_value), intent(inout) :: answer
      type(guatiao_error), allocatable, intent(out) :: error
    end subroutine
    subroutine counter_count(self, answer, error)
      import :: counter, guatiao_error, int64
      class(counter), intent(inout) :: self
      integer(int64), intent(out) :: answer
      type(guatiao_error), allocatable, intent(out) :: error
    end subroutine
    subroutine wide_echo(self, x, answer, error)
      import :: wide, guatiao_error, int64
      class(wide), intent(inout) :: self
      integer(int64), intent(in) :: x
      integer(int64), intent(out) :: answer
      type(guatiao_error), allocatable, intent(out) :: error
    end subroutine
  end interface

  ! ---- the remotes -----------------------------------------------------

  !> Another side's listener: owned (handed over, destroyed when the
  !> last copy goes or on close) or lent for one call.
  type, extends(listener) :: listener_remote
    type(guatiao_remote) :: r
  contains
    procedure :: heard => listener_remote_heard
    procedure :: close => listener_remote_close
  end type

  !> Another side's conversation, owned by this side.
  type, extends(conversation) :: conversation_remote
    type(guatiao_remote) :: r
  contains
    procedure :: say => conversation_remote_say
    procedure :: read => conversation_remote_read
    procedure :: turns => conversation_remote_turns
    procedure :: close => conversation_remote_close
  end type

  !> A greeter another side serves. A slot its table lacks answers
  !> GUATIAO_ERR_UNSUPPORTED (`has` says which it has).
  type, extends(greeter) :: greeter_remote
    type(guatiao_remote) :: r
  contains
    procedure :: greet => greeter_remote_greet
    procedure :: shout => greeter_remote_shout
    procedure :: start => greeter_remote_start
    procedure :: greet_into => greeter_remote_greet_into
    procedure :: greet_from => greeter_remote_greet_from
    procedure :: check => greeter_remote_check
    procedure :: has => greeter_remote_has
  end type

  type, extends(counter) :: counter_remote
    type(guatiao_remote) :: r
  contains
    procedure :: count => counter_remote_count
  end type

  type, extends(wide) :: wide_remote
    type(guatiao_remote) :: r
  contains
    procedure :: echo_i64 => wide_remote_echo_i64
    procedure :: echo_u64 => wide_remote_echo_u64
  end type

  ! ---- the C slots -----------------------------------------------------

  abstract interface
    function echo_c(ctx, x, out, err) bind(c) result(status)
      import :: c_ptr, c_int64_t, c_int32_t
      type(c_ptr), value :: ctx
      integer(c_int64_t), value :: x
      type(c_ptr), value :: out, err
      integer(c_int32_t) :: status
    end function
    function greet_c(ctx, name, out, err) bind(c) result(status)
      import :: c_ptr, gt_str, c_int32_t
      type(c_ptr), value :: ctx
      type(gt_str), value :: name
      type(c_ptr), value :: out, err
      integer(c_int32_t) :: status
    end function
    function shout_c(ctx, name, out) bind(c) result(status)
      import :: c_ptr, gt_str, c_int32_t
      type(c_ptr), value :: ctx
      type(gt_str), value :: name
      type(c_ptr), value :: out
      integer(c_int32_t) :: status
    end function
    ! See start_slot: under flang on AArch64 and Windows the object
    ! crosses as the address of a copy, as the ABI passes it.
#if defined(__flang__) && (defined(__aarch64__) || defined(_WIN32))
    function start_c(ctx, listener, out, err) bind(c) result(status)
      import :: c_ptr, c_int32_t
      type(c_ptr), value :: ctx
      type(c_ptr), value :: listener
      type(c_ptr), value :: out, err
      integer(c_int32_t) :: status
    end function
#else
    function start_c(ctx, listener, out, err) bind(c) result(status)
      import :: c_ptr, gt_object, c_int32_t
      type(c_ptr), value :: ctx
      type(gt_object), value :: listener
      type(c_ptr), value :: out, err
      integer(c_int32_t) :: status
    end function
#endif
    function greet_into_c(ctx, name, listener, err) bind(c) result(status)
      import :: c_ptr, gt_str, c_int32_t
      type(c_ptr), value :: ctx
      type(gt_str), value :: name
      type(c_ptr), value :: listener, err
      integer(c_int32_t) :: status
    end function
    function map_map_c(ctx, config, out, err) bind(c) result(status)
      import :: c_ptr, c_int32_t
      type(c_ptr), value :: ctx, config, out, err
      integer(c_int32_t) :: status
    end function
    function int_out_c(ctx, out) bind(c) result(status)
      import :: c_ptr, c_int32_t
      type(c_ptr), value :: ctx, out
      integer(c_int32_t) :: status
    end function
    function say_c(ctx, what, err) bind(c) result(status)
      import :: c_ptr, gt_str, c_int32_t
      type(c_ptr), value :: ctx
      type(gt_str), value :: what
      type(c_ptr), value :: err
      integer(c_int32_t) :: status
    end function
    function read_c(ctx, dst, out) bind(c) result(status)
      import :: c_ptr, gt_bytes_mut, c_int32_t
      type(c_ptr), value :: ctx
      type(gt_bytes_mut), value :: dst
      type(c_ptr), value :: out
      integer(c_int32_t) :: status
    end function
    function heard_c(ctx, what) bind(c) result(status)
      import :: c_ptr, gt_str, c_int32_t
      type(c_ptr), value :: ctx
      type(gt_str), value :: what
      integer(c_int32_t) :: status
    end function
  end interface

  ! The tables this module serves: a greeter's per set of optional
  ! slots (the bits of shout, start, greet_into, greet_from, check), one
  ! for every counter and object.
  type(greeter_vtable), target, save :: greeter_tables(0:31)
  type(counter_vtable), target, save :: counter_table
  type(wide_vtable), target, save :: wide_table
  type(conversation_vtable), target, save :: conversation_table
  type(listener_vtable), target, save :: listener_table

contains

  ! ---- descriptions ----------------------------------------------------

  recursive function listener_kind() result(k)
    type(guatiao_kind) :: k
    k%name = 'listener'
    k%floor_hash = LISTENER_FLOOR_HASH
    k%size = c_sizeof(listener_table)
    k%floor_size = k%size
    k%object = .true.
    listener_table%header%struct_size = int(k%size, c_int32_t)
    listener_table%header%floor_hash = LISTENER_FLOOR_HASH
    listener_table%destroy = c_funloc(destroy_slot)
    listener_table%heard = c_funloc(heard_slot)
    k%object_table = c_loc(listener_table)
    k%accepts => is_listener
  end function

  recursive function conversation_kind() result(k)
    type(guatiao_kind) :: k
    k%name = 'conversation'
    k%floor_hash = CONVERSATION_FLOOR_HASH
    k%size = c_sizeof(conversation_table)
    k%floor_size = k%size
    k%object = .true.
    conversation_table%header%struct_size = int(k%size, c_int32_t)
    conversation_table%header%floor_hash = CONVERSATION_FLOOR_HASH
    conversation_table%destroy = c_funloc(destroy_slot)
    conversation_table%say = c_funloc(say_slot)
    conversation_table%read = c_funloc(read_slot)
    conversation_table%turns = c_funloc(turns_slot)
    k%object_table = c_loc(conversation_table)
    k%accepts => is_conversation
  end function

  recursive function greeter_kind() result(k)
    type(guatiao_kind) :: k
    type(greeter_vtable) :: probe
    k%name = 'greeter'
    k%floor_hash = GREETER_FLOOR_HASH
    k%size = c_sizeof(probe)
    k%floor_size = c_sizeof(probe%header) + c_sizeof(probe%greet)
    k%accepts => is_greeter
  end function

  recursive function counter_kind() result(k)
    type(guatiao_kind) :: k
    k%name = 'counter'
    k%floor_hash = COUNTER_FLOOR_HASH
    k%size = c_sizeof(counter_table)
    k%floor_size = k%size
    k%accepts => is_counter
  end function

  recursive function wide_kind() result(k)
    type(guatiao_kind) :: k
    k%name = 'wide'
    k%floor_hash = WIDE_FLOOR_HASH
    k%size = c_sizeof(wide_table)
    k%floor_size = k%size
    k%accepts => is_wide
  end function

  recursive logical function is_listener(impl)
    class(guatiao_impl), intent(in) :: impl
    select type (impl)
    class is (listener)
      is_listener = .true.
    class default
      is_listener = .false.
    end select
  end function

  recursive logical function is_conversation(impl)
    class(guatiao_impl), intent(in) :: impl
    select type (impl)
    class is (conversation)
      is_conversation = .true.
    class default
      is_conversation = .false.
    end select
  end function

  recursive logical function is_greeter(impl)
    class(guatiao_impl), intent(in) :: impl
    select type (impl)
    class is (greeter)
      is_greeter = .true.
    class default
      is_greeter = .false.
    end select
  end function

  recursive logical function is_counter(impl)
    class(guatiao_impl), intent(in) :: impl
    select type (impl)
    class is (counter)
      is_counter = .true.
    class default
      is_counter = .false.
    end select
  end function

  recursive logical function is_wide(impl)
    class(guatiao_impl), intent(in) :: impl
    select type (impl)
    class is (wide)
      is_wide = .true.
    class default
      is_wide = .false.
    end select
  end function

  !> `impl` serving the greeter kind through a table with `greet` and
  !> each optional slot it says it overrides; with no `impl`, the table a
  !> configured provider's instances use.
  recursive function greeter_serving(impl, shout, start, greet_into, greet_from, check) result(s)
    class(greeter), intent(in), target, optional :: impl
    logical, intent(in), optional :: shout, start, greet_into, greet_from, check
    type(guatiao_serving) :: s
    integer :: bits
    type(greeter_vtable), pointer :: t
    bits = 0
    if (on(shout)) bits = ior(bits, 1)
    if (on(start)) bits = ior(bits, 2)
    if (on(greet_into)) bits = ior(bits, 4)
    if (on(greet_from)) bits = ior(bits, 8)
    if (on(check)) bits = ior(bits, 16)
    t => greeter_tables(bits)
    t%header%struct_size = int(storage_size(t) / 8, c_int32_t)
    t%header%floor_hash = GREETER_FLOOR_HASH
    t%greet = c_funloc(greet_slot)
    if (iand(bits, 1) /= 0) t%shout = c_funloc(shout_slot)
    if (iand(bits, 2) /= 0) t%start = c_funloc(start_slot)
    if (iand(bits, 4) /= 0) t%greet_into = c_funloc(greet_into_slot)
    if (iand(bits, 8) /= 0) t%greet_from = c_funloc(greet_from_slot)
    if (iand(bits, 16) /= 0) t%check = c_funloc(check_slot)
    s%kind = greeter_kind()
    s%table = c_loc(t)
    if (present(impl)) s%impl => impl
  contains
    recursive logical function on(flag)
      logical, intent(in), optional :: flag
      on = .false.
      if (present(flag)) on = flag
    end function
  end function

  recursive function counter_serving(impl) result(s)
    class(counter), intent(in), target, optional :: impl
    type(guatiao_serving) :: s
    counter_table%header%struct_size = int(c_sizeof(counter_table), c_int32_t)
    counter_table%header%floor_hash = COUNTER_FLOOR_HASH
    counter_table%count = c_funloc(count_slot)
    s%kind = counter_kind()
    s%table = c_loc(counter_table)
    if (present(impl)) s%impl => impl
  end function

  recursive function wide_serving(impl) result(s)
    class(wide), intent(in), target, optional :: impl
    type(guatiao_serving) :: s
    wide_table%header%struct_size = int(c_sizeof(wide_table), c_int32_t)
    wide_table%header%floor_hash = WIDE_FLOOR_HASH
    wide_table%echo_i64 = c_funloc(echo_i64_slot)
    wide_table%echo_u64 = c_funloc(echo_u64_slot)
    s%kind = wide_kind()
    s%table = c_loc(wide_table)
    if (present(impl)) s%impl => impl
  end function

  !> The provider under `key` as a greeter.
  recursive subroutine greeter_bind(reg, key, g, stat, errmsg)
    type(guatiao_registry), intent(in) :: reg
    character(*), intent(in) :: key
    type(greeter_remote), intent(inout) :: g
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    call guatiao_bind(reg, key, greeter_kind(), g%r, stat, errmsg)
  end subroutine

  !> An instance as a greeter.
  recursive subroutine greeter_bind_instance(inst, g, stat, errmsg)
    type(guatiao_instance), intent(in) :: inst
    type(greeter_remote), intent(inout) :: g
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    call guatiao_bind_instance(inst, greeter_kind(), g%r, stat, errmsg)
  end subroutine

  recursive subroutine counter_bind(reg, key, c, stat, errmsg)
    type(guatiao_registry), intent(in) :: reg
    character(*), intent(in) :: key
    type(counter_remote), intent(inout) :: c
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    call guatiao_bind(reg, key, counter_kind(), c%r, stat, errmsg)
  end subroutine

  recursive subroutine wide_bind(reg, key, w, stat, errmsg)
    type(guatiao_registry), intent(in) :: reg
    character(*), intent(in) :: key
    type(wide_remote), intent(inout) :: w
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    call guatiao_bind(reg, key, wide_kind(), w%r, stat, errmsg)
  end subroutine

  ! ---- optional slots a provider does not override --------------------

  recursive subroutine greeter_shout(self, name, answer, error)
    class(greeter), intent(inout) :: self
    character(*), intent(in) :: name
    character(:), allocatable, intent(out) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    answer = ''
    call guatiao_fail(error, GUATIAO_ERR_UNSUPPORTED, 'no shout here')
  end subroutine

  recursive subroutine greeter_start(self, listener_in, answer, error)
    class(greeter), intent(inout) :: self
    class(listener), allocatable, intent(inout) :: listener_in
    class(conversation), allocatable, intent(out) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    call guatiao_fail(error, GUATIAO_ERR_UNSUPPORTED, 'no conversations here')
  end subroutine

  recursive subroutine greeter_greet_into(self, name, listener_in, error)
    class(greeter), intent(inout) :: self
    character(*), intent(in) :: name
    class(listener), intent(inout), target :: listener_in
    type(guatiao_error), allocatable, intent(out) :: error
    call guatiao_fail(error, GUATIAO_ERR_UNSUPPORTED, 'no greet_into here')
  end subroutine

  !> `config` is handed over: the method owns it.
  recursive subroutine greeter_greet_from(self, config, answer, error)
    class(greeter), intent(inout) :: self
    type(guatiao_value), intent(inout) :: config
    type(guatiao_value), intent(inout) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    call guatiao_fail(error, GUATIAO_ERR_UNSUPPORTED, 'no greet_from here')
  end subroutine

  !> `config` is lent for the call: change it, never keep it.
  recursive subroutine greeter_check(self, config, answer, error)
    class(greeter), intent(inout) :: self
    type(guatiao_value), intent(inout) :: config
    logical, intent(out) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    answer = .false.
    call guatiao_fail(error, GUATIAO_ERR_UNSUPPORTED, 'no check here')
  end subroutine

  ! ---- serving: a bind(c) procedure per slot ---------------------------

  recursive function greet_slot(ctx, name, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx
    type(gt_str), value :: name
    type(c_ptr), value :: out, err
    integer(c_int32_t) :: status
    class(guatiao_impl), pointer :: impl
    character(:), allocatable :: text
    type(guatiao_value) :: carrier, answer
    type(guatiao_error), allocatable :: error
    integer :: st
    impl => guatiao_impl_at(ctx, 'greeter')
    status = GUATIAO_ERR_GONE
    if (.not. associated(impl)) return
    call guatiao_text_in(name, text)
    carrier = guatiao_map_carrier(out)
    answer = carrier
    select type (impl)
    class is (greeter)
      call impl%greet(text, answer, error)
    end select
    status = guatiao_failed(error, err)
    call guatiao_settle_map(carrier, answer, out, st, failed=allocated(error))
    if (status == GUATIAO_OK) status = int(st, c_int32_t)
  end function

  recursive function shout_slot(ctx, name, out) bind(c) result(status)
    type(c_ptr), value :: ctx
    type(gt_str), value :: name
    type(c_ptr), value :: out
    integer(c_int32_t) :: status
    class(guatiao_impl), pointer :: impl
    character(:), allocatable :: text, answer
    type(guatiao_error), allocatable :: error
    integer :: st
    impl => guatiao_impl_at(ctx, 'greeter')
    status = GUATIAO_ERR_GONE
    if (.not. associated(impl)) return
    call guatiao_text_in(name, text)
    select type (impl)
    class is (greeter)
      call impl%shout(text, answer, error)
    end select
    status = guatiao_warn(error, 'shout')
    if (status /= GUATIAO_OK) return
    if (.not. allocated(answer)) answer = ''
    call guatiao_string_out(answer, out, st)
    status = int(st, c_int32_t)
  end function

  ! A guatiao_object is 24 bytes, passed by value: flang 22 gets such an
  ! argument wrong in a bind(c) procedure, and in a call through a
  ! procedure pointer, on AArch64 and Windows, where the ABI passes the
  ! address of the caller's copy; that address is what crosses there.
#if defined(__flang__) && (defined(__aarch64__) || defined(_WIN32))
  recursive function start_slot(ctx, obj_at, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx
    type(c_ptr), value :: obj_at
    type(c_ptr), value :: out, err
    type(gt_object), pointer :: obj
#else
  recursive function start_slot(ctx, obj, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx
    type(gt_object), value :: obj
    type(c_ptr), value :: out, err
#endif
    integer(c_int32_t) :: status
    class(guatiao_impl), pointer :: impl
    class(listener), allocatable :: heard_by
    class(conversation), allocatable :: answer
    class(guatiao_impl), allocatable :: moved
    type(guatiao_error), allocatable :: error
    type(gt_object), pointer :: slot
    integer :: st
#if defined(__flang__) && (defined(__aarch64__) || defined(_WIN32))
    call c_f_pointer(obj_at, obj)
#endif
    impl => guatiao_impl_at(ctx, 'greeter')
    allocate(listener_remote :: heard_by)
    select type (heard_by)
    type is (listener_remote)
      call guatiao_take_object(obj, listener_kind(), heard_by%r, st)
    end select
    status = GUATIAO_ERR_GONE
    if (.not. associated(impl)) return
    status = int(st, c_int32_t)
    if (st /= GUATIAO_OK) return
    select type (impl)
    class is (greeter)
      call impl%start(heard_by, answer, error)
    end select
    if (allocated(heard_by)) deallocate(heard_by)
    status = guatiao_failed(error, err)
    if (status /= GUATIAO_OK) return
    call c_f_pointer(out, slot)
    if (.not. allocated(answer)) then
      call guatiao_fail(error, GUATIAO_ERR_NULL, 'start answered no conversation')
      status = guatiao_failed(error, err)
      return
    end if
    select type (answer)
    type is (conversation_remote)
      call answer%r%give(slot, st)
      status = int(st, c_int32_t)
      return
    end select
    call move_alloc(answer, moved)
    call guatiao_serve_object(moved, conversation_kind(), slot)
  end function

  recursive function greet_into_slot(ctx, name, lent, err) bind(c) result(status)
    type(c_ptr), value :: ctx
    type(gt_str), value :: name
    type(c_ptr), value :: lent, err
    integer(c_int32_t) :: status
    class(guatiao_impl), pointer :: impl
    character(:), allocatable :: text
    type(listener_remote) :: heard_by
    type(guatiao_error), allocatable :: error
    integer :: st
    impl => guatiao_impl_at(ctx, 'greeter')
    status = GUATIAO_ERR_GONE
    if (.not. associated(impl)) return
    call guatiao_lend_object(lent, listener_kind(), heard_by%r, st)
    if (st /= GUATIAO_OK) then
      call guatiao_fail(error, st, 'no listener was lent')
      status = guatiao_failed(error, err)
      return
    end if
    call guatiao_text_in(name, text)
    select type (impl)
    class is (greeter)
      call impl%greet_into(text, heard_by, error)
    end select
    call guatiao_end_lent_object(heard_by%r)
    status = guatiao_failed(error, err)
  end function

  recursive function greet_from_slot(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    class(guatiao_impl), pointer :: impl
    type(guatiao_value) :: taken, carrier, answer
    type(guatiao_error), allocatable :: error
    integer :: st
    taken = guatiao_taken_map(config, st)
    if (st /= GUATIAO_OK) then
      call guatiao_fail(error, st, "the map handed over could not be moved into this side's allocator")
      status = guatiao_failed(error, err)
      return
    end if
    impl => guatiao_impl_at(ctx, 'greeter')
    status = GUATIAO_ERR_GONE
    if (.not. associated(impl)) return
    carrier = guatiao_map_carrier(out)
    answer = carrier
    select type (impl)
    class is (greeter)
      call impl%greet_from(taken, answer, error)
    end select
    status = guatiao_failed(error, err)
    call guatiao_settle_map(carrier, answer, out, st, failed=allocated(error))
    if (status == GUATIAO_OK) status = int(st, c_int32_t)
  end function

  recursive function check_slot(ctx, config, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx, config, out, err
    integer(c_int32_t) :: status
    class(guatiao_impl), pointer :: impl
    type(guatiao_value) :: lent
    logical :: usable
    integer(c_int8_t), pointer :: answer
    type(guatiao_error), allocatable :: error
    impl => guatiao_impl_at(ctx, 'greeter')
    status = GUATIAO_ERR_GONE
    if (.not. associated(impl)) return
    lent = guatiao_lent_map(config)
    usable = .false.
    select type (impl)
    class is (greeter)
      call impl%check(lent, usable, error)
    end select
    call guatiao_end_lent_map(lent, config)
    status = guatiao_failed(error, err)
    if (status == GUATIAO_OK .and. c_associated(out)) then
      call c_f_pointer(out, answer)
      answer = merge(GUATIAO_C_TRUE, GUATIAO_C_FALSE, usable)
    end if
  end function

  recursive function count_slot(ctx, out) bind(c) result(status)
    type(c_ptr), value :: ctx, out
    integer(c_int32_t) :: status
    class(guatiao_impl), pointer :: impl
    integer(int64) :: n
    integer(c_int64_t), pointer :: answer
    type(guatiao_error), allocatable :: error
    impl => guatiao_impl_at(ctx, 'counter')
    status = GUATIAO_ERR_GONE
    if (.not. associated(impl)) return
    n = 0
    select type (impl)
    class is (counter)
      call impl%count(n, error)
    end select
    status = guatiao_warn(error, 'count')
    if (status == GUATIAO_OK .and. c_associated(out)) then
      call c_f_pointer(out, answer)
      answer = n
    end if
  end function

  recursive function echo_i64_slot(ctx, x, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx
    integer(c_int64_t), value :: x
    type(c_ptr), value :: out, err
    integer(c_int32_t) :: status
    class(guatiao_impl), pointer :: impl
    integer(int64) :: y
    integer(c_int64_t), pointer :: answer
    type(guatiao_error), allocatable :: error
    impl => guatiao_impl_at(ctx, 'wide')
    status = GUATIAO_ERR_GONE
    if (.not. associated(impl)) return
    y = 0
    select type (impl)
    class is (wide)
      call impl%echo_i64(x, y, error)
    end select
    status = guatiao_failed(error, err)
    if (status == GUATIAO_OK .and. c_associated(out)) then
      call c_f_pointer(out, answer)
      answer = y
    end if
  end function

  recursive function echo_u64_slot(ctx, x, out, err) bind(c) result(status)
    type(c_ptr), value :: ctx
    integer(c_int64_t), value :: x
    type(c_ptr), value :: out, err
    integer(c_int32_t) :: status
    class(guatiao_impl), pointer :: impl
    integer(int64) :: y
    integer(c_int64_t), pointer :: answer
    type(guatiao_error), allocatable :: error
    impl => guatiao_impl_at(ctx, 'wide')
    status = GUATIAO_ERR_GONE
    if (.not. associated(impl)) return
    y = 0
    select type (impl)
    class is (wide)
      call impl%echo_u64(x, y, error)
    end select
    status = guatiao_failed(error, err)
    if (status == GUATIAO_OK .and. c_associated(out)) then
      call c_f_pointer(out, answer)
      answer = y
    end if
  end function

  ! An object's destroy: the holder lets it go.
  recursive subroutine destroy_slot(ctx) bind(c)
    type(c_ptr), value :: ctx
    call guatiao_destroy_served(ctx)
  end subroutine

  recursive function say_slot(ctx, what, err) bind(c) result(status)
    type(c_ptr), value :: ctx
    type(gt_str), value :: what
    type(c_ptr), value :: err
    integer(c_int32_t) :: status
    class(guatiao_impl), pointer :: impl
    character(:), allocatable :: text
    type(guatiao_error), allocatable :: error
    impl => guatiao_impl_at(ctx, 'conversation')
    status = GUATIAO_ERR_GONE
    if (.not. associated(impl)) return
    call guatiao_text_in(what, text)
    select type (impl)
    class is (conversation)
      call impl%say(text, error)
    end select
    status = guatiao_failed(error, err)
  end function

  recursive function read_slot(ctx, dst, out) bind(c) result(status)
    type(c_ptr), value :: ctx
    type(gt_bytes_mut), value :: dst
    type(c_ptr), value :: out
    integer(c_int32_t) :: status
    class(guatiao_impl), pointer :: impl
    integer(int8), pointer :: bytes(:)
    integer(int64) :: n
    integer(c_int64_t), pointer :: answer
    type(guatiao_error), allocatable :: error
    logical :: scratch
    impl => guatiao_impl_at(ctx, 'conversation')
    status = GUATIAO_ERR_GONE
    if (.not. associated(impl)) return
    call guatiao_bytes_in(dst, bytes)
    scratch = dst%len == 0 .or. .not. c_associated(dst%ptr)
    n = 0
    select type (impl)
    class is (conversation)
      call impl%read(bytes, n, error)
    end select
    if (scratch) deallocate(bytes)
    status = guatiao_warn(error, 'read')
    if (status == GUATIAO_OK .and. c_associated(out)) then
      call c_f_pointer(out, answer)
      answer = n
    end if
  end function

  recursive function turns_slot(ctx, out) bind(c) result(status)
    type(c_ptr), value :: ctx, out
    integer(c_int32_t) :: status
    class(guatiao_impl), pointer :: impl
    integer(int64) :: n
    integer(c_int64_t), pointer :: answer
    type(guatiao_error), allocatable :: error
    impl => guatiao_impl_at(ctx, 'conversation')
    status = GUATIAO_ERR_GONE
    if (.not. associated(impl)) return
    n = 0
    select type (impl)
    class is (conversation)
      call impl%turns(n, error)
    end select
    status = guatiao_warn(error, 'turns')
    if (status == GUATIAO_OK .and. c_associated(out)) then
      call c_f_pointer(out, answer)
      answer = n
    end if
  end function

  recursive function heard_slot(ctx, what) bind(c) result(status)
    type(c_ptr), value :: ctx
    type(gt_str), value :: what
    integer(c_int32_t) :: status
    class(guatiao_impl), pointer :: impl
    character(:), allocatable :: text
    type(guatiao_error), allocatable :: error
    impl => guatiao_impl_at(ctx, 'listener')
    status = GUATIAO_ERR_GONE
    if (.not. associated(impl)) return
    call guatiao_text_in(what, text)
    select type (impl)
    class is (listener)
      call impl%heard(text, error)
    end select
    status = guatiao_warn(error, 'heard')
  end function

  ! ---- calling another side's tables -----------------------------------

  ! The slot as a procedure, or a refusal when the table lacks it.
  recursive logical function lacks(r, i, error)
    type(guatiao_remote), intent(in) :: r
    integer, intent(in) :: i
    type(guatiao_error), allocatable, intent(inout) :: error
    lacks = .not. r%has(i)
    if (lacks) then
      if (r%is_closed()) then
        call guatiao_fail(error, GUATIAO_ERR_GONE, 'the remote is closed')
      else
        call guatiao_fail(error, GUATIAO_ERR_UNSUPPORTED, 'the table has no such slot')
      end if
    end if
  end function

  recursive subroutine greeter_remote_greet(self, name, answer, error)
    class(greeter_remote), intent(inout) :: self
    character(*), intent(in) :: name
    type(guatiao_value), intent(inout) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    procedure(greet_c), pointer :: f
    character(:), allocatable, target :: text
    type(gt_provider_error), target :: err
    type(guatiao_value) :: got
    type(c_ptr) :: out
    integer(c_int32_t) :: st
    integer :: ist
    if (lacks(self%r, 1, error)) return
    call c_f_procpointer(self%r%slot(1), f)
    text = name
    call guatiao_map_out(got, out, self%r%alloc)
    st = f(self%r%ctx, guatiao_string_in(text), out, c_loc(err))
    call guatiao_call_error(st, err, error)
    if (st /= GUATIAO_OK) return
    call guatiao_settle_home(got, ist)
    if (ist /= GUATIAO_OK) then
      call guatiao_fail(error, ist, "greet's answer could not be moved into this side's allocator")
      return
    end if
    answer = got
  end subroutine

  recursive subroutine greeter_remote_shout(self, name, answer, error)
    class(greeter_remote), intent(inout) :: self
    character(*), intent(in) :: name
    character(:), allocatable, intent(out) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    procedure(shout_c), pointer :: f
    character(:), allocatable, target :: text
    type(gt_owned), target :: out
    type(gt_value), target :: node
    type(guatiao_value) :: said
    integer(c_int32_t) :: st
    answer = ''
    if (lacks(self%r, 2, error)) return
    call c_f_procpointer(self%r%slot(2), f)
    text = name
    st = f(self%r%ctx, guatiao_string_in(text), c_loc(out))
    if (st /= GUATIAO_OK) then
      call guatiao_fail(error, int(st), 'shout failed')
      return
    end if
    node%tag = GUATIAO_TAG_STRING
    node%payload = out
    said = guatiao_value_from_raw(c_loc(node))
    answer = said%string_or('')
  end subroutine

  recursive subroutine greeter_remote_start(self, listener_in, answer, error)
    class(greeter_remote), intent(inout) :: self
    class(listener), allocatable, intent(inout) :: listener_in
    class(conversation), allocatable, intent(out) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    procedure(start_c), pointer :: f
    type(gt_object), target :: given
    type(gt_object), target :: got
    type(gt_provider_error), target :: err
    class(guatiao_impl), allocatable :: moved
    logical :: remote
    integer(c_int32_t) :: st
    integer :: ist
    if (lacks(self%r, 3, error)) return
    call c_f_procpointer(self%r%slot(3), f)
    if (.not. allocated(listener_in)) then
      call guatiao_fail(error, GUATIAO_ERR_NULL, 'no listener to hand over')
      return
    end if
    ist = GUATIAO_OK
    remote = .false.
    select type (listener_in)
    type is (listener_remote)
      call listener_in%r%give(given, ist)
      remote = .true.
    end select
    if (remote) then
      deallocate(listener_in)
    else
      call move_alloc(listener_in, moved)
      call guatiao_serve_object(moved, listener_kind(), given)
    end if
    if (ist /= GUATIAO_OK) then
      call guatiao_fail(error, ist, 'a listener this side does not own cannot be handed over')
      return
    end if
#if defined(__flang__) && (defined(__aarch64__) || defined(_WIN32))
    st = f(self%r%ctx, c_loc(given), c_loc(got), c_loc(err))
#else
    st = f(self%r%ctx, given, c_loc(got), c_loc(err))
#endif
    call guatiao_call_error(st, err, error)
    if (st /= GUATIAO_OK) return
    allocate(conversation_remote :: answer)
    select type (answer)
    type is (conversation_remote)
      call guatiao_take_object(got, conversation_kind(), answer%r, ist)
    end select
    if (ist /= GUATIAO_OK) call guatiao_fail(error, ist, 'the answer is not a conversation')
  end subroutine

  recursive subroutine greeter_remote_greet_into(self, name, listener_in, error)
    class(greeter_remote), intent(inout) :: self
    character(*), intent(in) :: name
    class(listener), intent(inout), target :: listener_in
    type(guatiao_error), allocatable, intent(out) :: error
    procedure(greet_into_c), pointer :: f
    character(:), allocatable, target :: text
    type(gt_object), target :: lent
    type(gt_provider_error), target :: err
    logical :: served
    integer(c_int32_t) :: st
    if (lacks(self%r, 4, error)) return
    call c_f_procpointer(self%r%slot(4), f)
    text = name
    served = .false.
    select type (listener_in)
    type is (listener_remote)
      lent%table = listener_in%r%table
      lent%size = listener_in%r%size
      lent%ctx = listener_in%r%ctx
    class default
      call guatiao_lend_impl(listener_in, listener_kind(), lent)
      served = .true.
    end select
    st = f(self%r%ctx, guatiao_string_in(text), c_loc(lent), c_loc(err))
    if (served) call guatiao_destroy_served(lent%ctx, close=.false.)
    call guatiao_call_error(st, err, error)
  end subroutine

  recursive subroutine greeter_remote_greet_from(self, config, answer, error)
    class(greeter_remote), intent(inout) :: self
    type(guatiao_value), intent(inout) :: config
    type(guatiao_value), intent(inout) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    procedure(map_map_c), pointer :: f
    type(gt_value), target :: handed
    type(gt_provider_error), target :: err
    type(guatiao_value) :: got
    type(c_ptr) :: out
    integer(c_int32_t) :: st
    integer :: ist
    if (lacks(self%r, 5, error)) return
    call c_f_procpointer(self%r%slot(5), f)
    call guatiao_hand_over(config, GUATIAO_TAG_MAP, handed, ist)
    if (ist /= GUATIAO_OK) then
      call guatiao_fail(error, ist, 'greet_from takes a map')
      return
    end if
    call guatiao_map_out(got, out, self%r%alloc)
    st = f(self%r%ctx, c_loc(handed%payload), out, c_loc(err))
    call guatiao_call_error(st, err, error)
    if (st /= GUATIAO_OK) return
    call guatiao_settle_home(got, ist)
    if (ist /= GUATIAO_OK) then
      call guatiao_fail(error, ist, "greet_from's answer could not be moved into this side's allocator")
      return
    end if
    answer = got
  end subroutine

  recursive subroutine greeter_remote_check(self, config, answer, error)
    class(greeter_remote), intent(inout) :: self
    type(guatiao_value), intent(inout) :: config
    logical, intent(out) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    procedure(map_map_c), pointer :: f
    type(gt_provider_error), target :: err
    integer(c_int8_t), target :: out
    type(c_ptr) :: lent, home
    integer(c_int32_t) :: st
    integer :: ist
    answer = .false.
    if (lacks(self%r, 6, error)) return
    call c_f_procpointer(self%r%slot(6), f)
    call guatiao_lend_tree(config, GUATIAO_TAG_MAP, lent, ist, home)
    if (ist /= GUATIAO_OK) then
      call guatiao_fail(error, ist, 'check takes a writable map')
      return
    end if
    out = GUATIAO_C_FALSE
    st = f(self%r%ctx, lent, c_loc(out), c_loc(err))
    call guatiao_settle_lent(config, GUATIAO_TAG_MAP, home, ist)
    call guatiao_call_error(st, err, error)
    if (st /= GUATIAO_OK) return
    if (ist /= GUATIAO_OK) then
      call guatiao_fail(error, ist, "moving what check put into a lent tree into that tree's allocator")
      return
    end if
    answer = out /= GUATIAO_C_FALSE
  end subroutine

  !> Whether the table has the slot named `slot` (`greet`, `shout`, ...).
  recursive logical function greeter_remote_has(self, slot)
    class(greeter_remote), intent(in) :: self
    character(*), intent(in) :: slot
    integer :: i
    select case (slot)
    case ('greet'); i = 1
    case ('shout'); i = 2
    case ('start'); i = 3
    case ('greet_into'); i = 4
    case ('greet_from'); i = 5
    case ('check'); i = 6
    case default; i = 99
    end select
    greeter_remote_has = self%r%has(i)
  end function

  recursive subroutine counter_remote_count(self, answer, error)
    class(counter_remote), intent(inout) :: self
    integer(int64), intent(out) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    procedure(int_out_c), pointer :: f
    integer(c_int64_t), target :: out
    integer(c_int32_t) :: st
    answer = 0
    if (lacks(self%r, 1, error)) return
    call c_f_procpointer(self%r%slot(1), f)
    out = 0
    st = f(self%r%ctx, c_loc(out))
    if (st /= GUATIAO_OK) call guatiao_fail(error, int(st), 'count failed')
    answer = out
  end subroutine

  recursive subroutine wide_remote_echo_i64(self, x, answer, error)
    class(wide_remote), intent(inout) :: self
    integer(int64), intent(in) :: x
    integer(int64), intent(out) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    procedure(echo_c), pointer :: f
    integer(c_int64_t), target :: out
    type(gt_provider_error), target :: err
    integer(c_int32_t) :: st
    answer = 0
    if (lacks(self%r, 1, error)) return
    call c_f_procpointer(self%r%slot(1), f)
    out = 0
    st = f(self%r%ctx, x, c_loc(out), c_loc(err))
    call guatiao_call_error(st, err, error)
    answer = out
  end subroutine

  recursive subroutine wide_remote_echo_u64(self, x, answer, error)
    class(wide_remote), intent(inout) :: self
    integer(int64), intent(in) :: x
    integer(int64), intent(out) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    procedure(echo_c), pointer :: f
    integer(c_int64_t), target :: out
    type(gt_provider_error), target :: err
    integer(c_int32_t) :: st
    answer = 0
    if (lacks(self%r, 2, error)) return
    call c_f_procpointer(self%r%slot(2), f)
    out = 0
    st = f(self%r%ctx, x, c_loc(out), c_loc(err))
    call guatiao_call_error(st, err, error)
    answer = out
  end subroutine

  recursive subroutine listener_remote_heard(self, what, error)
    class(listener_remote), intent(inout) :: self
    character(*), intent(in) :: what
    type(guatiao_error), allocatable, intent(out) :: error
    procedure(heard_c), pointer :: f
    character(:), allocatable, target :: text
    integer(c_int32_t) :: st
    if (lacks(self%r, 2, error)) return
    call c_f_procpointer(self%r%slot(2), f)
    text = what
    st = f(self%r%ctx, guatiao_string_in(text))
    if (st /= GUATIAO_OK) call guatiao_fail(error, int(st), 'heard failed')
  end subroutine

  recursive subroutine listener_remote_close(self)
    class(listener_remote), intent(inout) :: self
    call self%r%close()
  end subroutine

  recursive subroutine conversation_remote_say(self, what, error)
    class(conversation_remote), intent(inout) :: self
    character(*), intent(in) :: what
    type(guatiao_error), allocatable, intent(out) :: error
    procedure(say_c), pointer :: f
    character(:), allocatable, target :: text
    type(gt_provider_error), target :: err
    integer(c_int32_t) :: st
    if (lacks(self%r, 2, error)) return
    call c_f_procpointer(self%r%slot(2), f)
    text = what
    st = f(self%r%ctx, guatiao_string_in(text), c_loc(err))
    call guatiao_call_error(st, err, error)
  end subroutine

  recursive subroutine conversation_remote_read(self, dst, answer, error)
    class(conversation_remote), intent(inout) :: self
    integer(int8), intent(inout) :: dst(:)
    integer(int64), intent(out) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    procedure(read_c), pointer :: f
    integer(int8), allocatable, target :: buf(:)
    type(gt_bytes_mut) :: view
    integer(c_int64_t), target :: out
    integer(c_int32_t) :: st
    answer = 0
    if (lacks(self%r, 3, error)) return
    call c_f_procpointer(self%r%slot(3), f)
    ! The callee writes into memory that is surely contiguous: a copy of
    ! `dst`, written back.
    buf = dst
    view%len = size(buf, kind=c_size_t)
    if (view%len > 0) view%ptr = c_loc(buf)
    out = 0
    st = f(self%r%ctx, view, c_loc(out))
    dst = buf
    if (st /= GUATIAO_OK) call guatiao_fail(error, int(st), 'read failed')
    answer = out
  end subroutine

  recursive subroutine conversation_remote_turns(self, answer, error)
    class(conversation_remote), intent(inout) :: self
    integer(int64), intent(out) :: answer
    type(guatiao_error), allocatable, intent(out) :: error
    procedure(int_out_c), pointer :: f
    integer(c_int64_t), target :: out
    integer(c_int32_t) :: st
    answer = 0
    if (lacks(self%r, 4, error)) return
    call c_f_procpointer(self%r%slot(4), f)
    out = 0
    st = f(self%r%ctx, c_loc(out))
    if (st /= GUATIAO_OK) call guatiao_fail(error, int(st), 'turns failed')
    answer = out
  end subroutine

  !> Destroys the conversation, once.
  recursive subroutine conversation_remote_close(self)
    class(conversation_remote), intent(inout) :: self
    call self%r%close()
  end subroutine

end module greeter_kinds
