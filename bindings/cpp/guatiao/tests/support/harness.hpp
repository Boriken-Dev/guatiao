// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// What the C++ suites share: counted checks, stated skips, reading a
// repository file, and unwrapping a result a test cannot go on without.
// Written against the `try_` forms, so a suite builds with exceptions on
// and off alike.

#ifndef GT_TEST_HARNESS_HPP
#define GT_TEST_HARNESS_HPP

#include <guatiao/guatiao.hpp>

#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <sstream>
#include <string>

namespace gt_test {

struct tally_t {
  int passed = 0;
  int failed = 0;
  int skipped = 0;
};

inline tally_t& tally() {
  static tally_t t;
  return t;
}

inline bool require_all() {
  const char* v = std::getenv("GUATIAO_REQUIRE_ALL");
  return v && std::string(v) == "1";
}

inline bool check(bool ok, const std::string& what, const char* file, int line) {
  if (ok) {
    ++tally().passed;
  } else {
    ++tally().failed;
    std::printf("  FAIL %s:%d: %s\n", file, line, what.c_str());
  }
  return ok;
}

/// A stated skip; a failure under GUATIAO_REQUIRE_ALL=1.
inline void skip(const std::string& why) {
  if (require_all()) {
    ++tally().failed;
    std::printf("  FAIL (GUATIAO_REQUIRE_ALL) skipped: %s\n", why.c_str());
  } else {
    ++tally().skipped;
    std::printf("  SKIP %s\n", why.c_str());
  }
}

inline int finish(const char* name) {
  std::printf("%s: %d passed, %d failed, %d skipped\n", name, tally().passed, tally().failed, tally().skipped);
  return tally().failed ? 1 : 0;
}

/// The value of a result a test cannot go on without; stops the program
/// with the error otherwise.
template <class T>
T must(guatiao::result<T>&& r, const char* what, const char* file, int line) {
  if (!r.ok()) {
    std::printf("  FATAL %s:%d: %s: %s (status %u)\n", file, line, what, r.error().what(),
                static_cast<unsigned>(r.code()));
    std::exit(2);
  }
  return std::move(*r);
}

inline void must(guatiao::result<void>&& r, const char* what, const char* file, int line) {
  if (!r.ok()) {
    std::printf("  FATAL %s:%d: %s: %s (status %u)\n", file, line, what, r.error().what(),
                static_cast<unsigned>(r.code()));
    std::exit(2);
  }
}

/// A file of the repository, by its path from the root.
inline std::string read_repo_file(const std::string& rel) {
  std::string path = std::string(GUATIAO_REPO) + "/" + rel;
  std::ifstream in(path, std::ios::binary);
  if (!in) {
    std::printf("  FATAL cannot read %s\n", path.c_str());
    std::exit(2);
  }
  std::ostringstream out;
  out << in.rdbuf();
  return out.str();
}

inline std::string hex_of(const std::vector<std::uint8_t>& b) {
  static const char* digits = "0123456789abcdef";
  std::string s;
  for (std::uint8_t c : b) {
    s.push_back(digits[c >> 4]);
    s.push_back(digits[c & 15]);
  }
  return s;
}

inline std::vector<std::uint8_t> bytes_of_hex(std::string_view h) {
  auto nib = [](char c) -> int { return c <= '9' ? c - '0' : (c | 0x20) - 'a' + 10; };
  std::vector<std::uint8_t> out;
  for (std::size_t i = 0; i + 1 < h.size(); i += 2)
    out.push_back(static_cast<std::uint8_t>((nib(h[i]) << 4) | nib(h[i + 1])));
  return out;
}

}  // namespace gt_test

#define CHECK(cond) ::gt_test::check(static_cast<bool>(cond), #cond, __FILE__, __LINE__)
#define CHECK_MSG(cond, msg) ::gt_test::check(static_cast<bool>(cond), (msg), __FILE__, __LINE__)
#define MUST(expr) ::gt_test::must((expr), #expr, __FILE__, __LINE__)

#endif  // GT_TEST_HARNESS_HPP
