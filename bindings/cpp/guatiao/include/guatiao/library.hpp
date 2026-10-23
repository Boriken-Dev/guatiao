// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// guatiao for C++: a library of providers written in C++. A function
// answers a `library` (its id, version, meta and providers); either a
// host linking it registers it (`registry::register_library<f>(name)`),
// or GUATIAO_LIBRARY(f, kinds...) makes the shared library holding it one
// any host's scan finds, exporting `guatiao_library_entry` and the
// `guatiao_declares` a scan reads before mapping the file. Part of
// <guatiao/guatiao.hpp>.

#ifndef GUATIAO_LIBRARY_HPP
#define GUATIAO_LIBRARY_HPP

#include <guatiao/kind.hpp>

#include <memory>

#if defined(_WIN32)
#define GUATIAO_EXPORT __declspec(dllexport)
#else
#define GUATIAO_EXPORT __attribute__((visibility("default")))
#endif

/// Makes this shared library a guatiao library: `make` is a function
/// `guatiao::library make()`, called once, the first time a host asks;
/// the kinds listed after it are what the file declares to a scan
/// (`kind=greeter`), read before the file is mapped. Use it once per
/// shared library, at namespace scope.
#define GUATIAO_LIBRARY(make, ...)                                                                         \
  namespace {                                                                                              \
  /* Keys this file's library by a type no other shared library can name. */                              \
  struct guatiao_library_here {};                                                                          \
  }                                                                                                        \
  extern "C" GUATIAO_EXPORT const ::guatiao::detail::declaration<::guatiao::detail::declares_size<__VA_ARGS__>()> \
      guatiao_declares = ::guatiao::detail::declares_of<__VA_ARGS__>();                                    \
  extern "C" GUATIAO_EXPORT const guatiao_library_info* guatiao_library_entry(const guatiao_host_info* host) { \
    return ::guatiao::detail::library_entry<&make, guatiao_library_here>(host);                            \
  }

namespace guatiao {
inline namespace GUATIAO_ABI {

class library;

namespace detail {

/// The bytes `guatiao_declares` holds: `kind=<name>\0` per kind.
template <std::size_t N>
struct declaration {
  char text[N];
};

template <class... Ks>
constexpr std::size_t declares_size() noexcept {
  static_assert(sizeof...(Ks) > 0, "guatiao: GUATIAO_LIBRARY names the kinds the library declares");
  return ((sizeof("kind=") - 1 + std::char_traits<char>::length(Ks::kind_name) + 1) + ... + 0) + 1;
}

template <class... Ks>
constexpr declaration<declares_size<Ks...>()> declares_of() noexcept {
  declaration<declares_size<Ks...>()> d{};
  std::size_t at = 0;
  auto put = [&](const char* s) {
    for (std::size_t i = 0; s[i]; ++i) d.text[at++] = s[i];
  };
  ((put("kind="), put(Ks::kind_name), d.text[at++] = '\0'), ...);
  d.text[at] = '\0';
  return d;
}

/// A library's say in being unmapped: refused while anything this binary
/// serves (an instance, an object) is alive.
inline guatiao_status unload_slot() noexcept { return live().load() > 0 ? GUATIAO_ERR_BUSY : GUATIAO_OK; }

/// A provider class's `unavailable()` (a `const char *`, null when it can
/// run here), as the descriptor's `available` slot.
template <class P, class = void>
struct has_unavailable : std::false_type {};
template <class P>
struct has_unavailable<P, std::void_t<decltype(std::declval<const P&>().unavailable())>> : std::true_type {};

template <class P>
bool available_slot(void* ctx, guatiao_str* reason) noexcept {
  if (!ctx) return true;
  const char* why = nullptr;
#if GUATIAO_EXCEPTIONS
  try {
    why = static_cast<const P*>(ctx)->unavailable();
  } catch (...) {
    why = "its availability could not be asked";
  }
#else
  why = static_cast<const P*>(ctx)->unavailable();
#endif
  if (!why) return true;
  if (reason) *reason = str(why);
  return false;
}

/// An instance built from a configuration: `P::make(view)` when the class
/// has one (answering a `std::unique_ptr<P>` or a `result` of one), else
/// `new P(view)`.
template <class P, class = void>
struct has_make : std::false_type {};
template <class P>
struct has_make<P, std::void_t<decltype(P::make(std::declval<view>()))>> : std::true_type {};

template <class P>
result<std::unique_ptr<P>> build_instance(view config) noexcept {
#if GUATIAO_EXCEPTIONS
  try {
    try {
#endif
      if constexpr (has_make<P>::value) {
        using M = decltype(P::make(config));
        if constexpr (is_result<M>::value) return P::make(config);
        else return std::unique_ptr<P>(P::make(config));
      } else {
        return std::unique_ptr<P>(new P(config));
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
    return error(status::internal);
  }
#endif
}

template <class P>
guatiao_status create_slot(void*, const guatiao_value* config, void** out, guatiao_provider_error* err) noexcept {
  if (!out) return GUATIAO_ERR_NULL;
  *out = nullptr;
  result<std::unique_ptr<P>> made = build_instance<P>(view(config));
  if (!made.ok()) {
    write_words(err, static_cast<guatiao_status>(made.code()), made.error().message());
    return static_cast<guatiao_status>(made.code());
  }
  if (!*made) {
    write_words(err, GUATIAO_ERR_INTERNAL, "no instance was built");
    return GUATIAO_ERR_INTERNAL;
  }
  live().fetch_add(1);
  *out = made->release();
  return GUATIAO_OK;
}

template <class P>
void destroy_slot_for(void*, void* instance) noexcept {
  if (!instance) return;
  delete static_cast<P*>(instance);
  live().fetch_sub(1);
}

/// The C++ library a function answers, built once, its descriptor
/// complete before any host reads it; null when the function failed.
/// `Here` is a type of the file that makes the library, so two shared
/// libraries whose functions share a name keep a library each whatever
/// the platform's loader merges.
template <library (*Make)(), class Here>
library* the_library() noexcept;

template <library (*Make)(), class Here>
const guatiao_library_info* library_entry(const guatiao_host_info* host) noexcept;

template <class P, class K>
guatiao_kind_table table_entry() noexcept {
  static_assert(!K::is_object, "guatiao: a provider serves provider kinds; an object kind is served by object<K>");
  guatiao_kind_table t;
  t.struct_size = static_cast<std::uint32_t>(sizeof(guatiao_kind_table));
  t.vtable_size = static_cast<std::uint32_t>(sizeof(typename K::table_type));
  t.kind = str(K::kind_name);
  t.vtable = &served_table<K, P, as_provider>;
  return t;
}

}  // namespace detail

/// A library of providers: built by a function, read by a host once it is
/// finished. Its id and version are what the library declares (semver);
/// each provider's id is `{library id}_{name}` unless set. What it holds
/// (its meta, a provider's schema) is built in `system_alloc()`, alive as
/// long as the library; a host reads and copies it.
class library {
  struct entry {
    std::string name, id, display, version;
    std::vector<guatiao_str> kinds;
    std::vector<guatiao_kind_table> tables;
    std::shared_ptr<void> ctx;
    value config;
    value meta;
    bool (*available)(void*, guatiao_str*) = nullptr;
    guatiao_status (*create)(void*, const guatiao_value*, void**, guatiao_provider_error*) = nullptr;
    void (*destroy)(void*, void*) = nullptr;
  };

 public:
  library(std::string id, std::string version) : id_(std::move(id)), version_(std::move(version)) {}
  /// A moved library is laid out again by `finish()`: its descriptor
  /// pointed into the strings and values it held before.
  library(library&& o) noexcept
      : id_(std::move(o.id_)),
        version_(std::move(o.version_)),
        meta_(std::move(o.meta_)),
        entries_(std::move(o.entries_)),
        orphans_(std::move(o.orphans_)) {
    o.finished_ = false;
  }
  library& operator=(library&& o) noexcept {
    if (this != &o) {
      id_ = std::move(o.id_);
      version_ = std::move(o.version_);
      meta_ = std::move(o.meta_);
      entries_ = std::move(o.entries_);
      orphans_ = std::move(o.orphans_);
      infos_.clear();
      info_ = guatiao_library_info{};
      finished_ = false;
      o.finished_ = false;
    }
    return *this;
  }
  library(const library&) = delete;
  library& operator=(const library&) = delete;

  /// Anything else the library says, as a map (a summary, a licence,
  /// notices): read and copied by a host.
  library& meta(const item& m) {
    meta_ = guatiao::try_make(m, system_alloc()).value_or(value());
    return *this;
  }

  /// What `single` and `configured` answer: the provider just added, to
  /// say more about.
  class provider_ref {
   public:
    provider_ref& id(std::string_view v) { return p_->id = std::string(v), *this; }
    provider_ref& display_name(std::string_view v) { return p_->display = std::string(v), *this; }
    /// Its own version, when it does not move with the library's.
    provider_ref& version(std::string_view v) { return p_->version = std::string(v), *this; }
    provider_ref& meta(const item& m) {
      return p_->meta = guatiao::try_make(m, system_alloc()).value_or(value()), *this;
    }

   private:
    friend class library;
    explicit provider_ref(entry* p) noexcept : p_(p) {}
    entry* p_;
  };

  /// A provider that is its one instance: a `P` built from `args`, its
  /// methods serving the kinds `Ks...`.
  template <class P, class... Ks, class... Args>
  provider_ref single(std::string_view name, Args&&... args) {
    return single_of<P, Ks...>(name, std::unique_ptr<P>(new P(std::forward<Args>(args)...)));
  }
  template <class P, class... Ks>
  provider_ref single_of(std::string_view name, std::unique_ptr<P> instance) {
    static_assert(sizeof...(Ks) > 0, "guatiao: a provider names the kinds it serves");
    auto e = std::make_unique<entry>();
    e->name = std::string(name);
    e->kinds = {detail::str(Ks::kind_name)...};
    e->tables = {detail::table_entry<P, Ks>()...};
    e->ctx = std::shared_ptr<void>(instance.release(), [](void* p) { delete static_cast<P*>(p); });
    if constexpr (detail::has_unavailable<P>::value) e->available = &detail::available_slot<P>;
    return add(std::move(e));
  }
  /// A provider building an instance per configuration: `P::make(view)` or
  /// `P(view)` from a configuration the host checked against `schema`
  /// (`P::schema()` when not given). A refusal it throws or answers
  /// reaches the host with its words.
  template <class P, class... Ks>
  provider_ref configured(std::string_view name) {
    return configured<P, Ks...>(name, P::schema());
  }
  template <class P, class... Ks>
  provider_ref configured(std::string_view name, const item& schema) {
    static_assert(sizeof...(Ks) > 0, "guatiao: a provider names the kinds it serves");
    auto e = std::make_unique<entry>();
    e->name = std::string(name);
    e->kinds = {detail::str(Ks::kind_name)...};
    e->tables = {detail::table_entry<P, Ks>()...};
    e->config = guatiao::try_make(schema, system_alloc()).value_or(value());
    e->create = &detail::create_slot<P>;
    e->destroy = &detail::destroy_slot_for<P>;
    return add(std::move(e));
  }

  const std::string& id() const noexcept { return id_; }
  const std::string& version() const noexcept { return version_; }

  /// The descriptor a host reads: built by `finish()`, valid while this
  /// library lives, and never changed after.
  const guatiao_library_info* info() const noexcept { return finished_ ? &info_ : nullptr; }

  /// Lays the descriptor out, once the library is where it stays (the
  /// descriptor points into it). Nothing is added after.
  void finish() {
    if (finished_) return;
    infos_.assign(entries_.size(), guatiao_provider_info{});
    for (std::size_t i = 0; i < entries_.size(); ++i) {
      entry& e = *entries_[i];
      if (e.id.empty()) e.id = id_ + "_" + e.name;
      guatiao_provider_info& p = infos_[i];
      p.struct_size = static_cast<std::uint32_t>(sizeof(guatiao_provider_info));
      p.kinds.ptr = e.kinds.data();
      p.kinds.len = e.kinds.size();
      p.id = detail::str(e.id);
      p.display_name = detail::str(e.display);
      p.config = e.config.is_absent() ? nullptr : e.config.raw();
      p.ctx = e.ctx.get();
      p.meta = e.meta.is_map() ? &e.meta.raw()->payload.map : nullptr;
      p.version = detail::str(e.version);
      p.available = e.available;
      p.tables.ptr = e.tables.data();
      p.tables.len = e.tables.size();
      p.tables.stride = sizeof(guatiao_kind_table);
      p.create = e.create;
      p.destroy = e.destroy;
    }
    info_ = guatiao_library_info{};
    info_.struct_size = static_cast<std::uint32_t>(sizeof(guatiao_library_info));
    info_.abi_version = GUATIAO_ABI_VERSION;
    info_.id = detail::str(id_);
    info_.version = detail::str(version_);
    info_.providers.ptr = infos_.data();
    info_.providers.len = infos_.size();
    info_.providers.stride = sizeof(guatiao_provider_info);
    info_.meta = meta_.is_map() ? &meta_.raw()->payload.map : nullptr;
    info_.unload = &detail::unload_slot;
    finished_ = true;
  }

 private:
  provider_ref add(std::unique_ptr<entry> e) {
    entry* raw = e.get();
    if (!finished_) entries_.push_back(std::move(e));
    else orphans_.push_back(std::move(e));
    return provider_ref(raw);
  }

  std::string id_, version_;
  value meta_;
  std::vector<std::unique_ptr<entry>> entries_;
  // What was added after finish(): kept so its provider_ref stays valid,
  // never described.
  std::vector<std::unique_ptr<entry>> orphans_;
  std::vector<guatiao_provider_info> infos_;
  guatiao_library_info info_{};
  bool finished_ = false;
};

namespace detail {

template <library (*Make)(), class Here>
library* the_library() noexcept {
  static std::unique_ptr<library> lib = []() noexcept -> std::unique_ptr<library> {
#if GUATIAO_EXCEPTIONS
    try {
#endif
      auto made = std::make_unique<library>(Make());
      made->finish();
      return made;
#if GUATIAO_EXCEPTIONS
    } catch (...) {
      return nullptr;
    }
#endif
  }();
  return lib.get();
}

template <library (*Make)(), class Here>
const guatiao_library_info* library_entry(const guatiao_host_info* host) noexcept {
  if (!host || host->abi_version != GUATIAO_ABI_VERSION) return nullptr;
  library* lib = the_library<Make, Here>();
  return lib ? lib->info() : nullptr;
}

}  // namespace detail

}  // namespace GUATIAO_ABI
}  // namespace guatiao

#endif  // GUATIAO_LIBRARY_HPP
