/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/* The source of libimports.so (aarch64 Linux): a shared library that only
 * IMPORTS the entry symbol, so its dynamic symbol table names it as
 * undefined. Not a library; the scan tests read it as data. */
extern const void *guatiao_library_entry(const void *host);
const void *call(void) { return guatiao_library_entry(0); }
