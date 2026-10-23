// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// An allocator that counts the blocks it has out: a test builds through
// it, lets everything go, and checks the count is back to zero.

#ifndef GT_TEST_COUNTING_HPP
#define GT_TEST_COUNTING_HPP

#include <guatiao/guatiao.hpp>

#include <atomic>
#include <cstdlib>

namespace gt_test {

class counting {
 public:
  counting() noexcept {
    a_.struct_size = static_cast<std::uint32_t>(sizeof(guatiao_alloc));
    a_.ctx = this;
    a_.alloc = &counting::alloc_fn;
    a_.free = &counting::free_fn;
    a_.release = nullptr;
  }
  counting(const counting&) = delete;
  counting& operator=(const counting&) = delete;

  const guatiao_alloc* get() const noexcept { return &a_; }
  long live() const noexcept { return live_.load(); }
  long total() const noexcept { return total_.load(); }
  /// Refuse every allocation after `n` more, to walk the failure paths;
  /// a negative `n` allows them all again.
  void fail_after(long n) noexcept {
    budget_.store(n < 0 ? 0 : n);
    limited_.store(n >= 0);
  }

 private:
  static void* alloc_fn(void* ctx, std::size_t size, std::size_t) {
    auto* self = static_cast<counting*>(ctx);
    if (self->limited_.load()) {
      long left = self->budget_.load();
      do {
        if (left <= 0) return nullptr;
      } while (!self->budget_.compare_exchange_weak(left, left - 1));
    }
    void* p = std::malloc(size);
    if (p) {
      self->live_.fetch_add(1);
      self->total_.fetch_add(1);
    }
    return p;
  }
  static void free_fn(void* ctx, void* p, std::size_t, std::size_t) {
    if (!p) return;
    static_cast<counting*>(ctx)->live_.fetch_sub(1);
    std::free(p);
  }

  guatiao_alloc a_{};
  std::atomic<long> live_{0};
  std::atomic<long> total_{0};
  std::atomic<long> budget_{0};
  std::atomic<bool> limited_{false};
};

}  // namespace gt_test

#endif  // GT_TEST_COUNTING_HPP
