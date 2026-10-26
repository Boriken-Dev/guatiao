// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// guatiao for C++: a host's registry of libraries — loading, scanning,
// sources and refresh(), ranking, retiring and unloading — and the
// instances a provider builds from a configuration. Part of
// <guatiao/guatiao.hpp>.

#ifndef GUATIAO_REGISTRY_HPP
#define GUATIAO_REGISTRY_HPP

#include <guatiao/value.hpp>

namespace guatiao {
inline namespace GUATIAO_ABI {

class registry;
class library;

namespace detail {
template <library (*Make)(), class Here>
const guatiao_library_info* library_entry(const guatiao_host_info* host) noexcept;
/// The key of a library this program registers itself.
struct registered_here {};
}  // namespace detail

/// Rules a scan applies to what each library declares before it is
/// mapped, one `[!]KEY=VALUE` each: `rules({"kind=greeter"})`.
inline std::string rules(std::initializer_list<std::string_view> each) {
  std::string out;
  for (std::string_view r : each) {
    if (!out.empty()) out.push_back('\n');
    out.append(r.data(), r.size());
  }
  return out;
}

namespace detail {

/// The value a registry call wrote, or its status.
inline result<value> answered(guatiao_status s, guatiao_value& out, const char* why) {
  if (s != GUATIAO_OK) {
    guatiao_value_free(&out);
    return error_of(s, why);
  }
  return value::adopt(out);
}

/// Releases a provider error's words when it goes, whatever happened
/// before: copying them can throw.
struct words_guard {
  guatiao_provider_error& err;
  ~words_guard() { release_string(err.message); }
};

/// A provider's words in a guatiao_provider_error, taken and released.
inline std::string take_message(guatiao_provider_error& err) {
  words_guard g{err};
  return std::string(text(err.message));
}

}  // namespace detail

/// One provider instance a registry built from a configuration: its
/// provider's `ctx` for calls on it. Destroyed through the provider that
/// built it, even after its library was retired or refreshed; it keeps
/// that library open until then. Move-only, and closed before its
/// registry is freed.
class instance {
 public:
  instance() noexcept = default;
  instance(instance&& o) noexcept : reg_(o.reg_), key_(std::move(o.key_)), ctx_(o.ctx_), alloc_(o.alloc_) {
    o.ctx_ = nullptr;
  }
  instance& operator=(instance&& o) noexcept {
    if (this != &o) {
      close();
      reg_ = o.reg_;
      key_ = std::move(o.key_);
      ctx_ = o.ctx_;
      alloc_ = o.alloc_;
      o.ctx_ = nullptr;
    }
    return *this;
  }
  instance(const instance&) = delete;
  instance& operator=(const instance&) = delete;
  ~instance() { close(); }

  /// Destroys the instance now; later calls through it are refused.
  void close() noexcept {
    if (ctx_ && reg_) guatiao_registry_provider_destroy(reg_, detail::str(key_), ctx_);
    ctx_ = nullptr;
  }
  explicit operator bool() const noexcept { return ctx_ != nullptr; }
  void* ctx() const noexcept { return ctx_; }
  const std::string& key() const noexcept { return key_; }
  const guatiao_registry* registry_handle() const noexcept { return reg_; }
  /// The allocator its registry builds answers in.
  const guatiao_alloc* alloc() const noexcept { return alloc_; }

 private:
  friend class registry;
  instance(const guatiao_registry* reg, std::string key, void* ctx, const guatiao_alloc* alloc)
      : reg_(reg), key_(std::move(key)), ctx_(ctx), alloc_(alloc) {}
  const guatiao_registry* reg_ = nullptr;
  std::string key_;
  void* ctx_ = nullptr;
  const guatiao_alloc* alloc_ = nullptr;
};

/// A host's registry: the libraries it loaded or registered, the sources
/// it keeps, and what they offer. Every answer that is data is a `value`
/// built in the registry's allocator. Not safe to use from two threads at
/// once; a provider reached through it may be called from any. Move-only.
/// Freeing it unmaps nothing.
class registry {
 public:
  /// A registry introducing its host as `id`/`version`; `alloc` is offered
  /// to every library and builds every answer.
  static result<registry> try_open(std::string_view id, std::string_view version,
                                   const guatiao_alloc* alloc = default_alloc()) {
    guatiao_registry* r = guatiao_registry_new(detail::str(id), detail::str(version), alloc);
    if (!r) return detail::error_of(GUATIAO_ERR_BAD_VALUE, "the registry was refused: an id, a version or an allocator");
    return registry(r, alloc);
  }
#if GUATIAO_EXCEPTIONS
  registry(std::string_view id, std::string_view version, const guatiao_alloc* alloc = default_alloc())
      : registry(try_open(id, version, alloc).value()) {}
#endif
  registry(registry&& o) noexcept : r_(o.r_), alloc_(o.alloc_) { o.r_ = nullptr; }
  registry& operator=(registry&& o) noexcept {
    if (this != &o) {
      guatiao_registry_free(r_);
      r_ = o.r_;
      alloc_ = o.alloc_;
      o.r_ = nullptr;
    }
    return *this;
  }
  registry(const registry&) = delete;
  registry& operator=(const registry&) = delete;
  ~registry() { guatiao_registry_free(r_); }

  guatiao_registry* raw() const noexcept { return r_; }
  const guatiao_alloc* alloc() const noexcept { return alloc_; }
  /// How this registry introduces itself to a library: a block alive for
  /// the process.
  const guatiao_host_info* host() const noexcept { return guatiao_registry_host(r_); }

  // ---- loading -----------------------------------------------------------

  /// Maps one file and loads it: `{"loaded": ...}`, `{"skipped": why, ...}`
  /// or `{"failed": message}`. Mapping runs the file's static
  /// initialisers: name files you are willing to run.
  result<value> try_load_file(std::string_view path) {
    guatiao_value out = detail::blank();
    return detail::answered(guatiao_registry_load_file(r_, detail::str(path), alloc_, &out), out, "not loaded");
  }
  /// Registers a library this program links, through its entry point.
  result<value> try_register_entry(std::string_view name,
                                   const guatiao_library_info* (*entry)(const guatiao_host_info*)) {
    guatiao_value out = detail::blank();
    return detail::answered(guatiao_registry_register_entry(r_, detail::str(name), entry, alloc_, &out), out,
                            "not registered");
  }
  /// Registers a C++ library this program carries: `Make` is the function
  /// answering it (as GUATIAO_LIBRARY takes it). Needs
  /// <guatiao/library.hpp>.
  template <library (*Make)()>
  result<value> try_register_library(std::string_view name) {
    return try_register_entry(name, &detail::library_entry<Make, detail::registered_here>);
  }
  /// Scans a directory, mapping only files whose declarations pass
  /// `rules` (`rules({...})`): `{"loaded": [path], "skipped": [...],
  /// "failed": [...]}`.
  result<value> try_scan_dir(std::string_view dir, std::string_view scan_rules = {}, bool descending = false) {
    guatiao_value out = detail::blank();
    return detail::answered(
        guatiao_registry_scan_dir_rules(r_, detail::str(dir), descending, detail::str(scan_rules), alloc_, &out), out,
        "not scanned");
  }
  /// Walks a search path (`;` on Windows, `:` elsewhere) under `rules`.
  result<value> try_scan_path(std::string_view spec, std::string_view scan_rules = {}, bool descending = false) {
    guatiao_value out = detail::blank();
    return detail::answered(
        guatiao_registry_scan_path(r_, detail::str(spec), descending, detail::str(scan_rules), alloc_, &out), out,
        "not scanned");
  }

  // ---- sources ----------------------------------------------------------

  /// Watches a directory: each `refresh()` loads what appeared, retires
  /// what vanished, reloads what changed.
  result<void> try_watch_dir(std::string_view dir, std::string_view scan_rules = {}) {
    return detail::check(guatiao_registry_watch_dir(r_, detail::str(dir), detail::str(scan_rules)));
  }
  /// Maps a file exporting `guatiao_source` (a bridge to another language)
  /// and adds the source it answers for `config` (a map); kept for the
  /// registry's life. A refusal carries the source's words.
  result<void> try_add_source_file(std::string_view path, view config = {}) {
    guatiao_provider_error err{};
    guatiao_status s = guatiao_registry_add_source_file(r_, detail::str(path), config.raw(), &err);
    std::string m = detail::take_message(err);
    if (s != GUATIAO_OK) return detail::error_of(s, m.empty() ? "the source was refused" : m);
    return {};
  }
  /// Adds a source this program links: a table whose slots and `ctx` live
  /// as long as the registry.
  result<void> try_add_source(std::string_view name, const guatiao_source_info* source) {
    guatiao_provider_error err{};
    guatiao_status s = guatiao_registry_add_source(r_, detail::str(name), source, &err);
    std::string m = detail::take_message(err);
    if (s != GUATIAO_OK) return detail::error_of(s, m.empty() ? "the source was refused" : m);
    return {};
  }
  /// Asks every source and watched directory what it serves now and makes
  /// the registry agree: `{"loaded", "retired", "skipped", "refused"}`.
  /// Refusals are part of the answer, never a failed call.
  result<value> try_refresh() {
    guatiao_value out = detail::blank();
    return detail::answered(guatiao_registry_refresh(r_, alloc_, &out), out, "not refreshed");
  }

  // ---- what is loaded ---------------------------------------------------

  result<value> try_libraries() const {
    guatiao_value out = detail::blank();
    return detail::answered(guatiao_registry_libraries(r_, alloc_, &out), out, "no answer");
  }
  /// Every provider serving `kind`, best first (every provider when
  /// `kind` is empty): a list of maps.
  result<value> try_providers(std::string_view kind = {}) const {
    guatiao_value out = detail::blank();
    return detail::answered(guatiao_registry_providers(r_, detail::str(kind), alloc_, &out), out, "no answer");
  }
  /// One provider by the key it is filed under; NOT_FOUND for none.
  result<value> try_provider(std::string_view key) const {
    guatiao_value out = detail::blank();
    return detail::answered(guatiao_registry_provider(r_, detail::str(key), alloc_, &out), out, "no such provider");
  }
  /// The providers serving `kind` that can run here now, best first.
  result<value> try_available(std::string_view kind) const {
    guatiao_value out = detail::blank();
    return detail::answered(guatiao_registry_available(r_, detail::str(kind), alloc_, &out), out, "no answer");
  }
  /// Why nothing can serve `kind`: `{"available": bool, "why", ...}`.
  result<value> try_why_not(std::string_view kind) const {
    guatiao_value out = detail::blank();
    return detail::answered(guatiao_registry_why_not(r_, detail::str(kind), alloc_, &out), out, "no answer");
  }
  /// The best provider serving `kind` that can run here; NOT_FOUND for none.
  result<value> try_best(std::string_view kind) const {
    guatiao_value out = detail::blank();
    return detail::answered(guatiao_registry_best(r_, detail::str(kind), alloc_, &out), out, "nothing serves it");
  }
  /// Nothing when the provider can run here (or no provider has `key`),
  /// else why not. Asked every time, never cached.
  std::optional<std::string> provider_unavailable(std::string_view key) const {
    guatiao_str why{nullptr, 0};
    if (guatiao_registry_provider_available(r_, detail::str(key), &why)) return std::nullopt;
    return std::string(detail::text(why));
  }
  /// A provider's configuration schema, borrowed from the registry until
  /// its library is retired; absent when it declares none.
  view provider_config(std::string_view key) const noexcept {
    return view(guatiao_registry_provider_config(r_, detail::str(key)));
  }
  /// A copy of a provider's `meta`; ABSENT when it declares none.
  result<value> try_provider_meta(std::string_view key) const {
    guatiao_value out = detail::blank();
    return detail::answered(guatiao_registry_provider_meta(r_, detail::str(key), alloc_, &out), out,
                            "no such provider");
  }
  /// The table a provider speaks `kind` through and the size it was
  /// compiled at; a null table when it does not.
  std::pair<const void*, std::size_t> provider_table(std::string_view key, std::string_view kind) const noexcept {
    std::size_t size = 0;
    const void* t = guatiao_registry_provider_table(r_, detail::str(key), detail::str(kind), &size);
    return {t, t ? size : 0};
  }
  /// The `ctx` every slot of the provider's tables takes.
  void* provider_ctx(std::string_view key) const noexcept { return guatiao_registry_provider_ctx(r_, detail::str(key)); }

  // ---- ranking and keys -------------------------------------------------

  result<void> try_set_priority(std::string_view id, std::int32_t priority) {
    return detail::check(guatiao_registry_set_priority(r_, detail::str(id), priority));
  }
  std::int32_t priority(std::string_view id) const noexcept { return guatiao_registry_priority(r_, detail::str(id)); }
  /// Files providers under `key_template` (`%id`, `%version`, `%name`,
  /// `%library`, `%source`) instead of `%id`.
  result<void> try_keyed_by(std::string_view key_template) {
    return detail::check(guatiao_registry_keyed_by(r_, detail::str(key_template)));
  }
  result<void> try_libraries_keyed_by(std::string_view key_template) {
    return detail::check(guatiao_registry_libraries_keyed_by(r_, detail::str(key_template)));
  }

  // ---- instances --------------------------------------------------------

  /// Builds an instance of the provider filed under `key` from `config`,
  /// checked against its schema first (BAD_VALUE naming the field).
  /// NOT_FOUND for no such provider, NULL for one that builds none.
  result<instance> try_create(std::string_view key, view config) {
    std::string held(key);  // copied first: nothing can throw once the instance exists
    void* out = nullptr;
    guatiao_provider_error err{};
    guatiao_status s = guatiao_registry_provider_create(r_, detail::str(key), config.raw(), &out, &err);
    std::string m = detail::take_message(err);
    if (s != GUATIAO_OK) return detail::error_of(s, m.empty() ? "no instance was built" : m);
    return instance(r_, std::move(held), out, alloc_);
  }

  // ---- leaving ----------------------------------------------------------

  /// Takes a LIBRARY (by its library key) out and leaves it mapped until
  /// nothing uses it. No table or ctx taken from it is used once closed.
  result<void> try_retire(std::string_view library_key) {
    return detail::check(guatiao_registry_retire(r_, detail::str(library_key)));
  }
  /// Takes a library out and unmaps it: BUSY while something it handed out
  /// lives, UNSUPPORTED with no `unload` slot, WRONG_KIND for one this
  /// program links.
  result<void> try_unload(std::string_view library_key) {
    return detail::check(guatiao_registry_unload(r_, detail::str(library_key)));
  }
  /// `unload` past a lease still held and a missing slot: the caller's
  /// word that nothing from the library is alive.
  result<void> try_unload_unchecked(std::string_view library_key) {
    return detail::check(guatiao_registry_unload_unchecked(r_, detail::str(library_key)));
  }
  /// Closes every retired library nothing uses: their keys, a list.
  result<value> try_collect() {
    guatiao_value out = detail::blank();
    return detail::answered(guatiao_registry_collect(r_, alloc_, &out), out, "not collected");
  }

#if GUATIAO_EXCEPTIONS
  value load_file(std::string_view path) { return try_load_file(path).value(); }
  value register_entry(std::string_view name, const guatiao_library_info* (*entry)(const guatiao_host_info*)) {
    return try_register_entry(name, entry).value();
  }
  template <library (*Make)()>
  value register_library(std::string_view name) {
    return try_register_library<Make>(name).value();
  }
  value scan_dir(std::string_view dir, std::string_view scan_rules = {}, bool descending = false) {
    return try_scan_dir(dir, scan_rules, descending).value();
  }
  value scan_path(std::string_view spec, std::string_view scan_rules = {}, bool descending = false) {
    return try_scan_path(spec, scan_rules, descending).value();
  }
  void watch_dir(std::string_view dir, std::string_view scan_rules = {}) { try_watch_dir(dir, scan_rules).value(); }
  void add_source_file(std::string_view path, view config = {}) { try_add_source_file(path, config).value(); }
  void add_source(std::string_view name, const guatiao_source_info* source) { try_add_source(name, source).value(); }
  value refresh() { return try_refresh().value(); }
  value libraries() const { return try_libraries().value(); }
  value providers(std::string_view kind = {}) const { return try_providers(kind).value(); }
  value provider(std::string_view key) const { return try_provider(key).value(); }
  value available(std::string_view kind) const { return try_available(kind).value(); }
  value why_not(std::string_view kind) const { return try_why_not(kind).value(); }
  value best(std::string_view kind) const { return try_best(kind).value(); }
  value provider_meta(std::string_view key) const { return try_provider_meta(key).value(); }
  void set_priority(std::string_view id, std::int32_t priority) { try_set_priority(id, priority).value(); }
  void keyed_by(std::string_view key_template) { try_keyed_by(key_template).value(); }
  void libraries_keyed_by(std::string_view key_template) { try_libraries_keyed_by(key_template).value(); }
  instance create(std::string_view key, view config) { return try_create(key, config).value(); }
  void retire(std::string_view library_key) { try_retire(library_key).value(); }
  void unload(std::string_view library_key) { try_unload(library_key).value(); }
  void unload_unchecked(std::string_view library_key) { try_unload_unchecked(library_key).value(); }
  value collect() { return try_collect().value(); }
#endif

 private:
  registry(guatiao_registry* r, const guatiao_alloc* alloc) noexcept : r_(r), alloc_(alloc) {}
  guatiao_registry* r_ = nullptr;
  const guatiao_alloc* alloc_ = nullptr;
};

}  // namespace GUATIAO_ABI
}  // namespace guatiao

#endif  // GUATIAO_REGISTRY_HPP
