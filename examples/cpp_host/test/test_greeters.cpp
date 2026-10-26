// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// One behaviour, five languages: a C++ host calls the greeters written in
// Rust, C, Python, Go and C++ through one description of the kind, every
// slot, and has a Rust caller call each of them through the kind's Rust
// proxy. Every greeter comes through a source the registry keeps (watched
// directories, the guatiao-source-py bridge), loaded by one refresh(); the C++
// one is a library file like the rest. Native threads then call the C++
// greeter, a configured provider builds instances, and the C++ library is
// unmapped once nothing it served is alive.
//
// Where each greeter is: GUATIAO_RUST_GREETERS (the workspace's build,
// holding derived_greeter and greeter_caller), the C and C++ ones built by
// this project, GUATIAO_GO_LIBRARY (a directory holding the Go greeter
// built with -buildmode=c-shared), GUATIAO_PY_BRIDGE (the guatiao-source-py
// bridge). One missing is a stated skip, and a failure under
// GUATIAO_REQUIRE_ALL=1.

#include "greeter_kind.hpp"

#include <cstdlib>
#include <thread>

#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#else
#include <dlfcn.h>
#endif

namespace {

namespace gk = greeter_kind;
using guatiao::item;
using guatiao::view;

int passed = 0, failed = 0, skipped = 0;

void check(bool ok, const std::string& what) {
  if (ok) {
    ++passed;
  } else {
    ++failed;
    std::printf("  FAIL %s\n", what.c_str());
  }
}

std::string env(const char* name) {
#if defined(_WIN32)
  char* v = nullptr;
  std::size_t n = 0;
  if (_dupenv_s(&v, &n, name) != 0 || !v) return {};
  std::string out(v);
  std::free(v);
  return out;
#else
  const char* v = std::getenv(name);
  return v ? v : "";
#endif
}

void skip(const std::string& why) {
  if (env("GUATIAO_REQUIRE_ALL") == "1") {
    ++failed;
    std::printf("  FAIL (GUATIAO_REQUIRE_ALL) skipped: %s\n", why.c_str());
  } else {
    ++skipped;
    std::printf("  SKIP %s\n", why.c_str());
  }
}

/// A listener: what it was told, and whether it was destroyed.
struct ear {
  explicit ear(std::vector<std::string>* h = nullptr, bool* d = nullptr) noexcept : heard_so_far(h), destroyed(d) {}
  ear(const ear&) = delete;
  ear& operator=(const ear&) = delete;
  ~ear() {
    if (destroyed) *destroyed = true;
  }
  void heard(std::string_view what) {
    if (heard_so_far) heard_so_far->emplace_back(what);
  }
  std::vector<std::string>* heard_so_far;
  bool* destroyed;
};

std::string upper(std::string s) {
  for (char& c : s)
    if (c >= 'a' && c <= 'z') c = static_cast<char>(c - 'a' + 'A');
  return s;
}

std::string json_of(view v);

std::string json_of(view v) {
  switch (v.tag()) {
    case guatiao::tag::null: return "null";
    case guatiao::tag::boolean: return v.bool_or(false) ? "true" : "false";
    case guatiao::tag::number: return std::string(*v.number_text());
    case guatiao::tag::string: return "\"" + std::string(*v.string()) + "\"";
    case guatiao::tag::list: {
      std::string s = "[";
      for (view x : v.as_list()) s += (s.size() > 1 ? "," : "") + json_of(x);
      return s + "]";
    }
    case guatiao::tag::map: {
      std::string s = "{";
      for (guatiao::entry e : v.as_map()) s += (s.size() > 1 ? ",\"" : "\"") + std::string(e.key) + "\":" + json_of(e.value);
      return s + "}";
    }
    default: return "absent";
  }
}

struct greeter_case {
  const char* who;
  const char* key;
  const char* greeting;
  bool present = false;
};

std::string library_file(const std::string& dir, const std::string& stem) {
#if defined(_WIN32)
  return dir + "/" + stem + ".dll";
#elif defined(__APPLE__)
  return dir + "/lib" + stem + ".dylib";
#else
  return dir + "/lib" + stem + ".so";
#endif
}

bool exists(const std::string& path) {
  FILE* f = nullptr;
#if defined(_WIN32)
  if (fopen_s(&f, path.c_str(), "rb") != 0) f = nullptr;
#else
  f = std::fopen(path.c_str(), "rb");
#endif
  if (f) std::fclose(f);
  return f != nullptr;
}

void add_sources(guatiao::registry& reg, std::vector<greeter_case>& cases) {
  std::string rust = env("GUATIAO_RUST_GREETERS");
  if (rust.empty()) rust = RUST_GREETERS_DIR;
  if (exists(library_file(rust, "derived_greeter"))) {
    reg.watch_dir(rust, guatiao::rules({"kind=greeter"}));
    cases[0].present = true;
  } else {
    skip("no derived_greeter in " + rust + ": set GUATIAO_RUST_GREETERS to the workspace's build");
  }
  reg.watch_dir(C_GREETER_DIR, guatiao::rules({"kind=greeter"}));
  cases[1].present = true;
  std::string bridge = env("GUATIAO_PY_BRIDGE");
  if (bridge.empty()) {
    skip("GUATIAO_PY_BRIDGE names no guatiao-source-py bridge: the Python greeters are not served");
  } else {
    std::string repo = GUATIAO_REPO;
    guatiao::value config = guatiao::make(item::map(
        {{"dirs", item::list({repo + "/examples/python_provider"})},
         {"paths", item::list({repo + "/bindings/python/guatiao/src", repo + "/examples/greeter_kind/python"})}}));
    auto added = reg.try_add_source_file(bridge, config);
    check(added.ok(), "the bridge maps: " + (added.ok() ? std::string() : added.error().message()));
    cases[2].present = added.ok();
  }
  std::string go = env("GUATIAO_GO_LIBRARY");
  if (go.empty()) {
    skip("GUATIAO_GO_LIBRARY names no directory holding the Go greeter built with -buildmode=c-shared");
  } else {
    reg.watch_dir(go, guatiao::rules({"kind=greeter"}));
    cases[3].present = true;
  }
  reg.watch_dir(CPP_GREETER_DIR, guatiao::rules({"kind=greeter"}));
  cases[4].present = true;

  guatiao::value changed = reg.refresh();
  check(changed["refused"].size() == 0, "one refresh refuses nothing: " + json_of(changed["refused"]));
  std::string loaded;
  for (view one : changed["loaded"].as_list()) loaded += " " + std::string(one["key"].string_or("?"));
  if (cases[0].present) check(loaded.find(" derived_greeter") != std::string::npos, "the refresh loads the Rust greeters");
  check(loaded.find(" c_greeter_library") != std::string::npos, "and the C one");
  if (cases[2].present) check(loaded.find(" py_greeter") != std::string::npos, "and the Python ones");
  if (cases[3].present) check(loaded.find("go_greeter") != std::string::npos, "and the Go ones");
  check(loaded.find(" cpp_greeter") != std::string::npos, "and the C++ ones");
}

void a_cpp_host_calls_it(guatiao::registry& reg, const greeter_case& c) {
  std::string who = c.who;
  std::string greeting = c.greeting;
  auto bound = gk::greeter::try_bind(reg, c.key);
  check(bound.ok(), who + ": bound as a greeter");
  if (!bound.ok()) return;
  guatiao::remote<gk::greeter> g = *bound;

  auto answer = g.try_greet("ana");
  check(answer.ok() && json_of(*answer) == "{\"greeting\":\"" + greeting + "\"}", who + ": the greeting");
  check(answer.ok() && answer->as_map().raw()->alloc == reg.alloc(), who + ": the answer is in this host's allocator");

  auto nobody = g.try_greet("");
  check(nobody.code() == guatiao::status::bad_value &&
            nobody.error().message().find("nobody to greet") != std::string::npos,
        who + ": nobody is refused, with BAD_VALUE and its words");

  auto bug = g.try_greet("panic");
  if (who == "c") check(bug.ok(), who + ": greets anyone");
  else check(bug.code() == guatiao::status::internal, who + ": a provider bug is refused as INTERNAL");

  guatiao::map config = guatiao::map{{"name", "bo"}};
  auto usable = g.try_check(config.ref());
  check(usable.ok() && *usable, who + ": check");
  check(config["name"].string_or("") == "bo", who + ": the lent map keeps its name");
  check(config.size() == 2, who + ": and gains an entry");

  auto from = g.try_greet_from(guatiao::map{{"name", "ana"}});
  check(from.ok() && json_of(*from) == "{\"greeting\":\"" + greeting + "\"}", who + ": greets from the map handed over");

  std::vector<std::string> heard;
  bool lent_destroyed = false;
  ear lent(&heard, &lent_destroyed);
  auto into = g.try_greet_into("ana", lent);
  check(into.ok() && heard.size() == 1 && heard[0] == greeting, who + ": the lent listener heard the greeting");
  check(!lent_destroyed, who + ": a lent listener is not destroyed");

  if (g.has_shout()) {
    auto shouted = g.try_shout("ana");
    check(shouted.ok() && *shouted == upper(greeting), who + ": shout");
  } else {
    check(g.try_shout("ana").code() == guatiao::status::unsupported, who + ": no shout in its table");
  }

  if (g.has_start()) {
    std::vector<std::string> told;
    bool given_destroyed = false;
    {
      auto started = g.try_start(guatiao::object<gk::listener>::make<ear>(&told, &given_destroyed));
      check(started.ok(), who + ": start");
      if (started.ok()) {
        guatiao::object<gk::conversation>& chat = *started;
        check(chat.try_say("hi").ok() && chat.try_say("again").ok(), who + ": say");
        check(chat.try_say("").code() == guatiao::status::bad_value, who + ": nothing to say is refused");
        std::uint8_t buf[64];
        auto n = chat.try_read(guatiao::bytes_mut(buf));
        check(n.ok() && std::string(reinterpret_cast<char*>(buf), static_cast<std::size_t>(*n)) == "hi\nagain",
              who + ": the transcript");
        check(chat.try_turns().value_or(0) == 2, who + ": two turns");
        check(told.size() == 2, who + ": the listener heard both");
      }
    }
    check(given_destroyed, who + ": the listener is destroyed with the conversation");
  } else {
    check(who == "c", who + ": every greeter but the C one holds conversations");
  }
}

using caller_fn = guatiao_status (*)(const guatiao_host_info*, guatiao_str, guatiao_str, guatiao_value*);

caller_fn open_caller() {
  std::string rust = env("GUATIAO_RUST_GREETERS");
  if (rust.empty()) rust = RUST_GREETERS_DIR;
  std::string path = library_file(rust, "greeter_caller");
#if defined(_WIN32)
  HMODULE lib = LoadLibraryA(path.c_str());
  if (!lib) return nullptr;
  return reinterpret_cast<caller_fn>(reinterpret_cast<void*>(GetProcAddress(lib, "greeter_caller_run")));
#else
  void* lib = dlopen(path.c_str(), RTLD_NOW | RTLD_LOCAL);
  if (!lib) return nullptr;
  return reinterpret_cast<caller_fn>(dlsym(lib, "greeter_caller_run"));
#endif
}

void the_rust_proxy_calls_it(guatiao::registry& reg, caller_fn run, const greeter_case& c) {
  std::string who = c.who;
  std::string greeting = c.greeting;
  guatiao_value out = guatiao::detail::blank();
  guatiao_status s = run(reg.host(), guatiao::detail::str(c.key), guatiao::detail::str("ana"), &out);
  guatiao::value report = guatiao::value::adopt(out);
  check(s == GUATIAO_OK, who + ": the Rust proxy ran");
  check(report["greet"].string_or("") == greeting, who + " through Rust: greet");
  check(report["refusal"]["status"].int_or(0) == GUATIAO_ERR_BAD_VALUE, who + " through Rust: the refusal");
  if (who == "c") check(report["failure"].int_or(-1) == 0, who + " through Rust: greets anyone");
  else check(report["failure"].int_or(0) == GUATIAO_ERR_INTERNAL, who + " through Rust: the failure");
  check(report["shout"].string_or("") == upper(greeting), who + " through Rust: shout");
  check(report["checked"].bool_or(false), who + " through Rust: check");
  check(report["filled"].size() == 2, who + " through Rust: the lent map filled");
  check(report["greet_from"].string_or("") == greeting, who + " through Rust: greet_from");
  check(report["heard"][0].string_or("") == greeting, who + " through Rust: greet_into");
  std::string conversation = json_of(report["conversation"]);
  if (who == "c") {
    check(conversation.find("this greeter holds no conversations") != std::string::npos,
          who + " through Rust: the default start");
  } else {
    check(conversation.find("\"turns\":2") != std::string::npos, who + " through Rust: two turns " + conversation);
    check(conversation.find("\"dropped\":true") != std::string::npos,
          who + " through Rust: the listener dropped " + conversation);
  }
}

void configured_providers(guatiao::registry& reg) {
  for (const char* key : {"cpp_greeter_shouter", "derived_greeter_shouter"}) {
    std::string who = key;
    if (reg.provider_config(key).is_absent()) {
      if (who == "cpp_greeter_shouter") check(false, who + ": declares its configuration");
      continue;
    }
    guatiao::value empty = guatiao::make(item::map({}));
    auto refused = reg.try_create(key, empty);
    check(refused.code() == guatiao::status::bad_value && refused.error().message().find("prefix") != std::string::npos,
          who + ": an empty configuration is refused, naming the field");
    guatiao::instance inst = reg.create(key, guatiao::make(item::map({{"prefix", "hey"}})));
    auto g = gk::greeter::bind(inst);
    check(json_of(g.greet("ana")) == "{\"greeting\":\"hey ANA\"}", who + ": an instance built from {\"prefix\": \"hey\"}");
    inst.close();
  }
}

void native_threads_call_the_cpp_greeter(guatiao::registry& reg) {
  auto g = gk::greeter::bind(reg, "cpp_greeter_hello");
  std::atomic<int> good{0};
  std::vector<std::thread> pool;
  for (int t = 0; t < 8; ++t) {
    pool.emplace_back([&g, &good, t] {
      std::string name = "t" + std::to_string(t);
      for (int i = 0; i < 500; ++i) {
        auto m = g.try_greet(name);
        if (m.ok() && (*m)["greeting"].string_or("") == "hello, " + name) good.fetch_add(1);
      }
    });
  }
  for (std::thread& t : pool) t.join();
  check(good.load() == 8 * 500, "8 native threads x 500 greetings, all answered");
}

void the_cpp_library_leaves_when_nothing_holds_it(guatiao::registry& reg) {
  guatiao::instance inst = reg.create("cpp_greeter_shouter", guatiao::make(item::map({{"prefix", "x"}})));
  check(reg.try_unload("cpp_greeter").code() == guatiao::status::busy, "unload is refused while an instance lives");
  inst.close();
  auto unloaded = reg.try_unload("cpp_greeter");
  check(unloaded.ok(), "and done once nothing it served is alive: " +
                           (unloaded.ok() ? std::string() : std::string(unloaded.error().what())));
}

/// Each of the next two unloads a library of its own: no other registry
/// keeps it mapped.
void an_answer_built_in_the_librarys_allocator_outlives_the_unload() {
  guatiao::registry own("cpp-careless", "0.1");
  own.scan_dir(C_CARELESS_GREETER_DIR, guatiao::rules({"kind=greeter"}));
  auto bound = gk::greeter::try_bind(own, "c_greeter");
  check(bound.ok(), "the careless greeter binds");
  if (!bound.ok()) return;
  guatiao::remote<gk::greeter> g = *bound;
  auto answer = g.try_greet("hal");
  check(answer.ok(), "and greets");
  if (!answer.ok()) return;
  // The C greeter has no `unload` function: nothing it handed out is alive,
  // which the caller states.
  check(own.try_unload_unchecked("c_greeter_library").ok(), "its library is unmapped");
  check(own.libraries().size() == 0, "none is left loaded");
  check(json_of(*answer) == "{\"greeting\":\"hello from C, hal\"}", "the answer outlives the unload");
}

void what_a_slot_grew_a_lent_map_with_outlives_the_unload() {
  guatiao::registry own("cpp-lent", "0.1");
  own.scan_dir(C_LENT_GREETER_DIR, guatiao::rules({"kind=greeter"}));
  auto bound = gk::greeter::try_bind(own, "c_greeter");
  check(bound.ok(), "the greeter binds");
  if (!bound.ok()) return;
  guatiao::remote<gk::greeter> g = *bound;
  guatiao::map config = guatiao::map{{"name", "ivy"}};
  auto usable = g.try_check(config.ref());
  check(usable.ok() && *usable, "check grows the lent map");
  check(own.try_unload_unchecked("c_greeter_library").ok(), "its library is unmapped");
  check(json_of(config) == "{\"name\":\"ivy\",\"checked_by\":\"C\"}", "the lent map outlives the unload");
}

/// An allocator that counts the blocks it has out and refuses every
/// allocation while `refuse` is set.
struct refusing {
  guatiao_alloc a{};
  bool refuse = false;
  long live = 0;
  refusing() noexcept {
    a.struct_size = static_cast<std::uint32_t>(sizeof(guatiao_alloc));
    a.ctx = this;
    a.alloc = [](void* ctx, std::size_t size, std::size_t) -> void* {
      auto* self = static_cast<refusing*>(ctx);
      if (self->refuse || size == 0) return nullptr;
      void* p = std::malloc(size);
      if (p) ++self->live;
      return p;
    };
    a.free = [](void* ctx, void* p, std::size_t, std::size_t) {
      if (!p) return;
      --static_cast<refusing*>(ctx)->live;
      std::free(p);
    };
  }
};

void a_refused_move_of_a_lent_tree_is_the_calls_error() {
  // check rebuilds the lent map in its own allocator; moving it back into
  // the map's, which refuses, fails the call and leaves the map whole where
  // it stands.
  refusing alloc;
  guatiao::registry own("cpp-refused", "0.1");
  own.scan_dir(C_CARELESS_GREETER_DIR, guatiao::rules({"kind=greeter"}));
  auto bound = gk::greeter::try_bind(own, "c_greeter");
  check(bound.ok(), "the greeter binds");
  if (!bound.ok()) return;
  guatiao::remote<gk::greeter> g = *bound;
  {
    guatiao::map config({{"name", "ivy"}}, &alloc.a);
    alloc.refuse = true;
    auto usable = g.try_check(config.ref());
    alloc.refuse = false;
    check(!usable.ok() && usable.error().code() == guatiao::status::alloc,
          "a refused settle is the call's error, as ALLOC");
    check(!usable.ok() && std::string(usable.error().what()).find("lent tree") != std::string::npos,
          "naming the lent tree");
    check(json_of(config) == "{\"name\":\"ivy\",\"checked_by\":\"C\"}", "the lent map stands whole");
  }
  check(alloc.live == 0, "every block of the refusing allocator is back");
}

}  // namespace

int main() {
  std::vector<greeter_case> cases = {{"rust", "derived_greeter_hello", "hello, ana"},
                                     {"c", "c_greeter", "hello from C, ana"},
                                     {"python", "py_greeter_hello", "hello, ana"},
                                     {"go", "go_greeter_hello", "hello, ana"},
                                     {"c++", "cpp_greeter_hello", "hello, ana"}};
  {
    guatiao::registry reg("cpp-greeters", "0.1");
    add_sources(reg, cases);
    caller_fn run = open_caller();
    if (!run) skip("no greeter_caller beside the Rust greeters: the Rust proxy is not run");
    for (const greeter_case& c : cases) {
      if (!c.present) continue;
      a_cpp_host_calls_it(reg, c);
      if (run) the_rust_proxy_calls_it(reg, run, c);
    }
    configured_providers(reg);
    native_threads_call_the_cpp_greeter(reg);
    the_cpp_library_leaves_when_nothing_holds_it(reg);
  }
  an_answer_built_in_the_librarys_allocator_outlives_the_unload();
  what_a_slot_grew_a_lent_map_with_outlives_the_unload();
  a_refused_move_of_a_lent_tree_is_the_calls_error();
  std::printf("greeters: %d passed, %d failed, %d skipped\n", passed, failed, skipped);
  return failed ? 1 : 0;
}
