! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> The interfaces against the committed headers: every export this binding
!> calls is declared there with as many arguments as its abstract
!> interface takes; every status and tag has the header's number; every
!> mirror has the header's size on a 64-bit platform.
program test_abi
  use, intrinsic :: iso_c_binding
  use harness
  use guatiao_c
  use guatiao_serde_c
  use guatiao_intake_c
  implicit none

  type :: decl
    character(:), allocatable :: name
    integer :: nargs = -1
  end type

  type(decl), allocatable :: header(:), fortran(:)

  call against('crates/guatiao/include/guatiao.h', 'bindings/fortran/src/guatiao_c.f90', gt_core_exports)
  call against('crates/guatiao-serde/include/guatiao_serde.h', 'bindings/fortran/src/guatiao_serde_c.f90', &
               gt_serde_exports)
  call against('crates/guatiao-intake/include/guatiao_intake.h', 'bindings/fortran/src/guatiao_intake_c.f90', &
               gt_intake_exports)
  call constants()
  call sizes()
  call finish('abi')

contains

  subroutine against(header_path, source_path, table)
    character(*), intent(in) :: header_path, source_path
    type(gt_export), intent(in) :: table(:)
    character(:), allocatable :: text
    integer :: i, h, f
    call read_file(repo_path(header_path), text)
    call c_declarations(text, header)
    call read_file(repo_path(source_path), text)
    call fortran_interfaces(text, fortran)
    call check(size(header) > 5, header_path // ' declares functions')
    call check(size(fortran) == size(table), source_path // ': an interface per export in the table')
    do i = 1, size(table)
      h = find(header, trim(table(i)%name))
      f = find(fortran, trim(table(i)%name))
      call check(h > 0, trim(table(i)%name) // ' is declared in ' // header_path)
      call check(f > 0, trim(table(i)%name) // ' has an abstract interface')
      if (h > 0) call check(header(h)%nargs == table(i)%nargs, trim(table(i)%name) // &
                            ': the table has the header''s argument count')
      if (h > 0 .and. f > 0) call check(header(h)%nargs == fortran(f)%nargs, trim(table(i)%name) // &
                                        ': the interface has the header''s argument count')
    end do
  end subroutine

  integer function find(list, name)
    type(decl), intent(in) :: list(:)
    character(*), intent(in) :: name
    integer :: i
    find = 0
    do i = 1, size(list)
      if (list(i)%name == name) then
        find = i
        return
      end if
    end do
  end function

  ! The functions declared inside the header's extern "C" block, comments
  ! and static inline helpers aside, with their argument counts.
  subroutine c_declarations(text, out)
    character(*), intent(in) :: text
    type(decl), allocatable, intent(out) :: out(:)
    character(:), allocatable :: body, piece
    integer :: a, b, start, semi
    a = index(text, 'extern "C" {')
    b = index(text, '}  // extern "C"')
    body = strip_comments(text(a + 12:b - 1))
    allocate(out(0))
    start = 1
    do
      semi = index(body(start:), ';')
      if (semi == 0) exit
      piece = body(start:start + semi - 2)
      start = start + semi
      call one(piece, out)
    end do
  end subroutine

  subroutine one(piece, out)
    ! One declaration, when it is a function named guatiao_*.
    character(*), intent(in) :: piece
    type(decl), allocatable, intent(inout) :: out(:)
    integer :: open, i, depth, commas, s
    character(:), allocatable :: inside
    type(decl) :: d
    if (index(piece, 'static') > 0) return
    open = index(piece, '(')
    if (open == 0) return
    s = open - 1
    do while (s > 0)
      if (.not. ident(piece(s:s))) exit
      s = s - 1
    end do
    d%name = piece(s + 1:open - 1)
    if (index(d%name, 'guatiao_') /= 1) return
    depth = 0
    commas = 0
    inside = ''
    do i = open, len(piece)
      select case (piece(i:i))
      case ('(')
        depth = depth + 1
      case (')')
        depth = depth - 1
        if (depth == 0) exit
      case (',')
        if (depth == 1) commas = commas + 1
      case default
        if (depth == 1 .and. piece(i:i) /= ' ') inside = inside // piece(i:i)
      end select
    end do
    d%nargs = commas + 1
    if (inside == '' .or. inside == 'void') d%nargs = 0
    out = [out, d]
  end subroutine

  ! `function guatiao_<name>_i(a, b) bind(c)` / `subroutine ...` lines.
  subroutine fortran_interfaces(text, out)
    character(*), intent(in) :: text
    type(decl), allocatable, intent(out) :: out(:)
    integer :: at, open, close, i, n
    type(decl) :: d
    allocate(out(0))
    at = 1
    do
      n = index(text(at:), '_i(')
      if (n == 0) exit
      open = at + n + 1
      i = open - 3
      do while (i > 0)
        if (.not. ident(text(i:i))) exit
        i = i - 1
      end do
      d%name = text(i + 1:open - 3)
      close = open + index(text(open:), ')') - 1
      at = close
      if (index(text(max(1, i - 12):i), 'function') == 0 .and. &
          index(text(max(1, i - 12):i), 'subroutine') == 0) cycle
      if (close == open + 1) then
        d%nargs = 0
      else
        d%nargs = count_char(text(open:close), ',') + 1
      end if
      out = [out, d]
    end do
  end subroutine

  integer function count_char(s, c)
    character(*), intent(in) :: s
    character, intent(in) :: c
    integer :: i
    count_char = 0
    do i = 1, len(s)
      if (s(i:i) == c) count_char = count_char + 1
    end do
  end function

  logical function ident(c)
    character, intent(in) :: c
    ident = (c >= 'a' .and. c <= 'z') .or. (c >= 'A' .and. c <= 'Z') .or. (c >= '0' .and. c <= '9') &
            .or. c == '_'
  end function

  ! Text without /* */ or // comments or preprocessor lines, whitespace
  ! runs as one space.
  function strip_comments(text) result(out)
    character(*), intent(in) :: text
    character(:), allocatable :: out
    integer :: i, n, eol
    logical :: space
    allocate(character(len(text)) :: out)
    n = 0
    i = 1
    space = .false.
    do while (i <= len(text))
      if (text(i:i) == '#') then
        if (i == 1) then
          eol = 0
        else
          eol = index(text(1:i - 1), achar(10), back=.true.)
        end if
        if (len_trim(text(eol + 1:i - 1)) == 0) then
          eol = index(text(i:), achar(10))
          if (eol == 0) exit
          i = i + eol - 1
          cycle
        end if
      end if
      if (i < len(text)) then
        if (text(i:i + 1) == '/*') then
          i = i + index(text(i + 2:), '*/') + 3
          cycle
        else if (text(i:i + 1) == '//') then
          i = i + index(text(i:), achar(10))
          cycle
        end if
      end if
      if (text(i:i) == ' ' .or. text(i:i) == achar(10) .or. text(i:i) == achar(13) .or. &
          text(i:i) == achar(9)) then
        if (.not. space) then
          n = n + 1
          out(n:n) = ' '
        end if
        space = .true.
      else
        n = n + 1
        out(n:n) = text(i:i)
        space = .false.
      end if
      i = i + 1
    end do
    out = out(1:n)
  end function

  ! The numbers enum guatiao_status_code and enum guatiao_tag give.
  subroutine constants()
    character(:), allocatable :: text
    call read_file(repo_path('crates/guatiao/include/guatiao.h'), text)
    text = strip_comments(text)
    call named(text, 'GUATIAO_OK', GUATIAO_OK)
    call named(text, 'GUATIAO_ERR_BAD_VALUE', GUATIAO_ERR_BAD_VALUE)
    call named(text, 'GUATIAO_ERR_ALLOC', GUATIAO_ERR_ALLOC)
    call named(text, 'GUATIAO_ERR_WRONG_KIND', GUATIAO_ERR_WRONG_KIND)
    call named(text, 'GUATIAO_ERR_NOT_FOUND', GUATIAO_ERR_NOT_FOUND)
    call named(text, 'GUATIAO_ERR_NULL', GUATIAO_ERR_NULL)
    call named(text, 'GUATIAO_ERR_GONE', GUATIAO_ERR_GONE)
    call named(text, 'GUATIAO_ERR_UNSUPPORTED', GUATIAO_ERR_UNSUPPORTED)
    call named(text, 'GUATIAO_ERR_BUSY', GUATIAO_ERR_BUSY)
    call named(text, 'GUATIAO_ERR_INTERNAL', GUATIAO_ERR_INTERNAL)
    call named(text, 'GUATIAO_ABSENT', GUATIAO_TAG_ABSENT)
    call named(text, 'GUATIAO_NULL', GUATIAO_TAG_NULL)
    call named(text, 'GUATIAO_BOOL', GUATIAO_TAG_BOOL)
    call named(text, 'GUATIAO_NUMBER', GUATIAO_TAG_NUMBER)
    call named(text, 'GUATIAO_STRING', GUATIAO_TAG_STRING)
    call named(text, 'GUATIAO_BYTES', GUATIAO_TAG_BYTES)
    call named(text, 'GUATIAO_LIST', GUATIAO_TAG_LIST)
    call named(text, 'GUATIAO_MAP', GUATIAO_TAG_MAP)
    call check(index(text, 'GUATIAO_ERR_') > 0, 'the header declares statuses')
    call every_status_named(text)
  end subroutine

  subroutine named(text, name, want)
    character(*), intent(in) :: text, name
    integer(c_int32_t), intent(in) :: want
    integer :: at, eq, comma, got, ios
    at = index(text, ' ' // name // ' = ')
    call check(at > 0, name // ' is an enum member in the header')
    if (at == 0) return
    eq = at + len(name) + 3
    comma = eq + scan(text(eq:), ',}') - 1
    read (text(eq:comma - 1), *, iostat=ios) got
    call check(ios == 0 .and. got == want, name // ' has the header''s number')
  end subroutine

  ! No status in the header goes without a Fortran name.
  subroutine every_status_named(text)
    character(*), intent(in) :: text
    integer :: at, n, e
    character(:), allocatable :: name
    character(*), parameter :: known = ' GUATIAO_OK GUATIAO_ERR_BAD_VALUE GUATIAO_ERR_ALLOC ' // &
      'GUATIAO_ERR_WRONG_KIND GUATIAO_ERR_NOT_FOUND GUATIAO_ERR_NULL GUATIAO_ERR_GONE ' // &
      'GUATIAO_ERR_UNSUPPORTED GUATIAO_ERR_BUSY GUATIAO_ERR_INTERNAL '
    at = index(text, 'enum guatiao_status_code')
    e = at + index(text(at:), '};')
    at = at + index(text(at:), '{')
    do
      n = index(text(at:e), 'GUATIAO_')
      if (n == 0) exit
      at = at + n - 1
      name = text(at:at + scan(text(at:), ' =,') - 2)
      call check(index(known, ' ' // name // ' ') > 0, name // ' has a Fortran constant')
      at = at + len(name)
    end do
  end subroutine

  subroutine sizes()
    type(gt_value) :: v
    type(gt_entry) :: e
    type(gt_alloc) :: a
    type(gt_provider_error) :: pe
    type(gt_kinds) :: k
    type(gt_kind_table) :: kt
    type(gt_kind_tables) :: kts
    type(gt_provider_info) :: pi
    type(gt_host_services) :: hs
    type(gt_host_info) :: hi
    type(gt_providers) :: ps
    type(gt_library_info) :: li
    type(gt_span) :: sp
    type(gt_text_error) :: te
    type(gt_date) :: d
    type(gt_time) :: t
    type(gt_datetime) :: dt
    type(gt_timestamp) :: ts
    type(gt_kind_header) :: kh
    type(gt_object) :: o
    type(gt_bytes_mut) :: bm
    type(gt_parse_error) :: per
    type(gt_str) :: s
    type(gt_owned) :: ow
    call size_is(c_sizeof(v), 40, 'guatiao_value')
    call size_is(c_sizeof(e), 72, 'guatiao_entry')
    call size_is(c_sizeof(a), 40, 'guatiao_alloc')
    call size_is(c_sizeof(pe), 40, 'guatiao_provider_error')
    call size_is(c_sizeof(k), 16, 'guatiao_kinds')
    call size_is(c_sizeof(kt), 32, 'guatiao_kind_table')
    call size_is(c_sizeof(kts), 24, 'guatiao_kind_tables')
    call size_is(c_sizeof(pi), 152, 'guatiao_provider_info')
    call size_is(c_sizeof(hs), 72, 'guatiao_host_services')
    call size_is(c_sizeof(hi), 64, 'guatiao_host_info')
    call size_is(c_sizeof(ps), 24, 'guatiao_providers')
    call size_is(c_sizeof(li), 80, 'guatiao_library_info')
    call size_is(c_sizeof(sp), 64, 'guatiao_span')
    call size_is(c_sizeof(te), 48, 'guatiao_text_error')
    call size_is(c_sizeof(d), 8, 'guatiao_date')
    call size_is(c_sizeof(t), 8, 'guatiao_time')
    call size_is(c_sizeof(dt), 16, 'guatiao_datetime')
    call size_is(c_sizeof(ts), 16, 'guatiao_timestamp')
    call size_is(c_sizeof(kh), 8, 'guatiao_kind_header')
    call size_is(c_sizeof(o), 24, 'guatiao_object')
    call size_is(c_sizeof(bm), 16, 'guatiao_bytes_mut')
    call size_is(c_sizeof(per), 56, 'guatiao_parse_error')
    call size_is(c_sizeof(s), 16, 'guatiao_str')
    call size_is(c_sizeof(ow), 32, 'guatiao_string')
  end subroutine

  subroutine size_is(got, want, what)
    integer(c_size_t), intent(in) :: got
    integer, intent(in) :: want
    character(*), intent(in) :: what
    call check(got == want, what // ' is the size a C compiler gives it')
  end subroutine

end program test_abi
