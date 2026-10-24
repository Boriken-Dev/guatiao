// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// 64-bit integers cross a slot exactly: through a provider written in
// Rust, C or C++, and from Rust into the C++ one. A slot's C++ types are
// std::int64_t and std::uint64_t, so what a call passes converts as C++
// converts it (-1 to a uint64_t is UINT64_MAX): nothing is out of range at
// run time.

#include "greeter_kind.hpp"

#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <limits>
#include <string>

#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#else
#include <dlfcn.h>
#endif

namespace {

namespace gk = greeter_kind;

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

constexpr std::int64_t i64_min = std::numeric_limits<std::int64_t>::min();
constexpr std::int64_t i64_max = std::numeric_limits<std::int64_t>::max();
constexpr std::uint64_t u64_max = std::numeric_limits<std::uint64_t>::max();
constexpr std::int64_t two53_1 = (std::int64_t{1} << 53) + 1;

const std::int64_t signed_limits[3] = {two53_1, i64_min, i64_max};
const std::uint64_t unsigned_limits[3] = {static_cast<std::uint64_t>(two53_1), std::uint64_t{1} << 63, u64_max};
const char* const signed_text[3] = {"9007199254740993", "-9223372036854775808", "9223372036854775807"};
const char* const unsigned_text[3] = {"9007199254740993", "9223372036854775808", "18446744073709551615"};

/// Answers what it is handed, and refuses 7.
class echo {
 public:
  std::int64_t echo_i64(std::int64_t x) {
    if (x == 7) throw guatiao::error(guatiao::status::bad_value, "seven is no number");
    return x;
  }
  std::uint64_t echo_u64(std::uint64_t x) {
    if (x == 7) throw guatiao::error(guatiao::status::bad_value, "seven is no number");
    return x;
  }
};

guatiao::library wide_library() {
  guatiao::library lib("cpp_wide", "0.1.0");
  lib.single<echo, gk::wide>("echo");
  return lib;
}

struct provider {
  const char* who;
  const char* key;
  bool present = false;
};

void a_slot_carries_64_bit_integers_exactly(guatiao::registry& reg, const provider& p) {
  std::string who = p.who;
  auto bound = gk::wide::try_bind(reg, p.key);
  check(bound.ok(), who + ": bound as a wide provider");
  if (!bound.ok()) return;
  guatiao::remote<gk::wide> w = *bound;
  for (int k = 0; k < 3; ++k) {
    auto i = w.try_echo_i64(signed_limits[k]);
    check(i.ok() && *i == signed_limits[k], who + ": echo_i64 " + signed_text[k]);
    auto u = w.try_echo_u64(unsigned_limits[k]);
    check(u.ok() && *u == unsigned_limits[k], who + ": echo_u64 " + unsigned_text[k]);
  }
}

void a_cpp_provider_refuses_with_its_words(guatiao::registry& reg) {
  auto w = gk::wide::bind(reg, "cpp_wide_echo");
  auto i = w.try_echo_i64(7);
  check(i.code() == guatiao::status::bad_value && i.error().message().find("seven is no number") != std::string::npos,
        "echo_i64 refuses seven with BAD_VALUE and its words");
  auto u = w.try_echo_u64(7);
  check(u.code() == guatiao::status::bad_value && u.error().message().find("seven is no number") != std::string::npos,
        "echo_u64 refuses seven with BAD_VALUE and its words");
}

using wide_caller_fn = guatiao_status (*)(const guatiao_host_info*, guatiao_str, guatiao_value*);

wide_caller_fn open_caller() {
  std::string rust = env("GUATIAO_RUST_GREETERS");
  if (rust.empty()) rust = RUST_GREETERS_DIR;
  std::string path = library_file(rust, "greeter_caller");
#if defined(_WIN32)
  HMODULE lib = LoadLibraryA(path.c_str());
  if (!lib) return nullptr;
  return reinterpret_cast<wide_caller_fn>(reinterpret_cast<void*>(GetProcAddress(lib, "greeter_caller_wide")));
#else
  void* lib = dlopen(path.c_str(), RTLD_NOW | RTLD_LOCAL);
  if (!lib) return nullptr;
  return reinterpret_cast<wide_caller_fn>(dlsym(lib, "greeter_caller_wide"));
#endif
}

void the_rust_proxy_calls_it(guatiao::registry& reg, wide_caller_fn run, const provider& p) {
  std::string who = p.who;
  guatiao_value out = guatiao::detail::blank();
  guatiao_status s = run(reg.host(), guatiao::detail::str(p.key), &out);
  guatiao::value report = guatiao::value::adopt(out);
  check(s == GUATIAO_OK, who + ": the Rust proxy ran");
  for (int k = 0; k < 3; ++k) {
    check(report["i64"][static_cast<std::size_t>(k)].number_text().value_or("") == signed_text[k],
          who + " through Rust: i64 " + signed_text[k]);
    check(report["u64"][static_cast<std::size_t>(k)].number_text().value_or("") == unsigned_text[k],
          who + " through Rust: u64 " + unsigned_text[k]);
  }
}

}  // namespace

int main() {
  provider providers[3] = {{"rust", "derived_greeter_hello"}, {"c", "c_greeter"}, {"c++", "cpp_wide_echo"}};
  {
    guatiao::registry reg("cpp-wide", "0.1");
    std::string rust = env("GUATIAO_RUST_GREETERS");
    if (rust.empty()) rust = RUST_GREETERS_DIR;
    if (exists(library_file(rust, "derived_greeter"))) {
      reg.watch_dir(rust, guatiao::rules({"kind=greeter"}));
      providers[0].present = true;
    } else {
      std::printf("  FAIL no derived_greeter in %s: set GUATIAO_RUST_GREETERS to the workspace's build\n", rust.c_str());
      ++failed;
    }
    reg.watch_dir(C_GREETER_DIR, guatiao::rules({"kind=greeter"}));
    providers[1].present = true;
    reg.refresh();
    reg.register_library<wide_library>("cpp_wide");
    providers[2].present = true;

    wide_caller_fn run = open_caller();
    check(run != nullptr, "greeter_caller_wide is found beside the Rust greeters");
    for (const provider& p : providers) {
      if (!p.present) continue;
      a_slot_carries_64_bit_integers_exactly(reg, p);
      if (run) the_rust_proxy_calls_it(reg, run, p);
    }
    a_cpp_provider_refuses_with_its_words(reg);
  }
  std::printf("wide: %d passed, %d failed, %d skipped\n", passed, failed, skipped);
  return failed ? 1 : 0;
}
