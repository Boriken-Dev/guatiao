! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> Dates, times, instants, spans and exact lengths against the shared
!> vectors every implementation reads (crates/guatiao/tests/vectors/time.json),
!> read with guatiao's own JSON parser so 64-bit numbers stay exact.
program test_civil
  use, intrinsic :: iso_fortran_env, only: int64
  use harness
  use guatiao
  use guatiao_serde
  use guatiao_civil
  implicit none

  type(guatiao_value) :: vectors
  character(:), allocatable :: text

  call setup()
  call read_file(repo_path('crates/guatiao/tests/vectors/time.json'), text)
  call guatiao_parse(text, GUATIAO_FORMAT_JSON, vectors)
  call forms(GUATIAO_FORM_DATE, 'date')
  call forms(GUATIAO_FORM_TIME, 'time')
  call forms(GUATIAO_FORM_DATETIME, 'datetime')
  call forms(GUATIAO_FORM_TIMESTAMP, 'timestamp')
  call forms(GUATIAO_FORM_SPAN, 'span')
  call spans_applied('date_add_span', .false.)
  call spans_applied('datetime_add_span', .true.)
  call lengths()
  call out_of_width_fields_are_refused()
  call finish('civil')

contains

  ! Every vector of one form: read, its fields, written back canonical,
  ! normalized; or refused at its offset.
  subroutine forms(form, name)
    integer, intent(in) :: form
    character(*), intent(in) :: name
    type(guatiao_value) :: list, entry, item, elem
    character(:), allocatable :: input, canonical, written, normal
    integer(int64), allocatable :: got(:), want(:)
    type(guatiao_unparsed) :: why
    integer :: i, st, k, refused
    list = vectors%key(name)
    call check(list%length() > 5, name // ' vectors are there')
    do i = 1, list%length()
      entry = list%item(i)
      item = entry%key('text')
      call item%get(input)
      item = entry%key('refused_at')
      if (.not. item%is_absent()) then
        refused = int(item%int_or(-1_int64))
        call read_form(form, input, got, written, st, why)
        call check_status(st, GUATIAO_ERR_BAD_VALUE, name // ' "' // input // '" refused')
        call check(why%at == refused, name // ' "' // input // '" refused at its offset')
        call guatiao_normalize(form, input, normal, stat=st, unparsed=why)
        call check(st == GUATIAO_ERR_BAD_VALUE .and. why%at == refused, &
                   name // ' "' // input // '" refused when normalized')
        cycle
      end if
      item = entry%key('canonical')
      call item%get(canonical)
      item = entry%key('fields')
      allocate(want(item%length()))
      do k = 1, size(want)
        elem = item%item(k)
        call elem%get(want(k))
      end do
      call read_form(form, input, got, written, st, why)
      call check_status(st, GUATIAO_OK, name // ' "' // input // '" read')
      if (st == GUATIAO_OK) then
        call check(size(got) == size(want), name // ' "' // input // '" field count')
        if (size(got) == size(want)) call check(all(got == want), name // ' "' // input // '" fields')
        call check_text(written, canonical, name // ' "' // input // '" written canonical')
      end if
      call guatiao_normalize(form, input, normal, stat=st)
      call check_text(normal, canonical, name // ' "' // input // '" normalized')
      deallocate(want)
    end do
  end subroutine

  ! Reads `input` as `form`, answering its fields as the C structs hold
  ! them and its canonical text.
  subroutine read_form(form, input, fields, written, st, why)
    integer, intent(in) :: form
    character(*), intent(in) :: input
    integer(int64), allocatable, intent(out) :: fields(:)
    character(:), allocatable, intent(out) :: written
    integer, intent(out) :: st
    type(guatiao_unparsed), intent(out) :: why
    type(guatiao_date) :: d
    type(guatiao_time) :: t
    type(guatiao_datetime) :: dt
    type(guatiao_timestamp) :: ts
    type(guatiao_span) :: s
    written = ''
    allocate(fields(0))
    select case (form)
    case (GUATIAO_FORM_DATE)
      call guatiao_from_text(input, d, stat=st, unparsed=why)
      if (st /= GUATIAO_OK) return
      fields = [int(d%year, int64), int(d%month, int64), int(d%day, int64)]
      call guatiao_to_text(d, written)
    case (GUATIAO_FORM_TIME)
      call guatiao_from_text(input, t, stat=st, unparsed=why)
      if (st /= GUATIAO_OK) return
      fields = [int(t%hour, int64), int(t%minute, int64), int(t%second, int64), int(t%nanosecond, int64)]
      call guatiao_to_text(t, written)
    case (GUATIAO_FORM_DATETIME)
      call guatiao_from_text(input, dt, stat=st, unparsed=why)
      if (st /= GUATIAO_OK) return
      fields = [int(dt%date%year, int64), int(dt%date%month, int64), int(dt%date%day, int64), &
                int(dt%time%hour, int64), int(dt%time%minute, int64), int(dt%time%second, int64), &
                int(dt%time%nanosecond, int64)]
      call guatiao_to_text(dt, written)
    case (GUATIAO_FORM_TIMESTAMP)
      call guatiao_from_text(input, ts, stat=st, unparsed=why)
      if (st /= GUATIAO_OK) return
      fields = [ts%seconds, int(ts%nanoseconds, int64)]
      call guatiao_to_text(ts, written)
    case (GUATIAO_FORM_SPAN)
      call guatiao_from_text(input, s, stat=st, unparsed=why)
      if (st /= GUATIAO_OK) return
      fields = [s%years, s%months, s%weeks, s%days, s%hours, s%minutes, s%seconds, int(s%nanoseconds, int64)]
      call guatiao_to_text(s, written)
    end select
  end subroutine

  subroutine spans_applied(name, with_time)
    character(*), intent(in) :: name
    logical, intent(in) :: with_time
    type(guatiao_value) :: list, entry, item
    character(:), allocatable :: start, span_text, result, written
    type(guatiao_date) :: d, dr
    type(guatiao_datetime) :: dt, dtr
    type(guatiao_span) :: s
    integer :: i, st
    list = vectors%key(name)
    call check(list%length() > 5, name // ' vectors are there')
    do i = 1, list%length()
      entry = list%item(i)
      item = entry%key('start')
      call item%get(start)
      item = entry%key('span')
      call item%get(span_text)
      item = entry%key('result')
      call guatiao_from_text(span_text, s)
      if (with_time) then
        call guatiao_from_text(start, dt)
        call guatiao_add_span(dt, s, dtr, stat=st)
        if (st == GUATIAO_OK) call guatiao_to_text(dtr, written)
      else
        call guatiao_from_text(start, d)
        call guatiao_add_span(d, s, dr, stat=st)
        if (st == GUATIAO_OK) call guatiao_to_text(dr, written)
      end if
      if (item%is_null()) then
        call check(st /= GUATIAO_OK, name // ' ' // start // ' + ' // span_text // ' refused')
      else
        call item%get(result)
        call check_status(st, GUATIAO_OK, name // ' ' // start // ' + ' // span_text)
        if (st == GUATIAO_OK) call check_text(written, result, name // ' ' // start // ' + ' // span_text)
      end if
    end do
  end subroutine

  subroutine lengths()
    type(guatiao_value) :: list, entry, value, want, back, elem
    type(guatiao_length) :: got, again
    integer(int64) :: seconds, nanos
    integer :: i, st
    list = vectors%key('duration')
    call check(list%length() > 5, 'duration vectors are there')
    do i = 1, list%length()
      entry = list%item(i)
      value = entry%key('value')
      want = entry%key('duration')
      call guatiao_read_length(value, got, stat=st)
      if (want%is_null()) then
        call check(st /= GUATIAO_OK, 'a length refused')
        cycle
      end if
      call check_status(st, GUATIAO_OK, 'a length read')
      elem = want%item(1)
      call elem%get(seconds)
      elem = want%item(2)
      call elem%get(nanos)
      call check(got%seconds == seconds .and. got%nanoseconds == nanos, 'a length''s seconds and nanoseconds')
      back = guatiao_length_value(got)
      call guatiao_read_length(back, again)
      call check(again%seconds == got%seconds .and. again%nanoseconds == got%nanoseconds, &
                 'a length written as [s, ns] reads back')
    end do
  end subroutine

  subroutine out_of_width_fields_are_refused()
    type(guatiao_date) :: d
    character(:), allocatable :: text
    integer :: st
    d = guatiao_date(2026, 300, 1)
    call guatiao_to_text(d, text, stat=st)
    call check_status(st, GUATIAO_ERR_BAD_VALUE, 'a month past a byte is refused, not wrapped')
    d = guatiao_date(2026, 2, 30)
    call guatiao_to_text(d, text, stat=st)
    call check_status(st, GUATIAO_ERR_BAD_VALUE, 'a day the month lacks')
  end subroutine

end program test_civil
