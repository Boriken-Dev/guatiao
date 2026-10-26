/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * A MEX file a test calls as a library with threads of its own would: it
 * calls a greeter's `greet` slot on the interpreter's thread, from threads
 * it joins and from one it leaves running; destroys an object from another
 * thread; and runs greeter_caller_run and greeter_caller_wide, the Rust
 * callers of any greeter and of any provider of the wide kind.
 * Exports are found at run time from the files the test names, so this
 * links nothing of guatiao. Addresses cross as uint64 scalars.
 */

#include <stdio.h>
#include <string.h>

#include "mex.h"

#include <greeter_kind.h>
#include <guatiao.h>

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

static char failure[256];

static void *open_symbol(const mxArray *path, const char *name) {
  char p[4096];
  if (!mxIsChar(path) || mxGetString(path, p, sizeof p)) return NULL;
#ifdef _WIN32
  HMODULE h = LoadLibraryExA(p, NULL, LOAD_WITH_ALTERED_SEARCH_PATH);
  return h ? (void *)GetProcAddress(h, name) : NULL;
#else
  void *h = dlopen(p, RTLD_NOW | RTLD_LOCAL);
  return h ? dlsym(h, name) : NULL;
#endif
}

static void *addr(const mxArray *a) {
  if (!a || mxGetNumberOfElements(a) != 1) return NULL;
  if (mxGetClassID(a) == mxUINT64_CLASS) return (void *)(uintptr_t) * (const uint64_t *)mxGetData(a);
  return (void *)(uintptr_t)mxGetScalar(a);
}

static value_free_fn free_of(const mxArray *lib) {
  value_free_fn f = NULL;
  void *p = open_symbol(lib, "guatiao_value_free");
  if (p) memcpy(&f, &p, sizeof f);
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

static int fill(job *j, int nrhs, const mxArray *prhs[], int calls) {
  memset(j, 0, sizeof *j);
  if (nrhs < 6) return 0;
  j->table = (const greeter_vtable *)addr(prhs[2]);
  j->ctx = addr(prhs[3]);
  j->free_value = free_of(prhs[1]);
  if (!j->table || !j->free_value) return 0;
  mxGetString(prhs[4], j->name, sizeof j->name);
  mxGetString(prhs[5], j->expect, sizeof j->expect);
  j->calls = calls;
  return 1;
}

static mxArray *row(int n, const double *v) {
  mxArray *x = mxCreateDoubleMatrix(1, (mwSize)n, mxREAL);
  memcpy(mxGetData(x), v, sizeof(double) * (size_t)n);
  return x;
}

#define MAX_THREADS 16
static job detached;

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

static void free_err(value_free_fn free_value, guatiao_provider_error *err) {
  if (!err->message.cap) return;
  guatiao_value m;
  memset(&m, 0, sizeof m);
  m.tag = GUATIAO_STRING;
  m.payload.text = err->message;
  free_value(&m);
}

/* A listener of this file's: counts how many were destroyed. */
static int listeners_destroyed = 0;
static void fx_listener_destroy(void *ctx) {
  (void)ctx;
  listeners_destroyed++;
}
static guatiao_status fx_listener_heard(void *ctx, guatiao_str what) {
  (void)ctx;
  (void)what;
  return GUATIAO_OK;
}
static const listener_vtable FX_LISTENER = {
    {(uint32_t)sizeof(listener_vtable), listener_vtable_FLOOR_HASH}, fx_listener_destroy, fx_listener_heard};

/* Each command answers into plhs[0]; a refusal sets `failure`, raised at
 * the gateway's top below. */
static int dispatch(const char *cmd, int nlhs, mxArray *plhs[], int nrhs, const mxArray *prhs[]) {
  (void)nlhs;
  if (!strcmp(cmd, "greet_here")) {
    /* greet_here(lib, table, ctx, name, expect, calls) -> [ok refused other last] */
    job j;
    if (nrhs < 7 || !fill(&j, nrhs, prhs, (int)mxGetScalar(prhs[6]))) return 0;
    call_greet(&j);
    double v[4] = {j.ok, j.refused, j.other, (double)j.last};
    plhs[0] = row(4, v);
    return 1;
  }
  if (!strcmp(cmd, "greet_threads")) {
    /* greet_threads(lib, table, ctx, name, expect, threads, calls) -> [ok refused other] */
    if (nrhs < 8) return 0;
    int n = (int)mxGetScalar(prhs[6]);
    if (n < 1 || n > MAX_THREADS) return 0;
    job jobs[MAX_THREADS];
    for (int i = 0; i < n; i++)
      if (!fill(&jobs[i], nrhs, prhs, (int)mxGetScalar(prhs[7]))) return 0;
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
    double v[3] = {0, 0, 0};
    for (int i = 0; i < n; i++) {
      v[0] += jobs[i].ok;
      v[1] += jobs[i].refused;
      v[2] += jobs[i].other;
    }
    plhs[0] = row(3, v);
    return 1;
  }
  if (!strcmp(cmd, "greet_detached")) {
    /* greet_detached(lib, table, ctx, name, expect): one thread left running */
    if (!fill(&detached, nrhs, prhs, 1)) return 0;
#ifdef _WIN32
    HANDLE t = CreateThread(NULL, 0, run, &detached, 0, NULL);
    if (!t) return 0;
    CloseHandle(t);
#else
    pthread_t t;
    if (pthread_create(&t, NULL, run, &detached)) return 0;
    pthread_detach(t);
#endif
    plhs[0] = mxCreateDoubleMatrix(0, 0, mxREAL);
    return 1;
  }
  if (!strcmp(cmd, "detached")) {
    /* detached() -> the status the detached thread's call answered, [] until done */
    plhs[0] = detached.done ? mxCreateDoubleScalar((double)detached.last) : mxCreateDoubleMatrix(0, 0, mxREAL);
    return 1;
  }
  if (!strcmp(cmd, "destroy_on_thread")) {
    /* destroy_on_thread(table, size, ctx) */
    if (nrhs < 4) return 0;
    destroy_job d;
    d.object.table = addr(prhs[1]);
    d.object.size = (size_t)mxGetScalar(prhs[2]);
    d.object.ctx = addr(prhs[3]);
    if (!d.object.table) return 0;
#ifdef _WIN32
    HANDLE t = CreateThread(NULL, 0, run_destroy, &d, 0, NULL);
    WaitForSingleObject(t, INFINITE);
    CloseHandle(t);
#else
    pthread_t t;
    pthread_create(&t, NULL, run_destroy, &d);
    pthread_join(t, NULL);
#endif
    plhs[0] = mxCreateDoubleMatrix(0, 0, mxREAL);
    return 1;
  }
  if (!strcmp(cmd, "greeter_caller")) {
    /* greeter_caller(caller path, host, key, name, out node) -> status */
    if (nrhs < 6) return 0;
    void *p = open_symbol(prhs[1], "greeter_caller_run");
    if (!p) {
      snprintf(failure, sizeof failure, "greeter_caller_run not found");
      return 0;
    }
    caller_fn run_caller;
    memcpy(&run_caller, &p, sizeof run_caller);
    char k[256], n[256];
    mxGetString(prhs[3], k, sizeof k);
    mxGetString(prhs[4], n, sizeof n);
    guatiao_status st = run_caller((const guatiao_host_info *)addr(prhs[2]), guatiao_str_from(k, strlen(k)),
                                   guatiao_str_from(n, strlen(n)), (guatiao_value *)addr(prhs[5]));
    plhs[0] = mxCreateDoubleScalar((double)st);
    return 1;
  }
  if (!strcmp(cmd, "wide_caller")) {
    /* wide_caller(caller path, host, key, out node) -> status */
    if (nrhs < 5) return 0;
    void *p = open_symbol(prhs[1], "greeter_caller_wide");
    if (!p) {
      snprintf(failure, sizeof failure, "greeter_caller_wide not found");
      return 0;
    }
    wide_caller_fn run_wide;
    memcpy(&run_wide, &p, sizeof run_wide);
    char k[256];
    mxGetString(prhs[3], k, sizeof k);
    guatiao_status st = run_wide((const guatiao_host_info *)addr(prhs[2]), guatiao_str_from(k, strlen(k)),
                                 (guatiao_value *)addr(prhs[4]));
    plhs[0] = mxCreateDoubleScalar((double)st);
    return 1;
  }
  if (!strcmp(cmd, "greet_into_alloc")) {
    /* greet_into_alloc(lib, table, ctx, name, alloc) -> [status, whether the
     * answer's map records that allocator]; the answer is freed */
    if (nrhs < 6) return 0;
    value_free_fn free_value = free_of(prhs[1]);
    const greeter_vtable *t = (const greeter_vtable *)addr(prhs[2]);
    const guatiao_alloc *a = (const guatiao_alloc *)addr(prhs[5]);
    if (!free_value || !t) return 0;
    char n[256];
    mxGetString(prhs[4], n, sizeof n);
    guatiao_map out;
    memset(&out, 0, sizeof out);
    out.alloc = a;
    guatiao_provider_error err;
    memset(&err, 0, sizeof err);
    guatiao_status st = t->greet(addr(prhs[3]), guatiao_str_from(n, strlen(n)), &out, &err);
    int same = st == GUATIAO_OK && out.alloc == a;
    if (st == GUATIAO_OK) {
      guatiao_value v;
      memset(&v, 0, sizeof v);
      v.tag = GUATIAO_MAP;
      v.payload.map = out;
      free_value(&v);
    }
    free_err(free_value, &err);
    double v[2] = {(double)st, same};
    plhs[0] = row(2, v);
    return 1;
  }
  if (!strcmp(cmd, "library_unload")) {
    /* library_unload(info) -> what the descriptor's own unload slot answers */
    const guatiao_library_info *i = nrhs > 1 ? (const guatiao_library_info *)addr(prhs[1]) : NULL;
    if (!i) return 0;
    plhs[0] = mxCreateDoubleScalar(i->unload ? (double)i->unload() : -1);
    return 1;
  }
  if (!strcmp(cmd, "create_null_out")) {
    /* create_null_out(lib, info, which, config node) -> a create slot's status given nowhere to write */
    if (nrhs < 5) return 0;
    value_free_fn free_value = free_of(prhs[1]);
    const guatiao_library_info *i = (const guatiao_library_info *)addr(prhs[2]);
    if (!free_value || !i) return 0;
    const guatiao_provider_info *p =
        (const guatiao_provider_info *)((const char *)i->providers.ptr + i->providers.stride * (size_t)mxGetScalar(prhs[3]));
    guatiao_provider_error err;
    memset(&err, 0, sizeof err);
    guatiao_status st = p->create(p->ctx, (const guatiao_value *)addr(prhs[4]), NULL, &err);
    free_err(free_value, &err);
    plhs[0] = mxCreateDoubleScalar((double)st);
    return 1;
  }
  if (!strcmp(cmd, "both")) {
    /* both(table, ctx, a, b) -> the status of a pair's both(a, b), both
     * handed over: the map at each node, left absent, or null for 0 */
    if (nrhs < 5 || !addr(prhs[1])) return 0;
    typedef guatiao_status (*both_fn)(void *ctx, guatiao_map *a, guatiao_map *b, int64_t *out);
    both_fn both;
    memcpy(&both, (const char *)addr(prhs[1]) + sizeof(guatiao_kind_header), sizeof both);
    guatiao_map m[2];
    guatiao_map *given[2] = {NULL, NULL};
    for (int i = 0; i < 2; i++) {
      guatiao_value *v = (guatiao_value *)addr(prhs[3 + i]);
      if (!v) continue;
      m[i] = v->payload.map;
      memset(v, 0, sizeof *v);
      given[i] = &m[i];
    }
    int64_t out = 0;
    plhs[0] = mxCreateDoubleScalar((double)both(addr(prhs[2]), given[0], given[1], &out));
    return 1;
  }
  if (!strcmp(cmd, "put_refused")) {
    /* put_refused(table, ctx) -> the status of a pocket's put(label, item):
     * a label that is not UTF-8, then a listener of this file's handed over */
    if (nrhs < 3 || !addr(prhs[1])) return 0;
    typedef guatiao_status (*put_fn)(void *ctx, guatiao_str label, guatiao_object item, guatiao_provider_error *err);
    put_fn put;
    memcpy(&put, (const char *)addr(prhs[1]) + sizeof(guatiao_kind_header), sizeof put);
    guatiao_object item = {&FX_LISTENER, sizeof FX_LISTENER, &listeners_destroyed};
    guatiao_provider_error err;
    memset(&err, 0, sizeof err);
    guatiao_str label = {(const uint8_t *)"\xff", 1};
    guatiao_status st = put(addr(prhs[2]), label, item, &err);
    if (err.message.cap) err.message.alloc->free(err.message.alloc->ctx, (void *)err.message.ptr, err.message.cap, 1);
    plhs[0] = mxCreateDoubleScalar((double)st);
    return 1;
  }
  if (!strcmp(cmd, "listeners_destroyed")) {
    /* listeners_destroyed() -> how many of this file's listeners were destroyed */
    plhs[0] = mxCreateDoubleScalar((double)listeners_destroyed);
    return 1;
  }
  if (!strcmp(cmd, "entry_answers")) {
    /* entry_answers(entry) -> whether the entry, called with no host, answers a descriptor */
    void *p = nrhs > 1 ? addr(prhs[1]) : NULL;
    if (!p) return 0;
    typedef const guatiao_library_info *(*entry_fn)(const guatiao_host_info *);
    entry_fn e;
    memcpy(&e, &p, sizeof e);
    plhs[0] = mxCreateLogicalScalar(e(NULL) != NULL);
    return 1;
  }
  if (!strcmp(cmd, "shelf")) {
    /* shelf(path, control, arg): the C shelf's shelf_take, shelf_put,
     * shelf_refuse or shelf_closed -> what shelf_closed answers, else [] */
    if (nrhs < 4) return 0;
    char control[32];
    mxGetString(prhs[2], control, sizeof control);
    void *p = open_symbol(prhs[1], control);
    if (!p) {
      snprintf(failure, sizeof failure, "no control %s", control);
      return 0;
    }
    int arg = (int)mxGetScalar(prhs[3]);
    if (!strcmp(control, "shelf_closed")) {
      int (*f)(int);
      memcpy(&f, &p, sizeof f);
      plhs[0] = mxCreateDoubleScalar((double)f(arg));
    } else if (!strcmp(control, "shelf_refuse")) {
      void (*f)(bool);
      memcpy(&f, &p, sizeof f);
      f(arg != 0);
      plhs[0] = mxCreateDoubleMatrix(0, 0, mxREAL);
    } else {
      void (*f)(int);
      memcpy(&f, &p, sizeof f);
      f(arg);
      plhs[0] = mxCreateDoubleMatrix(0, 0, mxREAL);
    }
    return 1;
  }
  snprintf(failure, sizeof failure, "no command %s", cmd);
  return 0;
}

void mexFunction(int nlhs, mxArray *plhs[], int nrhs, const mxArray *prhs[]) {
  char cmd[64];
  if (nrhs < 1 || !mxIsChar(prhs[0]) || mxGetString(prhs[0], cmd, sizeof cmd))
    mexErrMsgIdAndTxt("gtfixture:usage", "gtfixture(command, ...)");
  failure[0] = '\0';
  if (!dispatch(cmd, nlhs, plhs, nrhs, prhs))
    mexErrMsgIdAndTxt("gtfixture:refused", "%s", failure[0] ? failure : "bad arguments");
}
