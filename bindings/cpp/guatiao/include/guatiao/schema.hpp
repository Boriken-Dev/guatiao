// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// guatiao for C++: checking a value against a schema, hiding its secrets,
// and layering one configuration over another. A schema is a value (JSON
// Schema 2020-12). Part of <guatiao/guatiao.hpp>.

#ifndef GUATIAO_SCHEMA_HPP
#define GUATIAO_SCHEMA_HPP

#include <guatiao/value.hpp>

namespace guatiao {

namespace detail {

/// An error whose detail is the map a call wrote, its message the map's
/// `message` when it has one.
inline ::guatiao::error with_detail(guatiao_status s, guatiao_value& d, const char* fallback) {
  std::string message = fallback;
  view dv(&d);
  if (auto m = dv.find("message").string()) message = std::string(*m);
  return ::guatiao::error(static_cast<status>(s), std::move(message), d);
}

}  // namespace detail

/// Whether `schema` accepts `config`. A refusal is BAD_VALUE with the
/// map <guatiao.h> describes as its detail (`key`, then `expected` or
/// `known`, and `message`): never the refused value itself.
inline result<void> try_validate(view schema, view config) {
  guatiao_value err = detail::blank();
  guatiao_status s = guatiao_schema_validate(schema.raw(), config.raw(), default_alloc(), &err);
  if (s == GUATIAO_OK) {
    guatiao_value_free(&err);
    return {};
  }
  return detail::with_detail(s, err, "the value does not fit its schema");
}

/// A copy of `v` with every value its schema marks secret (`x-sensitive`,
/// `writeOnly`), and everything the schema does not describe, replaced by
/// the string `placeholder`.
inline result<value> try_redact(view schema, view v, std::string_view placeholder,
                                const guatiao_alloc* alloc = default_alloc()) {
  guatiao_value out = detail::blank();
  guatiao_status s = guatiao_schema_redact(schema.raw(), v.raw(), detail::str(placeholder), alloc, &out);
  if (s != GUATIAO_OK) {
    guatiao_value_free(&out);
    return detail::error_of(s, "the value could not be redacted");
  }
  return value::adopt(out);
}

/// How two maps layer.
enum class merge_mode : std::uint32_t {
  /// Top-level keys replace; nested maps are not recursed into.
  simple = GUATIAO_MERGE_SIMPLE,
  /// Recursive maps, lists extended with unique items.
  deep = GUATIAO_MERGE_DEEP,
  /// Recursive maps, lists replaced whole. The usual choice.
  substitute = GUATIAO_MERGE_SUBSTITUTE,
};

/// What a merge also takes: whether `deep` merges the maps of two lists by
/// position, and per-path modes (`tls.ciphers`, `peers[0].name`).
struct merge_options {
  bool merge_lists = false;
  std::vector<std::pair<std::string, merge_mode>> overrides;
};

/// `later` layered over `earlier`, later winning: a new tree in `alloc`.
/// A disagreement is an error whose detail is the map describing it.
inline result<value> try_merge(merge_mode mode, view earlier, view later, const merge_options& options = {},
                               const guatiao_alloc* alloc = default_alloc()) {
  std::vector<guatiao_merge_override> overrides;
  overrides.reserve(options.overrides.size());
  for (const auto& o : options.overrides)
    overrides.push_back(guatiao_merge_override{detail::str(o.first), static_cast<std::uint32_t>(o.second)});
  guatiao_value out = detail::blank();
  guatiao_value err = detail::blank();
  guatiao_status s = guatiao_merge(static_cast<std::uint32_t>(mode), earlier.raw(), later.raw(), alloc,
                                   options.merge_lists ? GUATIAO_MERGE_OPT_MERGELISTS : 0u,
                                   overrides.empty() ? nullptr : overrides.data(), overrides.size(), &out, &err);
  if (s != GUATIAO_OK) {
    guatiao_value_free(&out);
    return detail::with_detail(s, err, "the two values do not merge");
  }
  guatiao_value_free(&err);
  return value::adopt(out);
}

#if GUATIAO_EXCEPTIONS
inline void validate(view schema, view config) { try_validate(schema, config).value(); }
inline value redact(view schema, view v, std::string_view placeholder, const guatiao_alloc* alloc = default_alloc()) {
  return try_redact(schema, v, placeholder, alloc).value();
}
inline value merge(merge_mode mode, view earlier, view later, const merge_options& options = {},
                   const guatiao_alloc* alloc = default_alloc()) {
  return try_merge(mode, earlier, later, options, alloc).value();
}
#endif

}  // namespace guatiao

#endif  // GUATIAO_SCHEMA_HPP
