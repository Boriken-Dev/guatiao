! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> A kind module written from a kinds document: what the `guatiao-kind`
!> program runs, and what a test checking a committed module calls.
!>
!> The document (`<crate>.kinds.json`, or one another language wrote) is
!> read by the library itself (`guatiao_kind_document_read`), so a library
!> new enough to export it must be found. The module it writes is what
!> "Writing a kind module" in the binding's AGENTS.md describes, one per
!> document: per kind an abstract type, a remote type, a description, a
!> serving function for a provider kind, and a bind(c) procedure per slot.
module guatiao_kind_writer
  use, intrinsic :: iso_c_binding
  use, intrinsic :: iso_fortran_env, only: int64
  use guatiao_c, only: gt_kind_document_read, GUATIAO_TAG_STRING
  use guatiao_native, only: GT_CORE, gt_need, gt_default_alloc, gt_view
  use guatiao_errors
  use guatiao_values, only: guatiao_value, gt_new_owned
  implicit none
  private

  public :: guatiao_kinds_source, guatiao_write_kinds

  ! An argument or an answer.
  type :: gk_type
    character(:), allocatable :: name      ! the Fortran name; '' for an answer
    character(:), allocatable :: type      ! as the description spells it
    character(:), allocatable :: pass      ! borrowed, taken, lent; '' for none
    character(:), allocatable :: kind      ! an object's kind, as its Fortran type
    logical :: nullable = .false.
  end type

  type :: gk_slot
    character(:), allocatable :: name, doc
    logical :: required = .false., fallible = .false., answers = .false.
    type(gk_type), allocatable :: args(:)
    type(gk_type) :: answer
    integer :: index = 0       ! the remote's slot number: from 1, destroy first for an object
  end type

  type :: gk_kind
    character(:), allocatable :: name, id, doc
    integer(int64) :: floor_hash = 0
    logical :: object = .false.
    type(gk_slot), allocatable :: slots(:)
  end type

  ! Text written so far: `text(:n)`. It grows by doubling, never by
  ! concatenation, which flang on Windows builds on the stack.
  type :: gk_out
    character(:), allocatable :: text
    integer :: n = 0
  end type

  character(*), parameter :: OBJ_BY_ADDRESS = '#if defined(__flang__) && (defined(__aarch64__) || defined(_WIN32))'
  character(*), parameter :: SPLIT_VIEW = '#elif defined(__flang__) && defined(__x86_64__)'
  character(*), parameter :: LF = achar(10)

contains

  !> The module named `module` for the kinds document `document` (its JSON
  !> text); `source` names the document in the module's first line.
  !> GUATIAO_ERR_BAD_VALUE, `errmsg` naming the place, for a document
  !> that is no description or holds what this binding cannot carry.
  recursive subroutine guatiao_kinds_source(document, module, source, text, stat, errmsg)
    character(*), intent(in) :: document, module, source
    character(:), allocatable, intent(out) :: text
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_value) :: doc, why
    type(gk_kind), allocatable :: kinds(:)
    character(:), allocatable :: msg
    type(c_ptr) :: root, why_root
    integer :: st
    text = ''
    msg = ''
    call gt_need(GT_CORE, st, msg)
    if (st == GUATIAO_OK .and. .not. associated(gt_kind_document_read)) then
      st = GUATIAO_STAT_MISSING_SYMBOL
      msg = 'guatiao: the loaded library reads no kinds document (guatiao_kind_document_read): use a newer one'
    end if
    if (st == GUATIAO_OK) then
      call gt_new_owned(doc, gt_default_alloc(), root)
      call gt_new_owned(why, gt_default_alloc(), why_root)
      st = gt_kind_document_read(gt_view(document), gt_default_alloc(), root, why_root)
      if (st /= GUATIAO_OK) msg = source // ': ' // why%string_or('not a kinds document')
    end if
    if (st == GUATIAO_OK) then
      call read_kinds(doc, kinds, msg)
      if (len(msg) == 0) call name_arguments(kinds, msg)
      if (len(msg) == 0) call check_names(kinds, module, msg)
      if (len(msg) == 0) call emit_module(kinds, module, source, text, msg)
      if (len(msg) > 0) then
        st = GUATIAO_ERR_BAD_VALUE
        msg = source // ': ' // msg
        text = ''
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> Writes the module for the kinds document at `document` to `path`;
  !> with `check`, writes nothing and answers GUATIAO_ERR_BAD_VALUE when
  !> `path` does not hold what it would write. `module` defaults to the
  !> document's name without `.kinds.json`, plus `s` (`greeter_kinds`).
  recursive subroutine guatiao_write_kinds(document, path, module, check, stat, errmsg)
    character(*), intent(in) :: document, path
    character(*), intent(in), optional :: module
    logical, intent(in), optional :: check
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    character(:), allocatable :: json, there, text, base, name, msg
    integer :: st, u, ios, cut
    logical :: found
    msg = ''
    base = document
    cut = max(index(base, '/', back=.true.), index(base, '\', back=.true.))
    base = base(cut + 1:)
    name = base
    if (index(name, '.') > 0) name = name(:index(name, '.') - 1)
    if (len(base) > 11) then
      if (base(len(base) - 10:) == '.kinds.json') name = base(:len(base) - 11) // 's'
    end if
    if (present(module)) name = module
    call read_text(document, json, found)
    if (.not. found) then
      st = GUATIAO_ERR_NOT_FOUND
      msg = 'guatiao-kind: no kinds document at ' // document
      call gt_report(st, msg, stat, errmsg)
      return
    end if
    call guatiao_kinds_source(json, name, base, text, st, msg)
    if (st == GUATIAO_OK) then
      if (present(check)) then
        if (check) then
          call read_text(path, there, found)
          if (.not. found .or. len(there) /= len(text) .or. there /= text) then
            st = GUATIAO_ERR_BAD_VALUE
            msg = 'guatiao-kind: ' // path // ' is not what guatiao-kind writes from ' // base // ': write it again'
          end if
          call gt_report(st, msg, stat, errmsg)
          return
        end if
      end if
      open (newunit=u, file=path, access='stream', form='unformatted', action='write', status='replace', &
            iostat=ios)
      if (ios /= 0) then
        st = GUATIAO_ERR_NOT_FOUND
        msg = 'guatiao-kind: cannot write ' // path
      else
        write (u, iostat=ios) text
        close (u)
        if (ios /= 0) then
          st = GUATIAO_ERR_NOT_FOUND
          msg = 'guatiao-kind: cannot write ' // path
        end if
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  ! The bytes of the file at `path`, read in blocks: flang on Windows
  ! answers -1 for a file's size.
  recursive subroutine read_text(path, text, found)
    character(*), intent(in) :: path
    character(:), allocatable, intent(out) :: text
    logical, intent(out) :: found
    character(4096) :: block
    character :: one
    type(gk_out) :: o
    integer :: u, ios, at
    text = ''
    open (newunit=u, file=path, access='stream', form='unformatted', action='read', status='old', iostat=ios)
    found = ios == 0
    if (.not. found) return
    at = 1
    do
      read (u, pos=at, iostat=ios) block
      if (ios /= 0) exit
      call append(o, block)
      at = at + len(block)
    end do
    do
      read (u, pos=at, iostat=ios) one
      if (ios /= 0) exit
      call append(o, one)
      at = at + 1
    end do
    close (u)
    call take_text(o, text)
  end subroutine

  ! ---- reading the document ------------------------------------------------

  recursive function text_at(v, key) result(s)
    type(guatiao_value), intent(in) :: v
    character(*), intent(in) :: key
    character(:), allocatable :: s
    type(guatiao_value) :: at
    at = v%key(key)
    s = at%string_or('')
  end function

  recursive logical function bool_at(v, key)
    type(guatiao_value), intent(in) :: v
    character(*), intent(in) :: key
    type(guatiao_value) :: at
    at = v%key(key)
    bool_at = at%bool_or(.false.)
  end function

  recursive subroutine read_type(v, t)
    type(guatiao_value), intent(in) :: v
    type(gk_type), intent(out) :: t
    t%name = text_at(v, 'name')
    t%type = text_at(v, 'type')
    t%pass = text_at(v, 'pass')
    t%kind = text_at(v, 'kind')
    t%nullable = bool_at(v, 'nullable')
  end subroutine

  recursive subroutine read_kinds(doc, kinds, why)
    type(guatiao_value), intent(in) :: doc
    type(gk_kind), allocatable, intent(out) :: kinds(:)
    character(:), allocatable, intent(inout) :: why
    type(guatiao_value) :: list, k, slots, s, args, a, at
    integer :: i, j, m, n
    list = doc%key('kinds')
    allocate(kinds(list%length()))
    do i = 1, size(kinds)
      k = list%item(i)
      kinds(i)%name = text_at(k, 'name')
      kinds(i)%id = ident(kinds(i)%name)
      kinds(i)%doc = text_at(k, 'doc')
      at = k%key('floor_hash')
      kinds(i)%floor_hash = at%int_or(-1_int64)
      kinds(i)%object = bool_at(k, 'object')
      slots = k%key('slots')
      allocate(kinds(i)%slots(slots%length()))
      do j = 1, size(kinds(i)%slots)
        s = slots%item(j)
        kinds(i)%slots(j)%name = text_at(s, 'name')
        kinds(i)%slots(j)%doc = text_at(s, 'doc')
        kinds(i)%slots(j)%required = bool_at(s, 'required')
        kinds(i)%slots(j)%fallible = bool_at(s, 'fallible')
        kinds(i)%slots(j)%index = j
        if (kinds(i)%object) kinds(i)%slots(j)%index = j + 1
        args = s%key('args')
        allocate(kinds(i)%slots(j)%args(args%length()))
        do m = 1, size(kinds(i)%slots(j)%args)
          a = args%item(m)
          call read_type(a, kinds(i)%slots(j)%args(m))
        end do
        a = s%key('answer')
        kinds(i)%slots(j)%answers = .not. a%is_absent()
        if (kinds(i)%slots(j)%answers) call read_type(a, kinds(i)%slots(j)%answer)
      end do
    end do
    ! An object kind a slot names is one of these, by its Fortran name.
    do i = 1, size(kinds)
      do j = 1, size(kinds(i)%slots)
        do m = 1, size(kinds(i)%slots(j)%args)
          call resolve(kinds(i)%slots(j)%args(m), kinds(i)%name // '.' // kinds(i)%slots(j)%name)
        end do
        if (kinds(i)%slots(j)%answers) &
          call resolve(kinds(i)%slots(j)%answer, kinds(i)%name // '.' // kinds(i)%slots(j)%name)
        if (len(why) > 0) return
      end do
    end do
  contains
    recursive subroutine resolve(t, where)
      type(gk_type), intent(inout) :: t
      character(*), intent(in) :: where
      integer :: n2
      if (t%type /= 'object') return
      do n2 = 1, size(kinds)
        if (kinds(n2)%name == t%kind) then
          if (.not. kinds(n2)%object) then
            why = where // ': ' // t%kind // ' is a kind but no object kind'
            return
          end if
          t%kind = kinds(n2)%id
          return
        end if
      end do
      why = where // ': the object kind ' // t%kind // ' is not in this document'
    end subroutine
  end subroutine

  ! A Fortran name for `s`: letters, digits and underscores, starting
  ! with a letter.
  recursive function ident(s) result(r)
    character(*), intent(in) :: s
    character(:), allocatable :: r
    integer :: i
    character :: c
    r = s
    do i = 1, len(r)
      c = r(i:i)
      if (.not. (c >= 'a' .and. c <= 'z' .or. c >= 'A' .and. c <= 'Z' .or. c >= '0' .and. c <= '9')) r(i:i) = '_'
    end do
    if (len(r) == 0) then
      r = 'k'
    else if (.not. (r(1:1) >= 'a' .and. r(1:1) <= 'z' .or. r(1:1) >= 'A' .and. r(1:1) <= 'Z')) then
      r = 'k_' // r
    end if
  end function

  ! Every argument's Fortran name: the description's, unless the module
  ! already gives that name to something (a kind's type, a procedure) or
  ! the generated code uses it, when `_in` is added (`listener_in`).
  recursive subroutine name_arguments(kinds, why)
    type(gk_kind), intent(inout) :: kinds(:)
    character(:), allocatable, intent(inout) :: why
    character(*), parameter :: RESERVED = ' self answer error size merge int real allocated associated present &
      &move_alloc iand ior storage_size int8 int16 int32 int64 logical character integer type class impl '
    integer :: i, j, m, n
    character(:), allocatable :: f, where, low
    do i = 1, size(kinds)
      if (kinds(i)%floor_hash < 0) then
        why = kinds(i)%name // ': a floor hash is a 32-bit unsigned integer'
        return
      end if
      do j = 1, size(kinds(i)%slots)
        associate (s => kinds(i)%slots(j))
          where = kinds(i)%name // '.' // s%name
          if (ident(s%name) /= s%name .or. any(lower(s%name) == [character(12) :: 'r', 'has', 'close', 'destroy', &
              'impl', 'header', 'guatiao_impl']) .or. lower(s%name) == lower(kinds(i)%id)) then
            why = where // ': ' // s%name // ' names no method a kind type may have'
            return
          end if
          do m = 1, j - 1
            if (lower(kinds(i)%slots(m)%name) == lower(s%name)) then
              why = where // ': two slots are ' // s%name // ' in Fortran, which ignores case'
              return
            end if
          end do
          if (.not. s%required .and. any(lower(s%name) == [character(12) :: 'present', 'ior', 'iand', 'int', &
              'storage_size', 'c_loc', 'c_funloc', 'c_sizeof'])) then
            why = where // ': an appended slot is an argument of the kind''s serving function, which ' // &
                  'calls the intrinsic ' // s%name
            return
          end if
          if (.not. s%required .and. j < size(kinds(i)%slots)) then
            if (kinds(i)%slots(j + 1)%required) then
              why = kinds(i)%name // '.' // kinds(i)%slots(j + 1)%name // &
                    ': a required slot after an appended one: the required slots come first'
              return
            end if
          end if
          do m = 1, size(s%args)
            f = ident(s%args(m)%name)
            ! A variable, not an associate: gfortran 16 frees an associated
            ! function result twice here.
            low = lower(f)
            if (index(RESERVED, ' ' // low // ' ') > 0 .or. taken(low) .or. starts(low, 'gk_') .or. &
                starts(low, 'gt_') .or. starts(low, 'c_') .or. starts(low, 'guatiao_')) f = f // '_in'
            if (len(f) > 63) then
              why = where // '(' // s%args(m)%name // '): longer than a Fortran name may be'
              return
            end if
            do n = 1, m - 1
              if (lower(s%args(n)%name) == lower(f)) then
                why = where // ': two arguments are ' // f // ' in Fortran, which ignores case'
                return
              end if
            end do
            s%args(m)%name = f
          end do
        end associate
      end do
    end do
  contains
    ! Whether the module gives `f` to something of its own.
    recursive logical function taken(f)
      character(*), intent(in) :: f
      integer :: k2
      taken = .false.
      do k2 = 1, size(kinds)
        associate (id => kinds(k2)%id)
          if (f == id .or. f == id // '_remote' .or. f == id // '_kind' .or. f == id // '_serving' .or. &
              f == id // '_bind' .or. f == id // '_bind_instance' .or. f == id // '_vtable' .or. &
              f == id // '_tables' .or. f == id // '_table' .or. f == 'is_' // id .or. starts(f, id // '_')) then
            taken = .true.
            return
          end if
        end associate
      end do
    end function
  end subroutine

  ! Whether the slot lends a tree.
  recursive logical function lends_tree(s)
    type(gk_slot), intent(in) :: s
    integer :: m
    lends_tree = .false.
    do m = 1, size(s%args)
      if (s%args(m)%pass == 'lent' .and. s%args(m)%type /= 'object') lends_tree = .true.
    end do
  end function

  recursive function lower(s) result(r)
    character(*), intent(in) :: s
    character(:), allocatable :: r
    integer :: i
    r = s
    do i = 1, len(r)
      if (r(i:i) >= 'A' .and. r(i:i) <= 'Z') r(i:i) = achar(iachar(r(i:i)) + 32)
    end do
  end function

  ! Refuses two things the module would give one name, Fortran ignoring
  ! case, and a kind named like what the module uses.
  recursive subroutine check_names(kinds, module, why)
    type(gk_kind), intent(in) :: kinds(:)
    character(*), intent(in) :: module
    character(:), allocatable, intent(inout) :: why
    character(len=80), allocatable :: names(:)
    integer :: i, j, n, m
    allocate(names(4 + 13 * size(kinds) + 4 * sum([(size(kinds(i)%slots), i = 1, size(kinds))])))
    n = 0
    call add(module)
    call add('gk_lacks')
    call add('gk_on')
    call add('gk_destroy_slot')
    do i = 1, size(kinds)
      associate (id => kinds(i)%id)
        if (starts(lower(id), 'gt_') .or. starts(lower(id), 'c_') .or. starts(lower(id), 'gk_') .or. &
            starts(lower(id), 'guatiao') .or. any(lower(id) == [character(5) :: 'int8', 'int16', 'int32', 'int64'])) then
          why = kinds(i)%name // ': a kind named ' // id // ' would take a name the module uses'
          return
        end if
        call add(id)
        call add(id // '_remote')
        call add(id // '_kind')
        call add(id // '_serving')
        call add(id // '_bind')
        call add(id // '_bind_instance')
        call add(id // '_vtable')
        call add(id // '_table')
        call add(id // '_tables')
        call add('is_' // id)
        call add(id // '_floor_hash')
        call add(id // '_remote_has')
        call add(id // '_remote_close')
        do j = 1, size(kinds(i)%slots)
          call add(id // '_' // kinds(i)%slots(j)%name)
          call add(id // '_' // kinds(i)%slots(j)%name // '_c')
          call add(id // '_' // kinds(i)%slots(j)%name // '_slot')
          call add(id // '_remote_' // kinds(i)%slots(j)%name)
        end do
      end associate
    end do
    do i = 1, n
      do m = 1, i - 1
        if (names(m) == names(i)) then
          why = 'two things in the module would be named ' // trim(names(i)) // ' (Fortran ignores case)'
          return
        end if
      end do
      if (len_trim(names(i)) > 63) then
        why = trim(names(i)) // ' is longer than a Fortran name may be'
        return
      end if
    end do
  contains
    recursive subroutine add(name)
      character(*), intent(in) :: name
      n = n + 1
      names(n) = lower(name)
    end subroutine
  end subroutine

  recursive logical function starts(s, prefix)
    character(*), intent(in) :: s, prefix
    starts = .false.
    if (len(s) >= len(prefix)) starts = s(:len(prefix)) == prefix
  end function

  recursive function itoa(i) result(s)
    integer, intent(in) :: i
    character(:), allocatable :: s
    character(12) :: b
    write (b, '(i0)') i
    s = trim(b)
  end function

  recursive function utoa(i) result(s)
    integer(int64), intent(in) :: i
    character(:), allocatable :: s
    character(24) :: b
    write (b, '(i0)') i
    s = trim(b)
  end function

  recursive function upper(s) result(r)
    character(*), intent(in) :: s
    character(:), allocatable :: r
    integer :: i
    r = s
    do i = 1, len(r)
      if (r(i:i) >= 'a' .and. r(i:i) <= 'z') r(i:i) = achar(iachar(r(i:i)) - 32)
    end do
  end function

  recursive function quoted(s) result(r)
    character(*), intent(in) :: s
    character(:), allocatable :: r
    integer :: i
    r = "'"
    do i = 1, len(s)
      if (s(i:i) == "'") then
        r = r // "''"
      else
        r = r // s(i:i)
      end if
    end do
    r = r // "'"
  end function

  ! ---- writing ---------------------------------------------------------------

  recursive subroutine put(o, line)
    type(gk_out), intent(inout) :: o
    character(*), intent(in) :: line
    character(:), allocatable :: rest
    character :: quote
    integer :: at, i
    rest = line
    do while (len(rest) > 130 .and. rest(1:1) /= '#' .and. adjustl(rest) /= '' .and. &
              index(adjustl(rest), '!') /= 1)
      at = index(rest(:120), ' ', back=.true.)
      if (at <= 8) exit
      quote = ' '
      do i = 1, at
        if (quote == ' ' .and. (rest(i:i) == "'" .or. rest(i:i) == '"')) then
          quote = rest(i:i)
        else if (rest(i:i) == quote) then
          quote = ' '
        end if
      end do
      if (quote == ' ') then
        call append(o, rest(:at - 1) // ' &' // LF)
        rest = '        ' // rest(at + 1:)
      else
        call append(o, rest(:at) // '&' // LF)
        rest = '        &' // rest(at + 1:)
      end if
    end do
    call append(o, rest)
    call append(o, LF)
  end subroutine

  recursive subroutine append(o, s)
    type(gk_out), intent(inout) :: o
    character(*), intent(in) :: s
    character(:), allocatable :: bigger
    if (.not. allocated(o%text)) allocate(character(65536) :: o%text)
    if (o%n + len(s) > len(o%text)) then
      allocate(character(max(2 * len(o%text), o%n + len(s))) :: bigger)
      bigger(:o%n) = o%text(:o%n)
      call move_alloc(bigger, o%text)
    end if
    o%text(o%n + 1:o%n + len(s)) = s
    o%n = o%n + len(s)
  end subroutine

  ! What `o` holds, as text of its own length.
  recursive subroutine take_text(o, text)
    type(gk_out), intent(in) :: o
    character(:), allocatable, intent(out) :: text
    allocate(character(o%n) :: text)
    if (o%n > 0) text(:) = o%text(:o%n)
  end subroutine

  ! `head`, the items joined by ', ', then `tail`, on as many lines as
  ! keep each within 100 columns, continued with '&'.
  recursive subroutine put_list(o, head, items, tail)
    type(gk_out), intent(inout) :: o
    character(*), intent(in) :: head, tail
    character(*), intent(in) :: items(:)
    character(:), allocatable :: line, pad
    integer :: i
    pad = repeat(' ', len(head) - len(adjustl(head)) + 4)
    line = head
    do i = 1, size(items)
      if (i > 1) line = line // ', '
      if (len(line) + len_trim(items(i)) > 100 .and. len_trim(line) > len(pad)) then
        call put(o, trim(line) // ' &')
        line = pad
      end if
      line = line // trim(items(i))
    end do
    call put(o, line // tail)
  end subroutine

  ! A doc as `!>` lines at `indent`, each within 100 columns.
  recursive subroutine put_doc(o, indent, doc)
    type(gk_out), intent(inout) :: o
    character(*), intent(in) :: indent, doc
    character(:), allocatable :: rest, line
    integer :: at, cut
    if (len(doc) == 0) return
    rest = doc
    do
      at = index(rest, LF)
      if (at == 0) then
        line = rest
      else
        line = rest(:at - 1)
      end if
      do while (len(line) > 90)
        cut = index(line(:90), ' ', back=.true.)
        if (cut <= 1) exit
        call put(o, indent // '!> ' // line(:cut - 1))
        line = line(cut + 1:)
      end do
      if (len(line) == 0) then
        call put(o, indent // '!>')
      else
        call put(o, indent // '!> ' // line)
      end if
      if (at == 0) exit
      rest = rest(at + 1:)
    end do
  end subroutine

  recursive logical function is_scalar(t)
    character(*), intent(in) :: t
    is_scalar = any(t == [character(5) :: 'bool', 'i8', 'i16', 'i32', 'i64', 'u8', 'u16', 'u32', 'u64', &
                                          'isize', 'usize', 'f32', 'f64'])
  end function

  ! The Fortran type a method sees for a scalar.
  recursive function fortran_scalar(t) result(r)
    character(*), intent(in) :: t
    character(:), allocatable :: r
    select case (t)
    case ('bool'); r = 'logical'
    case ('i8', 'u8'); r = 'integer(int8)'
    case ('i16', 'u16'); r = 'integer(int16)'
    case ('i32', 'u32'); r = 'integer(int32)'
    case ('i64', 'u64'); r = 'integer(int64)'
    case ('isize'); r = 'integer(c_intptr_t)'
    case ('usize'); r = 'integer(c_size_t)'
    case ('f32'); r = 'real(c_float)'
    case default; r = 'real(c_double)'
    end select
  end function

  ! The C type a scalar crosses as.
  recursive function c_scalar(t) result(r)
    character(*), intent(in) :: t
    character(:), allocatable :: r
    select case (t)
    case ('bool', 'i8', 'u8'); r = 'integer(c_int8_t)'
    case ('i16', 'u16'); r = 'integer(c_int16_t)'
    case ('i32', 'u32'); r = 'integer(c_int32_t)'
    case ('i64', 'u64'); r = 'integer(c_int64_t)'
    case ('isize'); r = 'integer(c_intptr_t)'
    case ('usize'); r = 'integer(c_size_t)'
    case ('f32'); r = 'real(c_float)'
    case default; r = 'real(c_double)'
    end select
  end function

  recursive function tag_of(t) result(r)
    character(*), intent(in) :: t
    character(:), allocatable :: r
    select case (t)
    case ('map'); r = 'GUATIAO_TAG_MAP'
    case ('list'); r = 'GUATIAO_TAG_LIST'
    case default; r = 'GUATIAO_TAG_ABSENT'
    end select
  end function

  ! Refuses what this binding cannot carry, naming the place.
  recursive subroutine check_type(t, answer, object, where, why)
    type(gk_type), intent(in) :: t
    logical, intent(in) :: answer, object
    character(*), intent(in) :: where
    character(:), allocatable, intent(inout) :: why
    if (is_scalar(t%type) .or. t%type == 'object') return
    if (answer) then
      if (t%type == 'text' .or. t%type == 'map' .or. t%type == 'list' .or. t%type == 'value') return
      why = where // ': an answer cannot be ' // t%type
      return
    end if
    select case (t%type)
    case ('str', 'bytes')
      return
    case ('bytes_mut')
      if (.not. object) why = where // ': bytes written in place cross on an object kind only'
    case ('map')
      return
    case ('list')
      if (t%pass == 'borrowed') why = where // ': a list is not borrowed'
    case ('value')
      if (t%pass /= 'borrowed') why = where // ': a value ' // t%pass // &
        ' has no Fortran form yet: pass a map or a list, or borrow the value'
    case default
      why = where // ': ' // t%type // ' is not a type an argument can have'
    end select
  end subroutine

  ! A method's dummy declaration for argument `t`.
  recursive function method_decl(t) result(r)
    type(gk_type), intent(in) :: t
    character(:), allocatable :: r
    if (is_scalar(t%type)) then
      r = fortran_scalar(t%type) // ', intent(in) :: ' // t%name
      return
    end if
    select case (t%type)
    case ('str'); r = 'character(*), intent(in) :: ' // t%name
    case ('bytes'); r = 'integer(int8), intent(in) :: ' // t%name // '(:)'
    case ('bytes_mut'); r = 'integer(int8), intent(inout) :: ' // t%name // '(:)'
    case ('object')
      if (t%pass == 'taken') then
        r = 'class(' // t%kind // '), allocatable, intent(inout) :: ' // t%name
      else
        r = 'class(' // t%kind // '), intent(inout), target :: ' // t%name
      end if
    case default
      if (t%pass == 'borrowed') then
        r = 'type(guatiao_value), intent(in) :: ' // t%name
      else
        r = 'type(guatiao_value), intent(inout) :: ' // t%name
      end if
    end select
  end function

  ! A method's declaration of its answer.
  recursive function answer_decl(t) result(r)
    type(gk_type), intent(in) :: t
    character(:), allocatable :: r
    if (is_scalar(t%type)) then
      r = fortran_scalar(t%type) // ', intent(out) :: answer'
    else if (t%type == 'text') then
      r = 'character(:), allocatable, intent(out) :: answer'
    else if (t%type == 'object') then
      r = 'class(' // t%kind // '), allocatable, intent(out) :: answer'
    else
      r = 'type(guatiao_value), intent(inout) :: answer'
    end if
  end function

  ! The statement setting a scalar or text answer before anything else.
  recursive function answer_default(t) result(r)
    type(gk_type), intent(in) :: t
    character(:), allocatable :: r
    r = ''
    if (t%type == 'text') then
      r = "answer = ''"
    else if (t%type == 'bool') then
      r = 'answer = .false.'
    else if (is_scalar(t%type)) then
      r = 'answer = 0'
    end if
  end function

  ! The C parameters of the slot after ctx, as `x<i>` declarations; `how`
  ! 0 by value, 1 an object by address (flang on AArch64 and Windows), 2
  ! a view split in two words (flang on x86-64, at the fifth register).
  recursive subroutine c_params(o, s, how, indent)
    type(gk_out), intent(inout) :: o
    type(gk_slot), intent(in) :: s
    integer, intent(in) :: how
    character(*), intent(in) :: indent
    integer :: m
    character(:), allocatable :: x
    call put(o, indent // 'type(c_ptr), value :: ctx')
    do m = 1, size(s%args)
      x = 'x' // itoa(m)
      associate (t => s%args(m))
        if (is_scalar(t%type)) then
          call put(o, indent // c_scalar(t%type) // ', value :: ' // x)
        else if (t%type == 'str') then
          if (how == 2 .and. m == split_at(s)) then
            call put(o, indent // 'type(c_ptr), value :: ' // x // '_ptr')
            call put(o, indent // 'integer(c_size_t), value :: ' // x // '_len')
          else
            call put(o, indent // 'type(gt_str), value :: ' // x)
          end if
        else if (t%type == 'bytes' .or. t%type == 'bytes_mut') then
          if (how == 2 .and. m == split_at(s)) then
            call put(o, indent // 'type(c_ptr), value :: ' // x // '_ptr')
            call put(o, indent // 'integer(c_size_t), value :: ' // x // '_len')
          else
            call put(o, indent // 'type(gt_bytes_mut), value :: ' // x)
          end if
        else if (t%type == 'object' .and. t%pass == 'taken') then
          if (how == 1) then
            call put(o, indent // 'type(c_ptr), value :: ' // x // '_at')
          else
            call put(o, indent // 'type(gt_object), value :: ' // x)
          end if
        else
          call put(o, indent // 'type(c_ptr), value :: ' // x)
        end if
      end associate
    end do
    if (s%answers) call put(o, indent // 'type(c_ptr), value :: out')
    if (s%fallible) call put(o, indent // 'type(c_ptr), value :: err')
  end subroutine

  recursive function c_names(s, how) result(r)
    type(gk_slot), intent(in) :: s
    integer, intent(in) :: how
    character(:), allocatable :: r
    integer :: m
    r = 'ctx'
    do m = 1, size(s%args)
      if (how == 2 .and. m == split_at(s)) then
        r = r // ', x' // itoa(m) // '_ptr, x' // itoa(m) // '_len'
      else if (how == 1 .and. s%args(m)%type == 'object' .and. s%args(m)%pass == 'taken') then
        r = r // ', x' // itoa(m) // '_at'
      else
        r = r // ', x' // itoa(m)
      end if
    end do
    if (s%answers) r = r // ', out'
    if (s%fallible) r = r // ', err'
  end function

  ! The names of a list written 'a, b, c'.
  recursive function names_of(list) result(names)
    character(*), intent(in) :: list
    character(len=200), allocatable :: names(:)
    integer :: n, at, from
    n = 1
    do at = 1, len(list)
      if (list(at:at) == ',') n = n + 1
    end do
    allocate(names(n))
    from = 1
    n = 0
    do at = 1, len(list) + 1
      if (at > len(list)) then
        n = n + 1
        names(n) = adjustl(list(from:))
      else if (list(at:at) == ',') then
        n = n + 1
        names(n) = adjustl(list(from:at - 1))
        from = at + 1
      end if
    end do
  end function

  ! Whether the slot takes an object by value.
  recursive logical function takes_object(s)
    type(gk_slot), intent(in) :: s
    integer :: m
    takes_object = .false.
    do m = 1, size(s%args)
      if (s%args(m)%type == 'object' .and. s%args(m)%pass == 'taken') takes_object = .true.
    end do
  end function

  ! The argument whose 16-byte view would start at the fifth integer
  ! register of a call (ctx first; a pointer, an integer or a bool one, a
  ! view two, an object by value and a real none), which flang on x86-64
  ! passes wrongly through a procedure pointer; 0 for none.
  recursive integer function split_at(s)
    type(gk_slot), intent(in) :: s
    integer :: m, used
    split_at = 0
    used = 1
    do m = 1, size(s%args)
      associate (t => s%args(m)%type)
        if (t == 'str' .or. t == 'bytes' .or. t == 'bytes_mut') then
          if (used == 4) then
            split_at = m
            return
          end if
          used = used + 2
        else if (t == 'f32' .or. t == 'f64') then
          continue
        else if (t == 'object' .and. s%args(m)%pass == 'taken') then
          continue
        else
          used = used + 1
        end if
      end associate
    end do
  end function

  ! ---- the module ------------------------------------------------------------

  recursive subroutine emit_module(kinds, module, source, text, why)
    type(gk_kind), intent(in) :: kinds(:)
    character(*), intent(in) :: module, source
    character(:), allocatable, intent(out) :: text
    character(:), allocatable, intent(inout) :: why
    type(gk_out) :: o
    character(len=80), allocatable :: names(:)
    integer :: i, j, m, nopt
    logical :: any_object, any_flags
    text = ''
    if (len(ident(module)) == 0 .or. ident(module) /= module) then
      why = module // ' is no Fortran module name'
      return
    end if
    do i = 1, size(kinds)
      do j = 1, i - 1
        if (kinds(j)%id == kinds(i)%id) then
          why = 'two kinds are ' // kinds(i)%id // ' in Fortran'
          return
        end if
      end do
      if (len(kinds(i)%id) + 14 > 63) then
        why = kinds(i)%name // ': a Fortran name longer than 63 characters would be written for it'
        return
      end if
      nopt = 0
      do j = 1, size(kinds(i)%slots)
        associate (s => kinds(i)%slots(j))
          if (.not. s%required) nopt = nopt + 1
          if (len(kinds(i)%id) + len(s%name) + 8 > 63) then
            why = kinds(i)%name // '.' // s%name // ': a Fortran name longer than 63 characters would be written for it'
            return
          end if
          do m = 1, size(s%args)
            call check_type(s%args(m), .false., kinds(i)%object, kinds(i)%name // '.' // s%name // '(' // &
                            s%args(m)%name // ')', why)
          end do
          if (s%answers) call check_type(s%answer, .true., kinds(i)%object, kinds(i)%name // '.' // s%name, why)
          if (len(why) > 0) return
        end associate
      end do
      if (nopt > 12 .and. .not. kinds(i)%object) then
        why = kinds(i)%name // ': more than 12 appended slots (a table per set of them is built)'
        return
      end if
    end do
    any_object = any(kinds%object)
    any_flags = .false.
    do i = 1, size(kinds)
      if (.not. kinds(i)%object .and. any(.not. kinds(i)%slots%required)) any_flags = .true.
    end do

    call put(o, '! GENERATED by guatiao-kind from ' // source // '. DO NOT EDIT.')
    call put(o, '')
    call put(o, '!> The kinds of ' // source // ', written by guatiao-kind: per kind an abstract type')
    call put(o, '!> (a deferred procedure per required slot, an overridable one per appended')
    ! No apostrophe before the module statement: fpm reads one as opening
    ! a string, and finds no module in the file.
    call put(o, '!> slot), a remote type calling a table another side serves, a description')
    call put(o, '!> (`<kind>_kind()`), a serving function naming the appended slots a provider')
    call put(o, '!> overrides, and a bind(c) procedure per slot serving a table to any caller.')
    call put(o, 'module ' // module)
    call put(o, '  use, intrinsic :: iso_c_binding')
    call put(o, '  use, intrinsic :: iso_fortran_env, only: int8, int16, int32, int64')
    call put(o, '  use guatiao')
    call put(o, '  use guatiao_c, only: gt_str, gt_owned, gt_object, gt_bytes_mut, gt_provider_error, ' // &
             'gt_kind_header, gt_value')
    call put(o, '  use guatiao_values, only: gt_end_borrow')
    call put(o, '  use guatiao_host, only: guatiao_registry, guatiao_instance')
    call put(o, '  use guatiao_kinds')
    call put(o, '  implicit none')
    call put(o, '  private')
    call put(o, '')
    call put_public(o, '', '')
    call put_public(o, '', '_remote')
    call put_public(o, '', '_kind')
    if (any(.not. kinds%object)) then
      call put_public(o, '', '_serving', .true.)
      call put_public(o, '', '_bind', .true.)
      call put_public(o, '', '_bind_instance', .true.)
    end if
    allocate(names(size(kinds)))
    do i = 1, size(kinds)
      names(i) = upper(kinds(i)%id) // '_FLOOR_HASH'
    end do
    call put_list(o, '  public :: ', names, '')
    call put(o, '')
    call put(o, '  ! The kinds'' floor hashes, as the uint32 bits an int32 holds.')
    do i = 1, size(kinds)
      if (kinds(i)%floor_hash == 2147483648_int64) then
        call put(o, '  integer(c_int32_t), parameter :: ' // upper(kinds(i)%id) // &
                 '_FLOOR_HASH = -2147483647_c_int32_t - 1_c_int32_t  ! 2147483648')
      else if (kinds(i)%floor_hash > 2147483647_int64) then
        call put(o, '  integer(c_int32_t), parameter :: ' // upper(kinds(i)%id) // '_FLOOR_HASH = ' // &
                 utoa(kinds(i)%floor_hash - 4294967296_int64) // '_c_int32_t  ! ' // utoa(kinds(i)%floor_hash))
      else
        call put(o, '  integer(c_int32_t), parameter :: ' // upper(kinds(i)%id) // '_FLOOR_HASH = ' // &
                 utoa(kinds(i)%floor_hash) // '_c_int32_t')
      end if
    end do

    call put(o, '')
    call put(o, '  ! ---- the tables ------------------------------------------------------')
    do i = 1, size(kinds)
      call put(o, '')
      call put(o, '  type, bind(c) :: ' // kinds(i)%id // '_vtable')
      call put(o, '    type(gt_kind_header) :: header')
      if (kinds(i)%object) call put(o, '    type(c_funptr) :: destroy = c_null_funptr')
      do j = 1, size(kinds(i)%slots)
        call put(o, '    type(c_funptr) :: ' // kinds(i)%slots(j)%name // ' = c_null_funptr')
      end do
      call put(o, '  end type')
    end do

    call put(o, '')
    call put(o, '  ! ---- the kinds -------------------------------------------------------')
    do i = 1, size(kinds)
      call put(o, '')
      call put_doc(o, '  ', kinds(i)%doc)
      call put(o, '  type, abstract, extends(guatiao_impl) :: ' // kinds(i)%id)
      call put(o, '  contains')
      do j = 1, size(kinds(i)%slots)
        associate (s => kinds(i)%slots(j))
          if (s%required) then
            call put(o, '    procedure(' // kinds(i)%id // '_' // s%name // '), deferred :: ' // s%name)
          else
            call put(o, '    procedure :: ' // s%name // ' => ' // kinds(i)%id // '_' // s%name)
          end if
        end associate
      end do
      call put(o, '  end type')
    end do
    call put(o, '')
    call put(o, '  abstract interface')
    do i = 1, size(kinds)
      do j = 1, size(kinds(i)%slots)
        if (kinds(i)%slots(j)%required) call method(o, kinds(i), kinds(i)%slots(j), 'abstract')
      end do
    end do
    call put(o, '  end interface')

    call put(o, '')
    call put(o, '  ! ---- the remotes -----------------------------------------------------')
    do i = 1, size(kinds)
      call put(o, '')
      if (kinds(i)%object) then
        call put(o, '  !> Another side''s ' // kinds(i)%name // ': owned (handed over, destroyed when the last')
        call put(o, '  !> copy goes or on close) or lent for one call.')
      else
        call put(o, '  !> A ' // kinds(i)%name // ' another side serves. A slot its table lacks answers')
        call put(o, '  !> GUATIAO_ERR_UNSUPPORTED (`has` says which it has).')
      end if
      call put(o, '  type, extends(' // kinds(i)%id // ') :: ' // kinds(i)%id // '_remote')
      call put(o, '    type(guatiao_remote) :: r')
      call put(o, '  contains')
      do j = 1, size(kinds(i)%slots)
        call put(o, '    procedure :: ' // kinds(i)%slots(j)%name // ' => ' // kinds(i)%id // '_remote_' // &
                 kinds(i)%slots(j)%name)
      end do
      call put(o, '    procedure :: has => ' // kinds(i)%id // '_remote_has')
      if (kinds(i)%object) call put(o, '    procedure :: close => ' // kinds(i)%id // '_remote_close')
      call put(o, '  end type')
    end do

    call put(o, '')
    call put(o, '  ! ---- the C slots -----------------------------------------------------')
    call put(o, '')
    call put(o, '  abstract interface')
    do i = 1, size(kinds)
      do j = 1, size(kinds(i)%slots)
        call c_interface(o, kinds(i), kinds(i)%slots(j))
      end do
    end do
    call put(o, '  end interface')
    call put(o, '')
    call put(o, '  ! The tables this module serves: a provider kind''s per set of appended')
    call put(o, '  ! slots, one for every object of an object kind.')
    do i = 1, size(kinds)
      nopt = count(.not. kinds(i)%slots%required)
      if (kinds(i)%object .or. nopt == 0) then
        call put(o, '  type(' // kinds(i)%id // '_vtable), target, save :: ' // kinds(i)%id // '_table')
      else
        call put(o, '  type(' // kinds(i)%id // '_vtable), target, save :: ' // kinds(i)%id // '_tables(0:' // &
                 itoa(2**nopt - 1) // ')')
      end if
    end do

    call put(o, '')
    call put(o, 'contains')
    call put(o, '')
    call put(o, '  ! ---- descriptions ----------------------------------------------------')
    do i = 1, size(kinds)
      call describe(o, kinds(i))
    end do
    do i = 1, size(kinds)
      call put(o, '')
      call put(o, '  recursive logical function is_' // kinds(i)%id // '(impl)')
      call put(o, '    class(guatiao_impl), intent(in) :: impl')
      call put(o, '    select type (impl)')
      call put(o, '    class is (' // kinds(i)%id // ')')
      call put(o, '      is_' // kinds(i)%id // ' = .true.')
      call put(o, '    class default')
      call put(o, '      is_' // kinds(i)%id // ' = .false.')
      call put(o, '    end select')
      call put(o, '  end function')
    end do
    do i = 1, size(kinds)
      if (.not. kinds(i)%object) then
        call serving(o, kinds(i))
        call binds(o, kinds(i))
      end if
    end do
    if (any_flags) then
      call put(o, '')
      call put(o, '  recursive logical function gk_on(flag)')
      call put(o, '    logical, intent(in), optional :: flag')
      call put(o, '    gk_on = .false.')
      call put(o, '    if (present(flag)) gk_on = flag')
      call put(o, '  end function')
    end if

    call put(o, '')
    call put(o, '  ! ---- appended slots a provider does not override ---------------------')
    do i = 1, size(kinds)
      do j = 1, size(kinds(i)%slots)
        if (.not. kinds(i)%slots(j)%required) call method(o, kinds(i), kinds(i)%slots(j), 'default')
      end do
    end do

    call put(o, '')
    call put(o, '  ! ---- serving: a bind(c) procedure per slot ---------------------------')
    do i = 1, size(kinds)
      do j = 1, size(kinds(i)%slots)
        call serve_slot(o, kinds(i), kinds(i)%slots(j))
      end do
    end do
    if (any_object) then
      call put(o, '')
      call put(o, '  ! An object''s destroy: the holder lets it go.')
      call put(o, '  recursive subroutine gk_destroy_slot(ctx) bind(c)')
      call put(o, '    type(c_ptr), value :: ctx')
      call put(o, '    call guatiao_destroy_served(ctx)')
      call put(o, '  end subroutine')
    end if

    call put(o, '')
    call put(o, '  ! ---- calling another side''s tables -----------------------------------')
    call put(o, '')
    call put(o, '  ! The slot as a procedure, or a refusal when the table lacks it.')
    call put(o, '  recursive logical function gk_lacks(r, i, error)')
    call put(o, '    type(guatiao_remote), intent(in) :: r')
    call put(o, '    integer, intent(in) :: i')
    call put(o, '    type(guatiao_error), allocatable, intent(inout) :: error')
    call put(o, '    gk_lacks = .not. r%has(i)')
    call put(o, '    if (gk_lacks) then')
    call put(o, '      if (r%is_closed()) then')
    call put(o, "        call guatiao_fail(error, GUATIAO_ERR_GONE, 'the remote is closed')")
    call put(o, '      else')
    call put(o, "        call guatiao_fail(error, GUATIAO_ERR_UNSUPPORTED, 'the table has no such slot')")
    call put(o, '      end if')
    call put(o, '    end if')
    call put(o, '  end function')
    do i = 1, size(kinds)
      do j = 1, size(kinds(i)%slots)
        call method(o, kinds(i), kinds(i)%slots(j), 'remote')
      end do
      call put(o, '')
      call put(o, '  !> Whether the table has the slot named `slot`.')
      call put(o, '  recursive logical function ' // kinds(i)%id // '_remote_has(self, slot)')
      call put(o, '    class(' // kinds(i)%id // '_remote), intent(in) :: self')
      call put(o, '    character(*), intent(in) :: slot')
      call put(o, '    integer :: i')
      call put(o, '    select case (slot)')
      do j = 1, size(kinds(i)%slots)
        call put(o, "    case ('" // kinds(i)%slots(j)%name // "'); i = " // itoa(kinds(i)%slots(j)%index))
      end do
      call put(o, '    case default; i = 0')
      call put(o, '    end select')
      call put(o, '    ' // kinds(i)%id // '_remote_has = .false.')
      call put(o, '    if (i > 0) ' // kinds(i)%id // '_remote_has = self%r%has(i)')
      call put(o, '  end function')
      if (kinds(i)%object) then
        call put(o, '')
        call put(o, '  !> Destroys the object, once, when this side owns it.')
        call put(o, '  recursive subroutine ' // kinds(i)%id // '_remote_close(self)')
        call put(o, '    class(' // kinds(i)%id // '_remote), intent(inout) :: self')
        call put(o, '    call self%r%close()')
        call put(o, '  end subroutine')
      end if
    end do
    call put(o, '')
    call put(o, 'end module ' // module)
    call take_text(o, text)
  contains
    recursive subroutine put_public(o2, prefix, suffix, providers)
      type(gk_out), intent(inout) :: o2
      character(*), intent(in) :: prefix, suffix
      logical, intent(in), optional :: providers
      character(len=80), allocatable :: list(:)
      integer :: n2, k2
      n2 = 0
      allocate(list(size(kinds)))
      do k2 = 1, size(kinds)
        if (present(providers)) then
          if (kinds(k2)%object) cycle
        end if
        n2 = n2 + 1
        list(n2) = prefix // kinds(k2)%id // suffix
      end do
      call put_list(o2, '  public :: ', list(:n2), '')
    end subroutine
  end subroutine

  recursive subroutine describe(o, k)
    type(gk_out), intent(inout) :: o
    type(gk_kind), intent(in) :: k
    integer :: j, nreq
    character(:), allocatable :: id
    id = k%id
    nreq = count(k%slots%required)
    call put(o, '')
    call put(o, '  recursive function ' // id // '_kind() result(gk_k)')
    call put(o, '    type(guatiao_kind) :: gk_k')
    if (.not. k%object) call put(o, '    type(' // id // '_vtable) :: probe')
    call put(o, '    gk_k%name = ' // quoted(k%name))
    call put(o, '    gk_k%floor_hash = ' // upper(id) // '_FLOOR_HASH')
    if (k%object) then
      call put(o, '    gk_k%size = c_sizeof(' // id // '_table)')
      call put(o, '    gk_k%floor_size = c_sizeof(' // id // '_table%header) + ' // itoa(nreq + 1) // &
               ' * c_sizeof(' // id // '_table%destroy)')
      call put(o, '    gk_k%object = .true.')
      call put(o, '    ' // id // '_table%header%struct_size = int(gk_k%size, c_int32_t)')
      call put(o, '    ' // id // '_table%header%floor_hash = ' // upper(id) // '_FLOOR_HASH')
      call put(o, '    ' // id // '_table%destroy = c_funloc(gk_destroy_slot)')
      do j = 1, size(k%slots)
        call put(o, '    ' // id // '_table%' // k%slots(j)%name // ' = c_funloc(' // id // '_' // &
                 k%slots(j)%name // '_slot)')
      end do
      call put(o, '    gk_k%object_table = c_loc(' // id // '_table)')
    else
      call put(o, '    gk_k%size = c_sizeof(probe)')
      if (nreq == 0) then
        call put(o, '    gk_k%floor_size = c_sizeof(probe%header)')
      else
        call put(o, '    gk_k%floor_size = c_sizeof(probe%header) + ' // itoa(nreq) // ' * c_sizeof(probe%' // &
                 k%slots(1)%name // ')')
      end if
    end if
    call put(o, '    gk_k%accepts => is_' // id)
    call put(o, '  end function')
  end subroutine

  recursive subroutine serving(o, k)
    type(gk_out), intent(inout) :: o
    type(gk_kind), intent(in) :: k
    character(len=200), allocatable :: params(:), flags(:)
    integer :: j, n, bit
    character(:), allocatable :: id
    id = k%id
    n = count(.not. k%slots%required)
    allocate(flags(n), params(n + 1))
    params(1) = 'impl'
    bit = 0
    do j = 1, size(k%slots)
      if (.not. k%slots(j)%required) then
        bit = bit + 1
        flags(bit) = k%slots(j)%name
        params(bit + 1) = k%slots(j)%name
      end if
    end do
    call put(o, '')
    if (n > 0) then
      call put(o, '  !> `impl` serving the ' // k%name // ' kind through a table with its required slots')
      call put(o, '  !> and each appended slot it says it overrides; with no `impl`, the table a')
      call put(o, '  !> configured provider''s instances use.')
    else
      call put(o, '  !> `impl` serving the ' // k%name // ' kind; with no `impl`, the table a configured')
      call put(o, '  !> provider''s instances use.')
    end if
    call put_list(o, '  recursive function ' // id // '_serving(', params, ') result(gk_s)')
    call put(o, '    class(' // id // '), intent(in), target, optional :: impl')
    if (n > 0) call put_list(o, '    logical, intent(in), optional :: ', flags, '')
    call put(o, '    type(guatiao_serving) :: gk_s')
    if (n > 0) then
      call put(o, '    integer :: gk_bits')
      call put(o, '    type(' // id // '_vtable), pointer :: gk_t')
      call put(o, '    gk_bits = 0')
      do j = 1, n
        call put(o, '    if (gk_on(' // trim(flags(j)) // ')) gk_bits = ior(gk_bits, ' // itoa(2**(j - 1)) // ')')
      end do
      call put(o, '    gk_t => ' // id // '_tables(gk_bits)')
      call put(o, '    gk_t%header%struct_size = int(storage_size(gk_t) / 8, c_int32_t)')
      call put(o, '    gk_t%header%floor_hash = ' // upper(id) // '_FLOOR_HASH')
      bit = 0
      do j = 1, size(k%slots)
        if (k%slots(j)%required) then
          call put(o, '    gk_t%' // k%slots(j)%name // ' = c_funloc(' // id // '_' // k%slots(j)%name // '_slot)')
        else
          bit = bit + 1
          call put(o, '    if (iand(gk_bits, ' // itoa(2**(bit - 1)) // ') /= 0) gk_t%' // k%slots(j)%name // &
                   ' = c_funloc(' // id // '_' // k%slots(j)%name // '_slot)')
        end if
      end do
      call put(o, '    gk_s%table = c_loc(gk_t)')
    else
      call put(o, '    ' // id // '_table%header%struct_size = int(c_sizeof(' // id // '_table), c_int32_t)')
      call put(o, '    ' // id // '_table%header%floor_hash = ' // upper(id) // '_FLOOR_HASH')
      do j = 1, size(k%slots)
        call put(o, '    ' // id // '_table%' // k%slots(j)%name // ' = c_funloc(' // id // '_' // &
                 k%slots(j)%name // '_slot)')
      end do
      call put(o, '    gk_s%table = c_loc(' // id // '_table)')
    end if
    call put(o, '    gk_s%kind = ' // id // '_kind()')
    call put(o, '    if (present(impl)) gk_s%impl => impl')
    call put(o, '  end function')
  end subroutine

  recursive subroutine binds(o, k)
    type(gk_out), intent(inout) :: o
    type(gk_kind), intent(in) :: k
    character(:), allocatable :: id
    id = k%id
    call put(o, '')
    call put(o, '  !> The provider under `key` as a ' // k%name // '.')
    call put(o, '  recursive subroutine ' // id // '_bind(reg, key, g, stat, errmsg)')
    call put(o, '    type(guatiao_registry), intent(in) :: reg')
    call put(o, '    character(*), intent(in) :: key')
    call put(o, '    type(' // id // '_remote), intent(inout) :: g')
    call put(o, '    integer, intent(out), optional :: stat')
    call put(o, '    character(:), allocatable, intent(inout), optional :: errmsg')
    call put(o, '    call guatiao_bind(reg, key, ' // id // '_kind(), g%r, stat, errmsg)')
    call put(o, '  end subroutine')
    call put(o, '')
    call put(o, '  !> An instance as a ' // k%name // '.')
    call put(o, '  recursive subroutine ' // id // '_bind_instance(inst, g, stat, errmsg)')
    call put(o, '    type(guatiao_instance), intent(in) :: inst')
    call put(o, '    type(' // id // '_remote), intent(inout) :: g')
    call put(o, '    integer, intent(out), optional :: stat')
    call put(o, '    character(:), allocatable, intent(inout), optional :: errmsg')
    call put(o, '    call guatiao_bind_instance(inst, ' // id // '_kind(), g%r, stat, errmsg)')
    call put(o, '  end subroutine')
  end subroutine

  ! A slot's method: `what` 'abstract' (the deferred interface), 'default'
  ! (an appended slot's body a provider overrides) or 'remote'.
  recursive subroutine method(o, k, s, what)
    type(gk_out), intent(inout) :: o
    type(gk_kind), intent(in) :: k
    type(gk_slot), intent(in) :: s
    character(*), intent(in) :: what
    character(len=200), allocatable :: params(:)
    character(:), allocatable :: name, self_type, d
    integer :: m, n
    n = size(s%args)
    allocate(params(n + 2 + merge(1, 0, s%answers)))
    params(1) = 'self'
    do m = 1, n
      params(m + 1) = s%args(m)%name
    end do
    if (s%answers) params(n + 2) = 'answer'
    params(size(params)) = 'error'
    select case (what)
    case ('abstract')
      name = k%id // '_' // s%name
      self_type = 'class(' // k%id // ')'
      call put_doc(o, '    ', s%doc)
      call put_list(o, '    subroutine ' // name // '(', params, ')')
      call put(o, '      import')
      call put(o, '      ' // self_type // ', intent(inout) :: self')
      do m = 1, n
        call put(o, '      ' // method_decl(s%args(m)))
      end do
      if (s%answers) call put(o, '      ' // answer_decl(s%answer))
      call put(o, '      type(guatiao_error), allocatable, intent(out) :: error')
      call put(o, '    end subroutine')
      return
    case ('default')
      name = k%id // '_' // s%name
      self_type = 'class(' // k%id // ')'
    case default
      name = k%id // '_remote_' // s%name
      self_type = 'class(' // k%id // '_remote)'
    end select
    call put(o, '')
    if (what == 'default') call put_doc(o, '  ', s%doc)
    call put_list(o, '  recursive subroutine ' // name // '(', params, ')')
    call put(o, '    ' // self_type // ', intent(inout) :: self')
    do m = 1, n
      call put(o, '    ' // method_decl(s%args(m)))
    end do
    if (s%answers) call put(o, '    ' // answer_decl(s%answer))
    call put(o, '    type(guatiao_error), allocatable, intent(out) :: error')
    if (what == 'default') then
      d = ''
      if (s%answers) d = answer_default(s%answer)
      if (len(d) > 0) call put(o, '    ' // d)
      call put(o, '    call guatiao_fail(error, GUATIAO_ERR_UNSUPPORTED, ''no ' // s%name // ' here'')')
      call put(o, '  end subroutine')
      return
    end if
    call remote_body(o, k, s)
    call put(o, '  end subroutine')
  end subroutine

  recursive subroutine c_interface(o, k, s)
    type(gk_out), intent(inout) :: o
    type(gk_kind), intent(in) :: k
    type(gk_slot), intent(in) :: s
    logical :: by_address, split
    by_address = takes_object(s)
    split = split_at(s) > 0
    if (by_address) then
      call put(o, OBJ_BY_ADDRESS)
      call one(1)
    end if
    if (split) then
      if (by_address) then
        call put(o, SPLIT_VIEW // ' && !defined(_WIN32)')
      else
        call put(o, '#if defined(__flang__) && defined(__x86_64__) && !defined(_WIN32)')
      end if
      call one(2)
    end if
    if (by_address .or. split) call put(o, '#else')
    call one(0)
    if (by_address .or. split) call put(o, '#endif')
  contains
    recursive subroutine one(how)
      integer, intent(in) :: how
      call put_list(o, '    function ' // k%id // '_' // s%name // '_c(', names_of(c_names(s, how)), &
                    ') bind(c) result(status)')
      call put(o, '      import')
      call c_params(o, s, how, '      ')
      call put(o, '      integer(c_int32_t) :: status')
      call put(o, '    end function')
    end subroutine
  end subroutine

  ! The bind(c) procedure serving slot `s` of kind `k`.
  recursive subroutine serve_slot(o, k, s)
    type(gk_out), intent(inout) :: o
    type(gk_kind), intent(in) :: k
    type(gk_slot), intent(in) :: s
    character(len=200), allocatable :: call_args(:)
    character(:), allocatable :: x, a, done, fail
    integer :: m, n
    logical :: by_address
    n = size(s%args)
    by_address = takes_object(s)
    fail = 'guatiao_warn(error, ' // quoted(s%name) // ')'
    if (s%fallible) fail = 'guatiao_failed(error, err)'
    call put(o, '')
    if (by_address) then
      call put(o, OBJ_BY_ADDRESS)
      call head(1)
      call put(o, '#else')
    end if
    call head(0)
    if (by_address) call put(o, '#endif')
    call put(o, '    integer(c_int32_t) :: status')
    call put(o, '    class(guatiao_impl), pointer :: impl')
    do m = 1, n
      a = 'a' // itoa(m)
      associate (t => s%args(m))
        select case (t%type)
        case ('str')
          call put(o, '    character(:), allocatable :: ' // a)
        case ('bytes', 'bytes_mut')
          call put(o, '    integer(int8), pointer :: ' // a // '(:)')
          call put(o, '    logical :: s' // itoa(m))
        case ('bool')
          call put(o, '    logical :: ' // a)
        case ('object')
          if (t%pass == 'taken') then
            call put(o, '    class(' // t%kind // '), allocatable :: ' // a)
            call put(o, '    integer :: st' // itoa(m))
          else
            call put(o, '    type(' // t%kind // '_remote) :: ' // a)
          end if
        case ('map', 'list', 'value')
          call put(o, '    type(guatiao_value) :: ' // a)
          if (t%pass == 'taken') call put(o, '    integer :: st' // itoa(m))
        end select
      end associate
    end do
    if (s%answers) then
      associate (t => s%answer)
        if (is_scalar(t%type)) then
          call put(o, '    ' // fortran_scalar(t%type) // ' :: ans')
          call put(o, '    ' // c_scalar(t%type) // ', pointer :: ap')
        else if (t%type == 'text') then
          call put(o, '    character(:), allocatable :: ans')
        else if (t%type == 'object') then
          call put(o, '    class(' // t%kind // '), allocatable :: ans')
          call put(o, '    class(guatiao_impl), allocatable :: moved')
          call put(o, '    type(gt_object), pointer :: oslot')
        else if (t%type == 'value') then
          call put(o, '    type(guatiao_value) :: ans')
        else
          call put(o, '    type(guatiao_value) :: carrier, ans')
        end if
      end associate
    end if
    call put(o, '    type(guatiao_error), allocatable :: error')
    call put(o, '    integer :: st')
    if (by_address) then
      do m = 1, n
        if (s%args(m)%type == 'object' .and. s%args(m)%pass == 'taken') then
          call put(o, OBJ_BY_ADDRESS)
          call put(o, '    type(gt_object), pointer :: x' // itoa(m))
          call put(o, '#endif')
        end if
      end do
    end if
    ! What was handed over is this side's at once, whatever happens next.
    if (by_address) then
      call put(o, OBJ_BY_ADDRESS)
      do m = 1, n
        if (s%args(m)%type == 'object' .and. s%args(m)%pass == 'taken') &
          call put(o, '    call c_f_pointer(x' // itoa(m) // '_at, x' // itoa(m) // ')')
      end do
      call put(o, '#endif')
    end if
    do m = 1, n
      a = 'a' // itoa(m)
      x = 'x' // itoa(m)
      associate (t => s%args(m))
        if (t%type == 'object' .and. t%pass == 'taken') then
          call put(o, '    allocate(' // t%kind // '_remote :: ' // a // ')')
          call put(o, '    select type (' // a // ')')
          call put(o, '    type is (' // t%kind // '_remote)')
          call put(o, '      call guatiao_take_object(' // x // ', ' // t%kind // '_kind(), ' // a // '%r, st' // &
                   itoa(m) // ')')
          call put(o, '    end select')
        else if ((t%type == 'map' .or. t%type == 'list') .and. t%pass == 'taken') then
          call put(o, '    ' // a // ' = guatiao_taken_' // t%type // '(' // x // ', st' // itoa(m) // ')')
        end if
      end associate
    end do
    ! A tree handed over is moved into this side's allocator, so the
    ! method may keep it: a refused move refuses the call.
    do m = 1, n
      associate (t => s%args(m))
        if ((t%type == 'map' .or. t%type == 'list') .and. t%pass == 'taken') then
          call put(o, '    if (st' // itoa(m) // ' /= GUATIAO_OK) then')
          call put(o, '      call guatiao_fail(error, st' // itoa(m) // ', "the ' // t%type // ' handed over as ' // &
                   t%name // ' could not be moved into this side''s allocator")')
          call put(o, '      status = ' // fail)
          call put(o, '      return')
          call put(o, '    end if')
        end if
      end associate
    end do
    call put(o, '    impl => guatiao_impl_at(ctx, ' // quoted(k%name) // ')')
    call put(o, '    status = GUATIAO_ERR_GONE')
    call put(o, '    if (.not. associated(impl)) return')
    do m = 1, n
      if (s%args(m)%type == 'object' .and. s%args(m)%pass == 'taken') then
        call put(o, '    status = int(st' // itoa(m) // ', c_int32_t)')
        call put(o, '    if (status /= GUATIAO_OK) return')
      end if
    end do
    done = ''
    do m = 1, n
      a = 'a' // itoa(m)
      x = 'x' // itoa(m)
      associate (t => s%args(m))
        select case (t%type)
        case ('object')
          if (t%pass == 'lent') then
            call put(o, '    call guatiao_lend_object(' // x // ', ' // t%kind // '_kind(), ' // a // '%r, st)')
            call put(o, '    if (st /= GUATIAO_OK) then')
            if (len(done) > 0) call put(o, done)
            call put(o, "      call guatiao_fail(error, st, 'no " // t%kind // " was lent as " // t%name // "')")
            call put(o, '      status = ' // fail)
            call put(o, '      return')
            call put(o, '    end if')
            if (len(done) > 0) done = done // LF
            done = done // '      call guatiao_end_lent_object(' // a // '%r)'
          end if
        case ('str')
          call put(o, '    call guatiao_text_in(' // x // ', ' // a // ')')
        case ('bytes', 'bytes_mut')
          call put(o, '    call guatiao_bytes_in(' // x // ', ' // a // ')')
          call put(o, '    s' // itoa(m) // ' = ' // x // '%len == 0 .or. .not. c_associated(' // x // '%ptr)')
        case ('bool')
          call put(o, '    ' // a // ' = ' // x // ' /= 0')
        case ('map', 'list', 'value')
          if (t%pass == 'borrowed') then
            if (t%type == 'map') then
              call put(o, '    ' // a // ' = guatiao_borrowed_map(' // x // ')')
            else
              call put(o, '    ' // a // ' = guatiao_borrowed_value(' // x // ')')
            end if
          else if (t%pass == 'lent') then
            call put(o, '    ' // a // ' = guatiao_lent_' // t%type // '(' // x // ')')
          end if
        end select
      end associate
    end do
    if (s%answers) then
      associate (t => s%answer)
        if (t%type == 'map' .or. t%type == 'list') then
          call put(o, '    carrier = guatiao_' // t%type // '_carrier(out)')
          call put(o, '    ans = carrier')
        else if (t%type == 'value') then
          call put(o, '    ans = guatiao_value_carrier(out)')
        else if (t%type == 'bool') then
          call put(o, '    ans = .false.')
        else if (is_scalar(t%type)) then
          call put(o, '    ans = 0')
        end if
      end associate
    end if
    allocate(call_args(n + 1 + merge(1, 0, s%answers)))
    do m = 1, n
      if (is_scalar(s%args(m)%type) .and. s%args(m)%type /= 'bool') then
        call_args(m) = 'x' // itoa(m)
      else
        call_args(m) = 'a' // itoa(m)
      end if
    end do
    if (s%answers) call_args(n + 1) = 'ans'
    call_args(size(call_args)) = 'error'
    call put(o, '    select type (impl)')
    call put(o, '    class is (' // k%id // ')')
    call put_list(o, '      call impl%' // s%name // '(', call_args, ')')
    call put(o, '    end select')
    do m = 1, n
      a = 'a' // itoa(m)
      x = 'x' // itoa(m)
      associate (t => s%args(m))
        select case (t%type)
        case ('object')
          if (t%pass == 'lent') then
            call put(o, '    call guatiao_end_lent_object(' // a // '%r)')
          else
            call put(o, '    if (allocated(' // a // ')) deallocate(' // a // ')')
          end if
        case ('bytes', 'bytes_mut')
          call put(o, '    if (s' // itoa(m) // ') deallocate(' // a // ')')
        case ('map', 'list', 'value')
          if (t%pass == 'borrowed') then
            call put(o, '    call gt_end_borrow(' // a // ')')
          else if (t%pass == 'lent') then
            call put(o, '    call guatiao_end_lent_' // t%type // '(' // a // ', ' // x // ')')
          end if
        end select
      end associate
    end do
    call put(o, '    status = ' // fail)
    if (s%answers) then
      associate (t => s%answer)
        if (t%type == 'map' .or. t%type == 'list') then
          call put(o, '    call guatiao_settle_' // t%type // '(carrier, ans, out, st, failed=allocated(error))')
          call put(o, '    if (status == GUATIAO_OK) status = int(st, c_int32_t)')
        else if (t%type == 'value') then
          call put(o, '    if (status /= GUATIAO_OK) return')
          call put(o, '    call guatiao_settle_value(ans, out, st)')
          call put(o, '    status = int(st, c_int32_t)')
        else if (t%type == 'text') then
          call put(o, '    if (status /= GUATIAO_OK) return')
          call put(o, "    if (.not. allocated(ans)) ans = ''")
          call put(o, '    call guatiao_string_out(ans, out, st)')
          call put(o, '    status = int(st, c_int32_t)')
        else if (t%type == 'object') then
          call put(o, '    if (status /= GUATIAO_OK) return')
          call put(o, '    call c_f_pointer(out, oslot)')
          call put(o, '    if (.not. allocated(ans)) then')
          call put(o, "      call guatiao_fail(error, GUATIAO_ERR_NULL, '" // s%name // " answered no " // t%kind // "')")
          call put(o, '      status = ' // fail)
          call put(o, '      return')
          call put(o, '    end if')
          call put(o, '    select type (ans)')
          call put(o, '    type is (' // t%kind // '_remote)')
          call put(o, '      call ans%r%give(oslot, st)')
          call put(o, '      status = int(st, c_int32_t)')
          call put(o, '      return')
          call put(o, '    end select')
          call put(o, '    call move_alloc(ans, moved)')
          call put(o, '    call guatiao_serve_object(moved, ' // t%kind // '_kind(), oslot)')
        else
          call put(o, '    if (status == GUATIAO_OK .and. c_associated(out)) then')
          call put(o, '      call c_f_pointer(out, ap)')
          if (t%type == 'bool') then
            call put(o, '      ap = merge(GUATIAO_C_TRUE, GUATIAO_C_FALSE, ans)')
          else
            call put(o, '      ap = ans')
          end if
          call put(o, '    end if')
        end if
      end associate
    end if
    call put(o, '  end function')
  contains
    recursive subroutine head(how)
      integer, intent(in) :: how
      call put_list(o, '  recursive function ' // k%id // '_' // s%name // '_slot(', names_of(c_names(s, how)), &
                    ') bind(c) result(status)')
      call c_params(o, s, how, '    ')
    end subroutine
  end subroutine

  ! The body of the remote's method for slot `s`.
  recursive subroutine remote_body(o, k, s)
    type(gk_out), intent(inout) :: o
    type(gk_kind), intent(in) :: k
    type(gk_slot), intent(in) :: s
    character(:), allocatable :: x, j, d, tag
    integer :: m, n, cut
    logical :: by_address, split, lends
    n = size(s%args)
    by_address = takes_object(s)
    cut = split_at(s)
    split = cut > 0
    lends = .false.
    call put(o, '    procedure(' // k%id // '_' // s%name // '_c), pointer :: gk_f')
    if (s%fallible) call put(o, '    type(gt_provider_error), target :: gk_err')
    call put(o, '    integer(c_int32_t) :: gk_st')
    call put(o, '    integer :: gk_ist')
    do m = 1, n
      j = itoa(m)
      associate (t => s%args(m))
        select case (t%type)
        case ('str')
          call put(o, '    character(:), allocatable, target :: gk_t' // j)
          if (m == cut) call put(o, '    type(gt_str) :: gk_v' // j)
        case ('bytes', 'bytes_mut')
          call put(o, '    integer(int8), allocatable, target :: gk_b' // j // '(:)')
          call put(o, '    type(gt_bytes_mut) :: gk_v' // j)
        case ('object')
          call put(o, '    type(gt_object), target :: gk_o' // j)
          if (t%pass == 'taken') then
            call put(o, '    class(guatiao_impl), allocatable :: gk_m' // j)
            call put(o, '    logical :: gk_r' // j)
          else
            call put(o, '    logical :: gk_w' // j)
          end if
        case ('map', 'list', 'value')
          if (t%pass == 'borrowed') then
            call put(o, '    type(c_ptr) :: gk_p' // j)
          else if (t%pass == 'lent') then
            call put(o, '    type(c_ptr) :: gk_l' // j // ', gk_h' // j)
            call put(o, '    integer :: gk_s' // j)
            lends = .true.
          else
            call put(o, '    type(gt_value), target :: gk_n' // j)
          end if
        end select
      end associate
    end do
    if (s%answers) then
      associate (t => s%answer)
        if (t%type == 'text') then
          call put(o, '    type(gt_owned), target :: gk_out')
          call put(o, '    type(gt_value), target :: gk_node')
          call put(o, '    type(guatiao_value) :: gk_said')
        else if (is_scalar(t%type)) then
          call put(o, '    ' // c_scalar(t%type) // ', target :: gk_out')
        else if (t%type == 'object') then
          call put(o, '    type(gt_object), target :: gk_got')
          if (lends_tree(s)) &
            call put(o, '    type(' // t%kind // '_remote) :: gk_drop')
        else
          call put(o, '    type(guatiao_value) :: gk_got')
          call put(o, '    type(c_ptr) :: gk_out')
        end if
        d = answer_default(t)
        if (len(d) > 0) call put(o, '    ' // d)
      end associate
    end if
    call put(o, '    if (gk_lacks(self%r, ' // itoa(s%index) // ', error)) return')
    call put(o, '    call c_f_procpointer(self%r%slot(' // itoa(s%index) // '), gk_f)')
    ! What leaves everything as it was, if refused, first.
    do m = 1, n
      j = itoa(m)
      x = s%args(m)%name
      associate (t => s%args(m))
        select case (t%type)
        case ('str')
          call put(o, '    gk_t' // j // ' = ' // x)
          if (m == cut) call put(o, '    gk_v' // j // ' = guatiao_string_in(gk_t' // j // ')')
        case ('bytes', 'bytes_mut')
          call put(o, '    gk_b' // j // ' = ' // x)
          call put(o, '    gk_v' // j // '%len = size(gk_b' // j // ', kind=c_size_t)')
          call put(o, '    if (gk_v' // j // '%len > 0) gk_v' // j // '%ptr = c_loc(gk_b' // j // ')')
        case ('map', 'list', 'value')
          tag = tag_of(t%type)
          if (t%pass == 'borrowed') then
            call put(o, '    call guatiao_borrow_tree(' // x // ', ' // tag // ', gk_p' // j // ', gk_ist)')
            if (t%nullable) call put(o, '    if (gk_ist == GUATIAO_ERR_NULL) gk_ist = GUATIAO_OK')
            call put(o, '    if (gk_ist /= GUATIAO_OK) then')
            call put(o, "      call guatiao_fail(error, gk_ist, '" // s%name // " borrows a " // t%type // " as " // x // "')")
            call put(o, '      return')
            call put(o, '    end if')
          else if (t%pass == 'lent') then
            call put(o, '    call guatiao_lend_tree(' // x // ', ' // tag // ', gk_l' // j // ', gk_ist, gk_h' // j // ')')
            call put(o, '    if (gk_ist /= GUATIAO_OK) then')
            call put(o, "      call guatiao_fail(error, gk_ist, '" // s%name // " takes a writable " // t%type // &
                     " as " // x // "')")
            call put(o, '      return')
            call put(o, '    end if')
          end if
        end select
      end associate
    end do
    ! Then what would be refused before anything is handed over, so a
    ! refusal leaves every argument where it was.
    do m = 1, n
      x = s%args(m)%name
      associate (t => s%args(m))
        if ((t%type == 'map' .or. t%type == 'list') .and. t%pass == 'taken') then
          call put(o, '    if (' // x // '%tag() /= ' // tag_of(t%type) // ') then')
          call put(o, "      call guatiao_fail(error, merge(GUATIAO_ERR_NULL, GUATIAO_ERR_WRONG_KIND, " // x // &
                   "%is_absent()), '" // s%name // " takes a " // t%type // " as " // x // "')")
          call put(o, '      return')
          call put(o, '    end if')
        else if (t%type == 'object' .and. t%pass == 'taken') then
          call put(o, '    if (.not. allocated(' // x // ')) then')
          call put(o, "      call guatiao_fail(error, GUATIAO_ERR_NULL, 'no " // t%kind // " to hand over as " // x // "')")
          call put(o, '      return')
          call put(o, '    end if')
        end if
      end associate
    end do
    ! Then what is handed over.
    do m = 1, n
      j = itoa(m)
      x = s%args(m)%name
      associate (t => s%args(m))
        if ((t%type == 'map' .or. t%type == 'list') .and. t%pass == 'taken') then
          call put(o, '    call guatiao_hand_over(' // x // ', ' // tag_of(t%type) // ', gk_n' // j // ', gk_ist)')
          call put(o, '    if (gk_ist /= GUATIAO_OK) then')
          call put(o, "      call guatiao_fail(error, gk_ist, '" // s%name // " takes a " // t%type // " as " // x // "')")
          call put(o, '      return')
          call put(o, '    end if')
        else if (t%type == 'object' .and. t%pass == 'taken') then
          call put(o, '    gk_ist = GUATIAO_OK')
          call put(o, '    gk_r' // j // ' = .false.')
          call put(o, '    select type (' // x // ')')
          call put(o, '    type is (' // t%kind // '_remote)')
          call put(o, '      call ' // x // '%r%give(gk_o' // j // ', gk_ist)')
          call put(o, '      gk_r' // j // ' = .true.')
          call put(o, '    end select')
          call put(o, '    if (gk_r' // j // ') then')
          call put(o, '      deallocate(' // x // ')')
          call put(o, '    else')
          call put(o, '      call move_alloc(' // x // ', gk_m' // j // ')')
          call put(o, '      call guatiao_serve_object(gk_m' // j // ', ' // t%kind // '_kind(), gk_o' // j // ')')
          call put(o, '    end if')
          call put(o, '    if (gk_ist /= GUATIAO_OK) then')
          call put(o, "      call guatiao_fail(error, gk_ist, 'a " // t%kind // &
                   " this side does not own cannot be handed over')")
          call put(o, '      return')
          call put(o, '    end if')
        else if (t%type == 'object') then
          call put(o, '    gk_w' // j // ' = .false.')
          call put(o, '    select type (' // x // ')')
          call put(o, '    type is (' // t%kind // '_remote)')
          call put(o, '      gk_o' // j // '%table = ' // x // '%r%table')
          call put(o, '      gk_o' // j // '%size = ' // x // '%r%size')
          call put(o, '      gk_o' // j // '%ctx = ' // x // '%r%ctx')
          call put(o, '    class default')
          call put(o, '      call guatiao_lend_impl(' // x // ', ' // t%kind // '_kind(), gk_o' // j // ')')
          call put(o, '      gk_w' // j // ' = .true.')
          call put(o, '    end select')
        end if
      end associate
    end do
    if (s%answers) then
      associate (t => s%answer)
        if (t%type == 'map' .or. t%type == 'list' .or. t%type == 'value') then
          call put(o, '    call guatiao_' // t%type // '_out(gk_got, gk_out, self%r%alloc)')
        else if (is_scalar(t%type)) then
          if (t%type == 'bool') then
            call put(o, '    gk_out = GUATIAO_C_FALSE')
          else
            call put(o, '    gk_out = 0')
          end if
        end if
      end associate
    end if
    if (by_address) then
      call put(o, OBJ_BY_ADDRESS)
      call the_call(1)
    end if
    if (split) then
      if (by_address) then
        call put(o, SPLIT_VIEW // ' && !defined(_WIN32)')
      else
        call put(o, '#if defined(__flang__) && defined(__x86_64__) && !defined(_WIN32)')
      end if
      call the_call(2)
    end if
    if (by_address .or. split) call put(o, '#else')
    call the_call(0)
    if (by_address .or. split) call put(o, '#endif')
    do m = 1, n
      j = itoa(m)
      x = s%args(m)%name
      associate (t => s%args(m))
        if (t%pass == 'lent' .and. t%type /= 'object') then
          call put(o, '    call guatiao_settle_lent(' // x // ', ' // tag_of(t%type) // ', gk_h' // j // ', gk_s' // j // ')')
        else if (t%type == 'object' .and. t%pass == 'lent') then
          call put(o, '    if (gk_w' // j // ') call guatiao_destroy_served(gk_o' // j // '%ctx, close=.false.)')
        else if (t%type == 'bytes_mut') then
          call put(o, '    ' // x // ' = gk_b' // j)
        end if
      end associate
    end do
    if (s%fallible) then
      call put(o, '    call guatiao_call_error(gk_st, gk_err, error)')
    else
      call put(o, "    if (gk_st /= GUATIAO_OK) call guatiao_fail(error, int(gk_st), '" // s%name // " failed')")
    end if
    if (lends .or. s%answers) call put(o, '    if (gk_st /= GUATIAO_OK) return')
    do m = 1, n
      j = itoa(m)
      if (s%args(m)%pass == 'lent' .and. s%args(m)%type /= 'object') then
        call put(o, '    if (gk_s' // j // ' /= GUATIAO_OK) then')
        if (s%answers) then
          if (s%answer%type == 'text') then
            call put(o, '      gk_node%tag = GUATIAO_TAG_STRING')
            call put(o, '      gk_node%payload = gk_out')
            call put(o, '      gk_said = guatiao_value_from_raw(c_loc(gk_node))')
            call put(o, '      call gk_said%close()')
          else if (s%answer%type == 'object') then
            call put(o, '      call guatiao_take_object(gk_got, ' // s%answer%kind // '_kind(), gk_drop%r, gk_ist)')
            call put(o, '      call gk_drop%r%close()')
          end if
        end if
        call put(o, '      call guatiao_fail(error, gk_s' // j // ', guatiao_lent_refusal(gk_s' // j // &
                 ', "' // s%name // '", "' // s%args(m)%name // '"))')
        call put(o, '      return')
        call put(o, '    end if')
      end if
    end do
    if (s%answers) then
      associate (t => s%answer)
        if (t%type == 'text') then
          call put(o, '    gk_node%tag = GUATIAO_TAG_STRING')
          call put(o, '    gk_node%payload = gk_out')
          call put(o, '    gk_said = guatiao_value_from_raw(c_loc(gk_node))')
          call put(o, "    answer = gk_said%string_or('')")
        else if (t%type == 'bool') then
          call put(o, '    answer = gk_out /= GUATIAO_C_FALSE')
        else if (is_scalar(t%type)) then
          call put(o, '    answer = gk_out')
        else if (t%type == 'object') then
          call put(o, '    allocate(' // t%kind // '_remote :: answer)')
          call put(o, '    select type (answer)')
          call put(o, '    type is (' // t%kind // '_remote)')
          call put(o, '      call guatiao_take_object(gk_got, ' // t%kind // '_kind(), answer%r, gk_ist)')
          call put(o, '    end select')
          call put(o, "    if (gk_ist /= GUATIAO_OK) call guatiao_fail(error, gk_ist, 'the answer is not a " // &
                   t%kind // "')")
        else
          call put(o, '    call guatiao_settle_home(gk_got, gk_ist)')
          call put(o, '    if (gk_ist /= GUATIAO_OK) then')
          call put(o, "      call guatiao_fail(error, gk_ist, 'the answer of " // s%name // &
                   " could not be moved into the allocator of this side')")
          call put(o, '      return')
          call put(o, '    end if')
          call put(o, '    answer = gk_got')
        end if
      end associate
    end if
  contains
    recursive subroutine the_call(how)
      integer, intent(in) :: how
      character(len=200), allocatable :: actual(:)
      character(:), allocatable :: j2
      integer :: i2, at
      allocate(actual(2 * n + 3))
      actual(1) = 'self%r%ctx'
      at = 1
      do i2 = 1, n
        at = at + 1
        j2 = itoa(i2)
        associate (t => s%args(i2))
          select case (t%type)
          case ('str')
            if (how == 2 .and. i2 == cut) then
              actual(at) = 'gk_v' // j2 // '%ptr'
              at = at + 1
              actual(at) = 'gk_v' // j2 // '%len'
            else
              actual(at) = 'guatiao_string_in(gk_t' // j2 // ')'
            end if
          case ('bytes', 'bytes_mut')
            if (how == 2 .and. i2 == cut) then
              actual(at) = 'gk_v' // j2 // '%ptr'
              at = at + 1
              actual(at) = 'gk_v' // j2 // '%len'
            else
              actual(at) = 'gk_v' // j2
            end if
          case ('bool')
            actual(at) = 'merge(GUATIAO_C_TRUE, GUATIAO_C_FALSE, ' // t%name // ')'
          case ('object')
            if (t%pass == 'taken' .and. how /= 1) then
              actual(at) = 'gk_o' // j2
            else
              actual(at) = 'c_loc(gk_o' // j2 // ')'
            end if
          case ('map', 'list', 'value')
            if (t%pass == 'borrowed') then
              actual(at) = 'gk_p' // j2
            else if (t%pass == 'lent') then
              actual(at) = 'gk_l' // j2
            else
              actual(at) = 'c_loc(gk_n' // j2 // '%payload)'
            end if
          case default
            actual(at) = t%name
          end select
        end associate
      end do
      if (s%answers) then
        at = at + 1
        if (s%answer%type == 'map' .or. s%answer%type == 'list' .or. s%answer%type == 'value') then
          actual(at) = 'gk_out'
        else if (s%answer%type == 'object') then
          actual(at) = 'c_loc(gk_got)'
        else
          actual(at) = 'c_loc(gk_out)'
        end if
      end if
      if (s%fallible) then
        at = at + 1
        actual(at) = 'c_loc(gk_err)'
      end if
      call put_list(o, '    gk_st = gk_f(', actual(:at), ')')
    end subroutine
  end subroutine

end module guatiao_kind_writer
