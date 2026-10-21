// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Package threads calls a greeter table's greet slot from native threads
// Go did not start, as a library's own threads would: the examples' tests
// use it on a greeter written in Go.
package threads

/*
#cgo CFLAGS: -I${SRCDIR}/../../../crates/guatiao/include
#cgo linux LDFLAGS: -ldl -lpthread
#include <stdlib.h>
#include <string.h>
#ifdef _WIN32
#include <windows.h>
#else
#include <dlfcn.h>
#include <pthread.h>
#endif
#include "guatiao.h"

typedef guatiao_status (*th_greet)(void *ctx, guatiao_str name, guatiao_map *out, guatiao_provider_error *err);
typedef guatiao_status (*th_free)(guatiao_value *v);
typedef guatiao_status (*th_default)(guatiao_alloc *out);

typedef struct th_job {
  th_greet greet;
  th_free free_value;
  void *ctx;
  const guatiao_alloc *alloc;
  int calls;
  int bad;
  size_t want_len;
} th_job;

static void th_run(th_job *job) {
  for (int i = 0; i < job->calls; i++) {
    guatiao_map out;
    memset(&out, 0, sizeof out);
    out.alloc = job->alloc;
    guatiao_provider_error err;
    memset(&err, 0, sizeof err);
    guatiao_str name = {(const uint8_t *)"ana", 3};
    guatiao_status st = job->greet(job->ctx, name, &out, &err);
    guatiao_value v;
    memset(&v, 0, sizeof v);
    v.tag = GUATIAO_MAP;
    v.payload.map = out;
    if (st != 0 || out.len != 1 || out.ptr[0].value.payload.text.len != job->want_len) job->bad++;
    job->free_value(&v);
  }
}

#ifdef _WIN32
static DWORD WINAPI th_main(LPVOID p) { th_run((th_job *)p); return 0; }
#else
static void *th_main(void *p) { th_run((th_job *)p); return NULL; }
#endif

// Runs `threads` native threads, each calling greet `calls` times;
// answers how many calls answered wrong, or -1 when the library would not
// open.
static int th_greet_from_threads(const char *library, void *table, uintptr_t ctx, int threads, int calls,
                                 size_t want_len) {
#ifdef _WIN32
  HMODULE h = LoadLibraryA(library);
  if (!h) return -1;
  th_free free_value = (th_free)(void *)GetProcAddress(h, "guatiao_value_free");
  th_default alloc_default = (th_default)(void *)GetProcAddress(h, "guatiao_alloc_default");
#else
  void *h = dlopen(library, RTLD_NOW | RTLD_LOCAL);
  if (!h) return -1;
  th_free free_value = (th_free)dlsym(h, "guatiao_value_free");
  th_default alloc_default = (th_default)dlsym(h, "guatiao_alloc_default");
#endif
  if (!free_value || !alloc_default) return -1;
  static guatiao_alloc alloc;
  alloc_default(&alloc);
  th_greet greet = *(th_greet *)((char *)table + sizeof(guatiao_kind_header));
  th_job *jobs = calloc((size_t)threads, sizeof(th_job));
#ifdef _WIN32
  HANDLE *ids = calloc((size_t)threads, sizeof(HANDLE));
#else
  pthread_t *ids = calloc((size_t)threads, sizeof(pthread_t));
#endif
  for (int i = 0; i < threads; i++) {
    jobs[i] = (th_job){greet, free_value, (void *)ctx, &alloc, calls, 0, want_len};
#ifdef _WIN32
    ids[i] = CreateThread(NULL, 0, th_main, &jobs[i], 0, NULL);
#else
    pthread_create(&ids[i], NULL, th_main, &jobs[i]);
#endif
  }
  int bad = 0;
  for (int i = 0; i < threads; i++) {
#ifdef _WIN32
    WaitForSingleObject(ids[i], INFINITE);
    CloseHandle(ids[i]);
#else
    pthread_join(ids[i], NULL);
#endif
    bad += jobs[i].bad;
  }
  free(ids);
  free(jobs);
  return bad;
}

static long th_live;
static void *th_count_alloc(void *ctx, size_t size, size_t align) {
  (void)ctx;
  (void)align;
  th_live++;
  return malloc(size);
}
static void th_count_free(void *ctx, void *p, size_t size, size_t align) {
  (void)ctx;
  (void)size;
  (void)align;
  if (p) th_live--;
  free(p);
}

// Calls greet once with the out map naming a counting allocator: answers 1
// when the map and its key and text were built in it, 0 when not, -1 when
// the library would not open; *live is what is still out once the answer
// is freed.
static int th_greet_in_carrier(const char *library, void *table, uintptr_t ctx, long *live) {
#ifdef _WIN32
  HMODULE h = LoadLibraryA(library);
  th_free free_value = h ? (th_free)(void *)GetProcAddress(h, "guatiao_value_free") : NULL;
#else
  void *h = dlopen(library, RTLD_NOW | RTLD_LOCAL);
  th_free free_value = h ? (th_free)dlsym(h, "guatiao_value_free") : NULL;
#endif
  if (!free_value) return -1;
  static guatiao_alloc counting;
  counting.struct_size = (uint32_t)sizeof(guatiao_alloc);
  counting.alloc = th_count_alloc;
  counting.free = th_count_free;
  th_greet greet = *(th_greet *)((char *)table + sizeof(guatiao_kind_header));
  guatiao_map out;
  memset(&out, 0, sizeof out);
  out.alloc = &counting;
  guatiao_provider_error err;
  memset(&err, 0, sizeof err);
  guatiao_str name = {(const uint8_t *)"ana", 3};
  if (greet((void *)ctx, name, &out, &err) != 0) return 0;
  int home = out.alloc == &counting && out.len == 1 && out.ptr[0].key.alloc == &counting &&
             out.ptr[0].value.payload.text.alloc == &counting && th_live > 0;
  guatiao_value v;
  memset(&v, 0, sizeof v);
  v.tag = GUATIAO_MAP;
  v.payload.map = out;
  free_value(&v);
  *live = th_live;
  return home;
}
*/
import "C"

import (
	"fmt"
	"unsafe"
)

// Greet calls the greet slot of table over ctx from threads native
// threads, calls times each, and answers how many answers were not a
// one-key map holding a text of wantLen bytes.
func Greet(library string, table unsafe.Pointer, ctx uintptr, threads, calls, wantLen int) (int, error) {
	clib := C.CString(library)
	defer C.free(unsafe.Pointer(clib))
	bad := C.th_greet_from_threads(clib, table, C.uintptr_t(ctx), C.int(threads), C.int(calls), C.size_t(wantLen))
	if bad < 0 {
		return 0, fmt.Errorf("%s does not open", library)
	}
	return int(bad), nil
}

// GreetInCarrier calls the greet slot once with its out map naming an
// allocator of the caller's, and reports whether the answer was built in
// it, and how many blocks of it are still out once the answer is freed.
func GreetInCarrier(library string, table unsafe.Pointer, ctx uintptr) (bool, int, error) {
	clib := C.CString(library)
	defer C.free(unsafe.Pointer(clib))
	var live C.long
	home := C.th_greet_in_carrier(clib, table, C.uintptr_t(ctx), &live)
	if home < 0 {
		return false, 0, fmt.Errorf("%s does not open", library)
	}
	return home == 1, int(live), nil
}
