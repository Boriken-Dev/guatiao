! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> The wire encoding and channels against the shared vectors every
!> implementation reads (crates/guatiao/tests/vectors/wire.json): each
!> value both ways, each malformed input refused, the frames the builders
!> write, and a conversation with every refusal.
program test_wire
  use, intrinsic :: iso_c_binding, only: c_sizeof
  use, intrinsic :: iso_fortran_env, only: int8, int64
  use harness
  use guatiao
  use guatiao_c, only: gt_wire_event
  use guatiao_serde
  implicit none

  type(guatiao_value) :: vectors
  character(:), allocatable :: text

  call setup()
  call read_file(repo_path('crates/guatiao/tests/vectors/wire.json'), text)
  call guatiao_parse(text, GUATIAO_FORMAT_JSON, vectors)
  call values()
  call malformed()
  call frames()
  call conversation()
  call codes_and_layout()
  call finish('wire')

contains

  ! The bytes hex text spells.
  function unhex(h) result(b)
    character(*), intent(in) :: h
    integer(int8), allocatable :: b(:)
    integer :: i, n
    allocate(b(len(h) / 2))
    do i = 1, size(b)
      read (h(2 * i - 1:2 * i), '(z2)') n
      b(i) = int(merge(n - 256, n, n > 127), int8)
    end do
  end function

  ! Bytes as lowercase hex.
  function hex(b) result(h)
    integer(int8), intent(in) :: b(:)
    character(:), allocatable :: h
    character(*), parameter :: digits = '0123456789abcdef'
    integer :: i, n
    allocate(character(2 * size(b)) :: h)
    do i = 1, size(b)
      n = iand(int(b(i)), 255)
      h(2 * i - 1:2 * i - 1) = digits(n / 16 + 1:n / 16 + 1)
      h(2 * i:2 * i) = digits(mod(n, 16) + 1:mod(n, 16) + 1)
    end do
  end function

  ! The channel a frame names, from its LEB128: JSON's numbers do not all
  ! fit an int64, and a channel is 64 bits.
  integer(int64) function channel_of(frame)
    integer(int8), intent(in) :: frame(:)
    integer :: i, shift
    integer(int64) :: byte
    channel_of = 0
    shift = 0
    do i = 2, size(frame)
      byte = iand(int(frame(i), int64), 255_int64)
      channel_of = ior(channel_of, ishft(iand(byte, 127_int64), shift))
      if (iand(byte, 128_int64) == 0) exit
      shift = shift + 7
    end do
  end function

  function text_at(v, k) result(s)
    type(guatiao_value), intent(in) :: v
    character(*), intent(in) :: k
    character(:), allocatable :: s
    type(guatiao_value) :: item
    integer :: st
    item = v%key(k)
    call item%get(s, stat=st)
    if (st /= GUATIAO_OK) s = ''
  end function

  ! Whether the node `v` names is the vector tree `t` ([tag] or [tag, x]).
  recursive logical function matches(v, t) result(same)
    type(guatiao_value), intent(in) :: v, t
    type(guatiao_value) :: arg, item, pair, child, sub
    type(guatiao_text), allocatable :: keys(:)
    character(:), allocatable :: tag, s, k
    integer(int8), allocatable :: b(:)
    logical :: flag
    integer :: i, st
    item = t%item(1)
    call item%get(tag)
    arg = t%item(2)
    same = .false.
    select case (tag)
    case ('absent')
      same = v%tag() == GUATIAO_TAG_ABSENT
    case ('null')
      same = v%tag() == GUATIAO_TAG_NULL
    case ('bool')
      call arg%get(flag)
      same = v%tag() == GUATIAO_TAG_BOOL .and. (v%bool_or(.not. flag) .eqv. flag)
    case ('number')
      call arg%get(s)
      same = v%tag() == GUATIAO_TAG_NUMBER .and. v%number_text() == s .and. len(v%number_text()) == len(s)
    case ('string')
      call arg%get(s)
      if (v%tag() /= GUATIAO_TAG_STRING) return
      call v%get(k, stat=st)
      same = st == GUATIAO_OK .and. k == s .and. len(k) == len(s)
    case ('bytes')
      call arg%get(s)
      if (v%tag() /= GUATIAO_TAG_BYTES) return
      call v%get(b)
      same = hex(b) == s .and. len(hex(b)) == len(s)
    case ('list')
      if (v%tag() /= GUATIAO_TAG_LIST .or. v%length() /= arg%length()) return
      do i = 1, arg%length()
        item = arg%item(i)
        child = v%item(i)
        if (.not. matches(child, item)) return
      end do
      same = .true.
    case ('map')
      if (v%tag() /= GUATIAO_TAG_MAP .or. v%length() /= arg%length()) return
      keys = v%keys()
      do i = 1, arg%length()
        pair = arg%item(i)
        item = pair%item(1)
        call item%get(k)
        if (keys(i)%text /= k .or. len(keys(i)%text) /= len(k)) return
        child = v%key(k)
        sub = pair%item(2)
        if (.not. matches(child, sub)) return
      end do
      same = .true.
    end select
  end function

  ! The value a vector tree names.
  recursive function build(t) result(v)
    type(guatiao_value), intent(in) :: t
    type(guatiao_value) :: v
    type(guatiao_value) :: arg, item, pair, child, sub
    character(:), allocatable :: tag, s, k
    logical :: flag
    integer :: i
    item = t%item(1)
    call item%get(tag)
    arg = t%item(2)
    select case (tag)
    case ('absent')
      v = guatiao_absent()
    case ('null')
      v = guatiao_null()
    case ('bool')
      call arg%get(flag)
      v = guatiao_of(flag)
    case ('number')
      call arg%get(s)
      v = guatiao_number(s)
    case ('string')
      call arg%get(s)
      v = guatiao_of(s)
    case ('bytes')
      call arg%get(s)
      v = guatiao_of(unhex(s))
    case ('list')
      v = guatiao_list()
      do i = 1, arg%length()
        item = arg%item(i)
        child = build(item)
        call v%push(child)
      end do
    case default
      v = guatiao_map()
      do i = 1, arg%length()
        pair = arg%item(i)
        item = pair%item(1)
        call item%get(k)
        sub = pair%item(2)
        child = build(sub)
        call v%set(k, child)
      end do
    end select
  end function

  subroutine values()
    type(guatiao_value) :: list, entry, tree, value, built
    integer(int8), allocatable :: back(:)
    character(:), allocatable :: name, want
    integer :: i, st
    list = vectors%key('values')
    call check(list%length() > 10, 'value vectors are there')
    do i = 1, list%length()
      entry = list%item(i)
      name = text_at(entry, 'name')
      want = text_at(entry, 'hex')
      tree = entry%key('tree')
      call guatiao_from_wire(unhex(want), value, stat=st)
      call check_status(st, GUATIAO_OK, name // ': decodes')
      call check(matches(value, tree), name // ': decodes to its tree')
      call guatiao_to_wire(value, back, stat=st)
      call check_text(hex(back), want, name // ': encodes back to its bytes')
      built = build(tree)
      call guatiao_to_wire(built, back, stat=st)
      call check_text(hex(back), want, name // ': the tree built here encodes to its bytes')
      call value%close()
      call built%close()
    end do
  end subroutine

  subroutine malformed()
    type(guatiao_value) :: list, entry, value, before
    character(:), allocatable :: msg
    integer :: i, st
    list = vectors%key('malformed')
    call check(list%length() > 5, 'malformed vectors are there')
    before = guatiao_of('kept')
    value = before
    do i = 1, list%length()
      entry = list%item(i)
      call guatiao_from_wire(unhex(text_at(entry, 'hex')), value, stat=st, errmsg=msg)
      call check_status(st, GUATIAO_ERR_BAD_VALUE, text_at(entry, 'name') // ': refused')
      call check(value%string_or('') == 'kept', text_at(entry, 'name') // ': the value left as it was')
    end do
  end subroutine

  subroutine frames()
    type(guatiao_value) :: list, entry, tree
    integer(int8), allocatable :: want(:), got(:)
    character(:), allocatable :: build_kind
    integer(int64) :: channel
    integer :: i, st
    list = vectors%key('frames')
    do i = 1, list%length()
      entry = list%item(i)
      want = unhex(text_at(entry, 'hex'))
      channel = channel_of(want)
      build_kind = text_at(entry, 'build')
      select case (build_kind)
      case ('announce')
        tree = entry%key('tree')
        call guatiao_wire_announce(channel, build(tree), got, stat=st)
      case ('send')
        tree = entry%key('tree')
        call guatiao_wire_send(channel, build(tree), got, stat=st)
      case default
        call guatiao_wire_close(channel, got, stat=st)
      end select
      call check_status(st, GUATIAO_OK, build_kind // ' frame built')
      call check_text(hex(got), hex(want), build_kind // ' frame: the documented bytes')
    end do
  end subroutine

  integer function refusal_code(name)
    character(*), intent(in) :: name
    select case (name)
    case ('malformed'); refusal_code = GUATIAO_WIRE_MALFORMED
    case ('no_schema'); refusal_code = GUATIAO_WIRE_NO_SCHEMA
    case ('not_a_schema'); refusal_code = GUATIAO_WIRE_NOT_A_SCHEMA
    case ('invalid'); refusal_code = GUATIAO_WIRE_INVALID
    case default; refusal_code = -1
    end select
  end function

  subroutine conversation()
    type(guatiao_value) :: steps, step, item, tree, statuses
    type(guatiao_receiver) :: r, copy
    type(guatiao_wire_event) :: event
    type(guatiao_wire_refusal) :: why
    character(:), allocatable :: what, refusal, msg, label
    integer(int64) :: channel
    integer :: i, st
    character(8) :: n
    statuses = vectors%key('statuses')
    steps = vectors%key('conversation')
    call check(steps%length() > 10, 'the conversation is there')
    r = guatiao_open_receiver()
    copy = r
    do i = 1, steps%length()
      step = steps%item(i)
      write (n, '(i0)') i
      label = 'step ' // trim(n)
      item = step%key('channel')
      channel = item%int_or(0_int64)
      call r%accept(unhex(text_at(step, 'frame')), event, why, stat=st, errmsg=msg)
      what = text_at(step, 'event')
      if (len(what) > 0) then
        call check_status(st, GUATIAO_OK, label // ': read')
        select case (what)
        case ('schema'); call check(event%kind == GUATIAO_FRAME_SCHEMA, label // ': a schema')
        case ('value'); call check(event%kind == GUATIAO_FRAME_VALUE, label // ': a value')
        case default; call check(event%kind == GUATIAO_FRAME_CLOSE, label // ': a close')
        end select
        call check(event%channel == channel, label // ': its channel')
        tree = step%key('tree')
        if (tree%is_absent()) then
          call check(event%value%is_absent(), label // ': no value')
        else
          call check(matches(event%value, tree), label // ': the value is the vector''s')
        end if
        cycle
      end if
      refusal = text_at(step, 'refusal')
      call check_text(guatiao_status_name(st), text_at(statuses, refusal), label // ' (' // refusal // '): its status')
      call check(why%code == refusal_code(refusal), label // ' (' // refusal // '): its code')
      call check(why%channel == channel, label // ': its channel')
      call check(event%value%is_absent(), label // ': no value')
      call check(len(why%message) > 0, label // ': words')
      call check(index(why%message, 'hunter2') == 0 .and. index(msg, 'hunter2') == 0, &
                 label // ': the words quote nothing received')
      item = step%key('kind')
      if (.not. item%is_absent()) call check(why%kind == item%int_or(-1_int64), label // ': the frame''s kind')
      item = step%key('at')
      if (.not. item%is_absent()) call check(why%at == item%int_or(-1_int64), label // ': the byte')
      item = step%key('key')
      if (.not. item%is_absent()) then
        call check_text(why%key, text_at(step, 'key'), label // ': the refused field')
        call check(why%detail%tag() == GUATIAO_TAG_MAP, label // ': the detail map')
      else
        call check(why%detail%is_absent(), label // ': no detail')
      end if
    end do
    call copy%close()
    call check(r%is_closed(), 'closing one copy closes the receiver')
    call r%close()
    call r%accept(unhex('0201'), event, stat=st)
    call check_status(st, GUATIAO_STAT_CLOSED, 'a closed receiver refuses')
  end subroutine

  ! `name` is defined as `n` in the header.
  subroutine defined(header, name, n)
    character(*), intent(in) :: header, name
    integer, intent(in) :: n
    character(8) :: digits
    write (digits, '(i0)') n
    call check(index(header, '#define ' // name // ' ' // trim(digits) // new_line('a')) > 0, &
               name // ' is the header''s')
  end subroutine

  subroutine codes_and_layout()
    character(:), allocatable :: header
    type(gt_wire_event) :: ev
    call read_file(repo_path('crates/guatiao/include/guatiao.h'), header)
    call defined(header, 'GUATIAO_FRAME_SCHEMA', GUATIAO_FRAME_SCHEMA)
    call defined(header, 'GUATIAO_FRAME_VALUE', GUATIAO_FRAME_VALUE)
    call defined(header, 'GUATIAO_FRAME_CLOSE', GUATIAO_FRAME_CLOSE)
    call defined(header, 'GUATIAO_WIRE_ACCEPTED', GUATIAO_WIRE_ACCEPTED)
    call defined(header, 'GUATIAO_WIRE_MALFORMED', GUATIAO_WIRE_MALFORMED)
    call defined(header, 'GUATIAO_WIRE_NO_SCHEMA', GUATIAO_WIRE_NO_SCHEMA)
    call defined(header, 'GUATIAO_WIRE_NOT_A_SCHEMA', GUATIAO_WIRE_NOT_A_SCHEMA)
    call defined(header, 'GUATIAO_WIRE_INVALID', GUATIAO_WIRE_INVALID)
    call check(c_sizeof(ev) == 144, 'guatiao_wire_event is 144 bytes')
  end subroutine

end program test_wire
