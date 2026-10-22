! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> Libraries written in Fortran: providers declared with the objects
!> serving their kinds (one instance), or with a configuration type and a
!> build procedure (an instance per configuration), and the descriptor a
!> host reads, built on first use and kept for the process.
module guatiao_provide
  use, intrinsic :: iso_c_binding
  use, intrinsic :: iso_fortran_env, only: error_unit
  use guatiao_c
  use guatiao_native
  use guatiao_errors
  use guatiao_values
  use guatiao_schema
  use guatiao_host
  use guatiao_kinds
  use guatiao_pools
  implicit none
  private

  public :: guatiao_library, guatiao_new_library, guatiao_register_library, guatiao_build_i

  abstract interface
    !> Builds one instance from its decoded configuration: the objects
    !> serving each kind the provider declared.
    subroutine guatiao_build_i(config, made, error)
      import :: guatiao_config, guatiao_serving, guatiao_error
      class(guatiao_config), intent(in) :: config
      type(guatiao_serving), allocatable, intent(out) :: made(:)
      type(guatiao_error), allocatable, intent(out) :: error
    end subroutine
  end interface

  type :: gt_spec
    character(:), allocatable :: id, display_name, version
    type(guatiao_serving), allocatable :: servings(:)
    class(guatiao_config), allocatable :: prototype
    procedure(guatiao_build_i), pointer, nopass :: build => null()
    type(guatiao_value) :: schema
    integer :: pool = 0
  end type

  type :: gt_library_box
    character(:), allocatable :: id, version
    type(gt_spec), allocatable :: specs(:)
    type(guatiao_value) :: meta
    logical :: built = .false.
    type(gt_library_info), pointer :: info => null()
  end type

  type :: gt_pool_slot
    type(gt_library_box), pointer :: box => null()
    integer :: spec = 0
  end type

  !> A library: its id and version, the providers it offers and its meta.
  !> Copies share one library, kept for the process.
  type :: guatiao_library
    type(gt_library_box), pointer :: box => null()
  contains
    procedure :: single => lib_single
    procedure :: configured => lib_configured
    procedure :: answer => lib_answer
    procedure :: id => lib_id
    procedure :: version => lib_version
  end type

  type(gt_pool_slot), save :: creates(GT_POOL_CREATES)
  type(gt_pool_slot), save :: entries(GT_POOL_ENTRIES)
  integer, save :: creates_used = 0, entries_used = 0

contains

  !> A library named `id`, versioned `version` (semver): write them as
  !> the package's fpm.toml does. `summary`, `license` (SPDX) and
  !> `authors` go in its meta.
  function guatiao_new_library(id, version, summary, license, authors) result(lib)
    character(*), intent(in) :: id, version
    character(*), intent(in), optional :: summary, license
    character(*), intent(in), optional :: authors(:)
    type(guatiao_library) :: lib
    integer :: i
    type(guatiao_value) :: list
    allocate(lib%box)
    lib%box%id = id
    lib%box%version = version
    allocate(lib%box%specs(0))
    if (present(summary) .or. present(license) .or. present(authors)) then
      lib%box%meta = guatiao_map()
      if (present(summary)) call lib%box%meta%set('summary', summary)
      if (present(license)) call lib%box%meta%set('license', license)
      if (present(authors)) then
        call lib%box%meta%set('authors', guatiao_list())
        list = lib%box%meta%key('authors')
        do i = 1, size(authors)
          call list%push(trim(authors(i)))
        end do
      end if
    end if
  end function

  function lib_id(self) result(id)
    class(guatiao_library), intent(in) :: self
    character(:), allocatable :: id
    id = self%box%id
  end function

  function lib_version(self) result(version)
    class(guatiao_library), intent(in) :: self
    character(:), allocatable :: version
    version = self%box%version
  end function

  ! The checks every provider passes, and its spec appended.
  subroutine add(self, name, id, display_name, version, spec, st, msg)
    class(guatiao_library), intent(inout) :: self
    character(*), intent(in) :: name
    character(*), intent(in), optional :: id, display_name, version
    type(gt_spec), intent(inout) :: spec
    integer, intent(out) :: st
    character(:), allocatable, intent(out) :: msg
    integer :: i
    st = GUATIAO_OK
    msg = ''
    if (self%box%built) then
      st = GUATIAO_ERR_BUSY
      msg = 'guatiao: the library already answered a host; add providers before'
      return
    end if
    spec%id = self%box%id // '_' // name
    if (present(id)) spec%id = id
    spec%display_name = ''
    if (present(display_name)) spec%display_name = display_name
    spec%version = ''
    if (present(version)) spec%version = version
    do i = 1, size(self%box%specs)
      if (self%box%specs(i)%id == spec%id) then
        st = GUATIAO_ERR_BAD_VALUE
        msg = 'guatiao: two providers share the id "' // spec%id // '"'
        return
      end if
    end do
    call append_spec(self%box%specs, spec)
  end subroutine

  subroutine append_spec(specs, spec)
    type(gt_spec), allocatable, intent(inout) :: specs(:)
    type(gt_spec), intent(in) :: spec
    type(gt_spec), allocatable :: grown(:)
    integer :: n
    n = size(specs)
    allocate(grown(n + 1))
    if (n > 0) grown(1:n) = specs
    grown(n + 1) = spec
    call move_alloc(grown, specs)
  end subroutine

  !> A provider that is its one instance: `servings`, an object per kind
  !> it serves (a kind's `<kind>_serving` function makes one).
  subroutine lib_single(self, name, servings, display_name, id, version, stat, errmsg)
    class(guatiao_library), intent(inout) :: self
    character(*), intent(in) :: name
    type(guatiao_serving), intent(in) :: servings(:)
    character(*), intent(in), optional :: display_name, id, version
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_spec) :: spec
    integer :: st, i
    character(:), allocatable :: msg
    st = GUATIAO_OK
    msg = ''
    do i = 1, size(servings)
      if (.not. associated(servings(i)%impl)) then
        st = GUATIAO_ERR_NULL
        msg = 'guatiao: "' // name // '" serves ' // servings(i)%kind%name // ' with nothing'
      else if (servings(i)%kind%object) then
        st = GUATIAO_ERR_WRONG_KIND
        msg = 'guatiao: an object kind (' // servings(i)%kind%name // ') is no provider''s kind'
      else if (.not. servings(i)%kind%accepts(servings(i)%impl)) then
        st = GUATIAO_ERR_WRONG_KIND
        msg = 'guatiao: "' // name // '" does not serve ' // servings(i)%kind%name
      end if
    end do
    if (st == GUATIAO_OK) then
      spec%servings = servings
      call add(self, name, id, display_name, version, spec, st, msg)
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> A provider building an instance per configuration: `prototype` is
  !> the configuration type (its schema the provider's), `build` makes an
  !> instance's objects from a decoded copy, `serves` names the kinds (a
  !> kind's `<kind>_serving` with no object).
  subroutine lib_configured(self, name, prototype, build, serves, display_name, id, version, stat, errmsg)
    class(guatiao_library), intent(inout) :: self
    character(*), intent(in) :: name
    class(guatiao_config), intent(in) :: prototype
    procedure(guatiao_build_i) :: build
    type(guatiao_serving), intent(in) :: serves(:)
    character(*), intent(in), optional :: display_name, id, version
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(gt_spec) :: spec
    integer :: st, i
    character(:), allocatable :: msg
    st = GUATIAO_OK
    msg = ''
    do i = 1, size(serves)
      if (serves(i)%kind%object) then
        st = GUATIAO_ERR_WRONG_KIND
        msg = 'guatiao: an object kind (' // serves(i)%kind%name // ') is no provider''s kind'
      end if
    end do
    if (creates_used >= GT_POOL_CREATES) then
      st = GUATIAO_ERR_ALLOC
      msg = 'guatiao: a process holds at most 64 providers that build instances'
    end if
    if (st == GUATIAO_OK) then
      spec%servings = serves
      allocate(spec%prototype, source=prototype)
      spec%build => build
      spec%schema = guatiao_schema_of(prototype)
      call add(self, name, id, display_name, version, spec, st, msg)
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> The descriptor to answer `host` with (a `guatiao_library_info *`),
  !> built on first use: what a library file's `guatiao_library_entry`
  !> returns. Null, declining the host, for a host of another ABI or when
  !> guatiao cannot be opened here (the reason on standard error).
  function lib_answer(self, host) result(info)
    class(guatiao_library), intent(inout) :: self
    type(c_ptr), intent(in) :: host
    type(c_ptr) :: info
    type(gt_host_info), pointer :: h
    integer :: st
    character(:), allocatable :: msg
    info = c_null_ptr
    if (.not. associated(self%box) .or. .not. c_associated(host)) return
    call c_f_pointer(host, h)
    if (h%abi_version /= GUATIAO_ABI_VERSION) return
    call gt_need(GT_CORE, st, msg)
    if (st /= GUATIAO_OK) then
      write (error_unit, '(a)') 'guatiao: ' // self%box%id // ' declines the host: ' // msg
      return
    end if
    if (.not. self%box%built) call build_descriptor(self%box, st, msg)
    if (st /= GUATIAO_OK) then
      write (error_unit, '(a)') 'guatiao: ' // self%box%id // ' declines the host: ' // msg
      return
    end if
    info = c_loc(self%box%info)
  end function

  !> Registers `lib` with `reg` in this process (linked, not mapped:
  !> unload answers GUATIAO_ERR_WRONG_KIND, retire takes it out).
  subroutine guatiao_register_library(reg, lib, answer, stat, errmsg)
    type(guatiao_registry), intent(in) :: reg
    type(guatiao_library), intent(in) :: lib
    type(guatiao_value), intent(inout), optional :: answer
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    integer :: n
    if (entries_used >= GT_POOL_ENTRIES) then
      call gt_report(GUATIAO_ERR_ALLOC, 'guatiao: a process registers at most 16 Fortran libraries', stat, errmsg)
      return
    end if
    gt_entry_target => pool_entry
    entries_used = entries_used + 1
    n = entries_used
    entries(n)%box => lib%box
    call reg%register_entry(lib%box%id, gt_pool_entry_at(n), answer, stat, errmsg)
  end subroutine

  function pool_entry(index, host) result(info)
    integer, intent(in) :: index
    type(c_ptr), intent(in) :: host
    type(c_ptr) :: info
    type(guatiao_library) :: lib
    lib%box => entries(index)%box
    info = lib%answer(host)
  end function

  ! Every pointer the descriptor holds addresses memory kept for the
  ! process: strings and arrays allocated here and never freed.
  subroutine build_descriptor(box, st, msg)
    type(gt_library_box), pointer, intent(in) :: box
    integer, intent(out) :: st
    character(:), allocatable, intent(out) :: msg
    type(gt_provider_info), pointer :: infos(:)
    type(gt_str), pointer :: kinds(:)
    type(gt_kind_table), pointer :: tables(:)
    type(gt_value), pointer :: root
    integer :: p, k, n
    st = GUATIAO_OK
    msg = ''
    n = size(box%specs)
    allocate(infos(max(n, 1)))
    do p = 1, n
      associate (spec => box%specs(p), info => infos(p))
        allocate(kinds(max(size(spec%servings), 1)))
        allocate(tables(max(size(spec%servings), 1)))
        do k = 1, size(spec%servings)
          kinds(k) = keep(spec%servings(k)%kind%name)
          tables(k)%struct_size = int(c_sizeof(tables(k)), c_int32_t)
          tables(k)%vtable_size = int(spec%servings(k)%kind%size, c_int32_t)
          tables(k)%kind = kinds(k)
          tables(k)%vtable = spec%servings(k)%table
        end do
        info%struct_size = int(c_sizeof(info), c_int32_t)
        info%kinds%ptr = c_loc(kinds)
        info%kinds%len = size(spec%servings, kind=c_size_t)
        info%id = keep(spec%id)
        info%display_name = keep(spec%display_name)
        info%version = keep(spec%version)
        info%tables%ptr = c_loc(tables)
        info%tables%len = size(spec%servings, kind=c_size_t)
        info%tables%stride = c_sizeof(tables(1))
        info%destroy = c_funloc(destroy_instance)
        if (allocated(spec%prototype)) then
          info%config = gt_root(spec%schema)
          if (creates_used >= GT_POOL_CREATES) then
            st = GUATIAO_ERR_ALLOC
            msg = 'guatiao: a process holds at most 64 providers that build instances'
            return
          end if
          gt_create_target => pool_create
          creates_used = creates_used + 1
          spec%pool = creates_used
          creates(spec%pool)%box => box
          creates(spec%pool)%spec = p
          info%create = gt_pool_create_at(spec%pool)
        else
          info%ctx = guatiao_new_served(spec%servings)
          info%destroy = c_null_funptr
        end if
      end associate
    end do
    allocate(box%info)
    box%info%struct_size = int(c_sizeof(box%info), c_int32_t)
    box%info%abi_version = GUATIAO_ABI_VERSION
    box%info%id = keep(box%id)
    box%info%version = keep(box%version)
    box%info%providers%ptr = c_loc(infos)
    box%info%providers%len = int(n, c_size_t)
    box%info%providers%stride = c_sizeof(infos(1))
    ! `meta` is a map, not a value: the address of the root's map arm.
    if (c_associated(gt_root(box%meta))) then
      call c_f_pointer(gt_root(box%meta), root)
      if (root%tag == GUATIAO_TAG_MAP) box%info%meta = c_loc(root%payload)
    end if
    box%info%unload = c_funloc(refuse_unload)
    box%built = .true.
  end subroutine

  ! A view of a copy of `text` that lives for the process.
  function keep(text) result(v)
    character(*), intent(in) :: text
    type(gt_str) :: v
    character(:), pointer :: copy
    v = gt_str()
    if (len(text) == 0) return
    allocate(character(len(text)) :: copy)
    copy = text
    v%ptr = c_loc(copy)
    v%len = len(text, kind=c_size_t)
  end function

  ! A provider's `create`, reached through its pool index: the host's
  ! configuration checked against the schema, decoded into a copy of the
  ! prototype, built.
  function pool_create(index, ctx, config, out, err) result(status)
    integer, intent(in) :: index
    type(c_ptr), intent(in) :: ctx, config, out, err
    integer(c_int32_t) :: status
    type(guatiao_value) :: given
    class(guatiao_config), allocatable :: decoded
    type(guatiao_serving), allocatable :: made(:)
    type(guatiao_error), allocatable :: error
    type(c_ptr), pointer :: slot
    integer :: st, i, k
    character(:), allocatable :: msg
    status = GUATIAO_ERR_NULL
    if (.not. c_associated(out)) return
    call c_f_pointer(out, slot)
    slot = c_null_ptr
    associate (spec => creates(index)%box%specs(creates(index)%spec))
      given = guatiao_borrowed_value(config)
      allocate(decoded, source=spec%prototype)
      call guatiao_decode_config(decoded, given, st, msg)
      call gt_end_borrow(given)
      if (st /= GUATIAO_OK) then
        call guatiao_fail(error, st, msg)
        status = guatiao_failed(error, err)
        return
      end if
      call spec%build(decoded, made, error)
      if (.not. allocated(error) .and. .not. allocated(made)) then
        call guatiao_fail(error, GUATIAO_ERR_INTERNAL, 'the build made nothing')
      end if
      if (allocated(error)) then
        status = guatiao_failed(error, err)
        return
      end if
      ! Each object serves its kind through the descriptor's table.
      do i = 1, size(made)
        made(i)%owned = .true.
        do k = 1, size(spec%servings)
          if (spec%servings(k)%kind%name == made(i)%kind%name) made(i)%table = spec%servings(k)%table
        end do
      end do
      slot = guatiao_new_served(made)
      status = GUATIAO_OK
    end associate
  end function

  ! A provider's `destroy`: the instance is the box `create` answered.
  subroutine destroy_instance(ctx, instance) bind(c)
    type(c_ptr), value :: ctx, instance
    call guatiao_destroy_served(instance)
  end subroutine

  ! A Fortran library is never unmapped: a Fortran runtime linked into it
  ! registers exit handlers and keeps unit state no host can see.
  function refuse_unload() bind(c) result(status)
    integer(c_int32_t) :: status
    status = GUATIAO_ERR_BUSY
  end function

end module guatiao_provide
