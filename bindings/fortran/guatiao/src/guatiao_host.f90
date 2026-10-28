! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> A host's registry of libraries: loading, scanning, sources, ranking,
!> instances, retiring and unloading. Every answer that is data is a
!> `guatiao_value` the caller owns, built in the registry's allocator.
module guatiao_host
  use, intrinsic :: iso_c_binding
  use guatiao_c
  use guatiao_native
  use guatiao_errors
  use guatiao_values
  use guatiao_holds
  use guatiao_loader, only: gt_windows
  implicit none
  private

  public :: guatiao_registry, guatiao_instance, guatiao_open_registry, gt_registry_ptr, gt_registry_hold, &
            gt_instance_ctx, gt_instance_key, gt_instance_reg

  !> A registry. Copies share it; `close()` frees it once nothing else
  !> (an instance) holds it, and every call then answers
  !> GUATIAO_STAT_CLOSED. Not safe to use from two threads at once.
  type :: guatiao_registry
    type(gt_hold) :: hold
    type(c_ptr) :: alloc = c_null_ptr
  contains
    procedure :: close => reg_close
    procedure :: is_closed => reg_is_closed
    procedure :: host => reg_host
    procedure :: load_file => reg_load_file
    procedure :: register_entry => reg_register_entry
    procedure :: scan_dir => reg_scan_dir
    procedure :: scan_path => reg_scan_path
    procedure :: add_source_file => reg_add_source_file
    procedure :: watch_dir => reg_watch_dir
    procedure :: refresh => reg_refresh
    procedure :: libraries => reg_libraries
    procedure :: providers => reg_providers
    procedure :: provider => reg_provider
    procedure :: available => reg_available
    procedure :: why_not => reg_why_not
    procedure :: best => reg_best
    procedure :: provider_available => reg_provider_available
    procedure :: keyed_by => reg_keyed_by
    procedure :: libraries_keyed_by => reg_libraries_keyed_by
    procedure :: set_priority => reg_set_priority
    procedure :: priority => reg_priority
    procedure :: set_import_search => reg_set_import_search
    procedure :: import_search => reg_import_search
    procedure :: provider_table => reg_provider_table
    procedure :: provider_ctx => reg_provider_ctx
    procedure :: provider_config => reg_provider_config
    procedure :: provider_meta => reg_provider_meta
    procedure :: create => reg_create
    procedure :: retire => reg_retire
    procedure :: unload => reg_unload
    procedure :: unload_unchecked => reg_unload_unchecked
    procedure :: collect => reg_collect
  end type

  !> An instance a provider built from a configuration. `close()` (or the
  !> last copy going) reaches the provider that built it, even after a
  !> retire or a refresh loaded a new build under its key.
  type :: guatiao_instance
    type(gt_hold) :: hold
  contains
    procedure :: close => inst_close
    procedure :: is_closed => inst_is_closed
  end type

contains

  !> A registry introducing its host as `id`/`version`; its answers, and
  !> the allocator it offers libraries, are `alloc` (else guatiao's heap).
  recursive function guatiao_open_registry(id, version, alloc, stat, errmsg) result(reg)
    character(*), intent(in), target :: id, version
    type(guatiao_allocator), intent(in), optional :: alloc
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_registry) :: reg
    type(c_ptr) :: p, a
    integer :: st
    character(:), allocatable :: msg
    call gt_need(GT_CORE, st, msg)
    if (st == GUATIAO_OK) call gt_need_symbol(associated(gt_registry_new), 'guatiao_registry_new', 'load', st, msg)
    if (st == GUATIAO_OK) then
      a = gt_alloc_of(alloc)
      p = gt_registry_new(gt_view(id), gt_view(version), a)
      if (c_associated(p)) then
        reg%hold = gt_new_hold(GT_HOLD_REGISTRY, reg=p)
        reg%alloc = a
      else
        st = GUATIAO_ERR_BAD_VALUE
        msg = 'guatiao: a registry needs UTF-8 text and a complete allocator'
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end function

  !> The registry's C pointer, or null once closed.
  recursive function gt_registry_ptr(reg) result(p)
    type(guatiao_registry), intent(in) :: reg
    type(c_ptr) :: p
    p = c_null_ptr
    if (reg%hold%is_closed()) return
    p = reg%hold%box%reg
  end function

  recursive function gt_registry_hold(reg) result(h)
    type(guatiao_registry), intent(in) :: reg
    type(gt_hold) :: h
    h = reg%hold
  end function

  ! The registry pointer, or a status saying it is closed.
  recursive subroutine open_reg(self, p, st, msg)
    class(guatiao_registry), intent(in) :: self
    type(c_ptr), intent(out) :: p
    integer, intent(out) :: st
    character(:), allocatable, intent(out) :: msg
    p = gt_registry_ptr(self)
    st = GUATIAO_OK
    msg = ''
    if (.not. c_associated(p)) then
      st = GUATIAO_STAT_CLOSED
      msg = 'guatiao: the registry is closed'
    end if
  end subroutine

  ! A fresh answer node in the registry's allocator.
  recursive subroutine fresh(self, answer, out)
    class(guatiao_registry), intent(in) :: self
    type(guatiao_value), intent(inout) :: answer
    type(c_ptr), intent(out) :: out
    call gt_new_owned(answer, self%alloc, out)
  end subroutine

  ! The rules, one per line, as the C surface takes them.
  recursive subroutine joined(rules, text)
    character(*), intent(in), optional :: rules(:)
    character(:), allocatable, intent(out) :: text
    integer :: i
    text = ''
    if (.not. present(rules)) return
    do i = 1, size(rules)
      if (i > 1) text = text // new_line('a')
      text = text // trim(rules(i))
    end do
  end subroutine

  recursive subroutine reg_close(self)
    class(guatiao_registry), intent(inout) :: self
    call self%hold%close()
  end subroutine

  recursive logical function reg_is_closed(self)
    class(guatiao_registry), intent(in) :: self
    reg_is_closed = self%hold%is_closed()
  end function

  !> The `guatiao_host_info *` this registry introduces itself with, for
  !> a C function taking one.
  recursive function reg_host(self) result(p)
    class(guatiao_registry), intent(in) :: self
    type(c_ptr) :: p
    type(c_ptr) :: r
    p = c_null_ptr
    r = gt_registry_ptr(self)
    if (c_associated(r)) p = gt_registry_host(r)
  end function

  !> Maps one library file and registers its providers; `answer` says
  !> what happened (`{loaded: {...}}` or `{skipped: ...}`).
  recursive subroutine reg_load_file(self, path, answer, stat, errmsg)
    class(guatiao_registry), intent(in) :: self
    character(*), intent(in), target :: path
    type(guatiao_value), intent(inout), optional :: answer
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: p, out
    type(guatiao_value) :: got
    integer :: st
    character(:), allocatable :: msg
    call open_reg(self, p, st, msg)
    if (st == GUATIAO_OK) then
      call fresh(self, got, out)
      st = gt_registry_load_file(p, gt_view(path), self%alloc, out)
      if (st /= GUATIAO_OK) msg = 'guatiao: ' // path // ' did not load: ' // said(got)
      if (present(answer)) answer = got
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> Registers the library an entry function (a `guatiao_library_entry`,
  !> as `c_funloc` gives it) answers, under `name`.
  recursive subroutine reg_register_entry(self, name, entry, answer, stat, errmsg)
    class(guatiao_registry), intent(in) :: self
    character(*), intent(in), target :: name
    type(c_funptr), intent(in) :: entry
    type(guatiao_value), intent(inout), optional :: answer
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: p, out
    type(guatiao_value) :: got
    integer :: st
    character(:), allocatable :: msg
    call open_reg(self, p, st, msg)
    if (st == GUATIAO_OK) then
      call fresh(self, got, out)
      st = gt_registry_register_entry(p, gt_view(name), entry, self%alloc, out)
      if (st /= GUATIAO_OK) msg = 'guatiao: ' // name // ' was refused: ' // said(got)
      if (present(answer)) answer = got
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> Loads every library in `dir` the rules admit (`kind=greeter`,
  !> `!key=value`, matched against what a library declares, before it is
  !> mapped). `answer`: `{loaded, skipped, failed, unreadable}`.
  recursive subroutine reg_scan_dir(self, dir, rules, descending, answer, stat, errmsg)
    class(guatiao_registry), intent(in) :: self
    character(*), intent(in), target :: dir
    character(*), intent(in), optional :: rules(:)
    logical, intent(in), optional :: descending
    type(guatiao_value), intent(inout), optional :: answer
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: p, out
    type(guatiao_value) :: got
    character(:), allocatable, target :: text
    type(gt_str) :: rules_view
    integer(c_int8_t) :: down
    integer :: st
    character(:), allocatable :: msg
    down = GUATIAO_C_FALSE
    if (present(descending)) down = merge(GUATIAO_C_TRUE, GUATIAO_C_FALSE, descending)
    call joined(rules, text)
    call open_reg(self, p, st, msg)
    if (st == GUATIAO_OK) then
      call fresh(self, got, out)
      if (gt_windows) then
        st = gt_registry_scan_dir_rules(p, gt_view(dir), down, gt_view(text), self%alloc, out)
      else
        rules_view = gt_view(text)
        st = gt_registry_scan_dir_rules_split(p, gt_view(dir), down, rules_view%ptr, rules_view%len, &
                                              self%alloc, out)
      end if
      if (st /= GUATIAO_OK) msg = 'guatiao: the scan of ' // dir // ' failed: ' // said(got)
      if (present(answer)) answer = got
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> As scan_dir over a search path (the platform's separator between
  !> entries; a file entry is loaded as it is).
  recursive subroutine reg_scan_path(self, spec, rules, descending, answer, stat, errmsg)
    class(guatiao_registry), intent(in) :: self
    character(*), intent(in), target :: spec
    character(*), intent(in), optional :: rules(:)
    logical, intent(in), optional :: descending
    type(guatiao_value), intent(inout), optional :: answer
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: p, out
    type(guatiao_value) :: got
    character(:), allocatable, target :: text
    type(gt_str) :: rules_view
    integer(c_int8_t) :: down
    integer :: st
    character(:), allocatable :: msg
    down = GUATIAO_C_FALSE
    if (present(descending)) down = merge(GUATIAO_C_TRUE, GUATIAO_C_FALSE, descending)
    call joined(rules, text)
    call open_reg(self, p, st, msg)
    if (st == GUATIAO_OK) then
      call fresh(self, got, out)
      if (gt_windows) then
        st = gt_registry_scan_path(p, gt_view(spec), down, gt_view(text), self%alloc, out)
      else
        rules_view = gt_view(text)
        st = gt_registry_scan_path_split(p, gt_view(spec), down, rules_view%ptr, rules_view%len, &
                                         self%alloc, out)
      end if
      if (st /= GUATIAO_OK) msg = 'guatiao: the scan failed: ' // said(got)
      if (present(answer)) answer = got
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> Maps a file exporting `guatiao_source` (a bridge to another
  !> language) and adds the source it answers, configured by the map
  !> `config`; refused with the source's own words.
  recursive subroutine reg_add_source_file(self, path, config, stat, errmsg)
    class(guatiao_registry), intent(in) :: self
    character(*), intent(in), target :: path
    type(guatiao_value), intent(in) :: config
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: p, node
    type(gt_provider_error), target :: err
    integer :: st
    character(:), allocatable :: msg, words
    call open_reg(self, p, st, msg)
    if (st == GUATIAO_OK) then
      node = gt_node(config)
      if (.not. c_associated(node)) then
        st = GUATIAO_ERR_NULL
        msg = 'guatiao: a source is configured by a map'
      else
        st = gt_registry_add_source_file(p, gt_view(path), node, c_loc(err))
        call gt_text_of_owned(err%message, words)
        call free_message(err)
        if (st /= GUATIAO_OK) msg = 'guatiao: the source ' // path // ' was refused: ' // words
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> Watches a directory as a source of library files the rules admit.
  recursive subroutine reg_watch_dir(self, dir, rules, stat, errmsg)
    class(guatiao_registry), intent(in) :: self
    character(*), intent(in), target :: dir
    character(*), intent(in), optional :: rules(:)
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: p
    character(:), allocatable, target :: text
    integer :: st
    character(:), allocatable :: msg
    call joined(rules, text)
    call open_reg(self, p, st, msg)
    if (st == GUATIAO_OK) then
      st = gt_registry_watch_dir(p, gt_view(dir), gt_view(text))
      if (st /= GUATIAO_OK) msg = 'guatiao: ' // dir // ' cannot be watched'
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> Makes the registry agree with every source: `{loaded, retired,
  !> skipped, refused}`. Nothing refreshes on its own.
  recursive subroutine reg_refresh(self, answer, stat, errmsg)
    class(guatiao_registry), intent(in) :: self
    type(guatiao_value), intent(inout), optional :: answer
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: p, out
    type(guatiao_value) :: got
    integer :: st
    character(:), allocatable :: msg
    call open_reg(self, p, st, msg)
    if (st == GUATIAO_OK) then
      call fresh(self, got, out)
      st = gt_registry_refresh(p, self%alloc, out)
      if (st /= GUATIAO_OK) msg = 'guatiao: the refresh failed'
      if (present(answer)) answer = got
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> Every library: a list of maps (`key`, `id`, `version`, `path`, ...).
  recursive subroutine reg_libraries(self, answer, stat, errmsg)
    class(guatiao_registry), intent(in) :: self
    type(guatiao_value), intent(inout) :: answer
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: p, out
    integer :: st
    character(:), allocatable :: msg
    call open_reg(self, p, st, msg)
    if (st == GUATIAO_OK) then
      call fresh(self, answer, out)
      st = gt_registry_libraries(p, self%alloc, out)
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  ! The calls shaped `(reg, text, alloc, out)`.
  recursive subroutine by_text(self, which, text, answer, st, msg)
    class(guatiao_registry), intent(in) :: self
    integer, intent(in) :: which
    character(*), intent(in), target :: text
    type(guatiao_value), intent(inout) :: answer
    integer, intent(out) :: st
    character(:), allocatable, intent(out) :: msg
    type(c_ptr) :: p, out
    call open_reg(self, p, st, msg)
    if (st /= GUATIAO_OK) return
    call fresh(self, answer, out)
    select case (which)
    case (1)
      st = gt_registry_providers(p, gt_view(text), self%alloc, out)
    case (2)
      st = gt_registry_provider(p, gt_view(text), self%alloc, out)
      if (st /= GUATIAO_OK) msg = 'guatiao: no provider "' // text // '"'
    case (3)
      st = gt_registry_available(p, gt_view(text), self%alloc, out)
    case (4)
      st = gt_registry_why_not(p, gt_view(text), self%alloc, out)
    case (5)
      st = gt_registry_best(p, gt_view(text), self%alloc, out)
      if (st /= GUATIAO_OK) msg = 'guatiao: no provider of "' // text // '" can run here'
    case (6)
      st = gt_registry_provider_meta(p, gt_view(text), self%alloc, out)
      if (st == GUATIAO_ERR_NOT_FOUND) msg = 'guatiao: no provider "' // text // '"'
    end select
  end subroutine

  !> Every provider serving `kind` ('' for all), best first: a list of
  !> maps (`key`, `id`, `version`, `library`, `display_name`, `from`,
  !> `kinds`, `has_config`, `vtable_size`).
  recursive subroutine reg_providers(self, kind, answer, stat, errmsg)
    class(guatiao_registry), intent(in) :: self
    character(*), intent(in) :: kind
    type(guatiao_value), intent(inout) :: answer
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    integer :: st
    character(:), allocatable :: msg
    call by_text(self, 1, kind, answer, st, msg)
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> The provider filed under `key` (GUATIAO_ERR_NOT_FOUND for none).
  recursive subroutine reg_provider(self, key, answer, stat, errmsg)
    class(guatiao_registry), intent(in) :: self
    character(*), intent(in) :: key
    type(guatiao_value), intent(inout) :: answer
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    integer :: st
    character(:), allocatable :: msg
    call by_text(self, 2, key, answer, st, msg)
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> The providers of `kind` that can run here, best first.
  recursive subroutine reg_available(self, kind, answer, stat, errmsg)
    class(guatiao_registry), intent(in) :: self
    character(*), intent(in) :: kind
    type(guatiao_value), intent(inout) :: answer
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    integer :: st
    character(:), allocatable :: msg
    call by_text(self, 3, kind, answer, st, msg)
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> Why nothing serves `kind` here: `{available, why, ...}`.
  recursive subroutine reg_why_not(self, kind, answer, stat, errmsg)
    class(guatiao_registry), intent(in) :: self
    character(*), intent(in) :: kind
    type(guatiao_value), intent(inout) :: answer
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    integer :: st
    character(:), allocatable :: msg
    call by_text(self, 4, kind, answer, st, msg)
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> The best provider of `kind` that can run here.
  recursive subroutine reg_best(self, kind, answer, stat, errmsg)
    class(guatiao_registry), intent(in) :: self
    character(*), intent(in) :: kind
    type(guatiao_value), intent(inout) :: answer
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    integer :: st
    character(:), allocatable :: msg
    call by_text(self, 5, kind, answer, st, msg)
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> Whether the provider under `key` can run here, and if not, why.
  recursive subroutine reg_provider_available(self, key, ok, reason, stat, errmsg)
    class(guatiao_registry), intent(in) :: self
    character(*), intent(in), target :: key
    logical, intent(inout) :: ok
    character(:), allocatable, intent(inout), optional :: reason
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: p
    type(gt_str), target :: why
    integer :: st
    character(:), allocatable :: msg
    call open_reg(self, p, st, msg)
    if (st == GUATIAO_OK) then
      ok = gt_registry_provider_available(p, gt_view(key), c_loc(why)) /= GUATIAO_C_FALSE
      if (present(reason)) then
        if (ok) then
          reason = ''
        else
          call gt_copy_text(why%ptr, why%len, reason)
        end if
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> The template providers are filed under (`%id`, `%id@%version`, ...).
  recursive subroutine reg_keyed_by(self, template, stat, errmsg)
    class(guatiao_registry), intent(in) :: self
    character(*), intent(in), target :: template
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: p
    integer :: st
    character(:), allocatable :: msg
    call open_reg(self, p, st, msg)
    if (st == GUATIAO_OK) then
      st = gt_registry_keyed_by(p, gt_view(template))
      if (st /= GUATIAO_OK) msg = 'guatiao: not a key template: ' // template
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> The template libraries are filed under.
  recursive subroutine reg_libraries_keyed_by(self, template, stat, errmsg)
    class(guatiao_registry), intent(in) :: self
    character(*), intent(in), target :: template
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: p
    integer :: st
    character(:), allocatable :: msg
    call open_reg(self, p, st, msg)
    if (st == GUATIAO_OK) then
      st = gt_registry_libraries_keyed_by(p, gt_view(template))
      if (st /= GUATIAO_OK) msg = 'guatiao: not a key template: ' // template
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> Ranks the provider with id `id` (higher first).
  recursive subroutine reg_set_priority(self, id, priority, stat, errmsg)
    class(guatiao_registry), intent(in) :: self
    character(*), intent(in), target :: id
    integer, intent(in) :: priority
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: p
    integer :: st
    character(:), allocatable :: msg
    call open_reg(self, p, st, msg)
    if (st == GUATIAO_OK) then
      st = gt_registry_set_priority(p, gt_view(id), int(priority, c_int32_t))
      if (st /= GUATIAO_OK) msg = 'guatiao: no provider "' // id // '" to rank'
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  recursive integer function reg_priority(self, id)
    class(guatiao_registry), intent(in) :: self
    character(*), intent(in), target :: id
    type(c_ptr) :: p
    reg_priority = 0
    p = gt_registry_ptr(self)
    if (c_associated(p)) reg_priority = gt_registry_priority(p, gt_view(id))
  end function

  !> Where Windows finds what the files the registry opens from now on
  !> import: GUATIAO_IMPORT_SEARCH_LIBRARY (the default: beside the file,
  !> beside the host's executable, the system directory) or
  !> GUATIAO_IMPORT_SEARCH_PATH (also the current directory and PATH, as a
  !> host adding a Python bridge needs). Unix ignores it.
  recursive subroutine reg_set_import_search(self, search, stat, errmsg)
    class(guatiao_registry), intent(in) :: self
    integer, intent(in) :: search
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: p
    integer :: st
    character(:), allocatable :: msg
    call open_reg(self, p, st, msg)
    if (st == GUATIAO_OK) then
      st = gt_registry_set_import_search(p, int(search, c_int32_t))
      if (st /= GUATIAO_OK) msg = 'guatiao: no such import search'
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> GUATIAO_IMPORT_SEARCH_LIBRARY or _PATH; the first for a closed registry.
  recursive integer function reg_import_search(self)
    class(guatiao_registry), intent(in) :: self
    type(c_ptr) :: p
    reg_import_search = GUATIAO_IMPORT_SEARCH_LIBRARY
    p = gt_registry_ptr(self)
    if (c_associated(p)) reg_import_search = gt_registry_import_search(p)
  end function

  !> The table the provider under `key` offers for `kind` ('' for its
  !> single table) and its size; GUATIAO_ERR_NOT_FOUND for none.
  recursive subroutine reg_provider_table(self, key, kind, table, size, stat, errmsg)
    class(guatiao_registry), intent(in) :: self
    character(*), intent(in), target :: key, kind
    type(c_ptr), intent(out) :: table
    integer(c_size_t), intent(out) :: size
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: p
    integer(c_size_t), target :: n
    integer :: st
    character(:), allocatable :: msg
    table = c_null_ptr
    size = 0
    call open_reg(self, p, st, msg)
    if (st == GUATIAO_OK) then
      n = 0
      if (len(kind) == 0) then
        table = gt_registry_provider_vtable(p, gt_view(key), c_loc(n))
      else
        table = gt_registry_provider_table(p, gt_view(key), gt_view(kind), c_loc(n))
      end if
      size = n
      if (.not. c_associated(table)) then
        st = GUATIAO_ERR_NOT_FOUND
        msg = 'guatiao: "' // key // '" offers no ' // kind // ' table'
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> The `ctx` the provider under `key` passes its table's slots, null for
  !> one that builds instances only.
  recursive function reg_provider_ctx(self, key) result(ctx)
    class(guatiao_registry), intent(in) :: self
    character(*), intent(in), target :: key
    type(c_ptr) :: ctx
    type(c_ptr) :: p
    ctx = c_null_ptr
    p = gt_registry_ptr(self)
    if (c_associated(p)) ctx = gt_registry_provider_ctx(p, gt_view(key))
  end function

  !> The schema of the provider's configuration: a read-only handle,
  !> absent for a provider that takes none.
  recursive function reg_provider_config(self, key) result(schema)
    class(guatiao_registry), intent(in) :: self
    character(*), intent(in), target :: key
    type(guatiao_value) :: schema
    type(c_ptr) :: p, node
    p = gt_registry_ptr(self)
    node = c_null_ptr
    if (c_associated(p)) node = gt_registry_provider_config(p, gt_view(key))
    if (c_associated(node)) then
      schema = gt_borrowed(node, .true., self%alloc)
    else
      schema = guatiao_absent()
    end if
  end function

  !> What the provider under `key` declared beyond its schema (its legal
  !> notices, or its form under GUATIAO_INTAKE_PROVIDER_FORM_KEY): a copy the
  !> caller owns, outliving the library; absent when it declares nothing.
  !> GUATIAO_ERR_NOT_FOUND for an unknown key.
  recursive subroutine reg_provider_meta(self, key, answer, stat, errmsg)
    class(guatiao_registry), intent(in) :: self
    character(*), intent(in) :: key
    type(guatiao_value), intent(inout) :: answer
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    integer :: st
    character(:), allocatable :: msg
    call by_text(self, 6, key, answer, st, msg)
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> Builds an instance of the provider under `key` from `config`, which
  !> is checked against the provider's schema first.
  recursive subroutine reg_create(self, key, config, instance, stat, errmsg)
    class(guatiao_registry), intent(in) :: self
    character(*), intent(in), target :: key
    type(guatiao_value), intent(in) :: config
    type(guatiao_instance), intent(inout) :: instance
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: p, node
    type(c_ptr), target :: made
    type(gt_provider_error), target :: err
    integer :: st
    character(:), allocatable :: msg, words
    call open_reg(self, p, st, msg)
    if (st == GUATIAO_OK) then
      node = gt_node(config)
      made = c_null_ptr
      st = gt_registry_provider_create(p, gt_view(key), node, c_loc(made), c_loc(err))
      call gt_text_of_owned(err%message, words)
      call free_message(err)
      if (st == GUATIAO_OK) then
        instance%hold = gt_new_hold(GT_HOLD_INSTANCE, reg=p, ctx=made, key=key)
        call gt_hold_parent(instance%hold, self%hold)
      else
        msg = 'guatiao: "' // key // '" built no instance'
        if (len(words) > 0) msg = msg // ': ' // words
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> Takes the library under `key` out; it stays mapped until nothing
  !> uses it.
  recursive subroutine reg_retire(self, key, stat, errmsg)
    class(guatiao_registry), intent(in) :: self
    character(*), intent(in), target :: key
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: p
    integer :: st
    character(:), allocatable :: msg
    call open_reg(self, p, st, msg)
    if (st == GUATIAO_OK) then
      st = gt_registry_retire(p, gt_view(key))
      if (st /= GUATIAO_OK) msg = 'guatiao: no library "' // key // '" to retire'
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> Unmaps the library under `key` when it agrees: GUATIAO_ERR_BUSY while
  !> something it handed out lives, GUATIAO_ERR_UNSUPPORTED with no say in
  !> it, GUATIAO_ERR_WRONG_KIND for one registered rather than mapped.
  recursive subroutine reg_unload(self, key, stat, errmsg)
    class(guatiao_registry), intent(in) :: self
    character(*), intent(in), target :: key
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: p
    integer :: st
    character(:), allocatable :: msg
    call open_reg(self, p, st, msg)
    if (st == GUATIAO_OK) then
      st = gt_registry_unload(p, gt_view(key))
      if (st /= GUATIAO_OK) msg = 'guatiao: "' // key // '" was not unloaded'
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> Unmaps whatever the library says. Nothing taken from it is used again.
  recursive subroutine reg_unload_unchecked(self, key, stat, errmsg)
    class(guatiao_registry), intent(in) :: self
    character(*), intent(in), target :: key
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: p
    integer :: st
    character(:), allocatable :: msg
    call open_reg(self, p, st, msg)
    if (st == GUATIAO_OK) then
      st = gt_registry_unload_unchecked(p, gt_view(key))
      if (st /= GUATIAO_OK) msg = 'guatiao: "' // key // '" was not unloaded'
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> Closes every retired library whose own unload agrees; `answer` lists
  !> their keys.
  recursive subroutine reg_collect(self, answer, stat, errmsg)
    class(guatiao_registry), intent(in) :: self
    type(guatiao_value), intent(inout), optional :: answer
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: p, out
    type(guatiao_value) :: got
    integer :: st
    character(:), allocatable :: msg
    call open_reg(self, p, st, msg)
    if (st == GUATIAO_OK) then
      call fresh(self, got, out)
      st = gt_registry_collect(p, self%alloc, out)
      if (present(answer)) answer = got
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  ! ---- instances -------------------------------------------------------

  !> Releases the instance through the provider that built it, now.
  recursive subroutine inst_close(self)
    class(guatiao_instance), intent(inout) :: self
    call self%hold%close()
  end subroutine

  recursive logical function inst_is_closed(self)
    class(guatiao_instance), intent(in) :: self
    inst_is_closed = self%hold%is_closed()
  end function

  recursive function gt_instance_ctx(inst) result(ctx)
    type(guatiao_instance), intent(in) :: inst
    type(c_ptr) :: ctx
    ctx = c_null_ptr
    if (inst%hold%is_closed()) return
    ctx = inst%hold%box%ctx
  end function

  recursive function gt_instance_reg(inst) result(reg)
    type(guatiao_instance), intent(in) :: inst
    type(c_ptr) :: reg
    reg = c_null_ptr
    if (inst%hold%is_closed()) return
    reg = inst%hold%box%reg
  end function

  recursive function gt_instance_key(inst) result(key)
    type(guatiao_instance), intent(in) :: inst
    character(:), allocatable :: key
    key = ''
    if (inst%hold%is_closed()) return
    key = inst%hold%box%key
  end function

  ! ---- helpers ---------------------------------------------------------

  ! A refusal's words in an answer map (`error`, `reason`, `why`), or ''.
  recursive function said(v) result(words)
    type(guatiao_value), intent(in) :: v
    character(:), allocatable :: words
    type(guatiao_value) :: w
    words = ''
    w = v%key('error')
    if (w%tag() == GUATIAO_TAG_STRING) words = w%string_or('')
  end function

  ! Frees a provider error's message through the allocator it records.
  recursive subroutine free_message(err)
    type(gt_provider_error), intent(inout), target :: err
    type(gt_value), target :: node
    integer(c_int32_t) :: ignored
    if (err%message%cap == 0) return
    node%tag = GUATIAO_TAG_STRING
    node%payload = err%message
    ignored = gt_value_free(c_loc(node))
    err%message = gt_owned()
  end subroutine

end module guatiao_host
