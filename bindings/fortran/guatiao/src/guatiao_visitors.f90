! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> Walks with no value built: a visitor receives a walk's events (a
!> document read, say) and a walker hands its own data to a sink (a
!> document written). This is the way between a document and a derived
!> type of your own; guatiao_serde's guatiao_visit and
!> guatiao_emit_walker drive them.
module guatiao_visitors
  use, intrinsic :: iso_c_binding
  use, intrinsic :: iso_fortran_env, only: int64
  use guatiao_c, only: gt_str, gt_visitor, gt_walker, GUATIAO_OK, GUATIAO_ERR_WRONG_KIND, GUATIAO_LEN_UNKNOWN
  use guatiao_native, only: gt_copy_text, gt_view, gt_bytes_view
  implicit none
  private

  public :: guatiao_visitor, guatiao_walker, guatiao_sink, GUATIAO_LEN_UNKNOWN
  public :: gt_visitor_box, gt_visitor_table, gt_walker_box, gt_walker_table

  !> What receives a walk's events, in order: a container's begin (with
  !> how many items follow, or GUATIAO_LEN_UNKNOWN), each item (a map's
  !> after its key) as one event or a nested container, then its end.
  !> Extend it and override the events your data takes (`on_number`,
  !> `on_key`, `on_map_begin`, ...); each answers
  !> GUATIAO_OK to go on, or a status, which stops the walk and is
  !> answered. An event left as it is refuses with GUATIAO_ERR_WRONG_KIND.
  !> Text is UTF-8 and a number its text in the JSON number grammar.
  type, abstract :: guatiao_visitor
  contains
    procedure :: on_absent => refuse
    procedure :: on_null => refuse
    procedure :: on_boolean => refuse_bool
    procedure :: on_number => refuse_text
    procedure :: on_string => refuse_text
    procedure :: on_bytes => refuse_bytes
    procedure :: on_list_begin => refuse_len
    procedure :: on_list_end => refuse
    procedure :: on_map_begin => refuse_len
    procedure :: on_key => refuse_text
    procedure :: on_map_end => refuse
  end type

  !> Where a walker writes one value's events, valid only during its
  !> `walk`; each answers the status the receiving side gave, which the
  !> walk should stop at and answer.
  type :: guatiao_sink
    private
    type(gt_visitor), pointer :: c => null()
  contains
    procedure :: absent => sink_absent
    procedure :: null => sink_null
    procedure :: boolean => sink_boolean
    procedure :: number => sink_number
    procedure :: string => sink_string
    procedure :: bytes => sink_bytes
    procedure :: list_begin => sink_list_begin
    procedure :: list_end => sink_list_end
    procedure :: map_begin => sink_map_begin
    procedure :: key => sink_key
    procedure :: map_end => sink_map_end
  end type

  !> What hands its own data to a sink: extend it and write `walk`, which
  !> writes one value's events (a container's begin, its items, its end)
  !> and answers GUATIAO_OK, or the first status an event answered.
  type, abstract :: guatiao_walker
  contains
    procedure(walk_i), deferred :: walk
  end type

  abstract interface
    recursive function walk_i(self, into) result(st)
      import :: guatiao_walker, guatiao_sink
      class(guatiao_walker), intent(inout) :: self
      type(guatiao_sink), intent(in) :: into
      integer :: st
    end function
  end interface

  !> The Fortran object behind a C table's `ctx`.
  type :: gt_visitor_box
    class(guatiao_visitor), pointer :: v => null()
  end type
  type :: gt_walker_box
    class(guatiao_walker), pointer :: w => null()
  end type

  ! The C visitor's functions, called from a sink.
  abstract interface
    function c_event_fn(ctx) bind(c) result(r)
      import :: c_ptr, c_int32_t
      type(c_ptr), value :: ctx
      integer(c_int32_t) :: r
    end function
    function c_bool_fn(ctx, b) bind(c) result(r)
      import :: c_ptr, c_int32_t, c_int8_t
      type(c_ptr), value :: ctx
      integer(c_int8_t), value :: b
      integer(c_int32_t) :: r
    end function
    function c_text_fn(ctx, t) bind(c) result(r)
      import :: c_ptr, c_int32_t, gt_str
      type(c_ptr), value :: ctx
      type(gt_str), value :: t
      integer(c_int32_t) :: r
    end function
    function c_len_fn(ctx, n) bind(c) result(r)
      import :: c_ptr, c_int32_t, c_int64_t
      type(c_ptr), value :: ctx
      integer(c_int64_t), value :: n
      integer(c_int32_t) :: r
    end function
  end interface

contains

  ! ---- the events a visitor does not take --------------------------------------------

  recursive function refuse(self) result(st)
    class(guatiao_visitor), intent(inout) :: self
    integer :: st
    st = GUATIAO_ERR_WRONG_KIND
  end function

  recursive function refuse_bool(self, value) result(st)
    class(guatiao_visitor), intent(inout) :: self
    logical, intent(in) :: value
    integer :: st
    st = GUATIAO_ERR_WRONG_KIND
  end function

  recursive function refuse_text(self, text) result(st)
    class(guatiao_visitor), intent(inout) :: self
    character(*), intent(in) :: text
    integer :: st
    st = GUATIAO_ERR_WRONG_KIND
  end function

  recursive function refuse_bytes(self, bytes) result(st)
    class(guatiao_visitor), intent(inout) :: self
    integer(c_int8_t), intent(in) :: bytes(:)
    integer :: st
    st = GUATIAO_ERR_WRONG_KIND
  end function

  recursive function refuse_len(self, len) result(st)
    class(guatiao_visitor), intent(inout) :: self
    integer(int64), intent(in) :: len
    integer :: st
    st = GUATIAO_ERR_WRONG_KIND
  end function

  ! ---- a Fortran visitor as a C table -----------------------------------------------------

  !> The C table whose `ctx` is `box` (a `c_loc` of the caller's target):
  !> every event reaches `box%v`.
  recursive function gt_visitor_table(box) result(t)
    type(c_ptr), intent(in) :: box
    type(gt_visitor) :: t
    t%struct_size = int(c_sizeof(t), c_int32_t)
    t%ctx = box
    t%absent = c_funloc(tr_absent)
    t%null = c_funloc(tr_null)
    t%boolean = c_funloc(tr_boolean)
    t%number = c_funloc(tr_number)
    t%string = c_funloc(tr_string)
    t%bytes = c_funloc(tr_bytes)
    t%list_begin = c_funloc(tr_list_begin)
    t%list_end = c_funloc(tr_list_end)
    t%map_begin = c_funloc(tr_map_begin)
    t%key = c_funloc(tr_key)
    t%map_end = c_funloc(tr_map_end)
  end function

  recursive function box_of(ctx) result(box)
    type(c_ptr), intent(in) :: ctx
    type(gt_visitor_box), pointer :: box
    call c_f_pointer(ctx, box)
  end function

  recursive function tr_absent(ctx) bind(c) result(r)
    type(c_ptr), value :: ctx
    integer(c_int32_t) :: r
    type(gt_visitor_box), pointer :: box
    box => box_of(ctx)
    r = int(box%v%on_absent(), c_int32_t)
  end function

  recursive function tr_null(ctx) bind(c) result(r)
    type(c_ptr), value :: ctx
    integer(c_int32_t) :: r
    type(gt_visitor_box), pointer :: box
    box => box_of(ctx)
    r = int(box%v%on_null(), c_int32_t)
  end function

  recursive function tr_boolean(ctx, b) bind(c) result(r)
    type(c_ptr), value :: ctx
    integer(c_int8_t), value :: b
    integer(c_int32_t) :: r
    type(gt_visitor_box), pointer :: box
    box => box_of(ctx)
    r = int(box%v%on_boolean(b /= 0), c_int32_t)
  end function

  recursive function tr_number(ctx, t) bind(c) result(r)
    type(c_ptr), value :: ctx
    type(gt_str), value :: t
    integer(c_int32_t) :: r
    type(gt_visitor_box), pointer :: box
    character(:), allocatable :: s
    box => box_of(ctx)
    call gt_copy_text(t%ptr, t%len, s)
    r = int(box%v%on_number(s), c_int32_t)
  end function

  recursive function tr_string(ctx, t) bind(c) result(r)
    type(c_ptr), value :: ctx
    type(gt_str), value :: t
    integer(c_int32_t) :: r
    type(gt_visitor_box), pointer :: box
    character(:), allocatable :: s
    box => box_of(ctx)
    call gt_copy_text(t%ptr, t%len, s)
    r = int(box%v%on_string(s), c_int32_t)
  end function

  recursive function tr_bytes(ctx, t) bind(c) result(r)
    type(c_ptr), value :: ctx
    type(gt_str), value :: t
    integer(c_int32_t) :: r
    type(gt_visitor_box), pointer :: box
    integer(c_int8_t), pointer :: b(:)
    integer(c_int8_t), target :: none(0)
    box => box_of(ctx)
    if (t%len > 0 .and. c_associated(t%ptr)) then
      call c_f_pointer(t%ptr, b, [t%len])
    else
      b => none
    end if
    r = int(box%v%on_bytes(b), c_int32_t)
  end function

  recursive function tr_list_begin(ctx, n) bind(c) result(r)
    type(c_ptr), value :: ctx
    integer(c_int64_t), value :: n
    integer(c_int32_t) :: r
    type(gt_visitor_box), pointer :: box
    box => box_of(ctx)
    r = int(box%v%on_list_begin(int(n, int64)), c_int32_t)
  end function

  recursive function tr_list_end(ctx) bind(c) result(r)
    type(c_ptr), value :: ctx
    integer(c_int32_t) :: r
    type(gt_visitor_box), pointer :: box
    box => box_of(ctx)
    r = int(box%v%on_list_end(), c_int32_t)
  end function

  recursive function tr_map_begin(ctx, n) bind(c) result(r)
    type(c_ptr), value :: ctx
    integer(c_int64_t), value :: n
    integer(c_int32_t) :: r
    type(gt_visitor_box), pointer :: box
    box => box_of(ctx)
    r = int(box%v%on_map_begin(int(n, int64)), c_int32_t)
  end function

  recursive function tr_key(ctx, t) bind(c) result(r)
    type(c_ptr), value :: ctx
    type(gt_str), value :: t
    integer(c_int32_t) :: r
    type(gt_visitor_box), pointer :: box
    character(:), allocatable :: s
    box => box_of(ctx)
    call gt_copy_text(t%ptr, t%len, s)
    r = int(box%v%on_key(s), c_int32_t)
  end function

  recursive function tr_map_end(ctx) bind(c) result(r)
    type(c_ptr), value :: ctx
    integer(c_int32_t) :: r
    type(gt_visitor_box), pointer :: box
    box => box_of(ctx)
    r = int(box%v%on_map_end(), c_int32_t)
  end function

  ! ---- a Fortran walker as a C table -------------------------------------------------------

  !> The C table whose `ctx` is `box` (a `c_loc` of the caller's target):
  !> its walk is `box%w%walk`.
  recursive function gt_walker_table(box) result(t)
    type(c_ptr), intent(in) :: box
    type(gt_walker) :: t
    t%struct_size = int(c_sizeof(t), c_int32_t)
    t%ctx = box
    t%walk = c_funloc(tr_walk)
  end function

  recursive function tr_walk(ctx, into) bind(c) result(r)
    type(c_ptr), value :: ctx
    type(c_ptr), value :: into
    integer(c_int32_t) :: r
    type(gt_walker_box), pointer :: box
    type(guatiao_sink) :: sink
    call c_f_pointer(ctx, box)
    call c_f_pointer(into, sink%c)
    r = int(box%w%walk(sink), c_int32_t)
  end function

  ! ---- a sink: the events into the C visitor it was handed ----------------------------------

  recursive function event(self, f) result(st)
    class(guatiao_sink), intent(in) :: self
    type(c_funptr), intent(in) :: f
    integer :: st
    procedure(c_event_fn), pointer :: p
    st = GUATIAO_ERR_WRONG_KIND
    if (.not. c_associated(f)) return
    call c_f_procpointer(f, p)
    st = int(p(self%c%ctx))
  end function

  recursive function text_event(self, f, t) result(st)
    class(guatiao_sink), intent(in) :: self
    type(c_funptr), intent(in) :: f
    type(gt_str), intent(in) :: t
    integer :: st
    procedure(c_text_fn), pointer :: p
    st = GUATIAO_ERR_WRONG_KIND
    if (.not. c_associated(f)) return
    call c_f_procpointer(f, p)
    st = int(p(self%c%ctx, t))
  end function

  recursive function len_event(self, f, n) result(st)
    class(guatiao_sink), intent(in) :: self
    type(c_funptr), intent(in) :: f
    integer(int64), intent(in) :: n
    integer :: st
    procedure(c_len_fn), pointer :: p
    st = GUATIAO_ERR_WRONG_KIND
    if (.not. c_associated(f)) return
    call c_f_procpointer(f, p)
    st = int(p(self%c%ctx, int(n, c_int64_t)))
  end function

  !> No value here: what a lookup answers, which a document has no
  !> spelling for (a map leaves the key out instead).
  recursive function sink_absent(self) result(st)
    class(guatiao_sink), intent(in) :: self
    integer :: st
    st = event(self, self%c%absent)
  end function

  recursive function sink_null(self) result(st)
    class(guatiao_sink), intent(in) :: self
    integer :: st
    st = event(self, self%c%null)
  end function

  recursive function sink_boolean(self, value) result(st)
    class(guatiao_sink), intent(in) :: self
    logical, intent(in) :: value
    integer :: st
    procedure(c_bool_fn), pointer :: p
    integer(c_int8_t) :: b
    st = GUATIAO_ERR_WRONG_KIND
    if (.not. c_associated(self%c%boolean)) return
    call c_f_procpointer(self%c%boolean, p)
    b = 0_c_int8_t
    if (value) b = 1_c_int8_t
    st = int(p(self%c%ctx, b))
  end function

  !> A number as its text, which must be in the JSON number grammar.
  recursive function sink_number(self, text) result(st)
    class(guatiao_sink), intent(in) :: self
    character(*), intent(in), target :: text
    integer :: st
    st = text_event(self, self%c%number, gt_view(text))
  end function

  !> Text, which must be UTF-8.
  recursive function sink_string(self, text) result(st)
    class(guatiao_sink), intent(in) :: self
    character(*), intent(in), target :: text
    integer :: st
    st = text_event(self, self%c%string, gt_view(text))
  end function

  recursive function sink_bytes(self, bytes) result(st)
    class(guatiao_sink), intent(in) :: self
    integer(c_int8_t), intent(in), target :: bytes(:)
    integer :: st
    st = text_event(self, self%c%bytes, gt_bytes_view(bytes))
  end function

  !> A list of `len` items opens (GUATIAO_LEN_UNKNOWN when not known).
  recursive function sink_list_begin(self, len) result(st)
    class(guatiao_sink), intent(in) :: self
    integer(int64), intent(in) :: len
    integer :: st
    st = len_event(self, self%c%list_begin, len)
  end function

  recursive function sink_list_end(self) result(st)
    class(guatiao_sink), intent(in) :: self
    integer :: st
    st = event(self, self%c%list_end)
  end function

  !> A map of `len` entries opens (GUATIAO_LEN_UNKNOWN when not known).
  recursive function sink_map_begin(self, len) result(st)
    class(guatiao_sink), intent(in) :: self
    integer(int64), intent(in) :: len
    integer :: st
    st = len_event(self, self%c%map_begin, len)
  end function

  !> The next entry's key; its value is the next event.
  recursive function sink_key(self, key) result(st)
    class(guatiao_sink), intent(in) :: self
    character(*), intent(in), target :: key
    integer :: st
    st = text_event(self, self%c%key, gt_view(key))
  end function

  recursive function sink_map_end(self) result(st)
    class(guatiao_sink), intent(in) :: self
    integer :: st
    st = event(self, self%c%map_end)
  end function

end module guatiao_visitors
