! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> Kinds: the root every kind's abstract type extends, a kind's
!> description, calling a table (a remote) and serving one (the boxes a
!> served `ctx` addresses, and the helpers a kind's bind(c) glue calls).
!>
!> A kind's own module, written once by its author, holds the abstract
!> type, the table mirror, the remote type and a bind(c) procedure per
!> slot; everything here is what those share.
module guatiao_kinds
  use, intrinsic :: iso_c_binding
  use, intrinsic :: iso_fortran_env, only: int8, int64, error_unit
  use guatiao_c
  use guatiao_native
  use guatiao_errors
  use guatiao_values
  use guatiao_holds
  use guatiao_host
  implicit none
  private

  public :: guatiao_impl, guatiao_kind, guatiao_serving, guatiao_remote
  public :: guatiao_bind, guatiao_bind_instance, guatiao_bind_table, guatiao_live_objects
  public :: guatiao_impl_at, guatiao_new_served, guatiao_destroy_served, guatiao_serve_object, guatiao_lend_impl, &
            guatiao_take_object, guatiao_lend_object, guatiao_end_lent_object, guatiao_failed, guatiao_warn, &
            guatiao_text_in, guatiao_map_carrier, guatiao_value_carrier, guatiao_settle_map, &
            guatiao_settle_value, guatiao_lent_map, guatiao_end_lent_map, guatiao_taken_map, &
            guatiao_borrowed_map, guatiao_borrowed_value, guatiao_string_out, guatiao_bytes_in, &
            guatiao_list_carrier, guatiao_settle_list, guatiao_lent_list, guatiao_end_lent_list, &
            guatiao_taken_list
  public :: guatiao_call_error, guatiao_map_out, guatiao_settle_home, guatiao_string_in, &
            guatiao_lend_tree, guatiao_settle_lent, guatiao_lent_refusal, guatiao_hand_over, guatiao_object_arg, &
            guatiao_list_out, guatiao_value_out, guatiao_borrow_tree

  !> What every kind's abstract type extends. `close` runs when the side
  !> holding the object last lets it go (an instance its host destroys,
  !> an object its holder destroys); it does nothing unless overridden.
  type, abstract :: guatiao_impl
  contains
    procedure :: close => impl_close
  end type

  abstract interface
    !> Whether `impl` is of the kind (a `select type` in the kind's module).
    logical function accepts_i(impl)
      import :: guatiao_impl
      class(guatiao_impl), intent(in) :: impl
    end function
  end interface

  !> A kind, as its module describes it: the name, the header's floor
  !> hash (its uint32 bits), the size of the header and the required
  !> slots, the whole table's size, whether it is an object kind (its
  !> first slot `destroy`), and the table every served object of it uses.
  type :: guatiao_kind
    character(:), allocatable :: name
    integer(c_int32_t) :: floor_hash = 0
    integer(c_size_t) :: floor_size = 0
    integer(c_size_t) :: size = 0
    logical :: object = .false.
    type(c_ptr) :: object_table = c_null_ptr
    procedure(accepts_i), pointer, nopass :: accepts => null()
  end type

  !> An object serving a kind through a table built for it. `owned`:
  !> closed and deallocated when what serves it is destroyed. A served
  !> object is moved into `kept`, never copied: a copy of an object
  !> holding a counted handle would release it twice.
  type :: guatiao_serving
    class(guatiao_impl), pointer :: impl => null()
    type(guatiao_kind) :: kind
    type(c_ptr) :: table = c_null_ptr
    logical :: owned = .false.
    class(guatiao_impl), allocatable :: kept
  end type

  ! What a served ctx addresses: the objects serving each of its kinds.
  type :: gt_served
    integer :: magic = 0
    type(guatiao_serving), allocatable :: servings(:)
  end type

  integer, parameter :: MAGIC = 1735292014

  !> A table another side serves, called through: its address and size,
  !> the ctx its slots take, and a hold keeping what it came from alive
  !> (the registry, an instance) or owning the object (destroyed once,
  !> when the last copy goes or on `close`).
  type :: guatiao_remote
    type(c_ptr) :: table = c_null_ptr
    integer(c_size_t) :: size = 0
    type(c_ptr) :: ctx = c_null_ptr
    type(c_ptr) :: alloc = c_null_ptr   ! what answers are built in: the registry's
    type(gt_hold) :: hold
  contains
    procedure :: slot => remote_slot
    procedure :: has => remote_has
    procedure :: is_closed => remote_is_closed
    procedure :: close => remote_close
    procedure :: give => remote_give
  end type

  type, bind(c) :: table_head
    type(gt_kind_header) :: header
  end type

  integer, save :: live = 0

contains

  recursive subroutine impl_close(self)
    class(guatiao_impl), intent(inout) :: self
  end subroutine

  ! ---- remotes ---------------------------------------------------------

  !> The provider under `key` as `kind`: its table for the kind, checked
  !> against the kind's floor (GUATIAO_STAT_FLOOR_MISMATCH), and its ctx.
  recursive subroutine guatiao_bind(reg, key, kind, r, stat, errmsg)
    type(guatiao_registry), intent(in) :: reg
    character(*), intent(in) :: key
    type(guatiao_kind), intent(in) :: kind
    type(guatiao_remote), intent(inout) :: r
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: table
    integer(c_size_t) :: size
    integer :: st
    character(:), allocatable :: msg
    call reg%provider_table(key, kind%name, table, size, st, msg)
    if (st == GUATIAO_OK) then
      call guatiao_bind_table(table, size, reg%provider_ctx(key), kind, r, st, msg)
      if (st == GUATIAO_OK) then
        r%hold = gt_registry_hold(reg)
        r%alloc = reg%alloc
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> An instance as `kind`: the table of the provider that built it, and
  !> the instance as its ctx; the remote keeps the instance alive.
  recursive subroutine guatiao_bind_instance(inst, kind, r, stat, errmsg)
    type(guatiao_instance), intent(in) :: inst
    type(guatiao_kind), intent(in) :: kind
    type(guatiao_remote), intent(inout) :: r
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: reg, table
    integer(c_size_t), target :: size
    character(:), allocatable, target :: key, name
    integer :: st
    character(:), allocatable :: msg
    reg = gt_instance_reg(inst)
    if (.not. c_associated(reg)) then
      call gt_report(GUATIAO_STAT_CLOSED, 'guatiao: the instance is closed', stat, errmsg)
      return
    end if
    key = gt_instance_key(inst)
    name = kind%name
    size = 0
    table = gt_registry_provider_table(reg, gt_view(key), gt_view(name), c_loc(size))
    if (.not. c_associated(table)) then
      call gt_report(GUATIAO_ERR_NOT_FOUND, 'guatiao: "' // key // '" offers no ' // name // ' table', stat, &
                     errmsg)
      return
    end if
    call guatiao_bind_table(table, size, gt_instance_ctx(inst), kind, r, st, msg)
    if (st == GUATIAO_OK) r%hold = inst%hold
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> Any table as `kind`: GUATIAO_STAT_FLOOR_MISMATCH when it is smaller
  !> than the kind's required slots or carries another floor hash.
  recursive subroutine guatiao_bind_table(table, size, ctx, kind, r, stat, errmsg)
    type(c_ptr), intent(in) :: table, ctx
    integer(c_size_t), intent(in) :: size
    type(guatiao_kind), intent(in) :: kind
    type(guatiao_remote), intent(inout) :: r
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(table_head), pointer :: head
    character(24) :: a, b
    if (.not. c_associated(table) .or. size < kind%floor_size) then
      call gt_report(GUATIAO_STAT_FLOOR_MISMATCH, 'guatiao: the table is smaller than the ' // kind%name // &
                     ' kind''s required slots', stat, errmsg)
      return
    end if
    call c_f_pointer(table, head)
    if (head%header%floor_hash /= kind%floor_hash) then
      write (a, '(i0)') iand(int(head%header%floor_hash, int64), 4294967295_int64)
      write (b, '(i0)') iand(int(kind%floor_hash, int64), 4294967295_int64)
      call gt_report(GUATIAO_STAT_FLOOR_MISMATCH, 'guatiao: the table''s floor hash ' // trim(a) // &
                     ' is not the ' // kind%name // ' kind''s ' // trim(b), stat, errmsg)
      return
    end if
    r%table = table
    r%size = size
    r%ctx = ctx
    call gt_report(GUATIAO_OK, '', stat, errmsg)
  end subroutine

  !> The `i`th slot after the header (from 1), or null when the table is
  !> too small for it or leaves it empty.
  recursive function remote_slot(self, i) result(f)
    class(guatiao_remote), intent(in) :: self
    integer, intent(in) :: i
    type(c_funptr) :: f
    type(c_funptr), pointer :: slots(:)
    integer(c_size_t) :: n, header
    f = c_null_funptr
    if (self%hold%is_closed() .and. associated(self%hold%box)) return
    if (.not. c_associated(self%table)) return
    ! The 8-byte header is one pointer on a 64-bit platform, two on a 32-bit one.
    header = 8 / c_sizeof(c_null_funptr)
    n = self%size / c_sizeof(c_null_funptr)
    if (int(i, c_size_t) + header > n) return
    call c_f_pointer(self%table, slots, [n])
    f = slots(header + i)
  end function

  recursive logical function remote_has(self, i)
    class(guatiao_remote), intent(in) :: self
    integer, intent(in) :: i
    remote_has = c_associated(self%slot(i))
  end function

  recursive logical function remote_is_closed(self)
    class(guatiao_remote), intent(in) :: self
    remote_is_closed = .not. c_associated(self%table)
    if (associated(self%hold%box)) remote_is_closed = remote_is_closed .or. self%hold%is_closed()
  end function

  !> Destroys an object this side owns, once; nothing for a provider's
  !> table or a lent object.
  recursive subroutine remote_close(self)
    class(guatiao_remote), intent(inout) :: self
    if (associated(self%hold%box)) then
      if (self%hold%box%what == GT_HOLD_OBJECT .or. self%hold%box%what == GT_HOLD_LENT) call self%hold%close()
    end if
  end subroutine

  !> Hands an object this side owns to another side, as the struct that
  !> crosses: the other side destroys it from now, and this remote reads
  !> closed. GUATIAO_ERR_WRONG_KIND for what this side does not own.
  recursive subroutine remote_give(self, obj, stat)
    class(guatiao_remote), intent(inout) :: self
    type(gt_object), intent(out) :: obj
    integer, intent(out) :: stat
    stat = GUATIAO_ERR_WRONG_KIND
    if (.not. associated(self%hold%box)) return
    if (self%hold%box%what /= GT_HOLD_OBJECT .or. self%hold%is_closed()) return
    obj%table = self%table
    obj%size = self%size
    obj%ctx = self%ctx
    call self%hold%give_up()
    stat = GUATIAO_OK
  end subroutine

  !> An object another side handed over (by value): this side owns it,
  !> and destroys it when the last copy of the remote goes or on close.
  recursive subroutine guatiao_take_object(obj, kind, r, stat)
    type(gt_object), intent(in) :: obj
    type(guatiao_kind), intent(in) :: kind
    type(guatiao_remote), intent(inout) :: r
    integer, intent(out) :: stat
    call guatiao_bind_table(obj%table, obj%size, obj%ctx, kind, r, stat)
    if (stat == GUATIAO_OK) then
      r%hold = gt_new_hold(GT_HOLD_OBJECT, table=obj%table, size=obj%size, ctx=obj%ctx)
    else if (c_associated(obj%table)) then
      ! Refused, but handed over all the same: destroyed here.
      r%hold = gt_new_hold(GT_HOLD_OBJECT, table=obj%table, size=obj%size, ctx=obj%ctx)
      call r%hold%close()
    end if
  end subroutine

  !> An object another side lent for one call (`const guatiao_object *`):
  !> never destroyed here; guatiao_end_lent_object ends it.
  recursive subroutine guatiao_lend_object(ptr, kind, r, stat)
    type(c_ptr), intent(in) :: ptr
    type(guatiao_kind), intent(in) :: kind
    type(guatiao_remote), intent(inout) :: r
    integer, intent(out) :: stat
    type(gt_object), pointer :: obj
    stat = GUATIAO_ERR_NULL
    if (.not. c_associated(ptr)) return
    call c_f_pointer(ptr, obj)
    call guatiao_bind_table(obj%table, obj%size, obj%ctx, kind, r, stat)
    if (stat == GUATIAO_OK) r%hold = gt_new_hold(GT_HOLD_LENT)
  end subroutine

  !> The call that lent an object returned: every copy reads closed.
  recursive subroutine guatiao_end_lent_object(r)
    type(guatiao_remote), intent(inout) :: r
    call r%hold%close()
  end subroutine

  ! ---- served boxes ----------------------------------------------------

  !> A box for `servings`, its address the ctx their tables' slots take.
  recursive function guatiao_new_served(servings) result(ctx)
    type(guatiao_serving), intent(in) :: servings(:)
    type(c_ptr) :: ctx
    type(gt_served), pointer :: box
    allocate(box)
    box%magic = MAGIC
    allocate(box%servings(size(servings)))
    box%servings = servings
    ctx = c_loc(box)
  end function

  !> The object serving `kind` behind `ctx`, or null.
  recursive function guatiao_impl_at(ctx, kind) result(impl)
    type(c_ptr), intent(in) :: ctx
    character(*), intent(in) :: kind
    class(guatiao_impl), pointer :: impl
    type(gt_served), pointer :: box
    integer :: i
    nullify(impl)
    if (.not. c_associated(ctx)) return
    call c_f_pointer(ctx, box)
    if (box%magic /= MAGIC) return
    do i = 1, size(box%servings)
      if (box%servings(i)%kind%name == kind) then
        impl => box%servings(i)%impl
        return
      end if
    end do
  end function

  !> Releases what `ctx` holds: each owned object closed (unless `close`
  !> says not: a copy served for one lent call) and deallocated, then the
  !> box.
  recursive subroutine guatiao_destroy_served(ctx, close)
    type(c_ptr), intent(in) :: ctx
    logical, intent(in), optional :: close
    type(gt_served), pointer :: box
    integer :: i
    logical :: closing
    if (.not. c_associated(ctx)) return
    call c_f_pointer(ctx, box)
    if (box%magic /= MAGIC) return
    box%magic = 0
    closing = .true.
    if (present(close)) closing = close
    do i = 1, size(box%servings)
      if (.not. box%servings(i)%owned) cycle
      if (allocated(box%servings(i)%kept)) then
        if (closing) call box%servings(i)%kept%close()
        deallocate(box%servings(i)%kept)
      else if (associated(box%servings(i)%impl)) then
        if (closing) call box%servings(i)%impl%close()
        deallocate(box%servings(i)%impl)
      end if
    end do
    if (any(box%servings%kind%object) .and. any(box%servings%owned)) live = live - 1
    deallocate(box)
  end subroutine

  !> Serves `impl` (moved: it is deallocated here) as an object of
  !> `kind`, answering the struct that crosses. Its holder destroys it.
  recursive subroutine guatiao_serve_object(impl, kind, obj)
    class(guatiao_impl), allocatable, intent(inout) :: impl
    type(guatiao_kind), intent(in) :: kind
    type(gt_object), intent(out) :: obj
    type(gt_served), pointer :: box
    allocate(box)
    box%magic = MAGIC
    allocate(box%servings(1))
    call move_alloc(impl, box%servings(1)%kept)
    box%servings(1)%impl => box%servings(1)%kept
    box%servings(1)%kind = kind
    box%servings(1)%table = kind%object_table
    box%servings(1)%owned = .true.
    obj%table = kind%object_table
    obj%size = kind%size
    obj%ctx = c_loc(box)
    live = live + 1
  end subroutine

  !> Serves `impl` for one call another side is lent it for: nothing is
  !> moved or copied; end it with guatiao_destroy_served(ctx, close=.false.)
  !> once the call returns.
  recursive subroutine guatiao_lend_impl(impl, kind, obj)
    class(guatiao_impl), intent(in), target :: impl
    type(guatiao_kind), intent(in) :: kind
    type(gt_object), intent(out) :: obj
    type(gt_served), pointer :: box
    allocate(box)
    box%magic = MAGIC
    allocate(box%servings(1))
    box%servings(1)%impl => impl
    box%servings(1)%kind = kind
    box%servings(1)%table = kind%object_table
    obj%table = kind%object_table
    obj%size = kind%size
    obj%ctx = c_loc(box)
  end subroutine

  !> Objects this process serves that a holder has not destroyed yet.
  recursive integer function guatiao_live_objects()
    guatiao_live_objects = live
  end function

  !> An object argument a remote hands to another side: a remote this
  !> side owns is given (its object crosses as it is); any other object
  !> is served (moved into a box the other side destroys).
  recursive subroutine guatiao_object_arg(impl, remote, kind, obj, stat)
    class(guatiao_impl), allocatable, intent(inout) :: impl
    type(guatiao_remote), intent(inout), optional :: remote
    type(guatiao_kind), intent(in) :: kind
    type(gt_object), intent(out) :: obj
    integer, intent(out) :: stat
    stat = GUATIAO_OK
    if (present(remote)) then
      call remote%give(obj, stat)
      if (allocated(impl)) deallocate(impl)
      return
    end if
    if (.not. allocated(impl)) then
      stat = GUATIAO_ERR_NULL
      return
    end if
    call guatiao_serve_object(impl, kind, obj)
  end subroutine

  ! ---- a served slot's arguments and answers ---------------------------

  !> The text a `guatiao_str` argument views.
  recursive subroutine guatiao_text_in(view, text)
    type(gt_str), intent(in) :: view
    character(:), allocatable, intent(out) :: text
    call gt_copy_text(view%ptr, view%len, text)
  end subroutine

  !> The writable bytes a `guatiao_bytes_mut` views, valid for the call.
  recursive subroutine guatiao_bytes_in(view, bytes)
    type(gt_bytes_mut), intent(in) :: view
    integer(int8), pointer, intent(out) :: bytes(:)
    if (view%len == 0 .or. .not. c_associated(view%ptr)) then
      allocate(bytes(0))
      return
    end if
    call c_f_pointer(view%ptr, bytes, [view%len])
  end subroutine

  ! The allocator a carrier names, else guatiao's own heap.
  recursive function carrier_alloc(a) result(p)
    type(c_ptr), intent(in) :: a
    type(c_ptr) :: p
    p = a
    if (.not. c_associated(p)) p = gt_default_alloc()
  end function

  !> An answer to fill, over the carrier a slot's `guatiao_map *out`
  !> holds: an empty map in the allocator the caller named.
  recursive function guatiao_map_carrier(out) result(answer)
    type(c_ptr), intent(in) :: out
    type(guatiao_value) :: answer
    answer = tree_carrier(out, GUATIAO_TAG_MAP)
  end function

  !> The same for a slot's `guatiao_list *out`: an empty list.
  recursive function guatiao_list_carrier(out) result(answer)
    type(c_ptr), intent(in) :: out
    type(guatiao_value) :: answer
    answer = tree_carrier(out, GUATIAO_TAG_LIST)
  end function

  recursive function tree_carrier(out, tag) result(answer)
    type(c_ptr), intent(in) :: out
    integer(c_int32_t), intent(in) :: tag
    type(guatiao_value) :: answer
    type(gt_owned), pointer :: carrier
    type(gt_value) :: node
    call c_f_pointer(out, carrier)
    node%tag = tag
    node%payload = carrier
    node%payload%alloc = carrier_alloc(carrier%alloc)
    answer = gt_wrap(node, .false., .false., node%payload%alloc)
  end function

  !> Writes the answer over the carrier: the map filled in place, or a
  !> copy, in the carrier's allocator, of whatever the method answered
  !> instead (GUATIAO_ERR_WRONG_KIND unless it is a map).
  !> When the method refused (`failed`), what it built is freed and the
  !> carrier left as it came.
  recursive subroutine guatiao_settle_map(carrier, answer, out, stat, failed)
    type(guatiao_value), intent(inout) :: carrier
    type(guatiao_value), intent(in) :: answer
    type(c_ptr), intent(in) :: out
    integer, intent(out) :: stat
    logical, intent(in), optional :: failed
    call settle_tree(carrier, answer, out, GUATIAO_TAG_MAP, stat, failed)
  end subroutine

  !> The same for a list answer (GUATIAO_ERR_WRONG_KIND unless a list).
  recursive subroutine guatiao_settle_list(carrier, answer, out, stat, failed)
    type(guatiao_value), intent(inout) :: carrier
    type(guatiao_value), intent(in) :: answer
    type(c_ptr), intent(in) :: out
    integer, intent(out) :: stat
    logical, intent(in), optional :: failed
    call settle_tree(carrier, answer, out, GUATIAO_TAG_LIST, stat, failed)
  end subroutine

  recursive subroutine settle_tree(carrier, answer, out, tag, stat, failed)
    type(guatiao_value), intent(inout) :: carrier
    integer(c_int32_t), intent(in) :: tag
    type(guatiao_value), intent(in) :: answer
    type(c_ptr), intent(in) :: out
    integer, intent(out) :: stat
    logical, intent(in), optional :: failed
    type(gt_owned), pointer :: slot
    type(gt_value), pointer :: root
    type(gt_value), target :: copy
    type(c_ptr) :: a, p
    integer(c_int32_t) :: ignored
    call c_f_pointer(out, slot)
    call c_f_pointer(gt_root(carrier), root)
    a = root%payload%alloc
    stat = GUATIAO_OK
    if (present(failed)) then
      if (failed) then
        ignored = gt_value_free(gt_root(carrier))
        call gt_end_borrow(carrier)
        return
      end if
    end if
    if (gt_same_tree(carrier, answer)) then
      slot = root%payload
      call gt_end_borrow(carrier)
      return
    end if
    ! Answered with another tree: the carrier's own map is let go first.
    ignored = gt_value_free(gt_root(carrier))
    call gt_end_borrow(carrier)
    p = gt_node(answer)
    if (.not. c_associated(p)) then
      stat = GUATIAO_ERR_WRONG_KIND
      return
    end if
    stat = gt_value_clone(a, p, c_loc(copy))
    if (stat /= GUATIAO_OK) return
    if (copy%tag /= tag) then
      ignored = gt_value_free(c_loc(copy))
      stat = GUATIAO_ERR_WRONG_KIND
      return
    end if
    slot = copy%payload
  end subroutine

  !> An answer to fill for a slot's `guatiao_value *out`: absent, in the
  !> allocator its carrier names.
  recursive function guatiao_value_carrier(out) result(answer)
    type(c_ptr), intent(in) :: out
    type(guatiao_value) :: answer
    type(gt_value), pointer :: carrier
    type(gt_value) :: none
    call c_f_pointer(out, carrier)
    answer = guatiao_absent()
    if (c_associated(carrier%payload%alloc)) answer = gt_wrap(none, .true., .false., carrier%payload%alloc)
  end function

  !> Writes a copy of the answer, in the carrier's allocator, over it.
  recursive subroutine guatiao_settle_value(answer, out, stat)
    type(guatiao_value), intent(in) :: answer
    type(c_ptr), intent(in) :: out
    integer, intent(out) :: stat
    type(gt_value), pointer :: slot
    type(gt_value) :: none
    type(c_ptr) :: a, p
    call c_f_pointer(out, slot)
    a = carrier_alloc(slot%payload%alloc)
    p = gt_node(answer)
    stat = GUATIAO_OK
    if (.not. c_associated(p)) then
      slot = none
      return
    end if
    stat = gt_value_clone(a, p, out)
  end subroutine

  !> A map the caller lent for the call (`guatiao_map *config`, its slot
  !> comment calling it lent): writable, grown through the allocator it
  !> records; guatiao_end_lent_map writes it back.
  recursive function guatiao_lent_map(ptr) result(v)
    type(c_ptr), intent(in) :: ptr
    type(guatiao_value) :: v
    v = lent_tree(ptr, GUATIAO_TAG_MAP)
  end function

  !> A list the caller lent for the call; guatiao_end_lent_list writes
  !> it back.
  recursive function guatiao_lent_list(ptr) result(v)
    type(c_ptr), intent(in) :: ptr
    type(guatiao_value) :: v
    v = lent_tree(ptr, GUATIAO_TAG_LIST)
  end function

  recursive function lent_tree(ptr, tag) result(v)
    type(c_ptr), intent(in) :: ptr
    integer(c_int32_t), intent(in) :: tag
    type(guatiao_value) :: v
    type(gt_owned), pointer :: map
    type(gt_value) :: node
    if (.not. c_associated(ptr)) then
      v = guatiao_absent()
      return
    end if
    call c_f_pointer(ptr, map)
    node%tag = tag
    node%payload = map
    v = gt_wrap(node, .false., .false., carrier_alloc(map%alloc))
  end function

  !> The call that lent the map returned: what it holds now goes back to
  !> the caller, and every copy reads closed.
  recursive subroutine guatiao_end_lent_map(v, ptr)
    type(guatiao_value), intent(inout) :: v
    type(c_ptr), intent(in) :: ptr
    type(gt_owned), pointer :: map
    type(gt_value), pointer :: root
    if (c_associated(ptr) .and. c_associated(gt_root(v))) then
      call c_f_pointer(ptr, map)
      call c_f_pointer(gt_root(v), root)
      map = root%payload
    end if
    call gt_end_borrow(v)
  end subroutine

  !> The call that lent the list returned (guatiao_end_lent_map's twin).
  recursive subroutine guatiao_end_lent_list(v, ptr)
    type(guatiao_value), intent(inout) :: v
    type(c_ptr), intent(in) :: ptr
    call guatiao_end_lent_map(v, ptr)
  end subroutine

  !> A map handed over (`guatiao_map *` neither const nor lent): the
  !> method owns it and may keep it, so it is moved out of the caller's
  !> allocators (which may go with the caller's library) into this
  !> binding's; it is freed when the last handle on it goes. A refused
  !> move frees it and is `stat` when given (absent comes back).
  recursive function guatiao_taken_map(ptr, stat) result(v)
    type(c_ptr), intent(in) :: ptr
    integer, intent(out), optional :: stat
    type(guatiao_value) :: v
    v = taken_tree(ptr, GUATIAO_TAG_MAP, stat)
  end function

  !> A list handed over, moved as guatiao_taken_map moves a map.
  recursive function guatiao_taken_list(ptr, stat) result(v)
    type(c_ptr), intent(in) :: ptr
    integer, intent(out), optional :: stat
    type(guatiao_value) :: v
    v = taken_tree(ptr, GUATIAO_TAG_LIST, stat)
  end function

  recursive function taken_tree(ptr, tag, stat) result(v)
    type(c_ptr), intent(in) :: ptr
    integer(c_int32_t), intent(in) :: tag
    integer, intent(out), optional :: stat
    type(guatiao_value) :: v
    type(gt_owned), pointer :: map
    type(gt_value), target :: node
    integer(c_int32_t) :: st
    if (present(stat)) stat = GUATIAO_OK
    if (.not. c_associated(ptr)) then
      v = guatiao_absent()
      return
    end if
    call c_f_pointer(ptr, map)
    node%tag = tag
    node%payload = map
    if (associated(gt_value_make_local)) then
      st = gt_value_make_local(c_loc(node), gt_default_alloc())
      if (st /= GUATIAO_OK) then
        st = gt_value_free(c_loc(node))
        if (present(stat)) stat = GUATIAO_ERR_ALLOC
        v = guatiao_absent()
        return
      end if
    end if
    v = gt_wrap(node, .true., .false., carrier_alloc(node%payload%alloc))
  end function

  !> A map borrowed for the call (`const guatiao_map *`): read-only;
  !> end it with gt_end_borrow.
  recursive function guatiao_borrowed_map(ptr) result(v)
    type(c_ptr), intent(in) :: ptr
    type(guatiao_value) :: v
    type(gt_owned), pointer :: map
    type(gt_value) :: node
    if (.not. c_associated(ptr)) then
      v = guatiao_absent()
      return
    end if
    call c_f_pointer(ptr, map)
    node%tag = GUATIAO_TAG_MAP
    node%payload = map
    v = gt_wrap(node, .false., .true., gt_default_alloc())
  end function

  !> A value borrowed for the call (`const guatiao_value *`): read-only.
  recursive function guatiao_borrowed_value(ptr) result(v)
    type(c_ptr), intent(in) :: ptr
    type(guatiao_value) :: v
    v = gt_borrowed(ptr, .true., gt_default_alloc())
  end function

  !> Writes `text` as a slot's `guatiao_string *out`, on guatiao's own
  !> heap (the caller frees it through the allocator it records).
  recursive subroutine guatiao_string_out(text, out, stat)
    character(*), intent(in), target :: text
    type(c_ptr), intent(in) :: out
    integer, intent(out) :: stat
    type(gt_owned), pointer :: slot
    type(gt_value), target :: node
    call c_f_pointer(out, slot)
    stat = gt_value_string(gt_default_alloc(), gt_view(text), c_loc(node))
    if (stat == GUATIAO_OK) then
      slot = node%payload
    else
      slot = gt_owned()
    end if
  end subroutine

  !> The status a slot answers for `error`, its words written into the
  !> slot's `guatiao_provider_error *err` (when there is one) in guatiao's
  !> own heap. GUATIAO_OK when `error` is unallocated.
  recursive function guatiao_failed(error, err) result(status)
    type(guatiao_error), allocatable, intent(in) :: error
    type(c_ptr), intent(in) :: err
    integer(c_int32_t) :: status
    type(gt_provider_error), pointer :: e
    type(gt_value), target :: node
    character(:), allocatable, target :: words
    integer(c_int32_t) :: st
    status = GUATIAO_OK
    if (.not. allocated(error)) return
    status = int(error%status, c_int32_t)
    if (status == GUATIAO_OK) status = GUATIAO_ERR_INTERNAL
    if (.not. c_associated(err)) return
    call c_f_pointer(err, e)
    e%status = status
    e%message = gt_owned()
    words = ''
    if (allocated(error%message)) words = error%message
    if (len(words) == 0) return
    st = gt_value_string(gt_default_alloc(), gt_view(words), c_loc(node))
    if (st == GUATIAO_OK) e%message = node%payload
  end function

  !> The status a slot with no `err` answers for `error`, whose words go
  !> to standard error.
  recursive function guatiao_warn(error, slot) result(status)
    type(guatiao_error), allocatable, intent(in) :: error
    character(*), intent(in) :: slot
    integer(c_int32_t) :: status
    status = GUATIAO_OK
    if (.not. allocated(error)) return
    status = int(error%status, c_int32_t)
    if (status == GUATIAO_OK) status = GUATIAO_ERR_INTERNAL
    if (allocated(error%message)) then
      write (error_unit, '(a)') 'guatiao: ' // slot // ' failed: ' // error%message
    else
      write (error_unit, '(a)') 'guatiao: ' // slot // ' failed'
    end if
  end function

  ! ---- a remote call's arguments and answers ---------------------------

  !> A text argument's view, over `text`, which outlives the call.
  recursive function guatiao_string_in(text) result(v)
    character(*), intent(in), target :: text
    type(gt_str) :: v
    v = gt_view(text)
  end function

  !> A fresh answer for a remote slot's `guatiao_map *out` and the
  !> address to pass: an empty map naming `alloc` (else guatiao's heap),
  !> the carrier the callee builds its answer in.
  recursive subroutine guatiao_map_out(answer, out, alloc)
    type(guatiao_value), intent(inout) :: answer
    type(c_ptr), intent(out) :: out
    type(c_ptr), intent(in) :: alloc
    type(gt_value), pointer :: root
    type(guatiao_allocator) :: a
    a%ptr = alloc
    answer = guatiao_map(a)
    call c_f_pointer(gt_root(answer), root)
    out = c_loc(root%payload)
  end subroutine

  !> The same for a remote slot's `guatiao_list *out`: an empty list.
  recursive subroutine guatiao_list_out(answer, out, alloc)
    type(guatiao_value), intent(inout) :: answer
    type(c_ptr), intent(out) :: out
    type(c_ptr), intent(in) :: alloc
    type(gt_value), pointer :: root
    type(guatiao_allocator) :: a
    a%ptr = alloc
    answer = guatiao_list(a)
    call c_f_pointer(gt_root(answer), root)
    out = c_loc(root%payload)
  end subroutine

  !> The same for a remote slot's `guatiao_value *out`: an absent value
  !> naming `alloc` (else guatiao's heap), the address of its node.
  recursive subroutine guatiao_value_out(answer, out, alloc)
    type(guatiao_value), intent(inout) :: answer
    type(c_ptr), intent(out) :: out
    type(c_ptr), intent(in) :: alloc
    type(gt_value), pointer :: root
    call gt_new_owned(answer, carrier_alloc(alloc), out)
    call c_f_pointer(out, root)
    root%payload%alloc = carrier_alloc(alloc)
  end subroutine

  !> A tree a remote slot borrows (`const guatiao_value *` or `const
  !> guatiao_map *`): for GUATIAO_TAG_MAP the map arm of the node `v`
  !> names, for GUATIAO_TAG_ABSENT the node itself, whatever it holds.
  !> GUATIAO_ERR_NULL for nothing there (a value that may be null passes
  !> a null `ptr` instead), GUATIAO_ERR_WRONG_KIND for another kind.
  recursive subroutine guatiao_borrow_tree(v, tag, ptr, stat)
    type(guatiao_value), intent(in) :: v
    integer(c_int32_t), intent(in) :: tag
    type(c_ptr), intent(out) :: ptr
    integer, intent(out) :: stat
    type(gt_value), pointer :: n
    ptr = gt_node(v)
    stat = GUATIAO_OK
    if (.not. c_associated(ptr)) then
      stat = GUATIAO_ERR_NULL
      return
    end if
    call c_f_pointer(ptr, n)
    if (n%tag == GUATIAO_TAG_ABSENT) then
      ptr = c_null_ptr
      stat = GUATIAO_ERR_NULL
      return
    end if
    if (tag == GUATIAO_TAG_ABSENT) return
    if (n%tag /= tag) then
      ptr = c_null_ptr
      stat = GUATIAO_ERR_WRONG_KIND
      return
    end if
    ptr = c_loc(n%payload)
  end subroutine

  !> Moves an answer into this tree's allocator, so it outlives the
  !> library: a slot that ignored the carrier built it in its own, wholly
  !> or in part. A core older than `guatiao_value_make_local` copies an
  !> answer whose top container records another allocator. A refusal
  !> leaves the answer as it came, and is `stat` when given: the caller
  !> then frees the answer while the library is loaded.
  recursive subroutine guatiao_settle_home(answer, stat)
    type(guatiao_value), intent(inout) :: answer
    integer, intent(out), optional :: stat
    type(gt_value), pointer :: root
    type(guatiao_value) :: home
    integer :: st
    if (present(stat)) stat = GUATIAO_OK
    if (.not. c_associated(gt_root(answer))) return
    if (associated(gt_value_make_local)) then
      st = int(gt_value_make_local(gt_root(answer), gt_box_alloc(answer)))
      if (present(stat)) stat = st
      return
    end if
    call c_f_pointer(gt_root(answer), root)
    if (.not. c_associated(root%payload%alloc)) return
    if (c_associated(root%payload%alloc, gt_box_alloc(answer))) return
    if (root%tag /= GUATIAO_TAG_MAP .and. root%tag /= GUATIAO_TAG_LIST) return
    home = answer%clone(stat=st)
    if (st == GUATIAO_OK) answer = home
    if (present(stat)) stat = st
  end subroutine

  !> A tree lent to a remote slot (see guatiao_lend_tree), once the call
  !> returned, answered or refused: moved where it stands into `home`, the
  !> allocator it recorded before the call, so what the callee grew it
  !> with, or wrote over it, outlives the callee's library; a map also
  !> loses any key the callee wrote that is not UTF-8
  !> (GUATIAO_ERR_BAD_VALUE). A refused move leaves it whole, where it was.
  !> Either is `stat` when given: the error of a call that answered, as
  !> the slot's own refusal is of one that did not (guatiao_lent_refusal
  !> words it).
  recursive subroutine guatiao_settle_lent(v, tag, home, stat)
    type(guatiao_value), intent(in) :: v
    integer(c_int32_t), intent(in) :: tag
    type(c_ptr), intent(in) :: home
    integer, intent(out), optional :: stat
    type(c_ptr) :: node, alloc
    integer :: st
    integer(c_int32_t) :: moved
    character(:), allocatable :: msg
    if (present(stat)) stat = GUATIAO_OK
    if (.not. associated(gt_value_make_local)) return
    call gt_writable_node(v, tag, node, alloc, st, msg)
    if (st /= GUATIAO_OK) return
    if (c_associated(home)) alloc = home
    if (tag == GUATIAO_TAG_MAP .and. associated(gt_map_settle_lent)) then
      moved = gt_map_settle_lent(node, alloc)
    else
      moved = gt_value_make_local(node, alloc)
    end if
    if (present(stat)) stat = int(moved)
  end subroutine

  !> The words of a call's error when guatiao_settle_lent answered `stat`
  !> for the tree lent to `slot` as `arg`.
  recursive function guatiao_lent_refusal(stat, slot, arg) result(words)
    integer, intent(in) :: stat
    character(*), intent(in) :: slot, arg
    character(:), allocatable :: words
    if (stat == GUATIAO_ERR_BAD_VALUE) then
      words = slot // ' wrote a key that is not UTF-8 into the lent map ' // arg // '; the entry was removed'
    else
      words = 'moving what ' // slot // ' put into the lent tree ' // arg // " into that tree's allocator"
    end if
  end function

  !> The error a remote slot answered: its status and the callee's words,
  !> whose string is freed through the allocator it records.
  recursive subroutine guatiao_call_error(status, err, error)
    integer(c_int32_t), intent(in) :: status
    type(gt_provider_error), intent(inout), target :: err
    type(guatiao_error), allocatable, intent(out) :: error
    type(gt_value), target :: node
    character(:), allocatable :: words
    integer(c_int32_t) :: ignored
    call gt_text_of_owned(err%message, words)
    if (err%message%cap > 0) then
      node%tag = GUATIAO_TAG_STRING
      node%payload = err%message
      ignored = gt_value_free(c_loc(node))
    end if
    err%message = gt_owned()
    if (status == GUATIAO_OK) return
    call guatiao_fail(error, int(status), words)
  end subroutine

  !> A tree lent to a remote slot (`guatiao_map *`, lent): the address of
  !> the map arm of the node `v` names, or a status saying why not; and in
  !> `home` the allocator the tree records, which guatiao_settle_lent
  !> moves it into once the call returns.
  recursive subroutine guatiao_lend_tree(v, tag, ptr, stat, home)
    type(guatiao_value), intent(in) :: v
    integer(c_int32_t), intent(in) :: tag
    type(c_ptr), intent(out) :: ptr
    integer, intent(out) :: stat
    type(c_ptr), intent(out), optional :: home
    type(gt_value), pointer :: n
    character(:), allocatable :: msg
    type(c_ptr) :: alloc
    if (present(home)) home = c_null_ptr
    call gt_writable_node(v, tag, ptr, alloc, stat, msg)
    if (stat /= GUATIAO_OK) return
    if (present(home)) home = alloc
    call c_f_pointer(ptr, n)
    ptr = c_loc(n%payload)
  end subroutine

  !> A tree handed to a remote slot: moved out of `v` (which then reads
  !> closed) when `v` owns its whole tree, else a copy; `node` holds it,
  !> and the map arm of `node` is what crosses.
  recursive subroutine guatiao_hand_over(v, tag, node, stat)
    type(guatiao_value), intent(inout) :: v
    integer(c_int32_t), intent(in) :: tag
    type(gt_value), intent(inout), target :: node
    integer, intent(out) :: stat
    type(guatiao_value) :: copy
    if (v%tag() /= tag) then
      stat = GUATIAO_ERR_WRONG_KIND
      if (v%is_absent()) stat = GUATIAO_ERR_NULL
      return
    end if
    call gt_take_tree(v, c_loc(node), stat)
    if (stat == GUATIAO_OK) return
    copy = v%clone(stat=stat)
    if (stat /= GUATIAO_OK) return
    call gt_take_tree(copy, c_loc(node), stat)
  end subroutine

end module guatiao_kinds
