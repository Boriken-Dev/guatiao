// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// One call into each library, from C++, through the three C headers.
//
// Parses JSON (guatiao_serde.h), reads a path out of it
// (guatiao_intake.h), copies what it found and frees both trees
// (guatiao.h). Prints "all checks passed" on success.

#include <cstdio>
#include <cstdlib>

#include "guatiao.h"
#include "guatiao_serde.h"
#include "guatiao_intake.h"

namespace {

// GUATIAO_ALLOC_MALLOC is C only, so a C++ caller fills the table itself.
void *heap_alloc(void *, size_t size, size_t) {
  return size ? std::malloc(size) : nullptr;
}

void heap_free(void *, void *ptr, size_t, size_t) { std::free(ptr); }

int failures = 0;

void check(bool ok, const char *what) {
  if (!ok) {
    std::printf("FAIL: %s\n", what);
    failures++;
  }
}

}  // namespace

int main() {
  guatiao_alloc alloc = {};
  alloc.struct_size = static_cast<uint32_t>(sizeof(guatiao_alloc));
  alloc.alloc = heap_alloc;
  alloc.free = heap_free;

  guatiao_value doc = {};
  guatiao_status st = guatiao_json_parse(
      guatiao_cstr("{\"agent\":[{\"name\":\"a\"},{\"name\":\"b\"}]}"), 0,
      &alloc, &doc);
  check(st == GUATIAO_OK, "guatiao_json_parse");

  const guatiao_value *name =
      guatiao_intake_path_get(&doc, guatiao_cstr("agent[1].name"));
  check(name != nullptr, "guatiao_intake_path_get");

  guatiao_value copy = {};
  st = guatiao_value_clone(&alloc, name, &copy);
  check(st == GUATIAO_OK, "guatiao_value_clone");
  check(guatiao_str_eq(guatiao_string_text(&copy), guatiao_cstr("b")),
        "the copy reads \"b\"");

  check(guatiao_value_free(&copy) == GUATIAO_OK, "free the copy");
  check(guatiao_value_free(&doc) == GUATIAO_OK, "free the document");

  if (failures != 0) {
    return 1;
  }
  std::printf("all checks passed\n");
  return 0;
}
