/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * A library and a source in one file that imports `import_dep`, so it
 * maps only where the loader finds that library. Its entry point declines
 * every host and its source refuses every configuration: the answer a
 * host gets back says the file mapped, and nothing else is needed.
 *
 * With IMPORT_PLUGIN_TWO it imports `import_dep2` as well. With
 * IMPORT_PLUGIN_LIBRARY its entry point answers a library with no
 * providers whose `unload` slot agrees, so a host can unload it.
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
#if defined(IMPORT_PLUGIN_TWO)
IMPORT int import_dep_second_mark(void);
#endif

#if defined(IMPORT_PLUGIN_LIBRARY)
#include <string.h>

static guatiao_status agree(void) { return GUATIAO_OK; }

static guatiao_library_info LIBRARY;
#endif

EXPORT const guatiao_library_info *guatiao_library_entry(const guatiao_host_info *host) {
  (void)host;
  (void)import_dep_mark();
#if defined(IMPORT_PLUGIN_TWO)
  (void)import_dep_second_mark();
#endif
#if defined(IMPORT_PLUGIN_LIBRARY)
  memset(&LIBRARY, 0, sizeof(LIBRARY));
  LIBRARY.struct_size = (uint32_t)sizeof(guatiao_library_info);
  LIBRARY.abi_version = GUATIAO_ABI_VERSION;
  LIBRARY.id = guatiao_cstr("import_plugin");
  LIBRARY.version = guatiao_cstr("1.0");
  LIBRARY.providers.stride = sizeof(guatiao_provider_info);
  LIBRARY.unload = agree;
  return &LIBRARY;
#else
  return NULL;
#endif
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
