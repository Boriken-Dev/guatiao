! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> Values as bytes, and channels: a schema announced before its values,
!> so a receiver knows what each value means and refuses what does not
!> fit it.
!>
!> The wire encoding is a tag byte per node, LEB128 lengths, a number as
!> its own text, a map in its order: the same bytes in every language. A
!> frame is `kind:u8 channel:LEB128 payload` (kind 0 a schema, 1 a value,
!> 2 the channel's end); where one ends is the transport's to say. Bytes
!> cross as `integer(int8)` arrays.
module guatiao_wire
  use, intrinsic :: iso_c_binding
  use, intrinsic :: iso_fortran_env, only: int8, int64
  use guatiao_c
  use guatiao_native
  use guatiao_errors
  use guatiao_values
  use guatiao_holds
  implicit none
  private

  public :: GUATIAO_FRAME_SCHEMA, GUATIAO_FRAME_VALUE, GUATIAO_FRAME_CLOSE
  public :: GUATIAO_WIRE_ACCEPTED, GUATIAO_WIRE_MALFORMED, GUATIAO_WIRE_NO_SCHEMA, GUATIAO_WIRE_NOT_A_SCHEMA, &
            GUATIAO_WIRE_INVALID
  public :: guatiao_to_wire, guatiao_from_wire, guatiao_wire_announce, guatiao_wire_send, guatiao_wire_close
  public :: guatiao_receiver, guatiao_open_receiver, guatiao_wire_event, guatiao_wire_refusal

  !> A frame's kind byte (`GUATIAO_FRAME_*`).
  integer, parameter :: GUATIAO_FRAME_SCHEMA = 0, GUATIAO_FRAME_VALUE = 1, GUATIAO_FRAME_CLOSE = 2

  !> Which refusal a receiver answered (`GUATIAO_WIRE_*`).
  integer, parameter :: GUATIAO_WIRE_ACCEPTED = 0, GUATIAO_WIRE_MALFORMED = 1, GUATIAO_WIRE_NO_SCHEMA = 2, &
                        GUATIAO_WIRE_NOT_A_SCHEMA = 3, GUATIAO_WIRE_INVALID = 4

  !> What a frame said: its kind, its channel (the 64 bits, read as
  !> unsigned by C), and for a schema or a value the tree this side now
  !> owns; absent for a channel's end.
  type :: guatiao_wire_event
    integer :: kind = GUATIAO_FRAME_SCHEMA
    integer(int64) :: channel = 0
    type(guatiao_value) :: value
  end type

  !> Why a frame was refused. `code` is GUATIAO_WIRE_MALFORMED (`at` the
  !> byte of the frame, from 0), _NO_SCHEMA, _NOT_A_SCHEMA or _INVALID
  !> (`key` the field, `expected` what it accepts when the schema says,
  !> `detail` the map guatiao_validate answers); `channel` and `kind` are
  !> what the frame named. Nothing the frame carried is quoted.
  type :: guatiao_wire_refusal
    integer :: code = GUATIAO_WIRE_ACCEPTED
    integer(int64) :: channel = 0
    integer :: kind = 0
    integer(int64) :: at = 0
    character(:), allocatable :: key
    character(:), allocatable :: expected
    character(:), allocatable :: message
    type(guatiao_value) :: detail
  end type

  !> Keeps each channel's schema and checks every value against it.
  !> Copies share one receiver; it is freed when the last goes or on
  !> `close()`. Not safe from two threads at once.
  type :: guatiao_receiver
    type(gt_hold), private :: hold
    type(c_ptr), private :: alloc = c_null_ptr
  contains
    procedure :: accept => receiver_accept
    procedure :: close => receiver_close
    procedure :: is_closed => receiver_is_closed
  end type

contains

  !> Copies the BYTES node at `node` into `bytes`.
  recursive subroutine bytes_of(node, bytes)
    type(gt_value), intent(in) :: node
    integer(int8), allocatable, intent(inout) :: bytes(:)
    integer(c_int8_t), pointer :: b(:)
    if (allocated(bytes)) deallocate(bytes)
    allocate(bytes(node%payload%len))
    if (node%payload%len == 0 .or. .not. c_associated(node%payload%ptr)) return
    call c_f_pointer(node%payload%ptr, b, [node%payload%len])
    bytes = b
  end subroutine

  !> What `value` names in the wire encoding.
  recursive subroutine guatiao_to_wire(value, bytes, stat, errmsg)
    type(guatiao_value), intent(in) :: value
    integer(int8), allocatable, intent(inout) :: bytes(:)
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_value), target :: out
    type(c_ptr) :: node
    integer :: st
    integer(c_int32_t) :: ignored
    character(:), allocatable :: msg
    msg = ''
    call gt_need(GT_CORE, st, msg)
    if (st == GUATIAO_OK) then
      node = gt_node(value)
      if (.not. c_associated(node)) then
        st = GUATIAO_ERR_NOT_FOUND
        msg = 'guatiao: nothing to encode'
      else
        st = gt_wire_encode(gt_default_alloc(), node, c_loc(out))
        if (st == GUATIAO_OK) then
          call bytes_of(out, bytes)
        else
          msg = 'guatiao: a node the encoding cannot carry (text that is not UTF-8, a number outside the grammar)'
        end if
        ignored = gt_value_free(c_loc(out))
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> The value the wire encoding in `bytes` holds, built in `alloc`.
  !> GUATIAO_ERR_BAD_VALUE for bytes that are not exactly one value
  !> (truncated, an unknown tag, a length not in its shortest form, text
  !> that is not UTF-8, a number outside the grammar, a repeated key,
  !> anything after it); `value` is then left as it was.
  recursive subroutine guatiao_from_wire(bytes, value, alloc, stat, errmsg)
    integer(int8), intent(in), target :: bytes(:)
    type(guatiao_value), intent(inout) :: value
    type(guatiao_allocator), intent(in), optional :: alloc
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_value) :: fresh
    type(c_ptr) :: root
    integer :: st
    character(:), allocatable :: msg
    msg = ''
    call gt_need(GT_CORE, st, msg)
    if (st == GUATIAO_OK) then
      call gt_new_owned(fresh, gt_alloc_of(alloc), root)
      st = gt_wire_decode(gt_alloc_of(alloc), gt_bytes_view(bytes), root)
      if (st == GUATIAO_OK) then
        value = fresh
      else
        call fresh%close()
        msg = 'guatiao: bytes that are not a value in the wire encoding'
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  ! A frame built through `what` (1 announce, 2 send, 3 close).
  recursive subroutine frame_of(what, channel, value, frame, stat, errmsg)
    integer, intent(in) :: what
    integer(int64), intent(in) :: channel
    type(guatiao_value), intent(in), optional :: value
    integer(int8), allocatable, intent(inout) :: frame(:)
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_value), target :: out
    type(c_ptr) :: node
    integer :: st
    integer(c_int32_t) :: ignored
    character(:), allocatable :: msg
    msg = ''
    call gt_need(GT_CORE, st, msg)
    if (st == GUATIAO_OK) then
      node = c_null_ptr
      if (present(value)) then
        node = gt_node(value)
        if (.not. c_associated(node)) then
          st = GUATIAO_ERR_NOT_FOUND
          msg = 'guatiao: nothing to send'
        end if
      end if
    end if
    if (st == GUATIAO_OK) then
      select case (what)
      case (1)
        st = gt_wire_announce(gt_default_alloc(), int(channel, c_int64_t), node, c_loc(out))
      case (2)
        st = gt_wire_send(gt_default_alloc(), int(channel, c_int64_t), node, c_loc(out))
      case default
        st = gt_wire_close(gt_default_alloc(), int(channel, c_int64_t), c_loc(out))
      end select
      if (st == GUATIAO_OK) then
        call bytes_of(out, frame)
      else
        msg = 'guatiao: a node the encoding cannot carry (text that is not UTF-8, a number outside the grammar)'
      end if
      ignored = gt_value_free(c_loc(out))
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> The frame that announces `schema` on `channel`; a later one replaces
  !> it. A schema is not judged here: a receiver refuses one that is not.
  recursive subroutine guatiao_wire_announce(channel, schema, frame, stat, errmsg)
    integer(int64), intent(in) :: channel
    type(guatiao_value), intent(in) :: schema
    integer(int8), allocatable, intent(inout) :: frame(:)
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    call frame_of(1, channel, schema, frame, stat, errmsg)
  end subroutine

  !> The frame that carries `value` on `channel`.
  recursive subroutine guatiao_wire_send(channel, value, frame, stat, errmsg)
    integer(int64), intent(in) :: channel
    type(guatiao_value), intent(in) :: value
    integer(int8), allocatable, intent(inout) :: frame(:)
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    call frame_of(2, channel, value, frame, stat, errmsg)
  end subroutine

  !> The frame that ends `channel`: a receiver forgets its schema.
  recursive subroutine guatiao_wire_close(channel, frame, stat, errmsg)
    integer(int64), intent(in) :: channel
    integer(int8), allocatable, intent(inout) :: frame(:)
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    call frame_of(3, channel, frame=frame, stat=stat, errmsg=errmsg)
  end subroutine

  !> A receiver building what it answers, and the schemas it keeps, in
  !> `alloc` (guatiao's own heap without it), which outlives it.
  recursive function guatiao_open_receiver(alloc, stat, errmsg) result(r)
    type(guatiao_allocator), intent(in), optional :: alloc
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_receiver) :: r
    type(c_ptr) :: a, p
    integer :: st
    character(:), allocatable :: msg
    msg = ''
    call gt_need(GT_CORE, st, msg)
    if (st == GUATIAO_OK) then
      a = gt_alloc_of(alloc)
      p = gt_wire_receiver_new(a)
      if (c_associated(p)) then
        r%hold = gt_new_hold(GT_HOLD_RECEIVER, ctx=p)
        r%alloc = a
      else
        st = GUATIAO_ERR_ALLOC
        msg = 'guatiao: a receiver needs a complete allocator'
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end function

  !> Reads one frame. A schema frame makes its schema the channel's; a
  !> value frame answers its value once its channel has a schema that
  !> accepts it; a close frame forgets the channel's schema. A refusal is
  !> its status (GUATIAO_ERR_BAD_VALUE for MALFORMED and INVALID,
  !> GUATIAO_ERR_NOT_FOUND for NO_SCHEMA, GUATIAO_ERR_WRONG_KIND for
  !> NOT_A_SCHEMA) with `refused` saying which and why, and changes
  !> nothing the receiver keeps.
  recursive subroutine receiver_accept(self, frame, event, refused, stat, errmsg)
    class(guatiao_receiver), intent(inout) :: self
    integer(int8), intent(in), target :: frame(:)
    type(guatiao_wire_event), intent(out) :: event
    type(guatiao_wire_refusal), intent(out), optional :: refused
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_wire_event), target :: ev
    type(gt_value) :: none
    type(guatiao_value) :: detail, item
    type(c_ptr) :: root
    type(gt_value), pointer :: to
    integer :: st, got
    integer(c_int32_t) :: ignored
    character(:), allocatable :: msg, words
    msg = ''
    if (self%hold%is_closed()) then
      call gt_report(GUATIAO_STAT_CLOSED, 'guatiao: the receiver is closed', stat, errmsg)
      return
    end if
    st = gt_wire_receiver_accept(self%hold%box%ctx, gt_bytes_view(frame), c_loc(ev))
    event%kind = int(ev%kind)
    event%channel = int(ev%channel, int64)
    if (st == GUATIAO_OK .and. ev%value%tag /= GUATIAO_TAG_ABSENT) then
      ! The tree moves into a value this binding owns; the event's copy
      ! is left ABSENT, so freeing it below frees nothing.
      call gt_new_owned(event%value, self%alloc, root)
      call c_f_pointer(root, to)
      to = ev%value
      ev%value = none
    end if
    if (ev%message%tag == GUATIAO_TAG_STRING) then
      call gt_text_of_owned(ev%message%payload, words)
    else
      words = ''
    end if
    if (ev%detail%tag /= GUATIAO_TAG_ABSENT) then
      call gt_new_owned(detail, self%alloc, root)
      call c_f_pointer(root, to)
      to = ev%detail
      ev%detail = none
    end if
    ignored = gt_value_free(c_loc(ev%value))
    ignored = gt_value_free(c_loc(ev%message))
    ignored = gt_value_free(c_loc(ev%detail))
    if (st /= GUATIAO_OK) then
      msg = 'guatiao: the frame was refused'
      if (len(words) > 0) msg = 'guatiao: ' // words
      if (present(refused)) then
        refused%code = int(ev%refusal)
        refused%channel = event%channel
        refused%kind = event%kind
        refused%at = int(ev%at, int64)
        refused%message = words
        refused%key = ''
        if (.not. detail%is_absent()) then
          item = detail%key('key')
          call item%get(refused%key, stat=got)
          if (got /= GUATIAO_OK) refused%key = ''
          item = detail%key('expected')
          if (item%tag() == GUATIAO_TAG_STRING) call item%get(refused%expected, stat=got)
          refused%detail = detail
        end if
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> Frees the receiver and the schemas it keeps, for every copy. Safe
  !> twice.
  recursive subroutine receiver_close(self)
    class(guatiao_receiver), intent(inout) :: self
    call self%hold%close()
  end subroutine

  recursive logical function receiver_is_closed(self)
    class(guatiao_receiver), intent(in) :: self
    receiver_is_closed = self%hold%is_closed()
  end function

end module guatiao_wire
