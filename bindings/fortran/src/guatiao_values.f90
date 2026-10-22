! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> Values: a `guatiao_value` is a handle on a tree and a place in it.
!>
!> The tree sits in a counted box every copy of the handle shares: a
!> copy (defined assignment) takes another count, a handle going out of
!> scope (its final procedure) gives one back, and the tree is freed when
!> the last goes or when `close()` is called, after which every handle
!> reads absent and answers GUATIAO_STAT_CLOSED. The place is a path of
!> keys and positions walked again on every access, never a cached
!> address: a write beside a node can move the array it sits in.
module guatiao_values
  use, intrinsic :: iso_c_binding
  use, intrinsic :: iso_fortran_env, only: int8, int32, int64, real32, real64
  use, intrinsic :: ieee_arithmetic, only: ieee_is_finite, ieee_is_negative
  use guatiao_c
  use guatiao_native
  use guatiao_errors
  use guatiao_intake_c, only: gt_intake_path_get, gt_intake_resolve
  implicit none
  private

  public :: guatiao_value, guatiao_text
  public :: guatiao_map, guatiao_list, guatiao_null, guatiao_absent, guatiao_of, guatiao_number
  public :: guatiao_value_from_raw
  public :: GT_VIA_PATH, GT_VIA_RESOLVE, gt_via
  public :: gt_wrap, gt_same_tree, gt_set_frees, gt_new_owned, gt_borrowed, gt_end_borrow, gt_node, gt_root, &
            gt_box_alloc, gt_writable_node, gt_take_tree, gt_is_closed, gt_read_only, gt_real_text, gt_parse_int64

  !> A piece of text, for a list of keys.
  type :: guatiao_text
    character(:), allocatable :: text
  end type

  ! A step through intake's path grammar or its flat-key resolution, read
  ! again on every walk like a key.
  integer, parameter :: GT_VIA_PATH = 2, GT_VIA_RESOLVE = 3

  type :: gt_step
    logical :: is_key = .false.
    integer :: via = 0
    character(:), allocatable :: key
    integer(int64) :: index = 0    ! from 0, or below 0 counting from the end
  end type

  type :: gt_box
    type(gt_value), pointer :: owned => null()  ! the root node, when this box made it
    type(c_ptr) :: root = c_null_ptr            ! the root node's address
    type(c_ptr) :: alloc = c_null_ptr           ! what new nodes of this tree are built in
    integer :: refs = 0
    logical :: closed = .false.
    logical :: read_only = .false.
    logical :: frees = .true.                    ! the tree is this box's to free
  end type

  type :: guatiao_value
    type(gt_box), pointer, private :: box => null()
    type(gt_step), allocatable, private :: path(:)
    logical, private :: view_only = .false.
  contains
    procedure, private :: assign
    generic :: assignment(=) => assign
    final :: finalize
    procedure :: close => value_close
    procedure :: is_closed => value_is_closed
    procedure :: key => value_key
    procedure :: item => value_item
    procedure :: at => value_at
    procedure :: tag => value_tag
    procedure :: is_absent => value_is_absent
    procedure :: is_null => value_is_null
    procedure :: length => value_length
    procedure :: has => value_has
    procedure :: keys => value_keys
    procedure :: path_text => value_path_text
    procedure :: bool_or => value_bool_or
    procedure :: int_or => value_int_or
    procedure :: real_or => value_real_or
    procedure :: string_or => value_string_or
    procedure :: number_text => value_number_text
    procedure, private :: get_logical, get_int32, get_int64, get_real32, get_real64, get_text, &
                          get_bytes, get_value
    generic :: get => get_logical, get_int32, get_int64, get_real32, get_real64, get_text, &
                      get_bytes, get_value
    procedure, private :: get_key_logical, get_key_int32, get_key_int64, get_key_real32, &
                          get_key_real64, get_key_text, get_key_bytes, get_key_value
    generic :: get_key => get_key_logical, get_key_int32, get_key_int64, get_key_real32, &
                          get_key_real64, get_key_text, get_key_bytes, get_key_value
    procedure, private :: set_logical, set_int32, set_int64, set_real32, set_real64, set_text, &
                          set_bytes, set_value
    generic :: set => set_logical, set_int32, set_int64, set_real32, set_real64, set_text, &
                      set_bytes, set_value
    procedure, private :: push_logical, push_int32, push_int64, push_real32, push_real64, &
                          push_text, push_bytes, push_value
    generic :: push => push_logical, push_int32, push_int64, push_real32, push_real64, &
                       push_text, push_bytes, push_value
    procedure :: remove => value_remove
    procedure :: remove_item => value_remove_item
    procedure :: clear => value_clear
    procedure :: copy_from => value_copy_from
    procedure :: clone => value_clone
    procedure :: raw => value_raw
  end type

  interface guatiao_of
    module procedure of_logical, of_int32, of_int64, of_real32, of_real64, of_text, of_bytes
  end interface

contains

  ! ---- the box ---------------------------------------------------------

  subroutine assign(lhs, rhs)
    class(guatiao_value), intent(inout) :: lhs
    type(guatiao_value), intent(in) :: rhs
    type(gt_box), pointer :: keep
    type(gt_step), allocatable :: path(:)
    keep => rhs%box
    if (associated(keep)) keep%refs = keep%refs + 1
    if (allocated(rhs%path)) path = rhs%path
    call release(lhs)
    lhs%box => keep
    lhs%view_only = rhs%view_only
    if (allocated(path)) call move_alloc(path, lhs%path)
  end subroutine

  subroutine release(v)
    class(guatiao_value), intent(inout) :: v
    type(gt_box), pointer :: b
    b => v%box
    nullify(v%box)
    if (allocated(v%path)) deallocate(v%path)
    if (.not. associated(b)) return
    b%refs = b%refs - 1
    if (b%refs > 0) return
    call close_box(b)
    deallocate(b)
  end subroutine

  impure elemental subroutine finalize(v)
    type(guatiao_value), intent(inout) :: v
    call release(v)
  end subroutine

  subroutine close_box(b)
    type(gt_box), pointer, intent(in) :: b
    integer(c_int32_t) :: st
    if (b%closed) return
    if (b%frees .and. c_associated(b%root)) st = gt_value_free(b%root)
    if (associated(b%owned)) deallocate(b%owned)
    b%root = c_null_ptr
    b%closed = .true.
  end subroutine

  !> Frees the tree now, for every handle on it; they then read absent.
  !> Safe to call twice.
  subroutine value_close(self)
    class(guatiao_value), intent(inout) :: self
    if (associated(self%box)) call close_box(self%box)
  end subroutine

  !> Whether the tree was closed (or this handle names none).
  logical function value_is_closed(self)
    class(guatiao_value), intent(in) :: self
    value_is_closed = .true.
    if (associated(self%box)) value_is_closed = self%box%closed
  end function

  logical function gt_is_closed(v)
    type(guatiao_value), intent(in) :: v
    gt_is_closed = v%is_closed()
  end function

  logical function gt_read_only(v)
    type(guatiao_value), intent(in) :: v
    gt_read_only = .false.
    if (associated(v%box)) gt_read_only = v%box%read_only
  end function

  !> A new tree this binding owns: an ABSENT root node, built in `alloc`.
  !> Answers the root's address for a C call to write the tree into.
  subroutine gt_new_owned(v, alloc, root)
    type(guatiao_value), intent(inout) :: v
    type(c_ptr), intent(in) :: alloc
    type(c_ptr), intent(out) :: root
    type(gt_box), pointer :: b
    call release(v)
    allocate(b)
    allocate(b%owned)
    b%root = c_loc(b%owned)
    b%alloc = alloc
    b%refs = 1
    v%box => b
    allocate(v%path(0))
    root = b%root
  end subroutine

  !> A handle on a tree this binding does not own (a borrowed argument, a
  !> lent one, a provider's schema): nothing is freed when it goes, and
  !> gt_end_borrow ends it for every copy.
  function gt_borrowed(root, read_only, alloc) result(v)
    type(c_ptr), intent(in) :: root
    logical, intent(in) :: read_only
    type(c_ptr), intent(in) :: alloc
    type(guatiao_value) :: v
    type(gt_box), pointer :: b
    allocate(b)
    b%root = root
    b%alloc = alloc
    b%refs = 1
    b%frees = .false.
    b%read_only = read_only
    if (.not. c_associated(root)) b%closed = .true.
    v%box => b
    allocate(v%path(0))
  end function

  !> Ends a borrowed or wrapped handle's tree: every copy reads closed
  !> from now, and nothing of the tree is freed.
  subroutine gt_end_borrow(v)
    type(guatiao_value), intent(inout) :: v
    if (.not. associated(v%box)) return
    if (associated(v%box%owned)) deallocate(v%box%owned)
    v%box%root = c_null_ptr
    v%box%closed = .true.
  end subroutine

  !> A handle on a root node of its own holding `node`'s 40 bytes: a map
  !> arm another side passed, made a value. `frees` says whether the tree
  !> is this handle's to free (a tree handed over) or not (lent,
  !> borrowed, a carrier until it is settled).
  function gt_wrap(node, frees, read_only, alloc) result(v)
    type(gt_value), intent(in) :: node
    logical, intent(in) :: frees, read_only
    type(c_ptr), intent(in) :: alloc
    type(guatiao_value) :: v
    type(c_ptr) :: root
    type(gt_value), pointer :: n
    call gt_new_owned(v, alloc, root)
    call c_f_pointer(root, n)
    n = node
    v%box%frees = frees
    v%box%read_only = read_only
  end function

  !> Whether two handles name the root of one tree.
  logical function gt_same_tree(a, b)
    type(guatiao_value), intent(in) :: a, b
    gt_same_tree = .false.
    if (.not. associated(a%box) .or. .not. associated(b%box)) return
    if (.not. associated(a%box, b%box)) return
    if (.not. allocated(a%path) .or. .not. allocated(b%path)) return
    gt_same_tree = size(a%path) == 0 .and. size(b%path) == 0
  end function

  !> Makes the tree this handle's to free, or not.
  subroutine gt_set_frees(v, frees)
    type(guatiao_value), intent(inout) :: v
    logical, intent(in) :: frees
    if (associated(v%box)) v%box%frees = frees
  end subroutine

  !> The root of the tree this handle is on, or null.
  function gt_root(v) result(p)
    type(guatiao_value), intent(in) :: v
    type(c_ptr) :: p
    p = c_null_ptr
    if (.not. associated(v%box)) return
    if (v%box%closed) return
    p = v%box%root
  end function

  !> The allocator the tree records for new nodes.
  function gt_box_alloc(v) result(p)
    type(guatiao_value), intent(in) :: v
    type(c_ptr) :: p
    p = gt_default_alloc()
    if (associated(v%box)) then
      if (c_associated(v%box%alloc)) p = v%box%alloc
    end if
  end function

  !> Moves the whole tree out of a handle this binding owns: the node at
  !> `dst` receives its 40 bytes and every handle then reads closed.
  !> GUATIAO_STAT_READ_ONLY for a borrowed tree, which is not this
  !> handle's to give.
  subroutine gt_take_tree(v, dst, stat)
    type(guatiao_value), intent(inout) :: v
    type(c_ptr), intent(in) :: dst
    integer, intent(out) :: stat
    type(gt_value), pointer :: from, to
    type(gt_value) :: none
    if (.not. associated(v%box)) then
      stat = GUATIAO_STAT_CLOSED
      return
    end if
    if (v%box%closed) then
      stat = GUATIAO_STAT_CLOSED
      return
    end if
    if (.not. v%box%frees .or. v%box%read_only .or. v%view_only .or. size(v%path) > 0) then
      stat = GUATIAO_STAT_READ_ONLY
      return
    end if
    call c_f_pointer(v%box%root, from)
    call c_f_pointer(dst, to)
    to = from
    from = none
    stat = GUATIAO_OK
    call close_box(v%box)
  end subroutine

  !> Takes over the tree at `node` (a `guatiao_value` a C function wrote):
  !> its 40 bytes move into a value this binding owns, and `node` is
  !> written ABSENT, so freeing it afterwards frees nothing.
  function guatiao_value_from_raw(node, alloc) result(v)
    type(c_ptr), intent(in) :: node
    type(guatiao_allocator), intent(in), optional :: alloc
    type(guatiao_value) :: v
    type(gt_value), pointer :: from, to
    type(gt_value) :: none
    type(c_ptr) :: root
    call gt_new_owned(v, gt_alloc_of(alloc), root)
    if (.not. c_associated(node)) return
    call c_f_pointer(node, from)
    call c_f_pointer(root, to)
    to = from
    from = none
  end function

  ! ---- walking ---------------------------------------------------------

  !> The address of the node this handle names now, or null.
  function gt_node(v) result(p)
    type(guatiao_value), intent(in) :: v
    type(c_ptr) :: p
    type(gt_value), pointer :: n
    type(gt_entry), pointer :: entries(:)
    type(gt_value), pointer :: items(:)
    integer :: i
    integer(c_size_t) :: j
    integer(int64) :: at
    character(:), allocatable, target :: text
    p = gt_root(v)
    if (.not. c_associated(p)) return
    if (.not. allocated(v%path)) return
    do i = 1, size(v%path)
      if (v%path(i)%via == GT_VIA_PATH .or. v%path(i)%via == GT_VIA_RESOLVE) then
        text = v%path(i)%key
        if (v%path(i)%via == GT_VIA_PATH) then
          p = gt_intake_path_get(p, gt_view(text))
        else
          p = gt_intake_resolve(p, gt_view(text))
        end if
        if (.not. c_associated(p)) return
        cycle
      end if
      call c_f_pointer(p, n)
      p = c_null_ptr
      if (v%path(i)%is_key) then
        if (n%tag /= GUATIAO_TAG_MAP .or. n%payload%len == 0) return
        call c_f_pointer(n%payload%ptr, entries, [n%payload%len])
        do j = 1, n%payload%len
          if (gt_text_equal(entries(j)%key%ptr, entries(j)%key%len, v%path(i)%key)) then
            p = c_loc(entries(j)%value)
            exit
          end if
        end do
        if (.not. c_associated(p)) return
      else
        if (n%tag /= GUATIAO_TAG_LIST) return
        at = v%path(i)%index
        if (at < 0) at = at + int(n%payload%len, int64)
        if (at < 0 .or. at >= int(n%payload%len, int64)) return
        call c_f_pointer(n%payload%ptr, items, [n%payload%len])
        p = c_loc(items(at + 1))
      end if
    end do
  end function

  !> A read-only handle reaching on from `v` through intake's path
  !> grammar (GT_VIA_PATH) or its flat-key resolution (GT_VIA_RESOLVE).
  function gt_via(v, via, text) result(r)
    type(guatiao_value), intent(in) :: v
    integer, intent(in) :: via
    character(*), intent(in) :: text
    type(guatiao_value) :: r
    type(gt_step) :: s
    s%via = via
    s%key = text
    r = extended(v, s)
    r%view_only = .true.
  end function

  function extended(self, step) result(r)
    class(guatiao_value), intent(in) :: self
    type(gt_step), intent(in) :: step
    type(guatiao_value) :: r
    integer :: n
    r%box => self%box
    if (associated(r%box)) r%box%refs = r%box%refs + 1
    r%view_only = self%view_only
    n = 0
    if (allocated(self%path)) n = size(self%path)
    allocate(r%path(n + 1))
    if (n > 0) r%path(1:n) = self%path
    r%path(n + 1) = step
  end function

  !> The member `k` of the map this names: a handle, checked on access.
  function value_key(self, k) result(r)
    class(guatiao_value), intent(in) :: self
    character(*), intent(in) :: k
    type(guatiao_value) :: r
    type(gt_step) :: s
    s%is_key = .true.
    s%key = k
    r = extended(self, s)
  end function

  !> The element `i` of the list this names, from 1; below 1 counts from
  !> the end (0 the last, -1 the one before).
  function value_item(self, i) result(r)
    class(guatiao_value), intent(in) :: self
    integer, intent(in) :: i
    type(guatiao_value) :: r
    type(gt_step) :: s
    s%index = int(i, int64) - 1
    r = extended(self, s)
  end function

  !> Several steps at once: each a key (text) or a position (an integer,
  !> as item takes it).
  function value_at(self, s1, s2, s3, s4, s5, s6, s7, s8) result(r)
    class(guatiao_value), intent(in) :: self
    class(*), intent(in) :: s1
    class(*), intent(in), optional :: s2, s3, s4, s5, s6, s7, s8
    type(guatiao_value) :: r
    r = self
    call step(r, s1)
    if (present(s2)) call step(r, s2)
    if (present(s3)) call step(r, s3)
    if (present(s4)) call step(r, s4)
    if (present(s5)) call step(r, s5)
    if (present(s6)) call step(r, s6)
    if (present(s7)) call step(r, s7)
    if (present(s8)) call step(r, s8)
  contains
    subroutine step(v, s)
      type(guatiao_value), intent(inout) :: v
      class(*), intent(in) :: s
      select type (s)
      type is (character(*))
        v = v%key(s)
      type is (integer(int32))
        v = v%item(s)
      type is (integer(int64))
        v = v%item(int(s))
      class default
        error stop 'guatiao: a step is text or an integer'
      end select
    end subroutine
  end function

  ! ---- reading ---------------------------------------------------------

  !> The node's tag (GUATIAO_TAG_ABSENT when the path names nothing or the
  !> tree is closed).
  integer function value_tag(self)
    class(guatiao_value), intent(in) :: self
    type(c_ptr) :: p
    type(gt_value), pointer :: n
    value_tag = GUATIAO_TAG_ABSENT
    p = gt_node(self)
    if (.not. c_associated(p)) return
    call c_f_pointer(p, n)
    value_tag = n%tag
  end function

  logical function value_is_absent(self)
    class(guatiao_value), intent(in) :: self
    value_is_absent = self%tag() == GUATIAO_TAG_ABSENT
  end function

  logical function value_is_null(self)
    class(guatiao_value), intent(in) :: self
    value_is_null = self%tag() == GUATIAO_TAG_NULL
  end function

  !> Entries of a map, elements of a list, bytes of a string, number or
  !> byte string; 0 for anything else.
  integer function value_length(self)
    class(guatiao_value), intent(in) :: self
    type(c_ptr) :: p
    type(gt_value), pointer :: n
    value_length = 0
    p = gt_node(self)
    if (.not. c_associated(p)) return
    call c_f_pointer(p, n)
    select case (n%tag)
    case (GUATIAO_TAG_MAP, GUATIAO_TAG_LIST, GUATIAO_TAG_STRING, GUATIAO_TAG_NUMBER, GUATIAO_TAG_BYTES)
      value_length = int(n%payload%len)
    end select
  end function

  !> Whether the map this names has `k`.
  logical function value_has(self, k)
    class(guatiao_value), intent(in) :: self
    character(*), intent(in) :: k
    type(guatiao_value) :: r
    r = self%key(k)
    value_has = .not. r%is_absent()
  end function

  !> The keys of the map this names, in insertion order (none for
  !> anything else).
  function value_keys(self) result(keys)
    class(guatiao_value), intent(in) :: self
    type(guatiao_text), allocatable :: keys(:)
    type(c_ptr) :: p
    type(gt_value), pointer :: n
    type(gt_entry), pointer :: entries(:)
    integer(c_size_t) :: j
    allocate(keys(0))
    p = gt_node(self)
    if (.not. c_associated(p)) return
    call c_f_pointer(p, n)
    if (n%tag /= GUATIAO_TAG_MAP .or. n%payload%len == 0) return
    call c_f_pointer(n%payload%ptr, entries, [n%payload%len])
    deallocate(keys)
    allocate(keys(n%payload%len))
    do j = 1, n%payload%len
      call gt_text_of_owned(entries(j)%key, keys(j)%text)
    end do
  end function

  !> The path in the intake grammar (`agent[1].name`, positions from 0).
  function value_path_text(self) result(text)
    class(guatiao_value), intent(in) :: self
    character(:), allocatable :: text
    character(24) :: buf
    integer :: i
    text = ''
    if (.not. allocated(self%path)) return
    do i = 1, size(self%path)
      if (self%path(i)%via /= 0) then
        if (len(text) > 0 .and. self%path(i)%via == GT_VIA_PATH) text = text // '.'
        text = text // self%path(i)%key
      else if (.not. self%path(i)%is_key) then
        write (buf, '(i0)') self%path(i)%index
        text = text // '[' // trim(buf) // ']'
      else if (plain(self%path(i)%key)) then
        if (len(text) > 0) text = text // '.'
        text = text // self%path(i)%key
      else
        text = text // '[' // quoted(self%path(i)%key) // ']'
      end if
    end do
  contains
    logical function plain(k)
      character(*), intent(in) :: k
      integer :: c
      plain = len(k) > 0
      do c = 1, len(k)
        select case (k(c:c))
        case ('a':'z', 'A':'Z', '0':'9', '_', '-')
        case default
          plain = .false.
        end select
      end do
    end function
    function quoted(k) result(q)
      character(*), intent(in) :: k
      character(:), allocatable :: q
      integer :: c
      q = '"'
      do c = 1, len(k)
        if (k(c:c) == '"' .or. k(c:c) == '\') q = q // '\'
        q = q // k(c:c)
      end do
      q = q // '"'
    end function
  end function

  logical function value_bool_or(self, fallback)
    class(guatiao_value), intent(in) :: self
    logical, intent(in) :: fallback
    integer :: st
    call self%get(value_bool_or, st)
    if (st /= GUATIAO_OK) value_bool_or = fallback
  end function

  !> A number as an int64, or `fallback`: never truncated, never rounded;
  !> a fraction, an exponent, another kind or out of range falls back.
  integer(int64) function value_int_or(self, fallback)
    class(guatiao_value), intent(in) :: self
    integer(int64), intent(in) :: fallback
    integer :: st
    call self%get(value_int_or, st)
    if (st /= GUATIAO_OK) value_int_or = fallback
  end function

  !> A number as the nearest real64, or `fallback` when there is no
  !> finite one.
  real(real64) function value_real_or(self, fallback)
    class(guatiao_value), intent(in) :: self
    real(real64), intent(in) :: fallback
    integer :: st
    call self%get(value_real_or, st)
    if (st /= GUATIAO_OK) value_real_or = fallback
  end function

  !> A string's text, or `fallback` for anything else (a number included).
  function value_string_or(self, fallback) result(s)
    class(guatiao_value), intent(in) :: self
    character(*), intent(in) :: fallback
    character(:), allocatable :: s
    integer :: st
    call self%get(s, st)
    if (st /= GUATIAO_OK) s = fallback
  end function

  !> A number's exact text, or '' for anything else.
  function value_number_text(self) result(s)
    class(guatiao_value), intent(in) :: self
    character(:), allocatable :: s
    type(c_ptr) :: p
    type(gt_value), pointer :: n
    s = ''
    p = gt_node(self)
    if (.not. c_associated(p)) return
    call c_f_pointer(p, n)
    if (n%tag == GUATIAO_TAG_NUMBER) call gt_text_of_owned(n%payload, s)
  end function

  ! The node, checked to be `tag`: GUATIAO_ERR_NOT_FOUND when absent,
  ! GUATIAO_STAT_CLOSED after close, GUATIAO_ERR_WRONG_KIND otherwise.
  subroutine reading(self, tag, n, st, msg)
    class(guatiao_value), intent(in) :: self
    integer(c_int32_t), intent(in) :: tag
    type(gt_value), pointer, intent(out) :: n
    integer, intent(out) :: st
    character(:), allocatable, intent(out) :: msg
    type(c_ptr) :: p
    nullify(n)
    msg = ''
    if (self%is_closed()) then
      st = GUATIAO_STAT_CLOSED
      msg = 'guatiao: the tree is closed'
      return
    end if
    p = gt_node(self)
    if (.not. c_associated(p)) then
      st = GUATIAO_ERR_NOT_FOUND
      msg = 'guatiao: nothing at ' // where(self)
      return
    end if
    call c_f_pointer(p, n)
    if (n%tag /= tag) then
      st = GUATIAO_ERR_WRONG_KIND
      msg = 'guatiao: at ' // where(self) // ': want ' // tag_name(tag) // ', got ' // tag_name(n%tag)
      return
    end if
    st = GUATIAO_OK
  end subroutine

  function where(v) result(w)
    class(guatiao_value), intent(in) :: v
    character(:), allocatable :: w
    w = v%path_text()
    if (len(w) == 0) w = 'the root'
  end function

  function tag_name(tag) result(name)
    integer(c_int32_t), intent(in) :: tag
    character(:), allocatable :: name
    select case (tag)
    case (GUATIAO_TAG_ABSENT); name = 'nothing'
    case (GUATIAO_TAG_NULL); name = 'null'
    case (GUATIAO_TAG_BOOL); name = 'a boolean'
    case (GUATIAO_TAG_NUMBER); name = 'a number'
    case (GUATIAO_TAG_STRING); name = 'a string'
    case (GUATIAO_TAG_BYTES); name = 'bytes'
    case (GUATIAO_TAG_LIST); name = 'a list'
    case (GUATIAO_TAG_MAP); name = 'a map'
    case default; name = 'an unknown kind'
    end select
  end function

  subroutine get_logical(self, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    logical, intent(inout) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_value), pointer :: n
    integer :: st
    character(:), allocatable :: msg
    call reading(self, GUATIAO_TAG_BOOL, n, st, msg)
    if (st == GUATIAO_OK) x = iand(transfer(n%payload%ptr, 0_c_intptr_t), 255_c_intptr_t) /= 0
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  subroutine get_int64(self, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    integer(int64), intent(inout) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_value), pointer :: n
    integer :: st
    character(:), allocatable :: msg, text
    integer(int64) :: got
    logical :: ok
    call reading(self, GUATIAO_TAG_NUMBER, n, st, msg)
    if (st == GUATIAO_OK) then
      call gt_text_of_owned(n%payload, text)
      call gt_parse_int64(text, got, ok)
      if (ok) then
        x = got
      else
        st = GUATIAO_ERR_BAD_VALUE
        msg = 'guatiao: at ' // where(self) // ': ' // text // ' is not a 64-bit integer'
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  subroutine get_int32(self, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    integer(int32), intent(inout) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    integer(int64) :: wide
    integer :: st
    character(:), allocatable :: msg
    wide = 0
    call get_int64(self, wide, st, msg)
    if (st == GUATIAO_OK) then
      if (wide < -huge(x) - 1_int64 .or. wide > huge(x)) then
        st = GUATIAO_ERR_BAD_VALUE
        msg = 'guatiao: at ' // where(self) // ': the number does not fit a 32-bit integer'
      else
        x = int(wide, int32)
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  subroutine get_real64(self, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    real(real64), intent(inout) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_value), pointer :: n
    integer :: st, ios
    character(:), allocatable :: msg, text
    real(real64) :: got
    call reading(self, GUATIAO_TAG_NUMBER, n, st, msg)
    if (st == GUATIAO_OK) then
      call gt_text_of_owned(n%payload, text)
      read (text, *, iostat=ios) got
      if (ios == 0) then
        if (.not. ieee_is_finite(got)) ios = 1
      end if
      if (ios == 0) then
        x = got
      else
        st = GUATIAO_ERR_BAD_VALUE
        msg = 'guatiao: at ' // where(self) // ': ' // text // ' has no finite real value'
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  subroutine get_real32(self, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    real(real32), intent(inout) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_value), pointer :: n
    integer :: st, ios
    character(:), allocatable :: msg, text
    real(real32) :: got
    call reading(self, GUATIAO_TAG_NUMBER, n, st, msg)
    if (st == GUATIAO_OK) then
      call gt_text_of_owned(n%payload, text)
      read (text, *, iostat=ios) got
      if (ios == 0) then
        if (.not. ieee_is_finite(got)) ios = 1
      end if
      if (ios == 0) then
        x = got
      else
        st = GUATIAO_ERR_BAD_VALUE
        msg = 'guatiao: at ' // where(self) // ': ' // text // ' has no finite real value'
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> A string's text (a number is not a string: GUATIAO_ERR_WRONG_KIND).
  subroutine get_text(self, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    character(:), allocatable, intent(inout) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_value), pointer :: n
    integer :: st
    character(:), allocatable :: msg
    call reading(self, GUATIAO_TAG_STRING, n, st, msg)
    if (st == GUATIAO_OK) call gt_text_of_owned(n%payload, x)
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> A byte string, copied.
  subroutine get_bytes(self, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    integer(int8), allocatable, intent(inout) :: x(:)
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_value), pointer :: n
    integer(c_int8_t), pointer :: b(:)
    integer :: st
    character(:), allocatable :: msg
    call reading(self, GUATIAO_TAG_BYTES, n, st, msg)
    if (st == GUATIAO_OK) then
      if (allocated(x)) deallocate(x)
      allocate(x(n%payload%len))
      if (n%payload%len > 0) then
        call c_f_pointer(n%payload%ptr, b, [n%payload%len])
        x = b
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> A deep copy of the node into a tree of its own.
  subroutine get_value(self, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    type(guatiao_value), intent(inout) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    integer :: st
    character(:), allocatable :: msg
    call clone_into(self, gt_box_alloc(self), x, st, msg)
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  subroutine get_key_logical(self, k, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    character(*), intent(in) :: k
    logical, intent(inout) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_value) :: r
    r = self%key(k)
    call r%get(x, stat, errmsg)
  end subroutine

  subroutine get_key_int32(self, k, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    character(*), intent(in) :: k
    integer(int32), intent(inout) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_value) :: r
    r = self%key(k)
    call r%get(x, stat, errmsg)
  end subroutine

  subroutine get_key_int64(self, k, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    character(*), intent(in) :: k
    integer(int64), intent(inout) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_value) :: r
    r = self%key(k)
    call r%get(x, stat, errmsg)
  end subroutine

  subroutine get_key_real32(self, k, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    character(*), intent(in) :: k
    real(real32), intent(inout) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_value) :: r
    r = self%key(k)
    call r%get(x, stat, errmsg)
  end subroutine

  subroutine get_key_real64(self, k, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    character(*), intent(in) :: k
    real(real64), intent(inout) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_value) :: r
    r = self%key(k)
    call r%get(x, stat, errmsg)
  end subroutine

  subroutine get_key_text(self, k, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    character(*), intent(in) :: k
    character(:), allocatable, intent(inout) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_value) :: r
    r = self%key(k)
    call r%get(x, stat, errmsg)
  end subroutine

  subroutine get_key_bytes(self, k, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    character(*), intent(in) :: k
    integer(int8), allocatable, intent(inout) :: x(:)
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_value) :: r
    r = self%key(k)
    call r%get(x, stat, errmsg)
  end subroutine

  subroutine get_key_value(self, k, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    character(*), intent(in) :: k
    type(guatiao_value), intent(inout) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_value) :: r
    r = self%key(k)
    call r%get(x, stat, errmsg)
  end subroutine

  !> A deep copy into a tree of its own, built in `alloc` (else the
  !> allocator this tree records).
  function value_clone(self, alloc, stat, errmsg) result(copy)
    class(guatiao_value), intent(in) :: self
    type(guatiao_allocator), intent(in), optional :: alloc
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_value) :: copy
    integer :: st
    character(:), allocatable :: msg
    type(c_ptr) :: a
    a = gt_box_alloc(self)
    if (present(alloc)) then
      if (c_associated(alloc%ptr)) a = alloc%ptr
    end if
    call clone_into(self, a, copy, st, msg)
    call gt_report(st, msg, stat, errmsg)
  end function

  subroutine clone_into(self, alloc, copy, st, msg)
    class(guatiao_value), intent(in) :: self
    type(c_ptr), intent(in) :: alloc
    type(guatiao_value), intent(inout) :: copy
    integer, intent(out) :: st
    character(:), allocatable, intent(out) :: msg
    type(c_ptr) :: p, root
    msg = ''
    if (self%is_closed()) then
      st = GUATIAO_STAT_CLOSED
      msg = 'guatiao: the tree is closed'
      return
    end if
    p = gt_node(self)
    if (.not. c_associated(p)) then
      st = GUATIAO_ERR_NOT_FOUND
      msg = 'guatiao: nothing at ' // where(self)
      return
    end if
    call gt_new_owned(copy, alloc, root)
    st = gt_value_clone(alloc, p, root)
    if (st /= GUATIAO_OK) msg = 'guatiao: the copy was refused'
  end subroutine

  !> The node's address now, for a C function taking one; null when the
  !> path names nothing.
  function value_raw(self) result(p)
    class(guatiao_value), intent(in) :: self
    type(c_ptr) :: p
    p = gt_node(self)
  end function

  ! ---- building --------------------------------------------------------

  !> An empty map, built in `alloc` (else guatiao's own heap).
  function guatiao_map(alloc, stat, errmsg) result(v)
    type(guatiao_allocator), intent(in), optional :: alloc
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_value) :: v
    integer :: st
    character(:), allocatable :: msg
    type(c_ptr) :: root
    call gt_need(GT_CORE, st, msg)
    if (st == GUATIAO_OK) then
      call gt_new_owned(v, gt_alloc_of(alloc), root)
      st = gt_value_map(gt_alloc_of(alloc), root)
      if (st /= GUATIAO_OK) msg = 'guatiao: no map from that allocator'
    end if
    call gt_report(st, msg, stat, errmsg)
  end function

  !> An empty list, built in `alloc` (else guatiao's own heap).
  function guatiao_list(alloc, stat, errmsg) result(v)
    type(guatiao_allocator), intent(in), optional :: alloc
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_value) :: v
    integer :: st
    character(:), allocatable :: msg
    type(c_ptr) :: root
    call gt_need(GT_CORE, st, msg)
    if (st == GUATIAO_OK) then
      call gt_new_owned(v, gt_alloc_of(alloc), root)
      st = gt_value_list(gt_alloc_of(alloc), root)
      if (st /= GUATIAO_OK) msg = 'guatiao: no list from that allocator'
    end if
    call gt_report(st, msg, stat, errmsg)
  end function

  !> The stored nothing.
  function guatiao_null() result(v)
    type(guatiao_value) :: v
    type(c_ptr) :: root
    type(gt_value), pointer :: n
    call gt_new_owned(v, c_null_ptr, root)
    call c_f_pointer(root, n)
    n%tag = GUATIAO_TAG_NULL
  end function

  !> No value: what a lookup that found nothing answers.
  function guatiao_absent() result(v)
    type(guatiao_value) :: v
    type(c_ptr) :: root
    call gt_new_owned(v, c_null_ptr, root)
  end function

  !> A number stored as its exact `text` (JSON's grammar, else
  !> GUATIAO_ERR_BAD_VALUE).
  function guatiao_number(text, alloc, stat, errmsg) result(v)
    character(*), intent(in), target :: text
    type(guatiao_allocator), intent(in), optional :: alloc
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_value) :: v
    integer :: st
    character(:), allocatable :: msg
    type(c_ptr) :: root
    call gt_need(GT_CORE, st, msg)
    if (st == GUATIAO_OK) then
      call gt_new_owned(v, gt_alloc_of(alloc), root)
      st = gt_value_number(gt_alloc_of(alloc), gt_view(text), root)
      if (st /= GUATIAO_OK) msg = 'guatiao: "' // text // '" is not a number'
    end if
    call gt_report(st, msg, stat, errmsg)
  end function

  ! The node a write goes to, checked to be `tag`, and the allocator to
  ! grow it through: the one the container records, else the tree's.
  subroutine gt_writable_node(self, tag, p, alloc, st, msg)
    class(guatiao_value), intent(in) :: self
    integer(c_int32_t), intent(in) :: tag
    type(c_ptr), intent(out) :: p, alloc
    integer, intent(out) :: st
    character(:), allocatable, intent(out) :: msg
    type(gt_value), pointer :: n
    p = c_null_ptr
    alloc = c_null_ptr
    call reading(self, tag, n, st, msg)
    if (st /= GUATIAO_OK) return
    if (self%box%read_only .or. self%view_only) then
      st = GUATIAO_STAT_READ_ONLY
      msg = 'guatiao: a borrowed view is read-only'
      return
    end if
    p = gt_node(self)
    alloc = n%payload%alloc
    if (.not. c_associated(alloc)) alloc = gt_box_alloc(self)
  end subroutine

  ! Builds the node `tmp` from a Fortran scalar, through `alloc`.
  subroutine build_text(alloc, x, tmp, st, msg)
    type(c_ptr), intent(in) :: alloc
    character(*), intent(in), target :: x
    type(gt_value), intent(inout), target :: tmp
    integer, intent(out) :: st
    character(:), allocatable, intent(out) :: msg
    msg = ''
    st = gt_value_string(alloc, gt_view(x), c_loc(tmp))
    if (st /= GUATIAO_OK) msg = 'guatiao: the text is not UTF-8, or the allocator refused'
  end subroutine

  subroutine build_number(alloc, text, tmp, st, msg)
    type(c_ptr), intent(in) :: alloc
    character(*), intent(in), target :: text
    type(gt_value), intent(inout), target :: tmp
    integer, intent(out) :: st
    character(:), allocatable, intent(out) :: msg
    msg = ''
    st = gt_value_number(alloc, gt_view(text), c_loc(tmp))
    if (st /= GUATIAO_OK) msg = 'guatiao: "' // text // '" is not a number'
  end subroutine

  subroutine build_int(alloc, x, tmp, st, msg)
    type(c_ptr), intent(in) :: alloc
    integer(int64), intent(in) :: x
    type(gt_value), intent(inout), target :: tmp
    integer, intent(out) :: st
    character(:), allocatable, intent(out) :: msg
    character(24) :: buf
    write (buf, '(i0)') x
    call build_number(alloc, trim(buf), tmp, st, msg)
  end subroutine

  subroutine build_real(alloc, x, single, tmp, st, msg)
    type(c_ptr), intent(in) :: alloc
    real(real64), intent(in) :: x
    logical, intent(in) :: single
    type(gt_value), intent(inout), target :: tmp
    integer, intent(out) :: st
    character(:), allocatable, intent(out) :: msg
    character(:), allocatable :: text
    logical :: ok
    call gt_real_text(x, single, text, ok)
    if (.not. ok) then
      st = GUATIAO_ERR_BAD_VALUE
      msg = 'guatiao: NaN and the infinities have no number form'
      return
    end if
    call build_number(alloc, text, tmp, st, msg)
  end subroutine

  subroutine build_logical(x, tmp, st, msg)
    logical, intent(in) :: x
    type(gt_value), intent(inout), target :: tmp
    integer, intent(out) :: st
    character(:), allocatable, intent(out) :: msg
    msg = ''
    st = gt_value_bool(logical(x, c_bool), c_loc(tmp))
  end subroutine

  subroutine build_bytes(alloc, x, tmp, st, msg)
    type(c_ptr), intent(in) :: alloc
    integer(int8), intent(in), target :: x(:)
    type(gt_value), intent(inout), target :: tmp
    integer, intent(out) :: st
    character(:), allocatable, intent(out) :: msg
    msg = ''
    st = gt_value_bytes(alloc, gt_bytes_view(x), c_loc(tmp))
    if (st /= GUATIAO_OK) msg = 'guatiao: the allocator refused the bytes'
  end subroutine

  subroutine build_copy(alloc, x, tmp, st, msg)
    type(c_ptr), intent(in) :: alloc
    type(guatiao_value), intent(in) :: x
    type(gt_value), intent(inout), target :: tmp
    integer, intent(out) :: st
    character(:), allocatable, intent(out) :: msg
    type(c_ptr) :: p
    msg = ''
    if (x%is_closed()) then
      st = GUATIAO_STAT_CLOSED
      msg = 'guatiao: the source tree is closed'
      return
    end if
    p = gt_node(x)
    if (.not. c_associated(p)) then
      st = GUATIAO_ERR_NOT_FOUND
      msg = 'guatiao: nothing at the source'
      return
    end if
    st = gt_value_clone(alloc, p, c_loc(tmp))
    if (st /= GUATIAO_OK) msg = 'guatiao: the copy was refused'
  end subroutine

  ! Stores `tmp` under `k` of the map at `p`, freeing it when refused.
  subroutine store(p, alloc, k, tmp, st, msg)
    type(c_ptr), intent(in) :: p, alloc
    character(*), intent(in), target :: k
    type(gt_value), intent(inout), target :: tmp
    integer, intent(inout) :: st
    character(:), allocatable, intent(inout) :: msg
    integer(c_int32_t) :: ignored
    if (st == GUATIAO_OK) then
      st = gt_map_set(alloc, p, gt_view(k), c_loc(tmp))
      if (st /= GUATIAO_OK) msg = 'guatiao: the map refused "' // k // '"'
    end if
    if (st /= GUATIAO_OK) ignored = gt_value_free(c_loc(tmp))
  end subroutine

  ! Appends `tmp` to the list at `p`, freeing it when refused.
  subroutine append(p, alloc, tmp, st, msg)
    type(c_ptr), intent(in) :: p, alloc
    type(gt_value), intent(inout), target :: tmp
    integer, intent(inout) :: st
    character(:), allocatable, intent(inout) :: msg
    integer(c_int32_t) :: ignored
    if (st == GUATIAO_OK) then
      st = gt_list_push(alloc, p, c_loc(tmp))
      if (st /= GUATIAO_OK) msg = 'guatiao: the list refused the element'
    end if
    if (st /= GUATIAO_OK) ignored = gt_value_free(c_loc(tmp))
  end subroutine

  !> Stores `x` under `k` of the map this names, replacing in place.
  subroutine set_logical(self, k, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    character(*), intent(in), target :: k
    logical, intent(in) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_value), target :: tmp
    type(c_ptr) :: p, alloc
    integer :: st
    character(:), allocatable :: msg
    call gt_writable_node(self, GUATIAO_TAG_MAP, p, alloc, st, msg)
    if (st == GUATIAO_OK) then
      call build_logical(x, tmp, st, msg)
      call store(p, alloc, k, tmp, st, msg)
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  subroutine set_int32(self, k, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    character(*), intent(in), target :: k
    integer(int32), intent(in) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    call set_int64(self, k, int(x, int64), stat, errmsg)
  end subroutine

  subroutine set_int64(self, k, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    character(*), intent(in), target :: k
    integer(int64), intent(in) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_value), target :: tmp
    type(c_ptr) :: p, alloc
    integer :: st
    character(:), allocatable :: msg
    call gt_writable_node(self, GUATIAO_TAG_MAP, p, alloc, st, msg)
    if (st == GUATIAO_OK) then
      call build_int(alloc, x, tmp, st, msg)
      call store(p, alloc, k, tmp, st, msg)
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  subroutine set_real32(self, k, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    character(*), intent(in), target :: k
    real(real32), intent(in) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_value), target :: tmp
    type(c_ptr) :: p, alloc
    integer :: st
    character(:), allocatable :: msg
    call gt_writable_node(self, GUATIAO_TAG_MAP, p, alloc, st, msg)
    if (st == GUATIAO_OK) then
      call build_real(alloc, real(x, real64), .true., tmp, st, msg)
      call store(p, alloc, k, tmp, st, msg)
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  subroutine set_real64(self, k, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    character(*), intent(in), target :: k
    real(real64), intent(in) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_value), target :: tmp
    type(c_ptr) :: p, alloc
    integer :: st
    character(:), allocatable :: msg
    call gt_writable_node(self, GUATIAO_TAG_MAP, p, alloc, st, msg)
    if (st == GUATIAO_OK) then
      call build_real(alloc, x, .false., tmp, st, msg)
      call store(p, alloc, k, tmp, st, msg)
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  subroutine set_text(self, k, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    character(*), intent(in), target :: k
    character(*), intent(in) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_value), target :: tmp
    type(c_ptr) :: p, alloc
    integer :: st
    character(:), allocatable :: msg
    call gt_writable_node(self, GUATIAO_TAG_MAP, p, alloc, st, msg)
    if (st == GUATIAO_OK) then
      call build_text(alloc, x, tmp, st, msg)
      call store(p, alloc, k, tmp, st, msg)
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  subroutine set_bytes(self, k, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    character(*), intent(in), target :: k
    integer(int8), intent(in) :: x(:)
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_value), target :: tmp
    type(c_ptr) :: p, alloc
    integer :: st
    character(:), allocatable :: msg
    call gt_writable_node(self, GUATIAO_TAG_MAP, p, alloc, st, msg)
    if (st == GUATIAO_OK) then
      call build_bytes(alloc, x, tmp, st, msg)
      call store(p, alloc, k, tmp, st, msg)
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> Stores a deep copy of what `x` names (the source is left as it was).
  subroutine set_value(self, k, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    character(*), intent(in), target :: k
    type(guatiao_value), intent(in) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_value), target :: tmp
    type(c_ptr) :: p, alloc
    integer :: st
    character(:), allocatable :: msg
    call gt_writable_node(self, GUATIAO_TAG_MAP, p, alloc, st, msg)
    if (st == GUATIAO_OK) then
      call build_copy(alloc, x, tmp, st, msg)
      call store(p, alloc, k, tmp, st, msg)
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> Appends `x` to the list this names.
  subroutine push_logical(self, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    logical, intent(in) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_value), target :: tmp
    type(c_ptr) :: p, alloc
    integer :: st
    character(:), allocatable :: msg
    call gt_writable_node(self, GUATIAO_TAG_LIST, p, alloc, st, msg)
    if (st == GUATIAO_OK) then
      call build_logical(x, tmp, st, msg)
      call append(p, alloc, tmp, st, msg)
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  subroutine push_int32(self, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    integer(int32), intent(in) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    call push_int64(self, int(x, int64), stat, errmsg)
  end subroutine

  subroutine push_int64(self, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    integer(int64), intent(in) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_value), target :: tmp
    type(c_ptr) :: p, alloc
    integer :: st
    character(:), allocatable :: msg
    call gt_writable_node(self, GUATIAO_TAG_LIST, p, alloc, st, msg)
    if (st == GUATIAO_OK) then
      call build_int(alloc, x, tmp, st, msg)
      call append(p, alloc, tmp, st, msg)
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  subroutine push_real32(self, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    real(real32), intent(in) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_value), target :: tmp
    type(c_ptr) :: p, alloc
    integer :: st
    character(:), allocatable :: msg
    call gt_writable_node(self, GUATIAO_TAG_LIST, p, alloc, st, msg)
    if (st == GUATIAO_OK) then
      call build_real(alloc, real(x, real64), .true., tmp, st, msg)
      call append(p, alloc, tmp, st, msg)
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  subroutine push_real64(self, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    real(real64), intent(in) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_value), target :: tmp
    type(c_ptr) :: p, alloc
    integer :: st
    character(:), allocatable :: msg
    call gt_writable_node(self, GUATIAO_TAG_LIST, p, alloc, st, msg)
    if (st == GUATIAO_OK) then
      call build_real(alloc, x, .false., tmp, st, msg)
      call append(p, alloc, tmp, st, msg)
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  subroutine push_text(self, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    character(*), intent(in) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_value), target :: tmp
    type(c_ptr) :: p, alloc
    integer :: st
    character(:), allocatable :: msg
    call gt_writable_node(self, GUATIAO_TAG_LIST, p, alloc, st, msg)
    if (st == GUATIAO_OK) then
      call build_text(alloc, x, tmp, st, msg)
      call append(p, alloc, tmp, st, msg)
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  subroutine push_bytes(self, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    integer(int8), intent(in) :: x(:)
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_value), target :: tmp
    type(c_ptr) :: p, alloc
    integer :: st
    character(:), allocatable :: msg
    call gt_writable_node(self, GUATIAO_TAG_LIST, p, alloc, st, msg)
    if (st == GUATIAO_OK) then
      call build_bytes(alloc, x, tmp, st, msg)
      call append(p, alloc, tmp, st, msg)
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  subroutine push_value(self, x, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    type(guatiao_value), intent(in) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_value), target :: tmp
    type(c_ptr) :: p, alloc
    integer :: st
    character(:), allocatable :: msg
    call gt_writable_node(self, GUATIAO_TAG_LIST, p, alloc, st, msg)
    if (st == GUATIAO_OK) then
      call build_copy(alloc, x, tmp, st, msg)
      call append(p, alloc, tmp, st, msg)
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> Removes `k` from the map this names (GUATIAO_ERR_NOT_FOUND when
  !> it has none).
  subroutine value_remove(self, k, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    character(*), intent(in), target :: k
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: p, alloc
    integer :: st
    character(:), allocatable :: msg
    call gt_writable_node(self, GUATIAO_TAG_MAP, p, alloc, st, msg)
    if (st == GUATIAO_OK) then
      st = gt_map_discard(p, gt_view(k))
      if (st /= GUATIAO_OK) msg = 'guatiao: no "' // k // '" to remove'
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> Removes element `i` (from 1) of the list this names.
  subroutine value_remove_item(self, i, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    integer, intent(in) :: i
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: p, alloc
    integer :: st
    character(:), allocatable :: msg
    call gt_writable_node(self, GUATIAO_TAG_LIST, p, alloc, st, msg)
    if (st == GUATIAO_OK) then
      if (i < 1) then
        st = GUATIAO_ERR_NOT_FOUND
      else
        st = gt_list_discard(p, int(i - 1, c_size_t))
      end if
      if (st /= GUATIAO_OK) msg = 'guatiao: no such element to remove'
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> Empties the map or list this names.
  subroutine value_clear(self, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: p, alloc
    integer :: st
    character(:), allocatable :: msg
    if (self%tag() == GUATIAO_TAG_LIST) then
      call gt_writable_node(self, GUATIAO_TAG_LIST, p, alloc, st, msg)
      if (st == GUATIAO_OK) st = gt_list_clear(p)
    else
      call gt_writable_node(self, GUATIAO_TAG_MAP, p, alloc, st, msg)
      if (st == GUATIAO_OK) st = gt_map_clear(p)
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> Copies every entry of the map `src` names into the map this names,
  !> replacing collisions.
  subroutine value_copy_from(self, src, stat, errmsg)
    class(guatiao_value), intent(in) :: self
    type(guatiao_value), intent(in) :: src
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: p, alloc, q
    integer :: st
    character(:), allocatable :: msg
    call gt_writable_node(self, GUATIAO_TAG_MAP, p, alloc, st, msg)
    if (st == GUATIAO_OK) then
      q = gt_node(src)
      if (.not. c_associated(q)) then
        st = GUATIAO_ERR_NOT_FOUND
        msg = 'guatiao: nothing at the source'
      else
        st = gt_map_copy_from(alloc, p, q)
        if (st /= GUATIAO_OK) msg = 'guatiao: the copy was refused'
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  ! ---- scalars to a value of their own ---------------------------------

  ! A one-node tree built by `build`, which reports through st/msg.
  subroutine own_one(v, alloc, st, msg)
    type(guatiao_value), intent(inout) :: v
    type(guatiao_allocator), intent(in), optional :: alloc
    integer, intent(out) :: st
    character(:), allocatable, intent(out) :: msg
    type(c_ptr) :: root
    call gt_need(GT_CORE, st, msg)
    if (st == GUATIAO_OK) call gt_new_owned(v, gt_alloc_of(alloc), root)
  end subroutine

  function of_logical(x, alloc, stat, errmsg) result(v)
    logical, intent(in) :: x
    type(guatiao_allocator), intent(in), optional :: alloc
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_value) :: v
    integer :: st
    character(:), allocatable :: msg
    type(gt_value), pointer :: n
    call own_one(v, alloc, st, msg)
    if (st == GUATIAO_OK) then
      call c_f_pointer(gt_root(v), n)
      call build_logical(x, n, st, msg)
    end if
    call gt_report(st, msg, stat, errmsg)
  end function

  function of_int32(x, alloc, stat, errmsg) result(v)
    integer(int32), intent(in) :: x
    type(guatiao_allocator), intent(in), optional :: alloc
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_value) :: v
    v = of_int64(int(x, int64), alloc, stat, errmsg)
  end function

  function of_int64(x, alloc, stat, errmsg) result(v)
    integer(int64), intent(in) :: x
    type(guatiao_allocator), intent(in), optional :: alloc
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_value) :: v
    integer :: st
    character(:), allocatable :: msg
    type(gt_value), pointer :: n
    call own_one(v, alloc, st, msg)
    if (st == GUATIAO_OK) then
      call c_f_pointer(gt_root(v), n)
      call build_int(gt_alloc_of(alloc), x, n, st, msg)
    end if
    call gt_report(st, msg, stat, errmsg)
  end function

  function of_real32(x, alloc, stat, errmsg) result(v)
    real(real32), intent(in) :: x
    type(guatiao_allocator), intent(in), optional :: alloc
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_value) :: v
    integer :: st
    character(:), allocatable :: msg
    type(gt_value), pointer :: n
    call own_one(v, alloc, st, msg)
    if (st == GUATIAO_OK) then
      call c_f_pointer(gt_root(v), n)
      call build_real(gt_alloc_of(alloc), real(x, real64), .true., n, st, msg)
    end if
    call gt_report(st, msg, stat, errmsg)
  end function

  function of_real64(x, alloc, stat, errmsg) result(v)
    real(real64), intent(in) :: x
    type(guatiao_allocator), intent(in), optional :: alloc
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_value) :: v
    integer :: st
    character(:), allocatable :: msg
    type(gt_value), pointer :: n
    call own_one(v, alloc, st, msg)
    if (st == GUATIAO_OK) then
      call c_f_pointer(gt_root(v), n)
      call build_real(gt_alloc_of(alloc), x, .false., n, st, msg)
    end if
    call gt_report(st, msg, stat, errmsg)
  end function

  function of_text(x, alloc, stat, errmsg) result(v)
    character(*), intent(in) :: x
    type(guatiao_allocator), intent(in), optional :: alloc
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_value) :: v
    integer :: st
    character(:), allocatable :: msg
    type(gt_value), pointer :: n
    call own_one(v, alloc, st, msg)
    if (st == GUATIAO_OK) then
      call c_f_pointer(gt_root(v), n)
      call build_text(gt_alloc_of(alloc), x, n, st, msg)
    end if
    call gt_report(st, msg, stat, errmsg)
  end function

  function of_bytes(x, alloc, stat, errmsg) result(v)
    integer(int8), intent(in) :: x(:)
    type(guatiao_allocator), intent(in), optional :: alloc
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_value) :: v
    integer :: st
    character(:), allocatable :: msg
    type(gt_value), pointer :: n
    call own_one(v, alloc, st, msg)
    if (st == GUATIAO_OK) then
      call c_f_pointer(gt_root(v), n)
      call build_bytes(gt_alloc_of(alloc), x, n, st, msg)
    end if
    call gt_report(st, msg, stat, errmsg)
  end function

  ! ---- numbers ---------------------------------------------------------

  !> Reads `text` (JSON's number grammar) as an int64 the way the header's
  !> guatiao_int_or does: a fraction, an exponent or a value out of range
  !> is not one.
  subroutine gt_parse_int64(text, n, ok)
    character(*), intent(in) :: text
    integer(int64), intent(out) :: n
    logical, intent(out) :: ok
    integer :: i, first, d
    logical :: negative
    n = 0
    ok = .false.
    if (len(text) == 0) return
    negative = text(1:1) == '-'
    first = 1
    if (negative) first = 2
    if (first > len(text)) return
    do i = first, len(text)
      d = iachar(text(i:i)) - iachar('0')
      if (d < 0 .or. d > 9) return
      if (negative) then
        if (n < (-huge(n) - 1_int64 + d) / 10) return
        n = n * 10 - d
      else
        if (n > (huge(n) - d) / 10) return
        n = n * 10 + d
      end if
    end do
    ok = .true.
  end subroutine

  !> The shortest decimal text that reads back as `x` (as a real32 when
  !> `single`), in JSON's grammar; `ok` false for NaN and the infinities.
  !> Negative zero keeps its sign.
  subroutine gt_real_text(x, single, text, ok)
    real(real64), intent(in) :: x
    logical, intent(in) :: single
    character(:), allocatable, intent(out) :: text
    logical, intent(out) :: ok
    character(48) :: buf
    character(16) :: fmt
    character(:), allocatable :: digits
    real(real64) :: back
    real(real32) :: back32
    integer :: d, most, e, epos, ios, i
    ok = ieee_is_finite(x)
    if (.not. ok) return
    if (x == 0) then
      if (ieee_is_negative(x)) then
        text = '-0'
      else
        text = '0'
      end if
      return
    end if
    most = 17
    if (single) most = 9
    do d = 1, most
      write (fmt, '(a,i0,a)') '(es40.', d - 1, 'e4)'
      write (buf, fmt) x
      if (single) then
        read (buf, *, iostat=ios) back32
        if (ios == 0 .and. back32 == real(x, real32)) exit
      else
        read (buf, *, iostat=ios) back
        if (ios == 0 .and. back == x) exit
      end if
    end do
    buf = adjustl(buf)
    epos = scan(buf, 'Ee')
    read (buf(epos + 1:), *) e
    digits = ''
    do i = 1, epos - 1
      if (buf(i:i) >= '0' .and. buf(i:i) <= '9') digits = digits // buf(i:i)
    end do
    do while (len(digits) > 1 .and. digits(len(digits):len(digits)) == '0')
      digits = digits(1:len(digits) - 1)
    end do
    if (e >= 0 .and. e < 21) then
      if (len(digits) <= e + 1) then
        text = digits // repeat('0', e + 1 - len(digits))
      else
        text = digits(1:e + 1) // '.' // digits(e + 2:)
      end if
    else if (e < 0 .and. e > -7) then
      text = '0.' // repeat('0', -e - 1) // digits
    else
      text = digits(1:1)
      if (len(digits) > 1) text = text // '.' // digits(2:)
      write (buf, '(i0)') e
      text = text // 'e' // trim(buf)
    end if
    if (x < 0) text = '-' // text
  end subroutine

end module guatiao_values
