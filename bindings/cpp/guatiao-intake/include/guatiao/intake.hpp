// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// guatiao-intake for C++: taking a value in from a person. The path
// grammar (`agent[1].name`), flat `key -> text` storage, and how a schema
// is shown (a form: sections, hints, conditions). Header-only over
// <guatiao_intake.h> and the guatiao_intake library.

#ifndef GUATIAO_INTAKE_HPP
#define GUATIAO_INTAKE_HPP

#include <guatiao/guatiao.hpp>
#include <guatiao_intake.h>

namespace guatiao {
namespace intake {

/// What this library writes onto a schema field, and where a provider's
/// `meta` carries its form.
constexpr std::string_view key_x_section = GUATIAO_INTAKE_KEY_X_SECTION;
constexpr std::string_view key_x_order = GUATIAO_INTAKE_KEY_X_ORDER;
constexpr std::string_view key_x_advanced = GUATIAO_INTAKE_KEY_X_ADVANCED;
constexpr std::string_view provider_form_key = GUATIAO_INTAKE_PROVIDER_FORM_KEY;

namespace detail {

using guatiao::detail::blank;
using guatiao::detail::error_of;
using guatiao::detail::str;

inline result<value> answer(guatiao_status s, guatiao_value& out, const char* why) {
  if (s != GUATIAO_OK) {
    guatiao_value_free(&out);
    return error_of(s, why);
  }
  return value::adopt(out);
}

}  // namespace detail

/// Whether `form` fits `schema`. A refusal is BAD_VALUE whose detail is a
/// map: `kind` (`malformed`, `unknown_field`, `duplicate_section`,
/// `condition_refused`, `cyclic_condition`), `at`, whichever of `path`,
/// `id`, `field` and `expected` apply, and `message`.
inline result<void> try_check(view schema, view form) {
  guatiao_value err = detail::blank();
  guatiao_status s = guatiao_intake_check(schema.raw(), form.raw(), default_alloc(), &err);
  if (s == GUATIAO_OK) {
    guatiao_value_free(&err);
    return {};
  }
  return guatiao::detail::with_detail(s, err, "the form does not fit its schema");
}

/// The schema's fields grouped into sections and put in order: a list of
/// `{"section": <map or null>, "fields": [key, ...]}`.
inline result<value> try_layout(view schema, view form, const guatiao_alloc* alloc = default_alloc()) {
  guatiao_value out = detail::blank();
  return detail::answer(guatiao_intake_layout(schema.raw(), form.raw(), alloc, &out), out, "no layout");
}

/// The form a schema implies, for when nobody wrote one.
inline result<value> try_for_schema(view schema, const guatiao_alloc* alloc = default_alloc()) {
  guatiao_value out = detail::blank();
  return detail::answer(guatiao_intake_for_schema(schema.raw(), alloc, &out), out, "no form");
}

/// The form to show the field at `path` with: the one `form` assigns, or
/// the one its schema implies; ABSENT for a field with neither, or a path
/// naming no field.
inline result<value> try_form_for(view form, view schema, std::string_view path,
                                  const guatiao_alloc* alloc = default_alloc()) {
  guatiao_value out = detail::blank();
  return detail::answer(guatiao_intake_form_for(form.raw(), schema.raw(), detail::str(path), alloc, &out), out,
                        "no form");
}

/// Whether the field under `key` is shown, given the `values` entered so
/// far. NOT_FOUND when `key`, or a key a condition reads, names no field.
inline result<bool> try_is_visible(view schema, view form, std::string_view key, view values) {
  bool out = false;
  guatiao_status s = guatiao_intake_is_visible(schema.raw(), form.raw(), detail::str(key), values.raw(), &out);
  if (s != GUATIAO_OK) return detail::error_of(s, "no such field");
  return out;
}

/// The value at `path` inside `v`, borrowed from it; absent when the path
/// names nothing or is not a path. Asks no schema.
inline view at(view v, std::string_view path) noexcept {
  return view(guatiao_intake_path_get(v.raw(), detail::str(path)));
}

/// The schema field governing the flat key `key`, borrowed from
/// `schema`; absent when none.
inline view resolve(view schema, std::string_view key) noexcept {
  return view(guatiao_intake_resolve(schema.raw(), detail::str(key)));
}

/// The flat keys the field `key` projects onto: its own, then one per
/// field of every arm.
inline result<std::vector<std::string>> try_flat_keys(view schema, std::string_view key) {
  guatiao_value out = detail::blank();
  guatiao_status s = guatiao_intake_flat_keys(schema.raw(), detail::str(key), default_alloc(), &out);
  if (s != GUATIAO_OK) {
    guatiao_value_free(&out);
    return detail::error_of(s, "no such field");
  }
  std::vector<std::string> keys;
  for (view k : view(&out).as_list()) keys.emplace_back(k.string_or(""));
  guatiao_value_free(&out);
  return keys;
}

/// `v`, the value of the field `key`, written into a flat store: a map of
/// `key -> text`, one entry per scalar leaf.
inline result<value> try_flatten(view schema, std::string_view key, view v,
                                 const guatiao_alloc* alloc = default_alloc()) {
  guatiao_value out = detail::blank();
  return detail::answer(guatiao_intake_flatten(schema.raw(), detail::str(key), v.raw(), alloc, &out), out,
                        "the value does not fit the field");
}

/// The value of the field `key` rebuilt from a flat store.
inline result<value> try_unflatten(view schema, std::string_view key, view flat,
                                   const guatiao_alloc* alloc = default_alloc()) {
  guatiao_value out = detail::blank();
  return detail::answer(guatiao_intake_unflatten(schema.raw(), detail::str(key), flat.raw(), alloc, &out), out,
                        "nothing is stored under the field");
}

#if GUATIAO_EXCEPTIONS
inline void check(view schema, view form) { try_check(schema, form).value(); }
inline value layout(view schema, view form, const guatiao_alloc* alloc = default_alloc()) {
  return try_layout(schema, form, alloc).value();
}
inline value for_schema(view schema, const guatiao_alloc* alloc = default_alloc()) {
  return try_for_schema(schema, alloc).value();
}
inline value form_for(view form, view schema, std::string_view path, const guatiao_alloc* alloc = default_alloc()) {
  return try_form_for(form, schema, path, alloc).value();
}
inline bool is_visible(view schema, view form, std::string_view key, view values) {
  return try_is_visible(schema, form, key, values).value();
}
inline std::vector<std::string> flat_keys(view schema, std::string_view key) {
  return try_flat_keys(schema, key).value();
}
inline value flatten(view schema, std::string_view key, view v, const guatiao_alloc* alloc = default_alloc()) {
  return try_flatten(schema, key, v, alloc).value();
}
inline value unflatten(view schema, std::string_view key, view flat, const guatiao_alloc* alloc = default_alloc()) {
  return try_unflatten(schema, key, flat, alloc).value();
}
#endif

}  // namespace intake
}  // namespace guatiao

#endif  // GUATIAO_INTAKE_HPP
