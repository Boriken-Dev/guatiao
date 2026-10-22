! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> Dates, times, instants, spans and exact lengths: the forms are text in
!> a value (https://boriken-dev.github.io/guatiao/encodings/), and every
!> text is read and written by the library's own functions.
module guatiao_civil
  use, intrinsic :: iso_c_binding
  use, intrinsic :: iso_fortran_env, only: int8, int32, int64
  use guatiao_c
  use guatiao_native
  use guatiao_errors
  use guatiao_values
  implicit none
  private

  public :: guatiao_date, guatiao_time, guatiao_datetime, guatiao_timestamp, guatiao_span, guatiao_length
  public :: guatiao_unparsed, guatiao_from_text, guatiao_to_text, guatiao_normalize, guatiao_add_span, &
            guatiao_read_length, guatiao_length_value
  public :: GUATIAO_FORM_DATE, GUATIAO_FORM_TIME, GUATIAO_FORM_DATETIME, GUATIAO_FORM_TIMESTAMP, &
            GUATIAO_FORM_SPAN

  integer, parameter :: GUATIAO_FORM_DATE = 1, GUATIAO_FORM_TIME = 2, GUATIAO_FORM_DATETIME = 3, &
                        GUATIAO_FORM_TIMESTAMP = 4, GUATIAO_FORM_SPAN = 5

  !> A day on the calendar: year 0 to 9999, month 1 to 12, day from 1.
  type :: guatiao_date
    integer :: year = 0, month = 0, day = 0
  end type

  !> A time of day with no offset, to the nanosecond.
  type :: guatiao_time
    integer :: hour = 0, minute = 0, second = 0, nanosecond = 0
  end type

  !> A civil date and time, as a wall clock shows them, with no zone.
  type :: guatiao_datetime
    type(guatiao_date) :: date
    type(guatiao_time) :: time
  end type

  !> An instant: seconds from 1970-01-01T00:00:00Z (negative before), then
  !> nanoseconds counted forward, 0 to 999999999.
  type :: guatiao_timestamp
    integer(int64) :: seconds = 0
    integer :: nanoseconds = 0
  end type

  !> A calendar length: every non-zero field carries the one sign.
  type :: guatiao_span
    integer(int64) :: years = 0, months = 0, weeks = 0, days = 0, hours = 0, minutes = 0, seconds = 0
    integer :: nanoseconds = 0
  end type

  !> An exact length of time. `seconds` holds an unsigned 64-bit count in
  !> an int64's bits: past huge(0_int64) it reads negative.
  type :: guatiao_length
    integer(int64) :: seconds = 0
    integer :: nanoseconds = 0
  end type

  !> Where a text was refused: the byte offset, from 0, and what would
  !> have been accepted there. It never quotes the text.
  type :: guatiao_unparsed
    integer :: at = 0
    character(:), allocatable :: expected
  end type

  interface guatiao_from_text
    module procedure date_from_text, time_from_text, datetime_from_text, timestamp_from_text, &
                     span_from_text
  end interface

  interface guatiao_to_text
    module procedure date_to_text, time_to_text, datetime_to_text, timestamp_to_text, span_to_text
  end interface

  interface guatiao_add_span
    module procedure date_add_span, datetime_add_span
  end interface

contains

  ! ---- the C structs ---------------------------------------------------

  ! A field of a C `uint8_t`: refused outside 0 to 255.
  recursive subroutine byte_of(n, b, ok)
    integer, intent(in) :: n
    integer(c_int8_t), intent(out) :: b
    logical, intent(inout) :: ok
    b = 0
    if (n < 0 .or. n > 255) then
      ok = .false.
    else
      b = int(transfer(int(n, c_int32_t), 0_c_int8_t), c_int8_t)
    end if
  end subroutine

  recursive integer function unbyte(b)
    integer(c_int8_t), intent(in) :: b
    unbyte = iand(int(b), 255)
  end function

  recursive subroutine to_c_date(d, c, ok)
    type(guatiao_date), intent(in) :: d
    type(gt_date), intent(out) :: c
    logical, intent(out) :: ok
    ok = .true.
    c%year = int(d%year, c_int32_t)
    call byte_of(d%month, c%month, ok)
    call byte_of(d%day, c%day, ok)
  end subroutine

  recursive subroutine to_c_time(t, c, ok)
    type(guatiao_time), intent(in) :: t
    type(gt_time), intent(out) :: c
    logical, intent(out) :: ok
    ok = .true.
    call byte_of(t%hour, c%hour, ok)
    call byte_of(t%minute, c%minute, ok)
    call byte_of(t%second, c%second, ok)
    if (t%nanosecond < 0) ok = .false.
    c%nanosecond = int(t%nanosecond, c_int32_t)
  end subroutine

  recursive function of_c_date(c) result(d)
    type(gt_date), intent(in) :: c
    type(guatiao_date) :: d
    d%year = c%year
    d%month = unbyte(c%month)
    d%day = unbyte(c%day)
  end function

  recursive function of_c_time(c) result(t)
    type(gt_time), intent(in) :: c
    type(guatiao_time) :: t
    t%hour = unbyte(c%hour)
    t%minute = unbyte(c%minute)
    t%second = unbyte(c%second)
    t%nanosecond = c%nanosecond
  end function

  recursive function to_c_span(s) result(c)
    type(guatiao_span), intent(in) :: s
    type(gt_span) :: c
    c%years = s%years
    c%months = s%months
    c%weeks = s%weeks
    c%days = s%days
    c%hours = s%hours
    c%minutes = s%minutes
    c%seconds = s%seconds
    c%nanoseconds = int(s%nanoseconds, c_int32_t)
  end function

  recursive function of_c_span(c) result(s)
    type(gt_span), intent(in) :: c
    type(guatiao_span) :: s
    s%years = c%years
    s%months = c%months
    s%weeks = c%weeks
    s%days = c%days
    s%hours = c%hours
    s%minutes = c%minutes
    s%seconds = c%seconds
    s%nanoseconds = c%nanoseconds
  end function

  ! What a text error says, and the expected string freed.
  recursive subroutine refusal(e, st, msg, unparsed)
    type(gt_text_error), intent(inout), target :: e
    integer, intent(in) :: st
    character(:), allocatable, intent(out) :: msg
    type(guatiao_unparsed), intent(out), optional :: unparsed
    character(:), allocatable :: expected
    character(24) :: buf
    integer(c_int32_t) :: ignored
    expected = ''
    if (e%expected%tag == GUATIAO_TAG_STRING) call gt_text_of_owned(e%expected%payload, expected)
    ignored = gt_value_free(c_loc(e%expected))
    msg = ''
    if (st == GUATIAO_OK) return
    write (buf, '(i0)') e%at
    msg = 'guatiao: refused at byte ' // trim(buf)
    if (len(expected) > 0) msg = msg // ': expected ' // expected
    if (present(unparsed)) then
      unparsed%at = int(e%at)
      unparsed%expected = expected
    end if
  end subroutine

  ! The text of a STRING value a format call answered, the value freed.
  recursive subroutine text_out(out, text)
    type(gt_value), intent(inout), target :: out
    character(:), allocatable, intent(inout) :: text
    integer(c_int32_t) :: ignored
    call gt_text_of_owned(out%payload, text)
    ignored = gt_value_free(c_loc(out))
  end subroutine

  ! ---- reading text ----------------------------------------------------

  recursive subroutine date_from_text(text, x, stat, errmsg, unparsed)
    character(*), intent(in), target :: text
    type(guatiao_date), intent(inout) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_unparsed), intent(out), optional :: unparsed
    type(gt_date), target :: c
    type(gt_text_error), target :: e
    integer :: st
    character(:), allocatable :: msg
    call gt_need(GT_CORE, st, msg)
    if (st == GUATIAO_OK) then
      st = gt_date_parse(gt_default_alloc(), gt_view(text), c_loc(c), c_loc(e))
      call refusal(e, st, msg, unparsed)
      if (st == GUATIAO_OK) x = of_c_date(c)
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  recursive subroutine time_from_text(text, x, stat, errmsg, unparsed)
    character(*), intent(in), target :: text
    type(guatiao_time), intent(inout) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_unparsed), intent(out), optional :: unparsed
    type(gt_time), target :: c
    type(gt_text_error), target :: e
    integer :: st
    character(:), allocatable :: msg
    call gt_need(GT_CORE, st, msg)
    if (st == GUATIAO_OK) then
      st = gt_time_parse(gt_default_alloc(), gt_view(text), c_loc(c), c_loc(e))
      call refusal(e, st, msg, unparsed)
      if (st == GUATIAO_OK) x = of_c_time(c)
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  recursive subroutine datetime_from_text(text, x, stat, errmsg, unparsed)
    character(*), intent(in), target :: text
    type(guatiao_datetime), intent(inout) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_unparsed), intent(out), optional :: unparsed
    type(gt_datetime), target :: c
    type(gt_text_error), target :: e
    integer :: st
    character(:), allocatable :: msg
    call gt_need(GT_CORE, st, msg)
    if (st == GUATIAO_OK) then
      st = gt_datetime_parse(gt_default_alloc(), gt_view(text), c_loc(c), c_loc(e))
      call refusal(e, st, msg, unparsed)
      if (st == GUATIAO_OK) then
        x%date = of_c_date(c%date)
        x%time = of_c_time(c%time)
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  recursive subroutine timestamp_from_text(text, x, stat, errmsg, unparsed)
    character(*), intent(in), target :: text
    type(guatiao_timestamp), intent(inout) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_unparsed), intent(out), optional :: unparsed
    type(gt_timestamp), target :: c
    type(gt_text_error), target :: e
    integer :: st
    character(:), allocatable :: msg
    call gt_need(GT_CORE, st, msg)
    if (st == GUATIAO_OK) then
      st = gt_timestamp_parse(gt_default_alloc(), gt_view(text), c_loc(c), c_loc(e))
      call refusal(e, st, msg, unparsed)
      if (st == GUATIAO_OK) then
        x%seconds = c%seconds
        x%nanoseconds = c%nanoseconds
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  recursive subroutine span_from_text(text, x, stat, errmsg, unparsed)
    character(*), intent(in), target :: text
    type(guatiao_span), intent(inout) :: x
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_unparsed), intent(out), optional :: unparsed
    type(gt_span), target :: c
    type(gt_text_error), target :: e
    integer :: st
    character(:), allocatable :: msg
    call gt_need(GT_CORE, st, msg)
    if (st == GUATIAO_OK) then
      st = gt_span_parse(gt_default_alloc(), gt_view(text), c_loc(c), c_loc(e))
      call refusal(e, st, msg, unparsed)
      if (st == GUATIAO_OK) x = of_c_span(c)
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  ! ---- writing text ----------------------------------------------------

  recursive subroutine date_to_text(x, text, stat, errmsg)
    type(guatiao_date), intent(in) :: x
    character(:), allocatable, intent(inout) :: text
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_date), target :: c
    type(gt_value), target :: out
    integer :: st
    logical :: ok
    character(:), allocatable :: msg
    call gt_need(GT_CORE, st, msg)
    if (st == GUATIAO_OK) then
      call to_c_date(x, c, ok)
      if (ok) then
        st = gt_date_format(gt_default_alloc(), c_loc(c), c_loc(out))
      else
        st = GUATIAO_ERR_BAD_VALUE
      end if
      if (st == GUATIAO_OK) then
        call text_out(out, text)
      else
        msg = 'guatiao: not a day on the calendar'
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  recursive subroutine time_to_text(x, text, stat, errmsg)
    type(guatiao_time), intent(in) :: x
    character(:), allocatable, intent(inout) :: text
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_time), target :: c
    type(gt_value), target :: out
    integer :: st
    logical :: ok
    character(:), allocatable :: msg
    call gt_need(GT_CORE, st, msg)
    if (st == GUATIAO_OK) then
      call to_c_time(x, c, ok)
      if (ok) then
        st = gt_time_format(gt_default_alloc(), c_loc(c), c_loc(out))
      else
        st = GUATIAO_ERR_BAD_VALUE
      end if
      if (st == GUATIAO_OK) then
        call text_out(out, text)
      else
        msg = 'guatiao: not a time on the clock'
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  recursive subroutine datetime_to_text(x, text, stat, errmsg)
    type(guatiao_datetime), intent(in) :: x
    character(:), allocatable, intent(inout) :: text
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_datetime), target :: c
    type(gt_value), target :: out
    integer :: st
    logical :: ok, ok2
    character(:), allocatable :: msg
    call gt_need(GT_CORE, st, msg)
    if (st == GUATIAO_OK) then
      call to_c_date(x%date, c%date, ok)
      call to_c_time(x%time, c%time, ok2)
      if (ok .and. ok2) then
        st = gt_datetime_format(gt_default_alloc(), c_loc(c), c_loc(out))
      else
        st = GUATIAO_ERR_BAD_VALUE
      end if
      if (st == GUATIAO_OK) then
        call text_out(out, text)
      else
        msg = 'guatiao: not a date and time'
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  recursive subroutine timestamp_to_text(x, text, stat, errmsg)
    type(guatiao_timestamp), intent(in) :: x
    character(:), allocatable, intent(inout) :: text
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_timestamp), target :: c
    type(gt_value), target :: out
    integer :: st
    character(:), allocatable :: msg
    call gt_need(GT_CORE, st, msg)
    if (st == GUATIAO_OK) then
      c%seconds = x%seconds
      c%nanoseconds = int(x%nanoseconds, c_int32_t)
      if (x%nanoseconds < 0) then
        st = GUATIAO_ERR_BAD_VALUE
      else
        st = gt_timestamp_format(gt_default_alloc(), c_loc(c), c_loc(out))
      end if
      if (st == GUATIAO_OK) then
        call text_out(out, text)
      else
        msg = 'guatiao: an instant outside the years 0 to 9999'
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  recursive subroutine span_to_text(x, text, stat, errmsg)
    type(guatiao_span), intent(in) :: x
    character(:), allocatable, intent(inout) :: text
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_span), target :: c
    type(gt_value), target :: out
    integer :: st
    character(:), allocatable :: msg
    call gt_need(GT_CORE, st, msg)
    if (st == GUATIAO_OK) then
      c = to_c_span(x)
      st = gt_span_format(gt_default_alloc(), c_loc(c), c_loc(out))
      if (st == GUATIAO_OK) then
        call text_out(out, text)
      else
        msg = 'guatiao: a span mixing signs, or a field out of range'
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> The canonical spelling of `text` in `form` (GUATIAO_FORM_*).
  recursive subroutine guatiao_normalize(form, text, canonical, stat, errmsg, unparsed)
    integer, intent(in) :: form
    character(*), intent(in), target :: text
    character(:), allocatable, intent(inout) :: canonical
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_unparsed), intent(out), optional :: unparsed
    type(gt_text_error), target :: e
    type(gt_value), target :: out
    integer :: st
    character(:), allocatable :: msg
    call gt_need(GT_CORE, st, msg)
    if (st == GUATIAO_OK) then
      select case (form)
      case (GUATIAO_FORM_DATE)
        st = gt_date_normalize(gt_default_alloc(), gt_view(text), c_loc(out), c_loc(e))
      case (GUATIAO_FORM_TIME)
        st = gt_time_normalize(gt_default_alloc(), gt_view(text), c_loc(out), c_loc(e))
      case (GUATIAO_FORM_DATETIME)
        st = gt_datetime_normalize(gt_default_alloc(), gt_view(text), c_loc(out), c_loc(e))
      case (GUATIAO_FORM_TIMESTAMP)
        st = gt_timestamp_normalize(gt_default_alloc(), gt_view(text), c_loc(out), c_loc(e))
      case (GUATIAO_FORM_SPAN)
        st = gt_span_normalize(gt_default_alloc(), gt_view(text), c_loc(out), c_loc(e))
      case default
        st = GUATIAO_ERR_WRONG_KIND
      end select
      if (st == GUATIAO_ERR_WRONG_KIND) then
        msg = 'guatiao: no such form'
      else
        call refusal(e, st, msg, unparsed)
        if (st == GUATIAO_OK) call text_out(out, canonical)
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  ! ---- spans applied ---------------------------------------------------

  recursive subroutine date_add_span(start, span, result, stat, errmsg)
    type(guatiao_date), intent(in) :: start
    type(guatiao_span), intent(in) :: span
    type(guatiao_date), intent(inout) :: result
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_date), target :: c, out
    type(gt_span), target :: s
    integer :: st
    logical :: ok
    character(:), allocatable :: msg
    call gt_need(GT_CORE, st, msg)
    if (st == GUATIAO_OK) then
      call to_c_date(start, c, ok)
      s = to_c_span(span)
      if (ok) then
        st = gt_date_add_span(c_loc(c), c_loc(s), c_loc(out))
      else
        st = GUATIAO_ERR_BAD_VALUE
      end if
      if (st == GUATIAO_OK) then
        result = of_c_date(out)
      else
        msg = 'guatiao: the span leaves the calendar, or does not apply to a date'
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  recursive subroutine datetime_add_span(start, span, result, stat, errmsg)
    type(guatiao_datetime), intent(in) :: start
    type(guatiao_span), intent(in) :: span
    type(guatiao_datetime), intent(inout) :: result
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_datetime), target :: c, out
    type(gt_span), target :: s
    integer :: st
    logical :: ok, ok2
    character(:), allocatable :: msg
    call gt_need(GT_CORE, st, msg)
    if (st == GUATIAO_OK) then
      call to_c_date(start%date, c%date, ok)
      call to_c_time(start%time, c%time, ok2)
      s = to_c_span(span)
      if (ok .and. ok2) then
        st = gt_datetime_add_span(c_loc(c), c_loc(s), c_loc(out))
      else
        st = GUATIAO_ERR_BAD_VALUE
      end if
      if (st == GUATIAO_OK) then
        result%date = of_c_date(out%date)
        result%time = of_c_time(out%time)
      else
        msg = 'guatiao: the span leaves the calendar'
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  ! ---- exact lengths ---------------------------------------------------

  !> Reads an exact length in any of its spellings: `[seconds,
  !> nanoseconds]`, a number of seconds, `{secs, nanos}`.
  recursive subroutine guatiao_read_length(value, length, stat, errmsg)
    type(guatiao_value), intent(in) :: value
    type(guatiao_length), intent(inout) :: length
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    integer(c_int64_t), target :: seconds
    integer(c_int32_t), target :: nanoseconds
    type(c_ptr) :: node
    integer :: st
    character(:), allocatable :: msg
    call gt_need(GT_CORE, st, msg)
    if (st == GUATIAO_OK) then
      node = gt_node(value)
      if (.not. c_associated(node)) then
        st = GUATIAO_ERR_NOT_FOUND
        msg = 'guatiao: no length there'
      else
        st = gt_duration_read(node, c_loc(seconds), c_loc(nanoseconds))
        if (st == GUATIAO_OK) then
          length%seconds = seconds
          length%nanoseconds = nanoseconds
        else
          msg = 'guatiao: not an exact length'
        end if
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> An exact length as its value, `[seconds, nanoseconds]`.
  recursive function guatiao_length_value(length, stat, errmsg) result(v)
    type(guatiao_length), intent(in) :: length
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_value) :: v
    integer :: st
    character(:), allocatable :: msg
    v = guatiao_list(stat=st, errmsg=msg)
    if (st == GUATIAO_OK) then
      if (length%nanoseconds < 0 .or. length%nanoseconds > 999999999) then
        st = GUATIAO_ERR_BAD_VALUE
        msg = 'guatiao: nanoseconds run from 0 to 999999999'
      else
        call v%push(guatiao_number(unsigned_text(length%seconds)), st, msg)
        if (st == GUATIAO_OK) call v%push(length%nanoseconds, st, msg)
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end function

  ! The decimal text of an unsigned 64-bit count held in an int64.
  recursive function unsigned_text(n) result(text)
    integer(int64), intent(in) :: n
    character(:), allocatable :: text
    character(24) :: buf
    integer(int64) :: high, low
    if (n >= 0) then
      write (buf, '(i0)') n
      text = trim(buf)
      return
    end if
    ! The count u = n + 2**64: u / 10 = (u / 2) / 5, and the last digit is
    ! twice (u / 2) mod 5 plus u's parity, all without overflow.
    high = ishft(n, -1) / 5
    low = 2 * mod(ishft(n, -1), 5_int64) + iand(n, 1_int64)
    write (buf, '(i0,i0)') high, low
    text = trim(buf)
  end function

end module guatiao_civil
