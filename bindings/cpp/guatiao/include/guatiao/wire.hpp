// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// guatiao for C++: a value as bytes (the wire encoding), and channels of
// frames a receiver checks against each channel's schema. Part of
// <guatiao/guatiao.hpp>.

#ifndef GUATIAO_WIRE_HPP
#define GUATIAO_WIRE_HPP

#include <guatiao/value.hpp>

namespace guatiao {
inline namespace GUATIAO_ABI {

namespace detail {

/// The bytes of a BYTES value a call wrote, the value freed.
inline result<std::vector<std::uint8_t>> take_bytes(guatiao_status s, guatiao_value& out) {
  if (s != GUATIAO_OK) {
    guatiao_value_free(&out);
    return error_of(s);
  }
  std::vector<std::uint8_t> b;
  if (out.tag == GUATIAO_BYTES && out.payload.bytes.len)
    b.assign(out.payload.bytes.ptr, out.payload.bytes.ptr + out.payload.bytes.len);
  guatiao_value_free(&out);
  return b;
}

}  // namespace detail

/// The wire bytes of `v`: the same bytes in every language.
inline result<std::vector<std::uint8_t>> try_to_wire(view v) {
  guatiao_value out = detail::blank();
  return detail::take_bytes(guatiao_wire_encode(default_alloc(), v.raw(), &out), out);
}

/// The value `bytes` encode, built in `alloc`; BAD_VALUE for bytes that
/// are not exactly one value.
inline result<value> try_from_wire(bytes_view bytes, const guatiao_alloc* alloc = default_alloc()) {
  guatiao_value out = detail::blank();
  guatiao_status s = guatiao_wire_decode(alloc, bytes.raw(), &out);
  if (s != GUATIAO_OK) {
    guatiao_value_free(&out);
    return detail::error_of(s, "the bytes are not one value");
  }
  return value::adopt(out);
}

/// A frame's kind byte.
enum class frame : std::uint32_t {
  schema = GUATIAO_FRAME_SCHEMA,
  value = GUATIAO_FRAME_VALUE,
  close = GUATIAO_FRAME_CLOSE,
};

/// Why a receiver refused a frame.
enum class refusal : std::uint32_t {
  accepted = GUATIAO_WIRE_ACCEPTED,
  malformed = GUATIAO_WIRE_MALFORMED,
  no_schema = GUATIAO_WIRE_NO_SCHEMA,
  not_a_schema = GUATIAO_WIRE_NOT_A_SCHEMA,
  invalid = GUATIAO_WIRE_INVALID,
};

constexpr const char* refusal_name(refusal r) noexcept {
  switch (r) {
    case refusal::accepted: return "accepted";
    case refusal::malformed: return "malformed";
    case refusal::no_schema: return "no_schema";
    case refusal::not_a_schema: return "not_a_schema";
    case refusal::invalid: return "invalid";
  }
  return "refusal";
}

/// The frame that announces `schema` on `channel`.
inline result<std::vector<std::uint8_t>> try_wire_announce(std::uint64_t channel, view schema) {
  guatiao_value out = detail::blank();
  return detail::take_bytes(guatiao_wire_announce(default_alloc(), channel, schema.raw(), &out), out);
}
/// The frame that carries `v` on `channel`.
inline result<std::vector<std::uint8_t>> try_wire_send(std::uint64_t channel, view v) {
  guatiao_value out = detail::blank();
  return detail::take_bytes(guatiao_wire_send(default_alloc(), channel, v.raw(), &out), out);
}
/// The frame that ends `channel`.
inline result<std::vector<std::uint8_t>> try_wire_close(std::uint64_t channel) {
  guatiao_value out = detail::blank();
  return detail::take_bytes(guatiao_wire_close(default_alloc(), channel, &out), out);
}

#if GUATIAO_EXCEPTIONS
inline std::vector<std::uint8_t> to_wire(view v) { return try_to_wire(v).value(); }
inline value from_wire(bytes_view bytes, const guatiao_alloc* alloc = default_alloc()) {
  return try_from_wire(bytes, alloc).value();
}
inline std::vector<std::uint8_t> wire_announce(std::uint64_t channel, view schema) {
  return try_wire_announce(channel, schema).value();
}
inline std::vector<std::uint8_t> wire_send(std::uint64_t channel, view v) { return try_wire_send(channel, v).value(); }
inline std::vector<std::uint8_t> wire_close(std::uint64_t channel) { return try_wire_close(channel).value(); }
#endif

/// What an accepted frame said: a schema now the channel's (a copy), a
/// value its schema accepted, or a close (`data` ABSENT).
struct event {
  frame kind = frame::schema;
  std::uint64_t channel = 0;
  value data;
};

/// Keeps each channel's schema and checks every value against it. A
/// refused frame is an error whose status is the refusal's
/// (malformed: BAD_VALUE, no_schema: NOT_FOUND, not_a_schema: WRONG_KIND,
/// invalid: BAD_VALUE), its message the refusal in words, and its detail
/// `{"refusal": <name>, "channel": n, "kind": n, "at": n, "detail": <the
/// refused field, for invalid>}`; it changes nothing the receiver keeps.
/// One caller at a time.
class receiver {
 public:
  explicit receiver(const guatiao_alloc* alloc = default_alloc()) noexcept : r_(guatiao_wire_receiver_new(alloc)) {}
  receiver(receiver&& o) noexcept : r_(o.r_) { o.r_ = nullptr; }
  receiver& operator=(receiver&& o) noexcept {
    if (this != &o) {
      guatiao_wire_receiver_free(r_);
      r_ = o.r_;
      o.r_ = nullptr;
    }
    return *this;
  }
  receiver(const receiver&) = delete;
  receiver& operator=(const receiver&) = delete;
  ~receiver() { guatiao_wire_receiver_free(r_); }

  /// False when the allocator was refused at construction.
  explicit operator bool() const noexcept { return r_ != nullptr; }

  result<event> try_accept(bytes_view frame_bytes) {
    if (!r_) return detail::error_of(GUATIAO_ERR_ALLOC, "the receiver has no allocator");
    guatiao_wire_event ev;
    std::memset(&ev, 0, sizeof ev);
    guatiao_status s = guatiao_wire_receiver_accept(r_, frame_bytes.raw(), &ev);
    value data = value::adopt(ev.value);
    value message = value::adopt(ev.message);
    value detail_tree = value::adopt(ev.detail);
    if (s == GUATIAO_OK) return event{static_cast<frame>(ev.kind), ev.channel, std::move(data)};
    if (ev.refusal == GUATIAO_WIRE_ACCEPTED)
      return detail::error_of(s, std::string(message.string_or("")));
    std::vector<std::pair<std::string, item>> why{
        {"refusal", refusal_name(static_cast<refusal>(ev.refusal))},
        {"channel", ev.channel},
        {"kind", ev.kind},
        {"at", ev.at}};
    if (!detail_tree.is_absent()) why.emplace_back("detail", item(detail_tree.get()));
    guatiao_value d = detail::blank();
    if (item::map_from(std::move(why)).build(default_alloc(), &d) != GUATIAO_OK) d = detail::blank();
    return ::guatiao::error(static_cast<status>(s), std::string(message.string_or("")), d);
  }
#if GUATIAO_EXCEPTIONS
  event accept(bytes_view frame_bytes) { return try_accept(frame_bytes).value(); }
#endif

  guatiao_wire_receiver* raw() const noexcept { return r_; }

 private:
  guatiao_wire_receiver* r_ = nullptr;
};

}  // namespace GUATIAO_ABI
}  // namespace guatiao

#endif  // GUATIAO_WIRE_HPP
