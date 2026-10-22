! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> Schemas: checking a value against one, hiding what one marks secret,
!> describing an object field by field, and a configuration as a derived
!> type that describes and decodes itself.
module guatiao_schema
  use, intrinsic :: iso_c_binding
  use, intrinsic :: iso_fortran_env, only: int64, real64
  use guatiao_c
  use guatiao_native
  use guatiao_errors
  use guatiao_values
  implicit none
  private

  public :: guatiao_validate, guatiao_redact, guatiao_schema_builder, guatiao_object_schema, guatiao_config, &
            guatiao_schema_of, guatiao_decode_config

  !> An object schema built field by field: `{"type": "object",
  !> "properties": {...}, "required": [...], "additionalProperties": false}`.
  type :: guatiao_schema_builder
    type(guatiao_value) :: doc
  contains
    procedure :: string_field => builder_string
    procedure :: integer_field => builder_integer
    procedure :: number_field => builder_number
    procedure :: boolean_field => builder_boolean
    procedure :: field => builder_field
    procedure :: value => builder_value
  end type

  !> A configuration: a derived type that says what it takes and reads it.
  type, abstract :: guatiao_config
  contains
    procedure(config_describe), deferred :: describe
    procedure(config_decode), deferred :: decode
  end type

  abstract interface
    !> Adds this configuration's fields to `schema`.
    subroutine config_describe(self, schema)
      import :: guatiao_config, guatiao_schema_builder
      class(guatiao_config), intent(in) :: self
      type(guatiao_schema_builder), intent(inout) :: schema
    end subroutine
    !> Reads `value`, already checked against the schema, into `self`.
    subroutine config_decode(self, value, error)
      import :: guatiao_config, guatiao_value, guatiao_error
      class(guatiao_config), intent(inout) :: self
      type(guatiao_value), intent(in) :: value
      type(guatiao_error), allocatable, intent(out) :: error
    end subroutine
  end interface

contains

  !> Whether what `value` names fits `schema`: GUATIAO_OK, or
  !> GUATIAO_ERR_BAD_VALUE with `why` holding the map that says why.
  recursive subroutine guatiao_validate(schema, value, why, stat, errmsg)
    type(guatiao_value), intent(in) :: schema, value
    type(guatiao_value), intent(inout), optional :: why
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: ps, pv, out
    integer :: st
    character(:), allocatable :: msg
    type(guatiao_value) :: answer, said
    call gt_need(GT_CORE, st, msg)
    if (st == GUATIAO_OK) then
      ps = gt_node(schema)
      pv = gt_node(value)
      if (.not. c_associated(ps) .or. .not. c_associated(pv)) then
        st = GUATIAO_ERR_NOT_FOUND
        msg = 'guatiao: nothing to check, or nothing to check it against'
      else
        call gt_new_owned(answer, gt_default_alloc(), out)
        st = gt_schema_validate(ps, pv, gt_default_alloc(), out)
        if (st /= GUATIAO_OK) then
          said = answer%key('message')
          msg = 'guatiao: ' // said%string_or('the value does not fit its schema')
        end if
        if (present(why)) why = answer
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> A copy of what `value` names with every field `schema` marks
  !> (`x-sensitive`, `writeOnly`) replaced by `placeholder`, and whatever
  !> it does not describe hidden too.
  recursive subroutine guatiao_redact(schema, value, placeholder, shown, stat, errmsg)
    type(guatiao_value), intent(in) :: schema, value
    character(*), intent(in), target :: placeholder
    type(guatiao_value), intent(inout) :: shown
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: ps, pv, out
    integer :: st
    character(:), allocatable :: msg
    type(guatiao_value) :: answer
    call gt_need(GT_CORE, st, msg)
    if (st == GUATIAO_OK) then
      ps = gt_node(schema)
      pv = gt_node(value)
      if (.not. c_associated(ps) .or. .not. c_associated(pv)) then
        st = GUATIAO_ERR_NOT_FOUND
        msg = 'guatiao: nothing to redact'
      else
        call gt_new_owned(answer, gt_default_alloc(), out)
        st = gt_schema_redact(ps, pv, gt_view(placeholder), gt_default_alloc(), out)
        if (st == GUATIAO_OK) then
          shown = answer
        else
          msg = 'guatiao: the schema is not a map'
        end if
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> An empty object schema to add fields to.
  recursive function guatiao_object_schema(title, description) result(s)
    character(*), intent(in), optional :: title, description
    type(guatiao_schema_builder) :: s
    s%doc = guatiao_map()
    call s%doc%set('type', 'object')
    if (present(title)) call s%doc%set('title', title)
    if (present(description)) call s%doc%set('description', description)
    call s%doc%set('properties', guatiao_map())
    call s%doc%set('additionalProperties', .false.)
  end function

  ! A new property `name` of `json_type`, required unless said otherwise.
  recursive subroutine property(self, name, json_type, description, required, field)
    class(guatiao_schema_builder), intent(inout) :: self
    character(*), intent(in) :: name, json_type
    character(*), intent(in), optional :: description
    logical, intent(in) :: required
    type(guatiao_value), intent(inout) :: field
    type(guatiao_value) :: props
    props = self%doc%key('properties')
    call props%set(name, guatiao_map())
    field = props%key(name)
    call field%set('type', json_type)
    if (present(description)) call field%set('description', description)
    if (required) call require(self, name)
  end subroutine

  recursive subroutine require(self, name)
    class(guatiao_schema_builder), intent(inout) :: self
    character(*), intent(in) :: name
    type(guatiao_value) :: list
    if (.not. self%doc%has('required')) call self%doc%set('required', guatiao_list())
    list = self%doc%key('required')
    call list%push(name)
  end subroutine

  recursive logical function wanted(required, has_default)
    logical, intent(in), optional :: required
    logical, intent(in) :: has_default
    wanted = .not. has_default
    if (present(required)) wanted = required
  end function

  !> A text field. Required unless `required` says otherwise or it has a
  !> default; `sensitive` marks it a secret (`x-sensitive`).
  recursive subroutine builder_string(self, name, description, default, min_length, max_length, pattern, format, &
                            sensitive, required)
    class(guatiao_schema_builder), intent(inout) :: self
    character(*), intent(in) :: name
    character(*), intent(in), optional :: description, default, pattern, format
    integer, intent(in), optional :: min_length, max_length
    logical, intent(in), optional :: sensitive, required
    type(guatiao_value) :: f
    call property(self, name, 'string', description, wanted(required, present(default)), f)
    if (present(default)) call f%set('default', default)
    if (present(min_length)) call f%set('minLength', min_length)
    if (present(max_length)) call f%set('maxLength', max_length)
    if (present(pattern)) call f%set('pattern', pattern)
    if (present(format)) call f%set('format', format)
    if (present(sensitive)) then
      if (sensitive) call f%set('x-sensitive', .true.)
    end if
  end subroutine

  !> An integer field, with optional bounds.
  recursive subroutine builder_integer(self, name, description, default, minimum, maximum, required)
    class(guatiao_schema_builder), intent(inout) :: self
    character(*), intent(in) :: name
    character(*), intent(in), optional :: description
    integer(int64), intent(in), optional :: default, minimum, maximum
    logical, intent(in), optional :: required
    type(guatiao_value) :: f
    call property(self, name, 'integer', description, wanted(required, present(default)), f)
    if (present(default)) call f%set('default', default)
    if (present(minimum)) call f%set('minimum', minimum)
    if (present(maximum)) call f%set('maximum', maximum)
  end subroutine

  !> A number field, with optional bounds.
  recursive subroutine builder_number(self, name, description, default, minimum, maximum, required)
    class(guatiao_schema_builder), intent(inout) :: self
    character(*), intent(in) :: name
    character(*), intent(in), optional :: description
    real(real64), intent(in), optional :: default, minimum, maximum
    logical, intent(in), optional :: required
    type(guatiao_value) :: f
    call property(self, name, 'number', description, wanted(required, present(default)), f)
    if (present(default)) call f%set('default', default)
    if (present(minimum)) call f%set('minimum', minimum)
    if (present(maximum)) call f%set('maximum', maximum)
  end subroutine

  !> A boolean field.
  recursive subroutine builder_boolean(self, name, description, default, required)
    class(guatiao_schema_builder), intent(inout) :: self
    character(*), intent(in) :: name
    character(*), intent(in), optional :: description
    logical, intent(in), optional :: default, required
    type(guatiao_value) :: f
    call property(self, name, 'boolean', description, wanted(required, present(default)), f)
    if (present(default)) call f%set('default', default)
  end subroutine

  !> A field whose schema is written out in full (a copy of `schema`).
  recursive subroutine builder_field(self, name, schema, required)
    class(guatiao_schema_builder), intent(inout) :: self
    character(*), intent(in) :: name
    type(guatiao_value), intent(in) :: schema
    logical, intent(in), optional :: required
    type(guatiao_value) :: props
    props = self%doc%key('properties')
    call props%set(name, schema)
    if (wanted(required, .false.)) call require(self, name)
  end subroutine

  !> The schema document built so far.
  recursive function builder_value(self) result(v)
    class(guatiao_schema_builder), intent(in) :: self
    type(guatiao_value) :: v
    v = self%doc
  end function

  !> The schema a configuration type describes.
  recursive function guatiao_schema_of(config) result(schema)
    class(guatiao_config), intent(in) :: config
    type(guatiao_value) :: schema
    type(guatiao_schema_builder) :: b
    b = guatiao_object_schema()
    call config%describe(b)
    schema = b%value()
  end function

  !> Checks `value` against the configuration's schema, then decodes it
  !> into `config`.
  recursive subroutine guatiao_decode_config(config, value, stat, errmsg)
    class(guatiao_config), intent(inout) :: config
    type(guatiao_value), intent(in) :: value
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_value) :: schema
    type(guatiao_error), allocatable :: error
    integer :: st
    character(:), allocatable :: msg
    schema = guatiao_schema_of(config)
    call guatiao_validate(schema, value, stat=st, errmsg=msg)
    if (st == GUATIAO_OK) then
      call config%decode(value, error)
      call gt_report_error(error, stat, errmsg)
    else
      call gt_report(st, msg, stat, errmsg)
    end if
  end subroutine

end module guatiao_schema
