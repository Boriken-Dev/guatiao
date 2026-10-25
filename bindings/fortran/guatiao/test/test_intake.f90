! This Source Code Form is subject to the terms of the Mozilla Public
! License, v. 2.0. If a copy of the MPL was not distributed with this
! file, You can obtain one at https://mozilla.org/MPL/2.0/.

!> Forms: checking one, laying fields out, which are shown, paths,
!> implied forms, and the flat `key -> text` store both ways.
program test_intake
  use harness
  use guatiao
  use guatiao_serde
  use guatiao_intake
  implicit none

  character(*), parameter :: NL = new_line('a')
  character(*), parameter :: SIMPLE = '{"type": "object", "properties": {"name": {"type": "string", ' // &
    '"title": "Name"}}, "required": ["name"], "additionalProperties": false}'
  character(*), parameter :: FLAT = '{"type": "object", "properties": {' // NL // &
    '"auth": {"type": "object", "x-variant-tag": "auth", "oneOf": [' // NL // &
    '{"type": "object", "properties": {"auth": {"const": "sso"}}, "required": ["auth"]},' // NL // &
    '{"type": "object", "properties": {"auth": {"const": "userpass"}, "username": {"type": "string"},' // NL // &
    '"password": {"type": "string", "x-sensitive": true}}, "required": ["auth"]}]},' // NL // &
    '"agent": {"type": "array", "items": {"type": "object", "properties": {"name": {"type": "string"}}}},' // NL // &
    '"env": {"type": "object", "additionalProperties": {"type": "string"}},' // NL // &
    '"host": {"type": "string", "title": "Host"}}}'

  call setup()
  call checks_a_form()
  call lays_out_and_says_what_is_shown()
  call names_a_place_with_a_path()
  call makes_a_form_out_of_a_schema()
  call resolves_a_flat_key()
  call flat_keys_flatten_and_unflatten()
  call a_view_is_gone_once_its_source_is_closed()
  call a_providers_form_from_its_meta_fits_its_schema()
  call finish('intake')

contains

  function json(text) result(v)
    character(*), intent(in) :: text
    type(guatiao_value) :: v
    call guatiao_parse(text, GUATIAO_FORMAT_JSON, v)
  end function

  subroutine a_providers_form_from_its_meta_fits_its_schema()
    type(guatiao_registry) :: reg
    type(guatiao_value) :: meta, form, schema, why, got
    logical :: shown
    integer :: st
    reg = guatiao_open_registry('fortran-intake', '0.1')
    call reg%scan_dir(library_dir(), rules=['kind=greeter'])
    call reg%provider_meta('derived_greeter_shouter', meta)
    form = meta%key(GUATIAO_INTAKE_PROVIDER_FORM_KEY)
    schema = reg%provider_config('derived_greeter_shouter')
    call guatiao_intake_check(schema, form, why, st)
    call check_status(st, GUATIAO_OK, 'the provider''s form fits its schema')
    got = form%at('fields', 'prefix', 'placeholder')
    call check_text(got%string_or(''), 'hey', 'a placeholder')
    got = form%at('fields', 'marks', 'unit')
    call check_text(got%string_or(''), 'marks', 'a unit')
    shown = .true.
    call guatiao_intake_is_visible(schema, form, 'marks', json('{"exclaim": false}'), shown)
    call check(.not. shown, 'hidden while exclaim is false')
    call guatiao_intake_is_visible(schema, form, 'marks', json('{"exclaim": true}'), shown)
    call check(shown, 'shown once it is true')
  end subroutine

  subroutine checks_a_form()
    type(guatiao_value) :: schema, why, kind
    integer :: st
    schema = json(SIMPLE)
    call guatiao_intake_check(schema, json('{}'), why, st)
    call check_status(st, GUATIAO_OK, 'an empty form fits')
    call check(why%is_absent(), 'and says nothing')
    call guatiao_intake_check(schema, json('{"sections": [], "fields": {"nope": {}}}'), why, st)
    call check_status(st, GUATIAO_ERR_BAD_VALUE, 'a form naming no field')
    kind = why%key('kind')
    call check_text(kind%string_or(''), 'unknown_field', 'says which rule')
  end subroutine

  subroutine lays_out_and_says_what_is_shown()
    type(guatiao_value) :: schema, layout, field
    logical :: shown
    integer :: st
    schema = json(SIMPLE)
    call guatiao_intake_layout(schema, json('{}'), layout)
    call check(layout%length() == 1, 'one section')
    field = layout%at(1, 'fields', 1)
    call check_text(field%string_or(''), 'name', 'holding the field')
    shown = .false.
    call guatiao_intake_is_visible(schema, json('{}'), 'name', json('{"name": "ana"}'), shown)
    call check(shown, 'a field with no condition is shown')
    call guatiao_intake_is_visible(schema, json('{}'), 'nope', json('{}'), shown, st)
    call check_status(st, GUATIAO_ERR_NOT_FOUND, 'a key the schema does not declare')
  end subroutine

  subroutine names_a_place_with_a_path()
    type(guatiao_value) :: v, at
    integer :: st
    v = json('{"agent": [{"name": "one"}, {"name": "two"}], "env": {"PATH": "/bin", "a.b": "dotted", "1": "keyed"}}')
    at = guatiao_intake_at(v, 'agent[0].name')
    call check_text(at%string_or(''), 'one', 'agent[0].name')
    at = guatiao_intake_at(v, 'agent[1].name')
    call check_text(at%string_or(''), 'two', 'agent[1].name')
    at = guatiao_intake_at(v, 'env[PATH]')
    call check_text(at%string_or(''), '/bin', 'env[PATH]')
    at = guatiao_intake_at(v, 'env[a.b]')
    call check_text(at%string_or(''), 'dotted', 'env[a.b]')
    at = guatiao_intake_at(v, 'env["1"]')
    call check_text(at%string_or(''), 'keyed', 'env["1"]')
    at = guatiao_intake_at(v, 'agent[9].name')
    call check(at%is_absent(), 'past the end names nothing')
    at = guatiao_intake_at(v, 'agent[')
    call check(at%is_absent(), 'and so does what is not a path')
    at = guatiao_intake_at(v, 'agent[0]')
    call at%set('name', 'changed', st)
    call check_status(st, GUATIAO_STAT_READ_ONLY, 'a path is read-only')
  end subroutine

  subroutine makes_a_form_out_of_a_schema()
    type(guatiao_value) :: schema, made, id, why, implied
    integer :: st
    schema = json('{"type": "object", "properties": {"host": {"type": "string", "x-section": "net"}, ' // &
      '"tls": {"type": "object", "properties": {"verify": {"type": "boolean", "x-section": "trust"}}, ' // &
      '"additionalProperties": false}, "label": {"type": "string"}}, "additionalProperties": false}')
    call guatiao_intake_for_schema(schema, made)
    id = made%at('sections', 1, 'id')
    call check_text(id%string_or(''), 'net', 'its first section')
    call guatiao_intake_check(schema, made, why, st)
    call check_status(st, GUATIAO_OK, 'and it fits')
    call guatiao_intake_form_for(json('{}'), schema, 'tls', implied)
    call check(.not. implied%is_absent(), 'a member implies a form')
    call guatiao_intake_form_for(json('{}'), schema, 'label', implied)
    call check(implied%is_absent(), 'a plain field has none')
    call guatiao_intake_form_for(json('{}'), schema, 'nonesuch', implied)
    call check(implied%is_absent(), 'nor does a path naming nothing')
  end subroutine

  subroutine resolves_a_flat_key()
    type(guatiao_value) :: schema, field, title, flag
    schema = json(FLAT)
    field = guatiao_intake_resolve(schema, 'host')
    title = field%key('title')
    call check_text(title%string_or(''), 'Host', 'host')
    field = guatiao_intake_resolve(schema, 'auth.password')
    flag = field%key('x-sensitive')
    call check(flag%bool_or(.false.), 'auth.password, through the arm')
    field = guatiao_intake_resolve(schema, 'nonesuch')
    call check(field%is_absent(), 'nonesuch')
  end subroutine

  subroutine flat_keys_flatten_and_unflatten()
    type(guatiao_value) :: schema, store, back, item
    type(guatiao_text), allocatable :: keys(:)
    character(:), allocatable :: text
    integer :: st
    schema = json(FLAT)
    call guatiao_intake_flat_keys(schema, 'auth', keys)
    call check(size(keys) == 3, 'three flat keys')
    if (size(keys) == 3) call check(keys(1)%text == 'auth' .and. keys(2)%text == 'auth.username' .and. &
                                    keys(3)%text == 'auth.password', 'auth, auth.username, auth.password')
    call guatiao_intake_flat_keys(schema, 'nonesuch', keys, st)
    call check_status(st, GUATIAO_ERR_WRONG_KIND, 'a key naming no field')
    call round('auth', '{"auth": "userpass", "username": "alice", "password": "s3cret"}', &
               '{"auth":"userpass","auth.password":"s3cret","auth.username":"alice"}')
    call round('auth', '{"auth": "sso"}', '{"auth":"sso"}')
    call round('agent', '[{"name": "one"}, {"name": "two"}]', '{"agent[0].name":"one","agent[1].name":"two"}')
    call round('env', '{"PATH": "/bin", "a.b": "dotted"}', '{"env[PATH]":"/bin","env[a.b]":"dotted"}')
    call round('host', '"example.org"', '{"host":"example.org"}')
    call guatiao_intake_unflatten(schema, 'auth', json('{}'), back)
    call check(back%is_absent(), 'nothing stored is nothing rebuilt')
    call guatiao_intake_unflatten(schema, 'auth', json('{"auth": 1}'), back, st)
    call check_status(st, GUATIAO_ERR_WRONG_KIND, 'a store that is not text to text')
    store = json('{"agent[0].name": "x"}')
    call guatiao_intake_unflatten(schema, 'agent', store, back)
    item = back%at(1, 'name')
    call check_text(item%string_or(''), 'x', 'a list rebuilt')
    call guatiao_emit(back, GUATIAO_FORMAT_JSON, text)
    call check_text(text, '[{"name":"x"}]', 'and only that')
  end subroutine

  subroutine round(key, value_json, flat_json)
    character(*), intent(in) :: key, value_json, flat_json
    type(guatiao_value) :: schema, value, store, back
    character(:), allocatable :: text, again
    schema = json(FLAT)
    value = json(value_json)
    call guatiao_intake_flatten(schema, key, value, store)
    call guatiao_emit(store, GUATIAO_FORMAT_JSON, text)
    call check_text(text, flat_json, 'flatten ' // key)
    call guatiao_intake_unflatten(schema, key, store, back)
    call guatiao_emit(back, GUATIAO_FORMAT_JSON, again)
    call guatiao_emit(value, GUATIAO_FORMAT_JSON, text)
    call check_text(again, text, 'unflatten ' // key)
  end subroutine

  subroutine a_view_is_gone_once_its_source_is_closed()
    type(guatiao_value) :: v, at
    v = json('{"a": {"b": 1}}')
    at = guatiao_intake_at(v, 'a.b')
    call check(.not. at%is_absent(), 'found')
    call v%set('a', 'changed')
    call check(at%is_absent(), 'found again: the source changed')
    call v%close()
    call check(at%is_closed(), 'gone with its source')
  end subroutine

end program test_intake
