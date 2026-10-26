// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// guatiao for C++: kinds. A kind is declared once over its C table with
// GUATIAO_KIND (GUATIAO_OBJECT_KIND for an object kind), naming each
// slot's C++ signature; from that the binding calls any table of the
// kind (remote<K>, object<K>, lent<K>) and serves one from any C++ class
// whose methods fit, the tables and trampolines built by templates. A
// method that does not fit its slot, or a required slot left out, is a
// compile error. Part of <guatiao/guatiao.hpp>.

#ifndef GUATIAO_KIND_HPP
#define GUATIAO_KIND_HPP

#include <guatiao/registry.hpp>
#include <guatiao/value.hpp>

#include <atomic>
#include <cstddef>
#include <tuple>

// ---- the preprocessor list GUATIAO_KIND walks (up to 48 slots) -----------

#define GUATIAO_PP_EXPAND(x) x
#define GUATIAO_PP_CAT(a, b) GUATIAO_PP_CAT_(a, b)
#define GUATIAO_PP_CAT_(a, b) a##b
#define GUATIAO_PP_NARG(...) GUATIAO_PP_EXPAND(GUATIAO_PP_ARG_N(__VA_ARGS__, 48, 47, 46, 45, 44, 43, 42, 41, 40, 39, 38, 37, 36, 35, 34, 33, 32, 31, 30, 29, 28, 27, 26, 25, 24, 23, 22, 21, 20, 19, 18, 17, 16, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0))
#define GUATIAO_PP_ARG_N(_1, _2, _3, _4, _5, _6, _7, _8, _9, _10, _11, _12, _13, _14, _15, _16, _17, _18, _19, _20, _21, _22, _23, _24, _25, _26, _27, _28, _29, _30, _31, _32, _33, _34, _35, _36, _37, _38, _39, _40, _41, _42, _43, _44, _45, _46, _47, _48, N, ...) N
#define GUATIAO_PP_FOR_EACH(m, ...) \
  GUATIAO_PP_EXPAND(GUATIAO_PP_CAT(GUATIAO_PP_FE_, GUATIAO_PP_NARG(__VA_ARGS__))(m, __VA_ARGS__))
#define GUATIAO_PP_FE_1(m, x) m x
#define GUATIAO_PP_FE_2(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_1(m, __VA_ARGS__))
#define GUATIAO_PP_FE_3(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_2(m, __VA_ARGS__))
#define GUATIAO_PP_FE_4(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_3(m, __VA_ARGS__))
#define GUATIAO_PP_FE_5(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_4(m, __VA_ARGS__))
#define GUATIAO_PP_FE_6(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_5(m, __VA_ARGS__))
#define GUATIAO_PP_FE_7(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_6(m, __VA_ARGS__))
#define GUATIAO_PP_FE_8(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_7(m, __VA_ARGS__))
#define GUATIAO_PP_FE_9(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_8(m, __VA_ARGS__))
#define GUATIAO_PP_FE_10(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_9(m, __VA_ARGS__))
#define GUATIAO_PP_FE_11(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_10(m, __VA_ARGS__))
#define GUATIAO_PP_FE_12(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_11(m, __VA_ARGS__))
#define GUATIAO_PP_FE_13(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_12(m, __VA_ARGS__))
#define GUATIAO_PP_FE_14(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_13(m, __VA_ARGS__))
#define GUATIAO_PP_FE_15(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_14(m, __VA_ARGS__))
#define GUATIAO_PP_FE_16(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_15(m, __VA_ARGS__))
#define GUATIAO_PP_FE_17(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_16(m, __VA_ARGS__))
#define GUATIAO_PP_FE_18(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_17(m, __VA_ARGS__))
#define GUATIAO_PP_FE_19(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_18(m, __VA_ARGS__))
#define GUATIAO_PP_FE_20(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_19(m, __VA_ARGS__))
#define GUATIAO_PP_FE_21(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_20(m, __VA_ARGS__))
#define GUATIAO_PP_FE_22(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_21(m, __VA_ARGS__))
#define GUATIAO_PP_FE_23(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_22(m, __VA_ARGS__))
#define GUATIAO_PP_FE_24(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_23(m, __VA_ARGS__))
#define GUATIAO_PP_FE_25(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_24(m, __VA_ARGS__))
#define GUATIAO_PP_FE_26(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_25(m, __VA_ARGS__))
#define GUATIAO_PP_FE_27(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_26(m, __VA_ARGS__))
#define GUATIAO_PP_FE_28(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_27(m, __VA_ARGS__))
#define GUATIAO_PP_FE_29(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_28(m, __VA_ARGS__))
#define GUATIAO_PP_FE_30(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_29(m, __VA_ARGS__))
#define GUATIAO_PP_FE_31(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_30(m, __VA_ARGS__))
#define GUATIAO_PP_FE_32(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_31(m, __VA_ARGS__))
#define GUATIAO_PP_FE_33(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_32(m, __VA_ARGS__))
#define GUATIAO_PP_FE_34(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_33(m, __VA_ARGS__))
#define GUATIAO_PP_FE_35(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_34(m, __VA_ARGS__))
#define GUATIAO_PP_FE_36(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_35(m, __VA_ARGS__))
#define GUATIAO_PP_FE_37(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_36(m, __VA_ARGS__))
#define GUATIAO_PP_FE_38(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_37(m, __VA_ARGS__))
#define GUATIAO_PP_FE_39(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_38(m, __VA_ARGS__))
#define GUATIAO_PP_FE_40(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_39(m, __VA_ARGS__))
#define GUATIAO_PP_FE_41(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_40(m, __VA_ARGS__))
#define GUATIAO_PP_FE_42(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_41(m, __VA_ARGS__))
#define GUATIAO_PP_FE_43(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_42(m, __VA_ARGS__))
#define GUATIAO_PP_FE_44(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_43(m, __VA_ARGS__))
#define GUATIAO_PP_FE_45(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_44(m, __VA_ARGS__))
#define GUATIAO_PP_FE_46(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_45(m, __VA_ARGS__))
#define GUATIAO_PP_FE_47(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_46(m, __VA_ARGS__))
#define GUATIAO_PP_FE_48(m, x, ...) m x GUATIAO_PP_EXPAND(GUATIAO_PP_FE_47(m, __VA_ARGS__))

#define GUATIAO_DETAIL_REQ_required true
#define GUATIAO_DETAIL_REQ_optional false

/// Declares a provider kind over its C table:
///
///     GUATIAO_KIND(greeter, greeter_vtable,
///         (required, greet, guatiao::map(std::string_view)),
///         (optional, shout, std::string(std::string_view)))
///
/// `greeter` becomes a struct naming the kind; each slot is `(required |
/// optional, <C field>, <C++ signature>)`, in any order. The C++ types a
/// signature names, and the C types they stand for, are in the binding's
/// AGENTS.md; a signature that does not match the table's field is a
/// compile error. The kind's name is the struct's; GUATIAO_KIND_NAMED
/// gives another (`"session-backend"`).
#define GUATIAO_KIND(name, table, ...) GUATIAO_DETAIL_KIND(name, #name, table, false, __VA_ARGS__)
#define GUATIAO_KIND_NAMED(name, kind_name, table, ...) \
  GUATIAO_DETAIL_KIND(name, kind_name, table, false, __VA_ARGS__)
/// An object kind: a handle one caller owns, its table starting with
/// `destroy`, which is not listed (the serving class's destructor).
#define GUATIAO_OBJECT_KIND(name, table, ...) GUATIAO_DETAIL_KIND(name, #name, table, true, __VA_ARGS__)
#define GUATIAO_OBJECT_KIND_NAMED(name, kind_name, table, ...) \
  GUATIAO_DETAIL_KIND(name, kind_name, table, true, __VA_ARGS__)

#define GUATIAO_DETAIL_KIND(name_, spelled_, table_, object_, ...)                                           \
  struct name_ {                                                                                             \
    using table_type = table_;                                                                               \
    static constexpr char kind_name[] = spelled_;                                                            \
    static constexpr std::uint32_t floor_hash = table_##_FLOOR_HASH;                                         \
    static constexpr bool is_object = object_;                                                               \
    GUATIAO_PP_FOR_EACH(GUATIAO_DETAIL_SLOT, __VA_ARGS__)                                                    \
    /* The table's size up to its last required slot: a smaller one is not of this kind. */                 \
    static constexpr std::size_t floor_size() noexcept {                                                     \
      std::size_t f = sizeof(guatiao_kind_header) + (object_ ? sizeof(void (*)(void*)) : 0);                 \
      GUATIAO_PP_FOR_EACH(GUATIAO_DETAIL_SLOT_FLOOR, __VA_ARGS__)                                            \
      return f;                                                                                              \
    }                                                                                                        \
    template <class Served, int Mode>                                                                        \
    static constexpr table_type make_table() noexcept {                                                      \
      table_type t{};                                                                                        \
      t.header.struct_size = static_cast<std::uint32_t>(sizeof(table_type));                                 \
      t.header.floor_hash = floor_hash;                                                                      \
      ::guatiao::detail::destroy_slot<name_, Served, Mode, object_>::put(t);                                 \
      GUATIAO_PP_FOR_EACH(GUATIAO_DETAIL_SLOT_FILL, __VA_ARGS__)                                             \
      return t;                                                                                              \
    }                                                                                                        \
    /* A method per slot, `try_` forms, and whether a table carries one. */                                  \
    template <class Self>                                                                                    \
    struct calls {                                                                                           \
      GUATIAO_PP_FOR_EACH(GUATIAO_DETAIL_SLOT_CALL, __VA_ARGS__)                                             \
    };                                                                                                       \
    /* A provider kind's table from a registry or an instance: templates, so an object kind's */             \
    /* declaration never instantiates them. */                                                               \
    template <class R = ::guatiao::registry>                                                                 \
    static auto try_bind(const R& r, std::string_view key) {                                                 \
      return ::guatiao::try_bind<name_>(r, key);                                                             \
    }                                                                                                        \
    template <class I = ::guatiao::instance>                                                                 \
    static auto try_bind(const I& i) {                                                                       \
      return ::guatiao::try_bind<name_>(i);                                                                  \
    }                                                                                                        \
    template <class R = ::guatiao::registry>                                                                 \
    static auto bind(const R& r, std::string_view key) {                                                     \
      return ::guatiao::detail::or_throw(::guatiao::try_bind<name_>(r, key));                                \
    }                                                                                                        \
    template <class I = ::guatiao::instance>                                                                 \
    static auto bind(const I& i) {                                                                           \
      return ::guatiao::detail::or_throw(::guatiao::try_bind<name_>(i));                                     \
    }                                                                                                        \
  }

#define GUATIAO_DETAIL_SLOT(req_, n_, ...)                                                                   \
  struct slot_##n_ {                                                                                         \
    using signature = __VA_ARGS__;                                                                           \
    using c_type = decltype(table_type::n_);                                                                 \
    static constexpr bool required = GUATIAO_PP_CAT(GUATIAO_DETAIL_REQ_, req_);                              \
    static constexpr const char* name = #n_;                                                                 \
    static constexpr std::size_t offset = offsetof(table_type, n_);                                          \
    template <class P, class... A>                                                                           \
    using answer_of = decltype(std::declval<P&>().n_(std::declval<A>()...));                                 \
    template <class P, class = void>                                                                         \
    struct named : std::false_type {};                                                                       \
    template <class P>                                                                                       \
    struct named<P, std::void_t<decltype(&P::n_)>> : std::true_type {};                                      \
    template <class P, class... A>                                                                           \
    static decltype(auto) invoke(P& p, A&&... a) {                                                           \
      return p.n_(std::forward<A>(a)...);                                                                    \
    }                                                                                                        \
    template <class Served, int Mode>                                                                        \
    static constexpr void fill(table_type& t) noexcept {                                                     \
      constexpr bool fits = ::guatiao::detail::fits<slot_##n_, Served>::value;                               \
      static_assert(fits || !named<Served>::value,                                                           \
                    "guatiao: the method `" #n_ "` does not fit its slot: its parameters and answer are the " \
                    "kind's declaration of `" #n_ "`");                                                      \
      static_assert(fits || named<Served>::value || !required, "guatiao: `" #n_ "` is a required slot of a kind this class serves, "  \
                                       "and the class has no method that fits it");                          \
      if constexpr (fits) t.n_ = &::guatiao::detail::server<slot_##n_, Served, Mode>::call;                  \
    }                                                                                                        \
  };                                                                                                         \
  static_assert(::guatiao::detail::shape<slot_##n_>::fits_table,                                             \
                "guatiao: the C++ signature of `" #n_ "` does not stand for its field in the C table");

#define GUATIAO_DETAIL_SLOT_FLOOR(req_, n_, ...)                                       \
  f = ::guatiao::detail::floor_with(f, GUATIAO_PP_CAT(GUATIAO_DETAIL_REQ_, req_), \
                                    offsetof(table_type, n_) + sizeof(void (*)(void*)));

#define GUATIAO_DETAIL_SLOT_FILL(req_, n_, ...) slot_##n_::template fill<Served, Mode>(t);

/* A provider's table is shared: its slots are called through a const handle, from any thread. An */ \
/* object is one holder's: its slots are called through a handle that is not const, one at a time. */ \
#define GUATIAO_DETAIL_SLOT_CALL(req_, n_, ...)                                                    \
  template <class... A, class S_ = Self, std::enable_if_t<::guatiao::detail::shared_v<S_>, int> = 0> \
  decltype(auto) n_(A&&... a) const {                                                              \
    return ::guatiao::detail::call_slot<slot_##n_>(static_cast<const S_&>(*this), std::forward<A>(a)...); \
  }                                                                                                \
  template <class... A, class S_ = Self, std::enable_if_t<!::guatiao::detail::shared_v<S_>, int> = 0> \
  decltype(auto) n_(A&&... a) {                                                                    \
    return ::guatiao::detail::call_slot<slot_##n_>(static_cast<const S_&>(*this), std::forward<A>(a)...); \
  }                                                                                                \
  template <class... A, class S_ = Self, std::enable_if_t<::guatiao::detail::shared_v<S_>, int> = 0> \
  auto try_##n_(A&&... a) const {                                                                  \
    return ::guatiao::detail::try_call_slot<slot_##n_>(static_cast<const S_&>(*this), std::forward<A>(a)...); \
  }                                                                                                \
  template <class... A, class S_ = Self, std::enable_if_t<!::guatiao::detail::shared_v<S_>, int> = 0> \
  auto try_##n_(A&&... a) {                                                                        \
    return ::guatiao::detail::try_call_slot<slot_##n_>(static_cast<const S_&>(*this), std::forward<A>(a)...); \
  }                                                                                                \
  bool has_##n_() const noexcept { return ::guatiao::detail::has_slot<slot_##n_>(static_cast<const Self&>(*this)); }

namespace guatiao {
inline namespace GUATIAO_ABI {

template <class K>
class remote;
template <class K>
class object;
template <class K>
class lent;

namespace detail {

/// How a table is served: a provider's (no `destroy`), an object handed
/// over (`destroy` deletes it), an object lent (`destroy` frees nothing).
enum serve_mode : int { as_provider = 0, as_owned = 1, as_lent = 2 };

/// What this binary serves that is still alive: provider instances and
/// objects. A library's `unload` slot refuses while it is not zero.
inline std::atomic<long>& live() noexcept {
  static std::atomic<long> n{0};
  return n;
}

template <class T>
struct is_result : std::false_type {};
template <class T>
struct is_result<result<T>> : std::true_type {};

template <class T>
struct is_object_handle : std::false_type {};
template <class K>
struct is_object_handle<object<K>> : std::true_type {};
template <class T>
struct is_lent_handle : std::false_type {};
template <class K>
struct is_lent_handle<lent<K>> : std::true_type {};
template <class T>
struct is_remote_handle : std::false_type {};
template <class K>
struct is_remote_handle<remote<K>> : std::true_type {};
/// Whether a handle's slots may be called through it from several
/// threads: a provider's table, not an object.
template <class T>
constexpr bool shared_v = is_remote_handle<T>::value;

/// A plain number or truth value crossing as itself.
template <class T>
constexpr bool is_scalar_v = std::is_arithmetic_v<T> && !is_char_v<T>;

// ---- the C type each C++ type stands for -----------------------------------

template <class T, class = void>
struct c_arg {};
template <>
struct c_arg<std::string_view> {
  using type = guatiao_str;
};
template <>
struct c_arg<bytes_view> {
  using type = guatiao_bytes;
};
template <>
struct c_arg<bytes_mut> {
  using type = guatiao_bytes_mut;
};
template <class T>
struct c_arg<T, std::enable_if_t<is_scalar_v<T>>> {
  using type = T;
};
template <>
struct c_arg<view> {
  using type = const guatiao_value*;
};
template <>
struct c_arg<map_view> {
  using type = const guatiao_map*;
};
template <>
struct c_arg<list_view> {
  using type = const guatiao_list*;
};
template <>
struct c_arg<value> {
  using type = guatiao_value*;
};
template <>
struct c_arg<map> {
  using type = guatiao_map*;
};
template <>
struct c_arg<list> {
  using type = guatiao_list*;
};
template <>
struct c_arg<node> {
  using type = guatiao_value*;
};
template <>
struct c_arg<map_ref> {
  using type = guatiao_map*;
};
template <>
struct c_arg<list_ref> {
  using type = guatiao_list*;
};
template <class K>
struct c_arg<object<K>> {
  using type = guatiao_object;
};
template <class K>
struct c_arg<lent<K>> {
  using type = const guatiao_object*;
};

template <class R, class = void>
struct c_out {};
template <>
struct c_out<void> {
  using type = void;
};
template <>
struct c_out<value> {
  using type = guatiao_value*;
};
template <>
struct c_out<map> {
  using type = guatiao_map*;
};
template <>
struct c_out<list> {
  using type = guatiao_list*;
};
template <>
struct c_out<std::string> {
  using type = guatiao_string*;
};
template <class T>
struct c_out<T, std::enable_if_t<is_scalar_v<T>>> {
  using type = T*;
};
template <class K>
struct c_out<object<K>> {
  using type = guatiao_object*;
};

template <class Out, bool Err, class... A>
struct c_fn;
template <class... A>
struct c_fn<void, false, A...> {
  using type = guatiao_status (*)(void*, A...);
};
template <class... A>
struct c_fn<void, true, A...> {
  using type = guatiao_status (*)(void*, A..., guatiao_provider_error*);
};
template <class Out, class... A>
struct c_fn<Out, false, A...> {
  using type = guatiao_status (*)(void*, A..., Out);
};
template <class Out, class... A>
struct c_fn<Out, true, A...> {
  using type = guatiao_status (*)(void*, A..., Out, guatiao_provider_error*);
};

template <class Sig>
struct sig_tag {};

template <class Sig>
struct sig_arity;
template <class R, class... A>
struct sig_arity<R(A...)> : std::integral_constant<std::size_t, sizeof...(A)> {};

/// The floor grown to reach the end of a required slot.
constexpr std::size_t floor_with(std::size_t f, bool required, std::size_t end) noexcept {
  return required && f < end ? end : f;
}

/// A slot's C++ signature against its C field: whether they stand for one
/// another, and whether the slot can refuse with words.
template <class Slot, class Sig = typename Slot::signature>
struct shape;
template <class Slot, class R, class... A>
struct shape<Slot, R(A...)> {
  using answer = R;
  using with_words = typename c_fn<typename c_out<R>::type, true, typename c_arg<A>::type...>::type;
  using without_words = typename c_fn<typename c_out<R>::type, false, typename c_arg<A>::type...>::type;
  static constexpr bool refuses = std::is_same_v<typename Slot::c_type, with_words>;
  static constexpr bool fits_table = refuses || std::is_same_v<typename Slot::c_type, without_words>;
};

/// Whether a method answers what a slot answers: `R` itself, something
/// that becomes one, or a `result<R>`.
template <class M, class R>
constexpr bool answers_v = std::is_same_v<M, result<R>> ||
                           (std::is_void_v<R> ? std::is_void_v<M> : std::is_convertible_v<M, R>);

/// Whether `P` has a method fitting `Slot`: callable with the slot's
/// parameters, answering its answer.
template <class Slot, class P, class Sig = typename Slot::signature, class = void>
struct fits : std::false_type {};
template <class Slot, class P, class R, class... A>
struct fits<Slot, P, R(A...), std::void_t<typename Slot::template answer_of<P, A...>>>
    : std::bool_constant<answers_v<typename Slot::template answer_of<P, A...>, R>> {};

// ---- reading a table ---------------------------------------------------------

/// The slot's function in the table `self` calls through, or null when
/// the table is too short to hold it or leaves it empty.
template <class Slot, class Self>
typename Slot::c_type slot_fn(const Self& self) noexcept {
  typename Slot::c_type f = nullptr;
  if (!self.table() || self.table_size() < Slot::offset + sizeof f) return nullptr;
  std::memcpy(&f, static_cast<const char*>(self.table()) + Slot::offset, sizeof f);
  return f;
}

template <class Slot, class Self>
bool has_slot(const Self& self) noexcept {
  return self.usable() && slot_fn<Slot>(self) != nullptr;
}

/// Whether a table is one of kind `K`: its header's hash, and a size
/// reaching its last required slot.
template <class K>
bool table_of(const void* table, std::size_t size) noexcept {
  if (!table || size < K::floor_size()) return false;
  guatiao_kind_header h;
  std::memcpy(&h, table, sizeof h);
  return h.floor_hash == K::floor_hash;
}

inline bool usable_alloc(const guatiao_alloc* a) noexcept {
  return a && a->struct_size >= sizeof(guatiao_alloc) && a->alloc && a->free;
}

/// A refusal's words written into a slot's error, in this binary's
/// default allocator; the caller frees them through the one they record.
inline void write_words(guatiao_provider_error* err, guatiao_status s, std::string_view why) noexcept {
  if (!err) return;
  err->status = s;
  if (!make_string(default_alloc(), why, err->message)) err->message = guatiao_string{};
}

// ---- calling a slot: C++ arguments to C ---------------------------------------

template <class P, class = void>
struct send;
template <>
struct send<std::string_view> {
  std::string_view v;
  guatiao_str arg() const noexcept { return str(v); }
  guatiao_status after(const guatiao_alloc*) noexcept { return GUATIAO_OK; }
};
template <>
struct send<bytes_view> {
  bytes_view v;
  guatiao_bytes arg() const noexcept { return v.raw(); }
  guatiao_status after(const guatiao_alloc*) noexcept { return GUATIAO_OK; }
};
template <>
struct send<bytes_mut> {
  bytes_mut v;
  guatiao_bytes_mut arg() const noexcept { return v.raw(); }
  guatiao_status after(const guatiao_alloc*) noexcept { return GUATIAO_OK; }
};
template <class T>
struct send<T, std::enable_if_t<is_scalar_v<T>>> {
  T v;
  T arg() const noexcept { return v; }
  guatiao_status after(const guatiao_alloc*) noexcept { return GUATIAO_OK; }
};
template <>
struct send<view> {
  view v;
  const guatiao_value* arg() const noexcept { return v.raw(); }
  guatiao_status after(const guatiao_alloc*) noexcept { return GUATIAO_OK; }
};
template <>
struct send<map_view> {
  map_view v;
  const guatiao_map* arg() const noexcept { return v.raw(); }
  guatiao_status after(const guatiao_alloc*) noexcept { return GUATIAO_OK; }
};
template <>
struct send<list_view> {
  list_view v;
  const guatiao_list* arg() const noexcept { return v.raw(); }
  guatiao_status after(const guatiao_alloc*) noexcept { return GUATIAO_OK; }
};
/// A tree handed over: held until the call, then the callee's whatever it
/// answers; freed here when the call never happens.
template <>
struct send<value> {
  value v;
  guatiao_value node = blank();
  guatiao_value* arg() noexcept {
    node = v.release();
    return &node;
  }
  guatiao_status after(const guatiao_alloc*) noexcept { return GUATIAO_OK; }
};
template <>
struct send<map> {
  map v;
  guatiao_map c{};
  guatiao_map* arg() noexcept {
    c = v.release();
    return &c;
  }
  guatiao_status after(const guatiao_alloc*) noexcept { return GUATIAO_OK; }
};
template <>
struct send<list> {
  list v;
  guatiao_list c{};
  guatiao_list* arg() noexcept {
    c = v.release();
    return &c;
  }
  guatiao_status after(const guatiao_alloc*) noexcept { return GUATIAO_OK; }
};
/// A tree lent for the call: anything the callee built into it elsewhere,
/// or wrote over it, is moved once the call returns into the allocator the
/// tree recorded when it was lent; `after` answers a refusal, which leaves
/// the tree whole where it stands.
template <>
struct send<node> {
  node v;
  const guatiao_alloc* lent_in = nullptr;
  guatiao_value* arg() noexcept {
    if (v.raw()) lent_in = recorded(*v.raw());
    return v.raw();
  }
  guatiao_status after(const guatiao_alloc* home) noexcept {
    if (!v.raw()) return GUATIAO_OK;
    return settle(*v.raw(), lent_in ? lent_in : home);
  }
};
template <>
struct send<map_ref> {
  map_ref v;
  const guatiao_alloc* lent_in = nullptr;
  guatiao_map* arg() noexcept {
    guatiao_value* n = v.as_node().raw();
    if (n) lent_in = recorded(*n);
    return n && n->tag == GUATIAO_MAP ? &n->payload.map : nullptr;
  }
  // Moved home, and any key the callee wrote that is not UTF-8 removed:
  // GUATIAO_ERR_BAD_VALUE then.
  guatiao_status after(const guatiao_alloc* home) noexcept {
    guatiao_value* n = v.as_node().raw();
    const guatiao_alloc* to = lent_in ? lent_in : home;
    if (!n || !to) return GUATIAO_OK;
    return guatiao_map_settle_lent(n, to);
  }
};
template <>
struct send<list_ref> {
  list_ref v;
  const guatiao_alloc* lent_in = nullptr;
  guatiao_list* arg() noexcept {
    guatiao_value* n = v.as_node().raw();
    if (n) lent_in = recorded(*n);
    return n && n->tag == GUATIAO_LIST ? &n->payload.list : nullptr;
  }
  guatiao_status after(const guatiao_alloc* home) noexcept { return send<node>{v.as_node(), lent_in}.after(home); }
};
template <class K>
struct send<object<K>> {
  object<K> v;
  guatiao_object arg() noexcept { return v.release(); }
  guatiao_status after(const guatiao_alloc*) noexcept { return GUATIAO_OK; }
};
template <class K>
struct send<lent<K>> {
  lent<K> v;
  const guatiao_object* arg() const noexcept { return &v.raw(); }
  guatiao_status after(const guatiao_alloc*) noexcept { return GUATIAO_OK; }
};

// ---- calling a slot: the answer back -------------------------------------------

template <class R, class = void>
struct receive;
template <>
struct receive<void> {
  explicit receive(const guatiao_alloc*) noexcept {}
  void drop() noexcept {}
  result<void> take(const guatiao_alloc*) noexcept { return {}; }
};
/// A tree answered: the out slot carries the allocator wanted; whatever
/// arrives built elsewhere is moved there.
template <>
struct receive<value> {
  guatiao_value c = blank();
  explicit receive(const guatiao_alloc* home) noexcept { c.payload.map.alloc = home; }
  guatiao_value* arg() noexcept { return &c; }
  void drop() noexcept { guatiao_value_free(&c); }
  result<value> take(const guatiao_alloc* home) {
    guatiao_status s = settle(c, home);
    if (s != GUATIAO_OK) {
      drop();
      return error_of(s, "the answer could not be moved into its allocator");
    }
    return value::adopt(c);
  }
};
template <>
struct receive<map> {
  guatiao_map c{};
  explicit receive(const guatiao_alloc* home) noexcept { c.alloc = home; }
  guatiao_map* arg() noexcept { return &c; }
  void drop() noexcept { map::adopt(c); }
  result<map> take(const guatiao_alloc* home) {
    guatiao_value v = blank();
    v.tag = GUATIAO_MAP;
    v.payload.map = c;
    guatiao_status s = settle(v, home);
    if (s != GUATIAO_OK) {
      guatiao_value_free(&v);
      return error_of(s, "the answer could not be moved into its allocator");
    }
    return map::adopt(v.payload.map);
  }
};
template <>
struct receive<list> {
  guatiao_list c{};
  explicit receive(const guatiao_alloc* home) noexcept { c.alloc = home; }
  guatiao_list* arg() noexcept { return &c; }
  void drop() noexcept { list::adopt(c); }
  result<list> take(const guatiao_alloc* home) {
    guatiao_value v = blank();
    v.tag = GUATIAO_LIST;
    v.payload.list = c;
    guatiao_status s = settle(v, home);
    if (s != GUATIAO_OK) {
      guatiao_value_free(&v);
      return error_of(s, "the answer could not be moved into its allocator");
    }
    return list::adopt(v.payload.list);
  }
};
template <>
struct receive<std::string> {
  guatiao_string c{};
  explicit receive(const guatiao_alloc*) noexcept {}
  guatiao_string* arg() noexcept { return &c; }
  void drop() noexcept { release_string(c); }
  result<std::string> take(const guatiao_alloc*) {
    struct release_guard {
      guatiao_string& s;
      ~release_guard() { release_string(s); }
    } released{c};
    return std::string(text(c));
  }
};
template <class T>
struct receive<T, std::enable_if_t<is_scalar_v<T>>> {
  T c{};
  explicit receive(const guatiao_alloc*) noexcept {}
  T* arg() noexcept { return &c; }
  void drop() noexcept {}
  result<T> take(const guatiao_alloc*) noexcept { return c; }
};
template <class K>
struct receive<object<K>> {
  guatiao_object c{};
  explicit receive(const guatiao_alloc*) noexcept {}
  guatiao_object* arg() noexcept { return &c; }
  void drop() noexcept { object<K>::adopt(c); }
  result<object<K>> take(const guatiao_alloc*);
};

template <bool Words, class R, class Fn, class... X>
guatiao_status invoke_c(Fn fn, void* ctx, receive<R>& out, guatiao_provider_error& err, X... x) {
  if constexpr (std::is_void_v<R>) {
    if constexpr (Words) return fn(ctx, x..., &err);
    else return fn(ctx, x...);
  } else {
    if constexpr (Words) return fn(ctx, x..., out.arg(), &err);
    else return fn(ctx, x..., out.arg());
  }
}

template <class Slot, class Self, class R, class... P, class... Args>
result<R> call_with(const Self& self, sig_tag<R(P...)>, Args&&... args) {
  static_assert(sizeof...(Args) == sizeof...(P), "guatiao: a slot called with the wrong number of arguments");
  static_assert((std::is_convertible_v<Args&&, P> && ...),
                "guatiao: an argument is not one the slot takes (a tree handed over is moved in: std::move)");
  // What is handed over is held from here: freed when the call never
  // happens, the callee's once it does.
  std::tuple<send<P>...> sends{send<P>{P(std::forward<Args>(args))}...};
  if (!self.usable()) return error_of(GUATIAO_ERR_WRONG_KIND, "the table is not one of this kind");
  auto fn = slot_fn<Slot>(self);
  if (!fn) return error_of(GUATIAO_ERR_UNSUPPORTED, std::string("the table has no ") + Slot::name);
  const guatiao_alloc* home = self.answer_alloc();
  receive<R> out(home);
  guatiao_provider_error err{};
  guatiao_status s = std::apply(
      [&](auto&... x) { return invoke_c<shape<Slot>::refuses>(fn, self.ctx(), out, err, x.arg()...); }, sends);
  // Answered or refused; the slot's own refusal wins over a lent tree's
  // settle, which is otherwise the call's error.
  guatiao_status settled = GUATIAO_OK;
  std::apply(
      [&](auto&... x) {
        [[maybe_unused]] auto first = [&](guatiao_status st) {
          if (settled == GUATIAO_OK) settled = st;
        };
        (first(x.after(home)), ...);
      },
      sends);
  words_guard words{err};
  if (s != GUATIAO_OK || settled != GUATIAO_OK) {
    struct drop_guard {
      receive<R>& o;
      ~drop_guard() { o.drop(); }
    } dropped{out};
    if (s != GUATIAO_OK) return error_of(s, std::string(text(err.message)));
    if (settled == GUATIAO_ERR_BAD_VALUE)
      return error_of(settled, std::string(Slot::name) +
                                   " wrote a key that is not UTF-8 into the map it was lent; the entry was removed");
    return error_of(settled, std::string("moving what ") + Slot::name + " put into a lent tree into that tree's allocator");
  }
  return out.take(home);
}

template <class Slot, class Self, class... Args>
auto try_call_slot(const Self& self, Args&&... args) {
  return call_with<Slot>(self, sig_tag<typename Slot::signature>{}, std::forward<Args>(args)...);
}

template <class T>
constexpr bool always_false = false;

/// A result's value, thrown when it failed; with exceptions off, only the
/// `try_` forms exist.
template <class T>
T or_throw(result<T>&& r) {
#if GUATIAO_EXCEPTIONS
  return std::move(r).value();
#else
  static_assert(always_false<T>, "guatiao: built without exceptions, call the try_ form");
  return std::move(*r);
#endif
}
inline void or_throw(result<void>&& r) {
#if GUATIAO_EXCEPTIONS
  r.value();
#else
  (void)r;
#endif
}

/// The answer by value: `value() &&` refers into the temporary result,
/// which is gone once this returns.
template <class Slot, class Self, class... Args>
typename shape<Slot>::answer call_slot(const Self& self, Args&&... args) {
#if GUATIAO_EXCEPTIONS
  return try_call_slot<Slot>(self, std::forward<Args>(args)...).value();
#else
  static_assert(always_false<Self>, "guatiao: built without exceptions, call the try_ form of the slot");
  return try_call_slot<Slot>(self, std::forward<Args>(args)...);
#endif
}

// ---- serving a slot: C arguments to C++ ------------------------------------------

template <class P, class = void>
struct take;
template <>
struct take<std::string_view> {
  guatiao_str c;
  guatiao_status check() const noexcept { return is_utf8(text(c)) ? GUATIAO_OK : GUATIAO_ERR_BAD_VALUE; }
  std::string_view get() noexcept { return text(c); }
  void finish() noexcept {}
};
template <>
struct take<bytes_view> {
  guatiao_bytes c;
  guatiao_status check() const noexcept { return GUATIAO_OK; }
  bytes_view get() noexcept { return bytes_view(c.len ? c.ptr : nullptr, c.len); }
  void finish() noexcept {}
};
template <>
struct take<bytes_mut> {
  guatiao_bytes_mut c;
  guatiao_status check() const noexcept { return GUATIAO_OK; }
  bytes_mut get() noexcept { return bytes_mut(c.len ? c.ptr : nullptr, c.len); }
  void finish() noexcept {}
};
template <class T>
struct take<T, std::enable_if_t<is_scalar_v<T>>> {
  T c;
  guatiao_status check() const noexcept { return GUATIAO_OK; }
  T get() noexcept { return c; }
  void finish() noexcept {}
};
template <>
struct take<view> {
  const guatiao_value* c;
  guatiao_status check() const noexcept { return GUATIAO_OK; }
  view get() noexcept { return view(c); }
  void finish() noexcept {}
};
template <>
struct take<map_view> {
  const guatiao_map* c;
  guatiao_status check() const noexcept { return GUATIAO_OK; }
  map_view get() noexcept { return map_view(c); }
  void finish() noexcept {}
};
template <>
struct take<list_view> {
  const guatiao_list* c;
  guatiao_status check() const noexcept { return GUATIAO_OK; }
  list_view get() noexcept { return list_view(c); }
  void finish() noexcept {}
};
/// `raw`, a tree handed over, moved into this side's default allocator
/// out of the caller's, which may go with the caller's library while the
/// method keeps it; `moved` answers a refusal, which leaves it whole.
inline guatiao_value& homed(guatiao_value& raw, guatiao_status& moved) noexcept {
  moved = settle(raw, default_alloc());
  return raw;
}
inline guatiao_map homed(guatiao_map& m, guatiao_status& moved) noexcept {
  guatiao_value v = blank();
  v.tag = GUATIAO_MAP;
  v.payload.map = m;
  return m = homed(v, moved).payload.map;
}
inline guatiao_list homed(guatiao_list& l, guatiao_status& moved) noexcept {
  guatiao_value v = blank();
  v.tag = GUATIAO_LIST;
  v.payload.list = l;
  return l = homed(v, moved).payload.list;
}
/// A tree handed over: owned from the first moment, so a refused call
/// frees it, and in this side's allocator, so the method may keep it.
template <>
struct take<value> {
  guatiao_status moved = GUATIAO_OK;
  value v;
  explicit take(guatiao_value* p) noexcept : v(p ? value::adopt(homed(*p, moved)) : value()) {}
  guatiao_status check() const noexcept { return moved; }
  value&& get() noexcept { return std::move(v); }
  void finish() noexcept {}
};
template <>
struct take<map> {
  guatiao_status moved = GUATIAO_OK;
  map v;
  bool given;
  explicit take(guatiao_map* p) noexcept
      : v(p ? map::adopt(homed(*p, moved)) : map(nullptr)), given(p != nullptr) {}
  guatiao_status check() const noexcept { return given ? moved : GUATIAO_ERR_NULL; }
  map&& get() noexcept { return std::move(v); }
  void finish() noexcept {}
};
template <>
struct take<list> {
  guatiao_status moved = GUATIAO_OK;
  list v;
  bool given;
  explicit take(guatiao_list* p) noexcept
      : v(p ? list::adopt(homed(*p, moved)) : list(nullptr)), given(p != nullptr) {}
  guatiao_status check() const noexcept { return given ? moved : GUATIAO_ERR_NULL; }
  list&& get() noexcept { return std::move(v); }
  void finish() noexcept {}
};
/// A tree lent: the method changes it in place; what it built through
/// another allocator is moved into the one the tree records afterwards.
template <>
struct take<node> {
  guatiao_value* c;
  guatiao_status check() const noexcept { return c ? GUATIAO_OK : GUATIAO_ERR_NULL; }
  node get() noexcept { return node(c, default_alloc()); }
  void finish() noexcept {
    if (!c) return;
    const guatiao_alloc* a = recorded(*c);
    (void)settle(*c, a ? a : default_alloc());
  }
};
/// A map lent as its container: changed as a node standing in for it, then
/// written back, whatever the method answered.
template <>
struct take<map_ref> {
  guatiao_map* c;
  guatiao_value stand_in = blank();
  explicit take(guatiao_map* p) noexcept : c(p) {
    if (!p) return;
    stand_in.tag = GUATIAO_MAP;
    stand_in.payload.map = *p;
  }
  guatiao_status check() const noexcept { return c ? GUATIAO_OK : GUATIAO_ERR_NULL; }
  map_ref get() noexcept { return map_ref(node(&stand_in, default_alloc())); }
  /// Writes the map back, whatever the method did: one it replaced with
  /// another kind (through `as_node()`) has freed the caller's buffer, so
  /// the caller gets an empty map recording its allocator, and what
  /// replaced it is freed here.
  void finish() noexcept {
    if (!c) return;
    if (stand_in.tag != GUATIAO_MAP) {
      const guatiao_alloc* a = c->alloc;
      guatiao_value_free(&stand_in);
      *c = guatiao_map{nullptr, 0, 0, a};
      return;
    }
    const guatiao_alloc* a = recorded(stand_in);
    (void)settle(stand_in, a ? a : default_alloc());
    if (stand_in.tag == GUATIAO_MAP) *c = stand_in.payload.map;
  }
};
template <>
struct take<list_ref> {
  guatiao_list* c;
  guatiao_value stand_in = blank();
  explicit take(guatiao_list* p) noexcept : c(p) {
    if (!p) return;
    stand_in.tag = GUATIAO_LIST;
    stand_in.payload.list = *p;
  }
  guatiao_status check() const noexcept { return c ? GUATIAO_OK : GUATIAO_ERR_NULL; }
  list_ref get() noexcept { return list_ref(node(&stand_in, default_alloc())); }
  /// As take<map_ref>::finish.
  void finish() noexcept {
    if (!c) return;
    if (stand_in.tag != GUATIAO_LIST) {
      const guatiao_alloc* a = c->alloc;
      guatiao_value_free(&stand_in);
      *c = guatiao_list{nullptr, 0, 0, a};
      return;
    }
    const guatiao_alloc* a = recorded(stand_in);
    (void)settle(stand_in, a ? a : default_alloc());
    if (stand_in.tag == GUATIAO_LIST) *c = stand_in.payload.list;
  }
};
template <class K>
struct take<object<K>> {
  object<K> v;
  explicit take(guatiao_object raw) noexcept : v(object<K>::adopt(raw)) {}
  guatiao_status check() const noexcept { return v.usable() ? GUATIAO_OK : GUATIAO_ERR_WRONG_KIND; }
  object<K>&& get() noexcept { return std::move(v); }
  void finish() noexcept {}
};
template <class K>
struct take<lent<K>> {
  lent<K> v;
  explicit take(const guatiao_object* p) noexcept : v(p) {}
  guatiao_status check() const noexcept { return v.usable() ? GUATIAO_OK : GUATIAO_ERR_WRONG_KIND; }
  lent<K> get() noexcept { return v; }
  void finish() noexcept {}
};

/// Calls the method, catching what it throws: a guatiao::error crosses as
/// its status and words, std::bad_alloc as ALLOC, anything else as
/// INTERNAL. Nothing unwinds past this.
template <class Slot, class R, class P, class... X>
result<R> run_method(P& p, X&&... x) noexcept {
#if GUATIAO_EXCEPTIONS
  try {
    try {
#endif
      using M = decltype(Slot::invoke(p, std::forward<X>(x)...));
      if constexpr (is_result<M>::value) {
        return Slot::invoke(p, std::forward<X>(x)...);
      } else if constexpr (std::is_void_v<R>) {
        Slot::invoke(p, std::forward<X>(x)...);
        return {};
      } else {
        return result<R>(R(Slot::invoke(p, std::forward<X>(x)...)));
      }
#if GUATIAO_EXCEPTIONS
    } catch (const error& e) {
      return error(e.code() == status::ok ? status::internal : e.code(), e.message());
    } catch (const std::bad_alloc&) {
      return error(status::alloc, "out of memory");
    } catch (const std::exception& e) {
      return error(status::internal, e.what());
    } catch (...) {
      return error(status::internal, "an exception that is not a std::exception");
    }
  } catch (...) {
    // Building the words threw: the status crosses alone.
    return error(status::internal);
  }
#endif
}

/// Writes an answer over the out slot without freeing what it holds (a
/// carrier owns nothing): a tree moved into the allocator the carrier
/// names, else this binary's default.
inline const guatiao_alloc* carried(const guatiao_value* out) noexcept {
  if (!out) return default_alloc();
  const guatiao_alloc* a = nullptr;
  if (out->tag == GUATIAO_ABSENT) a = out->payload.map.alloc;
  else if (out->tag == GUATIAO_MAP && !out->payload.map.len && !out->payload.map.cap) a = out->payload.map.alloc;
  else if (out->tag == GUATIAO_LIST && !out->payload.list.len && !out->payload.list.cap) a = out->payload.list.alloc;
  return usable_alloc(a) ? a : default_alloc();
}

inline guatiao_status put(guatiao_value* out, value&& v) noexcept {
  const guatiao_alloc* home = carried(out);
  guatiao_value tree = v.release();
  guatiao_status s = settle(tree, home);
  if (s != GUATIAO_OK || !out) {
    guatiao_value_free(&tree);
    return out ? s : GUATIAO_ERR_NULL;
  }
  *out = tree;
  return GUATIAO_OK;
}
inline guatiao_status put(guatiao_map* out, map&& m) noexcept {
  const guatiao_alloc* a = out && !out->len && !out->cap ? out->alloc : nullptr;
  const guatiao_alloc* home = usable_alloc(a) ? a : default_alloc();
  guatiao_value tree = std::move(m).into_value().release();
  guatiao_status s = settle(tree, home);
  if (s != GUATIAO_OK || !out || tree.tag != GUATIAO_MAP) {
    guatiao_value_free(&tree);
    return !out ? GUATIAO_ERR_NULL : s != GUATIAO_OK ? s : GUATIAO_ERR_INTERNAL;
  }
  *out = tree.payload.map;
  return GUATIAO_OK;
}
inline guatiao_status put(guatiao_list* out, list&& l) noexcept {
  const guatiao_alloc* a = out && !out->len && !out->cap ? out->alloc : nullptr;
  const guatiao_alloc* home = usable_alloc(a) ? a : default_alloc();
  guatiao_value tree = std::move(l).into_value().release();
  guatiao_status s = settle(tree, home);
  if (s != GUATIAO_OK || !out || tree.tag != GUATIAO_LIST) {
    guatiao_value_free(&tree);
    return !out ? GUATIAO_ERR_NULL : s != GUATIAO_OK ? s : GUATIAO_ERR_INTERNAL;
  }
  *out = tree.payload.list;
  return GUATIAO_OK;
}
inline guatiao_status put(guatiao_string* out, std::string&& s) noexcept {
  if (!out) return GUATIAO_ERR_NULL;
  if (!is_utf8(s)) return GUATIAO_ERR_BAD_VALUE;
  return make_string(default_alloc(), s, *out) ? GUATIAO_OK : GUATIAO_ERR_ALLOC;
}
template <class T, std::enable_if_t<is_scalar_v<T>, int> = 0>
guatiao_status put(T* out, T v) noexcept {
  if (!out) return GUATIAO_ERR_NULL;
  *out = v;
  return GUATIAO_OK;
}
template <class K>
guatiao_status put(guatiao_object* out, object<K>&& o) noexcept {
  if (!out) return GUATIAO_ERR_NULL;
  *out = o.release();
  return GUATIAO_OK;
}

template <class Slot, class P, class R, class... A, class... C, std::size_t... I>
guatiao_status serve(void* ctx, std::tuple<C...> c, sig_tag<R(A...)>, std::index_sequence<I...>) noexcept {
  constexpr bool words = shape<Slot>::refuses;
  guatiao_provider_error* err = nullptr;
  if constexpr (words) err = std::get<sizeof...(C) - 1>(c);
  // Handed-over trees and objects are owned from here, so every refusal
  // below frees them.
  std::tuple<take<A>...> args{take<A>{std::get<I>(c)}...};
  if (!ctx) {
    write_words(err, GUATIAO_ERR_NULL, "no provider behind this ctx");
    return GUATIAO_ERR_NULL;
  }
  guatiao_status refused = GUATIAO_OK;
  ((refused = refused != GUATIAO_OK ? refused : std::get<I>(args).check()), ...);
  if (refused != GUATIAO_OK) {
    write_words(err, refused,
                refused == GUATIAO_ERR_BAD_VALUE    ? "text that is not UTF-8"
                : refused == GUATIAO_ERR_WRONG_KIND ? "an object that is not of its kind"
                : refused == GUATIAO_ERR_ALLOC      ? "a tree handed over could not be moved into this side's allocator"
                                                    : "a required argument was null");
    return refused;
  }
  result<R> r = run_method<Slot, R>(*static_cast<P*>(ctx), std::get<I>(args).get()...);
  (std::get<I>(args).finish(), ...);
  if (!r.ok()) {
    write_words(err, static_cast<guatiao_status>(r.code()), r.error().message());
    return static_cast<guatiao_status>(r.code());
  }
  if constexpr (!std::is_void_v<R>) {
    guatiao_status s = put(std::get<sizeof...(A)>(c), std::move(*r));
    if (s != GUATIAO_OK) {
      write_words(err, s, "the answer could not be written");
      return s;
    }
  }
  return GUATIAO_OK;
}

/// The C function a slot of a class `P` is served by.
template <class Slot, class P, int Mode, class Fn = typename Slot::c_type>
struct server;
template <class Slot, class P, int Mode, class... C>
struct server<Slot, P, Mode, guatiao_status (*)(void*, C...)> {
  static guatiao_status call(void* ctx, C... c) noexcept {
    return serve<Slot, P>(ctx, std::tuple<C...>(c...), sig_tag<typename Slot::signature>{},
                          std::make_index_sequence<sig_arity<typename Slot::signature>::value>{});
  }
};

template <class T>
void destroy_owned(void* ctx) noexcept {
  delete static_cast<T*>(ctx);
  live().fetch_sub(1);
}
inline void destroy_nothing(void*) noexcept {}

template <class K, class T, int Mode, bool Object>
struct destroy_slot {
  static constexpr void put(typename K::table_type&) noexcept {}
};
template <class K, class T>
struct destroy_slot<K, T, as_owned, true> {
  static constexpr void put(typename K::table_type& t) noexcept { t.destroy = &destroy_owned<T>; }
};
template <class K, class T>
struct destroy_slot<K, T, as_lent, true> {
  static constexpr void put(typename K::table_type& t) noexcept { t.destroy = &destroy_nothing; }
};

/// The table class `T` serves kind `K` through, one per binary.
template <class K, class T, int Mode>
inline constexpr typename K::table_type served_table = K::template make_table<T, Mode>();

}  // namespace detail

// ---- handles on a table -------------------------------------------------------

/// A provider's table of kind `K` and the `ctx` its slots take, with a
/// method per slot (`r.greet("ana")`, `r.try_greet(...)`,
/// `r.has_shout()`). Answers arrive in `alloc`. Valid while the provider's
/// library is open (until it is unloaded, or collected once retired).
template <class K>
class remote : public K::template calls<remote<K>> {
  static_assert(!K::is_object, "guatiao: an object kind is held as object<K> or lent<K>");

 public:
  remote() noexcept = default;
  remote(const void* table, std::size_t size, void* ctx, const guatiao_alloc* alloc = default_alloc()) noexcept
      : table_(table), size_(size), ctx_(ctx), alloc_(alloc) {}

  const void* table() const noexcept { return table_; }
  std::size_t table_size() const noexcept { return size_; }
  void* ctx() const noexcept { return ctx_; }
  const guatiao_alloc* answer_alloc() const noexcept { return alloc_; }
  /// Whether the table is one of kind `K`.
  bool usable() const noexcept { return detail::table_of<K>(table_, size_); }

 private:
  const void* table_ = nullptr;
  std::size_t size_ = 0;
  void* ctx_ = nullptr;
  const guatiao_alloc* alloc_ = nullptr;
};

/// An object of kind `K` this side owns: destroyed through its table's
/// `destroy` when this handle goes, or handed over by moving it into a
/// call. Move-only. `object<K>::make<T>(...)` serves a C++ class as one.
template <class K>
class object : public K::template calls<object<K>> {
  static_assert(K::is_object, "guatiao: object<K> is for an object kind");

 public:
  object() noexcept = default;
  object(object&& o) noexcept : raw_(o.release()) {}
  object& operator=(object&& o) noexcept {
    if (this != &o) {
      reset();
      raw_ = o.release();
    }
    return *this;
  }
  object(const object&) = delete;
  object& operator=(const object&) = delete;
  ~object() { reset(); }

  /// Takes over an object a call handed over.
  static object adopt(const guatiao_object& raw) noexcept {
    object o;
    o.raw_ = raw;
    return o;
  }
  /// A new `T`, served as an object of kind `K`: its methods are the
  /// slots, its destructor runs when the holder destroys it.
  template <class T, class... Args>
  static object make(Args&&... args) {
    return of(std::unique_ptr<T>(new T(std::forward<Args>(args)...)));
  }
  template <class T>
  static object of(std::unique_ptr<T> p) noexcept {
    if (!p) return object();
    detail::live().fetch_add(1);
    guatiao_object raw;
    raw.table = &detail::served_table<K, T, detail::as_owned>;
    raw.size = sizeof(typename K::table_type);
    raw.ctx = p.release();
    return adopt(raw);
  }
  /// Gives the object up: the caller destroys or hands it over.
  guatiao_object release() noexcept {
    guatiao_object out = raw_;
    raw_ = guatiao_object{};
    return out;
  }
  /// Destroys the object now.
  void reset() noexcept {
    guatiao_object o = release();
    void (*destroy)(void*) = nullptr;
    if (o.table && o.size >= sizeof(guatiao_kind_header) + sizeof destroy) {
      std::memcpy(&destroy, static_cast<const char*>(o.table) + sizeof(guatiao_kind_header), sizeof destroy);
      if (destroy) destroy(o.ctx);
    }
  }

  const guatiao_object& raw() const noexcept { return raw_; }
  explicit operator bool() const noexcept { return raw_.table != nullptr; }
  const void* table() const noexcept { return raw_.table; }
  std::size_t table_size() const noexcept { return raw_.size; }
  void* ctx() const noexcept { return raw_.ctx; }
  const guatiao_alloc* answer_alloc() const noexcept { return default_alloc(); }
  bool usable() const noexcept { return detail::table_of<K>(raw_.table, raw_.size); }

 private:
  guatiao_object raw_{};
};

/// An object of kind `K` lent for one call: its slots called through, never
/// destroyed or kept. Lend a C++ object (`lent<listener>(my_ear)`) or an
/// owned one (`lent<listener>(an_object)`); a lent argument a slot
/// receives arrives as one.
template <class K>
class lent : public K::template calls<lent<K>> {
  static_assert(K::is_object, "guatiao: lent<K> is for an object kind");

 public:
  lent() noexcept = default;
  explicit lent(const guatiao_object* raw) noexcept : raw_(raw ? *raw : guatiao_object{}) {}
  lent(const object<K>& o) noexcept : raw_(o.raw()) {}
  template <class T,
            std::enable_if_t<!std::is_same_v<std::decay_t<T>, lent> && !detail::is_object_handle<std::decay_t<T>>::value &&
                                 !std::is_pointer_v<T>,
                             int> = 0>
  lent(T& served) noexcept {
    raw_.table = &detail::served_table<K, T, detail::as_lent>;
    raw_.size = sizeof(typename K::table_type);
    raw_.ctx = &served;
  }

  const guatiao_object& raw() const noexcept { return raw_; }
  const void* table() const noexcept { return raw_.table; }
  std::size_t table_size() const noexcept { return raw_.size; }
  void* ctx() const noexcept { return raw_.ctx; }
  const guatiao_alloc* answer_alloc() const noexcept { return default_alloc(); }
  bool usable() const noexcept { return detail::table_of<K>(raw_.table, raw_.size); }

 private:
  guatiao_object raw_{};
};

namespace detail {
template <class K>
result<object<K>> receive<object<K>>::take(const guatiao_alloc*) {
  object<K> o = object<K>::adopt(c);
  c = guatiao_object{};
  if (!o) return error_of(GUATIAO_ERR_NULL, "no object was answered");
  if (!o.usable()) return error_of(GUATIAO_ERR_WRONG_KIND, "the object answered is not of its kind");
  return o;
}
}  // namespace detail

/// A table of kind `K` and its `ctx`; WRONG_KIND for a table of another
/// kind (another floor hash, or too short for its required slots).
template <class K>
result<remote<K>> try_bind_table(const void* table, std::size_t size, void* ctx,
                                 const guatiao_alloc* alloc = default_alloc()) {
  if (!table) return detail::error_of(GUATIAO_ERR_NOT_FOUND, std::string("no table of kind ") + K::kind_name);
  if (!detail::table_of<K>(table, size))
    return detail::error_of(GUATIAO_ERR_WRONG_KIND, std::string("a table that is not of kind ") + K::kind_name);
  return remote<K>(table, size, ctx, alloc);
}

/// The provider `key` of a registry, as kind `K`: NOT_FOUND when it does
/// not serve the kind through a table.
template <class K>
result<remote<K>> try_bind(const registry& r, std::string_view key) {
  auto t = r.provider_table(key, K::kind_name);
  return try_bind_table<K>(t.first, t.second, r.provider_ctx(key), r.alloc());
}

/// An instance a registry built, as kind `K`.
template <class K>
result<remote<K>> try_bind(const instance& i) {
  if (!i) return detail::error_of(GUATIAO_ERR_NULL, "the instance is closed");
  std::size_t size = 0;
  const void* t = guatiao_registry_provider_table(i.registry_handle(), detail::str(i.key()), detail::str(K::kind_name), &size);
  return try_bind_table<K>(t, size, i.ctx(), i.alloc());
}

}  // namespace GUATIAO_ABI
}  // namespace guatiao

#endif  // GUATIAO_KIND_HPP
