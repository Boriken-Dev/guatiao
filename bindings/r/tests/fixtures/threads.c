/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * C a test calls through .Call, as a library with threads of its own
 * would: it calls a greeter's `greet` slot on R's thread, from threads it
 * joins and from one it leaves running; destroys an object from another
 * thread; and runs greeter_caller_run and greeter_caller_wide, the Rust
 * callers of any greeter and any wide provider.
 * Exports are found at run time from the files the test names, so this
 * links nothing of guatiao.
 */

#define R_NO_REMAP
#include <R.h>
#include <Rinternals.h>
#include <string.h>

#include <guatiao.h>
#include <greeter_kind.h>

#ifdef _WIN32
#include <windows.h>
#else
#include <dlfcn.h>
#include <pthread.h>
#endif

typedef guatiao_status (*value_free_fn)(guatiao_value *value);
typedef guatiao_status (*caller_fn)(const guatiao_host_info *host, guatiao_str key, guatiao_str name,
                                    guatiao_value *out);
typedef guatiao_status (*wide_caller_fn)(const guatiao_host_info *host, guatiao_str key, guatiao_value *out);

static void *open_symbol(SEXP path, const char *name) {
  const char *p = CHAR(STRING_ELT(path, 0));
#ifdef _WIN32
  HMODULE h = LoadLibraryExA(p, NULL, LOAD_WITH_ALTERED_SEARCH_PATH);
  return h ? (void *)GetProcAddress(h, name) : NULL;
#else
  void *h = dlopen(p, RTLD_NOW | RTLD_LOCAL);
  return h ? dlsym(h, name) : NULL;
#endif
}

static value_free_fn free_of(SEXP lib) {
  value_free_fn f;
  void *p = open_symbol(lib, "guatiao_value_free");
  if (!p) Rf_error("guatiao_value_free not found");
  memcpy(&f, &p, sizeof f);
  return f;
}

typedef struct job {
  const greeter_vtable *table;
  void *ctx;
  value_free_fn free_value;
  char name[64];
  char expect[64];
  int calls;
  int ok;
  int refused;
  int other;
  guatiao_status last;
  volatile int done;
} job;

static void call_greet(job *j) {
  guatiao_str name = {(const uint8_t *)j->name, strlen(j->name)};
  size_t want = strlen(j->expect);
  for (int i = 0; i < j->calls; i++) {
    guatiao_map out;
    guatiao_provider_error err;
    memset(&out, 0, sizeof out);
    memset(&err, 0, sizeof err);
    guatiao_status status = j->table->greet(j->ctx, name, &out, &err);
    j->last = status;
    if (status == GUATIAO_OK) {
      int ok = 0;
      for (size_t k = 0; k < out.len; k++) {
        const guatiao_entry *e = &out.ptr[k];
        if (e->key.len == 8 && memcmp(e->key.ptr, "greeting", 8) == 0 && e->value.tag == GUATIAO_STRING &&
            e->value.payload.text.len == want && memcmp(e->value.payload.text.ptr, j->expect, want) == 0) {
          ok = 1;
        }
      }
      guatiao_value answer;
      memset(&answer, 0, sizeof answer);
      answer.tag = GUATIAO_MAP;
      answer.payload.map = out;
      j->free_value(&answer);
      if (ok) j->ok++;
      else j->other++;
    } else {
      if (status == GUATIAO_ERR_UNSUPPORTED) j->refused++;
      else j->other++;
    }
    if (err.message.cap) {
      guatiao_value message;
      memset(&message, 0, sizeof message);
      message.tag = GUATIAO_STRING;
      message.payload.text = err.message;
      j->free_value(&message);
    }
  }
  j->done = 1;
}

#ifdef _WIN32
static DWORD WINAPI run(LPVOID p) {
  call_greet((job *)p);
  return 0;
}
#else
static void *run(void *p) {
  call_greet((job *)p);
  return NULL;
}
#endif

static void copy_text(char *dst, size_t cap, const char *src) {
  size_t n = strlen(src);
  if (n >= cap) n = cap - 1;
  memcpy(dst, src, n);
  dst[n] = '\0';
}

static void fill(job *j, SEXP lib, SEXP table, SEXP ctx, SEXP name, SEXP expect, int calls) {
  memset(j, 0, sizeof *j);
  j->table = (const greeter_vtable *)R_ExternalPtrAddr(table);
  j->ctx = R_ExternalPtrAddr(ctx);
  j->free_value = free_of(lib);
  copy_text(j->name, sizeof j->name, CHAR(STRING_ELT(name, 0)));
  copy_text(j->expect, sizeof j->expect, CHAR(STRING_ELT(expect, 0)));
  j->calls = calls;
}

/* Calls greet `calls` times on R's own thread, the way a host's call does. */
SEXP fx_greet_here(SEXP lib, SEXP table, SEXP ctx, SEXP name, SEXP expect, SEXP calls) {
  job j;
  fill(&j, lib, table, ctx, name, expect, Rf_asInteger(calls));
  call_greet(&j);
  SEXP out = PROTECT(Rf_allocVector(INTSXP, 4));
  INTEGER(out)[0] = j.ok;
  INTEGER(out)[1] = j.refused;
  INTEGER(out)[2] = j.other;
  INTEGER(out)[3] = (int)j.last;
  UNPROTECT(1);
  return out;
}

#define MAX_THREADS 16

/* From `threads` threads this waits for, each calling greet `calls` times. */
SEXP fx_greet_threads(SEXP lib, SEXP table, SEXP ctx, SEXP name, SEXP expect, SEXP threads, SEXP calls) {
  int n = Rf_asInteger(threads);
  if (n < 1 || n > MAX_THREADS) Rf_error("1 to %d threads", MAX_THREADS);
  job jobs[MAX_THREADS];
  for (int i = 0; i < n; i++) fill(&jobs[i], lib, table, ctx, name, expect, Rf_asInteger(calls));
#ifdef _WIN32
  HANDLE t[MAX_THREADS];
  for (int i = 0; i < n; i++) t[i] = CreateThread(NULL, 0, run, &jobs[i], 0, NULL);
  WaitForMultipleObjects((DWORD)n, t, TRUE, INFINITE);
  for (int i = 0; i < n; i++) CloseHandle(t[i]);
#else
  pthread_t t[MAX_THREADS];
  for (int i = 0; i < n; i++) pthread_create(&t[i], NULL, run, &jobs[i]);
  for (int i = 0; i < n; i++) pthread_join(t[i], NULL);
#endif
  SEXP out = PROTECT(Rf_allocVector(INTSXP, 3));
  INTEGER(out)[0] = INTEGER(out)[1] = INTEGER(out)[2] = 0;
  for (int i = 0; i < n; i++) {
    INTEGER(out)[0] += jobs[i].ok;
    INTEGER(out)[1] += jobs[i].refused;
    INTEGER(out)[2] += jobs[i].other;
  }
  UNPROTECT(1);
  return out;
}

static job detached;

/* One thread calling greet while R goes on; fx_detached says how it went. */
SEXP fx_greet_detached(SEXP lib, SEXP table, SEXP ctx, SEXP name, SEXP expect) {
  fill(&detached, lib, table, ctx, name, expect, 1);
#ifdef _WIN32
  HANDLE t = CreateThread(NULL, 0, run, &detached, 0, NULL);
  if (!t) Rf_error("no thread");
  CloseHandle(t);
#else
  pthread_t t;
  if (pthread_create(&t, NULL, run, &detached)) Rf_error("no thread");
  pthread_detach(t);
#endif
  return R_NilValue;
}

SEXP fx_detached(void) {
  if (!detached.done) return Rf_ScalarInteger(NA_INTEGER);
  return Rf_ScalarInteger((int)detached.last);
}

typedef struct destroy_job {
  guatiao_object object;
} destroy_job;

static void destroy_now(destroy_job *d) {
  void (*destroy)(void *ctx) = *(void (*const *)(void *))((const char *)d->object.table + 8);
  destroy(d->object.ctx);
}

#ifdef _WIN32
static DWORD WINAPI run_destroy(LPVOID p) {
  destroy_now((destroy_job *)p);
  return 0;
}
#else
static void *run_destroy(void *p) {
  destroy_now((destroy_job *)p);
  return NULL;
}
#endif

/* Destroys an object of an object kind from a thread this waits for. */
SEXP fx_destroy_on_thread(SEXP table, SEXP size, SEXP ctx) {
  destroy_job d;
  d.object.table = R_ExternalPtrAddr(table);
  d.object.size = (size_t)Rf_asReal(size);
  d.object.ctx = (void *)(uintptr_t)Rf_asReal(ctx);
#ifdef _WIN32
  HANDLE t = CreateThread(NULL, 0, run_destroy, &d, 0, NULL);
  WaitForSingleObject(t, INFINITE);
  CloseHandle(t);
#else
  pthread_t t;
  pthread_create(&t, NULL, run_destroy, &d);
  pthread_join(t, NULL);
#endif
  return R_NilValue;
}

/* greeter_caller_run from the library at `caller`, writing into `out`. */
SEXP fx_greeter_caller(SEXP caller, SEXP host, SEXP key, SEXP name, SEXP out) {
  void *p = open_symbol(caller, "greeter_caller_run");
  if (!p) Rf_error("greeter_caller_run not found");
  caller_fn run_caller;
  memcpy(&run_caller, &p, sizeof run_caller);
  const char *k = CHAR(STRING_ELT(key, 0));
  const char *n = CHAR(STRING_ELT(name, 0));
  guatiao_status st = run_caller((const guatiao_host_info *)R_ExternalPtrAddr(host), guatiao_str_from(k, strlen(k)),
                                 guatiao_str_from(n, strlen(n)), (guatiao_value *)R_ExternalPtrAddr(out));
  return Rf_ScalarInteger((int)st);
}

/* greeter_caller_wide from the library at `caller`, writing into `out`. */
SEXP fx_wide_caller(SEXP caller, SEXP host, SEXP key, SEXP out) {
  void *p = open_symbol(caller, "greeter_caller_wide");
  if (!p) Rf_error("greeter_caller_wide not found");
  wide_caller_fn run_caller;
  memcpy(&run_caller, &p, sizeof run_caller);
  const char *k = CHAR(STRING_ELT(key, 0));
  guatiao_status st = run_caller((const guatiao_host_info *)R_ExternalPtrAddr(host), guatiao_str_from(k, strlen(k)),
                                 (guatiao_value *)R_ExternalPtrAddr(out));
  return Rf_ScalarInteger((int)st);
}

/* greet with a carrier naming `alloc`: c(status, whether the answer's map
 * records that allocator); the answer is freed before this returns. */
SEXP fx_greet_into_alloc(SEXP lib, SEXP table, SEXP ctx, SEXP name, SEXP alloc) {
  value_free_fn free_value = free_of(lib);
  const greeter_vtable *t = (const greeter_vtable *)R_ExternalPtrAddr(table);
  const guatiao_alloc *a = (const guatiao_alloc *)R_ExternalPtrAddr(alloc);
  const char *n = CHAR(STRING_ELT(name, 0));
  guatiao_map out;
  memset(&out, 0, sizeof out);
  out.alloc = a;
  guatiao_provider_error err;
  memset(&err, 0, sizeof err);
  guatiao_status st = t->greet(R_ExternalPtrAddr(ctx), guatiao_str_from(n, strlen(n)), &out, &err);
  int same = st == GUATIAO_OK && out.alloc == a;
  if (st == GUATIAO_OK) {
    guatiao_value v;
    memset(&v, 0, sizeof v);
    v.tag = GUATIAO_MAP;
    v.payload.map = out;
    free_value(&v);
  }
  if (err.message.cap) {
    guatiao_value m;
    memset(&m, 0, sizeof m);
    m.tag = GUATIAO_STRING;
    m.payload.text = err.message;
    free_value(&m);
  }
  SEXP res = PROTECT(Rf_allocVector(INTSXP, 2));
  INTEGER(res)[0] = (int)st;
  INTEGER(res)[1] = same;
  UNPROTECT(1);
  return res;
}

/* A library descriptor's own unload slot, asked directly. */
SEXP fx_library_unload(SEXP info) {
  const guatiao_library_info *i = (const guatiao_library_info *)R_ExternalPtrAddr(info);
  return Rf_ScalarInteger(i->unload ? (int)i->unload() : -1);
}

/* A provider's create slot given nowhere to write the instance. */
SEXP fx_create_null_out(SEXP lib, SEXP info, SEXP which, SEXP config) {
  value_free_fn free_value = free_of(lib);
  const guatiao_library_info *i = (const guatiao_library_info *)R_ExternalPtrAddr(info);
  const guatiao_provider_info *p =
      (const guatiao_provider_info *)((const char *)i->providers.ptr + i->providers.stride * (size_t)Rf_asInteger(which));
  guatiao_provider_error err;
  memset(&err, 0, sizeof err);
  guatiao_status st = p->create(p->ctx, (const guatiao_value *)R_ExternalPtrAddr(config), NULL, &err);
  if (err.message.cap) {
    guatiao_value m;
    memset(&m, 0, sizeof m);
    m.tag = GUATIAO_STRING;
    m.payload.text = err.message;
    free_value(&m);
  }
  return Rf_ScalarInteger((int)st);
}

/* A library's entry, called with no host: whether it answers a descriptor. */
SEXP fx_entry_answers(SEXP entry) {
  typedef const guatiao_library_info *(*entry_fn)(const guatiao_host_info *);
  entry_fn e;
  void *p = R_ExternalPtrAddr(entry);
  memcpy(&e, &p, sizeof e);
  return Rf_ScalarLogical(e(NULL) != NULL);
}

/* The C shelf's controls: `op` is take, put, refuse or closed. */
SEXP fx_shelf(SEXP path, SEXP op, SEXP arg) {
  const char *o = CHAR(STRING_ELT(op, 0));
  void *p = open_symbol(path, strcmp(o, "take") == 0     ? "shelf_take"
                              : strcmp(o, "put") == 0    ? "shelf_put"
                              : strcmp(o, "refuse") == 0 ? "shelf_refuse"
                                                         : "shelf_closed");
  if (!p) Rf_error("no such control");
  if (strcmp(o, "closed") == 0) {
    int (*f)(int);
    memcpy(&f, &p, sizeof f);
    return Rf_ScalarInteger(f(Rf_asInteger(arg)));
  }
  if (strcmp(o, "refuse") == 0) {
    void (*f)(bool);
    memcpy(&f, &p, sizeof f);
    f(Rf_asLogical(arg) == 1);
    return R_NilValue;
  }
  void (*f)(int);
  memcpy(&f, &p, sizeof f);
  f(Rf_asInteger(arg));
  return R_NilValue;
}
