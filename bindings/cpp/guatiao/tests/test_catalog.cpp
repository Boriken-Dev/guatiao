// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The registry's catalog from C++: every provider of a library written in
// C listed with all its fields, the library's digest, and nothing built to
// answer. Through the `try_` forms only, so it runs with exceptions on and
// off.

#include <guatiao/guatiao.hpp>

#include <array>
#include <chrono>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <string>
#include <vector>

#ifdef _WIN32
#include <windows.h>
#else
#include <dlfcn.h>
#endif

#include "support/harness.hpp"

namespace {

namespace fs = std::filesystem;
using guatiao::item;

// ---- SHA-256, to compare with the digest the library reports ------------

std::string sha256_hex(const std::vector<std::uint8_t>& data) {
  static const std::array<std::uint32_t, 64> k = {
      0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
      0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
      0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
      0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
      0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
      0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
      0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
      0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2};
  std::array<std::uint32_t, 8> h = {0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
                                    0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19};
  std::vector<std::uint8_t> m = data;
  const std::uint64_t bits = static_cast<std::uint64_t>(data.size()) * 8;
  m.push_back(0x80);
  while (m.size() % 64 != 56) m.push_back(0);
  for (int i = 7; i >= 0; --i) m.push_back(static_cast<std::uint8_t>(bits >> (8 * i)));
  auto rotr = [](std::uint32_t x, int n) { return (x >> n) | (x << (32 - n)); };
  for (std::size_t chunk = 0; chunk < m.size(); chunk += 64) {
    std::array<std::uint32_t, 64> w{};
    for (int i = 0; i < 16; ++i)
      w[i] = (std::uint32_t{m[chunk + 4 * i]} << 24) | (std::uint32_t{m[chunk + 4 * i + 1]} << 16) |
             (std::uint32_t{m[chunk + 4 * i + 2]} << 8) | std::uint32_t{m[chunk + 4 * i + 3]};
    for (int i = 16; i < 64; ++i) {
      std::uint32_t s0 = rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ (w[i - 15] >> 3);
      std::uint32_t s1 = rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ (w[i - 2] >> 10);
      w[i] = w[i - 16] + s0 + w[i - 7] + s1;
    }
    auto v = h;
    for (int i = 0; i < 64; ++i) {
      std::uint32_t s1 = rotr(v[4], 6) ^ rotr(v[4], 11) ^ rotr(v[4], 25);
      std::uint32_t ch = (v[4] & v[5]) ^ (~v[4] & v[6]);
      std::uint32_t t1 = v[7] + s1 + ch + k[i] + w[i];
      std::uint32_t s0 = rotr(v[0], 2) ^ rotr(v[0], 13) ^ rotr(v[0], 22);
      std::uint32_t maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
      std::uint32_t t2 = s0 + maj;
      v = {t1 + t2, v[0], v[1], v[2], v[3] + t1, v[4], v[5], v[6]};
    }
    for (int i = 0; i < 8; ++i) h[i] += v[i];
  }
  std::vector<std::uint8_t> out;
  for (std::uint32_t word : h)
    for (int i = 3; i >= 0; --i) out.push_back(static_cast<std::uint8_t>(word >> (8 * i)));
  return gt_test::hex_of(out);
}

// ---- the catalog library written in C ----------------------------------

/// The catalog library copied into a directory of its own, so each test
/// counts from zero and may rename the file.
fs::path fresh_copy() {
  static int n = 0;
  fs::path dir = fs::temp_directory_path() /
                 ("guatiao-cpp-catalog-" + std::to_string(std::chrono::steady_clock::now().time_since_epoch().count()) +
                  "-" + std::to_string(++n));
  fs::create_directories(dir);
  fs::path from = GUATIAO_CATALOG_LIBRARY;
  fs::path to = dir / from.filename();
  fs::copy_file(from, to);
  return to;
}

/// How many times any `create` of the library at `path` was called.
int built_count(const fs::path& path) {
#ifdef _WIN32
  HMODULE h = LoadLibraryW(path.c_str());
  if (!h) return -1;
  auto f = reinterpret_cast<int (*)()>(reinterpret_cast<void*>(GetProcAddress(h, "catalog_built")));
#else
  void* h = dlopen(path.c_str(), RTLD_NOW | RTLD_LOCAL);
  if (!h) return -1;
  auto f = reinterpret_cast<int (*)()>(dlsym(h, "catalog_built"));
#endif
  return f ? f() : -1;
}

std::vector<std::uint8_t> bytes_of(const fs::path& path) {
  std::ifstream in(path, std::ios::binary);
  return {std::istreambuf_iterator<char>(in), std::istreambuf_iterator<char>()};
}

void lists_every_provider_and_builds_none() {
  fs::path file = fresh_copy();
  {
    guatiao::registry reg = MUST(guatiao::registry::try_open("catalog-host", "1.0"));
    guatiao::value report = MUST(reg.try_scan_dir(file.parent_path().string()));
    CHECK(report["loaded"].size() == 1 && report["failed"].size() == 0);
    guatiao::value catalog = MUST(reg.try_catalog());
    CHECK(catalog.size() == 4);
    auto entry = [&](std::string_view id) {
      for (guatiao::view e : catalog.as_list())
        if (e["id"].string_or("") == id) return e;
      CHECK_MSG(false, "the catalog lists " + std::string(id));
      return guatiao::view{};
    };
    std::string digest = "sha256:" + sha256_hex(bytes_of(file));

    guatiao::view configured = entry("catalog_configured");
    CHECK(configured["kinds"][0].string_or("") == "counter");
    CHECK(configured["display_name"].string_or("") == "Configured, with a form");
    CHECK(configured["version"].string_or("") == "2.0");
    CHECK(configured["library"].string_or("") == "catalog_library");
    CHECK(configured["library_version"].string_or("") == "1.0");
    CHECK(configured["digest"].string_or("") == digest);
    CHECK(configured["available"].bool_or(false) && !configured.contains("reason"));
    CHECK(configured["config"]["required"][1].string_or("") == "port");
    CHECK(configured["meta"]["form"]["sections"][0]["title"].string_or("") == "Network");

    guatiao::view implied = entry("catalog_implied");
    CHECK(implied["version"].string_or("") == "1.0");
    CHECK(!implied.contains("meta"));
    CHECK(implied["config"]["properties"]["token"]["x-sensitive"].bool_or(false));

    guatiao::view unavailable = entry("catalog_unavailable");
    CHECK(!unavailable["available"].bool_or(true));
    CHECK(unavailable["reason"].string_or("") == "no device here");
    CHECK(!unavailable.contains("config") && !unavailable["has_config"].bool_or(true));
    CHECK(entry("catalog_plain")["available"].bool_or(false));

    // Listing asked the library for nothing: its count says so, and so does
    // its refusal of the first instance asked for afterwards.
    CHECK(built_count(file) == 0);
    guatiao::value config = MUST(guatiao::try_make(item::map({{"host", "localhost"}, {"port", 8080}})));
    auto refused = reg.try_create("catalog_configured", config);
    CHECK(!refused.ok());
    if (!refused.ok())
      CHECK_MSG(std::string(refused.error().message()).find("built nothing: the first create this library was asked") !=
                    std::string::npos,
                refused.error().message());
    CHECK(built_count(file) == 1);
  }
}

void names_why_a_library_has_no_digest() {
  fs::path file = fresh_copy();
  guatiao::registry reg = MUST(guatiao::registry::try_open("catalog-host", "1.0"));
  MUST(reg.try_load_file(file.string()));
  // Another file at the path, before anything asked: renaming a mapped file
  // works on every platform, where overwriting one does not.
  fs::rename(file, file.parent_path() / "moved_away.bin");
  {
    std::ofstream out(file, std::ios::binary);
    out << "not the library";
  }
  guatiao::value catalog = MUST(reg.try_catalog());
  CHECK(catalog.size() == 4);
  for (guatiao::view e : catalog.as_list()) {
    CHECK(!e.contains("digest"));
    CHECK(e["digest_error"].string_or("") == "the file changed since it was loaded");
  }
}

}  // namespace

int main() {
  lists_every_provider_and_builds_none();
  names_why_a_library_has_no_digest();
  return gt_test::finish("catalog");
}
