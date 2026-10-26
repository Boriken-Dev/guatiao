! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> Documents straight into a derived type of the program's own and back
!> out of it, through a visitor and a walker, with no value built; what
!> each side refuses.
module visit_fleet
  use, intrinsic :: iso_fortran_env, only: int64, real64
  use, intrinsic :: iso_c_binding, only: c_int8_t
  use guatiao
  use guatiao_serde
  implicit none

  type :: server
    character(64) :: host = ''
    integer :: port = 0
    real(real64) :: weight = 0
    character(32) :: weight_text = ''
    logical :: enabled = .false.
    character(16), allocatable :: tags(:)
  end type

  !> Reads {"name": ..., "servers": [{"host", "port", "weight", "enabled",
  !> "tags"}, ...]}: a key it does not know is skipped with its value.
  type, extends(guatiao_visitor) :: fleet_reader
    character(:), allocatable :: name
    type(server), allocatable :: servers(:)
    character(:), allocatable :: key
    integer :: depth = 0
    integer :: skipping = 0 ! how deep inside a value whose key is not known
    integer :: events = 0
  contains
    procedure :: on_null => rd_null
    procedure :: on_boolean => rd_boolean
    procedure :: on_number => rd_number
    procedure :: on_string => rd_string
    procedure :: on_list_begin => rd_list_begin
    procedure :: on_list_end => rd_end
    procedure :: on_map_begin => rd_map_begin
    procedure :: on_key => rd_key
    procedure :: on_map_end => rd_end
  end type

  !> Writes a fleet back out; `mode` breaks it on purpose.
  type, extends(guatiao_walker) :: fleet_writer
    character(:), allocatable :: name
    type(server), allocatable :: servers(:)
    integer :: mode = 0
  contains
    procedure :: walk => wr_walk
  end type

  integer, parameter :: WRITE_WHOLE = 0, WRITE_KEY_TWICE = 1, WRITE_BAD_NUMBER = 2, WRITE_NOTHING = 3, &
                        WRITE_OWN_STATUS = 4, WRITE_LIST = 5, WRITE_UNCLOSED = 6, WRITE_ABSENT = 7

contains

  ! Whether the value now arriving belongs to a key this reader skips.
  logical function skipped(self)
    class(fleet_reader), intent(in) :: self
    skipped = self%skipping > 0
  end function

  integer function rd_null(self) result(st)
    class(fleet_reader), intent(inout) :: self
    self%events = self%events + 1
    st = GUATIAO_OK
  end function

  integer function rd_boolean(self, value) result(st)
    class(fleet_reader), intent(inout) :: self
    logical, intent(in) :: value
    self%events = self%events + 1
    st = GUATIAO_OK
    if (skipped(self)) return
    if (self%depth == 3 .and. self%key == 'enabled') self%servers(size(self%servers))%enabled = value
  end function

  integer function rd_number(self, text) result(st)
    class(fleet_reader), intent(inout) :: self
    character(*), intent(in) :: text
    integer :: n, at
    self%events = self%events + 1
    st = GUATIAO_OK
    if (skipped(self) .or. self%depth /= 3) return
    at = size(self%servers)
    select case (self%key)
    case ('port')
      read (text, *) n
      if (n < 0) then
        st = GUATIAO_ERR_BAD_VALUE ! a port is never negative: the walk stops here
        return
      end if
      self%servers(at)%port = n
    case ('weight')
      read (text, *) self%servers(at)%weight
      self%servers(at)%weight_text = text
    end select
  end function

  integer function rd_string(self, text) result(st)
    class(fleet_reader), intent(inout) :: self
    character(*), intent(in) :: text
    self%events = self%events + 1
    st = GUATIAO_OK
    if (skipped(self)) return
    if (self%depth == 1 .and. self%key == 'name') then
      self%name = text
    else if (self%depth == 3 .and. self%key == 'host') then
      self%servers(size(self%servers))%host = text
    else if (self%depth == 4) then
      call add_tag(self%servers(size(self%servers)), text)
    end if
  end function

  subroutine add_tag(s, tag)
    type(server), intent(inout) :: s
    character(*), intent(in) :: tag
    character(16), allocatable :: grown(:)
    allocate(grown(size(s%tags) + 1))
    grown(1:size(s%tags)) = s%tags
    grown(size(grown)) = tag
    call move_alloc(grown, s%tags)
  end subroutine

  integer function rd_list_begin(self, len) result(st)
    class(fleet_reader), intent(inout) :: self
    integer(int64), intent(in) :: len
    self%events = self%events + 1
    st = GUATIAO_OK
    call opened(self)
    if (.not. skipped(self) .and. self%depth == 2) then
      if (allocated(self%servers)) deallocate(self%servers)
      allocate(self%servers(0))
    end if
  end function

  integer function rd_map_begin(self, len) result(st)
    class(fleet_reader), intent(inout) :: self
    integer(int64), intent(in) :: len
    type(server), allocatable :: grown(:)
    integer :: n
    self%events = self%events + 1
    st = GUATIAO_OK
    call opened(self)
    if (.not. skipped(self) .and. self%depth == 3) then
      n = size(self%servers)
      allocate(grown(n + 1))
      if (n > 0) grown(1:n) = self%servers
      allocate(grown(n + 1)%tags(0))
      call move_alloc(grown, self%servers)
    end if
  end function

  ! A container opens: one level deeper, and inside a skipped value deeper still.
  subroutine opened(self)
    class(fleet_reader), intent(inout) :: self
    self%depth = self%depth + 1
    if (self%skipping > 0) self%skipping = self%skipping + 1
  end subroutine

  integer function rd_end(self) result(st)
    class(fleet_reader), intent(inout) :: self
    self%events = self%events + 1
    st = GUATIAO_OK
    self%depth = self%depth - 1
    if (self%skipping > 0) self%skipping = self%skipping - 1
  end function

  integer function rd_key(self, text) result(st)
    class(fleet_reader), intent(inout) :: self
    character(*), intent(in) :: text
    self%events = self%events + 1
    st = GUATIAO_OK
    if (self%skipping > 1) return
    self%key = text
    self%skipping = 0
    select case (self%depth)
    case (1)
      if (text /= 'name' .and. text /= 'servers') self%skipping = 1
    case (3)
      if (text /= 'host' .and. text /= 'port' .and. text /= 'weight' .and. text /= 'enabled' .and. &
          text /= 'tags') self%skipping = 1
    end select
  end function

  integer function wr_walk(self, into) result(st)
    class(fleet_writer), intent(inout) :: self
    type(guatiao_sink), intent(in) :: into
    integer :: i, j
    character(16) :: buf
    select case (self%mode)
    case (WRITE_NOTHING)
      st = GUATIAO_OK
      return
    case (WRITE_OWN_STATUS)
      st = GUATIAO_ERR_UNSUPPORTED
      return
    case (WRITE_LIST)
      st = into%list_begin(1_int64)
      if (st == GUATIAO_OK) st = into%null()
      if (st == GUATIAO_OK) st = into%list_end()
      return
    case (WRITE_UNCLOSED)
      st = into%map_begin(GUATIAO_LEN_UNKNOWN)
      return
    case (WRITE_ABSENT)
      st = into%map_begin(1_int64)
      if (st == GUATIAO_OK) st = into%key('gone')
      if (st == GUATIAO_OK) st = into%absent()
      if (st == GUATIAO_OK) st = into%map_end()
      return
    end select
    st = into%map_begin(2_int64)
    if (st == GUATIAO_OK) st = into%key('name')
    if (st == GUATIAO_OK) st = into%string(self%name)
    if (st == GUATIAO_OK) st = into%key('servers')
    if (self%mode == WRITE_KEY_TWICE .and. st == GUATIAO_OK) then
      st = into%null()
      if (st == GUATIAO_OK) st = into%key('servers')
    end if
    if (st == GUATIAO_OK) st = into%list_begin(int(size(self%servers), int64))
    do i = 1, size(self%servers)
      associate (s => self%servers(i))
        if (st == GUATIAO_OK) st = into%map_begin(GUATIAO_LEN_UNKNOWN)
        if (st == GUATIAO_OK) st = into%key('host')
        if (st == GUATIAO_OK) st = into%string(trim(s%host))
        if (st == GUATIAO_OK) st = into%key('port')
        write (buf, '(i0)') s%port
        if (self%mode == WRITE_BAD_NUMBER) buf = '1.'
        if (st == GUATIAO_OK) st = into%number(trim(buf))
        if (st == GUATIAO_OK) st = into%key('weight')
        if (st == GUATIAO_OK) st = into%number(trim(s%weight_text))
        if (st == GUATIAO_OK) st = into%key('enabled')
        if (st == GUATIAO_OK) st = into%boolean(s%enabled)
        if (st == GUATIAO_OK) st = into%key('tags')
        if (st == GUATIAO_OK) st = into%list_begin(int(size(s%tags), int64))
        do j = 1, size(s%tags)
          if (st == GUATIAO_OK) st = into%string(trim(s%tags(j)))
        end do
        if (st == GUATIAO_OK) st = into%list_end()
        if (st == GUATIAO_OK) st = into%map_end()
      end associate
    end do
    if (st == GUATIAO_OK) st = into%list_end()
    if (st == GUATIAO_OK) st = into%map_end()
  end function

end module visit_fleet

program test_visit
  use, intrinsic :: iso_fortran_env, only: int64, real64
  use, intrinsic :: iso_c_binding, only: c_int8_t
  use harness
  use guatiao
  use guatiao_serde
  use visit_fleet
  implicit none

  character(*), parameter :: fleet = '{"name":"fleet","servers":[' // &
    '{"host":"a.example","port":8000,"weight":0.5,"enabled":true,"tags":["edge","eu"]},' // &
    '{"host":"b.example","port":8001,"weight":1.25,"enabled":false,"tags":[]}]}'

  call setup()
  call reads_into_its_own_type()
  call skips_what_it_does_not_know()
  call writes_its_own_type_back()
  call a_visitor_stops_the_walk()
  call an_event_not_taken_is_refused()
  call a_walker_is_held_to_the_encoding()
  call other_formats()
  call finish('visit')

contains

  subroutine reads_into_its_own_type()
    type(fleet_reader) :: r
    integer :: st
    call guatiao_visit(fleet, GUATIAO_FORMAT_JSON, r, stat=st)
    call check_status(st, GUATIAO_OK, 'the document is read')
    call check_text(r%name, 'fleet', 'a top-level string')
    call check(size(r%servers) == 2, 'one server per element')
    call check_text(trim(r%servers(1)%host), 'a.example', 'a nested string')
    call check(r%servers(2)%port == 8001, 'a number read as an integer')
    call check(abs(r%servers(2)%weight - 1.25_real64) < 1e-12_real64, 'a number read as a real')
    call check_text(trim(r%servers(1)%weight_text), '0.5', 'a number''s text')
    call check(r%servers(1)%enabled .and. .not. r%servers(2)%enabled, 'booleans')
    call check(size(r%servers(1)%tags) == 2, 'a nested list')
    call check_text(trim(r%servers(1)%tags(2)), 'eu', 'its items')
    call check(size(r%servers(2)%tags) == 0, 'an empty list')
    call check(r%depth == 0, 'every container closed')
  end subroutine

  subroutine skips_what_it_does_not_know()
    type(fleet_reader) :: r
    integer :: st
    call guatiao_visit('{"name":"x","extra":{"deep":[1,{"name":"not this"}]},"servers":[' // &
                       '{"host":"h","note":[true,null],"port":1}]}', GUATIAO_FORMAT_JSON, r, stat=st)
    call check_status(st, GUATIAO_OK, 'a document with keys the type lacks')
    call check_text(r%name, 'x', 'a nested key of the same name is not taken')
    call check(size(r%servers) == 1 .and. r%servers(1)%port == 1, 'keys after a skipped value')
  end subroutine

  subroutine writes_its_own_type_back()
    type(fleet_reader) :: r
    type(fleet_writer) :: w
    character(:), allocatable :: text
    integer :: st
    call guatiao_visit(fleet, GUATIAO_FORMAT_JSON, r)
    w%name = r%name
    w%servers = r%servers
    call guatiao_emit_walker(w, GUATIAO_FORMAT_JSON, text, stat=st)
    call check_status(st, GUATIAO_OK, 'the walk is written')
    if (st == GUATIAO_OK) call check_text(text, fleet, 'the document again')
    call guatiao_emit_walker(w, GUATIAO_FORMAT_JSON, text, pretty=.true.)
    call check(index(text, new_line('a')) > 0, 'pretty output has lines')
  end subroutine

  subroutine a_visitor_stops_the_walk()
    type(fleet_reader) :: r
    type(guatiao_parse_error) :: where
    integer :: st
    character(:), allocatable :: msg
    call guatiao_visit('{"servers":[{"port":-1},{"port":2}]}', GUATIAO_FORMAT_JSON, r, stat=st, errmsg=msg)
    call check_status(st, GUATIAO_ERR_BAD_VALUE, 'the status a visitor answered')
    call check(index(msg, 'visitor') > 0, 'the message says the visitor stopped it')
    call check(size(r%servers) == 1, 'nothing after the stop')
    call guatiao_visit('{"name": "x",' // new_line('a') // ' "servers": ]', GUATIAO_FORMAT_JSON, r, where=where, &
                       stat=st, errmsg=msg)
    call check_status(st, GUATIAO_ERR_BAD_VALUE, 'a document that does not parse')
    call check(where%line == 2, 'the line')
    call check(index(msg, 'line 2') > 0, 'the message says where')
  end subroutine

  subroutine an_event_not_taken_is_refused()
    type(fleet_reader) :: r
    integer :: st
    call guatiao_visit('{"name":"data:;base64,AQID"}', GUATIAO_FORMAT_JSON, r, how=GUATIAO_READ_DATA_URIS, stat=st)
    call check_status(st, GUATIAO_ERR_WRONG_KIND, 'bytes, which the reader does not take')
  end subroutine

  subroutine a_walker_is_held_to_the_encoding()
    type(fleet_reader) :: r
    type(fleet_writer) :: w
    character(:), allocatable :: text
    integer :: st
    call guatiao_visit(fleet, GUATIAO_FORMAT_JSON, r)
    w%name = r%name
    w%servers = r%servers
    w%mode = WRITE_KEY_TWICE
    call guatiao_emit_walker(w, GUATIAO_FORMAT_JSON, text, stat=st)
    call check_status(st, GUATIAO_ERR_BAD_VALUE, 'a key twice in one map')
    w%mode = WRITE_BAD_NUMBER
    call guatiao_emit_walker(w, GUATIAO_FORMAT_JSON, text, stat=st)
    call check_status(st, GUATIAO_ERR_BAD_VALUE, 'a number outside the grammar')
    w%mode = WRITE_NOTHING
    call guatiao_emit_walker(w, GUATIAO_FORMAT_JSON, text, stat=st)
    call check_status(st, GUATIAO_ERR_WRONG_KIND, 'a walk that gave no value')
    w%mode = WRITE_ABSENT
    call guatiao_emit_walker(w, GUATIAO_FORMAT_JSON, text, stat=st)
    call check_status(st, GUATIAO_ERR_BAD_VALUE, 'an absent value, which a document cannot spell')
    w%mode = WRITE_UNCLOSED
    call guatiao_emit_walker(w, GUATIAO_FORMAT_JSON, text, stat=st)
    call check_status(st, GUATIAO_ERR_WRONG_KIND, 'a map left open')
    w%mode = WRITE_OWN_STATUS
    call guatiao_emit_walker(w, GUATIAO_FORMAT_JSON, text, stat=st)
    call check_status(st, GUATIAO_ERR_UNSUPPORTED, 'the status a walk answered')
  end subroutine

  subroutine other_formats()
    type(fleet_reader) :: r
    type(fleet_writer) :: w
    character(:), allocatable :: text, msg
    integer :: st
    call guatiao_visit('name: fleet' // new_line('a') // 'servers:' // new_line('a') // '  - host: y.example' // &
                       new_line('a') // '    port: 7' // new_line('a'), GUATIAO_FORMAT_YAML, r, stat=st, errmsg=msg)
    if (st == GUATIAO_STAT_MISSING_SYMBOL) then
      call skip('YAML: ' // msg)
    else
      call check_status(st, GUATIAO_OK, 'YAML read')
      call check(size(r%servers) == 1 .and. r%servers(1)%port == 7, 'YAML into the same type')
    end if
    w%mode = WRITE_LIST
    call guatiao_emit_walker(w, GUATIAO_FORMAT_TOML, text, stat=st, errmsg=msg)
    if (st == GUATIAO_STAT_MISSING_SYMBOL) then
      call skip('TOML: ' // msg)
    else
      call check_status(st, GUATIAO_ERR_WRONG_KIND, 'a TOML document is a map')
    end if
    call guatiao_visit('{}', 99, r, stat=st)
    call check_status(st, GUATIAO_ERR_BAD_VALUE, 'no such format')
  end subroutine

end program test_visit
