/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "native.h"

#ifdef _WIN32
#include <windows.h>
#else
#include <dlfcn.h>
#endif

gt_core_fns gt_core;
gt_serde_fns gt_serde;
gt_intake_fns gt_intake;

#define GT_MAX_MISSING 64

typedef struct gt_missing_list {
  int len;
  const char *name[GT_MAX_MISSING];
  const char *feature[GT_MAX_MISSING];
} gt_missing_list;

static gt_missing_list gt_missing_of[3];

static void *gt_open_file(const char *path, char *err, size_t errlen) {
#ifdef _WIN32
  int wide = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, path, -1, NULL, 0);
  if (wide <= 0) {
    snprintf(err, errlen, "the path is not UTF-8");
    return NULL;
  }
  wchar_t *wpath = (wchar_t *)malloc(sizeof(wchar_t) * (size_t)wide);
  if (!wpath) {
    snprintf(err, errlen, "out of memory");
    return NULL;
  }
  MultiByteToWideChar(CP_UTF8, 0, path, -1, wpath, wide);
  /* A path with a directory loads the libraries it needs from beside it. */
  DWORD flags = (wcschr(wpath, L'\\') || wcschr(wpath, L'/')) ? LOAD_WITH_ALTERED_SEARCH_PATH : 0;
  HMODULE h = LoadLibraryExW(wpath, NULL, flags);
  free(wpath);
  if (!h) snprintf(err, errlen, "LoadLibraryExW failed with error %lu", (unsigned long)GetLastError());
  return (void *)h;
#else
  void *h = dlopen(path, RTLD_NOW | RTLD_LOCAL);
  if (!h) {
    const char *why = dlerror();
    snprintf(err, errlen, "%s", why ? why : "dlopen failed");
  }
  return h;
#endif
}

static void *gt_symbol(void *h, const char *name) {
#ifdef _WIN32
  return (void *)GetProcAddress((HMODULE)h, name);
#else
  return dlsym(h, name);
#endif
}

/* Resolves one export into its field; records an optional one that is
 * absent, and answers 0 for a required one that is. */
static int gt_resolve(void *h, void **field, const char *name, const char *feature, gt_missing_list *missing,
                      char *err, size_t errlen) {
  *field = gt_symbol(h, name);
  if (*field) return 1;
  if (feature[0] == '\0') {
    snprintf(err, errlen, "%s", name);
    return 0;
  }
  if (missing->len < GT_MAX_MISSING) {
    missing->name[missing->len] = name;
    missing->feature[missing->len] = feature;
    missing->len++;
  }
  return 1;
}

#define GT_RESOLVE(table, ret, name, params, args, feature)                                         \
  if (ok && !gt_resolve(h, (void **)&table.name, #name, feature, missing, err, errlen)) ok = 0;
#define GT_RESOLVE_VOID(table, name, params, args, feature)                                         \
  if (ok && !gt_resolve(h, (void **)&table.name, #name, feature, missing, err, errlen)) ok = 0;

static void gt_close_file(void *h) {
#ifdef _WIN32
  FreeLibrary((HMODULE)h);
#else
  dlclose(h);
#endif
}

int gt_open(int lib, const char *path, char *err, size_t errlen) {
  if (lib < 0 || lib > 2) {
    snprintf(err, errlen, "no such library");
    return 1;
  }
  void *h = gt_open_file(path, err, errlen);
  if (!h) return 1;
  gt_missing_list *missing = &gt_missing_of[lib];
  missing->len = 0;
  int ok = 1;
  switch (lib) {
  case GT_LIB_CORE:
#define X(ret, name, params, args, feature) GT_RESOLVE(gt_core, ret, name, params, args, feature)
#define GT_VOID(name, params, args, feature) GT_RESOLVE_VOID(gt_core, name, params, args, feature)
    GT_CORE(X)
#undef X
#undef GT_VOID
    break;
  case GT_LIB_SERDE:
#define X(ret, name, params, args, feature) GT_RESOLVE(gt_serde, ret, name, params, args, feature)
    GT_SERDE(X)
#undef X
    break;
  default:
#define X(ret, name, params, args, feature) GT_RESOLVE(gt_intake, ret, name, params, args, feature)
    GT_INTAKE(X)
#undef X
    break;
  }
  if (!ok) {
    /* Nothing of a refused file is called: its table is cleared. */
    switch (lib) {
    case GT_LIB_CORE: memset(&gt_core, 0, sizeof gt_core); break;
    case GT_LIB_SERDE: memset(&gt_serde, 0, sizeof gt_serde); break;
    default: memset(&gt_intake, 0, sizeof gt_intake); break;
    }
    missing->len = 0;
    gt_close_file(h);
    return 2;
  }
  /* The library stays open for the process: trees it built record its
   * allocator, whose code lives in its image. */
  return 0;
}

int gt_missing(int lib, int index, const char **name, const char **feature) {
  if (lib < 0 || lib > 2 || index < 0 || index >= gt_missing_of[lib].len) return 0;
  *name = gt_missing_of[lib].name[index];
  *feature = gt_missing_of[lib].feature[index];
  return 1;
}

/* ---- a counting allocator, for the tests that prove every path frees ---- */

static void *gt_counting_alloc(void *ctx, size_t size, size_t align) {
  (void)align;
  if (size == 0) return NULL;
  void *p = malloc(size);
  if (p) {
    gt_counting *c = (gt_counting *)ctx;
#ifdef _WIN32
    InterlockedIncrement64((volatile LONG64 *)&c->live);
    InterlockedIncrement64((volatile LONG64 *)&c->total);
#else
    __atomic_add_fetch(&c->live, 1, __ATOMIC_SEQ_CST);
    __atomic_add_fetch(&c->total, 1, __ATOMIC_SEQ_CST);
#endif
  }
  return p;
}

static void gt_counting_release(void *ctx, void *p, size_t size, size_t align) {
  (void)size;
  (void)align;
  if (!p) return;
  gt_counting *c = (gt_counting *)ctx;
#ifdef _WIN32
  InterlockedDecrement64((volatile LONG64 *)&c->live);
#else
  __atomic_sub_fetch(&c->live, 1, __ATOMIC_SEQ_CST);
#endif
  free(p);
}

gt_counting *gt_counting_new(void) {
  gt_counting *c = (gt_counting *)calloc(1, sizeof(gt_counting));
  if (!c) return NULL;
  c->alloc.struct_size = (uint32_t)sizeof(guatiao_alloc);
  c->alloc.ctx = c;
  c->alloc.alloc = gt_counting_alloc;
  c->alloc.free = gt_counting_release;
  c->alloc.release = NULL;
  return c;
}

void gt_counting_free(gt_counting *c) { free(c); }

int64_t gt_counting_live(const gt_counting *c) {
#ifdef _WIN32
  return InterlockedCompareExchange64((volatile LONG64 *)&c->live, 0, 0);
#else
  return __atomic_load_n(&c->live, __ATOMIC_SEQ_CST);
#endif
}
