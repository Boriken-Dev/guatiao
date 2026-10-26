/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

#ifdef _WIN32
#include <windows.h>
#else
#include <dlfcn.h>
#include <pthread.h>
#endif

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "native.h"

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
static void *gt_handle_of[3];

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

static void gt_close_file(void *h) {
#ifdef _WIN32
  FreeLibrary((HMODULE)h);
#else
  dlclose(h);
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

#define GT_RESOLVE(table, name, feature) \
  if (ok && !gt_resolve(h, (void **)&table.name, #name, feature, missing, err, errlen)) ok = 0;

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
#define X(ret, name, params, args, feature) GT_RESOLVE(gt_core, name, feature)
#define GT_VOID(name, params, args, feature) GT_RESOLVE(gt_core, name, feature)
    GT_CORE(X)
#undef X
#undef GT_VOID
    break;
  case GT_LIB_SERDE:
#define X(ret, name, params, args, feature) GT_RESOLVE(gt_serde, name, feature)
    GT_SERDE(X)
#undef X
    break;
  default:
#define X(ret, name, params, args, feature) GT_RESOLVE(gt_intake, name, feature)
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
  gt_handle_of[lib] = h;
  return 0;
}

int gt_file_carries(const char *path, const char *symbol) {
  char err[256];
  void *h = gt_open_file(path, err, sizeof err);
  if (!h) return 0;
  int found = gt_symbol(h, symbol) != NULL;
  gt_close_file(h);
  return found;
}

int gt_is_open(int lib) { return lib >= 0 && lib <= 2 && gt_handle_of[lib] != NULL; }

const char *gt_missing_feature(int lib, const char *name) {
  if (lib < 0 || lib > 2) return NULL;
  for (int i = 0; i < gt_missing_of[lib].len; i++)
    if (strcmp(gt_missing_of[lib].name[i], name) == 0) return gt_missing_of[lib].feature[i];
  return NULL;
}

void *gt_symbol_of(int lib, const char *name) {
  if (!gt_is_open(lib)) return NULL;
  return gt_symbol(gt_handle_of[lib], name);
}

/* ---- allocators ---------------------------------------------------------- */

static void *gt_malloc_alloc(void *ctx, size_t size, size_t align) {
  (void)ctx;
  (void)align;
  return size ? malloc(size) : NULL;
}

static void gt_malloc_free(void *ctx, void *p, size_t size, size_t align) {
  (void)ctx;
  (void)size;
  (void)align;
  free(p);
}

const guatiao_alloc gt_binding_alloc = {(uint32_t)sizeof(guatiao_alloc), NULL, gt_malloc_alloc, gt_malloc_free,
                                        NULL};

void *gt_malloc(size_t n) { return n ? malloc(n) : NULL; }

int gt_getenv(const char *name, char *buf, size_t n) {
#ifdef _WIN32
  wchar_t wname[128];
  wchar_t wval[4096];
  if (!MultiByteToWideChar(CP_UTF8, 0, name, -1, wname, 128)) return 0;
  DWORD len = GetEnvironmentVariableW(wname, wval, 4096);
  if (len == 0 || len >= 4096) return 0;
  return WideCharToMultiByte(CP_UTF8, 0, wval, -1, buf, (int)n, NULL, NULL) > 0;
#else
  const char *v = getenv(name);
  if (!v || strlen(v) >= n) return 0;
  memcpy(buf, v, strlen(v) + 1);
  return 1;
#endif
}

static void *gt_counting_alloc(void *ctx, size_t size, size_t align) {
  (void)align;
  gt_counting *c = (gt_counting *)ctx;
  if (size == 0 || c->refusing) return NULL;
  void *p = malloc(size);
  if (p) {
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

/* Kept for the process: every tree built through one records its address. */
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

int64_t gt_counting_live(const gt_counting *c) {
#ifdef _WIN32
  return InterlockedCompareExchange64((volatile LONG64 *)&c->live, 0, 0);
#else
  return __atomic_load_n(&c->live, __ATOMIC_SEQ_CST);
#endif
}

/* ---- the interpreter's thread ------------------------------------------- */

#ifdef _WIN32
static DWORD gt_main_thread;
static CRITICAL_SECTION gt_mutex;
static int gt_mutex_ready;
void gt_note_main_thread(void) {
  gt_main_thread = GetCurrentThreadId();
  if (!gt_mutex_ready) {
    InitializeCriticalSection(&gt_mutex);
    gt_mutex_ready = 1;
  }
}
int gt_on_main_thread(void) { return GetCurrentThreadId() == gt_main_thread; }
void gt_lock(void) { EnterCriticalSection(&gt_mutex); }
void gt_unlock(void) { LeaveCriticalSection(&gt_mutex); }
#else
static pthread_t gt_main_thread;
static pthread_mutex_t gt_mutex = PTHREAD_MUTEX_INITIALIZER;
void gt_note_main_thread(void) { gt_main_thread = pthread_self(); }
int gt_on_main_thread(void) { return pthread_equal(pthread_self(), gt_main_thread); }
void gt_lock(void) { pthread_mutex_lock(&gt_mutex); }
void gt_unlock(void) { pthread_mutex_unlock(&gt_mutex); }
#endif
