// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// guatiao-serde for C++: a value read from and written as JSON, TOML or
// YAML, header-only over <guatiao_serde.h> and the guatiao_serde library.
//
// JSON is in every build. TOML and YAML are in the library only when it
// was built with those features, and declared here only when
// GUATIAO_SERDE_TOML / GUATIAO_SERDE_YAML say so, as <guatiao_serde.h>
// does; asked for a format this build leaves out, a call answers
// UNSUPPORTED.

#ifndef GUATIAO_SERDE_HPP
#define GUATIAO_SERDE_HPP

#include <guatiao/guatiao.hpp>
#include <guatiao_serde.h>

namespace guatiao {
inline namespace GUATIAO_ABI {
namespace serde {

/// A document format.
enum class format : std::uint32_t {
  json = GUATIAO_FORMAT_JSON,
  toml = GUATIAO_FORMAT_TOML,
  yaml = GUATIAO_FORMAT_YAML,
};

constexpr const char* format_name(format f) noexcept {
  switch (f) {
    case format::json: return "json";
    case format::toml: return "toml";
    case format::yaml: return "yaml";
  }
  return "format";
}

/// The `how` argument: its low byte says how a byte string is spelled
/// where the format has none, the bits above are flags. Zero is the
/// default everywhere.
namespace how {
constexpr std::uint32_t data_uri = GUATIAO_BYTES_DATA_URI;
constexpr std::uint32_t base64 = GUATIAO_BYTES_BASE64;
constexpr std::uint32_t array = GUATIAO_BYTES_ARRAY;
constexpr std::uint32_t refuse = GUATIAO_BYTES_REFUSE;
/// Turn a `data:;base64,...` string back into bytes when reading.
constexpr std::uint32_t read_data_uris = GUATIAO_READ_DATA_URIS;
/// Indent JSON for a person to read.
constexpr std::uint32_t pretty = GUATIAO_PRETTY;
}  // namespace how

namespace detail {

using guatiao::detail::blank;
using guatiao::detail::error_of;
using guatiao::detail::str;

inline ::guatiao::error unsupported(format f) {
  return error_of(GUATIAO_ERR_UNSUPPORTED, std::string("this build leaves out ") + format_name(f));
}

/// A document's refusal: BAD_VALUE, the parser's words as the message and
/// `{"format", "line", "column"}` as the detail (line and column from 1,
/// 0 when the parser did not say).
inline ::guatiao::error parse_failure(guatiao_status s, guatiao_parse_error& pe) {
  std::string message;
  if (pe.message.tag == GUATIAO_STRING) message = std::string(guatiao::detail::text(pe.message.payload.text));
  guatiao_value_free(&pe.message);
  guatiao_value d = blank();
  item why = item::map({{"format", format_name(static_cast<format>(pe.format))},
                        {"line", pe.line},
                        {"column", pe.column}});
  if (why.build(default_alloc(), &d) != GUATIAO_OK) d = blank();
  return ::guatiao::error(static_cast<status>(s), std::move(message), d);
}

}  // namespace detail

/// `text` read as a document of format `f`, built in `alloc`.
inline result<value> try_parse(format f, std::string_view text, std::uint32_t how = 0,
                               const guatiao_alloc* alloc = default_alloc()) {
  guatiao_value out = detail::blank();
  guatiao_parse_error pe;
  std::memset(&pe, 0, sizeof pe);
  guatiao_status s = GUATIAO_ERR_UNSUPPORTED;
  switch (f) {
    case format::json: s = guatiao_json_parse_ex(detail::str(text), how, alloc, &out, &pe); break;
    case format::toml:
#if defined(GUATIAO_SERDE_TOML)
      s = guatiao_toml_parse_ex(detail::str(text), how, alloc, &out, &pe);
      break;
#else
      return detail::unsupported(f);
#endif
    case format::yaml:
#if defined(GUATIAO_SERDE_YAML)
      s = guatiao_yaml_parse_ex(detail::str(text), how, alloc, &out, &pe);
      break;
#else
      return detail::unsupported(f);
#endif
  }
  if (s != GUATIAO_OK) {
    guatiao_value_free(&out);
    if (pe.message.tag != GUATIAO_ABSENT) return detail::parse_failure(s, pe);
    return detail::error_of(s);
  }
  guatiao_value_free(&pe.message);
  return value::adopt(out);
}

/// `v` written as a document of format `f`. JSON is compact unless `how`
/// carries `how::pretty`; TOML takes a map only.
inline result<std::string> try_emit(format f, view v, std::uint32_t how = 0) {
  guatiao_value out = detail::blank();
  guatiao_status s = GUATIAO_ERR_UNSUPPORTED;
  switch (f) {
    case format::json: s = guatiao_json_emit(v.raw(), how, default_alloc(), &out); break;
    case format::toml:
#if defined(GUATIAO_SERDE_TOML)
      s = guatiao_toml_emit(v.raw(), how, default_alloc(), &out);
      break;
#else
      return detail::unsupported(f);
#endif
    case format::yaml:
#if defined(GUATIAO_SERDE_YAML)
      s = guatiao_yaml_emit(v.raw(), how, default_alloc(), &out);
      break;
#else
      return detail::unsupported(f);
#endif
  }
  if (s != GUATIAO_OK) {
    guatiao_value_free(&out);
    return detail::error_of(s, std::string("the value cannot be written as ") + format_name(f));
  }
  std::string text(view(&out).string_or(""));
  guatiao_value_free(&out);
  return text;
}

/// A document of format `f` straight to wire bytes, no tree built.
inline result<std::vector<std::uint8_t>> try_to_wire(format f, std::string_view text, std::uint32_t how = 0) {
  guatiao_value out = detail::blank();
  guatiao_parse_error pe;
  std::memset(&pe, 0, sizeof pe);
  guatiao_status s = GUATIAO_ERR_UNSUPPORTED;
  switch (f) {
    case format::json: s = guatiao_json_to_wire(detail::str(text), how, default_alloc(), &out, &pe); break;
    case format::toml:
#if defined(GUATIAO_SERDE_TOML)
      s = guatiao_toml_to_wire(detail::str(text), how, default_alloc(), &out, &pe);
      break;
#else
      return detail::unsupported(f);
#endif
    case format::yaml:
#if defined(GUATIAO_SERDE_YAML)
      s = guatiao_yaml_to_wire(detail::str(text), how, default_alloc(), &out, &pe);
      break;
#else
      return detail::unsupported(f);
#endif
  }
  if (s != GUATIAO_OK) {
    guatiao_value_free(&out);
    if (pe.message.tag != GUATIAO_ABSENT) return detail::parse_failure(s, pe);
    return detail::error_of(s);
  }
  guatiao_value_free(&pe.message);
  return guatiao::detail::take_bytes(s, out);
}

/// Wire bytes straight to a document of format `f`, no tree built.
inline result<std::string> try_from_wire(format f, bytes_view bytes, std::uint32_t how = 0) {
  guatiao_value out = detail::blank();
  guatiao_status s = GUATIAO_ERR_UNSUPPORTED;
  switch (f) {
    case format::json: s = guatiao_json_from_wire(bytes.raw(), how, default_alloc(), &out); break;
    case format::toml:
#if defined(GUATIAO_SERDE_TOML)
      s = guatiao_toml_from_wire(bytes.raw(), how, default_alloc(), &out);
      break;
#else
      return detail::unsupported(f);
#endif
    case format::yaml:
#if defined(GUATIAO_SERDE_YAML)
      s = guatiao_yaml_from_wire(bytes.raw(), how, default_alloc(), &out);
      break;
#else
      return detail::unsupported(f);
#endif
  }
  return guatiao::detail::take_text(s, out);
}

#if GUATIAO_EXCEPTIONS
inline value parse(format f, std::string_view text, std::uint32_t how = 0,
                   const guatiao_alloc* alloc = default_alloc()) {
  return try_parse(f, text, how, alloc).value();
}
inline std::string emit(format f, view v, std::uint32_t how = 0) { return try_emit(f, v, how).value(); }
inline std::vector<std::uint8_t> to_wire(format f, std::string_view text, std::uint32_t how = 0) {
  return try_to_wire(f, text, how).value();
}
inline std::string from_wire(format f, bytes_view bytes, std::uint32_t how = 0) {
  return try_from_wire(f, bytes, how).value();
}
#endif

}  // namespace serde
}  // namespace GUATIAO_ABI
}  // namespace guatiao

#endif  // GUATIAO_SERDE_HPP
