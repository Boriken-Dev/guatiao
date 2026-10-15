/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/* The source of libexports.so (aarch64 Linux) and libexports.dylib
 * (arm64 macOS): a shared library DEFINING the entry symbol. The scan
 * tests read them as data and never map them. */
const void *guatiao_library_entry(const void *host) { return 0; }
