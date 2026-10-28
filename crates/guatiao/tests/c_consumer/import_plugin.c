/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * A library and a source in one file that imports `import_dep`, so it
 * maps only where the loader finds that library. Its entry point declines
 * every host and its source refuses every configuration: the answer a
 * host gets back says the file mapped, and nothing else is needed.
 */

#include "guatiao.h"

#if defined(_WIN32)
#define EXPORT __declspec(dllexport)
#define IMPORT __declspec(dllimport)
#else
#define EXPORT __attribute__((visibility("default")))
#define IMPORT
#endif

IMPORT int import_dep_mark(void);

EXPORT const guatiao_library_info *guatiao_library_entry(const guatiao_host_info *host) {
  (void)host;
  (void)import_dep_mark();
  return NULL;
}

EXPORT guatiao_status guatiao_source(const guatiao_host_info *host, const guatiao_value *config,
                                     const guatiao_source_info **out,
                                     guatiao_provider_error *err) {
  (void)host;
  (void)config;
  (void)out;
  (void)err;
  return import_dep_mark() == 7 ? GUATIAO_ERR_UNSUPPORTED : GUATIAO_ERR_INTERNAL;
}
