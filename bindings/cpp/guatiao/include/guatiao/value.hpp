// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// guatiao for C++: statuses, errors, results, allocators and values.
// Part of <guatiao/guatiao.hpp>; every type here is a thin owner or view
// over the structs of <guatiao.h>, and every operation calls its C
// function.

#ifndef GUATIAO_VALUE_HPP
#define GUATIAO_VALUE_HPP

#include <guatiao.h>

#include <charconv>
#include <cstddef>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <exception>
#include <initializer_list>
#include <iterator>
#include <limits>
#include <memory>
#include <new>
#include <optional>
#include <string>
#include <string_view>
#include <type_traits>
#include <utility>
#include <variant>
#include <vector>

#if !defined(_WIN32)
#include <stdlib.h>  // posix_memalign
#endif

#if defined(GUATIAO_NO_EXCEPTIONS)
#define GUATIAO_EXCEPTIONS 0
#elif defined(__cpp_exceptions) || defined(__EXCEPTIONS) || defined(_CPPUNWIND)
#define GUATIAO_EXCEPTIONS 1
#else
#define GUATIAO_EXCEPTIONS 0
#endif

#define GUATIAO_CPP_VERSION "0.0.0-alpha.1"

namespace guatiao {

// ---- statuses ---------------------------------------------------------

/// A `guatiao_status`: the codes <guatiao.h> names, and any other a
/// library answers.
enum class status : std::uint32_t {
  ok = GUATIAO_OK,
  bad_value = GUATIAO_ERR_BAD_VALUE,
  alloc = GUATIAO_ERR_ALLOC,
  wrong_kind = GUATIAO_ERR_WRONG_KIND,
  not_found = GUATIAO_ERR_NOT_FOUND,
  null = GUATIAO_ERR_NULL,
  gone = GUATIAO_ERR_GONE,
  unsupported = GUATIAO_ERR_UNSUPPORTED,
  busy = GUATIAO_ERR_BUSY,
  internal = GUATIAO_ERR_INTERNAL,
};

/// The constant's name without its prefix (`"bad value"`), or
/// `"status"` for a code <guatiao.h> does not name.
constexpr const char* status_name(status s) noexcept {
  switch (s) {
    case status::ok: return "ok";
    case status::bad_value: return "bad value";
    case status::alloc: return "allocation failed";
    case status::wrong_kind: return "wrong kind";
    case status::not_found: return "not found";
    case status::null: return "null argument";
    case status::gone: return "gone";
    case status::unsupported: return "unsupported";
    case status::busy: return "busy";
    case status::internal: return "internal";
  }
  return "status";
}

/// The kind of a stored value: `guatiao_value.tag`.
enum class tag : std::uint32_t {
  absent = GUATIAO_ABSENT,
  null = GUATIAO_NULL,
  boolean = GUATIAO_BOOL,
  number = GUATIAO_NUMBER,
  string = GUATIAO_STRING,
  bytes = GUATIAO_BYTES,
  list = GUATIAO_LIST,
  map = GUATIAO_MAP,
};

class view;
class value;
class node;
class map;
class list;

namespace detail {
template <class I>
using if_index = std::enable_if_t<std::is_integral_v<I> && !std::is_same_v<I, bool>, int>;
}  // namespace detail

// ---- errors -------------------------------------------------------------

/// A failed call: its status, the other side's words (empty when it gave
/// none), and, for the calls that say more, a map of detail (see each
/// call). Thrown by the throwing forms; carried by a `result`.
class error : public std::exception {
 public:
  error() noexcept = default;
  explicit error(status code, std::string message = {}) : code_(code), message_(std::move(message)) {}
  /// Takes `detail`, a tree the call built, as this error's detail.
  error(status code, std::string message, guatiao_value detail);

  status code() const noexcept { return code_; }
  const std::string& message() const noexcept { return message_; }
  /// The detail map, or an absent view.
  view detail() const noexcept;
  const char* what() const noexcept override {
    return message_.empty() ? status_name(code_) : message_.c_str();
  }

 private:
  status code_ = status::internal;
  std::string message_;
  std::shared_ptr<guatiao_value> detail_;
};

namespace detail {

/// Throws `e`, or prints it and aborts where exceptions are off.
[[noreturn]] inline void fail(const error& e) {
#if GUATIAO_EXCEPTIONS
  throw e;
#else
  std::fprintf(stderr, "guatiao: %s\n", e.what());
  std::abort();
#endif
}

}  // namespace detail

/// What a `try_` call answers: its value, or the `error` it failed with.
/// `value()` throws that error (aborts where exceptions are off).
template <class T>
class [[nodiscard]] result {
  static_assert(!std::is_reference_v<T>, "a result holds a value, not a reference");

 public:
  result(T v) : v_(std::in_place_index<0>, std::move(v)) {}
  result(::guatiao::error e) : v_(std::in_place_index<1>, std::move(e)) {}

  bool ok() const noexcept { return v_.index() == 0; }
  explicit operator bool() const noexcept { return ok(); }
  /// `status::ok`, or the error's status.
  status code() const noexcept { return ok() ? status::ok : std::get_if<1>(&v_)->code(); }
  /// The error; only after `!ok()`.
  const ::guatiao::error& error() const noexcept { return *std::get_if<1>(&v_); }

  T& value() & {
    if (!ok()) detail::fail(error());
    return *std::get_if<0>(&v_);
  }
  const T& value() const& {
    if (!ok()) detail::fail(error());
    return *std::get_if<0>(&v_);
  }
  T&& value() && {
    if (!ok()) detail::fail(error());
    return std::move(*std::get_if<0>(&v_));
  }
  T value_or(T fallback) && { return ok() ? std::move(*std::get_if<0>(&v_)) : std::move(fallback); }

  /// Unchecked: only after `ok()`.
  T& operator*() & noexcept { return *std::get_if<0>(&v_); }
  const T& operator*() const& noexcept { return *std::get_if<0>(&v_); }
  T&& operator*() && noexcept { return std::move(*std::get_if<0>(&v_)); }
  T* operator->() noexcept { return std::get_if<0>(&v_); }
  const T* operator->() const noexcept { return std::get_if<0>(&v_); }

 private:
  std::variant<T, ::guatiao::error> v_;
};

/// A call that answers nothing but whether it happened.
template <>
class [[nodiscard]] result<void> {
 public:
  result() noexcept = default;
  result(::guatiao::error e) : e_(std::move(e)) {}

  bool ok() const noexcept { return !e_.has_value(); }
  explicit operator bool() const noexcept { return ok(); }
  status code() const noexcept { return ok() ? status::ok : e_->code(); }
  const ::guatiao::error& error() const noexcept { return *e_; }
  void value() const {
    if (!ok()) detail::fail(*e_);
  }

 private:
  std::optional<::guatiao::error> e_;
};

namespace detail {

inline ::guatiao::error error_of(guatiao_status s, std::string message = {}) {
  return ::guatiao::error(static_cast<status>(s), std::move(message));
}

inline result<void> check(guatiao_status s) {
  if (s == GUATIAO_OK) return {};
  return error_of(s);
}

}  // namespace detail

// ---- allocators ----------------------------------------------------------

namespace detail {

constexpr std::size_t plain_align = alignof(std::max_align_t);

inline void* system_alloc_fn(void*, std::size_t size, std::size_t align) {
  if (size == 0) return nullptr;
  if (align <= plain_align) return std::malloc(size);
#if defined(_WIN32)
  return _aligned_malloc(size, align);
#else
  void* p = nullptr;
  return posix_memalign(&p, align < sizeof(void*) ? sizeof(void*) : align, size) == 0 ? p : nullptr;
#endif
}

inline void system_free_fn(void*, void* p, std::size_t, std::size_t align) {
  if (!p) return;
#if defined(_WIN32)
  if (align > plain_align) {
    _aligned_free(p);
    return;
  }
#else
  (void)align;
#endif
  std::free(p);
}

}  // namespace detail

/// An allocator over the C library's heap: one per binary that includes
/// this header, alive for the process.
inline const guatiao_alloc* system_alloc() noexcept {
  static const guatiao_alloc a = {
      static_cast<std::uint32_t>(sizeof(guatiao_alloc)), nullptr, &detail::system_alloc_fn,
      &detail::system_free_fn, nullptr};
  return &a;
}

/// The allocator a call builds in when it is not given one, and the one
/// this binding's own scratch trees use: `system_alloc()`, unless the
/// translation unit defines `GUATIAO_CPP_DEFAULT_ALLOC` as the name of a
/// function answering another before including this header (what a test
/// counting every block does; define it the same way in every file of a
/// program, or not at all).
inline const guatiao_alloc* default_alloc() noexcept {
#if defined(GUATIAO_CPP_DEFAULT_ALLOC)
  return GUATIAO_CPP_DEFAULT_ALLOC();
#else
  return system_alloc();
#endif
}

// ---- bytes --------------------------------------------------------------

/// Borrowed bytes: what a BYTES value holds, and what a call reads.
class bytes_view {
 public:
  constexpr bytes_view() noexcept = default;
  constexpr bytes_view(const std::uint8_t* data, std::size_t size) noexcept : data_(data), size_(size) {}
  bytes_view(const std::vector<std::uint8_t>& v) noexcept : data_(v.data()), size_(v.size()) {}
  /// The bytes of a text, as they are.
  static bytes_view of(std::string_view s) noexcept {
    return {reinterpret_cast<const std::uint8_t*>(s.data()), s.size()};
  }

  constexpr const std::uint8_t* data() const noexcept { return data_; }
  constexpr std::size_t size() const noexcept { return size_; }
  constexpr bool empty() const noexcept { return size_ == 0; }
  constexpr const std::uint8_t* begin() const noexcept { return data_; }
  constexpr const std::uint8_t* end() const noexcept { return data_ + size_; }
  constexpr std::uint8_t operator[](std::size_t i) const noexcept { return data_[i]; }
  std::vector<std::uint8_t> to_vector() const { return {begin(), end()}; }
  friend bool operator==(bytes_view a, bytes_view b) noexcept {
    return a.size_ == b.size_ && (a.size_ == 0 || std::memcmp(a.data_, b.data_, a.size_) == 0);
  }
  friend bool operator!=(bytes_view a, bytes_view b) noexcept { return !(a == b); }

  guatiao_bytes raw() const noexcept { return guatiao_bytes{size_ ? data_ : nullptr, size_}; }

 private:
  const std::uint8_t* data_ = nullptr;
  std::size_t size_ = 0;
};

/// Borrowed writable bytes: the caller's memory, written in place for one
/// call (an object kind's `&mut [u8]`).
class bytes_mut {
 public:
  constexpr bytes_mut() noexcept = default;
  constexpr bytes_mut(std::uint8_t* data, std::size_t size) noexcept : data_(data), size_(size) {}
  bytes_mut(std::vector<std::uint8_t>& v) noexcept : data_(v.data()), size_(v.size()) {}
  template <std::size_t N>
  constexpr bytes_mut(std::uint8_t (&a)[N]) noexcept : data_(a), size_(N) {}

  constexpr std::uint8_t* data() const noexcept { return data_; }
  constexpr std::size_t size() const noexcept { return size_; }
  constexpr std::uint8_t* begin() const noexcept { return data_; }
  constexpr std::uint8_t* end() const noexcept { return data_ + size_; }
  constexpr std::uint8_t& operator[](std::size_t i) const noexcept { return data_[i]; }

  guatiao_bytes_mut raw() const noexcept { return guatiao_bytes_mut{size_ ? data_ : nullptr, size_}; }

 private:
  std::uint8_t* data_ = nullptr;
  std::size_t size_ = 0;
};

namespace detail {

inline guatiao_str str(std::string_view s) noexcept {
  guatiao_str out;
  out.ptr = reinterpret_cast<const std::uint8_t*>(s.data());
  out.len = s.size();
  return out;
}

inline std::string_view text(guatiao_str s) noexcept {
  return s.len ? std::string_view(reinterpret_cast<const char*>(s.ptr), s.len) : std::string_view();
}

inline std::string_view text(const guatiao_string& s) noexcept {
  return s.len ? std::string_view(reinterpret_cast<const char*>(s.ptr), s.len) : std::string_view();
}

/// Whether `s` is UTF-8: what every text crossing into guatiao must be.
inline bool is_utf8(std::string_view s) noexcept {
  const auto* p = reinterpret_cast<const unsigned char*>(s.data());
  std::size_t n = s.size(), i = 0;
  while (i < n) {
    unsigned c = p[i];
    if (c < 0x80) {
      ++i;
      continue;
    }
    std::size_t len;
    unsigned min;
    if ((c & 0xE0) == 0xC0) {
      len = 2, min = 0x80, c &= 0x1F;
    } else if ((c & 0xF0) == 0xE0) {
      len = 3, min = 0x800, c &= 0x0F;
    } else if ((c & 0xF8) == 0xF0) {
      len = 4, min = 0x10000, c &= 0x07;
    } else {
      return false;
    }
    if (n - i < len) return false;
    for (std::size_t k = 1; k < len; ++k) {
      unsigned d = p[i + k];
      if ((d & 0xC0) != 0x80) return false;
      c = (c << 6) | (d & 0x3F);
    }
    if (c < min || c > 0x10FFFF || (c >= 0xD800 && c <= 0xDFFF)) return false;
    i += len;
  }
  return true;
}

/// A guatiao_string's bytes released through the allocator it records.
inline void release_string(guatiao_string& s) noexcept {
  if (s.cap && s.alloc && s.alloc->free) s.alloc->free(s.alloc->ctx, s.ptr, s.cap, 1);
  s = guatiao_string{};
}

/// An owned copy of `text` in `a`, as a guatiao_string; false when `a`
/// refuses.
inline bool make_string(const guatiao_alloc* a, std::string_view text, guatiao_string& out) noexcept {
  out = guatiao_string{};
  out.alloc = a;
  if (text.empty()) return true;
  void* p = a->alloc(a->ctx, text.size(), 1);
  if (!p) return false;
  std::memcpy(p, text.data(), text.size());
  out.ptr = static_cast<std::uint8_t*>(p);
  out.len = out.cap = text.size();
  return true;
}

inline guatiao_value blank() noexcept {
  guatiao_value v;
  std::memset(&v, 0, sizeof v);
  return v;
}

/// The allocator a container node records: its own buffer's, or null.
inline const guatiao_alloc* recorded(const guatiao_value& v) noexcept {
  switch (v.tag) {
    case GUATIAO_STRING: return v.payload.text.alloc;
    case GUATIAO_NUMBER: return v.payload.number.alloc;
    case GUATIAO_BYTES: return v.payload.bytes.alloc;
    case GUATIAO_LIST: return v.payload.list.alloc;
    case GUATIAO_MAP: return v.payload.map.alloc;
    default: return nullptr;
  }
}

/// A number's exact text read as an integer of type `I`, refusing a
/// fraction, an exponent, and anything out of range: never truncated.
template <class I>
std::optional<I> integer_of(std::string_view t) noexcept {
  if (t.empty()) return std::nullopt;
  for (char c : t)
    if (c == '.' || c == 'e' || c == 'E') return std::nullopt;
  I out{};
  auto r = std::from_chars(t.data(), t.data() + t.size(), out);
  if (r.ec != std::errc() || r.ptr != t.data() + t.size()) return std::nullopt;
  return out;
}

inline std::optional<double> double_of(std::string_view t) noexcept {
  if (t.empty()) return std::nullopt;
#if defined(__cpp_lib_to_chars) && __cpp_lib_to_chars >= 201611L
  double out = 0;
  auto r = std::from_chars(t.data(), t.data() + t.size(), out);
  if (r.ec != std::errc() || r.ptr != t.data() + t.size()) return std::nullopt;
  return out;
#else
  char buf[344];
  if (t.size() >= sizeof buf) return std::nullopt;
  std::memcpy(buf, t.data(), t.size());
  buf[t.size()] = '\0';
  char* end = nullptr;
  double out = std::strtod(buf, &end);
  if (end != buf + t.size()) return std::nullopt;
  return out;
#endif
}

/// The shortest text that reads back as `d`; empty for a NaN or an
/// infinity, which no number spells.
inline std::string text_of(double d) {
  if (d != d || d > std::numeric_limits<double>::max() || d < -std::numeric_limits<double>::max())
    return {};
  char buf[64];
#if defined(__cpp_lib_to_chars) && __cpp_lib_to_chars >= 201611L
  auto r = std::to_chars(buf, buf + sizeof buf, d);
  std::string s(buf, r.ptr);
#else
  std::snprintf(buf, sizeof buf, "%.17g", d);
  std::string s(buf);
#endif
  // JSON has no `inf`/`nan` and needs a digit before an exponent; to_chars
  // writes neither for a finite double.
  return s;
}

template <class I>
std::string text_of_integer(I i) {
  char buf[32];
  auto r = std::to_chars(buf, buf + sizeof buf, i);
  return std::string(buf, r.ptr);
}

}  // namespace detail

// ---- reading ------------------------------------------------------------

class map_view;
class list_view;

/// The read operations every value handle shares, over the node
/// `Derived::address()` answers (null reads as absent).
template <class Derived>
class reader {
 public:
  ::guatiao::tag tag() const noexcept {
    const guatiao_value* n = self();
    return n ? static_cast<::guatiao::tag>(n->tag) : ::guatiao::tag::absent;
  }
  bool is_absent() const noexcept { return tag() == ::guatiao::tag::absent; }
  bool is_null() const noexcept { return tag() == ::guatiao::tag::null; }
  bool is_bool() const noexcept { return tag() == ::guatiao::tag::boolean; }
  bool is_number() const noexcept { return tag() == ::guatiao::tag::number; }
  bool is_string() const noexcept { return tag() == ::guatiao::tag::string; }
  bool is_bytes() const noexcept { return tag() == ::guatiao::tag::bytes; }
  bool is_list() const noexcept { return tag() == ::guatiao::tag::list; }
  bool is_map() const noexcept { return tag() == ::guatiao::tag::map; }

  /// The boolean, or nothing for another kind.
  std::optional<bool> boolean() const noexcept {
    if (!is_bool()) return std::nullopt;
    return self()->payload.b;
  }
  /// A STRING's text, or nothing for another kind (a NUMBER included).
  std::optional<std::string_view> string() const noexcept {
    if (!is_string()) return std::nullopt;
    return detail::text(self()->payload.text);
  }
  /// A NUMBER's exact text, or nothing for another kind.
  std::optional<std::string_view> number_text() const noexcept {
    if (!is_number()) return std::nullopt;
    return detail::text(self()->payload.number);
  }
  /// A BYTES value's bytes, or nothing for another kind.
  std::optional<bytes_view> bytes() const noexcept {
    if (!is_bytes()) return std::nullopt;
    const guatiao_buffer& b = self()->payload.bytes;
    return bytes_view(b.len ? b.ptr : nullptr, b.len);
  }

  /// This value as `T`, or nothing when it is another kind or does not
  /// fit: `bool`, every integer type (from a number's text, exactly:
  /// never truncated or rounded), `float` and `double` (the nearest),
  /// `std::string_view` and `std::string` (a STRING), `bytes_view`.
  template <class T>
  std::optional<T> as() const {
    if constexpr (std::is_same_v<T, bool>) {
      return boolean();
    } else if constexpr (std::is_integral_v<T>) {
      auto t = number_text();
      if (!t) return std::nullopt;
      return detail::integer_of<T>(*t);
    } else if constexpr (std::is_floating_point_v<T>) {
      auto t = number_text();
      if (!t) return std::nullopt;
      auto d = detail::double_of(*t);
      if (!d) return std::nullopt;
      return static_cast<T>(*d);
    } else if constexpr (std::is_same_v<T, std::string_view>) {
      return string();
    } else if constexpr (std::is_same_v<T, std::string>) {
      auto s = string();
      if (!s) return std::nullopt;
      return std::string(*s);
    } else if constexpr (std::is_same_v<T, bytes_view>) {
      return bytes();
    } else {
      static_assert(sizeof(T) == 0, "guatiao: as<T> reads bool, integers, floating point, text and bytes");
    }
  }

  bool bool_or(bool fallback) const noexcept { return boolean().value_or(fallback); }
  std::int64_t int_or(std::int64_t fallback) const noexcept {
    return as<std::int64_t>().value_or(fallback);
  }
  double float_or(double fallback) const { return as<double>().value_or(fallback); }
  std::string_view string_or(std::string_view fallback) const noexcept {
    return string().value_or(fallback);
  }

  /// The map this value is; an empty view for another kind (`is_map()`
  /// tells the two apart). A view, so `for (auto e : v.as_map())` is safe.
  map_view as_map() const noexcept;
  /// The list this value is; an empty view for another kind.
  list_view as_list() const noexcept;

  /// The value under `key` in a map; absent when there is none, or this is
  /// not a map. Keys compare as bytes.
  view find(std::string_view key) const noexcept;
  bool contains(std::string_view key) const noexcept;
  /// The element at `index` of a list; absent past the end, or when this
  /// is not a list.
  view at(std::size_t index) const noexcept;
  view operator[](std::string_view key) const noexcept;
  view operator[](const char* key) const noexcept;
  template <class I, detail::if_index<I> = 0>
  view operator[](I index) const noexcept;
  /// How many entries a map holds, or elements a list; 0 for anything
  /// else.
  std::size_t size() const noexcept {
    const guatiao_value* n = self();
    if (!n) return 0;
    if (n->tag == GUATIAO_MAP) return n->payload.map.len;
    if (n->tag == GUATIAO_LIST) return n->payload.list.len;
    return 0;
  }

  /// A deep copy, built in `alloc`.
  result<value> try_clone(const guatiao_alloc* alloc = default_alloc()) const;
#if GUATIAO_EXCEPTIONS
  value clone(const guatiao_alloc* alloc = default_alloc()) const;
#endif

 private:
  const guatiao_value* self() const noexcept { return static_cast<const Derived*>(this)->address(); }
};

/// A borrowed, read-only node: valid while what it points into is, and
/// until a write moves the array it sits in. A null pointer is ABSENT.
class view : public reader<view> {
 public:
  constexpr view() noexcept = default;
  constexpr explicit view(const guatiao_value* node) noexcept : node_(node) {}
  /// The node's address, or null for an absent view over nothing.
  constexpr const guatiao_value* address() const noexcept { return node_; }
  constexpr const guatiao_value* raw() const noexcept { return node_; }

 private:
  const guatiao_value* node_ = nullptr;
};

/// Deep equality: the same kind, the same text, bytes and booleans, maps
/// holding the same pairs IN THE SAME ORDER. Numbers compare as text.
bool operator==(view a, view b);
inline bool operator!=(view a, view b) { return !(a == b); }

/// One map entry, read-only.
struct entry {
  std::string_view key;
  view value;
};

/// A borrowed map: a `const guatiao_map *`, entries in insertion order.
class map_view {
 public:
  class iterator {
   public:
    using iterator_category = std::random_access_iterator_tag;
    using value_type = entry;
    using difference_type = std::ptrdiff_t;
    using pointer = void;
    using reference = entry;
    iterator() noexcept = default;
    explicit iterator(const guatiao_entry* p) noexcept : p_(p) {}
    entry operator*() const noexcept { return entry{detail::text(p_->key), view(&p_->value)}; }
    iterator& operator++() noexcept { return ++p_, *this; }
    iterator operator++(int) noexcept { return iterator(p_++); }
    iterator& operator--() noexcept { return --p_, *this; }
    iterator& operator+=(difference_type d) noexcept { return p_ += d, *this; }
    friend iterator operator+(iterator i, difference_type d) noexcept { return i += d; }
    friend difference_type operator-(iterator a, iterator b) noexcept { return a.p_ - b.p_; }
    entry operator[](difference_type d) const noexcept { return *(*this + d); }
    friend bool operator==(iterator a, iterator b) noexcept { return a.p_ == b.p_; }
    friend bool operator!=(iterator a, iterator b) noexcept { return a.p_ != b.p_; }
    friend bool operator<(iterator a, iterator b) noexcept { return a.p_ < b.p_; }

   private:
    const guatiao_entry* p_ = nullptr;
  };

  constexpr map_view() noexcept = default;
  constexpr explicit map_view(const guatiao_map* m) noexcept : m_(m) {}
  const guatiao_map* raw() const noexcept { return m_; }

  std::size_t size() const noexcept { return m_ ? m_->len : 0; }
  bool empty() const noexcept { return size() == 0; }
  iterator begin() const noexcept { return iterator(size() ? m_->ptr : nullptr); }
  iterator end() const noexcept { return iterator(size() ? m_->ptr + m_->len : nullptr); }
  /// The value under `key`, absent when there is none.
  view find(std::string_view key) const noexcept {
    for (std::size_t i = 0; i < size(); ++i)
      if (detail::text(m_->ptr[i].key) == key) return view(&m_->ptr[i].value);
    return view();
  }
  bool contains(std::string_view key) const noexcept { return !find(key).is_absent(); }
  view operator[](std::string_view key) const noexcept { return find(key); }
  /// The keys, in order.
  std::vector<std::string_view> keys() const {
    std::vector<std::string_view> out;
    out.reserve(size());
    for (entry e : *this) out.push_back(e.key);
    return out;
  }

 private:
  const guatiao_map* m_ = nullptr;
};

/// A borrowed list: a `const guatiao_list *`.
class list_view {
 public:
  class iterator {
   public:
    using iterator_category = std::random_access_iterator_tag;
    using value_type = view;
    using difference_type = std::ptrdiff_t;
    using pointer = void;
    using reference = view;
    iterator() noexcept = default;
    explicit iterator(const guatiao_value* p) noexcept : p_(p) {}
    view operator*() const noexcept { return view(p_); }
    iterator& operator++() noexcept { return ++p_, *this; }
    iterator operator++(int) noexcept { return iterator(p_++); }
    iterator& operator--() noexcept { return --p_, *this; }
    iterator& operator+=(difference_type d) noexcept { return p_ += d, *this; }
    friend iterator operator+(iterator i, difference_type d) noexcept { return i += d; }
    friend difference_type operator-(iterator a, iterator b) noexcept { return a.p_ - b.p_; }
    view operator[](difference_type d) const noexcept { return view(p_ + d); }
    friend bool operator==(iterator a, iterator b) noexcept { return a.p_ == b.p_; }
    friend bool operator!=(iterator a, iterator b) noexcept { return a.p_ != b.p_; }
    friend bool operator<(iterator a, iterator b) noexcept { return a.p_ < b.p_; }

   private:
    const guatiao_value* p_ = nullptr;
  };

  constexpr list_view() noexcept = default;
  constexpr explicit list_view(const guatiao_list* l) noexcept : l_(l) {}
  const guatiao_list* raw() const noexcept { return l_; }

  std::size_t size() const noexcept { return l_ ? l_->len : 0; }
  bool empty() const noexcept { return size() == 0; }
  iterator begin() const noexcept { return iterator(size() ? l_->ptr : nullptr); }
  iterator end() const noexcept { return iterator(size() ? l_->ptr + l_->len : nullptr); }
  /// The element at `index`, absent past the end.
  view at(std::size_t index) const noexcept { return index < size() ? view(&l_->ptr[index]) : view(); }
  view operator[](std::size_t index) const noexcept { return at(index); }

 private:
  const guatiao_list* l_ = nullptr;
};

template <class D>
map_view reader<D>::as_map() const noexcept {
  if (!is_map()) return map_view();
  return map_view(&self()->payload.map);
}

template <class D>
list_view reader<D>::as_list() const noexcept {
  if (!is_list()) return list_view();
  return list_view(&self()->payload.list);
}

template <class D>
view reader<D>::find(std::string_view key) const noexcept {
  return as_map().find(key);
}

template <class D>
bool reader<D>::contains(std::string_view key) const noexcept {
  return !find(key).is_absent();
}

template <class D>
view reader<D>::at(std::size_t index) const noexcept {
  return as_list().at(index);
}

template <class D>
view reader<D>::operator[](std::string_view key) const noexcept {
  return find(key);
}

template <class D>
view reader<D>::operator[](const char* key) const noexcept {
  return find(key ? std::string_view(key) : std::string_view());
}

template <class D>
template <class I, detail::if_index<I>>
view reader<D>::operator[](I index) const noexcept {
  if constexpr (std::is_signed_v<I>) {
    if (index < 0) return view();
  }
  return at(static_cast<std::size_t>(index));
}

inline view error::detail() const noexcept { return view(detail_.get()); }

// ---- building -------------------------------------------------------------

/// A description of a tree to build: a scalar, a text, a list or a map of
/// further items, or a copy of a tree already built. `make` builds one in
/// an allocator; every setter and pusher takes one.
///
/// ```cpp
/// auto v = guatiao::make(guatiao::item::map({{"host", "10.0.0.1"}, {"port", 5900},
///                                            {"tags", guatiao::item::list({"a", "b"})}}));
/// ```
class item {
 public:
  struct number_text {
    std::string text;
  };
  struct list_items {
    std::vector<item> items;
  };
  struct map_items {
    std::vector<std::pair<std::string, item>> items;
  };
  struct copy_of {
    const guatiao_value* node;
  };
  struct absent_marker {};

  /// Null.
  item() noexcept : v_(std::monostate{}) {}
  item(std::nullptr_t) noexcept : v_(std::monostate{}) {}
  item(bool b) noexcept : v_(b) {}
  template <class I, std::enable_if_t<std::is_integral_v<I> && !std::is_same_v<I, bool> &&
                                          !std::is_same_v<I, char> && std::is_signed_v<I>,
                                      int> = 0>
  item(I i) noexcept : v_(static_cast<std::int64_t>(i)) {}
  template <class I, std::enable_if_t<std::is_integral_v<I> && !std::is_same_v<I, bool> &&
                                          !std::is_same_v<I, char> && std::is_unsigned_v<I>,
                                      int> = 0>
  item(I i) noexcept : v_(static_cast<std::uint64_t>(i)) {}
  item(char) = delete;
  /// A finite double, as the shortest text reading back as it; a NaN or an
  /// infinity is refused when built.
  item(double d) noexcept : v_(d) {}
  item(float f) noexcept : v_(static_cast<double>(f)) {}
  item(const char* s) : v_(std::string(s ? s : "")) {}
  item(std::string_view s) : v_(std::string(s)) {}
  item(const std::string& s) : v_(s) {}
  item(std::string&& s) noexcept : v_(std::move(s)) {}
  item(bytes_view b) : v_(b.to_vector()) {}
  /// A deep copy of the tree `v` reads; `v` must outlive the build.
  item(view v) noexcept : v_(copy_of{v.raw()}) {}
  item(const value& v) noexcept;

  /// A number given as its text, stored verbatim (the JSON grammar, or
  /// refused when built).
  static item number(std::string_view text) { return item(number_text{std::string(text)}); }
  static item bytes(bytes_view b) { return item(b); }
  static item list(std::initializer_list<item> items) { return item(list_items{std::vector<item>(items)}); }
  static item list_from(std::vector<item> items) { return item(list_items{std::move(items)}); }
  static item map(std::initializer_list<std::pair<std::string_view, item>> items) {
    map_items m;
    m.items.reserve(items.size());
    for (const auto& kv : items) m.items.emplace_back(std::string(kv.first), kv.second);
    return item(std::move(m));
  }
  static item map_from(std::vector<std::pair<std::string, item>> items) { return item(map_items{std::move(items)}); }
  /// The marker for nothing: what a lookup answers when it found nothing.
  static item absent() { return item(absent_marker{}); }

  /// Builds the tree into `out`, which holds nothing on entry and nothing
  /// after a failure.
  guatiao_status build(const guatiao_alloc* a, guatiao_value* out) const;

 private:
  using storage = std::variant<std::monostate, bool, std::int64_t, std::uint64_t, double, std::string,
                               number_text, std::vector<std::uint8_t>, list_items, map_items, copy_of,
                               absent_marker>;
  explicit item(number_text t) : v_(std::move(t)) {}
  explicit item(list_items t) : v_(std::move(t)) {}
  explicit item(map_items t) : v_(std::move(t)) {}
  explicit item(absent_marker t) : v_(t) {}
  storage v_;
};

/// The owner of one tree: a root node, freed when this is destroyed.
/// Move-only; `clone()` copies. A moved-from value is ABSENT.
class value : public reader<value> {
 public:
  /// ABSENT.
  value() noexcept : node_(detail::blank()) {}
  value(value&& o) noexcept : node_(o.node_) { o.node_ = detail::blank(); }
  value& operator=(value&& o) noexcept {
    if (this != &o) {
      guatiao_value_free(&node_);
      node_ = o.node_;
      o.node_ = detail::blank();
    }
    return *this;
  }
  value(const value&) = delete;
  value& operator=(const value&) = delete;
  ~value() { guatiao_value_free(&node_); }

  /// Takes over a root node a call wrote: this value frees it.
  static value adopt(const guatiao_value& raw) noexcept {
    value v;
    v.node_ = raw;
    return v;
  }
  /// Gives the root node up: the caller frees it (or hands it over), and
  /// this value is ABSENT.
  guatiao_value release() noexcept {
    guatiao_value out = node_;
    node_ = detail::blank();
    return out;
  }

  static value null() noexcept {
    value v;
    v.node_.tag = GUATIAO_NULL;
    return v;
  }

  const guatiao_value* address() const noexcept { return &node_; }
  guatiao_value* raw() noexcept { return &node_; }
  const guatiao_value* raw() const noexcept { return &node_; }
  view get() const noexcept { return view(&node_); }
  operator view() const noexcept { return get(); }

  using reader<value>::operator[];
  /// The writable node under `key` (a map) or at `index` (a list); a null
  /// node when there is none.
  node operator[](std::string_view key) noexcept;
  node operator[](const char* key) noexcept;
  template <class I, detail::if_index<I> = 0>
  node operator[](I index) noexcept;
  /// The root as a writable node.
  node ref() noexcept;

  // Writes, as on `node`; the root's own allocator grows it.
  result<void> try_set(std::string_view key, value&& v);
  result<void> try_set(std::string_view key, const item& x);
  result<bool> try_erase(std::string_view key);
  result<void> try_push(value&& v);
  result<void> try_push(const item& x);
  result<void> try_erase_at(std::size_t index);
  result<void> try_clear();
#if GUATIAO_EXCEPTIONS
  value& set(std::string_view key, value&& v) { return try_set(key, std::move(v)).value(), *this; }
  value& set(std::string_view key, const item& x) { return try_set(key, x).value(), *this; }
  bool erase(std::string_view key) { return try_erase(key).value(); }
  value& push(value&& v) { return try_push(std::move(v)).value(), *this; }
  value& push(const item& x) { return try_push(x).value(), *this; }
  void erase_at(std::size_t index) { try_erase_at(index).value(); }
  void clear() { try_clear().value(); }
#endif

 private:
  guatiao_value node_;
};

inline item::item(const value& v) noexcept : v_(copy_of{v.raw()}) {}

/// Builds `x` in `alloc`.
inline result<value> try_make(const item& x, const guatiao_alloc* alloc = default_alloc()) {
  guatiao_value out = detail::blank();
  guatiao_status s = x.build(alloc, &out);
  if (s != GUATIAO_OK) return detail::error_of(s, "the value could not be built");
  return value::adopt(out);
}
#if GUATIAO_EXCEPTIONS
inline value make(const item& x, const guatiao_alloc* alloc = default_alloc()) {
  return try_make(x, alloc).value();
}
#endif

/// A writable place inside a tree: a `guatiao_value *` and the allocator
/// to grow it through when the node records none. Like an iterator, it is
/// valid until the container holding it changes (a set or a push on its
/// parent may move the array it sits in). Its writes go through the
/// allocator the node records, so a tree it grows stays in one allocator.
class node : public reader<node> {
 public:
  constexpr node() noexcept = default;
  constexpr node(guatiao_value* n, const guatiao_alloc* fallback = nullptr) noexcept : n_(n), fallback_(fallback) {}

  const guatiao_value* address() const noexcept { return n_; }
  guatiao_value* raw() const noexcept { return n_; }
  view get() const noexcept { return view(n_); }
  operator view() const noexcept { return get(); }
  explicit operator bool() const noexcept { return n_ != nullptr; }

  /// The allocator a write here builds through: the one the node records,
  /// else the one this node was given, else `default_alloc()`.
  const guatiao_alloc* alloc() const noexcept {
    const guatiao_alloc* a = n_ ? detail::recorded(*n_) : nullptr;
    if (a) return a;
    return fallback_ ? fallback_ : default_alloc();
  }

  using reader<node>::operator[];
  node child(std::string_view key) const noexcept {
    if (!n_ || n_->tag != GUATIAO_MAP) return {};
    guatiao_map& m = n_->payload.map;
    for (std::size_t i = 0; i < m.len; ++i)
      if (detail::text(m.ptr[i].key) == key) return {&m.ptr[i].value, alloc()};
    return {};
  }
  node child(std::size_t index) const noexcept {
    if (!n_ || n_->tag != GUATIAO_LIST || index >= n_->payload.list.len) return {};
    return {&n_->payload.list.ptr[index], alloc()};
  }
  node operator[](std::string_view key) noexcept { return child(key); }
  node operator[](const char* key) noexcept { return child(key ? std::string_view(key) : std::string_view()); }
  template <class I, detail::if_index<I> = 0>
  node operator[](I index) noexcept {
    if constexpr (std::is_signed_v<I>) {
      if (index < 0) return {};
    }
    return child(static_cast<std::size_t>(index));
  }

  /// Stores `v` under `key` in a map, moving it (`v` is ABSENT after);
  /// replaces an entry in place. A refusal leaves `v` as it was.
  result<void> try_set(std::string_view key, value&& v) const {
    if (!n_) return detail::error_of(GUATIAO_ERR_NULL);
    guatiao_status s = guatiao_map_set(alloc(), n_, detail::str(key), v.raw());
    if (s != GUATIAO_OK) return detail::error_of(s);
    v = value();  // what the call left behind owns nothing
    return {};
  }
  /// Builds `x` through this node's allocator and stores it under `key`.
  result<void> try_set(std::string_view key, const item& x) const {
    if (!n_) return detail::error_of(GUATIAO_ERR_NULL);
    if (n_->tag != GUATIAO_MAP) return detail::error_of(GUATIAO_ERR_WRONG_KIND);
    guatiao_value built = detail::blank();
    guatiao_status s = x.build(alloc(), &built);
    if (s != GUATIAO_OK) return detail::error_of(s, "the value could not be built");
    value v = value::adopt(built);
    return try_set(key, std::move(v));
  }
  /// Removes `key` from a map: false when there was none.
  result<bool> try_erase(std::string_view key) const {
    if (!n_) return detail::error_of(GUATIAO_ERR_NULL);
    guatiao_status s = guatiao_map_discard(n_, detail::str(key));
    if (s == GUATIAO_ERR_NOT_FOUND) return false;
    if (s != GUATIAO_OK) return detail::error_of(s);
    return true;
  }
  /// Appends `v` to a list, moving it (`v` is ABSENT after). A refusal
  /// leaves `v` as it was.
  result<void> try_push(value&& v) const {
    if (!n_) return detail::error_of(GUATIAO_ERR_NULL);
    guatiao_status s = guatiao_list_push(alloc(), n_, v.raw());
    if (s != GUATIAO_OK) return detail::error_of(s);
    v = value();
    return {};
  }
  result<void> try_push(const item& x) const {
    if (!n_) return detail::error_of(GUATIAO_ERR_NULL);
    if (n_->tag != GUATIAO_LIST) return detail::error_of(GUATIAO_ERR_WRONG_KIND);
    guatiao_value built = detail::blank();
    guatiao_status s = x.build(alloc(), &built);
    if (s != GUATIAO_OK) return detail::error_of(s, "the value could not be built");
    value v = value::adopt(built);
    return try_push(std::move(v));
  }
  /// Removes and frees a list's element at `index`.
  result<void> try_erase_at(std::size_t index) const {
    if (!n_) return detail::error_of(GUATIAO_ERR_NULL);
    return detail::check(guatiao_list_discard(n_, index));
  }
  /// Empties a map or a list, keeping its capacity.
  result<void> try_clear() const {
    if (!n_) return detail::error_of(GUATIAO_ERR_NULL);
    if (n_->tag == GUATIAO_MAP) return detail::check(guatiao_map_clear(n_));
    if (n_->tag == GUATIAO_LIST) return detail::check(guatiao_list_clear(n_));
    return detail::error_of(GUATIAO_ERR_WRONG_KIND);
  }
  /// Copies every entry of the map `src` into this map, replacing
  /// collisions.
  result<void> try_update(view src) const {
    if (!n_) return detail::error_of(GUATIAO_ERR_NULL);
    return detail::check(guatiao_map_copy_from(alloc(), n_, src.raw()));
  }
  /// Appends text to a STRING.
  result<void> try_append(std::string_view text) const {
    if (!n_) return detail::error_of(GUATIAO_ERR_NULL);
    return detail::check(guatiao_string_push(alloc(), n_, detail::str(text)));
  }
  /// Appends bytes to a BYTES value.
  result<void> try_append(bytes_view bytes) const {
    if (!n_) return detail::error_of(GUATIAO_ERR_NULL);
    return detail::check(guatiao_buffer_push(alloc(), n_, bytes.raw()));
  }
  /// Replaces this node with `v`, freeing what it held.
  void assign(value&& v) const noexcept {
    if (!n_) return;
    guatiao_value_free(n_);
    *n_ = v.release();
  }
  /// Moves this node out, leaving NULL in its place (an ABSENT is never
  /// stored inside a container).
  value take() const noexcept {
    if (!n_) return value();
    value out = value::adopt(*n_);
    *n_ = detail::blank();
    n_->tag = GUATIAO_NULL;
    return out;
  }

#if GUATIAO_EXCEPTIONS
  const node& set(std::string_view key, value&& v) const { return try_set(key, std::move(v)).value(), *this; }
  const node& set(std::string_view key, const item& x) const { return try_set(key, x).value(), *this; }
  bool erase(std::string_view key) const { return try_erase(key).value(); }
  const node& push(value&& v) const { return try_push(std::move(v)).value(), *this; }
  const node& push(const item& x) const { return try_push(x).value(), *this; }
  void erase_at(std::size_t index) const { try_erase_at(index).value(); }
  void clear() const { try_clear().value(); }
  void update(view src) const { try_update(src).value(); }
  void append(std::string_view text) const { try_append(text).value(); }
  void append(bytes_view bytes) const { try_append(bytes).value(); }
#endif

 private:
  guatiao_value* n_ = nullptr;
  const guatiao_alloc* fallback_ = nullptr;
};

inline node value::ref() noexcept { return node(&node_); }
inline node value::operator[](std::string_view key) noexcept { return ref().child(key); }
inline node value::operator[](const char* key) noexcept { return ref()[key]; }
template <class I, detail::if_index<I>>
node value::operator[](I index) noexcept {
  return ref()[index];
}
inline result<void> value::try_set(std::string_view key, value&& v) { return ref().try_set(key, std::move(v)); }
inline result<void> value::try_set(std::string_view key, const item& x) { return ref().try_set(key, x); }
inline result<bool> value::try_erase(std::string_view key) { return ref().try_erase(key); }
inline result<void> value::try_push(value&& v) { return ref().try_push(std::move(v)); }
inline result<void> value::try_push(const item& x) { return ref().try_push(x); }
inline result<void> value::try_erase_at(std::size_t index) { return ref().try_erase_at(index); }
inline result<void> value::try_clear() { return ref().try_clear(); }

/// A writable map inside a tree: a node known to be a map. What a kind's
/// slot is LENT as a map arrives as one, valid for the call.
class map_ref : public reader<map_ref> {
 public:
  map_ref() noexcept = default;
  /// Over `n`, which must be a map node.
  explicit map_ref(node n) noexcept : n_(n) {}
  const guatiao_value* address() const noexcept { return n_.raw(); }
  node as_node() const noexcept { return n_; }
  map_view get() const noexcept { return map_view(n_.raw() ? &n_.raw()->payload.map : nullptr); }
  operator map_view() const noexcept { return get(); }
  operator view() const noexcept { return n_.get(); }
  const guatiao_alloc* alloc() const noexcept { return n_.alloc(); }
  map_view::iterator begin() const noexcept { return get().begin(); }
  map_view::iterator end() const noexcept { return get().end(); }
  std::vector<std::string_view> keys() const { return get().keys(); }
  using reader<map_ref>::operator[];
  node operator[](std::string_view key) noexcept { return n_.child(key); }
  node operator[](const char* key) noexcept { return n_[key]; }

  result<void> try_set(std::string_view key, value&& v) const { return n_.try_set(key, std::move(v)); }
  result<void> try_set(std::string_view key, const item& x) const { return n_.try_set(key, x); }
  result<bool> try_erase(std::string_view key) const { return n_.try_erase(key); }
  result<void> try_clear() const { return n_.try_clear(); }
  result<void> try_update(view src) const { return n_.try_update(src); }
#if GUATIAO_EXCEPTIONS
  const map_ref& set(std::string_view key, value&& v) const { return n_.set(key, std::move(v)), *this; }
  const map_ref& set(std::string_view key, const item& x) const { return n_.set(key, x), *this; }
  bool erase(std::string_view key) const { return n_.erase(key); }
  void clear() const { n_.clear(); }
  void update(view src) const { n_.update(src); }
#endif

 private:
  node n_;
};

/// A writable list inside a tree: a node known to be a list.
class list_ref : public reader<list_ref> {
 public:
  list_ref() noexcept = default;
  explicit list_ref(node n) noexcept : n_(n) {}
  const guatiao_value* address() const noexcept { return n_.raw(); }
  node as_node() const noexcept { return n_; }
  list_view get() const noexcept { return list_view(n_.raw() ? &n_.raw()->payload.list : nullptr); }
  operator list_view() const noexcept { return get(); }
  operator view() const noexcept { return n_.get(); }
  const guatiao_alloc* alloc() const noexcept { return n_.alloc(); }
  list_view::iterator begin() const noexcept { return get().begin(); }
  list_view::iterator end() const noexcept { return get().end(); }
  using reader<list_ref>::operator[];
  template <class I, detail::if_index<I> = 0>
  node operator[](I index) noexcept {
    return n_[index];
  }

  result<void> try_push(value&& v) const { return n_.try_push(std::move(v)); }
  result<void> try_push(const item& x) const { return n_.try_push(x); }
  result<void> try_erase_at(std::size_t index) const { return n_.try_erase_at(index); }
  result<void> try_clear() const { return n_.try_clear(); }
#if GUATIAO_EXCEPTIONS
  const list_ref& push(value&& v) const { return n_.push(std::move(v)), *this; }
  const list_ref& push(const item& x) const { return n_.push(x), *this; }
  void erase_at(std::size_t index) const { n_.erase_at(index); }
  void clear() const { n_.clear(); }
#endif

 private:
  node n_;
};

/// An owned map: a value that is always a map. What a kind's slot answers
/// as a map, or is handed over as one.
class map : public reader<map> {
 public:
  /// An empty map growing through `alloc`; it allocates nothing until an
  /// entry is set.
  explicit map(const guatiao_alloc* alloc = default_alloc()) noexcept {
    guatiao_value v = detail::blank();
    v.tag = GUATIAO_MAP;
    v.payload.map.alloc = alloc;
    v_ = value::adopt(v);
  }
  /// The map `x` describes, built in `alloc`; a non-map is WRONG_KIND.
  static result<map> try_of(const item& x, const guatiao_alloc* alloc = default_alloc()) {
    auto v = try_make(x, alloc);
    if (!v) return v.error();
    return from(std::move(*v));
  }
  static result<map> try_of(std::initializer_list<std::pair<std::string_view, item>> entries,
                            const guatiao_alloc* alloc = default_alloc()) {
    return try_of(item::map(entries), alloc);
  }
#if GUATIAO_EXCEPTIONS
  map(std::initializer_list<std::pair<std::string_view, item>> entries, const guatiao_alloc* alloc = default_alloc())
      : map(try_of(entries, alloc).value()) {}
#endif
  /// `v` as a map; anything else is WRONG_KIND and is freed.
  static result<map> from(value&& v) {
    if (!v.is_map()) return detail::error_of(GUATIAO_ERR_WRONG_KIND, "not a map");
    map m(nullptr);
    m.v_ = std::move(v);
    return m;
  }
  /// Takes over a map container a call wrote.
  static map adopt(const guatiao_map& raw) noexcept {
    map m(nullptr);
    guatiao_value v = detail::blank();
    v.tag = GUATIAO_MAP;
    v.payload.map = raw;
    m.v_ = value::adopt(v);
    return m;
  }
  /// Gives the container up, leaving this an empty map with no allocator.
  guatiao_map release() noexcept {
    guatiao_map out = v_.raw()->payload.map;
    v_.raw()->payload.map = guatiao_map{};
    return out;
  }

  const guatiao_value* address() const noexcept { return v_.raw(); }
  map_view get() const noexcept { return map_view(&v_.raw()->payload.map); }
  operator map_view() const noexcept { return get(); }
  operator view() const noexcept { return v_.get(); }
  map_ref ref() noexcept { return map_ref(v_.ref()); }
  /// The value, a map.
  value into_value() && noexcept { return std::move(v_); }
  operator value() && noexcept { return std::move(v_); }
  map_view::iterator begin() const noexcept { return get().begin(); }
  map_view::iterator end() const noexcept { return get().end(); }
  std::vector<std::string_view> keys() const { return get().keys(); }
  using reader<map>::operator[];
  node operator[](std::string_view key) noexcept { return v_[key]; }
  node operator[](const char* key) noexcept { return v_[key]; }

  result<void> try_set(std::string_view key, value&& v) { return v_.try_set(key, std::move(v)); }
  result<void> try_set(std::string_view key, const item& x) { return v_.try_set(key, x); }
  result<bool> try_erase(std::string_view key) { return v_.try_erase(key); }
  result<void> try_clear() { return v_.try_clear(); }
#if GUATIAO_EXCEPTIONS
  map& set(std::string_view key, value&& v) { return v_.set(key, std::move(v)), *this; }
  map& set(std::string_view key, const item& x) { return v_.set(key, x), *this; }
  bool erase(std::string_view key) { return v_.erase(key); }
  void clear() { v_.clear(); }
#endif

 private:
  value v_;
};

/// An owned list: a value that is always a list.
class list : public reader<list> {
 public:
  explicit list(const guatiao_alloc* alloc = default_alloc()) noexcept {
    guatiao_value v = detail::blank();
    v.tag = GUATIAO_LIST;
    v.payload.list.alloc = alloc;
    v_ = value::adopt(v);
  }
  static result<list> try_of(const item& x, const guatiao_alloc* alloc = default_alloc()) {
    auto v = try_make(x, alloc);
    if (!v) return v.error();
    return from(std::move(*v));
  }
  static result<list> try_of(std::initializer_list<item> items, const guatiao_alloc* alloc = default_alloc()) {
    return try_of(item::list(items), alloc);
  }
#if GUATIAO_EXCEPTIONS
  list(std::initializer_list<item> items, const guatiao_alloc* alloc = default_alloc())
      : list(try_of(items, alloc).value()) {}
#endif
  static result<list> from(value&& v) {
    if (!v.is_list()) return detail::error_of(GUATIAO_ERR_WRONG_KIND, "not a list");
    list l(nullptr);
    l.v_ = std::move(v);
    return l;
  }
  static list adopt(const guatiao_list& raw) noexcept {
    list l(nullptr);
    guatiao_value v = detail::blank();
    v.tag = GUATIAO_LIST;
    v.payload.list = raw;
    l.v_ = value::adopt(v);
    return l;
  }
  guatiao_list release() noexcept {
    guatiao_list out = v_.raw()->payload.list;
    v_.raw()->payload.list = guatiao_list{};
    return out;
  }

  const guatiao_value* address() const noexcept { return v_.raw(); }
  list_view get() const noexcept { return list_view(&v_.raw()->payload.list); }
  operator list_view() const noexcept { return get(); }
  operator view() const noexcept { return v_.get(); }
  list_ref ref() noexcept { return list_ref(v_.ref()); }
  value into_value() && noexcept { return std::move(v_); }
  operator value() && noexcept { return std::move(v_); }
  list_view::iterator begin() const noexcept { return get().begin(); }
  list_view::iterator end() const noexcept { return get().end(); }
  using reader<list>::operator[];
  template <class I, detail::if_index<I> = 0>
  node operator[](I index) noexcept {
    return v_[index];
  }

  result<void> try_push(value&& v) { return v_.try_push(std::move(v)); }
  result<void> try_push(const item& x) { return v_.try_push(x); }
  result<void> try_erase_at(std::size_t index) { return v_.try_erase_at(index); }
  result<void> try_clear() { return v_.try_clear(); }
#if GUATIAO_EXCEPTIONS
  list& push(value&& v) { return v_.push(std::move(v)), *this; }
  list& push(const item& x) { return v_.push(x), *this; }
  void erase_at(std::size_t index) { v_.erase_at(index); }
  void clear() { v_.clear(); }
#endif

 private:
  value v_;
};

// ---- the definitions that needed `value` -----------------------------------

inline error::error(status code, std::string message, guatiao_value detail)
    : code_(code), message_(std::move(message)) {
  if (detail.tag == GUATIAO_ABSENT) return;
  auto* held = new (std::nothrow) guatiao_value(detail);
  if (!held) {
    guatiao_value_free(&detail);
    return;
  }
  detail_ = std::shared_ptr<guatiao_value>(held, [](guatiao_value* v) {
    guatiao_value_free(v);
    delete v;
  });
}

template <class D>
result<value> reader<D>::try_clone(const guatiao_alloc* alloc) const {
  guatiao_value out = detail::blank();
  guatiao_status s = guatiao_value_clone(alloc, self(), &out);
  if (s != GUATIAO_OK) return detail::error_of(s, "the value could not be copied");
  return value::adopt(out);
}

#if GUATIAO_EXCEPTIONS
template <class D>
value reader<D>::clone(const guatiao_alloc* alloc) const {
  return try_clone(alloc).value();
}
#endif

inline guatiao_status item::build(const guatiao_alloc* a, guatiao_value* out) const {
  *out = detail::blank();
  struct builder {
    const guatiao_alloc* a;
    guatiao_value* out;
    guatiao_status operator()(std::monostate) const { return guatiao_value_null(out); }
    guatiao_status operator()(absent_marker) const { return guatiao_value_absent(out); }
    guatiao_status operator()(bool b) const { return guatiao_value_bool(b, out); }
    guatiao_status operator()(std::int64_t i) const {
      std::string t = detail::text_of_integer(i);
      return guatiao_value_number(a, detail::str(t), out);
    }
    guatiao_status operator()(std::uint64_t i) const {
      std::string t = detail::text_of_integer(i);
      return guatiao_value_number(a, detail::str(t), out);
    }
    guatiao_status operator()(double d) const {
      std::string t = detail::text_of(d);
      if (t.empty()) return GUATIAO_ERR_BAD_VALUE;
      return guatiao_value_number(a, detail::str(t), out);
    }
    guatiao_status operator()(const std::string& s) const { return guatiao_value_string(a, detail::str(s), out); }
    guatiao_status operator()(const number_text& n) const { return guatiao_value_number(a, detail::str(n.text), out); }
    guatiao_status operator()(const std::vector<std::uint8_t>& b) const {
      return guatiao_value_bytes(a, bytes_view(b).raw(), out);
    }
    guatiao_status operator()(const list_items& l) const {
      guatiao_status s = guatiao_value_list(a, out);
      if (s != GUATIAO_OK) return s;
      for (const item& x : l.items) {
        guatiao_value child = detail::blank();
        s = x.build(a, &child);
        if (s == GUATIAO_OK) s = guatiao_list_push(a, out, &child);
        if (s != GUATIAO_OK) {
          guatiao_value_free(&child);
          guatiao_value_free(out);
          *out = detail::blank();
          return s;
        }
      }
      return GUATIAO_OK;
    }
    guatiao_status operator()(const map_items& m) const {
      guatiao_status s = guatiao_value_map(a, out);
      if (s != GUATIAO_OK) return s;
      for (const auto& kv : m.items) {
        guatiao_value child = detail::blank();
        s = kv.second.build(a, &child);
        if (s == GUATIAO_OK) s = guatiao_map_set(a, out, detail::str(kv.first), &child);
        if (s != GUATIAO_OK) {
          guatiao_value_free(&child);
          guatiao_value_free(out);
          *out = detail::blank();
          return s;
        }
      }
      return GUATIAO_OK;
    }
    guatiao_status operator()(copy_of c) const {
      if (!c.node) return guatiao_value_absent(out);
      return guatiao_value_clone(a, c.node, out);
    }
  };
  guatiao_status s = std::visit(builder{a, out}, v_);
  if (s != GUATIAO_OK) *out = detail::blank();
  return s;
}

namespace detail {

/// Whether two nodes are equal, by the rule `operator==` states; a loop
/// over an explicit stack, so a deep tree costs no native stack.
inline bool equal_nodes(const guatiao_value* a, const guatiao_value* b) {
  struct pair {
    const guatiao_value* a;
    const guatiao_value* b;
  };
  std::vector<pair> todo;
  pair cur{a, b};
  for (;;) {
    const guatiao_value* x = cur.a;
    const guatiao_value* y = cur.b;
    std::uint32_t tx = x ? x->tag : GUATIAO_ABSENT;
    std::uint32_t ty = y ? y->tag : GUATIAO_ABSENT;
    if (tx != ty) return false;
    switch (tx) {
      case GUATIAO_BOOL:
        if (x->payload.b != y->payload.b) return false;
        break;
      case GUATIAO_NUMBER:
        if (text(x->payload.number) != text(y->payload.number)) return false;
        break;
      case GUATIAO_STRING:
        if (text(x->payload.text) != text(y->payload.text)) return false;
        break;
      case GUATIAO_BYTES:
        if (bytes_view(x->payload.bytes.ptr, x->payload.bytes.len) !=
            bytes_view(y->payload.bytes.ptr, y->payload.bytes.len))
          return false;
        break;
      case GUATIAO_LIST:
        if (x->payload.list.len != y->payload.list.len) return false;
        for (std::size_t i = x->payload.list.len; i-- > 0;)
          todo.push_back({&x->payload.list.ptr[i], &y->payload.list.ptr[i]});
        break;
      case GUATIAO_MAP:
        if (x->payload.map.len != y->payload.map.len) return false;
        for (std::size_t i = x->payload.map.len; i-- > 0;) {
          if (text(x->payload.map.ptr[i].key) != text(y->payload.map.ptr[i].key)) return false;
          todo.push_back({&x->payload.map.ptr[i].value, &y->payload.map.ptr[i].value});
        }
        break;
      default: break;
    }
    if (todo.empty()) return true;
    cur = todo.back();
    todo.pop_back();
  }
}

/// What one node owns: its buffer's length, capacity and allocator.
struct owned {
  std::size_t len;
  std::size_t cap;
  const guatiao_alloc* alloc;
  bool has;
};

inline owned owned_by(const guatiao_value& n) noexcept {
  switch (n.tag) {
    case GUATIAO_STRING: return {n.payload.text.len, n.payload.text.cap, n.payload.text.alloc, true};
    case GUATIAO_NUMBER: return {n.payload.number.len, n.payload.number.cap, n.payload.number.alloc, true};
    case GUATIAO_BYTES: return {n.payload.bytes.len, n.payload.bytes.cap, n.payload.bytes.alloc, true};
    case GUATIAO_LIST: return {n.payload.list.len, n.payload.list.cap, n.payload.list.alloc, true};
    case GUATIAO_MAP: return {n.payload.map.len, n.payload.map.cap, n.payload.map.alloc, true};
    default: return {0, 0, nullptr, false};
  }
}

/// Whether a buffer needs nothing of anyone but `home`: owned through it,
/// or empty and owning nothing. A literal or a borrow (`len > 0`,
/// `cap == 0`) is someone else's memory.
inline bool at_home(std::size_t len, std::size_t cap, const guatiao_alloc* alloc,
                    const guatiao_alloc* home) noexcept {
  if (cap == 0) return len == 0;
  return alloc == home;
}

/// Whether every buffer under `root`, keys included, is at home in `home`.
inline bool lives_in(const guatiao_value& root, const guatiao_alloc* home) {
  std::vector<const guatiao_value*> todo{&root};
  while (!todo.empty()) {
    const guatiao_value* n = todo.back();
    todo.pop_back();
    owned o = owned_by(*n);
    if (o.has && !at_home(o.len, o.cap, o.alloc, home)) return false;
    if (n->tag == GUATIAO_LIST) {
      for (std::size_t i = 0; i < n->payload.list.len; ++i) todo.push_back(&n->payload.list.ptr[i]);
    } else if (n->tag == GUATIAO_MAP) {
      for (std::size_t i = 0; i < n->payload.map.len; ++i) {
        const guatiao_string& k = n->payload.map.ptr[i].key;
        if (!at_home(k.len, k.cap, k.alloc, home)) return false;
        todo.push_back(&n->payload.map.ptr[i].value);
      }
    }
  }
  return true;
}

/// Points every empty buffer under `root` that records an allocator at
/// `home`: it owns nothing, but would grow through what it records.
inline void record_home(guatiao_value& root, const guatiao_alloc* home) {
  std::vector<guatiao_value*> todo{&root};
  while (!todo.empty()) {
    guatiao_value* n = todo.back();
    todo.pop_back();
    switch (n->tag) {
      case GUATIAO_STRING:
        if (!n->payload.text.cap && n->payload.text.alloc) n->payload.text.alloc = home;
        break;
      case GUATIAO_NUMBER:
        if (!n->payload.number.cap && n->payload.number.alloc) n->payload.number.alloc = home;
        break;
      case GUATIAO_BYTES:
        if (!n->payload.bytes.cap && n->payload.bytes.alloc) n->payload.bytes.alloc = home;
        break;
      case GUATIAO_LIST:
        if (!n->payload.list.cap) n->payload.list.alloc = home;
        for (std::size_t i = 0; i < n->payload.list.len; ++i) todo.push_back(&n->payload.list.ptr[i]);
        break;
      case GUATIAO_MAP:
        if (!n->payload.map.cap) n->payload.map.alloc = home;
        for (std::size_t i = 0; i < n->payload.map.len; ++i) todo.push_back(&n->payload.map.ptr[i].value);
        break;
      default: break;
    }
  }
}

/// Moves the tree at `root` into `home` when anything in it lives
/// elsewhere: a copy built through `home`, the original freed through its
/// own allocators; then every empty buffer records `home`. So a tree
/// crossing to another side needs nothing of the side that built it. A
/// refused copy leaves `root` as it was.
inline guatiao_status settle(guatiao_value& root, const guatiao_alloc* home) {
  if (!home) return GUATIAO_OK;
  if (!lives_in(root, home)) {
    guatiao_value copy = blank();
    guatiao_status s = guatiao_value_clone(home, &root, &copy);
    if (s != GUATIAO_OK) return s;
    guatiao_value_free(&root);
    root = copy;
  }
  record_home(root, home);
  return GUATIAO_OK;
}

}  // namespace detail

inline bool operator==(view a, view b) { return detail::equal_nodes(a.raw(), b.raw()); }

}  // namespace guatiao

#endif  // GUATIAO_VALUE_HPP
