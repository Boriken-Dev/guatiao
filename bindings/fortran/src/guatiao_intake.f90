! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> Taking a value in from a person, through guatiao_intake: whether a
!> form fits its schema, how its fields are laid out and which are shown,
!> paths in the intake grammar, and the flat `key -> text` store.
module guatiao_intake
  use, intrinsic :: iso_c_binding
  use guatiao_c
  use guatiao_intake_c
  use guatiao_native
  use guatiao_errors
  use guatiao_values
  implicit none
  private

  public :: guatiao_intake_check, guatiao_intake_layout, guatiao_intake_is_visible, guatiao_intake_at, &
            guatiao_intake_for_schema, guatiao_intake_form_for, guatiao_intake_resolve, &
            guatiao_intake_flat_keys, guatiao_intake_flatten, guatiao_intake_unflatten

contains

  ! The node each argument names, or a status saying why not.
  subroutine nodes(st, msg, a, pa, b, pb, c, pc)
    integer, intent(out) :: st
    character(:), allocatable, intent(out) :: msg
    type(guatiao_value), intent(in) :: a
    type(c_ptr), intent(out) :: pa
    type(guatiao_value), intent(in), optional :: b, c
    type(c_ptr), intent(out), optional :: pb, pc
    call gt_need(GT_INTAKE, st, msg)
    if (st /= GUATIAO_OK) return
    call one(a, pa)
    if (present(b)) call one(b, pb)
    if (present(c)) call one(c, pc)
  contains
    subroutine one(v, p)
      type(guatiao_value), intent(in) :: v
      type(c_ptr), intent(out) :: p
      p = gt_node(v)
      if (st /= GUATIAO_OK) return
      if (v%is_closed()) then
        st = GUATIAO_STAT_CLOSED
        msg = 'guatiao: an argument''s tree is closed'
      else if (.not. c_associated(p)) then
        st = GUATIAO_ERR_NOT_FOUND
        msg = 'guatiao: an argument names nothing'
      end if
    end subroutine
  end subroutine

  !> Whether `form` fits `schema`: GUATIAO_OK, or GUATIAO_ERR_BAD_VALUE
  !> with `why` holding the map that says why (`kind`, `at`, `message`, ...).
  subroutine guatiao_intake_check(schema, form, why, stat, errmsg)
    type(guatiao_value), intent(in) :: schema, form
    type(guatiao_value), intent(inout) :: why
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: ps, pf, out
    integer :: st
    character(:), allocatable :: msg
    type(guatiao_value) :: answer, said
    call nodes(st, msg, schema, ps, form, pf)
    if (st == GUATIAO_OK) then
      call gt_new_owned(answer, gt_default_alloc(), out)
      st = gt_intake_check(ps, pf, gt_default_alloc(), out)
      why = answer
      if (st /= GUATIAO_OK) then
        said = why%key('message')
        msg = 'guatiao: the form does not fit its schema: ' // said%string_or('')
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> The schema's fields in sections, in order: a list of
  !> `{"section": map or null, "fields": [key, ...]}`.
  subroutine guatiao_intake_layout(schema, form, layout, stat, errmsg)
    type(guatiao_value), intent(in) :: schema, form
    type(guatiao_value), intent(inout) :: layout
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: ps, pf, out
    integer :: st
    character(:), allocatable :: msg
    type(guatiao_value) :: answer
    call nodes(st, msg, schema, ps, form, pf)
    if (st == GUATIAO_OK) then
      call gt_new_owned(answer, gt_default_alloc(), out)
      st = gt_intake_layout(ps, pf, gt_default_alloc(), out)
      if (st == GUATIAO_OK) then
        layout = answer
      else
        msg = 'guatiao: no layout for that schema and form'
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> Whether the field under `key` is shown, given the `values` entered
  !> so far (GUATIAO_ERR_NOT_FOUND for a key the schema does not declare).
  subroutine guatiao_intake_is_visible(schema, form, key, values, shown, stat, errmsg)
    type(guatiao_value), intent(in) :: schema, form, values
    character(*), intent(in), target :: key
    logical, intent(inout) :: shown
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: ps, pf, pv
    logical(c_bool), target :: out
    integer :: st
    character(:), allocatable :: msg
    call nodes(st, msg, schema, ps, form, pf, values, pv)
    if (st == GUATIAO_OK) then
      out = .false.
      st = gt_intake_is_visible(ps, pf, gt_view(key), pv, c_loc(out))
      if (st == GUATIAO_OK) then
        shown = out
      else
        msg = 'guatiao: the schema declares no field "' // key // '"'
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> The node at `path` (`agent[1].name`) inside what `value` names: a
  !> read-only handle on the same tree, found again on every access.
  function guatiao_intake_at(value, path, stat, errmsg) result(at)
    type(guatiao_value), intent(in) :: value
    character(*), intent(in) :: path
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_value) :: at
    integer :: st
    character(:), allocatable :: msg
    call gt_need(GT_INTAKE, st, msg)
    if (st == GUATIAO_OK) at = gt_via(value, GT_VIA_PATH, path)
    call gt_report(st, msg, stat, errmsg)
  end function

  !> The field of `schema` governing the flat key `key`: a read-only
  !> handle on the schema's tree, absent for a key it does not declare.
  function guatiao_intake_resolve(schema, key, stat, errmsg) result(field)
    type(guatiao_value), intent(in) :: schema
    character(*), intent(in) :: key
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(guatiao_value) :: field
    integer :: st
    character(:), allocatable :: msg
    call gt_need(GT_INTAKE, st, msg)
    if (st == GUATIAO_OK) field = gt_via(schema, GT_VIA_RESOLVE, key)
    call gt_report(st, msg, stat, errmsg)
  end function

  !> The form a schema implies, for when nobody wrote one.
  subroutine guatiao_intake_for_schema(schema, form, stat, errmsg)
    type(guatiao_value), intent(in) :: schema
    type(guatiao_value), intent(inout) :: form
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: ps, out
    integer :: st
    character(:), allocatable :: msg
    type(guatiao_value) :: answer
    call nodes(st, msg, schema, ps)
    if (st == GUATIAO_OK) then
      call gt_new_owned(answer, gt_default_alloc(), out)
      st = gt_intake_for_schema(ps, gt_default_alloc(), out)
      if (st == GUATIAO_OK) then
        form = answer
      else
        msg = 'guatiao: no form for that schema'
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> The form to show the field at `path` with: the one `form` assigns,
  !> or the one its schema implies; absent when there is none.
  subroutine guatiao_intake_form_for(form, schema, path, sub, stat, errmsg)
    type(guatiao_value), intent(in) :: form, schema
    character(*), intent(in), target :: path
    type(guatiao_value), intent(inout) :: sub
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: pf, ps, out
    integer :: st
    character(:), allocatable :: msg
    type(guatiao_value) :: answer
    call nodes(st, msg, form, pf, schema, ps)
    if (st == GUATIAO_OK) then
      call gt_new_owned(answer, gt_default_alloc(), out)
      st = gt_intake_form_for(pf, ps, gt_view(path), gt_default_alloc(), out)
      if (st == GUATIAO_OK) then
        sub = answer
      else
        msg = 'guatiao: no form for "' // path // '"'
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> The flat keys the field `key` projects onto.
  subroutine guatiao_intake_flat_keys(schema, key, keys, stat, errmsg)
    type(guatiao_value), intent(in) :: schema
    character(*), intent(in), target :: key
    type(guatiao_text), allocatable, intent(inout) :: keys(:)
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: ps, out
    integer :: st, i
    character(:), allocatable :: msg
    type(guatiao_value) :: answer, item
    call nodes(st, msg, schema, ps)
    if (st == GUATIAO_OK) then
      call gt_new_owned(answer, gt_default_alloc(), out)
      st = gt_intake_flat_keys(ps, gt_view(key), gt_default_alloc(), out)
      if (st == GUATIAO_OK) then
        if (allocated(keys)) deallocate(keys)
        allocate(keys(answer%length()))
        do i = 1, size(keys)
          item = answer%item(i)
          call item%get(keys(i)%text)
        end do
      else
        msg = 'guatiao: the schema declares no field "' // key // '"'
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> Writes what `value` names into a flat store (a map of text to text),
  !> one entry per scalar leaf of the field `key`.
  subroutine guatiao_intake_flatten(schema, key, value, store, stat, errmsg)
    type(guatiao_value), intent(in) :: schema, value
    character(*), intent(in), target :: key
    type(guatiao_value), intent(inout) :: store
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: ps, pv, out
    integer :: st
    character(:), allocatable :: msg
    type(guatiao_value) :: answer
    call nodes(st, msg, schema, ps, value, pv)
    if (st == GUATIAO_OK) then
      call gt_new_owned(answer, gt_default_alloc(), out)
      st = gt_intake_flatten(ps, gt_view(key), pv, gt_default_alloc(), out)
      if (st == GUATIAO_OK) then
        store = answer
      else
        msg = 'guatiao: "' // key // '" does not apply to that value'
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

  !> Rebuilds the field `key` from a flat store; `value` is absent when
  !> nothing is stored under it. A store that is not text to text is
  !> GUATIAO_ERR_WRONG_KIND.
  subroutine guatiao_intake_unflatten(schema, key, store, value, stat, errmsg)
    type(guatiao_value), intent(in) :: schema, store
    character(*), intent(in), target :: key
    type(guatiao_value), intent(inout) :: value
    integer, intent(out), optional :: stat
    character(:), allocatable, intent(inout), optional :: errmsg
    type(c_ptr) :: ps, pv, out
    integer :: st, i
    character(:), allocatable :: msg
    type(guatiao_value) :: answer, entry
    type(guatiao_text), allocatable :: names(:)
    call nodes(st, msg, schema, ps, store, pv)
    if (st == GUATIAO_OK) then
      if (store%tag() /= GUATIAO_TAG_MAP) st = GUATIAO_ERR_WRONG_KIND
      names = store%keys()
      do i = 1, size(names)
        entry = store%key(names(i)%text)
        if (entry%tag() /= GUATIAO_TAG_STRING) st = GUATIAO_ERR_WRONG_KIND
      end do
      if (st /= GUATIAO_OK) msg = 'guatiao: a flat store maps text to text'
    end if
    if (st == GUATIAO_OK) then
      call gt_new_owned(answer, gt_default_alloc(), out)
      st = gt_intake_unflatten(ps, gt_view(key), pv, gt_default_alloc(), out)
      if (st == GUATIAO_ERR_WRONG_KIND) then
        call answer%close()
        value = guatiao_absent()
        st = GUATIAO_OK
      else if (st == GUATIAO_OK) then
        value = answer
      else
        msg = 'guatiao: the store does not rebuild "' // key // '"'
      end if
    end if
    call gt_report(st, msg, stat, errmsg)
  end subroutine

end module guatiao_intake
