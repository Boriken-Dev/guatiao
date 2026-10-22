! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> Values as JSON, TOML and YAML documents, through guatiao_serde.
!> TOML and YAML need a guatiao_serde built with them
!> (GUATIAO_STAT_MISSING_SYMBOL otherwise).
module guatiao_serde
  use, intrinsic :: iso_c_binding
  use guatiao_c
  use guatiao_serde_c
  use guatiao_loader, only: gt_dl_sym
  use guatiao_native
  use guatiao_errors
  use guatiao_values
  implicit none
  private

  public :: GUATIAO_FORMAT_JSON, GUATIAO_FORMAT_TOML, GUATIAO_FORMAT_YAML
  public :: GUATIAO_BYTES_DATA_URI, GUATIAO_BYTES_BASE64, GUATIAO_BYTES_ARRAY, GUATIAO_BYTES_REFUSE, &
            GUATIAO_READ_DATA_URIS, GUATIAO_PRETTY
  public :: guatiao_parse_error, guatiao_parse, guatiao_emit, guatiao_serde_use_library

  !> Where a document stopped parsing: its format, the line and column
  !> (from 1, the column in characters; 0 when the parser did not say)
  !> and the parser's reason, which never quotes the document.
  type :: guatiao_parse_error
    integer :: format = GUATIAO_FORMAT_JSON
    integer :: line = 0
    integer :: column = 0
    character(:), allocatable :: reason
  end type

contains

  !> Names the guatiao_serde library to use, before the first call that needs
  !> it: a file exporting its functions, whatever it is called (an
  !> application's own library carrying several surfaces, say), or a
  !> directory holding it. Without it the library is found through
  !> GUATIAO_SERDE_LIBRARY, then GUATIAO_LIBRARY (a directory, or a file
  !> exporting it, else the file beside it), then the platform's search.
  !> Naming another file once one is open is GUATIAO_ERR_BUSY.
  recursive subroutine guatiao_serde_use_library(path, stat, errmsg)
    character(*), intent(in) :: path
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    call gt_use(GT_SERDE, path, stat, errmsg)
  end subroutine

  ! Opens guatiao_serde and resolves its exports, once.
  recursive subroutine serde_need(stat, msg)
    integer, intent(out) :: stat
    character(:), allocatable, intent(out) :: msg
    logical, save :: resolved = .false.
    type(c_ptr) :: h
    integer :: i
    call gt_need(GT_SERDE, stat, msg)
    if (stat /= GUATIAO_OK .or. resolved) return
    h = gt_handle(GT_SERDE)
    do i = 1, size(gt_serde_exports)
      if (len_trim(gt_serde_exports(i)%feature) > 0) cycle
      if (.not. c_associated(gt_dl_sym(h, trim(gt_serde_exports(i)%name)))) then
        stat = GUATIAO_STAT_MISSING_SYMBOL
        msg = 'guatiao: ' // guatiao_library_path(GT_SERDE) // ' does not export ' // &
              trim(gt_serde_exports(i)%name)
        return
      end if
    end do
    call gt_resolve_serde(h)
    resolved = .true.
  end subroutine

  !> Reads `text` as a `format` document into `value`, built in `alloc`.
  !> `how` holds the GUATIAO_BYTES_* / GUATIAO_READ_DATA_URIS flags. A
  !> document that does not parse is GUATIAO_ERR_BAD_VALUE, and `where`
  !> says where.
  recursive subroutine guatiao_parse(text, format, value, how, alloc, where, stat, errmsg)
    character(*), intent(in), target :: text
    integer, intent(in) :: format
    type(guatiao_value), intent(inout) :: value
    integer, intent(in), optional :: how
    type(guatiao_allocator), intent(in), optional :: alloc
    type(guatiao_parse_error), intent(out), optional :: where
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_parse_error), target :: err
    type(guatiao_value) :: fresh
    type(c_ptr) :: root
    integer :: st
    integer(c_int32_t) :: flags, ignored
    character(:), allocatable :: msg, reason
    flags = 0
    if (present(how)) flags = int(how, c_int32_t)
    call serde_need(st, msg)
    if (st == GUATIAO_OK) call usable(format, st, msg)
    if (st == GUATIAO_OK) then
      call gt_new_owned(fresh, gt_alloc_of(alloc), root)
      select case (format)
      case (GUATIAO_FORMAT_TOML)
        st = gt_toml_parse_ex(gt_view(text), flags, gt_alloc_of(alloc), root, c_loc(err))
      case (GUATIAO_FORMAT_YAML)
        st = gt_yaml_parse_ex(gt_view(text), flags, gt_alloc_of(alloc), root, c_loc(err))
      case default
        st = gt_json_parse_ex(gt_view(text), flags, gt_alloc_of(alloc), root, c_loc(err))
      end select
      if (err%message%tag == GUATIAO_TAG_STRING) then
        call gt_text_of_owned(err%message%payload, reason)
      else
        reason = ''
      end if
      ignored = gt_value_free(c_loc(err%message))
      if (present(where)) then
        where%format = err%format
        where%line = err%line
        where%column = err%column
        where%reason = reason
      end if
      if (st == GUATIAO_OK) then
        value = fresh
      else
        call fresh%close()
        msg = 'guatiao: the document does not parse at line ' // itoa(int(err%line)) // ', column ' // itoa(int(err%column))
        if (len(reason) > 0) msg = msg // ': ' // reason
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> Writes what `value` names as a `format` document into `text`;
  !> `pretty` indents JSON (the other formats are already written for a
  !> reader). A TOML document is a table: anything else is
  !> GUATIAO_ERR_WRONG_KIND.
  recursive subroutine guatiao_emit(value, format, text, pretty, how, stat, errmsg)
    type(guatiao_value), intent(in) :: value
    integer, intent(in) :: format
    character(:), allocatable, intent(inout) :: text
    logical, intent(in), optional :: pretty
    integer, intent(in), optional :: how
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_value), target :: out
    type(c_ptr) :: node
    integer :: st
    integer(c_int32_t) :: flags, ignored
    character(:), allocatable :: msg
    flags = 0
    if (present(how)) flags = int(how, c_int32_t)
    if (present(pretty)) then
      if (pretty) flags = ior(flags, GUATIAO_PRETTY)
    end if
    call serde_need(st, msg)
    if (st == GUATIAO_OK) call usable(format, st, msg)
    if (st == GUATIAO_OK) then
      node = gt_node(value)
      if (value%is_closed()) then
        st = GUATIAO_STAT_CLOSED
        msg = 'guatiao: the tree is closed'
      else if (.not. c_associated(node)) then
        st = GUATIAO_ERR_NOT_FOUND
        msg = 'guatiao: nothing to write'
      else
        select case (format)
        case (GUATIAO_FORMAT_TOML)
          st = gt_toml_emit(node, flags, gt_default_alloc(), c_loc(out))
        case (GUATIAO_FORMAT_YAML)
          st = gt_yaml_emit(node, flags, gt_default_alloc(), c_loc(out))
        case default
          st = gt_json_emit(node, flags, gt_default_alloc(), c_loc(out))
        end select
        if (st == GUATIAO_OK) then
          call gt_text_of_owned(out%payload, text)
        else
          msg = 'guatiao: the value has no such document'
          if (st == GUATIAO_ERR_WRONG_KIND) msg = 'guatiao: a TOML document is a map'
        end if
        ignored = gt_value_free(c_loc(out))
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  recursive subroutine usable(format, st, msg)
    integer, intent(in) :: format
    integer, intent(out) :: st
    character(:), allocatable, intent(out) :: msg
    select case (format)
    case (GUATIAO_FORMAT_JSON)
      st = GUATIAO_OK
      msg = ''
    case (GUATIAO_FORMAT_TOML)
      call gt_need_symbol(associated(gt_toml_parse_ex) .and. associated(gt_toml_emit), &
                          'guatiao_toml_parse_ex', 'toml', st, msg)
    case (GUATIAO_FORMAT_YAML)
      call gt_need_symbol(associated(gt_yaml_parse_ex) .and. associated(gt_yaml_emit), &
                          'guatiao_yaml_parse_ex', 'yaml', st, msg)
    case default
      st = GUATIAO_ERR_BAD_VALUE
      msg = 'guatiao: no such format'
    end select
  end subroutine

  recursive function itoa(n) result(s)
    integer, intent(in) :: n
    character(:), allocatable :: s
    character(16) :: buf
    write (buf, '(i0)') n
    s = trim(buf)
  end function

end module guatiao_serde
