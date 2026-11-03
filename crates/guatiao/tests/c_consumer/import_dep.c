/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * A shared library another library imports: `import_plugin.c` links it,
 * and `tests/import_search.rs` places this file where the platform's
 * loader may or may not look for it.
 *
 * With IMPORT_DEP_REFUSES (Windows only), its DllMain fails the attach,
 * so a library bound to this copy does not load: the test plants it where
 * the loader must not look first.
 *
 * With IMPORT_DEP_SECOND it is a second library, `import_dep2`, exporting
 * `import_dep_second_mark` instead.
 *
 * With IMPORT_DEP_FORWARDS (Windows, an MSVC-style linker) it is
 * `import_fwd`, whose `import_dep_mark` forwards to `import_dep`'s, as the
 * stable ABI's `python3.dll` forwards to `python3XX.dll`.
 */

#if defined(_WIN32)
#define EXPORT __declspec(dllexport)
#else
#define EXPORT __attribute__((visibility("default")))
#endif

#if defined(IMPORT_DEP_FORWARDS)
#pragma comment(linker, "/export:import_dep_mark=import_dep.import_dep_mark")
EXPORT int import_fwd_own(void) { return 0; }
#elif defined(IMPORT_DEP_SECOND)
EXPORT int import_dep_second_mark(void) { return 7; }
#else
EXPORT int import_dep_mark(void) { return 7; }
#endif

#if defined(_WIN32) && defined(IMPORT_DEP_REFUSES)
#include <windows.h>

BOOL WINAPI DllMain(HINSTANCE instance, DWORD reason, LPVOID reserved) {
  (void)instance;
  (void)reserved;
  return reason == DLL_PROCESS_ATTACH ? FALSE : TRUE;
}
#endif
