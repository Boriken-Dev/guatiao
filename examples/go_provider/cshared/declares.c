/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/* What this library declares, read by a scan as data before the file is
 * mapped: one KEY=VALUE per NUL-terminated string, an empty one last. */
#ifdef _WIN32
__declspec(dllexport)
#else
__attribute__((visibility("default")))
#endif
const char guatiao_declares[] = "kind=greeter\0kind=counter\0";
