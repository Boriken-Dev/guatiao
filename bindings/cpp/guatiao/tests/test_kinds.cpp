// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Kinds from C++: a class serving the example kinds, registered in this
// process and called through its tables as any host calls them; objects
// both ways, maps lent and handed over, a refusal and a provider bug
// reaching the caller as statuses, answers built in the caller's
// allocator, calls from several threads, a configured provider's
// instances, and the library's say in being unmapped. Through the `try_`
// forms, so it runs with exceptions off too (a provider then refuses by
// answering a `result`).

#include "support/guatiao_counting_default.hpp"  // first: it names the default allocator

#include "greeter_kind.hpp"

#include <thread>

#include "support/harness.hpp"

namespace {

namespace gk = greeter_kind;
using guatiao::item;
using gt_test::counted;

#define CHECK_NONE_LEFT(what) \
  CHECK_MSG(counted().live() == 0, std::string(what) + ": " + std::to_string(counted().live()) + " left")

// ---- the classes serving the kinds -------------------------------------------

struct ear {
  ear(std::vector<std::string>* h, bool* d) noexcept : heard_so_far(h), destroyed(d) {}
  ear(const ear&) = delete;
  ear& operator=(const ear&) = delete;
  std::vector<std::string>* heard_so_far;
  bool* destroyed;
  void heard(std::string_view what) {
    if (heard_so_far) heard_so_far->emplace_back(what);
  }
  ~ear() {
    if (destroyed) *destroyed = true;
  }
};

struct chat {
  guatiao::object<gk::listener> listener;
  std::vector<std::string> said;
  guatiao::result<void> say(std::string_view what) {
    if (what.empty()) return guatiao::error(guatiao::status::bad_value, "nothing to say");
    said.emplace_back(what);
    auto heard = listener.try_heard(what);
    return heard;
  }
  std::int64_t read(guatiao::bytes_mut dst) {
    std::string all;
    for (const std::string& s : said) all += (all.empty() ? "" : "\n") + s;
    std::size_t n = all.size() < dst.size() ? all.size() : dst.size();
    if (n) std::memcpy(dst.data(), all.data(), n);
    return static_cast<std::int64_t>(n);
  }
  std::int64_t turns() { return static_cast<std::int64_t>(said.size()); }
};

struct hello {
  std::atomic<std::int64_t> asked{0};
  guatiao::result<guatiao::map> greet(std::string_view name) {
    if (name == "panic") return guatiao::error(guatiao::status::internal, "a provider bug, on request");
    if (name.empty()) return guatiao::error(guatiao::status::bad_value, "nobody to greet");
    return guatiao::map::try_of({{"greeting", "hello, " + std::string(name)}});
  }
  std::string shout(std::string_view name) {
    std::string s = "HELLO, " + std::string(name);
    return s;
  }
  guatiao::object<gk::conversation> start(guatiao::object<gk::listener> l) {
    return guatiao::object<gk::conversation>::of(std::unique_ptr<chat>(new chat{std::move(l), {}}));
  }
  guatiao::result<void> greet_into(std::string_view name, guatiao::lent<gk::listener> l) {
    return l.try_heard("hello, " + std::string(name));
  }
  guatiao::result<guatiao::map> greet_from(guatiao::map config) { return greet(config["name"].string_or("")); }
  guatiao::result<bool> check(guatiao::map_ref config) {
    std::string name(config["name"].string_or("world"));
    auto set = config.try_set("greeting", "hello, " + name);
    if (!set) return set.error();
    return !name.empty();
  }
  std::int64_t count() { return asked.fetch_add(1) + 1; }
  std::int64_t echo_i64(std::int64_t x) { return x; }
  std::uint64_t echo_u64(std::uint64_t x) { return x; }
};

/// Built from `{"prefix": text}`.
struct shouter {
  std::string prefix;
  static guatiao::result<std::unique_ptr<shouter>> make(guatiao::view config) {
    auto p = config["prefix"].string();
    if (!p || p->empty()) return guatiao::error(guatiao::status::bad_value, "a shouter needs a prefix");
    return std::unique_ptr<shouter>(new shouter{std::string(*p)});
  }
  static item schema() {
    return item::map({{"type", "object"},
                      {"properties", item::map({{"prefix", item::map({{"type", "string"}, {"minLength", 1}})}})},
                      {"required", item::list({"prefix"})},
                      {"additionalProperties", false}});
  }
  guatiao::result<guatiao::map> greet(std::string_view name) {
    std::string loud(name);
    for (char& c : loud)
      if (c >= 'a' && c <= 'z') c = static_cast<char>(c - 32);
    return guatiao::map::try_of({{"greeting", prefix + " " + loud}});
  }
};

/// Replaces the map it is lent with another kind, through the node.
struct replacer {
  guatiao::result<guatiao::map> greet(std::string_view) { return guatiao::map::try_of({{"greeting", "?"}}); }
  bool check(guatiao::map_ref config) {
    config.as_node().assign(guatiao::try_make(item(5)).value_or(guatiao::value()));
    return true;
  }
};

/// What `keeper::greet_from` was handed, kept past the call.
std::vector<guatiao::map>& kept() {
  static std::vector<guatiao::map> maps;
  return maps;
}

/// Keeps every map handed to it.
struct keeper {
  guatiao::result<guatiao::map> greet(std::string_view name) {
    return guatiao::map::try_of({{"greeting", std::string(name)}});
  }
  guatiao::result<guatiao::map> greet_from(guatiao::map config) {
    kept().push_back(std::move(config));
    return guatiao::map::try_of({{"greeting", "kept"}});
  }
};

guatiao::library cpp_keepers() {
  guatiao::library lib("cpp_keepers", "1.0.0");
  lib.single<keeper, gk::greeter>("keeper");
  return lib;
}

guatiao::library cpp_kinds() {
  guatiao::library lib("cpp_kinds", "1.0.0");
  lib.meta(item::map({{"summary", "greeters written in C++, in a test"}}));
  lib.single<hello, gk::greeter, gk::counter, gk::wide>("hello").display_name("Hello, in C++");
  lib.configured<shouter, gk::greeter>("shouter");
  lib.single<replacer, gk::greeter>("replacer");
  return lib;
}

// ---- calls through the tables ----------------------------------------------------

void every_slot(guatiao::registry& reg) {
  auto g = MUST(gk::greeter::try_bind(reg, "cpp_kinds_hello"));
  CHECK(g.has_greet() && g.has_shout() && g.has_start() && g.has_check());

  guatiao::map answer = MUST(g.try_greet("ana"));
  CHECK(answer["greeting"].string_or("") == "hello, ana");
  CHECK(answer.as_map().raw()->alloc == counted().get());  // built where this caller asked

  auto refused = g.try_greet("");
  CHECK(refused.code() == guatiao::status::bad_value && refused.error().message() == "nobody to greet");
  auto bug = g.try_greet("panic");
  CHECK(bug.code() == guatiao::status::internal && bug.error().message() == "a provider bug, on request");

  CHECK(MUST(g.try_shout("ana")) == "HELLO, ana");

  guatiao::map config = MUST(guatiao::map::try_of({{"name", "bo"}}));
  CHECK(MUST(g.try_check(config.ref())));
  CHECK(config["greeting"].string_or("") == "hello, bo");  // the lent map, changed in place
  CHECK(config.size() == 2 && config.as_map().raw()->alloc == counted().get());

  guatiao::map handed = MUST(guatiao::map::try_of({{"name", "cy"}}));
  guatiao::map from = MUST(g.try_greet_from(std::move(handed)));
  CHECK(from["greeting"].string_or("") == "hello, cy");

  std::vector<std::string> heard;
  bool lent_destroyed = false;
  ear lent_ear{&heard, &lent_destroyed};
  MUST(g.try_greet_into("dee", lent_ear));
  CHECK(heard.size() == 1 && heard[0] == "hello, dee");
  CHECK(!lent_destroyed);  // lent: never destroyed by the callee

  std::vector<std::string> told;
  bool given_destroyed = false;
  {
    guatiao::object<gk::conversation> c =
        MUST(g.try_start(guatiao::object<gk::listener>::make<ear>(&told, &given_destroyed)));
    CHECK(static_cast<bool>(c) && c.usable());
    MUST(c.try_say("hi"));
    MUST(c.try_say("again"));
    CHECK(c.try_say("").code() == guatiao::status::bad_value);
    std::uint8_t buf[64];
    std::int64_t n = MUST(c.try_read(guatiao::bytes_mut(buf)));
    CHECK(std::string(reinterpret_cast<char*>(buf), static_cast<std::size_t>(n)) == "hi\nagain");
    CHECK(MUST(c.try_turns()) == 2);
    CHECK(told.size() == 2 && !given_destroyed);
  }
  CHECK(given_destroyed);  // destroyed with the conversation that held it

  auto c = MUST(gk::counter::try_bind(reg, "cpp_kinds_hello"));
  CHECK(MUST(c.try_count()) == 1);
  CHECK(MUST(c.try_count()) == 2);
  auto w = MUST(gk::wide::try_bind(reg, "cpp_kinds_hello"));
  CHECK(MUST(w.try_echo_i64(std::numeric_limits<std::int64_t>::min())) == std::numeric_limits<std::int64_t>::min());
  CHECK(MUST(w.try_echo_u64(std::numeric_limits<std::uint64_t>::max())) == std::numeric_limits<std::uint64_t>::max());
}

void what_is_not_the_kind(guatiao::registry& reg) {
  // A counter table is no greeter; a provider with no table of a kind is
  // not found as that kind.
  auto t = reg.provider_table("cpp_kinds_hello", "counter");
  CHECK(guatiao::try_bind_table<gk::greeter>(t.first, t.second, reg.provider_ctx("cpp_kinds_hello")).code() ==
        guatiao::status::wrong_kind);
  CHECK(gk::counter::try_bind(reg, "cpp_kinds_shouter").code() == guatiao::status::not_found);
  // A table shorter than a slot answers UNSUPPORTED for it, and the call
  // frees what it was given.
  auto g = reg.provider_table("cpp_kinds_hello", "greeter");
  guatiao::remote<gk::greeter> short_table(g.first, offsetof(greeter_vtable, shout), reg.provider_ctx("cpp_kinds_hello"),
                                           counted().get());
  CHECK(!short_table.has_shout() && short_table.has_greet());
  CHECK(short_table.try_shout("x").code() == guatiao::status::unsupported);
  guatiao::map handed = MUST(guatiao::map::try_of({{"name", "x"}}));
  CHECK(short_table.try_greet_from(std::move(handed)).code() == guatiao::status::unsupported);
}

void a_lent_map_replaced_comes_back_empty(guatiao::registry& reg) {
  auto g = MUST(gk::greeter::try_bind(reg, "cpp_kinds_replacer"));
  guatiao::map config = MUST(guatiao::map::try_of({{"name", "x"}, {"more", 1}}));
  CHECK(MUST(g.try_check(config.ref())));
  // The method freed the caller's buffer replacing it: the caller holds
  // an empty map in its own allocator, not a pointer to freed memory.
  CHECK(config.is_map() && config.size() == 0);
  CHECK(config.as_map().raw()->alloc == counted().get());
  MUST(config.try_set("again", 2));
  CHECK(config.size() == 1);
}

void configured_instances(guatiao::registry& reg) {
  CHECK(reg.provider_config("cpp_kinds_shouter").is_map());
  guatiao::value empty = MUST(guatiao::try_make(item::map({})));
  auto refused = reg.try_create("cpp_kinds_shouter", empty);
  CHECK(refused.code() == guatiao::status::bad_value);
  if (!refused.ok()) CHECK(refused.error().message().find("prefix") != std::string::npos);
  guatiao::value config = MUST(guatiao::try_make(item::map({{"prefix", "hey"}})));
  guatiao::instance inst = MUST(reg.try_create("cpp_kinds_shouter", config));
  CHECK(guatiao::detail::live().load() >= 1);
  auto g = MUST(gk::greeter::try_bind(inst));
  CHECK(MUST(g.try_greet("ana"))["greeting"].string_or("") == "hey ANA");
  CHECK(!g.has_shout());
  inst.close();
}

void threads(guatiao::registry& reg) {
  auto g = MUST(gk::greeter::try_bind(reg, "cpp_kinds_hello"));
  std::atomic<int> good{0};
  std::vector<std::thread> pool;
  for (int t = 0; t < 8; ++t) {
    pool.emplace_back([&g, &good, t] {
      for (int i = 0; i < 500; ++i) {
        auto m = g.try_greet("t" + std::to_string(t));
        if (m.ok() && (*m)["greeting"].string_or("") == "hello, t" + std::to_string(t)) good.fetch_add(1);
      }
    });
  }
  for (std::thread& t : pool) t.join();
  CHECK(good.load() == 8 * 500);
}

void the_library_says_when_it_may_go() {
  // Registered in this process the library is linked, never unmapped; its
  // own `unload` slot is what a host asks before unmapping a file.
  using here = guatiao::detail::registered_here;
  const guatiao_library_info* info = guatiao::detail::library_entry<&cpp_kinds, here>(nullptr);
  CHECK(info == nullptr);  // no host, no library
  guatiao::registry probe = MUST(guatiao::registry::try_open("probe", "1"));
  info = guatiao::detail::library_entry<&cpp_kinds, here>(probe.host());
  CHECK(info && info->unload && info->unload() == GUATIAO_OK);
  guatiao::object<gk::listener> held = guatiao::object<gk::listener>::make<ear>(nullptr, nullptr);
  CHECK(info->unload() == GUATIAO_ERR_BUSY);
  held.reset();
  CHECK(info->unload() == GUATIAO_OK);
  CHECK(info->providers.len == 3 && info->meta != nullptr);
}

/// Handed over in the caller's allocator, a map is moved into this side's
/// before the method keeps it.
void a_map_handed_over_and_kept_is_the_providers_own() {
  gt_test::counting caller;
  {
    guatiao::registry reg = MUST(guatiao::registry::try_open("cpp-keepers", "0.1", counted().get()));
    guatiao::value loaded = MUST(reg.try_register_library<&cpp_keepers>("cpp_keepers"));
    CHECK(loaded.contains("loaded"));
    auto g = MUST(gk::greeter::try_bind(reg, "cpp_keepers_keeper"));
    guatiao::map handed = MUST(guatiao::map::try_of({{"name", "ivy"}}, caller.get()));
    guatiao::map answer = MUST(g.try_greet_from(std::move(handed)));
    CHECK(answer["greeting"].string_or("") == "kept");
    CHECK_MSG(caller.live() == 0,
              "what the provider keeps holds " + std::to_string(caller.live()) + " blocks of the caller's allocator");
    CHECK(kept().size() == 1 && kept()[0]["name"].string_or("") == "ivy");
    kept().clear();

    // This side's allocator refusing, the move is refused: the call with it,
    // and the tree freed through the caller's.
    guatiao::map refused = MUST(guatiao::map::try_of({{"name", "ivy"}}, caller.get()));
    counted().fail_after(0);
    auto moved = g.try_greet_from(std::move(refused));
    counted().fail_after(-1);
    CHECK(!moved.ok() && moved.code() == guatiao::status::alloc);
    CHECK_MSG(caller.live() == 0, "a refused move leaves " + std::to_string(caller.live()) + " blocks");
    CHECK(kept().empty());
  }
}

}  // namespace

int main() {
  {
    guatiao::registry reg = MUST(guatiao::registry::try_open("cpp-kinds", "0.1", counted().get()));
    guatiao::value loaded = MUST(reg.try_register_library<&cpp_kinds>("cpp_kinds"));
    CHECK(loaded.contains("loaded"));
    every_slot(reg);
    what_is_not_the_kind(reg);
    configured_instances(reg);
    a_lent_map_replaced_comes_back_empty(reg);
    threads(reg);
    guatiao::value providers = MUST(reg.try_providers("greeter"));
    CHECK(providers.size() == 3);
  }
  the_library_says_when_it_may_go();
  a_map_handed_over_and_kept_is_the_providers_own();
  CHECK(guatiao::detail::live().load() == 0);
  CHECK_NONE_LEFT("every call, refusal, object and instance");
  return gt_test::finish("kinds");
}
