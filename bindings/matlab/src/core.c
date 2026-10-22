/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/* Calls and refusals, scratch memory, text, libraries, the handle table,
 * owners and the walk to a node. */

#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>

#include "gt.h"

/* ---- refusals ------------------------------------------------------------------ */

static int fail_va(gt_call *c, guatiao_status st, const char *detail, const char *fmt, va_list ap) {
  c->status = st ? st : GUATIAO_ERR_INTERNAL;
  snprintf(c->detail, sizeof c->detail, "%s", detail ? detail : "");
  vsnprintf(c->msg, sizeof c->msg, fmt, ap);
  return 0;
}

int gt_fail(gt_call *c, guatiao_status st, const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  fail_va(c, st, NULL, fmt, ap);
  va_end(ap);
  return 0;
}

int gt_fail_detail(gt_call *c, guatiao_status st, const char *detail, const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  fail_va(c, st, detail, fmt, ap);
  va_end(ap);
  return 0;
}

void gt_err_free(guatiao_provider_error *err) {
  if (!err || !err->message.cap) return;
  guatiao_value m;
  memset(&m, 0, sizeof m);
  m.tag = GUATIAO_STRING;
  m.payload.text = err->message;
  gt_free(&m);
  memset(&err->message, 0, sizeof err->message);
}

int gt_fail_err(gt_call *c, guatiao_status st, guatiao_provider_error *err, const char *fallback) {
  size_t n = 0;
  char buf[sizeof c->msg];
  if (err && err->message.len && err->message.ptr) {
    n = err->message.len < sizeof buf - 1 ? err->message.len : sizeof buf - 1;
    memcpy(buf, err->message.ptr, n);
  }
  buf[n] = '\0';
  gt_err_free(err);
  if (n == 0) return gt_fail(c, st, "%s", fallback ? fallback : "");
  return gt_fail(c, st, "%s", buf);
}

const char *gt_status_name(guatiao_status st) {
  switch (st) {
  case GUATIAO_OK: return "OK";
  case GUATIAO_ERR_BAD_VALUE: return "BadValue";
  case GUATIAO_ERR_ALLOC: return "AllocFailed";
  case GUATIAO_ERR_WRONG_KIND: return "WrongKind";
  case GUATIAO_ERR_NOT_FOUND: return "NotFound";
  case GUATIAO_ERR_NULL: return "NullArgument";
  case GUATIAO_ERR_GONE: return "Gone";
  case GUATIAO_ERR_UNSUPPORTED: return "Unsupported";
  case GUATIAO_ERR_BUSY: return "Busy";
  case GUATIAO_ERR_INTERNAL: return "Internal";
  default: return "Unknown";
  }
}

/* ---- scratch --------------------------------------------------------------------- */

void *gt_scratch(gt_call *c, size_t n) {
  /* The header takes 16 bytes, so what follows keeps malloc's alignment. */
  gt_block *b = (gt_block *)malloc(16 + n);
  if (!b) {
    gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
    return NULL;
  }
  b->next = c->scratch;
  c->scratch = b;
  return (char *)b + 16;
}

void gt_scratch_release(gt_call *c) {
  while (c->scratch) {
    gt_block *next = c->scratch->next;
    free(c->scratch);
    c->scratch = next;
  }
}

/* ---- arguments ---------------------------------------------------------------------- */

const mxArray *gt_arg(gt_call *c, int i) { return i >= 0 && i < c->nrhs ? c->prhs[i] : NULL; }

int gt_arg_text(gt_call *c, int i, const char *what, guatiao_str *out) {
  const mxArray *a = gt_arg(c, i);
  if (!a) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "%s is missing", what);
  return gt_mx_utf8(c, a, what, out);
}

int gt_arg_double(gt_call *c, int i, const char *what, double *out) {
  const mxArray *a = gt_arg(c, i);
  if (!a || mxGetNumberOfElements(a) != 1 || !(mxIsNumeric(a) || mxIsLogical(a)) || mxIsComplex(a))
    return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "%s is one real number", what);
  *out = mxGetScalar(a);
  return 1;
}

int gt_arg_bool(gt_call *c, int i, const char *what, int *out) {
  double d = 0;
  if (!gt_arg_double(c, i, what, &d)) return 0;
  *out = d != 0;
  return 1;
}

int gt_arg_id(gt_call *c, int i, uint64_t *out) {
  double d = 0;
  if (!gt_arg_double(c, i, "a handle", &d)) return 0;
  if (!(d >= 1 && d <= 9007199254740992.0 && d == floor(d)))
    return gt_fail_detail(c, GUATIAO_ERR_GONE, "closed", "the value, registry or handle is closed");
  *out = (uint64_t)d;
  return 1;
}

void gt_answer(gt_call *c, int i, mxArray *x) {
  if (i == 0 || i < c->nlhs) c->plhs[i] = x;
  else mxDestroyArray(x);
}

/* ---- text ------------------------------------------------------------------------------ */

/* MATLAB's mxChar is a UTF-16 code unit; Octave's is a byte of UTF-8. */
#define GT_WIDE_CHARS (sizeof(mxChar) == 2)

int gt_mx_utf8(gt_call *c, const mxArray *a, const char *what, guatiao_str *out) {
  if (!mxIsChar(a) || mxGetNumberOfDimensions(a) > 2 || (mxGetM(a) > 1 && mxGetNumberOfElements(a) > 0))
    return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "%s is text (a character row)", what);
  size_t n = mxGetNumberOfElements(a);
  const mxChar *ch = mxGetChars(a);
  if (!GT_WIDE_CHARS) {
    char *p = (char *)gt_scratch(c, n + 1);
    if (!p) return 0;
    if (n) memcpy(p, ch, n);
    p[n] = '\0';
    *out = guatiao_str_from(p, n);
    return 1;
  }
  const uint16_t *u = (const uint16_t *)ch;
  unsigned char *p = (unsigned char *)gt_scratch(c, n * 3 + 1);
  if (!p) return 0;
  size_t k = 0;
  for (size_t i = 0; i < n; i++) {
    uint32_t cp = u[i];
    if (cp >= 0xD800 && cp <= 0xDBFF) {
      if (i + 1 >= n || u[i + 1] < 0xDC00 || u[i + 1] > 0xDFFF)
        return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "%s holds a lone surrogate, which is not text", what);
      cp = 0x10000 + ((cp - 0xD800) << 10) + (u[i + 1] - 0xDC00);
      i++;
    } else if (cp >= 0xDC00 && cp <= 0xDFFF) {
      return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "%s holds a lone surrogate, which is not text", what);
    }
    if (cp < 0x80) {
      p[k++] = (unsigned char)cp;
    } else if (cp < 0x800) {
      p[k++] = (unsigned char)(0xC0 | (cp >> 6));
      p[k++] = (unsigned char)(0x80 | (cp & 0x3F));
    } else if (cp < 0x10000) {
      p[k++] = (unsigned char)(0xE0 | (cp >> 12));
      p[k++] = (unsigned char)(0x80 | ((cp >> 6) & 0x3F));
      p[k++] = (unsigned char)(0x80 | (cp & 0x3F));
    } else {
      p[k++] = (unsigned char)(0xF0 | (cp >> 18));
      p[k++] = (unsigned char)(0x80 | ((cp >> 12) & 0x3F));
      p[k++] = (unsigned char)(0x80 | ((cp >> 6) & 0x3F));
      p[k++] = (unsigned char)(0x80 | (cp & 0x3F));
    }
  }
  p[k] = '\0';
  *out = guatiao_str_from((const char *)p, k);
  return 1;
}

/* Decodes one UTF-8 sequence at `p` (of `n` bytes left); answers its
 * length, or 0 when it is not UTF-8. */
static size_t utf8_next(const unsigned char *p, size_t n, uint32_t *cp) {
  unsigned char b = p[0];
  if (b < 0x80) {
    *cp = b;
    return 1;
  }
  size_t len = (b & 0xE0) == 0xC0 ? 2 : (b & 0xF0) == 0xE0 ? 3 : (b & 0xF8) == 0xF0 ? 4 : 0;
  if (!len || len > n) return 0;
  uint32_t v = b & (0x7F >> len);
  for (size_t i = 1; i < len; i++) {
    if ((p[i] & 0xC0) != 0x80) return 0;
    v = (v << 6) | (p[i] & 0x3F);
  }
  if ((len == 2 && v < 0x80) || (len == 3 && v < 0x800) || (len == 4 && (v < 0x10000 || v > 0x10FFFF)) ||
      (v >= 0xD800 && v <= 0xDFFF))
    return 0;
  *cp = v;
  return len;
}

mxArray *gt_utf8_mx(guatiao_str t) {
  const unsigned char *p = (const unsigned char *)t.ptr;
  mwSize dims[2] = {1, 0};
  if (!GT_WIDE_CHARS) {
    for (size_t i = 0; i < t.len;) {
      uint32_t cp;
      size_t k = utf8_next(p + i, t.len - i, &cp);
      if (!k) return NULL;
      i += k;
    }
    dims[1] = (mwSize)t.len;
    mxArray *x = mxCreateCharArray(2, dims);
    if (t.len) memcpy(mxGetChars(x), p, t.len);
    return x;
  }
  size_t units = 0;
  for (size_t i = 0; i < t.len;) {
    uint32_t cp;
    size_t k = utf8_next(p + i, t.len - i, &cp);
    if (!k) return NULL;
    units += cp >= 0x10000 ? 2 : 1;
    i += k;
  }
  dims[1] = (mwSize)units;
  mxArray *x = mxCreateCharArray(2, dims);
  uint16_t *u = (uint16_t *)mxGetChars(x);
  size_t j = 0;
  for (size_t i = 0; i < t.len;) {
    uint32_t cp = 0;
    i += utf8_next(p + i, t.len - i, &cp);
    if (cp >= 0x10000) {
      cp -= 0x10000;
      u[j++] = (uint16_t)(0xD800 + (cp >> 10));
      u[j++] = (uint16_t)(0xDC00 + (cp & 0x3FF));
    } else {
      u[j++] = (uint16_t)cp;
    }
  }
  return x;
}

int gt_text_out(gt_call *c, guatiao_str t, mxArray **out) {
  *out = gt_utf8_mx(t);
  if (!*out) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "the text is not UTF-8");
  return 1;
}

mxArray *gt_cstr_mx(const char *s) {
  mxArray *x = gt_utf8_mx(guatiao_str_from(s, strlen(s)));
  return x ? x : mxCreateString("");
}

/* ---- libraries ------------------------------------------------------------------------- */

static const char *const lib_base[3] = {"guatiao", "guatiao_serde", "guatiao_intake"};
static char chosen[4096];
static char opened[3][4096];
static int locked;

static void file_name(int lib, char *out, size_t n) {
#if defined(_WIN32)
  snprintf(out, n, "%s.dll", lib_base[lib]);
#elif defined(__APPLE__)
  snprintf(out, n, "lib%s.dylib", lib_base[lib]);
#else
  snprintf(out, n, "lib%s.so", lib_base[lib]);
#endif
}

static int is_dir(const char *p) {
  struct stat s;
  return stat(p, &s) == 0 && (s.st_mode & S_IFMT) == S_IFDIR;
}

/* The candidate for `lib` given a file or directory `p`. */
static void candidate_from(int lib, const char *p, char *out, size_t n) {
  char file[64];
  file_name(lib, file, sizeof file);
  if (is_dir(p)) {
    size_t len = strlen(p);
    int sep = len && (p[len - 1] == '/' || p[len - 1] == '\\');
    snprintf(out, n, "%s%s%s", p, sep ? "" : "/", file);
    return;
  }
  if (lib == GT_LIB_CORE) {
    snprintf(out, n, "%s", p);
    return;
  }
  const char *slash = strrchr(p, '/');
#ifdef _WIN32
  const char *back = strrchr(p, '\\');
  if (back && (!slash || back > slash)) slash = back;
#endif
  if (!slash) {
    snprintf(out, n, "%s", file);
    return;
  }
  snprintf(out, n, "%.*s/%s", (int)(slash - p), p, file);
}

int gt_use_library(gt_call *c, guatiao_str path) {
  if (path.len >= sizeof chosen) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "the path is too long");
  char want[4096];
  memcpy(want, path.ptr, path.len);
  want[path.len] = '\0';
  if (opened[GT_LIB_CORE][0]) {
    char as_core[4096];
    candidate_from(GT_LIB_CORE, want, as_core, sizeof as_core);
    if (strcmp(as_core, opened[GT_LIB_CORE]) != 0)
      return gt_fail(c, GUATIAO_ERR_BUSY, "guatiao is already loaded from %s", opened[GT_LIB_CORE]);
  }
  memcpy(chosen, want, path.len + 1);
  return 1;
}

int gt_need(gt_call *c, int lib) {
  if (gt_is_open(lib)) return 1;
  char path[4096], err[512], env[4096];
  if (chosen[0]) candidate_from(lib, chosen, path, sizeof path);
  else if (gt_getenv("GUATIAO_LIBRARY", env, sizeof env) && env[0]) candidate_from(lib, env, path, sizeof path);
  else file_name(lib, path, sizeof path);
  int r = gt_open(lib, path, err, sizeof err);
  if (r == 0) {
    snprintf(opened[lib], sizeof opened[lib], "%s", path);
    /* Trees and descriptors may now record code in this image: it stays. */
    gt_lock_gateway();
    return 1;
  }
  if (r == 2)
    return gt_fail_detail(c, GUATIAO_ERR_NOT_FOUND, "library", "%s at %s lacks the export %s: not a build of %s",
                          lib_base[lib], path, err, lib_base[lib]);
  return gt_fail_detail(c, GUATIAO_ERR_NOT_FOUND, "library",
                        "%s not found; looked at %s (%s). Set GUATIAO_LIBRARY or call guatiao.use_library().",
                        lib_base[lib], path, err);
}

int gt_missing(gt_call *c, int lib, const char *name) {
  const char *feature = gt_missing_feature(lib, name);
  if (feature)
    return gt_fail_detail(c, GUATIAO_ERR_UNSUPPORTED, "symbol", "%s at %s lacks %s: build it with the '%s' feature",
                          lib_base[lib], opened[lib], name, feature);
  return gt_fail_detail(c, GUATIAO_ERR_UNSUPPORTED, "symbol", "%s at %s lacks %s", lib_base[lib], opened[lib], name);
}

int gt_locked(void) { return locked; }

void gt_lock_gateway(void) {
  if (!locked) {
    mexLock();
    locked = 1;
  }
}

const char *gt_opened_path(int lib) { return lib >= 0 && lib <= 2 ? opened[lib] : ""; }

/* ---- the table of handles ---------------------------------------------------------------- */

typedef struct gt_entry {
  uint64_t id; /* 0 empty, UINT64_MAX a removed entry */
  int type;
  void *ptr;
} gt_entry;

static gt_entry *table;
static size_t table_cap, table_used, table_live;
static uint64_t next_id = 1;
#define GT_TOMB UINT64_MAX

static size_t slot_of(uint64_t id, size_t cap) { return (size_t)((id * 0x9E3779B97F4A7C15ULL) >> 7) & (cap - 1); }

static int table_grow(void) {
  size_t cap = table_cap ? table_cap * 2 : 256;
  while (table_live * 2 >= cap) cap *= 2;
  gt_entry *t = (gt_entry *)calloc(cap, sizeof(gt_entry));
  if (!t) return 0;
  for (size_t i = 0; i < table_cap; i++) {
    if (table[i].id == 0 || table[i].id == GT_TOMB) continue;
    size_t s = slot_of(table[i].id, cap);
    while (t[s].id) s = (s + 1) & (cap - 1);
    t[s] = table[i];
  }
  free(table);
  table = t;
  table_cap = cap;
  table_used = table_live;
  return 1;
}

uint64_t gt_handle_add(int type, void *ptr) {
  if ((table_used + 1) * 2 > table_cap && !table_grow()) return 0;
  uint64_t id = next_id++;
  size_t s = slot_of(id, table_cap);
  while (table[s].id && table[s].id != GT_TOMB) s = (s + 1) & (table_cap - 1);
  if (table[s].id == 0) table_used++;
  table[s].id = id;
  table[s].type = type;
  table[s].ptr = ptr;
  table_live++;
  return id;
}

static gt_entry *find(uint64_t id) {
  if (!table_cap || id == 0 || id == GT_TOMB) return NULL;
  size_t s = slot_of(id, table_cap);
  while (table[s].id) {
    if (table[s].id == id) return &table[s];
    s = (s + 1) & (table_cap - 1);
  }
  return NULL;
}

void *gt_handle_get(uint64_t id, int type) {
  gt_entry *e = find(id);
  return e && e->type == type ? e->ptr : NULL;
}

void *gt_handle_take(uint64_t id, int type) {
  gt_entry *e = find(id);
  if (!e || e->type != type) return NULL;
  void *p = e->ptr;
  e->id = GT_TOMB;
  e->ptr = NULL;
  table_live--;
  return p;
}

size_t gt_handle_count(int type) {
  size_t n = 0;
  for (size_t i = 0; i < table_cap; i++)
    if (table[i].id && table[i].id != GT_TOMB && table[i].type == type) n++;
  return n;
}

int gt_arg_handle(gt_call *c, int i, int type, const char *what, void **out) {
  uint64_t id = 0;
  if (!gt_arg_id(c, i, &id)) return 0;
  gt_entry *e = find(id);
  if (!e) return gt_fail_detail(c, GUATIAO_ERR_GONE, "closed", "the %s is closed", what);
  if (e->type != type) return gt_fail(c, GUATIAO_ERR_WRONG_KIND, "not a %s", what);
  *out = e->ptr;
  return 1;
}

/* ---- owners ---------------------------------------------------------------------------------- */

void gt_free(guatiao_value *v) {
  if (!v) return;
  switch (v->tag) {
  case GUATIAO_ABSENT:
  case GUATIAO_NULL:
  case GUATIAO_BOOL: break;
  default:
    if (gt_core.guatiao_value_free) gt_core.guatiao_value_free(v);
    return;
  }
  memset(v, 0, sizeof *v);
  v->tag = GUATIAO_NULL;
}

void gt_owner_drop(gt_owner *o) {
  if (!o) return;
  if (o->flags & GT_OWNED) {
    gt_free(o->root);
    free(o->root);
  }
  free(o);
}

uint64_t gt_owner_new(const guatiao_value *v, const guatiao_alloc *a) {
  gt_owner *o = (gt_owner *)calloc(1, sizeof(gt_owner));
  guatiao_value *root = (guatiao_value *)calloc(1, sizeof(guatiao_value));
  if (!o || !root) {
    free(o);
    free(root);
    return 0;
  }
  *root = *v;
  o->root = root;
  o->alloc = a ? a : &gt_binding_alloc;
  o->flags = GT_OWNED;
  uint64_t id = gt_handle_add(GT_H_OWNER, o);
  if (!id) {
    free(root);
    free(o);
  }
  return id;
}

uint64_t gt_owner_borrow(guatiao_value *node, const guatiao_alloc *a, int readonly) {
  gt_owner *o = (gt_owner *)calloc(1, sizeof(gt_owner));
  if (!o) return 0;
  o->root = node;
  o->alloc = a ? a : &gt_binding_alloc;
  o->flags = readonly ? GT_READONLY : 0;
  uint64_t id = gt_handle_add(GT_H_OWNER, o);
  if (!id) free(o);
  return id;
}

void gt_owner_end(uint64_t id) { gt_owner_drop((gt_owner *)gt_handle_take(id, GT_H_OWNER)); }

/* 0 found; 1 the owner is closed; 2 the path leads nowhere; 3 not a path. */
static int walk(const mxArray *owner_id, const mxArray *path, gt_owner **owner, guatiao_value **node) {
  if (!owner_id || mxGetNumberOfElements(owner_id) != 1 || !mxIsDouble(owner_id)) return 3;
  double d = mxGetScalar(owner_id);
  if (!(d >= 1 && d == floor(d) && d <= 9007199254740992.0)) return 1;
  gt_owner *o = (gt_owner *)gt_handle_get((uint64_t)d, GT_H_OWNER);
  if (!o) return 1;
  guatiao_value *v = o->root;
  if (path && !mxIsEmpty(path)) {
    if (!mxIsCell(path)) return 3;
    size_t n = mxGetNumberOfElements(path);
    for (size_t i = 0; i < n; i++) {
      const mxArray *step = mxGetCell(path, (mwIndex)i);
      if (!step) return 3;
      if (mxIsChar(step)) {
        gt_call scratch_call;
        memset(&scratch_call, 0, sizeof scratch_call);
        guatiao_str k;
        if (!gt_mx_utf8(&scratch_call, step, "a key", &k)) {
          gt_scratch_release(&scratch_call);
          return 3;
        }
        const guatiao_value *ch = guatiao_map_find(v, k);
        gt_scratch_release(&scratch_call);
        if (!ch) return 2;
        v = (guatiao_value *)ch;
      } else if (mxIsDouble(step) && mxGetNumberOfElements(step) == 1) {
        double at = mxGetScalar(step);
        if (!(at >= 0 && at == floor(at))) return 2;
        const guatiao_value *ch = guatiao_list_at(v, (size_t)at);
        if (!ch) return 2;
        v = (guatiao_value *)ch;
      } else {
        return 3;
      }
    }
  }
  if (owner) *owner = o;
  *node = v;
  return 0;
}

static int walk_failed(gt_call *c, int r) {
  if (r == 1) return gt_fail_detail(c, GUATIAO_ERR_GONE, "closed", "the value is closed");
  if (r == 2) return gt_fail(c, GUATIAO_ERR_NOT_FOUND, "the node this view addressed is no longer there");
  return gt_fail(c, GUATIAO_ERR_WRONG_KIND, "not a guatiao value");
}

int gt_arg_node(gt_call *c, int i, gt_owner **owner, guatiao_value **node) {
  int r = walk(gt_arg(c, i), gt_arg(c, i + 1), owner, node);
  return r ? walk_failed(c, r) : 1;
}

int gt_node_again(gt_call *c, int i, guatiao_value **node) { return gt_arg_node(c, i, NULL, node); }

int gt_walk_mx(gt_call *c, const mxArray *owner_id, const mxArray *path, guatiao_value **node) {
  int r = walk(owner_id, path, NULL, node);
  return r ? walk_failed(c, r) : 1;
}

const guatiao_alloc *gt_alloc_for(const guatiao_value *node, const gt_owner *o) {
  const guatiao_alloc *rec = NULL;
  if (node->tag == GUATIAO_MAP) rec = node->payload.map.alloc;
  else if (node->tag == GUATIAO_LIST) rec = node->payload.list.alloc;
  if (rec) return rec;
  return o && o->alloc ? o->alloc : &gt_binding_alloc;
}

int gt_arg_alloc(gt_call *c, int i, const guatiao_alloc **out) {
  const mxArray *a = gt_arg(c, i);
  if (!a || mxIsEmpty(a)) {
    *out = &gt_binding_alloc;
    return 1;
  }
  gt_counting *k = NULL;
  if (!gt_arg_handle(c, i, GT_H_COUNTING, "allocator", (void **)&k)) return 0;
  *out = &k->alloc;
  return 1;
}

/* ---- calling M code ---------------------------------------------------------------------------- */

int gt_call_m(gt_call *c, const char *fn, int nout, mxArray **out, int nin, mxArray **in) {
  for (int i = 0; i < nout; i++) out[i] = NULL;
  mxArray *ex = mexCallMATLABWithTrap(nout, out, nin, in, fn);
  if (!ex) return 1;
  /* The words of what was raised, through the dispatcher's own reader. */
  mxArray *words = NULL;
  mxArray *arg[1] = {ex};
  mxArray *trap = mexCallMATLABWithTrap(1, &words, 1, arg, "guatiao.internal.message_of");
  char buf[1024];
  buf[0] = '\0';
  if (!trap && words && mxIsChar(words)) {
    gt_call tmp;
    memset(&tmp, 0, sizeof tmp);
    guatiao_str t;
    if (gt_mx_utf8(&tmp, words, "a message", &t)) snprintf(buf, sizeof buf, "%.*s", (int)t.len, t.ptr);
    gt_scratch_release(&tmp);
  }
  if (trap) mxDestroyArray(trap);
  if (words) mxDestroyArray(words);
  mxDestroyArray(ex);
  return gt_fail(c, GUATIAO_ERR_INTERNAL, "%s", buf[0] ? buf : "M code raised an error");
}
